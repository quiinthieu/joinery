//! Reviewed SDK writes keep acknowledgement evidence outside generic row counts.
use super::*;

pub(super) fn sdk_profile(profile: &ConnectionProfile) -> bool {
    profile.engine == EngineKind::Snowflake
        && profile
            .snowflake
            .as_ref()
            .is_some_and(|config| config.authentication == SnowflakeAuthentication::ExternalBrowser)
}

pub(super) fn require_sdk_read_plan(
    profile: &ConnectionProfile,
    plan: &PreparedPlan,
) -> AppResult<()> {
    if sdk_profile(profile)
        && plan
            .statements
            .iter()
            .any(|statement| statement.kind != StatementKind::Read)
    {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Snowflake SDK writes require a single supported reviewed INSERT, UPDATE or DELETE; use the explicit controls for transaction boundaries",
        ));
    }
    Ok(())
}

fn writable_profile(profile: &ConnectionProfile) -> AppResult<()> {
    if !sdk_profile(profile)
        || !profile
            .snowflake
            .as_ref()
            .is_some_and(|config| config.reviewed_dml)
        || safety::effective_mode(profile) == SafeMode::ReadOnly
    {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "Enable reviewed Snowflake DML on a writable external-browser connection first",
        ));
    }
    Ok(())
}

fn dml_plan(
    session_id: &str,
    profile: &ConnectionProfile,
    canonical: &safety::snowflake_dml::Canonical,
) -> AppResult<PreparedPlan> {
    writable_profile(profile)?;
    let mut plan = safety::plan_for_statements(
        session_id,
        profile,
        vec![StatementPlan {
            sql: canonical.original_sql().into(),
            kind: StatementKind::Write,
            missing_where: canonical.missing_where(),
            span: None,
        }],
    )?;
    plan.reviewed_snowflake_dml = true;
    plan.bound_parameters = canonical.validated_inputs().to_vec();
    plan.binding_digest = Some(hex::encode(Sha256::digest(
        serde_json::to_vec(canonical.bound_statement())
            .map_err(|_| AppError::internal("Could not fingerprint DML parameters"))?,
    )));
    plan.warnings.push(
        "This statement writes to the fully qualified SQL name shown above. Concurrent schema changes or temporary objects can change name resolution; defaults, policies and dependencies can cause indirect effects. This is not an immutable object identity guarantee."
            .into(),
    );
    plan.warnings.push(
        "This uses the console's existing Snowflake session and transaction settings and may resume a billable warehouse. A successful statement acknowledgement does not prove committed data effects. No automatic retry is performed."
            .into(),
    );
    Ok(plan)
}

impl Studio {
    /// Strictly admitted DML is routed here by ordinary SQL preparation. No SQL
    /// or metadata request is sent while preparing this local review.
    pub(super) fn prepare_snowflake_dml(
        &self,
        session_id: &str,
        sql: &str,
        parameters: Option<&[QueryParamInput]>,
    ) -> AppResult<PreparedPlan> {
        let _configuration = self.configuration_mutation_guard()?;
        let session = self.session(session_id)?;
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The database session is busy",
            ));
        }
        let profile = self.persistence.profile(&session.profile.id)?;
        writable_profile(&profile)?;
        let (database, revision) = session
            .observed_runtime
            .lock()
            .as_ref()
            .and_then(|state| match &state.transaction_control_authority {
                Some(TransactionControlAuthority::Observed { revision, .. }) => {
                    Some((state.database.clone(), revision.clone()))
                }
                _ => None,
            })
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::Conflict,
                    "Refresh transaction state before preparing Snowflake DML",
                )
            })?;
        let canonical = safety::snowflake_dml::compile(sql, parameters, &database)?;
        let request = SnowflakeDmlRequest {
            sql: sql.into(),
            parameters: canonical.validated_inputs().to_vec(),
            revision,
        };
        session.driver.validate_snowflake_dml(&request)?;
        let plan = dml_plan(session_id, &profile, &canonical)?;
        let bytes = serde_json::to_vec(&request)
            .map_err(|_| AppError::internal("Could not encode reviewed DML"))?
            .len()
            .saturating_add(
                serde_json::to_vec(canonical.bound_statement())
                    .map_err(|_| AppError::internal("Could not encode DML parameters"))?
                    .len(),
            );
        self.store_plan(
            plan,
            Action::SnowflakeDml {
                request,
                bound: canonical.bound_statement().clone(),
                database,
                mode: safety::effective_mode(&profile),
            },
            bytes,
        )
    }

    pub async fn execute_snowflake_dml(
        self: &Arc<Self>,
        plan_id: &str,
        approval: Option<&str>,
    ) -> AppResult<SnowflakeDmlResult> {
        let _recovery_activity = self.begin_recovery_activity()?;
        // Bound queued tasks before TaskTracker allocates/spawns them.
        let memory = self.outcome_memory.ordinary()?;
        let studio = self.clone();
        let plan_id = plan_id.to_owned();
        let approval = approval.map(str::to_owned);
        // Detaching the IPC waiter must not drop acknowledged remote evidence.
        // The owner task finishes retirement and durable history independently.
        let owner = {
            let _configuration = self.configuration_mutation_guard()?;
            // TaskTracker.close does not itself reject new tasks. Admission
            // and shutdown share this gate so the final drain cannot miss one.
            if self.dml_tasks.is_closed() {
                return Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "Joinery is closing; no database write was started",
                ));
            }
            let recovery_activity = self.retain_recovery_activity()?;
            self.dml_tasks.spawn(async move {
                let _recovery_activity = recovery_activity;
                studio
                    .execute_snowflake_dml_owned(&plan_id, approval.as_deref(), memory)
                    .await
            })
        };
        owner.await.map_err(|_| {
            AppError::new(
                ErrorCode::OutcomeUnknown,
                "The write task stopped unexpectedly; inspect history before retrying",
            )
        })?
    }

    async fn execute_snowflake_dml_owned(
        &self,
        plan_id: &str,
        approval: Option<&str>,
        memory: outcome_memory::MemoryLease,
    ) -> AppResult<SnowflakeDmlResult> {
        // A queued review remains revocable by a profile/passphrase save.
        let session_id = {
            let plans = self.plans.lock();
            let stored = plans.get(plan_id).ok_or_else(expired)?;
            if !matches!(stored.action, Action::SnowflakeDml { .. }) {
                return Err(AppError::invalid(
                    "This is not a reviewed Snowflake DML action",
                ));
            }
            stored.plan.session_id.clone()
        };
        let session = self.session(&session_id)?;
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        let (request, kind, cell, mut history_owner) = {
            let _configuration = self.configuration_mutation_guard()?;
            self.flush_pending_history()?;
            self.flush_pending_snowflake_receipts()?;
            let stored = self.plans.lock().remove(plan_id).ok_or_else(expired)?;
            if stored.deadline <= Instant::now() || stored.plan.session_id != session_id {
                return Err(expired());
            }
            let Action::SnowflakeDml {
                request,
                bound,
                database,
                mode,
            } = stored.action
            else {
                return Err(AppError::invalid(
                    "This is not a reviewed Snowflake DML action",
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
            let profile = self.persistence.profile(&session.profile.id)?;
            writable_profile(&profile)?;
            let canonical =
                safety::snowflake_dml::compile(&request.sql, Some(&request.parameters), &database)?;
            canonical.verify_bound(&bound)?;
            let current = dml_plan(&session_id, &profile, &canonical)?;
            if mode != safety::effective_mode(&profile)
                || current.requires_approval != stored.plan.requires_approval
                || current.requires_passphrase != stored.plan.requires_passphrase
                || current.confirmation_text != stored.plan.confirmation_text
                || current.binding_digest != stored.plan.binding_digest
            {
                return Err(expired());
            }
            if current.requires_approval
                && (stored.approval.is_none() || stored.approval.as_deref() != approval)
            {
                return Err(AppError::new(
                    ErrorCode::ApprovalRequired,
                    "Review and authorize this exact DML statement before running it",
                ));
            }
            session.driver.validate_snowflake_dml(&request)?;
            let seed = HistoryEntry {
                provenance: HistoryProvenance::Admitted,
                execution_summary: None,
                snowflake_dml: None,
                snowflake_ddl: None,
                id: new_id(),
                connection_id: profile.id.clone(),
                connection_name: profile.name.clone(),
                started_at: now_ms(),
                sql: profile.store_query_text.then(|| request.sql.clone()),
                statement_count: 1,
                status: None,
                elapsed_ms: None,
            };
            let cell = self.admit_outcome(
                memory,
                Some(&seed),
                crate::persistence::OutcomeDescriptor {
                    operation_id: seed.id.clone(),
                    connection_id: profile.id.clone(),
                    session_id: session_id.clone(),
                    started_at: seed.started_at,
                    digest: stored.digest.clone(),
                    kind: crate::persistence::OutcomeKind::SnowflakeDml {
                        kind: canonical.kind(),
                    },
                },
                crate::persistence::OutcomePool::Ordinary,
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
            (request, canonical.kind(), cell, history_owner)
        };
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        let started = Instant::now();
        let mut result = session.driver.snowflake_dml(&session_id, &request).await?;
        if result.kind != kind || result.state.session_id != session_id {
            self.disconnect(&session_id);
            return Err(AppError::internal("Invalid Snowflake DML response"));
        }
        if result.retired
            || result.statement_outcome == SnowflakeDmlOutcome::Unknown
            || session.driver.is_closed()
            || session.closed.load(Ordering::Acquire)
        {
            result.retired = true;
            result.state.transaction_control_authority = None;
        }
        // Neither local persistence nor a late close may erase remote ACKs.
        let completion = HistoryCompletion {
            provenance: HistoryProvenance::Recorded,
            execution_summary: None,
            snowflake_dml: Some(SnowflakeDmlEvidence {
                kind: result.kind,
                statement_outcome: result.statement_outcome,
                summary: result.summary.clone(),
                retired: result.retired,
            }),
            snowflake_ddl: None,
            status: Some(match result.statement_outcome {
                SnowflakeDmlOutcome::Succeeded => RunStatus::Completed,
                SnowflakeDmlOutcome::Rejected => RunStatus::Failed,
                SnowflakeDmlOutcome::Unknown => RunStatus::OutcomeUnknown,
            }),
            elapsed_ms: Some(started.elapsed().as_millis().min(u32::MAX as u128) as u32),
        };
        let recording = self.finish_outcome_history(&cell, &completion);
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
        // The adapter already took its final sample; no extra observer SQL.
        Ok(result)
    }
}
