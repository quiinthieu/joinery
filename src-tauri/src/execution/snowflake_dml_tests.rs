// Included in browse_classification_tests so these exercise the real Studio
// review, configuration, persistence and serialized session lanes.
fn sdk_dml_fixture(
    mode: SafeMode,
    barrier: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
) -> (
    tempfile::TempDir,
    Arc<Studio>,
    Arc<RecordingDriver>,
    Arc<Session>,
) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(
        root.path(),
        Arc::new(crate::secrets::MemorySecrets::default()),
    )
    .unwrap();
    let mut profile = profile(EngineKind::Snowflake, mode);
    profile.database = "DB".into();
    profile.host = "org-account.snowflakecomputing.com".into();
    profile.port = 443;
    profile.tls_mode = TlsMode::VerifyFull;
    profile.snowflake = Some(SnowflakeConfig {
        authentication: SnowflakeAuthentication::ExternalBrowser,
        reviewed_dml: true,
        reviewed_ddl: false,
        reviewed_console: false,
        reviewed_console_timeout_seconds: None,
        role: "ROLE".into(),
        warehouse: "WH".into(),
        schema: "PUBLIC".into(),
    });
    studio.persistence.save_profile(&profile).unwrap();
    let id = new_id();
    let state = SessionRuntimeState {
        session_id: id.clone(),
        transaction_mode: TransactionMode::AutoCommit,
        transaction_state: TransactionState::Idle,
        context_kind: SessionContextKind::Schema,
        active_context: Some("PUBLIC".into()),
        database: "DB".into(),
        role: Some("ROLE".into()),
        search_path: vec!["DB.PUBLIC".into()],
        context_fingerprint: Some("synthetic-context".into()),
        transaction_observation: Some(TransactionObservation::Observed {
            transaction_id: None,
            abort_on_error: false,
            usability: TransactionUsability::Unknown,
        }),
        transaction_control_authority: Some(TransactionControlAuthority::Observed {
            revision: new_id(),
            allowed_actions: vec![
                ExplicitTransactionAction::Begin,
                ExplicitTransactionAction::Rollback,
            ],
        }),
    };
    let driver = Arc::new(RecordingDriver {
        context_enabled: true,
        runtime_override: Mutex::new(Some(state.clone())),
        dml_result: Mutex::new(Some(SnowflakeDmlResult {
            kind: SnowflakeDmlKind::Update,
            statement_outcome: SnowflakeDmlOutcome::Succeeded,
            summary: SnowflakeDmlSummary {
                status: SnowflakeDmlSummaryStatus::Validated,
                affected_rows: Some("9007199254740993".into()),
                multi_joined_rows: Some("7".into()),
            },
            state: state.clone(),
            retired: false,
            error: None,
        })),
        boundary_barrier: barrier,
        ..Default::default()
    });
    let session = Arc::new(Session {
        rollback_memory: Mutex::new(None),
        #[cfg(unix)]
        snowflake_external: None,
        info: SessionInfo {
            console_mode: false,
            id: id.clone(),
            connection_id: profile.id.clone(),
            engine: EngineKind::Snowflake,
            server_version: "synthetic".into(),
            latency_ms: 0,
            encrypted: true,
            capabilities: serde_json::from_value(serde_json::json!({"sql":true,
                "transactions":false,"explicitTransactionControls":true,"cancellation":true,
                "tableBrowsing":false,"relationBrowsing":true,"tableEditing":false,
                "tableFiltering":false,"processes":false}))
            .unwrap(),
        },
        profile,
        driver: driver.clone(),
        lane: tokio::sync::Mutex::new(()),
        cancellation_gate: tokio::sync::Mutex::new(()),
        active_run: Mutex::new(None),
        closed: AtomicBool::new(false),
        tunnel: None,
        completion: Mutex::new(None),
        metadata_gate: Mutex::new(MetadataGate::default()),
        observed_runtime: Mutex::new(Some(state)),
    });
    studio.sessions.lock().insert(id, session.clone());
    (root, studio, driver, session)
}

const SDK_DML_SQL: &str = "UPDATE DB.PUBLIC.T SET V = 2 WHERE V = 1";

fn assert_no_dml_or_observer(driver: &RecordingDriver) {
    assert!(driver.dml_calls.lock().is_empty());
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn dml_studio_policy_modes_require_exact_review_and_consume_once() {
    for mode in [
        SafeMode::ReadOnly,
        SafeMode::Confirm,
        SafeMode::Guard,
        SafeMode::Silent,
    ] {
        let (_root, studio, driver, session) = sdk_dml_fixture(mode, None);
        let prepared = studio.prepare_execution(&session.info.id, SDK_DML_SQL);
        if mode == SafeMode::ReadOnly {
            assert_eq!(prepared.unwrap_err().code, ErrorCode::PolicyBlocked);
            assert_no_dml_or_observer(&driver);
            continue;
        }
        let mut plan = prepared.unwrap();
        assert!(plan.reviewed_snowflake_dml);
        assert_eq!(plan.statements[0].sql, SDK_DML_SQL);
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("temporary objects"))
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("indirect effects"))
        );
        assert!(!plan.requires_passphrase);
        assert_eq!(plan.requires_approval, mode != SafeMode::Silent);
        let token = if plan.requires_approval {
            assert_eq!(
                studio
                    .execute_snowflake_dml(&plan.id, None)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::ApprovalRequired
            );
            assert_no_dml_or_observer(&driver);
            plan = studio
                .prepare_execution(&session.info.id, SDK_DML_SQL)
                .unwrap();
            Some(studio.authorize(&plan.id, None, None).unwrap())
        } else {
            None
        };
        let result = studio
            .execute_snowflake_dml(&plan.id, token.as_deref())
            .await
            .unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
        assert_eq!(
            result.summary.affected_rows.as_deref(),
            Some("9007199254740993")
        );
        assert_eq!(result.summary.multi_joined_rows.as_deref(), Some("7"));
        assert!(!result.retired);
        assert_eq!(driver.dml_calls.lock().len(), 1);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
        assert!(
            studio
                .execute_snowflake_dml(&plan.id, token.as_deref())
                .await
                .is_err()
        );
        assert_eq!(driver.dml_calls.lock().len(), 1);
    }
}

#[tokio::test]
async fn dml_studio_guard_and_production_require_confirmation_and_passphrase() {
    for production in [false, true] {
        let (_root, studio, driver, session) = sdk_dml_fixture(SafeMode::Guard, None);
        studio
            .set_passphrase("synthetic-dml-production-passphrase", None)
            .unwrap();
        if production {
            let mut profile = session.profile.clone();
            profile.environment = Environment::Production;
            studio.persistence.save_profile(&profile).unwrap();
        }
        let sql = "UPDATE DB.PUBLIC.T SET V = 2";
        let plan = studio.prepare_execution(&session.info.id, sql).unwrap();
        assert!(plan.statements[0].missing_where);
        assert_eq!(
            plan.confirmation_text.as_deref(),
            Some(session.profile.name.as_str())
        );
        assert_eq!(plan.requires_passphrase, production);
        assert!(
            studio
                .authorize(&plan.id, None, Some("wrong name"))
                .is_err()
        );
        if production {
            assert!(
                studio
                    .authorize(
                        &plan.id,
                        Some("wrong-passphrase"),
                        Some(&session.profile.name)
                    )
                    .is_err()
            );
        }
        assert_no_dml_or_observer(&driver);
        let token = studio
            .authorize(
                &plan.id,
                production.then_some("synthetic-dml-production-passphrase"),
                Some(&session.profile.name),
            )
            .unwrap();
        let replacement = studio.prepare_execution(&session.info.id, sql).unwrap();
        assert_eq!(
            studio
                .execute_snowflake_dml(&replacement.id, Some(&token))
                .await
                .unwrap_err()
                .code,
            ErrorCode::ApprovalRequired
        );
        assert_no_dml_or_observer(&driver);
        let plan = studio.prepare_execution(&session.info.id, sql).unwrap();
        let token = studio
            .authorize(
                &plan.id,
                production.then_some("synthetic-dml-production-passphrase"),
                Some(&session.profile.name),
            )
            .unwrap();
        assert_eq!(
            studio
                .execute_snowflake_dml(&plan.id, Some(&token))
                .await
                .unwrap()
                .statement_outcome,
            SnowflakeDmlOutcome::Succeeded
        );
    }
}

#[tokio::test]
async fn dml_studio_profile_and_stale_authority_changes_deny_before_dispatch() {
    for scenario in [
        "opt-out",
        "oauth",
        "read-only",
        "stronger-mode",
        "production",
        "revision",
        "rollback-only",
        "expired",
    ] {
        let (_root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
        let plan = studio
            .prepare_execution(&session.info.id, SDK_DML_SQL)
            .unwrap();
        let mut profile = session.profile.clone();
        match scenario {
            "opt-out" => profile.snowflake.as_mut().unwrap().reviewed_dml = false,
            "oauth" => {
                profile.snowflake.as_mut().unwrap().authentication =
                    SnowflakeAuthentication::LocalApplicationOauth
            }
            "read-only" => profile.safe_mode = SafeMode::ReadOnly,
            "stronger-mode" => profile.safe_mode = SafeMode::Confirm,
            "production" => profile.environment = Environment::Production,
            "revision" => {
                driver
                    .runtime_override
                    .lock()
                    .as_mut()
                    .unwrap()
                    .transaction_control_authority = Some(TransactionControlAuthority::Observed {
                    revision: new_id(),
                    allowed_actions: vec![],
                })
            }
            "rollback-only" => {
                driver
                    .runtime_override
                    .lock()
                    .as_mut()
                    .unwrap()
                    .transaction_control_authority =
                    Some(TransactionControlAuthority::RollbackOnly { revision: new_id() })
            }
            "expired" => studio.plans.lock().get_mut(&plan.id).unwrap().deadline = Instant::now(),
            _ => unreachable!(),
        }
        studio.persistence.save_profile(&profile).unwrap();
        assert!(
            studio.execute_snowflake_dml(&plan.id, None).await.is_err(),
            "{scenario}"
        );
        assert_no_dml_or_observer(&driver);
    }
}

#[tokio::test]
async fn dml_studio_generic_and_unsupported_sql_paths_never_submit_writes() {
    let (_root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
    for sql in [
        "BEGIN",
        "CREATE TABLE DB.PUBLIC.T (V NUMBER)",
        "MERGE INTO DB.PUBLIC.T USING DB.PUBLIC.S ON T.V = S.V WHEN MATCHED THEN DELETE",
        "UPDATE DB.PUBLIC.T SET V = PRIVATE_FN(V)",
        "DELETE FROM DB.PUBLIC.T; DELETE FROM DB.PUBLIC.T",
    ] {
        assert!(
            studio.prepare_execution(&session.info.id, sql).is_err(),
            "{sql}"
        );
    }
    let plan = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    assert!(studio.execute(&plan.id, None, Arc::new(|_| true)).is_err());
    let read = studio
        .prepare_execution(&session.info.id, "SELECT 1")
        .unwrap();
    assert!(!read.reviewed_snowflake_dml);
    assert!(studio.execute_snowflake_dml(&read.id, None).await.is_err());
    assert_no_dml_or_observer(&driver);
}

#[tokio::test]
async fn dml_studio_recompiles_original_binds_and_rejects_private_plan_tampering() {
    for tamper in [
        "none",
        "wire",
        "bind",
        "input",
        "database",
        "mode",
        "binding-digest",
    ] {
        let (_root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
        let parameters = vec![QueryParamInput {
            key: "value".into(),
            kind: QueryParamKind::Integer,
            value: Some("9007199254740997".into()),
        }];
        let sql = "UPDATE DB.PUBLIC.T SET V = :value WHERE V = :value";
        let plan = studio
            .prepare_execution_with_params(&session.info.id, sql, Some(parameters.clone()))
            .unwrap();
        {
            let mut plans = studio.plans.lock();
            let stored = plans.get_mut(&plan.id).unwrap();
            let Action::SnowflakeDml {
                request,
                bound,
                database,
                mode,
            } = &mut stored.action
            else {
                panic!("typed action");
            };
            match tamper {
                "none" => {}
                "wire" => bound.rewritten_sql.push_str(" OR TRUE"),
                "bind" => bound.binds[0].value = Some("3".into()),
                "input" => request.parameters[0].value = Some("3".into()),
                "database" => *database = "FOREIGN".into(),
                "mode" => *mode = SafeMode::Confirm,
                "binding-digest" => stored.plan.binding_digest = Some("forged".into()),
                _ => unreachable!(),
            }
        }
        let result = studio.execute_snowflake_dml(&plan.id, None).await;
        if tamper == "none" {
            assert_eq!(
                result.unwrap().statement_outcome,
                SnowflakeDmlOutcome::Succeeded
            );
            let calls = driver.dml_calls.lock();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].sql, sql);
            assert_eq!(calls[0].parameters, parameters);
            let history = studio.history("").unwrap();
            assert_eq!(history.len(), 1);
            let serialized = serde_json::to_string(&history).unwrap();
            assert!(!serialized.contains("9007199254740997") && !serialized.contains(":value"));
        } else {
            assert!(result.is_err(), "{tamper}");
            assert_no_dml_or_observer(&driver);
        }
    }
}

#[tokio::test]
async fn dml_studio_queued_review_remains_revocable_by_passphrase_and_profile_save() {
    for change in ["passphrase", "profile"] {
        let (_root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
        let plan = studio
            .prepare_execution(&session.info.id, SDK_DML_SQL)
            .unwrap();
        let lane = session.lane.lock().await;
        let owner = studio.clone();
        let task = tokio::spawn(async move { owner.execute_snowflake_dml(&plan.id, None).await });
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        if change == "passphrase" {
            studio
                .set_passphrase("synthetic-revoking-passphrase", None)
                .unwrap();
        } else {
            let mut profile = session.profile.clone();
            profile.snowflake.as_mut().unwrap().reviewed_dml = false;
            studio.save_profile(profile, None).unwrap();
        }
        drop(lane);
        assert!(task.await.unwrap().is_err());
        assert_no_dml_or_observer(&driver);
    }
}

#[tokio::test]
async fn dml_studio_ack_survives_late_disconnect_and_is_durable() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (_root, studio, driver, session) =
        sdk_dml_fixture(SafeMode::Silent, Some((entered.clone(), release.clone())));
    let plan = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    let owner = studio.clone();
    let task = tokio::spawn(async move { owner.execute_snowflake_dml(&plan.id, None).await });
    entered.notified().await;
    studio.disconnect(&session.info.id);
    release.notify_one();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert!(result.retired && result.state.transaction_control_authority.is_none());
    assert_eq!(
        result.summary.affected_rows.as_deref(),
        Some("9007199254740993")
    );
    let history = studio.history("").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, Some(RunStatus::Completed));
    let evidence = history[0].snowflake_dml.as_ref().unwrap();
    assert_eq!(evidence.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert_eq!(evidence.summary, result.summary);
    assert!(evidence.retired);
    assert_eq!(driver.dml_calls.lock().len(), 1);
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn dml_studio_dropped_ipc_waiter_still_finishes_durable_evidence() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (_root, studio, driver, session) =
        sdk_dml_fixture(SafeMode::Silent, Some((entered.clone(), release.clone())));
    let plan = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    let owner = studio.clone();
    let waiter = tokio::spawn(async move { owner.execute_snowflake_dml(&plan.id, None).await });
    entered.notified().await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    release.notify_one();
    let history = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let history = studio.history("").unwrap();
            if history.iter().any(|entry| entry.snowflake_dml.is_some()) {
                break history;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(history.len(), 1);
    let evidence = history[0].snowflake_dml.as_ref().unwrap();
    assert_eq!(evidence.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert_eq!(
        evidence.summary.affected_rows.as_deref(),
        Some("9007199254740993")
    );
    assert_eq!(driver.dml_calls.lock().len(), 1);
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn dml_studio_post_statement_audit_or_history_failure_cannot_erase_ack() {
    for table in ["audit", "history"] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (root, studio, driver, session) =
            sdk_dml_fixture(SafeMode::Silent, Some((entered.clone(), release.clone())));
        let plan = studio
            .prepare_execution(&session.info.id, SDK_DML_SQL)
            .unwrap();
        let owner = studio.clone();
        let task = tokio::spawn(async move { owner.execute_snowflake_dml(&plan.id, None).await });
        entered.notified().await;
        let database = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
        reject_receipt_write(&database, table);
        release.notify_one();
        let result = task.await.unwrap().unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
        assert_eq!(
            result.summary.affected_rows.as_deref(),
            Some("9007199254740993")
        );
        assert_eq!(result.error.unwrap().code, ErrorCode::Conflict);
        assert!(!result.retired);
        assert_eq!(driver.dml_calls.lock().len(), 1);
        let row = studio.history("").unwrap().remove(0);
        assert_eq!(row.provenance, HistoryProvenance::Admitted);
        assert!(row.snowflake_dml.is_none());
        assert!(
            studio
                .pending_snowflake_receipts
                .lock()
                .values()
                .next()
                .unwrap()
                .is_captured()
        );
        database
            .execute_batch(&format!("DROP TRIGGER reject_{table}"))
            .unwrap();
        studio.shutdown_and_wait().await.unwrap();
        assert_eq!(
            studio.history("").unwrap()[0]
                .snowflake_dml
                .as_ref()
                .unwrap()
                .statement_outcome,
            SnowflakeDmlOutcome::Succeeded
        );
        assert_eq!(terminal_audits(&database, "snowflake-dml", "succeeded"), 1);
    }
}

#[tokio::test]
async fn dml_studio_pre_statement_audit_failure_prevents_submission() {
    let (root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
    let plan = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    let database = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
    database.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit_v8 BEGIN SELECT RAISE(ABORT, 'synthetic audit failure'); END;").unwrap();
    assert!(studio.execute_snowflake_dml(&plan.id, None).await.is_err());
    assert_no_dml_or_observer(&driver);
}

#[tokio::test]
async fn dml_studio_unknown_outcome_retires_but_rejection_retains_recovery() {
    for outcome in [SnowflakeDmlOutcome::Rejected, SnowflakeDmlOutcome::Unknown] {
        let (_root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
        {
            let mut result = driver.dml_result.lock();
            let result = result.as_mut().unwrap();
            result.statement_outcome = outcome;
            result.summary = SnowflakeDmlSummary::unavailable();
            result.state.transaction_control_authority =
                Some(TransactionControlAuthority::RollbackOnly { revision: new_id() });
        }
        let plan = studio
            .prepare_execution(&session.info.id, SDK_DML_SQL)
            .unwrap();
        let result = studio.execute_snowflake_dml(&plan.id, None).await.unwrap();
        assert_eq!(result.statement_outcome, outcome);
        assert_eq!(result.retired, outcome == SnowflakeDmlOutcome::Unknown);
        assert_eq!(
            studio.session(&session.info.id).is_err(),
            outcome == SnowflakeDmlOutcome::Unknown
        );
        assert_eq!(
            result.state.transaction_control_authority.is_none(),
            outcome == SnowflakeDmlOutcome::Unknown
        );
        let history = studio.history("").unwrap();
        assert_eq!(
            history[0].snowflake_dml.as_ref().unwrap().statement_outcome,
            outcome
        );
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn dml_studio_native_hold_blocks_queued_and_late_automatic_metadata() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (_root, studio, driver, session) =
        sdk_dml_fixture(SafeMode::Silent, Some((entered.clone(), release.clone())));
    let plan = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    let owner = studio.clone();
    let task = tokio::spawn(async move { owner.execute_snowflake_dml(&plan.id, None).await });
    entered.notified().await;
    assert!(session.metadata_gate.lock().held);
    let health_owner = studio.clone();
    let health_id = session.info.id.clone();
    let health = tokio::spawn(async move { health_owner.health(&health_id).await });
    let metadata_owner = studio.clone();
    let metadata_id = session.info.id.clone();
    let metadata =
        tokio::spawn(async move { metadata_owner.completion_catalog(&metadata_id).await });
    tokio::task::yield_now().await;
    release.notify_one();
    assert_eq!(
        task.await.unwrap().unwrap().statement_outcome,
        SnowflakeDmlOutcome::Succeeded
    );
    assert_eq!(health.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(
        metadata.await.unwrap().unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio.health(&session.info.id).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(studio.relation_catalog(&session.info.id).await.is_err());
    assert!(studio.session_contexts(&session.info.id).await.is_err());
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
    assert_eq!(driver.dml_calls.lock().len(), 1);
    assert!(session.metadata_gate.lock().held);
    studio
        .session_contexts_with_refresh(&session.info.id, true)
        .await
        .unwrap();
    assert!(!session.metadata_gate.lock().held);
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 1);
    assert_eq!(studio.health(&session.info.id).await.unwrap(), 1);
}

#[tokio::test]
async fn dml_studio_queued_old_explicit_refresh_cannot_clear_new_hold() {
    let (_root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
    let lane = session.lane.lock().await;
    let owner = studio.clone();
    let id = session.info.id.clone();
    let refresh = tokio::spawn(async move { owner.session_contexts_with_refresh(&id, true).await });
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    session.metadata_gate.lock().hold();
    drop(lane);
    assert_eq!(
        refresh.await.unwrap().unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    assert!(session.metadata_gate.lock().held);
}

#[tokio::test]
async fn dml_studio_unavailable_explicit_refresh_does_not_resume_metadata() {
    let (_root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
    session.metadata_gate.lock().hold();
    {
        let mut runtime = driver.runtime_override.lock();
        let runtime = runtime.as_mut().unwrap();
        runtime.transaction_observation = Some(TransactionObservation::Unavailable);
        runtime.transaction_control_authority = None;
    }
    let _ = studio
        .session_contexts_with_refresh(&session.info.id, true)
        .await;
    assert!(session.metadata_gate.lock().held);
    assert_eq!(
        studio.health(&session.info.id).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn dml_studio_shutdown_waits_for_detached_owner_and_durable_ack_history() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (_root, studio, driver, session) =
        sdk_dml_fixture(SafeMode::Silent, Some((entered.clone(), release.clone())));
    let plan = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    let owner = studio.clone();
    let waiter = tokio::spawn(async move { owner.execute_snowflake_dml(&plan.id, None).await });
    // RecordingDriver clones the successful statement evidence before this
    // barrier, then holds completion so the owner has not persisted it yet.
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let pending = studio.history("").unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].provenance, HistoryProvenance::Admitted);
    assert!(pending[0].status.is_none());
    assert!(pending[0].snowflake_dml.is_none());

    let shutdown_owner = studio.clone();
    let mut shutdown = tokio::spawn(async move { shutdown_owner.shutdown_and_wait().await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !session.closed.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Let shutdown reach its drain while the owner is deterministically held.
    // Capture an early receipt, then release and reap before asserting, so even
    // the failing regression leaves no deliberately stalled task behind.
    let early_receipt = tokio::time::timeout(Duration::from_millis(50), &mut shutdown).await;
    let pending = studio.history("").unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].provenance, HistoryProvenance::Admitted);
    assert!(pending[0].status.is_none());
    assert!(pending[0].snowflake_dml.is_none());
    assert!(
        studio
            .prepare_execution(&session.info.id, SDK_DML_SQL)
            .is_err()
    );
    assert_eq!(
        studio
            .prepare_connection(&session.profile.id)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );

    release.notify_one();
    let history_at_shutdown_receipt = if early_receipt.is_err() {
        tokio::time::timeout(Duration::from_secs(3), &mut shutdown)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        Some(studio.history("").unwrap())
    } else {
        None
    };
    let history = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let history = studio.history("").unwrap();
            if history.iter().any(|entry| entry.snowflake_dml.is_some()) {
                break history;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, Some(RunStatus::Completed));
    let evidence = history[0].snowflake_dml.as_ref().unwrap();
    assert_eq!(evidence.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert_eq!(
        evidence.summary.affected_rows.as_deref(),
        Some("9007199254740993")
    );
    assert_eq!(evidence.summary.multi_joined_rows.as_deref(), Some("7"));
    assert!(evidence.retired);
    assert!(studio.sessions.lock().is_empty());
    assert_eq!(driver.dml_calls.lock().len(), 1);
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    assert!(
        early_receipt.is_err(),
        "shutdown_and_wait returned before the detached DML owner persisted its ACK history"
    );
    assert_eq!(history_at_shutdown_receipt.unwrap().len(), 1);
    assert_eq!(
        studio
            .execute_snowflake_dml("late-write-after-shutdown", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ConnectionClosed
    );
}

#[tokio::test]
async fn receipt_dml_dropped_waiter_storage_failure_blocks_shutdown_until_local_retry() {
    for tables in [&["history"][..], &["audit"][..], &["audit", "history"][..]] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (root, studio, driver, session) =
            sdk_dml_fixture(SafeMode::Silent, Some((entered.clone(), release.clone())));
        let plan = studio
            .prepare_snowflake_dml(&session.info.id, SDK_DML_SQL, None)
            .unwrap();
        let owner = studio.clone();
        let waiter = tokio::spawn(async move { owner.execute_snowflake_dml(&plan.id, None).await });
        tokio::time::timeout(Duration::from_secs(3), entered.notified())
            .await
            .unwrap();
        let database = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
        for table in tables {
            reject_receipt_write(&database, table);
        }
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        release.notify_one();
        assert_eq!(
            studio.shutdown_and_wait().await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert!(studio.shutdown_and_wait().await.is_err());
        for table in tables {
            database
                .execute_batch(&format!("DROP TRIGGER reject_{table};"))
                .unwrap();
        }
        studio.shutdown_and_wait().await.unwrap();
        studio.shutdown_and_wait().await.unwrap();
        let history = studio.history("").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0].snowflake_dml.as_ref().unwrap().statement_outcome,
            SnowflakeDmlOutcome::Succeeded
        );
        let audits: u32 = database
            .query_row(
                "SELECT COUNT(*) FROM audit_v8 WHERE action='snowflake-dml' AND outcome='succeeded'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1);
        assert_eq!(driver.dml_calls.lock().len(), 1);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn receipt_boundary_terminal_audit_failure_preserves_ack_and_blocks_shutdown() {
    let (root, studio, driver, session, request) =
        explicit_control_fixture(SafeMode::Silent, ExplicitTransactionAction::Begin, None);
    let plan = studio
        .prepare_transaction_control(&session.info.id, &request)
        .unwrap();
    let database = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
    database.execute_batch("CREATE TRIGGER reject_terminal BEFORE INSERT ON audit_v8 WHEN NEW.outcome != 'started' BEGIN SELECT RAISE(ABORT, 'synthetic local failure'); END;").unwrap();
    let result = studio
        .execute_transaction_control(&plan.id, None)
        .await
        .unwrap();
    assert_eq!(
        result.boundary_outcome,
        TransactionBoundaryOutcome::Acknowledged
    );
    assert!(
        result.error.is_some(),
        "The known ACK must carry a local-recording notice"
    );
    assert_eq!(
        studio.shutdown_and_wait().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    database
        .execute_batch("DROP TRIGGER reject_terminal;")
        .unwrap();
    studio.shutdown_and_wait().await.unwrap();
    studio.shutdown_and_wait().await.unwrap();
    let audits: u32 = database.query_row("SELECT COUNT(*) FROM audit_v8 WHERE action='transaction-boundary' AND outcome='acknowledged'", [], |row| row.get(0)).unwrap();
    assert_eq!(audits, 1);
    assert_eq!(driver.boundary_calls.lock().len(), 1);
}
