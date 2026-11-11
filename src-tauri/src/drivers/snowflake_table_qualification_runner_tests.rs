//! Closed synthetic-only composition of the actual Studio/helper routes.
//! No account SDK, profile lookup, external execution entry or approval exists.
use super::{
    Plan, SCHEMA, SchemaCensus, StatementId,
    journal::FILE_NAME,
    policy::Policy,
    runner::{
        CallError, Collector, EVENT_LIMIT, FixtureControls, Output, Pause, READ_BYTES, Runner,
        RunnerReceipt, wait_finished,
    },
};
use crate::{
    domain::*,
    drivers::snowflake_qualification_transport::{TransportAttachment, TransportPolicy},
    execution::Studio,
    file_capability::DisabledFileDialog,
    secrets::MemorySecrets,
    snowflake_runtime::studio_fixture::Fixture,
};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, sync::Arc, time::Duration};

// Only this module constructs the harness. All configurable inputs below are
// synthetic test cases; there is no production profile/account/session selector.
struct OfflineHarness {
    fixture: Fixture,
    journal: tempfile::TempDir,
    studio: Arc<Studio>,
    policy: Arc<Policy>,
    runner: Option<Runner>,
    transport: TransportAttachment,
    plan: Plan,
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
        // These closed row transformations exercise real parsing/callbacks. The
        // injection point is derived from the same closed ledger, not SQL input.
        let drift_point = match fault["kind"].as_str() {
            Some("censusOverflow" | "censusReversed") => Some((2, 6)),
            Some("censusBeforeCreate") => Some((3, 6)),
            Some("censusCatalog") => Some((4, 14)),
            Some("contextRoleBeforePin" | "contextWarehouseBeforePin") => Some((2, 1)),
            Some("contextSecondaryRoles" | "contextSearchPath") => Some((4, 1)),
            _ => None,
        };
        let drift = if let Some((phase, ordinal)) = drift_point {
            let names = std::iter::once(SCHEMA.to_owned())
                .chain((2..=schema_count).map(|n| format!("S{n:02}")))
                .collect::<Vec<_>>();
            let ledger = plan
                .expand_physical_ledger(&SchemaCensus::freeze(&names).unwrap())
                .unwrap();
            let index = ledger
                .entries()
                .iter()
                .position(|entry| entry.phase() == phase && entry.phase_ordinal() == ordinal)
                .unwrap();
            json!({"kind":fault["kind"],"at":index+1,"sql":ledger.entries()[index].sql()})
        } else {
            Value::Null
        };
        let sdk_fault = if child_exit.is_null() && drift.is_null() {
            fault
        } else {
            Value::Null
        };
        let options = json!({"schemaCount":schema_count,"tableName":plan.table(),"statements":statements,"fault":sdk_fault});
        let wrapper = format!(
            r#"import {{makeSdk}} from '../../lifecycle-sdk.mjs';
import {{appendFileSync}} from 'node:fs';
import {{Readable}} from 'node:stream';
const options={options};
const childExit={child_exit};
const drift={drift};
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
if (drift !== null) {{
  const create=sdk.createConnection.bind(sdk);
  sdk.createConnection=profile=>{{
    const connection=create(profile);
    const execute=connection.execute.bind(connection);
    let ordinal=0;
    connection.execute=request=>{{
      const current=++ordinal;
      if (current === drift.at && request.sqlText !== drift.sql)
        throw Error('Synthetic drift point disagrees with closed ledger');
      const selected=request.sqlText === drift.sql && (current === drift.at ||
        drift.kind.startsWith('context') && current > drift.at);
      return execute({{...request,complete:(error,statement)=>{{
        if (!error && selected) {{
          const stream=statement.streamRows.bind(statement);
          statement.streamRows=()=>Readable.from((async function* () {{
            const before=[];
            for await (const row of stream()) before.push([...row]);
            const rows=before.map(row=>[...row]);
            const names=statement.getColumns().map(column=>column.getName());
            if (drift.kind === 'censusOverflow') rows.push(['SANDBOX','S33']);
            else if (drift.kind === 'censusReversed') rows.reverse();
            else if (drift.kind === 'censusBeforeCreate' || drift.kind === 'censusCatalog') {{
              if (rows.length !== 2 || rows[1][1] !== 'S02')
                throw Error('Synthetic census swap requires its two-schema baseline');
              rows[1][1]='S03';
            }} else {{
              if (rows.length !== 1) throw Error('Synthetic context drift requires one row');
              const field=drift.kind === 'contextRoleBeforePin' ? 'ROLE_NAME' :
                drift.kind === 'contextWarehouseBeforePin' ? 'WAREHOUSE_NAME' :
                drift.kind === 'contextSecondaryRoles' ? 'SECONDARY_ROLES' : 'SEARCH_PATH';
              const index=names.indexOf(field);
              if (index < 0) throw Error('Synthetic context field is missing');
              rows[0][index]=drift.kind === 'contextRoleBeforePin' ? 'OTHER_ROLE' :
                drift.kind === 'contextWarehouseBeforePin' ? 'OTHER_WAREHOUSE' :
                drift.kind === 'contextSecondaryRoles' ? JSON.stringify({{roles:'EXTRA_ROLE',value:'ALL'}}) :
                JSON.stringify(['SANDBOX.BILLIE_TESTING','SANDBOX.S02']);
            }}
            append('drift-events',{{kind:drift.kind,ordinal:current,sql:request.sqlText,names,before,after:rows}});
            yield* rows;
          }})());
        }}
        request.complete(error,statement);
      }}}});
    }};
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
        let runner =
            Runner::fixture(studio.clone(), policy.clone(), plan.clone(), attempt).unwrap();
        let transport = runner.transport();
        Self {
            fixture,
            journal,
            studio,
            policy,
            runner: Some(runner),
            transport,
            plan,
            cancel_read,
            pause: None,
            break_journal_after: None,
        }
    }

    async fn run(self) -> Receipt {
        // This detached fixture owner keeps tempdirs/SDK material alive even if
        // its waiter drops while the shared core still owns an in-flight call.
        tokio::spawn(async move {
            let mut harness = self;
            let mut runner = harness.runner.take().unwrap();
            runner.set_controls(FixtureControls {
                cancel_read: harness.cancel_read,
                pause: harness.pause.take(),
                break_journal: harness
                    .break_journal_after
                    .map(|phase| (phase, harness.journal.path().join(FILE_NAME))),
            });
            let RunnerReceipt {
                outputs,
                failure,
                original_call_failure,
                completion_record_error,
                shutdown,
                settled,
                persisted,
            } = runner.start().await.unwrap();
            let journal = fs::read_to_string(harness.journal.path().join(FILE_NAME))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let queries = fs::read_to_string(harness.fixture.controls.join("queries"))
                .unwrap_or_default()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            Receipt {
                harness,
                outputs,
                failure,
                original_call_failure,
                completion_record_error,
                journal,
                queries,
                shutdown,
                settled,
                persisted,
            }
        })
        .await
        .unwrap()
    }
}

struct Receipt {
    harness: OfflineHarness,
    outputs: Vec<Output>,
    failure: Option<(u8, AppError)>,
    original_call_failure: Option<CallError>,
    completion_record_error: Option<AppError>,
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
    let snapshot = collector.complete_snapshot();
    assert!(!snapshot.complete);
    assert_eq!(snapshot.events.len(), EVENT_LIMIT);
    assert_eq!(snapshot.error.unwrap().code, ErrorCode::Internal);
    assert!(
        !snapshot
            .events
            .iter()
            .any(|event| matches!(event, QueryEvent::Finished { .. }))
    );
}

#[test]
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
    let snapshot = collector.complete_snapshot();
    assert!(!snapshot.complete);
    assert!(snapshot.events.is_empty());
    assert_eq!(snapshot.error.unwrap().code, ErrorCode::Internal);
    assert!(
        !snapshot
            .events
            .iter()
            .any(|event| matches!(event, QueryEvent::Finished { .. }))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_fixture_rejects_plan_mismatch_before_launch() {
    let harness = OfflineHarness::new(1, Value::Null).await;
    let different = Plan::generate().unwrap();
    assert_ne!(different.run(), harness.plan.run());
    assert_ne!(different.table(), harness.plan.table());
    assert!(
        Runner::fixture(
            harness.studio.clone(),
            harness.policy.clone(),
            different,
            "unused-attempt".into()
        )
        .is_err()
    );
    assert!(!harness.fixture.controls.join("queries").exists());
    assert_eq!(fs::read_dir(&harness.fixture.state).unwrap().count(), 0);
    let report = harness.transport.report();
    assert_eq!(report.admitted_writes, 0);
    assert_eq!(report.stdout_bytes, 0);
    let records: Vec<Value> = fs::read_to_string(harness.journal.path().join(FILE_NAME))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(!records.iter().any(|record| {
        record["event"].get("wireId").is_some()
            || matches!(
                record["event"]["type"].as_str(),
                Some("validatedPhase" | "operationCompletion")
            )
    }));
    harness.studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn qualification_studio_runner_missing_finished_is_typed_incomplete_prefix() {
    let mut collector = Collector::default();
    let original = QueryEvent::Notice {
        message: "original prefix".into(),
    };
    assert!(collector.push(original.clone()));
    let (sender, mut receiver) = tokio::sync::watch::channel((false, false));
    drop(sender);
    let error = wait_finished(&mut receiver).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Internal);
    let snapshot = collector.complete_snapshot();
    assert!(!snapshot.complete);
    assert!(snapshot.error.is_some());
    assert_eq!(
        serde_json::to_value(snapshot.events).unwrap(),
        serde_json::to_value([original]).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_incomplete_capture_journals_original_error_and_prefix() {
    let harness = OfflineHarness::new(1, Value::Null).await;
    let mut collector = Collector::default();
    for index in 0..EVENT_LIMIT {
        assert!(collector.push(QueryEvent::Notice {
            message: format!("original-{index}")
        }));
    }
    assert!(!collector.push(QueryEvent::Finished {
        run_id: "original-run".into(),
        status: RunStatus::Cancelled,
        elapsed_ms: 9
    }));
    let original_error = AppError::new(ErrorCode::Cancelled, "distinct original cancellation");
    let cleanup_error = AppError::new(
        ErrorCode::ConnectionClosed,
        "distinct local cleanup failure",
    );
    let mut original = CallError::captured(original_error.clone(), collector.complete_snapshot());
    original.cleanup_error = Some(Box::new(cleanup_error.clone()));
    let expected_events = serde_json::to_value(&original.events).unwrap();
    assert!(!original.capture_complete);
    assert_eq!(original.error.code, ErrorCode::Internal);
    assert!(
        harness
            .policy
            .record_operation_completion(original.completion())
            .await
            .unwrap()
            .is_some()
    );
    assert!(TransportPolicy::check(&*harness.policy).is_err());
    let journal: Vec<Value> = fs::read_to_string(harness.journal.path().join(FILE_NAME))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let completion = &journal
        .iter()
        .find(|record| record["event"]["type"] == "operationCompletion")
        .unwrap()["event"]["completion"];
    assert_eq!(completion["status"], "incompleteRead");
    let durable = &completion["original"];
    assert_eq!(
        durable["error"],
        serde_json::to_value(&original.error).unwrap()
    );
    assert_eq!(
        durable["originalError"],
        serde_json::to_value(original_error).unwrap()
    );
    assert_eq!(
        durable["cleanupError"],
        serde_json::to_value(cleanup_error).unwrap()
    );
    assert_eq!(durable["events"], expected_events);
    assert_eq!(durable["events"].as_array().unwrap().len(), EVENT_LIMIT);
    assert!(
        !journal
            .iter()
            .any(|record| record["event"]["type"] == "validatedPhase")
    );
    assert!(!harness.fixture.controls.join("queries").exists());
    harness.studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_late_finished_keeps_incomplete_original_provenance() {
    let harness = OfflineHarness::new(1, Value::Null).await;
    let mut collector = Collector::default();
    assert!(collector.push(QueryEvent::Notice {
        message: "original prefix before failed drain".into()
    }));
    let original_error = AppError::new(
        ErrorCode::Cancelled,
        "actual cancellation before late Finished",
    );
    let mut original = CallError::captured(original_error.clone(), collector.complete_snapshot());
    assert!(!original.capture_complete);
    assert!(collector.push(QueryEvent::Finished {
        run_id: "original-run".into(),
        status: RunStatus::Cancelled,
        elapsed_ms: 17
    }));
    let late = collector.complete_snapshot();
    assert!(late.complete);
    original.retain_cleanup_snapshot(late);
    assert!(!original.capture_complete);
    assert!(matches!(
        original.events.last(),
        Some(QueryEvent::Finished { .. })
    ));
    assert!(
        harness
            .policy
            .record_operation_completion(original.completion())
            .await
            .unwrap()
            .is_some()
    );
    assert!(TransportPolicy::check(&*harness.policy).is_err());
    let journal: Vec<Value> = fs::read_to_string(harness.journal.path().join(FILE_NAME))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let completion = &journal
        .iter()
        .find(|record| record["event"]["type"] == "operationCompletion")
        .unwrap()["event"]["completion"];
    assert_eq!(completion["status"], "incompleteRead");
    assert_eq!(
        completion["original"]["originalError"],
        serde_json::to_value(original_error).unwrap()
    );
    assert_eq!(completion["original"]["error"]["code"], "internal");
    assert_eq!(
        completion["original"]["events"],
        serde_json::to_value(&original.events).unwrap()
    );
    assert!(
        !journal
            .iter()
            .any(|record| record["event"]["type"] == "validatedPhase")
    );
    assert!(!harness.fixture.controls.join("queries").exists());
    harness.studio.shutdown_and_wait().await.unwrap();
}

#[test]
fn qualification_studio_runner_recovered_snapshot_keeps_failed_first_drain() {
    let mut collector = Collector::default();
    assert!(collector.push(QueryEvent::Notice {
        message: "original prefix before failed first drain".into()
    }));
    assert!(collector.push(QueryEvent::Finished {
        run_id: "original-run".into(),
        status: RunStatus::Cancelled,
        elapsed_ms: 23
    }));
    let recovered = collector.complete_snapshot();
    assert!(recovered.complete);
    let expected_events = serde_json::to_value(&recovered.events).unwrap();
    // The same boundary helper is used after the real read's first failed drain,
    // when disconnect's subsequent drain has already recovered Finished.
    let snapshot = recovered.after_failed_drain();
    let original_error = AppError::new(
        ErrorCode::Cancelled,
        "original cancellation before recovered disconnect",
    );
    let cleanup_error = AppError::new(ErrorCode::ConnectionClosed, "original disconnect failure");
    let mut original = CallError::captured(original_error.clone(), snapshot);
    original.cleanup_error = Some(Box::new(cleanup_error.clone()));
    assert!(!original.capture_complete);
    let record = serde_json::to_value(original.completion()).unwrap();
    assert_eq!(record["status"], "incompleteRead");
    assert_eq!(record["original"]["error"]["code"], "internal");
    assert_eq!(
        record["original"]["originalError"],
        serde_json::to_value(original_error).unwrap()
    );
    assert_eq!(
        record["original"]["cleanupError"],
        serde_json::to_value(cleanup_error).unwrap()
    );
    assert_eq!(record["original"]["events"], expected_events);
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
async fn qualification_studio_runner_dropped_core_handle_retains_original_completion_and_cleanup() {
    let mut harness = OfflineHarness::new(1, Value::Null).await;
    let mut runner = harness.runner.take().unwrap();
    let (reached, ready) = tokio::sync::oneshot::channel();
    let resume = Arc::new(tokio::sync::Notify::new());
    runner.set_controls(FixtureControls {
        pause: Some(Pause {
            phase: 3,
            reached,
            resume: resume.clone(),
        }),
        ..FixtureControls::default()
    });
    let owner = runner.start();
    tokio::time::timeout(Duration::from_secs(30), ready)
        .await
        .unwrap()
        .unwrap();
    drop(owner);
    resume.notify_one();
    let settled = harness
        .transport
        .wait(Duration::from_secs(90))
        .await
        .unwrap();
    assert!(settled.settled);
    // Transport settlement precedes the owner's final journal flush. The policy
    // gate joins that flush locally, without issuing another database call.
    harness.policy.persist_final_evidence().await.unwrap();
    harness.studio.shutdown_and_wait().await.unwrap();
    let journal: Vec<Value> = fs::read_to_string(harness.journal.path().join(FILE_NAME))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        journal
            .iter()
            .filter(|record| record["event"]["type"] == "validatedPhase")
            .count(),
        32
    );
    assert_eq!(
        journal
            .iter()
            .filter(|record| record["event"]["type"] == "operationCompletion")
            .count(),
        32
    );
    assert_eq!(fs::read_dir(&harness.fixture.state).unwrap().count(), 0);
    let queries: Vec<String> = fs::read_to_string(harness.fixture.controls.join("queries"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let ledger = harness
        .plan
        .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
        .unwrap();
    assert_eq!(
        queries,
        ledger
            .entries()
            .iter()
            .map(|entry| entry.sql().to_owned())
            .collect::<Vec<_>>()
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
        let retained = receipt.original_call_failure.as_ref().unwrap();
        assert!(retained.capture_complete);
        assert!(retained.original_error.is_none());
        assert_eq!(
            serde_json::to_value(&retained.events).unwrap(),
            completion["original"]["events"]
        );
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

async fn drift_failure(kind: &str, schemas: usize, phase: u8, stop_ordinal: u16) {
    let receipt = start(schemas, json!({"kind":kind})).await.unwrap();
    let completion = assert_stopped_at(&receipt, phase);
    assert!(receipt.persisted.is_err());
    assert_eq!(completion["status"], "failed");
    assert_eq!(completion["original"]["events"], json!([]));
    let original_error = &receipt.failure.as_ref().unwrap().1;
    assert_eq!(original_error.code, ErrorCode::InvalidInput);
    assert_eq!(
        completion["original"]["error"],
        serde_json::to_value(original_error).unwrap()
    );
    assert_eq!(receipt.outputs.len(), usize::from(phase - 1));

    let names = std::iter::once(SCHEMA.to_owned())
        .chain((2..=schemas).map(|n| format!("S{n:02}")))
        .collect::<Vec<_>>();
    let ledger = receipt
        .harness
        .plan
        .expand_physical_ledger(&SchemaCensus::freeze(&names).unwrap())
        .unwrap();
    let stop = ledger
        .entries()
        .iter()
        .position(|entry| entry.phase() == phase && entry.phase_ordinal() == stop_ordinal)
        .unwrap();
    assert_eq!(
        receipt.queries,
        ledger.entries()[..=stop]
            .iter()
            .map(|entry| entry.sql().to_owned())
            .collect::<Vec<_>>(),
        "drift must stop at its first rejecting callback: {kind}"
    );
    assert!(
        !receipt
            .queries
            .iter()
            .any(|sql| sql.starts_with("ALTER TABLE ") || sql.starts_with("DROP TABLE "))
    );
    assert!(
        receipt
            .journal
            .iter()
            .all(|record| record["event"]["report"]["certainty"] != "OutcomeUnknown")
    );
    let frozen = receipt
        .journal
        .iter()
        .filter(|record| record["event"]["type"] == "frozenLedger")
        .count();
    assert_eq!(frozen, usize::from(phase > 2));
    let read_reports = receipt
        .journal
        .iter()
        .filter(|record| {
            record["event"]["admission"]["kind"]["sql"]["ordinal"]
                == ledger.entries()[stop].ordinal()
        })
        .collect::<Vec<_>>();
    assert_eq!(read_reports.len(), 1);
    assert_eq!(read_reports[0]["event"]["report"]["ok"], true);
    assert!(read_reports[0]["event"]["report"]["retirement"].is_null());

    // Reconciliation drift cannot retroactively erase or downgrade the original
    // CREATE. Earlier failures must never fabricate a mutation or unknown ACK.
    assert_eq!(
        receipt
            .queries
            .iter()
            .filter(|sql| sql.starts_with("CREATE TABLE "))
            .count(),
        usize::from(phase == 4)
    );
    let creates = receipt
        .journal
        .iter()
        .filter(|record| {
            record["event"]["type"] == "operationCompletion"
                && record["event"]["phase"] == 3
                && record["event"]["completion"]["status"] == "returned"
        })
        .collect::<Vec<_>>();
    if phase == 4 {
        assert_eq!(creates.len(), 1);
        let original = receipt.outputs[2].evidence().record().unwrap();
        assert_eq!(creates[0]["event"]["completion"]["original"], original);
        assert_eq!(original["value"]["statementOutcome"], "succeeded");
        assert_eq!(original["value"]["retired"], false);
        assert!(original["value"]["error"].is_null());
        let acks = receipt
            .journal
            .iter()
            .filter(|record| !record["event"]["report"]["ddlAcknowledged"].is_null())
            .collect::<Vec<_>>();
        assert_eq!(acks.len(), 1);
        let report = &acks[0]["event"]["report"];
        assert_eq!(report["ok"], true);
        assert_eq!(report["certainty"], "StatementSucceeded");
        let ack = &report["ddlAcknowledged"]["statement"];
        assert!(ack["sdkRequestId"].as_str().is_some_and(|v| !v.is_empty()));
        assert!(ack["queryId"].as_str().is_some_and(|v| !v.is_empty()));
        assert_eq!(report["statements"].as_array().unwrap().len(), 1);
        assert_eq!(&report["statements"][0], ack);
    } else {
        assert!(creates.is_empty());
        assert!(
            receipt
                .journal
                .iter()
                .all(|record| record["event"]["report"]["ddlAcknowledged"].is_null())
        );
    }

    let markers = fs::read_to_string(receipt.harness.fixture.controls.join("drift-events"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        markers.len(),
        1,
        "unexercised or repeated drift injection: {kind}"
    );
    let marker = &markers[0];
    assert_eq!(marker["kind"], kind);
    let injection_ordinal = if kind.starts_with("context") {
        1
    } else {
        stop_ordinal
    };
    let injection = ledger
        .entries()
        .iter()
        .find(|entry| entry.phase() == phase && entry.phase_ordinal() == injection_ordinal)
        .unwrap();
    assert_eq!(marker["ordinal"], injection.ordinal());
    assert_eq!(marker["sql"], injection.sql());
    let before = marker["before"].as_array().unwrap();
    let after = marker["after"].as_array().unwrap();
    assert_ne!(before, after);
    if kind.starts_with("census") {
        let baseline = names
            .iter()
            .map(|name| json!(["SANDBOX", name]))
            .collect::<Vec<_>>();
        assert_eq!(before, &baseline);
        let mut expected = baseline;
        match kind {
            "censusOverflow" => {
                assert_eq!(schemas, 32);
                expected.push(json!(["SANDBOX", "S33"]));
                assert_eq!(after.len(), 33);
            }
            "censusReversed" => expected.reverse(),
            "censusBeforeCreate" | "censusCatalog" => {
                assert_eq!(schemas, 2);
                expected[1][1] = json!("S03");
            }
            _ => unreachable!("closed census case"),
        }
        assert_eq!(after, &expected);
    } else {
        assert_eq!(before.len(), 1);
        assert_eq!(after.len(), 1);
        let field = match kind {
            "contextRoleBeforePin" => "ROLE_NAME",
            "contextWarehouseBeforePin" => "WAREHOUSE_NAME",
            "contextSecondaryRoles" => "SECONDARY_ROLES",
            "contextSearchPath" => "SEARCH_PATH",
            _ => unreachable!("closed context case"),
        };
        let column = marker["names"]
            .as_array()
            .unwrap()
            .iter()
            .position(|name| name.as_str() == Some(field))
            .unwrap();
        let mut expected = before[0].as_array().unwrap().clone();
        expected[column] = match kind {
            "contextRoleBeforePin" => json!("OTHER_ROLE"),
            "contextWarehouseBeforePin" => json!("OTHER_WAREHOUSE"),
            "contextSecondaryRoles" => json!(r#"{"roles":"EXTRA_ROLE","value":"ALL"}"#),
            "contextSearchPath" => json!(r#"["SANDBOX.BILLIE_TESTING","SANDBOX.S02"]"#),
            _ => unreachable!("closed context case"),
        };
        assert_eq!(after[0], json!(expected));
        // All other fields, including bootstrap principal/server-session, stay
        // unchanged; these are coherent valid context rows, not parse failures.
        assert_eq!(before[0][0], "SERVER_USER");
        assert_eq!(before[0][5], "SESSION");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_drift_rejects_initial_census_overflow_and_wire_order() {
    drift_failure("censusOverflow", 32, 2, 6).await;
    drift_failure("censusReversed", 2, 2, 6).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_drift_stops_before_create_and_catalog_dependencies() {
    drift_failure("censusBeforeCreate", 2, 3, 6).await;
    drift_failure("censusCatalog", 2, 4, 14).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_drift_rejects_role_and_warehouse_before_first_pin() {
    drift_failure("contextRoleBeforePin", 2, 2, 6).await;
    drift_failure("contextWarehouseBeforePin", 2, 2, 6).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_runner_drift_rejects_secondary_roles_and_search_path_after_pin() {
    drift_failure("contextSecondaryRoles", 2, 4, 6).await;
    drift_failure("contextSearchPath", 2, 4, 6).await;
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
        assert!(receipt.completion_record_error.is_none());
        assert!(receipt.original_call_failure.is_none());
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
