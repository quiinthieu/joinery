//! Generated DDL has its own review, terminal evidence and catalog hold.
use super::*;

pub(super) struct Preflight {
    action: SnowflakeDdlAction,
    response: SnowflakeDdlPreflightResponse,
    deadline: Instant,
}

fn writable_profile(profile: &ConnectionProfile) -> AppResult<()> {
    if !snowflake_dml::sdk_profile(profile)
        || !profile
            .snowflake
            .as_ref()
            .is_some_and(|config| config.reviewed_ddl)
        || safety::effective_mode(profile) == SafeMode::ReadOnly
    {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "Enable reviewed Snowflake DDL on a writable external-browser connection first",
        ));
    }
    Ok(())
}

fn ddl_plan(
    session_id: &str,
    profile: &ConnectionProfile,
    request: &SnowflakeDdlRequest,
    response: &SnowflakeDdlPreflightResponse,
) -> AppResult<PreparedPlan> {
    writable_profile(profile)?;
    // Recompile using the adapter's server-held, fully qualified target.
    let table = TableRef {
        schema: response.schema.clone(),
        name: response.name.clone(),
    };
    let action = match &request.action {
        SnowflakeDdlAction::CreateTable {
            database,
            schema,
            name,
            columns,
        } => {
            if database != &response.database
                || schema != &response.schema
                || name != &response.name
            {
                return Err(expired());
            }
            crate::ddl::TableObjectAction::CreateTable {
                table,
                columns: columns
                    .iter()
                    .map(SnowflakeDdlColumnDraft::to_table_column_draft)
                    .collect(),
                comment: None,
            }
        }
        SnowflakeDdlAction::AddColumn { column, .. } => crate::ddl::TableObjectAction::AddColumn {
            table,
            column: column.to_table_column_draft(),
        },
        SnowflakeDdlAction::DropTable { .. } => crate::ddl::TableObjectAction::DropObject {
            object: DatabaseObjectRef {
                schema: table.schema,
                name: table.name,
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
            cascade: false,
        },
    };
    let canonical = safety::snowflake_ddl::compile(&action, &response.database)?;
    let qualified = canonical
        .qualified_target()
        .iter()
        .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".");
    if response.revision != request.revision
        || response.sql != request.sql
        || canonical.sql() != request.sql
        || SnowflakeDdlKind::from(canonical.kind()) != response.kind
        || qualified != response.qualified_target
    {
        return Err(expired());
    }
    let mut plan = safety::plan_for_statements(
        session_id,
        profile,
        vec![StatementPlan {
            sql: request.sql.clone(),
            kind: StatementKind::Ddl,
            missing_where: false,
            span: None,
        }],
    )?;
    plan.reviewed_snowflake_ddl = true;
    plan.requires_approval = true;
    plan.binding_digest = Some(hex::encode(Sha256::digest(
        serde_json::to_vec(&(request, response))
            .map_err(|_| AppError::internal("Could not fingerprint generated DDL"))?,
    )));
    if response.kind == SnowflakeDdlKind::DropTable {
        plan.target_confirmation_text = Some(response.qualified_target.clone());
    }
    plan.warnings = response.warnings.clone();
    plan.warnings.push("Snowflake DDL commits independently and cannot be rolled back. This action requires an idle session with AUTOCOMMIT enabled; it never changes transaction settings.".into());
    plan.warnings.push("Use only a dedicated disposable schema with no concurrent DDL actors. Names and catalog samples do not establish immutable object identity; temporary shadows, concurrent replacement and dependencies may change effects.".into());
    plan.warnings.push("A statement acknowledgement is separate from later catalog visibility. No automatic retry or catalog refresh occurs; explicitly reconcile the catalog before continuing metadata work. The existing session may resume a billable warehouse.".into());
    Ok(plan)
}

fn coherent_idle(state: &SessionRuntimeState, session_id: &str) -> bool {
    state.session_id == session_id
        && state.transaction_mode == TransactionMode::AutoCommit
        && state.transaction_state == TransactionState::Idle
        && state.context_fingerprint.is_some()
        && matches!(
            state.transaction_observation,
            Some(TransactionObservation::Observed {
                transaction_id: None,
                ..
            })
        )
        && matches!(&state.transaction_control_authority, Some(TransactionControlAuthority::Observed { allowed_actions, .. }) if allowed_actions.contains(&ExplicitTransactionAction::Begin))
}

impl Studio {
    pub async fn preflight_snowflake_ddl(
        &self,
        request: &SnowflakeDdlPreflightRequest,
    ) -> AppResult<SnowflakeDdlPreflightResponse> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.flush_pending_snowflake_receipts()?;
        let session = self.session(&request.session_id)?;
        let revision = session.metadata_gate.lock().revision.clone();
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        {
            let _configuration = self.configuration_mutation_guard()?;
            writable_profile(&self.persistence.profile(&session.profile.id)?)?;
            if !session.info.capabilities.snowflake_reviewed_ddl {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "Reviewed Snowflake DDL is unavailable for this session",
                ));
            }
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
            session.metadata_gate.lock().check(Some(&revision), false)?;
            self.ddl_preflights.lock().remove(&request.session_id);
            self.plans
                .lock()
                .retain(|_, plan| plan.plan.session_id != request.session_id);
            *session.observed_runtime.lock() = None;
            *session.completion.lock() = None;
        }
        let response = session
            .driver
            .preflight_snowflake_ddl(&request.session_id, request)
            .await?;
        if serde_json::to_vec(&(request, &response))
            .map_err(|_| AppError::internal("Could not encode DDL preflight"))?
            .len()
            > 128 * 1024
        {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "DDL preflight exceeded its review bound",
            ));
        }
        let _configuration = self.configuration_mutation_guard()?;
        let profile = self.persistence.profile(&session.profile.id)?;
        writable_profile(&profile)?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        session.metadata_gate.lock().check(Some(&revision), false)?;
        let checked = SnowflakeDdlRequest {
            revision: response.revision.clone(),
            action: request.action.clone(),
            sql: response.sql.clone(),
        };
        ddl_plan(&request.session_id, &profile, &checked, &response)?;
        session.driver.validate_snowflake_ddl(&checked)?;
        let mut preflights = self.ddl_preflights.lock();
        preflights.retain(|_, grant| grant.deadline > Instant::now());
        if preflights.len() >= 32 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Too many pending DDL reviews",
            ));
        }
        preflights.insert(
            request.session_id.clone(),
            Preflight {
                action: request.action.clone(),
                response: response.clone(),
                deadline: Instant::now() + Duration::from_secs(60),
            },
        );
        Ok(response)
    }

    pub fn prepare_snowflake_ddl(
        &self,
        session_id: &str,
        request: &SnowflakeDdlRequest,
    ) -> AppResult<PreparedPlan> {
        let _configuration = self.configuration_mutation_guard()?;
        let session = self.session(session_id)?;
        session.metadata_gate.lock().check(None, false)?;
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The database session is busy",
            ));
        }
        let profile = self.persistence.profile(&session.profile.id)?;
        writable_profile(&profile)?;
        let preflights = self.ddl_preflights.lock();
        let preflight = preflights.get(session_id).ok_or_else(expired)?;
        if preflight.deadline <= Instant::now() || preflight.action != request.action {
            return Err(expired());
        }
        let plan = ddl_plan(session_id, &profile, request, &preflight.response)?;
        session.driver.validate_snowflake_ddl(request)?;
        let bytes = serde_json::to_vec(&(request, &preflight.response))
            .map_err(|_| AppError::internal("Could not encode reviewed DDL"))?
            .len();
        self.store_plan(
            plan,
            Action::SnowflakeDdl {
                request: request.clone(),
                preflight: preflight.response.clone(),
                mode: safety::effective_mode(&profile),
            },
            bytes,
        )
    }

    pub fn authorize_snowflake_ddl(
        &self,
        plan_id: &str,
        passphrase: Option<&str>,
        confirmation: Option<&str>,
        target_confirmation: Option<&str>,
    ) -> AppResult<String> {
        self.authorize_plan(plan_id, passphrase, confirmation, target_confirmation, true)
    }

    pub async fn execute_snowflake_ddl(
        self: &Arc<Self>,
        plan_id: &str,
        approval: Option<&str>,
    ) -> AppResult<SnowflakeDdlResult> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let studio = self.clone();
        let plan_id = plan_id.to_owned();
        let approval = approval.map(str::to_owned);
        let owner = {
            let _configuration = self.configuration_mutation_guard()?;
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
                    .execute_snowflake_ddl_owned(&plan_id, approval.as_deref())
                    .await
            })
        };
        owner.await.map_err(|_| {
            AppError::new(
                ErrorCode::OutcomeUnknown,
                "The DDL task stopped unexpectedly; inspect history before retrying",
            )
        })?
    }

    async fn execute_snowflake_ddl_owned(
        &self,
        plan_id: &str,
        approval: Option<&str>,
    ) -> AppResult<SnowflakeDdlResult> {
        let session_id = {
            let plans = self.plans.lock();
            let stored = plans.get(plan_id).ok_or_else(expired)?;
            if !matches!(stored.action, Action::SnowflakeDdl { .. }) {
                return Err(AppError::invalid(
                    "This is not a reviewed Snowflake DDL action",
                ));
            }
            stored.plan.session_id.clone()
        };
        let session = self.session(&session_id)?;
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        let (request, preflight, digest, history_ticket, history_started_at) = {
            let _configuration = self.configuration_mutation_guard()?;
            self.flush_pending_history()?;
            self.flush_pending_snowflake_receipts()?;
            let stored = self.plans.lock().remove(plan_id).ok_or_else(expired)?;
            if stored.deadline <= Instant::now() || stored.plan.session_id != session_id {
                return Err(expired());
            }
            let Action::SnowflakeDdl {
                request,
                preflight,
                mode,
            } = stored.action
            else {
                return Err(AppError::invalid(
                    "This is not a reviewed Snowflake DDL action",
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
            session.metadata_gate.lock().check(None, false)?;
            let profile = self.persistence.profile(&session.profile.id)?;
            let current = ddl_plan(&session_id, &profile, &request, &preflight)?;
            if mode != safety::effective_mode(&profile)
                || current.requires_approval != stored.plan.requires_approval
                || current.requires_passphrase != stored.plan.requires_passphrase
                || current.confirmation_text != stored.plan.confirmation_text
                || current.target_confirmation_text != stored.plan.target_confirmation_text
                || current.binding_digest != stored.plan.binding_digest
            {
                return Err(expired());
            }
            if stored.approval.is_none() || stored.approval.as_deref() != approval {
                return Err(AppError::new(
                    ErrorCode::ApprovalRequired,
                    "Review and authorize this exact DDL action before running it",
                ));
            }
            session.driver.validate_snowflake_ddl(&request)?;
            self.persistence.audit(
                &session.profile.id,
                "snowflake-ddl",
                &stored.digest,
                "started",
            )?;
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
            let history_ticket = self
                .persistence
                .begin_owned_history(&self.history_owner, &seed)?;
            session.metadata_gate.lock().hold_catalog();
            *session.observed_runtime.lock() = None;
            *session.completion.lock() = None;
            self.ddl_preflights.lock().remove(&session_id);
            self.plans
                .lock()
                .retain(|_, plan| plan.plan.session_id != session_id);
            // The adapter consumes its composite grant before invalidating its
            // catalog. Calling invalidate_relation_catalog here would revoke it.
            (
                request,
                preflight,
                stored.digest,
                history_ticket,
                seed.started_at,
            )
        };
        let mut history_owner = history_receipts::AdmissionOwner::new(
            self,
            history_ticket.clone(),
            &session.profile.id,
            &digest,
            "snowflake-ddl",
        );
        let started = Instant::now();
        let mut result = session.driver.snowflake_ddl(&session_id, &request).await?;
        if result.kind != preflight.kind
            || result.qualified_target != preflight.qualified_target
            || result.state.session_id != session_id
        {
            self.disconnect(&session_id);
            return Err(AppError::internal("Invalid Snowflake DDL response"));
        }
        result.catalog_reconciliation_required = true;
        if result.retired
            || result.statement_outcome == SnowflakeDdlOutcome::Unknown
            || session.driver.is_closed()
            || session.closed.load(Ordering::Acquire)
        {
            result.retired = true;
            result.state.transaction_control_authority = None;
            self.disconnect(&session_id);
        }
        let entry = HistoryEntry {
            provenance: HistoryProvenance::Recorded,
            execution_summary: None,
            snowflake_ddl: Some(SnowflakeDdlEvidence {
                kind: result.kind,
                qualified_target: result.qualified_target.clone(),
                statement_outcome: result.statement_outcome,
                summary_status: result.summary_status,
                catalog_reconciliation_required: true,
                retired: result.retired,
            }),
            snowflake_dml: None,
            id: history_ticket.id().into(),
            connection_id: session.profile.id.clone(),
            connection_name: session.profile.name.clone(),
            started_at: history_started_at,
            sql: None,
            statement_count: 1,
            status: Some(match result.statement_outcome {
                SnowflakeDdlOutcome::Succeeded => RunStatus::Completed,
                SnowflakeDdlOutcome::Rejected => RunStatus::Failed,
                SnowflakeDdlOutcome::Unknown => RunStatus::OutcomeUnknown,
            }),
            elapsed_ms: Some(started.elapsed().as_millis().min(u32::MAX as u128) as u32),
        };
        let recording =
            self.retain_snowflake_receipt(snowflake_receipts::PendingReceipt::statement(
                history_ticket,
                entry,
                digest,
                snowflake_receipts::OriginalResult::Ddl(result.clone()),
            ));
        history_owner.disarm();
        if recording.is_err() {
            snowflake_receipts::recording_notice(&mut result.error);
        }
        if !result.retired {
            self.publish_observed_runtime(&session, &result.state);
        }
        Ok(result)
    }

    /// Explicit owner request only: catalog read and idle sample share the
    /// console lane. An older queued refresh cannot clear a newer hold.
    pub async fn refresh_snowflake_ddl_metadata(
        &self,
        session_id: &str,
    ) -> AppResult<SessionContextOptions> {
        let _recovery_activity = self.begin_recovery_activity()?;
        #[cfg(all(unix, feature = "test-harness"))]
        let reconciled = self
            .refresh_snowflake_ddl_metadata_qualification(session_id)
            .await;
        #[cfg(not(all(unix, feature = "test-harness")))]
        let reconciled = self.reconcile_snowflake_ddl_metadata(session_id).await;
        reconciled.map(|(options, _catalog)| options)
    }

    /// Preserve both original results from the same bounded reconciliation.
    #[cfg(all(unix, feature = "test-harness"))]
    pub(crate) async fn refresh_snowflake_ddl_metadata_qualification(
        &self,
        session_id: &str,
    ) -> AppResult<(SessionContextOptions, BrowseCatalog)> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.reconcile_snowflake_ddl_metadata(session_id).await
    }

    async fn reconcile_snowflake_ddl_metadata(
        &self,
        session_id: &str,
    ) -> AppResult<(SessionContextOptions, BrowseCatalog)> {
        let session = self.session(session_id)?;
        let revision = session.metadata_gate.lock().revision.clone();
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        session.metadata_gate.lock().check(Some(&revision), true)?;
        if !snowflake_dml::sdk_profile(&session.profile)
            || !session.metadata_gate.lock().catalog_required
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "No Snowflake DDL catalog reconciliation is pending",
            ));
        }
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
        *session.completion.lock() = None;
        self.plans
            .lock()
            .retain(|_, plan| plan.plan.session_id != session_id);
        self.ddl_preflights.lock().remove(session_id);
        session.driver.invalidate_relation_catalog();
        let reconciled = async {
            // A previous denied metadata read may have restricted the adapter
            // to observation/rollback recovery. Establish coherent idle state
            // explicitly before asking it to admit bounded catalog work.
            let before = self.build_session_contexts(&session).await?;
            if !coherent_idle(&before.state, session_id) {
                return Err(AppError::new(ErrorCode::Conflict,
                    "The session could not be coherently observed as idle before catalog reconciliation"));
            }
            let catalog =
                Self::publish_relation(&session, session.driver.relation_catalog().await)?;
            if catalog.relations.len() > 10_000
                || serde_json::to_vec(&catalog)
                    .map_err(|_| AppError::internal("Could not encode catalog reconciliation"))?
                    .len()
                    > 2 * 1024 * 1024
            {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "Catalog reconciliation exceeded its bound",
                ));
            }
            let options = self.build_session_contexts(&session).await?;
            if !coherent_idle(&options.state, session_id)
                || before.state.context_fingerprint != options.state.context_fingerprint
                || catalog.database != options.state.database
                || catalog.identity.database != options.state.database
                || Some(catalog.identity.role.as_str()) != options.state.role.as_deref()
                || catalog.identity.context_fingerprint != options.state.context_fingerprint
                || catalog.identity.search_path != options.state.search_path
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Catalog and idle session state could not be coherently reconciled",
                ));
            }
            let mut gate = session.metadata_gate.lock();
            gate.check(Some(&revision), true)?;
            gate.held = false;
            gate.catalog_required = false;
            Ok((options, catalog))
        }
        .await;
        if reconciled.is_err() {
            *session.observed_runtime.lock() = None;
            *session.completion.lock() = None;
            session.driver.invalidate_relation_catalog();
        }
        reconciled.map_err(AppError::with_completion_invalidated)
    }
}
