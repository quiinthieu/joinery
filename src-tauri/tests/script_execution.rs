use joinery_core::{domain::*, execution::Studio, results::ResultStore, secrets::MemorySecrets};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

async fn setup(budget: usize, row_limit: usize) -> (tempfile::TempDir, Arc<Studio>, SessionInfo) {
    setup_with_text(budget, row_limit, false).await
}

async fn setup_with_text(
    budget: usize,
    row_limit: usize,
    store_query_text: bool,
) -> (tempfile::TempDir, Arc<Studio>, SessionInfo) {
    let root = tempfile::tempdir().unwrap();
    let mut studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    // Rust-only injection into this disposable Studio; production limits and
    // profile settings are unchanged.
    Arc::get_mut(&mut studio).unwrap().results = ResultStore::new(budget, row_limit);
    let profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "Script accounting fixture".into(),
                group_id: "local".into(),
                engine: EngineKind::Postgres,
                host: "127.0.0.1".into(),
                port: std::env::var("JOINERY_TEST_PORT")
                    .unwrap_or_else(|_| "55432".into())
                    .parse()
                    .unwrap(),
                database: "joinery".into(),
                username: "joinery".into(),
                environment: Environment::Local,
                safe_mode: SafeMode::Silent,
                tls_mode: TlsMode::Disable,
                ca_cert_path: None,
                ssh: None,
                remember_password: false,
                store_query_text,
                store_drafts: false,
                snowflake: None,
            },
            None,
        )
        .unwrap();
    let session = studio
        .connect(
            &profile.id,
            ConnectionSecrets {
                password: Some("joinery-local-only".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    (root, studio, session)
}

fn start(
    studio: &Arc<Studio>,
    session: &SessionInfo,
    sql: &str,
) -> (String, mpsc::UnboundedReceiver<QueryEvent>) {
    let plan = studio.prepare_execution(&session.id, sql).unwrap();
    let approval = plan.requires_approval.then(|| {
        studio
            .authorize(&plan.id, None, plan.confirmation_text.as_deref())
            .unwrap()
    });
    let (tx, rx) = mpsc::unbounded_channel();
    let run_id = studio
        .execute(
            &plan.id,
            approval.as_deref(),
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .unwrap();
    (run_id, rx)
}

async fn finish(mut rx: mpsc::UnboundedReceiver<QueryEvent>) -> Vec<QueryEvent> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            let finished = matches!(event, QueryEvent::Finished { .. });
            events.push(event);
            if finished {
                return events;
            }
        }
        panic!("stream ended without Finished");
    })
    .await
    .expect("script deadline")
}

async fn run(studio: &Arc<Studio>, session: &SessionInfo, sql: &str) -> Vec<QueryEvent> {
    finish(start(studio, session, sql).1).await
}

fn terminal(events: &[QueryEvent]) -> serde_json::Value {
    serde_json::to_value(events.last().expect("terminal event")).unwrap()
}

fn release(studio: &Studio, events: &[QueryEvent]) {
    for event in events {
        if let QueryEvent::Result { result } = event {
            studio.release_result(&result.id);
        }
    }
}

async fn assert_limit_stops_suffix(first: &str, row_limit: usize) {
    let (root, studio, session) = setup(32 * 1024, row_limit).await;
    let setup_events = run(
        &studio,
        &session,
        "CREATE TEMP SEQUENCE script_suffix_probe",
    )
    .await;
    assert_eq!(terminal(&setup_events)["status"], "completed");
    release(&studio, &setup_events);
    let sql = format!("{first}; SELECT nextval('script_suffix_probe')");
    let plan = studio.prepare_execution(&session.id, &sql).unwrap();
    assert_eq!(plan.statements[0].kind, StatementKind::Read);
    let events = run(&studio, &session, &sql).await;
    assert!(events.iter().any(|event| matches!(event,
        QueryEvent::Result { result } if result.status == RetentionStatus::Truncated)));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, QueryEvent::StatementStarted { index: 1, .. }))
    );
    release(&studio, &events);
    let probe = run(
        &studio,
        &session,
        "SELECT is_called FROM script_suffix_probe",
    )
    .await;
    let result = probe
        .iter()
        .find_map(|event| match event {
            QueryEvent::Result { result } => Some(result),
            _ => None,
        })
        .unwrap();
    // Actual database evidence: the distinctive suffix never advanced this
    // session-local sequence, rather than only being absent from UI events.
    assert_eq!(
        studio.results.row(&result.id, 0).unwrap()[0].as_deref(),
        Some("f")
    );
    let terminal = terminal(&events);
    studio.disconnect_and_wait(&session.id).await.unwrap();
    drop(studio);
    let restarted = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let history = restarted
        .history("")
        .unwrap()
        .into_iter()
        .find(|entry| Some(entry.id.as_str()) == terminal["runId"].as_str())
        .unwrap();
    assert!(history.sql.is_none());
    assert_eq!(
        serde_json::to_value(history).unwrap()["executionSummary"],
        terminal["executionSummary"]
    );
    assert_eq!(
        terminal["status"], "stopped",
        "a skipped suffix is not a completed script"
    );
    assert_eq!(
        terminal["executionSummary"],
        serde_json::json!({
            "planned": 2, "attempted": 1, "completed": 1, "notAttempted": 1,
            "stopReason": "resultLimit"
        })
    );
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; only synthetic reads and session-local sequence"]
async fn postgres_row_limit_reports_unattempted_script_suffix() {
    assert_limit_stops_suffix("SELECT n FROM generate_series(1,4) n", 3).await;
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; only synthetic reads and session-local sequence"]
async fn postgres_byte_limit_reports_unattempted_script_suffix() {
    assert_limit_stops_suffix(&format!("SELECT '{}' AS payload", "x".repeat(65536)), 1000).await;
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; only synthetic reads and session-local sequence"]
async fn postgres_complete_script_executes_suffix_and_retains_accounting_on_restart() {
    let (root, studio, session) = setup(32 * 1024, 3).await;
    release(
        &studio,
        &run(
            &studio,
            &session,
            "CREATE TEMP SEQUENCE script_suffix_probe",
        )
        .await,
    );
    let events = run(
        &studio,
        &session,
        "SELECT 7; SELECT nextval('script_suffix_probe')",
    )
    .await;
    let done = terminal(&events);
    assert_eq!(done["status"], "completed");
    assert_eq!(
        done["executionSummary"],
        serde_json::json!({
            "planned": 2, "attempted": 2, "completed": 2, "notAttempted": 0,
            "stopReason": null
        })
    );
    let second = events
        .iter()
        .find_map(|event| match event {
            QueryEvent::Result { result } if result.statement_index == 1 => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        studio.results.row(&second.id, 0).unwrap()[0].as_deref(),
        Some("1")
    );
    let run_id = done["runId"].as_str().unwrap().to_owned();
    studio.disconnect_and_wait(&session.id).await.unwrap();
    drop(studio);
    let restarted = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let entry = restarted
        .history("")
        .unwrap()
        .into_iter()
        .find(|entry| entry.id == run_id)
        .unwrap();
    assert!(
        entry.sql.is_none(),
        "execution counts do not opt into SQL persistence"
    );
    assert_eq!(
        serde_json::to_value(entry).unwrap()["executionSummary"],
        done["executionSummary"]
    );
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; only synthetic reads and session-local sequence"]
async fn postgres_error_preserves_successful_prefix_and_unattempted_suffix() {
    let (_root, studio, session) = setup(32 * 1024, 3).await;
    release(
        &studio,
        &run(
            &studio,
            &session,
            "CREATE TEMP SEQUENCE script_suffix_probe",
        )
        .await,
    );
    let events = run(
        &studio,
        &session,
        "SELECT 7; SELECT 1 / 0; SELECT nextval('script_suffix_probe')",
    )
    .await;
    let done = terminal(&events);
    assert_eq!(done["status"], "failed");
    assert_eq!(
        done["executionSummary"],
        serde_json::json!({
            "planned": 3, "attempted": 2, "completed": 1, "notAttempted": 1,
            "stopReason": "failed"
        })
    );
    assert!(events.iter().any(|event| matches!(
        event,
        QueryEvent::Error {
            statement_index: 1,
            ..
        }
    )));
    release(&studio, &events);
    let probe = run(
        &studio,
        &session,
        "SELECT is_called FROM script_suffix_probe",
    )
    .await;
    let result = probe
        .iter()
        .find_map(|event| match event {
            QueryEvent::Result { result } => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        studio.results.row(&result.id, 0).unwrap()[0].as_deref(),
        Some("f")
    );
    studio.disconnect_and_wait(&session.id).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; writes only to a session-local temporary table"]
async fn postgres_limit_skips_commit_and_requires_explicit_rollback() {
    let (_root, studio, session) = setup(32 * 1024, 3).await;
    release(
        &studio,
        &run(
            &studio,
            &session,
            "CREATE TEMP TABLE script_tx_probe (id integer)",
        )
        .await,
    );
    let events = run(&studio, &session,
        "BEGIN; INSERT INTO script_tx_probe VALUES (1); SELECT n FROM generate_series(1,4) n; COMMIT").await;
    let done = terminal(&events);
    assert_eq!(done["status"], "stopped");
    assert_eq!(
        done["executionSummary"],
        serde_json::json!({
            "planned": 4, "attempted": 3, "completed": 3, "notAttempted": 1,
            "stopReason": "resultLimit"
        })
    );
    assert_eq!(
        studio
            .session_contexts(&session.id)
            .await
            .unwrap()
            .state
            .transaction_state,
        TransactionState::InTransaction
    );
    release(&studio, &events);
    let probe = run(&studio, &session, "SELECT count(*) FROM script_tx_probe").await;
    let result = probe
        .iter()
        .find_map(|event| match event {
            QueryEvent::Result { result } => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        studio.results.row(&result.id, 0).unwrap()[0].as_deref(),
        Some("1")
    );
    release(&studio, &probe);
    assert_eq!(
        studio
            .rollback_transaction(&session.id)
            .await
            .unwrap()
            .transaction_state,
        TransactionState::Idle
    );
    let probe = run(&studio, &session, "SELECT count(*) FROM script_tx_probe").await;
    let result = probe
        .iter()
        .find_map(|event| match event {
            QueryEvent::Result { result } => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        studio.results.row(&result.id, 0).unwrap()[0].as_deref(),
        Some("0")
    );
    studio.disconnect_and_wait(&session.id).await.unwrap();
}

async fn observer() -> (tokio_postgres::Client, tokio::task::JoinHandle<()>) {
    let mut config = tokio_postgres::Config::new();
    config
        .host("127.0.0.1")
        .port(
            std::env::var("JOINERY_TEST_PORT")
                .unwrap_or_else(|_| "55432".into())
                .parse()
                .unwrap(),
        )
        .user("joinery")
        .password("joinery-local-only")
        .dbname("joinery");
    let (client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
    let pump = tokio::spawn(async move {
        let _ = connection.await;
    });
    (client, pump)
}

async fn wait_for_sleep(client: &tokio_postgres::Client, session: &SessionInfo) -> i32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let rows = client.query(
                "SELECT pid FROM pg_stat_activity WHERE application_name=$1 AND state='active' AND query='SELECT pg_sleep(10)'",
                &[&format!("Joinery {}", session.id)],
            ).await.unwrap();
            if let Some(row) = rows.first() { return row.get(0); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("second statement must be executing in the real database")
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; cancels only this test's synthetic sleep"]
async fn postgres_cancellation_preserves_prefix_and_stops_suffix() {
    let (_root, studio, session) = setup(32 * 1024, 3).await;
    let (observer, pump) = observer().await;
    let (run_id, rx) = start(
        &studio,
        &session,
        "SELECT 7; SELECT pg_sleep(10); SELECT 987654321",
    );
    wait_for_sleep(&observer, &session).await;
    studio.cancel(&run_id).await.unwrap();
    let events = finish(rx).await;
    let done = terminal(&events);
    assert_eq!(done["status"], "cancelled");
    assert_eq!(
        done["executionSummary"],
        serde_json::json!({
            "planned": 3, "attempted": 2, "completed": 1, "notAttempted": 1,
            "stopReason": "cancelled"
        })
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, QueryEvent::StatementStarted { index: 2, .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, QueryEvent::SessionClosed { .. }))
    );
    assert!(studio.health(&session.id).await.is_err());
    drop(observer);
    pump.await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; terminates only this test's own synthetic session"]
async fn postgres_interrupted_connection_preserves_prefix_without_replaying_suffix() {
    let (_root, studio, session) = setup(32 * 1024, 3).await;
    let (observer, pump) = observer().await;
    let (_run_id, rx) = start(
        &studio,
        &session,
        "SELECT 7; SELECT pg_sleep(10); SELECT 987654321",
    );
    let pid = wait_for_sleep(&observer, &session).await;
    let terminated: bool = observer
        .query_one("SELECT pg_terminate_backend($1)", &[&pid])
        .await
        .unwrap()
        .get(0);
    assert!(terminated);
    let events = finish(rx).await;
    let done = terminal(&events);
    // Administrative termination returns an explicit PostgreSQL 57P01 error.
    // This is a server-reported failure, rather than an unacknowledged socket
    // loss. Do not substitute it for the separate unknown-outcome regression.
    assert!(events.iter().any(|event| matches!(event,
        QueryEvent::Error { error, statement_index: 1 }
            if error.code == ErrorCode::Database
                && error.location.as_ref().and_then(|location| location.sql_state.as_deref()) == Some("57P01"))));
    assert_eq!(done["status"], "failed");
    assert_eq!(
        done["executionSummary"],
        serde_json::json!({
            "planned": 3, "attempted": 2, "completed": 1, "notAttempted": 1,
            "stopReason": "failed"
        })
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, QueryEvent::StatementStarted { index: 2, .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, QueryEvent::SessionClosed { .. }))
    );
    assert!(studio.health(&session.id).await.is_err());
    drop(observer);
    pump.await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; only fixed synthetic scalar reads"]
async fn postgres_history_admission_preserves_exact_original_before_dispatch_and_restart() {
    let (root, studio, session) = setup_with_text(32 * 1024, 100, true).await;
    let sql = " /* keep original 雪 */\nSELECT 42 AS answer;  -- trailing comment\n";
    let plan = studio.prepare_execution(&session.id, sql).unwrap();
    let reader = studio.clone();
    let (tx, rx) = mpsc::unbounded_channel();
    let id = studio
        .execute(
            &plan.id,
            None,
            Arc::new(move |event| {
                if let QueryEvent::Started { run_id } = &event {
                    let entry = reader.history_entry(run_id).unwrap().unwrap();
                    assert_eq!(entry.sql.as_deref(), Some(sql));
                    assert_eq!(entry.provenance, HistoryProvenance::Admitted);
                    assert_eq!(entry.status, None);
                    assert_eq!(entry.elapsed_ms, None);
                }
                tx.send(event).is_ok()
            }),
        )
        .unwrap();
    let events = finish(rx).await;
    assert_eq!(terminal(&events)["status"], "completed");
    let result = events
        .iter()
        .find_map(|e| match e {
            QueryEvent::Result { result } => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        studio.results.row(&result.id, 0).unwrap()[0].as_deref(),
        Some("42")
    );
    release(&studio, &events);
    studio.shutdown_and_wait().await.unwrap();
    drop(studio);
    let restarted = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let entry = restarted.history_entry(&id).unwrap().unwrap();
    assert_eq!(entry.sql.as_deref(), Some(sql));
    assert_eq!(entry.status, Some(RunStatus::Completed));
    assert_eq!(entry.provenance, HistoryProvenance::Recorded);
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; only synthetic scalar reads and own-session sleep"]
async fn postgres_history_privacy_actions_cannot_be_undone_by_late_completion() {
    for action in ["clear", "optout-reenable", "delete-recreate"] {
        let (root, studio, session) = setup_with_text(32 * 1024, 100, true).await;
        let (observer, pump) = observer().await;
        let sql = "SELECT pg_sleep(1)";
        let (id, rx) = start(&studio, &session, sql);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let count: i64 = observer.query_one("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND state='active' AND query=$2", &[&format!("Joinery {}",session.id), &sql]).await.unwrap().get(0);
                if count == 1 { break; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert_eq!(
            studio.history_entry(&id).unwrap().unwrap().sql.as_deref(),
            Some(sql)
        );
        let mut profile = studio
            .bootstrap()
            .unwrap()
            .profiles
            .into_iter()
            .find(|p| p.id == session.connection_id)
            .unwrap();
        match action {
            "clear" => studio.clear_history().unwrap(),
            "optout-reenable" => {
                profile.store_query_text = false;
                studio.save_profile(profile.clone(), None).unwrap();
                assert!(studio.history_entry(&id).unwrap().unwrap().sql.is_none());
                profile.store_query_text = true;
                studio.save_profile(profile, None).unwrap();
            }
            "delete-recreate" => {
                studio.delete_profile(&profile.id).unwrap();
                studio.save_profile(profile, None).unwrap();
            }
            _ => unreachable!(),
        }
        let events = finish(rx).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, QueryEvent::Finished { .. }))
        );
        let entry = studio.history_entry(&id).unwrap();
        if action == "clear" {
            assert!(entry.is_none());
        } else {
            assert!(entry.unwrap().sql.is_none());
        }
        studio.shutdown_and_wait().await.unwrap();
        drop(studio);
        let restarted =
            Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        assert!(
            restarted
                .history_entry(&id)
                .unwrap()
                .is_none_or(|e| e.sql.is_none()),
            "{action} resurrected SQL on restart"
        );
        drop(observer);
        pump.await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL; only fixed synthetic scalar reads"]
async fn postgres_history_clear_during_failed_completion_retry_never_reinserts_sql() {
    let (root, studio, session) = setup_with_text(32 * 1024, 100, true).await;
    let database = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
    database.execute_batch("CREATE TRIGGER reject_history_completion BEFORE UPDATE ON history_v6 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    let (id, rx) = start(&studio, &session, "SELECT 123 AS retained_answer");
    let events = finish(rx).await;
    assert_eq!(terminal(&events)["status"], "completed");
    assert!(
        events.iter().any(
            |e| matches!(e, QueryEvent::Notice { message } if message.contains("local history"))
        )
    );
    assert_eq!(
        studio.history_entry(&id).unwrap().unwrap().provenance,
        HistoryProvenance::Admitted
    );
    release(&studio, &events);
    studio.clear_history().unwrap();
    database
        .execute_batch("DROP TRIGGER reject_history_completion")
        .unwrap();
    studio.shutdown_and_wait().await.unwrap();
    assert!(studio.history_entry(&id).unwrap().is_none());
    drop(studio);
    let restarted = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    assert!(restarted.history_entry(&id).unwrap().is_none());
}
