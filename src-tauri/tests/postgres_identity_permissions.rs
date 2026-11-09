//! Opt-in qualification against the fixed disposable PostgreSQL fixture only.
//! Roles and objects are UUID-owned; passwords are public synthetic fixture data.
use futures_util::{FutureExt, TryStreamExt};
use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use std::{error::Error, sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::timeout};
use tokio_postgres::{Client, Config, NoTls};

type R<T> = Result<T, Box<dyn Error + Send + Sync>>;
const DEADLINE: Duration = Duration::from_secs(10);
const ROLE_PASSWORD: &str = "joinery-public-synthetic-identity-only";

struct Connection {
    client: Client,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn connect(user: &str, password: &str) -> R<Connection> {
    let mut config = Config::new();
    config
        .host("127.0.0.1")
        .port(55432)
        .dbname("joinery")
        .user(user)
        .password(password)
        .connect_timeout(Duration::from_secs(3));
    let (client, connection) = timeout(DEADLINE, config.connect(NoTls)).await??;
    let owned = Connection {
        client,
        task: tokio::spawn(async move {
            let _ = connection.await;
        }),
    };
    sql(
        &owned.client,
        "SET statement_timeout='5s'; SET lock_timeout='1s'",
    )
    .await?;
    Ok(owned)
}
async fn sql(client: &Client, statement: &str) -> R<()> {
    timeout(DEADLINE, client.batch_execute(statement)).await??;
    Ok(())
}
fn q(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

struct App {
    studio: Arc<Studio>,
    session: SessionInfo,
    _root: tempfile::TempDir,
}
impl Drop for App {
    fn drop(&mut self) {
        self.studio.shutdown();
    }
}
impl App {
    async fn connect(user: &str, password: &str, mode: SafeMode) -> R<Self> {
        let root = tempfile::tempdir()?;
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default()))?;
        let profile = studio.save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "Owned identity permissions fixture".into(),
                group_id: "local".into(),
                engine: EngineKind::Postgres,
                host: "127.0.0.1".into(),
                port: 55432,
                database: "joinery".into(),
                username: user.into(),
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
        )?;
        let session = match timeout(
            DEADLINE,
            studio.connect(
                &profile.id,
                ConnectionSecrets {
                    password: Some(password.into()),
                    ..Default::default()
                },
            ),
        )
        .await
        {
            Ok(Ok(session)) => session,
            other => {
                studio.shutdown();
                return Err(format!("fixture login failed: {other:?}").into());
            }
        };
        Ok(Self {
            studio,
            session,
            _root: root,
        })
    }
    async fn execute(&self, plan: &PreparedPlan) -> R<Vec<QueryEvent>> {
        let approval = if plan.requires_approval {
            Some(
                self.studio
                    .authorize(&plan.id, None, plan.confirmation_text.as_deref())?,
            )
        } else {
            None
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let run = self.studio.execute(
            &plan.id,
            approval.as_deref(),
            Arc::new(move |event| tx.send(event).is_ok()),
        )?;
        let outcome = timeout(DEADLINE, async {
            let mut events = Vec::new();
            while let Some(event) = rx.recv().await {
                let finished = matches!(event, QueryEvent::Finished { .. });
                events.push(event);
                if finished {
                    return Ok(events);
                }
            }
            Err("execution ended without Finished".into())
        })
        .await;
        match outcome {
            Ok(result) => result,
            Err(_) => {
                let _ = timeout(Duration::from_secs(3), self.studio.cancel(&run)).await;
                Err("fixture execution exceeded deadline".into())
            }
        }
    }
    async fn query(&self, statement: &str) -> R<Vec<QueryEvent>> {
        let plan = self.studio.prepare_execution(&self.session.id, statement)?;
        self.execute(&plan).await
    }
    fn rows(&self, result: &ResultInfo) -> R<Vec<TextRow>> {
        (0..result.retained_rows)
            .map(|index| {
                self.studio
                    .results
                    .row(&result.id, index)
                    .map_err(Into::into)
            })
            .collect()
    }
}
fn status(events: &[QueryEvent]) -> Option<RunStatus> {
    events.iter().find_map(|event| {
        if let QueryEvent::Finished { status, .. } = event {
            Some(*status)
        } else {
            None
        }
    })
}
fn result(events: &[QueryEvent]) -> R<ResultInfo> {
    events
        .iter()
        .rev()
        .find_map(|event| {
            if let QueryEvent::Result { result } = event {
                Some(result.clone())
            } else {
                None
            }
        })
        .ok_or_else(|| format!("missing result: {events:?}").into())
}
fn update(result: &ResultInfo) -> TableChanges {
    TableChanges {
        result_id: Some(result.id.clone()),
        updates: vec![ResultChanges {
            result_id: result.id.clone(),
            changes: vec![CellChange {
                row: 0,
                column: "c1".into(),
                value: Some("approved".into()),
            }],
        }],
        deletes: vec![],
        inserts: vec![],
    }
}
fn insert(result: &ResultInfo, id: i32) -> TableChanges {
    TableChanges {
        result_id: Some(result.id.clone()),
        updates: vec![],
        deletes: vec![],
        inserts: vec![RowInsert {
            values: vec![
                InsertValue {
                    column: "c0".into(),
                    value: Some(id.to_string()),
                },
                InsertValue {
                    column: "c1".into(),
                    value: Some("inserted".into()),
                },
            ],
        }],
    }
}
async fn stored_rows(observer: &Client, table: &str) -> R<Vec<(i32, String)>> {
    Ok(timeout(
        DEADLINE,
        observer.query(&format!("SELECT id,note FROM {table} ORDER BY id"), &[]),
    )
    .await??
    .into_iter()
    .map(|row| (row.get(0), row.get(1)))
    .collect())
}

#[derive(Clone, Copy)]
enum Case {
    ColumnGrants,
    OwnerRls,
    RawSideEffect,
    RawLateError,
}

async fn exercise(case: Case, schema: &str, role: &str) -> R<()> {
    let observer = connect("joinery", "joinery-local-only").await?;
    let target = format!("{}.target", q(schema));
    sql(&observer.client, &format!("CREATE SCHEMA {}; CREATE TABLE {target}(id integer PRIMARY KEY,note text NOT NULL); INSERT INTO {target} VALUES(1,'before'),(2,'hidden')", q(schema))).await?;
    let table = TableRef {
        schema: schema.into(),
        name: "target".into(),
    };
    match case {
        Case::ColumnGrants => {
            sql(&observer.client, &format!("CREATE ROLE {} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS PASSWORD '{ROLE_PASSWORD}'; GRANT USAGE ON SCHEMA {} TO {}; GRANT SELECT(id,note),UPDATE(note),INSERT(id,note) ON {target} TO {}", q(role),q(schema),q(role),q(role))).await?;
            let direct = connect(role, ROLE_PASSWORD).await?;
            assert_eq!(
                stored_rows(&direct.client, &target).await?,
                vec![(1, "before".into()), (2, "hidden".into())]
            );
            let denied = timeout(
                DEADLINE,
                direct
                    .client
                    .query(&format!("SELECT tableoid FROM {target} LIMIT 0"), &[]),
            )
            .await?
            .unwrap_err();
            assert_eq!(
                denied.code(),
                Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
            );
            let app = App::connect(role, ROLE_PASSWORD, SafeMode::Silent).await?;
            let initial_schema =
                timeout(DEADLINE, app.studio.schema(&app.session.id, &table)).await??;
            assert!(!initial_schema.editable);
            assert!(
                initial_schema
                    .columns
                    .iter()
                    .all(|column| !column.editable && !column.insertable)
            );
            let events = app
                .query(&format!("SELECT id,note FROM {target} ORDER BY id"))
                .await?;
            assert_eq!(status(&events), Some(RunStatus::Completed), "{events:?}");
            let query = result(&events)?;
            assert!(
                matches!(query.editability, Some(ResultEditability::ReadOnly { .. })),
                "{query:?}"
            );
            assert_eq!(
                app.rows(&query)?,
                vec![
                    vec![Some("1".into()), Some("before".into())],
                    vec![Some("2".into()), Some("hidden".into())]
                ]
            );
            assert_eq!(
                app.studio
                    .prepare_table_changes(&app.session.id, update(&query))
                    .unwrap_err()
                    .code,
                ErrorCode::PolicyBlocked
            );
            app.studio.release_result(&query.id);
            let page =
                timeout(DEADLINE, app.studio.browse(&app.session.id, &table, None)).await??;
            assert_eq!(initial_schema.columns, page.result.columns);
            assert_eq!(
                app.rows(&page.result)?,
                vec![
                    vec![Some("1".into()), Some("before".into())],
                    vec![Some("2".into()), Some("hidden".into())]
                ]
            );
            assert!(
                page.result
                    .columns
                    .iter()
                    .all(|column| !column.editable && !column.insertable)
            );
            assert_eq!(
                app.studio
                    .prepare_table_changes(&app.session.id, update(&page.result))
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput
            );
            app.studio.release_result(&page.result.id);
            // Qualification never grants table-level/system-column SELECT as a workaround.
            assert!(
                timeout(
                    DEADLINE,
                    direct
                        .client
                        .query(&format!("SELECT tableoid FROM {target} LIMIT 0"), &[])
                )
                .await?
                .is_err()
            );
            assert_eq!(
                stored_rows(&observer.client, &target).await?,
                vec![(1, "before".into()), (2, "hidden".into())]
            );
        }
        Case::OwnerRls => {
            sql(&observer.client, &format!("CREATE ROLE {} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS PASSWORD '{ROLE_PASSWORD}'; GRANT USAGE ON SCHEMA {} TO {}; ALTER TABLE {target} OWNER TO {}; ALTER TABLE {target} ENABLE ROW LEVEL SECURITY; ALTER TABLE {target} FORCE ROW LEVEL SECURITY; CREATE POLICY visible ON {target} FOR SELECT USING(id IN(1,3)); CREATE POLICY editable ON {target} FOR UPDATE USING(id IN(1,3)) WITH CHECK(id IN(1,3)); CREATE POLICY insertable ON {target} FOR INSERT WITH CHECK(id=3)", q(role),q(schema),q(role),q(role))).await?;
            // This is a real role login, including the driver's auxiliary connections.
            let app = App::connect(role, ROLE_PASSWORD, SafeMode::Silent).await?;
            let page =
                timeout(DEADLINE, app.studio.browse(&app.session.id, &table, None)).await??;
            assert_eq!(
                app.rows(&page.result)?,
                vec![vec![Some("1".into()), Some("before".into())]]
            );
            let preview = app
                .studio
                .prepare_table_changes(&app.session.id, update(&page.result))?;
            let events = app.execute(&preview.plan).await?;
            assert_eq!(status(&events), Some(RunStatus::Completed), "{events:?}");
            app.studio.release_result(&page.result.id);
            let page =
                timeout(DEADLINE, app.studio.browse(&app.session.id, &table, None)).await??;
            let preview = app
                .studio
                .prepare_table_changes(&app.session.id, insert(&page.result, 3))?;
            let events = app.execute(&preview.plan).await?;
            assert_eq!(status(&events), Some(RunStatus::Completed), "{events:?}");
            app.studio.release_result(&page.result.id);
            let page =
                timeout(DEADLINE, app.studio.browse(&app.session.id, &table, None)).await??;
            assert_eq!(page.result.retained_rows, 2);
            let preview = app
                .studio
                .prepare_table_changes(&app.session.id, insert(&page.result, 4))?;
            let events = app.execute(&preview.plan).await?;
            assert_eq!(status(&events), Some(RunStatus::Failed), "{events:?}");
            assert!(events.iter().any(|event| matches!(event, QueryEvent::Error { error, .. } if error.code == ErrorCode::PermissionDenied)), "{events:?}");
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, QueryEvent::SessionClosed { .. }))
            );
            app.studio.release_result(&page.result.id);
            assert_eq!(
                stored_rows(&observer.client, &target).await?,
                vec![
                    (1, "approved".into()),
                    (2, "hidden".into()),
                    (3, "inserted".into())
                ]
            );
            let usable = app.query("SELECT 1").await?;
            assert_eq!(status(&usable), Some(RunStatus::Completed));
        }
        Case::RawSideEffect => {
            let marker = format!("{}.marker", q(schema));
            let function = format!("{}.mark", q(schema));
            sql(&observer.client, &format!("DELETE FROM {target} WHERE id=2; CREATE TABLE {marker}(id integer NOT NULL); CREATE FUNCTION {function}(value integer) RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ BEGIN INSERT INTO {marker} VALUES(value); RETURN true; END $$")).await?;
            let app = App::connect("joinery", "joinery-local-only", SafeMode::Confirm).await?;
            let query = BrowseQuery {
                where_clause: Some(format!("{function}(id)")),
                ..Default::default()
            };
            assert_eq!(
                timeout(
                    DEADLINE,
                    app.studio
                        .browse_with(&app.session.id, &table, None, &query)
                )
                .await?
                .unwrap_err()
                .code,
                ErrorCode::ApprovalRequired
            );
            let plan = timeout(
                DEADLINE,
                app.studio.prepare_browse(&app.session.id, &table, &query),
            )
            .await??
            .ok_or("raw WHERE did not require review")?;
            let before: i64 = timeout(
                DEADLINE,
                observer
                    .client
                    .query_one(&format!("SELECT count(*) FROM {marker}"), &[]),
            )
            .await??
            .get(0);
            assert_eq!(before, 0, "preparation must not execute the function");
            let approval = BrowseApproval {
                plan_id: plan.id.clone(),
                approval: app.studio.authorize(&plan.id, None, None)?,
            };
            let page = timeout(
                DEADLINE,
                app.studio.browse_with_approval(
                    &app.session.id,
                    &table,
                    None,
                    &query,
                    Some(&approval),
                ),
            )
            .await??;
            assert_eq!(
                app.rows(&page.result)?,
                vec![vec![Some("1".into()), Some("before".into())]]
            );
            assert!(!page.has_more);
            app.studio.release_result(&page.result.id);
            let marked: Vec<i32> = timeout(
                DEADLINE,
                observer
                    .client
                    .query(&format!("SELECT id FROM {marker}"), &[]),
            )
            .await??
            .into_iter()
            .map(|row| row.get(0))
            .collect();
            assert_eq!(
                marked,
                vec![1],
                "approved raw browse must commit its successful function effects"
            );
            assert_eq!(
                timeout(
                    DEADLINE,
                    app.studio.browse_with_approval(
                        &app.session.id,
                        &table,
                        None,
                        &query,
                        Some(&approval)
                    )
                )
                .await?
                .unwrap_err()
                .code,
                ErrorCode::ApprovalExpired
            );
            let count: i64 = timeout(
                DEADLINE,
                observer
                    .client
                    .query_one(&format!("SELECT count(*) FROM {marker}"), &[]),
            )
            .await??
            .get(0);
            assert_eq!(
                count, 1,
                "rejected approval replay must not call the function"
            );
        }
        Case::RawLateError => late_browse_error(&observer.client, schema, role, &table).await?,
    }
    Ok(())
}

async fn late_browse_error(observer: &Client, schema: &str, role: &str, table: &TableRef) -> R<()> {
    const NOTE_BYTES: usize = 900 * 1024;
    const ERROR: &str = "owned browse error after three wide rows";
    let target = format!("{}.{}", q(schema), q(&table.name));
    let marker = format!("{}.marker", q(schema));
    let calls = format!("{}.calls", q(schema));
    let function = format!("{}.mark_then_fail", q(schema));
    sql(observer, &format!(
        "TRUNCATE {target}; INSERT INTO {target} SELECT id,repeat('x',{NOTE_BYTES}) FROM generate_series(1,4) AS id; \
         CREATE TABLE {marker}(id integer NOT NULL); CREATE SEQUENCE {calls}; \
         CREATE FUNCTION {function}(value integer) RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ \
         BEGIN PERFORM nextval('{calls}'::regclass); INSERT INTO {marker} VALUES(value); \
         IF value=4 THEN PERFORM pg_sleep(0.1); RAISE EXCEPTION '{ERROR}' USING ERRCODE='P0001'; END IF; RETURN true; END $$; \
         CREATE ROLE {} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS PASSWORD '{ROLE_PASSWORD}'; \
         ALTER ROLE {} SET enable_seqscan=off; ALTER ROLE {} SET enable_bitmapscan=off; \
         ALTER ROLE {} SET enable_sort=off; ALTER ROLE {} SET jit=off; \
         GRANT USAGE ON SCHEMA {} TO {}; GRANT SELECT ON {target} TO {}; \
         GRANT INSERT ON {marker} TO {}; GRANT USAGE,SELECT ON SEQUENCE {calls} TO {}; \
         GRANT EXECUTE ON FUNCTION {function}(integer) TO {}",
        q(role), q(role), q(role), q(role), q(role), q(schema), q(role), q(role), q(role), q(role), q(role)
    )).await?;

    let direct = connect(role, ROLE_PASSWORD).await?;
    // Match browse_target's hidden origin and text projection. The role's login
    // defaults also apply to every auxiliary connection opened by the app.
    let statement = format!(
        "SELECT {target}.tableoid AS __joinery_origin, {target}.\"id\"::text AS __joinery_c0,{target}.\"note\"::text AS __joinery_c1 FROM {target} WHERE (\n{function}(id)\n) ORDER BY id\n LIMIT 501"
    );
    let explain = timeout(
        DEADLINE,
        direct.client.query(&format!("EXPLAIN {statement}"), &[]),
    )
    .await??
    .iter()
    .map(|row| row.get::<_, &str>(0))
    .collect::<Vec<_>>()
    .join("\n");
    assert!(explain.contains("Index Scan"), "{explain}");
    assert!(!explain.contains("Sort"), "{explain}");
    timeout(DEADLINE, async {
        let mut stream = Box::pin(
            direct
                .client
                .query_raw(
                    statement.as_str(),
                    std::iter::empty::<&(dyn tokio_postgres::types::ToSql + Sync)>(),
                )
                .await?,
        );
        let mut delivered = Vec::new();
        loop {
            match stream.try_next().await {
                Ok(Some(row)) => {
                    delivered.push(row.get::<_, &str>(1).parse::<i32>()?);
                    assert_eq!(row.get::<_, &str>(2).len(), NOTE_BYTES);
                }
                Ok(None) => return Err("wide-row control query unexpectedly completed".into()),
                Err(error) => {
                    assert_eq!(
                        delivered,
                        vec![1, 2, 3],
                        "error arrived before the retention cutoff"
                    );
                    assert_eq!(
                        error.code(),
                        Some(&tokio_postgres::error::SqlState::RAISE_EXCEPTION)
                    );
                    assert_eq!(
                        error.as_db_error().map(|error| error.message()),
                        Some(ERROR)
                    );
                    return Ok::<(), Box<dyn Error + Send + Sync>>(());
                }
            }
        }
    })
    .await??;
    // The sequence is intentionally nontransactional: reset after the independent
    // wire-order witness so exactly four calls below also detects an app replay.
    sql(observer, &format!("ALTER SEQUENCE {calls} RESTART WITH 1")).await?;
    let app = App::connect(role, ROLE_PASSWORD, SafeMode::Confirm).await?;
    let query = BrowseQuery {
        where_clause: Some(format!("{function}(id)")),
        order_clause: Some("id".into()),
        ..Default::default()
    };
    let plan = timeout(
        DEADLINE,
        app.studio.prepare_browse(&app.session.id, table, &query),
    )
    .await??
    .ok_or("late-error raw WHERE did not require review")?;
    let called: bool = timeout(
        DEADLINE,
        observer.query_one(&format!("SELECT is_called FROM {calls}"), &[]),
    )
    .await??
    .get(0);
    assert!(!called, "preparation must not call the raw expression");
    let approval = BrowseApproval {
        plan_id: plan.id.clone(),
        approval: app.studio.authorize(&plan.id, None, None)?,
    };
    let error = timeout(
        DEADLINE,
        app.studio
            .browse_with_approval(&app.session.id, table, None, &query, Some(&approval)),
    )
    .await?
    .err()
    .ok_or("browse discarded an error after the 2 MiB retention cutoff")?;
    assert_eq!(error.code, ErrorCode::Database);
    assert_eq!(error.message, ERROR);
    assert_eq!(
        timeout(
            DEADLINE,
            app.studio
                .browse_with_approval(&app.session.id, table, None, &query, Some(&approval),)
        )
        .await?
        .unwrap_err()
        .code,
        ErrorCode::ApprovalExpired
    );
    let marked: i64 = timeout(
        DEADLINE,
        observer.query_one(&format!("SELECT count(*) FROM {marker}"), &[]),
    )
    .await??
    .get(0);
    let call_count: i64 = timeout(
        DEADLINE,
        observer.query_one(&format!("SELECT last_value FROM {calls}"), &[]),
    )
    .await??
    .get(0);
    assert_eq!(marked, 0, "the failed browse must roll back every marker");
    assert_eq!(
        call_count, 4,
        "the failed or consumed browse must not replay"
    );
    let unchanged: i64 = timeout(
        DEADLINE,
        observer.query_one(
            &format!("SELECT count(*) FROM {target} WHERE note=repeat('x',{NOTE_BYTES})"),
            &[],
        ),
    )
    .await??
    .get(0);
    assert_eq!(unchanged, 4);
    assert_eq!(
        status(&app.query("SELECT 1").await?),
        Some(RunStatus::Completed)
    );
    Ok(())
}

async fn qualify(case: Case) -> R<()> {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let schema = format!("identity_permissions_{suffix}");
    let role = format!("identity_role_{suffix}");
    // All names are registered before setup. Unwinding drops each Studio and
    // owned connection before a fresh observer attempts every cleanup operation.
    let work = std::panic::AssertUnwindSafe(exercise(case, &schema, &role))
        .catch_unwind()
        .await;
    let cleanup: R<()> = async {
        let observer = connect("joinery", "joinery-local-only").await?;
        let mut errors = Vec::new();
        for statement in [format!("DROP SCHEMA IF EXISTS {} CASCADE", q(&schema)), format!("DROP ROLE IF EXISTS {}", q(&role))] {
            if let Err(error) = sql(&observer.client, &statement).await { errors.push(error.to_string()); }
        }
        let remaining: i64 = timeout(DEADLINE, observer.client.query_one("SELECT (SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname=$1)+(SELECT count(*) FROM pg_catalog.pg_roles WHERE rolname=$2)", &[&schema,&role])).await??.get(0);
        if remaining != 0 { errors.push(format!("{remaining} owned resources remain")); }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ").into()) }
    }.await;
    match (work, cleanup) {
        (Ok(Ok(())), Ok(())) => Ok(()),
        (work, cleanup) => {
            let work = match work {
                Ok(Ok(())) => "passed".into(),
                Ok(Err(error)) => error.to_string(),
                Err(_) => "panicked".into(),
            };
            let cleanup = match cleanup {
                Ok(()) => "verified".into(),
                Err(error) => error.to_string(),
            };
            Err(format!(
                "qualification: {work}; cleanup: {cleanup}; owned schema={schema} role={role}"
            )
            .into())
        }
    }
}

#[tokio::test]
#[ignore = "explicit opt-in: disposable PostgreSQL 127.0.0.1:55432; creates UUID-owned LOGIN role"]
async fn column_select_without_system_column_privilege_remains_readable_and_read_only() -> R<()> {
    qualify(Case::ColumnGrants).await
}
#[tokio::test]
#[ignore = "explicit opt-in: disposable PostgreSQL 127.0.0.1:55432; creates UUID-owned LOGIN role"]
async fn forced_owner_rls_preserves_visibility_and_generated_write_policies() -> R<()> {
    qualify(Case::OwnerRls).await
}
#[tokio::test]
#[ignore = "explicit opt-in: disposable PostgreSQL 127.0.0.1:55432; creates owned side-effect function"]
async fn approved_raw_browse_commits_function_effects_once() -> R<()> {
    qualify(Case::RawSideEffect).await
}
#[tokio::test]
#[ignore = "explicit opt-in: disposable PostgreSQL 127.0.0.1:55432; creates owned role and late-error function"]
async fn approved_raw_browse_propagates_errors_after_the_retention_budget() -> R<()> {
    qualify(Case::RawLateError).await
}
