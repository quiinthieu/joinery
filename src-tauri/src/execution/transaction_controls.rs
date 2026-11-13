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
        let _configuration = self.configuration_mutation_guard()?;
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
        self: &Arc<Self>,
        plan_id: &str,
        approval: Option<&str>,
    ) -> AppResult<TransactionControlResult> {
        let _recovery_activity = self.begin_recovery_activity()?;
        // Bound tasks before spawning. Rollback uses the safety cell reserved
        // before authentication, independently of ordinary recording capacity.
        let memory = {
            let (session_id, action) = {
                let plans = self.plans.lock();
                let stored = plans.get(plan_id).ok_or_else(expired)?;
                let Action::TransactionControl { request } = &stored.action else {
                    return Err(AppError::invalid(
                        "This is not a prepared transaction action",
                    ));
                };
                (stored.plan.session_id.clone(), request.action)
            };
            if action == ExplicitTransactionAction::Rollback {
                self.rollback_memory(self.session(&session_id)?.as_ref())?
            } else {
                self.outcome_memory.ordinary()?
            }
        };
        let studio = self.clone();
        let plan_id = plan_id.to_owned();
        let approval = approval.map(str::to_owned);
        let owner = {
            let _configuration = self.configuration_mutation_guard()?;
            if self.dml_tasks.is_closed() {
                return Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "Joinery is closing; no transaction action was started",
                ));
            }
            let recovery_activity = self.retain_recovery_activity()?;
            self.dml_tasks.spawn(async move {
                let _recovery_activity = recovery_activity;
                studio
                    .execute_transaction_control_owned(&plan_id, approval.as_deref(), memory)
                    .await
            })
        };
        owner.await.map_err(|_| {
            AppError::new(
                ErrorCode::OutcomeUnknown,
                "The transaction task stopped unexpectedly; inspect its outcome before retrying",
            )
        })?
    }

    async fn execute_transaction_control_owned(
        &self,
        plan_id: &str,
        approval: Option<&str>,
        memory: outcome_memory::MemoryLease,
    ) -> AppResult<TransactionControlResult> {
        // Keep the stored plan pending while queued. A profile/passphrase save
        // must still be able to invalidate it before the console lane is owned.
        let (session_id, action) = {
            let plans = self.plans.lock();
            let stored = plans.get(plan_id).ok_or_else(expired)?;
            let Action::TransactionControl { request } = &stored.action else {
                return Err(AppError::invalid(
                    "This is not a prepared transaction action",
                ));
            };
            (stored.plan.session_id.clone(), request.action)
        };
        let session = self.session(&session_id)?;
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        let (request, cell, mut history_owner) = {
            let _configuration = self.configuration_mutation_guard()?;
            // Recovery rollback remains available while local recording is
            // unavailable. It never replays the earlier action or its SQL.
            let generic_flush = self.flush_pending_history();
            let typed_flush = self.flush_pending_snowflake_receipts();
            if action != ExplicitTransactionAction::Rollback {
                generic_flush?;
                typed_flush?;
            } else if self.has_pending_rollback_receipt(&session_id) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "The earlier rollback outcome is still awaiting local recording; restore local storage before another rollback",
                ));
            }
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
            let pool = if request.action == ExplicitTransactionAction::Rollback {
                crate::persistence::OutcomePool::EmergencyRollback
            } else {
                crate::persistence::OutcomePool::Ordinary
            };
            let cell = self.admit_outcome(
                memory,
                None,
                crate::persistence::OutcomeDescriptor {
                    operation_id: new_id(),
                    connection_id: session.profile.id.clone(),
                    session_id: session_id.clone(),
                    started_at: now_ms(),
                    digest: stored.digest,
                    kind: crate::persistence::OutcomeKind::Boundary {
                        action: request.action,
                    },
                },
                pool,
            )?;
            // Arm before invoking any driver callback after admission.
            let history_owner = history_receipts::AdmissionOwner::new(self, cell.clone());
            session.metadata_gate.lock().hold();
            *session.observed_runtime.lock() = None;
            *session.completion.lock() = None;
            session.driver.invalidate_relation_catalog();
            self.plans
                .lock()
                .retain(|_, plan| plan.plan.session_id != session_id);
            (request, cell, history_owner)
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
        if result.retired
            || result.boundary_outcome == TransactionBoundaryOutcome::Unknown
            || session.driver.is_closed()
            || session.closed.load(Ordering::Acquire)
        {
            result.retired = true;
            result.state.transaction_control_authority = None;
        }
        let recording = self.finish_outcome_boundary(
            &cell,
            &crate::persistence::BoundaryEvidence {
                action: result.action,
                boundary_outcome: result.boundary_outcome,
                retired: result.retired,
            },
        );
        history_owner.disarm();
        // Preserve the returned original before retiring a driver; close may panic.
        if result.retired {
            self.disconnect(&session_id);
        }
        if let Err(recording) = recording {
            snowflake_receipts::recording_notice(&mut result.error, &recording);
        }
        if !result.retired {
            self.publish_observed_runtime(&session, &result.state);
        }
        // The adapter already observed its final state. An automatic generic
        // observer would add ten reads and could destroy recovery publication.
        Ok(result)
    }
}
