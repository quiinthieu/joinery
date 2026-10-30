use joinery_core::{domain::*, execution::Studio, safety, secrets::MemorySecrets};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
mod common;

fn profile(port: u16, mode: SafeMode) -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Disposable MySQL".into(),
        group_id: "local".into(),
        engine: EngineKind::Mysql,
        host: "127.0.0.1".into(),
        port,
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
    }
}

#[test]
fn mysql_is_available_in_the_connection_manager() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    assert!(
        studio
            .bootstrap()
            .unwrap()
            .supported_engines
            .contains(&EngineKind::Mysql)
    );
}

#[test]
fn executable_mysql_comments_and_session_parser_changes_are_blocked() {
    let p = profile(53306, SafeMode::ReadOnly);
    assert!(safety::prepare("s", &p, "SELECT '/*! harmless text */' AS comment_text").is_ok());
    for sql in [
        "SELECT 1; /*!50000 DELETE FROM sample */",
        "SELECT 1; /*M! DELETE FROM sample */",
    ] {
        assert!(
            safety::prepare("s", &p, sql).is_err(),
            "accepted executable comment"
        );
    }
    let p = profile(53306, SafeMode::Silent);
    for sql in [
        "SET sql_mode='ANSI_QUOTES'",
        "SET @@session.sql_mode='NO_BACKSLASH_ESCAPES'",
        "SET NAMES latin1",
    ] {
        assert!(
            safety::prepare("s", &p, sql).is_err(),
            "accepted parser-affecting session change"
        );
    }
}

async fn setup(port: u16, mode: SafeMode) -> (tempfile::TempDir, Arc<Studio>, SessionInfo) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let p = studio.save_profile(profile(port, mode), None).unwrap();
    let session = studio
        .connect(
            &p.id,
            ConnectionSecrets {
                password: Some("joinery-local-only".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    (root, studio, session)
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB live-context fixtures"]
async fn mysql_live_completion_tracks_database_roles_and_column_grants() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        let initial = studio.completion_catalog(&session.id).await.unwrap();
        assert_eq!(initial.identity.database, "joinery");
        assert_eq!(initial.identity.case_sensitive_table_names, Some(true));
        assert!(
            !initial
                .tables
                .iter()
                .any(|t| t.table.schema == "completion_alt")
        );
        completed(
            &run(
                &studio,
                &session,
                "SET ROLE completion_reader; USE completion_alt",
            )
            .await,
        );
        let observed = result(&run(&studio, &session, "SELECT DATABASE()").await);
        assert_eq!(
            studio.results.row(&observed.id, 0).unwrap()[0].as_deref(),
            Some("completion_alt")
        );
        let active = studio.completion_catalog(&session.id).await.unwrap();
        assert_eq!(active.identity.database, "completion_alt");
        assert!(active.identity.role.contains("completion_reader"));
        assert_ne!(active.revision, initial.revision);
        let table = TableRef {
            schema: "completion_alt".into(),
            name: "sample".into(),
        };
        assert!(
            studio
                .completion_schema(&session.id, &initial.revision, &table)
                .await
                .is_err()
        );
        let columns = studio
            .completion_schema(&session.id, &active.revision, &table)
            .await
            .unwrap();
        assert_eq!(columns.columns[0].name, "alt_note");
        assert!(!columns.editable && columns.columns.iter().all(|c| !c.editable));
        let grants = TableRef {
            schema: "completion_alt".into(),
            name: "column_grants".into(),
        };
        let visible = studio
            .completion_schema(&session.id, &active.revision, &grants)
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
        completed(&run(&studio, &session, "BEGIN; USE joinery; ROLLBACK").await);
        // USE is not transaction-local in either engine.
        assert_eq!(
            studio
                .completion_catalog(&session.id)
                .await
                .unwrap()
                .identity
                .database,
            "joinery"
        );
        completed(
            &run(
                &studio,
                &session,
                "CREATE TEMPORARY TABLE joinery.sample (temp_note TEXT)",
            )
            .await,
        );
        let temp = studio.completion_catalog(&session.id).await.unwrap();
        let shadow = studio
            .completion_schema(
                &session.id,
                &temp.revision,
                &TableRef {
                    schema: "joinery".into(),
                    name: "sample".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            shadow
                .columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["temp_note"]
        );
        completed(&run(&studio, &session, "USE completion_alt; SET ROLE NONE").await);
        let revoked = studio.completion_catalog(&session.id).await.unwrap();
        assert!(
            !revoked
                .tables
                .iter()
                .any(|t| t.table.schema == "completion_alt")
        );
        assert!(
            studio
                .completion_schema(&session.id, &revoked.revision, &table)
                .await
                .is_err()
        );
        studio.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires disposable case-folded MySQL and MariaDB fixtures"]
async fn mysql_live_completion_uses_server_case_policy() {
    for port in [53406, 53407] {
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        let initial = studio.completion_catalog(&session.id).await.unwrap();
        assert_eq!(initial.identity.case_sensitive_table_names, Some(false));
        assert_eq!(initial.identity.database, "joinery");

        completed(
            &run(
                &studio,
                &session,
                "SET ROLE completion_reader; USE COMPLETION_ALT",
            )
            .await,
        );
        let active = studio.completion_catalog(&session.id).await.unwrap();
        assert_eq!(active.identity.database, "completion_alt");
        assert_ne!(active.revision, initial.revision);
        let case_table = active
            .tables
            .iter()
            .find(|t| {
                t.table.schema.eq_ignore_ascii_case("completion_alt")
                    && t.table.name.eq_ignore_ascii_case("CaseOnly")
            })
            .expect("role-visible case-folded table");
        assert!(active.unqualified_tables.contains(&case_table.table));
        let schema = studio
            .completion_schema(&session.id, &active.revision, &case_table.table)
            .await
            .unwrap();
        assert_eq!(schema.columns[0].name, "case_note");
        assert!(
            studio
                .completion_schema(&session.id, &initial.revision, &case_table.table)
                .await
                .is_err()
        );
        completed(
            &run(
                &studio,
                &session,
                "SELECT CASE_NOTE FROM COMPLETION_ALT.CASEONLY LIMIT 1",
            )
            .await,
        );
        studio.shutdown();
    }
}

async fn run(studio: &Arc<Studio>, session: &SessionInfo, sql: &str) -> Vec<QueryEvent> {
    let plan = studio.prepare_execution(&session.id, sql).unwrap();
    execute(studio, &plan).await
}
async fn execute(studio: &Arc<Studio>, plan: &PreparedPlan) -> Vec<QueryEvent> {
    let approval = plan.requires_approval.then(|| {
        studio
            .authorize(&plan.id, None, plan.confirmation_text.as_deref())
            .unwrap()
    });
    let (tx, mut rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &plan.id,
            approval.as_deref(),
            Arc::new(move |e| tx.send(e).is_ok()),
        )
        .unwrap();
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
                .any(|e| matches!(e, QueryEvent::Finished { .. }))
        );
        events
    })
    .await
    .unwrap()
}

fn completed(events: &[QueryEvent]) {
    assert!(
        events.iter().any(|e| matches!(
            e,
            QueryEvent::Finished {
                status: RunStatus::Completed,
                ..
            }
        )),
        "{events:?}"
    );
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
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn both_engines_stream_exact_values_and_individual_results() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::ReadOnly).await;
        let events=run(&studio,&session,"SELECT CAST(18446744073709551615 AS UNSIGNED) AS exact_integer, CAST(12345678901234567890.123456789 AS DECIMAL(40,9)) AS exact_decimal, NULL AS nil, '' AS empty_value, 'héllo;世界' AS unicode; SELECT 2 AS second").await;
        completed(&events);
        let results = events
            .iter()
            .filter_map(|e| {
                if let QueryEvent::Result { result } = e
                    && result.status == RetentionStatus::Complete
                {
                    Some(result)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 2);
        let values = studio.results.row(&results[0].id, 0).unwrap();
        assert_eq!(values[0].as_deref(), Some("18446744073709551615"));
        assert_eq!(values[1].as_deref(), Some("12345678901234567890.123456789"));
        assert_eq!(values[2], None);
        assert_eq!(values[3].as_deref(), Some(""));
        assert_eq!(values[4].as_deref(), Some("héllo;世界"));
        assert!(!studio.range(&results[0].id, 0, 1).unwrap().is_empty());
        assert!(studio.history("").unwrap().iter().all(|h| h.sql.is_none()));
        assert!(
            studio
                .prepare_execution(&session.id, "UPDATE sample SET note='blocked' WHERE id=1")
                .is_err()
        );
        studio.disconnect(&session.id);
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn catalog_keys_paging_and_read_only_tables_work_on_both_engines() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        let table = TableRef {
            schema: "joinery".into(),
            name: "sample".into(),
        };
        assert!(
            studio
                .catalog(&session.id)
                .await
                .unwrap()
                .iter()
                .any(|t| t.table == table)
        );
        let schema = studio.schema(&session.id, &table).await.unwrap();
        assert!(schema.editable);
        assert_eq!(schema.primary_key, ["id"]);
        assert!(
            !schema
                .columns
                .iter()
                .find(|c| c.name == "generated_value")
                .unwrap()
                .editable
        );
        assert!(
            !schema
                .columns
                .iter()
                .find(|c| c.name == "raw_value")
                .unwrap()
                .editable
        );
        let first = studio.browse(&session.id, &table, None).await.unwrap();
        assert_eq!(first.result.retained_rows, 500);
        assert!(first.has_more);
        assert_eq!(
            studio.results.row(&first.result.id, 0).unwrap()[0].as_deref(),
            Some("9007199254740992")
        );
        let second = studio
            .browse(&session.id, &table, first.next_key)
            .await
            .unwrap();
        assert_eq!(second.result.retained_rows, 500);
        assert!(!second.has_more);
        assert_eq!(
            studio.results.row(&second.result.id, 0).unwrap()[0].as_deref(),
            Some("9007199254741492")
        );
        assert_eq!(
            studio.results.row(&first.result.id, 0).unwrap()[4].as_deref(),
            Some("0x00ff")
        );
        assert_eq!(
            studio.results.row(&first.result.id, 0).unwrap()[5].as_deref(),
            Some("2026-01-02 03:04:05.123456")
        );
        for name in ["nontransactional", "sample_view"] {
            let readonly = TableRef {
                schema: "joinery".into(),
                name: name.into(),
            };
            let schema = studio.schema(&session.id, &readonly).await.unwrap();
            assert!(!schema.editable);
            assert!(schema.columns.iter().all(|c| !c.editable));
        }
        let composite = studio
            .schema(
                &session.id,
                &TableRef {
                    schema: "joinery".into(),
                    name: "composite_keys".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(composite.primary_key, ["name", "part"]);
        let page = studio
            .browse(&session.id, &composite.table, None)
            .await
            .unwrap();
        assert_eq!(
            studio.results.row(&page.result.id, 1).unwrap()[0].as_deref(),
            Some("10")
        );
        let child = studio
            .schema(
                &session.id,
                &TableRef {
                    schema: "joinery".into(),
                    name: "child_rows".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(child.foreign_keys[0].target, table);
        studio.disconnect(&session.id);
        assert_eq!(studio.usage().retained_bytes, 0);
        assert_eq!(studio.usage().session_count, 0);
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn read_only_profiles_can_browse_editable_tables() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::ReadOnly).await;
        let table = TableRef {
            schema: "joinery".into(),
            name: "sample".into(),
        };
        let schema = studio.schema(&session.id, &table).await.unwrap();
        assert!(!schema.editable && schema.columns.iter().all(|c| !c.editable));
        let first = studio.browse(&session.id, &table, None).await.unwrap();
        assert_eq!(first.result.retained_rows, 500, "port {port}");
        let second = studio
            .browse(&session.id, &table, first.next_key)
            .await
            .unwrap();
        assert_eq!(second.result.retained_rows, 500, "port {port}");
        studio.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn reviewed_edits_are_bound_atomic_conflict_checked_and_separate_from_editor_transactions() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        let table = TableRef {
            schema: "joinery".into(),
            name: "sample".into(),
        };
        completed(&run(&studio,&session,"UPDATE sample SET note=CASE id WHEN 9007199254740992 THEN NULL WHEN 9007199254740993 THEN '' WHEN 9007199254740994 THEN 'row 2' ELSE 'row 500' END WHERE id IN (9007199254740992,9007199254740993,9007199254740994,9007199254741492)").await);
        let first = studio.browse(&session.id, &table, None).await.unwrap();
        let second = studio
            .browse(&session.id, &table, first.next_key.clone())
            .await
            .unwrap();
        let previous = studio.results.row(&first.result.id, 0).unwrap()[2].clone();
        let preview = studio
            .prepare_changes(
                &session.id,
                &first.result.id,
                vec![CellChange {
                    row: 0,
                    column: "c2".into(),
                    value: Some("reviewed edit".into()),
                }],
            )
            .unwrap();
        assert!(preview.sql[0].contains("`joinery`.`sample`"));
        assert!(preview.sql[0].contains("CAST(? AS UNSIGNED)"));
        assert!(!preview.sql[0].contains("reviewed edit"));
        completed(&execute(&studio, &preview.plan).await);
        let fresh = studio.browse(&session.id, &table, None).await.unwrap();
        assert_eq!(
            studio.results.row(&fresh.result.id, 0).unwrap()[2].as_deref(),
            Some("reviewed edit")
        );
        // CLIENT_FOUND_ROWS must accept an update whose new value is unchanged.
        let same = studio
            .prepare_changes(
                &session.id,
                &fresh.result.id,
                vec![CellChange {
                    row: 0,
                    column: "c2".into(),
                    value: Some("reviewed edit".into()),
                }],
            )
            .unwrap();
        completed(&execute(&studio, &same.plan).await);
        completed(
            &run(
                &studio,
                &session,
                "UPDATE sample SET note='concurrent writer' WHERE id=9007199254741492",
            )
            .await,
        );
        let stale = studio
            .prepare_changes_batch(
                &session.id,
                vec![
                    ResultChanges {
                        result_id: fresh.result.id.clone(),
                        changes: vec![CellChange {
                            row: 0,
                            column: "c2".into(),
                            value: Some("must roll back".into()),
                        }],
                    },
                    ResultChanges {
                        result_id: second.result.id.clone(),
                        changes: vec![CellChange {
                            row: 0,
                            column: "c2".into(),
                            value: Some("also roll back".into()),
                        }],
                    },
                ],
            )
            .unwrap();
        let events = execute(&studio, &stale.plan).await;
        assert!(
            events.iter().any(|e| matches!(
                e,
                QueryEvent::Error {
                    error: AppError {
                        code: ErrorCode::Conflict,
                        ..
                    },
                    ..
                }
            )),
            "{events:?}"
        );
        let check = run(
            &studio,
            &session,
            "SELECT note FROM sample WHERE id=9007199254740992",
        )
        .await;
        completed(&check);
        assert_eq!(
            studio.results.row(&result(&check).id, 0).unwrap()[0].as_deref(),
            Some("reviewed edit")
        );
        completed(
            &run(
                &studio,
                &session,
                "BEGIN; UPDATE sample SET note='editor uncommitted' WHERE id=9007199254740993",
            )
            .await,
        );
        let independent = studio.browse(&session.id, &table, None).await.unwrap();
        let independent_plan = studio
            .prepare_changes(
                &session.id,
                &independent.result.id,
                vec![CellChange {
                    row: 2,
                    column: "c2".into(),
                    value: Some("independent commit".into()),
                }],
            )
            .unwrap();
        completed(&execute(&studio, &independent_plan.plan).await);
        let observer = studio.browse(&session.id, &table, None).await.unwrap();
        assert_ne!(
            studio.results.row(&observer.result.id, 1).unwrap()[2].as_deref(),
            Some("editor uncommitted")
        );
        assert_eq!(
            studio.results.row(&observer.result.id, 2).unwrap()[2].as_deref(),
            Some("independent commit")
        );
        completed(&run(&studio, &session, "ROLLBACK").await);
        // Restore only task-owned fixture values for repeatable local runs.
        let restore = studio.browse(&session.id, &table, None).await.unwrap();
        let plan = studio
            .prepare_changes(
                &session.id,
                &restore.result.id,
                vec![CellChange {
                    row: 0,
                    column: "c2".into(),
                    value: previous,
                }],
            )
            .unwrap();
        completed(&execute(&studio, &plan.plan).await);
        studio.disconnect(&session.id);
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn cancellation_interrupts_only_the_owned_query_and_quarantines_the_session() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        let context = studio.completion_catalog(&session.id).await.unwrap();
        let plan = studio
            .prepare_execution(&session.id, "SELECT SLEEP(20)")
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = studio
            .execute(&plan.id, None, Arc::new(move |e| tx.send(e).is_ok()))
            .unwrap();
        while let Some(e) = rx.recv().await {
            if matches!(e, QueryEvent::StatementStarted { .. }) {
                break;
            }
        }
        // Include catalog work while the editor is busy; it owns a different connection.
        assert!(!studio.catalog(&session.id).await.unwrap().is_empty());
        assert_eq!(
            studio
                .completion_catalog(&session.id)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio
                .completion_schema(
                    &session.id,
                    &context.revision,
                    &TableRef {
                        schema: "joinery".into(),
                        name: "sample".into()
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        studio.cancel(&id).await.unwrap();
        let events = tokio::time::timeout(Duration::from_secs(5), async {
            let mut e = Vec::new();
            while let Some(event) = rx.recv().await {
                let done = matches!(event, QueryEvent::Finished { .. });
                e.push(event);
                if done {
                    break;
                }
            }
            e
        })
        .await
        .unwrap();
        assert!(
            events.iter().any(|e| matches!(
                e,
                QueryEvent::Finished {
                    status: RunStatus::Cancelled,
                    ..
                }
            )),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, QueryEvent::SessionClosed { .. })),
            "{events:?}"
        );
        assert_eq!(studio.usage().session_count, 0);
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL/MariaDB and scripts/mysql-tls-fixture.py"]
async fn verified_tls_through_ssh_keeps_the_original_database_hostname() {
    use russh::keys::{Algorithm, PrivateKey};
    use std::sync::atomic::Ordering;
    let ca = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache/mysql-fixture-tls/ca.pem")
        .canonicalize()
        .unwrap();
    for port in [53306, 53307] {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let bastion = common::bastion(port, &identity, None, 0).await;
        let mut p = profile(port, SafeMode::ReadOnly);
        p.host = "database.fixture.test".into();
        p.tls_mode = TlsMode::VerifyFull;
        p.ca_cert_path = Some(ca.to_string_lossy().into());
        p.ssh = Some(SshConfig {
            host: "127.0.0.1".into(),
            port: bastion.port,
            username: "fixture".into(),
            authentication: SshAuthentication::Password,
            private_key_path: None,
            agent_socket_path: None,
        });
        let saved = studio.save_profile(p.clone(), None).unwrap();
        let host = studio.probe_ssh(&saved.id).await.unwrap();
        assert_eq!(
            host.fingerprint,
            bastion
                .key
                .public_key()
                .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                .to_string()
        );
        studio
            .trust_ssh_key(host.challenge_id.as_ref().unwrap(), &host.fingerprint)
            .unwrap();
        let credentials = || ConnectionSecrets {
            password: Some("joinery-local-only".into()),
            ssh_password: Some("ssh-test-only".into()),
            ..Default::default()
        };
        let session = studio.connect(&saved.id, credentials()).await.unwrap();
        assert!(session.encrypted);
        completed(&run(&studio, &session, "SELECT 1").await);
        assert!(!studio.catalog(&session.id).await.unwrap().is_empty());
        studio.disconnect(&session.id);
        p.id = new_id();
        p.host = "wrong.fixture.test".into();
        let wrong = studio.save_profile(p, None).unwrap();
        assert!(
            studio.connect(&wrong.id, credentials()).await.is_err(),
            "wrong TLS hostname accepted"
        );
        assert!(bastion.connections.load(Ordering::SeqCst) > 0);
        studio.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn wide_rows_respect_byte_budgets_and_lossy_edits_roll_back() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        completed(&run(&studio,&session,"DROP TABLE IF EXISTS joinery_wide_qualification; CREATE TABLE joinery_wide_qualification (id INT PRIMARY KEY,body LONGTEXT, amount DECIMAL(6,2)) ENGINE=InnoDB; INSERT INTO joinery_wide_qualification VALUES (1,REPEAT('x',900000),1.00),(2,REPEAT('y',900000),2.00),(3,REPEAT('z',900000),3.00)").await);
        let table = TableRef {
            schema: "joinery".into(),
            name: "joinery_wide_qualification".into(),
        };
        let first = studio.browse(&session.id, &table, None).await.unwrap();
        assert_eq!(first.result.retained_rows, 2);
        assert!(first.has_more);
        let second = studio
            .browse(&session.id, &table, first.next_key)
            .await
            .unwrap();
        assert_eq!(second.result.retained_rows, 1);
        assert!(!second.has_more);
        assert_eq!(
            studio
                .prepare_changes(
                    &session.id,
                    &second.result.id,
                    vec![CellChange {
                        row: 0,
                        column: "c2".into(),
                        value: Some("3.001".into())
                    }]
                )
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        let result_set = run(
            &studio,
            &session,
            "SELECT amount FROM joinery_wide_qualification WHERE id=3",
        )
        .await;
        completed(&result_set);
        assert_eq!(
            studio.results.row(&result(&result_set).id, 0).unwrap()[0].as_deref(),
            Some("3.00")
        );
        completed(
            &run(
                &studio,
                &session,
                "UPDATE joinery_wide_qualification SET body=REPEAT('x',2200000) WHERE id=1",
            )
            .await,
        );
        assert_eq!(
            studio
                .browse(&session.id, &table, None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
        completed(&run(&studio, &session, "DROP TABLE joinery_wide_qualification").await);
        studio.disconnect(&session.id);
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn cancelling_a_grid_lock_wait_rolls_back_and_keeps_editor_writes_uncommitted() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        completed(
            &run(
                &studio,
                &session,
                "UPDATE sample SET note=NULL WHERE id=9007199254740992",
            )
            .await,
        );
        let table = TableRef {
            schema: "joinery".into(),
            name: "sample".into(),
        };
        let page = studio.browse(&session.id, &table, None).await.unwrap();
        completed(
            &run(
                &studio,
                &session,
                "BEGIN; UPDATE sample SET note='uncommitted editor' WHERE id=9007199254740992",
            )
            .await,
        );
        let edit = studio
            .prepare_changes(
                &session.id,
                &page.result.id,
                vec![CellChange {
                    row: 0,
                    column: "c2".into(),
                    value: Some("cancelled grid".into()),
                }],
            )
            .unwrap();
        let approval = studio
            .authorize(&edit.plan.id, None, edit.plan.confirmation_text.as_deref())
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = studio
            .execute(
                &edit.plan.id,
                Some(&approval),
                Arc::new(move |e| tx.send(e).is_ok()),
            )
            .unwrap();
        while let Some(e) = rx.recv().await {
            if matches!(e, QueryEvent::Started { .. }) {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        studio.cancel(&id).await.unwrap();
        let status = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(QueryEvent::Finished { status, .. }) = rx.recv().await {
                    break status;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(status, RunStatus::Cancelled);
        let (_root2, observer, other) = setup(port, SafeMode::ReadOnly).await;
        let check = run(
            &observer,
            &other,
            "SELECT note FROM sample WHERE id=9007199254740992",
        )
        .await;
        completed(&check);
        assert_eq!(
            observer.results.row(&result(&check).id, 0).unwrap()[0],
            None
        );
        observer.shutdown();
        studio.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn cancellation_before_submission_and_during_a_write_has_a_known_terminal_outcome() {
    for port in [53306, 53307] {
        for _ in 0..5 {
            let (_root, studio, session) = setup(port, SafeMode::Silent).await;
            let plan = studio
                .prepare_execution(&session.id, "SELECT SLEEP(20)")
                .unwrap();
            let (tx, mut rx) = mpsc::unbounded_channel();
            let id = studio
                .execute(&plan.id, None, Arc::new(move |e| tx.send(e).is_ok()))
                .unwrap();
            while let Some(e) = rx.recv().await {
                if matches!(e, QueryEvent::StatementStarted { .. }) {
                    break;
                }
            }
            studio.cancel(&id).await.unwrap();
            let status = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(QueryEvent::Finished { status, .. }) = rx.recv().await {
                        break status;
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(status, RunStatus::Cancelled);
        }
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        completed(&run(&studio,&session,"DROP TABLE IF EXISTS joinery_cancel_qualification; CREATE TABLE joinery_cancel_qualification (id INT PRIMARY KEY,value INT) ENGINE=InnoDB").await);
        let plan = studio
            .prepare_execution(
                &session.id,
                "INSERT INTO joinery_cancel_qualification SELECT 1,SLEEP(20)",
            )
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = studio
            .execute(&plan.id, None, Arc::new(move |e| tx.send(e).is_ok()))
            .unwrap();
        while let Some(e) = rx.recv().await {
            if matches!(e, QueryEvent::StatementStarted { .. }) {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        studio.cancel(&id).await.unwrap();
        let status = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(QueryEvent::Finished { status, .. }) = rx.recv().await {
                    break status;
                }
            }
        })
        .await
        .unwrap();
        // MySQL/MariaDB may report SLEEP interruption as a successful scalar
        // result. Preserve a known committed outcome rather than falsely label
        // an acknowledged write as cancelled.
        assert!(matches!(
            status,
            RunStatus::Completed | RunStatus::Cancelled
        ));
        let (_root2, observer, other) = setup(port, SafeMode::Silent).await;
        let check = run(
            &observer,
            &other,
            "SELECT COUNT(*) FROM joinery_cancel_qualification",
        )
        .await;
        completed(&check);
        assert_eq!(
            observer.results.row(&result(&check).id, 0).unwrap()[0].as_deref(),
            Some(if status == RunStatus::Completed {
                "1"
            } else {
                "0"
            })
        );
        completed(&run(&observer, &other, "DROP TABLE joinery_cancel_qualification").await);
        observer.shutdown();
        studio.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn a_full_result_registry_cannot_misreport_a_write_as_failed() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        completed(
            &run(
                &studio,
                &session,
                "DROP PROCEDURE IF EXISTS joinery_budget_qualification",
            )
            .await,
        );
        completed(&run(&studio,&session,"CREATE PROCEDURE joinery_budget_qualification() BEGIN UPDATE sample SET note='budget write acknowledged by server' WHERE id=9007199254740992; SELECT id,note FROM sample LIMIT 1; END").await);
        for _ in studio.usage().result_count..100 {
            studio.results.create("occupier", 0, vec![], None).unwrap();
        }
        let events = run(&studio, &session, "CALL joinery_budget_qualification()").await;
        completed(&events);
        assert!(events.iter().any(
            |e| matches!(e,QueryEvent::Notice{message} if message.contains("Close a result tab"))
        ));
        let (_root2, observer, other) = setup(port, SafeMode::Silent).await;
        let check = run(
            &observer,
            &other,
            "SELECT note FROM sample WHERE id=9007199254740992",
        )
        .await;
        completed(&check);
        assert_eq!(
            observer.results.row(&result(&check).id, 0).unwrap()[0].as_deref(),
            Some("budget write acknowledged by server")
        );
        completed(
            &run(
                &observer,
                &other,
                "DROP PROCEDURE joinery_budget_qualification",
            )
            .await,
        );
        observer.shutdown();
        studio.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn foreign_keys_group_composite_constraints_on_both_engines() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::ReadOnly).await;
        let schema = studio
            .schema(
                &session.id,
                &TableRef {
                    schema: "joinery".into(),
                    name: "line_notes".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(schema.foreign_keys.len(), 2, "port {port}");
        let line = &schema.foreign_keys[0];
        assert_eq!(line.name, "line_notes_line_fk");
        assert_eq!(line.columns, ["note_order", "note_line"]);
        assert_eq!(line.target.name, "order_lines");
        assert_eq!(line.target_columns, ["order_id", "line_no"]);
        let order = &schema.foreign_keys[1];
        assert_eq!(order.name, "line_notes_order_fk");
        assert_eq!(order.columns, ["note_order"]);
        assert_eq!(order.target_columns, ["id"]);
        studio.shutdown();
    }
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
    completed(&events);
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
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn browse_sorted_filtered_pages_match_reference_on_both_engines() {
    let sort = |column: &str, descending| SortKey {
        column: column.into(),
        descending,
    };
    let filter = |column: &str, op, value: Option<&str>| ColumnFilter {
        column: column.into(),
        op,
        value: value.map(str::to_owned),
    };
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::ReadOnly).await;
        let table = TableRef {
            schema: "joinery".into(),
            name: "browse_sort".into(),
        };
        let cases = [
            (
                BrowseQuery {
                    sort: vec![sort("bucket", true), sort("label", false)],
                    ..Default::default()
                },
                "SELECT id FROM browse_sort ORDER BY bucket IS NULL DESC, bucket DESC, label IS NULL, label, id",
            ),
            (
                BrowseQuery {
                    sort: vec![sort("label", true), sort("bucket", false)],
                    filters: vec![filter("bucket", FilterOp::NotEq, Some("3"))],
                    ..Default::default()
                },
                "SELECT id FROM browse_sort WHERE bucket <> 3 ORDER BY label IS NULL DESC, label DESC, bucket IS NULL, bucket, id",
            ),
            (
                BrowseQuery {
                    filters: vec![filter("label", FilterOp::Contains, Some("_%"))],
                    ..Default::default()
                },
                "SELECT id FROM browse_sort WHERE label LIKE '%!_!%%' ESCAPE '!' ORDER BY id",
            ),
            (
                BrowseQuery {
                    sort: vec![sort("bucket", false)],
                    filters: vec![filter("label", FilterOp::IsNull, None)],
                    ..Default::default()
                },
                "SELECT id FROM browse_sort WHERE label IS NULL ORDER BY bucket IS NULL, bucket, id",
            ),
            (
                BrowseQuery {
                    filters: vec![filter("label", FilterOp::StartsWith, Some("ALP"))],
                    ..Default::default()
                },
                "SELECT id FROM browse_sort WHERE label LIKE 'ALP%' ORDER BY id",
            ),
        ];
        for (query, reference) in cases {
            let (ids, pages) = browse_ids(&studio, &session, &table, &query).await;
            let expected = reference_ids(&studio, &session, reference).await;
            assert!(!expected.is_empty(), "port {port}: {reference}");
            assert_eq!(ids, expected, "port {port}: {query:?}");
            if expected.len() > 500 {
                assert!(pages > 1, "port {port}: {query:?} should page");
            }
        }
        let hostile = BrowseQuery {
            filters: vec![filter(
                "label",
                FilterOp::Eq,
                Some("' OR 1=1; DROP TABLE browse_sort; -- "),
            )],
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
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn raw_browse_clauses_enforce_safety_gates_on_mysql_and_mariadb() {
    let table = TableRef {
        schema: "joinery".into(),
        name: "browse_sort".into(),
    };
    for port in [53306, 53307] {
        let (_ro_root, ro_studio, ro_session) = setup(port, SafeMode::ReadOnly).await;
        let read_where = BrowseQuery {
            where_clause: Some("bucket IS NOT NULL AND id > 10".into()),
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
        assert!(pages > 1, "port {port}");
        assert_eq!(
            ids,
            reference_ids(
                &ro_studio,
                &ro_session,
                "SELECT id FROM browse_sort WHERE bucket IS NOT NULL AND id > 10 ORDER BY id"
            )
            .await
        );

        let unclassified = BrowseQuery {
            where_clause: Some("id = 1 AND BENCHMARK(1, 1) = 0".into()),
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
        ro_studio.shutdown();

        let (_rw_root, rw_studio, rw_session) = setup(port, SafeMode::Confirm).await;
        let plan = rw_studio
            .prepare_browse(&rw_session.id, &table, &unclassified)
            .await
            .unwrap()
            .expect("unclassified clause must require approval");
        let token = rw_studio.authorize(&plan.id, None, None).unwrap();
        let approval = BrowseApproval {
            plan_id: plan.id.clone(),
            approval: token,
        };
        let page = rw_studio
            .browse_with_approval(&rw_session.id, &table, None, &unclassified, Some(&approval))
            .await
            .unwrap();
        assert_eq!(page.result.retained_rows, 1);
        assert!(!page.has_more);
        rw_studio.release_result(&page.result.id);
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
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn referencing_foreign_keys_group_composite_inbound_constraints_on_mysql_and_mariadb() {
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::ReadOnly).await;
        let note_orders = TableRef {
            schema: "joinery".into(),
            name: "note_orders".into(),
        };
        let order_lines = TableRef {
            schema: "joinery".into(),
            name: "order_lines".into(),
        };
        let browse_sort = TableRef {
            schema: "joinery".into(),
            name: "browse_sort".into(),
        };

        let inbound_orders = studio
            .referencing_foreign_keys(&session.id, &note_orders)
            .await
            .unwrap();
        assert_eq!(inbound_orders.len(), 2, "port {port}");
        assert_eq!(inbound_orders[0].source.name, "line_notes");
        assert_eq!(inbound_orders[0].columns, ["note_order"]);
        assert_eq!(inbound_orders[0].target_columns, ["id"]);
        assert_eq!(inbound_orders[1].source.name, "order_lines");
        assert_eq!(inbound_orders[1].columns, ["order_id"]);
        assert_eq!(inbound_orders[1].target_columns, ["id"]);

        let inbound_lines = studio
            .referencing_foreign_keys(&session.id, &order_lines)
            .await
            .unwrap();
        assert_eq!(
            inbound_lines,
            vec![InboundForeignKey {
                name: "line_notes_line_fk".into(),
                source: TableRef {
                    schema: "joinery".into(),
                    name: "line_notes".into(),
                },
                columns: vec!["note_order".into(), "note_line".into()],
                target_columns: vec!["order_id".into(), "line_no".into()],
            }],
            "port {port}"
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
                        schema: "joinery".into(),
                        name: "missing_table".into(),
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        studio.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn table_changes_insert_update_delete_atomically_and_conflict_check_deletes_on_both_engines()
{
    for port in [53306, 53307] {
        let (_root, studio, session) = setup(port, SafeMode::Silent).await;
        completed(
            &run(
                &studio,
                &session,
                "DROP TABLE IF EXISTS joinery_batch2_mutations, joinery_batch2_defaults;
                 CREATE TABLE joinery_batch2_mutations (
                   id INT AUTO_INCREMENT PRIMARY KEY,
                   label VARCHAR(64) NOT NULL DEFAULT 'from-default',
                   amount DECIMAL(8,2) NOT NULL,
                   doubled DECIMAL(9,2) AS (amount * 2) STORED,
                   note VARCHAR(64)
                 ) ENGINE=InnoDB;
                 INSERT INTO joinery_batch2_mutations (label, amount, note)
                 VALUES ('alpha', 10.00, 'keep'), ('beta', 20.00, 'delete-me');
                 CREATE TABLE joinery_batch2_defaults (
                   id INT AUTO_INCREMENT PRIMARY KEY,
                   tag VARCHAR(32) NOT NULL DEFAULT 'auto'
                 ) ENGINE=InnoDB;",
            )
            .await,
        );

        let table = TableRef {
            schema: "joinery".into(),
            name: "joinery_batch2_mutations".into(),
        };
        let schema = studio.schema(&session.id, &table).await.unwrap();
        let col = |name: &str| schema.columns.iter().find(|c| c.name == name).unwrap();
        assert!(
            !col("id").editable && col("id").insertable && col("id").has_default,
            "port {port}"
        );
        assert!(
            col("label").editable && col("label").insertable && col("label").has_default,
            "port {port}"
        );
        assert!(
            col("amount").editable && col("amount").insertable && !col("amount").has_default,
            "port {port}"
        );
        assert!(
            !col("doubled").editable && !col("doubled").insertable && col("doubled").has_default,
            "port {port}"
        );
        assert!(
            col("note").editable && col("note").insertable && !col("note").has_default,
            "port {port}"
        );

        // ReadOnly profiles mask insertable=false without triggering false schema-drift conflicts on browse.
        let (_ro_root, ro_studio, ro_session) = setup(port, SafeMode::ReadOnly).await;
        let ro_schema = ro_studio.schema(&ro_session.id, &table).await.unwrap();
        assert!(
            !ro_schema.editable
                && ro_schema
                    .columns
                    .iter()
                    .all(|c| !c.editable && !c.insertable),
            "port {port}"
        );
        let ro_page = ro_studio
            .browse(&ro_session.id, &table, None)
            .await
            .unwrap();
        assert_eq!(ro_page.result.retained_rows, 2, "port {port}");
        ro_studio.release_result(&ro_page.result.id);
        ro_studio.shutdown();

        let page = studio.browse(&session.id, &table, None).await.unwrap();
        assert_eq!(page.result.retained_rows, 2, "port {port}");

        // Lossy decimal precision on insert is rejected before preparing SQL.
        assert_eq!(
            studio
                .prepare_table_changes(
                    &session.id,
                    TableChanges {
                        result_id: Some(page.result.id.clone()),
                        updates: vec![],
                        deletes: vec![],
                        inserts: vec![RowInsert {
                            values: vec![InsertValue {
                                column: "c2".into(),
                                value: Some("10.009".into()),
                            }],
                        }],
                    },
                )
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput,
            "port {port}"
        );

        // Mixed atomic batch: update row 0, delete row 1, insert 1 row relying on AUTO_INCREMENT + DEFAULT.
        let preview = studio
            .prepare_table_changes(
                &session.id,
                TableChanges {
                    result_id: Some(page.result.id.clone()),
                    updates: vec![ResultChanges {
                        result_id: page.result.id.clone(),
                        changes: vec![CellChange {
                            row: 0,
                            column: "c2".into(),
                            value: Some("15.50".into()),
                        }],
                    }],
                    deletes: vec![ResultDeletes {
                        result_id: page.result.id.clone(),
                        rows: vec![1],
                    }],
                    inserts: vec![RowInsert {
                        values: vec![InsertValue {
                            column: "c2".into(),
                            value: Some("30.25".into()),
                        }],
                    }],
                },
            )
            .unwrap();
        assert_eq!(preview.row_count, 3, "port {port}");
        completed(&execute(&studio, &preview.plan).await);
        studio.release_result(&page.result.id);

        let after = studio.browse(&session.id, &table, None).await.unwrap();
        assert_eq!(after.result.retained_rows, 2, "port {port}");
        let r0 = studio.results.row(&after.result.id, 0).unwrap();
        assert_eq!(r0[0].as_deref(), Some("1"));
        assert_eq!(r0[2].as_deref(), Some("15.50"));
        assert_eq!(r0[3].as_deref(), Some("31.00"));
        let r1 = studio.results.row(&after.result.id, 1).unwrap();
        assert_eq!(r1[1].as_deref(), Some("from-default"));
        assert_eq!(r1[2].as_deref(), Some("30.25"));
        assert_eq!(r1[3].as_deref(), Some("60.50"));
        assert_eq!(r1[4], None);

        // Empty-table insert with 0 explicit values generates INSERT INTO ... () VALUES ().
        let defaults_table = TableRef {
            schema: "joinery".into(),
            name: "joinery_batch2_defaults".into(),
        };
        let empty_page = studio
            .browse(&session.id, &defaults_table, None)
            .await
            .unwrap();
        assert_eq!(empty_page.result.retained_rows, 0, "port {port}");
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
        assert!(
            defaults_preview.sql[0].contains("() VALUES ()"),
            "port {port}: {}",
            defaults_preview.sql[0]
        );
        completed(&execute(&studio, &defaults_preview.plan).await);
        studio.release_result(&empty_page.result.id);
        let defaults_after = studio
            .browse(&session.id, &defaults_table, None)
            .await
            .unwrap();
        assert_eq!(defaults_after.result.retained_rows, 1, "port {port}");
        assert_eq!(
            studio.results.row(&defaults_after.result.id, 0).unwrap()[1].as_deref(),
            Some("auto"),
            "port {port}"
        );
        studio.release_result(&defaults_after.result.id);

        // Concurrent modification causes FOR UPDATE conflict on Delete and rolls back the whole batch.
        completed(
            &run(
                &studio,
                &session,
                "UPDATE joinery_batch2_mutations SET note = 'concurrent-touch' WHERE id = 3",
            )
            .await,
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
                            column: "c2".into(),
                            value: Some("999.00".into()),
                        }],
                    }],
                    deletes: vec![ResultDeletes {
                        result_id: after.result.id.clone(),
                        rows: vec![1],
                    }],
                    inserts: vec![RowInsert {
                        values: vec![InsertValue {
                            column: "c2".into(),
                            value: Some("50.00".into()),
                        }],
                    }],
                },
            )
            .unwrap();
        let conflict_events = execute(&studio, &stale_batch.plan).await;
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
            "port {port}: {conflict_events:?}"
        );
        studio.release_result(&after.result.id);

        let rolled_back = studio.browse(&session.id, &table, None).await.unwrap();
        assert_eq!(rolled_back.result.retained_rows, 2, "port {port}");
        assert_eq!(
            studio.results.row(&rolled_back.result.id, 0).unwrap()[2].as_deref(),
            Some("15.50"),
            "port {port}"
        );
        assert_eq!(
            studio.results.row(&rolled_back.result.id, 1).unwrap()[4].as_deref(),
            Some("concurrent-touch"),
            "port {port}"
        );
        studio.release_result(&rolled_back.result.id);

        completed(
            &run(
                &studio,
                &session,
                "DROP TABLE joinery_batch2_mutations, joinery_batch2_defaults",
            )
            .await,
        );
        studio.disconnect(&session.id);
    }
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
#[ignore = "requires disposable MySQL and MariaDB fixtures"]
async fn mysql_and_mariadb_stream_multi_page_exports_without_retaining_rows() {
    use joinery_core::file_capability::SandboxFileDialog;

    for port in [53306, 53307] {
        let root = tempfile::tempdir().unwrap();
        let sandbox = SandboxFileDialog::new(&root.path().join("exports")).unwrap();
        let studio = Studio::with_secrets_and_dialog(
            root.path(),
            Arc::new(MemorySecrets::default()),
            sandbox.clone(),
        )
        .unwrap();
        let rw = studio
            .save_profile(profile(port, SafeMode::Silent), None)
            .unwrap();
        let mut ro_prof = profile(port, SafeMode::ReadOnly);
        ro_prof.name = format!("MySQL Export ReadOnly {port}");
        let ro = studio.save_profile(ro_prof, None).unwrap();

        let session = studio
            .connect(
                &rw.id,
                ConnectionSecrets {
                    password: Some("joinery-local-only".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let ro_session = studio
            .connect(
                &ro.id,
                ConnectionSecrets {
                    password: Some("joinery-local-only".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        completed(
            &run(
                &studio,
                &session,
                "DROP TABLE IF EXISTS joinery_r2_mysql_export; \
                 CREATE TABLE joinery_r2_mysql_export ( \
                   id BIGINT PRIMARY KEY, \
                   big_val BIGINT NOT NULL, \
                   dec_val DECIMAL(30, 8) NOT NULL, \
                   note VARCHAR(128) NULL \
                 )",
            )
            .await,
        );
        for chunk_start in (1..=1550).step_by(250) {
            let chunk_end = (chunk_start + 249).min(1550);
            let tuples = (chunk_start..=chunk_end)
                .map(|g| {
                    let big = if g == 1 {
                        "9223372036854775807".to_string()
                    } else {
                        (-9223372036854775000_i64 + g as i64).to_string()
                    };
                    let note = match g % 3 {
                        0 => "NULL".to_string(),
                        1 => "''".to_string(),
                        _ => "'café\\\\line'".to_string(),
                    };
                    format!("({g}, {big}, 12345678901234567890.12345678, {note})")
                })
                .collect::<Vec<_>>()
                .join(", ");
            completed(
                &run(
                    &studio,
                    &session,
                    &format!(
                        "INSERT INTO joinery_r2_mysql_export (id, big_val, dec_val, note) VALUES {tuples}"
                    ),
                )
                .await,
            );
        }
        studio.clear_history().unwrap();

        let table = TableRef {
            schema: "joinery".into(),
            name: "joinery_r2_mysql_export".into(),
        };
        let grant = studio
            .pick_export_destination("mysql-stream.sql", ExportFormat::SqlInsert)
            .await
            .unwrap()
            .unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        studio
            .start_export(
                ExportRequest {
                    destination_token: grant.token,
                    format: ExportFormat::SqlInsert,
                    source: ExportSource::Table {
                        session_id: ro_session.id.clone(),
                        table: table.clone(),
                        query: BrowseQuery::default(),
                        approval: None,
                        column_ids: Vec::new(),
                    },
                    include_headers: true,
                    engine: Some(EngineKind::Mysql),
                    target_table: Some(table.clone()),
                },
                Arc::new(move |e| tx.send(e).is_ok()),
            )
            .await
            .unwrap();
        let events = finish_export(rx).await;
        assert_eq!(studio.results.usage(), (0, 0), "port {port}");
        assert!(
            matches!(
                events.last(),
                Some(ExportEvent::Finished {
                    status: RunStatus::Completed,
                    rows_written: 1550,
                    ..
                })
            ),
            "port {port}: {events:?}"
        );
        let sql_text = std::fs::read_to_string(sandbox.root().join("mysql-stream.sql")).unwrap();
        assert!(sql_text.contains(
            "INSERT INTO `joinery`.`joinery_r2_mysql_export` (`id`, `big_val`, `dec_val`, `note`)"
        ));
        assert!(sql_text.contains("(1, 9223372036854775807, 12345678901234567890.12345678, '')"));
        assert!(sql_text.contains("'café\\\\line'"));

        // ReadOnly blocks GET_LOCK raw clause
        let ro_grant = studio
            .pick_export_destination("blocked-mysql.csv", ExportFormat::Csv)
            .await
            .unwrap()
            .unwrap();
        let err = studio
            .start_export(
                ExportRequest {
                    destination_token: ro_grant.token,
                    format: ExportFormat::Csv,
                    source: ExportSource::Table {
                        session_id: ro_session.id.clone(),
                        table: table.clone(),
                        query: BrowseQuery {
                            where_clause: Some("GET_LOCK('joinery', 0) = 1".into()),
                            ..Default::default()
                        },
                        approval: None,
                        column_ids: Vec::new(),
                    },
                    include_headers: true,
                    engine: Some(EngineKind::Mysql),
                    target_table: Some(table.clone()),
                },
                Arc::new(|_| true),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::PolicyBlocked, "port {port}");

        completed(&run(&studio, &session, "DROP TABLE joinery_r2_mysql_export").await);
        studio.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL and MariaDB services"]
async fn mysql_and_mariadb_import_csv_batches_enforce_read_only_and_rollback_conflicts() {
    use joinery_core::file_capability::SandboxFileDialog;

    for port in [53306, 53307] {
        let root = tempfile::tempdir().unwrap();
        let sandbox = SandboxFileDialog::new(&root.path().join("mysql-imports")).unwrap();
        let studio = Studio::with_secrets_and_dialog(
            root.path(),
            Arc::new(MemorySecrets::default()),
            sandbox.clone(),
        )
        .unwrap();

        let rw_p = studio
            .save_profile(profile(port, SafeMode::Silent), None)
            .unwrap();
        let ro_p = studio
            .save_profile(profile(port, SafeMode::ReadOnly), None)
            .unwrap();
        let make_secrets = || ConnectionSecrets {
            password: Some("joinery-local-only".into()),
            ..Default::default()
        };
        let session = studio.connect(&rw_p.id, make_secrets()).await.unwrap();
        let ro_session = studio.connect(&ro_p.id, make_secrets()).await.unwrap();

        completed(
            &run(
                &studio,
                &session,
                "DROP TABLE IF EXISTS joinery_r2_mysql_import;
                 CREATE TABLE joinery_r2_mysql_import (
                   id BIGINT AUTO_INCREMENT PRIMARY KEY,
                   sku VARCHAR(64) NOT NULL UNIQUE,
                   big_val BIGINT NOT NULL,
                   price DECIMAL(14,4) NOT NULL,
                   note VARCHAR(128) NULL,
                   active TINYINT(1) NOT NULL,
                   origin VARCHAR(32) NOT NULL DEFAULT 'csv-default'
                 ) ENGINE=InnoDB;",
            )
            .await,
        );

        let table = TableRef {
            schema: "joinery".into(),
            name: "joinery_r2_mysql_import".into(),
        };

        // Invalid precision decimal rejected upfront
        let bad_csv = "sku,big_val,price,note,active\nSKU-1,10,12.12345,,true\n";
        std::fs::write(sandbox.root().join("bad-mysql.csv"), bad_csv.as_bytes()).unwrap();
        sandbox.queue_open(Some("bad-mysql.csv")).unwrap();
        let bad_grant = studio.pick_import_source().await.unwrap().unwrap();

        assert_eq!(
            studio
                .preview_csv_import(
                    &ro_session.id,
                    &bad_grant.token,
                    &table,
                    CsvImportOptions::default(),
                    Vec::new(),
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::PolicyBlocked,
            "port {port}"
        );

        let bad_preview = studio
            .preview_csv_import(
                &session.id,
                &bad_grant.token,
                &table,
                CsvImportOptions::default(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert!(!bad_preview.can_prepare, "port {port}");
        assert_eq!(bad_preview.issues.len(), 1);
        assert_eq!(bad_preview.issues[0].column.as_deref(), Some("price"));

        // Valid CSV with TINYINT(1) boolean coercion ("true"/"false"), NULL vs "" vs "\N", and <default> columns
        let valid_csv = "sku,big_val,price,note,active\nSKU-A,9223372036854775807,999.1234,,true\nSKU-B,-9223372036854775808,0.5000,\"\",false\nSKU-C,42,10.0000,\"\\N\",1\n";
        std::fs::write(sandbox.root().join("good-mysql.csv"), valid_csv.as_bytes()).unwrap();
        sandbox.queue_open(Some("good-mysql.csv")).unwrap();
        let good_grant = studio.pick_import_source().await.unwrap().unwrap();

        let preview = studio
            .preview_csv_import(
                &session.id,
                &good_grant.token,
                &table,
                CsvImportOptions::default(),
                Vec::new(),
            )
            .await
            .unwrap();
        assert!(preview.can_prepare, "port {port}: {:?}", preview.issues);
        assert_eq!(preview.unmapped_defaults, vec!["id", "origin"]);

        let batch = studio
            .prepare_csv_import_batch(
                &session.id,
                &good_grant.token,
                &table,
                CsvImportOptions::default(),
                preview.mappings.clone(),
                0,
            )
            .await
            .unwrap();
        let token = studio
            .authorize(
                &batch.plan.id,
                None,
                batch.plan.confirmation_text.as_deref(),
            )
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        studio
            .execute(
                &batch.plan.id,
                Some(&token),
                Arc::new(move |e| tx.send(e).is_ok()),
            )
            .unwrap();
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            let done = matches!(ev, QueryEvent::Finished { .. });
            events.push(ev);
            if done {
                break;
            }
        }
        completed(&events);

        // Mid-batch duplicate UNIQUE key rolls back atomically on InnoDB
        let conflict_csv = "sku,big_val,price,note,active\nSKU-ROLLBACK,7,1.0000,first,true\nSKU-A,8,2.0000,dup,false\n";
        std::fs::write(
            sandbox.root().join("conflict-mysql.csv"),
            conflict_csv.as_bytes(),
        )
        .unwrap();
        sandbox.queue_open(Some("conflict-mysql.csv")).unwrap();
        let conflict_grant = studio.pick_import_source().await.unwrap().unwrap();
        let conflict_batch = studio
            .prepare_csv_import_batch(
                &session.id,
                &conflict_grant.token,
                &table,
                CsvImportOptions::default(),
                preview.mappings,
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
        let (ctx, mut crx) = mpsc::unbounded_channel();
        studio
            .execute(
                &conflict_batch.plan.id,
                Some(&conflict_token),
                Arc::new(move |e| ctx.send(e).is_ok()),
            )
            .unwrap();
        let mut cevents = Vec::new();
        while let Some(ev) = crx.recv().await {
            let done = matches!(ev, QueryEvent::Finished { .. });
            cevents.push(ev);
            if done {
                break;
            }
        }
        assert!(
            cevents.iter().any(|e| matches!(
                e,
                QueryEvent::Finished {
                    status: RunStatus::Failed,
                    ..
                }
            )),
            "port {port}: {cevents:?}"
        );

        let page = studio.browse(&session.id, &table, None).await.unwrap();
        assert_eq!(page.result.retained_rows, 3, "port {port}");
        let r0 = studio.results.row(&page.result.id, 0).unwrap();
        let r1 = studio.results.row(&page.result.id, 1).unwrap();
        let r2 = studio.results.row(&page.result.id, 2).unwrap();
        assert_eq!(r0[1].as_deref(), Some("SKU-A"));
        assert_eq!(r0[2].as_deref(), Some("9223372036854775807"));
        assert_eq!(r0[4], None);
        assert_eq!(r0[5].as_deref(), Some("1"));
        assert_eq!(r0[6].as_deref(), Some("csv-default"));
        assert_eq!(r1[4].as_deref(), Some(""));
        assert_eq!(r1[5].as_deref(), Some("0"));
        assert_eq!(r2[4].as_deref(), Some("\\N"));
        studio.release_result(&page.result.id);

        completed(&run(&studio, &session, "DROP TABLE joinery_r2_mysql_import").await);
        studio.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires disposable MySQL, MariaDB, and PostgreSQL services"]
async fn mysql_mariadb_and_cross_engine_postgres_data_comparison() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let make_secrets = || ConnectionSecrets {
        password: Some("joinery-local-only".into()),
        ..Default::default()
    };

    let mysql_p = studio
        .save_profile(profile(53306, SafeMode::Silent), None)
        .unwrap();
    let maria_p = studio
        .save_profile(profile(53307, SafeMode::Silent), None)
        .unwrap();
    let pg_port: u16 = std::env::var("JOINERY_TEST_PORT")
        .unwrap_or_else(|_| "55432".into())
        .parse()
        .unwrap();
    let pg_p = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "PG Cross Compare".into(),
                group_id: "local".into(),
                engine: EngineKind::Postgres,
                host: "127.0.0.1".into(),
                port: pg_port,
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

    let mysql_s = studio.connect(&mysql_p.id, make_secrets()).await.unwrap();
    let maria_s = studio.connect(&maria_p.id, make_secrets()).await.unwrap();
    let pg_s = studio.connect(&pg_p.id, make_secrets()).await.unwrap();

    completed(
        &run(
            &studio,
            &mysql_s,
            "DROP TABLE IF EXISTS joinery_r2_xengine_cmp;
             CREATE TABLE joinery_r2_xengine_cmp (
               id BIGINT NOT NULL PRIMARY KEY,
               sku VARCHAR(64) NOT NULL,
               amount DECIMAL(18,4) NOT NULL,
               note VARCHAR(128) NULL
             ) ENGINE=InnoDB;
             INSERT INTO joinery_r2_xengine_cmp VALUES
               (1, 'SAME-MAX', 99999999.1234, 'identical'),
               (2, 'DIFF-NOTE', 10.5000, NULL),
               (3, 'ONLY-MYSQL', 1.0000, 'left');",
        )
        .await,
    );

    completed(
        &run(
            &studio,
            &maria_s,
            "DROP TABLE IF EXISTS joinery_r2_xengine_cmp;
             CREATE TABLE joinery_r2_xengine_cmp (
               id BIGINT NOT NULL PRIMARY KEY,
               sku VARCHAR(64) NOT NULL,
               amount DECIMAL(18,4) NOT NULL,
               note VARCHAR(128) NULL
             ) ENGINE=InnoDB;
             INSERT INTO joinery_r2_xengine_cmp VALUES
               (1, 'SAME-MAX', 99999999.1234, 'identical'),
               (2, 'DIFF-NOTE', 10.5000, ''),
               (4, 'ONLY-MARIA', 2.0000, 'right');",
        )
        .await,
    );

    completed(
        &run(
            &studio,
            &pg_s,
            "DROP TABLE IF EXISTS public.joinery_r2_xengine_cmp;
             CREATE TABLE public.joinery_r2_xengine_cmp (
               id BIGINT NOT NULL PRIMARY KEY,
               sku TEXT NOT NULL,
               amount NUMERIC(18,4) NOT NULL,
               note TEXT NULL
             );
             INSERT INTO public.joinery_r2_xengine_cmp VALUES
               (1, 'SAME-MAX', 99999999.1234, 'identical'),
               (2, 'DIFF-NOTE', 10.5000, ''),
               (5, 'ONLY-PG', 3.0000, 'pg-only');",
        )
        .await,
    );

    // MySQL 8.4 <-> MariaDB 11.4 LiveTable comparison
    let mysql_vs_maria = studio
        .compare_data(CompareRequest {
            left: CompareSide::LiveTable {
                session_id: mysql_s.id.clone(),
                table: TableRef {
                    schema: "joinery".into(),
                    name: "joinery_r2_xengine_cmp".into(),
                },
                query: BrowseQuery::default(),
                approval: None,
                label: Some("MySQL 8.4".into()),
            },
            right: CompareSide::LiveTable {
                session_id: maria_s.id.clone(),
                table: TableRef {
                    schema: "joinery".into(),
                    name: "joinery_r2_xengine_cmp".into(),
                },
                query: BrowseQuery::default(),
                approval: None,
                label: Some("MariaDB 11.4".into()),
            },
            key_columns: Vec::new(),
            ignore_column_case: false,
        })
        .await
        .unwrap();
    assert_eq!(mysql_vs_maria.identical_count, 1);
    assert_eq!(mysql_vs_maria.different_count, 1);
    assert_eq!(mysql_vs_maria.only_left_count, 1);
    assert_eq!(mysql_vs_maria.only_right_count, 1);

    // MySQL 8.4 <-> PostgreSQL cross-engine LiveTable comparison
    let mysql_vs_pg = studio
        .compare_data(CompareRequest {
            left: CompareSide::LiveTable {
                session_id: mysql_s.id.clone(),
                table: TableRef {
                    schema: "joinery".into(),
                    name: "joinery_r2_xengine_cmp".into(),
                },
                query: BrowseQuery::default(),
                approval: None,
                label: Some("MySQL".into()),
            },
            right: CompareSide::LiveTable {
                session_id: pg_s.id.clone(),
                table: TableRef {
                    schema: "public".into(),
                    name: "joinery_r2_xengine_cmp".into(),
                },
                query: BrowseQuery::default(),
                approval: None,
                label: Some("PostgreSQL".into()),
            },
            key_columns: Vec::new(),
            ignore_column_case: false,
        })
        .await
        .unwrap();
    assert_eq!(mysql_vs_pg.key_columns, vec!["id"]);
    assert_eq!(mysql_vs_pg.identical_count, 1);
    assert_eq!(mysql_vs_pg.different_count, 1);
    assert_eq!(mysql_vs_pg.only_left_count, 1);
    assert_eq!(mysql_vs_pg.only_right_count, 1);

    completed(&run(&studio, &mysql_s, "DROP TABLE joinery_r2_xengine_cmp").await);
    completed(&run(&studio, &maria_s, "DROP TABLE joinery_r2_xengine_cmp").await);
    completed(&run(&studio, &pg_s, "DROP TABLE public.joinery_r2_xengine_cmp").await);
    studio.shutdown();
}
