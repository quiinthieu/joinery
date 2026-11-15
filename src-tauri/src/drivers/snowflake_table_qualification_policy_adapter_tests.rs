//! Real production adapter/supervisor/helper routes with a synthetic SDK only.
//! These tests do not import the account SDK, authenticate, or execute remotely.

use super::{PhaseEvidence, Policy, profile, records};
use crate::{
    domain::{
        AppError, AppResult, ConnectionProfile, EngineKind, Environment, SafeMode, SessionInfo,
        SnowflakeAuthentication, SnowflakeConfig, SnowflakeDdlAction, SnowflakeDdlColumnDraft,
        SnowflakeDdlPreflightRequest, TlsMode,
    },
    drivers::{
        Driver,
        snowflake_external::{ExternalBrowserOptions, SnowflakeExternal},
        snowflake_process::{CleanupObserver, Launch, Report},
        snowflake_qualification::QualificationAttachment,
        snowflake_qualification_metadata::{
            MetadataObserver, RelationObservation, SchemaObservation,
        },
        snowflake_qualification_transport::{
            Disposition, LocalTerminal, LocalTerminalOrigin, PhysicalRequest, PhysicalTerminal,
            TransportLimits, TransportPolicy,
        },
    },
};
use serde_json::{Value, json};
use std::{
    fs,
    future::Future,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, watch};

const CONSOLE: &str = "fixture-console";

// The real Helper owns all request parsing, SDK callback/stream processing,
// terminal reports, row credits, and framed output. Only the SDK is synthetic.
const SYNTHETIC_SDK: &str = r#"
import { Readable } from 'node:stream';
import { appendFileSync, existsSync, watch } from 'node:fs';
const { Helper } = await import(HELPER_URL);
const { Decoder, writer } = await import(FRAMING_URL);
const file = name => new URL(name, import.meta.url);
const pauseVersion = PAUSE_VERSION;
// Install before accepting any helper request. The production row-batch timer
// can publish the yielded version row while this stream still awaits EOF.
const versionReleased = pauseVersion ? new Promise(resolve => {
  const watcher = watch(file('.'), () => {
    if (existsSync(file('release-version'))) { watcher.close(); resolve(); }
  });
}) : Promise.resolve();
const bootstrap = 'SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SESSION()';
const context = 'SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES';
let sequence = 0;
const uuid = n => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const sdk = { createConnection() {
  appendFileSync(file('connections'), '1');
  return {
    connectAsync(callback) { queueMicrotask(() => callback(null)); return Promise.resolve(); },
    destroy(callback) { callback(null); },
    execute(options) {
      const sql = options.sqlText, id = ++sequence;
      appendFileSync(file('queries'), JSON.stringify(sql) + '\n');
      let names, rows;
      if (sql === bootstrap) {
        names = ['U','R','D','S','W','ID'];
        rows = [['FIXTURE','QUALIFICATION','SANDBOX','BILLIE_TESTING','FIXTURE_WH','SESSION']];
      } else if (sql === 'SELECT CURRENT_VERSION()') {
        names = ['VERSION']; rows = [['synthetic-qualification-version']];
      } else if (sql === context) {
        names = ['PRINCIPAL','ROLE_NAME','DATABASE_NAME','SCHEMA_NAME','WAREHOUSE_NAME','SESSION_ID','SEARCH_PATH','SECONDARY_ROLES'];
        rows = [['FIXTURE','QUALIFICATION','SANDBOX','BILLIE_TESTING','FIXTURE_WH','SESSION','["SANDBOX.BILLIE_TESTING"]','{"roles":"","value":"ALL"}']];
      } else if (sql.startsWith('SHOW PARAMETERS LIKE ')) {
        const parameter = sql.split("'")[1];
        if (!['QUOTED_IDENTIFIERS_IGNORE_CASE','AUTOCOMMIT','TRANSACTION_ABORT_ON_ERROR'].includes(parameter)) throw new Error('unexpected parameter');
        names = ['key','value','default','level'];
        const value = parameter === 'AUTOCOMMIT' ? 'true' : 'false';
        rows = [[parameter,value,value === 'true' ? 'false' : 'true','ACCOUNT']];
      } else if (sql === 'SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID') {
        names = ['TRANSACTION_ID']; rows = [[null]];
      } else if (sql.includes('.INFORMATION_SCHEMA.SCHEMATA ')) {
        names = ['CATALOG_NAME','SCHEMA_NAME']; rows = [['SANDBOX','BILLIE_TESTING']];
      } else if (sql.includes('.INFORMATION_SCHEMA.TABLES ')) {
        names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','TABLE_TYPE','IS_TRANSIENT','IS_TEMPORARY','IS_ICEBERG','IS_DYNAMIC','IS_HYBRID','IS_IMMUTABLE','CREATED','LAST_DDL'];
        rows = [];
      } else {
        throw new Error('unexpected qualification SQL');
      }
      const statement = {
        getRequestId: () => uuid(id), getQueryId: () => uuid(id + 1000),
        getColumns: () => names.map(name => ({ getName: () => name, getType: () => 'TEXT' })),
        streamRows: () => Readable.from((async function* () {
          for (const row of rows) yield row;
          if (pauseVersion && sql === 'SELECT CURRENT_VERSION()') await versionReleased;
        })()), cancel(callback) { callback(null); },
      };
      setImmediate(() => options.complete(null, statement));
      return statement;
    },
  };
}};
const send = writer(process.stdout.write.bind(process.stdout), () => process.exit(2));
let stopping = false;
const retire = () => { if (stopping) return; stopping = true; send.flush().then(() => process.exit(0)); };
const helper = new Helper(sdk, send, retire, { clientConfigFile:'/nonexistent/synthetic', closeTimeoutMs:200 });
const decoder = new Decoder();
process.stdin.on('data', chunk => { try { for (const frame of decoder.push(chunk)) {
  appendFileSync(file('frames'), JSON.stringify(frame) + '\n');
  helper.receive(frame);
} } catch { retire(); } });
process.stdin.on('end', retire);
"#;

struct PausedCredit {
    first: AtomicBool,
    before_admission: bool,
    scripts: OnceLock<PathBuf>,
    admitted: watch::Sender<Option<Value>>,
    version_terminal: watch::Sender<Option<Report>>,
    local_terminal: watch::Sender<Option<Value>>,
    release: Semaphore,
}

impl PausedCredit {
    fn at_admission(before_admission: bool) -> Self {
        Self {
            first: AtomicBool::new(true),
            before_admission,
            scripts: OnceLock::new(),
            admitted: watch::channel(None).0,
            version_terminal: watch::channel(None).0,
            local_terminal: watch::channel(None).0,
            release: Semaphore::new(0),
        }
    }

    async fn block(&self, original: Value) {
        self.admitted.send_replace(Some(original));
        self.release
            .acquire()
            .await
            .expect("credit barrier open")
            .forget();
    }
}

struct PausedPolicy {
    inner: Arc<Policy>,
    pause: Arc<PausedCredit>,
}

impl TransportPolicy for PausedPolicy {
    fn check(&self) -> AppResult<()> {
        TransportPolicy::check(self.inner.as_ref())
    }
    fn failed(&self, error: AppError) {
        TransportPolicy::failed(self.inner.as_ref(), error);
    }
    fn terminal(&self, terminal: PhysicalTerminal<'_>) -> AppResult<()> {
        let is_version = self
            .pause
            .admitted
            .borrow()
            .as_ref()
            .is_some_and(|admission| {
                admission["envelope"]["payload"]["target"].as_str() == Some(terminal.wire_id)
            });
        let report = terminal.report.clone();
        let result = TransportPolicy::terminal(self.inner.as_ref(), terminal);
        if is_version {
            self.pause.version_terminal.send_replace(Some(report));
        }
        result
    }
    fn local_terminal(&self, terminal: LocalTerminal<'_>) -> AppResult<()> {
        let admitted = match terminal.origin {
            LocalTerminalOrigin::InactiveControl { admitted } => admitted,
            _ => return TransportPolicy::local_terminal(self.inner.as_ref(), terminal),
        };
        let original = json!({"operation":terminal.operation,"logicalId":terminal.operation_id,
            "ticket":terminal.ticket,"wireId":terminal.wire_id,"candidateWireId":terminal.candidate_wire_id,
            "admitted":admitted,"payload":terminal.payload,"target":terminal.target,
            "report":{"ok":terminal.report.ok,"certainty":format!("{:?}",terminal.report.certainty),
                "phase":format!("{:?}",terminal.report.phase),"result":terminal.report.result,
                "errorCode":terminal.report.error_code,"retirement":terminal.report.retirement.map(|r|format!("{r:?}")),
                "statements":terminal.report.statements.len(),
                "hasAck":terminal.report.use_acknowledged.is_some() || terminal.report.boundary_acknowledged.is_some()
                    || terminal.report.dml_acknowledged.is_some() || terminal.report.ddl_acknowledged.is_some()}});
        let result = TransportPolicy::local_terminal(self.inner.as_ref(), terminal);
        self.pause.local_terminal.send_replace(Some(original));
        result
    }
    fn admit<'a>(
        &'a self,
        request: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let pause =
                request.operation == "credit" && self.pause.first.swap(false, Ordering::SeqCst);
            let original = json!({"envelope":request.payload,"ticket":request.ticket,
                "logicalId":request.operation_id,"target":request.target});
            if pause && self.pause.before_admission {
                self.pause.block(original.clone()).await;
            }
            TransportPolicy::admit(self.inner.as_ref(), request).await?;
            if pause && !self.pause.before_admission {
                self.pause.block(original).await;
            }
            Ok(())
        })
    }
}

impl MetadataObserver for PausedPolicy {
    fn check(&self) -> AppResult<()> {
        MetadataObserver::check(self.inner.as_ref())
    }
    fn observe_schemas(&self, observation: SchemaObservation<'_>) -> AppResult<()> {
        self.inner.observe_schemas(observation)
    }
    fn observe_relations(&self, observation: RelationObservation<'_>) -> AppResult<()> {
        self.inner.observe_relations(observation)
    }
    fn observe_runtime(&self, state: &crate::domain::SessionRuntimeState) -> AppResult<()> {
        self.inner.observe_runtime(state)
    }
}

async fn barrier<T: Clone>(mut receiver: watch::Receiver<Option<T>>, label: &str) -> T {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(value) = receiver.borrow().clone() {
                return value;
            }
            receiver
                .changed()
                .await
                .expect("barrier sender remains owned");
        }
    })
    .await
    .expect(label)
}

struct Harness {
    driver: Arc<SnowflakeExternal>,
    info: SessionInfo,
    policy: Arc<Policy>,
    attachment: QualificationAttachment,
    cleanup: CleanupObserver,
    state_path: PathBuf,
    scripts: tempfile::TempDir,
    journal: tempfile::TempDir,
}

impl Harness {
    async fn open() -> Self {
        Self::open_with_pause(None).await
    }

    async fn open_with_pause(pause: Option<Arc<PausedCredit>>) -> Self {
        let journal = tempfile::tempdir().unwrap();
        fs::set_permissions(journal.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let policy = Arc::new(
            Policy::create(journal.path().to_owned(), profile())
                .await
                .unwrap(),
        );
        let attachment = if let Some(pause) = &pause {
            QualificationAttachment::new(
                Arc::new(PausedPolicy {
                    inner: policy.clone(),
                    pause: pause.clone(),
                }),
                TransportLimits::default(),
            )
            .unwrap()
        } else {
            QualificationAttachment::new(policy.clone(), TransportLimits::default()).unwrap()
        };
        let scripts = tempfile::tempdir().unwrap();
        if let Some(pause) = &pause {
            pause.scripts.set(scripts.path().to_owned()).unwrap();
        }
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let source_url = |name| {
            url::Url::from_file_path(repo.join("snowflake-helper/src").join(name))
                .unwrap()
                .to_string()
        };
        let script = SYNTHETIC_SDK
            .replace(
                "PAUSE_VERSION",
                if pause.is_some() { "true" } else { "false" },
            )
            .replace(
                "HELPER_URL",
                &serde_json::to_string(&source_url("helper.mjs")).unwrap(),
            )
            .replace(
                "FRAMING_URL",
                &serde_json::to_string(&source_url("framing.mjs")).unwrap(),
            );
        let path = scripts.path().join("qualification-adapter-sdk.mjs");
        fs::write(&path, script).unwrap();
        let node = PathBuf::from(
            std::env::var_os("JOINERY_TEST_NODE").expect("explicit JOINERY_TEST_NODE required"),
        );
        assert!(node.is_absolute() && node.is_file());
        let mut launch = Launch::new(
            node,
            Some(path),
            scripts.path().to_owned(),
            tempfile::tempdir().unwrap(),
        )
        .unwrap();
        let cleanup = launch.cleanup_observer();
        let state_path = launch.state_directory().to_owned();
        launch
            .attach_transport(attachment.transport.clone())
            .unwrap();
        let profile = ConnectionProfile {
            id: "fixture-profile".into(),
            name: "Synthetic qualification adapter".into(),
            group_id: "test".into(),
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
                reviewed_dml: false,
                reviewed_ddl: true,
                reviewed_console: false,
                reviewed_console_timeout_seconds: None,
                role: "QUALIFICATION".into(),
                warehouse: "FIXTURE_WH".into(),
                schema: "BILLIE_TESTING".into(),
            }),
        };
        let (driver, info) = tokio::time::timeout(
            Duration::from_secs(3),
            SnowflakeExternal::open(
                &profile,
                CONSOLE,
                launch,
                ExternalBrowserOptions {
                    expected_server_user: Some("FIXTURE".into()),
                    metadata_observer: Some(attachment.metadata.clone()),
                },
            ),
        )
        .await
        .expect("qualification adapter startup deadline")
        .expect("production adapter startup");
        Self {
            driver,
            info,
            policy,
            attachment,
            cleanup,
            state_path,
            scripts,
            journal,
        }
    }

    fn create_request(&self) -> SnowflakeDdlPreflightRequest {
        let name = self.policy.state.lock().sequence.plan().table().to_owned();
        SnowflakeDdlPreflightRequest {
            session_id: CONSOLE.into(),
            action: SnowflakeDdlAction::CreateTable {
                database: "SANDBOX".into(),
                schema: "BILLIE_TESTING".into(),
                name,
                columns: [
                    ("ID", "NUMBER(38,0)"),
                    ("NOTE", "VARCHAR(128)"),
                    ("ACTIVE", "BOOLEAN"),
                ]
                .into_iter()
                .map(|(name, data_type)| SnowflakeDdlColumnDraft {
                    name: name.into(),
                    data_type: data_type.into(),
                    nullable: true,
                })
                .collect(),
            },
        }
    }

    fn queries(&self) -> Vec<String> {
        fs::read_to_string(self.scripts.path().join("queries"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn frames(&self) -> Vec<Value> {
        fs::read_to_string(self.scripts.path().join("frames"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    async fn close(&self, expected_failure: bool) {
        let mut shutdown = self.driver.shutdown_receiver();
        self.driver.close();
        let shutdown = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(report) = shutdown.borrow().clone() {
                    break report;
                }
                shutdown.changed().await.expect("shutdown report channel");
            }
        })
        .await
        .expect("actual helper shutdown deadline");
        let cleanup = self
            .cleanup
            .wait(Duration::from_secs(3))
            .await
            .expect("cleanup receipt");
        assert!(cleanup.child_started && cleanup.reaped && cleanup.temporary_root_removed);
        assert!(!self.state_path.exists());
        let report = self
            .attachment
            .transport
            .wait(Duration::from_secs(3))
            .await
            .expect("transport tasks settled");
        assert!(report.settled);
        assert_ne!(report.stdout, Disposition::Running);
        assert_ne!(report.stderr, Disposition::Running);
        assert_ne!(report.writer, Disposition::Running);
        assert!(report.stdout_bytes > 0 && report.complete_frames > 0);
        assert_eq!(report.stderr_bytes, 0);
        if expected_failure {
            assert!(report.failed);
            assert!(self.attachment.transport.first_error().is_some());
            assert!(self.policy.persist_final_evidence().await.is_err());
        } else {
            assert!(shutdown.logout_acknowledged);
            assert!(!report.failed);
            assert!(self.attachment.transport.first_error().is_none());
            self.policy.persist_final_evidence().await.unwrap();
            let admitted_inactive = records(self.journal.path())
                .iter()
                .filter(|row| {
                    let event = &row["event"];
                    event["origin"]["kind"] == "inactiveControl"
                        && event["origin"]["admitted"] == true
                        && event["wireId"].is_null()
                        && event["candidateWireId"].is_string()
                })
                .count() as u64;
            assert_eq!(
                report.admitted_writes,
                report.complete_writes + admitted_inactive,
                "every unwritten admission has its own durable original local receipt"
            );
        }
        assert_eq!(
            fs::read_to_string(self.scripts.path().join("connections")).unwrap(),
            "1"
        );
    }
}

#[tokio::test]
async fn owning_adapter_real_helper_startup_and_create_preflight_finish_two_typed_phases() {
    tokio::time::timeout(Duration::from_secs(9), async {
        let harness = Harness::open().await;
        assert_eq!(harness.info.id, CONSOLE);
        assert_eq!(
            harness.info.server_version,
            "synthetic-qualification-version"
        );
        assert_eq!(harness.driver.verified_identity()["session"], "SESSION");
        assert_eq!(harness.queries().len(), 2);
        {
            let state = harness.policy.state.lock();
            assert_eq!(state.sequence.phase().unwrap().number(), 1);
            assert!(state.sequence.phase_ready().is_ok());
            assert!(state.runtime_pin.is_none());
            assert!(state.lifecycle.context().is_none());
        }
        harness
            .policy
            .finish_phase(PhaseEvidence::Startup(&harness.info))
            .await
            .unwrap();
        let request = harness.create_request();
        let preflight = tokio::time::timeout(
            Duration::from_secs(3),
            harness.driver.preflight_snowflake_ddl(CONSOLE, &request),
        )
        .await
        .expect("production preflight deadline")
        .expect("production CREATE preflight");
        let expected = {
            let state = harness.policy.state.lock();
            state.check().unwrap();
            assert_eq!(state.sequence.phase().unwrap().number(), 2);
            assert!(state.sequence.phase_ready().is_ok());
            assert_eq!(state.phase_runtime_count, 1);
            assert_eq!(state.authority_revisions.len(), 1);
            assert!(state.runtime_pin.is_some());
            assert_eq!(
                state.lifecycle.census().unwrap().names(),
                &["BILLIE_TESTING".to_owned()]
            );
            assert_eq!(state.lifecycle.context().unwrap().principal, "FIXTURE");
            assert_eq!(state.last_runtime.as_ref().unwrap().session_id, CONSOLE);
            assert_eq!(
                preflight.sql,
                state
                    .sequence
                    .plan()
                    .statement(super::super::super::StatementId::Create)
                    .source_sql()
            );
            state
                .sequence
                .ledger()
                .entries()
                .iter()
                .take(14)
                .map(|entry| entry.sql().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(harness.queries(), expected);
        let frames = harness.frames();
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["op"] == "connect")
                .count(),
            1
        );
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["op"] == "context")
                .count(),
            1
        );
        assert_eq!(
            frames.iter().filter(|frame| frame["op"] == "query").count(),
            13
        );
        assert_eq!(frames[0]["payload"], profile());
        assert!(!frames.iter().any(|frame| frame["op"] == "ddl"));
        harness
            .policy
            .finish_phase(PhaseEvidence::Preflight(&preflight))
            .await
            .unwrap();
        {
            let state = harness.policy.state.lock();
            assert_eq!(state.sequence.phase().unwrap().number(), 3);
            assert_eq!(state.sequence.next_entry().unwrap().ordinal(), 15);
            assert_eq!(state.sequence.ledger().entries().len(), 393);
            assert!(state.sequence.census_frozen());
            assert!(state.pending.is_empty() && state.debt.is_empty());
            assert_eq!(state.phase_runtime_count, 0);
        }
        let journal = records(harness.journal.path());
        let events = journal.iter().map(|row| &row["event"]).collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .filter(|event| event["type"] == "frozenLedger")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["type"] == "runtime")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["type"] == "validatedPhase")
                .map(|event| event["phase"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        let recorded_sql = events
            .iter()
            .filter_map(|event| event["kind"]["sql"]["sql"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            recorded_sql,
            expected.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert_eq!(
            events
                .iter()
                .find(|event| event["type"] == "validatedPhase" && event["phase"] == 2)
                .unwrap()["evidence"]["value"],
            serde_json::to_value(&preflight).unwrap()
        );
        harness.close(false).await;
        assert_eq!(harness.queries(), expected, "normal close submits no SQL");
        assert_eq!(
            harness
                .frames()
                .iter()
                .filter(|frame| frame["op"] == "close")
                .count(),
            1
        );
    })
    .await
    .expect("complete production route and cleanup bounded to nine seconds");
}

#[tokio::test]
async fn owning_adapter_real_helper_cannot_start_preflight_before_typed_startup_finish() {
    tokio::time::timeout(Duration::from_secs(9), async {
        let harness = Harness::open().await;
        let before = harness.queries();
        assert_eq!(before.len(), 2);
        let request = harness.create_request();
        assert!(
            tokio::time::timeout(
                Duration::from_secs(3),
                harness.driver.preflight_snowflake_ddl(CONSOLE, &request)
            )
            .await
            .expect("denied preflight deadline")
            .is_err()
        );
        assert_eq!(
            harness.queries(),
            before,
            "next-phase SQL never reaches SDK"
        );
        {
            let state = harness.policy.state.lock();
            assert_eq!(state.sequence.phase().unwrap().number(), 1);
            assert!(state.check().is_err());
            assert!(!state.sequence.census_frozen());
            assert!(state.lifecycle.context().is_none());
        }
        harness.close(true).await;
        let journal = records(harness.journal.path());
        assert!(
            !journal
                .iter()
                .any(|row| row["event"]["type"] == "validatedPhase")
        );
        assert_eq!(harness.queries(), before);
    })
    .await
    .expect("denial and actual child cleanup bounded to nine seconds");
}

#[tokio::test]
async fn owning_adapter_real_helper_stale_credit_after_durable_admission_finishes_locally() {
    stale_credit_route(false).await;
}

#[tokio::test]
async fn owning_adapter_real_helper_stale_credit_before_inner_admission_finishes_locally() {
    stale_credit_route(true).await;
}

async fn stale_credit_route(before_admission: bool) {
    tokio::time::timeout(Duration::from_secs(9), async {
        let pause = Arc::new(PausedCredit::at_admission(before_admission));
        let opening_pause = pause.clone();
        let opening =
            tokio::spawn(async move { Harness::open_with_pause(Some(opening_pause)).await });
        let admitted = barrier(
            pause.admitted.subscribe(),
            "version credit admission barrier",
        )
        .await;
        assert_eq!(admitted["envelope"]["op"], "credit");
        assert_eq!(admitted["envelope"]["payload"]["batches"], 1);
        assert_eq!(admitted["envelope"]["payload"]["discard"], false);
        let candidate = admitted["envelope"]["id"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let version_wire = admitted["envelope"]["payload"]["target"]
            .as_str()
            .unwrap()
            .to_owned();
        let candidate_string = candidate.to_string();

        // The SDK stream watcher was installed before startup. Releasing EOF
        // makes the real Helper send the original version terminal while the
        // writer is suspended either before inner admission (like an awaited
        // Guard scan) or after its intent has already been durably recorded.
        fs::write(
            pause.scripts.get().unwrap().join("release-version"),
            b"release",
        )
        .unwrap();
        let original_version = barrier(
            pause.version_terminal.subscribe(),
            "original version terminal barrier",
        )
        .await;
        assert!(original_version.ok);
        assert_eq!(
            original_version.certainty,
            crate::drivers::snowflake_process::Certainty::StatementSucceeded
        );
        assert_eq!(original_version.result.as_ref().unwrap()["rowsSeen"], 1);
        assert_eq!(original_version.statements.len(), 1);
        pause.release.add_permits(1);
        let local = barrier(
            pause.local_terminal.subscribe(),
            "inactive admitted credit completion barrier",
        )
        .await;
        assert_eq!(local["operation"], "credit");
        assert_eq!(local["logicalId"], admitted["logicalId"]);
        assert_eq!(local["ticket"], admitted["ticket"]);
        assert_eq!(local["admitted"], true);
        assert!(
            local["wireId"].is_null(),
            "no byte was assigned to an actual wire ID"
        );
        assert_eq!(
            local["candidateWireId"].as_str(),
            Some(candidate_string.as_str())
        );
        assert_eq!(local["target"], admitted["target"]);
        assert_eq!(local["report"]["ok"], true);
        assert_eq!(local["report"]["certainty"], "NotSubmitted");
        assert_eq!(local["report"]["phase"], "Queued");
        assert_eq!(local["report"]["result"], json!({"inactive":true}));
        assert_eq!(local["report"]["statements"], 0);
        assert_eq!(local["report"]["hasAck"], false);
        assert!(local["report"]["errorCode"].is_null());
        assert!(local["report"]["retirement"].is_null());
        let harness = opening.await.expect("owned adapter startup task");
        assert_eq!(harness.queries().len(), 2);
        {
            let state = harness.policy.state.lock();
            assert!(
                state.sequence.phase_ready().is_ok(),
                "an original local inactive CREDIT must release only its own reservation"
            );
            assert_eq!(state.sequence.phase().unwrap().number(), 1);
            assert_eq!(state.sequence.next_entry().unwrap().ordinal(), 3);
            assert!(
                state.debt.is_empty(),
                "local control creates no metadata debt"
            );
            assert!(state.runtime_pin.is_none() && state.lifecycle.context().is_none());
        }
        harness
            .policy
            .finish_phase(PhaseEvidence::Startup(&harness.info))
            .await
            .unwrap();
        let request = harness.create_request();
        let preflight = harness
            .driver
            .preflight_snowflake_ddl(CONSOLE, &request)
            .await
            .expect("first preflight after locally completed stale credit");
        {
            let state = harness.policy.state.lock();
            assert!(state.sequence.phase_ready().is_ok());
            assert_eq!(state.sequence.phase().unwrap().number(), 2);
            assert_eq!(state.phase_runtime_count, 1);
        }
        harness
            .policy
            .finish_phase(PhaseEvidence::Preflight(&preflight))
            .await
            .unwrap();
        let expected = harness
            .policy
            .state
            .lock()
            .sequence
            .ledger()
            .entries()
            .iter()
            .take(14)
            .map(|entry| entry.sql().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            harness.queries(),
            expected,
            "stale credit adds no SQL or replay"
        );
        let frames = harness.frames();
        assert!(
            !frames
                .iter()
                .any(|frame| frame["id"].as_str() == Some(candidate_string.as_str())),
            "an admitted-but-unwritten candidate is never reused"
        );
        let ids = frames
            .iter()
            .map(|frame| frame["id"].as_str().unwrap().parse::<u64>().unwrap())
            .collect::<Vec<_>>();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        let first_preflight = frames
            .iter()
            .filter(|frame| frame["op"] == "query")
            .nth(1)
            .unwrap();
        assert!(
            first_preflight["id"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
                > candidate
        );
        assert_eq!(
            frames.iter().filter(|frame| frame["op"] == "query").count(),
            13
        );
        assert!(!frames.iter().any(|frame| frame["op"] == "ddl"));
        let journal = records(harness.journal.path());
        let events = journal.iter().map(|row| &row["event"]).collect::<Vec<_>>();
        let recorded_local = events
            .iter()
            .filter(|event| {
                event["origin"]["kind"] == "inactiveControl"
                    && event["candidateWireId"].as_str() == Some(candidate_string.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(recorded_local.len(), 1);
        let recorded_local = recorded_local[0];
        assert_eq!(recorded_local["origin"]["admitted"], true);
        assert_eq!(recorded_local["logicalId"], local["logicalId"]);
        assert_eq!(recorded_local["ticket"], local["ticket"]);
        assert!(recorded_local["wireId"].is_null());
        assert_eq!(recorded_local["report"]["result"], json!({"inactive":true}));
        assert_eq!(recorded_local["report"]["certainty"], "NotSubmitted");
        assert_eq!(recorded_local["report"]["phase"], "Queued");
        assert_eq!(recorded_local["admission"]["wireId"], candidate);
        assert_eq!(
            recorded_local["admission"]["kind"],
            if before_admission {
                json!({"inactiveControl":"Credit"})
            } else {
                json!({"control":"Credit"})
            }
        );
        let version = events
            .iter()
            .find(|event| {
                event["wireId"] == version_wire && event["report"]["statements"].is_array()
            })
            .unwrap();
        assert_eq!(
            version["report"]["result"],
            original_version.result.as_ref().unwrap().clone()
        );
        assert_eq!(
            version["report"]["statements"][0]["sdkRequestId"],
            original_version.statements[0].sdk_request_id
        );
        let next_sql = events
            .iter()
            .find(|event| event["kind"]["sql"]["ordinal"] == 3)
            .unwrap();
        assert!(next_sql["wireId"].as_u64().unwrap() > candidate);
        assert_eq!(
            events
                .iter()
                .filter(|event| event["type"] == "validatedPhase")
                .map(|event| event["phase"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(harness.policy.state.lock().pending.is_empty());
        harness.close(false).await;
        assert_eq!(
            harness.queries(),
            expected,
            "settlement and close submit no SQL"
        );
        let after = records(harness.journal.path());
        assert_eq!(
            after
                .iter()
                .filter(|row| row["event"]["origin"]["kind"] == "inactiveControl"
                    && row["event"]["candidateWireId"].as_str() == Some(candidate_string.as_str()))
                .count(),
            1
        );
    })
    .await
    .expect("deterministic stale-credit route and real child cleanup bounded to nine seconds");
}
