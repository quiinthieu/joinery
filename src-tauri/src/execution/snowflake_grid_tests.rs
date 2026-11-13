//! Studio -> verified private Node -> real helper -> synthetic SDK, with real
//! SQLite batch outcome capture/settlement, protected rollback and cleanup.
//! These tests never authenticate against or execute SQL on a real account.
use super::snowflake_console_tests::FIXTURE_LANE;
use super::snowflake_grid::GridTestFault;
use super::*;
use crate::{
    drivers::{
        snowflake_batch::{Failure, FinishAction, Mode, Outcome},
        snowflake_edit_snapshot::{PreparedSnowflakeEdits, SnapshotMutation},
        snowflake_mutations::CellValue,
        snowflake_process::Launch,
    },
    secrets::MemorySecrets,
    snowflake_runtime::studio_fixture::Fixture,
};
use rusqlite::Connection;
use std::os::unix::fs::PermissionsExt;

const GRID_SYNTHETIC_SDK: &str = r#"
import { Readable } from 'node:stream';
import { appendFileSync, existsSync, writeFileSync } from 'node:fs';
const controls = new URL('../../', import.meta.url);
const marker = name => new URL(name, controls);
let sequence = 0;
const uuid = n => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
export default {
  configure() {},
  createConnection(profile) {
    if (profile.authenticator !== 'EXTERNALBROWSER' && profile.authenticator !== 'externalbrowser') {
      throw Error('incorrect route');
    }
    let currentTransaction = null;
    let disposition = 'running';
    let committedNote0 = null;
    let pendingNote0 = undefined;
    return {
      connectAsync(callback) {
        writeFileSync(marker('auth-started'), JSON.stringify({ pid: process.pid }));
        queueMicrotask(() => callback(null));
        return Promise.resolve();
      },
      destroy(callback) { setTimeout(() => callback(null), 10); },
      execute(options) {
        const sql = options.sqlText;
        const id = ++sequence;
        appendFileSync(marker('queries'), sql + '\n');
        const boundary = ['BEGIN TRANSACTION', 'COMMIT', 'ROLLBACK'].includes(sql);
        const dml = /^(INSERT INTO|UPDATE|DELETE FROM) /u.test(sql);
        const contextPrincipal = sql.startsWith('SELECT CURRENT_USER() AS PRINCIPAL');
        const schemas = sql.includes('.INFORMATION_SCHEMA.SCHEMATA ');
        const relations = sql.includes('.INFORMATION_SCHEMA.TABLES ');
        const columnMetadata = sql.includes('.INFORMATION_SCHEMA.COLUMNS AS C ');
        const describeTable = sql.startsWith('DESCRIBE TABLE ');
        const editPreview = sql.startsWith('SELECT "ID", "NOTES" FROM ');
        const zeroRows = sql.startsWith('SELECT * FROM ') && sql.endsWith(' LIMIT 0');
        let names = ['C0'];
        let types = ['TEXT'];
        let rows = [['1']];
        if (dml) {
          appendFileSync(marker('dml-binds'), JSON.stringify(options.binds) + '\n');
          writeFileSync(marker('dml-entered'), String(id));
          if (sql.startsWith('UPDATE ') && Array.isArray(options.binds) && options.binds.length >= 1) {
            if (currentTransaction !== null) pendingNote0 = options.binds[0];
            else committedNote0 = options.binds[0];
          }
          names = [sql.startsWith('INSERT') ? 'number of rows inserted' : sql.startsWith('UPDATE') ? 'number of rows updated' : 'number of rows deleted'];
          types = ['FIXED'];
          rows = [['1']];
        } else if (boundary) {
          if (sql === 'COMMIT' && pendingNote0 !== undefined) committedNote0 = pendingNote0;
          if (sql === 'COMMIT' || sql === 'ROLLBACK') pendingNote0 = undefined;
          disposition = sql === 'COMMIT' ? 'committed' : sql === 'ROLLBACK' ? 'rolled back' : 'running';
          currentTransaction = sql === 'BEGIN TRANSACTION' ? '123456789' : null;
          appendFileSync(marker('transaction-effects'), sql + '\n');
          names = [];
          types = [];
          rows = [];
        } else if (sql === 'DESCRIBE TRANSACTION 123456789') {
          names = ['id', 'session', 'state'];
          types = ['TEXT', 'TEXT', 'TEXT'];
          rows = [['123456789', 'SESSION', disposition]];
        } else if (contextPrincipal) {
          names = ['PRINCIPAL', 'ROLE_NAME', 'DATABASE_NAME', 'SCHEMA_NAME', 'WAREHOUSE_NAME', 'SESSION_ID', 'SEARCH_PATH', 'SECONDARY_ROLES'];
          types = names.map(() => 'TEXT');
          rows = [['SERVER_USER', profile.role, profile.database, profile.schema, profile.warehouse, 'SESSION', JSON.stringify([`${profile.database}.PUBLIC`, `${profile.database}.OTHER`]), JSON.stringify({ roles: '', value: 'NONE' })]];
        } else if (sql.startsWith('SELECT CURRENT_USER(),')) {
          rows = [['SERVER_USER', profile.role, profile.database, profile.schema, profile.warehouse, 'SESSION']];
          if (sql.includes('CURRENT_TRANSACTION()')) rows[0].push(currentTransaction);
          names = rows[0].map((_, index) => `C${index}`);
          types = names.map(() => 'TEXT');
        } else if (sql === 'SELECT CURRENT_VERSION()') {
          rows = [['synthetic-version']];
        } else if (sql.startsWith('SHOW PARAMETERS LIKE ')) {
          const param = sql.split("'")[1];
          const val = param === 'AUTOCOMMIT' ? 'true' : 'false';
          names = ['key', 'value', 'default', 'level'];
          types = names.map(() => 'TEXT');
          rows = [[param, val, val === 'true' ? 'false' : 'true', 'ACCOUNT']];
        } else if (sql === 'SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID') {
          names = ['TRANSACTION_ID'];
          types = ['TEXT'];
          rows = [[currentTransaction]];
        } else if (schemas) {
          names = ['CATALOG_NAME', 'SCHEMA_NAME'];
          types = ['TEXT', 'TEXT'];
          rows = [[profile.database, 'OTHER'], [profile.database, 'PUBLIC']];
        } else if (relations) {
          const schema = sql.includes("TABLE_SCHEMA = 'OTHER'") ? 'OTHER' : 'PUBLIC';
          const table = schema === 'OTHER' ? 'Mixed.Name' : 'ORDERS';
          names = ['TABLE_CATALOG', 'TABLE_SCHEMA', 'TABLE_NAME', 'TABLE_TYPE', 'IS_TRANSIENT', 'IS_TEMPORARY', 'IS_ICEBERG', 'IS_DYNAMIC', 'IS_HYBRID', 'IS_IMMUTABLE', 'CREATED', 'LAST_DDL'];
          types = names.map(() => 'TEXT');
          rows = [[profile.database, schema, table, 'BASE TABLE', 'NO', 'NO', 'NO', 'NO', 'NO', 'NO', 'created', 'last-ddl']];
        } else if (columnMetadata) {
          names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','COLUMN_NAME','ORDINAL_POSITION','COLUMN_DEFAULT','IS_NULLABLE','DATA_TYPE','DATA_TYPE_ALIAS','CHARACTER_MAXIMUM_LENGTH','CHARACTER_OCTET_LENGTH','NUMERIC_PRECISION','NUMERIC_PRECISION_RADIX','NUMERIC_SCALE','IS_IDENTITY','IDENTITY_GENERATION','IDENTITY_START','IDENTITY_INCREMENT','IDENTITY_ORDERED','COMMENT','EXPRESSION','KIND'];
          types = names.map(() => 'TEXT');
          rows = [
            [profile.database,'PUBLIC','ORDERS','ID','1','42','NO','NUMBER','BIGINT',null,null,'38','10','0','NO',null,null,null,null,'identifier',null,'COLUMN'],
            [profile.database,'PUBLIC','ORDERS','NOTES','3',null,'YES','TEXT',null,'100','400',null,null,null,'NO',null,null,null,null,null,null,'COLUMN']
          ];
        } else if (describeTable) {
          names = ['name','type','kind','null?','default','expression','comment'];
          types = names.map(() => 'TEXT');
          rows = [['ID','NUMBER(38,0)','COLUMN','N','42',null,'identifier'],['NOTES','VARCHAR(100)','COLUMN','Y',null,null,null]];
        } else if (editPreview) {
          names = ['ID', 'NOTES'];
          types = ['FIXED', 'TEXT'];
          const note0 = currentTransaction !== null && pendingNote0 !== undefined ? pendingNote0 : committedNote0;
          rows = [['9007199254740993', note0], ['9007199254740993', ''], ['2', '雪']];
        } else if (zeroRows) {
          names = ['ID', 'NOTES'];
          types = ['FIXED', 'TEXT'];
          rows = [];
        }
        let cancelled = false;
        const statement = {
          getRequestId: () => uuid(id),
          getQueryId: () => uuid(id + 100000),
          getColumns: () => names.map((name, index) => ({ getName: () => name, getType: () => types[index] ?? 'TEXT' })),
          streamRows() { return Readable.from(rows); },
          cancel: callback => { cancelled = true; callback(null); },
        };
        setImmediate(() => {
          const complete = () => {
            if (dml && existsSync(marker('hold-dml-ack')) && !existsSync(marker('release-dml-ack')) && !cancelled) {
              setTimeout(complete, 5);
              return;
            }
            if (sql === 'ROLLBACK' && existsSync(marker('hold-rollback-ack')) && !existsSync(marker('release-rollback-ack')) && !cancelled) {
              setTimeout(complete, 5);
              return;
            }
            options.complete(null, statement);
          };
          complete();
        });
        return statement;
      },
    };
  },
};
"#;

fn grid_fixture() -> Fixture {
    let f = Fixture::new();
    std::fs::write(
        f.controls.join("node_modules/snowflake-sdk/index.mjs"),
        GRID_SYNTHETIC_SDK,
    )
    .unwrap();
    f
}

fn grid_profile() -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Synthetic writable grid".into(),
        group_id: "local".into(),
        engine: EngineKind::Snowflake,
        host: "org-account.snowflakecomputing.com".into(),
        port: 443,
        database: "DB".into(),
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
            reviewed_ddl: false,
            reviewed_console: false,
            reviewed_console_timeout_seconds: None,
            role: "ROLE".into(),
            warehouse: "WH".into(),
            schema: "PUBLIC".into(),
        }),
    }
}

fn new_studio(f: &Fixture) -> Arc<Studio> {
    Studio::with_snowflake_runtime(
        &f.directory.path().join("workspace"),
        Arc::new(MemorySecrets::default()),
        Arc::new(DisabledFileDialog),
        Some(f.runtime.clone()),
    )
    .unwrap()
}

fn sqlite_db(f: &Fixture) -> Connection {
    Connection::open(f.directory.path().join("workspace/joinery.sqlite")).unwrap()
}

async fn connected(f: &Fixture) -> (Arc<Studio>, SessionInfo) {
    let studio = new_studio(f);
    let saved = studio.save_profile(grid_profile(), None).unwrap();
    let info = studio
        .connect(&saved.id, ConnectionSecrets::default())
        .await
        .unwrap();
    assert!(!info.console_mode);
    (studio, info)
}

fn queries(f: &Fixture) -> Vec<String> {
    std::fs::read_to_string(f.controls.join("queries"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn dml_count(f: &Fixture) -> usize {
    queries(f)
        .iter()
        .filter(|sql| {
            sql.starts_with("UPDATE ")
                || sql.starts_with("INSERT INTO ")
                || sql.starts_with("DELETE FROM ")
        })
        .count()
}

fn update_row0(value: &str) -> SnapshotMutation {
    SnapshotMutation::Update {
        row_index: 0,
        changes: vec![(1, CellValue::Text(value.into()))],
    }
}

async fn browse_and_prepare(
    studio: &Studio,
    session_id: &str,
    mutations: &[SnapshotMutation],
) -> (
    PreviewRelationRequest,
    RelationPreview,
    PreparedSnowflakeEdits,
) {
    let catalog = studio.relation_catalog(session_id).await.unwrap();
    let target_id = catalog
        .relations
        .iter()
        .find(|r| r.table.schema == "PUBLIC" && r.table.name == "ORDERS")
        .unwrap()
        .target_id
        .clone();
    let inspection = studio
        .inspect_relation(
            session_id,
            &InspectRelationRequest {
                revision: catalog.revision.clone(),
                target_id: target_id.clone(),
            },
        )
        .await
        .unwrap();
    let req = PreviewRelationRequest {
        revision: catalog.revision,
        target_id,
        inspection_revision: inspection.revision,
    };
    let preview = studio.preview_relation(session_id, &req).await.unwrap();
    let snapshot_id = preview.edit_snapshot_id.as_deref().unwrap();
    let edits = studio
        .prepare_snowflake_grid_edits(session_id, &req, snapshot_id, mutations)
        .await
        .unwrap();
    (req, preview, edits)
}

async fn wait_marker(f: &Fixture, name: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !f.controls.join(name).exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for marker {name}"));
}

#[tokio::test]
async fn grid_studio_automatic_commits_settles_evidence_and_enforces_policy() {
    let _guard = FIXTURE_LANE.lock().await;
    let f = grid_fixture();
    let (studio, info) = connected(&f).await;

    let (_, _, edits) = browse_and_prepare(
        &studio,
        &info.id,
        &[
            update_row0("雪-committed"),
            SnapshotMutation::Delete { row_index: 1 },
        ],
    )
    .await;

    let exec = studio
        .execute_snowflake_grid_edits(&info.id, edits, Mode::Automatic)
        .await
        .unwrap();
    assert_eq!(exec.outcome, Outcome::Committed);
    assert!(exec.transaction_id.is_none());
    assert!(exec.recording_error.is_none());
    assert!(studio.batch_outcomes.is_empty());
    assert!(studio.snowflake_grid.is_empty());
    assert_eq!(dml_count(&f), 2);
    assert_eq!(
        queries(&f)
            .iter()
            .filter(|s| s.as_str() == "COMMIT")
            .count(),
        1
    );

    let retained = studio
        .persistence
        .batch_evidence(&exec.operation_id)
        .unwrap()
        .unwrap();
    assert!(retained.settled && !retained.imported);

    // Metadata remains paused after write until explicit refresh.
    assert_eq!(
        studio.session_contexts(&info.id).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    studio
        .session_contexts_with_refresh(&info.id, true)
        .await
        .unwrap();
    studio.relation_catalog(&info.id).await.unwrap();
    studio.prepare_execution(&info.id, "SELECT 1").unwrap();

    // Policy gating refuses read-only or reviewed_dml=false profiles without SQL.
    let mut readonly = grid_profile();
    readonly.safe_mode = SafeMode::ReadOnly;
    readonly.snowflake.as_mut().unwrap().reviewed_dml = false;
    let saved_ro = studio.save_profile(readonly, None).unwrap();
    let info_ro = studio
        .connect(&saved_ro.id, ConnectionSecrets::default())
        .await
        .unwrap();
    let catalog_ro = studio.relation_catalog(&info_ro.id).await.unwrap();
    let target_ro = catalog_ro
        .relations
        .iter()
        .find(|r| r.table.schema == "PUBLIC")
        .unwrap()
        .target_id
        .clone();
    let inspect_ro = studio
        .inspect_relation(
            &info_ro.id,
            &InspectRelationRequest {
                revision: catalog_ro.revision.clone(),
                target_id: target_ro.clone(),
            },
        )
        .await
        .unwrap();
    let preview_req_ro = PreviewRelationRequest {
        revision: catalog_ro.revision,
        target_id: target_ro,
        inspection_revision: inspect_ro.revision,
    };
    let preview_ro = studio
        .preview_relation(&info_ro.id, &preview_req_ro)
        .await
        .unwrap();
    assert!(preview_ro.edit_snapshot_id.is_none());
    let before = queries(&f).len();
    assert_eq!(
        studio
            .prepare_snowflake_grid_edits(
                &info_ro.id,
                &preview_req_ro,
                &new_id(),
                &[update_row0("blocked")]
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::PolicyBlocked
    );
    assert_eq!(queries(&f).len(), before);

    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn grid_studio_manual_pending_blocks_ordinary_work_continues_and_finishes() {
    let _guard = FIXTURE_LANE.lock().await;
    for finish_action in [FinishAction::Commit, FinishAction::Rollback] {
        let f = grid_fixture();
        let (studio, info) = connected(&f).await;
        let session = studio.sessions.lock().get(&info.id).cloned().unwrap();
        let slot = studio.session_safety_slot(&session).unwrap();
        assert!(slot.is_available());

        let (preview_req, preview, edits) =
            browse_and_prepare(&studio, &info.id, &[update_row0("first-manual")]).await;

        let pending = studio
            .execute_snowflake_grid_edits(&info.id, edits, Mode::Manual)
            .await
            .unwrap();
        assert_eq!(pending.outcome, Outcome::Pending);
        assert!(pending.recording_error.is_none());
        let tx_id = pending.transaction_id.clone().unwrap();
        assert!(
            slot.is_available(),
            "SafetySlot remains standby-available until Rollback claims it"
        );
        assert!(!studio.snowflake_grid.is_empty());

        let page = studio
            .persistence
            .manual_transaction_evidence(&tx_id, 0, 16)
            .unwrap();
        assert_eq!(page.disposition, "open");
        assert_eq!(page.members.len(), 1);

        // Ordinary SQL, relation browsing and session_contexts_with_refresh are all blocked while Pending.
        let before = queries(&f).len();
        assert_eq!(
            studio
                .prepare_execution(&info.id, "SELECT 1")
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio.relation_catalog(&info.id).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio
                .inspect_relation(
                    &info.id,
                    &InspectRelationRequest {
                        revision: preview_req.revision.clone(),
                        target_id: preview_req.target_id.clone(),
                    },
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio
                .preview_relation(&info.id, &preview_req)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio
                .switch_session_context(&info.id, "BILLIE_TESTING")
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio
                .session_contexts_with_refresh(&info.id, true)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(queries(&f).len(), before);

        // Owner-scoped pending preview observes uncommitted row 0 ("first-manual"),
        // revokes any earlier snapshot, and prepares a fresh continuation snapshot!
        let old_snapshot_id = preview.edit_snapshot_id.as_deref().unwrap();
        assert_eq!(
            studio
                .prepare_snowflake_grid_edits(
                    &info.id,
                    &preview_req,
                    old_snapshot_id,
                    &[SnapshotMutation::Delete { row_index: 2 }],
                )
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio
                .prepare_snowflake_grid_continuation_edits(
                    &tx_id,
                    &preview_req,
                    old_snapshot_id,
                    &[SnapshotMutation::Delete { row_index: 2 }],
                )
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::Conflict
        );

        let before_pending_preview = queries(&f).len();
        let pending_preview_bad = studio
            .preview_snowflake_grid_transaction_relation(&tx_id, &preview_req)
            .await
            .unwrap();
        assert_eq!(queries(&f).len() - before_pending_preview, 12);
        assert_eq!(
            pending_preview_bad.rows[0],
            vec![Some("9007199254740993".into()), Some("first-manual".into()),]
        );
        let snap_bad = pending_preview_bad.edit_snapshot_id.as_deref().unwrap();

        // Continuation requires explicit whole-transaction rollback_prior = true.
        let cont_bad = studio
            .prepare_snowflake_grid_continuation_edits(
                &tx_id,
                &preview_req,
                snap_bad,
                &[SnapshotMutation::Delete { row_index: 2 }],
            )
            .await
            .unwrap();
        assert_eq!(
            studio
                .continue_snowflake_grid_edits(&tx_id, cont_bad, false)
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidInput
        );

        // Refresh pending preview again and update row 0 from "first-manual" to "second-manual"!
        let pending_preview_ok = studio
            .preview_snowflake_grid_transaction_relation(&tx_id, &preview_req)
            .await
            .unwrap();
        assert_eq!(
            studio
                .prepare_snowflake_grid_continuation_edits(
                    &tx_id,
                    &preview_req,
                    snap_bad,
                    &[update_row0("stale")],
                )
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::Conflict,
            "newer pending preview must revoke prior pending snapshot"
        );
        let cont_ok = studio
            .prepare_snowflake_grid_continuation_edits(
                &tx_id,
                &preview_req,
                pending_preview_ok.edit_snapshot_id.as_deref().unwrap(),
                &[update_row0("second-manual")],
            )
            .await
            .unwrap();
        let continued = studio
            .continue_snowflake_grid_edits(&tx_id, cont_ok, true)
            .await
            .unwrap();
        assert_eq!(continued.outcome, Outcome::Pending);
        assert_eq!(continued.transaction_id.as_deref(), Some(tx_id.as_str()));
        assert!(continued.recording_error.is_none());

        let before_finish = queries(&f).len();
        let finished = studio
            .finish_snowflake_grid_transaction(&tx_id, finish_action)
            .await
            .unwrap();
        assert!(finished.recording_error.is_none());
        if finish_action == FinishAction::Commit {
            assert_eq!(finished.outcome, Outcome::Committed);
        } else {
            assert_eq!(finished.outcome, Outcome::RolledBack);
            // Safety rollback performs only ROLLBACK + idle observation + DESCRIBE TRANSACTION (3 SQLs).
            assert_eq!(queries(&f).len() - before_finish, 3);
        }

        assert!(
            slot.is_available(),
            "settled finish must release session SafetySlot"
        );
        assert!(studio.batch_outcomes.is_empty());
        assert!(studio.snowflake_grid.is_empty());

        let final_page = studio
            .persistence
            .manual_transaction_evidence(&tx_id, 0, 16)
            .unwrap();
        assert_eq!(
            final_page.disposition,
            if finish_action == FinishAction::Commit {
                "committed"
            } else {
                "rolledBack"
            }
        );
        assert_eq!(final_page.members.len(), 3);

        studio
            .session_contexts_with_refresh(&info.id, true)
            .await
            .unwrap();
        let (_, post_finish_preview, _) = browse_and_prepare(
            &studio,
            &info.id,
            &[SnapshotMutation::Delete { row_index: 2 }],
        )
        .await;
        assert_eq!(
            post_finish_preview.rows[0][1].as_deref(),
            if finish_action == FinishAction::Commit {
                Some("second-manual")
            } else {
                None
            }
        );
        studio.prepare_execution(&info.id, "SELECT 1").unwrap();
        studio.shutdown_and_wait().await.unwrap();
    }
}

#[test]
fn grid_studio_unpolled_spawn_releases_guards_without_sql() {
    let f = grid_fixture();
    let setup_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (studio, info, edits, baseline) = setup_rt.block_on(async {
        let _guard = FIXTURE_LANE.lock().await;
        let (studio, info) = connected(&f).await;
        let (_, _, edits) =
            browse_and_prepare(&studio, &info.id, &[update_row0("never-polled")]).await;
        let baseline = queries(&f).len();
        (studio, info, edits, baseline)
    });

    // Spawn onto an unpolled runtime and drop it before polling the task.
    let unpolled_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    {
        let _enter = unpolled_rt.enter();
        let handle = studio
            .start_snowflake_grid_edits(&info.id, edits, Mode::Manual)
            .unwrap();
        drop(handle);
    }
    drop(unpolled_rt);

    assert_eq!(queries(&f).len(), baseline);
    assert!(studio.snowflake_grid.is_empty());
    assert!(studio.batch_outcomes.is_empty());
    let session = studio.sessions.lock().get(&info.id).cloned().unwrap();
    assert!(studio.session_safety_slot(&session).unwrap().is_available());

    setup_rt.block_on(async {
        let _guard = FIXTURE_LANE.lock().await;
        studio.relation_catalog(&info.id).await.unwrap();
        studio.shutdown_and_wait().await.unwrap();
    });
}

#[tokio::test]
async fn grid_studio_detached_caller_completes_records_original_and_preserves_rollback() {
    let _guard = FIXTURE_LANE.lock().await;
    let f = grid_fixture();
    let (studio, info) = connected(&f).await;
    let (_, _, edits) =
        browse_and_prepare(&studio, &info.id, &[update_row0("detached-caller")]).await;

    std::fs::write(f.controls.join("hold-dml-ack"), "").unwrap();
    let handle = studio
        .start_snowflake_grid_edits(&info.id, edits, Mode::Manual)
        .unwrap();
    wait_marker(&f, "dml-entered").await;
    // Drop the caller's JoinHandle mid-flight while DML is still held in the helper.
    drop(handle);

    std::fs::write(f.controls.join("release-dml-ack"), "").unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !studio.dml_tasks.is_empty() || studio.batch_outcomes.pending_count() > 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    let db = sqlite_db(&f);
    let tx_id: String = db
        .query_row(
            "SELECT transaction_id FROM manual_transactions_v8",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let page = studio
        .persistence
        .manual_transaction_evidence(&tx_id, 0, 16)
        .unwrap();
    assert_eq!(page.disposition, "open");
    assert_eq!(page.members.len(), 1);
    assert!(
        studio
            .persistence
            .batch_evidence(&page.members[0].operation_id)
            .unwrap()
            .unwrap()
            .settled
    );

    // Even if sessions were empty, restore_metadata_backup and start_metadata_recovery are blocked while open.
    let saved_session = studio.sessions.lock().remove(&info.id).unwrap();
    assert_eq!(
        studio
            .restore_metadata_backup(&new_id(), &"0".repeat(64), "RESTORE", None)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio
            .start_metadata_recovery(
                new_id(),
                RecoveryRequest {
                    kind: RecoveryOperationKind::Export,
                    backup_id: Some(new_id()),
                    digest: Some("0".repeat(64)),
                    confirmation: String::new(),
                },
                Zeroizing::new(String::new()),
                Zeroizing::new(String::new()),
                None,
            )
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    studio
        .sessions
        .lock()
        .insert(info.id.clone(), saved_session);

    let rolled = studio
        .finish_snowflake_grid_transaction(&tx_id, FinishAction::Rollback)
        .await
        .unwrap();
    assert_eq!(rolled.outcome, Outcome::RolledBack);
    assert!(rolled.recording_error.is_none());
    assert!(studio.snowflake_grid.is_empty());
    assert!(studio.batch_outcomes.is_empty());

    studio
        .session_contexts_with_refresh(&info.id, true)
        .await
        .unwrap();
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn grid_studio_panics_after_admission_retire_helper_and_reconcile_after_reap() {
    let _guard = FIXTURE_LANE.lock().await;
    for fault in [
        GridTestFault::PanicBeforeDispatch,
        GridTestFault::PanicInAdmissionAfterQueue,
        GridTestFault::PanicAfterRecordOriginal,
    ] {
        let f = grid_fixture();
        let (studio, info) = connected(&f).await;
        let session = studio.sessions.lock().get(&info.id).cloned().unwrap();
        let slot = studio.session_safety_slot(&session).unwrap();

        let (_, _, edits) =
            browse_and_prepare(&studio, &info.id, &[update_row0("panic-window")]).await;
        studio.snowflake_grid.inject_fault(fault);

        let err = studio
            .execute_snowflake_grid_edits(&info.id, edits, Mode::Manual)
            .await
            .err()
            .unwrap();
        assert_eq!(err.code, ErrorCode::OutcomeUnknown);

        // Wait for the retired helper child to reap and drain through shutdown_and_wait / flush.
        studio
            .snowflake_cleanup
            .wait_all(Duration::from_secs(10))
            .await
            .unwrap();
        studio.flush_snowflake_grid_outcomes().unwrap();

        assert!(studio.batch_outcomes.is_empty());
        assert!(studio.snowflake_grid.is_empty());
        assert!(slot.is_available());
        match fault {
            GridTestFault::PanicBeforeDispatch => assert_eq!(dml_count(&f), 0),
            GridTestFault::PanicInAdmissionAfterQueue => assert!(dml_count(&f) <= 1),
            GridTestFault::PanicAfterRecordOriginal => assert_eq!(dml_count(&f), 1),
            GridTestFault::RefuseAdmissionBeforeDispatch => unreachable!(),
        }
        assert!(!queries(&f).iter().any(|s| s == "COMMIT" || s == "ROLLBACK"));

        let db = sqlite_db(&f);
        let (tx_id, disposition): (String, String) = db
            .query_row(
                "SELECT transaction_id, disposition FROM manual_transactions_v8",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(disposition, "unknown");
        let terminal_kind: String = db
            .query_row("SELECT terminal_kind FROM batch_operations_v8", [], |r| {
                r.get(0)
            })
            .unwrap();
        match fault {
            GridTestFault::PanicBeforeDispatch => assert_eq!(terminal_kind, "noOriginal"),
            GridTestFault::PanicInAdmissionAfterQueue => {
                assert!(matches!(terminal_kind.as_str(), "noOriginal" | "original"))
            }
            GridTestFault::PanicAfterRecordOriginal => assert_eq!(terminal_kind, "original"),
            GridTestFault::RefuseAdmissionBeforeDispatch => unreachable!(),
        }
        let bindings: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM manual_transaction_bindings_v8 WHERE transaction_id = ?1",
                [&tx_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bindings, 0);

        if fault == GridTestFault::PanicAfterRecordOriginal {
            let page = studio
                .persistence
                .manual_transaction_evidence(&tx_id, 0, 16)
                .unwrap();
            assert_eq!(page.members.len(), 1);
            assert!(
                studio
                    .persistence
                    .batch_evidence(&page.members[0].operation_id)
                    .unwrap()
                    .unwrap()
                    .settled
            );
        }

        studio.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test]
async fn grid_studio_retains_both_originals_across_sqlite_faults_pool_saturation_and_stale_settlement()
 {
    let _guard = FIXTURE_LANE.lock().await;
    let f = grid_fixture();
    let (studio, info) = connected(&f).await;
    let session = studio.sessions.lock().get(&info.id).cloned().unwrap();
    let slot = studio.session_safety_slot(&session).unwrap();

    let (_, _, edits) =
        browse_and_prepare(&studio, &info.id, &[update_row0("sqlite-fault-pending")]).await;

    let db = sqlite_db(&f);
    db.execute_batch(
        "CREATE TRIGGER fail_grid_capture BEFORE INSERT ON batch_captures_v8 \
         BEGIN SELECT RAISE(ABORT, 'synthetic batch_captures_v8 fault'); END;",
    )
    .unwrap();

    let pending = studio
        .execute_snowflake_grid_edits(&info.id, edits, Mode::Manual)
        .await
        .unwrap();
    assert_eq!(pending.outcome, Outcome::Pending);
    assert!(pending.recording_error.is_some());
    let tx_id = pending.transaction_id.clone().unwrap();
    assert!(slot.is_available());
    assert_eq!(studio.batch_outcomes.pending_count(), 1);

    // Commit is blocked while Pending recording has not settled.
    let before = queries(&f).len();
    assert_eq!(
        studio
            .finish_snowflake_grid_transaction(&tx_id, FinishAction::Commit)
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(queries(&f).len(), before);

    // Exhaust ALL remaining 31 ordinary MemoryLease slots so ordinary pool is 100% full.
    let mut saturated = Vec::new();
    while let Ok(lease) = studio.outcome_memory.ordinary() {
        saturated.push(lease);
    }
    assert_eq!(saturated.len(), 31);
    assert_eq!(
        studio.outcome_memory.ordinary().err().unwrap().code,
        ErrorCode::ResultLimitReached
    );

    // Protected Rollback still succeeds using the pre-authenticated SafetySlot with zero fresh SQL probes!
    let rolled = studio
        .finish_snowflake_grid_transaction(&tx_id, FinishAction::Rollback)
        .await
        .unwrap();
    assert_eq!(rolled.outcome, Outcome::RolledBack);
    assert!(rolled.recording_error.is_some());
    assert!(
        !slot.is_available(),
        "unsettled Rollback must hold the session SafetySlot"
    );
    assert_eq!(queries(&f).len() - before, 3);
    assert_eq!(studio.batch_outcomes.pending_count(), 2);

    // Stale settlement callback with the older Pending operation ID cannot release Finished!
    let native = studio.snowflake_grid.test_native_owner(&tx_id).unwrap();
    assert_eq!(
        native
            .recording_settled(&pending.operation_id)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio.relation_catalog(&info.id).await.unwrap_err().code,
        ErrorCode::Conflict
    );

    // Both restore_metadata_backup and start_metadata_recovery are blocked by grid ownership even if sessions are empty.
    let saved_session = studio.sessions.lock().remove(&info.id).unwrap();
    assert_eq!(
        studio
            .restore_metadata_backup(&new_id(), &"0".repeat(64), "RESTORE", None)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio
            .start_metadata_recovery(
                new_id(),
                RecoveryRequest {
                    kind: RecoveryOperationKind::Export,
                    backup_id: Some(new_id()),
                    digest: Some("0".repeat(64)),
                    confirmation: String::new(),
                },
                Zeroizing::new(String::new()),
                Zeroizing::new(String::new()),
                None,
            )
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    studio
        .sessions
        .lock()
        .insert(info.id.clone(), saved_session);

    drop(saturated);

    // Clear the SQLite trigger; a single flush settles Pending then Rollback and reclaims buffers.
    db.execute_batch("DROP TRIGGER fail_grid_capture;").unwrap();
    studio.flush_snowflake_grid_outcomes().unwrap();

    assert!(studio.batch_outcomes.is_empty());
    assert!(studio.snowflake_grid.is_empty());
    assert!(slot.is_available());

    let page = studio
        .persistence
        .manual_transaction_evidence(&tx_id, 0, 16)
        .unwrap();
    assert_eq!(page.disposition, "rolledBack");
    assert_eq!(
        page.members
            .iter()
            .map(|m| m.operation_id.as_str())
            .collect::<Vec<_>>(),
        [pending.operation_id.as_str(), rolled.operation_id.as_str()]
    );
    for op_id in [&pending.operation_id, &rolled.operation_id] {
        let ev = studio.persistence.batch_evidence(op_id).unwrap().unwrap();
        assert!(ev.settled && !ev.imported);
    }

    studio
        .session_contexts_with_refresh(&info.id, true)
        .await
        .unwrap();
    studio.relation_catalog(&info.id).await.unwrap();
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn grid_studio_no_dispatch_refusals_retire_initial_and_authorize_post_pending_rollback() {
    let _guard = FIXTURE_LANE.lock().await;

    // 1. Initial manual no-dispatch after durable SQLite reservation/pinning retires exact helper.
    {
        let f = grid_fixture();
        let (studio, info) = connected(&f).await;
        let session = studio.sessions.lock().get(&info.id).cloned().unwrap();
        let slot = studio.session_safety_slot(&session).unwrap();
        let (_, _, edits) =
            browse_and_prepare(&studio, &info.id, &[update_row0("initial-refused")]).await;
        let baseline = queries(&f).len();

        studio
            .snowflake_grid
            .inject_fault(GridTestFault::RefuseAdmissionBeforeDispatch);
        assert_eq!(
            studio
                .execute_snowflake_grid_edits(&info.id, edits, Mode::Manual)
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::Conflict
        );
        // Only the 11 read-only freshness probes ran; zero DML or transaction boundaries ran.
        assert_eq!(queries(&f).len() - baseline, 11);
        assert_eq!(dml_count(&f), 0);
        assert!(
            !queries(&f)
                .iter()
                .any(|s| ["BEGIN TRANSACTION", "COMMIT", "ROLLBACK"].contains(&s.as_str()))
        );

        studio
            .snowflake_cleanup
            .wait_all(Duration::from_secs(10))
            .await
            .unwrap();
        studio.flush_snowflake_grid_outcomes().unwrap();
        assert!(studio.batch_outcomes.is_empty());
        assert!(studio.snowflake_grid.is_empty());
        assert!(slot.is_available());

        let db = sqlite_db(&f);
        let (tx_id, disposition): (String, String) = db
            .query_row(
                "SELECT transaction_id, disposition FROM manual_transactions_v8",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(disposition, "unknown");
        let terminal_kind: String = db
            .query_row("SELECT terminal_kind FROM batch_operations_v8", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(terminal_kind, "noOriginal");
        let bindings: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM manual_transaction_bindings_v8 WHERE transaction_id = ?1",
                [&tx_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bindings, 0);
        studio.shutdown_and_wait().await.unwrap();
    }

    // 2. Post-Pending Commit no-dispatch retains GridNoDispatch proof for protected Rollback across SQLite fault.
    {
        let f = grid_fixture();
        let (studio, info) = connected(&f).await;
        let (_, _, edits) =
            browse_and_prepare(&studio, &info.id, &[update_row0("commit-refused")]).await;
        let pending = studio
            .execute_snowflake_grid_edits(&info.id, edits, Mode::Manual)
            .await
            .unwrap();
        assert_eq!(pending.outcome, Outcome::Pending);
        let tx_id = pending.transaction_id.clone().unwrap();

        studio
            .snowflake_grid
            .inject_fault(GridTestFault::RefuseAdmissionBeforeDispatch);
        assert_eq!(
            studio
                .finish_snowflake_grid_transaction(&tx_id, FinishAction::Commit)
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::Conflict
        );
        assert!(!queries(&f).iter().any(|s| s == "COMMIT"));

        // Further Commit is blocked; only Rollback is permitted.
        let before_retry = queries(&f).len();
        assert_eq!(
            studio
                .finish_snowflake_grid_transaction(&tx_id, FinishAction::Commit)
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(queries(&f).len(), before_retry);

        // Fault SQLite during protected Rollback to verify session stays held until reconciliation.
        let db = sqlite_db(&f);
        db.execute_batch(
            "CREATE TRIGGER fail_nodispatch_rb BEFORE INSERT ON batch_captures_v8 \
             BEGIN SELECT RAISE(ABORT, 'synthetic capture fault'); END;",
        )
        .unwrap();

        let rolled = studio
            .finish_snowflake_grid_transaction(&tx_id, FinishAction::Rollback)
            .await
            .unwrap();
        assert_eq!(rolled.outcome, Outcome::RolledBack);
        assert!(rolled.recording_error.is_some());
        assert_eq!(
            studio.relation_catalog(&info.id).await.unwrap_err().code,
            ErrorCode::Conflict
        );

        db.execute_batch("DROP TRIGGER fail_nodispatch_rb;")
            .unwrap();
        studio
            .session_contexts_with_refresh(&info.id, true)
            .await
            .unwrap();
        assert!(studio.batch_outcomes.is_empty());
        assert!(studio.snowflake_grid.is_empty());

        let page = studio
            .persistence
            .manual_transaction_evidence(&tx_id, 0, 16)
            .unwrap();
        assert_eq!(page.disposition, "rolledBack");
        assert_eq!(page.members.len(), 3);
        assert_eq!(page.members[0].operation_id, pending.operation_id);
        assert_eq!(page.members[2].operation_id, rolled.operation_id);
        assert!(
            studio
                .persistence
                .batch_evidence(&page.members[1].operation_id)
                .unwrap()
                .is_none(),
            "noDispatch Commit member must not invent a batch_evidence envelope"
        );
        let kind: String = db
            .query_row(
                "SELECT terminal_kind FROM batch_operations_v8 WHERE operation_id = ?1",
                [&page.members[1].operation_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "noDispatch");

        studio.relation_catalog(&info.id).await.unwrap();
        studio.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test]
async fn grid_studio_queued_zero_step_rejected_continuation_preserves_transaction_for_rollback() {
    let _guard = FIXTURE_LANE.lock().await;
    let f = grid_fixture();
    let studio = new_studio(&f);
    let saved = studio.save_profile(grid_profile(), None).unwrap();

    // Launch a real production Helper + mutation-batch + mutation-read child whose
    // submitGuard refuses new batch capacity reservations once `exhaust-guard-capacity`
    // is set, while keeping the already-reserved 5-slot manual finish capacity intact.
    let helper_dir = tempfile::tempdir().unwrap();
    let helper_canon = helper_dir.path().canonicalize().unwrap();
    let state_root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(state_root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

    let helper_src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../snowflake-helper/src")
        .canonicalize()
        .unwrap();
    let helper_url = url::Url::from_file_path(helper_src.join("helper.mjs")).unwrap();
    let framing_url = url::Url::from_file_path(helper_src.join("framing.mjs")).unwrap();
    let sdk_path = f
        .controls
        .join("node_modules/snowflake-sdk/index.mjs")
        .canonicalize()
        .unwrap();
    let sdk_url = url::Url::from_file_path(sdk_path).unwrap();
    let node_bin = f
        .controls
        .parent()
        .unwrap()
        .join("bin/node")
        .canonicalize()
        .unwrap();

    let script = helper_canon.join("capped-helper.mjs");
    let exhaust_marker = f.controls.join("exhaust-guard-capacity");
    std::fs::write(
        &script,
        format!(
            r#"import {{ existsSync }} from 'node:fs';
import {{ Helper }} from {helper_url:?};
import {{ Decoder, writer }} from {framing_url:?};
const sdk = (await import({sdk_url:?})).default;
const rawWrite = process.stdout.write.bind(process.stdout);
process.stdout.write = (_c, ...a) => {{ a.find(x => typeof x === 'function')?.(); return true; }};
process.stderr.write = (_c, ...a) => {{ a.find(x => typeof x === 'function')?.(); return true; }};
let registered = 0, reserved = 0, op = null, stmt = null;
const leases = new WeakMap();
const guard = {{
  setAuthority() {{}},
  beginOperation() {{ op = {{}}; return op; }},
  endOperation() {{ op = null; }},
  reserveCapacity(n) {{
    if (existsSync({exhaust_marker:?})) return null;
    const l = {{}};
    leases.set(l, n);
    reserved += n;
    return l;
  }},
  releaseCapacity(l) {{
    reserved -= leases.get(l);
    leases.delete(l);
  }},
  beginStatement(_o, l) {{ stmt = {{ l }}; return stmt; }},
  registerRequestId(_t, _id) {{
    if (stmt.l) {{ leases.set(stmt.l, leases.get(stmt.l) - 1); reserved--; }}
    registered++;
  }},
  endStatement() {{ stmt = null; }},
}};
const out = writer(rawWrite, () => process.exit(2));
const helper = new Helper(sdk, out, () => out.flush().then(() => process.exit(0)), {{
  clientConfigFile: {config_file:?},
  submitGuard: guard,
  isFileTransfer: () => false,
}});
const dec = new Decoder();
process.stdin.on('data', chunk => {{ for (const r of dec.push(chunk)) helper.receive(r); }});
process.stdin.on('end', () => {{ helper.stop(); }});
"#,
            helper_url = helper_url.as_str(),
            framing_url = framing_url.as_str(),
            sdk_url = sdk_url.as_str(),
            exhaust_marker = exhaust_marker,
            config_file = state_root.path().join("client-config.json"),
        ),
    )
    .unwrap();
    std::fs::write(state_root.path().join("client-config.json"), "{}").unwrap();

    let session_id = new_id();
    let launch = Launch::new(
        node_bin,
        Some(script),
        state_root.path().canonicalize().unwrap(),
        state_root,
    )
    .unwrap();
    let (driver, info) = SnowflakeExternal::open(
        &saved,
        &session_id,
        launch,
        ExternalBrowserOptions::default(),
    )
    .await
    .unwrap();
    let safety_slot = studio.outcome_memory.safety().unwrap();
    studio.sessions.lock().insert(
        session_id.clone(),
        Arc::new(Session {
            rollback_memory: Mutex::new(Some(safety_slot.clone())),
            metadata_gate: Mutex::new(MetadataGate::default()),
            info: info.clone(),
            profile: saved,
            driver: driver.clone(),
            snowflake_external: Some(driver.clone()),
            lane: tokio::sync::Mutex::new(()),
            cancellation_gate: tokio::sync::Mutex::new(()),
            active_run: Mutex::new(None),
            closed: AtomicBool::new(false),
            tunnel: None,
            completion: Mutex::new(None),
            observed_runtime: Mutex::new(None),
        }),
    );

    let (preview_req, _preview, edits) =
        browse_and_prepare(&studio, &session_id, &[update_row0("initial-before-quota")]).await;
    let pending = studio
        .execute_snowflake_grid_edits(&session_id, edits, Mode::Manual)
        .await
        .unwrap();
    assert_eq!(pending.outcome, Outcome::Pending);
    let tx_id = pending.transaction_id.clone().unwrap();

    let cont_preview_first = studio
        .preview_snowflake_grid_transaction_relation(&tx_id, &preview_req)
        .await
        .unwrap();
    let cont_edits_first = studio
        .prepare_snowflake_grid_continuation_edits(
            &tx_id,
            &preview_req,
            cont_preview_first.edit_snapshot_id.as_deref().unwrap(),
            &[SnapshotMutation::Delete { row_index: 2 }],
        )
        .await
        .unwrap();

    // 1. Exhaust guard batch capacity and fault SQLite capture; verify delayed flush clears successor_recovery.
    std::fs::write(&exhaust_marker, "").unwrap();
    let db = sqlite_db(&f);
    db.execute_batch(
        "CREATE TRIGGER fail_rejected_capture BEFORE INSERT ON batch_captures_v8 \
         BEGIN SELECT RAISE(ABORT, 'synthetic rejected capture fault'); END;",
    )
    .unwrap();

    let rejected_first = studio
        .continue_snowflake_grid_edits(&tx_id, cont_edits_first, true)
        .await
        .unwrap();
    assert_eq!(rejected_first.outcome, Outcome::Rejected);
    assert!(rejected_first.recording_error.is_some());
    assert!(
        studio.snowflake_grid.test_has_successor_recovery(&tx_id),
        "unsettled zero-step Rejected must retain RejectedAdmission recovery"
    );

    db.execute_batch("DROP TRIGGER fail_rejected_capture;")
        .unwrap();
    studio.flush_snowflake_grid_outcomes().unwrap();
    assert!(
        !studio.snowflake_grid.test_has_successor_recovery(&tx_id),
        "reconciling a settled RejectedAdmission cell must clear stale successor_recovery"
    );

    // 2. Prepare a second continuation after recording settles, fault SQLite again, and run protected Rollback while unsettled!
    let cont_preview = studio
        .preview_snowflake_grid_transaction_relation(&tx_id, &preview_req)
        .await
        .unwrap();
    let cont_edits = studio
        .prepare_snowflake_grid_continuation_edits(
            &tx_id,
            &preview_req,
            cont_preview.edit_snapshot_id.as_deref().unwrap(),
            &[SnapshotMutation::Delete { row_index: 2 }],
        )
        .await
        .unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_rejected_capture BEFORE INSERT ON batch_captures_v8 \
         BEGIN SELECT RAISE(ABORT, 'synthetic rejected capture fault'); END;",
    )
    .unwrap();

    let rejected = studio
        .continue_snowflake_grid_edits(&tx_id, cont_edits, true)
        .await
        .unwrap();
    assert_eq!(rejected.outcome, Outcome::Rejected);
    assert!(rejected.recording_error.is_some());
    let summary = rejected
        .report
        .batch
        .as_ref()
        .unwrap()
        .summary
        .as_ref()
        .unwrap();
    assert_eq!(summary.failure, Some(Failure::Admission));
    assert!(rejected.report.batch.as_ref().unwrap().steps.is_empty());
    assert_eq!(summary.transaction_id.as_deref(), Some("123456789"));

    // Protected Rollback still succeeds SQL-free while the Rejected original is unsettled!
    let rolled = studio
        .finish_snowflake_grid_transaction(&tx_id, FinishAction::Rollback)
        .await
        .unwrap();
    assert_eq!(rolled.outcome, Outcome::RolledBack);
    assert!(rolled.recording_error.is_some());

    db.execute_batch("DROP TRIGGER fail_rejected_capture;")
        .unwrap();
    studio.flush_snowflake_grid_outcomes().unwrap();

    assert!(studio.batch_outcomes.is_empty());
    assert!(studio.snowflake_grid.is_empty());
    assert!(safety_slot.is_available());

    let page = studio
        .persistence
        .manual_transaction_evidence(&tx_id, 0, 16)
        .unwrap();
    assert_eq!(page.disposition, "rolledBack");
    assert_eq!(
        page.members
            .iter()
            .map(|m| m.operation_id.as_str())
            .collect::<Vec<_>>(),
        [
            pending.operation_id.as_str(),
            rejected_first.operation_id.as_str(),
            rejected.operation_id.as_str(),
            rolled.operation_id.as_str()
        ]
    );
    let nodispatch_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM batch_operations_v8 WHERE terminal_kind = 'noDispatch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        nodispatch_count, 0,
        "queued Rejected must never be stored as noDispatch"
    );
    for op_id in [
        &pending.operation_id,
        &rejected_first.operation_id,
        &rejected.operation_id,
        &rolled.operation_id,
    ] {
        assert!(
            studio
                .persistence
                .batch_evidence(op_id)
                .unwrap()
                .unwrap()
                .settled
        );
    }

    // 3. Also verify NoDispatch (ordinal 3) after a settled zero-step Rejected (ordinal 2) still authorizes protected Rollback!
    std::fs::remove_file(&exhaust_marker).unwrap();
    studio
        .session_contexts_with_refresh(&session_id, true)
        .await
        .unwrap();
    let (preview_req2, _preview2, edits2) =
        browse_and_prepare(&studio, &session_id, &[update_row0("second-tx")]).await;
    let pending2 = studio
        .execute_snowflake_grid_edits(&session_id, edits2, Mode::Manual)
        .await
        .unwrap();
    assert_eq!(pending2.outcome, Outcome::Pending);
    let tx_id2 = pending2.transaction_id.clone().unwrap();

    let cont_preview2 = studio
        .preview_snowflake_grid_transaction_relation(&tx_id2, &preview_req2)
        .await
        .unwrap();
    let cont_edits2 = studio
        .prepare_snowflake_grid_continuation_edits(
            &tx_id2,
            &preview_req2,
            cont_preview2.edit_snapshot_id.as_deref().unwrap(),
            &[SnapshotMutation::Delete { row_index: 2 }],
        )
        .await
        .unwrap();
    std::fs::write(&exhaust_marker, "").unwrap();
    let rejected2 = studio
        .continue_snowflake_grid_edits(&tx_id2, cont_edits2, true)
        .await
        .unwrap();
    assert_eq!(rejected2.outcome, Outcome::Rejected);
    assert!(rejected2.recording_error.is_none());

    std::fs::remove_file(&exhaust_marker).unwrap();
    studio
        .snowflake_grid
        .inject_fault(GridTestFault::RefuseAdmissionBeforeDispatch);
    assert_eq!(
        studio
            .finish_snowflake_grid_transaction(&tx_id2, FinishAction::Commit)
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );

    let rolled2 = studio
        .finish_snowflake_grid_transaction(&tx_id2, FinishAction::Rollback)
        .await
        .unwrap();
    assert_eq!(rolled2.outcome, Outcome::RolledBack);
    assert!(rolled2.recording_error.is_none());
    let page2 = studio
        .persistence
        .manual_transaction_evidence(&tx_id2, 0, 16)
        .unwrap();
    assert_eq!(page2.disposition, "rolledBack");
    assert_eq!(page2.members.len(), 4);

    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn grid_studio_removed_session_and_disconnect_retire_unknown_without_inventing_rollback() {
    let _guard = FIXTURE_LANE.lock().await;

    // 1. Removed session-map entry while helper stays alive still allows protected Rollback.
    {
        let f = grid_fixture();
        let (studio, info) = connected(&f).await;
        let (_, _, edits) =
            browse_and_prepare(&studio, &info.id, &[update_row0("session-removed")]).await;
        let pending = studio
            .execute_snowflake_grid_edits(&info.id, edits, Mode::Manual)
            .await
            .unwrap();
        let tx_id = pending.transaction_id.unwrap();

        let removed = studio.sessions.lock().remove(&info.id);
        assert!(removed.is_some());
        assert!(!studio.snowflake_grid.is_empty());

        let rolled = studio
            .finish_snowflake_grid_transaction(&tx_id, FinishAction::Rollback)
            .await
            .unwrap();
        assert_eq!(rolled.outcome, Outcome::RolledBack);
        assert!(studio.snowflake_grid.is_empty());
        assert!(studio.batch_outcomes.is_empty());
        drop(removed);
        studio.shutdown_and_wait().await.unwrap();
    }

    // 2. disconnect() + shutdown_and_wait() while Pending retires group as unknown after reap (with SQLite retry).
    {
        let f = grid_fixture();
        let (studio, info) = connected(&f).await;
        let (_, _, edits) =
            browse_and_prepare(&studio, &info.id, &[update_row0("disconnect-pending")]).await;
        let pending = studio
            .execute_snowflake_grid_edits(&info.id, edits, Mode::Manual)
            .await
            .unwrap();
        let tx_id = pending.transaction_id.unwrap();

        let db = sqlite_db(&f);
        db.execute_batch(
            "CREATE TRIGGER fail_retire_unknown BEFORE DELETE ON manual_transaction_bindings_v8 \
             BEGIN SELECT RAISE(ABORT, 'synthetic retirement fault'); END;",
        )
        .unwrap();

        studio.disconnect(&info.id);
        assert_eq!(
            studio
                .restore_metadata_backup(&new_id(), &"0".repeat(64), "RESTORE", None)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio
                .start_metadata_recovery(
                    new_id(),
                    RecoveryRequest {
                        kind: RecoveryOperationKind::Export,
                        backup_id: Some(new_id()),
                        digest: Some("0".repeat(64)),
                        confirmation: String::new(),
                    },
                    Zeroizing::new(String::new()),
                    Zeroizing::new(String::new()),
                    None,
                )
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );

        // First shutdown_and_wait reaps the helper but fails closed when retirement SQLite write faults.
        assert_eq!(
            studio.shutdown_and_wait().await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert!(!studio.batch_outcomes.is_empty());
        assert!(!studio.snowflake_grid.is_empty());
        assert!(!queries(&f).iter().any(|s| s == "ROLLBACK" || s == "COMMIT"));

        // Dropping the trigger lets a retry of shutdown_and_wait retire the group as unknown.
        db.execute_batch("DROP TRIGGER fail_retire_unknown;")
            .unwrap();
        studio.shutdown_and_wait().await.unwrap();
        assert!(studio.batch_outcomes.is_empty());
        assert!(studio.snowflake_grid.is_empty());

        let page = studio
            .persistence
            .manual_transaction_evidence(&tx_id, 0, 16)
            .unwrap();
        assert_eq!(page.disposition, "unknown");
    }
}

#[tokio::test]
async fn grid_studio_inflight_protected_rollback_allows_global_flush_and_settled_volatile_fallback_settles()
 {
    let _guard = FIXTURE_LANE.lock().await;

    // 1. In-flight protected Rollback must not block flush_snowflake_grid_outcomes() when SQLite recovers!
    {
        let f = grid_fixture();
        let (studio, info) = connected(&f).await;
        let (_, _, edits) =
            browse_and_prepare(&studio, &info.id, &[update_row0("inflight-rb-flush")]).await;

        let db = sqlite_db(&f);
        db.execute_batch(
            "CREATE TRIGGER fail_grid_capture BEFORE INSERT ON batch_captures_v8 \
             BEGIN SELECT RAISE(ABORT, 'synthetic capture fault'); END;",
        )
        .unwrap();

        let pending = studio
            .execute_snowflake_grid_edits(&info.id, edits, Mode::Manual)
            .await
            .unwrap();
        assert_eq!(pending.outcome, Outcome::Pending);
        assert!(pending.recording_error.is_some());
        let tx_id = pending.transaction_id.clone().unwrap();

        // Remaining ordinary Studio session entry points reject cleanly while owned edits are open.
        assert_eq!(
            studio.catalog(&info.id).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio.database_objects(&info.id).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio.completion_catalog(&info.id).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio
                .preflight_snowflake_ddl(&SnowflakeDdlPreflightRequest {
                    session_id: info.id.clone(),
                    action: SnowflakeDdlAction::DropTable {
                        browse_revision: new_id(),
                        target_id: new_id(),
                    },
                })
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio
                .prepare_transaction_control(
                    &info.id,
                    &TransactionControlRequest {
                        revision: new_id(),
                        action: ExplicitTransactionAction::Rollback,
                    }
                )
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );

        std::fs::write(f.controls.join("hold-rollback-ack"), "").unwrap();
        let rollback_handle = studio
            .start_finish_snowflake_grid_transaction(&tx_id, FinishAction::Rollback)
            .unwrap();

        for _ in 0..200 {
            if queries(&f).iter().any(|s| s == "ROLLBACK") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(queries(&f).iter().any(|s| s == "ROLLBACK"));
        assert_eq!(studio.usage().active_runs, 1);
        assert_eq!(studio.batch_outcomes.pending_count(), 2);

        // SQLite recovers while ROLLBACK is still in flight; global flush settles Pending and reclaims its buffer!
        db.execute_batch("DROP TRIGGER fail_grid_capture;").unwrap();
        studio.flush_snowflake_grid_outcomes().unwrap();
        assert_eq!(studio.batch_outcomes.pending_count(), 1);
        assert!(
            studio
                .persistence
                .batch_evidence(&pending.operation_id)
                .unwrap()
                .unwrap()
                .settled
        );

        std::fs::write(f.controls.join("release-rollback-ack"), "").unwrap();
        let rolled = rollback_handle.await.unwrap().unwrap();
        assert_eq!(rolled.outcome, Outcome::RolledBack);
        assert!(rolled.recording_error.is_none());
        assert_eq!(studio.usage().active_runs, 0);
        assert!(studio.batch_outcomes.is_empty());
        assert!(studio.snowflake_grid.is_empty());
        studio.shutdown_and_wait().await.unwrap();
    }

    // 2. SettledPending volatile rollback fallback + reset_fixtures() on detached manual owner.
    {
        let f = grid_fixture();
        let (studio, info) = connected(&f).await;
        let (_, _, edits) =
            browse_and_prepare(&studio, &info.id, &[update_row0("settled-volatile-rb")]).await;

        let pending = studio
            .execute_snowflake_grid_edits(&info.id, edits, Mode::Manual)
            .await
            .unwrap();
        assert_eq!(pending.outcome, Outcome::Pending);
        assert!(pending.recording_error.is_none());
        assert_eq!(studio.batch_outcomes.pending_count(), 0);
        let tx_id = pending.transaction_id.clone().unwrap();

        // Fault SQLite reservation ONLY for emergencyRollback after Pending has already settled.
        let db = sqlite_db(&f);
        db.execute_batch(
            "CREATE TRIGGER reject_rb_reserve BEFORE INSERT ON batch_operations_v8 \
             WHEN NEW.pool = 'emergencyRollback' \
             BEGIN SELECT RAISE(ABORT, 'synthetic rollback reserve fault'); END;",
        )
        .unwrap();

        let rolled = studio
            .finish_snowflake_grid_transaction(&tx_id, FinishAction::Rollback)
            .await
            .unwrap();
        assert_eq!(rolled.outcome, Outcome::RolledBack);
        assert!(rolled.recording_error.is_some());
        assert_eq!(studio.batch_outcomes.pending_count(), 1);

        db.execute_batch("DROP TRIGGER reject_rb_reserve;").unwrap();
        studio.flush_snowflake_grid_outcomes().unwrap();
        assert!(studio.batch_outcomes.is_empty());
        assert!(studio.snowflake_grid.is_empty());

        let page = studio
            .persistence
            .manual_transaction_evidence(&tx_id, 0, 16)
            .unwrap();
        assert_eq!(page.disposition, "rolledBack");
        assert_eq!(page.members.len(), 2);

        // Resume metadata, open another manual transaction, detach from sessions map, and verify reset_fixtures() reaps helper.
        studio
            .session_contexts_with_refresh(&info.id, true)
            .await
            .unwrap();
        let (_, _, edits2) =
            browse_and_prepare(&studio, &info.id, &[update_row0("reset-detached")]).await;
        let pending2 = studio
            .execute_snowflake_grid_edits(&info.id, edits2, Mode::Manual)
            .await
            .unwrap();
        let tx2_id = pending2.transaction_id.unwrap();
        let detached = studio.sessions.lock().remove(&info.id);
        assert!(detached.is_some());
        drop(detached);

        studio.reset_fixtures().await.unwrap();
        studio.flush_snowflake_grid_outcomes().unwrap();
        assert!(studio.batch_outcomes.is_empty());
        assert!(studio.snowflake_grid.is_empty());
        assert_eq!(
            studio
                .persistence
                .manual_transaction_evidence(&tx2_id, 0, 16)
                .unwrap()
                .disposition,
            "unknown"
        );
        studio.shutdown_and_wait().await.unwrap();
    }
}
