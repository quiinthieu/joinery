use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
#[path = "common/commit_proxy.rs"]
mod commit_proxy;
use commit_proxy::{CommitProxy, Fault, Protocol};

async fn connect(engine: EngineKind, port: u16) -> (tempfile::TempDir, Arc<Studio>, SessionInfo) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let session = connect_profile(&studio, engine, port).await;
    (root, studio, session)
}
async fn connect_profile(studio: &Arc<Studio>, engine: EngineKind, port: u16) -> SessionInfo {
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
    studio
        .connect(
            &profile.id,
            ConnectionSecrets {
                password: Some("joinery-local-only".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
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

async fn reviewed_writer_fault(csv: bool, fault: Fault) {
    use joinery_core::file_capability::SandboxFileDialog;

    let proxy = CommitProxy::start(Protocol::Postgres, 55432).await;
    let (_observer_root, observer, observer_session) = connect(EngineKind::Postgres, 55432).await;
    let root = tempfile::tempdir().unwrap();
    let sandbox = SandboxFileDialog::new(&root.path().join("imports")).unwrap();
    let studio = Studio::with_secrets_and_dialog(
        root.path(),
        Arc::new(MemorySecrets::default()),
        sandbox.clone(),
    )
    .unwrap();
    let session = connect_profile(&studio, EngineKind::Postgres, proxy.port).await;
    let table = TableRef {
        schema: "public".into(),
        name: format!("reviewed_transport_{}", new_id().replace('-', "")),
    };
    run(
        &observer,
        &observer_session,
        &format!(
            "CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)",
            table.name
        ),
    )
    .await;
    let preview = if csv {
        std::fs::write(sandbox.root().join("row.csv"), "id,value\n1,1\n").unwrap();
        sandbox.queue_open(Some("row.csv")).unwrap();
        let grant = studio.pick_import_source().await.unwrap().unwrap();
        studio
            .prepare_csv_import_batch(
                &session.id,
                &grant.token,
                &table,
                CsvImportOptions::default(),
                vec![
                    CsvColumnMapping {
                        csv_index: 0,
                        target_column: Some("id".into()),
                    },
                    CsvColumnMapping {
                        csv_index: 1,
                        target_column: Some("value".into()),
                    },
                ],
                0,
            )
            .await
            .unwrap()
    } else {
        let page = studio.browse(&session.id, &table, None).await.unwrap();
        studio
            .prepare_table_changes(
                &session.id,
                TableChanges {
                    result_id: Some(page.result.id),
                    inserts: vec![RowInsert {
                        values: page
                            .result
                            .columns
                            .iter()
                            .map(|column| InsertValue {
                                column: column.id.clone(),
                                value: Some("1".into()),
                            })
                            .collect(),
                    }],
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let approval = studio.authorize(&preview.plan.id, None, None).unwrap();
    proxy.arm(fault);
    let (tx, mut rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &preview.plan.id,
            Some(&approval),
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .unwrap();
    let events = tokio::time::timeout(Duration::from_secs(15), async {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            let finished = matches!(event, QueryEvent::Finished { .. });
            events.push(event);
            if finished {
                break;
            }
        }
        events
    })
    .await
    .expect("reviewed writer response deadline");
    let value = committed_value(&observer, &observer_session, &table.name).await;
    run(
        &observer,
        &observer_session,
        &format!("DROP TABLE {}", table.name),
    )
    .await;
    assert!(proxy.acknowledged(), "server did not acknowledge COMMIT");
    assert_eq!(proxy.commits(), 1, "reviewed writer replayed COMMIT");
    assert_eq!(value, "1", "reviewed row did not commit exactly once");
    assert!(
        studio
            .execute(&preview.plan.id, Some(&approval), Arc::new(|_| true))
            .is_err(),
        "reviewed approval was reusable"
    );
    if matches!(fault, Fault::PostCommitRefresh) {
        assert!(
            !proxy.triggered(),
            "writer queried again after acknowledged COMMIT"
        );
        assert!(
            !events.iter().any(|event| matches!(
                event,
                QueryEvent::Error { .. } | QueryEvent::SessionClosed { .. }
            )),
            "{events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                QueryEvent::Finished {
                    status: RunStatus::Completed,
                    ..
                }
            )),
            "{events:?}"
        );
        assert_eq!(studio.usage().session_count, 1);
        studio.disconnect(&session.id);
        observer.disconnect(&observer_session.id);
        return;
    }
    assert!(proxy.triggered(), "fault did not follow a real COMMIT");
    assert!(events.iter().any(|event| matches!(event, QueryEvent::Error { error, .. } if error.code == ErrorCode::OutcomeUnknown)), "{events:?}");
    assert!(
        events.iter().any(|event| matches!(
            event,
            QueryEvent::Finished {
                status: RunStatus::OutcomeUnknown,
                ..
            }
        )),
        "{events:?}"
    );
    assert!(events.iter().any(|event| matches!(event, QueryEvent::SessionClosed { session_id, .. } if session_id == &session.id)), "terminal writer failure did not retire session: {events:?}");
    assert_eq!(studio.usage().session_count, 0);
    observer.disconnect(&observer_session.id);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_reviewed_grid_terminal_commit_response_is_unknown_without_replay() {
    reviewed_writer_fault(false, Fault::CommitFatalResponse).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_reviewed_csv_terminal_commit_response_is_unknown_without_replay() {
    reviewed_writer_fault(true, Fault::CommitFatalResponse).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_reviewed_grid_lost_commit_acknowledgement_is_unknown_without_replay() {
    reviewed_writer_fault(false, Fault::CommitAcknowledgement).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_reviewed_grid_acknowledged_commit_needs_no_context_refresh() {
    reviewed_writer_fault(false, Fault::PostCommitRefresh).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_fault_proxy_delivers_terminal_database_error() {
    let proxy = CommitProxy::start(Protocol::Postgres, 55432).await;
    let (client, connection) = tokio_postgres::Config::new()
        .host("127.0.0.1")
        .port(proxy.port)
        .user("joinery")
        .password("joinery-local-only")
        .dbname("joinery")
        .connect(tokio_postgres::NoTls)
        .await
        .unwrap();
    let task = tokio::spawn(connection);
    proxy.arm(Fault::CommitFatalResponse);
    let error = client.batch_execute("COMMIT").await.unwrap_err();
    task.abort();
    assert!(error.as_db_error().is_some(), "{error:?}");
    assert_eq!(error.code().unwrap().code(), "08006");
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn postgres_queued_reviewed_writer_cannot_reconnect_after_session_retirement() {
    let proxy = CommitProxy::start(Protocol::Postgres, 55432).await;
    let (_observer_root, observer, observer_session) = connect(EngineKind::Postgres, 55432).await;
    let (_root, studio, session) = connect(EngineKind::Postgres, proxy.port).await;
    let table = TableRef {
        schema: "public".into(),
        name: format!("queued_reviewed_{}", new_id().replace('-', "")),
    };
    run(&observer, &observer_session, &format!("CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER NOT NULL); INSERT INTO {} VALUES (1,0)", table.name, table.name)).await;
    let page = studio.browse(&session.id, &table, None).await.unwrap();
    let first = studio
        .prepare_table_changes(
            &session.id,
            TableChanges {
                result_id: Some(page.result.id.clone()),
                updates: vec![ResultChanges {
                    result_id: page.result.id.clone(),
                    changes: vec![CellChange {
                        row: 0,
                        column: page.result.columns[1].id.clone(),
                        value: Some("1".into()),
                    }],
                }],
                ..Default::default()
            },
        )
        .unwrap();
    let first_approval = studio.authorize(&first.plan.id, None, None).unwrap();
    // Keep the first writer alive while the second approved job acquires its
    // session reference and queues behind the same execution lane.
    run(
        &observer,
        &observer_session,
        &format!("BEGIN; UPDATE {} SET value=10 WHERE id=1", table.name),
    )
    .await;
    let before = proxy.connections();
    proxy.arm(Fault::CommitAcknowledgement);
    let (tx, mut first_rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &first.plan.id,
            Some(&first_approval),
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while proxy.connections() == before {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("first auxiliary writer did not connect");
    let first_connections = proxy.connections();
    let second = studio
        .prepare_table_changes(
            &session.id,
            TableChanges {
                result_id: Some(page.result.id.clone()),
                inserts: vec![RowInsert {
                    values: page
                        .result
                        .columns
                        .iter()
                        .enumerate()
                        .map(|(index, column)| InsertValue {
                            column: column.id.clone(),
                            value: Some(if index == 0 { "2" } else { "20" }.into()),
                        })
                        .collect(),
                }],
                ..Default::default()
            },
        )
        .unwrap();
    let second_approval = studio.authorize(&second.plan.id, None, None).unwrap();
    let (tx, mut second_rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &second.plan.id,
            Some(&second_approval),
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .unwrap();
    run(&observer, &observer_session, "ROLLBACK").await;
    let (first_events, second_events) = tokio::time::timeout(Duration::from_secs(15), async {
        let mut first_events = Vec::new();
        while let Some(event) = first_rx.recv().await {
            let done = matches!(event, QueryEvent::Finished { .. });
            first_events.push(event);
            if done {
                break;
            }
        }
        let mut second_events = Vec::new();
        while let Some(event) = second_rx.recv().await {
            let done = matches!(event, QueryEvent::Finished { .. });
            second_events.push(event);
            if done {
                break;
            }
        }
        (first_events, second_events)
    })
    .await
    .expect("queued writer deadline");
    let events = run(
        &observer,
        &observer_session,
        &format!("SELECT SUM(value), COUNT(*) FROM {}", table.name),
    )
    .await;
    let id = events
        .iter()
        .find_map(|event| match event {
            QueryEvent::Result { result } => Some(&result.id),
            _ => None,
        })
        .unwrap();
    let actual = observer.results.row(id, 0).unwrap();
    run(
        &observer,
        &observer_session,
        &format!("DROP TABLE {}", table.name),
    )
    .await;
    assert!(proxy.acknowledged() && proxy.triggered());
    assert!(
        first_events.iter().any(|event| matches!(
            event,
            QueryEvent::Finished {
                status: RunStatus::OutcomeUnknown,
                ..
            }
        )),
        "{first_events:?}"
    );
    assert_eq!(
        actual,
        vec![Some("1".into()), Some("1".into())],
        "queued writer reached the database after retirement"
    );
    assert_eq!(
        proxy.commits(),
        1,
        "queued writer committed after retirement"
    );
    assert_eq!(
        proxy.connections(),
        first_connections,
        "queued writer opened a fresh auxiliary connection after retirement"
    );
    assert!(
        second_events.iter().any(|event| matches!(
            event,
            QueryEvent::Finished {
                status: RunStatus::Cancelled,
                ..
            }
        )),
        "{second_events:?}"
    );
    assert!(
        studio
            .execute(&second.plan.id, Some(&second_approval), Arc::new(|_| true))
            .is_err()
    );
    assert_eq!(studio.usage().session_count, 0);
    observer.disconnect(&observer_session.id);
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
