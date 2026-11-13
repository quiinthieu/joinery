//! Opt-in process-crash qualification against the disposable loopback fixture.
//! The process is killed, without Studio shutdown/cancellation, while PostgreSQL
//! is demonstrably executing a statement. Remote outcome remains unknown.

use futures_util::FutureExt;
use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    panic::AssertUnwindSafe,
    path::Path,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;

const HELPER: &str = "postgres_history_crash_child";
const COMPLETED_SQL: &str =
    " /* completed original 雪 */\nSELECT 42 AS retained_answer /* keep me */;  \n";

#[derive(Serialize, Deserialize)]
struct Ready {
    child_id: u32,
    profile_id: String,
    session_id: String,
    run_id: String,
    completed_run_id: String,
    backend_pid: i32,
}

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        // Only the exact child spawned below is ever signalled or reaped.
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn port() -> u16 {
    std::env::var("JOINERY_TEST_PORT")
        .unwrap_or_else(|_| "55432".into())
        .parse()
        .unwrap()
}

async fn observer() -> (tokio_postgres::Client, tokio::task::JoinHandle<()>) {
    let mut config = tokio_postgres::Config::new();
    config
        .host("127.0.0.1")
        .port(port())
        .user("joinery")
        .password("joinery-local-only")
        .dbname("joinery")
        .connect_timeout(Duration::from_secs(5));
    let (client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
    let pump = tokio::spawn(async move {
        let _ = connection.await;
    });
    (client, pump)
}

fn pending_sql(table: &str, sequence: &str) -> String {
    format!(
        " /* admitted original 雪 */\nINSERT INTO {table} VALUES ('prefix', nextval('{sequence}'));\n\
         SELECT pg_sleep(60), '{table}' AS crash_probe;\n\
         INSERT INTO {table} VALUES ('suffix', nextval('{sequence}')) /* exact tail */;  \n"
    )
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
            Arc::new(move |event| {
                if matches!(&event, QueryEvent::Error { .. }) {
                    eprintln!(
                        "synthetic fixture query error: {}",
                        serde_json::to_string(&event).unwrap()
                    );
                }
                tx.send(event).is_ok()
            }),
        )
        .unwrap();
    (run_id, rx)
}

async fn wait_for_sleep(client: &tokio_postgres::Client, application: &str, table: &str) -> i32 {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let rows = client
                .query(
                    "SELECT pid FROM pg_stat_activity WHERE application_name=$1 \
                     AND state='active' AND wait_event='PgSleep' \
                     AND position($2 in query)>0",
                    &[&application, &table],
                )
                .await
                .unwrap();
            if let Some(row) = rows.first() {
                return row.get(0);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("owned child must reach actual PostgreSQL sleep")
}

async fn assert_no_replay(client: &tokio_postgres::Client, table: &str, sequence: &str) {
    let phases: Vec<String> = client
        .query(&format!("SELECT phase FROM {table} ORDER BY attempt"), &[])
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(phases, ["prefix"], "restart must not dispatch the suffix");
    let row = client
        .query_one(
            &format!("SELECT last_value, is_called FROM {sequence}"),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 1, "prefix must never be replayed");
    assert!(row.get::<_, bool>(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires isolated disposable loopback PostgreSQL; kills only an owned child and uses unique synthetic objects"]
async fn postgres_history_recovers_only_after_real_process_death_without_replay() {
    let root = tempfile::tempdir().unwrap();
    let token = new_id().replace('-', "");
    let table = format!("history_crash_{token}");
    let sequence = format!("history_attempt_{token}");
    let (observer, pump) = observer().await;
    observer
        .batch_execute(&format!(
            "CREATE TABLE {table} (phase text NOT NULL, attempt bigint NOT NULL); \
             CREATE SEQUENCE {sequence}; SET statement_timeout='5s'; SET lock_timeout='2s'"
        ))
        .await
        .unwrap();

    // Catch assertion unwinding so fixture cleanup also runs on a red baseline.
    let outcome = AssertUnwindSafe(async {
        let log = fs::File::create(root.path().join("child.log")).unwrap();
        let mut child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", HELPER, "--ignored", "--nocapture"])
                .env("JOINERY_HISTORY_CHILD_ROOT", root.path())
                .env("JOINERY_HISTORY_CHILD_TABLE", &table)
                .env("JOINERY_HISTORY_CHILD_SEQUENCE", &sequence)
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        let ready: Ready = tokio::time::timeout(Duration::from_secs(25), async {
            loop {
                let path = root.path().join("ready.json");
                if path.exists() {
                    break serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                }
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "child exited before readiness: {}",
                    fs::read_to_string(root.path().join("child.log")).unwrap()
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("child readiness deadline");
        assert_eq!(ready.child_id, child.0.id());
        assert_eq!(
            wait_for_sleep(&observer, &format!("Joinery {}", ready.session_id), &table).await,
            ready.backend_pid
        );
        assert_no_replay(&observer, &table, &sequence).await;

        // Opening another Studio must not steal recovery from a live owner.
        let live_reader =
            Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        let admitted = live_reader.history_entry(&ready.run_id).unwrap().unwrap();
        assert_eq!(
            admitted.sql.as_deref(),
            Some(pending_sql(&table, &sequence).as_str())
        );
        assert_eq!(admitted.provenance, HistoryProvenance::Admitted);
        assert_eq!(admitted.status, None);
        assert_eq!(admitted.elapsed_ms, None);
        drop(live_reader);
        assert!(child.0.try_wait().unwrap().is_none());

        child.0.kill().unwrap();
        let exit = child.0.wait().unwrap();
        assert!(
            !exit.success(),
            "the helper must die without graceful shutdown"
        );

        let restarted =
            Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        let interrupted = restarted.history_entry(&ready.run_id).unwrap().unwrap();
        assert_eq!(interrupted.connection_id, ready.profile_id);
        assert_eq!(
            interrupted.sql.as_deref(),
            Some(pending_sql(&table, &sequence).as_str())
        );
        assert_eq!(interrupted.provenance, HistoryProvenance::Interrupted);
        assert_eq!(
            interrupted.status, None,
            "no remote outcome was acknowledged"
        );
        assert_eq!(interrupted.elapsed_ms, None);
        assert!(interrupted.execution_summary.is_none());
        assert_eq!(interrupted.statement_count, 3);
        assert!(restarted.health(&ready.session_id).await.is_err());
        let completed = restarted
            .history_entry(&ready.completed_run_id)
            .unwrap()
            .unwrap();
        assert_eq!(completed.sql.as_deref(), Some(COMPLETED_SQL));
        assert_eq!(completed.provenance, HistoryProvenance::Recorded);
        assert_eq!(completed.status, Some(RunStatus::Completed));
        assert!(completed.elapsed_ms.is_some());

        // A restart is a local observation. It neither submits a new query nor
        // infers that killing a client cancelled or rolled back remote work.
        for _ in 0..10 {
            assert_no_replay(&observer, &table, &sequence).await;
            let rows = observer
                .query(
                    "SELECT pid FROM pg_stat_activity WHERE application_name LIKE 'Joinery %' \
                     AND position($1 in query)>0",
                    &[&table],
                )
                .await
                .unwrap();
            assert!(
                rows.iter()
                    .all(|row| row.get::<_, i32>(0) == ready.backend_pid)
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        drop(restarted);
    })
    .catch_unwind()
    .await;

    // A disconnected sleeping backend may survive the client process. Cancel
    // only this unique synthetic statement before dropping our fixture objects.
    let cleanup = async {
        observer
            .query(
                "SELECT pg_cancel_backend(pid) FROM pg_stat_activity \
                 WHERE application_name LIKE 'Joinery %' AND wait_event='PgSleep' \
                 AND position($1 in query)>0",
                &[&table],
            )
            .await?;
        observer
            .batch_execute(&format!("DROP TABLE {table}; DROP SEQUENCE {sequence}"))
            .await
    }
    .await;
    drop(observer);
    pump.await.unwrap();
    if let Err(panic) = outcome {
        assert!(cleanup.is_ok(), "fixture cleanup failed: {cleanup:?}");
        std::panic::resume_unwind(panic);
    }
    cleanup.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "child helper: invoked only by the process-death regression with disposable fixture environment"]
async fn postgres_history_crash_child() {
    // Explicitly running all ignored tests must not launch this helper alone.
    let Ok(root) = std::env::var("JOINERY_HISTORY_CHILD_ROOT") else {
        return;
    };
    let table = std::env::var("JOINERY_HISTORY_CHILD_TABLE").unwrap();
    let sequence = std::env::var("JOINERY_HISTORY_CHILD_SEQUENCE").unwrap();
    assert!(
        table.starts_with("history_crash_")
            && table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    );
    assert!(
        sequence.starts_with("history_attempt_")
            && sequence
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
    );
    let root = Path::new(&root);
    let studio = Studio::with_secrets(root, Arc::new(MemorySecrets::default())).unwrap();
    let profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "Process crash synthetic fixture".into(),
                group_id: "local".into(),
                engine: EngineKind::Postgres,
                host: "127.0.0.1".into(),
                port: port(),
                database: "joinery".into(),
                username: "joinery".into(),
                environment: Environment::Local,
                safe_mode: SafeMode::Silent,
                tls_mode: TlsMode::Disable,
                ca_cert_path: None,
                ssh: None,
                remember_password: false,
                store_query_text: true,
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
    let (completed_run_id, mut events) = start(&studio, &session, COMPLETED_SQL);
    let mut result_ids = Vec::new();
    let mut diagnostics = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.expect("completed run event stream");
            diagnostics.push(serde_json::to_value(&event).unwrap());
            match event {
                QueryEvent::Result { result } => result_ids.push(result.id),
                QueryEvent::Finished { status, .. } => {
                    assert_eq!(
                        status,
                        RunStatus::Completed,
                        "completed fixture events: {}",
                        serde_json::to_string_pretty(&diagnostics).unwrap()
                    );
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("completed history deadline");
    // Result announces column metadata before rows arrive; releasing it while
    // streaming can interrupt the fixture query. Retain until Finished.
    for id in result_ids {
        studio.release_result(&id);
    }
    let (run_id, _pending_events) = start(&studio, &session, &pending_sql(&table, &sequence));
    let (observer, _pump) = observer().await;
    let backend_pid = wait_for_sleep(&observer, &format!("Joinery {}", session.id), &table).await;
    assert_no_replay(&observer, &table, &sequence).await;
    let ready = Ready {
        child_id: std::process::id(),
        profile_id: profile.id,
        session_id: session.id,
        run_id,
        completed_run_id,
        backend_pid,
    };
    fs::write(root.join("ready.tmp"), serde_json::to_vec(&ready).unwrap()).unwrap();
    fs::rename(root.join("ready.tmp"), root.join("ready.json")).unwrap();
    // No shutdown, cancellation, or manually fabricated admission is involved.
    tokio::time::sleep(Duration::from_secs(45)).await;
    panic!("parent failed to kill the owned helper before its deadline");
}
