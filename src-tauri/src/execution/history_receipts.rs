//! Completion retries carry no query text. Admission owns the sole durable SQL row.
use super::*;

pub(super) type PendingHistory = Arc<outcome_recording::OutcomeCell>;

pub(super) struct AdmissionOwner<'a> {
    studio: &'a Studio,
    cell: Arc<outcome_recording::OutcomeCell>,
    armed: bool,
}

impl<'a> AdmissionOwner<'a> {
    pub(super) fn new(studio: &'a Studio, cell: Arc<outcome_recording::OutcomeCell>) -> Self {
        Self {
            studio,
            cell,
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
        self.studio.interrupt_outcome(&self.cell);
    }
}

/// Dropping a task cannot turn an admitted row into a fabricated completion.
pub(super) struct RunOwner {
    studio: Arc<Studio>,
    session: Arc<Session>,
    cell: Arc<outcome_recording::OutcomeCell>,
    events: EventHandler,
    completion: Arc<Mutex<Option<HistoryCompletion>>>,
    armed: bool,
}

impl RunOwner {
    pub(super) fn new(
        studio: Arc<Studio>,
        session: Arc<Session>,
        cell: Arc<outcome_recording::OutcomeCell>,
        events: EventHandler,
        completion: Arc<Mutex<Option<HistoryCompletion>>>,
    ) -> Self {
        Self {
            studio,
            session,
            cell,
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
        let id = self.cell.descriptor.operation_id.as_str();
        // A callback may panic after we retained or committed the known result.
        // Never replace that stronger evidence with an interrupted projection.
        let completion = self.completion.lock().clone();
        self.studio.interrupt_outcome(&self.cell);
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
    pub(super) fn flush_pending_history(&self) -> AppResult<()> {
        self.flush_outcome_map(&self.pending_history)
    }
}
