//! TaskTracker-owned Studio execution for native Snowflake generated edits.
//! Retains the physical manual owner, compact originals and rollback authority
//! across detached callers, storage faults, session removal and app shutdown.

use super::batch_outcome_recording::{
    BatchOutcomeCell, MintedGridAuthority, ProtectedRollbackSource,
};
use super::outcome_memory::MemoryLease;
use super::*;
use crate::{
    drivers::{
        snowflake_batch::{BatchRequest, FinishAction, Mode, Outcome},
        snowflake_edit_snapshot::{PreparedSnowflakeEdits, SnapshotMutation},
        snowflake_external::{
            SnowflakeExternal,
            grid::{
                GridAdmission, GridManualTransaction, GridNoDispatch, GridOriginalRecorder,
                PreparedGridOperation,
            },
        },
        snowflake_mutations::{CellValue, MAX_ROWS},
        snowflake_process::{HelperRetirementObserver, Operation, Report},
    },
    persistence::batch_outcomes::{
        BatchDescriptor, BatchPlan, BatchReservation, ManualTransactionLease,
        PendingBatchRollbackAuthority, RejectedBatchRollbackAuthority,
    },
};
use futures_util::FutureExt;
use serde::Serialize;
use std::sync::atomic::AtomicUsize;

#[derive(Clone, Serialize)]
pub(super) enum SnowflakeGridPlanTarget {
    Initial {
        mode: Mode,
    },
    Continuation {
        transaction_id: String,
        rollback_prior: bool,
    },
}

#[cfg(all(test, unix, feature = "test-harness"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GridTestFault {
    RefuseAdmissionBeforeDispatch,
    PanicBeforeDispatch,
    PanicInAdmissionAfterQueue,
    PanicAfterRecordOriginal,
}

#[derive(Default)]
pub(super) struct State {
    owners: Mutex<HashMap<String, Arc<ManualOwner>>>,
    active_runs: AtomicUsize,
    #[cfg(all(test, unix, feature = "test-harness"))]
    next_fault: Mutex<Option<GridTestFault>>,
}

impl State {
    pub(super) fn is_empty(&self) -> bool {
        self.active_runs.load(Ordering::Acquire) == 0 && self.owners.lock().is_empty()
    }

    pub(super) fn active_run_count(&self) -> usize {
        self.active_runs.load(Ordering::Acquire)
    }

    pub(super) fn close_all_helpers(&self) {
        let drivers = self
            .owners
            .lock()
            .values()
            .map(|owner| owner.driver.clone())
            .collect::<Vec<_>>();
        for driver in drivers {
            Driver::close(driver.as_ref());
        }
    }

    pub(super) fn close_profile_helpers(&self, profile_id: &str) {
        let drivers = self
            .owners
            .lock()
            .values()
            .filter(|owner| owner.connection_id == profile_id)
            .map(|owner| owner.driver.clone())
            .collect::<Vec<_>>();
        for driver in drivers {
            Driver::close(driver.as_ref());
        }
    }

    pub(super) fn close_session_helpers(&self, session_id: &str) {
        let drivers = self
            .owners
            .lock()
            .values()
            .filter(|owner| owner.session_id == session_id)
            .map(|owner| owner.driver.clone())
            .collect::<Vec<_>>();
        for driver in drivers {
            Driver::close(driver.as_ref());
        }
    }

    pub(super) fn require_ordinary_session(&self, session_id: &str) -> AppResult<()> {
        if self
            .owners
            .lock()
            .values()
            .any(|owner| owner.session_id == session_id)
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This Snowflake session has owned generated edits; finish or disconnect them before ordinary SQL or metadata",
            ));
        }
        Ok(())
    }

    #[cfg(all(test, unix, feature = "test-harness"))]
    pub(super) fn inject_fault(&self, fault: GridTestFault) {
        *self.next_fault.lock() = Some(fault);
    }

    #[cfg(all(test, unix, feature = "test-harness"))]
    pub(super) fn test_native_owner(
        &self,
        transaction_id: &str,
    ) -> Option<Arc<GridManualTransaction>> {
        self.owners
            .lock()
            .get(transaction_id)
            .map(|owner| owner.native.clone())
    }

    #[cfg(all(test, unix, feature = "test-harness"))]
    pub(super) fn test_has_successor_recovery(&self, transaction_id: &str) -> bool {
        self.owners
            .lock()
            .get(transaction_id)
            .is_some_and(|owner| owner.state.lock().successor_recovery.is_some())
    }

    #[cfg(all(test, unix, feature = "test-harness"))]
    fn take_fault_if(&self, expected: GridTestFault) -> bool {
        let mut slot = self.next_fault.lock();
        if *slot == Some(expected) {
            *slot = None;
            true
        } else {
            false
        }
    }
}

pub(crate) struct SnowflakeGridExecution {
    pub operation_id: String,
    pub transaction_id: Option<String>,
    pub outcome: Outcome,
    #[cfg_attr(not(all(test, unix, feature = "test-harness")), allow(dead_code))]
    pub report: Report,
    pub recording_error: Option<AppError>,
}

struct ManualOwner {
    group: ManualTransactionLease,
    connection_id: String,
    session_id: String,
    driver: Arc<SnowflakeExternal>,
    observer: HelperRetirementObserver,
    native: Arc<GridManualTransaction>,
    lane: tokio::sync::Mutex<()>,
    state: Mutex<ManualOwnerState>,
}

#[derive(Default)]
struct ManualOwnerState {
    pending_authority: Option<(PendingBatchRollbackAuthority, Arc<BatchOutcomeCell>)>,
    successor_recovery: Option<SuccessorRollbackRecovery>,
    blocked_cell: Option<Arc<BatchOutcomeCell>>,
    finished_or_unknown: bool,
}

struct NoDispatchRecovery {
    successor: BatchReservation,
    request: BatchRequest,
    proof: GridNoDispatch,
}

struct RejectedAdmissionRecovery {
    authority: RejectedBatchRollbackAuthority,
    cell: Arc<BatchOutcomeCell>,
}

enum SuccessorRollbackRecovery {
    NoDispatch(Box<NoDispatchRecovery>),
    RejectedAdmission(Box<RejectedAdmissionRecovery>),
}

struct RunGuard {
    studio: Arc<Studio>,
    _recovery_activity: recovery_activity::OwnedRecoveryActivity,
    memory: Option<MemoryLease>,
    admitted_cell: Option<Arc<BatchOutcomeCell>>,
    retire_driver_on_abort: Option<Arc<SnowflakeExternal>>,
    manual_owner_on_abort: Option<Arc<ManualOwner>>,
    completed: bool,
}

impl RunGuard {
    fn new(
        studio: Arc<Studio>,
        recovery_activity: recovery_activity::OwnedRecoveryActivity,
        memory: MemoryLease,
    ) -> Self {
        studio
            .snowflake_grid
            .active_runs
            .fetch_add(1, Ordering::AcqRel);
        Self {
            studio,
            _recovery_activity: recovery_activity,
            memory: Some(memory),
            admitted_cell: None,
            retire_driver_on_abort: None,
            manual_owner_on_abort: None,
            completed: false,
        }
    }

    fn take_memory(&mut self) -> AppResult<MemoryLease> {
        self.memory.take().ok_or_else(conflict)
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if !self.completed {
            if let Some(owner) = self.manual_owner_on_abort.take() {
                let mut st = owner.state.lock();
                st.pending_authority = None;
                st.successor_recovery = None;
                st.blocked_cell = None;
                st.finished_or_unknown = true;
            }
            if let Some(driver) = self.retire_driver_on_abort.take() {
                Driver::close(driver.as_ref());
            }
            if let Some(cell) = self.admitted_cell.take() {
                if !cell.was_dispatched() {
                    let _ = self.studio.abandon_batch_before_dispatch(&cell);
                } else {
                    let _ = self.studio.flush_and_forget_batch_cell(&cell);
                }
            }
        }
        self.studio
            .snowflake_grid
            .active_runs
            .fetch_sub(1, Ordering::AcqRel);
    }
}

struct StudioGridAdmission<'a> {
    studio: &'a Studio,
    cell: &'a Arc<BatchOutcomeCell>,
}

impl GridAdmission for StudioGridAdmission<'_> {
    fn admit(
        &mut self,
        dispatch: &mut dyn FnMut() -> AppResult<Operation>,
    ) -> AppResult<Operation> {
        #[cfg(all(test, unix, feature = "test-harness"))]
        if self
            .studio
            .snowflake_grid
            .take_fault_if(GridTestFault::RefuseAdmissionBeforeDispatch)
        {
            return Err(conflict());
        }
        let operation = self.studio.dispatch_batch_operation(self.cell, dispatch)?;
        #[cfg(all(test, unix, feature = "test-harness"))]
        assert!(
            !self
                .studio
                .snowflake_grid
                .take_fault_if(GridTestFault::PanicInAdmissionAfterQueue),
            "synthetic grid panic after queue admission"
        );
        Ok(operation)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecorderRole {
    Automatic,
    InitialManual,
    ContinueOrCommit,
    Rollback,
}

struct StudioGridRecorder<'a> {
    studio: &'a Studio,
    role: RecorderRole,
    driver: &'a Arc<SnowflakeExternal>,
    cell: &'a Arc<BatchOutcomeCell>,
    manual: Option<&'a Arc<ManualOwner>>,
    prepared_request: BatchRequest,
    successor_reservation: Option<BatchReservation>,
    prior_pending: Option<PendingBatchRollbackAuthority>,
    predecessor_cells: Vec<Arc<BatchOutcomeCell>>,
}

impl GridOriginalRecorder for StudioGridRecorder<'_> {
    fn record_original(&mut self, report: &Report, operation: &Operation) -> AppResult<()> {
        let summary_outcome = report
            .batch
            .as_ref()
            .and_then(|evidence| evidence.summary.as_ref())
            .map(|summary| summary.outcome);
        let role = self.role;
        let cell = self.cell;
        let manual = self.manual;
        let result = self.studio.record_grid_batch_outcome(
            cell,
            report,
            operation,
            self.prior_pending.as_ref(),
            |minted| {
                let Some(owner) = manual else {
                    return;
                };
                let mut st = owner.state.lock();
                match minted {
                    Some(MintedGridAuthority::Pending(authority)) => {
                        st.pending_authority = Some((*authority, cell.clone()));
                        st.successor_recovery = None;
                        st.blocked_cell = Some(cell.clone());
                    }
                    Some(MintedGridAuthority::Rejected(rejected)) => {
                        st.successor_recovery = Some(SuccessorRollbackRecovery::RejectedAdmission(
                            Box::new(RejectedAdmissionRecovery {
                                authority: *rejected,
                                cell: cell.clone(),
                            }),
                        ));
                        st.blocked_cell = Some(cell.clone());
                    }
                    None => {
                        st.pending_authority = None;
                        st.successor_recovery = None;
                        st.finished_or_unknown = true;
                        let releases_session_when_settled = matches!(
                            summary_outcome,
                            Some(Outcome::Committed | Outcome::RolledBack)
                        ) || (role
                            == RecorderRole::InitialManual
                            && summary_outcome == Some(Outcome::Rejected));
                        st.blocked_cell = if releases_session_when_settled {
                            Some(cell.clone())
                        } else {
                            None
                        };
                    }
                }
            },
        );
        #[cfg(all(test, unix, feature = "test-harness"))]
        assert!(
            !self
                .studio
                .snowflake_grid
                .take_fault_if(GridTestFault::PanicAfterRecordOriginal),
            "synthetic grid panic after original capture"
        );
        if result.is_ok() {
            if let Some(owner) = self.manual {
                let mut st = owner.state.lock();
                st.blocked_cell = None;
                if matches!(
                    st.successor_recovery,
                    Some(SuccessorRollbackRecovery::RejectedAdmission(_))
                ) {
                    st.successor_recovery = None;
                }
            }
            for pred in &self.predecessor_cells {
                let _ = self.studio.flush_and_forget_batch_cell(pred);
            }
        }
        result
    }

    fn record_no_dispatch(&mut self, proof: &GridNoDispatch) -> AppResult<()> {
        match self.role {
            RecorderRole::Automatic => {
                if !self.cell.was_dispatched() {
                    let _ = self.studio.abandon_batch_before_dispatch(self.cell);
                }
            }
            RecorderRole::InitialManual => {
                if let Some(owner) = self.manual {
                    let mut st = owner.state.lock();
                    st.pending_authority = None;
                    st.successor_recovery = None;
                    st.blocked_cell = None;
                    st.finished_or_unknown = true;
                }
                // Initial manual admission already reserved and pinned this
                // group in SQLite; retire this exact helper so post-reap
                // retirement can release its binding truthfully.
                Driver::close(self.driver.as_ref());
                if !self.cell.was_dispatched() {
                    let _ = self.studio.abandon_batch_before_dispatch(self.cell);
                }
            }
            RecorderRole::ContinueOrCommit => {
                if let (Some(owner), Some(successor)) =
                    (self.manual, self.successor_reservation.clone())
                {
                    let mut st = owner.state.lock();
                    st.successor_recovery = Some(SuccessorRollbackRecovery::NoDispatch(Box::new(
                        NoDispatchRecovery {
                            successor,
                            request: self.prepared_request.clone(),
                            proof: proof.clone(),
                        },
                    )));
                    st.blocked_cell = None;
                }
                if !self.cell.was_dispatched() {
                    let _ = self.studio.abandon_batch_before_dispatch(self.cell);
                }
            }
            RecorderRole::Rollback => {
                if let Some(owner) = self.manual {
                    let mut st = owner.state.lock();
                    st.pending_authority = None;
                    st.successor_recovery = None;
                    st.blocked_cell = None;
                    st.finished_or_unknown = true;
                }
                Driver::close(self.driver.as_ref());
                if !self.cell.was_dispatched() {
                    let _ = self.studio.abandon_batch_before_dispatch(self.cell);
                }
            }
        }
        Ok(())
    }
}

fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake generated edit ownership or local recording state changed; no new SQL was admitted",
    )
}

fn busy() -> AppError {
    AppError::new(ErrorCode::Conflict, "The database session is busy")
}

fn disconnected() -> AppError {
    AppError::new(
        ErrorCode::ConnectionClosed,
        "This database session is disconnected",
    )
}

fn task_panicked() -> AppError {
    AppError::new(
        ErrorCode::OutcomeUnknown,
        "The Snowflake edit task stopped unexpectedly; inspect recorded batch outcomes before retrying",
    )
}

fn writable_grid_profile(profile: &ConnectionProfile) -> AppResult<()> {
    if !snowflake_dml::sdk_profile(profile)
        || profile
            .snowflake
            .as_ref()
            .is_none_or(|config| !config.reviewed_dml || config.reviewed_console)
        || safety::effective_mode(profile) == SafeMode::ReadOnly
    {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "Snowflake generated edits require an explicitly enabled writable external-browser profile outside the retained SQL console",
        ));
    }
    Ok(())
}

fn external_driver(session: &Session) -> AppResult<Arc<SnowflakeExternal>> {
    session.snowflake_external.clone().ok_or_else(|| {
        AppError::new(
            ErrorCode::Unsupported,
            "Snowflake generated edits are unavailable for this session",
        )
    })
}

fn report_outcome(report: &Report) -> Outcome {
    report
        .batch
        .as_ref()
        .and_then(|evidence| evidence.summary.as_ref())
        .map_or(Outcome::Unknown, |summary| summary.outcome)
}

fn convert_grid_cell(cell: &SnowflakeGridCellInput) -> CellValue {
    match cell {
        SnowflakeGridCellInput::Default => CellValue::Default,
        SnowflakeGridCellInput::Null => CellValue::Null,
        SnowflakeGridCellInput::Text { value } => CellValue::Text(value.clone()),
    }
}

fn convert_grid_mutations(mutations: &[SnowflakeGridMutation]) -> AppResult<Vec<SnapshotMutation>> {
    if mutations.is_empty() {
        return Err(AppError::invalid(
            "Stage at least one row change before reviewing",
        ));
    }
    if mutations.len() > MAX_ROWS {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            "Generated Snowflake edits allow at most 500 row mutations per batch",
        ));
    }
    Ok(mutations
        .iter()
        .map(|mutation| match mutation {
            SnowflakeGridMutation::Insert { values } => SnapshotMutation::Insert {
                values: values
                    .iter()
                    .map(|item| (item.column_index, convert_grid_cell(&item.value)))
                    .collect(),
            },
            SnowflakeGridMutation::Update { row_index, changes } => SnapshotMutation::Update {
                row_index: *row_index,
                changes: changes
                    .iter()
                    .map(|item| (item.column_index, convert_grid_cell(&item.value)))
                    .collect(),
            },
            SnowflakeGridMutation::Delete { row_index } => SnapshotMutation::Delete {
                row_index: *row_index,
            },
        })
        .collect())
}

fn map_grid_outcome(outcome: Outcome) -> SnowflakeGridBatchOutcome {
    match outcome {
        Outcome::Committed => SnowflakeGridBatchOutcome::Committed,
        Outcome::Pending => SnowflakeGridBatchOutcome::Pending,
        Outcome::RolledBack => SnowflakeGridBatchOutcome::RolledBack,
        Outcome::Rejected => SnowflakeGridBatchOutcome::Rejected,
        Outcome::Unknown => SnowflakeGridBatchOutcome::Unknown,
    }
}

impl Studio {
    fn to_grid_result(
        &self,
        session_id: String,
        exec: SnowflakeGridExecution,
    ) -> SnowflakeGridResult {
        let retired = self
            .sessions
            .lock()
            .get(&session_id)
            .is_none_or(|s| s.closed.load(Ordering::Acquire) || s.driver.is_closed());
        SnowflakeGridResult {
            session_id,
            operation_id: exec.operation_id,
            outcome: map_grid_outcome(exec.outcome),
            transaction_id: exec.transaction_id,
            recording_error: exec.recording_error.map(|error| error.message),
            metadata_refresh_required: true,
            retired,
        }
    }

    pub async fn prepare_snowflake_grid_plan(
        &self,
        request: &PrepareSnowflakeGridEditsRequest,
    ) -> AppResult<ChangePreview> {
        let mutations = convert_grid_mutations(&request.mutations)?;
        let (target, edits) = if let Some(tx_id) = &request.transaction_id {
            if request.mode != SnowflakeGridMode::Manual {
                return Err(AppError::invalid(
                    "Continuing an open Snowflake grid transaction requires manual mode",
                ));
            }
            if !request.rollback_prior {
                return Err(AppError::invalid(
                    "Continuing generated edits requires explicit whole-transaction rollback permission",
                ));
            }
            let owner_session = self
                .snowflake_grid
                .owners
                .lock()
                .get(tx_id)
                .map(|owner| owner.session_id.clone())
                .ok_or_else(conflict)?;
            if owner_session != request.session_id {
                return Err(conflict());
            }
            let edits = self
                .prepare_snowflake_grid_continuation_edits(
                    tx_id,
                    &request.preview,
                    &request.snapshot_id,
                    &mutations,
                )
                .await?;
            (
                SnowflakeGridPlanTarget::Continuation {
                    transaction_id: tx_id.clone(),
                    rollback_prior: true,
                },
                edits,
            )
        } else {
            let mode = match request.mode {
                SnowflakeGridMode::Automatic => Mode::Automatic,
                SnowflakeGridMode::Manual => Mode::Manual,
            };
            let edits = self
                .prepare_snowflake_grid_edits(
                    &request.session_id,
                    &request.preview,
                    &request.snapshot_id,
                    &mutations,
                )
                .await?;
            (SnowflakeGridPlanTarget::Initial { mode }, edits)
        };
        let sql: Vec<String> = edits
            .compiled()
            .units()
            .iter()
            .map(|unit| unit.sql().to_owned())
            .collect();
        let bindings: Vec<Vec<Option<String>>> = edits
            .compiled()
            .units()
            .iter()
            .map(|unit| {
                unit.bindings()
                    .iter()
                    .map(|binding| binding.value().map(str::to_owned))
                    .collect()
            })
            .collect();
        let statements = sql
            .iter()
            .map(|statement_sql| StatementPlan {
                sql: statement_sql.clone(),
                kind: StatementKind::Write,
                missing_where: false,
                span: None,
            })
            .collect::<Vec<_>>();
        let plan = {
            let _configuration = self.configuration_mutation_guard()?;
            let session = self.session(&request.session_id)?;
            if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
                return Err(disconnected());
            }
            let profile = self.persistence.profile(&session.profile.id)?;
            writable_grid_profile(&profile)?;
            let mut plan = safety::plan_for_statements(&request.session_id, &profile, statements)?;
            plan.reviewed_snowflake_grid = true;
            plan.requires_approval = true;
            let digest = hex::encode(Sha256::digest(
                serde_json::to_vec(&(&target, edits.snapshot_id(), &sql, &bindings))
                    .map_err(|_| AppError::internal("Could not fingerprint Snowflake grid plan"))?,
            ));
            plan.binding_digest = Some(digest.clone());
            plan.warnings.push(
                "Generated edits target the fully qualified SQL name shown above using exact retained original-row predicates. Concurrent schema changes or duplicate keyless rows are checked by per-statement cardinality; any mismatch rolls back the transaction."
                    .into(),
            );
            match &target {
                SnowflakeGridPlanTarget::Initial {
                    mode: Mode::Automatic,
                } => plan.warnings.push(
                    "This batch runs inside BEGIN TRANSACTION ... COMMIT on this Snowflake session and may resume a billable warehouse."
                        .into(),
                ),
                SnowflakeGridPlanTarget::Initial { mode: Mode::Manual } => plan.warnings.push(
                    "This batch starts an explicit manual transaction on this Snowflake session and leaves it PENDING until you explicitly Commit or Rollback."
                        .into(),
                ),
                SnowflakeGridPlanTarget::Continuation { transaction_id, .. } => plan.warnings.push(
                    format!(
                        "This batch continues open manual transaction {transaction_id}; any statement failure or cardinality mismatch rolls back the entire transaction."
                    ),
                ),
            }
            let bytes = edits
                .compiled()
                .accounted_bytes()
                .saturating_add(sql.iter().map(String::len).sum::<usize>());
            let effective_mode = safety::effective_mode(&profile);
            self.store_plan(
                plan,
                Action::SnowflakeGrid {
                    target,
                    edits,
                    digest,
                    mode: effective_mode,
                },
                bytes,
            )?
        };
        Ok(ChangePreview {
            plan,
            row_count: mutations.len(),
            sql,
            bindings,
        })
    }

    pub async fn execute_snowflake_grid_plan(
        self: &Arc<Self>,
        plan_id: &str,
        approval: Option<&str>,
    ) -> AppResult<SnowflakeGridResult> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let (session_id, target, edits) = {
            let _configuration = self.configuration_mutation_guard()?;
            if self.dml_tasks.is_closed() {
                return Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "Joinery is closing; no database write was started",
                ));
            }
            let stored = self.plans.lock().remove(plan_id).ok_or_else(expired)?;
            if stored.deadline <= Instant::now() {
                return Err(expired());
            }
            let Action::SnowflakeGrid {
                target,
                edits,
                digest,
                mode,
            } = stored.action
            else {
                return Err(AppError::invalid(
                    "This is not a reviewed Snowflake grid edit plan",
                ));
            };
            if stored.plan.binding_digest.as_deref() != Some(digest.as_str()) {
                return Err(expired());
            }
            let session = self.session(&stored.plan.session_id)?;
            if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
                return Err(disconnected());
            }
            let profile = self.persistence.profile(&session.profile.id)?;
            writable_grid_profile(&profile)?;
            if mode != safety::effective_mode(&profile)
                || stored.plan.requires_passphrase != (mode == SafeMode::PasswordGated)
            {
                return Err(expired());
            }
            if stored.approval.is_none() || stored.approval.as_deref() != approval {
                return Err(AppError::new(
                    ErrorCode::ApprovalRequired,
                    "Review and authorize these generated Snowflake edits before applying them",
                ));
            }
            (stored.plan.session_id, target, edits)
        };
        let exec = match target {
            SnowflakeGridPlanTarget::Initial { mode } => {
                self.execute_snowflake_grid_edits(&session_id, edits, mode)
                    .await?
            }
            SnowflakeGridPlanTarget::Continuation {
                transaction_id,
                rollback_prior,
            } => {
                self.continue_snowflake_grid_edits(&transaction_id, edits, rollback_prior)
                    .await?
            }
        };
        Ok(self.to_grid_result(session_id, exec))
    }

    pub async fn execute_snowflake_grid_finish(
        self: &Arc<Self>,
        transaction_id: &str,
        action: SnowflakeGridFinishAction,
    ) -> AppResult<SnowflakeGridResult> {
        let session_id = self
            .snowflake_grid
            .owners
            .lock()
            .get(transaction_id)
            .map(|owner| owner.session_id.clone())
            .ok_or_else(conflict)?;
        let finish_action = match action {
            SnowflakeGridFinishAction::Commit => FinishAction::Commit,
            SnowflakeGridFinishAction::Rollback => FinishAction::Rollback,
        };
        let exec = self
            .finish_snowflake_grid_transaction(transaction_id, finish_action)
            .await?;
        Ok(self.to_grid_result(session_id, exec))
    }

    pub(crate) async fn prepare_snowflake_grid_edits(
        &self,
        session_id: &str,
        preview: &PreviewRelationRequest,
        snapshot_id: &str,
        mutations: &[SnapshotMutation],
    ) -> AppResult<PreparedSnowflakeEdits> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let session = self.session(session_id)?;
        let driver = external_driver(&session)?;
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| busy())?;
        self.snowflake_grid.require_ordinary_session(session_id)?;
        {
            let _configuration = self.configuration_mutation_guard()?;
            writable_grid_profile(&self.persistence.profile(&session.profile.id)?)?;
            if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
                return Err(disconnected());
            }
            if session.active_run.lock().is_some() {
                return Err(busy());
            }
            session.metadata_gate.lock().check(None, false)?;
        }
        driver
            .prepare_relation_edits(preview, snapshot_id, mutations)
            .await
    }

    pub async fn preview_snowflake_grid_transaction_relation(
        &self,
        transaction_id: &str,
        request: &PreviewRelationRequest,
    ) -> AppResult<RelationPreview> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let owner = self
            .snowflake_grid
            .owners
            .lock()
            .get(transaction_id)
            .cloned()
            .ok_or_else(conflict)?;
        let session = self.session(&owner.session_id)?;
        let _session_lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| busy())?;
        let _owner_lane = tokio::time::timeout(Duration::from_secs(5), owner.lane.lock())
            .await
            .map_err(|_| busy())?;
        self.validate_continuable_grid_owner(&owner, &session)?;
        owner.native.preview_relation(request).await
    }

    pub(crate) async fn prepare_snowflake_grid_continuation_edits(
        &self,
        transaction_id: &str,
        preview: &PreviewRelationRequest,
        snapshot_id: &str,
        mutations: &[SnapshotMutation],
    ) -> AppResult<PreparedSnowflakeEdits> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let owner = self
            .snowflake_grid
            .owners
            .lock()
            .get(transaction_id)
            .cloned()
            .ok_or_else(conflict)?;
        let session = self.session(&owner.session_id)?;
        let _session_lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| busy())?;
        let _owner_lane = tokio::time::timeout(Duration::from_secs(5), owner.lane.lock())
            .await
            .map_err(|_| busy())?;
        self.validate_continuable_grid_owner(&owner, &session)?;
        owner
            .native
            .prepare_continuation_edits(preview, snapshot_id, mutations)
            .await
    }

    fn validate_continuable_grid_owner(
        &self,
        owner: &Arc<ManualOwner>,
        session: &Session,
    ) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        self.flush_pending_history()?;
        self.flush_pending_snowflake_receipts()?;
        self.flush_console_recordings()?;
        self.flush_snowflake_grid_outcomes()?;
        writable_grid_profile(&self.persistence.profile(&owner.connection_id)?)?;
        if session.closed.load(Ordering::Acquire)
            || session.driver.is_closed()
            || owner.driver.is_closed()
        {
            return Err(disconnected());
        }
        if session.active_run.lock().is_some() {
            return Err(busy());
        }
        if !self
            .snowflake_grid
            .owners
            .lock()
            .get(owner.group.id())
            .is_some_and(|current| Arc::ptr_eq(current, owner))
        {
            return Err(conflict());
        }
        let st = owner.state.lock();
        if st.finished_or_unknown
            || st.blocked_cell.is_some()
            || st.successor_recovery.is_some()
            || st.pending_authority.is_none()
        {
            return Err(conflict());
        }
        Ok(())
    }

    pub(crate) fn start_snowflake_grid_edits(
        self: &Arc<Self>,
        session_id: &str,
        edits: PreparedSnowflakeEdits,
        mode: Mode,
    ) -> AppResult<tokio::task::JoinHandle<AppResult<SnowflakeGridExecution>>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let memory = self.outcome_memory.ordinary()?;
        let session_id = session_id.to_owned();
        let studio = self.clone();
        let _configuration = self.configuration_mutation_guard()?;
        if self.dml_tasks.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "Joinery is closing; no database write was started",
            ));
        }
        let recovery_activity = self.retain_recovery_activity()?;
        let mut guard = RunGuard::new(studio.clone(), recovery_activity, memory);
        Ok(self.dml_tasks.spawn(async move {
            let fallback_studio = studio.clone();
            let fallback_session_id = session_id.clone();
            match std::panic::AssertUnwindSafe(studio.run_initial_grid_edits(
                &session_id,
                edits,
                mode,
                &mut guard,
            ))
            .catch_unwind()
            .await
            {
                Ok(result) => {
                    fallback_studio.reconcile_snowflake_grid_owners();
                    result
                }
                Err(_) => {
                    if let Some(session) = fallback_studio
                        .sessions
                        .lock()
                        .get(&fallback_session_id)
                        .cloned()
                    {
                        session.closed.store(true, Ordering::Release);
                        session.driver.close();
                    }
                    fallback_studio.reconcile_snowflake_grid_owners();
                    Err(task_panicked())
                }
            }
        }))
    }

    pub(crate) async fn execute_snowflake_grid_edits(
        self: &Arc<Self>,
        session_id: &str,
        edits: PreparedSnowflakeEdits,
        mode: Mode,
    ) -> AppResult<SnowflakeGridExecution> {
        self.start_snowflake_grid_edits(session_id, edits, mode)?
            .await
            .map_err(|_| task_panicked())?
    }

    pub(crate) fn start_continue_snowflake_grid_edits(
        self: &Arc<Self>,
        transaction_id: &str,
        edits: PreparedSnowflakeEdits,
        rollback_prior: bool,
    ) -> AppResult<tokio::task::JoinHandle<AppResult<SnowflakeGridExecution>>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let memory = self.outcome_memory.ordinary()?;
        let owner = self
            .snowflake_grid
            .owners
            .lock()
            .get(transaction_id)
            .cloned()
            .ok_or_else(conflict)?;
        let studio = self.clone();
        let _configuration = self.configuration_mutation_guard()?;
        if self.dml_tasks.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "Joinery is closing; no database write was started",
            ));
        }
        let recovery_activity = self.retain_recovery_activity()?;
        let mut guard = RunGuard::new(studio.clone(), recovery_activity, memory);
        Ok(self.dml_tasks.spawn(async move {
            let fallback_studio = studio.clone();
            let fallback_owner = owner.clone();
            match std::panic::AssertUnwindSafe(studio.run_continue_grid_edits(
                owner,
                edits,
                rollback_prior,
                &mut guard,
            ))
            .catch_unwind()
            .await
            {
                Ok(result) => {
                    fallback_studio.reconcile_snowflake_grid_owners();
                    result
                }
                Err(_) => {
                    Driver::close(fallback_owner.driver.as_ref());
                    fallback_studio.reconcile_snowflake_grid_owners();
                    Err(task_panicked())
                }
            }
        }))
    }

    pub(crate) async fn continue_snowflake_grid_edits(
        self: &Arc<Self>,
        transaction_id: &str,
        edits: PreparedSnowflakeEdits,
        rollback_prior: bool,
    ) -> AppResult<SnowflakeGridExecution> {
        self.start_continue_snowflake_grid_edits(transaction_id, edits, rollback_prior)?
            .await
            .map_err(|_| task_panicked())?
    }

    pub(crate) fn start_finish_snowflake_grid_transaction(
        self: &Arc<Self>,
        transaction_id: &str,
        action: FinishAction,
    ) -> AppResult<tokio::task::JoinHandle<AppResult<SnowflakeGridExecution>>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let owner = self
            .snowflake_grid
            .owners
            .lock()
            .get(transaction_id)
            .cloned()
            .ok_or_else(conflict)?;
        let memory = if action == FinishAction::Rollback {
            self.claim_batch_rollback_memory(&owner.group)?
        } else {
            self.outcome_memory.ordinary()?
        };
        let studio = self.clone();
        let _configuration = self.configuration_mutation_guard()?;
        if self.dml_tasks.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "Joinery is closing; no transaction action was started",
            ));
        }
        let recovery_activity = self.retain_recovery_activity()?;
        let mut guard = RunGuard::new(studio.clone(), recovery_activity, memory);
        Ok(self.dml_tasks.spawn(async move {
            let fallback_studio = studio.clone();
            let fallback_owner = owner.clone();
            match std::panic::AssertUnwindSafe(
                studio.run_finish_grid_transaction(owner, action, &mut guard),
            )
            .catch_unwind()
            .await
            {
                Ok(result) => {
                    fallback_studio.reconcile_snowflake_grid_owners();
                    result
                }
                Err(_) => {
                    Driver::close(fallback_owner.driver.as_ref());
                    fallback_studio.reconcile_snowflake_grid_owners();
                    Err(task_panicked())
                }
            }
        }))
    }

    pub(crate) async fn finish_snowflake_grid_transaction(
        self: &Arc<Self>,
        transaction_id: &str,
        action: FinishAction,
    ) -> AppResult<SnowflakeGridExecution> {
        self.start_finish_snowflake_grid_transaction(transaction_id, action)?
            .await
            .map_err(|_| task_panicked())?
    }

    pub(super) fn flush_snowflake_grid_outcomes(&self) -> AppResult<()> {
        let result = self.flush_batch_outcomes();
        self.reconcile_snowflake_grid_owners();
        result
    }

    fn reconcile_snowflake_grid_owners(&self) {
        let snapshot = self
            .snowflake_grid
            .owners
            .lock()
            .iter()
            .map(|(id, owner)| (id.clone(), owner.clone()))
            .collect::<Vec<_>>();
        for (group_id, owner) in snapshot {
            let settled_operation_id = {
                let mut st = owner.state.lock();
                if matches!(
                    &st.successor_recovery,
                    Some(SuccessorRollbackRecovery::RejectedAdmission(rec))
                        if rec.cell.is_settled()
                ) {
                    st.successor_recovery = None;
                }
                if let Some(cell) = &st.blocked_cell {
                    if cell.is_settled() {
                        let id = cell.operation_id().to_owned();
                        st.blocked_cell = None;
                        Some(id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(operation_id) = settled_operation_id {
                let _ = owner.native.recording_settled(&operation_id);
            }
            let removable = {
                let st = owner.state.lock();
                (st.finished_or_unknown || owner.observer.reaped_generation().is_some())
                    && st.blocked_cell.is_none()
                    && !self.has_manual_batch_group(&group_id)
            };
            if removable {
                self.snowflake_grid.owners.lock().remove(&group_id);
            }
        }
    }

    fn hold_session_after_grid_admission(&self, session: &Session, session_id: &str) {
        session.metadata_gate.lock().hold();
        *session.observed_runtime.lock() = None;
        *session.completion.lock() = None;
        self.plans
            .lock()
            .retain(|_, plan| plan.plan.session_id != session_id);
        self.ddl_preflights.lock().remove(session_id);
    }

    async fn run_initial_grid_edits(
        &self,
        session_id: &str,
        edits: PreparedSnowflakeEdits,
        mode: Mode,
        guard: &mut RunGuard,
    ) -> AppResult<SnowflakeGridExecution> {
        let session = self.session(session_id)?;
        let driver = external_driver(&session)?;
        let _session_lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| busy())?;
        let safety_slot = {
            let _configuration = self.configuration_mutation_guard()?;
            self.flush_pending_history()?;
            self.flush_pending_snowflake_receipts()?;
            self.flush_console_recordings()?;
            self.flush_snowflake_grid_outcomes()?;
            writable_grid_profile(&self.persistence.profile(&session.profile.id)?)?;
            if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
                return Err(disconnected());
            }
            if session.active_run.lock().is_some() {
                return Err(busy());
            }
            session.metadata_gate.lock().check(None, false)?;
            if mode == Mode::Manual {
                let slot = self.session_safety_slot(&session)?;
                if !slot.is_available() {
                    return Err(conflict());
                }
                Some(slot)
            } else {
                None
            }
        };
        let prepared = driver.prepare_grid_operation(edits, mode).await?;
        let observer = driver.retirement_observer();
        let (cell, manual_owner) = {
            let _configuration = self.configuration_mutation_guard()?;
            writable_grid_profile(&self.persistence.profile(&session.profile.id)?)?;
            if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
                return Err(disconnected());
            }
            if session.active_run.lock().is_some() {
                return Err(busy());
            }
            session.metadata_gate.lock().check(None, false)?;
            let descriptor = BatchDescriptor {
                operation_id: prepared.operation_id().into(),
                connection_id: session.profile.id.clone(),
                session_id: session_id.into(),
                helper_generation: observer.helper_generation().generation().into(),
                started_at: now_ms(),
                digest: prepared.digest().into(),
            };
            let plan = BatchPlan::new(prepared.request().clone())?;
            let memory = guard.take_memory()?;
            let cell = match self.admit_batch_outcome(
                memory,
                descriptor,
                plan,
                None,
                safety_slot,
                observer.clone(),
            ) {
                Ok(cell) => cell,
                Err(error) => {
                    if mode == Mode::Manual && error.code == ErrorCode::Internal {
                        Driver::close(driver.as_ref());
                    }
                    return Err(error);
                }
            };
            guard.admitted_cell = Some(cell.clone());
            let manual_owner = if mode == Mode::Manual {
                let group = cell.manual_transaction().ok_or_else(conflict)?;
                let native = prepared.manual_owner().ok_or_else(conflict)?;
                let entry = Arc::new(ManualOwner {
                    group: group.clone(),
                    connection_id: session.profile.id.clone(),
                    session_id: session_id.into(),
                    driver: driver.clone(),
                    observer: observer.clone(),
                    native,
                    lane: tokio::sync::Mutex::new(()),
                    state: Mutex::new(ManualOwnerState::default()),
                });
                let mut owners = self.snowflake_grid.owners.lock();
                if owners.contains_key(group.id()) {
                    Driver::close(driver.as_ref());
                    return Err(conflict());
                }
                owners.insert(group.id().into(), entry.clone());
                guard.retire_driver_on_abort = Some(driver.clone());
                guard.manual_owner_on_abort = Some(entry.clone());
                Some(entry)
            } else {
                None
            };
            self.hold_session_after_grid_admission(&session, session_id);
            (cell, manual_owner)
        };
        #[cfg(all(test, unix, feature = "test-harness"))]
        assert!(
            !self
                .snowflake_grid
                .take_fault_if(GridTestFault::PanicBeforeDispatch),
            "synthetic grid panic after durable reservation before dispatch"
        );
        self.execute_prepared_grid_operation(
            prepared,
            if mode == Mode::Manual {
                RecorderRole::InitialManual
            } else {
                RecorderRole::Automatic
            },
            &driver,
            &cell,
            manual_owner.as_ref(),
            None,
            Vec::new(),
            guard,
        )
        .await
    }

    async fn run_continue_grid_edits(
        &self,
        owner: Arc<ManualOwner>,
        edits: PreparedSnowflakeEdits,
        rollback_prior: bool,
        guard: &mut RunGuard,
    ) -> AppResult<SnowflakeGridExecution> {
        let maybe_session = self.sessions.lock().get(&owner.session_id).cloned();
        let _session_lane = if let Some(ref session) = maybe_session {
            Some(
                tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
                    .await
                    .map_err(|_| busy())?,
            )
        } else {
            None
        };
        let _owner_lane = tokio::time::timeout(Duration::from_secs(5), owner.lane.lock())
            .await
            .map_err(|_| busy())?;
        {
            let _configuration = self.configuration_mutation_guard()?;
            self.flush_pending_history()?;
            self.flush_pending_snowflake_receipts()?;
            self.flush_console_recordings()?;
            self.flush_snowflake_grid_outcomes()?;
            writable_grid_profile(&self.persistence.profile(&owner.connection_id)?)?;
            let Some(ref session) = maybe_session else {
                return Err(disconnected());
            };
            if session.closed.load(Ordering::Acquire) || owner.driver.is_closed() {
                return Err(disconnected());
            }
            if session.active_run.lock().is_some() {
                return Err(busy());
            }
        }
        let prepared = owner
            .native
            .prepare_grid_operation(edits, rollback_prior)
            .await?;
        let (cell, prior_pending) = {
            let _configuration = self.configuration_mutation_guard()?;
            writable_grid_profile(&self.persistence.profile(&owner.connection_id)?)?;
            if maybe_session
                .as_ref()
                .is_none_or(|s| s.closed.load(Ordering::Acquire))
                || owner.driver.is_closed()
            {
                return Err(disconnected());
            }
            let descriptor = BatchDescriptor {
                operation_id: prepared.operation_id().into(),
                connection_id: owner.connection_id.clone(),
                session_id: owner.session_id.clone(),
                helper_generation: owner.observer.helper_generation().generation().into(),
                started_at: now_ms(),
                digest: prepared.digest().into(),
            };
            let plan = BatchPlan::new(prepared.request().clone())?;
            let memory = guard.take_memory()?;
            let cell = match self.admit_batch_outcome(
                memory,
                descriptor,
                plan,
                Some(&owner.group),
                None,
                owner.observer.clone(),
            ) {
                Ok(cell) => cell,
                Err(error) => {
                    if error.code == ErrorCode::Internal {
                        owner.state.lock().finished_or_unknown = true;
                        Driver::close(owner.driver.as_ref());
                    }
                    return Err(error);
                }
            };
            guard.admitted_cell = Some(cell.clone());
            guard.retire_driver_on_abort = Some(owner.driver.clone());
            guard.manual_owner_on_abort = Some(owner.clone());
            let prior = owner
                .state
                .lock()
                .pending_authority
                .as_ref()
                .map(|(authority, _)| authority.clone());
            if let Some(ref session) = maybe_session {
                self.hold_session_after_grid_admission(session, &owner.session_id);
            }
            (cell, prior)
        };
        #[cfg(all(test, unix, feature = "test-harness"))]
        assert!(
            !self
                .snowflake_grid
                .take_fault_if(GridTestFault::PanicBeforeDispatch),
            "synthetic grid panic after durable continuation reservation"
        );
        self.execute_prepared_grid_operation(
            prepared,
            RecorderRole::ContinueOrCommit,
            &owner.driver,
            &cell,
            Some(&owner),
            prior_pending,
            Vec::new(),
            guard,
        )
        .await
    }

    async fn run_finish_grid_transaction(
        &self,
        owner: Arc<ManualOwner>,
        action: FinishAction,
        guard: &mut RunGuard,
    ) -> AppResult<SnowflakeGridExecution> {
        let maybe_session = self.sessions.lock().get(&owner.session_id).cloned();
        let _session_lane = if let Some(ref session) = maybe_session {
            Some(
                tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
                    .await
                    .map_err(|_| busy())?,
            )
        } else {
            None
        };
        let _owner_lane = tokio::time::timeout(Duration::from_secs(5), owner.lane.lock())
            .await
            .map_err(|_| busy())?;
        if let Some(ref session) = maybe_session
            && session.active_run.lock().is_some()
        {
            return Err(busy());
        }
        {
            let _configuration = self.configuration_mutation_guard()?;
            if action != FinishAction::Rollback {
                self.flush_pending_history()?;
                self.flush_pending_snowflake_receipts()?;
                self.flush_console_recordings()?;
                self.flush_snowflake_grid_outcomes()?;
                writable_grid_profile(&self.persistence.profile(&owner.connection_id)?)?;
                if maybe_session
                    .as_ref()
                    .is_none_or(|s| s.closed.load(Ordering::Acquire))
                    || owner.driver.is_closed()
                {
                    return Err(disconnected());
                }
            } else {
                let _ = self.flush_snowflake_grid_outcomes();
                if owner.driver.is_closed() {
                    return Err(disconnected());
                }
            }
        }
        let prepared = owner.native.prepare_finish(action).await?;
        let BatchRequest::Finish {
            owner: helper_owner,
            action: prepared_action,
        } = *prepared.request()
        else {
            return Err(conflict());
        };
        if prepared_action != action {
            return Err(conflict());
        }
        let (cell, prior_pending, predecessor_cells) = {
            let _configuration = self.configuration_mutation_guard()?;
            if action != FinishAction::Rollback {
                writable_grid_profile(&self.persistence.profile(&owner.connection_id)?)?;
                if maybe_session
                    .as_ref()
                    .is_none_or(|s| s.closed.load(Ordering::Acquire))
                    || owner.driver.is_closed()
                {
                    return Err(disconnected());
                }
            } else if owner.driver.is_closed() {
                return Err(disconnected());
            }
            let descriptor = BatchDescriptor {
                operation_id: prepared.operation_id().into(),
                connection_id: owner.connection_id.clone(),
                session_id: owner.session_id.clone(),
                helper_generation: owner.observer.helper_generation().generation().into(),
                started_at: now_ms(),
                digest: prepared.digest().into(),
            };
            let memory = guard.take_memory()?;
            if action == FinishAction::Commit {
                let cell = match self.admit_batch_finish(
                    memory,
                    descriptor,
                    &owner.group,
                    helper_owner,
                    FinishAction::Commit,
                    owner.observer.clone(),
                ) {
                    Ok(cell) => cell,
                    Err(error) => {
                        if error.code == ErrorCode::Internal {
                            owner.state.lock().finished_or_unknown = true;
                            Driver::close(owner.driver.as_ref());
                        }
                        return Err(error);
                    }
                };
                guard.admitted_cell = Some(cell.clone());
                guard.retire_driver_on_abort = Some(owner.driver.clone());
                guard.manual_owner_on_abort = Some(owner.clone());
                let prior = owner
                    .state
                    .lock()
                    .pending_authority
                    .as_ref()
                    .map(|(authority, _)| authority.clone());
                if let Some(ref session) = maybe_session {
                    self.hold_session_after_grid_admission(session, &owner.session_id);
                }
                (cell, prior, Vec::new())
            } else {
                let mut predecessors = Vec::new();
                let (pending_snapshot, taken_recovery) = {
                    let mut st = owner.state.lock();
                    if matches!(
                        &st.successor_recovery,
                        Some(SuccessorRollbackRecovery::RejectedAdmission(rec))
                            if rec.cell.is_settled()
                    ) {
                        st.successor_recovery = None;
                    }
                    if let Some((_, ref pending_cell)) = st.pending_authority
                        && !pending_cell.is_settled()
                    {
                        predecessors.push(pending_cell.clone());
                    }
                    if let Some(SuccessorRollbackRecovery::RejectedAdmission(ref rec)) =
                        st.successor_recovery
                        && !rec.cell.is_settled()
                    {
                        predecessors.push(rec.cell.clone());
                    }
                    (st.pending_authority.clone(), st.successor_recovery.take())
                };
                let admitted = match &taken_recovery {
                    Some(SuccessorRollbackRecovery::NoDispatch(rec)) => {
                        let (authority, _) = pending_snapshot.as_ref().ok_or_else(conflict)?;
                        self.admit_protected_grid_rollback(
                            memory,
                            descriptor,
                            &owner.group,
                            helper_owner,
                            owner.observer.clone(),
                            ProtectedRollbackSource::NoDispatch {
                                authority,
                                successor: &rec.successor,
                                request: &rec.request,
                                proof: &rec.proof,
                            },
                        )
                    }
                    Some(SuccessorRollbackRecovery::RejectedAdmission(rec))
                        if !rec.cell.is_settled() =>
                    {
                        self.admit_protected_grid_rollback(
                            memory,
                            descriptor,
                            &owner.group,
                            helper_owner,
                            owner.observer.clone(),
                            ProtectedRollbackSource::RejectedAdmission {
                                rejected: &rec.authority,
                                cell: &rec.cell,
                            },
                        )
                    }
                    _ => {
                        if let Some((ref authority, ref pending_cell)) = pending_snapshot
                            && !pending_cell.is_settled()
                        {
                            self.admit_protected_grid_rollback(
                                memory,
                                descriptor,
                                &owner.group,
                                helper_owner,
                                owner.observer.clone(),
                                ProtectedRollbackSource::UnsettledPending {
                                    authority,
                                    cell: pending_cell,
                                },
                            )
                        } else {
                            self.admit_batch_finish(
                                memory,
                                descriptor,
                                &owner.group,
                                helper_owner,
                                FinishAction::Rollback,
                                owner.observer.clone(),
                            )
                        }
                    }
                };
                let cell = match admitted {
                    Ok(cell) => {
                        let mut st = owner.state.lock();
                        st.successor_recovery = taken_recovery;
                        st.blocked_cell = Some(cell.clone());
                        cell
                    }
                    Err(error) => {
                        owner.state.lock().successor_recovery = taken_recovery;
                        return Err(error);
                    }
                };
                guard.admitted_cell = Some(cell.clone());
                guard.retire_driver_on_abort = Some(owner.driver.clone());
                guard.manual_owner_on_abort = Some(owner.clone());
                if let Some(ref session) = maybe_session {
                    self.hold_session_after_grid_admission(session, &owner.session_id);
                }
                (cell, None, predecessors)
            }
        };
        #[cfg(all(test, unix, feature = "test-harness"))]
        assert!(
            !self
                .snowflake_grid
                .take_fault_if(GridTestFault::PanicBeforeDispatch),
            "synthetic grid panic after durable finish reservation"
        );
        self.execute_prepared_grid_operation(
            prepared,
            if action == FinishAction::Rollback {
                RecorderRole::Rollback
            } else {
                RecorderRole::ContinueOrCommit
            },
            &owner.driver,
            &cell,
            Some(&owner),
            prior_pending,
            predecessor_cells,
            guard,
        )
        .await
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep prepared native execution, durable cell and rollback lineage in one call"
    )]
    async fn execute_prepared_grid_operation(
        &self,
        prepared: PreparedGridOperation,
        role: RecorderRole,
        driver: &Arc<SnowflakeExternal>,
        cell: &Arc<BatchOutcomeCell>,
        manual: Option<&Arc<ManualOwner>>,
        prior_pending: Option<PendingBatchRollbackAuthority>,
        predecessor_cells: Vec<Arc<BatchOutcomeCell>>,
        guard: &mut RunGuard,
    ) -> AppResult<SnowflakeGridExecution> {
        let operation_id = prepared.operation_id().to_owned();
        let transaction_id = manual.map(|owner| owner.group.id().to_owned());
        let prepared_request = prepared.request().clone();
        let successor_reservation = cell.reservation();
        let mut admission = StudioGridAdmission { studio: self, cell };
        let mut recorder = StudioGridRecorder {
            studio: self,
            role,
            driver,
            cell,
            manual,
            prepared_request,
            successor_reservation,
            prior_pending,
            predecessor_cells,
        };
        match prepared.execute(&mut admission, &mut recorder).await {
            Ok(grid_outcome) => {
                guard.completed = true;
                let outcome = report_outcome(&grid_outcome.report);
                Ok(SnowflakeGridExecution {
                    operation_id,
                    transaction_id,
                    outcome,
                    report: grid_outcome.report,
                    recording_error: grid_outcome.recording_error,
                })
            }
            Err(error) => {
                if !cell.was_dispatched() {
                    let _ = self.abandon_batch_before_dispatch(cell);
                    if matches!(role, RecorderRole::InitialManual | RecorderRole::Rollback) {
                        if let Some(owner) = manual {
                            let mut st = owner.state.lock();
                            st.pending_authority = None;
                            st.successor_recovery = None;
                            st.blocked_cell = None;
                            st.finished_or_unknown = true;
                        }
                        Driver::close(driver.as_ref());
                    }
                }
                guard.completed = true;
                Err(error)
            }
        }
    }
}
