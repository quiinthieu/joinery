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

async fn finish_export(mut rx: mpsc::UnboundedReceiver<ExportEvent>) -> Vec<ExportEvent> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            let done = matches!(event, ExportEvent::Finished { .. });
            events.push(event);
            if done {
                break;
            }
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ExportEvent::Finished { .. })),
            "export did not finish"
        );
        events
    })
    .await
    .expect("export deadline")
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL fixture"]
async fn streaming_table_export_preserves_exact_values_enforces_read_only_and_marks_partials() {
    use joinery_core::file_capability::SandboxFileDialog;

    let root = tempfile::tempdir().unwrap();
    let sandbox = SandboxFileDialog::new(&root.path().join("exports")).unwrap();
    let studio = Studio::with_secrets_and_dialog(
        root.path(),
        Arc::new(MemorySecrets::default()),
        sandbox.clone(),
    )
    .unwrap();
    let port: u16 = std::env::var("JOINERY_TEST_PORT")
        .unwrap_or_else(|_| "55432".into())
        .parse()
        .unwrap();
    let writable_profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "PG Export Writable".into(),
                group_id: "local".into(),
                engine: EngineKind::Postgres,
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
                store_query_text: true,
                store_drafts: false,
                snowflake: None,
            },
            None,
        )
        .unwrap();
    let readonly_profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "PG Export ReadOnly".into(),
                group_id: "local".into(),
                engine: EngineKind::Postgres,
                host: "127.0.0.1".into(),
                port,
                database: "joinery".into(),
                username: "joinery".into(),
                environment: Environment::Local,
                safe_mode: SafeMode::ReadOnly,
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
            &writable_profile.id,
            ConnectionSecrets {
                password: Some("joinery-local-only".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ro_session = studio
        .connect(
            &readonly_profile.id,
            ConnectionSecrets {
                password: Some("joinery-local-only".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "DROP TABLE IF EXISTS joinery_r2_export; \
                 CREATE TABLE joinery_r2_export ( \
                   id BIGINT PRIMARY KEY, \
                   big_val BIGINT NOT NULL, \
                   dec_val NUMERIC(38, 10) NOT NULL, \
                   note TEXT \
                 ); \
                 INSERT INTO joinery_r2_export (id, big_val, dec_val, note) \
                 SELECT g, \
                        CASE WHEN g = 1 THEN 9223372036854775807 ELSE -9223372036854775808 + g END, \
                        12345678901234567890.1234567890, \
                        CASE WHEN g % 3 = 0 THEN NULL WHEN g % 3 = 1 THEN '' ELSE 'héllo;世界' END \
                 FROM generate_series(1, 1600) AS g"
            )
            .await
        ),
        RunStatus::Completed
    );
    studio.clear_history().unwrap();
    let table = TableRef {
        schema: "public".into(),
        name: "joinery_r2_export".into(),
    };

    // 1. Multi-page (>1,500 rows) streaming export without ResultStore retention
    let grant = studio
        .pick_export_destination("pg-stream.csv", ExportFormat::Csv)
        .await
        .unwrap()
        .unwrap();
    let usage_before = studio.results.usage();
    assert_eq!(usage_before.0, 0);
    let (tx, rx) = mpsc::unbounded_channel();
    studio
        .start_export(
            ExportRequest {
                destination_token: grant.token,
                format: ExportFormat::Csv,
                source: ExportSource::Table {
                    session_id: ro_session.id.clone(),
                    table: table.clone(),
                    query: BrowseQuery::default(),
                    approval: None,
                    column_ids: vec!["c0".into(), "c1".into(), "c2".into(), "c3".into()],
                },
                include_headers: true,
                engine: Some(EngineKind::Postgres),
                target_table: Some(table.clone()),
            },
            Arc::new(move |e| tx.send(e).is_ok()),
        )
        .await
        .unwrap();
    let export_events = finish_export(rx).await;
    assert_eq!(studio.results.usage(), usage_before);
    assert!(matches!(
        export_events.last(),
        Some(ExportEvent::Finished {
            status: RunStatus::Completed,
            rows_written: 1600,
            output_file_name: Some(name),
            partial_file_name: None,
            ..
        }) if name == "pg-stream.csv"
    ));
    let csv_contents = std::fs::read_to_string(sandbox.root().join("pg-stream.csv")).unwrap();
    assert!(!sandbox.root().join("pg-stream.csv.partial").exists());
    let csv_lines: Vec<&str> = csv_contents.lines().collect();
    assert_eq!(csv_lines.len(), 1601);
    assert_eq!(csv_lines[0], "id,big_val,dec_val,note");
    assert_eq!(
        csv_lines[1],
        "1,9223372036854775807,12345678901234567890.1234567890,\"\""
    );
    assert_eq!(
        csv_lines[2],
        "2,-9223372036854775806,12345678901234567890.1234567890,héllo;世界"
    );
    assert_eq!(
        csv_lines[3],
        "3,-9223372036854775805,12345678901234567890.1234567890,"
    );

    // 2. ReadOnly blocks non-read raw clause without consuming grant or writing a file
    let ro_grant = studio
        .pick_export_destination("blocked.csv", ExportFormat::Csv)
        .await
        .unwrap()
        .unwrap();
    let blocked = studio
        .start_export(
            ExportRequest {
                destination_token: ro_grant.token.clone(),
                format: ExportFormat::Csv,
                source: ExportSource::Table {
                    session_id: ro_session.id.clone(),
                    table: table.clone(),
                    query: BrowseQuery {
                        where_clause: Some("pg_advisory_lock(1) IS NULL".into()),
                        ..Default::default()
                    },
                    approval: None,
                    column_ids: Vec::new(),
                },
                include_headers: true,
                engine: Some(EngineKind::Postgres),
                target_table: Some(table.clone()),
            },
            Arc::new(|_| true),
        )
        .await
        .unwrap_err();
    assert_eq!(blocked.code, ErrorCode::PolicyBlocked);
    assert!(!sandbox.root().join("blocked.csv").exists());
    assert!(!sandbox.root().join("blocked.csv.partial").exists());

    // 3. Cancellation leaves only marked .partial file and zero SQLite row/path persistence
    let cancel_grant = studio
        .pick_export_destination("cancelled-pg.csv", ExportFormat::Csv)
        .await
        .unwrap()
        .unwrap();
    let (cancel_tx, cancel_rx) = mpsc::unbounded_channel();
    // Returning false from event callback cancels the export mid-stream after Started/first chunk
    studio
        .start_export(
            ExportRequest {
                destination_token: cancel_grant.token,
                format: ExportFormat::Csv,
                source: ExportSource::Table {
                    session_id: ro_session.id.clone(),
                    table: table.clone(),
                    query: BrowseQuery::default(),
                    approval: None,
                    column_ids: Vec::new(),
                },
                include_headers: true,
                engine: Some(EngineKind::Postgres),
                target_table: Some(table.clone()),
            },
            Arc::new(move |e| {
                let keep_open = !matches!(e, ExportEvent::Progress { .. });
                let _ = cancel_tx.send(e);
                keep_open
            }),
        )
        .await
        .unwrap();
    let cancelled_events = finish_export(cancel_rx).await;
    assert!(matches!(
        cancelled_events.last(),
        Some(ExportEvent::Finished {
            status: RunStatus::Cancelled,
            output_file_name: None,
            partial_file_name: Some(partial),
            ..
        }) if partial == "cancelled-pg.csv.partial"
    ));
    assert!(!sandbox.root().join("cancelled-pg.csv").exists());
    let partial_text =
        std::fs::read_to_string(sandbox.root().join("cancelled-pg.csv.partial")).unwrap();
    assert!(partial_text.contains("# JOINERY EXPORT INCOMPLETE"));

    assert!(studio.history("").unwrap().is_empty());
    let audit_dump = serde_json::to_string(&studio.audit_log().unwrap()).unwrap();
    assert!(!audit_dump.contains("pg-stream.csv"));
    assert!(!audit_dump.contains("héllo;世界"));

    assert_eq!(
        status(&run(&studio, &session, "DROP TABLE joinery_r2_export").await),
        RunStatus::Completed
    );
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL fixture"]
async fn csv_import_remaps_columns_enforces_read_only_validates_upfront_and_rolls_back_failed_batches()
 {
    use joinery_core::file_capability::SandboxFileDialog;

    let root = tempfile::tempdir().unwrap();
    let sandbox = SandboxFileDialog::new(&root.path().join("imports")).unwrap();
    let studio = Studio::with_secrets_and_dialog(
        root.path(),
        Arc::new(MemorySecrets::default()),
        sandbox.clone(),
    )
    .unwrap();
    let port: u16 = std::env::var("JOINERY_TEST_PORT")
        .unwrap_or_else(|_| "55432".into())
        .parse()
        .unwrap();

    let rw_profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "PG Import Writable".into(),
                group_id: "local".into(),
                engine: EngineKind::Postgres,
                host: "127.0.0.1".into(),
                port,
                database: "joinery".into(),
                username: "joinery".into(),
                environment: Environment::Local,
                safe_mode: SafeMode::Confirm,
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
    let ro_profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "PG Import ReadOnly".into(),
                safe_mode: SafeMode::ReadOnly,
                ..rw_profile.clone()
            },
            None,
        )
        .unwrap();

    let make_secrets = || ConnectionSecrets {
        password: Some("joinery-local-only".into()),
        ..Default::default()
    };
    let session = studio
        .connect(&rw_profile.id, make_secrets())
        .await
        .unwrap();
    let ro_session = studio
        .connect(&ro_profile.id, make_secrets())
        .await
        .unwrap();

    // Setup via Silent helper profile or approve DDL on Confirm session
    let setup_plan = studio
        .prepare_execution(
            &session.id,
            "DROP TABLE IF EXISTS joinery_r2_csv_import;
             CREATE TABLE joinery_r2_csv_import (
               id BIGSERIAL PRIMARY KEY,
               code TEXT NOT NULL UNIQUE,
               big_ref BIGINT NOT NULL,
               amount NUMERIC(16,4) NOT NULL,
               note TEXT,
               tag TEXT NOT NULL DEFAULT 'default-tag',
               active BOOLEAN NOT NULL
             )",
        )
        .unwrap();
    let setup_token = studio
        .authorize(
            &setup_plan.id,
            None,
            setup_plan.confirmation_text.as_deref(),
        )
        .unwrap();
    assert_eq!(
        status(&finish(start(&studio, &setup_plan, Some(&setup_token)).1).await),
        RunStatus::Completed
    );

    let table = TableRef {
        schema: "public".into(),
        name: "joinery_r2_csv_import".into(),
    };

    // 1. Invalid CSV row is caught upfront and cannot prepare a batch
    let bad_csv = "external_code,big_ref,amount,note,active,ignored_col\nC-1,9223372036854775807,10.2500,,true,x\nC-2,not-an-int,20.5000,\"\",false,y\n";
    std::fs::write(sandbox.root().join("invalid.csv"), bad_csv.as_bytes()).unwrap();
    sandbox.queue_open(Some("invalid.csv")).unwrap();
    let bad_grant = studio.pick_import_source().await.unwrap().unwrap();

    let remapped = vec![
        CsvColumnMapping {
            csv_index: 0,
            target_column: Some("code".into()),
        },
        CsvColumnMapping {
            csv_index: 1,
            target_column: Some("big_ref".into()),
        },
        CsvColumnMapping {
            csv_index: 2,
            target_column: Some("amount".into()),
        },
        CsvColumnMapping {
            csv_index: 3,
            target_column: Some("note".into()),
        },
        CsvColumnMapping {
            csv_index: 4,
            target_column: Some("active".into()),
        },
        CsvColumnMapping {
            csv_index: 5,
            target_column: None,
        },
    ];

    // ReadOnly session blocks CSV import preview and preparation
    assert_eq!(
        studio
            .preview_csv_import(
                &ro_session.id,
                &bad_grant.token,
                &table,
                CsvImportOptions::default(),
                remapped.clone(),
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::PolicyBlocked
    );

    let bad_preview = studio
        .preview_csv_import(
            &session.id,
            &bad_grant.token,
            &table,
            CsvImportOptions::default(),
            remapped.clone(),
        )
        .await
        .unwrap();
    assert!(!bad_preview.can_prepare);
    assert_eq!(bad_preview.issues.len(), 1);
    assert_eq!(bad_preview.issues[0].line, 3);
    assert_eq!(bad_preview.issues[0].column.as_deref(), Some("big_ref"));
    assert_eq!(
        studio
            .prepare_csv_import_batch(
                &session.id,
                &bad_grant.token,
                &table,
                CsvImportOptions::default(),
                remapped.clone(),
                0,
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );

    // 2. Valid 520-row CSV (2 reviewed batches: 500 + 20) with column remapping, <default> columns, and NULL vs "" vs "\N"
    let mut valid_csv = String::from("external_code,big_ref,amount,note,active,ignored_col\n");
    for idx in 0..520 {
        let note_cell = match idx % 3 {
            0 => "",        // unquoted empty -> SQL NULL
            1 => "\"\"",    // quoted empty -> ""
            _ => "\"\\N\"", // quoted \N -> literal "\N"
        };
        let big = if idx == 0 {
            9_223_372_036_854_775_807i64
        } else {
            idx as i64
        };
        valid_csv.push_str(&format!(
            "CODE-{idx:04},{big},12345.6789,{note_cell},{},skip-{idx}\n",
            idx % 2 == 0
        ));
    }
    std::fs::write(sandbox.root().join("valid-520.csv"), valid_csv.as_bytes()).unwrap();
    sandbox.queue_open(Some("valid-520.csv")).unwrap();
    let valid_grant = studio.pick_import_source().await.unwrap().unwrap();

    let preview = studio
        .preview_csv_import(
            &session.id,
            &valid_grant.token,
            &table,
            CsvImportOptions::default(),
            remapped.clone(),
        )
        .await
        .unwrap();
    assert!(preview.can_prepare, "issues: {:?}", preview.issues);
    assert_eq!(preview.total_rows, 520);
    assert_eq!(preview.batch_count, 2);
    assert_eq!(preview.unmapped_defaults, vec!["id", "tag"]);

    for (batch_idx, expected_rows) in [(0usize, 500usize), (1usize, 20usize)] {
        let batch_preview = studio
            .prepare_csv_import_batch(
                &session.id,
                &valid_grant.token,
                &table,
                CsvImportOptions::default(),
                remapped.clone(),
                batch_idx,
            )
            .await
            .unwrap();
        assert_eq!(batch_preview.row_count, expected_rows);
        assert!(batch_preview.plan.requires_approval);
        let token = studio
            .authorize(
                &batch_preview.plan.id,
                None,
                batch_preview.plan.confirmation_text.as_deref(),
            )
            .unwrap();
        assert_eq!(
            status(&finish(start(&studio, &batch_preview.plan, Some(&token)).1).await),
            RunStatus::Completed
        );
    }

    // 3. Mid-batch unique constraint conflict rolls back the entire batch atomically
    let conflict_csv = "external_code,big_ref,amount,note,active,ignored_col\nNEW-BEFORE-CONFLICT,42,1.0000,should-rollback,true,a\nCODE-0005,43,2.0000,duplicate-code,false,b\n";
    std::fs::write(sandbox.root().join("conflict.csv"), conflict_csv.as_bytes()).unwrap();
    sandbox.queue_open(Some("conflict.csv")).unwrap();
    let conflict_grant = studio.pick_import_source().await.unwrap().unwrap();
    let conflict_batch = studio
        .prepare_csv_import_batch(
            &session.id,
            &conflict_grant.token,
            &table,
            CsvImportOptions::default(),
            remapped,
            0,
        )
        .await
        .unwrap();
    let conflict_token = studio
        .authorize(
            &conflict_batch.plan.id,
            None,
            conflict_batch.plan.confirmation_text.as_deref(),
        )
        .unwrap();
    let conflict_events =
        finish(start(&studio, &conflict_batch.plan, Some(&conflict_token)).1).await;
    assert_eq!(status(&conflict_events), RunStatus::Failed);

    // Verify exact row count = 520 (NEW-BEFORE-CONFLICT rolled back!) and exact values
    let verify_plan = studio
        .prepare_execution(
            &session.id,
            "SELECT count(*)::text,
                    (SELECT big_ref::text FROM joinery_r2_csv_import WHERE code = 'CODE-0000'),
                    (SELECT note FROM joinery_r2_csv_import WHERE code = 'CODE-0000'),
                    (SELECT note FROM joinery_r2_csv_import WHERE code = 'CODE-0001'),
                    (SELECT note FROM joinery_r2_csv_import WHERE code = 'CODE-0002'),
                    (SELECT tag FROM joinery_r2_csv_import WHERE code = 'CODE-0000')
             FROM joinery_r2_csv_import",
        )
        .unwrap();
    let verify_events = finish(start(&studio, &verify_plan, None).1).await;
    let verify_result_id = result(&verify_events).id;
    let row = studio.results.row(&verify_result_id, 0).unwrap();
    assert_eq!(row[0].as_deref(), Some("520"));
    assert_eq!(row[1].as_deref(), Some("9223372036854775807"));
    assert_eq!(row[2], None);
    assert_eq!(row[3].as_deref(), Some(""));
    assert_eq!(row[4].as_deref(), Some("\\N"));
    assert_eq!(row[5].as_deref(), Some("default-tag"));
    studio.release_result(&verify_result_id);

    // Zero CSV paths or imported values in SQLite audit_log
    let audit_dump = serde_json::to_string(&studio.audit_log().unwrap()).unwrap();
    assert!(!audit_dump.contains("valid-520.csv"));
    assert!(!audit_dump.contains("CODE-0000"));

    let drop_plan = studio
        .prepare_execution(&session.id, "DROP TABLE joinery_r2_csv_import")
        .unwrap();
    let drop_token = studio
        .authorize(&drop_plan.id, None, drop_plan.confirmation_text.as_deref())
        .unwrap();
    assert_eq!(
        status(&finish(start(&studio, &drop_plan, Some(&drop_token)).1).await),
        RunStatus::Completed
    );
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL fixture"]
async fn data_comparison_diffs_multi_page_live_tables_snapshots_anomalies_and_raw_clause_approvals()
{
    let (_root, studio, session) = setup(SafeMode::Confirm).await;

    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "DROP TABLE IF EXISTS joinery_r2_cmp_left, joinery_r2_cmp_right;
                 CREATE TABLE joinery_r2_cmp_left (
                   tenant_id INT NOT NULL,
                   item_no BIGINT NOT NULL,
                   big_val BIGINT NOT NULL,
                   amount NUMERIC(18,4) NOT NULL,
                   note TEXT,
                   PRIMARY KEY (tenant_id, item_no)
                 );
                 CREATE TABLE joinery_r2_cmp_right (
                   tenant_id INT NOT NULL,
                   item_no BIGINT NOT NULL,
                   big_val BIGINT NOT NULL,
                   amount NUMERIC(18,4) NOT NULL,
                   note TEXT,
                   PRIMARY KEY (tenant_id, item_no)
                 );
                 INSERT INTO joinery_r2_cmp_left
                   SELECT 1, i, 9223372036854775807, 123456.7890, 'row-' || i
                   FROM generate_series(1, 610) i;
                 INSERT INTO joinery_r2_cmp_right
                   SELECT 1, i, 9223372036854775807, 123456.7890, 'row-' || i
                   FROM generate_series(1, 610) i;
                 -- Modify (1, 10): NULL vs empty string + decimal diff
                 UPDATE joinery_r2_cmp_left SET note = NULL, amount = 50.2500 WHERE item_no = 10;
                 UPDATE joinery_r2_cmp_right SET note = '', amount = 50.5000 WHERE item_no = 10;
                 -- Only in left: (1, 901); Only in right: (1, 902)
                 INSERT INTO joinery_r2_cmp_left VALUES (1, 901, 42, 1.0000, 'private-cmp-left');
                 INSERT INTO joinery_r2_cmp_right VALUES (1, 902, 42, 2.0000, 'private-cmp-right');",
            )
            .await
        ),
        RunStatus::Completed
    );

    let left_table = TableRef {
        schema: "public".into(),
        name: "joinery_r2_cmp_left".into(),
    };
    let right_table = TableRef {
        schema: "public".into(),
        name: "joinery_r2_cmp_right".into(),
    };

    // 1. Multi-page (> 500 rows) LiveTable vs LiveTable without retaining in ResultStore
    let usage_before = studio.usage();
    assert_eq!(usage_before.retained_bytes, 0);
    let live_report = studio
        .compare_data(CompareRequest {
            left: CompareSide::LiveTable {
                session_id: session.id.clone(),
                table: left_table.clone(),
                query: BrowseQuery::default(),
                approval: None,
                label: Some("PG Left".into()),
            },
            right: CompareSide::LiveTable {
                session_id: session.id.clone(),
                table: right_table.clone(),
                query: BrowseQuery::default(),
                approval: None,
                label: Some("PG Right".into()),
            },
            key_columns: Vec::new(),
            ignore_column_case: false,
        })
        .await
        .unwrap();

    assert_eq!(live_report.key_columns, vec!["tenant_id", "item_no"]);
    assert_eq!(live_report.identical_count, 609);
    assert_eq!(live_report.different_count, 1);
    assert_eq!(live_report.only_left_count, 1);
    assert_eq!(live_report.only_right_count, 1);
    assert_eq!(studio.usage().retained_bytes, 0);
    assert_eq!(studio.usage().result_count, usage_before.result_count);

    let modified = live_report
        .diffs
        .iter()
        .find(|d| d.kind == RowDiffKind::Different)
        .unwrap();
    assert_eq!(modified.key_values, vec!["1", "10"]);
    assert!(modified.cells.contains(&CellDiff {
        column: "note".into(),
        left: None,
        right: Some("".into()),
        type_mismatch: false,
    }));

    // 2. Unapproved non-allowlisted SQL function in WHERE under Confirm Safe Mode requires BrowseApproval
    let gated_query = BrowseQuery {
        sort: Vec::new(),
        filters: Vec::new(),
        where_clause: Some("item_no <= 20 AND pg_backend_pid() > 0".into()),
        order_clause: None,
    };
    assert_eq!(
        studio
            .compare_data(CompareRequest {
                left: CompareSide::LiveTable {
                    session_id: session.id.clone(),
                    table: left_table.clone(),
                    query: gated_query.clone(),
                    approval: None,
                    label: None,
                },
                right: CompareSide::LiveTable {
                    session_id: session.id.clone(),
                    table: right_table.clone(),
                    query: BrowseQuery {
                        where_clause: Some("item_no <= 20".into()),
                        ..Default::default()
                    },
                    approval: None,
                    label: None,
                },
                key_columns: Vec::new(),
                ignore_column_case: false,
            })
            .await
            .unwrap_err()
            .code,
        ErrorCode::ApprovalRequired
    );

    let browse_plan = studio
        .prepare_browse(&session.id, &left_table, &gated_query)
        .await
        .unwrap()
        .expect("subquery WHERE must require approval");
    let browse_token = studio.authorize(&browse_plan.id, None, None).unwrap();
    let approved_report = studio
        .compare_data(CompareRequest {
            left: CompareSide::LiveTable {
                session_id: session.id.clone(),
                table: left_table.clone(),
                query: gated_query,
                approval: Some(BrowseApproval {
                    plan_id: browse_plan.id,
                    approval: browse_token,
                }),
                label: None,
            },
            right: CompareSide::LiveTable {
                session_id: session.id.clone(),
                table: right_table.clone(),
                query: BrowseQuery {
                    where_clause: Some("item_no <= 20".into()),
                    ..Default::default()
                },
                approval: None,
                label: None,
            },
            key_columns: Vec::new(),
            ignore_column_case: false,
        })
        .await
        .unwrap();
    assert_eq!(approved_report.identical_count, 19);
    assert_eq!(approved_report.different_count, 1);

    // 3. Snapshot with duplicate keys, NULL keys, and uppercase column names vs filtered LiveTable
    let snap_events = run(
        &studio,
        &session,
        "SELECT 1 AS \"TENANT_ID\", 1::bigint AS \"ITEM_NO\", 9223372036854775807::bigint AS \"BIG_VAL\", 123456.7890::numeric(18,4) AS \"AMOUNT\", 'row-1'::text AS \"NOTE\"
         UNION ALL
         SELECT 1, 2, 1, 1.0000, 'dup-1'
         UNION ALL
         SELECT 1, 2, 1, 1.0000, 'dup-2'
         UNION ALL
         SELECT 1, NULL, 1, 1.0000, 'null-key'",
    )
    .await;
    assert_eq!(status(&snap_events), RunStatus::Completed);
    let snap_res = result(&snap_events);

    let snap_report = studio
        .compare_data(CompareRequest {
            left: CompareSide::Snapshot {
                result_ids: vec![snap_res.id.clone()],
                label: Some("Query Snapshot".into()),
                truncated_hint: true,
            },
            right: CompareSide::LiveTable {
                session_id: session.id.clone(),
                table: right_table.clone(),
                query: BrowseQuery {
                    where_clause: Some("item_no IN (1, 2)".into()),
                    ..Default::default()
                },
                approval: None,
                label: Some("Filtered Right".into()),
            },
            key_columns: Vec::new(),
            ignore_column_case: true,
        })
        .await
        .unwrap();

    assert!(snap_report.left_truncated);
    assert_eq!(snap_report.identical_count, 1);
    assert_eq!(snap_report.only_right_count, 0);
    assert_eq!(snap_report.left_anomalies.null_key_count, 1);
    assert_eq!(snap_report.left_anomalies.duplicate_key_rows, 2);
    assert_eq!(snap_report.right_anomalies.excluded_counterpart_rows, 1);
    studio.release_result(&snap_res.id);

    // Zero private cell text in audit log
    let audit_dump = serde_json::to_string(&studio.audit_log().unwrap()).unwrap();
    assert!(!audit_dump.contains("private-cmp-left"));
    assert!(!audit_dump.contains("private-cmp-right"));

    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "DROP TABLE joinery_r2_cmp_left, joinery_r2_cmp_right",
            )
            .await
        ),
        RunStatus::Completed
    );
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn object_explorer_discovers_overloads_ddl_dependencies_and_permission_limits() {
    let (_root, studio, session) = setup(SafeMode::Silent).await;
    assert!(session.capabilities.object_explorer);

    let cleanup =
        "DROP VIEW IF EXISTS public.r3_summary_v, public.r3_cycle_a, public.r3_cycle_b CASCADE;
                   DROP TABLE IF EXISTS public.\"r3_Order \"\"Items\"\"\" CASCADE;
                   DROP FUNCTION IF EXISTS public.r3_calc(integer) CASCADE;
                   DROP FUNCTION IF EXISTS public.r3_calc(numeric, integer) CASCADE;
                   DROP FUNCTION IF EXISTS public.r3_touch_fn() CASCADE;
                   DROP PROCEDURE IF EXISTS public.r3_noop_proc(text) CASCADE;
                   DROP SEQUENCE IF EXISTS public.r3_ticket_seq CASCADE;";
    assert_eq!(
        status(&run(&studio, &session, cleanup).await),
        RunStatus::Completed
    );

    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                "CREATE SEQUENCE public.r3_ticket_seq AS bigint INCREMENT BY 5 MINVALUE 10 MAXVALUE 99999 START WITH 10 CACHE 1 CYCLE;
                 SELECT nextval('public.r3_ticket_seq');
                 CREATE FUNCTION public.r3_calc(a integer) RETURNS integer LANGUAGE sql IMMUTABLE AS 'SELECT a * 2';
                 CREATE FUNCTION public.r3_calc(a numeric, b integer) RETURNS numeric LANGUAGE sql STABLE SECURITY DEFINER AS 'SELECT round(a, b)';
                 CREATE PROCEDURE public.r3_noop_proc(IN label text) LANGUAGE sql AS 'SELECT 1';
                 CREATE FUNCTION public.r3_touch_fn() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END';
                 CREATE TABLE public.\"r3_Order \"\"Items\"\"\" (
                   id bigint PRIMARY KEY DEFAULT nextval('public.r3_ticket_seq'),
                   order_id bigint NOT NULL REFERENCES public.orders(id),
                   sku text NOT NULL UNIQUE,
                   qty integer NOT NULL CHECK (qty > 0)
                 );
                 CREATE INDEX r3_idx_partial_expr ON public.\"r3_Order \"\"Items\"\"\" (lower(sku)) WHERE qty > 1;
                 CREATE TRIGGER r3_items_audit_trg BEFORE INSERT OR UPDATE ON public.\"r3_Order \"\"Items\"\"\" FOR EACH ROW EXECUTE FUNCTION public.r3_touch_fn();
                 CREATE VIEW public.r3_summary_v AS SELECT id, public.r3_calc(qty) AS doubled FROM public.\"r3_Order \"\"Items\"\"\";
                 CREATE VIEW public.r3_cycle_a AS SELECT 1::integer AS x;
                 CREATE VIEW public.r3_cycle_b AS SELECT x FROM public.r3_cycle_a;
                 CREATE OR REPLACE VIEW public.r3_cycle_a AS SELECT x FROM public.r3_cycle_b WHERE false;",
            )
            .await
        ),
        RunStatus::Completed
    );

    let objects = studio.database_objects(&session.id).await.unwrap();
    let overloads: Vec<_> = objects
        .iter()
        .filter(|o| o.object.schema == "public" && o.object.name == "r3_calc")
        .collect();
    assert_eq!(overloads.len(), 2);
    assert!(
        overloads
            .iter()
            .any(|o| o.object.identity.as_deref() == Some("integer"))
    );
    assert!(
        overloads
            .iter()
            .any(|o| o.object.identity.as_deref() == Some("numeric, integer"))
    );
    assert!(objects.iter().any(|o| {
        o.object.name == "r3_noop_proc" && o.object.kind == DatabaseObjectKind::Procedure
    }));
    assert!(objects.iter().any(|o| {
        o.object.name == "r3_ticket_seq" && o.object.kind == DatabaseObjectKind::Sequence
    }));
    assert!(objects.iter().any(|o| {
        o.object.name == "r3_items_audit_trg"
            && o.object.kind == DatabaseObjectKind::Trigger
            && o.owner_table.as_deref() == Some("r3_Order \"Items\"")
    }));
    assert!(objects.iter().any(|o| {
        o.object.name == "r3_idx_partial_expr" && o.object.kind == DatabaseObjectKind::Index
    }));

    // Table details: quoted DDL, constraints, partial/expression index, trigger, and dependencies
    let table_details = studio
        .object_details(
            &session.id,
            &DatabaseObjectRef {
                schema: "public".into(),
                name: "r3_Order \"Items\"".into(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
        )
        .await
        .unwrap();
    assert!(
        table_details
            .constraints
            .iter()
            .any(|c| c.kind == ConstraintKind::Check && c.definition.contains("qty > 0"))
    );
    assert!(table_details.constraints.iter().any(|c| {
        c.kind == ConstraintKind::ForeignKey
            && c.target
                .as_ref()
                .is_some_and(|t| t.schema == "public" && t.name == "orders")
    }));
    let partial_idx = table_details
        .indexes
        .iter()
        .find(|i| i.name == "r3_idx_partial_expr")
        .expect("partial expression index present");
    assert_eq!(partial_idx.expressions, vec!["lower(sku)"]);
    assert_eq!(partial_idx.predicate.as_deref(), Some("(qty > 1)"));
    let trig = table_details
        .triggers
        .iter()
        .find(|t| t.name == "r3_items_audit_trg")
        .expect("trigger present");
    assert_eq!(trig.timing, "BEFORE");
    assert_eq!(trig.events, vec!["INSERT", "UPDATE"]);
    assert_eq!(trig.orientation, "ROW");
    assert!(trig.enabled);
    match &table_details.ddl {
        ObjectDdlStatus::Available { ddl, truncated } => {
            assert!(!truncated);
            assert!(ddl.contains("CREATE TABLE \"public\".\"r3_Order \"\"Items\"\"\""));
            assert!(ddl.contains("r3_idx_partial_expr"));
        }
        other => panic!("expected available table DDL, got {other:?}"),
    }
    assert!(
        table_details
            .uses
            .iter()
            .any(|e| e.object.name == "orders" && e.object.kind == DatabaseObjectKind::Table)
    );
    assert!(
        table_details
            .uses
            .iter()
            .any(|e| e.object.name == "r3_ticket_seq"
                && e.object.kind == DatabaseObjectKind::Sequence)
    );
    assert!(
        table_details
            .used_by
            .iter()
            .any(|e| e.object.name == "r3_summary_v" && e.object.kind == DatabaseObjectKind::View)
    );

    // Overloaded function inspection
    let fn_int = studio
        .object_details(
            &session.id,
            &DatabaseObjectRef {
                schema: "public".into(),
                name: "r3_calc".into(),
                kind: DatabaseObjectKind::Function,
                identity: Some("integer".into()),
            },
        )
        .await
        .unwrap();
    let r_int = fn_int.routine.as_ref().unwrap();
    assert_eq!(r_int.identity_arguments, "integer");
    assert_eq!(r_int.volatility.as_deref(), Some("IMMUTABLE"));
    assert!(!r_int.security_definer);
    assert!(
        fn_int
            .used_by
            .iter()
            .any(|e| e.object.name == "r3_summary_v" && e.object.kind == DatabaseObjectKind::View)
    );

    let fn_num = studio
        .object_details(
            &session.id,
            &DatabaseObjectRef {
                schema: "public".into(),
                name: "r3_calc".into(),
                kind: DatabaseObjectKind::Function,
                identity: Some("numeric, integer".into()),
            },
        )
        .await
        .unwrap();
    let r_num = fn_num.routine.as_ref().unwrap();
    assert_eq!(r_num.identity_arguments, "numeric, integer");
    assert_eq!(r_num.volatility.as_deref(), Some("STABLE"));
    assert!(r_num.security_definer);

    // Cyclic views terminate cleanly without self-edges or duplicates
    let cycle_a = studio
        .object_details(
            &session.id,
            &DatabaseObjectRef {
                schema: "public".into(),
                name: "r3_cycle_a".into(),
                kind: DatabaseObjectKind::View,
                identity: None,
            },
        )
        .await
        .unwrap();
    assert!(cycle_a.uses.iter().all(|e| e.object.name != "r3_cycle_a"));
    assert!(cycle_a.uses.iter().any(|e| e.object.name == "r3_cycle_b"));
    assert!(
        cycle_a
            .used_by
            .iter()
            .any(|e| e.object.name == "r3_cycle_b")
    );

    // Sequence inspection by owner vs restricted role without USAGE/SELECT on sequence
    let seq_owner = studio
        .object_details(
            &session.id,
            &DatabaseObjectRef {
                schema: "public".into(),
                name: "r3_ticket_seq".into(),
                kind: DatabaseObjectKind::Sequence,
                identity: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        seq_owner.sequence.as_ref().unwrap().last_value.as_deref(),
        Some("10")
    );

    run(
        &studio,
        &session,
        "DROP ROLE IF EXISTS joinery_r3_limited;
         CREATE ROLE joinery_r3_limited LOGIN PASSWORD 'joinery-r3-limited-only';
         GRANT CONNECT ON DATABASE joinery TO joinery_r3_limited;
         GRANT USAGE ON SCHEMA public TO joinery_r3_limited;",
    )
    .await;

    let mut limited_profile = studio.bootstrap().unwrap().profiles[0].clone();
    limited_profile.id = new_id();
    limited_profile.name = "R3 limited".into();
    limited_profile.username = "joinery_r3_limited".into();
    limited_profile.safe_mode = SafeMode::ReadOnly;
    let saved = studio.save_profile(limited_profile, None).unwrap();
    let limited_session = studio
        .connect(
            &saved.id,
            ConnectionSecrets {
                password: Some("joinery-r3-limited-only".into()),
                ssh_password: None,
                key_passphrase: None,
            },
        )
        .await
        .unwrap();
    let seq_limited = studio
        .object_details(
            &limited_session.id,
            &DatabaseObjectRef {
                schema: "public".into(),
                name: "r3_ticket_seq".into(),
                kind: DatabaseObjectKind::Sequence,
                identity: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(seq_limited.sequence.as_ref().unwrap().last_value, None);
    assert!(matches!(seq_limited.ddl, ObjectDdlStatus::Available { .. }));
    studio.disconnect(&limited_session.id);

    assert_eq!(
        status(
            &run(
                &studio,
                &session,
                &format!(
                    "{cleanup}
                     REVOKE USAGE ON SCHEMA public FROM joinery_r3_limited;
                     REVOKE CONNECT ON DATABASE joinery FROM joinery_r3_limited;
                     DROP ROLE joinery_r3_limited;"
                ),
            )
            .await
        ),
        RunStatus::Completed
    );
    studio.shutdown();
}
