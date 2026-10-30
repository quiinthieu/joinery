use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

async fn setup(mode: SafeMode) -> (tempfile::TempDir, Arc<Studio>, SessionInfo) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "Integration".into(),
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
                safe_mode: mode,
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

fn start(
    studio: &Arc<Studio>,
    plan: &PreparedPlan,
    approval: Option<&str>,
) -> (String, mpsc::UnboundedReceiver<QueryEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let id = studio
        .execute(&plan.id, approval, Arc::new(move |e| tx.send(e).is_ok()))
        .unwrap();
    (id, rx)
}

async fn finish(mut rx: mpsc::UnboundedReceiver<QueryEvent>) -> Vec<QueryEvent> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            let done = matches!(event, QueryEvent::Finished { .. });
            events.push(event);
            if done {
                break;
            }
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, QueryEvent::Finished { .. })),
            "run did not finish"
        );
        events
    })
    .await
    .expect("query deadline")
}

async fn run(studio: &Arc<Studio>, session: &SessionInfo, sql: &str) -> Vec<QueryEvent> {
    let plan = studio.prepare_execution(&session.id, sql).unwrap();
    let approval = if plan.requires_approval {
        Some(
            studio
                .authorize(&plan.id, None, plan.confirmation_text.as_deref())
                .unwrap(),
        )
    } else {
        None
    };
    finish(start(studio, &plan, approval.as_deref()).1).await
}

fn status(events: &[QueryEvent]) -> RunStatus {
    events
        .iter()
        .find_map(|e| {
            if let QueryEvent::Finished { status, .. } = e {
                Some(*status)
            } else {
                None
            }
        })
        .unwrap()
}

fn result(events: &[QueryEvent]) -> ResultInfo {
    events
        .iter()
        .rev()
        .find_map(|e| {
            if let QueryEvent::Result { result } = e {
                Some(result.clone())
            } else {
                None
            }
        })
        .unwrap()
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn repeated_connect_run_release_disconnect_returns_resource_counters_to_zero() {
    let (_root, studio, initial) = setup(SafeMode::ReadOnly).await;
    let profile_id = studio.bootstrap().unwrap().profiles[0].id.clone();
    studio.disconnect(&initial.id);
    for _ in 0..10 {
        let session = studio
            .connect(
                &profile_id,
                ConnectionSecrets {
                    password: Some("joinery-local-only".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let sql = format!(
            "SELECT i, '{}' AS value FROM generate_series(1, 128) i",
            "x".repeat(1024)
        );
        let events = run(&studio, &session, &sql).await;
        assert_eq!(status(&events), RunStatus::Completed);
        let retained = result(&events);
        assert!(studio.usage().retained_bytes > 0);
        assert!(!studio.range(&retained.id, 0, 128).unwrap().is_empty());
        studio.release_result(&retained.id);
        studio.disconnect(&session.id);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let usage = studio.usage();
                if usage.active_runs == 0 && usage.session_count == 0 {
                    assert_eq!(usage.retained_bytes, 0);
                    assert_eq!(usage.result_count, 0);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("resource cleanup deadline");
    }
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn streams_exact_values_and_individual_results_without_disk_rows() {
    let (root, studio, session) = setup(SafeMode::Silent).await;
    let events=run(&studio,&session,"SELECT 9223372036854775807::bigint AS n, 12345678901234567890.123456789::numeric AS d, NULL::text AS nil, ''::text AS empty, 'private-result-needle'::text AS secret; SELECT 'héllo;世界'::text AS value").await;
    assert_eq!(status(&events), RunStatus::Completed);
    let completed = events
        .iter()
        .filter_map(|e| {
            if let QueryEvent::Result { result } = e {
                if result.status == RetentionStatus::Complete {
                    Some(result)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(completed.len(), 2);
    let first = studio.results.row(&completed[0].id, 0).unwrap();
    assert_eq!(first[0].as_deref(), Some("9223372036854775807"));
    assert_eq!(first[1].as_deref(), Some("12345678901234567890.123456789"));
    assert_eq!(first[2], None);
    assert_eq!(first[3].as_deref(), Some(""));
    assert_eq!(
        studio.results.row(&completed[1].id, 0).unwrap()[0].as_deref(),
        Some("héllo;世界")
    );
    let bytes = studio.range(&completed[0].id, 0, 1).unwrap();
    assert!(arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes), None).is_ok());
    assert!(studio.history("").unwrap().iter().all(|e| e.sql.is_none()));
    studio.disconnect(&session.id);
    assert_eq!(studio.usage().retained_bytes, 0);
    let saved = std::fs::read(root.path().join("joinery.sqlite")).unwrap();
    assert!(
        !saved
            .windows("private-result-needle".len())
            .any(|w| w == b"private-result-needle")
    );
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn grid_changes_are_bound_atomic_and_detect_concurrent_updates() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let name = format!("grid_{}", uuid::Uuid::new_v4().simple());
    let table = TableRef {
        schema: "public".into(),
        name: name.clone(),
    };
    let sql = format!(
        "CREATE TABLE public.\"{name}\" (tenant integer, id bigint, note text NOT NULL, amount numeric(30,9), PRIMARY KEY(tenant,id)); INSERT INTO public.\"{name}\" VALUES(1,1,'before',1.123456789),(1,2,'second',2.5)"
    );
    assert_eq!(
        status(&run(&studio, &session, &sql).await),
        RunStatus::Completed
    );
    let schema = studio.schema(&session.id, &table).await.unwrap();
    assert_eq!(schema.primary_key.len(), 2);
    assert!(schema.editable);
    let page = studio.browse(&session.id, &table, None).await.unwrap();
    let note = schema
        .columns
        .iter()
        .find(|c| c.name == "note")
        .unwrap()
        .id
        .clone();
    let malicious = "literal '; DROP TABLE customers; --";
    let preview = studio
        .prepare_changes(
            &session.id,
            &page.result.id,
            vec![CellChange {
                row: 0,
                column: note.clone(),
                value: Some(malicious.into()),
            }],
        )
        .unwrap();
    assert!(preview.plan.requires_approval);
    assert!(!preview.sql[0].contains(malicious));
    let approval = studio.authorize(&preview.plan.id, None, None).unwrap();
    let events = finish(start(&studio, &preview.plan, Some(&approval)).1).await;
    assert_eq!(status(&events), RunStatus::Completed);
    assert!(
        studio
            .execute(&preview.plan.id, Some(&approval), Arc::new(|_| true))
            .is_err()
    );
    let preview = studio
        .prepare_changes(
            &session.id,
            &page.result.id,
            vec![CellChange {
                row: 0,
                column: note,
                value: Some("stale overwrite".into()),
            }],
        )
        .unwrap();
    let approval = studio.authorize(&preview.plan.id, None, None).unwrap();
    let events = finish(start(&studio, &preview.plan, Some(&approval)).1).await;
    assert_eq!(status(&events), RunStatus::Failed);
    assert!(
        events
            .iter()
            .any(|e| matches!(e,QueryEvent::Error{error,..} if error.code==ErrorCode::Conflict))
    );
    let current = studio.browse(&session.id, &table, None).await.unwrap();
    assert_eq!(
        studio.results.row(&current.result.id, 0).unwrap()[2].as_deref(),
        Some(malicious)
    );
    let events = run(&studio, &session, &format!("DROP TABLE public.\"{name}\"")).await;
    assert_eq!(status(&events), RunStatus::Completed);
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn queued_cancellation_does_not_cancel_the_active_query() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let plan = studio
        .prepare_execution(&session.id, "SELECT pg_sleep(0.6)")
        .unwrap();
    let (_, mut first) = start(&studio, &plan, None);
    while !matches!(
        first.recv().await.unwrap(),
        QueryEvent::StatementStarted { .. }
    ) {}
    let queued = studio.prepare_execution(&session.id, "SELECT 2").unwrap();
    let (queued_id, queued_rx) = start(&studio, &queued, None);
    studio.cancel(&queued_id).await.unwrap();
    let events = finish(first).await;
    assert_eq!(status(&events), RunStatus::Completed);
    assert_eq!(status(&finish(queued_rx).await), RunStatus::Cancelled);
    assert_eq!(
        status(&run(&studio, &session, "SELECT 3").await),
        RunStatus::Completed
    );
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn explicit_cancel_quarantines_the_connection_and_limits_bound_retention() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let plan = studio
        .prepare_execution(&session.id, "SELECT pg_sleep(10)")
        .unwrap();
    let (id, mut rx) = start(&studio, &plan, None);
    while !matches!(
        rx.recv().await.unwrap(),
        QueryEvent::StatementStarted { .. }
    ) {}
    tokio::time::sleep(Duration::from_millis(100)).await;
    studio.cancel(&id).await.unwrap();
    let events = finish(rx).await;
    assert_eq!(status(&events), RunStatus::Cancelled);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, QueryEvent::SessionClosed { .. }))
    );
    assert_eq!(
        studio
            .prepare_execution(&session.id, "SELECT 1")
            .unwrap_err()
            .code,
        ErrorCode::ConnectionClosed
    );
    studio.shutdown();
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let events = run(&studio, &session, "SELECT generate_series(1,200000) AS n").await;
    assert_eq!(status(&events), RunStatus::Completed);
    let result = result(&events);
    assert_eq!(result.status, RetentionStatus::Truncated);
    assert!(result.retained_rows > 0 && result.retained_rows <= 100000);
    assert!(studio.usage().retained_bytes <= 64 * 1024 * 1024);
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn wide_pages_resume_without_skipping_and_cross_page_changes_rollback_together() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let name = format!("wide_{}", uuid::Uuid::new_v4().simple());
    let table = TableRef {
        schema: "public".into(),
        name: name.clone(),
    };
    assert_eq!(status(&run(&studio, &session, &format!("CREATE TABLE public.\"{name}\" (id integer PRIMARY KEY, note text NOT NULL); INSERT INTO public.\"{name}\" SELECT n,repeat('x',350000) FROM generate_series(1,20) n")).await), RunStatus::Completed);
    let first = studio.browse(&session.id, &table, None).await.unwrap();
    assert!(first.has_more && first.result.retained_rows < 500);
    let second = studio
        .browse(&session.id, &table, first.next_key.clone())
        .await
        .unwrap();
    let next_id = studio.results.row(&second.result.id, 0).unwrap()[0]
        .clone()
        .unwrap();
    assert_eq!(
        next_id.parse::<usize>().unwrap(),
        first.result.retained_rows + 1
    );
    let inputs = || {
        vec![
            ResultChanges {
                result_id: first.result.id.clone(),
                changes: vec![CellChange {
                    row: 0,
                    column: "c1".into(),
                    value: Some("first edit".into()),
                }],
            },
            ResultChanges {
                result_id: second.result.id.clone(),
                changes: vec![CellChange {
                    row: 0,
                    column: "c1".into(),
                    value: Some("second edit".into()),
                }],
            },
        ]
    };
    // A stale second target must roll back the first update in the same commit.
    run(
        &studio,
        &session,
        &format!("UPDATE public.\"{name}\" SET note='concurrent' WHERE id={next_id}"),
    )
    .await;
    let preview = studio.prepare_changes_batch(&session.id, inputs()).unwrap();
    assert_eq!(preview.row_count, 2);
    let approval = studio.authorize(&preview.plan.id, None, None).unwrap();
    let events = finish(start(&studio, &preview.plan, Some(&approval)).1).await;
    assert_eq!(status(&events), RunStatus::Failed);
    let events = run(
        &studio,
        &session,
        &format!("SELECT note FROM public.\"{name}\" WHERE id=1"),
    )
    .await;
    assert_eq!(
        studio.results.row(&result(&events).id, 0).unwrap()[0]
            .as_ref()
            .unwrap()
            .len(),
        350000
    );

    // A separate grid transaction cannot commit work pending in the SQL editor.
    let page = studio.browse(&session.id, &table, None).await.unwrap();
    run(
        &studio,
        &session,
        &format!("BEGIN; UPDATE public.\"{name}\" SET note='uncommitted' WHERE id=20"),
    )
    .await;
    let preview = studio
        .prepare_changes(
            &session.id,
            &page.result.id,
            vec![CellChange {
                row: 0,
                column: "c1".into(),
                value: Some("committed grid".into()),
            }],
        )
        .unwrap();
    let approval = studio.authorize(&preview.plan.id, None, None).unwrap();
    assert_eq!(
        status(&finish(start(&studio, &preview.plan, Some(&approval)).1).await),
        RunStatus::Completed
    );
    run(&studio, &session, "ROLLBACK").await;
    let events = run(
        &studio,
        &session,
        &format!("SELECT id,note FROM public.\"{name}\" WHERE id IN (1,20) ORDER BY id"),
    )
    .await;
    let id = result(&events).id;
    assert_eq!(
        studio.results.row(&id, 0).unwrap()[1].as_deref(),
        Some("committed grid")
    );
    assert_eq!(
        studio.results.row(&id, 1).unwrap()[1]
            .as_ref()
            .unwrap()
            .len(),
        350000
    );
    run(&studio, &session, &format!("DROP TABLE public.\"{name}\"")).await;
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn read_only_and_password_gates_apply_in_the_core() {
    let (_root, studio, session) = setup(SafeMode::ReadOnly).await;
    assert_eq!(
        studio
            .prepare_execution(&session.id, "UPDATE orders SET notes='blocked' WHERE id=-1")
            .unwrap_err()
            .code,
        ErrorCode::PolicyBlocked
    );
    let schema = studio
        .schema(
            &session.id,
            &TableRef {
                schema: "public".into(),
                name: "orders".into(),
            },
        )
        .await
        .unwrap();
    assert!(!schema.editable && schema.columns.iter().all(|c| !c.editable));
    studio.shutdown();

    let (_root, studio, session) = setup(SafeMode::PasswordGated).await;
    studio
        .set_passphrase("disposable-passphrase-only", None)
        .unwrap();
    let plan = studio
        .prepare_execution(&session.id, "UPDATE orders SET notes=notes WHERE id=-1")
        .unwrap();
    assert!(plan.requires_approval && plan.requires_passphrase);
    assert_eq!(
        studio
            .authorize(&plan.id, Some("incorrect"), None)
            .unwrap_err()
            .code,
        ErrorCode::InvalidPassphrase
    );
    let approval = studio
        .authorize(&plan.id, Some("disposable-passphrase-only"), None)
        .unwrap();
    assert_eq!(
        status(&finish(start(&studio, &plan, Some(&approval)).1).await),
        RunStatus::Completed
    );
    assert!(
        studio
            .execute(&plan.id, Some(&approval), Arc::new(|_| true))
            .is_err()
    );
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn process_cancellation_can_interrupt_a_busy_query_lane() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let plan = studio
        .prepare_execution(&session.id, "SELECT pg_sleep(10)")
        .unwrap();
    let (_, mut events) = start(&studio, &plan, None);
    while !matches!(
        events.recv().await.unwrap(),
        QueryEvent::StatementStarted { .. }
    ) {}
    tokio::time::sleep(Duration::from_millis(100)).await;
    let process = studio
        .processes(&session.id)
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.own_session)
        .unwrap();
    let cancel = studio
        .prepare_process_cancel(&session.id, &process.id, &process.identity)
        .unwrap();
    let approval = studio.authorize(&cancel.id, None, None).unwrap();
    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        finish(start(&studio, &cancel, Some(&approval)).1),
    )
    .await
    .unwrap();
    assert_eq!(status(&outcome), RunStatus::Completed);
    assert_eq!(status(&finish(events).await), RunStatus::Cancelled);
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn closed_viewers_and_full_result_budgets_do_not_misreport_committed_writes() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let name = format!("outcome_{}", uuid::Uuid::new_v4().simple());
    run(&studio, &session, &format!("CREATE TABLE public.\"{name}\" (id integer PRIMARY KEY, value integer); INSERT INTO public.\"{name}\" VALUES(1,0)")).await;
    let plan = studio
        .prepare_execution(
            &session.id,
            &format!("UPDATE public.\"{name}\" SET value=value+1 WHERE id=1 RETURNING value"),
        )
        .unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &plan.id,
            None,
            Arc::new(move |event| {
                if matches!(
                    event,
                    QueryEvent::Result { .. } | QueryEvent::Progress { .. }
                ) {
                    return false;
                }
                tx.send(event).is_ok()
            }),
        )
        .unwrap();
    assert_eq!(status(&finish(rx).await), RunStatus::Completed);
    let mut filler = Vec::new();
    while studio.usage().result_count < 100 {
        filler.push(
            studio
                .results
                .create("budget-fixture", 0, vec![], None)
                .unwrap()
                .id,
        );
    }
    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                &format!("UPDATE public.\"{name}\" SET value=value+1 WHERE id=1")
            )
            .await
        ),
        RunStatus::Completed
    );
    for id in filler {
        studio.release_result(&id);
    }
    let events = run(
        &studio,
        &session,
        &format!("SELECT value FROM public.\"{name}\" WHERE id=1"),
    )
    .await;
    assert_eq!(
        studio.results.row(&result(&events).id, 0).unwrap()[0].as_deref(),
        Some("2")
    );
    run(&studio, &session, &format!("DROP TABLE public.\"{name}\"")).await;
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires disposable live-context PostgreSQL fixtures"]
async fn live_completion_tracks_search_path_role_rollback_and_permissions() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let initial = studio.completion_catalog(&session.id).await.unwrap();
    assert!(
        initial
            .tables
            .iter()
            .any(|t| t.table.schema == "pg_catalog" && t.table.name == "pg_type"),
        "implicit system relations must participate in name resolution"
    );
    let (first, second) = tokio::join!(
        studio.completion_catalog(&session.id),
        studio.completion_catalog(&session.id)
    );
    assert_eq!(
        first.unwrap().revision,
        second.unwrap().revision,
        "unchanged concurrent readers must share a valid context revision"
    );
    assert!(initial.identity.search_path.contains(&"public".to_string()));
    let public = TableRef {
        schema: "public".into(),
        name: "orders".into(),
    };
    assert!(
        studio
            .completion_schema(&session.id, &initial.revision, &public)
            .await
            .unwrap()
            .columns
            .iter()
            .any(|c| c.name == "notes")
    );
    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "SET search_path TO completion_alt,public"
            )
            .await
        ),
        RunStatus::Completed
    );
    assert!(
        studio
            .completion_schema(&session.id, &initial.revision, &public)
            .await
            .is_err()
    );
    let changed = studio.completion_catalog(&session.id).await.unwrap();
    assert_ne!(initial.revision, changed.revision);
    assert!(
        changed
            .identity
            .search_path
            .iter()
            .position(|s| s == "completion_alt")
            .unwrap()
            < changed
                .identity
                .search_path
                .iter()
                .position(|s| s == "public")
                .unwrap()
    );
    assert_eq!(
        status(&run(&studio, &session, "BEGIN; SET LOCAL search_path TO public").await),
        RunStatus::Completed
    );
    let local = studio.completion_catalog(&session.id).await.unwrap();
    assert!(
        !local
            .identity
            .search_path
            .contains(&"completion_alt".into())
    );
    assert_eq!(
        status(&run(&studio, &session, "ROLLBACK").await),
        RunStatus::Completed
    );
    let restored = studio.completion_catalog(&session.id).await.unwrap();
    assert_eq!(restored.identity, changed.identity);
    assert_ne!(restored.revision, local.revision);
    assert_eq!(
        status(&run(&studio, &session, "SET ROLE joinery_completion_reader").await),
        RunStatus::Completed
    );
    let restricted = studio.completion_catalog(&session.id).await.unwrap();
    assert_eq!(restricted.identity.role, "joinery_completion_reader");
    assert!(
        !restricted
            .tables
            .iter()
            .any(|t| t.table.schema == "public" && t.table.name == "orders")
    );
    assert!(
        studio
            .completion_schema(&session.id, &restricted.revision, &public)
            .await
            .is_err()
    );
    let alt = TableRef {
        schema: "completion_alt".into(),
        name: "orders".into(),
    };
    assert_eq!(
        studio
            .completion_schema(&session.id, &restricted.revision, &alt)
            .await
            .unwrap()
            .columns[0]
            .name,
        "alt_note"
    );
    let column_table = TableRef {
        schema: "completion_alt".into(),
        name: "column_grants".into(),
    };
    let visible = studio
        .completion_schema(&session.id, &restricted.revision, &column_table)
        .await
        .unwrap();
    assert_eq!(
        visible
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["visible_note"]
    );
    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "SET search_path TO public,completion_alt"
            )
            .await
        ),
        RunStatus::Completed
    );
    let shadowed = studio.completion_catalog(&session.id).await.unwrap();
    assert!(
        shadowed
            .tables
            .iter()
            .any(|t| t.table.schema == "completion_alt" && t.table.name == "orders")
    );
    assert!(
        !shadowed
            .unqualified_tables
            .iter()
            .any(|t| t.name == "orders"),
        "a denied relation must still shadow an accessible later name"
    );
    assert_eq!(
        status(&run(&studio, &session, "RESET ROLE; BEGIN; SELECT 1/0").await),
        RunStatus::Failed
    );
    assert!(studio.completion_catalog(&session.id).await.is_err());
    assert_eq!(
        status(&run(&studio, &session, "ROLLBACK").await),
        RunStatus::Completed
    );
    assert!(studio.completion_catalog(&session.id).await.is_ok());
    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "CREATE TEMP TABLE orders (temp_note text)"
            )
            .await
        ),
        RunStatus::Completed
    );
    let temporary = studio.completion_catalog(&session.id).await.unwrap();
    let table = temporary
        .tables
        .iter()
        .find(|t| t.table.name == "orders" && t.table.schema.starts_with("pg_temp_"))
        .unwrap();
    assert!(temporary.identity.search_path.contains(&table.table.schema));
    let schema = studio
        .completion_schema(&session.id, &temporary.revision, &table.table)
        .await
        .unwrap();
    assert_eq!(schema.columns[0].name, "temp_note");
    assert!(!schema.editable && schema.columns.iter().all(|c| !c.editable));
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL service"]
async fn live_completion_busy_reads_do_not_block_query_cancellation() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let plan = studio
        .prepare_execution(&session.id, "SELECT pg_sleep(10)")
        .unwrap();
    let (id, mut rx) = start(&studio, &plan, None);
    while let Some(event) = rx.recv().await {
        if matches!(event, QueryEvent::StatementStarted { .. }) {
            break;
        }
    }
    let error = studio.completion_catalog(&session.id).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if studio
                .processes(&session.id)
                .await
                .unwrap()
                .iter()
                .any(|p| {
                    p.own_session
                        && p.state == "active"
                        && p.query.as_deref() == Some("SELECT pg_sleep(10)")
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("sleep query must be executing before cancellation");
    studio.cancel(&id).await.unwrap();
    let events = finish(rx).await;
    assert_eq!(status(&events), RunStatus::Cancelled);
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL service"]
async fn live_context_preserves_legacy_grid_foreign_keys() {
    let (_root, studio, session) = setup(SafeMode::ReadOnly).await;
    let table = TableRef {
        schema: "public".into(),
        name: "orders".into(),
    };
    let schema = studio.schema(&session.id, &table).await.unwrap();
    assert_eq!(schema.primary_key, vec!["id"]);
    assert!(
        schema
            .foreign_keys
            .iter()
            .any(|key| key.columns == ["customer_id"]
                && key.target.name == "customers"
                && key.target_columns == ["id"])
    );
    assert!(!schema.editable);
    let page = studio.browse(&session.id, &table, None).await.unwrap();
    assert_eq!(page.result.retained_rows, 500);
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL service"]
async fn foreign_keys_group_composite_constraints() {
    let (_root, studio, session) = setup(SafeMode::ReadOnly).await;
    let schema = studio
        .schema(
            &session.id,
            &TableRef {
                schema: "public".into(),
                name: "line_notes".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(schema.foreign_keys.len(), 2);
    let line = &schema.foreign_keys[0];
    assert_eq!(line.name, "line_notes_line_fk");
    assert_eq!(line.columns, ["note_order", "note_line"]);
    assert_eq!(line.target.name, "order_lines");
    assert_eq!(line.target_columns, ["order_id", "line_no"]);
    let order = &schema.foreign_keys[1];
    assert_eq!(order.name, "line_notes_order_fk");
    assert_eq!(order.columns, ["note_order"]);
    assert_eq!(order.target.name, "orders");
    assert_eq!(order.target_columns, ["id"]);
    assert!(
        schema
            .columns
            .iter()
            .filter(|c| c.name.starts_with("note_"))
            .all(|c| c.foreign_key)
    );
    studio.shutdown();
}

async fn browse_ids(
    studio: &Arc<Studio>,
    session: &SessionInfo,
    table: &TableRef,
    query: &BrowseQuery,
) -> (Vec<String>, usize) {
    let mut ids = Vec::new();
    let mut after = None;
    let mut pages = 0;
    loop {
        let page = studio
            .browse_with(&session.id, table, after, query)
            .await
            .unwrap();
        pages += 1;
        for i in 0..page.result.retained_rows {
            ids.push(
                studio.results.row(&page.result.id, i).unwrap()[0]
                    .clone()
                    .unwrap(),
            );
        }
        studio.release_result(&page.result.id);
        if !page.has_more {
            break;
        }
        after = page.next_key;
    }
    (ids, pages)
}

async fn reference_ids(studio: &Arc<Studio>, session: &SessionInfo, sql: &str) -> Vec<String> {
    let events = run(studio, session, sql).await;
    assert_eq!(status(&events), RunStatus::Completed);
    let retained = result(&events);
    let ids = (0..retained.retained_rows)
        .map(|i| {
            studio.results.row(&retained.id, i).unwrap()[0]
                .clone()
                .unwrap()
        })
        .collect();
    studio.release_result(&retained.id);
    ids
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL service"]
async fn browse_sorted_filtered_pages_match_reference() {
    let (_root, studio, session) = setup(SafeMode::ReadOnly).await;
    let table = TableRef {
        schema: "public".into(),
        name: "browse_sort".into(),
    };
    let sort = |column: &str, descending| SortKey {
        column: column.into(),
        descending,
    };
    let cases = [
        (
            BrowseQuery {
                sort: vec![sort("bucket", true), sort("label", false)],
                ..Default::default()
            },
            "SELECT id FROM browse_sort ORDER BY bucket DESC NULLS FIRST, label ASC NULLS LAST, id",
        ),
        (
            BrowseQuery {
                sort: vec![sort("label", true), sort("bucket", false)],
                filters: vec![ColumnFilter {
                    column: "bucket".into(),
                    op: FilterOp::NotEq,
                    value: Some("3".into()),
                }],
                ..Default::default()
            },
            "SELECT id FROM browse_sort WHERE bucket <> 3 ORDER BY label DESC NULLS FIRST, bucket ASC NULLS LAST, id",
        ),
        (
            BrowseQuery {
                filters: vec![ColumnFilter {
                    column: "label".into(),
                    op: FilterOp::Contains,
                    value: Some("_%".into()),
                }],
                ..Default::default()
            },
            "SELECT id FROM browse_sort WHERE label LIKE '%!_!%%' ESCAPE '!' ORDER BY id",
        ),
        (
            BrowseQuery {
                sort: vec![sort("bucket", false)],
                filters: vec![ColumnFilter {
                    column: "label".into(),
                    op: FilterOp::IsNull,
                    value: None,
                }],
                ..Default::default()
            },
            "SELECT id FROM browse_sort WHERE label IS NULL ORDER BY bucket ASC NULLS LAST, id",
        ),
    ];
    for (query, reference) in cases {
        let (ids, pages) = browse_ids(&studio, &session, &table, &query).await;
        let expected = reference_ids(&studio, &session, reference).await;
        assert!(!expected.is_empty(), "{reference}");
        assert_eq!(ids, expected, "{query:?}");
        if expected.len() > 500 {
            assert!(pages > 1, "{query:?} should page");
        }
    }
    let alpha = BrowseQuery {
        filters: vec![ColumnFilter {
            column: "label".into(),
            op: FilterOp::StartsWith,
            value: Some("ALP".into()),
        }],
        ..Default::default()
    };
    let (ids, _) = browse_ids(&studio, &session, &table, &alpha).await;
    assert_eq!(
        ids,
        reference_ids(
            &studio,
            &session,
            "SELECT id FROM browse_sort WHERE label = 'alpha' ORDER BY id"
        )
        .await,
        "PostgreSQL text search is case-insensitive"
    );
    let hostile = BrowseQuery {
        filters: vec![ColumnFilter {
            column: "label".into(),
            op: FilterOp::Eq,
            value: Some("'; DROP TABLE browse_sort; --".into()),
        }],
        ..Default::default()
    };
    assert!(
        browse_ids(&studio, &session, &table, &hostile)
            .await
            .0
            .is_empty()
    );
    assert_eq!(studio.usage().retained_bytes, 0);
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL service"]
async fn raw_browse_clauses_preserve_types_and_enforce_safety_gates() {
    let table = TableRef {
        schema: "public".into(),
        name: "browse_sort".into(),
    };
    let (_ro_root, ro_studio, ro_session) = setup(SafeMode::ReadOnly).await;
    // Pure Read WHERE clause pages across multiple 500-row pages and resolves table-qualified columns.
    let read_where = BrowseQuery {
        where_clause: Some("browse_sort.bucket IS NOT NULL AND id > 10".into()),
        ..Default::default()
    };
    assert!(
        ro_studio
            .prepare_browse(&ro_session.id, &table, &read_where)
            .await
            .unwrap()
            .is_none()
    );
    let (ids, pages) = browse_ids(&ro_studio, &ro_session, &table, &read_where).await;
    assert!(pages > 1);
    assert_eq!(
        ids,
        reference_ids(
            &ro_studio,
            &ro_session,
            "SELECT id FROM browse_sort WHERE bucket IS NOT NULL AND id > 10 ORDER BY id"
        )
        .await
    );

    // Custom ORDER BY sorts numeric `id` numerically (not lexicographically as text) and returns one bounded page.
    let custom_order = BrowseQuery {
        where_clause: Some("id IN (2, 10, 100)".into()),
        order_clause: Some("id DESC".into()),
        ..Default::default()
    };
    let page = ro_studio
        .browse_with(&ro_session.id, &table, None, &custom_order)
        .await
        .unwrap();
    assert!(!page.has_more);
    let got: Vec<String> = (0..page.result.retained_rows)
        .map(|i| {
            ro_studio.results.row(&page.result.id, i).unwrap()[0]
                .clone()
                .unwrap()
        })
        .collect();
    ro_studio.release_result(&page.result.id);
    assert_eq!(got, ["100", "10", "2"]);

    // Stacked statements and non-Read functions are blocked in ReadOnly Safe Mode.
    let stacked = BrowseQuery {
        where_clause: Some("id = 1; SELECT 1".into()),
        ..Default::default()
    };
    assert_eq!(
        ro_studio
            .prepare_browse(&ro_session.id, &table, &stacked)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    let unclassified = BrowseQuery {
        where_clause: Some("id = 1 AND current_setting('server_version_num') <> ''".into()),
        ..Default::default()
    };
    assert_eq!(
        ro_studio
            .prepare_browse(&ro_session.id, &table, &unclassified)
            .await
            .unwrap_err()
            .code,
        ErrorCode::PolicyBlocked
    );
    assert_eq!(
        ro_studio
            .browse_with(&ro_session.id, &table, None, &unclassified)
            .await
            .unwrap_err()
            .code,
        ErrorCode::PolicyBlocked
    );
    ro_studio.shutdown();

    // Writable profiles require one-use approval for non-Read expressions.
    let (_rw_root, rw_studio, rw_session) = setup(SafeMode::Confirm).await;
    assert_eq!(
        rw_studio
            .browse_with(&rw_session.id, &table, None, &unclassified)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ApprovalRequired
    );
    let plan = rw_studio
        .prepare_browse(&rw_session.id, &table, &unclassified)
        .await
        .unwrap()
        .expect("unclassified clause must require approval");
    assert!(plan.requires_approval);
    let token = rw_studio.authorize(&plan.id, None, None).unwrap();
    let approval = BrowseApproval {
        plan_id: plan.id.clone(),
        approval: token,
    };
    let approved_page = rw_studio
        .browse_with_approval(&rw_session.id, &table, None, &unclassified, Some(&approval))
        .await
        .unwrap();
    assert_eq!(approved_page.result.retained_rows, 1);
    assert!(!approved_page.has_more);
    rw_studio.release_result(&approved_page.result.id);
    // Reusing the consumed one-use token fails closed.
    assert_eq!(
        rw_studio
            .browse_with_approval(&rw_session.id, &table, None, &unclassified, Some(&approval))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ApprovalExpired
    );
    rw_studio.shutdown();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL service"]
async fn referencing_foreign_keys_preserve_composite_ordinals_and_filter_unreadable_tables() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    let orders = TableRef {
        schema: "public".into(),
        name: "orders".into(),
    };
    let order_lines = TableRef {
        schema: "public".into(),
        name: "order_lines".into(),
    };
    let browse_sort = TableRef {
        schema: "public".into(),
        name: "browse_sort".into(),
    };

    let inbound_orders = studio
        .referencing_foreign_keys(&session.id, &orders)
        .await
        .unwrap();
    assert_eq!(
        inbound_orders,
        vec![
            InboundForeignKey {
                name: "line_notes_order_fk".into(),
                source: TableRef {
                    schema: "public".into(),
                    name: "line_notes".into(),
                },
                columns: vec!["note_order".into()],
                target_columns: vec!["id".into()],
            },
            InboundForeignKey {
                name: "order_lines_order_id_fkey".into(),
                source: order_lines.clone(),
                columns: vec!["order_id".into()],
                target_columns: vec!["id".into()],
            },
        ]
    );

    let inbound_lines = studio
        .referencing_foreign_keys(&session.id, &order_lines)
        .await
        .unwrap();
    assert_eq!(
        inbound_lines,
        vec![InboundForeignKey {
            name: "line_notes_line_fk".into(),
            source: TableRef {
                schema: "public".into(),
                name: "line_notes".into(),
            },
            columns: vec!["note_order".into(), "note_line".into()],
            target_columns: vec!["order_id".into(), "line_no".into()],
        }]
    );

    assert!(
        studio
            .referencing_foreign_keys(&session.id, &browse_sort)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        studio
            .referencing_foreign_keys(
                &session.id,
                &TableRef {
                    schema: "public".into(),
                    name: "missing_table".into(),
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );

    // Non-superuser role without SELECT on line_notes only sees order_lines referencing orders.
    run(
        &studio,
        &session,
        "DROP ROLE IF EXISTS joinery_inbound_limited;
         CREATE ROLE joinery_inbound_limited LOGIN PASSWORD 'joinery-limited-only';
         GRANT CONNECT ON DATABASE joinery TO joinery_inbound_limited;
         GRANT USAGE ON SCHEMA public TO joinery_inbound_limited;
         GRANT SELECT ON public.orders, public.order_lines TO joinery_inbound_limited;",
    )
    .await;

    let mut limited_profile = studio.bootstrap().unwrap().profiles[0].clone();
    limited_profile.id = new_id();
    limited_profile.name = "Inbound limited".into();
    limited_profile.username = "joinery_inbound_limited".into();
    limited_profile.safe_mode = SafeMode::ReadOnly;
    let saved = studio.save_profile(limited_profile, None).unwrap();
    let limited_session = studio
        .connect(
            &saved.id,
            ConnectionSecrets {
                password: Some("joinery-limited-only".into()),
                ssh_password: None,
                key_passphrase: None,
            },
        )
        .await
        .unwrap();
    let visible = studio
        .referencing_foreign_keys(&limited_session.id, &orders)
        .await
        .unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].source, order_lines);
    studio.disconnect(&limited_session.id);

    run(
        &studio,
        &session,
        "REVOKE ALL PRIVILEGES ON public.orders, public.order_lines FROM joinery_inbound_limited;
         REVOKE USAGE ON SCHEMA public FROM joinery_inbound_limited;
         REVOKE CONNECT ON DATABASE joinery FROM joinery_inbound_limited;
         DROP ROLE joinery_inbound_limited;",
    )
    .await;
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL service"]
async fn table_changes_insert_update_delete_atomically_and_conflict_check_deletes() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "DROP TABLE IF EXISTS joinery_batch2_mutations, joinery_batch2_defaults;
                 CREATE TABLE joinery_batch2_mutations (
                   id INT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
                   strict_seq INT GENERATED ALWAYS AS IDENTITY,
                   label TEXT NOT NULL DEFAULT 'from-default',
                   amount INT NOT NULL,
                   doubled INT GENERATED ALWAYS AS (amount * 2) STORED,
                   note TEXT
                 );
                 INSERT INTO joinery_batch2_mutations (label, amount, note)
                 VALUES ('alpha', 10, 'keep'), ('beta', 20, 'delete-me');
                 CREATE TABLE joinery_batch2_defaults (
                   id SERIAL PRIMARY KEY,
                   tag TEXT NOT NULL DEFAULT 'auto'
                 );",
            )
            .await
        ),
        RunStatus::Completed
    );

    let table = TableRef {
        schema: "public".into(),
        name: "joinery_batch2_mutations".into(),
    };
    let schema = studio.schema(&session.id, &table).await.unwrap();
    let col = |name: &str| schema.columns.iter().find(|c| c.name == name).unwrap();
    assert!(!col("id").editable && col("id").insertable && col("id").has_default);
    assert!(
        !col("strict_seq").editable
            && !col("strict_seq").insertable
            && col("strict_seq").has_default
    );
    assert!(col("label").editable && col("label").insertable && col("label").has_default);
    assert!(col("amount").editable && col("amount").insertable && !col("amount").has_default);
    assert!(!col("doubled").editable && !col("doubled").insertable && col("doubled").has_default);
    assert!(col("note").editable && col("note").insertable && !col("note").has_default);

    let (_ro_root, ro_studio, ro_session) = setup(SafeMode::ReadOnly).await;
    let ro_schema = ro_studio.schema(&ro_session.id, &table).await.unwrap();
    assert!(
        !ro_schema.editable
            && ro_schema
                .columns
                .iter()
                .all(|c| !c.editable && !c.insertable)
    );
    let ro_page = ro_studio
        .browse(&ro_session.id, &table, None)
        .await
        .unwrap();
    assert_eq!(
        ro_studio
            .prepare_table_changes(
                &ro_session.id,
                TableChanges {
                    result_id: Some(ro_page.result.id.clone()),
                    updates: vec![],
                    deletes: vec![ResultDeletes {
                        result_id: ro_page.result.id.clone(),
                        rows: vec![0],
                    }],
                    inserts: vec![],
                },
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    ro_studio.shutdown();

    let page = studio.browse(&session.id, &table, None).await.unwrap();
    assert_eq!(page.result.retained_rows, 2);

    // Missing required column without default is rejected.
    assert_eq!(
        studio
            .prepare_table_changes(
                &session.id,
                TableChanges {
                    result_id: Some(page.result.id.clone()),
                    updates: vec![],
                    deletes: vec![],
                    inserts: vec![RowInsert { values: vec![] }],
                },
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    // Non-insertable GENERATED ALWAYS column is rejected.
    assert_eq!(
        studio
            .prepare_table_changes(
                &session.id,
                TableChanges {
                    result_id: Some(page.result.id.clone()),
                    updates: vec![],
                    deletes: vec![],
                    inserts: vec![RowInsert {
                        values: vec![
                            InsertValue {
                                column: "c1".into(),
                                value: Some("5".into()),
                            },
                            InsertValue {
                                column: "c3".into(),
                                value: Some("10".into()),
                            },
                        ],
                    }],
                },
            )
            .unwrap_err()
            .code,
        ErrorCode::PolicyBlocked
    );
    // Overlapping update and delete on the same row is rejected.
    assert_eq!(
        studio
            .prepare_table_changes(
                &session.id,
                TableChanges {
                    result_id: Some(page.result.id.clone()),
                    updates: vec![ResultChanges {
                        result_id: page.result.id.clone(),
                        changes: vec![CellChange {
                            row: 1,
                            column: "c3".into(),
                            value: Some("25".into()),
                        }],
                    }],
                    deletes: vec![ResultDeletes {
                        result_id: page.result.id.clone(),
                        rows: vec![1],
                    }],
                    inserts: vec![],
                },
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );

    // Mixed atomic batch: update row 0, delete row 1, insert 2 rows.
    let preview = studio
        .prepare_table_changes(
            &session.id,
            TableChanges {
                result_id: Some(page.result.id.clone()),
                updates: vec![ResultChanges {
                    result_id: page.result.id.clone(),
                    changes: vec![CellChange {
                        row: 0,
                        column: "c3".into(),
                        value: Some("15".into()),
                    }],
                }],
                deletes: vec![ResultDeletes {
                    result_id: page.result.id.clone(),
                    rows: vec![1],
                }],
                inserts: vec![
                    RowInsert {
                        values: vec![InsertValue {
                            column: "c3".into(),
                            value: Some("30".into()),
                        }],
                    },
                    RowInsert {
                        values: vec![
                            InsertValue {
                                column: "c0".into(),
                                value: Some("99".into()),
                            },
                            InsertValue {
                                column: "c2".into(),
                                value: Some("explicit".into()),
                            },
                            InsertValue {
                                column: "c3".into(),
                                value: Some("40".into()),
                            },
                            InsertValue {
                                column: "c5".into(),
                                value: Some("custom".into()),
                            },
                        ],
                    },
                ],
            },
        )
        .unwrap();
    assert_eq!(preview.row_count, 4);
    assert!(preview.sql.iter().any(|s| s.starts_with("DELETE FROM")));
    assert!(preview.sql.iter().any(|s| s.starts_with("UPDATE")));
    assert_eq!(
        preview
            .sql
            .iter()
            .filter(|s| s.starts_with("INSERT INTO"))
            .count(),
        2
    );
    let token = studio
        .authorize(
            &preview.plan.id,
            None,
            preview.plan.confirmation_text.as_deref(),
        )
        .unwrap();
    let committed = finish(start(&studio, &preview.plan, Some(&token)).1).await;
    assert_eq!(status(&committed), RunStatus::Completed);
    studio.release_result(&page.result.id);

    let after = studio.browse(&session.id, &table, None).await.unwrap();
    assert_eq!(after.result.retained_rows, 3);
    let r0 = studio.results.row(&after.result.id, 0).unwrap();
    assert_eq!(r0[0].as_deref(), Some("1"));
    assert_eq!(r0[2].as_deref(), Some("alpha"));
    assert_eq!(r0[3].as_deref(), Some("15"));
    assert_eq!(r0[4].as_deref(), Some("30"));
    let r1 = studio.results.row(&after.result.id, 1).unwrap();
    assert_eq!(r1[2].as_deref(), Some("from-default"));
    assert_eq!(r1[3].as_deref(), Some("30"));
    assert_eq!(r1[4].as_deref(), Some("60"));
    assert_eq!(r1[5], None);
    let r2 = studio.results.row(&after.result.id, 2).unwrap();
    assert_eq!(r2[0].as_deref(), Some("99"));
    assert_eq!(r2[2].as_deref(), Some("explicit"));
    assert_eq!(r2[3].as_deref(), Some("40"));
    assert_eq!(r2[4].as_deref(), Some("80"));
    assert_eq!(r2[5].as_deref(), Some("custom"));

    // Empty-table insert with 0 explicit values generates DEFAULT VALUES.
    let defaults_table = TableRef {
        schema: "public".into(),
        name: "joinery_batch2_defaults".into(),
    };
    let empty_page = studio
        .browse(&session.id, &defaults_table, None)
        .await
        .unwrap();
    assert_eq!(empty_page.result.retained_rows, 0);
    let defaults_preview = studio
        .prepare_table_changes(
            &session.id,
            TableChanges {
                result_id: Some(empty_page.result.id.clone()),
                updates: vec![],
                deletes: vec![],
                inserts: vec![RowInsert { values: vec![] }],
            },
        )
        .unwrap();
    assert!(defaults_preview.sql[0].contains("DEFAULT VALUES"));
    let defaults_token = studio
        .authorize(
            &defaults_preview.plan.id,
            None,
            defaults_preview.plan.confirmation_text.as_deref(),
        )
        .unwrap();
    assert_eq!(
        status(&finish(start(&studio, &defaults_preview.plan, Some(&defaults_token)).1).await),
        RunStatus::Completed
    );
    studio.release_result(&empty_page.result.id);
    let defaults_after = studio
        .browse(&session.id, &defaults_table, None)
        .await
        .unwrap();
    assert_eq!(defaults_after.result.retained_rows, 1);
    assert_eq!(
        studio.results.row(&defaults_after.result.id, 0).unwrap()[1].as_deref(),
        Some("auto")
    );
    studio.release_result(&defaults_after.result.id);

    // Concurrent row modification causes FOR UPDATE conflict on Delete and rolls back the whole batch.
    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "UPDATE joinery_batch2_mutations SET note = 'concurrent-touch' WHERE id = 99"
            )
            .await
        ),
        RunStatus::Completed
    );
    let stale_batch = studio
        .prepare_table_changes(
            &session.id,
            TableChanges {
                result_id: Some(after.result.id.clone()),
                updates: vec![ResultChanges {
                    result_id: after.result.id.clone(),
                    changes: vec![CellChange {
                        row: 0,
                        column: "c3".into(),
                        value: Some("999".into()),
                    }],
                }],
                deletes: vec![ResultDeletes {
                    result_id: after.result.id.clone(),
                    rows: vec![2],
                }],
                inserts: vec![RowInsert {
                    values: vec![InsertValue {
                        column: "c3".into(),
                        value: Some("50".into()),
                    }],
                }],
            },
        )
        .unwrap();
    let stale_token = studio
        .authorize(
            &stale_batch.plan.id,
            None,
            stale_batch.plan.confirmation_text.as_deref(),
        )
        .unwrap();
    let conflict_events = finish(start(&studio, &stale_batch.plan, Some(&stale_token)).1).await;
    assert!(
        conflict_events.iter().any(|e| matches!(
            e,
            QueryEvent::Error {
                error: AppError {
                    code: ErrorCode::Conflict,
                    ..
                },
                ..
            }
        )),
        "{conflict_events:?}"
    );
    studio.release_result(&after.result.id);

    let rolled_back = studio.browse(&session.id, &table, None).await.unwrap();
    assert_eq!(rolled_back.result.retained_rows, 3);
    assert_eq!(
        studio.results.row(&rolled_back.result.id, 0).unwrap()[3].as_deref(),
        Some("15")
    );
    assert_eq!(
        studio.results.row(&rolled_back.result.id, 2).unwrap()[5].as_deref(),
        Some("concurrent-touch")
    );
    studio.release_result(&rolled_back.result.id);

    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "DROP TABLE joinery_batch2_mutations, joinery_batch2_defaults"
            )
            .await
        ),
        RunStatus::Completed
    );
    studio.shutdown();
}
