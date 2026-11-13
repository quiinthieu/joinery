//! Studio-owned compact originals, independent of caller and tab lifetime.
//! This layer neither dispatches SQL nor grants a manual helper owner. The
//! lane-bearing adapter must retain its separate native owner across Pending.
#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Retained batch recording foundation precedes the application grid dispatcher"
    )
)]
use super::outcome_memory::{MemoryLease, SafetySlot};
use super::*;
use crate::{
    drivers::{
        snowflake_batch::{BatchEvidence, BatchRequest, FinishAction, Mode, Outcome},
        snowflake_process::{HelperRetirementObserver, Operation, Report, Retirement},
    },
    persistence::{
        OperationGate,
        batch_outcomes::{
            BatchDescriptor, BatchPlan, BatchReservation, BatchTermination, ManualTransactionLease,
            OriginalBatchOutcome, VolatileBatchRollbackToken,
        },
    },
};
use tokio::sync::watch;

const MAX_CELLS: usize = 64;
const MAX_GROUPS: usize = 32;

#[derive(Default)]
pub(super) struct State {
    pending: Mutex<HashMap<String, Arc<BatchOutcomeCell>>>,
    groups: Mutex<HashMap<String, IdleGroup>>,
}
impl State {
    pub(super) fn is_empty(&self) -> bool {
        self.pending.lock().is_empty() && self.groups.lock().is_empty()
    }
}
struct IdleGroup {
    lease: ManualTransactionLease,
    observer: HelperRetirementObserver,
    rollback_memory: Arc<SafetySlot>,
}
enum Attempt {
    Batch {
        plan: BatchPlan,
        group: Option<ManualTransactionLease>,
        rollback_memory: Option<Arc<SafetySlot>>,
    },
    Finish {
        group: ManualTransactionLease,
        owner: uuid::Uuid,
        action: FinishAction,
    },
}
impl Attempt {
    fn group(&self) -> Option<&ManualTransactionLease> {
        match self {
            Self::Batch { group, .. } => group.as_ref(),
            Self::Finish { group, .. } => Some(group),
        }
    }
    fn new_manual(&self) -> bool {
        matches!(self,Self::Batch {plan,..} if matches!(plan.request(),BatchRequest::Batch {mode:Mode::Manual,owner:None,..}))
    }
}
pub(super) struct BatchOutcomeCell {
    descriptor: BatchDescriptor,
    observer: HelperRetirementObserver,
    state: Mutex<Cell>,
}
struct Cell {
    attempt: Attempt,
    memory: Option<MemoryLease>,
    gate: Option<OperationGate>,
    reservation: Option<BatchReservation>,
    volatile: Option<VolatileBatchRollbackToken>,
    group: Option<ManualTransactionLease>,
    terminal: Option<watch::Receiver<Option<Report>>>,
    original: Option<OriginalBatchOutcome>,
    ready: bool,
    dispatched: bool,
    no_original: bool,
    encoding_failed: bool,
    captured: bool,
    settled: bool,
    close_group: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RecordingState {
    pub captured: bool,
    pub settled: bool,
    pub awaiting_terminal: bool,
    pub encoding_failed: bool,
}
impl BatchOutcomeCell {
    pub(super) fn operation_id(&self) -> &str {
        &self.descriptor.operation_id
    }
    pub(super) fn manual_transaction(&self) -> Option<ManualTransactionLease> {
        self.state.lock().group.clone()
    }
    pub(super) fn recording_state(&self) -> RecordingState {
        let s = self.state.lock();
        RecordingState {
            captured: s.captured,
            settled: s.settled && s.memory.is_none(),
            awaiting_terminal: s.dispatched
                && s.original.is_none()
                && !s.no_original
                && !s.encoding_failed,
            encoding_failed: s.encoding_failed,
        }
    }
    pub(super) fn is_settled(&self) -> bool {
        self.recording_state().settled
    }
}
fn recording_error(captured: bool) -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        if captured {
            "The original batch outcome was captured locally, but local settlement is pending; keep Joinery open"
        } else {
            "The original batch outcome is not durably recorded; keep Joinery open until local recording settles"
        },
    )
}
fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The batch owner or recording state changed; no new SQL was admitted",
    )
}

impl Studio {
    pub(super) fn admit_batch_outcome(
        &self,
        memory: MemoryLease,
        descriptor: BatchDescriptor,
        plan: BatchPlan,
        group: Option<&ManualTransactionLease>,
        rollback_memory: Option<Arc<SafetySlot>>,
        observer: HelperRetirementObserver,
    ) -> AppResult<Arc<BatchOutcomeCell>> {
        let new_manual = matches!(
            plan.request(),
            BatchRequest::Batch {
                mode: Mode::Manual,
                owner: None,
                ..
            }
        );
        if new_manual != rollback_memory.is_some() {
            return Err(conflict());
        }
        self.admit_batch_cell(
            memory,
            descriptor,
            Attempt::Batch {
                plan,
                group: group.cloned(),
                rollback_memory,
            },
            observer,
        )
    }
    pub(super) fn admit_batch_finish(
        &self,
        memory: MemoryLease,
        descriptor: BatchDescriptor,
        group: &ManualTransactionLease,
        helper_owner: uuid::Uuid,
        action: FinishAction,
        observer: HelperRetirementObserver,
    ) -> AppResult<Arc<BatchOutcomeCell>> {
        self.admit_batch_cell(
            memory,
            descriptor,
            Attempt::Finish {
                group: group.clone(),
                owner: helper_owner,
                action,
            },
            observer,
        )
    }
    fn admit_batch_cell(
        &self,
        memory: MemoryLease,
        descriptor: BatchDescriptor,
        attempt: Attempt,
        observer: HelperRetirementObserver,
    ) -> AppResult<Arc<BatchOutcomeCell>> {
        let canonical =
            |value: &str| uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value);
        if !canonical(&descriptor.operation_id)
            || !canonical(&descriptor.session_id)
            || !canonical(&descriptor.helper_generation)
            || descriptor.connection_id.is_empty()
            || descriptor.connection_id.len() > 256
            || descriptor.connection_id.contains('\0')
            || descriptor.started_at > 9_007_199_254_740_991
            || descriptor.digest.len() != 64
            || !descriptor
                .digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(AppError::invalid("Batch recording descriptor is invalid"));
        }
        if descriptor.helper_generation != observer.helper_generation().generation() {
            return Err(conflict());
        }
        if let Some(group) = attempt.group() {
            let groups = self.batch_outcomes.groups.lock();
            let retained = groups.get(group.id()).ok_or_else(conflict)?;
            if !retained
                .observer
                .helper_generation()
                .same_helper(&observer.helper_generation())
            {
                return Err(conflict());
            }
        }
        let gate = self.persistence.operation_gate(&self.history_owner)?;
        let cell = Arc::new(BatchOutcomeCell {
            descriptor,
            observer,
            state: Mutex::new(Cell {
                attempt,
                memory: Some(memory),
                gate: Some(gate),
                reservation: None,
                volatile: None,
                group: None,
                terminal: None,
                original: None,
                ready: false,
                dispatched: false,
                no_original: false,
                encoding_failed: false,
                captured: false,
                settled: false,
                close_group: false,
            }),
        });
        {
            let mut pending = self.batch_outcomes.pending.lock();
            if pending.len() >= MAX_CELLS || pending.contains_key(cell.operation_id()) {
                return Err(conflict());
            }
            pending.insert(cell.operation_id().into(), cell.clone());
        }
        // Retain the exact candidate before committing. An admission failure
        // never dispatches, even if retry later finds its durable reservation.
        let result = self.reserve_batch_cell(&cell);
        if let Err(error) = result {
            let mut s = cell.state.lock();
            s.no_original = true;
            // A definite pre-write refusal owns no recording obligation. An
            // I/O error may hide a committed reservation: retain its candidate
            // and retry exactly, without ever dispatching it later.
            if s.reservation.is_none() && s.volatile.is_none() && error.code != ErrorCode::Internal
            {
                s.memory.take();
                s.gate.take();
                s.settled = true;
            }
            drop(s);
            self.forget_batch_cell(&cell);
            return Err(error);
        }
        Ok(cell)
    }
    fn reserve_batch_cell(&self, cell: &BatchOutcomeCell) -> AppResult<()> {
        let mut s = cell.state.lock();
        if s.ready {
            return Ok(());
        }
        let gate = s.gate.as_ref().ok_or_else(conflict)?;
        if s.reservation.is_none() && s.volatile.is_none() {
            let reserved = match &s.attempt {
                Attempt::Batch { plan, group, .. } => self.persistence.reserve_owned_batch(
                    &self.history_owner,
                    gate,
                    &cell.descriptor,
                    plan,
                    group.as_ref(),
                ),
                Attempt::Finish { group, action, .. } => {
                    self.persistence.reserve_owned_batch_finish(
                        &self.history_owner,
                        gate,
                        &cell.descriptor,
                        group,
                        *action,
                    )
                }
            };
            match reserved {
                Ok(r) => s.reservation = Some(r),
                Err(error) => {
                    if let Attempt::Finish {
                        group,
                        owner,
                        action: FinishAction::Rollback,
                    } = &s.attempt
                    {
                        let token = self.persistence.make_volatile_batch_rollback_token(
                            &self.history_owner,
                            gate,
                            &cell.descriptor,
                            group,
                            *owner,
                        )?;
                        s.group = Some(group.clone());
                        s.volatile = Some(token);
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        if let Some(r) = s.reservation.as_ref() {
            let group = r.manual_transaction();
            if let Some(group) = group {
                let mut groups = self.batch_outcomes.groups.lock();
                if let Some(old) = groups.get(group.id()) {
                    if !old
                        .observer
                        .helper_generation()
                        .same_helper(&cell.observer.helper_generation())
                    {
                        return Err(conflict());
                    }
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(conflict());
                    }
                    let rollback_memory = match &s.attempt {
                        Attempt::Batch {
                            rollback_memory: Some(memory),
                            ..
                        } => memory.clone(),
                        _ => return Err(conflict()),
                    };
                    groups.insert(
                        group.id().into(),
                        IdleGroup {
                            lease: group.clone(),
                            observer: cell.observer.clone(),
                            rollback_memory,
                        },
                    );
                }
                drop(groups);
                if s.attempt.new_manual() {
                    self.persistence.pin_manual_transaction_helper(
                        &self.history_owner,
                        s.gate.as_ref().unwrap(),
                        r,
                        &cell.observer.helper_generation(),
                    )?;
                }
                s.group = Some(group);
            }
        }
        s.ready = true;
        Ok(())
    }

    /// Borrow the pre-authentication safety buffer retained by this exact local
    /// group. Ordinary exhaustion and removal from the session map cannot free it.
    pub(super) fn claim_batch_rollback_memory(
        &self,
        group: &ManualTransactionLease,
    ) -> AppResult<MemoryLease> {
        self.batch_outcomes
            .groups
            .lock()
            .get(group.id())
            .ok_or_else(conflict)?
            .rollback_memory
            .claim()
    }

    /// The lane owner supplies only its single synchronous Supervisor request.
    /// Err guarantees that no operation entered the queue. Do not await, call
    /// observers, or perform fallible post-admission work inside this closure.
    pub(super) fn dispatch_batch_operation(
        &self,
        cell: &Arc<BatchOutcomeCell>,
        dispatch: impl FnOnce() -> AppResult<Operation>,
    ) -> AppResult<Operation> {
        let (result, discarded) = {
            let mut s = cell.state.lock();
            if !s.ready || s.dispatched || s.no_original || s.encoding_failed || s.settled {
                return Err(conflict());
            }
            let expected = match &s.attempt {
                Attempt::Batch { plan, .. } => plan.request().clone(),
                Attempt::Finish { owner, action, .. } => BatchRequest::Finish {
                    owner: *owner,
                    action: *action,
                },
            };
            // A panic after queue admission cannot leave a negative witness
            // admissible. Only a returned Err discharges this conservative hold.
            s.dispatched = true;
            s.encoding_failed = true;
            match dispatch() {
                Ok(operation) => {
                    s.terminal = Some(operation.terminal_receiver());
                    if operation.matches_batch_binding(
                        &cell.observer.helper_generation(),
                        cell.operation_id(),
                        &expected,
                    ) {
                        s.encoding_failed = false;
                        (Ok(operation), None)
                    } else {
                        (Err(conflict()), Some(operation))
                    }
                }
                Err(error) => {
                    s.dispatched = false;
                    s.encoding_failed = false;
                    s.no_original = true;
                    (Err(error), None)
                }
            }
        };
        // Retiring a mismatched operation can publish transport completion.
        // Neither that drop nor storage retries run under the cell mutex.
        drop(discarded);
        if cell.state.lock().no_original {
            let _ = self.flush_batch_cell(cell);
            self.forget_batch_cell(cell);
        }
        result
    }

    /// Record once, after the adapter has frozen its actual terminal/prefix.
    /// Incremental ACK prefixes must not prematurely replace a later terminal.
    pub(super) fn record_batch_outcome(
        &self,
        cell: &Arc<BatchOutcomeCell>,
        evidence: &BatchEvidence,
        termination: BatchTermination,
    ) -> AppResult<()> {
        self.encode_batch_cell(cell, evidence, termination)?;
        let result = self.flush_batch_cell(cell);
        self.forget_batch_cell(cell);
        result
    }
    fn encode_batch_cell(
        &self,
        cell: &BatchOutcomeCell,
        evidence: &BatchEvidence,
        termination: BatchTermination,
    ) -> AppResult<()> {
        let _encoding = self.outcome_memory.encoding.lock();
        let mut s = cell.state.lock();
        if s.original.is_some() || s.settled {
            return Ok(());
        }
        if s.encoding_failed || s.no_original || !s.ready {
            return Err(recording_error(s.captured));
        }
        s.encoding_failed = true;
        let buffer = s.memory.as_mut().ok_or_else(conflict)?.take_buffer();
        let original = if let Some(r) = &s.reservation {
            OriginalBatchOutcome::capture_in(r, evidence, termination, now_ms(), buffer)
        } else {
            OriginalBatchOutcome::volatile_rollback_in(
                s.volatile.as_ref().ok_or_else(conflict)?,
                evidence,
                termination,
                now_ms(),
                buffer,
            )
        }
        .map_err(|_| recording_error(false))?;
        s.close_group = matches!(
            evidence.summary.as_ref().map(|summary| summary.outcome),
            Some(Outcome::Committed | Outcome::RolledBack)
        ) || (s.attempt.new_manual()
            && evidence
                .summary
                .as_ref()
                .is_some_and(|summary| summary.outcome == Outcome::Rejected));
        s.original = Some(original);
        s.encoding_failed = false;
        s.terminal.take();
        Ok(())
    }
    pub(super) fn abandon_batch_before_dispatch(
        &self,
        cell: &Arc<BatchOutcomeCell>,
    ) -> AppResult<()> {
        {
            let mut s = cell.state.lock();
            if s.dispatched || s.original.is_some() || s.encoding_failed {
                return Err(conflict());
            }
            s.no_original = true;
        }
        let result = self.flush_batch_cell(cell);
        self.forget_batch_cell(cell);
        result
    }
    fn poll_batch_terminal(&self, cell: &BatchOutcomeCell) -> AppResult<()> {
        let report = {
            let s = cell.state.lock();
            if s.original.is_some() || s.no_original || s.settled {
                return Ok(());
            }
            if s.encoding_failed {
                return Err(recording_error(s.captured));
            }
            s.terminal
                .as_ref()
                .and_then(|receiver| receiver.borrow().clone())
        };
        let Some(report) = report else {
            return Ok(());
        };
        if let Some(evidence) = &report.batch {
            let termination = match report.retirement {
                Some(Retirement::Eof) => BatchTermination::Eof,
                Some(Retirement::Deadline) => BatchTermination::Deadline,
                Some(Retirement::Protocol | Retirement::Write) => BatchTermination::ProtocolFailure,
                Some(Retirement::Requested | Retirement::Dropped) => BatchTermination::Cancelled,
                None if evidence.summary.is_some() => BatchTermination::Complete,
                None => BatchTermination::ProtocolFailure,
            };
            self.encode_batch_cell(cell, evidence, termination)
        } else {
            // A genuine final report with no batch evidence is a negative
            // witness. An empty/closed watch by itself is never sufficient.
            let mut s = cell.state.lock();
            if s.original.is_none() && !s.encoding_failed {
                s.no_original = true;
                s.terminal.take();
            }
            Ok(())
        }
    }
    fn flush_batch_cell(&self, cell: &BatchOutcomeCell) -> AppResult<()> {
        self.poll_batch_terminal(cell)?;
        {
            let s = cell.state.lock();
            if s.memory.is_none() {
                return Ok(());
            }
            if s.encoding_failed {
                return Err(recording_error(s.captured));
            }
            if s.original.is_none() && !s.no_original {
                return Ok(());
            }
        }
        self.reserve_batch_cell(cell)
            .map_err(|_| recording_error(false))?;
        let mut s = cell.state.lock();
        if s.reservation.is_none() {
            s.reservation = Some(
                self.persistence
                    .promote_volatile_batch_rollback(
                        &self.history_owner,
                        s.gate.as_ref().unwrap(),
                        s.volatile.as_ref().ok_or_else(conflict)?,
                    )
                    .map_err(|_| recording_error(false))?,
            );
            s.volatile.take();
        }
        if !s.settled {
            let gate = s.gate.as_ref().unwrap();
            let r = s.reservation.as_ref().unwrap();
            if s.no_original {
                self.persistence
                    .abandon_reserved_batch(&self.history_owner, gate, r)
                    .map_err(|_| recording_error(false))?;
            } else {
                let original = s.original.as_ref().ok_or_else(conflict)?;
                self.persistence
                    .capture_batch_outcome(&self.history_owner, gate, r, original)
                    .map_err(|_| recording_error(false))?;
                s.captured = true;
                self.persistence
                    .settle_batch_outcome(
                        &self.history_owner,
                        s.gate.as_ref().unwrap(),
                        s.reservation.as_ref().unwrap(),
                        s.original.as_ref().unwrap(),
                    )
                    .map_err(|_| recording_error(true))?;
            }
            s.settled = true;
        }
        if let Some(original) = s.original.take() {
            match original.try_reclaim_buffer() {
                Ok(buffer) => s.memory.as_mut().unwrap().restore_buffer(buffer),
                Err(original) => {
                    s.original = Some(original);
                    return Err(recording_error(true));
                }
            }
        }
        s.memory.take();
        s.gate.take();
        s.volatile.take();
        if let Attempt::Batch {
            rollback_memory, ..
        } = &mut s.attempt
        {
            rollback_memory.take();
        }
        if s.close_group
            && let Some(group) = &s.group
        {
            self.batch_outcomes.groups.lock().remove(group.id());
        }
        Ok(())
    }
    fn forget_batch_cell(&self, cell: &BatchOutcomeCell) {
        if cell.is_settled() {
            self.batch_outcomes
                .pending
                .lock()
                .remove(cell.operation_id());
        }
    }
    /// Retry local storage only. Never reconnect, replay SQL, or infer rollback.
    pub(super) fn flush_batch_outcomes(&self) -> AppResult<()> {
        let cells = self
            .batch_outcomes
            .pending
            .lock()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut failed = false;
        for cell in cells {
            if self.flush_batch_cell(&cell).is_err() {
                failed = true;
            }
            self.forget_batch_cell(&cell);
        }
        let groups = self
            .batch_outcomes
            .groups
            .lock()
            .values()
            .map(|group| (group.lease.clone(), group.observer.clone()))
            .collect::<Vec<_>>();
        for (group, observer) in groups {
            let Some(proof) = observer.reaped_generation() else {
                continue;
            };
            if self.batch_outcomes.pending.lock().values().any(|cell| {
                let s = cell.state.lock();
                s.group
                    .as_ref()
                    .or(s.attempt.group())
                    .is_some_and(|g| g.id() == group.id())
            }) {
                continue;
            }
            let result = self
                .persistence
                .operation_gate(&self.history_owner)
                .and_then(|gate| {
                    self.persistence.retire_manual_transaction(
                        &self.history_owner,
                        &gate,
                        &group,
                        &proof,
                    )
                });
            if result.is_ok() {
                self.batch_outcomes.groups.lock().remove(group.id());
            } else {
                failed = true;
            }
        }
        if failed {
            Err(recording_error(false))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "batch_outcome_recording_tests.rs"]
mod tests;
