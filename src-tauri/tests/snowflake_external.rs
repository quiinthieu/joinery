#![cfg(all(unix, feature = "test-harness"))]
//! Local child-process adapter checks using the real helper and an injected fake SDK.
//! These do not authenticate to Snowflake or qualify server behavior, TLS, retries, or writes.
use arrow_array::{Array, StringArray};
use arrow_ipc::reader::StreamReader;
use joinery_core::{
    domain::*,
    drivers::{
        Driver, QuerySink,
        snowflake_external::{ExternalBrowserOptions, SnowflakeExternal},
        snowflake_process::{CleanupObserver, Launch},
    },
    export::{ExportStreamWriter, ExportWriterOptions},
    params,
    results::ResultStore,
};
use sha2::{Digest, Sha256};
use std::{io::Write, path::PathBuf, sync::Arc, time::Duration};

const LIMIT: Duration = Duration::from_secs(10);
const MILLION_ROWS: usize = 1_000_000;
const STARTER: &str =
    "-- A little space to think.\nSELECT CURRENT_DATABASE(), CURRENT_USER(), CURRENT_VERSION();";
const SYNTHETIC_READ: &str = "WITH SYNTHETIC AS (SELECT ROW_NUMBER() OVER (ORDER BY SEQ4()) AS N FROM TABLE(GENERATOR(ROWCOUNT => 12))) SELECT N AS ORDINAL, CAST(9007199254740992 + N AS NUMBER(38,0)) AS EXACT_INTEGER, CASE WHEN MOD(N, 2) = 0 THEN CAST(NULL AS VARCHAR) ELSE '' END AS NULL_OR_EMPTY, REPEAT('x', 96) AS PAD FROM SYNTHETIC ORDER BY N";
const FAKE: &str = r#"
import { Readable } from 'node:stream';
import { writeFileSync, appendFileSync, existsSync, readFileSync } from 'node:fs';
const { Helper } = await import(HELPER_URL);
const { Decoder, writer } = await import(FRAMING_URL);
let next = 0, executions = 0, relationChecks = 0;
const uuid = n => `00000000-0000-4000-8000-${n.toString(16).padStart(12,'0')}`;
const sdk = { createConnection(options) {
  appendFileSync(new URL('./connections',import.meta.url),'1');
  if (options.host !== 'org-account.region.privatelink.snowflakecomputing.com' || options.account !== 'org-account.region.privatelink') throw new Error('host changed');
  return {
    connectAsync(callback) { queueMicrotask(() => callback(null)); return Promise.resolve(); },
    destroy(callback) { setTimeout(() => callback(null), 40); },
    execute(options) {
      const sql = options.sqlText, id = ++next;
      appendFileSync(new URL('./queries',import.meta.url),JSON.stringify(sql)+'\n');
      const settingsFile = new URL('./metadata-settings.json',import.meta.url);
      const settings = existsSync(settingsFile) ? JSON.parse(readFileSync(settingsFile,'utf8')) : {};
      if (sql.startsWith('SELECT WAIT_FOR_CANCEL')) writeFileSync(new URL('./active',import.meta.url),'');
      if (sql === 'SELECT TRANSPORT_FAILURE') appendFileSync(new URL('./transport-submits',import.meta.url),'1');
      executions++;
      let cancelled = false;
      const million = sql === 'SELECT SYNTHETIC_MILLION';
      const identity = sql.startsWith('SELECT CURRENT_USER');
      let names = identity ? ['U','R','D','S','W','ID'] : sql === 'SELECT EXACT_VALUES' ? ['DUP','DUP','N','EMPTY','UTF8','JSON','DECIMAL','DATE'] : million ? ['ORDINAL','EXACT_INTEGER'] : options.binds.length > 0 ? options.binds.map((_,i) => `B${i}`) : ['VALUE'];
      let values = identity ? [['SERVER_USER','ROLE','DB','PUBLIC','WH','SESSION']]
        : sql === 'SELECT CURRENT_VERSION()' ? [['synthetic-version']]
        : sql === 'SELECT EXACT_VALUES' ? [['9007199254740993','-99999999999999999999999999999999999999',null,'','東京😀','{"x":1}','1.2300','2026-11-09']]
        : sql === 'SELECT EXECUTION_COUNT' ? [[String(executions)]]
        : options.binds.length > 0 ? [options.binds.map(x => x === null ? null : typeof x === 'boolean' ? String(x) : x)] : [['1']];
      if (sql === '-- A little space to think.\nSELECT CURRENT_DATABASE(), CURRENT_USER(), CURRENT_VERSION();') {
        names = ['CURRENT_DATABASE()','CURRENT_USER()','CURRENT_VERSION()'];
        values = [['DB','SERVER_USER','synthetic-version']];
      }
      const synthetic = sql === 'WITH SYNTHETIC AS (SELECT ROW_NUMBER() OVER (ORDER BY SEQ4()) AS N FROM TABLE(GENERATOR(ROWCOUNT => 12))) SELECT N AS ORDINAL, CAST(9007199254740992 + N AS NUMBER(38,0)) AS EXACT_INTEGER, CASE WHEN MOD(N, 2) = 0 THEN CAST(NULL AS VARCHAR) ELSE \'\' END AS NULL_OR_EMPTY, REPEAT(\'x\', 96) AS PAD FROM SYNTHETIC ORDER BY N';
      if (synthetic) {
        names = ['ORDINAL','EXACT_INTEGER','NULL_OR_EMPTY','PAD'];
        values = Array.from({length:12},(_,i)=>[String(i+1),(9007199254740993n+BigInt(i)).toString(),(i+1)%2===0?null:'','x'.repeat(96)]);
      }
      const context = sql.includes(' AS PRINCIPAL');
      if (identity && !context && settings.startupIdentity) {
        values = [['SERVER_USER', 'ROLE', 'DB', 'PUBLIC', 'WH', 'SESSION'].map((value, index) => Object.hasOwn(settings.startupIdentity, index) ? settings.startupIdentity[index] : value)];
      }
      const schemas = sql.includes('.INFORMATION_SCHEMA.SCHEMATA ');
      const relations = sql.includes('.INFORMATION_SCHEMA.TABLES ');
      const columnMetadata = sql.includes('.INFORMATION_SCHEMA.COLUMNS AS C ');
      const describe = sql.startsWith('DESCRIBE TABLE ');
      const preview = sql.startsWith('SELECT "ID", "NOTES" FROM ');
      const exactRelation = relations && sql.includes(" AND TABLE_NAME = ");
      const browsing = relations && sql.includes('CREATED::VARCHAR AS CREATED');
      const columns = sql.startsWith('SELECT * FROM "DB".') && sql.endsWith(' LIMIT 0');
      if (context) {
        names = ['PRINCIPAL','ROLE_NAME','DATABASE_NAME','SCHEMA_NAME','WAREHOUSE_NAME','SESSION_ID','SEARCH_PATH','SECONDARY_ROLES'];
        values = [['SERVER_USER','ROLE',Object.hasOwn(settings,'database')?settings.database:'DB',settings.schema??'PUBLIC',settings.warehouse??'WH',settings.session??'SESSION','["DB.PUBLIC"]',JSON.stringify({roles:settings.roles??'',value:'ALL'})]];
      } else if(sql.startsWith("SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE'")) {
        names=['key','value']; values=[['QUOTED_IDENTIFIERS_IGNORE_CASE',settings.policy??'false']];
      } else if(schemas) {
        names=['CATALOG_NAME','SCHEMA_NAME']; values=[['DB','OTHER'],['DB','PUBLIC']];
      } else if(relations) {
        names=['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','TABLE_TYPE','IS_TRANSIENT','IS_TEMPORARY','IS_ICEBERG','IS_DYNAMIC','IS_HYBRID','IS_IMMUTABLE'];
        const schema=sql.includes("TABLE_SCHEMA = 'OTHER'")?'OTHER':'PUBLIC';
        const row=n=>['DB',schema,n,'BASE TABLE','NO','NO','NO','NO','NO','NO'];
        values=settings.overflow?Array.from({length:10001},(_,i)=>row('T'+i)):[row(schema==='OTHER'?'Mixed.Name':'ORDERS')];
        if(browsing) {
          names.push('CREATED','LAST_DDL');
          if(exactRelation) relationChecks++;
          values=values.map(row=>[...row,'created',settings.relationDriftAfter && relationChecks>=settings.relationDriftAfter?'changed':'last-ddl']);
          if(settings.collision) values.push([...values[0].slice(0,3),'TEMPORARY TABLE','NO','YES','NO','NO','NO','NO','created','last-ddl']);
        }
        if(settings.catalogDrift) writeFileSync(settingsFile,JSON.stringify({...settings,warehouse:'CHANGED',catalogDrift:false}));
      } else if(columnMetadata) {
        names=['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','COLUMN_NAME','ORDINAL_POSITION','COLUMN_DEFAULT','IS_NULLABLE','DATA_TYPE','DATA_TYPE_ALIAS','CHARACTER_MAXIMUM_LENGTH','CHARACTER_OCTET_LENGTH','NUMERIC_PRECISION','NUMERIC_PRECISION_RADIX','NUMERIC_SCALE','IS_IDENTITY','IDENTITY_GENERATION','IDENTITY_START','IDENTITY_INCREMENT','IDENTITY_ORDERED','COMMENT','EXPRESSION','KIND'];
        const schema=sql.includes("TABLE_SCHEMA = 'OTHER'")?'OTHER':'PUBLIC';
        const table=schema==='OTHER'?'Mixed.Name':'ORDERS';
        values=[['DB',schema,table,'ID','1','42','NO','NUMBER','BIGINT',null,null,'38','10','0','NO',null,null,null,null,'identifier',null,'COLUMN'],['DB',schema,table,'NOTES','3',null,'YES','TEXT',null,'100','400',null,null,null,'NO',null,null,null,null,null,null,'COLUMN']];
        if(settings.badOrdinal) values[1][4]='1';
      } else if(describe) {
        names=['name','type','kind','null?','default','expression','comment'];
        values=[['ID','NUMBER(38,0)','COLUMN','N','42',null,'identifier'],['NOTES','VARCHAR(100)','COLUMN','Y',null,null,null]];
      } else if(preview) {
        names=settings.previewWrongColumns?['NOTES','ID']:['ID','NOTES'];
        values=Array.from({length:settings.previewRows??2},(_,i)=>[String(i),settings.previewBytes?'x'.repeat(40000):i===0?null:'note']);
        if(settings.previewDrift) writeFileSync(settingsFile,JSON.stringify({...settings,warehouse:'CHANGED',previewDrift:false}));
      } else if(columns) {
        names=['ID','NOTES']; values=settings.columnRows?[['1','unexpected']]:[];
      }
      const statement = {
        getRequestId: () => uuid(id), getQueryId: () => uuid(id + 1000),
        getColumns: () => names.map((name,index) => ({getName:()=>name,getType:()=> ((columns||preview)&&index===0)||(synthetic&&index<2)||(million&&index===0)?'FIXED':'TEXT'})),
        streamRows() {
          if (million) return Readable.from((async function*() {
            for (let i=1;i<=1000000;i++) yield [String(i),(9007199254740992n+BigInt(i)).toString()];
          })(), {objectMode:true, highWaterMark:1});
          if(describe && settings.holdDescribe) {
            writeFileSync(new URL('./active',import.meta.url),'');
            return Readable.from((async function*() {
              while(!existsSync(new URL('./release',import.meta.url))) await new Promise(resolve=>setTimeout(resolve,5));
              yield* values;
            })());
          }
          if (columns && settings.columnWait) {
            writeFileSync(new URL('./active',import.meta.url),'');
            return Readable.from((async function*() {
              while (!cancelled) await new Promise(resolve=>setTimeout(resolve,5));
              throw {sqlState:'57014'};
            })());
          }
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
      if ((settings.denied && relations) || (settings.inspectDenied && columnMetadata) || (settings.previewDenied && preview)) setImmediate(() => options.complete({sqlState:'42501'},statement));
      else if (sql === 'SELECT TRANSPORT_FAILURE') setImmediate(() => options.complete({sqlState:'08006'},statement));
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
            reviewed_dml: false,
            reviewed_ddl: false,
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
    fn metadata_settings(&self, value: serde_json::Value) {
        std::fs::write(
            self._scripts.path().join("metadata-settings.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    fn queries(&self) -> Vec<String> {
        std::fs::read_to_string(self._scripts.path().join("queries"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
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
        assert!(info.capabilities.relation_browsing);
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

struct ResultStoreSink<'a> {
    store: &'a ResultStore,
    result_id: Option<String>,
}

impl QuerySink for ResultStoreSink<'_> {
    fn limit(&mut self, _: &str) {}

    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()> {
        self.result_id = Some(self.store.create("synthetic-million", 0, columns, None)?.id);
        Ok(())
    }

    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        self.store.append(
            self.result_id
                .as_deref()
                .ok_or_else(|| AppError::internal("Rows arrived before columns"))?,
            rows,
        )
    }

    fn complete(&mut self, rows_seen: u64) -> AppResult<()> {
        let id = self
            .result_id
            .as_deref()
            .ok_or_else(|| AppError::internal("Result completed before columns"))?;
        if self.store.info(id)?.retained_rows as u64 != rows_seen {
            return Err(AppError::internal("Synthetic result row count diverged"));
        }
        self.store.finish(id, false)?;
        Ok(())
    }
}

#[derive(Default)]
struct HashWriter {
    digest: Sha256,
    bytes: u64,
}

impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.digest.update(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
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
async fn startup_context_mismatch_or_null_rejects_before_version_and_reaps() {
    for index in [1, 2, 3, 4] {
        for value in [serde_json::json!("DIFFERENT"), serde_json::Value::Null] {
            let (launch, scripts) = launch();
            let cleanup = launch.cleanup_observer();
            let mut identity = serde_json::Map::new();
            identity.insert(index.to_string(), value);
            std::fs::write(
                scripts.path().join("metadata-settings.json"),
                serde_json::to_vec(&serde_json::json!({"startupIdentity":identity})).unwrap(),
            )
            .unwrap();
            let opened = tokio::time::timeout(
                LIMIT,
                SnowflakeExternal::open(
                    &profile(),
                    "startup-context-rejection",
                    launch,
                    ExternalBrowserOptions::default(),
                ),
            )
            .await
            .unwrap();
            let Err(error) = opened else {
                panic!("mismatched startup field {index} was admitted");
            };
            assert_eq!(error.code, ErrorCode::PolicyBlocked);
            let queries: Vec<String> = std::fs::read_to_string(scripts.path().join("queries"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(
                queries,
                vec![
                    "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SESSION()"
                ]
            );
            let cleaned = cleanup.wait(LIMIT).await.unwrap();
            assert!(cleaned.child_started && cleaned.reaped && cleaned.temporary_root_removed);
        }
    }
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
async fn synthetic_million_rows_stream_through_driver_store_and_csv_hash() {
    let h = Harness::open().await;
    let store = ResultStore::new(8 * 1024 * 1024, MILLION_ROWS);
    store.set_temporary_disk_enabled(true);
    let mut sink = ResultStoreSink {
        store: &store,
        result_id: None,
    };
    let rows_seen = tokio::time::timeout(
        Duration::from_secs(120),
        h.driver
            .execute("SELECT SYNTHETIC_MILLION", StatementKind::Read, &mut sink),
    )
    .await
    .expect("million-row Snowflake adapter deadline")
    .unwrap();
    let result_id = sink.result_id.as_deref().unwrap();
    let result = store.info(result_id).unwrap();
    assert_eq!(rows_seen, MILLION_ROWS as u64);
    assert_eq!(result.retained_rows, MILLION_ROWS);
    assert_eq!(result.status, RetentionStatus::Complete);
    assert!(store.usage().0 <= 8 * 1024 * 1024);
    assert!(
        store.disk_usage() > 0,
        "synthetic result should exercise disk spill"
    );

    for index in [0, 1, 127, 128, 8_191, 123_456, MILLION_ROWS - 1] {
        assert_eq!(
            store.row(result_id, index).unwrap(),
            vec![
                Some((index + 1).to_string()),
                Some((9_007_199_254_740_992u64 + index as u64 + 1).to_string()),
            ]
        );
    }
    let end_page = store.range(result_id, MILLION_ROWS - 10, 512).unwrap();
    let mut reader = StreamReader::try_new(std::io::Cursor::new(end_page), None).unwrap();
    let batch = reader.next().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 10);
    let ordinals = batch.columns()[0]
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(ordinals.value(0), "999991");
    assert_eq!(ordinals.value(9), "1000000");

    let mut csv = ExportStreamWriter::try_new(
        HashWriter::default(),
        ExportWriterOptions::new(
            ExportFormat::Csv,
            result.columns.clone(),
            true,
            Some(EngineKind::Snowflake),
            None,
        ),
    )
    .unwrap();
    for batch_index in 0..store.batch_count(result_id).unwrap() {
        let rows = store
            .batch_rows(result_id, batch_index, &[0, 1])
            .unwrap()
            .unwrap();
        csv.write_rows(&rows).unwrap();
    }
    let (csv, exported_rows, exported_bytes) = csv.finish().unwrap();
    assert_eq!(exported_rows, MILLION_ROWS as u64);
    assert_eq!(exported_bytes, csv.bytes);

    let mut expected = Sha256::new();
    expected.update(b"ORDINAL,EXACT_INTEGER\n");
    for index in 0..MILLION_ROWS {
        if index > 0 {
            expected.update(b"\n");
        }
        expected.update((index + 1).to_string().as_bytes());
        expected.update(b",");
        expected.update(
            (9_007_199_254_740_992u64 + index as u64 + 1)
                .to_string()
                .as_bytes(),
        );
    }
    assert_eq!(csv.digest.finalize(), expected.finalize());

    store.release(result_id);
    assert_eq!(store.usage(), (0, 0));
    assert_eq!(store.disk_usage(), 0);
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

#[tokio::test]
async fn sdk_completion_uses_one_existing_connection_and_resolved_zero_row_columns() {
    let h = Harness::open().await;
    let before = h.queries().len();
    let identity = h.driver.completion_identity().await.unwrap();
    assert_eq!(identity.database, "DB");
    assert!(
        identity
            .context_fingerprint
            .as_ref()
            .is_some_and(|s| s.len() == 64)
    );
    let catalog = h.driver.completion_catalog(&identity).await.unwrap();
    assert_eq!(catalog.tables.len(), 2);
    let orders = TableRef {
        schema: "PUBLIC".into(),
        name: "ORDERS".into(),
    };
    assert_eq!(catalog.unqualified_tables, vec![orders.clone()]);
    assert!(catalog.tables.iter().any(|t| t.table.name == "Mixed.Name"));
    let columns = h.driver.completion_schema(&orders).await.unwrap();
    assert_eq!(
        columns
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["ID", "NOTES"]
    );
    assert_eq!(columns.columns[0].database_type, "FIXED");
    assert!(!columns.editable && columns.columns.iter().all(|c| !c.editable));
    assert!(columns.primary_key.is_empty());
    assert_eq!(
        h.driver.inspection_schema(&orders).await.unwrap_err().code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        h.driver.catalog().await.unwrap_err().code,
        ErrorCode::Unsupported
    );
    // This metadata fixture intentionally has no transaction-observer result
    // shapes. An incomplete observation must clear verified runtime context.
    let state = h.driver.session_state("synthetic-session").await.unwrap();
    assert_eq!(state.transaction_mode, TransactionMode::Unknown);
    assert_eq!(state.transaction_state, TransactionState::Unknown);
    assert_eq!(
        state.transaction_observation,
        Some(TransactionObservation::Unavailable)
    );
    assert!(state.active_context.is_none() && state.context_fingerprint.is_none());
    assert!(state.database.is_empty() && state.role.is_none() && state.search_path.is_empty());
    let queries = h.queries();
    assert!(
        queries[before..]
            .iter()
            .all(|sql| sql.starts_with("SELECT ") || sql.starts_with("SHOW PARAMETERS "))
    );
    assert!(
        queries
            .iter()
            .any(|sql| sql == "SELECT * FROM \"DB\".\"PUBLIC\".\"ORDERS\" LIMIT 0")
    );
    assert!(
        queries
            .iter()
            .filter(|sql| sql.contains(".INFORMATION_SCHEMA.TABLES "))
            .all(|sql| sql.contains("TABLE_SCHEMA = ") && sql.ends_with("LIMIT 10001"))
    );
    assert_eq!(
        std::fs::read_to_string(h._scripts.path().join("connections")).unwrap(),
        "1"
    );
    h.close().await;
}

#[tokio::test]
async fn sdk_completion_fingerprint_tracks_hidden_context_and_rejects_old_identity() {
    let h = Harness::open().await;
    let original = h.driver.completion_identity().await.unwrap();
    for settings in [
        serde_json::json!({"warehouse":"OTHER_WH"}),
        serde_json::json!({"schema":"OTHER"}),
        serde_json::json!({"roles":"OTHER_ROLE"}),
    ] {
        h.metadata_settings(settings);
        let changed = h.driver.completion_identity().await.unwrap();
        assert_ne!(original.context_fingerprint, changed.context_fingerprint);
        let error = match h.driver.completion_catalog(&original).await {
            Err(error) => error,
            Ok(_) => panic!("old completion identity accepted"),
        };
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(!h.driver.is_closed());
    }
    h.metadata_settings(serde_json::json!({}));
    assert_eq!(h.driver.completion_identity().await.unwrap(), original);
    h.close().await;
}

#[tokio::test]
async fn sdk_metadata_rejects_context_drift_and_session_generation_change() {
    let h = Harness::open().await;
    let identity = h.driver.completion_identity().await.unwrap();
    h.metadata_settings(serde_json::json!({"catalogDrift":true}));
    let error = match h.driver.completion_catalog(&identity).await {
        Err(error) => error,
        Ok(_) => panic!("mixed-context catalog published"),
    };
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.completion_invalidated, Some(true));
    assert!(!h.driver.is_closed());
    h.metadata_settings(serde_json::json!({"session":"replacement-generation"}));
    assert_eq!(
        h.driver.completion_identity().await.unwrap_err().code,
        ErrorCode::ConnectionClosed
    );
    assert!(h.driver.is_closed());
    let before = h.queries().len();
    assert_eq!(
        h.driver.completion_identity().await.unwrap_err().code,
        ErrorCode::ConnectionClosed
    );
    assert_eq!(h.queries().len(), before);
    h.close().await;
}

#[tokio::test]
async fn sdk_metadata_never_returns_partial_overflow_denied_or_column_rows() {
    let h = Harness::open().await;
    let identity = h.driver.completion_identity().await.unwrap();
    for (settings, expected) in [
        (
            serde_json::json!({"overflow":true}),
            ErrorCode::ResultLimitReached,
        ),
        (serde_json::json!({"denied":true}), ErrorCode::Database),
    ] {
        h.metadata_settings(settings);
        let error = match h.driver.completion_catalog(&identity).await {
            Err(error) => error,
            Ok(_) => panic!("incomplete catalog returned"),
        };
        assert_eq!(error.code, expected);
        assert!(!h.driver.is_closed());
    }
    h.metadata_settings(serde_json::json!({"columnRows":true}));
    let table = TableRef {
        schema: "PUBLIC".into(),
        name: "ORDERS".into(),
    };
    assert_eq!(
        h.driver.completion_schema(&table).await.unwrap_err().code,
        ErrorCode::ResultLimitReached
    );
    h.metadata_settings(serde_json::json!({"policy":"true"}));
    assert_eq!(
        h.driver.completion_identity().await.unwrap_err().code,
        ErrorCode::Unsupported
    );
    h.metadata_settings(serde_json::json!({"database":null}));
    let before = h.queries().len();
    let error = h.driver.completion_schema(&table).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert_eq!(error.completion_invalidated, Some(true));
    assert!(
        h.queries()[before..]
            .iter()
            .all(|sql| !sql.ends_with(" LIMIT 0"))
    );
    h.metadata_settings(serde_json::json!({}));
    assert_eq!(h.driver.completion_identity().await.unwrap(), identity);
    h.close().await;
}

#[tokio::test]
async fn sdk_metadata_cannot_interleave_an_active_editor_read() {
    let h = Harness::open().await;
    let driver = h.driver.clone();
    let read = tokio::spawn(async move {
        let mut sink = Sink::default();
        execute(&driver, "SELECT WAIT_FOR_CANCEL", &mut sink).await
    });
    tokio::time::timeout(LIMIT, async {
        while !h._scripts.path().join("active").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let before = h.queries().len();
    assert_eq!(
        h.driver.completion_identity().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), before);
    // The synthetic SQLSTATE 57014 remains the original known database error;
    // a cancel acknowledgement must not replace that terminal evidence.
    assert_eq!(
        h.driver.cancel().await.unwrap_err().code,
        ErrorCode::Database
    );
    assert_eq!(read.await.unwrap().unwrap_err().code, ErrorCode::Database);
    h.close().await;
}

#[tokio::test]
async fn sdk_cancelled_metadata_preserves_known_error_and_invalidates_context() {
    let h = Harness::open().await;
    h.metadata_settings(serde_json::json!({"columnWait":true}));
    let driver = h.driver.clone();
    let read = tokio::spawn(async move {
        driver
            .completion_schema(&TableRef {
                schema: "PUBLIC".into(),
                name: "ORDERS".into(),
            })
            .await
    });
    tokio::time::timeout(LIMIT, async {
        while !h._scripts.path().join("active").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        h.driver.cancel().await.unwrap_err().code,
        ErrorCode::Database
    );
    let error = tokio::time::timeout(LIMIT, read)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Database);
    assert_eq!(error.completion_invalidated, Some(true));
    assert!(h.driver.is_closed());
    assert_eq!(
        std::fs::read_to_string(h._scripts.path().join("connections")).unwrap(),
        "1"
    );
    h.close().await;
}

#[tokio::test]
async fn sdk_starter_executes_once_and_returns_observer_values() {
    let h = Harness::open().await;
    let before = h.queries().len();
    let mut sink = Sink::default();
    assert_eq!(execute(&h.driver, STARTER, &mut sink).await.unwrap(), 1);
    assert_eq!(&h.queries()[before..], &[STARTER]);
    assert_eq!(sink.columns.len(), 3);
    assert_eq!(
        sink.rows,
        vec![vec![
            Some("DB".into()),
            Some("SERVER_USER".into()),
            Some("synthetic-version".into()),
        ]]
    );
    assert_eq!(sink.completed, [1]);
    let after_starter = h.queries().len();
    for sql in [
        "SELECT \"CURRENT_USER\"()",
        "SELECT PUBLIC.CURRENT_USER()",
        "SELECT CURRENT_USER(1)",
        "SELECT CURRENT_USER() OVER ()",
        "SELECT {fn CURRENT_USER()}",
        "SELECT \"CURRENT_VERSION\"()",
        "SELECT PUBLIC.CURRENT_VERSION()",
        "SELECT CURRENT_VERSION(1)",
        "SELECT CURRENT_VERSION(DISTINCT)",
        "SELECT CURRENT_VERSION() FILTER (WHERE true)",
        "SELECT CURRENT_VERSION() WITHIN GROUP (ORDER BY 1)",
        "SELECT CURRENT_VERSION() IGNORE NULLS",
        "SELECT CURRENT_VERSION()(1)",
        "SELECT {fn CURRENT_VERSION()}",
        "SELECT CURRENT_UNREVIEWED()",
    ] {
        let mut rejected_sink = Sink::default();
        let error = execute(&h.driver, sql, &mut rejected_sink)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PolicyBlocked, "{sql}");
        assert!(rejected_sink.columns.is_empty() && rejected_sink.rows.is_empty());
        assert_eq!(h.queries().len(), after_starter, "{sql}");
    }
    assert!(!h.driver.is_closed());
    h.close().await;
}

#[tokio::test]
async fn sdk_synthetic_read_preserves_values_and_rejects_unreviewed_calls_before_dispatch() {
    let h = Harness::open().await;
    let before = h.queries().len();
    let mut sink = Sink::default();
    assert_eq!(
        execute(&h.driver, SYNTHETIC_READ, &mut sink).await.unwrap(),
        12
    );
    assert_eq!(&h.queries()[before..], &[SYNTHETIC_READ]);
    assert_eq!(sink.columns.len(), 4);
    for (index, row) in sink.rows.iter().enumerate() {
        let n = index + 1;
        assert_eq!(
            row,
            &vec![
                Some(n.to_string()),
                Some((9_007_199_254_740_992_u64 + n as u64).to_string()),
                (n % 2 != 0).then(String::new),
                Some("x".repeat(96)),
            ]
        );
    }
    assert_eq!(sink.rows.len(), 12);
    assert_eq!(sink.completed, [12]);
    let after = h.queries().len();
    for sql in [
        "SELECT MOD(UNKNOWN_FUNCTION(),2)",
        "SELECT PUBLIC.MOD(1,2)",
        "SELECT \"REPEAT\"('x',2)",
        "SELECT * FROM TABLE(PUBLIC.GENERATOR(ROWCOUNT => 12))",
        "SELECT * FROM TABLE(UNKNOWN_GENERATOR(ROWCOUNT => 12))",
        "SELECT REPEAT('x',2); DELETE FROM T WHERE ID=1",
    ] {
        let mut rejected_sink = Sink::default();
        assert_eq!(
            execute(&h.driver, sql, &mut rejected_sink)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PolicyBlocked,
            "{sql}"
        );
        assert!(rejected_sink.columns.is_empty() && rejected_sink.rows.is_empty());
        assert_eq!(h.queries().len(), after, "{sql}");
    }
    assert!(!h.driver.is_closed());
    h.close().await;
}

#[tokio::test]
async fn sdk_history_relative_forms_remain_blocked_without_helper_dispatch() {
    let h = Harness::open().await;
    h.driver.completion_identity().await.unwrap();
    let before = h.queries().len();
    for sql in [
        "SELECT LAST_QUERY_ID()",
        "SELECT LAST_QUERY_ID(-1)",
        "SELECT LAST_QUERY_ID(1)",
        "SELECT * FROM TABLE(RESULT_SCAN())",
        "SELECT * FROM TABLE(RESULT_SCAN(-1))",
        "SELECT * FROM TABLE(RESULT_SCAN(1))",
        "SELECT * FROM TABLE(RESULT_SCAN(LAST_QUERY_ID()))",
    ] {
        let error = execute(&h.driver, sql, &mut Sink::default())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PolicyBlocked, "{sql}");
    }
    assert_eq!(h.queries().len(), before);
    assert!(!h.driver.is_closed());
    h.close().await;
}

fn browse_request(catalog: &BrowseCatalog) -> InspectRelationRequest {
    InspectRelationRequest {
        revision: catalog.revision.clone(),
        target_id: catalog
            .relations
            .iter()
            .find(|target| target.table.schema == "PUBLIC")
            .unwrap()
            .target_id
            .clone(),
    }
}
fn preview_request(
    request: &InspectRelationRequest,
    inspection: &RelationInspection,
) -> PreviewRelationRequest {
    PreviewRelationRequest {
        revision: request.revision.clone(),
        target_id: request.target_id.clone(),
        inspection_revision: inspection.revision.clone(),
    }
}

#[tokio::test]
async fn sdk_relation_workflow_inspects_gapped_ordinals_and_bounded_preview_on_one_session() {
    let h = Harness::open().await;
    let catalog = h.driver.relation_catalog().await.unwrap();
    assert_eq!(catalog.database, "DB");
    assert_eq!(catalog.relations.len(), 2);
    assert!(
        catalog
            .relations
            .iter()
            .all(|r| r.availability == RelationAvailability::Available)
    );
    let request = browse_request(&catalog);
    let inspection = h.driver.inspect_relation(&request).await.unwrap();
    assert_eq!(
        inspection.constraints,
        ConstraintInspectionStatus::NotFetched
    );
    assert_eq!(
        inspection
            .columns
            .iter()
            .map(|c| c.ordinal)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(inspection.columns[0].full_type, "NUMBER(38,0)");
    assert_eq!(
        inspection.columns[0].default_expression.as_deref(),
        Some("42")
    );
    assert!(!inspection.columns[0].nullable);
    assert_eq!(
        inspection.columns[1].character_maximum_length.as_deref(),
        Some("100")
    );
    h.metadata_settings(serde_json::json!({"previewRows":501}));
    let preview = h
        .driver
        .preview_relation(&preview_request(&request, &inspection))
        .await
        .unwrap();
    assert_eq!(preview.rows.len(), 500);
    assert!(preview.truncated);
    assert_eq!(preview.rows[0], [Some("0".into()), None]);
    assert!(
        preview
            .columns
            .iter()
            .all(|column| !column.editable && !column.insertable)
    );
    assert_eq!(
        std::fs::read_to_string(h._scripts.path().join("connections")).unwrap(),
        "1"
    );
    let queries = h.queries();
    assert!(
        queries.iter().any(
            |sql| sql == "SELECT \"ID\", \"NOTES\" FROM \"DB\".\"PUBLIC\".\"ORDERS\" LIMIT 501"
        )
    );
    assert!(
        queries
            .iter()
            .all(|sql| !sql.starts_with("USE ") && !sql.starts_with("COMMIT"))
    );
    h.close().await;
}

#[tokio::test]
async fn sdk_relation_stale_tokens_and_failed_refresh_dispatch_no_target_sql() {
    let h = Harness::open().await;
    let first = h.driver.relation_catalog().await.unwrap();
    let old = browse_request(&first);
    let first_inspection = h.driver.inspect_relation(&old).await.unwrap();
    let second = h.driver.relation_catalog().await.unwrap();
    assert_ne!(first.revision, second.revision);
    let before = h.queries().len();
    assert_eq!(
        h.driver.inspect_relation(&old).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        h.driver
            .preview_relation(&preview_request(&old, &first_inspection))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let forged = InspectRelationRequest {
        revision: second.revision.clone(),
        target_id: "DB.PUBLIC.ORDERS; DROP TABLE ORDERS".into(),
    };
    assert_eq!(
        h.driver.inspect_relation(&forged).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), before);
    h.metadata_settings(serde_json::json!({"denied":true}));
    assert_eq!(
        h.driver.relation_catalog().await.unwrap_err().code,
        ErrorCode::Database
    );
    let before = h.queries().len();
    assert_eq!(
        h.driver
            .inspect_relation(&browse_request(&second))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), before);
    h.close().await;
}

#[tokio::test]
async fn sdk_relation_failed_reinspection_forgets_previous_preview_authority() {
    let h = Harness::open().await;
    let request = browse_request(&h.driver.relation_catalog().await.unwrap());
    let inspection = h.driver.inspect_relation(&request).await.unwrap();
    h.metadata_settings(serde_json::json!({"inspectDenied":true}));
    assert_eq!(
        h.driver.inspect_relation(&request).await.unwrap_err().code,
        ErrorCode::Database
    );
    let before = h.queries().len();
    assert_eq!(
        h.driver
            .preview_relation(&preview_request(&request, &inspection))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), before);
    h.close().await;
}

#[tokio::test]
async fn sdk_relation_collision_keeps_distinct_rows_but_refuses_actions_before_sql() {
    let h = Harness::open().await;
    h.metadata_settings(serde_json::json!({"collision":true}));
    let catalog = h.driver.relation_catalog().await.unwrap();
    assert_eq!(catalog.relations.len(), 4);
    assert_eq!(
        catalog
            .relations
            .iter()
            .filter(|r| r.availability == RelationAvailability::Shadowed)
            .count(),
        2
    );
    assert_eq!(
        catalog
            .relations
            .iter()
            .filter(|r| r.availability == RelationAvailability::Ambiguous)
            .count(),
        2
    );
    let before = h.queries().len();
    for relation in catalog.relations {
        assert!(
            h.driver
                .inspect_relation(&InspectRelationRequest {
                    revision: catalog.revision.clone(),
                    target_id: relation.target_id
                })
                .await
                .is_err()
        );
    }
    assert_eq!(h.queries().len(), before);
    h.close().await;
}

#[tokio::test]
async fn sdk_relation_drift_prevents_inspection_and_preview_publication() {
    for settings in [
        serde_json::json!({"relationDriftAfter":2}),
        serde_json::json!({"badOrdinal":true}),
    ] {
        let h = Harness::open().await;
        let request = browse_request(&h.driver.relation_catalog().await.unwrap());
        h.metadata_settings(settings);
        assert!(h.driver.inspect_relation(&request).await.is_err());
        let before = h.queries().len();
        assert!(
            h.driver
                .preview_relation(&PreviewRelationRequest {
                    revision: request.revision.clone(),
                    target_id: request.target_id.clone(),
                    inspection_revision: "never-published".into()
                })
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), before);
        h.close().await;
    }
    let h = Harness::open().await;
    let request = browse_request(&h.driver.relation_catalog().await.unwrap());
    let inspection = h.driver.inspect_relation(&request).await.unwrap();
    h.metadata_settings(serde_json::json!({"previewDrift":true}));
    let error = h
        .driver
        .preview_relation(&preview_request(&request, &inspection))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.completion_invalidated, Some(true));
    let before = h.queries().len();
    assert!(h.driver.inspect_relation(&request).await.is_err());
    assert_eq!(h.queries().len(), before);
    h.close().await;
}

#[tokio::test]
async fn sdk_relation_preview_rejects_byte_overflow_wrong_columns_and_permission_failure() {
    let h = Harness::open().await;
    let request = browse_request(&h.driver.relation_catalog().await.unwrap());
    let inspection = h.driver.inspect_relation(&request).await.unwrap();
    for (settings, expected) in [
        (
            serde_json::json!({"previewRows":60,"previewBytes":true}),
            ErrorCode::ResultLimitReached,
        ),
        (
            serde_json::json!({"previewWrongColumns":true}),
            ErrorCode::Conflict,
        ),
        (
            serde_json::json!({"previewDenied":true}),
            ErrorCode::Database,
        ),
    ] {
        h.metadata_settings(settings);
        assert_eq!(
            h.driver
                .preview_relation(&preview_request(&request, &inspection))
                .await
                .unwrap_err()
                .code,
            expected
        );
    }
    assert!(!h.driver.is_closed());
    h.close().await;
}

#[tokio::test]
async fn sdk_relation_initial_database_pin_and_explicit_invalidation_fail_closed() {
    let h = Harness::open().await;
    let request = browse_request(&h.driver.relation_catalog().await.unwrap());
    h.driver.invalidate_relation_catalog();
    let before = h.queries().len();
    assert_eq!(
        h.driver.inspect_relation(&request).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), before);
    h.metadata_settings(serde_json::json!({"database":"OTHER_DATABASE"}));
    let before = h.queries().len();
    assert_eq!(
        h.driver.relation_catalog().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(
        h.queries()[before..]
            .iter()
            .all(|sql| !sql.contains("INFORMATION_SCHEMA"))
    );
    h.close().await;
}

#[tokio::test]
async fn sdk_relation_invalidation_during_inspection_cannot_publish_a_late_snapshot() {
    let h = Harness::open().await;
    let request = browse_request(&h.driver.relation_catalog().await.unwrap());
    h.metadata_settings(serde_json::json!({"holdDescribe":true}));
    let driver = h.driver.clone();
    let pending_request = request.clone();
    let pending = tokio::spawn(async move { driver.inspect_relation(&pending_request).await });
    tokio::time::timeout(LIMIT, async {
        while !h._scripts.path().join("active").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    h.driver.invalidate_relation_catalog();
    std::fs::write(h._scripts.path().join("release"), "").unwrap();
    assert_eq!(
        tokio::time::timeout(LIMIT, pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let before = h.queries().len();
    assert!(h.driver.inspect_relation(&request).await.is_err());
    assert_eq!(h.queries().len(), before);
    h.close().await;
}

#[tokio::test]
async fn sdk_relation_busy_refresh_forgets_old_targets_before_lane_acquisition() {
    let h = Harness::open().await;
    let old = browse_request(&h.driver.relation_catalog().await.unwrap());
    let driver = h.driver.clone();
    let read = tokio::spawn(async move {
        execute(&driver, "SELECT WAIT_FOR_CANCEL", &mut Sink::default()).await
    });
    tokio::time::timeout(LIMIT, async {
        while !h._scripts.path().join("active").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let before = h.queries().len();
    assert_eq!(
        h.driver.relation_catalog().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        h.driver.inspect_relation(&old).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), before);
    assert_eq!(
        h.driver.cancel().await.unwrap_err().code,
        ErrorCode::Database
    );
    assert_eq!(read.await.unwrap().unwrap_err().code, ErrorCode::Database);
    h.close().await;
}

#[tokio::test]
async fn sdk_relation_superseding_failed_inspection_cannot_restore_older_preview_authority() {
    let h = Harness::open().await;
    let request = browse_request(&h.driver.relation_catalog().await.unwrap());
    let previous = h.driver.inspect_relation(&request).await.unwrap();
    h.metadata_settings(serde_json::json!({"holdDescribe":true}));
    let driver = h.driver.clone();
    let old_request = request.clone();
    let older = tokio::spawn(async move { driver.inspect_relation(&old_request).await });
    tokio::time::timeout(LIMIT, async {
        while !h._scripts.path().join("active").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // The newer attempt replaces authority immediately, even though its lane
    // acquisition fails. The older successful reads cannot restore it later.
    let before = h.queries().len();
    assert_eq!(
        h.driver.inspect_relation(&request).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        h.driver
            .preview_relation(&preview_request(&request, &previous))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), before);
    std::fs::write(h._scripts.path().join("release"), "").unwrap();
    assert_eq!(
        tokio::time::timeout(LIMIT, older)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let before = h.queries().len();
    assert_eq!(
        h.driver
            .preview_relation(&preview_request(&request, &previous))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), before);
    h.close().await;
}
