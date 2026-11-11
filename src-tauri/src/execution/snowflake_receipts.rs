//! Retain original finished typed SDK outcomes until local recording completes.
//! A pre-submission rejection has no invented terminal outcome or receipt.
//! This is an in-process outbox, not protection against forced exit or disk loss.
use super::*;

pub(super) enum OriginalResult {
    Ddl(SnowflakeDdlResult),
    Dml(SnowflakeDmlResult),
    Boundary(TransactionControlResult),
}

impl OriginalResult {
    fn audit(&self) -> (&'static str, &'static str) {
        match self {
            Self::Ddl(result) => (
                "snowflake-ddl",
                match result.statement_outcome {
                    SnowflakeDdlOutcome::Succeeded => "succeeded",
                    SnowflakeDdlOutcome::Rejected => "rejected",
                    SnowflakeDdlOutcome::Unknown => "unknown",
                },
            ),
            Self::Dml(result) => (
                "snowflake-dml",
                match result.statement_outcome {
                    SnowflakeDmlOutcome::Succeeded => "succeeded",
                    SnowflakeDmlOutcome::Rejected => "rejected",
                    SnowflakeDmlOutcome::Unknown => "unknown",
                },
            ),
            Self::Boundary(result) => (
                "transaction-boundary",
                match result.boundary_outcome {
                    TransactionBoundaryOutcome::Acknowledged => "acknowledged",
                    TransactionBoundaryOutcome::Rejected => "rejected",
                    TransactionBoundaryOutcome::Unknown => "unknown",
                },
            ),
        }
    }
}

pub(super) struct PendingReceipt {
    pub(super) entry: Option<HistoryEntry>,
    pub(super) original: OriginalResult,
    connection_id: String,
    digest: String,
    started_pending: bool,
    audit_pending: bool,
    history_pending: bool,
}

impl PendingReceipt {
    pub(super) fn statement(entry: HistoryEntry, digest: String, original: OriginalResult) -> Self {
        Self {
            connection_id: entry.connection_id.clone(),
            entry: Some(entry),
            original,
            digest,
            started_pending: false,
            audit_pending: true,
            history_pending: true,
        }
    }

    pub(super) fn boundary(
        connection_id: String,
        digest: String,
        result: TransactionControlResult,
        started_pending: bool,
    ) -> Self {
        Self {
            connection_id,
            digest,
            original: OriginalResult::Boundary(result),
            entry: None,
            started_pending,
            audit_pending: true,
            history_pending: false,
        }
    }
}

pub(super) fn recording_notice(error: &mut Option<AppError>) {
    const NOTICE: &str = "The action finished, but its local history or audit could not be recorded; keep Joinery open so the retained outcome can be recorded before closing";
    match error {
        Some(primary) => {
            primary.message.push_str(". ");
            primary.message.push_str(NOTICE);
        }
        None => *error = Some(AppError::internal(NOTICE)),
    }
}

impl Studio {
    pub(super) fn has_pending_rollback_receipt(&self, session_id: &str) -> bool {
        self.pending_snowflake_receipts.lock().values().any(|receipt| {
            matches!(&receipt.original, OriginalResult::Boundary(result)
                if result.action == ExplicitTransactionAction::Rollback && result.state.session_id == session_id)
        })
    }

    pub(super) fn retain_snowflake_receipt(&self, receipt: PendingReceipt) -> AppResult<()> {
        let id = receipt
            .entry
            .as_ref()
            .map_or_else(new_id, |entry| entry.id.clone());
        // Retain before the first persistence attempt, independently of caller
        // and session lifetime. A flush holds this same lock across all stages.
        self.pending_snowflake_receipts.lock().insert(id, receipt);
        self.flush_pending_snowflake_receipts()
    }

    pub(super) fn flush_pending_snowflake_receipts(&self) -> AppResult<()> {
        let mut pending = self.pending_snowflake_receipts.lock();
        for receipt in pending.values_mut() {
            let (action, outcome) = receipt.original.audit();
            if receipt.started_pending
                && self
                    .persistence
                    .audit(&receipt.connection_id, action, &receipt.digest, "started")
                    .is_ok()
            {
                receipt.started_pending = false;
            }
            // Rollback may run without a started audit. Record its ACK as
            // soon as that stage is writable, even if started still fails.
            if receipt.audit_pending
                && self
                    .persistence
                    .audit(&receipt.connection_id, action, &receipt.digest, outcome)
                    .is_ok()
            {
                receipt.audit_pending = false;
            }
            if receipt.history_pending
                && receipt
                    .entry
                    .as_ref()
                    .is_some_and(|entry| self.persistence.record_history(entry).is_ok())
            {
                receipt.history_pending = false;
            }
        }
        pending.retain(|_, receipt| {
            receipt.started_pending || receipt.audit_pending || receipt.history_pending
        });
        if pending.is_empty() {
            Ok(())
        } else {
            Err(AppError::new(
                ErrorCode::Conflict,
                "A retained Snowflake outcome could not be recorded locally; keep Joinery open and retry closing after local storage is available",
            ))
        }
    }
}
