//! Retained console execution owners. Remote SQL is admitted only by the private
//! adapter; this module owns review, local durability, results and task lifetime.
use super::*;
#[cfg(unix)]
use crate::drivers::{
    snowflake_external::console::{
        ReviewedConsole, ReviewedConsoleAckRecorder, ReviewedConsoleAdmission,
    },
    snowflake_process::{Certainty, Operation, ReviewedAcknowledgement, StatementIdentity},
};
#[cfg(unix)]
use crate::persistence::snowflake_console::validate_console_request_identity;
use crate::persistence::snowflake_console::{ConsoleAdmissionRejection, ConsoleJournalOwner};
use std::{
    collections::{HashSet, VecDeque},
    sync::atomic::AtomicU64,
};

const MAX_HOT_RUNS: usize = 32;
// Reserve the complete bounded run and its post-failure snapshot outbox before
// accepting it. This gate is independent of lifetime SQLite count limits.
const RUN_MEMORY_RESERVATION: usize = 32 * 1024 * 1024;
const HOT_MEMORY_BUDGET: usize = 256 * 1024 * 1024;
const MAX_PENDING_REFUSALS: usize = 32;
const DETAIL_CACHE_ENTRIES: usize = 16;
const DETAIL_CACHE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(super) struct State {
    generation: AtomicU64,
    previews: Mutex<HashMap<String, u64>>,
    journal: Mutex<Journal>,
    #[cfg(unix)]
    pub(super) owners: Mutex<HashMap<String, Arc<tokio::sync::Mutex<ReviewedConsole>>>>,
}

#[derive(Default)]
struct Journal {
    owner: Option<ConsoleJournalOwner>,
    runs: HashMap<String, Retained>,
    clients: HashMap<String, String>,
    active: HashSet<String>,
    publishers: HashSet<String>,
    dirty: HashSet<String>,
    result_runs: HashMap<String, (String, usize)>,
    details: DetailCache,
    rejections: HashMap<String, ConsoleAdmissionRejection>,
}

struct Retained {
    run: ConsoleRunSnapshot,
    writes: VecDeque<Write>,
    started_pending: bool,
    history: Option<HistoryEntry>,
    finished_pending: bool,
}

impl Retained {
    fn pending(&self) -> bool {
        !self.writes.is_empty()
            || self.started_pending
            || self.history.is_some()
            || self.finished_pending
    }
}

#[derive(Default)]
struct DetailCache {
    entries: VecDeque<(String, ConsoleRunSnapshot, usize)>,
    bytes: usize,
}

impl DetailCache {
    fn get(&mut self, client: &str) -> Option<ConsoleRunSnapshot> {
        let position = self.entries.iter().position(|(id, _, _)| id == client)?;
        let entry = self.entries.remove(position)?;
        let snapshot = entry.1.clone();
        self.entries.push_back(entry);
        Some(snapshot)
    }

    fn remove(&mut self, client: &str) {
        if let Some(position) = self.entries.iter().position(|(id, _, _)| id == client)
            && let Some((_, _, bytes)) = self.entries.remove(position)
        {
            self.bytes -= bytes;
        }
    }

    fn insert(&mut self, run: ConsoleRunSnapshot) {
        let client = run.metadata.client_request_id.clone();
        self.remove(&client);
        // JSON bounds every dynamic string's occupied bytes; account for the
        // owned structs/step allocation as well. The measuring writer allocates
        // no lifetime-sized serialization buffer.
        struct Size(usize);
        impl std::io::Write for Size {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self.0.saturating_add(bytes.len());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut encoded = Size(0);
        if serde_json::to_writer(&mut encoded, &run).is_err() {
            return;
        }
        let bytes = encoded
            .0
            .saturating_add(std::mem::size_of::<ConsoleRunSnapshot>())
            .saturating_add(run.steps.capacity() * std::mem::size_of::<ConsoleStep>());
        if bytes > DETAIL_CACHE_BYTES {
            return;
        }
        while self.entries.len() >= DETAIL_CACHE_ENTRIES
            || self.bytes.saturating_add(bytes) > DETAIL_CACHE_BYTES
        {
            if let Some((_, _, prior)) = self.entries.pop_front() {
                self.bytes -= prior;
            } else {
                return;
            }
        }
        self.bytes += bytes;
        self.entries.push_back((client, run, bytes));
    }
}

impl Journal {
    fn retain_owner(&mut self, owner: ConsoleJournalOwner) {
        // Unowned reads may predate another writer's late observations. Trust
        // cached originals only within the same uninterrupted lease lifetime.
        self.details = DetailCache::default();
        self.owner = Some(owner);
    }

    fn can_admit(&self) -> bool {
        self.runs.len() < MAX_HOT_RUNS
            && self.runs.len().saturating_add(1) * RUN_MEMORY_RESERVATION <= HOT_MEMORY_BUDGET
            && self.rejections.len() < MAX_PENDING_REFUSALS
    }

    fn result_ids(&self, run: &str) -> Vec<String> {
        let mut results: Vec<_> = self
            .result_runs
            .iter()
            .filter(|(_, (owner, _))| owner.as_str() == run)
            .map(|(id, (_, ordinal))| (*ordinal, id))
            .collect();
        results.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        results.into_iter().map(|(_, id)| id.clone()).collect()
    }

    fn archive_block(&self, client: &str) -> Option<&'static str> {
        if self.rejections.contains_key(client) {
            return Some("This refusal is waiting for local recording");
        }
        let id = self.clients.get(client)?;
        if self.active.contains(id) {
            Some("The console owner is still running")
        } else if self.publishers.contains(id) {
            Some("Console event publication is still finishing")
        } else if self.runs.get(id).is_some_and(Retained::pending) {
            Some("This receipt is waiting for local recording")
        } else {
            None
        }
    }

    fn refine_summary(&self, summary: &mut SnowflakeConsoleReceiptSummary) {
        if let Some(reason) = self.archive_block(&summary.client_request_id) {
            summary.archive_eligible = false;
            summary.archive_block_reason = Some(reason.into());
        }
    }

    fn compact(&mut self) {
        let ready: Vec<_> = self
            .runs
            .iter()
            .filter(|(id, r)| {
                r.run.terminal.is_some()
                    && !r.pending()
                    && !self.active.contains(*id)
                    && !self.publishers.contains(*id)
                    && !self.dirty.contains(*id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ready {
            if let Some(retained) = self.runs.remove(&id) {
                self.clients
                    .remove(&retained.run.metadata.client_request_id);
                self.details.insert(retained.run);
            }
        }
    }
}

struct Write {
    before: u64,
    after: ConsoleRunSnapshot,
    operation: Mutation,
}

enum Mutation {
    Prepare(ConsoleRunMetadata),
    Intent {
        ordinal: u32,
        at: u64,
    },
    Outcome {
        ordinal: u32,
        outcome: ConsoleStepOutcome,
        observations: ConsoleObservations,
    },
    Observe {
        ordinal: Option<u32>,
        observations: ConsoleObservations,
    },
    Finish {
        reason: ConsoleStopReason,
        at: u64,
    },
}

impl State {
    #[cfg(all(test, unix, feature = "test-harness"))]
    pub(super) fn pending_rejection(&self, id: &str) -> Option<ConsoleAdmissionRejection> {
        self.journal.lock().rejections.get(id).cloned()
    }
    #[cfg(all(test, unix, feature = "test-harness"))]
    pub(super) fn publisher_active(&self, id: &str) -> bool {
        self.journal.lock().publishers.contains(id)
    }
    pub(super) fn revoke(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn replace_preview(&self, session: &str) {
        let mut previews = self.previews.lock();
        let epoch = previews.entry(session.into()).or_default();
        *epoch = epoch.wrapping_add(1);
    }
    pub(super) fn forget_session(&self, session: &str) {
        self.previews.lock().remove(session);
    }
    fn preview(&self, session: &str) -> u64 {
        self.previews
            .lock()
            .get(session)
            .copied()
            .unwrap_or_default()
    }
    pub(super) fn owns_result(&self, id: &str) -> bool {
        self.journal.lock().result_runs.contains_key(id)
    }
    pub(super) fn release_result(&self, id: &str) {
        self.journal.lock().result_runs.remove(id);
    }
    pub(super) fn is_run(&self, id: &str) -> bool {
        self.journal.lock().active.contains(id)
    }
    pub(super) fn release_journal(&self) {
        let mut journal = self.journal.lock();
        journal.details = DetailCache::default();
        journal.owner.take();
    }

    /// The caller holds configuration admission closed and has drained owners.
    /// Taking this lease never performs abandoned-intent recovery.
    pub(super) fn take_recovery_owner(&self) -> AppResult<Option<ConsoleJournalOwner>> {
        #[cfg(unix)]
        if !self.owners.lock().is_empty() {
            return Err(recording_error());
        }
        let mut journal = self.journal.lock();
        if !journal.active.is_empty()
            || !journal.publishers.is_empty()
            || !journal.dirty.is_empty()
            || !journal.rejections.is_empty()
            || journal
                .runs
                .values()
                .any(|retained| retained.pending() || retained.run.terminal.is_none())
        {
            return Err(recording_error());
        }
        // No hot terminal snapshot may survive replacement and masquerade as a
        // fresh restored observation. Compaction here changes memory only.
        journal.compact();
        journal.details = DetailCache::default();
        Ok(journal.owner.take())
    }

    /// Recovery retains exclusive ownership until its worker and publication
    /// have settled. It returns that same lease before reopening admission.
    pub(super) fn return_recovery_owner(&self, owner: Option<ConsoleJournalOwner>) {
        if let Some(owner) = owner {
            self.journal.lock().retain_owner(owner);
        }
    }
}

fn recording_error() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "A reviewed console outcome is waiting for local recording. Keep Joinery open and retry recording before another run.",
    )
}

fn absorb(target: &mut ConsoleObservations, patch: &ConsoleObservations) {
    target.delivered |= patch.delivered;
    target.delivery_failed |= patch.delivery_failed;
    target.cancellation_observed |= patch.cancellation_observed;
    target.retired |= patch.retired;
    target.truncated |= patch.truncated;
    target.recording_failed |= patch.recording_failed;
    if target.first_error.is_none() {
        target.first_error = patch.first_error;
    }
}

fn counts(run: &ConsoleRunSnapshot) -> ConsoleAccounting {
    let mut c = ConsoleAccounting {
        planned: run.metadata.step_count,
        intended: run.steps.len() as u32,
        acknowledged: 0,
        rejected: 0,
        unknown: 0,
        not_dispatched: 0,
        not_intended: run.metadata.step_count - run.steps.len() as u32,
        not_attempted: 0,
    };
    for step in &run.steps {
        match &step.outcome {
            Some(ConsoleStepOutcome::Acknowledged(_)) => c.acknowledged += 1,
            Some(ConsoleStepOutcome::Rejected(_)) => c.rejected += 1,
            Some(ConsoleStepOutcome::Unknown(_)) => c.unknown += 1,
            Some(ConsoleStepOutcome::NotDispatched(_)) => c.not_dispatched += 1,
            None => {}
        }
    }
    c.not_attempted = c.not_intended + c.not_dispatched;
    c
}

fn stop_reason(run: &ConsoleRunSnapshot) -> ConsoleStopReason {
    let c = counts(run);
    let o = &run.observations;
    if c.unknown > 0 { ConsoleStopReason::OutcomeUnknown }
    else if o.recording_failed { ConsoleStopReason::RecordingFailed }
    else if c.rejected > 0 || run.steps.iter().any(|s| matches!(&s.outcome,
        Some(ConsoleStepOutcome::NotDispatched(n)) if n.reason != ConsoleNotDispatchedReason::Cancelled))
        || o.delivery_failed || o.first_error.is_some_and(|e| !matches!(e, ConsoleErrorCode::Cancelled | ConsoleErrorCode::ResultLimit)) {
        ConsoleStopReason::Failed
    } else if o.truncated { ConsoleStopReason::ResultLimit }
    else if o.cancellation_observed { ConsoleStopReason::Cancelled }
    else if o.retired || o.first_error.is_some() { ConsoleStopReason::Failed }
    else { ConsoleStopReason::Completed }
}

fn status(run: &ConsoleRunSnapshot) -> RunStatus {
    match run.terminal.as_ref().map(|t| t.reason) {
        Some(ConsoleStopReason::Completed) => RunStatus::Completed,
        Some(ConsoleStopReason::Cancelled) => RunStatus::Cancelled,
        Some(ConsoleStopReason::OutcomeUnknown | ConsoleStopReason::Interrupted) => {
            RunStatus::OutcomeUnknown
        }
        Some(ConsoleStopReason::ResultLimit) => RunStatus::Stopped,
        _ => RunStatus::Failed,
    }
}

impl Mutation {
    fn apply(&self, run: &mut ConsoleRunSnapshot) {
        match self {
            Self::Prepare(_) => return,
            Self::Intent { ordinal, at } => {
                run.state = ConsoleRunState::Running;
                run.steps.push(ConsoleStep {
                    ordinal: *ordinal,
                    intended_at: *at,
                    outcome: None,
                    observations: ConsoleObservations::default(),
                });
            }
            Self::Outcome {
                ordinal,
                outcome,
                observations,
            } => {
                let step = &mut run.steps[*ordinal as usize];
                step.outcome = Some(outcome.clone());
                absorb(&mut step.observations, observations);
                absorb(&mut run.observations, observations);
            }
            Self::Observe {
                ordinal,
                observations,
            } => {
                if let Some(i) = ordinal {
                    absorb(&mut run.steps[*i as usize].observations, observations);
                }
                absorb(&mut run.observations, observations);
            }
            Self::Finish { reason, at } => {
                run.terminal = Some(ConsoleTerminal {
                    reason: *reason,
                    finished_at: *at,
                    accounting: counts(run),
                    observations: run.observations.clone(),
                });
                run.state = ConsoleRunState::Terminal;
            }
        }
        run.revision += 1;
    }

    fn persist(
        &self,
        store: &Persistence,
        owner: &ConsoleJournalOwner,
        id: &str,
        revision: u64,
    ) -> AppResult<ConsoleRunSnapshot> {
        match self {
            Self::Prepare(metadata) => Ok(store.console_prepare_run(owner, metadata.clone())?.run),
            Self::Intent { ordinal, at } => {
                store.console_step_intent(owner, id, revision, *ordinal, *at)
            }
            Self::Outcome {
                ordinal,
                outcome,
                observations,
            } => store.console_step_outcome(
                owner,
                id,
                revision,
                *ordinal,
                outcome.clone(),
                observations.clone(),
            ),
            Self::Observe {
                ordinal,
                observations,
            } => store.console_observe(owner, id, revision, *ordinal, observations.clone()),
            Self::Finish { reason, at } => store.console_finish(owner, id, revision, *reason, *at),
        }
    }
}

impl Studio {
    /// Local SQLite only. The outbox is inserted before the first write and is
    /// never discarded on failure, event detachment or session removal.
    pub(super) fn flush_console_recordings(&self) -> AppResult<()> {
        let mut journal = self.console.journal.lock();
        if journal.dirty.is_empty() && journal.rejections.is_empty() {
            journal.compact();
            return Ok(());
        }
        let Journal {
            owner,
            runs,
            rejections,
            dirty,
            ..
        } = &mut *journal;
        let owner = owner.as_ref().ok_or_else(recording_error)?;
        let pending: Vec<_> = rejections.keys().cloned().collect();
        for id in pending {
            let receipt = &rejections[&id];
            // A commit error is uncertain. Only the exact immutable readback
            // establishes a durable refusal; otherwise retain the frozen outbox.
            if self
                .persistence
                .console_record_rejection(owner, receipt)
                .is_err()
                && self
                    .persistence
                    .console_rejection_by_client_request(&id)?
                    .as_ref()
                    != Some(receipt)
            {
                return Err(recording_error());
            }
            rejections.remove(&id);
        }
        let pending: Vec<_> = dirty.iter().cloned().collect();
        for id in pending {
            let retained = runs.get_mut(&id).ok_or_else(recording_error)?;
            while let Some(write) = retained.writes.front() {
                let durable = self
                    .persistence
                    .console_run(&retained.run.metadata.run_id)?;
                // Reconcile an uncertain local commit against the complete expected
                // snapshot, including immutable original acknowledgement evidence.
                if durable.as_ref() != Some(&write.after) {
                    match (&write.operation, &durable) {
                        (Mutation::Prepare(_), None) => {}
                        (Mutation::Prepare(_), Some(_)) => return Err(recording_error()),
                        (_, Some(run)) if run.revision == write.before => {}
                        _ => return Err(recording_error()),
                    }
                    let actual = write.operation.persist(
                        &self.persistence,
                        owner,
                        &retained.run.metadata.run_id,
                        write.before,
                    )?;
                    if actual != write.after {
                        return Err(recording_error());
                    }
                }
                retained.writes.pop_front();
            }
            let m = &retained.run.metadata;
            if retained.started_pending {
                self.persistence.record_audit_once(&AuditEntry {
                    id: format!("{}:started", m.run_id),
                    occurred_at: m.created_at,
                    connection_id: m.connection_id.clone(),
                    action: "snowflake-console".into(),
                    digest: m.plan_digest.clone(),
                    outcome: "started".into(),
                })?;
                retained.started_pending = false;
            }
            if let Some(history) = &retained.history {
                self.persistence.record_history_once(history)?;
                retained.history = None;
            }
            if retained.finished_pending {
                self.persistence.record_audit_once(&AuditEntry {
                    id: format!("{}:finished", m.run_id),
                    occurred_at: retained
                        .run
                        .terminal
                        .as_ref()
                        .ok_or_else(recording_error)?
                        .finished_at,
                    connection_id: m.connection_id.clone(),
                    action: "snowflake-console".into(),
                    digest: m.plan_digest.clone(),
                    outcome: format!("{:?}", status(&retained.run)),
                })?;
                retained.finished_pending = false;
            }
            dirty.remove(&id);
        }
        journal.compact();
        Ok(())
    }

    fn console_stage(&self, id: &str, operation: Mutation) -> AppResult<()> {
        {
            let mut journal = self.console.journal.lock();
            let retained = journal.runs.get_mut(id).ok_or_else(recording_error)?;
            let before = retained.run.revision;
            operation.apply(&mut retained.run);
            retained.writes.push_back(Write {
                before,
                after: retained.run.clone(),
                operation,
            });
            journal.dirty.insert(id.into());
        }
        self.flush_console_recordings()
    }

    fn console_snapshot(&self, id: &str) -> AppResult<ConsoleRunSnapshot> {
        self.console
            .journal
            .lock()
            .runs
            .get(id)
            .map(|r| r.run.clone())
            .ok_or_else(recording_error)
    }

    /// Read a durable admission decision without reconnecting or submitting SQL.
    /// Pending local refusal writes never masquerade as committed proof.
    pub fn snowflake_console_request(
        &self,
        client_request_id: &str,
    ) -> AppResult<Option<SnowflakeConsoleRequestLookup>> {
        let _configuration = self.configuration_mutation_guard()?;
        self.snowflake_console_request_inner(client_request_id)
    }

    fn snowflake_console_request_inner(
        &self,
        client_request_id: &str,
    ) -> AppResult<Option<SnowflakeConsoleRequestLookup>> {
        if uuid::Uuid::parse_str(client_request_id)
            .ok()
            .is_none_or(|id| id.to_string() != client_request_id)
        {
            return Err(AppError::invalid("Invalid console request identifier"));
        }
        if let Some(view) = self.snowflake_console_run_inner(client_request_id)? {
            return Ok(Some(SnowflakeConsoleRequestLookup::Started { view }));
        }
        if self
            .console
            .journal
            .lock()
            .rejections
            .contains_key(client_request_id)
        {
            return Err(recording_error());
        }
        Ok(self
            .persistence
            .console_rejection_by_client_request(client_request_id)?
            .map(refusal_view))
    }

    /// This lookup performs no helper request, reconnect, metadata query or SQL
    /// retrieval. Incomplete receipts can be sealed locally after exclusive journal
    /// ownership succeeds; completed receipt retrieval needs no lease.
    pub fn snowflake_console_run(
        &self,
        client_request_id: &str,
    ) -> AppResult<Option<SnowflakeConsoleRunView>> {
        let _configuration = self.configuration_mutation_guard()?;
        self.snowflake_console_run_inner(client_request_id)
    }

    fn snowflake_console_run_inner(
        &self,
        client_request_id: &str,
    ) -> AppResult<Option<SnowflakeConsoleRunView>> {
        if uuid::Uuid::parse_str(client_request_id)
            .ok()
            .is_none_or(|id| id.to_string() != client_request_id)
        {
            return Err(AppError::invalid("Invalid console request identifier"));
        }
        let _ = self.flush_console_recordings();
        let mut journal = self.console.journal.lock();
        if let Some(r) = journal
            .clients
            .get(client_request_id)
            .and_then(|id| journal.runs.get(id))
        {
            return Ok(Some(SnowflakeConsoleRunView {
                run: r.run.clone(),
                results: journal
                    .result_ids(&r.run.metadata.run_id)
                    .iter()
                    .filter_map(|id| self.results.info(id).ok())
                    .collect(),
                pending_recording: r.pending(),
            }));
        }
        // A held lease excludes another writer. Once released (including a
        // closed coordinator), read SQLite again instead of serving stale cache.
        if journal.owner.is_some()
            && let Some(run) = journal.details.get(client_request_id)
        {
            return Ok(Some(SnowflakeConsoleRunView {
                results: journal
                    .result_ids(&run.metadata.run_id)
                    .iter()
                    .filter_map(|id| self.results.info(id).ok())
                    .collect(),
                run,
                pending_recording: false,
            }));
        }
        drop(journal);
        let mut run = self
            .persistence
            .console_run_by_client_request(client_request_id)?;
        if run.as_ref().is_some_and(|r| r.terminal.is_none()) {
            let mut journal = self.console.journal.lock();
            // A fresh coordinator can seal abandoned intents only after taking
            // the exclusive OS lease. A live other owner leaves evidence unsealed.
            // Check shutdown under this mutex so lease release cannot be followed
            // by a closed coordinator reacquiring ownership through lookup.
            if !self.dml_tasks.is_closed()
                && journal.owner.is_none()
                && !self
                    .jobs
                    .lock()
                    .keys()
                    .any(|id| journal.runs.contains_key(id))
                && let Ok(owner) = self.persistence.claim_console_journal_owner()
            {
                journal.retain_owner(owner);
                run = self
                    .persistence
                    .console_run_by_client_request(client_request_id)?;
            }
        }
        Ok(run.map(|run| {
            let mut journal = self.console.journal.lock();
            let results = journal
                .result_ids(&run.metadata.run_id)
                .iter()
                .filter_map(|id| self.results.info(id).ok())
                .collect();
            if run.terminal.is_some() {
                journal.details.insert(run.clone());
            }
            SnowflakeConsoleRunView {
                pending_recording: run.terminal.is_none(),
                run,
                results,
            }
        }))
    }

    /// Local paged metadata only. No console admission, helper request or auth.
    pub fn snowflake_console_receipts(
        &self,
        request: &SnowflakeConsoleReceiptPageRequest,
    ) -> AppResult<SnowflakeConsoleReceiptPage> {
        let mut page = self.persistence.console_receipts(request)?;
        let journal = self.console.journal.lock();
        for summary in &mut page.receipts {
            journal.refine_summary(summary);
        }
        Ok(page)
    }

    pub fn snowflake_console_receipt_summary(
        &self,
        client_request_id: &str,
    ) -> AppResult<Option<SnowflakeConsoleReceiptSummary>> {
        let mut summary = self
            .persistence
            .console_receipt_summary(client_request_id)?;
        if let Some(summary) = &mut summary {
            self.console.journal.lock().refine_summary(summary);
        }
        Ok(summary)
    }

    pub fn snowflake_console_pending(&self) -> AppResult<SnowflakeConsolePending> {
        let _configuration = self.configuration_mutation_guard()?;
        let _ = self.flush_console_recordings();
        let journal = self.console.journal.lock();
        let mut clients: HashSet<_> = journal.rejections.keys().cloned().collect();
        for (id, retained) in &journal.runs {
            if retained.pending() || journal.active.contains(id) || journal.publishers.contains(id)
            {
                clients.insert(retained.run.metadata.client_request_id.clone());
            }
        }
        let mut client_request_ids: Vec<_> = clients.into_iter().collect();
        client_request_ids.sort();
        Ok(SnowflakeConsolePending {
            client_request_ids,
            recording_hold: !journal.rejections.is_empty() || !journal.dirty.is_empty(),
        })
    }

    pub fn snowflake_console_storage(&self) -> AppResult<SnowflakeConsoleStorage> {
        self.persistence.console_storage()
    }

    pub fn set_snowflake_console_receipt_archive(
        &self,
        request: &SnowflakeConsoleReceiptArchiveRequest,
    ) -> AppResult<SnowflakeConsoleReceiptArchiveResult> {
        let _configuration = self.configuration_mutation_guard()?;
        if self.dml_tasks.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "The app is closing",
            ));
        }
        let _ = self.flush_console_recordings();
        let journal = self.console.journal.lock();
        for item in &request.items {
            if let Some(reason) = journal.archive_block(&item.client_request_id) {
                return Err(AppError::new(ErrorCode::Conflict, reason));
            }
        }
        // Archive is browsing only. A temporary raw lease must neither seal
        // unrelated originals nor count as normal recovery for a later start.
        let temporary_owner = if journal.owner.is_none() {
            Some(self.persistence.claim_console_archive_owner()?)
        } else {
            None
        };
        self.persistence.console_archive_receipts(
            journal
                .owner
                .as_ref()
                .or(temporary_owner.as_ref())
                .ok_or_else(recording_error)?,
            request,
            now_ms(),
        )
    }

    /// Explicit row-buffer disposal is independent of archive and original proof.
    pub fn release_snowflake_console_results(&self, client_request_id: &str) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        self.flush_console_recordings()?;
        let journal = self.console.journal.lock();
        if let Some(reason) = journal.archive_block(client_request_id) {
            return Err(AppError::new(ErrorCode::Conflict, reason));
        }
        let run = self
            .persistence
            .console_run_by_client_request(client_request_id)?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Console run not found"))?;
        if run.terminal.is_none() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The console run has not settled",
            ));
        }
        let result_ids = journal.result_ids(&run.metadata.run_id);
        for id in &result_ids {
            if let Ok(result) = self.results.info(id)
                && result.run_id != run.metadata.run_id
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Console result ownership changed",
                ));
            }
        }
        // Studio::release_result invokes the State registry hook. Do not recurse
        // into that mutex while holding the journal guard.
        drop(journal);
        for id in result_ids {
            self.release_result(&id);
        }
        Ok(())
    }
}

#[cfg(all(test, unix, feature = "test-harness"))]
#[path = "snowflake_console_retention_tests.rs"]
mod retention_tests;

fn refusal_view(receipt: ConsoleAdmissionRejection) -> SnowflakeConsoleRequestLookup {
    SnowflakeConsoleRequestLookup::NotAdmitted {
        client_request_id: receipt.client_request_id,
        session_id: receipt.session_id,
        plan_digest: receipt.plan_digest,
        code: receipt.code,
    }
}

#[cfg(unix)]
fn original_decision(
    decision: SnowflakeConsoleRequestLookup,
    request: &SnowflakeConsoleStart,
) -> AppResult<SnowflakeConsoleStartResult> {
    let result = match decision {
        SnowflakeConsoleRequestLookup::Started { view }
            if view.run.metadata.session_id == request.session_id
                && view.run.metadata.plan_digest == request.plan_digest =>
        {
            Some(SnowflakeConsoleStartResult::Started {
                run_id: view.run.metadata.run_id,
            })
        }
        SnowflakeConsoleRequestLookup::NotAdmitted {
            client_request_id,
            session_id,
            plan_digest,
            code,
        } if client_request_id == request.client_request_id
            && session_id == request.session_id
            && plan_digest == request.plan_digest =>
        {
            Some(SnowflakeConsoleStartResult::NotAdmitted {
                client_request_id,
                session_id,
                plan_digest,
                code,
            })
        }
        _ => None,
    };
    result.ok_or_else(|| {
        AppError::new(
            ErrorCode::Conflict,
            "This console request identifier belongs to a different review",
        )
    })
}

#[cfg(unix)]
fn writable(profile: &ConnectionProfile) -> AppResult<()> {
    crate::drivers::snowflake::Snowflake::validate_profile(profile)?;
    if !snowflake_dml::sdk_profile(profile)
        || !profile
            .snowflake
            .as_ref()
            .is_some_and(|c| c.reviewed_console)
        || safety::effective_mode(profile) == SafeMode::ReadOnly
    {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "Enable the reviewed console on a writable Snowflake external-browser profile first",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn compile(
    request: &SnowflakeConsoleRequest,
) -> AppResult<safety::snowflake_script::CompiledScript> {
    use safety::snowflake_script::Mode;
    safety::snowflake_script::compile(
        &request.sql,
        match request.mode {
            SnowflakeConsoleMode::ParsedStatements => Mode::ParsedStatements,
            SnowflakeConsoleMode::SelectedUnit => Mode::SelectedUnit,
        },
        request.parameters.as_deref(),
    )
}

#[cfg(unix)]
fn review(
    session: &Session,
    profile: &ConnectionProfile,
    compiled: &safety::snowflake_script::CompiledScript,
) -> AppResult<PreparedPlan> {
    writable(profile)?;
    let mut plan = safety::plan_for_statements(
        &session.info.id,
        profile,
        compiled
            .units()
            .iter()
            .map(|u| StatementPlan {
                sql: u.original_sql().into(),
                kind: StatementKind::Unknown,
                missing_where: false,
                span: Some(u.span()),
            })
            .collect(),
    )?;
    // Exact reviewed SQL is potentially mutating regardless of local syntax or
    // Silent mode. This is a distinct whole-script opt-in, never an eligibility proof.
    plan.requires_approval = true;
    if matches!(
        safety::effective_mode(profile),
        SafeMode::Guard | SafeMode::PasswordGated
    ) {
        plan.confirmation_text = Some(profile.name.clone());
    }
    plan.bound_parameters = compiled.inputs().to_vec();
    plan.binding_digest = Some(hex::encode(Sha256::digest(
        serde_json::to_vec(
            &compiled
                .units()
                .iter()
                .map(|u| u.bound())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| AppError::internal("Could not fingerprint reviewed bindings"))?,
    )));
    plan.warnings.push(format!("Each unit has a {} second execution/result-delivery deadline from admission. Original acknowledgement recording and bounded control/terminal settlement can finish later. Expiry retires the session; it does not prove cancellation, rollback or committed effects.", profile.snowflake.as_ref().unwrap().effective_console_timeout_seconds()?));
    plan.warnings.push("These exact ordered units run in the retained Snowflake session. Statements can change session settings, invoke procedures or commit implicitly. An acknowledgement does not prove committed effects or complete results. No automatic retry, observer, COMMIT or ROLLBACK is issued.".into());
    Ok(plan)
}

#[cfg(unix)]
impl Studio {
    pub fn prepare_snowflake_console(
        &self,
        session_id: &str,
        request: &SnowflakeConsoleRequest,
    ) -> AppResult<SnowflakeConsolePlan> {
        let _configuration = self.configuration_mutation_guard()?;
        let session = self.session(session_id)?;
        if !session.info.console_mode || !self.console.owners.lock().contains_key(session_id) {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Connect a reviewed Snowflake console first",
            ));
        }
        if self
            .jobs
            .lock()
            .values()
            .any(|j| j.session_id == session_id)
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The reviewed console is busy",
            ));
        }
        self.flush_pending_snowflake_receipts()?;
        self.flush_console_recordings()?;
        let profile = self.persistence.profile(&session.profile.id)?;
        if serde_json::to_value(&profile).ok() != serde_json::to_value(&session.profile).ok() {
            return Err(expired());
        }
        let timeout_seconds = profile
            .snowflake
            .as_ref()
            .ok_or_else(expired)?
            .effective_console_timeout_seconds()?;
        let compiled = compile(request)?;
        let plan = review(&session, &profile, &compiled)?;
        let validations = compiled
            .units()
            .iter()
            .map(|u| match u.validation() {
                safety::snowflake_script::Validation::LocalSyntax => {
                    SnowflakeConsoleValidation::LocalSyntax
                }
                safety::snowflake_script::Validation::ServerAtExecution => {
                    SnowflakeConsoleValidation::ServerAtExecution
                }
            })
            .collect();
        let bytes = serde_json::to_vec(request)
            .map_err(|_| AppError::invalid("Invalid console request"))?
            .len()
            + serde_json::to_vec(&plan)
                .map_err(|_| AppError::invalid("Invalid console review"))?
                .len();
        let plan = self.store_plan(
            plan,
            Action::SnowflakeConsole {
                request: request.clone(),
                compiled,
                mode: safety::effective_mode(&profile),
                timeout_seconds,
            },
            bytes,
        )?;
        let digest = self
            .plans
            .lock()
            .get(&plan.id)
            .ok_or_else(expired)?
            .digest
            .clone();
        Ok(SnowflakeConsolePlan {
            review: plan,
            digest,
            mode: request.mode,
            timeout_seconds,
            validations,
        })
    }

    pub fn start_snowflake_console(
        self: &Arc<Self>,
        request: &SnowflakeConsoleStart,
        events: EventHandler,
    ) -> AppResult<SnowflakeConsoleStartResult> {
        let _configuration = self.configuration_mutation_guard()?;
        let recovery_activity = self.retain_recovery_activity()?;
        validate_console_request_identity(
            &request.client_request_id,
            &request.session_id,
            &request.plan_digest,
        )?;
        // Stable identity returns its original positive or negative decision even
        // after policy changes, plan consumption, session removal or shutdown.
        if let Some(existing) = self.snowflake_console_request_inner(&request.client_request_id)? {
            return original_decision(existing, request);
        }
        // Shutdown releases the lease only after recording and cleanup finish.
        // Keep the closed coordinator readable, but never let a fresh request
        // reacquire that lease and exclude its replacement.
        if self.dml_tasks.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "The app is closing; this console cannot admit another request",
            ));
        }
        if !self.console.journal.lock().can_admit() {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Console recording memory is reserved by active or pending work; reconcile it before starting another request",
            ));
        }
        // A definite negative requires proving absence while holding the OS
        // journal lease. A failed lookup/lease can hide another owner's original;
        // propagate that uncertainty instead of manufacturing NotAdmitted.
        {
            let mut journal = self.console.journal.lock();
            if journal.owner.is_none() {
                journal.retain_owner(self.persistence.claim_console_journal_owner()?);
            }
        }
        if let Some(existing) = self.snowflake_console_request_inner(&request.client_request_id)? {
            return original_decision(existing, request);
        }
        let admission = (|| -> AppResult<String> {
            self.flush_pending_snowflake_receipts()?;
            self.flush_console_recordings()?;
            let session = self.session(&request.session_id)?;
            let owner = self
                .console
                .owners
                .lock()
                .get(&request.session_id)
                .cloned()
                .ok_or_else(|| {
                    AppError::new(
                        ErrorCode::PolicyBlocked,
                        "This session is not a retained reviewed console",
                    )
                })?;
            let busy = {
                let jobs = self.jobs.lock();
                jobs.len() >= 32 || jobs.values().any(|j| j.session_id == request.session_id)
            };
            if busy {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "The reviewed console is busy",
                ));
            }
            let mut plans = self.plans.lock();
            let stored = plans.get(&request.plan_id).ok_or_else(expired)?;
            if stored.deadline <= Instant::now()
                || stored.plan.session_id != request.session_id
                || stored.digest != request.plan_digest
            {
                return Err(expired());
            }
            let Action::SnowflakeConsole {
                request: original,
                compiled,
                mode,
                timeout_seconds,
            } = &stored.action
            else {
                return Err(AppError::invalid("This is not a reviewed console plan"));
            };
            let profile = self.persistence.profile(&session.profile.id)?;
            if serde_json::to_value(&profile).ok() != serde_json::to_value(&session.profile).ok()
                || *timeout_seconds
                    != profile
                        .snowflake
                        .as_ref()
                        .ok_or_else(expired)?
                        .effective_console_timeout_seconds()?
            {
                return Err(expired());
            }
            let canonical = compile(original)?;
            compiled.verify(
                &canonical
                    .units()
                    .iter()
                    .map(|u| u.bound().clone())
                    .collect::<Vec<_>>(),
            )?;
            let current = review(&session, &profile, &canonical)?;
            if *mode != safety::effective_mode(&profile)
                || current.requires_passphrase != stored.plan.requires_passphrase
                || current.confirmation_text != stored.plan.confirmation_text
                || current.binding_digest != stored.plan.binding_digest
            {
                return Err(expired());
            }
            if stored.approval.is_none() || stored.approval != request.approval {
                return Err(AppError::new(
                    ErrorCode::ApprovalRequired,
                    "Authorize this exact whole-script review first",
                ));
            }
            let metadata = ConsoleRunMetadata {
                run_id: new_id(),
                client_request_id: request.client_request_id.clone(),
                session_id: request.session_id.clone(),
                connection_id: profile.id.clone(),
                plan_digest: stored.digest.clone(),
                step_count: compiled.units().len() as u32,
                created_at: now_ms(),
            };
            let run_id = metadata.run_id.clone();
            {
                let mut journal = self.console.journal.lock();
                // The configuration gate serializes admissions. Owner completion
                // can only release reservations while this review is checked.
                if !journal.can_admit() {
                    return Err(recording_error());
                }
                let run = ConsoleRunSnapshot {
                    metadata: metadata.clone(),
                    revision: 0,
                    state: ConsoleRunState::Prepared,
                    observations: ConsoleObservations::default(),
                    steps: Vec::new(),
                    terminal: None,
                };
                let mut writes = VecDeque::new();
                writes.push_back(Write {
                    before: 0,
                    after: run.clone(),
                    operation: Mutation::Prepare(metadata),
                });
                journal.runs.insert(
                    run.metadata.run_id.clone(),
                    Retained {
                        run,
                        writes,
                        started_pending: true,
                        history: None,
                        finished_pending: false,
                    },
                );
                journal
                    .clients
                    .insert(request.client_request_id.clone(), run_id.clone());
                journal.active.insert(run_id.clone());
                journal.publishers.insert(run_id.clone());
                journal.dirty.insert(run_id.clone());
            }
            let stored = plans.remove(&request.plan_id).unwrap();
            drop(plans);
            let token = CancellationToken::new();
            self.jobs.lock().insert(
                run_id.clone(),
                Job {
                    session_id: request.session_id.clone(),
                    token: token.clone(),
                    cancellation_sent: Arc::new(AtomicBool::new(false)),
                },
            );
            let generation = (
                self.console.generation.load(Ordering::Acquire),
                self.console.preview(&request.session_id),
            );
            let studio = self.clone();
            let task_id = run_id.clone();
            self.dml_tasks.spawn(async move {
                let _recovery_activity = recovery_activity;
                use futures_util::FutureExt;
                // This guard outlives normal Finished publication AND the panic
                // fallback. Its drop is the final publisher/owner boundary.
                let completion = OwnerCompletion {
                    studio: studio.clone(),
                    run_id: task_id.clone(),
                };
                let fallback = studio.clone();
                let fallback_session = session.clone();
                let fallback_id = task_id.clone();
                let fallback_events = events.clone();
                if std::panic::AssertUnwindSafe(studio.console_owned(RunOwner {
                    session,
                    console: owner,
                    stored,
                    run_id: task_id,
                    token,
                    generation,
                    events,
                }))
                .catch_unwind()
                .await
                .is_err()
                {
                    fallback_session.closed.store(true, Ordering::Release);
                    fallback_session.driver.close();
                    fallback.console_owner_interrupted(
                        &fallback_id,
                        &fallback_session,
                        &fallback_events,
                    );
                }
                drop(completion);
            });
            Ok(run_id)
        })();
        match admission {
            Ok(run_id) => Ok(SnowflakeConsoleStartResult::Started { run_id }),
            Err(error) => {
                // Once a positive owner exists its outcome can only be Started,
                // including failures before the first dispatch or local commit.
                if let Some(existing) =
                    self.snowflake_console_run_inner(&request.client_request_id)?
                {
                    return original_decision(
                        SnowflakeConsoleRequestLookup::Started { view: existing },
                        request,
                    );
                }
                let receipt = ConsoleAdmissionRejection {
                    client_request_id: request.client_request_id.clone(),
                    session_id: request.session_id.clone(),
                    plan_digest: request.plan_digest.clone(),
                    code: error.code,
                    recorded_at: now_ms(),
                };
                self.console
                    .journal
                    .lock()
                    .rejections
                    .insert(receipt.client_request_id.clone(), receipt);
                // Ignore unrelated recording errors only if this exact frozen
                // refusal has independently become durable. Lookup enforces it.
                let _ = self.flush_console_recordings();
                let decision = self
                    .snowflake_console_request_inner(&request.client_request_id)?
                    .ok_or_else(recording_error)?;
                original_decision(decision, request)
            }
        }
    }
}

#[cfg(not(unix))]
impl Studio {
    pub fn prepare_snowflake_console(
        &self,
        _: &str,
        _: &SnowflakeConsoleRequest,
    ) -> AppResult<SnowflakeConsolePlan> {
        Err(snowflake_runtime_unavailable())
    }
    pub fn start_snowflake_console(
        self: &Arc<Self>,
        _: &SnowflakeConsoleStart,
        _: EventHandler,
    ) -> AppResult<SnowflakeConsoleStartResult> {
        Err(snowflake_runtime_unavailable())
    }
}

#[cfg(unix)]
fn local_error_code(error: &AppError) -> ConsoleErrorCode {
    match error.code {
        ErrorCode::Cancelled => ConsoleErrorCode::Cancelled,
        ErrorCode::ResultLimitReached => ConsoleErrorCode::ResultLimit,
        ErrorCode::OutcomeUnknown => ConsoleErrorCode::OutcomeUnknown,
        ErrorCode::ConnectionClosed => ConsoleErrorCode::ConnectionClosed,
        ErrorCode::Database => ConsoleErrorCode::Database,
        ErrorCode::Unsupported => ConsoleErrorCode::Unsupported,
        _ => ConsoleErrorCode::InvalidInput,
    }
}

#[cfg(unix)]
fn original_statement(s: &StatementIdentity) -> ConsoleStatement {
    ConsoleStatement {
        sdk_request_id: s.sdk_request_id.clone(),
        query_id: s.query_id.clone(),
    }
}

#[cfg(unix)]
fn original_ack(a: &ReviewedAcknowledgement) -> ConsoleAcknowledgement {
    ConsoleAcknowledgement {
        statement: original_statement(&a.statement),
        context: ConsoleContext {
            database: a.context.database.clone(),
            schema: a.context.schema.clone(),
            role: a.context.role.clone(),
            warehouse: a.context.warehouse.clone(),
        },
    }
}

#[cfg(unix)]
struct Recorder<'a> {
    studio: &'a Studio,
    run: &'a str,
    ordinal: u32,
}

#[cfg(unix)]
#[async_trait::async_trait]
impl ReviewedConsoleAckRecorder for Recorder<'_> {
    async fn record_ack(&mut self, ack: &ReviewedAcknowledgement) -> AppResult<()> {
        self.studio.console_stage(
            self.run,
            Mutation::Outcome {
                ordinal: self.ordinal,
                outcome: ConsoleStepOutcome::Acknowledged(original_ack(ack)),
                observations: ConsoleObservations::default(),
            },
        )
    }
}

#[cfg(unix)]
struct Admission<'a> {
    studio: &'a Studio,
    session: &'a Session,
    run: &'a str,
    ordinal: u32,
    generation: (u64, u64),
    token: &'a CancellationToken,
}

#[cfg(unix)]
impl ReviewedConsoleAdmission for Admission<'_> {
    fn admit(
        &mut self,
        dispatch: &mut dyn FnMut() -> AppResult<Operation>,
    ) -> AppResult<Operation> {
        let _configuration = self.studio.configuration_mutation_guard()?;
        if self.token.is_cancelled() {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "Reviewed console cancelled before submission",
            ));
        }
        if self.studio.console.generation.load(Ordering::Acquire) != self.generation.0
            || self.studio.console.preview(&self.session.info.id) != self.generation.1
        {
            return Err(expired());
        }
        if self.session.closed.load(Ordering::Acquire) || self.session.driver.is_closed() {
            return Err(connection_cancelled());
        }
        let profile = self.studio.persistence.profile(&self.session.profile.id)?;
        writable(&profile)?;
        if serde_json::to_value(&profile).ok() != serde_json::to_value(&self.session.profile).ok() {
            return Err(expired());
        }
        self.studio.flush_pending_snowflake_receipts()?;
        self.studio.flush_console_recordings()?;
        // Durable intent and synchronous request admission share the same
        // configuration gate. No callback, await or fallible post-submit work.
        self.studio.console_stage(
            self.run,
            Mutation::Intent {
                ordinal: self.ordinal,
                at: now_ms(),
            },
        )?;
        dispatch()
    }
}

#[cfg(unix)]
struct OwnerCompletion {
    studio: Arc<Studio>,
    run_id: String,
}

#[cfg(unix)]
impl Drop for OwnerCompletion {
    fn drop(&mut self) {
        {
            let mut journal = self.studio.console.journal.lock();
            journal.active.remove(&self.run_id);
            journal.publishers.remove(&self.run_id);
        }
        // Failure retains dirty full snapshots and the journal lease. Nothing
        // pending is placed in the evictable settled-detail cache.
        let _ = self.studio.flush_console_recordings();
    }
}

#[cfg(unix)]
struct RunOwner {
    session: Arc<Session>,
    console: Arc<tokio::sync::Mutex<ReviewedConsole>>,
    stored: StoredPlan,
    run_id: String,
    token: CancellationToken,
    generation: (u64, u64),
    events: EventHandler,
}

#[cfg(unix)]
impl Studio {
    fn console_owner_interrupted(&self, id: &str, session: &Session, events: &EventHandler) {
        let Ok(original) = self.console_snapshot(id) else {
            return;
        };
        if original.terminal.is_none()
            && let Some(step) = original.steps.last().filter(|s| s.outcome.is_none())
        {
            let _ = self.console_stage(
                id,
                Mutation::Outcome {
                    ordinal: step.ordinal,
                    outcome: ConsoleStepOutcome::Unknown(ConsoleUnknown { statement: None }),
                    observations: ConsoleObservations::default(),
                },
            );
        }
        let patch = ConsoleObservations {
            retired: true,
            delivery_failed: true,
            first_error: original
                .observations
                .first_error
                .or(Some(ConsoleErrorCode::Interrupted)),
            ..Default::default()
        };
        let mut merged = original.observations.clone();
        absorb(&mut merged, &patch);
        if merged != original.observations {
            let _ = self.console_stage(
                id,
                Mutation::Observe {
                    ordinal: None,
                    observations: patch,
                },
            );
        }
        if original.terminal.is_none()
            && let Ok(run) = self.console_snapshot(id)
        {
            let _ = self.console_stage(
                id,
                Mutation::Finish {
                    reason: stop_reason(&run),
                    at: now_ms(),
                },
            );
        }
        let result_ids = self.console.journal.lock().result_ids(id);
        for result in result_ids {
            if self
                .results
                .info(&result)
                .is_ok_and(|r| r.status == RetentionStatus::Streaming)
            {
                let _ = self.results.finish(&result, true);
            }
        }
        let Ok(snapshot) = self.console_snapshot(id) else {
            return;
        };
        if original.terminal.is_none() {
            let mut journal = self.console.journal.lock();
            let retained = journal.runs.get_mut(id).unwrap();
            retained.history = Some(HistoryEntry {
                id: id.into(),
                connection_id: session.profile.id.clone(),
                connection_name: session.profile.name.clone(),
                started_at: snapshot.metadata.created_at,
                sql: None,
                statement_count: snapshot.metadata.step_count as usize,
                status: status(&snapshot),
                elapsed_ms: now_ms()
                    .saturating_sub(snapshot.metadata.created_at)
                    .min(u32::MAX as u64) as u32,
                execution_summary: None,
                snowflake_dml: None,
                snowflake_ddl: None,
            });
            retained.finished_pending = true;
            journal.dirty.insert(id.into());
        }
        let _ = self.flush_console_recordings();
        *session.active_run.lock() = None;
        self.jobs.lock().remove(id);
        self.console.journal.lock().active.remove(id);
        // Already durable success ACKs remain known even when publication panics.
        events(QueryEvent::Finished {
            run_id: id.into(),
            status: status(&snapshot),
            elapsed_ms: 0,
            execution_summary: None,
        });
    }

    async fn console_owned(self: Arc<Self>, owner: RunOwner) {
        let RunOwner {
            session,
            console: owner,
            stored,
            run_id,
            token,
            generation,
            events,
        } = owner;
        let began = Instant::now();
        let _lane = session.lane.lock().await;
        *session.active_run.lock() = Some(run_id.clone());
        let Action::SnowflakeConsole {
            compiled,
            timeout_seconds,
            ..
        } = &stored.action
        else {
            unreachable!()
        };
        let mut console = owner.lock().await;
        let delivery_open = events(QueryEvent::Started {
            run_id: run_id.clone(),
        });
        let mut error = None;
        if !delivery_open {
            let _ = self.console_stage(
                &run_id,
                Mutation::Observe {
                    ordinal: None,
                    observations: ConsoleObservations {
                        delivery_failed: true,
                        first_error: Some(ConsoleErrorCode::Delivery),
                        ..Default::default()
                    },
                },
            );
        } else if let Err(e) = self.flush_console_recordings() {
            error = Some(e);
        }
        if delivery_open && error.is_none() {
            match if console.timeout_seconds() != *timeout_seconds {
                Err(expired())
            } else {
                console.begin_run(compiled, token.clone()).await
            } {
                Err(e) => error = Some(e),
                Ok(mut run) => {
                    while !run.is_terminal() {
                        let ordinal = run.next_index() as u32;
                        // StatementStarted is a UI progress hint, not intent or
                        // dispatch evidence. Failure stops locally before SQL.
                        if !events(QueryEvent::StatementStarted {
                            run_id: run_id.clone(),
                            index: ordinal as usize,
                        }) {
                            let _ = self.console_stage(
                                &run_id,
                                Mutation::Observe {
                                    ordinal: None,
                                    observations: ConsoleObservations {
                                        delivery_failed: true,
                                        first_error: Some(ConsoleErrorCode::Delivery),
                                        ..Default::default()
                                    },
                                },
                            );
                            run.stop();
                            break;
                        }
                        let mut sink = ConsoleOutput {
                            studio: &self,
                            session_id: &session.info.id,
                            run_id: &run_id,
                            index: ordinal as usize,
                            events: &events,
                            current: None,
                            truncated: false,
                            delivery_failed: false,
                            published: false,
                        };
                        let mut recorder = Recorder {
                            studio: &self,
                            run: &run_id,
                            ordinal,
                        };
                        let mut admission = Admission {
                            studio: &self,
                            session: &session,
                            run: &run_id,
                            ordinal,
                            generation,
                            token: &token,
                        };
                        let outcome = run.next(&mut sink, &mut recorder, &mut admission).await;
                        match outcome {
                            Ok(Some(outcome)) => {
                                let report = outcome.report.as_ref();
                                let snapshot = self.console_snapshot(&run_id).unwrap();
                                let intended = snapshot.steps.get(ordinal as usize);
                                if intended.is_some_and(|s| s.outcome.is_none()) {
                                    let original = if let Some(ack) =
                                        report.and_then(|r| r.statement_acknowledged.as_ref())
                                    {
                                        ConsoleStepOutcome::Acknowledged(original_ack(ack))
                                    } else if !outcome.attempted
                                        || report
                                            .is_some_and(|r| r.certainty == Certainty::NotSubmitted)
                                    {
                                        ConsoleStepOutcome::NotDispatched(ConsoleNotDispatched {
                                            reason: if outcome.cancellation_observed {
                                                ConsoleNotDispatchedReason::Cancelled
                                            } else {
                                                ConsoleNotDispatchedReason::AdmissionRejected
                                            },
                                        })
                                    } else if report
                                        .is_some_and(|r| r.certainty == Certainty::ServerTerminal)
                                    {
                                        ConsoleStepOutcome::Rejected(ConsoleRejection {
                                            statement: report
                                                .and_then(|r| r.statements.first())
                                                .map(original_statement),
                                            code: ConsoleErrorCode::Database,
                                        })
                                    } else {
                                        ConsoleStepOutcome::Unknown(ConsoleUnknown {
                                            statement: report
                                                .and_then(|r| r.statements.first())
                                                .map(original_statement),
                                        })
                                    };
                                    if let Err(e) = self.console_stage(
                                        &run_id,
                                        Mutation::Outcome {
                                            ordinal,
                                            outcome: original,
                                            observations: ConsoleObservations::default(),
                                        },
                                    ) {
                                        error = Some(e);
                                    }
                                }
                                sink.finish_incomplete();
                                let mut observations = ConsoleObservations {
                                    delivered: sink.published,
                                    delivery_failed: sink.delivery_failed
                                        || outcome.delivery_error.as_ref().is_some_and(|e| {
                                            !matches!(
                                                e.code,
                                                ErrorCode::Cancelled
                                                    | ErrorCode::ResultLimitReached
                                            )
                                        }),
                                    cancellation_observed: outcome.cancellation_observed
                                        || token.is_cancelled(),
                                    retired: report.is_some_and(|r| r.retirement.is_some())
                                        || session.driver.is_closed(),
                                    truncated: outcome.truncated
                                        || sink.truncated
                                        || outcome.delivery_error.as_ref().is_some_and(|e| {
                                            e.code == ErrorCode::ResultLimitReached
                                        }),
                                    recording_failed: outcome.recording_error.is_some()
                                        || error.is_some(),
                                    first_error: None,
                                };
                                observations.first_error = if observations.recording_failed {
                                    Some(ConsoleErrorCode::Recording)
                                } else if observations.delivery_failed {
                                    Some(ConsoleErrorCode::Delivery)
                                } else if observations.truncated {
                                    Some(ConsoleErrorCode::ResultLimit)
                                } else if observations.cancellation_observed {
                                    Some(ConsoleErrorCode::Cancelled)
                                } else {
                                    None
                                };
                                let snapshot = self.console_snapshot(&run_id).unwrap();
                                let mut merged = snapshot.observations.clone();
                                absorb(&mut merged, &observations);
                                if merged != snapshot.observations {
                                    let index =
                                        snapshot.steps.get(ordinal as usize).map(|_| ordinal);
                                    if let Err(e) = self.console_stage(
                                        &run_id,
                                        Mutation::Observe {
                                            ordinal: index,
                                            observations,
                                        },
                                    ) {
                                        error = Some(e);
                                    }
                                }
                                if error.is_some()
                                    || sink.delivery_failed
                                    || outcome.recording_error.is_some()
                                    || outcome.delivery_error.is_some()
                                {
                                    run.stop();
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                let snapshot = self.console_snapshot(&run_id).unwrap();
                                if snapshot
                                    .steps
                                    .get(ordinal as usize)
                                    .is_some_and(|s| s.outcome.is_none())
                                {
                                    let _ = self.console_stage(&run_id, Mutation::Outcome { ordinal, outcome: ConsoleStepOutcome::NotDispatched(ConsoleNotDispatched {
                                        reason: if e.code == ErrorCode::Cancelled { ConsoleNotDispatchedReason::Cancelled } else { ConsoleNotDispatchedReason::AdmissionRejected },
                                    }), observations: ConsoleObservations::default() });
                                }
                                error = Some(e);
                                run.stop();
                            }
                        }
                    }
                }
            }
        }
        if let Some(e) = &error {
            let old = self.console_snapshot(&run_id).unwrap();
            let patch = ConsoleObservations {
                cancellation_observed: e.code == ErrorCode::Cancelled || token.is_cancelled(),
                recording_failed: !self.console.journal.lock().runs[&run_id].writes.is_empty(),
                first_error: old.observations.first_error.or(Some(
                    if e.code == ErrorCode::Cancelled {
                        ConsoleErrorCode::Cancelled
                    } else {
                        local_error_code(e)
                    },
                )),
                ..Default::default()
            };
            let mut merged = old.observations.clone();
            absorb(&mut merged, &patch);
            if merged != old.observations {
                let _ = self.console_stage(
                    &run_id,
                    Mutation::Observe {
                        ordinal: None,
                        observations: patch,
                    },
                );
            }
            events(QueryEvent::Error {
                error: e.clone(),
                statement_index: old.steps.len().saturating_sub(1),
            });
        }
        let snapshot = self.console_snapshot(&run_id).unwrap();
        let _ = self.console_stage(
            &run_id,
            Mutation::Finish {
                reason: stop_reason(&snapshot),
                at: now_ms(),
            },
        );
        let snapshot = self.console_snapshot(&run_id).unwrap();
        let elapsed_ms = began.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let history = HistoryEntry {
            id: run_id.clone(),
            connection_id: session.profile.id.clone(),
            connection_name: session.profile.name.clone(),
            started_at: snapshot.metadata.created_at,
            sql: session
                .profile
                .store_query_text
                .then(|| compiled.original_sql().to_owned()),
            statement_count: compiled.units().len(),
            status: status(&snapshot),
            elapsed_ms,
            execution_summary: None,
            snowflake_dml: None,
            snowflake_ddl: None,
        };
        {
            let mut journal = self.console.journal.lock();
            let retained = journal.runs.get_mut(&run_id).unwrap();
            retained.history = Some(history);
            retained.finished_pending = true;
            journal.dirty.insert(run_id.clone());
        }
        if self.flush_console_recordings().is_err() {
            events(QueryEvent::Notice { message: "The original console outcome is retained in memory, but local recording is pending. Keep Joinery open.".into() });
        }
        *session.active_run.lock() = None;
        self.jobs.lock().remove(&run_id);
        self.console.journal.lock().active.remove(&run_id);
        if !events(QueryEvent::Finished {
            run_id: run_id.clone(),
            status: status(&snapshot),
            elapsed_ms,
            execution_summary: None,
        }) && !snapshot.observations.delivery_failed
        {
            let _ = self.console_stage(
                &run_id,
                Mutation::Observe {
                    ordinal: None,
                    observations: ConsoleObservations {
                        delivery_failed: true,
                        first_error: snapshot
                            .observations
                            .first_error
                            .or(Some(ConsoleErrorCode::Delivery)),
                        ..Default::default()
                    },
                },
            );
        }
    }
}

#[cfg(unix)]
struct ConsoleOutput<'a> {
    studio: &'a Studio,
    session_id: &'a str,
    run_id: &'a str,
    index: usize,
    events: &'a EventHandler,
    current: Option<String>,
    truncated: bool,
    delivery_failed: bool,
    published: bool,
}

#[cfg(unix)]
impl ConsoleOutput<'_> {
    fn publish(&mut self, event: QueryEvent) -> AppResult<()> {
        if !self.delivery_failed && (self.events)(event) {
            self.published = true;
            Ok(())
        } else {
            self.delivery_failed = true;
            Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "Console result delivery closed",
            ))
        }
    }
    fn finish_incomplete(&mut self) {
        if let Some(id) = &self.current
            && self
                .studio
                .results
                .info(id)
                .is_ok_and(|r| r.status == RetentionStatus::Streaming)
        {
            let _ = self.studio.results.finish(id, true);
        }
    }
}

#[cfg(unix)]
impl QuerySink for ConsoleOutput<'_> {
    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()> {
        let result = self.studio.results.create_with_editability(
            self.run_id,
            self.index,
            columns,
            None,
            Some(ResultEditability::ReadOnly {
                reason: "Reviewed console rows do not establish editing authority".into(),
            }),
        )?;
        self.studio
            .result_sessions
            .lock()
            .insert(result.id.clone(), self.session_id.into());
        self.studio
            .console
            .journal
            .lock()
            .result_runs
            .insert(result.id.clone(), (self.run_id.into(), self.index));
        self.current = Some(result.id.clone());
        self.publish(QueryEvent::Result { result })
    }
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        let id = self
            .current
            .as_deref()
            .ok_or_else(|| AppError::internal("Rows preceded console metadata"))?;
        let retained = match self.studio.results.append(id, rows) {
            Ok(retained) => retained,
            Err(error) if error.code == ErrorCode::ResultLimitReached => {
                self.truncated = true;
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        self.truncated |= !retained;
        let result = self.studio.results.info(id)?;
        self.publish(QueryEvent::Progress {
            result_id: result.id,
            retained_rows: result.retained_rows,
        })?;
        Ok(retained)
    }
    fn complete(&mut self, _: u64) -> AppResult<()> {
        if self.current.is_none() {
            self.describe(Vec::new())?;
        }
        let result = self
            .studio
            .results
            .finish(self.current.as_deref().unwrap(), self.truncated)?;
        self.publish(QueryEvent::Result { result })
    }
    fn limit(&mut self, reason: &str) {
        self.truncated = true;
        let _ = self.publish(QueryEvent::Notice {
            message: reason.into(),
        });
    }
}
