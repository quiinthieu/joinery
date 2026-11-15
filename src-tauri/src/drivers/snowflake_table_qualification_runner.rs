//! Retained closed-plan Studio execution. Consumed construction requires the
//! sealed offline envelope; the private factory supplies fresh components.
//! No account CLI exists.
use super::{
    Operation, Plan, Readback, SCHEMA, StatementId,
    envelope::ConsumedEnvelope,
    evidence::PhaseEvidence,
    guard::Guard,
    policy::{OperationCompletion, Policy},
};
use crate::drivers::snowflake_qualification_transport::TransportLimits;
use crate::{
    domain::*,
    drivers::{
        snowflake_qualification::QualificationAttachment,
        snowflake_qualification_transport::{TransportAttachment, TransportPolicy},
    },
    execution::{EventHandler, Studio},
    relation_inspection::{BrowseCatalog, InspectRelationRequest, RelationInspection},
};
use arrow_array::{Array, StringArray};
use arrow_ipc::reader::StreamReader;
use parking_lot::Mutex;
use std::{io::Cursor, sync::Arc, time::Duration};

pub(super) const EVENT_LIMIT: usize = 32;
pub(super) const READ_BYTES: usize = 64 * 1024;
const AUTHENTICATION_LIMIT: Duration = Duration::from_secs(120);

fn invalid() -> AppError {
    AppError::invalid("Closed qualification runner result is incomplete")
}

pub(super) enum Output {
    Startup(SessionInfo),
    Preflight(SnowflakeDdlPreflightResponse),
    Ddl(SnowflakeDdlResult),
    Reconcile(SessionContextOptions, BrowseCatalog),
    Catalog(BrowseCatalog),
    Inspect(RelationInspection),
    Read(ReadCapture),
    Boundary(TransactionControlResult),
    Dml(SnowflakeDmlResult),
    Options(SessionContextOptions),
}

impl Output {
    pub(super) fn evidence(&self) -> PhaseEvidence<'_> {
        match self {
            Self::Startup(v) => PhaseEvidence::Startup(v),
            Self::Preflight(v) => PhaseEvidence::Preflight(v),
            Self::Ddl(v) => PhaseEvidence::Ddl(v),
            Self::Reconcile(options, catalog) => PhaseEvidence::Reconcile { options, catalog },
            Self::Catalog(v) => PhaseEvidence::Catalog(v),
            Self::Inspect(v) => PhaseEvidence::Inspect(v),
            Self::Read(v) => PhaseEvidence::Read {
                events: &v.events,
                result: &v.result,
                rows: &v.rows,
            },
            Self::Boundary(v) => PhaseEvidence::Boundary(v),
            Self::Dml(v) => PhaseEvidence::Dml(v),
            Self::Options(v) => PhaseEvidence::Options(v),
        }
    }
}

pub(super) struct ReadCapture {
    pub(super) events: Vec<QueryEvent>,
    pub(super) result: ResultInfo,
    pub(super) rows: Vec<TextRow>,
}

pub(super) struct CallError {
    pub(super) error: AppError,
    pub(super) events: Vec<QueryEvent>,
    /// False means events is only the bounded original prefix, never a complete receipt.
    pub(super) capture_complete: bool,
    pub(super) original_error: Option<Box<AppError>>,
    pub(super) cleanup_error: Option<Box<AppError>>,
    pending: Option<Box<PendingCapture>>,
}

impl From<AppError> for CallError {
    fn from(error: AppError) -> Self {
        Self {
            error,
            events: Vec::new(),
            capture_complete: true,
            original_error: None,
            cleanup_error: None,
            pending: None,
        }
    }
}

struct PendingCapture {
    collector: Arc<Mutex<Collector>>,
    completion: tokio::sync::watch::Receiver<(bool, bool)>,
}

impl CallError {
    pub(super) fn captured(error: AppError, snapshot: CaptureSnapshot) -> Self {
        let complete = snapshot.complete;
        Self {
            error: match snapshot.error {
                Some(incomplete) => incomplete,
                None => error.clone(),
            },
            events: snapshot.events,
            capture_complete: complete,
            original_error: if complete {
                None
            } else {
                Some(Box::new(error))
            },
            cleanup_error: None,
            pending: None,
        }
    }

    pub(super) fn completion(&self) -> OperationCompletion<'_> {
        if self.capture_complete {
            OperationCompletion::Failed {
                error: &self.error,
                events: &self.events,
            }
        } else {
            OperationCompletion::IncompleteRead {
                error: &self.error,
                original_error: self.original_error.as_deref(),
                events: &self.events,
                cleanup_error: self.cleanup_error.as_deref(),
            }
        }
    }

    async fn after_shutdown(&mut self) {
        if let Some(mut pending) = self.pending.take() {
            let _ = wait_finished(&mut pending.completion).await;
            let snapshot = pending.collector.lock().complete_snapshot();
            self.retain_cleanup_snapshot(snapshot);
        }
    }

    pub(super) fn retain_cleanup_snapshot(&mut self, snapshot: CaptureSnapshot) {
        self.events = snapshot.events;
        // A failed original collection remains incomplete even if cleanup
        // later delivers Finished; retain that actual suffix without upgrading
        // the failed drain or dropping its original error provenance.
        self.capture_complete &= snapshot.complete;
        if let Some(error) = snapshot.error {
            self.error = error;
        }
    }
}

pub(super) struct CaptureSnapshot {
    pub(super) events: Vec<QueryEvent>,
    pub(super) complete: bool,
    pub(super) error: Option<AppError>,
}

impl CaptureSnapshot {
    /// A timeout/channel failure in the first terminal drain remains a failed
    /// collection even when disconnect subsequently delivers the original Finished.
    pub(super) fn after_failed_drain(mut self) -> Self {
        self.complete = false;
        self.error = Some(incomplete_capture());
        self
    }
}

#[derive(Default)]
pub(super) struct Collector {
    events: Vec<QueryEvent>,
    bytes: usize,
    error: Option<AppError>,
}

fn incomplete_capture() -> AppError {
    AppError::internal(
        "Incomplete original qualification read event prefix (bounded capture or terminal drain failed)",
    )
}

impl Collector {
    pub(super) fn complete_snapshot(&self) -> CaptureSnapshot {
        let complete =
            self.error.is_none() && matches!(self.events.last(), Some(QueryEvent::Finished { .. }));
        CaptureSnapshot {
            events: self.events.clone(),
            complete,
            error: if complete {
                None
            } else {
                Some(incomplete_capture())
            },
        }
    }

    pub(super) fn push(&mut self, event: QueryEvent) -> bool {
        if self.error.is_some() {
            return false;
        }
        let bytes = match serde_json::to_vec(&event) {
            Ok(v) => v.len(),
            Err(_) => {
                self.error = Some(incomplete_capture());
                return false;
            }
        };
        if self.events.len() >= EVENT_LIMIT || self.bytes.saturating_add(bytes) > READ_BYTES {
            self.error = Some(incomplete_capture());
            return false;
        }
        self.bytes += bytes;
        self.events.push(event);
        true
    }
}

pub(super) async fn wait_finished(
    completion: &mut tokio::sync::watch::Receiver<(bool, bool)>,
) -> AppResult<()> {
    match tokio::time::timeout(Duration::from_secs(15), async {
        while !completion.borrow_and_update().1 {
            completion
                .changed()
                .await
                .map_err(|_| incomplete_capture())?;
        }
        Ok(())
    })
    .await
    {
        Ok(result) => result,
        Err(_) => Err(incomplete_capture()),
    }
}

fn approve(studio: &Studio, plan: &PreparedPlan) -> AppResult<Option<String>> {
    if plan.requires_passphrase {
        return Err(invalid());
    }
    if plan.requires_approval {
        studio
            .authorize(&plan.id, None, plan.confirmation_text.as_deref())
            .map(Some)
    } else {
        Ok(None)
    }
}

async fn read(
    studio: &Arc<Studio>,
    session: &str,
    closed_plan: &Plan,
    expected: Readback,
    cancel_on_stream: bool,
    completion_deadline: Option<std::time::Instant>,
) -> Result<ReadCapture, CallError> {
    let id = if expected == Readback::Ownership {
        StatementId::OwnershipRead
    } else {
        StatementId::DataRead
    };
    let plan = studio.prepare_execution(session, closed_plan.statement(id).source_sql())?;
    if plan.reviewed_snowflake_dml
        || plan.reviewed_snowflake_ddl
        || plan.statements.len() != 1
        || !plan.bound_parameters.is_empty()
    {
        return Err(invalid().into());
    }
    let approval = approve(studio, &plan)?;
    let capture = Arc::new(Mutex::new(Collector::default()));
    let (progress, mut completion) = tokio::sync::watch::channel((false, false));
    let handler: EventHandler = Arc::new({
        let capture = capture.clone();
        move |event| {
            let finished = matches!(event, QueryEvent::Finished { .. });
            let streaming = matches!(&event, QueryEvent::Result { result } if result.status == RetentionStatus::Streaming);
            let accepted = capture.lock().push(event);
            progress.send_modify(|value| {
                value.0 |= streaming;
                value.1 |= finished;
            });
            accepted
        }
    });
    let run = studio.execute(&plan.id, approval.as_deref(), handler)?;
    let deadline = completion_deadline
        .map(tokio::time::Instant::from_std)
        .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(15));
    let settled = tokio::time::timeout_at(deadline, async {
        while {
            let (streaming, finished) = *completion.borrow_and_update();
            !(finished || cancel_on_stream && streaming)
        } {
            completion.changed().await.map_err(|_| invalid())?;
        }
        Ok::<_, AppError>(())
    })
    .await;
    let cancelled = cancel_on_stream || !matches!(settled, Ok(Ok(())));
    if cancelled {
        let cancel_result = studio.cancel(&run).await;
        // Cancel requests do not join run_plan. Retain the collector until its
        // original terminal event, including late errors/state/metrics, arrives.
        // A failed local drain is a harness failure, never a complete receipt.
        let drained = wait_finished(&mut completion).await;
        let cleanup_error = if drained.is_err() {
            let disconnected = studio.disconnect_and_wait(session).await.err();
            let _ = wait_finished(&mut completion).await;
            disconnected
        } else {
            None
        };
        let snapshot = capture.lock().complete_snapshot();
        let snapshot = if drained.is_err() {
            snapshot.after_failed_drain()
        } else {
            snapshot
        };
        let error = match snapshot
            .events
            .iter()
            .find_map(|event| match event {
                QueryEvent::Error { error, .. } => Some(error.clone()),
                _ => None,
            })
            .or_else(|| cancel_result.err())
        {
            Some(error) => error,
            None => AppError::new(
                ErrorCode::Cancelled,
                "Closed qualification read collection cancelled",
            ),
        };
        let finished = completion.borrow().1;
        let pending = if finished {
            None
        } else {
            Some(Box::new(PendingCapture {
                collector: capture,
                completion,
            }))
        };
        let mut original = CallError::captured(error, snapshot);
        original.pending = pending;
        original.cleanup_error = cleanup_error.map(Box::new);
        return Err(original);
    }
    let snapshot = capture.lock().complete_snapshot();
    if !snapshot.complete {
        return Err(CallError::captured(incomplete_capture(), snapshot));
    }
    let events = snapshot.events;
    let parsed = (|| {
        if let Some(error) = events.iter().find_map(|event| match event {
            QueryEvent::Error { error, .. } => Some(error.clone()),
            _ => None,
        }) {
            return Err(error);
        }
        let result = events
            .iter()
            .rev()
            .find_map(|event| match event {
                QueryEvent::Result { result } if result.status == RetentionStatus::Complete => {
                    Some(result.clone())
                }
                _ => None,
            })
            .ok_or_else(invalid)?;
        if result.retained_rows > 1 {
            return Err(invalid());
        }
        let bytes = studio.range(&result.id, 0, 2)?;
        if bytes.len() > READ_BYTES {
            return Err(invalid());
        }
        let reader = StreamReader::try_new(Cursor::new(bytes), None).map_err(|_| invalid())?;
        if reader.schema().fields().len() != result.columns.len() {
            return Err(invalid());
        }
        let mut rows = Vec::new();
        for batch in reader {
            let batch = batch.map_err(|_| invalid())?;
            if rows.len().saturating_add(batch.num_rows()) > 1 {
                return Err(invalid());
            }
            for row in 0..batch.num_rows() {
                let values = batch
                    .columns()
                    .iter()
                    .map(|column| {
                        let column = column
                            .as_any()
                            .downcast_ref::<StringArray>()
                            .ok_or_else(invalid)?;
                        Ok(if column.is_null(row) {
                            None
                        } else {
                            Some(column.value(row).to_owned())
                        })
                    })
                    .collect::<AppResult<TextRow>>()?;
                rows.push(values);
            }
        }
        if rows.len() != result.retained_rows {
            return Err(invalid());
        }
        Ok((result, rows))
    })();
    match parsed {
        Ok((result, rows)) => Ok(ReadCapture {
            events,
            result,
            rows,
        }),
        Err(error) => Err(CallError {
            error,
            events,
            capture_complete: true,
            original_error: None,
            cleanup_error: None,
            pending: None,
        }),
    }
}

#[derive(Default)]
struct RouteState {
    session: Option<SessionInfo>,
    runtime: Option<SessionRuntimeState>,
    catalog: Option<BrowseCatalog>,
    inspection: Option<RelationInspection>,
    preflight: Option<(SnowflakeDdlAction, SnowflakeDdlPreflightResponse)>,
}

impl RouteState {
    fn accept(&mut self, output: &Output) {
        match output {
            Output::Startup(v) => self.session = Some(v.clone()),
            Output::Ddl(v) => self.runtime = Some(v.state.clone()),
            Output::Reconcile(options, _) | Output::Options(options) => {
                self.runtime = Some(options.state.clone())
            }
            Output::Catalog(v) => {
                self.catalog = Some(v.clone());
                self.inspection = None;
            }
            Output::Inspect(v) => self.inspection = Some(v.clone()),
            Output::Read(v) => {
                self.runtime = v.events.iter().find_map(|event| match event {
                    QueryEvent::SessionState { state } => Some(state.clone()),
                    _ => None,
                })
            }
            Output::Boundary(v) => self.runtime = Some(v.state.clone()),
            Output::Dml(v) => self.runtime = Some(v.state.clone()),
            Output::Preflight(_) => {}
        }
    }
}

pub(super) struct Runner {
    studio: Arc<Studio>,
    policy: Arc<Policy>,
    guard: Option<Arc<Guard>>,
    attachment: Option<QualificationAttachment>,
    transport: TransportAttachment,
    plan: Plan,
    attempt: String,
    state: RouteState,
    #[cfg(test)]
    controls: FixtureControls,
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct FixtureControls {
    pub(super) cancel_read: bool,
    pub(super) authentication_limit: Option<Duration>,
    pub(super) finished: Option<tokio::sync::oneshot::Sender<()>>,
    pub(super) pause: Option<Pause>,
    pub(super) break_journal: Option<(u8, std::path::PathBuf)>,
}

#[cfg(test)]
pub(super) struct Pause {
    pub(super) phase: u8,
    pub(super) reached: tokio::sync::oneshot::Sender<()>,
    pub(super) resume: Arc<tokio::sync::Notify>,
}

pub(super) struct RunnerReceipt {
    pub(super) outputs: Vec<Output>,
    pub(super) failure: Option<(u8, AppError)>,
    pub(super) original_call_failure: Option<CallError>,
    pub(super) completion_record_error: Option<AppError>,
    pub(super) shutdown: AppResult<()>,
    pub(super) settled: bool,
    pub(super) persisted: AppResult<()>,
    /// Final integrity/deadline check, independent of all original call outcomes.
    pub(super) guard_revalidation: Option<AppResult<()>>,
    /// Blocking inventory owners may outlive a cancelled or timed-out waiter.
    pub(super) guard_settled: Option<bool>,
    pub(super) guard_recorded: Option<AppResult<()>>,
}

impl Runner {
    pub(super) fn retained_guard(&self) -> Option<Arc<Guard>> {
        self.guard.clone()
    }

    #[cfg(test)]
    pub(super) fn fixture_guard(&self) -> Option<Arc<Guard>> {
        self.retained_guard()
    }

    /// Same closed Studio route, backed by the consumed envelope and owning guard.
    /// This is not a manifest/account entry point or a source of profile authority.
    pub(super) fn consumed(
        studio: Arc<Studio>,
        policy: Arc<Policy>,
        consumed: ConsumedEnvelope,
        attempt: String,
    ) -> AppResult<Self> {
        if attempt.is_empty() {
            return Err(invalid());
        }
        let guard = Guard::new(consumed, policy.clone())?;
        let plan = guard.plan();
        let attachment = QualificationAttachment::new(guard.clone(), TransportLimits::default())?;
        let transport = attachment.transport.clone();
        Ok(Self {
            studio,
            policy,
            guard: Some(guard),
            attachment: Some(attachment),
            transport,
            plan,
            attempt,
            state: RouteState::default(),
            #[cfg(test)]
            controls: FixtureControls::default(),
        })
    }

    /// Not an account entry: only isolated fixtures may assemble execution components.
    #[cfg(test)]
    pub(super) fn fixture(
        studio: Arc<Studio>,
        policy: Arc<Policy>,
        plan: Plan,
        attempt: String,
    ) -> AppResult<Self> {
        let retained = policy.plan();
        if retained.canonical_json()? != plan.canonical_json()?
            || retained.sha256()? != plan.sha256()?
            || attempt.is_empty()
        {
            return Err(AppError::invalid(
                "Qualification runner component identity does not match",
            ));
        }
        let attachment = QualificationAttachment::new(policy.clone(), TransportLimits::default())?;
        let transport = attachment.transport.clone();
        Ok(Self {
            studio,
            policy,
            guard: None,
            attachment: Some(attachment),
            transport,
            plan,
            attempt,
            state: RouteState::default(),
            controls: FixtureControls::default(),
        })
    }

    #[cfg(test)]
    pub(super) fn transport(&self) -> TransportAttachment {
        self.transport.clone()
    }

    #[cfg(test)]
    pub(super) fn set_controls(&mut self, controls: FixtureControls) {
        self.controls = controls;
    }

    fn authentication_limit(&self) -> Duration {
        #[cfg(test)]
        if let Some(limit) = self.controls.authentication_limit {
            return limit.min(AUTHENTICATION_LIMIT);
        }
        AUTHENTICATION_LIMIT
    }

    fn cancel_read(&self, expected: Readback) -> bool {
        #[cfg(test)]
        {
            self.controls.cancel_read && expected != Readback::Ownership
        }
        #[cfg(not(test))]
        {
            let _ = expected;
            false
        }
    }

    /// The task owns all calls, original completion recording and cleanup. Dropping
    /// this JoinHandle detaches the owner; it does not cancel its database work.
    pub(super) fn start(self) -> tokio::task::JoinHandle<RunnerReceipt> {
        tokio::spawn(self.run())
    }

    async fn call(&mut self, operation: Operation) -> Result<Output, CallError> {
        if operation == Operation::StartupIdentityAndVersion {
            // Full filesystem validation is awaited before any helper launch/SSO.
            // No process, Studio or policy mutex is held across this boundary.
            if let Some(guard) = &self.guard {
                guard.revalidate().await?;
            }
            let attachment = self.attachment.take().ok_or_else(invalid)?;
            return match tokio::time::timeout(
                self.authentication_limit(),
                self.studio.connect_prepared_qualification(
                    &self.attempt,
                    ConnectionSecrets::default(),
                    attachment,
                ),
            )
            .await
            {
                Ok(result) => result.map(Output::Startup).map_err(Into::into),
                Err(_) => {
                    let error =
                        AppError::internal("Qualification authentication deadline exceeded");
                    if let Some(guard) = &self.guard {
                        TransportPolicy::failed(guard.as_ref(), error.clone());
                    } else {
                        TransportPolicy::failed(self.policy.as_ref(), error.clone());
                    }
                    Err(error.into())
                }
            };
        }
        let session = self.state.session.as_ref().ok_or_else(invalid)?.id.clone();
        let output = match operation {
            Operation::DdlPreflight(id) => {
                let (catalog, inspection) = match id {
                    StatementId::Create => (None, None),
                    StatementId::Add => {
                        (self.state.catalog.as_ref(), self.state.inspection.as_ref())
                    }
                    StatementId::Drop => (self.state.catalog.as_ref(), None),
                    _ => return Err(invalid().into()),
                };
                let action = self.plan.ddl_action(id, catalog, inspection)?;
                let result = self
                    .studio
                    .preflight_snowflake_ddl(&SnowflakeDdlPreflightRequest {
                        session_id: session,
                        action: action.clone(),
                    })
                    .await?;
                self.state.preflight = Some((action, result.clone()));
                Output::Preflight(result)
            }
            Operation::ReviewedDdl(_) => {
                let (action, response) = self.state.preflight.take().ok_or_else(invalid)?;
                let plan = self.studio.prepare_snowflake_ddl(
                    &session,
                    &SnowflakeDdlRequest {
                        revision: response.revision,
                        action,
                        sql: response.sql,
                    },
                )?;
                if !plan.reviewed_snowflake_ddl || plan.requires_passphrase {
                    return Err(invalid().into());
                }
                let approval = self.studio.authorize_snowflake_ddl(
                    &plan.id,
                    None,
                    plan.confirmation_text.as_deref(),
                    plan.target_confirmation_text.as_deref(),
                )?;
                Output::Ddl(
                    self.studio
                        .execute_snowflake_ddl(&plan.id, Some(&approval))
                        .await?,
                )
            }
            Operation::CoherentReconciliation(_) => {
                let (options, catalog) = self
                    .studio
                    .refresh_snowflake_ddl_metadata_qualification(&session)
                    .await?;
                Output::Reconcile(options, catalog)
            }
            Operation::Catalog(_) => Output::Catalog(self.studio.relation_catalog(&session).await?),
            Operation::Inspect(_) => {
                let catalog = self.state.catalog.as_ref().ok_or_else(invalid)?;
                let relation = catalog
                    .relations
                    .iter()
                    .find(|r| r.table.schema == SCHEMA && r.table.name == self.plan.table())
                    .ok_or_else(invalid)?;
                Output::Inspect(
                    self.studio
                        .inspect_relation(
                            &session,
                            &InspectRelationRequest {
                                revision: catalog.revision.clone(),
                                target_id: relation.target_id.clone(),
                            },
                        )
                        .await?,
                )
            }
            Operation::Read(expected) => Output::Read(
                read(
                    &self.studio,
                    &session,
                    &self.plan,
                    expected,
                    self.cancel_read(expected),
                    self.guard
                        .as_ref()
                        .map(|guard| guard.completion_deadline())
                        .transpose()?,
                )
                .await?,
            ),
            Operation::TypedBoundary { action, .. } => {
                let state = self.state.runtime.as_ref().ok_or_else(invalid)?;
                let Some(TransactionControlAuthority::Observed { revision, .. }) =
                    &state.transaction_control_authority
                else {
                    return Err(invalid().into());
                };
                let plan = self.studio.prepare_transaction_control(
                    &session,
                    &TransactionControlRequest {
                        action,
                        revision: revision.clone(),
                    },
                )?;
                let approval = approve(&self.studio, &plan)?;
                Output::Boundary(
                    self.studio
                        .execute_transaction_control(&plan.id, approval.as_deref())
                        .await?,
                )
            }
            Operation::ReviewedDml(id) => {
                let plan = self
                    .studio
                    .prepare_execution(&session, self.plan.statement(id).source_sql())?;
                if !plan.reviewed_snowflake_dml || !plan.bound_parameters.is_empty() {
                    return Err(invalid().into());
                }
                let approval = approve(&self.studio, &plan)?;
                Output::Dml(
                    self.studio
                        .execute_snowflake_dml(&plan.id, approval.as_deref())
                        .await?,
                )
            }
            Operation::SessionOptionsResume => Output::Options(
                self.studio
                    .session_contexts_with_refresh(&session, true)
                    .await?,
            ),
            Operation::StartupIdentityAndVersion => return Err(invalid().into()),
        };
        Ok(output)
    }

    pub(super) async fn run(mut self) -> RunnerReceipt {
        let mut outputs = Vec::new();
        let mut failure = None;
        let mut original_call_failure = None;
        let mut completion_record_error = None;
        for phase in self.plan.phases().to_vec() {
            if let Some(guard) = &self.guard
                && let Err(error) = guard.check()
            {
                failure = Some((phase.number(), error));
                break;
            }
            match self.call(phase.operation()).await {
                Ok(output) => {
                    #[cfg(test)]
                    {
                        if self
                            .controls
                            .pause
                            .as_ref()
                            .is_some_and(|v| v.phase == phase.number())
                            && let Some(pause) = self.controls.pause.take()
                        {
                            let _ = pause.reached.send(());
                            pause.resume.notified().await;
                        }
                        if let Some((number, path)) = &self.controls.break_journal
                            && *number == phase.number()
                        {
                            use std::os::unix::fs::PermissionsExt;
                            if std::fs::set_permissions(
                                path,
                                std::fs::Permissions::from_mode(0o644),
                            )
                            .is_err()
                            {
                                let error = AppError::internal(
                                    "Qualification fixture journal fault could not be installed",
                                );
                                TransportPolicy::failed(&*self.policy, error.clone());
                                failure = Some((phase.number(), error));
                            }
                        }
                    }
                    let recorded = self
                        .policy
                        .record_operation_completion(OperationCompletion::Returned(
                            output.evidence(),
                        ))
                        .await;
                    let result = match recorded {
                        Ok(None) if failure.is_none() => {
                            match self.guard.as_ref().map(|guard| guard.check()).transpose() {
                                Ok(_) => self.policy.finish_phase(output.evidence()).await,
                                Err(error) => Err(error),
                            }
                        }
                        Ok(None) => Err(AppError::internal(
                            "Qualification fixture failed before phase completion",
                        )),
                        Ok(Some(error)) | Err(error) => {
                            completion_record_error = Some(error.clone());
                            Err(error)
                        }
                    };
                    if result.is_ok() {
                        self.state.accept(&output);
                        if let Output::Read(read) = &output {
                            self.studio.release_result(&read.result.id);
                        }
                    }
                    outputs.push(output);
                    if let Err(error) = result {
                        if failure.is_none() {
                            failure = Some((phase.number(), error));
                        }
                        break;
                    }
                }
                Err(mut original) => {
                    // An incomplete drain never releases the owner: local shutdown is
                    // joined before taking the final bounded original event prefix.
                    if original.pending.is_some() {
                        if let Err(error) = self.studio.shutdown_and_wait().await
                            && original.cleanup_error.is_none()
                        {
                            original.cleanup_error = Some(Box::new(error));
                        }
                        original.after_shutdown().await;
                    }
                    let recorded = self
                        .policy
                        .record_operation_completion(original.completion())
                        .await;
                    completion_record_error = match recorded {
                        Ok(None) => None,
                        Ok(Some(error)) | Err(error) => Some(error),
                    };
                    failure = Some((phase.number(), original.error.clone()));
                    original_call_failure = Some(original);
                    break;
                }
            }
        }
        if let Some((_, error)) = &failure {
            TransportPolicy::failed(&*self.policy, error.clone());
        }
        // Dispose an attachment rejected by the guard before Studio could claim
        // its Launch; otherwise an unstarted transport would never settle here.
        drop(self.attachment.take());
        let shutdown = self.studio.shutdown_and_wait().await;
        let settled = self.transport.wait(Duration::from_secs(10)).await.is_some();
        let (guard_revalidation, guard_settled) = match &self.guard {
            Some(guard) => (
                Some(guard.revalidate().await),
                Some(guard.wait_for_settlement().await),
            ),
            None => (None, None),
        };
        let guard_recorded = match (&guard_revalidation, guard_settled) {
            (Some(result), Some(settled)) => {
                Some(self.policy.record_final_guard(result, settled).await)
            }
            _ => None,
        };
        let persisted = self.policy.persist_final_evidence().await;
        #[cfg(test)]
        if let Some(finished) = self.controls.finished.take() {
            let _ = finished.send(());
        }
        RunnerReceipt {
            outputs,
            failure,
            original_call_failure,
            completion_record_error,
            shutdown,
            settled,
            persisted,
            guard_revalidation,
            guard_settled,
            guard_recorded,
        }
    }
}
