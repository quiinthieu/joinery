//! Owning harness-only admission and phase barriers. No account/profile entry,
//! approval envelope, resume path, or production qualification claim lives here.
//! The fixed runner must still supply original operation/collector evidence.
//! Separate local-completion observations retain original retirement reports;
//! complete production-route interruption qualification remains required.

use super::{
    ExpectedState, Operation, Plan, RuntimePin, SCHEMA, TransactionSlot,
    evidence::{self, PhaseEvidence},
    journal::Journal,
    ledger::{SqlEntry, SqlKind},
    lifecycle::Lifecycle,
    sequence::{AdmissionKind, Sequencer},
};
use crate::{
    domain::{AppError, AppResult, QueryEvent, SessionRuntimeState, TransactionControlAuthority},
    drivers::{
        snowflake_qualification_metadata::{
            MetadataObserver, MetadataSite, RelationObservation, SchemaObservation,
        },
        snowflake_qualification_transport::{
            LocalTerminal, PhysicalRequest, PhysicalTerminal, TransportPolicy,
        },
    },
};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
};

const MAX_PENDING: usize = 1024;

/// Original owning-operation outputs, captured before completion validation.
/// Borrowed inputs become an owned queue record before the first storage await.
#[derive(Serialize)]
#[serde(tag = "status", content = "original", rename_all = "camelCase")]
pub(super) enum OperationCompletion<'a> {
    Returned(PhaseEvidence<'a>),
    Failed {
        error: &'a AppError,
        events: &'a [QueryEvent],
    },
    /// A bounded original event prefix cannot be presented as a complete read.
    /// Keep its collection failure separate from the original operation error.
    IncompleteRead {
        error: &'a AppError,
        #[serde(rename = "originalError")]
        original_error: Option<&'a AppError>,
        events: &'a [QueryEvent],
        #[serde(rename = "cleanupError")]
        cleanup_error: Option<&'a AppError>,
    },
}

fn denied() -> AppError {
    AppError::invalid("Existing-schema qualification phase evidence is incomplete or out of order")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Observation {
    Schemas(MetadataSite),
    Relations(MetadataSite, Option<String>),
    Runtime,
}

struct State {
    sequence: Sequencer,
    lifecycle: Lifecycle,
    pending: VecDeque<(u64, Value)>,
    event_sequence: u64,
    debt: VecDeque<Observation>,
    last_sql: Option<SqlEntry>,
    console_id: Option<String>,
    runtime_pin: Option<RuntimePin>,
    last_runtime: Option<SessionRuntimeState>,
    phase_runtime_count: usize,
    authority_revisions: HashSet<String>,
    operation_completions: HashSet<u8>,
    transaction_a: Option<String>,
    transaction_b: Option<String>,
    failure: Option<AppError>,
    flushing: bool,
}

impl State {
    fn check(&self) -> AppResult<()> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        self.sequence.check()
    }

    fn fail(&mut self, error: AppError) -> AppError {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
        let error = self.failure.as_ref().expect("retained failure").clone();
        self.sequence.fail(error.clone());
        error
    }

    fn enqueue(&mut self, value: Value) -> AppResult<()> {
        if self.pending.len() >= MAX_PENDING {
            return Err(self.fail(denied()));
        }
        self.event_sequence = self.event_sequence.checked_add(1).ok_or_else(denied)?;
        self.pending.push_back((self.event_sequence, value));
        Ok(())
    }

    fn consume(&mut self, actual: Observation) -> AppResult<()> {
        self.check()?;
        if self.debt.front() != Some(&actual)
            || self.last_sql.as_ref().is_none_or(|entry| {
                self.sequence
                    .phase()
                    .is_none_or(|phase| entry.phase() != phase.number())
            })
        {
            return Err(denied());
        }
        self.debt.pop_front();
        Ok(())
    }

    fn validate_context_identity(&self) -> AppResult<()> {
        let context = self.lifecycle.context().ok_or_else(denied)?;
        self.sequence.validate_context(context)
    }

    fn runtime(&mut self, runtime: &SessionRuntimeState) -> AppResult<()> {
        self.consume(Observation::Runtime)?;
        self.validate_context_identity()?;
        let context = self.lifecycle.context().ok_or_else(denied)?;
        if self.console_id.as_deref() != Some(runtime.session_id.as_str())
            || runtime.context_fingerprint.as_deref() != Some(context.fingerprint()?.as_str())
            || runtime.role != context.role
        {
            return Err(denied());
        }
        let phase = self.sequence.phase().ok_or_else(denied)?.clone();
        if self.runtime_pin.is_none() {
            if phase.number() != 2 {
                return Err(denied());
            }
            self.runtime_pin = Some(RuntimePin::capture(runtime)?);
        }
        let pin = self.runtime_pin.as_ref().ok_or_else(denied)?;
        let transaction_id = phase
            .state()
            .validate(
                runtime,
                pin,
                self.transaction_a.as_deref(),
                self.transaction_b.as_deref(),
            )?
            .map(str::to_owned);
        if let ExpectedState::NewTransaction(slot) = phase.state() {
            match slot {
                TransactionSlot::A => self.transaction_a = transaction_id,
                TransactionSlot::B => self.transaction_b = transaction_id,
            }
        }
        let Some(TransactionControlAuthority::Observed { revision, .. }) =
            &runtime.transaction_control_authority
        else {
            return Err(denied());
        };
        if self.authority_revisions.len() >= 128
            || !self.authority_revisions.insert(revision.clone())
        {
            return Err(denied());
        }
        self.last_runtime = Some(runtime.clone());
        self.phase_runtime_count += 1;
        self.enqueue(json!({"type":"runtime", "phase":phase.number(), "state":runtime}))
    }
}

pub(super) struct Policy {
    state: Arc<Mutex<State>>,
    journal: Arc<std::sync::Mutex<Journal>>,
    gate: tokio::sync::Mutex<()>,
    flush_updates: tokio::sync::watch::Sender<u64>,
}

// Dropping an owner future after reservation must not allow an fsync/dispatch
// ambiguity to be retried. Detached blocking IO may finish; this attempt cannot
// continue or resume. No State/process lock is held across blocking IO or await.
struct Interrupted {
    state: Arc<Mutex<State>>,
    armed: bool,
}
impl Drop for Interrupted {
    fn drop(&mut self) {
        if self.armed {
            self.state.lock().fail(AppError::internal(
                "Qualification owner operation interrupted",
            ));
        }
    }
}

impl Policy {
    pub(super) async fn create(directory: PathBuf, frozen_profile: Value) -> AppResult<Self> {
        Self::create_with_plan(directory, frozen_profile, Plan::generate()?).await
    }

    /// Pin the caller's verified closed definition before journal creation.
    /// This supplies neither approval nor the still-required execution envelope.
    pub(super) async fn create_with_plan(
        directory: PathBuf,
        frozen_profile: Value,
        plan: Plan,
    ) -> AppResult<Self> {
        let sequence = Sequencer::with_plan(plan, frozen_profile)?;
        let lifecycle = Lifecycle::new(sequence.plan());
        let hash = sequence.plan().sha256()?;
        let journal = tokio::task::spawn_blocking(move || Journal::create(&directory, &hash))
            .await
            .map_err(|_| denied())??;
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                sequence,
                lifecycle,
                pending: VecDeque::new(),
                event_sequence: 0,
                debt: VecDeque::new(),
                last_sql: None,
                console_id: None,
                runtime_pin: None,
                last_runtime: None,
                phase_runtime_count: 0,
                authority_revisions: HashSet::new(),
                operation_completions: HashSet::new(),
                transaction_a: None,
                transaction_b: None,
                failure: None,
                flushing: false,
            })),
            journal: Arc::new(std::sync::Mutex::new(journal)),
            gate: tokio::sync::Mutex::new(()),
            flush_updates: tokio::sync::watch::channel(0).0,
        })
    }

    fn interrupted(&self) -> Interrupted {
        Interrupted {
            state: self.state.clone(),
            armed: true,
        }
    }

    pub(super) fn plan(&self) -> Plan {
        self.state.lock().sequence.plan().clone()
    }

    async fn flush(&self) -> AppResult<()> {
        self.flush_pending().await?;
        self.state.lock().check()
    }

    // Storage success is separate from the admission failure latch: original
    // operation outputs must remain recordable after an observer has failed.
    async fn flush_pending(&self) -> AppResult<()> {
        // An interrupted owner may leave its blocking write running. Wait for
        // that exact batch to settle before taking a new prefix, so final
        // cleanup cannot replay records whose fsync already succeeded.
        let mut updates = self.flush_updates.subscribe();
        let pending = loop {
            {
                let mut state = self.state.lock();
                if !state.flushing {
                    if state.pending.is_empty() {
                        return Ok(());
                    }
                    state.flushing = true;
                    break state.pending.iter().cloned().collect::<Vec<_>>();
                }
            }
            updates.changed().await.map_err(|_| denied())?;
        };
        let journal = self.journal.clone();
        let state = self.state.clone();
        let updates = self.flush_updates.clone();
        let persisted = tokio::task::spawn_blocking(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut journal = journal.lock().map_err(|_| denied())?;
                for (id, event) in pending {
                    let receipt = journal.append(&event)?;
                    let mut state = state.lock();
                    if state.pending.front().map(|item| item.0) != Some(id)
                        || receipt.sequence() == 0
                        || receipt.sha256().len() != 64
                    {
                        return Err(denied());
                    }
                    // The blocking batch owns receipt consumption even if its
                    // original async caller has disappeared. Originals leave
                    // pending only after their own successful fsync receipt.
                    state.pending.pop_front();
                }
                Ok(())
            }))
            .unwrap_or_else(|_| Err(denied()));
            let mut state = state.lock();
            state.flushing = false;
            if let Err(error) = &result {
                state.fail(error.clone());
            }
            updates.send_modify(|revision| *revision = revision.wrapping_add(1));
            result
        })
        .await
        .map_err(|_| denied())
        .and_then(|value| value);
        match persisted {
            Ok(()) => {}
            Err(error) => return Err(self.state.lock().fail(error)),
        }
        Ok(())
    }

    /// Sync one original completion and every earlier queued original without
    /// validating or advancing the phase. Ok(Some(error)) means storage succeeded
    /// but the retained first policy/operation failure still prohibits progress.
    /// A duplicate cannot replace an original, including after interrupted IO.
    pub(super) async fn record_operation_completion(
        &self,
        completion: OperationCompletion<'_>,
    ) -> AppResult<Option<AppError>> {
        let _gate = self.gate.lock().await;
        let mut interrupted = self.interrupted();
        let original =
            serde_json::to_value(&completion).map_err(|_| self.state.lock().fail(denied()))?;
        let event_id = {
            let mut state = self.state.lock();
            let Some(phase) = state.sequence.phase().map(|phase| phase.number()) else {
                return Err(state.fail(denied()));
            };
            if state.operation_completions.contains(&phase) {
                return Err(state.fail(denied()));
            }
            state.enqueue(json!({"type":"operationCompletion", "phase":phase,
                "completion":original}))?;
            state.operation_completions.insert(phase);
            if let OperationCompletion::Failed { error, .. }
            | OperationCompletion::IncompleteRead { error, .. } = completion
            {
                state.fail(error.clone());
            }
            state.event_sequence
        };
        self.flush_pending().await?;
        let result = {
            let mut state = self.state.lock();
            // Each prefix item leaves pending only after its own fsync receipt.
            // Never translate an existing failure into a storage-success claim.
            if state.pending.iter().any(|(id, _)| *id <= event_id) {
                return Err(state.fail(denied()));
            }
            state.check().err()
        };
        interrupted.armed = false;
        Ok(result)
    }

    pub(super) async fn finish_phase(&self, evidence: PhaseEvidence<'_>) -> AppResult<()> {
        let _gate = self.gate.lock().await;
        let mut interrupted = self.interrupted();
        let result = {
            let mut state = self.state.lock();
            (|| {
                state.check()?;
                state.sequence.phase_ready()?;
                let phase = state.sequence.phase().ok_or_else(denied)?.clone();
                if !state.debt.is_empty() {
                    return Err(denied());
                }
                if phase.number() != 1 {
                    let expected = match phase.operation() {
                        Operation::Catalog(_) | Operation::Inspect(_) => 0,
                        Operation::CoherentReconciliation(_) => 2,
                        _ => 1,
                    };
                    if state.phase_runtime_count != expected {
                        return Err(denied());
                    }
                }
                let (role, fingerprint) = match state.lifecycle.context() {
                    Some(context) => (
                        context.role.clone().ok_or_else(denied)?,
                        context.fingerprint()?,
                    ),
                    None if phase.number() == 1 => (String::new(), String::new()),
                    None => return Err(denied()),
                };
                let console_id = match &evidence {
                    PhaseEvidence::Startup(info) if phase.number() == 1 => info.id.clone(),
                    _ => state.console_id.clone().ok_or_else(denied)?,
                };
                let validated = evidence::validate(
                    state.sequence.plan(),
                    &phase,
                    evidence,
                    &role,
                    &fingerprint,
                    &console_id,
                    state.last_runtime.as_ref(),
                )?;
                if let Some(marker) = validated.ownership() {
                    state.lifecycle.validate_ownership(marker)?;
                }
                let original = validated.record()?;
                // The complete original operation evidence is retained before
                // syncing. No phase cursor changes until flush succeeds.
                state.enqueue(
                    json!({"type":"validatedPhase", "phase":phase.number(), "evidence":original}),
                )?;
                if phase.number() == 1 {
                    state.console_id = Some(console_id);
                }
                Ok(())
            })()
        };
        if let Err(error) = result {
            return Err(self.state.lock().fail(error));
        }
        self.flush().await?;
        {
            let mut state = self.state.lock();
            state.check()?;
            state.sequence.advance_phase()?;
            state.phase_runtime_count = 0;
            state.last_sql = None;
        }
        interrupted.armed = false;
        Ok(())
    }

    /// Final owner cleanup can persist originals after the shared failure latch
    /// has stopped admission. This never clears failure, advances, or retries SQL.
    /// It still returns that retained failure after successful evidence storage.
    pub(super) async fn persist_final_evidence(&self) -> AppResult<()> {
        let _gate = self.gate.lock().await;
        let mut interrupted = self.interrupted();
        let result = self.flush().await;
        interrupted.armed = false;
        result
    }
}

impl TransportPolicy for Policy {
    fn check(&self) -> AppResult<()> {
        self.state.lock().check()
    }
    fn failed(&self, error: AppError) {
        self.state.lock().fail(error);
    }
    fn terminal(&self, terminal: PhysicalTerminal<'_>) -> AppResult<()> {
        let mut state = self.state.lock();
        let original = state.sequence.terminal(terminal);
        // Always retain original validated helper report, including known ACKs,
        // before exposing a policy rejection to the shared failure latch.
        state.enqueue(original.record())?;
        if let Some(error) = original.error() {
            return Err(state.fail(error.clone()));
        }
        if original.report().ok
            && let Some(admission) = original.admission()
            && let AdmissionKind::Sql(entry) = admission.kind()
        {
            if !state.debt.is_empty() {
                return Err(state.fail(denied()));
            }
            state.debt = observations(entry, state.lifecycle.census().map(|c| c.names()))?;
            state.last_sql = Some(entry.clone());
        }
        Ok(())
    }
    fn local_terminal(&self, terminal: LocalTerminal<'_>) -> AppResult<()> {
        let mut state = self.state.lock();
        let original = state.sequence.local_terminal(terminal);
        // Preserve actual assignment and candidate intent separately, including
        // originals after failure. Benign stale controls create no SQL/debt.
        state.enqueue(original.record())?;
        if let Some(error) = original.error() {
            return Err(state.fail(error.clone()));
        }
        Ok(())
    }
    fn admit<'a>(
        &'a self,
        request: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let _gate = self.gate.lock().await;
            let mut interrupted = self.interrupted();
            let reservation = {
                let mut state = self.state.lock();
                (|| {
                    state.check()?;
                    if request.impact != crate::drivers::snowflake_process::Impact::Control
                        && !state.debt.is_empty()
                    {
                        return Err(denied());
                    }
                    let admission = state.sequence.reserve(&request)?;
                    state.enqueue(admission.record())
                })()
            };
            if let Err(error) = reservation {
                return Err(self.state.lock().fail(error));
            }
            self.flush().await?;
            interrupted.armed = false;
            Ok(())
        })
    }
}

impl MetadataObserver for Policy {
    fn check(&self) -> AppResult<()> {
        self.state.lock().check()
    }
    fn observe_schemas(&self, observation: SchemaObservation<'_>) -> AppResult<()> {
        let mut state = self.state.lock();
        let result = (|| {
            state.sequence.validate_context(observation.context)?;
            state.consume(Observation::Schemas(observation.site))?;
            let phase = state.sequence.phase().ok_or_else(denied)?.clone();
            let original = json!({"type":"schemas", "phase":phase.number(),
                "site":format!("{:?}", observation.site), "context":observation.context,
                "wireNames":observation.wire_names, "parsedNames":observation.parsed_names});
            let names = observation.wire_names.to_vec();
            state.lifecycle.observe_schemas(&phase, observation)?;
            state.validate_context_identity()?;
            state.enqueue(original)?;
            if !state.sequence.census_frozen() {
                state.sequence.freeze_census(&names)?;
                let ledger = serde_json::to_value(state.sequence.ledger()).map_err(|_| denied())?;
                state.enqueue(json!({"type":"frozenLedger", "ledger":ledger}))?;
            }
            Ok(())
        })();
        result.map_err(|error| state.fail(error))
    }
    fn observe_relations(&self, observation: RelationObservation<'_>) -> AppResult<()> {
        let mut state = self.state.lock();
        let result = (|| {
            state.sequence.validate_context(observation.context)?;
            state.consume(Observation::Relations(
                observation.site,
                observation.schema.map(str::to_owned),
            ))?;
            let phase = state.sequence.phase().ok_or_else(denied)?.clone();
            let original = json!({"type":"relations", "phase":phase.number(),
                "site":format!("{:?}", observation.site), "context":observation.context,
                "schema":observation.schema, "relations":observation.relations});
            state.lifecycle.observe_relations(&phase, observation)?;
            state.validate_context_identity()?;
            state.enqueue(original)
        })();
        result.map_err(|error| state.fail(error))
    }
    fn observe_runtime(&self, runtime: &SessionRuntimeState) -> AppResult<()> {
        let mut state = self.state.lock();
        state.runtime(runtime).map_err(|error| state.fail(error))
    }
}

fn observations(entry: &SqlEntry, census: Option<&[String]>) -> AppResult<VecDeque<Observation>> {
    use MetadataSite::*;
    let phase = entry.phase();
    let ordinal = entry.phase_ordinal();
    let mut expected = VecDeque::new();
    let schemas = match (phase, ordinal) {
        (2 | 3, 6) => Some(CreateTargetCheck),
        (4 | 10 | 31 | 27, 6) => Some(SessionContextOptions),
        (4 | 10 | 31, 14) | (6 | 11 | 28 | 32, 3) => Some(RelationCatalogBatch),
        _ => None,
    };
    if let Some(site) = schemas {
        expected.push_back(Observation::Schemas(site));
    }
    match (phase, ordinal) {
        (2 | 3, 7) => expected.push_back(Observation::Relations(
            CreateTargetCheck,
            Some(SCHEMA.into()),
        )),
        (8 | 9, 6 | 10) | (29 | 30, 6) | (7 | 12, 3 | 7) => expected.push_back(
            Observation::Relations(ExactRelationCheck, Some(SCHEMA.into())),
        ),
        _ => {}
    }
    if matches!(phase, 4 | 10 | 31 | 6 | 11 | 28 | 32) {
        let census = census.ok_or_else(denied)?;
        let count = u16::try_from(census.len()).map_err(|_| denied())?;
        let offset = if matches!(phase, 4 | 10 | 31) { 14 } else { 3 };
        if let Some(index) = ordinal.checked_sub(offset + 1).filter(|i| *i < count) {
            expected.push_back(Observation::Relations(
                RelationCatalogBatch,
                Some(census[index as usize].clone()),
            ));
        }
        if ordinal == offset + count + 2 {
            expected.push_back(Observation::Relations(RelationCatalogSnapshot, None));
        }
        if matches!(phase, 4 | 10 | 31) && ordinal == count + 22 {
            expected.push_back(Observation::Schemas(SessionContextOptions));
        }
    }
    let runtime = match phase {
        2 => ordinal == 12,
        3 => ordinal == 18,
        8 => ordinal == 15,
        9 => ordinal == 21,
        29 => ordinal == 11,
        30 => ordinal == 17,
        4 | 10 | 31 => ordinal == 11 || ordinal == census.ok_or_else(denied)?.len() as u16 + 27,
        5 | 13..=16 | 18..=26 | 27 => ordinal == 11,
        17 => ordinal == 6,
        _ => false,
    };
    if runtime {
        expected.push_back(Observation::Runtime);
    }
    // A SQL-kind check catches accidental schedule drift when production-derived
    // expansion changes; exact per-phase budgets remain independently tested.
    if !expected.is_empty() && !matches!(entry.kind(), SqlKind::Read(_)) {
        return Err(denied());
    }
    Ok(expected)
}

#[cfg(test)]
#[path = "snowflake_table_qualification_policy_tests.rs"]
mod tests;
