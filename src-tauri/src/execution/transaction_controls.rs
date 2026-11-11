//! Explicit SDK boundaries use the same review policy as SQL, but retain their
//! typed action and command evidence through a separate execution route.
use super::*;

fn boundary_plan(
    session_id: &str,
    profile: &ConnectionProfile,
    request: &TransactionControlRequest,
) -> AppResult<PreparedPlan> {
    let mut plan = safety::plan_for_statements(
        session_id,
        profile,
        vec![StatementPlan {
            sql: request.action.sql().into(),
            kind: StatementKind::Transaction,
            missing_where: false,
            span: None,
        }],
    )?;
    plan.warnings.push(
        "This action uses this console's existing Snowflake session. A command acknowledgement does not prove committed or rolled-back data effects. No automatic retry is performed."
            .into(),
    );
    Ok(plan)
}

impl Studio {
    pub fn prepare_transaction_control(
        &self,
        session_id: &str,
        request: &TransactionControlRequest,
    ) -> AppResult<PreparedPlan> {
        let _configuration = self.configuration_gate.lock();
        let session = self.session(session_id)?;
        if !session.info.capabilities.explicit_transaction_controls {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Reviewed explicit transaction controls are unavailable for this session",
            ));
        }
        if request.revision.len() > 128 || uuid::Uuid::parse_str(&request.revision).is_err() {
            return Err(AppError::invalid(
                "Refresh transaction state before continuing",
            ));
        }
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The database session is busy",
            ));
        }
        let profile = self.persistence.profile(&session.profile.id)?;
        let plan = boundary_plan(session_id, &profile, request)?;
        session.driver.validate_transaction_control(request)?;
        self.store_plan(
            plan,
            Action::TransactionControl {
                request: request.clone(),
            },
            request.revision.len() + 32,
        )
    }

    pub async fn execute_transaction_control(
        &self,
        plan_id: &str,
        approval: Option<&str>,
    ) -> AppResult<TransactionControlResult> {
        // Keep the stored plan pending while queued. A profile/passphrase save
        // must still be able to invalidate it before the console lane is owned.
        let session_id = {
            let plans = self.plans.lock();
            let stored = plans.get(plan_id).ok_or_else(expired)?;
            if !matches!(stored.action, Action::TransactionControl { .. }) {
                return Err(AppError::invalid(
                    "This is not a prepared transaction action",
                ));
            }
            stored.plan.session_id.clone()
        };
        let session = self.session(&session_id)?;
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        let (request, digest) = {
            let _configuration = self.configuration_gate.lock();
            let stored = self.plans.lock().remove(plan_id).ok_or_else(expired)?;
            if stored.deadline <= Instant::now() || stored.plan.session_id != session_id {
                return Err(expired());
            }
            let Action::TransactionControl { request } = stored.action else {
                return Err(AppError::invalid(
                    "This is not a prepared transaction action",
                ));
            };
            if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
                return Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "This database session is disconnected",
                ));
            }
            if session.active_run.lock().is_some() {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "The database session is busy",
                ));
            }
            // Derive requirements again from the current persisted policy.
            // Authorization of an earlier, weaker policy cannot elevate it.
            let profile = self.persistence.profile(&session.profile.id)?;
            let current = boundary_plan(&session_id, &profile, &request)?;
            if current.requires_approval != stored.plan.requires_approval
                || current.requires_passphrase != stored.plan.requires_passphrase
                || current.confirmation_text != stored.plan.confirmation_text
            {
                return Err(expired());
            }
            if current.requires_approval
                && (stored.approval.is_none() || stored.approval.as_deref() != approval)
            {
                return Err(AppError::new(
                    ErrorCode::ApprovalRequired,
                    "Review and authorize this exact transaction action before running it",
                ));
            }
            session.driver.validate_transaction_control(&request)?;
            let audit = self.persistence.audit(
                &session.profile.id,
                "transaction-boundary",
                &stored.digest,
                "started",
            );
            if request.action != ExplicitTransactionAction::Rollback {
                audit?;
            }
            session.metadata_gate.lock().hold();
            *session.observed_runtime.lock() = None;
            *session.completion.lock() = None;
            session.driver.invalidate_relation_catalog();
            self.plans
                .lock()
                .retain(|_, plan| plan.plan.session_id != session_id);
            (request, stored.digest)
        };
        // Profile saves close the driver too. Check again after policy review
        // and immediately before entering the adapter's own admission checks.
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        let mut result = session
            .driver
            .transaction_control(&session_id, &request)
            .await?;
        if result.action != request.action || result.state.session_id != session_id {
            self.disconnect(&session_id);
            return Err(AppError::internal("Invalid transaction action response"));
        }
        let outcome = match result.boundary_outcome {
            TransactionBoundaryOutcome::Acknowledged => "acknowledged",
            TransactionBoundaryOutcome::Rejected => "rejected",
            TransactionBoundaryOutcome::Unknown => "unknown",
        };
        // Preserve known command evidence even if later local auditing fails.
        let _ = self.persistence.audit(
            &session.profile.id,
            "transaction-boundary",
            &digest,
            outcome,
        );
        if result.retired
            || result.boundary_outcome == TransactionBoundaryOutcome::Unknown
            || session.driver.is_closed()
            || session.closed.load(Ordering::Acquire)
        {
            result.retired = true;
            result.state.transaction_control_authority = None;
            self.disconnect(&session_id);
        } else {
            self.publish_observed_runtime(&session, &result.state);
        }
        // The adapter already observed its final state. An automatic generic
        // observer would add ten reads and could destroy recovery publication.
        Ok(result)
    }
}
