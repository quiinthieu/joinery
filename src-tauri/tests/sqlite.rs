use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

async fn open_sqlite_studio(
    root: &Path,
    db_path: &str,
    mode: SafeMode,
) -> (Arc<Studio>, ConnectionProfile, SessionInfo) {
    let app_data_root = root.join("joinery-app-data");
    let studio = Studio::with_secrets(&app_data_root, Arc::new(MemorySecrets::default())).unwrap();
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
    let (exp_tx, exp_rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &explain_plan.id,
            None,
            Arc::new(move |e| exp_tx.send(e).is_ok()),
        )
        .unwrap();
    let exp_events = finish_events(exp_rx).await;
    let exp_result = exp_events
        .iter()
        .find_map(|e| match e {
            QueryEvent::Result { result } => Some(result.clone()),
            _ => None,
        })
        .expect("expected EXPLAIN QUERY PLAN result");
    assert_eq!(exp_result.columns.len(), 1);
    assert_eq!(exp_result.columns[0].name, "QUERY PLAN");
    let exp_cell = studio
        .results
        .row(&exp_result.id, 0)
        .unwrap()
        .remove(0)
        .unwrap();
    assert!(exp_cell.contains("\"sqliteQueryPlan\""));
    assert!(exp_cell.contains("idx_orders_status"));
    studio.release_result(&exp_result.id);

    // Lazy manual transaction mode: stays Idle until first statement, blocks conflicting grid
    // commits while uncommitted, rolls back cleanly, and switches back to AutoCommit!
    let manual_state = studio
        .set_transaction_mode(&session.id, TransactionMode::Manual)
        .await
        .unwrap();
    assert_eq!(manual_state.transaction_state, TransactionState::Idle);
    let tx_events = exec_sql(&studio, &session.id, "DELETE FROM orders WHERE id = 12;").await;
    assert!(tx_events.iter().any(|e| matches!(
        e,
        QueryEvent::SessionState { state }
            if state.transaction_state == TransactionState::InTransaction
    )));
    // Attempting to switch to AutoCommit while uncommitted is rejected with Conflict
    let mode_conflict = studio
        .set_transaction_mode(&session.id, TransactionMode::AutoCommit)
        .await
        .unwrap_err();
    assert_eq!(mode_conflict.code, ErrorCode::Conflict);

    // Attempting to apply a grid edit while an editor transaction is uncommitted is rejected rather than merged
    let pre_tx_page = after_page.clone();
    let grid_while_open = studio
        .prepare_table_changes(
            &session.id,
            TableChanges {
                result_id: Some(pre_tx_page.result.id.clone()),
                updates: vec![ResultChanges {
                    result_id: pre_tx_page.result.id.clone(),
                    changes: vec![CellChange {
                        row: 0,
                        column: "status".into(),
                        value: Some("closed".into()),
                    }],
                }],
                deletes: vec![],
                inserts: vec![],
            },
        )
        .unwrap();
    let grid_open_approval = studio
        .authorize(
            &grid_while_open.plan.id,
            None,
            grid_while_open.plan.confirmation_text.as_deref(),
        )
        .unwrap();
    let (gt_tx, gt_rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &grid_while_open.plan.id,
            Some(&grid_open_approval),
            Arc::new(move |e| gt_tx.send(e).is_ok()),
        )
        .unwrap();
    let grid_open_events = finish_events(gt_rx).await;
    assert!(grid_open_events.iter().any(|e| matches!(
        e,
        QueryEvent::Error { error, .. } if error.code == ErrorCode::Conflict
    )));
    assert!(grid_open_events.iter().any(|e| matches!(
        e,
        QueryEvent::Finished {
            status: RunStatus::Failed,
            ..
        }
    )));

    let rolled_back_state = studio.rollback_transaction(&session.id).await.unwrap();
    assert_eq!(rolled_back_state.transaction_state, TransactionState::Idle);
    studio
        .set_transaction_mode(&session.id, TransactionMode::AutoCommit)
        .await
        .unwrap();
    let survived = studio.browse(&session.id, &orders_ref, None).await.unwrap();
    assert_eq!(survived.result.retained_rows, 3);

    // Whole-row conflict check across non-PK + non-UTF8 BLOB columns, WITHOUT ROWID composite-PK paging,
    // and case-sensitive Contains/StartsWith search past lowercase decoys!
    {
        let ext = rusqlite::Connection::open(&sqlite_file).unwrap();
        ext.execute_batch(
            "CREATE TABLE kv_without_rowid (
               tenant TEXT NOT NULL,
               seq INTEGER NOT NULL,
               token TEXT NOT NULL,
               payload BLOB,
               PRIMARY KEY (tenant, seq)
             ) WITHOUT ROWID;",
        )
        .unwrap();
        let mut ins = ext
            .prepare("INSERT INTO kv_without_rowid (tenant, seq, token, payload) VALUES (?1, ?2, ?3, ?4)")
            .unwrap();
        for i in 1..=205 {
            let tok = if i <= 20 {
                format!("ada_lower_{i}")
            } else if i == 21 {
                "Ada_ExactCase_21".to_owned()
            } else {
                format!("tok_{i}")
            };
            let blob: Option<Vec<u8>> = if i == 1 {
                Some(vec![0xff, 0x00, 0xfe, 0x42])
            } else {
                None
            };
            ins.execute(rusqlite::params!["org", i, tok, blob]).unwrap();
        }
    }
    let kv_ref = TableRef {
        schema: "main".into(),
        name: "kv_without_rowid".into(),
    };
    let kv_p1 = studio.browse(&session.id, &kv_ref, None).await.unwrap();
    assert_eq!(kv_p1.result.retained_rows, 200);
    assert!(kv_p1.has_more);
    assert_eq!(
        studio.results.row(&kv_p1.result.id, 0).unwrap()[3].as_deref(),
        Some("0xff00fe42")
    );
    let kv_p2 = studio
        .browse(&session.id, &kv_ref, kv_p1.next_key.clone())
        .await
        .unwrap();
    assert_eq!(kv_p2.result.retained_rows, 5);
    assert!(!kv_p2.has_more);

    // Case-sensitive Contains search skips the 20 `ada_lower_*` rows and finds `Ada_ExactCase_21`!
    let cs_search = studio
        .search_database_data(DataSearchRequest {
            session_id: session.id.clone(),
            query: "Ada_".into(),
            schemas: vec!["main".into()],
            tables: vec![kv_ref.clone()],
            match_mode: DataSearchMatchMode::Contains,
            case_sensitive: true,
            max_tables: Some(10),
        })
        .await
        .unwrap();
    assert_eq!(cs_search.total_row_hits, 1);
    assert_eq!(cs_search.matches[0].rows[0].matched_columns, vec!["token"]);
    assert_eq!(cs_search.matches[0].rows[0].row[1].as_deref(), Some("21"));

    // Update row 0 of kv_without_rowid (verifying non-UTF8 BLOB `0xff00fe42` matches cleanly in conflict check!)
    let kv_ok_edit = studio
        .prepare_table_changes(
            &session.id,
            TableChanges {
                result_id: Some(kv_p1.result.id.clone()),
                updates: vec![ResultChanges {
                    result_id: kv_p1.result.id.clone(),
                    changes: vec![CellChange {
                        row: 0,
                        column: "token".into(),
                        value: Some("updated_with_blob_guard".into()),
                    }],
                }],
                deletes: vec![],
                inserts: vec![],
            },
        )
        .unwrap();
    let kv_ok_app = studio
        .authorize(
            &kv_ok_edit.plan.id,
            None,
            kv_ok_edit.plan.confirmation_text.as_deref(),
        )
        .unwrap();
    let (kv_tx, kv_rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &kv_ok_edit.plan.id,
            Some(&kv_ok_app),
            Arc::new(move |e| kv_tx.send(e).is_ok()),
        )
        .unwrap();
    let kv_ok_events = finish_events(kv_rx).await;
    assert!(kv_ok_events.iter().any(|e| matches!(
        e,
        QueryEvent::Finished {
            status: RunStatus::Completed,
            ..
        }
    )));

    // External modification of non-PK `payload` BLOB on row 1 causes stale conflict rejection!
    {
        let ext = rusqlite::Connection::open(&sqlite_file).unwrap();
        ext.execute(
            "UPDATE kv_without_rowid SET payload = X'CAFE' WHERE tenant = 'org' AND seq = 2",
            [],
        )
        .unwrap();
    }
    let kv_stale_edit = studio
        .prepare_table_changes(
            &session.id,
            TableChanges {
                result_id: Some(kv_p1.result.id.clone()),
                updates: vec![ResultChanges {
                    result_id: kv_p1.result.id.clone(),
                    changes: vec![CellChange {
                        row: 1,
                        column: "token".into(),
                        value: Some("should_conflict".into()),
                    }],
                }],
                deletes: vec![],
                inserts: vec![],
            },
        )
        .unwrap();
    let kv_stale_app = studio
        .authorize(
            &kv_stale_edit.plan.id,
            None,
            kv_stale_edit.plan.confirmation_text.as_deref(),
        )
        .unwrap();
    let (st_tx, st_rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &kv_stale_edit.plan.id,
            Some(&kv_stale_app),
            Arc::new(move |e| st_tx.send(e).is_ok()),
        )
        .unwrap();
    let kv_stale_events = finish_events(st_rx).await;
    assert!(kv_stale_events.iter().any(|e| matches!(
        e,
        QueryEvent::Error { error, .. } if error.code == ErrorCode::Conflict
    )));
    assert!(kv_stale_events.iter().any(|e| matches!(
        e,
        QueryEvent::Finished {
            status: RunStatus::Failed,
            ..
        }
    )));

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

#[tokio::test]
async fn sqlite_security_blocks_internal_metadata_symlinks_attach_and_readonly_limits() {
    let dir = tempfile::tempdir().unwrap();
    let app_data_root = dir.path().join("app-metadata");
    let user_dir = dir.path().join("user-workspace");
    std::fs::create_dir_all(&user_dir).unwrap();

    let studio = Studio::with_secrets(&app_data_root, Arc::new(MemorySecrets::default())).unwrap();

    let make_profile = |db: &str, mode: SafeMode| ConnectionProfile {
        id: new_id(),
        name: "SQLite Sec Check".into(),
        group_id: "local".into(),
        engine: EngineKind::Sqlite,
        host: String::new(),
        port: 0,
        database: db.into(),
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
    };

    // 1. Direct internal metadata files, backup subpaths, non-existent files inside Persistence::root,
    //    files named `joinery.sqlite*`, and Snowflake state directories must all be blocked!
    let symlink_to_app_db = user_dir.join("sneaky_link.db");
    let symlink_to_app_dir = user_dir.join("sneaky_dir");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(app_data_root.join("joinery.sqlite"), &symlink_to_app_db)
            .unwrap();
        std::os::unix::fs::symlink(&app_data_root, &symlink_to_app_dir).unwrap();
    }

    let blocked_paths = vec![
        app_data_root.join("joinery.sqlite"),
        app_data_root.join("backups").join("snapshot.sqlite"),
        app_data_root.join("nonexistent_new.sqlite"),
        user_dir.join("joinery.sqlite"),
        user_dir.join("joinery.sqlite-wal"),
        dir.path().join("snowflake-state").join("helper.sqlite"),
        #[cfg(unix)]
        symlink_to_app_db.clone(),
        #[cfg(unix)]
        symlink_to_app_dir.join("brand_new.sqlite"),
    ];

    for forbidden in &blocked_paths {
        let p_str = forbidden.to_str().unwrap();
        let save_err = studio
            .save_profile(make_profile(p_str, SafeMode::Confirm), None)
            .unwrap_err();
        assert_eq!(
            save_err.code,
            ErrorCode::PolicyBlocked,
            "Expected save_profile PolicyBlocked for {p_str}, got {save_err:?}"
        );
        let driver_err = joinery_core::drivers::sqlite::Sqlite::open(
            &make_profile(p_str, SafeMode::ReadOnly),
            "sec-test-sid",
            vec![app_data_root.clone()],
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("Expected Sqlite::open to reject {p_str}"));
        assert_eq!(driver_err.code, ErrorCode::PolicyBlocked);
    }

    // 2. Writable user SQLite session: benign ATTACH works, but ATTACH of joinery.sqlite / symlink
    //    and PRAGMA writable_schema = ON are blocked at the SQLite authorizer level!
    let user_db = user_dir.join("primary.sqlite");
    let aux_db = user_dir.join("secondary.sqlite");
    {
        let c = rusqlite::Connection::open(&aux_db).unwrap();
        c.execute_batch("CREATE TABLE aux_items (id INTEGER PRIMARY KEY, val TEXT); INSERT INTO aux_items VALUES (1, 'ok');")
            .unwrap();
    }
    let rw_prof = studio
        .save_profile(
            make_profile(user_db.to_str().unwrap(), SafeMode::Confirm),
            None,
        )
        .unwrap();
    let rw_sess = studio
        .connect(&rw_prof.id, ConnectionSecrets::default())
        .await
        .unwrap();
    assert!(rw_sess.capabilities.ddl_editing);

    // Benign user database attach succeeds
    let attach_ok = exec_sql(
        &studio,
        &rw_sess.id,
        &format!(
            "ATTACH DATABASE '{}' AS aux;",
            aux_db.to_str().unwrap().replace('\'', "''")
        ),
    )
    .await;
    assert!(attach_ok.iter().any(|e| matches!(
        e,
        QueryEvent::Finished {
            status: RunStatus::Completed,
            ..
        }
    )));

    // Attempting to ATTACH internal joinery.sqlite on a writable session is denied!
    let attach_internal = exec_sql(
        &studio,
        &rw_sess.id,
        &format!(
            "ATTACH DATABASE '{}' AS stolen;",
            app_data_root
                .join("joinery.sqlite")
                .to_str()
                .unwrap()
                .replace('\'', "''")
        ),
    )
    .await;
    assert!(attach_internal.iter().any(|e| matches!(
        e,
        QueryEvent::Error { error, .. } if error.code == ErrorCode::PolicyBlocked
    )));

    #[cfg(unix)]
    {
        let attach_symlink = exec_sql(
            &studio,
            &rw_sess.id,
            &format!(
                "ATTACH DATABASE '{}' AS stolen_link;",
                symlink_to_app_db.to_str().unwrap().replace('\'', "''")
            ),
        )
        .await;
        assert!(attach_symlink.iter().any(|e| matches!(
            e,
            QueryEvent::Error { error, .. } if error.code == ErrorCode::PolicyBlocked
        )));
    }

    let writable_schema_attempt =
        exec_sql(&studio, &rw_sess.id, "PRAGMA writable_schema = ON;").await;
    assert!(writable_schema_attempt.iter().any(|e| matches!(
        e,
        QueryEvent::Error { error, .. } if error.code == ErrorCode::PolicyBlocked
    )));
    studio.disconnect(&rw_sess.id);

    // 3. ReadOnly SQLite session: SQLITE_LIMIT_ATTACHED == 0 + authorizer blocks ALL ATTACH
    //    (even :memory: and benign files) and blocks PRAGMA query_only = OFF!
    let (ro_driver, ro_info) = joinery_core::drivers::sqlite::Sqlite::open(
        &make_profile(user_db.to_str().unwrap(), SafeMode::ReadOnly),
        "ro-limit-sid",
        vec![app_data_root.clone()],
    )
    .await
    .unwrap();
    assert!(!ro_info.capabilities.ddl_editing);
    assert!(!ro_info.capabilities.table_editing);

    struct NoopSink;
    impl joinery_core::drivers::QuerySink for NoopSink {
        fn limit(&mut self, _: &str) {}
        fn describe(&mut self, _: Vec<ColumnInfo>) -> AppResult<()> {
            Ok(())
        }
        fn rows(&mut self, _: Vec<TextRow>) -> AppResult<bool> {
            Ok(true)
        }
        fn complete(&mut self, _: u64) -> AppResult<()> {
            Ok(())
        }
    }

    let attach_mem_err = ro_driver
        .execute(
            "ATTACH DATABASE ':memory:' AS mem_aux",
            StatementKind::Unknown,
            &mut NoopSink,
        )
        .await
        .unwrap_err();
    assert_eq!(attach_mem_err.code, ErrorCode::PolicyBlocked);

    let pragma_escape_err = ro_driver
        .execute(
            "PRAGMA query_only = OFF",
            StatementKind::Unknown,
            &mut NoopSink,
        )
        .await
        .unwrap_err();
    assert_eq!(pragma_escape_err.code, ErrorCode::PolicyBlocked);
    ro_driver.close();
}

struct FixedCsvDialog(std::path::PathBuf);

#[async_trait::async_trait]
impl joinery_core::file_capability::FileDialogProvider for FixedCsvDialog {
    async fn pick_save_file(
        &self,
        _: &str,
        _: ExportFormat,
    ) -> AppResult<Option<std::path::PathBuf>> {
        Ok(None)
    }
    async fn pick_open_csv(&self) -> AppResult<Option<std::path::PathBuf>> {
        Ok(Some(self.0.clone()))
    }
    async fn pick_open_sql(&self) -> AppResult<Option<std::path::PathBuf>> {
        Ok(None)
    }
    async fn pick_save_sql(&self, _: &str) -> AppResult<Option<std::path::PathBuf>> {
        Ok(None)
    }
}

#[tokio::test]
async fn sqlite_ddl_editing_schema_migration_query_result_editing_csv_import_and_live_compare() {
    let dir = tempfile::tempdir().unwrap();
    let app_data_root = dir.path().join("app-metadata");
    let db_left = dir.path().join("left_dev.sqlite");
    let db_right = dir.path().join("right_target.sqlite");
    let csv_file = dir.path().join("import_products.csv");
    std::fs::write(
        &csv_file,
        "id,sku,price_cents\n101,SKU-ALPHA,1250\n102,SKU-BETA,3400\n",
    )
    .unwrap();

    let studio = Studio::with_secrets_and_dialog(
        &app_data_root,
        Arc::new(MemorySecrets::default()),
        Arc::new(FixedCsvDialog(csv_file.clone())),
    )
    .unwrap();

    let mk_profile = |name: &str, path: &Path| ConnectionProfile {
        id: new_id(),
        name: name.into(),
        group_id: "local".into(),
        engine: EngineKind::Sqlite,
        host: String::new(),
        port: 0,
        database: path.to_str().unwrap().into(),
        username: String::new(),
        environment: Environment::Local,
        safe_mode: SafeMode::Confirm,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: true,
        store_drafts: false,
        snowflake: None,
    };

    let p_left = studio
        .save_profile(mk_profile("SQLite Left", &db_left), None)
        .unwrap();
    let p_right = studio
        .save_profile(mk_profile("SQLite Right", &db_right), None)
        .unwrap();
    let s_left = studio
        .connect(&p_left.id, ConnectionSecrets::default())
        .await
        .unwrap();
    let s_right = studio
        .connect(&p_right.id, ConnectionSecrets::default())
        .await
        .unwrap();
    assert!(s_left.capabilities.ddl_editing);
    assert!(s_right.capabilities.ddl_editing);

    let run_plan = |plan: PreparedPlan| {
        let studio = studio.clone();
        async move {
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
            let evs = finish_events(rx).await;
            assert!(
                evs.iter().any(|e| matches!(
                    e,
                    QueryEvent::Finished {
                        status: RunStatus::Completed,
                        ..
                    }
                )),
                "Expected plan execution to complete: {evs:?}"
            );
            evs
        }
    };

    // 1. Structured TableObjectAction + IndexConstraintAction + TableMaintenanceAction on real SQLite!
    let products_ref = TableRef {
        schema: "main".into(),
        name: "products".into(),
    };
    let create_plan = studio
        .prepare_table_object_ddl(
            &s_left.id,
            joinery_core::ddl::TableObjectAction::CreateTable {
                table: products_ref.clone(),
                columns: vec![
                    joinery_core::ddl::TableColumnDraft {
                        name: "id".into(),
                        data_type: "INTEGER".into(),
                        nullable: false,
                        default_expression: None,
                        auto_increment: true,
                        primary_key: true,
                    },
                    joinery_core::ddl::TableColumnDraft {
                        name: "code".into(),
                        data_type: "TEXT".into(),
                        nullable: false,
                        default_expression: None,
                        auto_increment: false,
                        primary_key: false,
                    },
                ],
                comment: Some("Ignored with warning on SQLite".into()),
            },
        )
        .unwrap();
    run_plan(create_plan).await;

    // Add column `price_cents`, rename `code` -> `sku`, add partial index, run ANALYZE + REINDEX + VACUUM
    let add_col_plan = studio
        .prepare_table_object_ddl(
            &s_left.id,
            joinery_core::ddl::TableObjectAction::AddColumn {
                table: products_ref.clone(),
                column: joinery_core::ddl::TableColumnDraft {
                    name: "price_cents".into(),
                    data_type: "INTEGER".into(),
                    nullable: false,
                    default_expression: Some("0".into()),
                    auto_increment: false,
                    primary_key: false,
                },
            },
        )
        .unwrap();
    run_plan(add_col_plan).await;

    let rename_col_plan = studio
        .prepare_table_object_ddl(
            &s_left.id,
            joinery_core::ddl::TableObjectAction::AlterColumn {
                table: products_ref.clone(),
                column_name: "code".into(),
                new_name: Some("sku".into()),
                data_type: "TEXT".into(),
                nullable: false,
                default_expression: None,
                using_expression: None,
            },
        )
        .unwrap();
    run_plan(rename_col_plan).await;

    let create_idx_plan = studio
        .prepare_index_constraint_ddl(
            &s_left.id,
            joinery_core::ddl::IndexConstraintAction::CreateIndex {
                table: products_ref.clone(),
                name: "idx_products_expensive".into(),
                unique: false,
                method: "btree".into(),
                concurrently: false,
                expressions: vec!["sku".into()],
                raw_expressions: false,
                predicate: Some("price_cents > 1000".into()),
            },
        )
        .unwrap();
    assert!(
        create_idx_plan.statements[0]
            .sql
            .contains("CREATE INDEX \"main\".\"idx_products_expensive\" ON \"products\" (\"sku\") WHERE price_cents > 1000;")
    );
    run_plan(create_idx_plan).await;

    for maint in [
        joinery_core::ddl::TableMaintenanceAction::Analyze {
            table: products_ref.clone(),
            verbose: false,
            columns: vec![],
        },
        joinery_core::ddl::TableMaintenanceAction::ReindexTable {
            table: products_ref.clone(),
        },
        joinery_core::ddl::TableMaintenanceAction::Vacuum {
            table: products_ref.clone(),
            analyze: false,
            verbose: false,
            full: false,
        },
    ] {
        let m_plan = studio.prepare_table_maintenance(&s_left.id, maint).unwrap();
        run_plan(m_plan).await;
    }

    // 2. Reviewed CSV Import into `products` on SQLite!
    let picked = studio.pick_import_source().await.unwrap().unwrap();
    let csv_opts = CsvImportOptions::default();
    let csv_preview = studio
        .preview_csv_import(
            &s_left.id,
            &picked.token,
            &products_ref,
            csv_opts.clone(),
            vec![],
        )
        .await
        .unwrap();
    assert!(csv_preview.can_prepare);
    assert_eq!(csv_preview.total_rows, 2);
    let preview_token = csv_preview
        .preview_token
        .clone()
        .expect("SQLite CSV import must issue previewToken");
    let batch_preview = studio
        .prepare_csv_import_batch(
            &s_left.id,
            &picked.token,
            &products_ref,
            csv_opts,
            csv_preview.mappings.clone(),
            0,
            &preview_token,
        )
        .await
        .unwrap();
    assert_eq!(batch_preview.row_count, 2);
    run_plan(batch_preview.plan).await;

    // 3. Single-table SELECT query result editing + TEMP table shadow read-only safety guard!
    let sel_events = exec_sql(
        &studio,
        &s_left.id,
        "SELECT id, sku, price_cents FROM products ORDER BY id;",
    )
    .await;
    let sel_res = sel_events
        .iter()
        .find_map(|e| match e {
            QueryEvent::Result { result } => Some(result.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        matches!(
            sel_res.editability,
            Some(ResultEditability::Editable { .. })
        ),
        "Expected SQLite single-table SELECT to be editable, got {:?}",
        sel_res.editability
    );
    let q_edit = studio
        .prepare_table_changes(
            &s_left.id,
            TableChanges {
                result_id: Some(sel_res.id.clone()),
                updates: vec![ResultChanges {
                    result_id: sel_res.id.clone(),
                    changes: vec![CellChange {
                        row: 0,
                        column: sel_res.columns[2].id.clone(),
                        value: Some("1500".into()),
                    }],
                }],
                deletes: vec![],
                inserts: vec![],
            },
        )
        .unwrap();
    run_plan(q_edit.plan).await;

    // Shadow `products` with a TEMP table: unqualified SELECT * FROM products MUST become read-only!
    exec_sql(
        &studio,
        &s_left.id,
        "CREATE TEMP TABLE products (id INTEGER PRIMARY KEY, sku TEXT, price_cents INTEGER);",
    )
    .await;
    let shadow_events = exec_sql(&studio, &s_left.id, "SELECT * FROM products;").await;
    let shadow_res = shadow_events
        .iter()
        .find_map(|e| match e {
            QueryEvent::Result { result } => Some(result.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        matches!(
            shadow_res.editability,
            Some(ResultEditability::ReadOnly { .. })
        ),
        "Shadowed temp table SELECT must be ReadOnly, got {:?}",
        shadow_res.editability
    );
    exec_sql(&studio, &s_left.id, "DROP TABLE temp.products;").await;

    // 4. Schema Diff + Migration DDL execution converging `s_right` to match `s_left` (including FK, CHECK, Trigger, Generated Column, and View)!
    let create_reviews_evs = exec_sql(
        &studio,
        &s_left.id,
        "CREATE TABLE product_reviews (id INTEGER PRIMARY KEY, product_id INTEGER NOT NULL REFERENCES products(id), rating INTEGER NOT NULL DEFAULT 5 CONSTRAINT chk_rating CHECK (rating BETWEEN 1 AND 5), note TEXT CHECK (note != 'AS (fake)') GENERATED ALWAYS AS ('R:' || rating) VIRTUAL);",
    )
    .await;
    assert!(
        create_reviews_evs.iter().any(|e| matches!(
            e,
            QueryEvent::Finished {
                status: RunStatus::Completed,
                ..
            }
        )),
        "Expected CREATE TABLE product_reviews to succeed, got: {create_reviews_evs:?}"
    );
    {
        let seed_conn = rusqlite::Connection::open(&db_left).unwrap();
        seed_conn
            .execute_batch(
                "CREATE TRIGGER main.\"on\" AFTER INSERT ON product_reviews BEGIN UPDATE products SET price_cents = price_cents WHERE id = NEW.product_id; END;",
            )
            .unwrap();
    }
    exec_sql(
        &studio,
        &s_left.id,
        "CREATE VIEW expensive_products_v AS SELECT id, sku, price_cents FROM products WHERE price_cents >= 1500;",
    )
    .await;

    let reviews_details = studio
        .object_details(
            &s_left.id,
            &DatabaseObjectRef {
                schema: "main".into(),
                name: "product_reviews".into(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
        )
        .await
        .unwrap();
    assert!(
        reviews_details
            .constraints
            .iter()
            .any(|c| c.kind == joinery_core::domain::ConstraintKind::Check
                && c.name == "chk_rating"
                && c.definition == "CHECK (rating BETWEEN 1 AND 5)"),
        "Expected chk_rating CHECK constraint in SQLite object_details, got: {:?}",
        reviews_details.constraints
    );
    assert_eq!(reviews_details.triggers.len(), 1);
    assert_eq!(reviews_details.triggers[0].timing, "AFTER");
    assert_eq!(
        reviews_details.triggers[0].events,
        vec!["INSERT".to_string()],
        "Trigger header parser must not confuse trigger name or body UPDATE with trigger event"
    );

    let snap_left = studio.schema_snapshot(&s_left.id, "main").await.unwrap();
    let snap_right_before = studio.schema_snapshot(&s_right.id, "main").await.unwrap();
    let mig_plan =
        joinery_core::ddl::generate_migration_ddl(&snap_right_before, &snap_left, Some("main"))
            .unwrap();
    assert!(!mig_plan.statements.is_empty());

    let prepared_mig = studio
        .prepare_migration_ddl(&s_right.id, snap_right_before, mig_plan.statements)
        .await
        .unwrap();
    run_plan(prepared_mig).await;

    let snap_right_after = studio.schema_snapshot(&s_right.id, "main").await.unwrap();
    let post_diff =
        joinery_core::schema_diff::compare_schema_snapshots(snap_right_after, snap_left);
    assert!(
        post_diff.entries.is_empty(),
        "Expected 0 schema differences after applying SQLite migration DDL, got: {:?}",
        post_diff.entries
    );

    // 5. Seed one row in `s_right.products` and run LiveTable vs LiveTable `compare_data`!
    exec_sql(
        &studio,
        &s_right.id,
        "INSERT INTO products (id, sku, price_cents) VALUES (101, 'SKU-ALPHA', 1250);",
    )
    .await;
    let cmp_report = studio
        .compare_data(CompareRequest {
            left: CompareSide::LiveTable {
                session_id: s_left.id.clone(),
                table: products_ref.clone(),
                query: BrowseQuery::default(),
                approval: None,
                label: Some("Left products".into()),
            },
            right: CompareSide::LiveTable {
                session_id: s_right.id.clone(),
                table: products_ref.clone(),
                query: BrowseQuery::default(),
                approval: None,
                label: Some("Right products".into()),
            },
            key_columns: vec!["id".into()],
            ignore_column_case: false,
        })
        .await
        .unwrap();
    assert_eq!(cmp_report.different_count, 1); // id=101: 1500 vs 1250
    assert_eq!(cmp_report.only_left_count, 1); // id=102 only on left

    // Drop index via structured IndexConstraintAction::DropIndex
    let drop_idx_plan = studio
        .prepare_index_constraint_ddl(
            &s_left.id,
            joinery_core::ddl::IndexConstraintAction::DropIndex {
                table: products_ref.clone(),
                name: "idx_products_expensive".into(),
                concurrently: false,
            },
        )
        .unwrap();
    run_plan(drop_idx_plan).await;

    studio.disconnect(&s_left.id);
    studio.disconnect(&s_right.id);
}
