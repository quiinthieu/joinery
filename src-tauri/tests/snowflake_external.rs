#![cfg(all(unix, feature = "test-harness"))]
//! Local child-process adapter checks using the real helper and an injected fake SDK.
//! These do not authenticate to Snowflake or qualify server behavior, TLS, retries, or writes.
use joinery_core::{
    domain::*,
    drivers::{
        Driver, QuerySink,
        snowflake_external::{ExternalBrowserOptions, SnowflakeExternal},
        snowflake_process::{CleanupObserver, Launch},
    },
    params,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

const LIMIT: Duration = Duration::from_secs(10);
const FAKE: &str = r#"
import { Readable } from 'node:stream';
import { writeFileSync, appendFileSync } from 'node:fs';
const { Helper } = await import(HELPER_URL);
const { Decoder, writer } = await import(FRAMING_URL);
let next = 0, executions = 0;
const uuid = n => `00000000-0000-4000-8000-${n.toString(16).padStart(12,'0')}`;
const sdk = { createConnection(options) {
  if (options.host !== 'org-account.region.privatelink.snowflakecomputing.com' || options.account !== 'org-account.region.privatelink') throw new Error('host changed');
  return {
    connectAsync(callback) { queueMicrotask(() => callback(null)); return Promise.resolve(); },
    destroy(callback) { setTimeout(() => callback(null), 40); },
    execute(options) {
      const sql = options.sqlText, id = ++next;
      if (sql.startsWith('SELECT WAIT_FOR_CANCEL')) writeFileSync(new URL('./active',import.meta.url),'');
      if (sql === 'SELECT TRANSPORT_FAILURE') appendFileSync(new URL('./transport-submits',import.meta.url),'1');
      executions++;
      let cancelled = false;
      const identity = sql.startsWith('SELECT CURRENT_USER');
      const names = identity ? ['U','R','D','S','W','ID'] : sql === 'SELECT EXACT_VALUES' ? ['DUP','DUP','N','EMPTY','UTF8','JSON','DECIMAL','DATE'] : options.binds.length > 0 ? options.binds.map((_,i) => `B${i}`) : ['VALUE'];
      const values = identity ? [['SERVER_USER','ROLE','DB','PUBLIC','WH','SESSION']]
        : sql === 'SELECT CURRENT_VERSION()' ? [['synthetic-version']]
        : sql === 'SELECT EXACT_VALUES' ? [['9007199254740993','-99999999999999999999999999999999999999',null,'','東京😀','{"x":1}','1.2300','2026-11-09']]
        : sql === 'SELECT EXECUTION_COUNT' ? [[String(executions)]]
        : options.binds.length > 0 ? [options.binds.map(x => x === null ? null : typeof x === 'boolean' ? String(x) : x)] : [['1']];
      const statement = {
        getRequestId: () => uuid(id), getQueryId: () => uuid(id + 1000),
        getColumns: () => names.map(name => ({getName:()=>name,getType:()=> 'TEXT'})),
        streamRows() {
          if (sql === 'SELECT MANY_VALUES' || sql === 'SELECT LATE_FAILURE') return Readable.from((async function*() {
            for (let i=0;i<5000;i++) yield [String(i)];
            if (sql === 'SELECT LATE_FAILURE') throw {sqlState:'22000'};
          })());
          if (sql.startsWith('SELECT WAIT_FOR_CANCEL')) return Readable.from((async function*() {
            while (!cancelled) await new Promise(resolve=>setTimeout(resolve,5));
            await new Promise(resolve=>setTimeout(resolve,80));
            if (sql.endsWith('_SUCCESS')) { yield ['completed']; return; }
            throw {sqlState:sql.endsWith('_TRANSPORT') ? '08006' : '57014'};
          })());
          return Readable.from(values);
        },
        cancel(callback) {
          cancelled = true; callback(null);
          if(sql==='SELECT WAIT_FOR_CANCEL_UNKNOWN') setTimeout(()=>options.complete({sqlState:'08006'},statement),80);
        },
      };
      if (sql === 'SELECT TRANSPORT_FAILURE') setImmediate(() => options.complete({sqlState:'08006'},statement));
      else if(sql!=='SELECT WAIT_FOR_CANCEL_UNKNOWN') setImmediate(() => options.complete(null,statement));
      return statement;
    }
  };
}};
let stopping=false;
const send=writer(process.stdout.write.bind(process.stdout),()=>process.exit(2));
let raceId=null, heldTerminal=null;
const emit=frame=>{
  if(frame.id===raceId && frame.type==='terminal') { heldTerminal=frame; return Promise.resolve(); }
  return send(frame);
};
const retire=()=>{if(stopping)return;stopping=true;send.flush().then(()=>process.exit(0));};
const helper=new Helper(sdk,emit,retire,{clientConfigFile:'/nonexistent/synthetic',closeTimeoutMs:200});
const decoder=new Decoder();
process.stdin.on('data',chunk=>{try{for(const frame of decoder.push(chunk)) {
  if(frame.op==='query' && frame.payload.sql==='SELECT CONTROL_TERMINAL_RACE') raceId=frame.id;
  if(frame.op==='credit' && frame.payload.target===raceId) {
    if(!heldTerminal) throw new Error('terminal barrier was not reached');
    send(heldTerminal); heldTerminal=null; raceId=null;
    writeFileSync(new URL('./control-race',import.meta.url),'observed');
  }
  helper.receive(frame);
}}catch{retire();}});
process.stdin.on('end',retire);
"#;

fn profile() -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Synthetic external browser".into(),
        group_id: "test".into(),
        engine: EngineKind::Snowflake,
        host: "org-account.region.privatelink.snowflakecomputing.com".into(),
        port: 443,
        database: "DB".into(),
        username: "different-login@example.test".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::ReadOnly,
        tls_mode: TlsMode::VerifyFull,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: Some(SnowflakeConfig {
            authentication: SnowflakeAuthentication::ExternalBrowser,
            role: "ROLE".into(),
            warehouse: "WH".into(),
            schema: "PUBLIC".into(),
        }),
    }
}
fn launch() -> (Launch, tempfile::TempDir) {
    let node = PathBuf::from(
        std::env::var_os("JOINERY_TEST_NODE").expect("explicit JOINERY_TEST_NODE required"),
    );
    assert!(node.is_absolute() && node.is_file());
    let scripts = tempfile::tempdir().unwrap();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let url = |name| {
        url::Url::from_file_path(repo.join("snowflake-helper/src").join(name))
            .unwrap()
            .to_string()
    };
    let script = FAKE
        .replace(
            "HELPER_URL",
            &serde_json::to_string(&url("helper.mjs")).unwrap(),
        )
        .replace(
            "FRAMING_URL",
            &serde_json::to_string(&url("framing.mjs")).unwrap(),
        );
    let file = scripts.path().join("fake-sdk.mjs");
    std::fs::write(&file, script).unwrap();
    (
        Launch::new(
            node,
            Some(file),
            scripts.path().to_owned(),
            tempfile::tempdir().unwrap(),
        )
        .unwrap(),
        scripts,
    )
}
struct Harness {
    driver: Arc<SnowflakeExternal>,
    cleanup: CleanupObserver,
    _scripts: tempfile::TempDir,
}
impl Harness {
    async fn open() -> Self {
        let (launch, scripts) = launch();
        let cleanup = launch.cleanup_observer();
        let (driver, info) = tokio::time::timeout(
            LIMIT,
            SnowflakeExternal::open(
                &profile(),
                "synthetic-session",
                launch,
                ExternalBrowserOptions::default(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(info.server_version, "synthetic-version");
        assert!(info.capabilities.sql && info.capabilities.cancellation);
        assert!(
            !info.capabilities.transactions
                && !info.capabilities.table_browsing
                && !info.capabilities.table_editing
        );
        assert_eq!(driver.verified_identity()["user"], "SERVER_USER");
        Self {
            driver,
            cleanup,
            _scripts: scripts,
        }
    }
    async fn close(&self) {
        let mut shutdown = self.driver.shutdown_receiver();
        self.driver.close();
        assert!(self.driver.is_closed());
        tokio::time::timeout(LIMIT, async {
            while shutdown.borrow().is_none() {
                shutdown.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(shutdown.borrow().as_ref().unwrap().logout_acknowledged);
        let cleanup = self.cleanup.wait(LIMIT).await.unwrap();
        assert!(cleanup.child_started && cleanup.reaped && cleanup.temporary_root_removed);
    }
}
#[derive(Default)]
struct Sink {
    columns: Vec<ColumnInfo>,
    rows: Vec<TextRow>,
    completed: Vec<u64>,
    limits: usize,
    stop_after_first: bool,
    fail_storage: bool,
}
impl QuerySink for Sink {
    fn limit(&mut self, _: &str) {
        self.limits += 1;
    }
    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()> {
        self.columns = columns;
        Ok(())
    }
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        if self.fail_storage {
            return Err(AppError::internal("synthetic result storage failure"));
        }
        self.rows.extend(rows);
        Ok(!self.stop_after_first)
    }
    fn complete(&mut self, count: u64) -> AppResult<()> {
        self.completed.push(count);
        Ok(())
    }
}
async fn execute(driver: &SnowflakeExternal, sql: &str, sink: &mut Sink) -> AppResult<u64> {
    tokio::time::timeout(LIMIT, driver.execute(sql, StatementKind::Read, sink))
        .await
        .unwrap()
}
async fn count(driver: &SnowflakeExternal) -> u64 {
    let mut sink = Sink::default();
    execute(driver, "SELECT EXECUTION_COUNT", &mut sink)
        .await
        .unwrap();
    sink.rows[0][0].as_ref().unwrap().parse().unwrap()
}

#[tokio::test]
async fn exact_values_duplicate_columns_and_credit_driven_stream() {
    let h = Harness::open().await;
    let mut sink = Sink::default();
    assert_eq!(
        execute(&h.driver, "SELECT EXACT_VALUES", &mut sink)
            .await
            .unwrap(),
        1
    );
    assert_eq!(sink.columns[0].name, sink.columns[1].name);
    assert_ne!(sink.columns[0].id, sink.columns[1].id);
    assert!(sink.columns.iter().all(|c| !c.editable && !c.insertable));
    assert_eq!(
        sink.rows[0],
        vec![
            Some("9007199254740993".into()),
            Some("-99999999999999999999999999999999999999".into()),
            None,
            Some("".into()),
            Some("東京😀".into()),
            Some("{\"x\":1}".into()),
            Some("1.2300".into()),
            Some("2026-11-09".into())
        ]
    );
    let mut many = Sink::default();
    assert_eq!(
        execute(&h.driver, "SELECT MANY_VALUES", &mut many)
            .await
            .unwrap(),
        5000
    );
    assert_eq!(many.rows.len(), 5000);
    assert_eq!(many.rows[4999][0].as_deref(), Some("4999"));
    assert_eq!(many.completed, [5000]);
    h.close().await;
}

#[tokio::test]
async fn sink_cutoff_drains_terminal_and_late_error_is_never_completed_or_retried() {
    let h = Harness::open().await;
    let mut cutoff = Sink {
        stop_after_first: true,
        ..Sink::default()
    };
    assert_eq!(
        execute(&h.driver, "SELECT MANY_VALUES", &mut cutoff)
            .await
            .unwrap(),
        5000
    );
    assert!(cutoff.rows.len() <= 128);
    assert!(!cutoff.rows.is_empty());
    assert!(cutoff.limits > 0);
    assert_eq!(cutoff.completed, [5000]);
    let before = count(&h.driver).await;
    let mut late = Sink {
        stop_after_first: true,
        ..Sink::default()
    };
    assert_eq!(
        execute(&h.driver, "SELECT LATE_FAILURE", &mut late)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert!(late.completed.is_empty());
    assert_eq!(count(&h.driver).await, before + 2);
    assert!(!h.driver.is_closed());
    h.close().await;
}

#[tokio::test]
async fn sink_io_failure_preserves_known_terminal_and_usable_session() {
    let h = Harness::open().await;
    let mut sink = Sink {
        fail_storage: true,
        ..Sink::default()
    };
    let error = execute(&h.driver, "SELECT MANY_VALUES", &mut sink)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Internal);
    assert_eq!(error.message, "synthetic result storage failure");
    assert!(sink.completed.is_empty());
    assert!(!h.driver.is_closed());
    h.driver.health().await.unwrap();
    h.close().await;
}

#[tokio::test]
async fn stale_credit_and_discard_rejections_preserve_original_success() {
    for stop_after_first in [false, true] {
        let h = Harness::open().await;
        let mut sink = Sink {
            stop_after_first,
            ..Sink::default()
        };
        assert_eq!(
            execute(&h.driver, "SELECT CONTROL_TERMINAL_RACE", &mut sink)
                .await
                .unwrap(),
            1
        );
        assert_eq!(sink.completed, [1]);
        assert_eq!(
            std::fs::read_to_string(h._scripts.path().join("control-race")).unwrap(),
            "observed"
        );
        assert!(!h.driver.is_closed());
        h.driver.health().await.unwrap();
        h.close().await;
    }
}

#[tokio::test]
async fn mutation_or_forged_classification_never_reaches_sdk() {
    let h = Harness::open().await;
    let before = count(&h.driver).await;
    for (sql, kind) in [
        ("DELETE FROM CUSTOMERS", StatementKind::Read),
        ("SELECT 1; SELECT 2", StatementKind::Read),
        ("SELECT 1", StatementKind::Write),
    ] {
        assert!(
            h.driver
                .execute(sql, kind, &mut Sink::default())
                .await
                .is_err()
        );
    }
    assert_eq!(count(&h.driver).await, before + 1);
    assert!(!h.driver.is_closed());
    h.close().await;
}

#[tokio::test]
async fn explicit_expected_principal_rejection_reaps_child() {
    let (launch, _scripts) = launch();
    let cleanup = launch.cleanup_observer();
    let result = SnowflakeExternal::open(
        &profile(),
        "synthetic-session",
        launch,
        ExternalBrowserOptions {
            expected_server_user: Some("WRONG_USER".into()),
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(AppError {
            code: ErrorCode::PolicyBlocked,
            ..
        })
    ));
    let receipt = cleanup.wait(LIMIT).await.unwrap();
    assert!(receipt.reaped && receipt.temporary_root_removed);
}

#[tokio::test]
async fn transport_uncertainty_retires_without_automatic_retry_or_reuse() {
    let h = Harness::open().await;
    let mut sink = Sink::default();
    assert_eq!(
        execute(&h.driver, "SELECT TRANSPORT_FAILURE", &mut sink)
            .await
            .unwrap_err()
            .code,
        ErrorCode::OutcomeUnknown
    );
    assert!(sink.completed.is_empty());
    assert!(h.driver.is_closed());
    assert_eq!(
        execute(&h.driver, "SELECT 1", &mut Sink::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::ConnectionClosed
    );
    let receipt = h.cleanup.wait(LIMIT).await.unwrap();
    assert!(receipt.reaped && receipt.temporary_root_removed);
    assert_eq!(
        std::fs::read_to_string(h._scripts.path().join("transport-submits")).unwrap(),
        "1"
    );
}

#[tokio::test]
async fn cancellation_preserves_original_database_transport_and_success_terminals() {
    for (sql, expected) in [
        ("SELECT WAIT_FOR_CANCEL", Some(ErrorCode::Database)),
        (
            "SELECT WAIT_FOR_CANCEL_UNKNOWN",
            Some(ErrorCode::OutcomeUnknown),
        ),
        (
            "SELECT WAIT_FOR_CANCEL_TRANSPORT",
            Some(ErrorCode::ConnectionClosed),
        ),
        ("SELECT WAIT_FOR_CANCEL_SUCCESS", None),
    ] {
        let h = Harness::open().await;
        let driver = h.driver.clone();
        let query = tokio::spawn(async move {
            let mut sink = Sink::default();
            let result = driver.execute(sql, StatementKind::Read, &mut sink).await;
            (result, sink)
        });
        // The synthetic child marks actual SDK dispatch, removing the scheduling race.
        tokio::time::timeout(LIMIT, async {
            while !h._scripts.path().join("active").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let started = std::time::Instant::now();
        let cancel_error = tokio::time::timeout(LIMIT, h.driver.cancel())
            .await
            .unwrap()
            .unwrap_err();
        assert!(
            started.elapsed() >= Duration::from_millis(70),
            "cancel ACK must not replace delayed original terminal"
        );
        let (result, sink) = tokio::time::timeout(LIMIT, query).await.unwrap().unwrap();
        match expected {
            Some(code) => {
                assert_eq!(result.unwrap_err().code, code);
                assert_eq!(cancel_error.code, code);
                assert!(sink.completed.is_empty());
            }
            None => {
                assert_eq!(result.unwrap(), 1);
                assert_eq!(sink.completed, [1]);
                assert_eq!(cancel_error.code, ErrorCode::Conflict);
            }
        }
        let receipt = h.cleanup.wait(LIMIT).await.unwrap();
        assert!(receipt.reaped && receipt.temporary_root_removed);
    }
}

#[test]
fn close_without_current_runtime_still_retires_retained_driver() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let harness = runtime.block_on(Harness::open());
    // Keep both the runtime and adapter Arc alive, but leave the runtime context.
    harness.driver.close();
    assert!(harness.driver.is_closed());
    let receipt = runtime.block_on(harness.cleanup.wait(LIMIT)).unwrap();
    assert!(receipt.child_started && receipt.reaped && receipt.temporary_root_removed);
    let mut shutdown = harness.driver.shutdown_receiver();
    runtime.block_on(async {
        tokio::time::timeout(LIMIT, async {
            while shutdown.borrow().is_none() {
                shutdown.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    });
    let shutdown = shutdown.borrow();
    let shutdown = shutdown.as_ref().unwrap();
    assert!(!shutdown.logout_acknowledged);
    assert!(
        shutdown
            .cleanup
            .as_ref()
            .is_some_and(|r| r.reaped && r.temporary_root_removed)
    );
}

#[tokio::test]
async fn typed_bind_values_cross_transport_without_numeric_conversion() {
    let h = Harness::open().await;
    let sql = "SELECT :big, :number, :yes, :nothing, :date, :document";
    let slots = params::extract_query_params(EngineKind::Snowflake, sql).unwrap();
    let inputs = vec![
        QueryParamInput {
            key: "big".into(),
            kind: QueryParamKind::Integer,
            value: Some("9007199254740993".into()),
        },
        QueryParamInput {
            key: "number".into(),
            kind: QueryParamKind::Decimal,
            value: Some("1.2300".into()),
        },
        QueryParamInput {
            key: "yes".into(),
            kind: QueryParamKind::Boolean,
            value: Some("true".into()),
        },
        QueryParamInput {
            key: "nothing".into(),
            kind: QueryParamKind::Boolean,
            value: None,
        },
        QueryParamInput {
            key: "date".into(),
            kind: QueryParamKind::Date,
            value: Some("2026-11-09".into()),
        },
        QueryParamInput {
            key: "document".into(),
            kind: QueryParamKind::Json,
            value: Some("{\"x\":1}".into()),
        },
    ];
    let (_, validated) = params::validate_inputs(&slots, Some(&inputs)).unwrap();
    let bound = params::compile_statement(EngineKind::Snowflake, sql, &validated, &mut 0)
        .unwrap()
        .unwrap();
    assert!(bound.rewritten_sql.contains("CAST(? AS NUMBER(38,0))"));
    assert!(bound.rewritten_sql.contains("CAST(? AS BOOLEAN)"));
    let before = count(&h.driver).await;
    for forged_sql in ["DELETE FROM CUSTOMERS", "SELECT 1", "SELECT PARSE_JSON(?)"] {
        let mut forged = bound.clone();
        forged.rewritten_sql = forged_sql.into();
        assert_eq!(
            h.driver
                .execute_bound(sql, StatementKind::Read, &forged, &mut Sink::default())
                .await
                .unwrap_err()
                .code,
            ErrorCode::PolicyBlocked
        );
    }
    let mut wrong_order = bound.clone();
    wrong_order.binds.swap(0, 1);
    assert_eq!(
        h.driver
            .execute_bound(sql, StatementKind::Read, &wrong_order, &mut Sink::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::PolicyBlocked
    );
    for (index, value) in [(0, Some("not-an-integer")), (2, Some("not-a-boolean"))] {
        let mut malformed = bound.clone();
        malformed.binds[index].value = value.map(String::from);
        assert_eq!(
            h.driver
                .execute_bound(sql, StatementKind::Read, &malformed, &mut Sink::default())
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }
    let mut forged_null_metadata = bound.clone();
    forged_null_metadata.binds[3].decimal_dims = Some((38, 0));
    assert_eq!(
        h.driver
            .execute_bound(
                sql,
                StatementKind::Read,
                &forged_null_metadata,
                &mut Sink::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::PolicyBlocked
    );
    assert_eq!(count(&h.driver).await, before + 1);
    let mut sink = Sink::default();
    h.driver
        .execute_bound(sql, StatementKind::Read, &bound, &mut sink)
        .await
        .unwrap();
    assert_eq!(
        sink.rows[0],
        vec![
            Some("9007199254740993".into()),
            Some("1.2300".into()),
            Some("true".into()),
            None,
            Some("2026-11-09".into()),
            Some("{\"x\":1}".into())
        ]
    );
    h.close().await;
}

#[tokio::test]
async fn generated_binds_preserve_batch_positional_offsets_and_repeated_named_values() {
    let h = Harness::open().await;
    let sql = "SELECT ?, :label, :label, ?";
    let inputs = [
        QueryParamInput {
            key: "?1".into(),
            kind: QueryParamKind::Text,
            value: Some("one".into()),
        },
        QueryParamInput {
            key: "label".into(),
            kind: QueryParamKind::Text,
            value: Some("same".into()),
        },
        QueryParamInput {
            key: "?2".into(),
            kind: QueryParamKind::Text,
            value: Some("two".into()),
        },
    ];
    let slots = params::extract_query_params(EngineKind::Snowflake, sql).unwrap();
    let (_, mut validated) = params::validate_inputs(&slots, Some(&inputs)).unwrap();
    for (old, new) in [("?1", "?7"), ("?2", "?8")] {
        let mut bind = validated.remove(old).unwrap();
        bind.key = new.into();
        validated.insert(new.into(), bind);
    }
    let bound = params::compile_statement(EngineKind::Snowflake, sql, &validated, &mut 6)
        .unwrap()
        .unwrap();
    assert_eq!(
        bound
            .binds
            .iter()
            .map(|bind| bind.key.as_str())
            .collect::<Vec<_>>(),
        ["?7", "label", "label", "?8"]
    );
    let mut sink = Sink::default();
    h.driver
        .execute_bound(sql, StatementKind::Read, &bound, &mut sink)
        .await
        .unwrap();
    assert_eq!(
        sink.rows[0],
        [
            Some("one".into()),
            Some("same".into()),
            Some("same".into()),
            Some("two".into())
        ]
    );
    h.close().await;
}
