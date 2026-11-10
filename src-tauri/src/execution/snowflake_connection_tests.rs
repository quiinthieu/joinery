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
    // Every post-setup statement is an observed metadata read. In particular,
    // none may USE/SET/ALTER/COMMIT or start an independent authentication path.
    assert!(submitted[2..].iter().all(|sql| {
        sql.starts_with("SELECT ")
            || [
                "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION",
                "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION",
                "SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION",
            ]
            .contains(&sql.as_str())
    }));
}

fn observation_sample_sql() -> [&'static str; 5] {
    [
        "SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_JSON(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES",
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
        assert_eq!(stable.state, options.state);
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
        assert_ne!(changed.state, stable.state, "{key}");
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
    let denied = studio.session_contexts(&info.id).await.unwrap_err();
    assert_eq!(denied.code, ErrorCode::Database);
    assert_eq!(denied.completion_invalidated, Some(true));
    assert_eq!(queries(&fixture)[before..], sample[..3]);
    assert!(retained.observed_runtime.lock().is_none());
    std::fs::remove_file(fixture.controls.join("deny-observer")).unwrap();
    let before = queries(&fixture).len();
    let recovered = studio.session_contexts(&info.id).await.unwrap();
    assert_options_reads(&fixture, before);
    assert_ne!(prior.revision, recovered.revision);
    assert_eq!(prior.state, recovered.state);
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
