//! Completion retries carry no query text. Admission owns the sole durable SQL row.
use super::*;

pub(super) struct PendingHistory {
    ticket: HistoryTicket,
    completion: HistoryCompletion,
    audit: AuditEntry,
    history_pending: bool,
    audit_pending: bool,
}

pub(super) struct AdmissionOwner<'a> {
    studio: &'a Studio,
    ticket: HistoryTicket,
    connection_id: String,
    digest: String,
    action: &'static str,
    armed: bool,
}

impl<'a> AdmissionOwner<'a> {
    pub(super) fn new(
        studio: &'a Studio,
        ticket: HistoryTicket,
        connection_id: &str,
        digest: &str,
        action: &'static str,
    ) -> Self {
        Self {
            studio,
            ticket,
            connection_id: connection_id.into(),
            digest: digest.into(),
            action,
            armed: true,
        }
    }
    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AdmissionOwner<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = self.studio.retain_history_completion_with_action(
            self.ticket.clone(),
            HistoryCompletion {
                provenance: HistoryProvenance::Interrupted,
                status: None,
                elapsed_ms: None,
                execution_summary: None,
                snowflake_dml: None,
                snowflake_ddl: None,
            },
            &self.connection_id,
            &self.digest,
            self.action,
        );
    }
}

/// Dropping a task cannot turn an admitted row into a fabricated completion.
pub(super) struct RunOwner {
    studio: Arc<Studio>,
    session: Arc<Session>,
    ticket: HistoryTicket,
    digest: String,
    events: EventHandler,
    completion: Arc<Mutex<Option<HistoryCompletion>>>,
    armed: bool,
}

impl RunOwner {
    pub(super) fn new(
        studio: Arc<Studio>,
        session: Arc<Session>,
        ticket: HistoryTicket,
        digest: String,
        events: EventHandler,
        completion: Arc<Mutex<Option<HistoryCompletion>>>,
    ) -> Self {
        Self {
            studio,
            session,
            ticket,
            digest,
            events,
            completion,
            armed: true,
        }
    }
    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RunOwner {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let id = self.ticket.id();
        // A callback may panic after we retained or committed the known result.
        // Never replace that stronger evidence with an interrupted projection.
        let completion = self.completion.lock().clone();
        if completion.is_none() {
            let _ = self.studio.retain_history_completion(
                self.ticket.clone(),
                HistoryCompletion {
                    provenance: HistoryProvenance::Interrupted,
                    status: None,
                    elapsed_ms: None,
                    execution_summary: None,
                    snowflake_dml: None,
                    snowflake_ddl: None,
                },
                &self.session.profile.id,
                &self.digest,
            );
        }
        self.session.closed.store(true, Ordering::Release);
        self.session.driver.close();
        self.studio.jobs.lock().remove(id);
        {
            let mut active = self.session.active_run.lock();
            if active.as_deref() == Some(id) {
                *active = None;
            }
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (self.events)(QueryEvent::Error {
                error: AppError::new(
                    ErrorCode::OutcomeUnknown,
                    "The query task stopped unexpectedly; inspect history and database state before retrying",
                ),
                statement_index: 0,
            });
            (self.events)(QueryEvent::Finished {
                run_id: id.into(),
                status: completion
                    .as_ref()
                    .and_then(|c| c.status)
                    .unwrap_or(RunStatus::OutcomeUnknown),
                elapsed_ms: completion.as_ref().and_then(|c| c.elapsed_ms).unwrap_or(0),
                execution_summary: completion
                    .as_ref()
                    .and_then(|c| c.execution_summary.clone()),
            });
        }));
    }
}

impl Studio {
    pub(super) fn retain_history_completion(
        &self,
        ticket: HistoryTicket,
        completion: HistoryCompletion,
        connection_id: &str,
        digest: &str,
    ) -> AppResult<()> {
        self.retain_history_completion_with_action(
            ticket,
            completion,
            connection_id,
            digest,
            "execute",
        )
    }

    fn retain_history_completion_with_action(
        &self,
        ticket: HistoryTicket,
        completion: HistoryCompletion,
        connection_id: &str,
        digest: &str,
        action: &str,
    ) -> AppResult<()> {
        let id = ticket.id().to_owned();
        let audit = AuditEntry {
            id: format!("{id}:finished"),
            occurred_at: now_ms(),
            connection_id: connection_id.into(),
            action: action.into(),
            digest: digest.into(),
            outcome: completion
                .status
                .map_or_else(|| "interrupted".into(), |status| format!("{status:?}")),
        };
        self.pending_history.lock().insert(
            id,
            PendingHistory {
                ticket,
                completion,
                audit,
                history_pending: true,
                audit_pending: true,
            },
        );
        self.flush_pending_history()
    }

    pub(super) fn flush_pending_history(&self) -> AppResult<()> {
        let mut pending = self.pending_history.lock();
        for receipt in pending.values_mut() {
            if receipt.history_pending
                && self
                    .persistence
                    .finish_history(&receipt.ticket, &receipt.completion)
                    .is_ok()
            {
                receipt.history_pending = false;
            }
            if receipt.audit_pending && self.persistence.record_audit_once(&receipt.audit).is_ok() {
                receipt.audit_pending = false;
            }
        }
        pending.retain(|_, receipt| receipt.history_pending || receipt.audit_pending);
        if pending.is_empty() {
            Ok(())
        } else {
            Err(AppError::new(
                ErrorCode::Conflict,
                "A query outcome could not be recorded locally; keep Joinery open and retry closing after local storage is available",
            ))
        }
    }
}
