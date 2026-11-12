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
use crate::persistence::snowflake_console::ConsoleJournalOwner;
use std::{collections::VecDeque, sync::atomic::AtomicU64};

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
}

struct Retained {
    run: ConsoleRunSnapshot,
    writes: VecDeque<Write>,
    results: Vec<String>,
    started_pending: bool,
    history: Option<HistoryEntry>,
    finished_pending: bool,
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
    pub(super) fn revoke(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn replace_preview(&self, session: &str) {
        let mut previews = self.previews.lock();
        let epoch = previews.entry(session.into()).or_default();
        *epoch = epoch.wrapping_add(1);
    }
    fn preview(&self, session: &str) -> u64 {
        self.previews
            .lock()
            .get(session)
            .copied()
            .unwrap_or_default()
    }
    pub(super) fn owns_result(&self, id: &str) -> bool {
        self.journal
            .lock()
            .runs
            .values()
            .any(|r| r.results.iter().any(|result| result == id))
    }
    pub(super) fn is_run(&self, id: &str) -> bool {
        self.journal.lock().runs.contains_key(id)
    }
    pub(super) fn release_journal(&self) {
        self.journal.lock().owner.take();
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
        if journal.runs.is_empty() {
            return Ok(());
        }
        let Journal { owner, runs } = &mut *journal;
        let owner = owner.as_ref().ok_or_else(recording_error)?;
        for retained in runs.values_mut() {
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
        }
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

    /// This lookup performs no helper request, reconnect, metadata query or SQL
    /// retrieval. Incomplete receipts can be sealed locally after exclusive journal
    /// ownership succeeds; completed receipt retrieval needs no lease.
    pub fn snowflake_console_run(
        &self,
        client_request_id: &str,
    ) -> AppResult<Option<SnowflakeConsoleRunView>> {
        uuid::Uuid::parse_str(client_request_id)
            .map_err(|_| AppError::invalid("Invalid console request identifier"))?;
        let _ = self.flush_console_recordings();
        let journal = self.console.journal.lock();
        if let Some(r) = journal
            .runs
            .values()
            .find(|r| r.run.metadata.client_request_id == client_request_id)
        {
            return Ok(Some(SnowflakeConsoleRunView {
                run: r.run.clone(),
                results: r
                    .results
                    .iter()
                    .filter_map(|id| self.results.info(id).ok())
                    .collect(),
                pending_recording: !r.writes.is_empty()
                    || r.started_pending
                    || r.history.is_some()
                    || r.finished_pending,
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
                journal.owner = Some(owner);
                run = self
                    .persistence
                    .console_run_by_client_request(client_request_id)?;
            }
        }
        Ok(run.map(|run| SnowflakeConsoleRunView {
            pending_recording: run.terminal.is_none(),
            run,
            results: Vec::new(),
        }))
    }
}

#[cfg(unix)]
fn writable(profile: &ConnectionProfile) -> AppResult<()> {
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
        let _configuration = self.configuration_gate.lock();
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
            validations,
        })
    }

    pub fn start_snowflake_console(
        self: &Arc<Self>,
        request: &SnowflakeConsoleStart,
        events: EventHandler,
    ) -> AppResult<SnowflakeConsoleStartResult> {
        let _configuration = self.configuration_gate.lock();
        uuid::Uuid::parse_str(&request.client_request_id)
            .map_err(|_| AppError::invalid("Invalid console request identifier"))?;
        // Stable request identity is authoritative even after its one-use plan
        // was consumed or its physical session was removed. Never re-submit.
        if let Some(existing) = self.snowflake_console_run(&request.client_request_id)? {
            if existing.run.metadata.session_id == request.session_id
                && existing.run.metadata.plan_digest == request.plan_digest
            {
                return Ok(SnowflakeConsoleStartResult::Started {
                    run_id: existing.run.metadata.run_id,
                });
            }
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This console request identifier belongs to a different review",
            ));
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
        // A definite negative requires proving absence while holding the OS
        // journal lease. A failed lookup/lease can hide another owner's original;
        // propagate that uncertainty instead of manufacturing NotAdmitted.
        {
            let mut journal = self.console.journal.lock();
            if journal.owner.is_none() {
                journal.owner = Some(self.persistence.claim_console_journal_owner()?);
            }
            if let Some(existing) = self
                .persistence
                .console_run_by_client_request(&request.client_request_id)?
            {
                if existing.metadata.session_id == request.session_id
                    && existing.metadata.plan_digest == request.plan_digest
                {
                    return Ok(SnowflakeConsoleStartResult::Started {
                        run_id: existing.metadata.run_id,
                    });
                }
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "This request identifier belongs to a different review",
                ));
            }
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
            } = &stored.action
            else {
                return Err(AppError::invalid("This is not a reviewed console plan"));
            };
            let profile = self.persistence.profile(&session.profile.id)?;
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
            {
                let mut journal = self.console.journal.lock();
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
                        results: Vec::new(),
                        started_pending: true,
                        history: None,
                        finished_pending: false,
                    },
                );
            }
            let stored = plans.remove(&request.plan_id).unwrap();
            drop(plans);
            let run_id = self
                .console
                .journal
                .lock()
                .runs
                .values()
                .find(|r| r.run.metadata.client_request_id == request.client_request_id)
                .unwrap()
                .run
                .metadata
                .run_id
                .clone();
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
                use futures_util::FutureExt;
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
            });
            Ok(run_id)
        })();
        Ok(match admission {
            Ok(run_id) => SnowflakeConsoleStartResult::Started { run_id },
            Err(error) => SnowflakeConsoleStartResult::NotAdmitted {
                client_request_id: request.client_request_id.clone(),
                session_id: request.session_id.clone(),
                plan_digest: request.plan_digest.clone(),
                code: error.code,
            },
        })
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
        let _configuration = self.studio.configuration_gate.lock();
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
        let result_ids = self.console.journal.lock().runs[id].results.clone();
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
        }
        let _ = self.flush_console_recordings();
        *session.active_run.lock() = None;
        self.jobs.lock().remove(id);
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
        let Action::SnowflakeConsole { compiled, .. } = &stored.action else {
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
            match console.begin_run(compiled, token.clone()).await {
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
        }
        if self.flush_console_recordings().is_err() {
            events(QueryEvent::Notice { message: "The original console outcome is retained in memory, but local recording is pending. Keep Joinery open.".into() });
        }
        *session.active_run.lock() = None;
        self.jobs.lock().remove(&run_id);
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
            .runs
            .get_mut(self.run_id)
            .unwrap()
            .results
            .push(result.id.clone());
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
