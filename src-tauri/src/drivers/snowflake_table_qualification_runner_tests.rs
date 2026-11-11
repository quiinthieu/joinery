//! Closed synthetic-only composition of the actual Studio/helper routes.
//! No account SDK, profile lookup, external execution entry or approval exists.
use super::{
    Operation, Plan, Readback, SCHEMA, SchemaCensus, StatementId,
    evidence::PhaseEvidence,
    journal::FILE_NAME,
    policy::{OperationCompletion, Policy},
};
use crate::{
    domain::*,
    drivers::{
        snowflake_qualification::QualificationAttachment,
        snowflake_qualification_transport::{
            TransportAttachment, TransportLimits, TransportPolicy,
        },
    },
    execution::{EventHandler, Studio},
    file_capability::DisabledFileDialog,
    relation_inspection::{BrowseCatalog, InspectRelationRequest, RelationInspection},
    secrets::MemorySecrets,
    snowflake_runtime::studio_fixture::Fixture,
};
use arrow_array::{Array, StringArray};
use arrow_ipc::reader::StreamReader;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{fs, io::Cursor, os::unix::fs::PermissionsExt, sync::Arc, time::Duration};

const EVENT_LIMIT: usize = 32;
const READ_BYTES: usize = 64 * 1024;

fn invalid() -> AppError {
    AppError::invalid("Synthetic qualification runner result is incomplete")
}

enum Output {
    Startup(SessionInfo),
    Preflight(SnowflakeDdlPreflightResponse),
    Ddl(SnowflakeDdlResult),
    Reconcile(SessionContextOptions, BrowseCatalog),
    Catalog(BrowseCatalog),
    Inspect(RelationInspection),
    Read(ReadCapture),
    Boundary(TransactionControlResult),
    Dml(SnowflakeDmlResult),
    Options(SessionContextOptions),
}

impl Output {
    fn evidence(&self) -> PhaseEvidence<'_> {
        match self {
            Self::Startup(v) => PhaseEvidence::Startup(v),
            Self::Preflight(v) => PhaseEvidence::Preflight(v),
            Self::Ddl(v) => PhaseEvidence::Ddl(v),
            Self::Reconcile(options, catalog) => PhaseEvidence::Reconcile { options, catalog },
            Self::Catalog(v) => PhaseEvidence::Catalog(v),
            Self::Inspect(v) => PhaseEvidence::Inspect(v),
            Self::Read(v) => PhaseEvidence::Read {
                events: &v.events,
                result: &v.result,
                rows: &v.rows,
            },
            Self::Boundary(v) => PhaseEvidence::Boundary(v),
            Self::Dml(v) => PhaseEvidence::Dml(v),
            Self::Options(v) => PhaseEvidence::Options(v),
        }
    }
}

struct ReadCapture {
    events: Vec<QueryEvent>,
    result: ResultInfo,
    rows: Vec<TextRow>,
}

struct CallError {
    error: AppError,
    events: Vec<QueryEvent>,
}

impl From<AppError> for CallError {
    fn from(error: AppError) -> Self {
        Self {
            error,
            events: Vec::new(),
        }
    }
}

#[derive(Default)]
struct Collector {
    events: Vec<QueryEvent>,
    bytes: usize,
    error: Option<AppError>,
}

impl Collector {
    fn complete_snapshot(&self) -> (Vec<QueryEvent>, Option<AppError>) {
        assert!(
            self.error.is_none() && matches!(self.events.last(), Some(QueryEvent::Finished { .. })),
            "incomplete original read event capture (bounded collector overflow or missing terminal)"
        );
        (self.events.clone(), self.error.clone())
    }

    fn push(&mut self, event: QueryEvent) -> bool {
        if self.error.is_some() {
            return false;
        }
        let bytes = match serde_json::to_vec(&event) {
            Ok(v) => v.len(),
            Err(_) => {
                self.error = Some(invalid());
                return false;
            }
        };
        if self.events.len() >= EVENT_LIMIT || self.bytes.saturating_add(bytes) > READ_BYTES {
            self.error = Some(invalid());
            return false;
        }
        self.bytes += bytes;
        self.events.push(event);
        true
    }
}

fn approve(studio: &Studio, plan: &PreparedPlan) -> AppResult<Option<String>> {
    if plan.requires_passphrase {
        return Err(invalid());
    }
    if plan.requires_approval {
        studio
            .authorize(&plan.id, None, plan.confirmation_text.as_deref())
            .map(Some)
    } else {
        Ok(None)
    }
}

async fn read(
    studio: &Arc<Studio>,
    session: &str,
    sql: &str,
    cancel_on_stream: bool,
) -> Result<ReadCapture, CallError> {
    let plan = studio.prepare_execution(session, sql)?;
    if plan.reviewed_snowflake_dml
        || plan.reviewed_snowflake_ddl
        || plan.statements.len() != 1
        || !plan.bound_parameters.is_empty()
    {
        return Err(invalid().into());
    }
    let approval = approve(studio, &plan)?;
    let capture = Arc::new(Mutex::new(Collector::default()));
    let (progress, mut completion) = tokio::sync::watch::channel((false, false));
    let handler: EventHandler = Arc::new({
        let capture = capture.clone();
        move |event| {
            let finished = matches!(event, QueryEvent::Finished { .. });
            let streaming = matches!(&event, QueryEvent::Result { result } if result.status == RetentionStatus::Streaming);
            let accepted = capture.lock().push(event);
            progress.send_modify(|value| {
                value.0 |= streaming;
                value.1 |= finished;
            });
            accepted
        }
    });
    let run = studio.execute(&plan.id, approval.as_deref(), handler)?;
    let settled = tokio::time::timeout(Duration::from_secs(15), async {
        while {
            let (streaming, finished) = *completion.borrow_and_update();
            !(finished || cancel_on_stream && streaming)
        } {
            completion.changed().await.map_err(|_| invalid())?;
        }
        Ok::<_, AppError>(())
    })
    .await;
    let cancelled = cancel_on_stream || !matches!(settled, Ok(Ok(())));
    if cancelled {
        let cancel_result = studio.cancel(&run).await;
        // Cancel requests do not join run_plan. Retain the collector until its
        // original terminal event, including late errors/state/metrics, arrives.
        // A failed local drain is a harness failure, never a complete receipt.
        let drained = tokio::time::timeout(Duration::from_secs(15), async {
            while !completion.borrow_and_update().1 {
                completion.changed().await.map_err(|_| invalid())?;
            }
            Ok::<_, AppError>(())
        })
        .await;
        if !matches!(drained, Ok(Ok(()))) {
            let _ = studio.disconnect_and_wait(session).await;
            tokio::time::timeout(Duration::from_secs(15), async {
                while !completion.borrow_and_update().1 {
                    completion.changed().await.unwrap();
                }
            })
            .await
            .expect("incomplete original read event capture after local disconnect");
        }
        let (events, _) = capture.lock().complete_snapshot();
        let error = events
            .iter()
            .find_map(|event| match event {
                QueryEvent::Error { error, .. } => Some(error.clone()),
                _ => None,
            })
            .or_else(|| cancel_result.err())
            .unwrap_or_else(|| {
                AppError::new(
                    ErrorCode::Cancelled,
                    "Synthetic qualification read collection cancelled",
                )
            });
        return Err(CallError { error, events });
    }
    let (events, error) = {
        let capture = capture.lock();
        capture.complete_snapshot()
    };
    let parsed = (|| {
        if let Some(error) = error {
            return Err(error);
        }
        if let Some(error) = events.iter().find_map(|event| match event {
            QueryEvent::Error { error, .. } => Some(error.clone()),
            _ => None,
        }) {
            return Err(error);
        }
        let result = events
            .iter()
            .rev()
            .find_map(|event| match event {
                QueryEvent::Result { result } if result.status == RetentionStatus::Complete => {
                    Some(result.clone())
                }
                _ => None,
            })
            .ok_or_else(invalid)?;
        if result.retained_rows > 1 {
            return Err(invalid());
        }
        let bytes = studio.range(&result.id, 0, 2)?;
        if bytes.len() > READ_BYTES {
            return Err(invalid());
        }
        let reader = StreamReader::try_new(Cursor::new(bytes), None).map_err(|_| invalid())?;
        if reader.schema().fields().len() != result.columns.len() {
            return Err(invalid());
        }
        let mut rows = Vec::new();
        for batch in reader {
            let batch = batch.map_err(|_| invalid())?;
            if rows.len().saturating_add(batch.num_rows()) > 1 {
                return Err(invalid());
            }
            for row in 0..batch.num_rows() {
                let values = batch
                    .columns()
                    .iter()
                    .map(|column| {
                        let column = column
                            .as_any()
                            .downcast_ref::<StringArray>()
                            .ok_or_else(invalid)?;
                        Ok(if column.is_null(row) {
                            None
                        } else {
                            Some(column.value(row).to_owned())
                        })
                    })
                    .collect::<AppResult<TextRow>>()?;
                rows.push(values);
            }
        }
        if rows.len() != result.retained_rows {
            return Err(invalid());
        }
        Ok((result, rows))
    })();
    match parsed {
        Ok((result, rows)) => Ok(ReadCapture {
            events,
            result,
            rows,
        }),
        Err(error) => Err(CallError { error, events }),
    }
}

#[derive(Default)]
struct RouteState {
    session: Option<SessionInfo>,
    runtime: Option<SessionRuntimeState>,
    catalog: Option<BrowseCatalog>,
    inspection: Option<RelationInspection>,
    preflight: Option<(SnowflakeDdlAction, SnowflakeDdlPreflightResponse)>,
}

impl RouteState {
    fn accept(&mut self, output: &Output) {
        match output {
            Output::Startup(v) => self.session = Some(v.clone()),
            Output::Ddl(v) => self.runtime = Some(v.state.clone()),
            Output::Reconcile(options, _) | Output::Options(options) => {
                self.runtime = Some(options.state.clone())
            }
            Output::Catalog(v) => {
                self.catalog = Some(v.clone());
                self.inspection = None;
            }
            Output::Inspect(v) => self.inspection = Some(v.clone()),
            Output::Read(v) => {
                self.runtime = v.events.iter().find_map(|event| match event {
                    QueryEvent::SessionState { state } => Some(state.clone()),
                    _ => None,
                })
            }
            Output::Boundary(v) => self.runtime = Some(v.state.clone()),
            Output::Dml(v) => self.runtime = Some(v.state.clone()),
            Output::Preflight(_) => {}
        }
    }
}

// Only this module constructs the harness. All configurable inputs below are
// synthetic test cases; there is no production profile/account/session selector.
struct Pause {
    phase: u8,
    reached: tokio::sync::oneshot::Sender<()>,
    resume: Arc<tokio::sync::Notify>,
}

struct OfflineHarness {
    fixture: Fixture,
    journal: tempfile::TempDir,
    studio: Arc<Studio>,
    policy: Arc<Policy>,
    attachment: Option<QualificationAttachment>,
    transport: TransportAttachment,
    plan: Plan,
    attempt: String,
    state: RouteState,
    cancel_read: bool,
    pause: Option<Pause>,
    break_journal_after: Option<u8>,
}

impl OfflineHarness {
    async fn new(schema_count: usize, fault: Value) -> Self {
        let cancel_read = matches!(fault["kind"].as_str(), Some("stallRead" | "stallReadError"));
        let fixture = Fixture::new();
        let studio = Studio::with_snowflake_runtime(
            &fixture.directory.path().join("workspace"),
            Arc::new(MemorySecrets::default()),
            Arc::new(DisabledFileDialog),
            Some(fixture.runtime.clone()),
        )
        .unwrap();
        let journal = tempfile::tempdir().unwrap();
        fs::set_permissions(journal.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let policy = Arc::new(Policy::create(journal.path().to_owned(), json!({
            "account":"synthetic-account", "host":"synthetic-account.snowflakecomputing.com",
            "username":"login@example.test", "role":"QUALIFICATION", "warehouse":"FIXTURE_WH",
            "database":"SANDBOX", "schema":"BILLIE_TESTING"
        })).await.unwrap());
        let plan = policy.plan();
        let mut statements = serde_json::Map::new();
        for (name, id) in [
            ("Create", StatementId::Create),
            ("Add", StatementId::Add),
            ("Drop", StatementId::Drop),
            ("RollbackInsert", StatementId::RollbackInsert),
            ("CommittedInsert", StatementId::CommittedInsert),
            ("Update", StatementId::Update),
            ("Delete", StatementId::Delete),
            ("OwnershipRead", StatementId::OwnershipRead),
            ("DataRead", StatementId::DataRead),
        ] {
            statements.insert(name.into(), json!(plan.statement(id).source_sql()));
        }
        fs::write(
            fixture.controls.join("lifecycle-sdk.mjs"),
            include_str!("../../tests/support/snowflake_table_qualification_sdk.mjs"),
        )
        .unwrap();
        // Child exit belongs only to this generated SDK wrapper. It is not an
        // SDK callback error, transport mock, or production runtime capability.
        let child_exit = match fault["kind"].as_str() {
            Some("childExitBeforeAck" | "childExitAfterAck") => {
                let statement = fault["statement"].as_str().unwrap();
                assert!(
                    [
                        "Create",
                        "Add",
                        "Drop",
                        "RollbackInsert",
                        "CommittedInsert",
                        "Update",
                        "Delete"
                    ]
                    .contains(&statement)
                );
                json!({"statement":statement,"afterAck":fault["kind"] == "childExitAfterAck"})
            }
            _ => Value::Null,
        };
        let sdk_fault = if child_exit.is_null() {
            fault
        } else {
            Value::Null
        };
        let options = json!({"schemaCount":schema_count,"tableName":plan.table(),"statements":statements,"fault":sdk_fault});
        let wrapper = format!(
            r#"import {{makeSdk}} from '../../lifecycle-sdk.mjs';
import {{appendFileSync}} from 'node:fs';
const options={options};
const childExit={child_exit};
const append=(name,value)=>appendFileSync(new URL('../../'+name,import.meta.url),JSON.stringify(value)+'\n');
const sdk=makeSdk({{...options,log:sql=>append('queries',sql)}});
if (childExit !== null) {{
  const create=sdk.createConnection.bind(sdk);
  sdk.createConnection=profile=>{{
    const connection=create(profile);
    const execute=connection.execute.bind(connection);
    let armed=false;
    connection.execute=request=>execute({{...request,complete:(error,statement)=>{{
      if (!error && request.sqlText === options.statements[childExit.statement]) {{
        append('child-exit-effects',{{statement:childExit.statement,sql:request.sqlText,
          sdkRequestId:statement.getRequestId(),queryId:statement.getQueryId()}});
        if (!childExit.afterAck) {{
          append('child-exit-events',{{afterAck:false,observerSql:null,pid:process.pid}});
          process.exit(0);
        }}
        armed=true;
      }} else if (!error && armed) {{
        // A new parent-originated observer request proves the preceding helper
        // terminal/ACK was consumed. Immediate exit after forwarding complete
        // would instead leave an ACK-vs-reaper race.
        append('child-exit-events',{{afterAck:true,observerSql:request.sqlText,pid:process.pid}});
        process.exit(0);
      }}
      request.complete(error,statement);
    }}}});
    return connection;
  }};
}}
export default sdk;
"#
        );
        fs::write(
            fixture
                .controls
                .join("node_modules/snowflake-sdk/index.mjs"),
            wrapper,
        )
        .unwrap();
        let profile = studio
            .save_profile(
                ConnectionProfile {
                    id: new_id(),
                    name: "Synthetic closed lifecycle".into(),
                    group_id: "local".into(),
                    engine: EngineKind::Snowflake,
                    host: "synthetic-account.snowflakecomputing.com".into(),
                    port: 443,
                    database: "SANDBOX".into(),
                    username: "login@example.test".into(),
                    environment: Environment::Local,
                    safe_mode: SafeMode::Confirm,
                    tls_mode: TlsMode::VerifyFull,
                    ca_cert_path: None,
                    ssh: None,
                    remember_password: false,
                    store_query_text: false,
                    store_drafts: false,
                    snowflake: Some(SnowflakeConfig {
                        authentication: SnowflakeAuthentication::ExternalBrowser,
                        reviewed_dml: true,
                        reviewed_ddl: true,
                        role: "QUALIFICATION".into(),
                        warehouse: "FIXTURE_WH".into(),
                        schema: SCHEMA.into(),
                    }),
                },
                None,
            )
            .unwrap();
        let attempt = studio.prepare_connection(&profile.id).unwrap();
        let attachment =
            QualificationAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
        let transport = attachment.transport.clone();
        Self {
            fixture,
            journal,
            studio,
            policy,
            attachment: Some(attachment),
            transport,
            plan,
            attempt,
            state: RouteState::default(),
            cancel_read,
            pause: None,
            break_journal_after: None,
        }
    }

    async fn call(&mut self, operation: Operation) -> Result<Output, CallError> {
        if operation == Operation::StartupIdentityAndVersion {
            return self
                .studio
                .connect_prepared_qualification(
                    &self.attempt,
                    ConnectionSecrets::default(),
                    self.attachment.take().ok_or_else(invalid)?,
                )
                .await
                .map(Output::Startup)
                .map_err(Into::into);
        }
        let session = self.state.session.as_ref().ok_or_else(invalid)?.id.clone();
        let output = match operation {
            Operation::DdlPreflight(id) => {
                let (catalog, inspection) = match id {
                    StatementId::Create => (None, None),
                    StatementId::Add => {
                        (self.state.catalog.as_ref(), self.state.inspection.as_ref())
                    }
                    StatementId::Drop => (self.state.catalog.as_ref(), None),
                    _ => return Err(invalid().into()),
                };
                let action = self.plan.ddl_action(id, catalog, inspection)?;
                let result = self
                    .studio
                    .preflight_snowflake_ddl(&SnowflakeDdlPreflightRequest {
                        session_id: session,
                        action: action.clone(),
                    })
                    .await?;
                self.state.preflight = Some((action, result.clone()));
                Output::Preflight(result)
            }
            Operation::ReviewedDdl(_) => {
                let (action, response) = self.state.preflight.take().ok_or_else(invalid)?;
                let plan = self.studio.prepare_snowflake_ddl(
                    &session,
                    &SnowflakeDdlRequest {
                        revision: response.revision,
                        action,
                        sql: response.sql,
                    },
                )?;
                if !plan.reviewed_snowflake_ddl || plan.requires_passphrase {
                    return Err(invalid().into());
                }
                let approval = self.studio.authorize_snowflake_ddl(
                    &plan.id,
                    None,
                    plan.confirmation_text.as_deref(),
                    plan.target_confirmation_text.as_deref(),
                )?;
                Output::Ddl(
                    self.studio
                        .execute_snowflake_ddl(&plan.id, Some(&approval))
                        .await?,
                )
            }
            Operation::CoherentReconciliation(_) => {
                let (options, catalog) = self
                    .studio
                    .refresh_snowflake_ddl_metadata_qualification(&session)
                    .await?;
                Output::Reconcile(options, catalog)
            }
            Operation::Catalog(_) => Output::Catalog(self.studio.relation_catalog(&session).await?),
            Operation::Inspect(_) => {
                let catalog = self.state.catalog.as_ref().ok_or_else(invalid)?;
                let relation = catalog
                    .relations
                    .iter()
                    .find(|r| r.table.schema == SCHEMA && r.table.name == self.plan.table())
                    .ok_or_else(invalid)?;
                Output::Inspect(
                    self.studio
                        .inspect_relation(
                            &session,
                            &InspectRelationRequest {
                                revision: catalog.revision.clone(),
                                target_id: relation.target_id.clone(),
                            },
                        )
                        .await?,
                )
            }
            Operation::Read(expected) => {
                let id = if expected == Readback::Ownership {
                    StatementId::OwnershipRead
                } else {
                    StatementId::DataRead
                };
                Output::Read(
                    read(
                        &self.studio,
                        &session,
                        self.plan.statement(id).source_sql(),
                        self.cancel_read && expected != Readback::Ownership,
                    )
                    .await?,
                )
            }
            Operation::TypedBoundary { action, .. } => {
                let state = self.state.runtime.as_ref().ok_or_else(invalid)?;
                let Some(TransactionControlAuthority::Observed { revision, .. }) =
                    &state.transaction_control_authority
                else {
                    return Err(invalid().into());
                };
                let plan = self.studio.prepare_transaction_control(
                    &session,
                    &TransactionControlRequest {
                        action,
                        revision: revision.clone(),
                    },
                )?;
                let approval = approve(&self.studio, &plan)?;
                Output::Boundary(
                    self.studio
                        .execute_transaction_control(&plan.id, approval.as_deref())
                        .await?,
                )
            }
            Operation::ReviewedDml(id) => {
                let plan = self
                    .studio
                    .prepare_execution(&session, self.plan.statement(id).source_sql())?;
                if !plan.reviewed_snowflake_dml || !plan.bound_parameters.is_empty() {
                    return Err(invalid().into());
                }
                let approval = approve(&self.studio, &plan)?;
                Output::Dml(
                    self.studio
                        .execute_snowflake_dml(&plan.id, approval.as_deref())
                        .await?,
                )
            }
            Operation::SessionOptionsResume => Output::Options(
                self.studio
                    .session_contexts_with_refresh(&session, true)
                    .await?,
            ),
            Operation::StartupIdentityAndVersion => return Err(invalid().into()),
        };
        Ok(output)
    }

    async fn run(mut self) -> Receipt {
        let mut outputs = Vec::new();
        let mut failure = None;
        for phase in self.plan.phases().to_vec() {
            match self.call(phase.operation()).await {
                Ok(output) => {
                    if self
                        .pause
                        .as_ref()
                        .is_some_and(|v| v.phase == phase.number())
                    {
                        let pause = self.pause.take().unwrap();
                        let _ = pause.reached.send(());
                        pause.resume.notified().await;
                    }
                    if self.break_journal_after == Some(phase.number()) {
                        fs::set_permissions(
                            self.journal.path().join(FILE_NAME),
                            fs::Permissions::from_mode(0o644),
                        )
                        .unwrap();
                    }
                    let recorded = self
                        .policy
                        .record_operation_completion(OperationCompletion::Returned(
                            output.evidence(),
                        ))
                        .await;
                    let result = match recorded {
                        Ok(None) => self.policy.finish_phase(output.evidence()).await,
                        Ok(Some(error)) | Err(error) => Err(error),
                    };
                    if result.is_ok() {
                        self.state.accept(&output);
                        if let Output::Read(read) = &output {
                            self.studio.release_result(&read.result.id);
                        }
                    }
                    outputs.push(output);
                    if let Err(error) = result {
                        failure = Some((phase.number(), error));
                        break;
                    }
                }
                Err(original) => {
                    let _ = self
                        .policy
                        .record_operation_completion(OperationCompletion::Failed {
                            error: &original.error,
                            events: &original.events,
                        })
                        .await;
                    failure = Some((phase.number(), original.error));
                    break;
                }
            }
        }
        if let Some((_, error)) = &failure {
            TransportPolicy::failed(&*self.policy, error.clone());
        }
        let shutdown = self.studio.shutdown_and_wait().await;
        let settled = self.transport.wait(Duration::from_secs(10)).await;
        let persisted = self.policy.persist_final_evidence().await;
        let journal = fs::read_to_string(self.journal.path().join(FILE_NAME))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let queries = fs::read_to_string(self.fixture.controls.join("queries"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        Receipt {
            harness: self,
            outputs,
            failure,
            journal,
            queries,
            shutdown,
            settled: settled.is_some(),
            persisted,
        }
    }
}

struct Receipt {
    harness: OfflineHarness,
    outputs: Vec<Output>,
    failure: Option<(u8, AppError)>,
    journal: Vec<Value>,
    queries: Vec<String>,
    shutdown: AppResult<()>,
    settled: bool,
    persisted: AppResult<()>,
}

fn start(schema_count: usize, fault: Value) -> tokio::task::JoinHandle<Receipt> {
    // The task owns setup, actual execution, original receipts and local reap.
    // Dropping its waiter detaches the owner; it does not abort a database call.
    tokio::spawn(async move { OfflineHarness::new(schema_count, fault).await.run().await })
}

fn assert_local_settlement(receipt: &Receipt) {
    assert!(receipt.shutdown.is_ok(), "{:?}", receipt.shutdown);
    assert!(receipt.settled, "helper not reaped");
    assert_eq!(
        fs::read_dir(&receipt.harness.fixture.state)
            .unwrap()
            .count(),
        0
    );
}

#[test]
#[should_panic(expected = "incomplete original read event capture")]
fn qualification_studio_runner_event_overflow_never_becomes_complete_capture() {
    let mut collector = Collector::default();
    for _ in 0..EVENT_LIMIT {
        assert!(collector.push(QueryEvent::Notice {
            message: "bounded".into()
        }));
    }
    assert!(!collector.push(QueryEvent::Finished {
        run_id: "run".into(),
        status: RunStatus::Cancelled,
        elapsed_ms: 0
    }));
    collector.complete_snapshot();
}

#[test]
#[should_panic(expected = "incomplete original read event capture")]
fn qualification_studio_runner_byte_overflow_never_becomes_complete_capture() {
    let mut collector = Collector::default();
    assert!(!collector.push(QueryEvent::Notice {
        message: "x".repeat(READ_BYTES)
    }));
    assert!(!collector.push(QueryEvent::Finished {
        run_id: "run".into(),
        status: RunStatus::Cancelled,
        elapsed_ms: 0
    }));
    collector.complete_snapshot();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_dropped_waiter_preserves_post_ack_owner() {
    let (reached, ready) = tokio::sync::oneshot::channel();
    let (finished, receipt) = tokio::sync::oneshot::channel();
    let resume = Arc::new(tokio::sync::Notify::new());
    let owner = tokio::spawn({
        let resume = resume.clone();
        async move {
            let mut harness = OfflineHarness::new(1, Value::Null).await;
            harness.pause = Some(Pause {
                phase: 3,
                reached,
                resume,
            });
            let _ = finished.send(harness.run().await);
        }
    });
    tokio::time::timeout(Duration::from_secs(30), ready)
        .await
        .unwrap()
        .unwrap();
    drop(owner);
    resume.notify_one();
    let receipt = tokio::time::timeout(Duration::from_secs(90), receipt)
        .await
        .unwrap()
        .unwrap();
    assert_local_settlement(&receipt);
    assert!(receipt.failure.is_none());
    assert!(receipt.persisted.is_ok());
    assert_eq!(receipt.outputs.len(), 32);
    let ledger = receipt
        .harness
        .plan
        .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
        .unwrap();
    assert_eq!(
        receipt.queries,
        ledger
            .entries()
            .iter()
            .map(|v| v.sql().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        receipt
            .journal
            .iter()
            .filter(|v| v["event"]["type"] == "operationCompletion")
            .count(),
        32
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_post_ack_journal_failure_stops_before_reconciliation() {
    let mut harness = OfflineHarness::new(1, Value::Null).await;
    harness.break_journal_after = Some(3);
    let receipt = harness.run().await;
    assert_local_settlement(&receipt);
    assert_eq!(
        receipt.failure.as_ref().map(|v| v.0),
        Some(3),
        "original failure={:?}; transport={:?}; first transport error={:?}; queries={:?}; journal tail={:?}",
        receipt.failure,
        receipt.harness.transport.report(),
        receipt.harness.transport.first_error(),
        receipt.queries,
        receipt
            .journal
            .iter()
            .rev()
            .take(8)
            .map(|record| {
                let event = &record["event"];
                json!({
                    "sequence":record["sequence"], "type":event["type"],
                    "phase":event["phase"], "operation":event["operation"],
                    "kind":event["kind"], "report":event["report"],
                    "error":event["error"]
                })
            })
            .collect::<Vec<_>>()
    );
    assert!(receipt.persisted.is_err());
    assert!(TransportPolicy::check(&*receipt.harness.policy).is_err());
    assert_eq!(receipt.outputs.len(), 3);
    let original = receipt.outputs.last().unwrap().evidence().record().unwrap();
    assert_eq!(original["value"]["statementOutcome"], "succeeded");
    assert!(
        receipt
            .journal
            .iter()
            .filter(|v| v["event"]["type"] == "validatedPhase")
            .all(|v| v["event"]["phase"].as_u64().unwrap() < 3)
    );
    let ledger = receipt
        .harness
        .plan
        .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
        .unwrap();
    assert_eq!(
        receipt.queries,
        ledger
            .entries()
            .iter()
            .take_while(|v| v.phase() <= 3)
            .map(|v| v.sql().to_owned())
            .collect::<Vec<_>>()
    );
}

fn assert_stopped_at(receipt: &Receipt, phase: u8) -> &Value {
    assert_eq!(
        receipt.failure.as_ref().map(|v| v.0),
        Some(phase),
        "original failure={:?}; transport={:?}; first transport error={:?}",
        receipt.failure,
        receipt.harness.transport.report(),
        receipt.harness.transport.first_error()
    );
    assert_local_settlement(receipt);
    assert!(TransportPolicy::check(&*receipt.harness.policy).is_err());
    assert!(
        receipt
            .journal
            .iter()
            .filter(|v| v["event"]["type"] == "validatedPhase")
            .all(|v| v["event"]["phase"].as_u64().unwrap() < u64::from(phase))
    );
    let completions = receipt
        .journal
        .iter()
        .filter(|v| v["event"]["type"] == "operationCompletion")
        .collect::<Vec<_>>();
    assert_eq!(completions.len(), usize::from(phase));
    assert_eq!(completions.last().unwrap()["event"]["phase"], phase);
    &completions.last().unwrap()["event"]["completion"]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_cancellation_drains_original_terminal_events() {
    for kind in ["stallRead", "stallReadError"] {
        let receipt = start(1, json!({"kind":kind, "statement":"DataRead"}))
            .await
            .unwrap();
        let completion = assert_stopped_at(&receipt, 13);
        assert_eq!(completion["status"], "failed");
        let events = completion["original"]["events"].as_array().unwrap();
        if kind == "stallReadError" {
            let original = events
                .iter()
                .find(|v| v["type"] == "error")
                .expect("original stream error absent");
            assert_eq!(completion["original"]["error"], original["error"]);
            assert_ne!(events.last().unwrap()["status"], "completed");
        } else {
            // Clean EOF remains authoritative success, even after cancel was sent.
            // The cancelled qualification owner still cannot advance to BEGIN A.
            assert_eq!(events.last().unwrap()["status"], "completed");
            assert_eq!(completion["original"]["error"]["code"], "conflict");
        }
        assert!(
            events
                .iter()
                .any(|v| v["type"] == "result" && v["result"]["status"] == "streaming")
        );
        assert!(events.iter().any(|v| v["type"] == "runMetrics"));
        assert_eq!(events.last().unwrap()["type"], "finished");
        assert_eq!(events.iter().filter(|v| v["type"] == "finished").count(), 1);
        let ledger = receipt
            .harness
            .plan
            .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
            .unwrap();
        let primary = ledger
            .entries()
            .iter()
            .position(|entry| {
                entry.phase() == 13 && matches!(entry.kind(), super::ledger::SqlKind::Readback(_))
            })
            .unwrap();
        assert_eq!(
            receipt.queries,
            ledger.entries()[..=primary]
                .iter()
                .map(|entry| entry.sql().to_owned())
                .collect::<Vec<_>>()
        );
    }
}

async fn mutation_failures(kind: &str) {
    for (name, id, phase) in [
        ("Create", StatementId::Create, 3),
        ("Add", StatementId::Add, 9),
        ("RollbackInsert", StatementId::RollbackInsert, 15),
        ("CommittedInsert", StatementId::CommittedInsert, 20),
        ("Update", StatementId::Update, 23),
        ("Delete", StatementId::Delete, 25),
        ("Drop", StatementId::Drop, 30),
    ] {
        let receipt = start(1, json!({"kind":kind,"statement":name}))
            .await
            .unwrap();
        let completion = assert_stopped_at(&receipt, phase);
        let ledger = receipt
            .harness
            .plan
            .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
            .unwrap();
        let source = receipt
            .harness
            .plan
            .statement(id)
            .source_sql()
            .trim_end_matches(';');
        let primary = ledger
            .entries()
            .iter()
            .position(|entry| entry.phase() == phase && entry.sql() == source)
            .unwrap();
        assert!(
            receipt.queries.len() > primary,
            "mutation absent: {kind}/{name}"
        );
        assert_eq!(
            receipt.queries[..=primary],
            ledger.entries()[..=primary]
                .iter()
                .map(|entry| entry.sql().to_owned())
                .collect::<Vec<_>>()
        );
        let tail = &receipt.queries[primary + 1..];
        if kind == "rejectMutation" {
            let allowed = ledger.rejection_suffix(primary).unwrap();
            assert!(
                tail.len() <= allowed.len(),
                "rejection tail too long: {kind}/{name}"
            );
            assert_eq!(
                tail,
                allowed[..tail.len()]
                    .iter()
                    .map(|entry| entry.sql().to_owned())
                    .collect::<Vec<_>>()
            );
        } else {
            let remaining = ledger.entries()[primary + 1..]
                .iter()
                .take_while(|entry| entry.phase() == phase && entry.kind().is_read())
                .collect::<Vec<_>>();
            assert!(
                tail.len() <= remaining.len(),
                "failure tail too long: {kind}/{name}"
            );
            assert_eq!(
                tail,
                remaining[..tail.len()]
                    .iter()
                    .map(|entry| entry.sql().to_owned())
                    .collect::<Vec<_>>()
            );
        }
        let reports = receipt
            .journal
            .iter()
            .filter(|v| {
                v["event"]["admission"]["kind"]["sql"]["phase"] == phase
                    && v["event"]["admission"]["kind"]["sql"]["sql"] == source
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reports.len(),
            1,
            "exact mutation terminal missing: {kind}/{name}"
        );
        let report = &reports[0]["event"]["report"];
        if kind == "observerMismatch" {
            assert_eq!(completion["status"], "returned");
            let original = receipt.outputs.last().unwrap().evidence().record().unwrap();
            assert_eq!(completion["original"], original);
            assert_eq!(original["value"]["statementOutcome"], "succeeded");
            assert_eq!(original["value"]["retired"], true);
            let key = if matches!(
                id,
                StatementId::Create | StatementId::Add | StatementId::Drop
            ) {
                "ddlAcknowledged"
            } else {
                "dmlAcknowledged"
            };
            let ack = &report[key]["statement"];
            assert!(ack["sdkRequestId"].as_str().is_some_and(|v| !v.is_empty()));
            assert!(ack["queryId"].as_str().is_some_and(|v| !v.is_empty()));
            assert_eq!(&report["statements"][0], ack);
        } else if completion["status"] == "returned" {
            assert_eq!(
                completion["original"]["value"]["statementOutcome"],
                if kind == "rejectMutation" {
                    "rejected"
                } else {
                    "unknown"
                }
            );
        } else {
            assert_eq!(completion["status"], "failed");
        }
        if kind != "observerMismatch" {
            assert_eq!(report["ok"], false);
            assert_eq!(
                report["errorCode"],
                if kind == "rejectMutation" {
                    "database"
                } else {
                    "outcomeUnknown"
                }
            );
            assert!(report["ddlAcknowledged"].is_null());
            assert!(report["dmlAcknowledged"].is_null());
            if kind == "rejectMutation" {
                assert_eq!(report["certainty"], "ServerTerminal");
                assert!(report["retirement"].is_null());
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_definitive_rejections_stop_all_seven_mutations() {
    mutation_failures("rejectMutation").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_sdk_callback_loss_stops_all_seven_mutations() {
    // SDK callback errors after fixture effects are not helper wire EOF tests.
    mutation_failures("loseMutationAck").await;
}

async fn mutation_child_exits(after_ack: bool) {
    for (name, id, phase) in [
        ("Create", StatementId::Create, 3),
        ("Add", StatementId::Add, 9),
        ("RollbackInsert", StatementId::RollbackInsert, 15),
        ("CommittedInsert", StatementId::CommittedInsert, 20),
        ("Update", StatementId::Update, 23),
        ("Delete", StatementId::Delete, 25),
        ("Drop", StatementId::Drop, 30),
    ] {
        let kind = if after_ack {
            "childExitAfterAck"
        } else {
            "childExitBeforeAck"
        };
        let receipt = start(1, json!({"kind":kind,"statement":name}))
            .await
            .unwrap();
        let completion = assert_stopped_at(&receipt, phase);
        assert!(receipt.persisted.is_err());
        let ledger = receipt
            .harness
            .plan
            .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
            .unwrap();
        let source = receipt.harness.plan.statement(id).source_sql();
        let primary = ledger
            .entries()
            .iter()
            .position(|entry| entry.phase() == phase && entry.sql() == source)
            .unwrap();
        let last = primary + usize::from(after_ack);
        if after_ack {
            assert_eq!(ledger.entries()[last].phase(), phase);
            assert!(ledger.entries()[last].kind().is_read());
        }
        assert_eq!(
            receipt.queries,
            ledger.entries()[..=last]
                .iter()
                .map(|entry| entry.sql().to_owned())
                .collect::<Vec<_>>(),
            "unexpected SQL prefix: {kind}/{name}"
        );
        // The prefix equality prohibits every retry and later phase. In
        // particular, shutdown must never issue a compensating cleanup DROP.
        assert_eq!(
            receipt
                .queries
                .iter()
                .filter(|sql| sql.as_str() == source)
                .count(),
            1
        );
        assert_eq!(
            receipt
                .queries
                .iter()
                .filter(|sql| sql.starts_with("DROP TABLE "))
                .count(),
            usize::from(id == StatementId::Drop)
        );

        let read_markers = |file: &str| {
            fs::read_to_string(receipt.harness.fixture.controls.join(file))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        };
        let effects = read_markers("child-exit-effects");
        assert_eq!(
            effects.len(),
            1,
            "missing/duplicate simulated effect: {kind}/{name}"
        );
        assert_eq!(effects[0]["statement"], name);
        assert_eq!(effects[0]["sql"], source);
        assert!(
            effects[0]["sdkRequestId"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );
        assert!(
            effects[0]["queryId"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );
        let exits = read_markers("child-exit-events");
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0]["afterAck"], after_ack);
        assert!(exits[0]["pid"].as_u64().is_some_and(|pid| pid > 0));
        if after_ack {
            assert_eq!(exits[0]["observerSql"], ledger.entries()[last].sql());
        } else {
            assert!(exits[0]["observerSql"].is_null());
        }

        // Preserve the actual typed result, not a manufactured summary of the
        // simulated effect. Before callback delivery the result must be unknown.
        assert_eq!(completion["status"], "returned");
        assert_eq!(receipt.outputs.len(), usize::from(phase));
        let original = receipt.outputs.last().unwrap().evidence().record().unwrap();
        assert_eq!(completion["original"], original);
        assert_eq!(
            original["value"]["statementOutcome"],
            if after_ack { "succeeded" } else { "unknown" }
        );
        assert_eq!(original["value"]["retired"], true);
        assert_eq!(
            original["value"]["error"]["code"],
            if after_ack {
                "connectionClosed"
            } else {
                "outcomeUnknown"
            }
        );
        let reports = receipt
            .journal
            .iter()
            .filter(|record| {
                record["event"]["admission"]["kind"]["sql"]["phase"] == phase
                    && record["event"]["admission"]["kind"]["sql"]["sql"] == source
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reports.len(),
            1,
            "missing/duplicate original mutation report: {kind}/{name}"
        );
        let event = &reports[0]["event"];
        let report = &event["report"];
        assert!(event["wireId"].as_str().is_some_and(|v| !v.is_empty()));
        if after_ack {
            assert_eq!(report["ok"], true);
            assert_eq!(report["certainty"], "StatementSucceeded");
            assert!(report["retirement"].is_null());
            let ack_key = if matches!(
                id,
                StatementId::Create | StatementId::Add | StatementId::Drop
            ) {
                "ddlAcknowledged"
            } else {
                "dmlAcknowledged"
            };
            let ack = &report[ack_key]["statement"];
            assert_eq!(ack["sdkRequestId"], effects[0]["sdkRequestId"]);
            assert_eq!(ack["queryId"], effects[0]["queryId"]);
            assert_eq!(report["statements"].as_array().unwrap().len(), 1);
            assert_eq!(&report["statements"][0], ack);
        } else {
            assert_eq!(event["origin"]["kind"], "retirement");
            assert_eq!(report["ok"], false);
            assert_eq!(report["certainty"], "OutcomeUnknown");
            assert_eq!(report["errorCode"], "outcomeUnknown");
            assert_eq!(report["retirement"], "Eof");
            assert!(report["ddlAcknowledged"].is_null());
            assert!(report["dmlAcknowledged"].is_null());
        }
        if after_ack {
            let observer_reports = receipt
                .journal
                .iter()
                .filter(|record| {
                    record["event"]["admission"]["kind"]["sql"]["phase"] == phase
                        && record["event"]["admission"]["kind"]["sql"]["sql"]
                            == ledger.entries()[last].sql()
                        && record["event"]["origin"]["kind"] == "retirement"
                })
                .collect::<Vec<_>>();
            assert_eq!(
                observer_reports.len(),
                1,
                "original observer retirement absent: {name}"
            );
            assert_eq!(observer_reports[0]["event"]["report"]["retirement"], "Eof");
            assert_eq!(observer_reports[0]["event"]["report"]["ok"], false);
        }
        // assert_stopped_at already requires shutdown, all transport tasks and
        // child reap, and deletion of the exact private runtime root. Whether
        // stdout or child.wait observed the exit first is deliberately immaterial.
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_child_exit_before_ack_preserves_unknown_for_all_mutations() {
    mutation_child_exits(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_child_exit_after_ack_preserves_success_for_all_mutations() {
    mutation_child_exits(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_observer_failures_retain_all_seven_mutation_acks() {
    mutation_failures("observerMismatch").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_completes_all_actual_routes_at_census_extremes() {
    for schemas in [1, 32] {
        let receipt = start(schemas, Value::Null).await.unwrap();
        assert!(
            receipt.failure.is_none(),
            "S={schemas}, failure={:?}; transport={:?}; first transport error={:?}; SQL={:?}; journal tail={:?}",
            receipt.failure,
            receipt.harness.transport.report(),
            receipt.harness.transport.first_error(),
            receipt.queries,
            receipt.journal.iter().rev().take(4).collect::<Vec<_>>()
        );
        assert!(receipt.shutdown.is_ok(), "{:?}", receipt.shutdown);
        assert!(receipt.settled);
        assert!(receipt.persisted.is_ok(), "{:?}", receipt.persisted);
        assert_eq!(
            fs::read_dir(&receipt.harness.fixture.state)
                .unwrap()
                .count(),
            0
        );
        assert_eq!(receipt.outputs.len(), 32);
        let names = std::iter::once(SCHEMA.to_owned())
            .chain((2..=schemas).map(|n| format!("S{n:02}")))
            .collect::<Vec<_>>();
        let census = SchemaCensus::freeze(&names).unwrap();
        let ledger = receipt
            .harness
            .plan
            .expand_physical_ledger(&census)
            .unwrap();
        assert_eq!(
            receipt.queries,
            ledger
                .entries()
                .iter()
                .map(|entry| entry.sql().to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(receipt.queries.len(), 386 + 7 * schemas);
        assert_eq!(
            receipt
                .journal
                .iter()
                .filter(|v| v["event"]["type"] == "validatedPhase")
                .count(),
            32
        );
        assert_eq!(
            receipt
                .journal
                .iter()
                .filter(|v| v["event"]["type"] == "runtime")
                .count(),
            28
        );
        assert_eq!(
            receipt
                .journal
                .iter()
                .filter(|v| v["event"]["type"] == "operationCompletion")
                .count(),
            32
        );
        for (index, output) in receipt.outputs.iter().enumerate() {
            let original = output.evidence().record().unwrap();
            let phase = json!(index + 1);
            let completion = receipt
                .journal
                .iter()
                .position(|v| {
                    v["event"]["type"] == "operationCompletion" && v["event"]["phase"] == phase
                })
                .expect("original completion absent");
            let validation = receipt
                .journal
                .iter()
                .position(|v| {
                    v["event"]["type"] == "validatedPhase" && v["event"]["phase"] == phase
                })
                .expect("validated phase absent");
            assert!(completion < validation);
            assert_eq!(
                receipt.journal[completion]["event"]["completion"]["status"],
                "returned"
            );
            assert_eq!(
                receipt.journal[completion]["event"]["completion"]["original"],
                original
            );
            assert_eq!(receipt.journal[validation]["event"]["evidence"], original);
        }
    }
}
