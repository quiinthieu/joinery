//! Typed generated edits on one physical helper. No public editing capability.
//! Callers must retain the private manual owner before invoking `execute` and
//! supply real durable admission/original recording; there are no no-op defaults.
#![cfg_attr(
    not(all(test, feature = "test-harness")),
    allow(
        dead_code,
        reason = "Typed native adapter precedes public UI staging and IPC wiring"
    )
)]
use super::*;
use crate::drivers::{
    snowflake_batch::{BatchRequest, FinishAction, Mode, Outcome},
    snowflake_edit_snapshot::{NativeEditBinding, PreparedSnowflakeEdits, SnapshotMutation},
    snowflake_mutations::{BindingKind, MutationKind},
    snowflake_process::HelperGeneration,
};
use sha2::{Digest, Sha256};
use tokio::sync::OwnedMutexGuard;

pub(crate) trait GridAdmission: Send {
    /// Err must mean no operation entered the supervisor. The implementation
    /// may only perform the synchronous audited dispatch handoff, without
    /// calling driver shutdown/observers or attempting any SQL itself.
    fn admit(&mut self, dispatch: &mut dyn FnMut() -> AppResult<Operation>)
    -> AppResult<Operation>;
}
pub(crate) trait GridOriginalRecorder: Send {
    /// Synchronous: own the exact frozen original before any await/publication.
    /// Ok means local capture AND settlement succeeded. Err retains the native
    /// terminal, reservation and original independently of this caller.
    fn record_original(&mut self, report: &Report, operation: &Operation) -> AppResult<()>;
    fn record_no_dispatch(&mut self, proof: &GridNoDispatch) -> AppResult<()>;
}

#[derive(Default)]
pub(super) struct State {
    owner: Mutex<Option<Arc<ManualState>>>,
}
impl State {
    pub(super) fn require_ordinary(&self) -> AppResult<()> {
        if self.owner.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This Snowflake session has owned generated edits; finish or disconnect them before ordinary SQL or metadata",
            ));
        }
        Ok(())
    }
}

struct ManualState {
    helper: HelperGeneration,
    context: LiveContext,
    state: Mutex<ManualStatus>,
}
struct ManualStatus {
    phase: ManualPhase,
    recording_blocked: Option<String>,
    rollback_only: bool,
}
#[derive(Clone)]
enum ManualPhase {
    Reserved,
    InFlight,
    Pending { token: String, transaction: String },
    Finished,
    Unknown,
    Released,
}

/// Opaque, same-session owner. This is independent of the storage group lease.
/// The Studio registry must retain this handle before dispatch, including across
/// recording failure, disconnected tabs and session-map removal.
pub(crate) struct GridManualTransaction {
    driver: Arc<SnowflakeExternal>,
    private: Arc<ManualState>,
}
impl Drop for GridManualTransaction {
    fn drop(&mut self) {
        self.release_unsubmitted();
        if !matches!(self.private.state.lock().phase, ManualPhase::Released) {
            self.driver.start_shutdown();
        }
    }
}

pub(crate) struct PreparedGridOperation {
    driver: Arc<SnowflakeExternal>,
    _lane: OwnedMutexGuard<()>,
    edits: Option<PreparedSnowflakeEdits>,
    manual: Option<Arc<GridManualTransaction>>,
    initial_manual: bool,
    id: String,
    op: &'static str,
    payload: Option<Value>,
    request: BatchRequest,
    digest: String,
    armed: bool,
    completed: bool,
    queued: bool,
}
#[derive(Clone)]
pub(crate) struct GridNoDispatch {
    helper: HelperGeneration,
    id: String,
    request: BatchRequest,
}
impl GridNoDispatch {
    pub(crate) fn matches(
        &self,
        helper: &HelperGeneration,
        id: &str,
        request: &BatchRequest,
    ) -> bool {
        self.helper.same_helper(helper) && self.id == id && self.request == *request
    }
}

pub(crate) struct GridOutcome {
    pub report: Report,
    pub recording_error: Option<AppError>,
    pub manual: Option<Arc<GridManualTransaction>>,
}

struct BoundedFrame(Vec<u8>);
#[derive(serde::Serialize)]
struct WireFrame<'a> {
    v: u8,
    generation: &'a str,
    id: &'a str,
    op: &'a str,
    payload: &'a Value,
}
impl std::io::Write for BoundedFrame {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > super::super::snowflake_process::MAX_FRAME)
        {
            return Err(std::io::Error::other("bounded generated edit frame"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake generated edit ownership changed; no new SQL was admitted",
    )
}
fn expected(context: &LiveContext) -> Value {
    json!({"user":context.principal,"role":context.role,"database":context.database,
        "schema":context.schema,"warehouse":context.warehouse,"session":context.session_id})
}

impl SnowflakeExternal {
    fn require_grid_profile(&self) -> AppResult<()> {
        if self.is_closed() {
            return Err(closed_error());
        }
        if self.console_mode.lock().is_some() || !self.edit_profile_enabled() {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Generated edits require an explicitly enabled writable external-browser profile outside the retained SQL console",
            ));
        }
        if self
            .transaction_authority
            .lock()
            .recovery
            .as_ref()
            .is_some_and(|r| r.restricted)
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake transaction state is unavailable; use the reviewed rollback control or disconnect",
            ));
        }
        #[cfg(all(unix, feature = "test-harness"))]
        self.check_metadata_observer()?;
        Ok(())
    }

    fn current_grid_binding_locked(
        &self,
        binding: &NativeEditBinding,
        browsing: &BrowseState,
    ) -> AppResult<NativeEditBinding> {
        if self.is_closed()
            || binding.session_id() != self.session_id
            || !binding
                .helper_generation()
                .same_helper(&self.process.helper_generation())
        {
            return Err(conflict());
        }
        self.edit_binding(
            browsing,
            &PreviewRelationRequest {
                revision: binding.browse_revision().into(),
                target_id: binding.target_id().into(),
                inspection_revision: binding.inspection_revision().into(),
            },
        )
    }

    fn current_grid_binding(&self, binding: &NativeEditBinding) -> AppResult<NativeEditBinding> {
        self.current_grid_binding_locked(binding, &self.browsing.lock())
    }

    pub(crate) async fn prepare_grid_operation(
        self: &Arc<Self>,
        edits: PreparedSnowflakeEdits,
        mode: Mode,
    ) -> AppResult<PreparedGridOperation> {
        let lane =
            tokio::time::timeout(Duration::from_millis(1500), self.lane.clone().lock_owned())
                .await
                .map_err(|_| conflict())?;
        self.require_unrestricted()?;
        self.require_grid_profile()?;
        edits.validate_current(&self.current_grid_binding(edits.binding())?)?;
        self.verify_grid_freshness(edits.binding(), None).await?;
        edits.validate_current(&self.current_grid_binding(edits.binding())?)?;
        let manual = if mode == Mode::Manual {
            let private = Arc::new(ManualState {
                helper: self.process.helper_generation(),
                context: edits.binding().context().clone(),
                state: Mutex::new(ManualStatus {
                    phase: ManualPhase::Reserved,
                    recording_blocked: None,
                    rollback_only: false,
                }),
            });
            let mut state = self.grid.owner.lock();
            if state.is_some() {
                return Err(conflict());
            }
            *state = Some(private.clone());
            Some(Arc::new(GridManualTransaction {
                driver: self.clone(),
                private,
            }))
        } else {
            None
        };
        PreparedGridOperation::edits(self.clone(), lane, edits, mode, manual, true)
    }
}

impl GridManualTransaction {
    fn check(&self, allow_recording_failure: bool) -> AppResult<(String, String)> {
        self.driver.require_grid_profile()?;
        if !self
            .private
            .helper
            .same_helper(&self.driver.process.helper_generation())
            || !self
                .driver
                .grid
                .owner
                .lock()
                .as_ref()
                .is_some_and(|s| Arc::ptr_eq(s, &self.private))
        {
            return Err(conflict());
        }
        let state = self.private.state.lock();
        if (state.recording_blocked.is_some() || state.rollback_only) && !allow_recording_failure {
            return Err(conflict());
        }
        match &state.phase {
            ManualPhase::Pending { token, transaction } => Ok((token.clone(), transaction.clone())),
            _ => Err(conflict()),
        }
    }

    /// SQL-free release hook, called only after the retained original settles.
    /// Unknown remote disposition can never restore ordinary session authority.
    pub(crate) fn recording_settled(&self, operation_id: &str) -> AppResult<()> {
        let mut owner = self.driver.grid.owner.lock();
        if !owner
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(s, &self.private))
        {
            return Err(conflict());
        }
        let mut state = self.private.state.lock();
        if state.recording_blocked.as_deref() != Some(operation_id) {
            return Err(conflict());
        }
        match state.phase {
            ManualPhase::Pending { .. } => state.recording_blocked = None,
            ManualPhase::Finished => {
                state.recording_blocked = None;
                state.phase = ManualPhase::Released;
                owner.take();
            }
            _ => return Err(conflict()),
        }
        Ok(())
    }

    pub(crate) async fn preview_relation(
        &self,
        request: &PreviewRelationRequest,
    ) -> AppResult<RelationPreview> {
        let _lane = tokio::time::timeout(
            Duration::from_millis(1500),
            self.driver.lane.clone().lock_owned(),
        )
        .await
        .map_err(|_| conflict())?;
        let (token, transaction) = self.check(false)?;
        let binding = {
            let state = self.driver.browsing.lock();
            self.driver.edit_binding(&state, request)?
        };
        if binding.context() != &self.private.context
            || !binding
                .helper_generation()
                .same_helper(&self.private.helper)
        {
            return Err(conflict());
        }
        let preview_revision = self.driver.begin_edit_preview(request)?;
        let preview = self
            .driver
            .grid_fresh_preview(
                &binding,
                super::grid_freshness::GridReadOwner {
                    token: &token,
                    transaction_id: &transaction,
                },
            )
            .await?;
        self.check(false)?;
        {
            let state = self.driver.browsing.lock();
            let after = self.driver.edit_binding(&state, request)?;
            if after.context() != &self.private.context
                || !after.helper_generation().same_helper(&self.private.helper)
            {
                return Err(conflict());
            }
        }
        self.driver
            .retain_edit_preview(request, &preview_revision, preview)
    }

    pub(crate) async fn prepare_continuation_edits(
        &self,
        request: &PreviewRelationRequest,
        snapshot_id: &str,
        mutations: &[SnapshotMutation],
    ) -> AppResult<PreparedSnowflakeEdits> {
        let _lane = tokio::time::timeout(
            Duration::from_millis(1500),
            self.driver.lane.clone().lock_owned(),
        )
        .await
        .map_err(|_| conflict())?;
        self.check(false)?;
        let state = self.driver.browsing.lock();
        if self.driver.is_closed() {
            return Err(closed_error());
        }
        let binding = self.driver.edit_binding(&state, request)?;
        if binding.context() != &self.private.context
            || !binding
                .helper_generation()
                .same_helper(&self.private.helper)
        {
            return Err(conflict());
        }
        self.driver.edit_snapshots.prepare(
            &self.driver.session_id,
            snapshot_id,
            &binding,
            mutations,
        )
    }

    pub(crate) async fn prepare_grid_operation(
        self: &Arc<Self>,
        edits: PreparedSnowflakeEdits,
        rollback_prior: bool,
    ) -> AppResult<PreparedGridOperation> {
        if !rollback_prior {
            return Err(AppError::invalid(
                "Continuing generated edits requires explicit whole-transaction rollback permission",
            ));
        }
        let lane = tokio::time::timeout(
            Duration::from_millis(1500),
            self.driver.lane.clone().lock_owned(),
        )
        .await
        .map_err(|_| conflict())?;
        let (token, transaction) = self.check(false)?;
        if edits.binding().context() != &self.private.context {
            return Err(conflict());
        }
        edits.validate_current(&self.driver.current_grid_binding(edits.binding())?)?;
        self.driver
            .verify_grid_freshness(
                edits.binding(),
                Some(super::grid_freshness::GridReadOwner {
                    token: &token,
                    transaction_id: &transaction,
                }),
            )
            .await?;
        self.check(false)?;
        edits.validate_current(&self.driver.current_grid_binding(edits.binding())?)?;
        PreparedGridOperation::edits(
            self.driver.clone(),
            lane,
            edits,
            Mode::Manual,
            Some(self.clone()),
            false,
        )
    }

    pub(crate) async fn prepare_finish(
        self: &Arc<Self>,
        action: FinishAction,
    ) -> AppResult<PreparedGridOperation> {
        let lane = tokio::time::timeout(
            Duration::from_millis(1500),
            self.driver.lane.clone().lock_owned(),
        )
        .await
        .map_err(|_| conflict())?;
        let (token, transaction) = self.check(action == FinishAction::Rollback)?;
        if action == FinishAction::Commit {
            self.driver
                .verify_grid_owner_context(
                    &self.private.context,
                    super::grid_freshness::GridReadOwner {
                        token: &token,
                        transaction_id: &transaction,
                    },
                )
                .await?;
            self.check(false)?;
        }
        // Safety rollback deliberately uses no stale snapshot or fresh SQL probe.
        let payload =
            json!({"owner":token,"action":action,"expected":expected(&self.private.context)});
        PreparedGridOperation::new(
            self.driver.clone(),
            lane,
            None,
            Some(self.clone()),
            false,
            "mutationFinish",
            payload,
        )
    }
}

impl PreparedGridOperation {
    fn edits(
        driver: Arc<SnowflakeExternal>,
        lane: OwnedMutexGuard<()>,
        mut edits: PreparedSnowflakeEdits,
        mode: Mode,
        manual: Option<Arc<GridManualTransaction>>,
        initial: bool,
    ) -> AppResult<Self> {
        edits.reserve_dispatch()?;
        let owner = if initial {
            None
        } else {
            Some(manual.as_ref().ok_or_else(conflict)?.check(false)?.0)
        };
        let units = edits
            .compiled()
            .units()
            .iter()
            .map(|unit| {
                let kind = match unit.kind() {
                    MutationKind::Insert => "insert",
                    MutationKind::Update => "update",
                    MutationKind::Delete => "delete",
                };
                let binds = unit
                    .bindings()
                    .iter()
                    .map(|bind| match (bind.kind(), bind.value()) {
                        (_, None) => Ok(Value::Null),
                        (BindingKind::Boolean, Some("true")) => Ok(Value::Bool(true)),
                        (BindingKind::Boolean, Some("false")) => Ok(Value::Bool(false)),
                        (BindingKind::Boolean, _) => Err(protocol_error()),
                        (_, Some(value)) => Ok(Value::String(value.into())),
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                Ok(json!({"kind":kind,"sql":unit.sql(),"binds":binds}))
            })
            .collect::<AppResult<Vec<_>>>()?;
        let payload = json!({"mode":mode,"owner":owner,"rollbackPrior":!initial,"expected":expected(edits.binding().context()),"units":units});
        Self::new(
            driver,
            lane,
            Some(edits),
            manual,
            initial,
            "mutationBatch",
            payload,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Retain every private admission component in one linear operation"
    )]
    fn new(
        driver: Arc<SnowflakeExternal>,
        lane: OwnedMutexGuard<()>,
        edits: Option<PreparedSnowflakeEdits>,
        manual: Option<Arc<GridManualTransaction>>,
        initial_manual: bool,
        op: &'static str,
        payload: Value,
    ) -> AppResult<Self> {
        let id = uuid::Uuid::new_v4().to_string();
        let request = BatchRequest::parse(op, &payload)
            .map_err(control_error)?
            .ok_or_else(protocol_error)?;
        let mut encoded = BoundedFrame(Vec::new());
        let helper = driver.process.helper_generation();
        let frame = WireFrame {
            v: 1,
            generation: helper.generation(),
            id: &id,
            op,
            payload: &payload,
        };
        if serde_json::to_writer(&mut encoded, &frame).is_err() {
            if let Some(owner) = &manual
                && initial_manual
            {
                owner.release_unsubmitted();
            }
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "The encoded generated edit batch exceeds the helper frame limit; split the reviewed batch before submission",
            ));
        }
        Ok(Self {
            driver,
            _lane: lane,
            edits,
            manual,
            initial_manual,
            id,
            op,
            payload: Some(payload),
            request,
            digest: format!("{:x}", Sha256::digest(encoded.0)),
            armed: false,
            completed: false,
            queued: false,
        })
    }

    pub(crate) fn operation_id(&self) -> &str {
        &self.id
    }
    pub(crate) fn request(&self) -> &BatchRequest {
        &self.request
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn manual_owner(&self) -> Option<Arc<GridManualTransaction>> {
        self.manual.clone()
    }

    pub(crate) async fn execute(
        mut self,
        admission: &mut dyn GridAdmission,
        recorder: &mut dyn GridOriginalRecorder,
    ) -> AppResult<GridOutcome> {
        let result = self.execute_inner(admission, recorder).await;
        if !self.queued {
            if let Some(owner) = &self.manual
                && !self.initial_manual
            {
                let mut state = owner.private.state.lock();
                state.recording_blocked = Some(self.id.clone());
                state.rollback_only = true;
            }
            let proof = GridNoDispatch {
                helper: self.driver.process.helper_generation(),
                id: self.id.clone(),
                request: self.request.clone(),
            };
            let retire_after_refusal = matches!(
                self.request,
                BatchRequest::Finish {
                    action: FinishAction::Rollback,
                    ..
                }
            );
            // Keep retirement armed through the synchronous recorder: even a
            // panic after retaining proof must not leave this consumed credit
            // attached to a live owner. Drop closes the exact helper on unwind.
            if retire_after_refusal {
                self.armed = true;
            }
            // Local no-dispatch recording failure cannot replace the primary
            // refusal. Its owner retains the private proof and reservation.
            let _ = recorder.record_no_dispatch(&proof);
            if retire_after_refusal {
                // Protected recording credit may already belong to this exact
                // rollback ID. A new ID cannot reuse it. Retire rather than
                // leave a live Pending owner offering an inadmissible retry;
                // reap is local cleanup evidence, never remote rollback proof.
                self.driver.start_shutdown();
            }
        }
        result
    }

    async fn execute_inner(
        &mut self,
        admission: &mut dyn GridAdmission,
        recorder: &mut dyn GridOriginalRecorder,
    ) -> AppResult<GridOutcome> {
        let driver = self.driver.clone();
        let mut prior_phase = None;
        let mut operation = {
            // Same lock as synchronous shutdown revocation. Callback is only the
            // audited atomic recording-cell handoff; no await or observers here.
            let shutdown_gate = driver.console_mode.lock();
            if driver.is_closed() {
                return Err(closed_error());
            }
            if shutdown_gate.is_some() {
                return Err(conflict());
            }
            let browsing = driver.browsing.lock();
            if let Some(edits) = &self.edits {
                edits.validate_current(
                    &driver.current_grid_binding_locked(edits.binding(), &browsing)?,
                )?;
            }
            let mut prior = None;
            if let Some(owner) = &self.manual {
                let mut state = owner.private.state.lock();
                if self.initial_manual {
                    if !matches!(state.phase, ManualPhase::Reserved) {
                        return Err(conflict());
                    }
                } else {
                    let rollback = matches!(
                        self.request,
                        BatchRequest::Finish {
                            action: FinishAction::Rollback,
                            ..
                        }
                    );
                    if !matches!(state.phase, ManualPhase::Pending { .. })
                        || ((state.recording_blocked.is_some() || state.rollback_only) && !rollback)
                    {
                        return Err(conflict());
                    }
                }
                prior_phase = Some(state.phase.clone());
                prior = Some(state.phase.clone());
                state.phase = ManualPhase::InFlight;
                state.recording_blocked = Some(self.id.clone());
            }
            self.armed = true;
            let mut submitted = false;
            let mut payload = self.payload.take();
            let mut dispatch = || {
                if submitted {
                    return Err(conflict());
                }
                submitted = true;
                let operation = driver
                    .process
                    .request(
                        &self.id,
                        self.op,
                        payload.take().ok_or_else(conflict)?,
                        Impact::Mutating,
                    )
                    .map_err(control_error)?;
                self.queued = true;
                Ok(operation)
            };
            let admitted = admission.admit(&mut dispatch);
            match admitted {
                Ok(operation)
                    if self.queued
                        && operation.matches_batch_binding(
                            &driver.process.helper_generation(),
                            &self.id,
                            &self.request,
                        ) =>
                {
                    operation
                }
                Ok(_) => return Err(protocol_error()),
                Err(error) => {
                    if !self.queued {
                        self.armed = false;
                        if let (Some(owner), Some(phase)) = (&self.manual, prior) {
                            let mut state = owner.private.state.lock();
                            state.phase = phase;
                            if self.initial_manual {
                                state.recording_blocked = None;
                            } else {
                                state.recording_blocked = Some(self.id.clone());
                                state.rollback_only = true;
                            }
                        }
                    }
                    return Err(error);
                }
            }
        };
        *driver.active.lock() = Some(Active {
            token: operation.token(),
            terminal: operation.terminal_receiver(),
        });
        let _active = ActiveLease(&driver.active);
        {
            let _browse = driver.browsing.lock();
            driver.revoke_edit_snapshots();
        }
        let report = operation.wait_bounded(Duration::from_secs(120)).await;
        if let Some(owner) = &self.manual {
            let mut state = owner.private.state.lock();
            state.phase = match report.batch.as_ref().and_then(|e| e.summary.as_ref()) {
                Some(summary) if summary.outcome == Outcome::Pending => {
                    match (&summary.owner, &summary.transaction_id) {
                        (Some(token), Some(transaction)) => ManualPhase::Pending {
                            token: token.to_string(),
                            transaction: transaction.clone(),
                        },
                        _ => ManualPhase::Unknown,
                    }
                }
                Some(summary)
                    if summary.outcome == Outcome::Rejected
                        && report.batch.as_ref().is_some_and(|e| e.steps.is_empty())
                        && matches!(&prior_phase, Some(ManualPhase::Pending {token,transaction})
                        if summary.owner.as_ref().map(ToString::to_string).as_deref()==Some(token.as_str())
                        && summary.transaction_id.as_deref()==Some(transaction.as_str())) =>
                {
                    prior_phase.clone().unwrap()
                }
                Some(summary)
                    if matches!(summary.outcome, Outcome::Committed | Outcome::RolledBack)
                        || self.initial_manual && summary.outcome == Outcome::Rejected =>
                {
                    ManualPhase::Finished
                }
                _ => ManualPhase::Unknown,
            };
        }
        // No await between the original native terminal and synchronous capture.
        let recording_error = recorder.record_original(&report, &operation).err();
        if let Some(owner) = &self.manual {
            if recording_error.is_none() {
                let _ = owner.recording_settled(&self.id);
            }
            if matches!(owner.private.state.lock().phase, ManualPhase::Unknown) {
                driver.start_shutdown();
            }
        } else if report
            .batch
            .as_ref()
            .and_then(|e| e.summary.as_ref())
            .is_none_or(|s| s.outcome == Outcome::Unknown)
        {
            driver.start_shutdown();
        }
        self.completed = true;
        Ok(GridOutcome {
            report,
            recording_error,
            manual: self.manual.clone(),
        })
    }
}

impl GridManualTransaction {
    fn release_unsubmitted(&self) {
        let mut owner = self.driver.grid.owner.lock();
        if !owner
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(s, &self.private))
        {
            return;
        }
        let mut state = self.private.state.lock();
        if matches!(state.phase, ManualPhase::Reserved) {
            state.phase = ManualPhase::Released;
            owner.take();
        }
    }
}
impl Drop for PreparedGridOperation {
    fn drop(&mut self) {
        if self.armed && !self.completed {
            self.driver.start_shutdown();
        }
        if !self.armed
            && self.initial_manual
            && let Some(owner) = &self.manual
        {
            owner.release_unsubmitted();
        }
    }
}
