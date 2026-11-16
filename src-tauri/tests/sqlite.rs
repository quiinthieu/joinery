use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

async fn open_sqlite_studio(
    root: &Path,
    db_path: &str,
    mode: SafeMode,
) -> (Arc<Studio>, ConnectionProfile, SessionInfo) {
    let studio = Studio::with_secrets(root, Arc::new(MemorySecrets::default())).unwrap();
    let profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: format!("SQLite ({db_path})"),
                group_id: "local".into(),
                engine: EngineKind::Sqlite,
                host: String::new(),
                port: 0,
                database: db_path.into(),
                username: String::new(),
                environment: Environment::Local,
                safe_mode: mode,
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
        .connect(&profile.id, ConnectionSecrets::default())
        .await
        .unwrap();
    (studio, profile, session)
}

async fn finish_events(mut rx: mpsc::UnboundedReceiver<QueryEvent>) -> Vec<QueryEvent> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            let done = matches!(ev, QueryEvent::Finished { .. });
            events.push(ev);
            if done {
                break;
            }
        }
        events
    })
    .await
    .expect("SQLite query deadline")
}

async fn exec_sql(studio: &Arc<Studio>, session_id: &str, sql: &str) -> Vec<QueryEvent> {
    let plan = studio.prepare_execution(session_id, sql).unwrap();
    let approval = if plan.requires_approval {
        Some(
            studio
                .authorize(&plan.id, None, plan.confirmation_text.as_deref())
                .unwrap(),
        )
    } else {
        None
    };
    let (tx, rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &plan.id,
            approval.as_deref(),
            Arc::new(move |e| tx.send(e).is_ok()),
        )
        .unwrap();
    finish_events(rx).await
}

#[tokio::test]
async fn sqlite_file_and_memory_end_to_end_explorer_grid_edits_transactions_search_and_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let sqlite_file = dir.path().join("workspace_test.sqlite");
    let sqlite_path_str = sqlite_file.to_str().unwrap();

    let (studio, profile, session) =
        open_sqlite_studio(dir.path(), sqlite_path_str, SafeMode::Confirm).await;
    assert_eq!(session.engine, EngineKind::Sqlite);
    assert!(session.server_version.starts_with("SQLite 3."));
    assert!(session.capabilities.table_browsing);
    assert!(session.capabilities.table_editing);
    assert!(session.capabilities.object_explorer);
    assert!(session.capabilities.schema_diagram);
    assert!(session.capabilities.schema_diff);

    // 1. Create schema (parent + child FK + index + view) and seed rows via Studio SQL execution
    for ddl in [
        "CREATE TABLE customers (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE);",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers(id), status TEXT NOT NULL, total_cents INTEGER NOT NULL);",
        "CREATE INDEX idx_orders_status ON orders (status);",
        "CREATE VIEW active_orders_v AS SELECT id, customer_id, total_cents FROM orders WHERE status = 'open';",
        "INSERT INTO customers (id, email) VALUES (1, 'ada@example.com'), (2, 'grace@example.com');",
        "INSERT INTO orders (id, customer_id, status, total_cents) VALUES (10, 1, 'open', 2500), (11, 2, 'paid', 4900);",
    ] {
        let events = exec_sql(&studio, &session.id, ddl).await;
        assert!(events.iter().any(|e| matches!(
            e,
            QueryEvent::Finished {
                status: RunStatus::Completed,
                ..
            }
        )));
    }
    // Seed a BEGIN..END trigger directly on the SQLite file (procedural batches with inner `;`
    // are intentionally gated in safety::prepare) so Object Explorer inspects real SQLite triggers.
    {
        let seed_conn = rusqlite::Connection::open(&sqlite_file).unwrap();
        seed_conn
            .execute_batch(
                "CREATE TRIGGER trg_orders_touch AFTER INSERT ON orders BEGIN SELECT 1; END;",
            )
            .unwrap();
    }

    // 2. Catalog, Object Explorer, Object Details, Schema Diagram & Schema Snapshot
    let cat = studio.catalog(&session.id).await.unwrap();
    let names: Vec<&str> = cat.iter().map(|t| t.table.name.as_str()).collect();
    assert_eq!(names, vec!["active_orders_v", "customers", "orders"]);

    let objects = studio.database_objects(&session.id).await.unwrap();
    assert!(objects.iter().any(
        |o| o.object.kind == DatabaseObjectKind::Index && o.object.name == "idx_orders_status"
    ));
    assert!(objects.iter().any(
        |o| o.object.kind == DatabaseObjectKind::Trigger && o.object.name == "trg_orders_touch"
    ));

    let order_details = studio
        .object_details(
            &session.id,
            &DatabaseObjectRef {
                schema: "main".into(),
                name: "orders".into(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        order_details.ddl,
        ObjectDdlStatus::Available { .. }
    ));
    assert!(
        order_details
            .indexes
            .iter()
            .any(|i| i.name == "idx_orders_status")
    );
    assert_eq!(order_details.triggers.len(), 1);
    assert_eq!(order_details.uses.len(), 1);
    assert_eq!(order_details.uses[0].object.name, "customers");

    let diagram = studio.schema_diagram(&session.id, "main").await.unwrap();
    assert_eq!(diagram.tables.len(), 3);
    assert_eq!(diagram.foreign_keys.len(), 1);
    assert_eq!(diagram.foreign_keys[0].source.name, "orders");
    assert_eq!(diagram.foreign_keys[0].target.name, "customers");

    let snapshot = studio.schema_snapshot(&session.id, "main").await.unwrap();
    assert_eq!(snapshot.engine, EngineKind::Sqlite);
    assert_eq!(snapshot.tables.len(), 3);

    // 3. Server-filtered & sorted table browsing + reviewed conflict-checked grid edits
    let orders_ref = TableRef {
        schema: "main".into(),
        name: "orders".into(),
    };
    let page = studio.browse(&session.id, &orders_ref, None).await.unwrap();
    assert_eq!(page.result.retained_rows, 2);

    let filtered = studio
        .browse_with(
            &session.id,
            &orders_ref,
            None,
            &BrowseQuery {
                sort: vec![SortKey {
                    column: "id".into(),
                    descending: true,
                }],
                filters: vec![ColumnFilter {
                    column: "status".into(),
                    op: FilterOp::Eq,
                    value: Some("open".into()),
                }],
                where_clause: None,
                order_clause: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(filtered.result.retained_rows, 1);

    let preview = studio
        .prepare_table_changes(
            &session.id,
            TableChanges {
                result_id: Some(page.result.id.clone()),
                updates: vec![ResultChanges {
                    result_id: page.result.id.clone(),
                    changes: vec![CellChange {
                        row: 0,
                        column: "total_cents".into(),
                        value: Some("3100".into()),
                    }],
                }],
                deletes: vec![],
                inserts: vec![RowInsert {
                    values: vec![
                        InsertValue {
                            column: "id".into(),
                            value: Some("12".into()),
                        },
                        InsertValue {
                            column: "customer_id".into(),
                            value: Some("1".into()),
                        },
                        InsertValue {
                            column: "status".into(),
                            value: Some("open".into()),
                        },
                        InsertValue {
                            column: "total_cents".into(),
                            value: Some("999".into()),
                        },
                    ],
                }],
            },
        )
        .unwrap();
    let edit_approval = studio
        .authorize(
            &preview.plan.id,
            None,
            preview.plan.confirmation_text.as_deref(),
        )
        .unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &preview.plan.id,
            Some(&edit_approval),
            Arc::new(move |e| tx.send(e).is_ok()),
        )
        .unwrap();
    let edit_events = finish_events(rx).await;
    assert!(edit_events.iter().any(|e| matches!(
        e,
        QueryEvent::Finished {
            status: RunStatus::Completed,
            ..
        }
    )));

    let after_page = studio.browse(&session.id, &orders_ref, None).await.unwrap();
    assert_eq!(after_page.result.retained_rows, 3);
    let updated_row = studio
        .browse_with(
            &session.id,
            &orders_ref,
            None,
            &BrowseQuery {
                sort: vec![],
                filters: vec![ColumnFilter {
                    column: "total_cents".into(),
                    op: FilterOp::Eq,
                    value: Some("3100".into()),
                }],
                where_clause: None,
                order_clause: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated_row.result.retained_rows, 1);

    // 4. Full-text Data Search + EXPLAIN QUERY PLAN + Manual Transaction Rollback
    let search_report = studio
        .search_database_data(DataSearchRequest {
            session_id: session.id.clone(),
            query: "grace@example".into(),
            schemas: vec!["main".into()],
            tables: vec![],
            match_mode: DataSearchMatchMode::Contains,
            case_sensitive: false,
            max_tables: Some(10),
        })
        .await
        .unwrap();
    assert_eq!(search_report.total_row_hits, 1);
    assert_eq!(search_report.matches[0].table.name, "customers");
    assert_eq!(
        search_report.matches[0].rows[0].matched_columns,
        vec!["email"]
    );

    let explain_plan = studio
        .prepare_explain(
            &session.id,
            "SELECT * FROM orders WHERE status = 'open'",
            false,
        )
        .unwrap();
    assert!(
        explain_plan.statements[0]
            .sql
            .starts_with("EXPLAIN QUERY PLAN\n")
    );

    studio
        .set_transaction_mode(&session.id, TransactionMode::Manual)
        .await
        .unwrap();
    let _ = exec_sql(&studio, &session.id, "DELETE FROM orders WHERE id = 12;").await;
    studio.rollback_transaction(&session.id).await.unwrap();
    studio.commit_transaction(&session.id).await.unwrap();
    // Switch back to auto-commit after committing clean state: rollback + commit in manual mode
    // re-opens a fresh deferred transaction, so verify row 12 survived rollback!
    let survived = studio.browse(&session.id, &orders_ref, None).await.unwrap();
    assert_eq!(survived.result.retained_rows, 3);

    // 5. Workspace SQL usages on SQLite dialect
    let usages = studio
        .find_sql_usages(SqlFindUsagesRequest {
            engine: EngineKind::Sqlite,
            connection_id: Some(profile.id.clone()),
            database: None,
            default_schema: Some("main".into()),
            schema: "main".into(),
            object_name: "orders".into(),
            column_name: Some("total_cents".into()),
            include_saved_queries: false,
            documents: vec![SqlUsageDocument {
                id: "t1".into(),
                title: "SQLite scratch".into(),
                source_kind: SqlUsageSourceKind::OpenTab,
                saved_query_id: None,
                sql: "SELECT o.total_cents FROM orders o WHERE o.status = 'open';".into(),
            }],
        })
        .unwrap();
    assert_eq!(usages.hits.len(), 1);

    studio.disconnect(&session.id);

    // 6. OS-level ReadOnly enforcement (`SQLITE_OPEN_READ_ONLY` + `PRAGMA query_only = ON`)
    let ro_dir = tempfile::tempdir().unwrap();
    let (ro_studio, _ro_prof, ro_sess) =
        open_sqlite_studio(ro_dir.path(), sqlite_path_str, SafeMode::ReadOnly).await;
    assert!(!ro_sess.capabilities.table_editing);
    let write_err = ro_studio
        .prepare_execution(&ro_sess.id, "DELETE FROM orders WHERE id = 10;")
        .unwrap_err();
    assert_eq!(write_err.code, ErrorCode::PolicyBlocked);
    let ro_browse = ro_studio
        .browse(&ro_sess.id, &orders_ref, None)
        .await
        .unwrap();
    assert_eq!(ro_browse.result.retained_rows, 3);
    ro_studio.disconnect(&ro_sess.id);
}
