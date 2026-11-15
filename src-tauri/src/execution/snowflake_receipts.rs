//! Typed producers retain only the bounded canonical outcome, never SDK results.
use super::*;

pub(super) type PendingReceipt = Arc<outcome_recording::OutcomeCell>;

pub(super) fn recording_notice(error: &mut Option<AppError>, recording: &AppError) {
    match error {
        Some(primary) => {
            primary.message.push_str(". ");
            primary.message.push_str(&recording.message);
        }
        None => *error = Some(recording.clone()),
    }
}

impl Studio {
    pub(super) fn has_pending_rollback_receipt(&self, session_id: &str) -> bool {
        self.pending_snowflake_receipts.lock().values().any(|cell| {
            matches!(
                cell.descriptor.kind,
                crate::persistence::OutcomeKind::Boundary {
                    action: ExplicitTransactionAction::Rollback
                }
            ) && cell.descriptor.session_id == session_id
                && !cell.is_settled()
        })
    }

    pub(super) fn flush_pending_snowflake_receipts(&self) -> AppResult<()> {
        self.flush_outcome_map(&self.pending_snowflake_receipts)
    }
}
