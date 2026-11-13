//! Studio -> verified private Node -> real helper -> synthetic SDK, with real
//! SQLite receipts and ResultStore. These tests never load an account/profile.
use super::*;
use crate::{secrets::MemorySecrets, snowflake_runtime::studio_fixture::Fixture};

pub(super) static FIXTURE_LANE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(super) fn profile() -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Synthetic retained console".into(),
        group_id: "local".into(),
        engine: EngineKind::Snowflake,
        host: "org-account.snowflakecomputing.com".into(),
        port: 443,
        database: "DB".into(),
        username: "login@example.test".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::Confirm,
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
            reviewed_console: true,
            reviewed_console_timeout_seconds: None,
            role: "ROLE".into(),
            warehouse: "WH".into(),
            schema: "PUBLIC".into(),
        }),
    }
}

pub(super) fn new_studio(f: &Fixture) -> Arc<Studio> {
    Studio::with_snowflake_runtime(
        &f.directory.path().join("workspace"),
        Arc::new(MemorySecrets::default()),
        Arc::new(DisabledFileDialog),
        Some(f.runtime.clone()),
    )
    .unwrap()
}

pub(super) async fn connected(f: &Fixture) -> (Arc<Studio>, SessionInfo) {
    let studio = new_studio(f);
    let profile = studio.save_profile(profile(), None).unwrap();
    let info = studio
        .connect(&profile.id, ConnectionSecrets::default())
        .await
        .unwrap();
    assert!(info.console_mode);
    (studio, info)
}

fn request(sql: &str) -> SnowflakeConsoleRequest {
    SnowflakeConsoleRequest {
        sql: sql.into(),
        mode: SnowflakeConsoleMode::ParsedStatements,
        parameters: None,
    }
}

pub(super) fn approved(studio: &Studio, session: &str, sql: &str) -> SnowflakeConsoleStart {
    let plan = studio
        .prepare_snowflake_console(session, &request(sql))
        .unwrap();
    assert!(plan.review.requires_approval);
    let approval = studio.authorize(&plan.review.id, None, None).unwrap();
    SnowflakeConsoleStart {
        session_id: session.into(),
        plan_id: plan.review.id,
        plan_digest: plan.digest,
        client_request_id: new_id(),
        approval: Some(approval),
    }
}

pub(super) fn queries(f: &Fixture) -> Vec<String> {
    std::fs::read_to_string(f.controls.join("queries"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

pub(super) async fn finished(
    studio: &Studio,
    request: &SnowflakeConsoleStart,
) -> SnowflakeConsoleRunView {
    tokio::time::timeout(Duration::from_secs(85), async {
        loop {
            let view = studio
                .snowflake_console_run(&request.client_request_id)
                .unwrap()
                .unwrap();
            if view.run.terminal.is_some()
                && !studio.jobs.lock().contains_key(&view.run.metadata.run_id)
                && !studio.console.publisher_active(&view.run.metadata.run_id)
            {
                return view;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("tracked console owner must finish")
}

async fn marker(f: &Fixture, name: &str) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !f.controls.join(name).exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("synthetic callback barrier must be reached");
}

pub(super) fn start_run(
    studio: &Arc<Studio>,
    request: &SnowflakeConsoleStart,
    events: EventHandler,
) -> AppResult<String> {
    match studio.start_snowflake_console(request, events)? {
        SnowflakeConsoleStartResult::Started { run_id } => Ok(run_id),
        SnowflakeConsoleStartResult::NotAdmitted { code, .. } => {
            Err(AppError::new(code, "Definite local admission rejection"))
        }
    }
}

async fn run(studio: &Arc<Studio>, session: &str, sql: &str) -> SnowflakeConsoleRunView {
    let request = approved(studio, session, sql);
    start_run(studio, &request, Arc::new(|_| true)).unwrap();
    finished(studio, &request).await
}

pub(super) fn database(f: &Fixture) -> rusqlite::Connection {
    rusqlite::Connection::open(f.directory.path().join("workspace/joinery.sqlite")).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reviewed_console_prepare_preserves_permitted_sql_before_started_or_dispatch() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let studio = new_studio(&f);
    let mut saved = profile();
    saved.store_query_text = true;
    let saved = studio.save_profile(saved, None).unwrap();
    let info = studio
        .connect(&saved.id, ConnectionSecrets::default())
        .await
        .unwrap();
    let sql = "SELECT 1";
    let start = approved(&studio, &info.id, sql);
    let before = queries(&f);
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let seen_tx = Mutex::new(Some(seen_tx));
    let observed = studio.clone();
    let run_id = start_run(
        &studio,
        &start,
        Arc::new(move |event| {
            if let QueryEvent::Started { run_id, .. } = event {
                let history = observed.persistence.history_entry(&run_id).unwrap();
                let receipt = observed.persistence.console_run(&run_id).unwrap();
                seen_tx
                    .lock()
                    .take()
                    .unwrap()
                    .send((history, receipt))
                    .unwrap();
                // Reject publication before the first admission. Neither this
                // check nor recovery may execute the retained statement.
                return false;
            }
            true
        }),
    )
    .unwrap();
    let (history, receipt) = tokio::time::timeout(Duration::from_secs(15), seen_rx)
        .await
        .unwrap()
        .unwrap();
    let history = history.expect("Prepare must durably preserve history before Started");
    assert_eq!(history.id, run_id);
    assert_eq!(history.sql.as_deref(), Some(sql));
    assert_eq!(history.provenance, HistoryProvenance::Admitted);
    assert_eq!(history.status, None);
    assert_eq!(history.elapsed_ms, None);
    let receipt = receipt.expect("positive admission must be durable before Started");
    assert!(receipt.steps.is_empty());
    assert!(receipt.terminal.is_none());
    finished(&studio, &start).await;
    assert_eq!(queries(&f), before);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_retains_one_session_across_runs_without_implicit_sql() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let bootstrap = queries(&f);
    assert_eq!(
        bootstrap.len(),
        2,
        "only disclosed identity/version bootstrap"
    );
    let first = run(&studio, &info.id, "SHOW TABLES").await;
    assert_eq!(
        first.run.terminal.unwrap().reason,
        ConsoleStopReason::Completed
    );
    let second = run(
        &studio,
        &info.id,
        "SELECT * FROM TABLE(RESULT_SCAN(LAST_QUERY_ID()))",
    )
    .await;
    assert_eq!(second.run.terminal.unwrap().accounting.acknowledged, 1);
    assert_eq!(
        studio
            .results
            .batch_rows(&second.results[0].id, 0, &[0])
            .unwrap()
            .unwrap(),
        vec![vec![Some("CONSOLE_TEMP".into())]]
    );
    let third = run(&studio, &info.id, "SET CONSOLE_VALUE = 'same session'; BEGIN TRANSACTION; CREATE TEMPORARY TABLE CONSOLE_TEMP (V NUMBER)").await;
    assert_eq!(third.run.terminal.unwrap().accounting.acknowledged, 3);
    let fourth = run(
        &studio,
        &info.id,
        "INSERT INTO CONSOLE_TEMP VALUES (7); SELECT * FROM CONSOLE_TEMP; SELECT $CONSOLE_VALUE",
    )
    .await;
    assert_eq!(fourth.run.terminal.unwrap().accounting.acknowledged, 3);
    assert_eq!(
        studio
            .results
            .batch_rows(&fourth.results[1].id, 0, &[0])
            .unwrap()
            .unwrap(),
        vec![vec![Some("7".into())]]
    );
    assert_eq!(
        studio
            .results
            .batch_rows(&fourth.results[2].id, 0, &[0])
            .unwrap()
            .unwrap(),
        vec![vec![Some("same session".into())]]
    );
    assert!(
        fourth
            .results
            .iter()
            .all(|r| matches!(&r.editability, Some(ResultEditability::ReadOnly { .. })))
    );
    let fifth = run(&studio, &info.id, "ROLLBACK").await;
    assert_eq!(
        fifth.run.terminal.unwrap().reason,
        ConsoleStopReason::Completed
    );
    let driver = studio.session(&info.id).unwrap().driver.clone();
    assert!(driver.health().await.is_err());
    assert!(driver.session_state(&info.id).await.is_err());
    assert!(studio.session_contexts(&info.id).await.is_err());
    assert!(studio.completion_catalog(&info.id).await.is_err());
    let before_idle = queries(&f);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(queries(&f), before_idle);
    assert_eq!(
        &before_idle[2..],
        [
            "SHOW TABLES",
            "SELECT * FROM TABLE(RESULT_SCAN(LAST_QUERY_ID()))",
            "SET CONSOLE_VALUE = 'same session'",
            "BEGIN TRANSACTION",
            "CREATE TEMPORARY TABLE CONSOLE_TEMP (V NUMBER)",
            "INSERT INTO CONSOLE_TEMP VALUES (7)",
            "SELECT * FROM CONSOLE_TEMP",
            "SELECT $CONSOLE_VALUE",
            "ROLLBACK"
        ]
    );
    assert!(
        studio
            .history("")
            .unwrap()
            .iter()
            .all(|h| h.sql.is_none() && h.execution_summary.is_none())
    );
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_whole_review_approval_and_client_dedup_are_one_use() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    assert!(
        studio
            .prepare_snowflake_console(&info.id, &request("SELECT 1; SELECT ("))
            .is_err()
    );
    assert_eq!(queries(&f).len(), 2);
    let mut start = approved(&studio, &info.id, "SELECT 1");
    let approval = start.approval.take();
    assert_eq!(
        start_run(&studio, &start, Arc::new(|_| true))
            .unwrap_err()
            .code,
        ErrorCode::ApprovalRequired
    );
    // A refused client is immutable even after authorization is restored.
    start.approval = approval;
    assert!(matches!(
        studio
            .start_snowflake_console(&start, Arc::new(|_| true))
            .unwrap(),
        SnowflakeConsoleStartResult::NotAdmitted {
            code: ErrorCode::ApprovalRequired,
            ..
        }
    ));
    assert_eq!(queries(&f).len(), 2);
    assert!(matches!(
        studio
            .snowflake_console_request(&start.client_request_id)
            .unwrap(),
        Some(SnowflakeConsoleRequestLookup::NotAdmitted {
            code: ErrorCode::ApprovalRequired,
            ..
        })
    ));
    start.client_request_id = new_id();
    let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    assert_eq!(
        start_run(
            &studio,
            &start,
            Arc::new(|_| panic!("duplicate must not create another event owner"))
        )
        .unwrap(),
        id
    );
    let view = finished(&studio, &start).await;
    assert_eq!(view.run.terminal.unwrap().accounting.acknowledged, 1);
    let mut changed = start.clone();
    changed.plan_digest = "0".repeat(64);
    assert_eq!(
        start_run(&studio, &changed, Arc::new(|_| true))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let mut replay = start.clone();
    replay.client_request_id = new_id();
    assert_eq!(
        start_run(&studio, &replay, Arc::new(|_| true))
            .unwrap_err()
            .code,
        ErrorCode::ApprovalExpired
    );
    studio.disconnect_and_wait(&info.id).await.unwrap();
    assert_eq!(start_run(&studio, &start, Arc::new(|_| true)).unwrap(), id);
    assert_eq!(queries(&f).len(), 3);
    assert_eq!(
        studio
            .snowflake_console_run(&start.client_request_id)
            .unwrap()
            .unwrap()
            .results
            .len(),
        1
    );
    studio.shutdown_and_wait().await.unwrap();
    let reopened = new_studio(&f);
    let receipt = reopened
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.run.metadata.run_id, id);
    assert_eq!(receipt.run.terminal.unwrap().accounting.acknowledged, 1);
    assert!(
        receipt.results.is_empty(),
        "restart does not invent retained rows"
    );
    assert_eq!(queries(&f).len(), 3, "lookup never launches a helper");
    reopened.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_durably_records_ack_before_every_result_publication() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let start = approved(&studio, &info.id, "SELECT 1; SELECT 2");
    let observed = Arc::new(Mutex::new(Vec::new()));
    let output = observed.clone();
    let owner = studio.clone();
    start_run(
        &studio,
        &start,
        Arc::new(move |event| {
            if let QueryEvent::Result { result } = event {
                let durable = owner
                    .persistence
                    .console_run(&result.run_id)
                    .unwrap()
                    .unwrap();
                assert!(matches!(
                    &durable.steps[result.statement_index].outcome,
                    Some(ConsoleStepOutcome::Acknowledged(_))
                ));
                output.lock().push(result);
            }
            true
        }),
    )
    .unwrap();
    let view = finished(&studio, &start).await;
    assert_eq!(view.run.terminal.unwrap().accounting.acknowledged, 2);
    assert_eq!(view.results.len(), 2);
    assert!(observed.lock().len() >= 4);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_delivery_loss_retains_ack_and_rows_without_cancelling_or_commit_suffix() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    start_run(
        &studio,
        &start,
        Arc::new(|e| !matches!(e, QueryEvent::Progress { .. })),
    )
    .unwrap();
    let view = finished(&studio, &start).await;
    let terminal = view.run.terminal.unwrap();
    assert_eq!(terminal.reason, ConsoleStopReason::Failed);
    assert_eq!(terminal.accounting.acknowledged, 1);
    assert_eq!(terminal.accounting.not_attempted, 1);
    assert!(view.run.observations.delivery_failed);
    assert_eq!(view.results[0].retained_rows, 1);
    assert_eq!(view.results[0].status, RetentionStatus::Truncated);
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    let next = run(&studio, &info.id, "ROLLBACK").await;
    assert_eq!(
        next.run.terminal.unwrap().reason,
        ConsoleStopReason::Completed
    );
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_policy_rotation_stops_suffix_after_original_ack() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    std::fs::write(f.controls.join("hold-reviewed-ack"), "").unwrap();
    let (studio, info) = connected(&f).await;
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    marker(&f, "reviewed-ack-entered").await;
    studio
        .set_passphrase("a synthetic long passphrase", None)
        .unwrap();
    std::fs::write(f.controls.join("release-reviewed-ack"), "").unwrap();
    let view = finished(&studio, &start).await;
    assert_eq!(view.run.terminal.unwrap().accounting.acknowledged, 1);
    assert_eq!(
        view.run.steps.len(),
        1,
        "revoked unit receives neither intent nor dispatch"
    );
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_partial_passphrase_rotation_revokes_suffix_before_failed_settings_write()
{
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    std::fs::write(f.controls.join("hold-reviewed-ack"), "").unwrap();
    let (studio, info) = connected(&f).await;
    let previous = "previous synthetic passphrase";
    let replacement = "replacement synthetic passphrase";
    studio.set_passphrase(previous, None).unwrap();
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    marker(&f, "reviewed-ack-entered").await;
    let intent = studio.persistence.console_run(&id).unwrap().unwrap();
    assert_eq!(intent.steps.len(), 1);
    assert!(
        intent.steps[0].outcome.is_none(),
        "original response remains held"
    );
    let db = database(&f);
    db.execute_batch("CREATE TRIGGER deny_rotated_setting BEFORE UPDATE ON settings WHEN NEW.key = 'passphrase-configured' BEGIN SELECT RAISE(FAIL,'fixture settings failure after secret rotation'); END;").unwrap();
    assert!(studio.set_passphrase(replacement, Some(previous)).is_err());
    // Prove this exercised a partial mutation, not a pre-mutation validation error.
    studio.verify_passphrase(Some(replacement)).unwrap();
    assert!(studio.verify_passphrase(Some(previous)).is_err());
    std::fs::write(f.controls.join("release-reviewed-ack"), "").unwrap();
    let view = finished(&studio, &start).await;
    let terminal = view.run.terminal.unwrap();
    assert_eq!(terminal.accounting.acknowledged, 1);
    assert_eq!(terminal.accounting.not_attempted, 1);
    assert_eq!(
        &queries(&f)[2..],
        ["SELECT 1"],
        "partial policy mutation must revoke COMMIT before dispatch"
    );
    db.execute_batch("DROP TRIGGER deny_rotated_setting;")
        .unwrap();
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_cancellation_after_ack_preserves_original_and_skips_commit() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    std::fs::write(f.controls.join("hold-reviewed-drain"), "").unwrap();
    let (studio, info) = connected(&f).await;
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        marker(&f, "reviewed-drain-entered").await;
        // The helper can enter streamRows before the parent commits its ACK.
        // Cancel only after SQLite proves the acknowledgement is durable.
        loop {
            let run = studio.persistence.console_run(&id).unwrap().unwrap();
            if matches!(
                &run.steps[0].outcome,
                Some(ConsoleStepOutcome::Acknowledged(_))
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("original acknowledgement must commit before cancellation");
    assert!(matches!(
        &studio.persistence.console_run(&id).unwrap().unwrap().steps[0].outcome,
        Some(ConsoleStepOutcome::Acknowledged(_))
    ));
    studio.cancel(&id).await.unwrap();
    let view = finished(&studio, &start).await;
    assert_eq!(view.run.terminal.unwrap().accounting.acknowledged, 1);
    assert!(view.run.observations.cancellation_observed);
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_lost_original_is_unknown_and_never_replayed() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    std::fs::write(f.controls.join("lose-reviewed-ack"), "").unwrap();
    let (studio, info) = connected(&f).await;
    let start = approved(
        &studio,
        &info.id,
        "INSERT INTO CONSOLE_TEMP VALUES (7); COMMIT",
    );
    let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    let view = finished(&studio, &start).await;
    let counts = view.run.terminal.unwrap();
    assert_eq!(counts.reason, ConsoleStopReason::OutcomeUnknown);
    assert_eq!(counts.accounting.unknown, 1);
    assert_eq!(counts.accounting.not_attempted, 1);
    assert_eq!(start_run(&studio, &start, Arc::new(|_| true)).unwrap(), id);
    assert_eq!(queries(&f).len(), 3);
    assert!(
        studio
            .prepare_snowflake_console(&info.id, &request("COMMIT"))
            .is_err()
    );
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_ack_recording_failure_blocks_rows_suffix_and_shutdown_until_reconciled() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let db = database(&f);
    db.execute_batch("CREATE TRIGGER deny_console_ack BEFORE UPDATE OF outcome_json ON snowflake_console_steps_v5 BEGIN SELECT RAISE(FAIL,'fixture receipt failure'); END;").unwrap();
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    let results = Arc::new(Mutex::new(0));
    let observed = results.clone();
    start_run(
        &studio,
        &start,
        Arc::new(move |event| {
            if matches!(event, QueryEvent::Result { .. }) {
                *observed.lock() += 1;
            }
            true
        }),
    )
    .unwrap();
    let view = finished(&studio, &start).await;
    assert!(view.pending_recording);
    assert_eq!(
        view.run.terminal.unwrap().accounting.acknowledged,
        1,
        "in-memory original retained independently of persistence"
    );
    assert_eq!(
        *results.lock(),
        0,
        "mandatory ACK recorder precedes metadata publication"
    );
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    assert!(
        studio
            .prepare_snowflake_console(&info.id, &request("COMMIT"))
            .is_err()
    );
    assert!(
        studio.shutdown_and_wait().await.is_err(),
        "local failure must keep close barrier"
    );
    db.execute_batch("DROP TRIGGER deny_console_ack;").unwrap();
    let recovered = studio
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert!(!recovered.pending_recording);
    assert_eq!(
        recovered.run.terminal.unwrap().reason,
        ConsoleStopReason::RecordingFailed
    );
    assert_eq!(studio.history("").unwrap().len(), 1);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_history_and_audit_outbox_are_idempotent_across_disconnect() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let db = database(&f);
    db.execute_batch("CREATE TRIGGER deny_console_history BEFORE UPDATE ON history_v8 BEGIN SELECT RAISE(FAIL,'fixture history failure'); END;").unwrap();
    let start = approved(&studio, &info.id, "SELECT 1");
    start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    assert!(finished(&studio, &start).await.pending_recording);
    studio.disconnect_and_wait(&info.id).await.unwrap();
    assert!(studio.shutdown_and_wait().await.is_err());
    db.execute_batch("DROP TRIGGER deny_console_history; CREATE TRIGGER deny_console_audit BEFORE INSERT ON audit_v8 WHEN NEW.outcome != 'started' BEGIN SELECT RAISE(FAIL,'fixture audit failure'); END;").unwrap();
    assert!(
        studio
            .snowflake_console_run(&start.client_request_id)
            .unwrap()
            .unwrap()
            .pending_recording
    );
    assert_eq!(studio.history("").unwrap().len(), 1);
    assert_eq!(
        studio.history("").unwrap()[0].provenance,
        HistoryProvenance::Admitted
    );
    assert_eq!(studio.history("").unwrap()[0].status, None);
    db.execute_batch("DROP TRIGGER deny_console_audit;")
        .unwrap();
    for _ in 0..3 {
        assert!(
            !studio
                .snowflake_console_run(&start.client_request_id)
                .unwrap()
                .unwrap()
                .pending_recording
        );
    }
    assert_eq!(studio.history("").unwrap().len(), 1);
    assert_eq!(
        studio
            .persistence
            .audit_log()
            .unwrap()
            .iter()
            .filter(|a| a.action == "snowflake-console")
            .count(),
        2
    );
    assert_eq!(queries(&f).len(), 3);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_queued_cancel_has_no_intent_and_no_submission() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let session = studio.session(&info.id).unwrap();
    let lane = session.lane.lock().await;
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    studio.cancel(&id).await.unwrap();
    drop(lane);
    let view = finished(&studio, &start).await;
    assert!(view.run.steps.is_empty());
    let terminal = view.run.terminal.unwrap();
    assert_eq!(terminal.reason, ConsoleStopReason::Cancelled);
    assert_eq!(terminal.accounting.not_attempted, 2);
    assert_eq!(queries(&f).len(), 2);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_limit_and_definite_rejection_keep_distinct_originals_and_suffix() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let mut studio = new_studio(&f);
    Arc::get_mut(&mut studio).unwrap().results = ResultStore::new(RESULT_BUDGET, 0);
    let p = studio.save_profile(profile(), None).unwrap();
    let info = studio
        .connect(&p.id, ConnectionSecrets::default())
        .await
        .unwrap();
    let limited = run(&studio, &info.id, "SELECT 1; COMMIT").await;
    let terminal = limited.run.terminal.unwrap();
    assert_eq!(terminal.reason, ConsoleStopReason::ResultLimit);
    assert_eq!(terminal.accounting.acknowledged, 1);
    assert_eq!(terminal.accounting.not_attempted, 1);
    let rejected = run(&studio, &info.id, "SELECT CONSOLE_REJECT; COMMIT").await;
    let terminal = rejected.run.terminal.unwrap();
    assert_eq!(terminal.reason, ConsoleStopReason::Failed);
    assert_eq!(terminal.accounting.rejected, 1);
    assert_eq!(terminal.accounting.unknown, 0);
    assert_eq!(terminal.accounting.not_attempted, 1);
    assert_eq!(&queries(&f)[2..], ["SELECT 1", "SELECT CONSOLE_REJECT"]);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_restart_lookup_seals_abandoned_intent_without_helper_or_replay() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let studio = new_studio(&f);
    let metadata = ConsoleRunMetadata {
        run_id: new_id(),
        client_request_id: new_id(),
        session_id: new_id(),
        connection_id: new_id(),
        plan_digest: "a".repeat(64),
        step_count: 2,
        created_at: now_ms(),
    };
    let owner = studio.persistence.claim_console_journal_owner().unwrap();
    studio
        .persistence
        .console_prepare_run(&owner, metadata.clone())
        .unwrap();
    studio
        .persistence
        .console_step_intent(&owner, &metadata.run_id, 0, 0, now_ms())
        .unwrap();
    drop(owner);
    drop(studio);
    let reopened = new_studio(&f);
    let view = reopened
        .snowflake_console_run(&metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        view.run.terminal.unwrap().reason,
        ConsoleStopReason::Interrupted
    );
    assert!(matches!(
        &view.run.steps[0].outcome,
        Some(ConsoleStepOutcome::Unknown(_))
    ));
    assert!(!view.pending_recording);
    assert!(!f.controls.join("queries").exists());
    assert!(!f.controls.join("auth-started").exists());
    reopened.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_prepare_recording_failure_retains_identity_but_never_dispatches_on_recovery()
 {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    let db = database(&f);
    db.execute_batch("CREATE TRIGGER deny_console_prepare BEFORE INSERT ON snowflake_console_runs_v5 BEGIN SELECT RAISE(FAIL,'fixture prepare failure'); END;").unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let observed = started.clone();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let done_tx = Mutex::new(Some(done_tx));
    assert!(
        studio
            .start_snowflake_console(
                &start,
                Arc::new(move |event| {
                    match event {
                        QueryEvent::Started { .. } => observed.store(true, Ordering::Release),
                        QueryEvent::Finished { run_id, .. } => {
                            done_tx.lock().take().unwrap().send(run_id).unwrap();
                        }
                        _ => {}
                    }
                    true
                })
            )
            .is_err()
    );
    let id = tokio::time::timeout(Duration::from_secs(15), done_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !started.load(Ordering::Acquire),
        "unconfirmed Prepare is not positively published"
    );
    assert!(
        studio
            .snowflake_console_run(&start.client_request_id)
            .is_err()
    );
    assert_eq!(queries(&f).len(), 2);
    db.execute_batch("DROP TRIGGER deny_console_prepare;")
        .unwrap();
    let recovered = finished(&studio, &start).await;
    assert!(!recovered.pending_recording);
    assert_eq!(recovered.run.metadata.run_id, id);
    assert_eq!(
        recovered
            .run
            .terminal
            .as_ref()
            .unwrap()
            .accounting
            .not_attempted,
        2
    );
    assert_eq!(
        recovered.run.terminal.unwrap().reason,
        ConsoleStopReason::RecordingFailed
    );
    assert_eq!(start_run(&studio, &start, Arc::new(|_| true)).unwrap(), id);
    assert_eq!(queries(&f).len(), 2, "local flush never resumes SQL");
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_ambiguous_prepare_commit_reconciles_exact_positive_without_sql() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let studio = new_studio(&f);
    let mut saved = profile();
    saved.store_query_text = true;
    let saved = studio.save_profile(saved, None).unwrap();
    let info = studio
        .connect(&saved.id, ConnectionSecrets::default())
        .await
        .unwrap();
    let sql = "SELECT 1; COMMIT";
    let start = approved(&studio, &info.id, sql);
    let before = queries(&f);
    let started = Arc::new(AtomicBool::new(false));
    let observed = started.clone();
    studio.console.fail_next_prepare_confirmation();
    // The real transaction commits, its response is lost, and the start path
    // proves that exact positive by readback. The retained failed-admission
    // owner must still never dispatch when that recording ambiguity resolves.
    let id = start_run(
        &studio,
        &start,
        Arc::new(move |event| {
            if matches!(event, QueryEvent::Started { .. }) {
                observed.store(true, Ordering::Release);
            }
            true
        }),
    )
    .unwrap();
    let view = finished(&studio, &start).await;
    assert_eq!(view.run.metadata.run_id, id);
    assert!(!view.pending_recording);
    assert!(!started.load(Ordering::Acquire));
    assert!(view.run.steps.is_empty());
    let terminal = view.run.terminal.as_ref().unwrap();
    assert_eq!(terminal.reason, ConsoleStopReason::RecordingFailed);
    assert_eq!(terminal.accounting.not_attempted, 2);
    let history = studio.persistence.history_entry(&id).unwrap().unwrap();
    assert_eq!(history.sql.as_deref(), Some(sql));
    assert_eq!(history.status, Some(RunStatus::Failed));
    assert_eq!(
        start_run(
            &studio,
            &start,
            Arc::new(|_| panic!("dedup must not republish"))
        )
        .unwrap(),
        id
    );
    assert_eq!(queries(&f), before);
    studio.shutdown_and_wait().await.unwrap();
    let restarted = new_studio(&f);
    let restored = restarted
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.run, view.run);
    assert_eq!(
        restarted
            .persistence
            .history_entry(&id)
            .unwrap()
            .unwrap()
            .sql
            .as_deref(),
        Some(sql)
    );
    assert_eq!(queries(&f), before);
    restarted.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_pending_history_privacy_races_preserve_ack_without_text_resurrection() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    for mutation in ["clear", "opt-out-and-in", "delete"] {
        let f = Fixture::new();
        std::fs::write(f.controls.join("hold-reviewed-drain"), "").unwrap();
        let studio = new_studio(&f);
        let mut saved = profile();
        saved.store_query_text = true;
        let mut saved = studio.save_profile(saved, None).unwrap();
        let info = studio
            .connect(&saved.id, ConnectionSecrets::default())
            .await
            .unwrap();
        let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
        let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
        marker(&f, "reviewed-drain-entered").await;
        let original = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let run = studio.persistence.console_run(&id).unwrap().unwrap();
                if matches!(
                    run.steps[0].outcome,
                    Some(ConsoleStepOutcome::Acknowledged(_))
                ) {
                    break run.steps[0].outcome.clone();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let pending = studio.persistence.history_entry(&id).unwrap().unwrap();
        assert_eq!(pending.provenance, HistoryProvenance::Admitted);
        assert_eq!(pending.sql.as_deref(), Some("SELECT 1; COMMIT"));
        match mutation {
            "clear" => {
                studio.clear_history().unwrap();
                studio.cancel(&id).await.unwrap();
            }
            "opt-out-and-in" => {
                saved.store_query_text = false;
                saved = studio.save_profile(saved, None).unwrap();
                assert!(
                    studio
                        .persistence
                        .history_entry(&id)
                        .unwrap()
                        .unwrap()
                        .sql
                        .is_none()
                );
                saved.store_query_text = true;
                studio.save_profile(saved, None).unwrap();
            }
            "delete" => studio.delete_profile(&saved.id).unwrap(),
            _ => unreachable!(),
        }
        let view = finished(&studio, &start).await;
        assert!(
            !view.pending_recording,
            "privacy suppression settles local history"
        );
        assert_eq!(view.run.steps[0].outcome, original);
        assert_eq!(
            view.run.terminal.as_ref().unwrap().accounting.acknowledged,
            1
        );
        assert_eq!(
            view.run.terminal.as_ref().unwrap().accounting.not_attempted,
            1
        );
        let history = studio.persistence.history_entry(&id).unwrap();
        if mutation == "clear" {
            assert!(
                history.is_none(),
                "late completion cannot undo Clear history"
            );
        } else {
            assert!(
                history.unwrap().sql.is_none(),
                "late completion cannot restore redacted SQL"
            );
        }
        assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
        studio.shutdown_and_wait().await.unwrap();
        let restarted = new_studio(&f);
        let restored = restarted
            .snowflake_console_run(&start.client_request_id)
            .unwrap()
            .unwrap();
        assert_eq!(restored.run, view.run);
        let history = restarted.persistence.history_entry(&id).unwrap();
        assert!(history.as_ref().is_none_or(|entry| entry.sql.is_none()));
        assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
        restarted.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test]
async fn reviewed_console_scalar_binding_and_selected_unit_keep_values_out_of_receipts() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let value = "synthetic bound value ' ; COMMIT";
    let plan = studio
        .prepare_snowflake_console(
            &info.id,
            &SnowflakeConsoleRequest {
                sql: "SELECT :payload".into(),
                mode: SnowflakeConsoleMode::SelectedUnit,
                parameters: Some(vec![QueryParamInput {
                    key: "payload".into(),
                    kind: QueryParamKind::Text,
                    value: Some(value.into()),
                }]),
            },
        )
        .unwrap();
    assert_eq!(
        plan.validations,
        vec![SnowflakeConsoleValidation::ServerAtExecution]
    );
    assert_eq!(plan.review.statements[0].sql, "SELECT :payload");
    let approval = studio.authorize(&plan.review.id, None, None).unwrap();
    let start = SnowflakeConsoleStart {
        session_id: info.id,
        plan_id: plan.review.id,
        plan_digest: plan.digest,
        client_request_id: new_id(),
        approval: Some(approval),
    };
    start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    let view = finished(&studio, &start).await;
    assert_eq!(
        view.run.terminal.as_ref().unwrap().accounting.acknowledged,
        1
    );
    assert_eq!(
        studio
            .results
            .batch_rows(&view.results[0].id, 0, &[0])
            .unwrap()
            .unwrap(),
        vec![vec![Some(value.into())]]
    );
    assert!(!serde_json::to_string(&view.run).unwrap().contains(value));
    assert!(studio.history("").unwrap()[0].sql.is_none());
    let spec: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(f.controls.join("reviewed-specs"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert_eq!(spec["sql"], "SELECT ?");
    assert_eq!(
        spec["parameters"],
        serde_json::json!({"MULTI_STATEMENT_COUNT": 1})
    );
    assert_eq!(&queries(&f)[2..], ["SELECT ?"]);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_publication_panic_keeps_durable_ack_and_records_failure_without_unknown_or_suffix()
 {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    let final_status = Arc::new(Mutex::new(None));
    let observed = final_status.clone();
    start_run(
        &studio,
        &start,
        Arc::new(move |event| {
            match event {
                QueryEvent::Result { .. } => {
                    panic!("synthetic publication panic after mandatory durable ACK")
                }
                QueryEvent::Finished { status, .. } => *observed.lock() = Some(status),
                _ => {}
            }
            true
        }),
    )
    .unwrap();
    let view = finished(&studio, &start).await;
    let terminal = view.run.terminal.unwrap();
    assert_eq!(terminal.reason, ConsoleStopReason::Failed);
    assert_eq!(terminal.accounting.acknowledged, 1);
    assert_eq!(terminal.accounting.unknown, 0);
    assert_eq!(terminal.accounting.not_attempted, 1);
    assert_eq!(*final_status.lock(), Some(RunStatus::Failed));
    assert_eq!(view.results[0].status, RetentionStatus::Truncated);
    let history = studio.history("").unwrap();
    assert_eq!(history[0].status, Some(RunStatus::Failed));
    assert_eq!(history[0].provenance, HistoryProvenance::Recorded);
    assert_eq!(history[0].elapsed_ms, None);
    assert_eq!(
        studio
            .persistence
            .audit_log()
            .unwrap()
            .iter()
            .filter(|a| a.action == "snowflake-console")
            .count(),
        2
    );
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_definite_negative_is_distinct_from_unreadable_original_lookup() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let mut start = approved(&studio, &info.id, "SELECT 1");
    let approval = start.approval.take();
    match studio
        .start_snowflake_console(&start, Arc::new(|_| true))
        .unwrap()
    {
        SnowflakeConsoleStartResult::NotAdmitted {
            client_request_id,
            session_id,
            plan_digest,
            code,
        } => {
            assert_eq!(client_request_id, start.client_request_id);
            assert_eq!(session_id, start.session_id);
            assert_eq!(plan_digest, start.plan_digest);
            assert_eq!(code, ErrorCode::ApprovalRequired);
        }
        SnowflakeConsoleStartResult::Started { .. } => {
            panic!("missing approval must not create an owner")
        }
    }
    assert!(
        studio
            .snowflake_console_run(&start.client_request_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(queries(&f).len(), 2);
    start.approval = approval;
    start.client_request_id = new_id();
    let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    finished(&studio, &start).await;
    // Another coordinator has no in-memory evidence. Failure to read the
    // original cannot turn an already acknowledged action into NotAdmitted.
    let observer = new_studio(&f);
    let db = database(&f);
    db.execute_batch("ALTER TABLE snowflake_console_runs_v5 RENAME TO held_console_runs;")
        .unwrap();
    assert!(
        observer
            .start_snowflake_console(&start, Arc::new(|_| true))
            .is_err()
    );
    db.execute_batch("ALTER TABLE held_console_runs RENAME TO snowflake_console_runs_v5;")
        .unwrap();
    assert_eq!(
        start_run(&observer, &start, Arc::new(|_| true)).unwrap(),
        id
    );
    assert_eq!(queries(&f).len(), 3);
    observer.shutdown_and_wait().await.unwrap();
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_closed_coordinator_keeps_dedup_without_reclaiming_replacement_lease() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (closed, info) = connected(&f).await;
    let start = approved(&closed, &info.id, "SELECT 1");
    let id = start_run(&closed, &start, Arc::new(|_| true)).unwrap();
    finished(&closed, &start).await;
    closed.shutdown_and_wait().await.unwrap();

    // The old Arc remains live after shutdown. Known originals remain readable
    // and deduplicate, while a new identity must not reacquire the released lease.
    assert_eq!(start_run(&closed, &start, Arc::new(|_| true)).unwrap(), id);
    let mut fresh = start.clone();
    fresh.client_request_id = new_id();
    assert_eq!(
        closed
            .start_snowflake_console(&fresh, Arc::new(|_| true))
            .unwrap_err()
            .code,
        ErrorCode::ConnectionClosed
    );
    let replacement = new_studio(&f);
    let owner = replacement
        .persistence
        .claim_console_journal_owner()
        .expect("a rejected fresh request must not reclaim the replacement's lease");
    let metadata = ConsoleRunMetadata {
        run_id: new_id(),
        client_request_id: new_id(),
        session_id: new_id(),
        connection_id: new_id(),
        plan_digest: "a".repeat(64),
        step_count: 2,
        created_at: now_ms(),
    };
    replacement
        .persistence
        .console_prepare_run(&owner, metadata.clone())
        .unwrap();
    replacement
        .persistence
        .console_step_intent(&owner, &metadata.run_id, 0, 0, now_ms())
        .unwrap();
    drop(owner);

    // Lookup also precedes start's shutdown guard. An abandoned original absent
    // from this closed coordinator's memory stays pending, without local recovery.
    let pending = closed
        .snowflake_console_run(&metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert!(pending.pending_recording);
    assert!(pending.run.terminal.is_none());
    assert!(pending.run.steps[0].outcome.is_none());
    let original = SnowflakeConsoleStart {
        session_id: metadata.session_id.clone(),
        plan_id: new_id(),
        plan_digest: metadata.plan_digest.clone(),
        client_request_id: metadata.client_request_id.clone(),
        approval: None,
    };
    assert_eq!(
        start_run(&closed, &original, Arc::new(|_| true)).unwrap(),
        metadata.run_id
    );
    let recovered = replacement
        .snowflake_console_run(&metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered.run.terminal.unwrap().reason,
        ConsoleStopReason::Interrupted
    );
    assert!(matches!(
        &recovered.run.steps[0].outcome,
        Some(ConsoleStepOutcome::Unknown(_))
    ));
    assert!(!recovered.pending_recording);
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    replacement.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_committed_refusal_survives_restart_without_helper_activity() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let mut start = approved(&studio, &info.id, "INSERT INTO CONSOLE_TEMP VALUES (7)");
    start.approval = None;
    // Discard the native response: recovery must rely on the actual SQLite row.
    let _lost = studio
        .start_snowflake_console(
            &start,
            Arc::new(|_| panic!("refusal emits no execution event")),
        )
        .unwrap();
    let original = studio
        .persistence
        .console_rejection_by_client_request(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(original.code, ErrorCode::ApprovalRequired);
    assert_eq!(queries(&f).len(), 2);
    studio.shutdown_and_wait().await.unwrap();
    drop(studio);
    let reopened = new_studio(&f);
    let lookup = reopened
        .snowflake_console_request(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        lookup,
        SnowflakeConsoleRequestLookup::NotAdmitted {
            code: ErrorCode::ApprovalRequired,
            ..
        }
    ));
    assert!(
        reopened
            .snowflake_console_run(&start.client_request_id)
            .unwrap()
            .is_none()
    );
    start.approval = Some("changed authorization must not execute".into());
    assert!(matches!(
        reopened
            .start_snowflake_console(&start, Arc::new(|_| panic!("dedup cannot execute")))
            .unwrap(),
        SnowflakeConsoleStartResult::NotAdmitted {
            code: ErrorCode::ApprovalRequired,
            ..
        }
    ));
    assert_eq!(
        reopened
            .persistence
            .console_rejection_by_client_request(&start.client_request_id)
            .unwrap()
            .unwrap(),
        original
    );
    let mut changed = start.clone();
    changed.session_id = new_id();
    assert_eq!(
        reopened
            .start_snowflake_console(&changed, Arc::new(|_| true))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    changed = start.clone();
    changed.plan_digest = "f".repeat(64);
    assert_eq!(
        reopened
            .start_snowflake_console(&changed, Arc::new(|_| true))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        queries(&f).len(),
        2,
        "recovery cannot connect or run helper SQL"
    );
    reopened.shutdown_and_wait().await.unwrap();
    // A closed coordinator may read a known negative but cannot reclaim a lease.
    assert!(matches!(
        reopened
            .start_snowflake_console(&start, Arc::new(|_| true))
            .unwrap(),
        SnowflakeConsoleStartResult::NotAdmitted { .. }
    ));
    let replacement = new_studio(&f);
    let _lease = replacement
        .persistence
        .claim_console_journal_owner()
        .unwrap();
}

#[tokio::test]
async fn reviewed_console_refusal_write_failure_remains_frozen_until_sql_free_reconciliation() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let mut start = approved(&studio, &info.id, "SELECT 1");
    let approval = start.approval.take();
    let backup = studio.create_metadata_backup().unwrap();
    let db = database(&f);
    db.execute_batch("CREATE TRIGGER deny_console_refusal BEFORE INSERT ON snowflake_console_rejections_v5 BEGIN SELECT RAISE(FAIL,'fixture refusal failure'); END;").unwrap();
    assert!(
        studio
            .start_snowflake_console(&start, Arc::new(|_| true))
            .is_err()
    );
    let frozen = studio
        .console
        .pending_rejection(&start.client_request_id)
        .unwrap();
    assert_eq!(frozen.code, ErrorCode::ApprovalRequired);
    assert!(
        studio
            .snowflake_console_request(&start.client_request_id)
            .is_err()
    );
    start.approval = approval;
    assert!(
        studio
            .start_snowflake_console(&start, Arc::new(|_| true))
            .is_err()
    );
    assert_eq!(
        studio.console.pending_rejection(&start.client_request_id),
        Some(frozen.clone())
    );
    assert_eq!(queries(&f).len(), 2);
    assert!(
        studio.shutdown_and_wait().await.is_err(),
        "refusal-only outbox must hold shutdown"
    );
    assert!(
        studio
            .restore_metadata_backup(&backup.id, &backup.digest, "RESTORE", None)
            .is_err(),
        "metadata restore must not bypass an unrecorded refusal"
    );
    db.execute_batch("DROP TRIGGER deny_console_refusal;")
        .unwrap();
    assert!(matches!(
        studio
            .snowflake_console_request(&start.client_request_id)
            .unwrap(),
        Some(SnowflakeConsoleRequestLookup::NotAdmitted {
            code: ErrorCode::ApprovalRequired,
            ..
        })
    ));
    assert_eq!(
        studio
            .persistence
            .console_rejection_by_client_request(&start.client_request_id)
            .unwrap(),
        Some(frozen)
    );
    assert!(
        studio
            .console
            .pending_rejection(&start.client_request_id)
            .is_none()
    );
    assert_eq!(
        queries(&f).len(),
        2,
        "outbox reconciliation cannot submit SQL"
    );
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_malformed_request_identity_never_creates_a_refusal() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let original = approved(&studio, &info.id, "SELECT 1");
    for field in ["client", "session", "digest"] {
        let mut invalid = original.clone();
        match field {
            "client" => invalid.client_request_id = "not-a-uuid".into(),
            "session" => invalid.session_id = "not-a-uuid".into(),
            _ => invalid.plan_digest = "A".repeat(64),
        }
        assert_eq!(
            studio
                .start_snowflake_console(&invalid, Arc::new(|_| true))
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }
    let db = database(&f);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM snowflake_console_rejections_v5",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        studio
            .console
            .pending_rejection(&original.client_request_id)
            .is_none()
    );
    assert_eq!(queries(&f).len(), 2);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_timeout_policy_is_reviewed_hashed_and_revalidated_before_dispatch() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let studio = new_studio(&f);
    let mut p = profile();
    p.snowflake
        .as_mut()
        .unwrap()
        .reviewed_console_timeout_seconds = Some(120);
    let saved = studio.save_profile(p, None).unwrap();
    let info = studio
        .connect(&saved.id, ConnectionSecrets::default())
        .await
        .unwrap();
    let req = request("SELECT 1; COMMIT");
    let first = studio.prepare_snowflake_console(&info.id, &req).unwrap();
    assert_eq!(first.timeout_seconds, 120);
    assert!(
        first
            .review
            .warnings
            .iter()
            .any(|warning| warning.contains("120 second"))
    );
    let original_digest = first.digest.clone();
    // Hold the exact same PreparedPlan identity and SQL/binds while changing
    // only trusted policy. Its fingerprint must change independently of text.
    let changed = studio
        .store_plan(
            first.review.clone(),
            Action::SnowflakeConsole {
                request: req.clone(),
                compiled: safety::snowflake_script::compile(
                    &req.sql,
                    safety::snowflake_script::Mode::ParsedStatements,
                    req.parameters.as_deref(),
                )
                .unwrap(),
                mode: SafeMode::Confirm,
                timeout_seconds: 121,
            },
            0,
        )
        .unwrap();
    assert_ne!(
        studio.plans.lock().get(&changed.id).unwrap().digest,
        original_digest
    );
    let approval = studio.authorize(&changed.id, None, None).unwrap();
    let wrong_policy = SnowflakeConsoleStart {
        session_id: info.id.clone(),
        plan_id: changed.id,
        plan_digest: studio
            .plans
            .lock()
            .get(&first.review.id)
            .unwrap()
            .digest
            .clone(),
        client_request_id: new_id(),
        approval: Some(approval),
    };
    assert_eq!(
        start_run(&studio, &wrong_policy, Arc::new(|_| true))
            .unwrap_err()
            .code,
        ErrorCode::ApprovalExpired
    );
    assert_eq!(queries(&f).len(), 2);
    let prepared = studio.prepare_snowflake_console(&info.id, &req).unwrap();
    let approval = studio.authorize(&prepared.review.id, None, None).unwrap();
    let start = SnowflakeConsoleStart {
        session_id: info.id.clone(),
        plan_id: prepared.review.id,
        plan_digest: prepared.digest,
        client_request_id: new_id(),
        approval: Some(approval),
    };
    // Simulate persisted policy drift without Studio's proactive session close;
    // both prepare and start must independently reject the retained mismatch.
    let mut changed = saved.clone();
    changed
        .snowflake
        .as_mut()
        .unwrap()
        .reviewed_console_timeout_seconds = Some(121);
    studio.persistence.save_profile(&changed).unwrap();
    assert_eq!(
        studio
            .prepare_snowflake_console(&info.id, &req)
            .unwrap_err()
            .code,
        ErrorCode::ApprovalExpired
    );
    assert_eq!(
        start_run(&studio, &start, Arc::new(|_| true))
            .unwrap_err()
            .code,
        ErrorCode::ApprovalExpired
    );
    assert_eq!(queries(&f).len(), 2);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_timeout_profile_edit_retires_owner_and_preserves_ack_without_suffix() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    std::fs::write(f.controls.join("hold-reviewed-drain"), "").unwrap();
    let (studio, info) = connected(&f).await;
    assert_eq!(
        studio
            .prepare_snowflake_console(&info.id, &request("SELECT 1"))
            .unwrap()
            .timeout_seconds,
        65
    );
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            // Start returns the allocated ID before the retained owner has
            // necessarily committed its prepared row. Wait for durable ACK,
            // including that initial preparation interval.
            if studio
                .persistence
                .console_run(&id)
                .unwrap()
                .is_some_and(|run| {
                    run.steps.first().is_some_and(|s| {
                        matches!(s.outcome, Some(ConsoleStepOutcome::Acknowledged(_)))
                    })
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let mut changed = studio.persistence.profile(&info.connection_id).unwrap();
    changed
        .snowflake
        .as_mut()
        .unwrap()
        .reviewed_console_timeout_seconds = Some(120);
    studio.save_profile(changed, None).unwrap();
    let view = finished(&studio, &start).await;
    let accounting = view.run.terminal.unwrap().accounting;
    assert_eq!(accounting.acknowledged, 1);
    assert_eq!(accounting.not_attempted, 1);
    assert_eq!(view.run.steps.len(), 1);
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    assert!(
        studio
            .prepare_snowflake_console(&info.id, &request("COMMIT"))
            .is_err()
    );
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_timeout_durable_unknown_skips_suffix_and_lookup_never_replays() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    std::fs::write(f.controls.join("hold-reviewed-ack"), "").unwrap();
    let studio = new_studio(&f);
    let mut p = profile();
    p.snowflake
        .as_mut()
        .unwrap()
        .reviewed_console_timeout_seconds = Some(1);
    let saved = studio.save_profile(p, None).unwrap();
    let info = studio
        .connect(&saved.id, ConnectionSecrets::default())
        .await
        .unwrap();
    let start = approved(&studio, &info.id, "SELECT 1; COMMIT");
    let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    let view = tokio::time::timeout(Duration::from_secs(8), finished(&studio, &start))
        .await
        .unwrap();
    let terminal = view.run.terminal.unwrap();
    assert_eq!(terminal.reason, ConsoleStopReason::OutcomeUnknown);
    assert_eq!(terminal.accounting.unknown, 1);
    assert_eq!(terminal.accounting.not_attempted, 1);
    assert!(view.results.is_empty());
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    assert_eq!(start_run(&studio, &start, Arc::new(|_| true)).unwrap(), id);
    studio.shutdown_and_wait().await.unwrap();
    let reopened = new_studio(&f);
    let recovered = reopened
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered.run.terminal.unwrap().reason,
        ConsoleStopReason::OutcomeUnknown
    );
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    reopened.shutdown_and_wait().await.unwrap();
}
