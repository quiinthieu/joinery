//! Actual Node + real helper + synthetic SDK, through Studio's production
//! routing and cleanup paths. No Snowflake login, server or remote writes.
use super::*;
use crate::{secrets::MemorySecrets, snowflake_runtime::studio_fixture::Fixture};

// Each fixture copies and verifies the real Node binary. Keep these local
// process tests sequential even when the surrounding library suite is parallel;
// saturating disk/CPU with nine copies must not consume the startup deadlines.
static FIXTURE_LANE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn profile() -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Synthetic SSO".into(),
        group_id: "local".into(),
        engine: EngineKind::Snowflake,
        host: "org-account.snowflakecomputing.com".into(),
        port: 443,
        database: "DB".into(),
        username: "login@example.test".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::ReadOnly,
        tls_mode: TlsMode::VerifyFull,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: Some(SnowflakeConfig {
            authentication: SnowflakeAuthentication::ExternalBrowser,
            reviewed_dml: false,
            reviewed_ddl: false,
            role: "ROLE".into(),
            warehouse: "WH".into(),
            schema: "PUBLIC".into(),
        }),
    }
}

fn studio(fixture: &Fixture) -> Arc<Studio> {
    Studio::with_snowflake_runtime(
        &fixture.directory.path().join("workspace"),
        Arc::new(MemorySecrets::default()),
        Arc::new(DisabledFileDialog),
        Some(fixture.runtime.clone()),
    )
    .unwrap()
}

async fn started(fixture: &Fixture) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(bytes) = std::fs::read(fixture.controls.join("auth-started"))
                && let Ok(value) = serde_json::from_slice(&bytes)
            {
                return value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("helper must enter synthetic authentication")
}

fn no_roots(fixture: &Fixture) {
    assert_eq!(std::fs::read_dir(&fixture.state).unwrap().count(), 0);
}

#[tokio::test]
async fn sdk_session_is_published_and_disconnect_waits_with_retained_driver() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let info = studio
        .connect(&profile.id, ConnectionSecrets::default())
        .await
        .unwrap();
    assert!(!info.capabilities.table_browsing);
    assert!(info.capabilities.relation_browsing);
    assert!(!info.capabilities.transactions);
    let retained = studio.session(&info.id).unwrap();
    let marker = started(&fixture).await;
    assert!(Path::new(marker["root"].as_str().unwrap()).exists());
    studio.disconnect_and_wait(&info.id).await.unwrap();
    assert!(retained.driver.is_closed());
    assert!(studio.sessions.lock().is_empty());
    no_roots(&fixture);
    assert_eq!(
        std::fs::read_to_string(fixture.controls.join("queries"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[tokio::test]
async fn cancellation_during_auth_waits_for_reaping_and_exact_root_cleanup() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    std::fs::write(fixture.controls.join("stall-auth"), b"").unwrap();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    let task = tokio::spawn({
        let studio = studio.clone();
        let attempt = attempt.clone();
        async move {
            studio
                .connect_prepared(&attempt, ConnectionSecrets::default())
                .await
        }
    });
    started(&fixture).await;
    studio
        .cancel_connection_attempt_and_wait(&attempt)
        .await
        .unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert!(studio.sessions.lock().is_empty());
    no_roots(&fixture);
    assert!(!fixture.controls.join("queries").exists());
}

#[tokio::test]
async fn abandoning_auth_future_retains_cleanup_until_shutdown_receipt() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    std::fs::write(fixture.controls.join("stall-auth"), b"").unwrap();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let task = tokio::spawn({
        let studio = studio.clone();
        async move {
            studio
                .connect(&profile.id, ConnectionSecrets::default())
                .await
        }
    });
    started(&fixture).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    studio.shutdown_and_wait().await.unwrap();
    no_roots(&fixture);
    assert!(studio.sessions.lock().is_empty());
}

#[tokio::test]
async fn shutdown_during_auth_cannot_publish_and_prevents_another_launch() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    std::fs::write(fixture.controls.join("stall-auth"), b"").unwrap();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let task = tokio::spawn({
        let studio = studio.clone();
        let id = profile.id.clone();
        async move { studio.connect(&id, ConnectionSecrets::default()).await }
    });
    started(&fixture).await;
    studio.shutdown_and_wait().await.unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(
        studio.prepare_connection(&profile.id).unwrap_err().code,
        ErrorCode::Cancelled
    );
    no_roots(&fixture);
}

fn queries(fixture: &Fixture) -> Vec<String> {
    std::fs::read_to_string(fixture.controls.join("queries"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn metadata_state(fixture: &Fixture, state: &serde_json::Value) {
    std::fs::write(
        fixture.controls.join("metadata-state.json"),
        serde_json::to_vec(state).unwrap(),
    )
    .unwrap();
}

fn public_table() -> TableRef {
    TableRef {
        schema: "PUBLIC".into(),
        name: "SAME_NAME".into(),
    }
}

async fn metadata_session(fixture: &Fixture, studio: &Studio) -> SessionInfo {
    let profile = studio.save_profile(profile(), None).unwrap();
    let info = studio
        .connect(&profile.id, ConnectionSecrets::default())
        .await
        .unwrap();
    // Preserve the lifecycle baseline: metadata must reuse the already verified
    // SDK session, rather than adding setup work to the connect operation.
    assert_eq!(queries(fixture).len(), 2);
    info
}

async fn close_metadata_session(fixture: &Fixture, studio: &Studio, id: &str) {
    close_synthetic_session(fixture, studio, id, &[]).await;
}

async fn close_synthetic_session(
    fixture: &Fixture,
    studio: &Studio,
    id: &str,
    boundaries: &[&str],
) {
    // A generation change logically closes the driver before explicit removal;
    // retain its exact published owner even though session() now rejects it.
    let retained = studio.sessions.lock().get(id).cloned().unwrap();
    studio.disconnect_and_wait(id).await.unwrap();
    assert!(retained.driver.is_closed());
    assert!(studio.sessions.lock().is_empty());
    no_roots(fixture);
    let events = std::fs::read_to_string(fixture.controls.join("sdk-events")).unwrap();
    assert_eq!(
        events.lines().collect::<Vec<_>>(),
        ["create", "connect", "destroy"]
    );
    let submitted = queries(fixture);
    assert_eq!(submitted[1], "SELECT CURRENT_VERSION()");
    // Existing metadata tests allow no boundary SQL. Transaction tests separately
    // assert their exact fixed ledger and explicitly opt into only those names.
    assert!(submitted[2..].iter().all(|sql| {
        sql.starts_with("SELECT ")
            || [
                "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION",
                "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION",
                "SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION",
            ]
            .contains(&sql.as_str())
            || boundaries.contains(&sql.as_str())
    }));
}

fn observation_without_authority(state: &SessionRuntimeState) -> SessionRuntimeState {
    let mut state = state.clone();
    state.transaction_control_authority = None;
    state
}

fn observation_sample_sql() -> [&'static str; 5] {
    [
        "SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES",
        "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION",
        "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION",
        "SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION",
        "SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID",
    ]
}

fn assert_options_reads(fixture: &Fixture, before: usize) {
    let mut expected = observation_sample_sql().to_vec();
    expected.push("SELECT CATALOG_NAME, SCHEMA_NAME FROM \"DB\".INFORMATION_SCHEMA.SCHEMATA WHERE CATALOG_NAME = 'DB' AND SCHEMA_NAME <> 'INFORMATION_SCHEMA' ORDER BY SCHEMA_NAME LIMIT 1001");
    expected.extend(observation_sample_sql());
    assert_eq!(queries(fixture)[before..], expected);
}

async fn transaction_session(fixture: &Fixture, studio: &Studio) -> SessionInfo {
    let mut configured = profile();
    configured.safe_mode = SafeMode::Confirm;
    let configured = studio.save_profile(configured, None).unwrap();
    let info = studio
        .connect(&configured.id, ConnectionSecrets::default())
        .await
        .unwrap();
    assert!(info.capabilities.explicit_transaction_controls);
    assert!(!info.capabilities.transactions);
    assert_eq!(queries(fixture).len(), 2);
    info
}

fn transaction_request(
    state: &SessionRuntimeState,
    action: ExplicitTransactionAction,
) -> TransactionControlRequest {
    let revision = match state
        .transaction_control_authority
        .as_ref()
        .expect("observer must expose authority")
    {
        TransactionControlAuthority::Observed {
            revision,
            allowed_actions,
        } => {
            assert!(allowed_actions.contains(&action));
            revision.clone()
        }
        TransactionControlAuthority::RollbackOnly { revision } => {
            assert_eq!(action, ExplicitTransactionAction::Rollback);
            revision.clone()
        }
    };
    TransactionControlRequest { action, revision }
}

async fn reviewed_transaction(
    studio: &Studio,
    fixture: &Fixture,
    id: &str,
    request: &TransactionControlRequest,
) -> TransactionControlResult {
    let before = queries(fixture).len();
    let plan = studio.prepare_transaction_control(id, request).unwrap();
    assert_eq!(plan.statements.len(), 1);
    assert_eq!(plan.statements[0].sql, request.action.sql());
    assert_eq!(plan.statements[0].kind, StatementKind::Transaction);
    assert_eq!(
        plan.requires_approval,
        request.action != ExplicitTransactionAction::Rollback,
        "Confirm reviews BEGIN/COMMIT while preserving the existing rollback recovery exemption"
    );
    let approval = studio.authorize(&plan.id, None, None).unwrap();
    assert_eq!(
        queries(fixture).len(),
        before,
        "prepare and authorization never send SQL"
    );
    let result = studio
        .execute_transaction_control(&plan.id, Some(&approval))
        .await
        .unwrap();
    assert!(
        studio
            .execute_transaction_control(&plan.id, Some(&approval))
            .await
            .is_err()
    );
    result
}

fn assert_boundary_reads(fixture: &Fixture, before: usize, action: ExplicitTransactionAction) {
    let mut expected = Vec::new();
    if action != ExplicitTransactionAction::Rollback {
        expected.extend(observation_sample_sql());
    }
    expected.push(action.sql());
    expected.extend(observation_sample_sql());
    assert_eq!(
        queries(fixture)[before..],
        expected,
        "Studio must not append another independent observer"
    );
}

#[tokio::test]
async fn sdk_studio_reviewed_begin_commit_and_rollback_publish_without_extra_observation() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = transaction_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    metadata_state(
        &fixture,
        &serde_json::json!({"role":null,"abortOnError":true}),
    );
    let before = queries(&fixture).len();
    let options = studio.session_contexts(&info.id).await.unwrap();
    assert_options_reads(&fixture, before);
    assert_eq!(options.state.role, None);
    let begin = transaction_request(&options.state, ExplicitTransactionAction::Begin);
    let count = queries(&fixture).len();
    let unapproved = studio
        .prepare_transaction_control(&info.id, &begin)
        .unwrap();
    assert_eq!(
        studio
            .execute_transaction_control(&unapproved.id, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ApprovalRequired
    );
    assert_eq!(queries(&fixture).len(), count);
    let mut state = options.state;
    for action in [
        ExplicitTransactionAction::Begin,
        ExplicitTransactionAction::Commit,
        ExplicitTransactionAction::Rollback,
    ] {
        let request = transaction_request(&state, action);
        let before = queries(&fixture).len();
        let result = reviewed_transaction(&studio, &fixture, &info.id, &request).await;
        assert_boundary_reads(&fixture, before, action);
        assert_eq!(
            result.boundary_outcome,
            TransactionBoundaryOutcome::Acknowledged
        );
        assert!(!result.retired && result.error.is_none());
        assert_eq!(result.state.role, None);
        assert_eq!(
            result.state.transaction_state,
            if action == ExplicitTransactionAction::Begin {
                TransactionState::InTransaction
            } else {
                TransactionState::Idle
            }
        );
        if action == ExplicitTransactionAction::Begin {
            assert!(
                matches!(result.state.transaction_observation, Some(TransactionObservation::Observed {transaction_id: Some(ref id), ..}) if id == "opaque-studio-tx:90071992547409931234567890")
            );
        }
        assert_eq!(
            retained.observed_runtime.lock().as_ref(),
            Some(&result.state)
        );
        assert!(retained.completion.lock().is_none());
        assert!(
            studio
                .prepare_transaction_control(&info.id, &request)
                .is_err()
        );
        state = result.state;
    }
    assert_eq!(
        std::fs::read_to_string(fixture.controls.join("transaction-effects"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["BEGIN TRANSACTION", "COMMIT", "ROLLBACK"]
    );
    close_synthetic_session(
        &fixture,
        &studio,
        &info.id,
        &["BEGIN TRANSACTION", "COMMIT", "ROLLBACK"],
    )
    .await;
}

#[tokio::test]
async fn sdk_studio_acknowledged_begin_denial_keeps_recovery_and_rollback_has_no_preread() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = transaction_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let options = studio.session_contexts(&info.id).await.unwrap();
    std::fs::write(fixture.controls.join("deny-observer-after-boundary"), b"").unwrap();
    let before = queries(&fixture).len();
    let begun = reviewed_transaction(
        &studio,
        &fixture,
        &info.id,
        &transaction_request(&options.state, ExplicitTransactionAction::Begin),
    )
    .await;
    let mut expected = observation_sample_sql().to_vec();
    expected.push("BEGIN TRANSACTION");
    expected.extend(&observation_sample_sql()[..3]);
    assert_eq!(queries(&fixture)[before..], expected);
    assert_eq!(
        begun.boundary_outcome,
        TransactionBoundaryOutcome::Acknowledged
    );
    assert_eq!(begun.error.as_ref().unwrap().code, ErrorCode::Database);
    assert!(!begun.retired && !retained.driver.is_closed());
    assert_eq!(begun.state.transaction_state, TransactionState::Unknown);
    assert_eq!(
        retained.observed_runtime.lock().as_ref(),
        Some(&begun.state)
    );
    let rollback = transaction_request(&begun.state, ExplicitTransactionAction::Rollback);
    for _ in 0..2 {
        let before = queries(&fixture).len();
        // Deliberate owner refresh is allowed to try an observer while the
        // durable metadata hold blocks automatic callers. Incoherent results
        // retain the hold and the existing rollback-only recovery capability.
        let failed = studio
            .session_contexts_with_refresh(&info.id, true)
            .await
            .unwrap_err();
        assert_eq!(failed.code, ErrorCode::Conflict);
        assert_eq!(queries(&fixture)[before..], observation_sample_sql()[..3]);
        assert_eq!(
            transaction_request(
                retained.observed_runtime.lock().as_ref().unwrap(),
                ExplicitTransactionAction::Rollback
            ),
            rollback
        );
    }
    let before = queries(&fixture).len();
    assert!(
        studio
            .prepare_transaction_control(
                &info.id,
                &TransactionControlRequest {
                    action: ExplicitTransactionAction::Commit,
                    revision: rollback.revision.clone()
                }
            )
            .is_err()
    );
    assert!(studio.completion_catalog(&info.id).await.is_err());
    assert_eq!(queries(&fixture).len(), before);
    std::fs::remove_file(fixture.controls.join("deny-observer")).unwrap();
    let before = queries(&fixture).len();
    let recovered = reviewed_transaction(&studio, &fixture, &info.id, &rollback).await;
    assert_boundary_reads(&fixture, before, ExplicitTransactionAction::Rollback);
    assert_eq!(queries(&fixture)[before], "ROLLBACK");
    assert_eq!(
        recovered.boundary_outcome,
        TransactionBoundaryOutcome::Acknowledged
    );
    assert_eq!(recovered.state.transaction_state, TransactionState::Idle);
    assert!(!recovered.retired && recovered.error.is_none());
    assert_eq!(
        retained.observed_runtime.lock().as_ref(),
        Some(&recovered.state)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.controls.join("transaction-effects"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["BEGIN TRANSACTION", "ROLLBACK"]
    );
    close_synthetic_session(
        &fixture,
        &studio,
        &info.id,
        &["BEGIN TRANSACTION", "ROLLBACK"],
    )
    .await;
}

#[tokio::test]
async fn sdk_studio_lost_boundary_ack_retires_once_and_cleans_exact_helper_root() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = transaction_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let options = studio.session_contexts(&info.id).await.unwrap();
    let request = transaction_request(&options.state, ExplicitTransactionAction::Begin);
    std::fs::write(fixture.controls.join("lose-transaction-ack"), b"").unwrap();
    let before = queries(&fixture).len();
    let result = reviewed_transaction(&studio, &fixture, &info.id, &request).await;
    let mut expected = observation_sample_sql().to_vec();
    expected.push("BEGIN TRANSACTION");
    assert_eq!(queries(&fixture)[before..], expected);
    assert_eq!(result.boundary_outcome, TransactionBoundaryOutcome::Unknown);
    assert!(result.retired && retained.driver.is_closed());
    assert_eq!(result.error.unwrap().code, ErrorCode::OutcomeUnknown);
    assert!(result.state.transaction_control_authority.is_none());
    assert!(studio.sessions.lock().is_empty());
    assert!(
        studio
            .prepare_transaction_control(&info.id, &request)
            .is_err()
    );
    studio.shutdown_and_wait().await.unwrap();
    no_roots(&fixture);
    assert_eq!(
        std::fs::read_to_string(fixture.controls.join("transaction-effects")).unwrap(),
        "BEGIN TRANSACTION\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.controls.join("sdk-events"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["create", "connect"],
        "abrupt helper loss has no logout receipt"
    );
}

#[tokio::test]
async fn sdk_studio_options_use_eleven_reads_with_null_role_and_opaque_transaction() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = metadata_session(&fixture, &studio).await;
    metadata_state(
        &fixture,
        &serde_json::json!({
            "role": null,
            "schema": "OMITTED_CURRENT_SCHEMA",
            "autocommit": false,
            "abortOnError": true,
            "transactionId": "opaque-tx:90071992547409931234567890"
        }),
    );
    let before = queries(&fixture).len();
    let options = studio.session_contexts(&info.id).await.unwrap();
    assert_options_reads(&fixture, before);
    assert!(
        options
            .revision
            .as_deref()
            .is_some_and(|value| { uuid::Uuid::parse_str(value).is_ok() })
    );
    assert_eq!(options.state.session_id, info.id);
    assert_eq!(options.state.role, None);
    assert_eq!(options.state.database, "DB");
    assert_eq!(
        options.state.active_context.as_deref(),
        Some("OMITTED_CURRENT_SCHEMA")
    );
    assert_eq!(options.available, ["OTHER", "PUBLIC"]);
    assert_eq!(options.state.transaction_mode, TransactionMode::Manual);
    assert_eq!(
        options.state.transaction_state,
        TransactionState::InTransaction
    );
    assert_eq!(
        options.state.transaction_observation,
        Some(TransactionObservation::Observed {
            transaction_id: Some("opaque-tx:90071992547409931234567890".into()),
            abort_on_error: true,
            usability: TransactionUsability::Unknown,
        })
    );
    assert_eq!(
        studio
            .session(&info.id)
            .unwrap()
            .observed_runtime
            .lock()
            .as_ref(),
        Some(&options.state)
    );
    close_metadata_session(&fixture, &studio, &info.id).await;
}

#[tokio::test]
async fn sdk_studio_stable_observation_preserves_plans_but_full_runtime_drift_revokes_them() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = metadata_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let mut options = studio.session_contexts(&info.id).await.unwrap();
    assert_eq!(
        options.state.transaction_observation,
        Some(TransactionObservation::Observed {
            transaction_id: None,
            abort_on_error: false,
            usability: TransactionUsability::Unknown,
        })
    );
    let mut state = serde_json::json!({});
    for (key, value) in [
        ("warehouse", "OTHER_WH"),
        ("transactionId", "opaque-open-tx"),
    ] {
        let catalog = studio.completion_catalog(&info.id).await.unwrap();
        let plan = studio.prepare_execution(&info.id, "SELECT 1").unwrap();
        let before = queries(&fixture).len();
        let stable = studio.session_contexts(&info.id).await.unwrap();
        assert_options_reads(&fixture, before);
        assert_eq!(
            observation_without_authority(&stable.state),
            observation_without_authority(&options.state)
        );
        assert_ne!(
            stable.revision, options.revision,
            "options authority is a generation, not a content digest"
        );
        assert_eq!(
            retained.completion.lock().as_ref().unwrap().revision,
            catalog.revision
        );
        assert!(studio.plans.lock().contains_key(&plan.id));

        state[key] = serde_json::json!(value);
        metadata_state(&fixture, &state);
        let before = queries(&fixture).len();
        let changed = studio.session_contexts(&info.id).await.unwrap();
        assert_options_reads(&fixture, before);
        assert_ne!(
            observation_without_authority(&changed.state),
            observation_without_authority(&stable.state),
            "{key}"
        );
        assert_ne!(changed.revision, stable.revision);
        if key == "transactionId" {
            assert_eq!(
                changed.state.context_fingerprint, stable.state.context_fingerprint,
                "transaction-only drift must invalidate even when the context digest is stable"
            );
        } else {
            assert_ne!(
                changed.state.context_fingerprint,
                stable.state.context_fingerprint
            );
        }
        assert!(retained.completion.lock().is_none(), "{key}");
        assert!(!studio.plans.lock().contains_key(&plan.id), "{key}");
        assert_eq!(
            retained.observed_runtime.lock().as_ref(),
            Some(&changed.state)
        );
        options = changed;
    }
    close_metadata_session(&fixture, &studio, &info.id).await;
}

#[tokio::test]
async fn sdk_studio_completed_result_survives_denied_post_query_observation() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = metadata_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let prior = studio.session_contexts(&info.id).await.unwrap();
    studio.completion_catalog(&info.id).await.unwrap();
    let pending = studio.prepare_execution(&info.id, "SELECT 2").unwrap();
    let plan = studio.prepare_execution(&info.id, "SELECT 1").unwrap();
    std::fs::write(fixture.controls.join("deny-observer"), b"").unwrap();
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
    let before = queries(&fixture).len();
    let run_id = studio
        .execute(
            &plan.id,
            None,
            Arc::new(move |event| send.send(event).is_ok()),
        )
        .unwrap();
    let events = tokio::time::timeout(Duration::from_secs(10), async {
        let mut events = Vec::new();
        loop {
            let event = receive
                .recv()
                .await
                .expect("run must publish a terminal event");
            let finished = matches!(&event, QueryEvent::Finished { .. });
            events.push(event);
            if finished {
                return events;
            }
        }
    })
    .await
    .expect("query and failed independent observer must finish promptly");
    let sample = observation_sample_sql();
    assert_eq!(
        queries(&fixture)[before..],
        ["SELECT 1", sample[0], sample[1], sample[2]]
    );
    assert!(events.iter().any(|event| matches!(event,
        QueryEvent::Finished { run_id: id, status: RunStatus::Completed, .. } if id == &run_id)));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, QueryEvent::StatementCompleted { .. }))
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        QueryEvent::Error { .. } | QueryEvent::SessionClosed { .. }
    )));
    let result = events
        .iter()
        .filter_map(|event| match event {
            QueryEvent::Result { result } if result.status == RetentionStatus::Complete => {
                Some(result)
            }
            _ => None,
        })
        .next_back()
        .expect("acknowledged result remains complete");
    assert_eq!(result.retained_rows, 1);
    assert_eq!(
        studio.results.info(&result.id).unwrap().status,
        RetentionStatus::Complete
    );
    assert_eq!(
        studio.results.row(&result.id, 0).unwrap(),
        vec![Some("1".into())]
    );
    let state = events
        .iter()
        .find_map(|event| match event {
            QueryEvent::SessionState { state } => Some(state),
            _ => None,
        })
        .expect("observer failure must publish explicit unknown state");
    assert_eq!(state.transaction_mode, TransactionMode::Unknown);
    assert_eq!(state.transaction_state, TransactionState::Unknown);
    assert_eq!(
        state.transaction_observation,
        Some(TransactionObservation::Unavailable)
    );
    assert_eq!(state.role, None);
    assert_eq!(state.database, "");
    assert_eq!(state.active_context, None);
    assert_eq!(state.context_fingerprint, None);
    assert!(state.search_path.is_empty());
    assert!(retained.completion.lock().is_none());
    assert!(!studio.plans.lock().contains_key(&pending.id));
    assert_eq!(retained.observed_runtime.lock().as_ref(), Some(state));
    assert!(!retained.driver.is_closed());

    // Options cannot reuse the previously successful list after an observation
    // failure; recovery must establish a new server-held revision.
    let before = queries(&fixture).len();
    let denied = studio.session_contexts(&info.id).await.unwrap();
    assert_eq!(denied.state.transaction_state, TransactionState::Unknown);
    assert!(matches!(
        denied.state.transaction_control_authority,
        Some(TransactionControlAuthority::RollbackOnly { .. })
    ));
    assert_eq!(
        denied.state.transaction_control_authority,
        state.transaction_control_authority
    );
    assert!(denied.available.is_empty() && denied.revision.is_none());
    assert_eq!(queries(&fixture)[before..], sample[..3]);
    assert_eq!(
        retained.observed_runtime.lock().as_ref(),
        Some(&denied.state)
    );
    std::fs::remove_file(fixture.controls.join("deny-observer")).unwrap();
    let before = queries(&fixture).len();
    let recovered = studio.session_contexts(&info.id).await.unwrap();
    assert_options_reads(&fixture, before);
    assert_ne!(prior.revision, recovered.revision);
    assert_eq!(
        observation_without_authority(&prior.state),
        observation_without_authority(&recovered.state)
    );
    assert_eq!(recovered.available, ["OTHER", "PUBLIC"]);
    assert_eq!(
        studio.results.row(&result.id, 0).unwrap(),
        vec![Some("1".into())]
    );
    close_metadata_session(&fixture, &studio, &info.id).await;
}

#[tokio::test]
async fn sdk_completion_refresh_and_lazy_columns_keep_stable_revision_on_one_connection() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = metadata_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let first = studio.completion_catalog(&info.id).await.unwrap();
    assert_eq!(first.tables.len(), 2);
    assert_eq!(first.unqualified_tables, vec![public_table()]);
    assert!(first.identity.context_fingerprint.is_some());
    let schema = studio
        .completion_schema(&info.id, &first.revision, &public_table())
        .await
        .unwrap();
    assert_eq!(
        schema
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["ID", "NAME"]
    );
    assert!(!schema.editable);
    let refreshed = studio.completion_catalog(&info.id).await.unwrap();
    assert_eq!(first.revision, refreshed.revision);
    assert_eq!(first.identity, refreshed.identity);
    assert!(Arc::ptr_eq(&retained, &studio.session(&info.id).unwrap()));
    close_metadata_session(&fixture, &studio, &info.id).await;
}

#[tokio::test]
async fn sdk_relation_workflow_uses_console_lane_and_revokes_stale_targets() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = metadata_session(&fixture, &studio).await;
    assert!(info.capabilities.relation_browsing);
    assert!(!info.capabilities.table_browsing);
    let retained = studio.session(&info.id).unwrap();
    let catalog = studio.relation_catalog(&info.id).await.unwrap();
    let target = catalog
        .relations
        .iter()
        .find(|target| target.table == public_table())
        .unwrap();
    let request = InspectRelationRequest {
        revision: catalog.revision.clone(),
        target_id: target.target_id.clone(),
    };
    let inspected = studio.inspect_relation(&info.id, &request).await.unwrap();
    assert_eq!(
        inspected
            .columns
            .iter()
            .map(|column| column.ordinal)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(
        inspected.constraints,
        ConstraintInspectionStatus::NotFetched
    );
    let preview_request = PreviewRelationRequest {
        revision: request.revision.clone(),
        target_id: request.target_id.clone(),
        inspection_revision: inspected.revision,
    };
    let preview = studio
        .preview_relation(&info.id, &preview_request)
        .await
        .unwrap();
    assert_eq!(
        preview.rows,
        vec![
            vec![Some("9007199254740993".into()), None],
            vec![Some("2".into()), Some("".into())]
        ]
    );
    assert!(!preview.truncated);
    let refreshed = studio.relation_catalog(&info.id).await.unwrap();
    assert_ne!(refreshed.revision, catalog.revision);
    let before = queries(&fixture).len();
    assert_eq!(
        studio
            .inspect_relation(&info.id, &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio
            .preview_relation(&info.id, &preview_request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        queries(&fixture).len(),
        before,
        "stale requests must submit no helper SQL"
    );
    let next_target = &refreshed.relations[0];
    let next_request = InspectRelationRequest {
        revision: refreshed.revision,
        target_id: next_target.target_id.clone(),
    };
    let held = retained.lane.lock().await;
    assert_eq!(
        studio.relation_catalog(&info.id).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    drop(held);
    assert_eq!(
        studio
            .inspect_relation(&info.id, &next_request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        queries(&fixture).len(),
        before,
        "a busy outer refresh must revoke targets before waiting"
    );
    assert!(Arc::ptr_eq(&retained, &studio.session(&info.id).unwrap()));
    studio.disconnect_and_wait(&info.id).await.unwrap();
    assert!(retained.driver.is_closed());
    no_roots(&fixture);
    let events = std::fs::read_to_string(fixture.controls.join("sdk-events")).unwrap();
    assert_eq!(
        events.lines().collect::<Vec<_>>(),
        ["create", "connect", "destroy"]
    );
    let submitted = queries(&fixture);
    assert!(submitted.iter().all(|sql| sql.starts_with("SELECT ")
        || sql.starts_with("DESCRIBE TABLE ")
        || sql == "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION"));
}

#[tokio::test]
async fn sdk_completion_hidden_context_changes_invalidate_old_revisions() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = metadata_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let mut prior = studio.completion_catalog(&info.id).await.unwrap();
    let mut state = serde_json::json!({});
    // The search path and visible relations stay fixed. These changes were not
    // represented in the old display identity and must be covered by its digest.
    for (key, value) in [
        ("schema", serde_json::json!("HIDDEN_SCHEMA")),
        ("warehouse", serde_json::json!("OTHER_WH")),
        (
            "secondaryRoles",
            serde_json::json!({"roles":"EXTRA_ROLE","value":"ALL"}),
        ),
    ] {
        state[key] = value;
        metadata_state(&fixture, &state);
        let refreshed = studio.completion_catalog(&info.id).await.unwrap();
        assert_ne!(prior.revision, refreshed.revision, "{key}");
        assert_ne!(
            prior.identity.context_fingerprint, refreshed.identity.context_fingerprint,
            "{key}"
        );
        assert_eq!(prior.identity.database, refreshed.identity.database);
        assert_eq!(prior.identity.role, refreshed.identity.role);
        assert_eq!(prior.identity.search_path, refreshed.identity.search_path);
        assert_eq!(
            prior
                .tables
                .iter()
                .map(|table| (&table.table, &table.kind))
                .collect::<Vec<_>>(),
            refreshed
                .tables
                .iter()
                .map(|table| (&table.table, &table.kind))
                .collect::<Vec<_>>()
        );
        let before = queries(&fixture).len();
        let error = studio
            .completion_schema(&info.id, &prior.revision, &public_table())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.completion_invalidated, Some(true));
        assert_eq!(
            queries(&fixture).len(),
            before,
            "stale revision must fail before submitting SQL"
        );
        assert_eq!(
            retained.completion.lock().as_ref().unwrap().revision,
            refreshed.revision,
            "rejecting an old revision must preserve the newer snapshot"
        );
        prior = refreshed;
    }
    close_metadata_session(&fixture, &studio, &info.id).await;
}

#[tokio::test]
async fn sdk_completion_denied_and_malformed_refreshes_remove_previous_snapshot() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = metadata_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let mut prior = studio.completion_catalog(&info.id).await.unwrap();
    for (marker, expected) in [
        ("deny-catalog", ErrorCode::Database),
        ("malformed-catalog", ErrorCode::ResultLimitReached),
    ] {
        std::fs::write(fixture.controls.join(marker), b"").unwrap();
        assert_eq!(
            studio.completion_catalog(&info.id).await.unwrap_err().code,
            expected
        );
        assert!(retained.completion.lock().is_none());
        let before = queries(&fixture).len();
        let error = studio
            .completion_schema(&info.id, &prior.revision, &public_table())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.completion_invalidated, Some(true));
        assert_eq!(queries(&fixture).len(), before);
        assert!(retained.completion.lock().is_none());
        std::fs::remove_file(fixture.controls.join(marker)).unwrap();
        let recovered = studio.completion_catalog(&info.id).await.unwrap();
        assert_ne!(prior.revision, recovered.revision);
        prior = recovered;
    }
    close_metadata_session(&fixture, &studio, &info.id).await;
}

#[tokio::test]
async fn sdk_completion_context_change_after_driver_schema_check_rejects_studio_publication() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = metadata_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let snapshot = studio.completion_catalog(&info.id).await.unwrap();
    std::fs::write(fixture.controls.join("change-after-schema-recheck"), b"").unwrap();
    let error = studio
        .completion_schema(&info.id, &snapshot.revision, &public_table())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    // Distinguish Studio's outer identity comparison from the driver's own
    // stable before/after check, which intentionally received the old context.
    assert_eq!(
        error.message,
        "Completion context changed; refresh suggestions"
    );
    assert_eq!(
        serde_json::to_value(&error).unwrap()["completionInvalidated"],
        true,
        "global context invalidation must reach the frontend error envelope"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.controls.join("schema-context-shifted")).unwrap(),
        "after-driver-recheck"
    );
    assert!(retained.completion.lock().is_none());
    assert_eq!(
        queries(&fixture)
            .iter()
            .filter(|sql| sql.ends_with(" LIMIT 0"))
            .count(),
        1
    );
    std::fs::remove_file(fixture.controls.join("change-after-schema-recheck")).unwrap();

    for (marker, expected, boundary) in [
        (
            "drift-during-schema",
            ErrorCode::Conflict,
            "Snowflake context changed; refresh suggestions",
        ),
        (
            "policy-error-after-schema-recheck",
            ErrorCode::Unsupported,
            "Snowflake completion requires QUOTED_IDENTIFIERS_IGNORE_CASE=FALSE",
        ),
    ] {
        let snapshot = studio.completion_catalog(&info.id).await.unwrap();
        std::fs::write(fixture.controls.join(marker), b"").unwrap();
        let error = studio
            .completion_schema(&info.id, &snapshot.revision, &public_table())
            .await
            .unwrap_err();
        assert_eq!(error.code, expected, "{marker}");
        assert_eq!(error.message, boundary, "{marker}");
        assert_eq!(
            serde_json::to_value(&error).unwrap()["completionInvalidated"],
            true,
            "{marker} invalidates the global context"
        );
        assert!(retained.completion.lock().is_none(), "{marker}");
        assert!(!retained.driver.is_closed(), "{marker}");
        if marker == "policy-error-after-schema-recheck" {
            assert_eq!(
                std::fs::read_to_string(fixture.controls.join("schema-policy-shifted")).unwrap(),
                "after-driver-policy-recheck"
            );
        } else {
            assert_eq!(
                std::fs::read_to_string(fixture.controls.join("schema-context-shifted")).unwrap(),
                "during-column-read"
            );
        }
        std::fs::remove_file(fixture.controls.join(marker)).unwrap();
    }

    // A definite permission failure concerns this relation, not the global
    // context. Preserve the catalog so another table's columns still resolve.
    let snapshot = studio.completion_catalog(&info.id).await.unwrap();
    std::fs::write(fixture.controls.join("deny-public-columns"), b"").unwrap();
    let error = studio
        .completion_schema(&info.id, &snapshot.revision, &public_table())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Database);
    assert!(
        serde_json::to_value(&error)
            .unwrap()
            .get("completionInvalidated")
            .is_none(),
        "per-table permission rejection must not invalidate the global catalog"
    );
    assert_eq!(
        retained.completion.lock().as_ref().unwrap().revision,
        snapshot.revision
    );
    let other = TableRef {
        schema: "OTHER".into(),
        name: "SAME_NAME".into(),
    };
    let schema = studio
        .completion_schema(&info.id, &snapshot.revision, &other)
        .await
        .unwrap();
    assert_eq!(schema.table, other);
    assert_eq!(schema.columns.len(), 2);
    assert_eq!(
        retained.completion.lock().as_ref().unwrap().revision,
        snapshot.revision
    );
    std::fs::remove_file(fixture.controls.join("deny-public-columns")).unwrap();
    close_metadata_session(&fixture, &studio, &info.id).await;
}

#[tokio::test]
async fn sdk_completion_server_session_replacement_retires_without_reconnecting() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = metadata_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    studio.completion_catalog(&info.id).await.unwrap();
    metadata_state(
        &fixture,
        &serde_json::json!({"session":"replacement-opaque-session"}),
    );
    assert_eq!(
        studio.completion_catalog(&info.id).await.unwrap_err().code,
        ErrorCode::ConnectionClosed
    );
    assert!(retained.driver.is_closed());
    assert!(retained.completion.lock().is_none());
    let submitted = queries(&fixture).len();
    assert_eq!(
        studio.completion_catalog(&info.id).await.unwrap_err().code,
        ErrorCode::ConnectionClosed
    );
    assert_eq!(
        queries(&fixture).len(),
        submitted,
        "retirement must not reconnect or replay"
    );
    close_metadata_session(&fixture, &studio, &info.id).await;
}

const DML_INPUT_SQL: &str = "UPDATE DB.PUBLIC.T SET V = :value WHERE V = :value";
const DML_WIRE_SQL: &str = "UPDATE DB.PUBLIC.T SET V = ? WHERE V = ?";

async fn dml_session(fixture: &Fixture, studio: &Studio) -> SessionInfo {
    let mut configured = profile();
    configured.safe_mode = SafeMode::Confirm;
    configured.snowflake.as_mut().unwrap().reviewed_dml = true;
    let configured = studio.save_profile(configured, None).unwrap();
    let info = studio
        .connect(&configured.id, ConnectionSecrets::default())
        .await
        .unwrap();
    assert_eq!(queries(fixture).len(), 2);
    let before = queries(fixture).len();
    studio.session_contexts(&info.id).await.unwrap();
    assert_options_reads(fixture, before);
    info
}

fn reviewed_dml(studio: &Studio, fixture: &Fixture, id: &str) -> (PreparedPlan, String) {
    let before = queries(fixture).len();
    let plan = studio
        .prepare_execution_with_params(
            id,
            DML_INPUT_SQL,
            Some(vec![QueryParamInput {
                key: "value".into(),
                kind: QueryParamKind::Text,
                value: Some("private-dml-parameter-never-persist".into()),
            }]),
        )
        .unwrap();
    assert!(plan.reviewed_snowflake_dml && plan.requires_approval);
    assert_eq!(plan.statements.len(), 1);
    assert_eq!(plan.statements[0].sql, DML_INPUT_SQL);
    let approval = studio.authorize(&plan.id, None, None).unwrap();
    assert_eq!(queries(fixture).len(), before, "review must remain local");
    (plan, approval)
}

fn dml_effects(fixture: &Fixture) -> Vec<serde_json::Value> {
    std::fs::read_to_string(fixture.controls.join("dml-effects"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_dml_reads(fixture: &Fixture, before: usize, final_sample: usize) {
    let mut expected = observation_sample_sql().to_vec();
    expected.push(DML_WIRE_SQL);
    expected.extend(&observation_sample_sql()[..final_sample]);
    assert_eq!(queries(fixture)[before..], expected);
    let effects = dml_effects(fixture);
    assert_eq!(effects.len(), 1, "a reviewed write must never replay");
    assert_eq!(effects[0]["sql"], DML_WIRE_SQL);
    assert_eq!(
        effects[0]["binds"],
        serde_json::json!([
            "private-dml-parameter-never-persist",
            "private-dml-parameter-never-persist"
        ])
    );
    assert!(!fixture.controls.join("transaction-effects").exists());
}

async fn dml_entered(fixture: &Fixture) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !fixture.controls.join("dml-entered").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("production helper must submit the synthetic DML once");
}

#[tokio::test]
async fn sdk_studio_dml_parameterized_uses_eleven_sql_and_persists_exact_evidence_on_restart() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = dml_session(&fixture, &studio).await;
    let (plan, approval) = reviewed_dml(&studio, &fixture, &info.id);
    let before = queries(&fixture).len();
    let result = studio
        .execute_snowflake_dml(&plan.id, Some(&approval))
        .await
        .unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert!(!result.retired && result.error.is_none());
    assert_eq!(result.summary.status, SnowflakeDmlSummaryStatus::Validated);
    assert_eq!(
        result.summary.affected_rows.as_deref(),
        Some("9007199254740993")
    );
    assert_eq!(result.summary.multi_joined_rows.as_deref(), Some("7"));
    assert_dml_reads(&fixture, before, 5);
    assert!(
        studio
            .execute_snowflake_dml(&plan.id, Some(&approval))
            .await
            .is_err()
    );
    assert_dml_reads(&fixture, before, 5);
    close_synthetic_session(&fixture, &studio, &info.id, &[DML_WIRE_SQL]).await;
    studio.shutdown_and_wait().await.unwrap();
    let reopened = Studio::with_secrets(
        &fixture.directory.path().join("workspace"),
        Arc::new(MemorySecrets::default()),
    )
    .unwrap();
    let history = reopened.history("").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, RunStatus::Completed);
    assert!(history[0].sql.is_none());
    let evidence = history[0].snowflake_dml.as_ref().unwrap();
    assert_eq!(evidence.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert_eq!(evidence.summary, result.summary);
    let persisted = serde_json::to_string(&history).unwrap();
    assert!(!persisted.contains("private-dml-parameter-never-persist"));
    assert!(!persisted.contains(":value"));
}

#[tokio::test]
async fn sdk_studio_dml_ack_survives_final_observer_denial_with_only_rollback_recovery() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = dml_session(&fixture, &studio).await;
    let (plan, approval) = reviewed_dml(&studio, &fixture, &info.id);
    std::fs::write(fixture.controls.join("deny-observer-after-dml"), b"").unwrap();
    let before = queries(&fixture).len();
    let result = studio
        .execute_snowflake_dml(&plan.id, Some(&approval))
        .await
        .unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert_eq!(
        result.summary.affected_rows.as_deref(),
        Some("9007199254740993")
    );
    assert_eq!(result.error.as_ref().unwrap().code, ErrorCode::Database);
    assert!(!result.retired);
    assert!(matches!(
        result.state.transaction_control_authority,
        Some(TransactionControlAuthority::RollbackOnly { .. })
    ));
    assert_dml_reads(&fixture, before, 3);
    assert!(studio.prepare_execution(&info.id, DML_INPUT_SQL).is_err());
    assert_eq!(queries(&fixture).len(), before + 9);
    let recovery = transaction_request(&result.state, ExplicitTransactionAction::Rollback);
    std::fs::remove_file(fixture.controls.join("deny-observer")).unwrap();
    let before_rollback = queries(&fixture).len();
    let rolled_back = reviewed_transaction(&studio, &fixture, &info.id, &recovery).await;
    assert_boundary_reads(
        &fixture,
        before_rollback,
        ExplicitTransactionAction::Rollback,
    );
    assert_eq!(
        rolled_back.boundary_outcome,
        TransactionBoundaryOutcome::Acknowledged
    );
    close_synthetic_session(&fixture, &studio, &info.id, &[DML_WIRE_SQL, "ROLLBACK"]).await;
}

#[tokio::test]
async fn sdk_studio_dml_lost_ack_is_unknown_retires_and_never_replays() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = dml_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let (plan, approval) = reviewed_dml(&studio, &fixture, &info.id);
    std::fs::write(fixture.controls.join("lose-dml-ack"), b"").unwrap();
    let before = queries(&fixture).len();
    let result = studio
        .execute_snowflake_dml(&plan.id, Some(&approval))
        .await
        .unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Unknown);
    assert_eq!(
        result.summary.status,
        SnowflakeDmlSummaryStatus::Unavailable
    );
    assert!(result.retired && result.state.transaction_control_authority.is_none());
    assert!(retained.driver.is_closed());
    assert_dml_reads(&fixture, before, 0);
    assert!(
        studio
            .execute_snowflake_dml(&plan.id, Some(&approval))
            .await
            .is_err()
    );
    assert!(studio.health(&info.id).await.is_err());
    assert_dml_reads(&fixture, before, 0);
    let history = studio.history("").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, RunStatus::OutcomeUnknown);
    assert_eq!(
        history[0].snowflake_dml.as_ref().unwrap().statement_outcome,
        SnowflakeDmlOutcome::Unknown
    );
    studio.shutdown_and_wait().await.unwrap();
    no_roots(&fixture);
}

#[tokio::test]
async fn sdk_studio_dml_queued_health_and_completion_submit_no_sql_after_hold() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = dml_session(&fixture, &studio).await;
    let (plan, approval) = reviewed_dml(&studio, &fixture, &info.id);
    std::fs::write(fixture.controls.join("hold-dml-ack"), b"").unwrap();
    let before = queries(&fixture).len();
    let owner = studio.clone();
    let write =
        tokio::spawn(async move { owner.execute_snowflake_dml(&plan.id, Some(&approval)).await });
    dml_entered(&fixture).await;
    let health_owner = studio.clone();
    let health_id = info.id.clone();
    let health = tokio::spawn(async move { health_owner.health(&health_id).await });
    let completion_owner = studio.clone();
    let completion_id = info.id.clone();
    let completion =
        tokio::spawn(async move { completion_owner.completion_catalog(&completion_id).await });
    tokio::task::yield_now().await;
    std::fs::write(fixture.controls.join("release-dml-ack"), b"").unwrap();
    let result = write.await.unwrap().unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert!(!result.retired && result.error.is_none());
    assert_eq!(health.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(
        completion.await.unwrap().unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_dml_reads(&fixture, before, 5);
    assert_eq!(
        studio.health(&info.id).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio.completion_catalog(&info.id).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_dml_reads(&fixture, before, 5);
    close_synthetic_session(&fixture, &studio, &info.id, &[DML_WIRE_SQL]).await;
}

#[tokio::test]
async fn sdk_studio_dml_dropped_waiter_still_records_helper_ack_once() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = dml_session(&fixture, &studio).await;
    let (plan, approval) = reviewed_dml(&studio, &fixture, &info.id);
    std::fs::write(fixture.controls.join("hold-dml-ack"), b"").unwrap();
    let before = queries(&fixture).len();
    let owner = studio.clone();
    let waiter =
        tokio::spawn(async move { owner.execute_snowflake_dml(&plan.id, Some(&approval)).await });
    dml_entered(&fixture).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    std::fs::write(fixture.controls.join("release-dml-ack"), b"").unwrap();
    let history = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let history = studio.history("").unwrap();
            if !history.is_empty() {
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
    assert_dml_reads(&fixture, before, 5);
    close_synthetic_session(&fixture, &studio, &info.id, &[DML_WIRE_SQL]).await;
}

async fn ddl_session(fixture: &Fixture, studio: &Studio) -> SessionInfo {
    let mut configured = profile();
    configured.safe_mode = SafeMode::Silent;
    configured.snowflake.as_mut().unwrap().reviewed_ddl = true;
    let configured = studio.save_profile(configured, None).unwrap();
    let info = studio
        .connect(&configured.id, ConnectionSecrets::default())
        .await
        .unwrap();
    assert!(info.capabilities.snowflake_reviewed_ddl);
    assert!(!info.capabilities.ddl_editing);
    assert_eq!(queries(fixture).len(), 2);
    info
}

async fn reviewed_ddl(studio: &Studio, fixture: &Fixture, id: &str) -> (PreparedPlan, String) {
    let before = queries(fixture).len();
    let preflight = SnowflakeDdlPreflightRequest {
        session_id: id.into(),
        action: SnowflakeDdlAction::CreateTable {
            database: "DB".into(),
            schema: "PUBLIC".into(),
            name: "DDL_TEST".into(),
            columns: vec![SnowflakeDdlColumnDraft {
                name: "V".into(),
                data_type: "NUMBER(10, 0)".into(),
                nullable: true,
            }],
        },
    };
    let response = studio.preflight_snowflake_ddl(&preflight).await.unwrap();
    assert_eq!(
        queries(fixture).len() - before,
        12,
        "coherent preflight: two five-read samples and two bounded catalog reads"
    );
    let before = queries(fixture).len();
    let plan = studio
        .prepare_snowflake_ddl(
            id,
            &SnowflakeDdlRequest {
                action: preflight.action,
                revision: response.revision,
                sql: response.sql,
            },
        )
        .unwrap();
    assert!(plan.reviewed_snowflake_ddl && plan.requires_approval);
    let token = studio
        .authorize_snowflake_ddl(&plan.id, None, None, None)
        .unwrap();
    assert_eq!(
        queries(fixture).len(),
        before,
        "prepare and review stay local"
    );
    (plan, token)
}

fn ddl_effects(fixture: &Fixture) -> Vec<serde_json::Value> {
    std::fs::read_to_string(fixture.controls.join("ddl-effects"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn ddl_entered(fixture: &Fixture) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !fixture.controls.join("ddl-entered").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("real helper must enter synthetic DDL");
}

#[tokio::test]
async fn sdk_studio_ddl_keeps_catalog_hold_until_explicit_bounded_reconciliation() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = ddl_session(&fixture, &studio).await;
    let retained = studio.session(&info.id).unwrap();
    let (plan, token) = reviewed_ddl(&studio, &fixture, &info.id).await;
    let sql = plan.statements[0].sql.clone();
    let before = queries(&fixture).len();
    let result = studio
        .execute_snowflake_ddl(&plan.id, Some(&token))
        .await
        .unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
    assert_eq!(
        result.summary_status,
        SnowflakeDdlSummaryStatus::Acknowledged
    );
    assert!(!result.retired && result.error.is_none());
    assert!(result.catalog_reconciliation_required);
    assert_eq!(
        queries(&fixture).len() - before,
        18,
        "two five-read samples, two catalog reads, one DDL, final five-read sample"
    );
    assert_eq!(ddl_effects(&fixture).len(), 1);
    assert!(retained.metadata_gate.lock().catalog_required);
    let before = queries(&fixture).len();
    assert!(
        studio
            .execute_snowflake_ddl(&plan.id, Some(&token))
            .await
            .is_err()
    );
    assert!(studio.health(&info.id).await.is_err());
    assert!(studio.completion_catalog(&info.id).await.is_err());
    assert!(
        studio
            .session_contexts_with_refresh(&info.id, true)
            .await
            .is_err()
    );
    assert_eq!(
        queries(&fixture).len(),
        before,
        "terminal result must not trigger background observer/catalog work"
    );
    std::fs::write(fixture.controls.join("deny-catalog"), b"").unwrap();
    assert!(
        studio
            .refresh_snowflake_ddl_metadata(&info.id)
            .await
            .is_err()
    );
    assert!(retained.metadata_gate.lock().held);
    std::fs::remove_file(fixture.controls.join("deny-catalog")).unwrap();
    studio
        .refresh_snowflake_ddl_metadata(&info.id)
        .await
        .unwrap();
    assert!(!retained.metadata_gate.lock().held);
    let catalog = studio.relation_catalog(&info.id).await.unwrap();
    assert!(
        catalog
            .relations
            .iter()
            .any(|relation| relation.table.name == "DDL_TEST")
    );
    assert_eq!(ddl_effects(&fixture).len(), 1);
    close_synthetic_session(&fixture, &studio, &info.id, &[&sql]).await;
    studio.shutdown_and_wait().await.unwrap();
    let reopened = Studio::with_secrets(
        &fixture.directory.path().join("workspace"),
        Arc::new(MemorySecrets::default()),
    )
    .unwrap();
    let history = reopened.history("").unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].sql.is_none());
    assert_eq!(
        history[0].snowflake_ddl.as_ref().unwrap().statement_outcome,
        SnowflakeDdlOutcome::Succeeded
    );
    assert!(!fixture.controls.join("transaction-effects").exists());
}

#[tokio::test]
async fn sdk_studio_ddl_lost_ack_retires_once_with_durable_unknown() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = ddl_session(&fixture, &studio).await;
    let (plan, token) = reviewed_ddl(&studio, &fixture, &info.id).await;
    std::fs::write(fixture.controls.join("lose-ddl-ack"), b"").unwrap();
    let result = studio
        .execute_snowflake_ddl(&plan.id, Some(&token))
        .await
        .unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Unknown);
    assert!(result.retired);
    assert!(
        studio
            .execute_snowflake_ddl(&plan.id, Some(&token))
            .await
            .is_err()
    );
    assert_eq!(ddl_effects(&fixture).len(), 1);
    assert_eq!(
        studio.history("").unwrap()[0]
            .snowflake_ddl
            .as_ref()
            .unwrap()
            .statement_outcome,
        SnowflakeDdlOutcome::Unknown
    );
    studio.shutdown_and_wait().await.unwrap();
    no_roots(&fixture);
}

#[tokio::test]
async fn sdk_studio_ddl_final_observer_failure_preserves_remote_ack() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = ddl_session(&fixture, &studio).await;
    let (plan, token) = reviewed_ddl(&studio, &fixture, &info.id).await;
    std::fs::write(fixture.controls.join("deny-observer-after-ddl"), b"").unwrap();
    let result = studio
        .execute_snowflake_ddl(&plan.id, Some(&token))
        .await
        .unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
    assert!(result.error.is_some());
    assert_eq!(ddl_effects(&fixture).len(), 1);
    assert_eq!(
        studio.history("").unwrap()[0]
            .snowflake_ddl
            .as_ref()
            .unwrap()
            .statement_outcome,
        SnowflakeDdlOutcome::Succeeded
    );
    studio.shutdown_and_wait().await.unwrap();
    no_roots(&fixture);
}

#[tokio::test]
async fn sdk_studio_ddl_dropped_waiter_still_persists_actual_helper_ack() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let info = ddl_session(&fixture, &studio).await;
    let (plan, token) = reviewed_ddl(&studio, &fixture, &info.id).await;
    std::fs::write(fixture.controls.join("hold-ddl-ack"), b"").unwrap();
    let owner = studio.clone();
    let waiter =
        tokio::spawn(async move { owner.execute_snowflake_ddl(&plan.id, Some(&token)).await });
    ddl_entered(&fixture).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    std::fs::write(fixture.controls.join("release-ddl-ack"), b"").unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while studio.history("").unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        studio.history("").unwrap()[0]
            .snowflake_ddl
            .as_ref()
            .unwrap()
            .statement_outcome,
        SnowflakeDdlOutcome::Succeeded
    );
    assert_eq!(ddl_effects(&fixture).len(), 1);
    studio.shutdown_and_wait().await.unwrap();
    no_roots(&fixture);
}

#[tokio::test]
async fn sdk_studio_ddl_shutdown_or_disconnect_after_ack_keeps_durable_success() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    for shutdown in [false, true] {
        let fixture = Fixture::new();
        let studio = studio(&fixture);
        let info = ddl_session(&fixture, &studio).await;
        let (plan, token) = reviewed_ddl(&studio, &fixture, &info.id).await;
        std::fs::write(fixture.controls.join("hold-ddl-final-observer"), b"").unwrap();
        let owner = studio.clone();
        let waiter =
            tokio::spawn(async move { owner.execute_snowflake_ddl(&plan.id, Some(&token)).await });
        tokio::time::timeout(Duration::from_secs(10), async {
            while !fixture.controls.join("ddl-final-observer-entered").exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // The real helper emitted ddlAcknowledged before the adapter could
        // begin this final observer. Retire while it is deliberately pending.
        if shutdown {
            waiter.abort();
            assert!(waiter.await.unwrap_err().is_cancelled());
            studio.shutdown_and_wait().await.unwrap();
        } else {
            studio.disconnect_and_wait(&info.id).await.unwrap();
            let result = waiter.await.unwrap().unwrap();
            assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
            assert!(result.retired);
            studio.shutdown_and_wait().await.unwrap();
        }
        let history = studio.history("").unwrap();
        assert_eq!(history.len(), 1);
        let evidence = history[0].snowflake_ddl.as_ref().unwrap();
        assert_eq!(evidence.statement_outcome, SnowflakeDdlOutcome::Succeeded);
        assert!(evidence.retired);
        assert_eq!(ddl_effects(&fixture).len(), 1);
        no_roots(&fixture);
    }
}
