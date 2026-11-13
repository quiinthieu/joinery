// Included beside the DML fixture so real policy, persistence and lanes apply.
fn sdk_ddl_fixture(
    mode: SafeMode,
    drop_table: bool,
    barrier: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
) -> (
    tempfile::TempDir,
    Arc<Studio>,
    Arc<RecordingDriver>,
    Arc<Session>,
    SnowflakeDdlPreflightRequest,
) {
    let (root, studio, driver, old) = sdk_dml_fixture(mode, barrier);
    let mut profile = old.profile.clone();
    profile.snowflake.as_mut().unwrap().reviewed_ddl = true;
    studio.persistence.save_profile(&profile).unwrap();
    let mut info = old.info.clone();
    info.capabilities.snowflake_reviewed_ddl = true;
    let session = Arc::new(Session {
        rollback_memory: Mutex::new(None),
        info,
        profile,
        driver: driver.clone(),
        lane: tokio::sync::Mutex::new(()),
        cancellation_gate: tokio::sync::Mutex::new(()),
        active_run: Mutex::new(None),
        closed: AtomicBool::new(false),
        tunnel: None,
        completion: Mutex::new(None),
        metadata_gate: Mutex::new(MetadataGate::default()),
        observed_runtime: Mutex::new(old.observed_runtime.lock().clone()),
    });
    studio
        .sessions
        .lock()
        .insert(session.info.id.clone(), session.clone());
    let action = if drop_table {
        SnowflakeDdlAction::DropTable {
            browse_revision: new_id(),
            target_id: new_id(),
        }
    } else {
        SnowflakeDdlAction::CreateTable {
            database: "DB".into(),
            schema: "PUBLIC".into(),
            name: "T".into(),
            columns: vec![SnowflakeDdlColumnDraft {
                name: "V".into(),
                data_type: "NUMBER(10, 0)".into(),
                nullable: true,
            }],
        }
    };
    let request = SnowflakeDdlPreflightRequest {
        session_id: session.info.id.clone(),
        action,
    };
    let action = if drop_table {
        crate::ddl::TableObjectAction::DropObject {
            object: DatabaseObjectRef {
                schema: "PUBLIC".into(),
                name: "T".into(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
            cascade: false,
        }
    } else {
        crate::ddl::TableObjectAction::CreateTable {
            table: TableRef {
                schema: "PUBLIC".into(),
                name: "T".into(),
            },
            columns: vec![
                SnowflakeDdlColumnDraft {
                    name: "V".into(),
                    data_type: "NUMBER(10, 0)".into(),
                    nullable: true,
                }
                .to_table_column_draft(),
            ],
            comment: None,
        }
    };
    let canonical = safety::snowflake_ddl::compile(&action, "DB").unwrap();
    let response = SnowflakeDdlPreflightResponse {
        revision: new_id(),
        kind: canonical.kind().into(),
        database: "DB".into(),
        schema: "PUBLIC".into(),
        name: "T".into(),
        qualified_target: "\"DB\".\"PUBLIC\".\"T\"".into(),
        sql: canonical.sql().into(),
        warnings: vec![],
    };
    *driver.ddl_preflight.lock() = Some((request.clone(), response.clone()));
    let state = driver.runtime_override.lock().clone().unwrap();
    *driver.ddl_result.lock() = Some(SnowflakeDdlResult {
        kind: response.kind,
        qualified_target: response.qualified_target,
        statement_outcome: SnowflakeDdlOutcome::Succeeded,
        summary_status: SnowflakeDdlSummaryStatus::Acknowledged,
        catalog_reconciliation_required: true,
        state: state.clone(),
        retired: false,
        error: None,
    });
    *driver.relation_result.lock() = Some(Ok(BrowseCatalog {
        revision: new_id(),
        database: state.database.clone(),
        relations: vec![],
        identity: CompletionIdentity {
            database: state.database,
            role: state.role.unwrap(),
            search_path: state.search_path,
            case_sensitive_table_names: Some(true),
            context_fingerprint: state.context_fingerprint,
        },
    }));
    (root, studio, driver, session, request)
}

async fn ddl_review(studio: &Studio, request: &SnowflakeDdlPreflightRequest) -> PreparedPlan {
    let response = studio.preflight_snowflake_ddl(request).await.unwrap();
    studio
        .prepare_snowflake_ddl(
            &request.session_id,
            &SnowflakeDdlRequest {
                revision: response.revision,
                action: request.action.clone(),
                sql: response.sql,
            },
        )
        .unwrap()
}

fn ddl_approve(studio: &Studio, plan: &PreparedPlan) -> String {
    studio
        .authorize_snowflake_ddl(
            &plan.id,
            None,
            plan.confirmation_text.as_deref(),
            plan.target_confirmation_text.as_deref(),
        )
        .unwrap()
}

#[tokio::test]
async fn ddl_studio_replaced_plan_cannot_reuse_approval_or_unobserved_request() {
    let (_root, studio, driver, session, request) = sdk_ddl_fixture(SafeMode::Silent, false, None);
    let response = driver.ddl_preflight.lock().as_ref().unwrap().1.clone();
    let unobserved = SnowflakeDdlRequest {
        revision: response.revision,
        action: request.action.clone(),
        sql: response.sql,
    };
    assert!(
        studio
            .prepare_snowflake_ddl(&session.info.id, &unobserved)
            .is_err()
    );
    let plan = ddl_review(&studio, &request).await;
    let token = ddl_approve(&studio, &plan);
    let replacement = ddl_review(&studio, &request).await;
    assert!(
        studio
            .execute_snowflake_ddl(&plan.id, Some(&token))
            .await
            .is_err()
    );
    assert_eq!(
        studio
            .execute_snowflake_ddl(&replacement.id, Some(&token))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ApprovalRequired
    );
    assert!(driver.ddl_calls.lock().is_empty());
}

#[tokio::test]
async fn ddl_studio_failed_start_audit_does_not_dispatch_or_claim_terminal_outcome() {
    let (root, studio, driver, session, request) = sdk_ddl_fixture(SafeMode::Silent, false, None);
    let plan = ddl_review(&studio, &request).await;
    let token = ddl_approve(&studio, &plan);
    let database = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
    database.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit_v8 BEGIN SELECT RAISE(ABORT, 'synthetic audit failure'); END;").unwrap();
    assert!(
        studio
            .execute_snowflake_ddl(&plan.id, Some(&token))
            .await
            .is_err()
    );
    assert!(driver.ddl_calls.lock().is_empty());
    assert!(studio.history("").unwrap().is_empty());
    assert!(!session.metadata_gate.lock().held);
}

#[tokio::test]
async fn ddl_studio_modes_always_require_exact_one_use_approval() {
    for mode in [
        SafeMode::ReadOnly,
        SafeMode::Confirm,
        SafeMode::Guard,
        SafeMode::Silent,
    ] {
        let (_root, studio, driver, _session, request) = sdk_ddl_fixture(mode, false, None);
        if mode == SafeMode::ReadOnly {
            assert_eq!(
                studio
                    .preflight_snowflake_ddl(&request)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::PolicyBlocked
            );
            continue;
        }
        let plan = ddl_review(&studio, &request).await;
        assert!(plan.reviewed_snowflake_ddl && plan.requires_approval);
        assert!(
            studio
                .authorize(&plan.id, None, plan.confirmation_text.as_deref())
                .is_err()
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("cannot be rolled back"))
        );
        assert_eq!(
            studio
                .execute_snowflake_ddl(&plan.id, None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::ApprovalRequired
        );
        assert!(driver.ddl_calls.lock().is_empty());
        let plan = ddl_review(&studio, &request).await;
        let token = ddl_approve(&studio, &plan);
        let result = studio
            .execute_snowflake_ddl(&plan.id, Some(&token))
            .await
            .unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
        assert!(
            studio
                .execute_snowflake_ddl(&plan.id, Some(&token))
                .await
                .is_err()
        );
        assert_eq!(driver.ddl_calls.lock().len(), 1);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
        assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
        let history = studio.history("").unwrap();
        assert!(history[0].sql.is_none());
        assert_eq!(
            history[0].snowflake_ddl.as_ref().unwrap().statement_outcome,
            SnowflakeDdlOutcome::Succeeded
        );
    }
}

#[tokio::test]
async fn ddl_studio_drop_requires_independent_exact_target_and_policy_confirmations() {
    let (_root, studio, driver, session, request) = sdk_ddl_fixture(SafeMode::Guard, true, None);
    let plan = ddl_review(&studio, &request).await;
    assert_eq!(
        plan.confirmation_text.as_deref(),
        Some(session.profile.name.as_str())
    );
    assert_eq!(
        plan.target_confirmation_text.as_deref(),
        Some("\"DB\".\"PUBLIC\".\"T\"")
    );
    assert!(
        studio
            .authorize(&plan.id, None, plan.confirmation_text.as_deref())
            .is_err()
    );
    for (connection, target) in [
        (None, plan.target_confirmation_text.as_deref()),
        (plan.confirmation_text.as_deref(), None),
        (plan.confirmation_text.as_deref(), Some("DB.PUBLIC.T")),
    ] {
        assert_eq!(
            studio
                .authorize_snowflake_ddl(&plan.id, None, connection, target)
                .unwrap_err()
                .code,
            ErrorCode::ApprovalRequired
        );
    }
    let token = ddl_approve(&studio, &plan);
    assert!(
        studio
            .execute(&plan.id, Some(&token), Arc::new(|_| true))
            .is_err()
    );
    assert!(driver.ddl_calls.lock().is_empty());
    let plan = ddl_review(&studio, &request).await;
    let token = ddl_approve(&studio, &plan);
    studio
        .execute_snowflake_ddl(&plan.id, Some(&token))
        .await
        .unwrap();
    assert_eq!(driver.ddl_calls.lock().len(), 1);
}

#[tokio::test]
async fn ddl_studio_password_gate_cannot_be_skipped() {
    let (_root, studio, driver, _session, request) =
        sdk_ddl_fixture(SafeMode::PasswordGated, true, None);
    studio
        .set_passphrase("synthetic-ddl-passphrase", None)
        .unwrap();
    let plan = ddl_review(&studio, &request).await;
    assert!(plan.requires_passphrase);
    assert!(
        studio
            .authorize_snowflake_ddl(
                &plan.id,
                None,
                plan.confirmation_text.as_deref(),
                plan.target_confirmation_text.as_deref()
            )
            .is_err()
    );
    assert!(
        studio
            .authorize_snowflake_ddl(
                &plan.id,
                Some("wrong"),
                plan.confirmation_text.as_deref(),
                plan.target_confirmation_text.as_deref()
            )
            .is_err()
    );
    assert!(driver.ddl_calls.lock().is_empty());
    let token = studio
        .authorize_snowflake_ddl(
            &plan.id,
            Some("synthetic-ddl-passphrase"),
            plan.confirmation_text.as_deref(),
            plan.target_confirmation_text.as_deref(),
        )
        .unwrap();
    studio
        .execute_snowflake_ddl(&plan.id, Some(&token))
        .await
        .unwrap();
}

#[tokio::test]
async fn ddl_studio_fresh_policy_and_preflight_binding_deny_drift() {
    for scenario in [
        "opt-out",
        "oauth",
        "read-only",
        "mode",
        "production",
        "name",
        "revision",
        "sql",
        "action",
        "expired",
    ] {
        let (_root, studio, driver, session, request) =
            sdk_ddl_fixture(SafeMode::Silent, false, None);
        let plan = ddl_review(&studio, &request).await;
        let token = ddl_approve(&studio, &plan);
        let mut profile = session.profile.clone();
        match scenario {
            "opt-out" => profile.snowflake.as_mut().unwrap().reviewed_ddl = false,
            "oauth" => {
                profile.snowflake.as_mut().unwrap().authentication =
                    SnowflakeAuthentication::LocalApplicationOauth
            }
            "read-only" => profile.safe_mode = SafeMode::ReadOnly,
            "mode" => profile.safe_mode = SafeMode::Guard,
            "production" => profile.environment = Environment::Production,
            "name" => {
                profile.name = "Changed".into();
                profile.safe_mode = SafeMode::Guard;
            }
            "revision" => driver.ddl_preflight.lock().as_mut().unwrap().1.revision = new_id(),
            "sql" => {
                let mut plans = studio.plans.lock();
                let Action::SnowflakeDdl { request, .. } =
                    &mut plans.get_mut(&plan.id).unwrap().action
                else {
                    unreachable!()
                };
                request.sql.push_str("; DROP DATABASE D");
            }
            "action" => {
                let mut plans = studio.plans.lock();
                let Action::SnowflakeDdl { request, .. } =
                    &mut plans.get_mut(&plan.id).unwrap().action
                else {
                    unreachable!()
                };
                let SnowflakeDdlAction::CreateTable { name, .. } = &mut request.action else {
                    unreachable!()
                };
                *name = "OTHER".into();
            }
            "expired" => {
                studio.plans.lock().get_mut(&plan.id).unwrap().deadline =
                    Instant::now() - Duration::from_secs(1)
            }
            _ => unreachable!(),
        }
        studio.persistence.save_profile(&profile).unwrap();
        assert!(
            studio
                .execute_snowflake_ddl(&plan.id, Some(&token))
                .await
                .is_err(),
            "{scenario}"
        );
        assert!(driver.ddl_calls.lock().is_empty(), "{scenario}");
    }
}

#[tokio::test]
async fn ddl_studio_hold_precedes_dispatch_and_only_catalog_reconciliation_clears_it() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (_root, studio, driver, session, request) = sdk_ddl_fixture(
        SafeMode::Silent,
        false,
        Some((entered.clone(), release.clone())),
    );
    let plan = ddl_review(&studio, &request).await;
    let token = ddl_approve(&studio, &plan);
    let owner = studio.clone();
    let task =
        tokio::spawn(async move { owner.execute_snowflake_ddl(&plan.id, Some(&token)).await });
    entered.notified().await;
    assert!(session.metadata_gate.lock().held);
    assert!(session.metadata_gate.lock().catalog_required);
    release.notify_one();
    task.await.unwrap().unwrap();
    assert!(
        studio
            .session_contexts_with_refresh(&session.info.id, true)
            .await
            .is_err()
    );
    assert!(studio.health(&session.info.id).await.is_err());
    assert!(studio.completion_catalog(&session.info.id).await.is_err());
    assert!(studio.relation_catalog(&session.info.id).await.is_err());
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
    studio
        .refresh_snowflake_ddl_metadata(&session.info.id)
        .await
        .unwrap();
    assert!(!session.metadata_gate.lock().held);
    assert!(!session.metadata_gate.lock().catalog_required);
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 2);
    assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn ddl_studio_refresh_preserves_options_only_api_and_single_reconciliation() {
    let (_root, studio, driver, session, _) = sdk_ddl_fixture(SafeMode::Confirm, false, None);
    let expected_state = driver.runtime_override.lock().clone().unwrap();
    session.metadata_gate.lock().hold_catalog();

    let options: SessionContextOptions = studio
        .refresh_snowflake_ddl_metadata(&session.info.id)
        .await
        .unwrap();

    assert_eq!(
        serde_json::to_value(&options.state).unwrap(),
        serde_json::to_value(&expected_state).unwrap()
    );
    assert_eq!(options.available, vec!["PUBLIC", "public"]);
    assert!(options.revision.is_none());
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 2);
    assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 1);
    assert!(driver.relation_invalidated.load(Ordering::Acquire));
    assert!(!session.metadata_gate.lock().held);
    assert!(!session.metadata_gate.lock().catalog_required);
}

#[cfg(all(unix, feature = "test-harness"))]
#[tokio::test]
async fn ddl_studio_qualification_refresh_returns_original_same_call_catalog() {
    use crate::relation_inspection::{RelationAvailability, RelationTargetSummary, RelationTraits};

    let (_root, studio, driver, session, _) = sdk_ddl_fixture(SafeMode::Confirm, false, None);
    let mut expected_catalog = driver.relation_result.lock().clone().unwrap().unwrap();
    expected_catalog.revision = "same-call-catalog-revision".into();
    expected_catalog.relations.push(RelationTargetSummary {
        target_id: "same-call-target-id".into(),
        database: "DB".into(),
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "T".into(),
        },
        table_type: "BASE TABLE".into(),
        traits: RelationTraits {
            transient: Some(false),
            temporary: Some(false),
            iceberg: Some(false),
            dynamic: Some(false),
            hybrid: Some(false),
            immutable: Some(false),
        },
        availability: RelationAvailability::Available,
    });
    *driver.relation_result.lock() = Some(Ok(expected_catalog.clone()));
    let expected_state = driver.runtime_override.lock().clone().unwrap();
    session.metadata_gate.lock().hold_catalog();

    let (options, catalog) = studio
        .refresh_snowflake_ddl_metadata_qualification(&session.info.id)
        .await
        .unwrap();

    assert_eq!(
        serde_json::to_value(&catalog).unwrap(),
        serde_json::to_value(&expected_catalog).unwrap(),
        "the original catalog revision, target IDs, identity and traits survive reconciliation"
    );
    assert_eq!(
        serde_json::to_value(&options.state).unwrap(),
        serde_json::to_value(&expected_state).unwrap()
    );
    assert_eq!(options.available, vec!["PUBLIC", "public"]);
    assert!(options.revision.is_none());
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 2);
    assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 1);
    assert!(!session.metadata_gate.lock().held);
    assert!(!session.metadata_gate.lock().catalog_required);

    assert!(
        studio
            .refresh_snowflake_ddl_metadata_qualification(&session.info.id)
            .await
            .is_err(),
        "returning catalog evidence does not bypass the pending-hold requirement"
    );
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 2);
    assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn ddl_studio_failed_or_incoherent_catalog_reconciliation_retains_hold() {
    for scenario in [
        "catalog-denied",
        "context-denied",
        "manual",
        "open",
        "unknown",
        "rollback-only",
        "database",
        "fingerprint",
        "oversize",
    ] {
        let (_root, studio, driver, session, _) = sdk_ddl_fixture(SafeMode::Silent, false, None);
        session.metadata_gate.lock().hold_catalog();
        match scenario {
            "catalog-denied" => {
                *driver.relation_result.lock() = Some(Err(AppError::new(
                    ErrorCode::PermissionDenied,
                    "Fixture denied",
                )))
            }
            "context-denied" => {
                driver
                    .runtime_override
                    .lock()
                    .as_mut()
                    .unwrap()
                    .transaction_observation = Some(TransactionObservation::Unavailable);
            }
            "manual" => {
                driver
                    .runtime_override
                    .lock()
                    .as_mut()
                    .unwrap()
                    .transaction_mode = TransactionMode::Manual
            }
            "open" => {
                driver
                    .runtime_override
                    .lock()
                    .as_mut()
                    .unwrap()
                    .transaction_state = TransactionState::InTransaction
            }
            "unknown" => {
                driver
                    .runtime_override
                    .lock()
                    .as_mut()
                    .unwrap()
                    .transaction_state = TransactionState::Unknown
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
            "database" => {
                driver
                    .relation_result
                    .lock()
                    .as_mut()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .database = "OTHER".into()
            }
            "fingerprint" => {
                driver
                    .relation_result
                    .lock()
                    .as_mut()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .identity
                    .context_fingerprint = Some("OTHER".into())
            }
            "oversize" => {
                driver
                    .relation_result
                    .lock()
                    .as_mut()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .database = "X".repeat(2 * 1024 * 1024)
            }
            _ => unreachable!(),
        }
        assert!(
            studio
                .refresh_snowflake_ddl_metadata(&session.info.id)
                .await
                .is_err(),
            "{scenario}"
        );
        assert!(session.metadata_gate.lock().held, "{scenario}");
        assert!(session.metadata_gate.lock().catalog_required, "{scenario}");
        assert!(session.observed_runtime.lock().is_none());
    }
}

#[tokio::test]
async fn ddl_studio_old_queued_refresh_cannot_clear_new_catalog_hold() {
    for catalog in [false, true] {
        let (_root, studio, driver, session, _) = sdk_ddl_fixture(SafeMode::Silent, false, None);
        session.metadata_gate.lock().hold_catalog();
        let lane = session.lane.lock().await;
        let owner = studio.clone();
        let id = session.info.id.clone();
        let refresh = tokio::spawn(async move {
            if catalog {
                owner.refresh_snowflake_ddl_metadata(&id).await
            } else {
                owner.session_contexts_with_refresh(&id, true).await
            }
        });
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        session.metadata_gate.lock().hold_catalog();
        drop(lane);
        assert_eq!(
            refresh.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict
        );
        assert!(session.metadata_gate.lock().held);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
        assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn ddl_studio_unknown_retires_and_rejected_ack_stays_truthful() {
    for outcome in [SnowflakeDdlOutcome::Rejected, SnowflakeDdlOutcome::Unknown] {
        let (_root, studio, driver, session, request) =
            sdk_ddl_fixture(SafeMode::Silent, false, None);
        driver.ddl_result.lock().as_mut().unwrap().statement_outcome = outcome;
        let plan = ddl_review(&studio, &request).await;
        let token = ddl_approve(&studio, &plan);
        let result = studio
            .execute_snowflake_ddl(&plan.id, Some(&token))
            .await
            .unwrap();
        assert_eq!(result.statement_outcome, outcome);
        assert_eq!(result.retired, outcome == SnowflakeDdlOutcome::Unknown);
        assert_eq!(
            studio.session(&session.info.id).is_err(),
            outcome == SnowflakeDdlOutcome::Unknown
        );
        assert_eq!(
            studio.history("").unwrap()[0]
                .snowflake_ddl
                .as_ref()
                .unwrap()
                .statement_outcome,
            outcome
        );
        assert!(session.metadata_gate.lock().catalog_required);
    }
}

#[tokio::test]
async fn ddl_studio_late_disconnect_or_persistence_failure_preserves_success() {
    for scenario in ["disconnect", "audit", "history"] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (root, studio, driver, session, request) = sdk_ddl_fixture(
            SafeMode::Silent,
            false,
            Some((entered.clone(), release.clone())),
        );
        let plan = ddl_review(&studio, &request).await;
        let token = ddl_approve(&studio, &plan);
        let owner = studio.clone();
        let task =
            tokio::spawn(async move { owner.execute_snowflake_ddl(&plan.id, Some(&token)).await });
        entered.notified().await;
        if scenario == "disconnect" {
            studio.disconnect(&session.info.id);
        } else {
            let db = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
            reject_receipt_write(&db, scenario);
        }
        release.notify_one();
        let result = task.await.unwrap().unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
        assert_eq!(
            result.summary_status,
            SnowflakeDdlSummaryStatus::Acknowledged
        );
        assert_eq!(result.retired, scenario == "disconnect");
        assert_eq!(result.error.is_some(), scenario != "disconnect");
        assert_eq!(driver.ddl_calls.lock().len(), 1);
        if scenario == "disconnect" {
            assert_eq!(
                studio.history("").unwrap()[0]
                    .snowflake_ddl
                    .as_ref()
                    .unwrap()
                    .statement_outcome,
                SnowflakeDdlOutcome::Succeeded
            );
        } else {
            let row = studio.history("").unwrap().remove(0);
            assert_eq!(row.provenance, HistoryProvenance::Admitted);
            assert!(row.snowflake_ddl.is_none());
            assert!(
                studio
                    .pending_snowflake_receipts
                    .lock()
                    .values()
                    .next()
                    .unwrap()
                    .is_captured()
            );
            let db = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
            db.execute_batch(&format!("DROP TRIGGER reject_{scenario}"))
                .unwrap();
            studio.shutdown_and_wait().await.unwrap();
            assert_eq!(
                studio.history("").unwrap()[0]
                    .snowflake_ddl
                    .as_ref()
                    .unwrap()
                    .statement_outcome,
                SnowflakeDdlOutcome::Succeeded
            );
            assert_eq!(terminal_audits(&db, "snowflake-ddl", "succeeded"), 1);
        }
    }
}
#[tokio::test]
async fn ddl_studio_dropped_waiter_and_shutdown_drain_durable_ack() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (_root, studio, driver, session, request) = sdk_ddl_fixture(
        SafeMode::Silent,
        false,
        Some((entered.clone(), release.clone())),
    );
    let plan = ddl_review(&studio, &request).await;
    let token = ddl_approve(&studio, &plan);
    let owner = studio.clone();
    let waiter =
        tokio::spawn(async move { owner.execute_snowflake_ddl(&plan.id, Some(&token)).await });
    entered.notified().await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let owner = studio.clone();
    let mut shutdown = tokio::spawn(async move { owner.shutdown_and_wait().await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !session.closed.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
            .await
            .is_err()
    );
    let pending = studio.history("").unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].provenance, HistoryProvenance::Admitted);
    assert!(pending[0].status.is_none());
    assert!(pending[0].snowflake_ddl.is_none());
    release.notify_one();
    shutdown.await.unwrap().unwrap();
    let history = studio.history("").unwrap();
    assert_eq!(history.len(), 1);
    let evidence = history[0].snowflake_ddl.as_ref().unwrap();
    assert_eq!(evidence.statement_outcome, SnowflakeDdlOutcome::Succeeded);
    assert!(evidence.retired);
    assert_eq!(driver.ddl_calls.lock().len(), 1);
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        studio
            .execute_snowflake_ddl("late", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ConnectionClosed
    );
}

#[tokio::test]
async fn ddl_studio_dropped_waiter_storage_failure_retains_ack_and_blocks_shutdown_until_local_retry()
 {
    for failed_tables in [&["history"][..], &["audit"][..], &["audit", "history"][..]] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (root, studio, driver, _session, request) = sdk_ddl_fixture(
            SafeMode::Silent,
            false,
            Some((entered.clone(), release.clone())),
        );
        let plan = ddl_review(&studio, &request).await;
        let token = ddl_approve(&studio, &plan);
        let owner = studio.clone();
        let waiter =
            tokio::spawn(async move { owner.execute_snowflake_ddl(&plan.id, Some(&token)).await });
        entered.notified().await;
        let database = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
        for table in failed_tables {
            reject_receipt_write(&database, table);
        }
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        release.notify_one();
        let error = studio.shutdown_and_wait().await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        let original = {
            let pending = studio.pending_snowflake_receipts.lock();
            assert_eq!(pending.len(), 1);
            let receipt = pending.values().next().unwrap();
            let payload: serde_json::Value =
                serde_json::from_slice(&receipt.original_bytes()).unwrap();
            let completion: HistoryCompletion =
                serde_json::from_value(payload["completion"].clone()).unwrap();
            let evidence = completion.snowflake_ddl.as_ref().unwrap();
            assert_eq!(evidence.statement_outcome, SnowflakeDdlOutcome::Succeeded);
            assert_eq!(evidence.qualified_target, "\"DB\".\"PUBLIC\".\"T\"");
            assert!(receipt.is_captured());
            (receipt.ticket().id().to_owned(), completion)
        };
        assert_eq!(driver.ddl_calls.lock().len(), 1);
        // Repeated close attempts keep the exact receipt and never submit again.
        assert!(studio.shutdown_and_wait().await.is_err());
        assert_eq!(
            studio
                .pending_snowflake_receipts
                .lock()
                .values()
                .next()
                .unwrap()
                .ticket()
                .id(),
            original.0
        );
        for table in failed_tables {
            database
                .execute_batch(&format!("DROP TRIGGER reject_{table};"))
                .unwrap();
        }
        studio.shutdown_and_wait().await.unwrap();
        studio.shutdown_and_wait().await.unwrap();
        assert!(studio.pending_snowflake_receipts.lock().is_empty());
        let history = studio.history("").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, original.0);
        assert_eq!(history[0].snowflake_ddl, original.1.snowflake_ddl);
        let terminal_audits: u32 = database.query_row("SELECT COUNT(*) FROM audit_v8 WHERE action = 'snowflake-ddl' AND outcome = 'succeeded'", [], |row| row.get(0)).unwrap();
        assert_eq!(terminal_audits, 1);
        assert_eq!(driver.ddl_calls.lock().len(), 1);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
        assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
    }
}
