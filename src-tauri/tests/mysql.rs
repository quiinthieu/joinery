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
