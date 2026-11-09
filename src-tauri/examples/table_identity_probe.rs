//! Opt-in diagnostic for the repository's disposable database fixtures.
//! A successful process in report-only mode means observations were collected,
//! not that retained edits are safe. --require-safe enforces that distinction.
use joinery_core::{
    domain::*, execution::Studio, file_capability::SandboxFileDialog, secrets::MemorySecrets,
};
use serde::Serialize;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

type ProbeResult<T> = Result<T, String>;

#[derive(Clone, Copy)]
struct Fixture {
    name: &'static str,
    engine: EngineKind,
    port: u16,
    schema: &'static str,
}

const FIXTURES: [Fixture; 3] = [
    Fixture {
        name: "postgres",
        engine: EngineKind::Postgres,
        port: 55432,
        schema: "public",
    },
    Fixture {
        name: "mysql",
        engine: EngineKind::Mysql,
        port: 53306,
        schema: "joinery",
    },
    Fixture {
        name: "mariadb",
        engine: EngineKind::Mysql,
        port: 53307,
        schema: "joinery",
    },
];

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Browse,
    Query,
    Csv,
}
#[derive(Clone, Copy, PartialEq)]
enum Mutation {
    Update,
    Insert,
    Delete,
}
#[derive(Clone, Copy, PartialEq)]
enum Timing {
    BeforePrepare,
    AfterApproval,
    BetweenBatches,
}
#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    source: Source,
    mutation: Mutation,
    timing: Timing,
    empty: bool,
}

const CASES: [Case; 9] = [
    Case {
        name: "browse_update_before_prepare",
        source: Source::Browse,
        mutation: Mutation::Update,
        timing: Timing::BeforePrepare,
        empty: false,
    },
    Case {
        name: "query_update_before_prepare",
        source: Source::Query,
        mutation: Mutation::Update,
        timing: Timing::BeforePrepare,
        empty: false,
    },
    Case {
        name: "query_update_after_approval",
        source: Source::Query,
        mutation: Mutation::Update,
        timing: Timing::AfterApproval,
        empty: false,
    },
    Case {
        name: "browse_insert_after_approval",
        source: Source::Browse,
        mutation: Mutation::Insert,
        timing: Timing::AfterApproval,
        empty: false,
    },
    Case {
        name: "browse_delete_after_approval",
        source: Source::Browse,
        mutation: Mutation::Delete,
        timing: Timing::AfterApproval,
        empty: false,
    },
    Case {
        name: "empty_browse_insert_after_approval",
        source: Source::Browse,
        mutation: Mutation::Insert,
        timing: Timing::AfterApproval,
        empty: true,
    },
    Case {
        name: "csv_after_preview",
        source: Source::Csv,
        mutation: Mutation::Insert,
        timing: Timing::BeforePrepare,
        empty: false,
    },
    Case {
        name: "csv_after_approval",
        source: Source::Csv,
        mutation: Mutation::Insert,
        timing: Timing::AfterApproval,
        empty: false,
    },
    Case {
        name: "csv_between_batches",
        source: Source::Csv,
        mutation: Mutation::Insert,
        timing: Timing::BetweenBatches,
        empty: false,
    },
];

struct Harness {
    _root: tempfile::TempDir,
    studio: Arc<Studio>,
    dialog: Arc<SandboxFileDialog>,
    session: SessionInfo,
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.studio.shutdown();
    }
}

fn app<T>(result: AppResult<T>) -> ProbeResult<T> {
    result.map_err(|e| format!("{:?}: {}", e.code, e.message))
}

impl Harness {
    async fn connect(fixture: Fixture) -> ProbeResult<Self> {
        let root = tempfile::tempdir().map_err(|e| e.to_string())?;
        let dialog = app(SandboxFileDialog::new(&root.path().join("imports")))?;
        let studio = app(Studio::with_secrets_and_dialog(
            root.path(),
            Arc::new(MemorySecrets::default()),
            dialog.clone(),
        ))?;
        let profile = app(studio.save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "Disposable identity probe".into(),
                group_id: "local".into(),
                engine: fixture.engine,
                host: "127.0.0.1".into(),
                port: fixture.port,
                database: "joinery".into(),
                username: "joinery".into(),
                environment: Environment::Local,
                safe_mode: SafeMode::Confirm,
                tls_mode: TlsMode::Disable,
                ca_cert_path: None,
                ssh: None,
                remember_password: false,
                store_query_text: false,
                store_drafts: false,
                snowflake: None,
            },
            None,
        ))?;
        let session = match studio
            .connect(
                &profile.id,
                ConnectionSecrets {
                    password: Some("joinery-local-only".into()),
                    ..Default::default()
                },
            )
            .await
        {
            Ok(session) => session,
            Err(error) => {
                studio.shutdown();
                return app(Err(error));
            }
        };
        Ok(Self {
            _root: root,
            studio,
            dialog,
            session,
        })
    }

    fn approve(&self, plan: &PreparedPlan) -> ProbeResult<Option<String>> {
        if plan.requires_approval {
            app(self
                .studio
                .authorize(&plan.id, None, plan.confirmation_text.as_deref()))
            .map(Some)
        } else {
            Ok(None)
        }
    }

    async fn execute(
        &self,
        plan: &PreparedPlan,
        token: Option<&str>,
    ) -> ProbeResult<Vec<QueryEvent>> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let run_id =
            app(self
                .studio
                .execute(&plan.id, token, Arc::new(move |e| tx.send(e).is_ok())))?;
        let collected = tokio::time::timeout(Duration::from_secs(15), async {
            let mut events = Vec::new();
            while let Some(event) = rx.recv().await {
                let finished = matches!(event, QueryEvent::Finished { .. });
                events.push(event);
                if finished {
                    return Ok(events);
                }
            }
            Err("event stream ended before Finished".into())
        })
        .await;
        match collected {
            Ok(result) => result,
            Err(_) => {
                let _ =
                    tokio::time::timeout(Duration::from_secs(3), self.studio.cancel(&run_id)).await;
                Err("query exceeded the 15-second diagnostic deadline".into())
            }
        }
    }

    async fn sql(&self, sql: &str) -> ProbeResult<Vec<QueryEvent>> {
        let plan = app(self.studio.prepare_execution(&self.session.id, sql))?;
        let token = self.approve(&plan)?;
        let events = self.execute(&plan, token.as_deref()).await?;
        if status(&events) != Some(RunStatus::Completed) {
            return Err(format!(
                "fixture SQL did not complete: {:?}",
                errors(&events)
            ));
        }
        Ok(events)
    }

    async fn rows(&self, qualified: &str) -> ProbeResult<Vec<TextRow>> {
        let events = self
            .sql(&format!("SELECT id, note FROM {qualified} ORDER BY id"))
            .await?;
        let result = result(&events)?;
        let rows = (0..result.retained_rows)
            .map(|i| app(self.studio.results.row(&result.id, i)))
            .collect();
        self.studio.release_result(&result.id);
        rows
    }
}

fn status(events: &[QueryEvent]) -> Option<RunStatus> {
    events.iter().find_map(|e| {
        if let QueryEvent::Finished { status, .. } = e {
            Some(*status)
        } else {
            None
        }
    })
}
fn errors(events: &[QueryEvent]) -> Vec<ErrorCode> {
    events
        .iter()
        .filter_map(|e| {
            if let QueryEvent::Error { error, .. } = e {
                Some(error.code)
            } else {
                None
            }
        })
        .collect()
}
fn result(events: &[QueryEvent]) -> ProbeResult<ResultInfo> {
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
        .ok_or_else(|| "missing result".into())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    engine: &'static str,
    case: &'static str,
    status: String,
    errors: Vec<ErrorCode>,
    replacement_rows: Vec<TextRow>,
    target_preserved: bool,
    rejection_verified: bool,
    approval_consumed: bool,
    fresh_control_succeeded: bool,
    cleanup_verified: bool,
}
impl Observation {
    fn safe(&self) -> bool {
        self.target_preserved
            && self.rejection_verified
            && self.approval_consumed
            && self.fresh_control_succeeded
            && self.cleanup_verified
    }
}

async fn create(h: &Harness, qualified: &str, fixture: Fixture, empty: bool) -> ProbeResult<()> {
    let engine = if fixture.engine == EngineKind::Mysql {
        " ENGINE=InnoDB"
    } else {
        ""
    };
    h.sql(&format!("CREATE TABLE {qualified} (id integer PRIMARY KEY, note varchar(64) NOT NULL DEFAULT 'before'){engine}")).await?;
    if !empty {
        h.sql(&format!("INSERT INTO {qualified} VALUES (1, 'before')"))
            .await?;
    }
    Ok(())
}

async fn replace(h: &Harness, qualified: &str, fixture: Fixture, empty: bool) -> ProbeResult<()> {
    h.sql(&format!("DROP TABLE {qualified}")).await?;
    create(h, qualified, fixture, empty).await
}

fn changes(result: &ResultInfo, mutation: Mutation) -> TableChanges {
    TableChanges {
        result_id: Some(result.id.clone()),
        updates: if mutation == Mutation::Update {
            vec![ResultChanges {
                result_id: result.id.clone(),
                changes: vec![CellChange {
                    row: 0,
                    column: "c1".into(),
                    value: Some("approved".into()),
                }],
            }]
        } else {
            vec![]
        },
        deletes: if mutation == Mutation::Delete {
            vec![ResultDeletes {
                result_id: result.id.clone(),
                rows: vec![0],
            }]
        } else {
            vec![]
        },
        inserts: if mutation == Mutation::Insert {
            vec![RowInsert {
                values: vec![InsertValue {
                    column: "c0".into(),
                    value: Some("2".into()),
                }],
            }]
        } else {
            vec![]
        },
    }
}

async fn observe(
    actor: &Harness,
    observer: &Harness,
    fixture: Fixture,
    case: Case,
    table: &TableRef,
) -> ProbeResult<Observation> {
    let qualified = format!("{}.{}", table.schema, table.name);
    let mappings = vec![
        CsvColumnMapping {
            csv_index: 0,
            target_column: Some("id".into()),
        },
        CsvColumnMapping {
            csv_index: 1,
            target_column: Some("note".into()),
        },
    ];
    let mut source_token = None;
    let selected = match case.source {
        Source::Browse => {
            Some(app(actor.studio.browse(&actor.session.id, table, None).await)?.result)
        }
        Source::Query => Some(result(
            &actor
                .sql(&format!("SELECT id, note FROM {qualified}"))
                .await?,
        )?),
        Source::Csv => {
            let count = if case.timing == Timing::BetweenBatches {
                501
            } else {
                1
            };
            let mut csv = String::from("id,note\n");
            for id in 2..count + 2 {
                csv.push_str(&format!("{id},approved\n"));
            }
            std::fs::write(actor.dialog.root().join("probe.csv"), csv)
                .map_err(|e| e.to_string())?;
            app(actor.dialog.queue_open(Some("probe.csv")))?;
            let grant =
                app(actor.studio.pick_import_source().await)?.ok_or("source was not selected")?;
            let preview = app(actor
                .studio
                .preview_csv_import(
                    &actor.session.id,
                    &grant.token,
                    table,
                    CsvImportOptions::default(),
                    mappings.clone(),
                )
                .await)?;
            if !preview.can_prepare {
                return Err("synthetic CSV preview was not eligible".into());
            }
            source_token = Some(grant.token);
            None
        }
    };
    let mut batch_index = 0;
    if case.timing == Timing::BetweenBatches {
        let first = app(actor
            .studio
            .prepare_csv_import_batch(
                &actor.session.id,
                source_token.as_deref().ok_or("missing source")?,
                table,
                CsvImportOptions::default(),
                mappings.clone(),
                0,
            )
            .await)?;
        let token = actor.approve(&first.plan)?;
        let events = actor.execute(&first.plan, token.as_deref()).await?;
        if status(&events) != Some(RunStatus::Completed)
            || observer.rows(&qualified).await?.len() != 501
        {
            return Err("first CSV batch did not commit exactly 500 added rows".into());
        }
        batch_index = 1;
    }
    if case.timing != Timing::AfterApproval {
        replace(observer, &qualified, fixture, case.empty).await?;
    }
    let baseline = if case.empty {
        vec![]
    } else {
        vec![vec![Some("1".into()), Some("before".into())]]
    };
    if observer.rows(&qualified).await? != baseline {
        return Err("replacement baseline mismatch".into());
    }
    let prepared = if let Some(token) = &source_token {
        actor
            .studio
            .prepare_csv_import_batch(
                &actor.session.id,
                token,
                table,
                CsvImportOptions::default(),
                mappings,
                batch_index,
            )
            .await
    } else {
        actor.studio.prepare_table_changes(
            &actor.session.id,
            changes(
                selected.as_ref().ok_or("missing original result")?,
                case.mutation,
            ),
        )
    };
    let (outcome, codes, consumed) = match prepared {
        Ok(preview) => {
            let token = actor.approve(&preview.plan)?;
            if case.timing == Timing::AfterApproval {
                replace(observer, &qualified, fixture, case.empty).await?;
            }
            let events = actor.execute(&preview.plan, token.as_deref()).await?;
            let consumed = actor
                .studio
                .execute(&preview.plan.id, token.as_deref(), Arc::new(|_| true))
                .is_err_and(|e| e.code == ErrorCode::ApprovalExpired);
            (
                format!("{:?}", status(&events).ok_or("missing terminal status")?),
                errors(&events),
                consumed,
            )
        }
        Err(error) => ("PreparationRejected".into(), vec![error.code], true),
    };
    let actual = observer.rows(&qualified).await?;
    let rejection_verified = outcome != "Completed"
        && codes
            .iter()
            .any(|c| matches!(c, ErrorCode::Conflict | ErrorCode::PolicyBlocked));
    // A new result must remain writable; disabling every edit is not success.
    let fresh = app(actor.studio.browse(&actor.session.id, table, None).await)?.result;
    let control = app(actor.studio.prepare_table_changes(
        &actor.session.id,
        TableChanges {
            result_id: Some(fresh.id),
            updates: vec![],
            deletes: vec![],
            inserts: vec![RowInsert {
                values: vec![
                    InsertValue {
                        column: "c0".into(),
                        value: Some("99".into()),
                    },
                    InsertValue {
                        column: "c1".into(),
                        value: Some("fresh control".into()),
                    },
                ],
            }],
        },
    ))?;
    let token = actor.approve(&control.plan)?;
    let control_events = actor.execute(&control.plan, token.as_deref()).await?;
    let fresh_control_succeeded = status(&control_events) == Some(RunStatus::Completed)
        && observer
            .rows(&qualified)
            .await?
            .iter()
            .any(|r| r == &vec![Some("99".into()), Some("fresh control".into())]);
    Ok(Observation {
        engine: fixture.name,
        case: case.name,
        status: outcome,
        errors: codes,
        target_preserved: actual == baseline,
        replacement_rows: actual,
        rejection_verified,
        approval_consumed: consumed,
        fresh_control_succeeded,
        cleanup_verified: false,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FixtureContext {
    engine: &'static str,
    loopback_port: u16,
    server_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    safety_passed: bool,
    fixtures: Vec<FixtureContext>,
    observations: Vec<Observation>,
    operational_errors: Vec<String>,
}

#[tokio::main]
async fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut engine = "all";
    let mut run = false;
    let mut require_safe = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--run-disposable-fixtures" if !run => run = true,
            "--require-safe" if !require_safe => require_safe = true,
            "--engine" if index + 1 < args.len() => {
                index += 1;
                engine = &args[index];
            }
            _ => {
                eprintln!(
                    "usage: table_identity_probe --run-disposable-fixtures [--engine all|postgres|mysql|mariadb] [--require-safe]"
                );
                std::process::exit(1);
            }
        }
        index += 1;
    }
    if !run || (engine != "all" && !FIXTURES.iter().any(|f| f.name == engine)) {
        eprintln!(
            "Explicit --run-disposable-fixtures and a supported fixture engine are required. No connection was opened."
        );
        std::process::exit(1);
    }
    let report = collect_report(engine).await;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serializable report")
    );
    if !report.operational_errors.is_empty() {
        std::process::exit(1);
    }
    if require_safe && !report.safety_passed {
        std::process::exit(2);
    }
}

async fn collect_report(engine: &str) -> Report {
    let mut report = Report {
        safety_passed: false,
        fixtures: vec![],
        observations: vec![],
        operational_errors: vec![],
    };
    for fixture in FIXTURES
        .into_iter()
        .filter(|f| engine == "all" || f.name == engine)
    {
        let observer = match Harness::connect(fixture).await {
            Ok(harness) => harness,
            Err(error) => {
                report
                    .operational_errors
                    .push(format!("{}: {error}", fixture.name));
                continue;
            }
        };
        report.fixtures.push(FixtureContext {
            engine: fixture.name,
            loopback_port: fixture.port,
            server_version: observer.session.server_version.clone(),
        });
        for case in CASES {
            let actor = match Harness::connect(fixture).await {
                Ok(harness) => harness,
                Err(error) => {
                    report
                        .operational_errors
                        .push(format!("{} {}: {error}", fixture.name, case.name));
                    continue;
                }
            };
            let table = TableRef {
                schema: fixture.schema.into(),
                name: format!("identity_probe_{}", new_id().replace('-', "_")),
            };
            let qualified = format!("{}.{}", table.schema, table.name);
            let observed = match create(&observer, &qualified, fixture, case.empty).await {
                Ok(()) => observe(&actor, &observer, fixture, case, &table).await,
                Err(error) => Err(error),
            };
            // Release any actor-held guards before cleanup, including on failure.
            drop(actor);
            let cleanup = observer
                .sql(&format!("DROP TABLE IF EXISTS {qualified}"))
                .await;
            let absent_sql = if fixture.engine == EngineKind::Postgres {
                format!(
                    "SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='{}' AND c.relname='{}'",
                    table.schema, table.name
                )
            } else {
                format!(
                    "SELECT count(*) FROM information_schema.tables WHERE table_schema='{}' AND table_name='{}'",
                    table.schema, table.name
                )
            };
            let absent = match observer.sql(&absent_sql).await {
                Ok(events) => result(&events)
                    .and_then(|r| {
                        let row = app(observer.studio.results.row(&r.id, 0));
                        observer.studio.release_result(&r.id);
                        row
                    })
                    .is_ok_and(|r| r == vec![Some("0".into())]),
                Err(_) => false,
            };
            match observed {
                Ok(mut observation) => {
                    observation.cleanup_verified = cleanup.is_ok() && absent;
                    report.observations.push(observation);
                }
                Err(error) => report.operational_errors.push(format!(
                    "{} {} ({qualified}): {error}",
                    fixture.name, case.name
                )),
            }
            if cleanup.is_err() || !absent {
                report
                    .operational_errors
                    .push(format!("cleanup not verified for owned table {qualified}"));
            }
        }
    }
    report.safety_passed = report.operational_errors.is_empty()
        && !report.observations.is_empty()
        && report.observations.iter().all(Observation::safe);
    report
}

#[cfg(test)]
mod regression {
    #[tokio::test]
    #[ignore = "explicit opt-in: actual disposable PostgreSQL fixture 55432"]
    async fn postgres_retained_targets_reject_replacement_and_allow_fresh_edits() {
        let report = super::collect_report("postgres").await;
        assert!(
            report.safety_passed,
            "{}",
            serde_json::to_string_pretty(&report).unwrap()
        );
        assert_eq!(report.observations.len(), 9);
    }
}
