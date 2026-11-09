use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
#[path = "common/commit_proxy.rs"]
mod commit_proxy;
use commit_proxy::{CommitProxy, Fault, Protocol};

async fn connect(engine: EngineKind, port: u16) -> (tempfile::TempDir, Arc<Studio>, SessionInfo) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "Disposable transaction transport".into(),
                group_id: "local".into(),
                engine,
                host: "127.0.0.1".into(),
                port,
                database: "joinery".into(),
                username: "joinery".into(),
                environment: Environment::Local,
                safe_mode: SafeMode::Silent,
                tls_mode: TlsMode::Disable,
                ca_cert_path: None,
                ssh: None,
                remember_password: false,
                store_query_text: false,
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
async fn run(studio: &Arc<Studio>, session: &SessionInfo, sql: &str) -> Vec<QueryEvent> {
    let plan = studio.prepare_execution(&session.id, sql).unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &plan.id,
            None,
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            let finished = matches!(event, QueryEvent::Finished { .. });
            events.push(event);
            if finished {
                break;
            }
        }
        assert!(
            events.iter().any(|e| matches!(
                e,
                QueryEvent::Finished {
                    status: RunStatus::Completed,
                    ..
                }
            )),
            "{sql}: {events:?}"
        );
        events
    })
    .await
    .expect("fixture query timed out")
}
async fn committed_value(studio: &Arc<Studio>, session: &SessionInfo, table: &str) -> String {
    let events = run(studio, session, &format!("SELECT value FROM {table}")).await;
    let id = events
        .iter()
        .find_map(|e| match e {
            QueryEvent::Result { result, .. } => Some(result.id.clone()),
            _ => None,
        })
        .unwrap();
    studio.results.row(&id, 0).unwrap()[0].clone().unwrap()
}

async fn commit_fault(engine: EngineKind, port: u16, fault: Fault) {
    let protocol = if engine == EngineKind::Postgres {
        Protocol::Postgres
    } else {
        Protocol::Mysql
    };
    let proxy = CommitProxy::start(protocol, port).await;
    let (_observer_root, observer, observer_session) = connect(engine, port).await;
    let (_root, studio, session) = connect(engine, proxy.port).await;
    let table = format!("commit_transport_{}", new_id().replace('-', ""));
    run(
        &observer,
        &observer_session,
        &format!("CREATE TABLE {table} (value INTEGER NOT NULL)"),
    )
    .await;
    run(
        &observer,
        &observer_session,
        &format!("INSERT INTO {table} VALUES (0)"),
    )
    .await;
    studio
        .set_transaction_mode(&session.id, TransactionMode::Manual)
        .await
        .unwrap();
    run(
        &studio,
        &session,
        &format!("UPDATE {table} SET value = value + 1"),
    )
    .await;
    proxy.arm(fault);
    let outcome = tokio::time::timeout(
        Duration::from_secs(15),
        studio.commit_transaction(&session.id),
    )
    .await;
    let value = committed_value(&observer, &observer_session, &table).await;
    run(&observer, &observer_session, &format!("DROP TABLE {table}")).await;
    assert!(
        proxy.acknowledged(),
        "proxy did not observe an actual server COMMIT acknowledgement"
    );
    assert!(
        proxy.triggered(),
        "proxy did not inject the requested fault"
    );
    assert_eq!(proxy.commits(), 1, "COMMIT was replayed");
    assert_eq!(value, "1", "server did not commit exactly once");
    let error = outcome
        .expect("COMMIT response deadline")
        .expect_err("fault must be reported");
    match fault {
        Fault::CommitAcknowledgement | Fault::CommitFatalResponse => {
            assert_eq!(error.code, ErrorCode::OutcomeUnknown, "{error:?}")
        }
        Fault::PostCommitRefresh => {
            assert_eq!(error.code, ErrorCode::ConnectionClosed, "{error:?}");
            assert_eq!(
                error.message,
                "Transaction committed, but session context could not be refreshed. Reconnect before continuing."
            );
        }
    }
    assert!(
        studio.prepare_execution(&session.id, "SELECT 1").is_err(),
        "terminal transaction failure left the session reusable"
    );
    observer.disconnect(&observer_session.id);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_lost_commit_acknowledgement_is_unknown_without_replay() {
    commit_fault(EngineKind::Postgres, 55432, Fault::CommitAcknowledgement).await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_terminal_error_without_commit_acknowledgement_is_unknown() {
    commit_fault(EngineKind::Postgres, 55432, Fault::CommitFatalResponse).await;
}
#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB"]
async fn mysql_lost_commit_acknowledgement_is_unknown_without_replay() {
    commit_fault(EngineKind::Mysql, 53306, Fault::CommitAcknowledgement).await;
}
#[tokio::test]
#[ignore = "requires disposable MariaDB"]
async fn mariadb_lost_commit_acknowledgement_is_unknown_without_replay() {
    commit_fault(EngineKind::Mysql, 53307, Fault::CommitAcknowledgement).await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_acknowledged_commit_with_refresh_failure_preserves_known_success() {
    commit_fault(EngineKind::Postgres, 55432, Fault::PostCommitRefresh).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_deferred_commit_failure_invalidates_plans_and_keeps_idle_session_usable() {
    let (_root, studio, session) = connect(EngineKind::Postgres, 55432).await;
    let table = format!("deferred_commit_{}", new_id().replace('-', ""));
    run(&studio, &session, &format!("CREATE TABLE public.{table} (id integer PRIMARY KEY, parent_id integer REFERENCES public.{table}(id) DEFERRABLE INITIALLY DEFERRED)")).await;
    studio
        .set_transaction_mode(&session.id, TransactionMode::Manual)
        .await
        .unwrap();
    run(
        &studio,
        &session,
        &format!("SET LOCAL search_path = pg_catalog; INSERT INTO public.{table} VALUES (1, 999)"),
    )
    .await;
    let pending = studio.prepare_execution(&session.id, "SELECT 1").unwrap();
    let error = studio.commit_transaction(&session.id).await.unwrap_err();
    let state = studio.session_contexts(&session.id).await.unwrap().state;
    let pending_rejected = studio
        .execute(&pending.id, None, Arc::new(|_| false))
        .is_err();
    studio
        .set_transaction_mode(&session.id, TransactionMode::AutoCommit)
        .await
        .unwrap();
    let count_events = run(
        &studio,
        &session,
        &format!("SELECT count(*) FROM public.{table}"),
    )
    .await;
    let count_id = count_events
        .iter()
        .find_map(|e| match e {
            QueryEvent::Result { result } => Some(result.id.clone()),
            _ => None,
        })
        .unwrap();
    let row = studio.results.row(&count_id, 0).unwrap();
    run(&studio, &session, &format!("DROP TABLE public.{table}")).await;
    studio.disconnect(&session.id);
    assert_eq!(error.code, ErrorCode::Database);
    assert_eq!(state.transaction_state, TransactionState::Idle);
    assert!(
        state.search_path.iter().any(|schema| schema == "public"),
        "SET LOCAL context was not restored: {state:?}"
    );
    assert_eq!(row[0].as_deref(), Some("0"));
    assert!(
        pending_rejected,
        "plan prepared before COMMIT failure survived the context transition"
    );
}
