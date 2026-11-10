#![cfg(all(unix, feature = "test-harness"))]

use joinery_core::drivers::snowflake_process::*;
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use tokio::time::timeout;

fn explicit_node() -> PathBuf {
    let node = PathBuf::from(
        std::env::var_os("JOINERY_TEST_NODE")
            .expect("JOINERY_TEST_NODE must name an explicit absolute test Node executable"),
    );
    assert!(
        node.is_absolute() && node.is_file(),
        "JOINERY_TEST_NODE must be an absolute executable file"
    );
    node
}

const LIMIT: Duration = Duration::from_secs(2);
const GENERATION: &str = "00000000-0000-4000-8000-000000000001";

#[tokio::test]
async fn unused_launch_and_failed_spawn_publish_cleanup_receipts() {
    for fail_spawn in [false, true] {
        let mut launch = launch();
        let root = launch.state_directory().to_owned();
        let observer = launch.cleanup_observer();
        if fail_spawn {
            launch.executable = root.join("missing-executable");
            assert!(
                Supervisor::spawn(launch, GENERATION, bounds())
                    .await
                    .is_err()
            );
        } else {
            // This also covers dropping a spawn future before its first poll.
            drop(Supervisor::spawn(launch, GENERATION, bounds()));
        }
        let receipt = observer
            .wait(LIMIT)
            .await
            .expect("no-child cleanup receipt");
        assert!(!receipt.child_started);
        assert!(!receipt.reaped);
        assert!(receipt.temporary_root_removed);
        assert!(!root.exists());
    }
}

fn launch() -> Launch {
    Launch::new(
        PathBuf::from(env!("CARGO_BIN_EXE_snowflake-protocol-fixture")),
        None,
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        tempfile::tempdir().unwrap(),
    )
    .unwrap()
}
fn bounds() -> Bounds {
    Bounds {
        handshake: LIMIT,
        frame: Duration::from_millis(150),
        write: Duration::from_millis(200),
        exit: Duration::from_millis(250),
    }
}
async fn spawn(scenario: &str) -> Supervisor {
    let supervisor = Supervisor::spawn(launch(), GENERATION, bounds())
        .await
        .unwrap();
    let mut connect = supervisor
        .request(
            "connect",
            "connect",
            json!({"scenario":scenario}),
            Impact::Control,
        )
        .unwrap();
    assert!(connect.wait_bounded(LIMIT).await.ok);
    supervisor
}
fn query(supervisor: &Supervisor, id: &str, impact: Impact, credits: u32) -> Operation {
    supervisor.request(id,"query",json!({"sql":"SELECT 1","binds":[],"classification":"read","credits":credits,"maxRows":500,"maxBytes":2097152}),impact).unwrap()
}
async fn columns(operation: &mut Operation) {
    timeout(LIMIT, async {
        while operation.columns.borrow().is_none() {
            operation.columns.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn child_has_explicit_paths_no_arguments_or_inherited_credentials() {
    let supervisor = spawn("healthy").await;
    let mut context = supervisor
        .request("context", "context", json!({}), Impact::Read)
        .unwrap();
    let report = context.wait_bounded(LIMIT).await;
    assert!(report.ok);
    let result = report.result.unwrap();
    assert_eq!(result["args"], json!([]));
    let environment = result["env"].as_array().unwrap();
    assert!(
        environment
            .iter()
            .any(|entry| entry[0] == "JOINERY_SNOWFLAKE_STATE_DIR")
    );
    for entry in environment {
        let key = entry[0].as_str().unwrap();
        if key == "JOINERY_SNOWFLAKE_STATE_DIR" {
            continue;
        }
        // The linked macOS fixture initializes this encoding value after exec,
        // despite env_clear. It is not an allowed launcher input or arbitrary env.
        assert!(
            cfg!(target_os = "macos") && key == "__CF_USER_TEXT_ENCODING",
            "unexpected child environment key: {key}"
        );
        let value = entry[1].as_str().unwrap();
        let parts: Vec<_> = value.split(':').collect();
        assert!(
            value.len() <= 64
                && parts.len() == 3
                && parts
                    .iter()
                    .all(|part| part
                        .strip_prefix("0x")
                        .is_some_and(|hex| !hex.is_empty()
                            && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))),
            "invalid OS-generated encoding value"
        );
    }
    assert!(supervisor.close().await.unwrap().ok);
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn exact_strings_and_duplicate_columns_survive_actual_child_framing() {
    let supervisor = spawn("healthy").await;
    let mut operation = query(&supervisor, "query", Impact::Read, 1);
    let batch = timeout(LIMIT, operation.rows.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        batch.rows,
        vec![vec![Some("9007199254740993".into()), Some("exact".into())]]
    );
    let report = operation.wait_bounded(LIMIT).await;
    assert!(report.ok);
    assert_eq!(operation.columns.borrow().as_ref().unwrap()[0].name, "same");
    assert_eq!(operation.columns.borrow().as_ref().unwrap()[1].name, "same");
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    supervisor.close().await;
}

#[tokio::test]
async fn sent_mutation_crash_is_unknown_even_without_dispatch_event() {
    for scenario in ["crash_no_phase", "crash_dispatch"] {
        let supervisor = spawn(scenario).await;
        let mut operation = query(&supervisor, "write", Impact::Mutating, 0);
        let report = operation.wait_bounded(LIMIT).await;
        assert!(!report.ok);
        assert_eq!(report.certainty, Certainty::OutcomeUnknown);
        assert_eq!(report.error_code.as_deref(), Some("outcomeUnknown"));
        assert!(supervisor.is_closed());
        assert!(supervisor.wait_exited().await);
        assert!(
            supervisor
                .request("retry", "query", json!({}), Impact::Mutating)
                .is_err()
        );
    }
}

#[tokio::test]
async fn acknowledged_success_is_not_rewritten_as_unknown_by_eof() {
    for (impact, expected) in [
        (Impact::Mutating, Certainty::StatementSucceeded),
        (Impact::Commit, Certainty::Committed),
        (Impact::Rollback, Certainty::RolledBack),
    ] {
        let supervisor = spawn("crash_ack").await;
        let mut operation = query(&supervisor, "operation", impact, 0);
        let report = operation.wait_bounded(LIMIT).await;
        assert!(!report.ok);
        assert_eq!(report.certainty, expected);
        assert_eq!(report.phase, Phase::Acknowledged);
        assert!(supervisor.wait_exited().await);
    }
}

#[tokio::test]
async fn malformed_frames_or_forged_routing_retire_and_reap() {
    for scenario in [
        "wrong_generation",
        "wrong_id",
        "oversize",
        "partial",
        "invalid_json",
        "without_credit",
        "early_columns",
        "duplicate_accepted",
    ] {
        let supervisor = spawn(scenario).await;
        let mut operation = query(&supervisor, "query", Impact::Read, 0);
        let report = operation.wait_bounded(LIMIT).await;
        assert!(!report.ok, "{scenario}");
        assert!(supervisor.is_closed(), "{scenario}");
        assert_eq!(report.retirement, Some(Retirement::Protocol), "{scenario}");
        assert!(supervisor.wait_exited().await, "{scenario}");
    }
}

#[tokio::test]
async fn cancel_and_close_bypass_zero_credit_stream_without_claiming_rollback() {
    let supervisor = spawn("backpressure").await;
    let mut operation = query(&supervisor, "query", Impact::Read, 0);
    columns(&mut operation).await;
    let mut cancel = supervisor.cancel(&operation.token()).unwrap();
    assert!(cancel.wait_bounded(LIMIT).await.ok);
    let report = operation.wait_bounded(LIMIT).await;
    assert!(!report.ok);
    assert_eq!(report.error_code.as_deref(), Some("cancelled"));
    assert_ne!(report.certainty, Certainty::RolledBack);
    assert!(supervisor.close().await.unwrap().ok);
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn cancel_request_acknowledgement_is_not_the_original_query_terminal() {
    let supervisor = spawn("cancel_ack_only").await;
    let mut operation = query(&supervisor, "query", Impact::Mutating, 0);
    columns(&mut operation).await;
    let mut cancel = supervisor.cancel(&operation.token()).unwrap();
    assert!(cancel.wait_bounded(LIMIT).await.ok);
    assert!(operation.report().is_none());
    let report = operation.wait_bounded(Duration::from_millis(40)).await;
    // Fixture had already sent a successful statement acknowledgement. Preserve it.
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    assert!(!report.ok);
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn credits_are_bounded_and_can_be_discarded_without_waiting_for_consumer() {
    let supervisor = spawn("backpressure").await;
    let mut operation = query(&supervisor, "query", Impact::Read, 0);
    columns(&mut operation).await;
    assert!(supervisor.credit(&operation.token(), 33).is_err());
    let mut credit = supervisor.credit(&operation.token(), 32).unwrap();
    assert!(credit.wait_bounded(LIMIT).await.ok);
    // Do not consume these rows: the reader must still process the discard terminal.
    let mut discard = supervisor.discard(&operation.token()).unwrap();
    assert!(discard.wait_bounded(LIMIT).await.ok);
    assert!(operation.wait_bounded(LIMIT).await.ok);
    let mut context = supervisor
        .request("count", "context", json!({}), Impact::Read)
        .unwrap();
    assert_eq!(
        context.wait_bounded(LIMIT).await.result.unwrap()["executions"],
        1
    );
    supervisor.close().await;
}

#[tokio::test]
async fn stale_cloned_tokens_cannot_control_a_reused_id_on_the_live_child() {
    let supervisor = spawn("backpressure").await;
    let mut old = query(&supervisor, "same", Impact::Read, 0);
    let stale = old.token().clone();
    columns(&mut old).await;
    let mut discard = supervisor.discard(&old.token()).unwrap();
    assert!(discard.wait_bounded(LIMIT).await.ok);
    assert!(old.wait_bounded(LIMIT).await.ok);
    let mut replacement = query(&supervisor, "same", Impact::Read, 0);
    columns(&mut replacement).await;
    assert!(supervisor.credit(&stale, 1).is_err());
    assert!(supervisor.cancel(&stale).is_err());
    assert!(supervisor.discard(&stale).is_err());
    assert!(replacement.report().is_none());
    assert!(!supervisor.is_closed());
    let mut cancel = supervisor.cancel(&replacement.token()).unwrap();
    assert!(cancel.wait_bounded(LIMIT).await.ok);
    assert_eq!(
        replacement.wait_bounded(LIMIT).await.error_code.as_deref(),
        Some("cancelled")
    );
    assert!(supervisor.close().await.unwrap().ok);
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn late_stream_error_is_not_completed_despite_statement_acknowledgement() {
    let supervisor = spawn("late_error").await;
    let mut operation = query(&supervisor, "query", Impact::Read, 1);
    assert_eq!(
        operation.rows.recv().await.unwrap().rows,
        vec![vec![Some("retained".into())]]
    );
    let report = operation.wait_bounded(LIMIT).await;
    assert!(!report.ok);
    assert_eq!(report.error_code.as_deref(), Some("database"));
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    supervisor.close().await;
}

#[tokio::test]
async fn duplicate_ids_and_reconnect_are_rejected_before_transmission() {
    let supervisor = spawn("healthy").await;
    let mut first = query(&supervisor, "unique", Impact::Read, 0);
    assert!(first.wait_bounded(LIMIT).await.ok);
    let mut reused = query(&supervisor, "unique", Impact::Read, 0);
    assert!(
        supervisor
            .request("unique", "query", json!({}), Impact::Mutating)
            .is_err()
    );
    assert!(reused.wait_bounded(LIMIT).await.ok);
    assert!(
        supervisor
            .request("connect2", "connect", json!({}), Impact::Control)
            .is_err()
    );
    let mut count = supervisor
        .request("count", "context", json!({}), Impact::Read)
        .unwrap();
    assert_eq!(
        count.wait_bounded(LIMIT).await.result.unwrap()["executions"],
        2
    );
    supervisor.close().await;
}

#[tokio::test]
async fn dropped_consumer_discards_then_healthy_connection_remains_usable() {
    let supervisor = spawn("backpressure").await;
    let mut operation = query(&supervisor, "query", Impact::Read, 0);
    columns(&mut operation).await;
    drop(operation);
    let mut context = supervisor
        .request("context", "context", json!({}), Impact::Read)
        .unwrap();
    assert!(context.wait_bounded(LIMIT).await.ok);
    assert!(!supervisor.is_closed());
    supervisor.close().await;
}

#[tokio::test]
async fn errors_never_expose_child_payload_or_unsanitized_sdk_messages() {
    let supervisor = spawn("secret_error").await;
    let secret = "NEVER-LOG-THIS-fixture-value";
    let mut operation = supervisor
        .request(
            "secret",
            "query",
            json!({"sql":secret,"credits":0}),
            Impact::Read,
        )
        .unwrap();
    let report = operation.wait_bounded(LIMIT).await;
    assert!(!format!("{report:?}").contains(secret));
    assert_eq!(report.error_code.as_deref(), Some("database"));
    supervisor.close().await;
}

#[tokio::test]
async fn pending_externalbrowser_simulation_has_bounded_close_and_reaping() {
    let supervisor = Supervisor::spawn(launch(), GENERATION, bounds())
        .await
        .unwrap();
    let mut connect = supervisor
        .request(
            "connect",
            "connect",
            json!({"scenario":"auth_hang"}),
            Impact::Control,
        )
        .unwrap();
    assert!(supervisor.close().await.unwrap().ok);
    let report = connect.wait_bounded(LIMIT).await;
    assert!(!report.ok);
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn frame_codec_handles_fragmentation_and_rejects_unbounded_allocation() {
    let (mut send, mut receive) = tokio::io::duplex(16);
    let writer = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        for byte in [0, 0, 0, 2, b'{', b'}'] {
            send.write_all(&[byte]).await.unwrap();
        }
    });
    assert_eq!(
        read_frame(&mut receive, LIMIT).await.unwrap(),
        Some(json!({}))
    );
    writer.await.unwrap();
    let (mut send, mut receive) = tokio::io::duplex(8);
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        send.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
    });
    assert!(read_frame(&mut receive, LIMIT).await.is_err());
    let mut invalid = launch();
    invalid.executable = PathBuf::from("snowflake-protocol-fixture");
    assert!(
        Supervisor::spawn(invalid, GENERATION, bounds())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn queued_cancel_never_reaches_child_and_partial_write_remains_unknown() {
    let supervisor = spawn("stop_reading").await;
    let mut first = supervisor
        .request(
            "large",
            "query",
            json!({"sql":"x".repeat(900_000),"credits":0}),
            Impact::Mutating,
        )
        .unwrap();
    timeout(LIMIT, async {
        while first.phase() == Phase::Queued {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(first.phase(), Phase::Sending);
    let mut second = query(&supervisor, "unsent", Impact::Mutating, 0);
    let mut cancel = supervisor.cancel(&second.token()).unwrap();
    assert_eq!(
        cancel.wait_bounded(LIMIT).await.result.unwrap()["cancelledLocally"],
        true
    );
    let report = second.wait_bounded(LIMIT).await;
    assert_eq!(report.certainty, Certainty::NotSubmitted);
    assert_eq!(report.phase, Phase::Queued);
    let report = first.wait_bounded(LIMIT).await;
    assert_eq!(report.certainty, Certainty::OutcomeUnknown);
    assert_eq!(report.retirement, Some(Retirement::Write));
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn wedged_close_is_killed_and_reaped_within_bound() {
    let supervisor = spawn("wedged_close").await;
    assert!(supervisor.close().await.is_none());
    assert!(supervisor.is_closed());
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn bounded_sdk_identifiers_survive_child_loss_without_becoming_retry_inputs() {
    let supervisor = spawn("statement_ids").await;
    let mut operation = query(&supervisor, "write", Impact::Mutating, 0);
    let report = operation.wait_bounded(LIMIT).await;
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    assert_eq!(
        report.statements,
        vec![StatementIdentity {
            sdk_request_id: "00000000-0000-4000-8000-000000000003".into(),
            query_id: Some("00000000-0000-4000-8000-000000000004".into())
        }]
    );
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn repeated_million_row_streams_use_bounded_credit_and_no_lifetime_id_set() {
    timeout(Duration::from_secs(90), async {
        let supervisor = spawn("million").await;
        for pass in 0..2 {
            let mut operation = query(&supervisor, "reusable-local-handle", Impact::Read, 0);
            columns(&mut operation).await;
            let mut seen = 0u64;
            while seen < 1_000_000 {
                let mut credit = supervisor.credit(&operation.token(), 1).unwrap();
                assert!(credit.wait_bounded(LIMIT).await.ok);
                let batch = timeout(LIMIT, operation.rows.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert!(batch.rows.len() <= 128);
                for row in &batch.rows {
                    assert_eq!(row[0].as_deref(), Some(seen.to_string().as_str()));
                    seen += 1;
                }
                drop(batch);
            }
            let report = operation.wait_bounded(LIMIT).await;
            assert!(report.ok, "pass {pass}");
            assert_eq!(report.result.unwrap()["rowsRetained"], 1_000_000);
        }
        assert!(!supervisor.is_closed());
        assert!(supervisor.close().await.unwrap().ok);
        assert!(supervisor.wait_exited().await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn actual_node_helper_interoperability_context_and_graceful_close() {
    // Deliberately explicit local Node; never discover it through PATH/global profiles.
    let node = explicit_node();
    assert!(
        node.is_file(),
        "pinned local Node required for this scratch interop test"
    );
    let launch = Launch::new(
        node,
        Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/support/snowflake_node_fixture.mjs"),
        ),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        tempfile::tempdir().unwrap(),
    )
    .unwrap();
    let supervisor = Supervisor::spawn(launch, GENERATION, Bounds::default())
        .await
        .unwrap();
    let mut connect=supervisor.request("connect","connect",json!({"account":"org-account","host":"org-account.snowflakecomputing.com","username":"LOGIN_NAME","role":"ROLE","warehouse":"WH","database":"DB","schema":"PUBLIC"}),Impact::Control).unwrap();
    assert!(connect.wait_bounded(LIMIT).await.ok);
    let mut context = supervisor
        .request("context", "context", json!({}), Impact::Read)
        .unwrap();
    let report = context.wait_bounded(LIMIT).await;
    assert!(report.ok, "{report:?}");
    assert_eq!(report.statements.len(), 1);
    assert!(report.use_acknowledged.is_none());
    assert_eq!(report.result.unwrap()["schema"], "PUBLIC");
    let mut query = query(&supervisor, "query", Impact::Read, 0);
    columns(&mut query).await;
    // Helper emits this query's connectionClosed before awaiting destroy. Do not kill early.
    let (first, second) = tokio::join!(supervisor.shutdown(), supervisor.shutdown());
    for receipt in [first, second] {
        assert!(receipt.logout_acknowledged, "{receipt:?}");
        assert_eq!(
            receipt.cleanup,
            Some(CleanupReport {
                child_started: true,
                reaped: true,
                temporary_root_removed: true
            })
        );
    }
    let interrupted = query.wait_bounded(LIMIT).await;
    assert!(!interrupted.ok);
    assert_eq!(interrupted.certainty, Certainty::StatementSucceeded);
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn pending_connect_cancel_bypasses_auth_and_reaps_without_reconnect() {
    let supervisor = Supervisor::spawn(launch(), GENERATION, bounds())
        .await
        .unwrap();
    let mut connect = supervisor
        .request(
            "connect",
            "connect",
            json!({"scenario":"auth_hang"}),
            Impact::Control,
        )
        .unwrap();
    timeout(LIMIT, async {
        while connect.phase() == Phase::Queued {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut cancel = supervisor.cancel(&connect.token()).unwrap();
    assert!(cancel.wait_bounded(LIMIT).await.ok);
    assert_eq!(
        connect.wait_bounded(LIMIT).await.error_code.as_deref(),
        Some("cancelled")
    );
    assert!(supervisor.wait_exited().await);
    assert!(supervisor.is_closed());
}

#[tokio::test]
async fn close_before_connect_transmission_never_violates_first_connect_envelope() {
    let supervisor = Supervisor::spawn(launch(), GENERATION, bounds())
        .await
        .unwrap();
    let report = supervisor.close().await.unwrap();
    assert!(report.ok);
    assert_eq!(report.certainty, Certainty::NotSubmitted);
    assert_eq!(report.result.unwrap()["logoutAcknowledged"], false);
    assert!(supervisor.wait_exited().await);
}

fn switch_payload(schema: &str) -> serde_json::Value {
    json!({"database":"DB","schema":schema,"expected":{"user":"SERVER_USER","role":null,"warehouse":null,"session":"OPAQUE-SESSION"}})
}

// Real Node/framing/Helper, injected local SDK only. This private script lives in
// the owned auth root and performs no account access or external network work.
fn switch_launch(scenario: &str) -> Launch {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("switch-fixture.mjs");
    let helper = serde_json::to_string(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../snowflake-helper/src/helper.mjs"),
    )
    .unwrap();
    let framing = serde_json::to_string(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../snowflake-helper/src/framing.mjs"),
    )
    .unwrap();
    let body = r#"
import { Readable } from 'node:stream';
import { writeFileSync } from 'node:fs';
const { Helper } = await import(HELPER_PATH);
const { Decoder, writer } = await import(FRAMING_PATH);
const scenario = SCENARIO;
const uuid = n => `00000000-0000-4000-8000-${String(n).padStart(12,'0')}`;
let count = 0, schema = 'PUBLIC', stopping = false;
const sdk = { createConnection: () => ({
  connectAsync(cb) { queueMicrotask(() => cb(null)); return Promise.resolve(); },
  destroy(cb) { cb(null); },
  execute(options) {
    const n = ++count, use = options.sqlText.startsWith('USE SCHEMA');
    if (use) {
      const parts = /^USE SCHEMA "((?:[^"]|"")*)"\."((?:[^"]|"")*)"$/u.exec(options.sqlText);
      if (!parts || parts[1] !== 'DB') throw Error('fixture SQL mismatch');
      schema = parts[2].replaceAll('""','"');
    }
    writeFileSync(process.env.JOINERY_SNOWFLAKE_STATE_DIR + '/stage', use ? 'use' : 'readback');
    let acknowledged = false;
    const stmt = {
      getRequestId: () => uuid(n), getQueryId: () => acknowledged ? uuid(n+100) : undefined,
      getColumns: () => Array.from({length:use?0:6},(_,i)=>({getName:()=>`c${i}`,getType:()=> 'TEXT'})),
      streamRows: () => Readable.from(use ? [] : [['SERVER_USER',null,'DB',scenario==='mismatch'?'WRONG':schema,null,'OPAQUE-SESSION']]),
      cancel(cb) { cb(null); }
    };
    if ((use && scenario==='use_pending') || (!use && scenario==='readback_pending')) return stmt;
    setImmediate(() => {
      if ((use && scenario==='use_denied') || (!use && scenario==='readback_denied')) options.complete({sqlState:'42501'},stmt);
      else { acknowledged = true; options.complete(null,stmt); }
    });
    return stmt;
  }
})};
const send = writer(process.stdout.write.bind(process.stdout),()=>process.exit(2));
const stop = () => { if (!stopping) { stopping=true; send.flush().then(()=>process.exit(0)); } };
const helper = new Helper(sdk,send,stop,{clientConfigFile:'/nonexistent/local-fixture',closeTimeoutMs:20});
const decoder = new Decoder();
process.stdin.on('data', chunk=> { try { for (const frame of decoder.push(chunk)) helper.receive(frame); } catch { stop(); } });
process.stdin.on('end',stop);
"#;
    std::fs::write(
        &script,
        body.replace("HELPER_PATH", &helper)
            .replace("FRAMING_PATH", &framing)
            .replace("SCENARIO", &serde_json::to_string(scenario).unwrap()),
    )
    .unwrap();
    Launch::new(
        explicit_node(),
        Some(script),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        root,
    )
    .unwrap()
}
async fn switch_supervisor(scenario: &str) -> (Supervisor, PathBuf) {
    let launch = switch_launch(scenario);
    let root = launch.state_directory().to_owned();
    let supervisor = Supervisor::spawn(launch, GENERATION, Bounds::default())
        .await
        .unwrap();
    let mut connect = supervisor.request("connect","connect",json!({"account":"org-account","host":"org-account.snowflakecomputing.com","username":"LOGIN_NAME","role":"ROLE","warehouse":"WH","database":"DB","schema":"PUBLIC"}),Impact::Control).unwrap();
    assert!(connect.wait_bounded(LIMIT).await.ok);
    (supervisor, root)
}
async fn wait_switch_stage(root: &std::path::Path, expected: &str) {
    timeout(LIMIT, async {
        loop {
            if std::fs::read_to_string(root.join("stage")).is_ok_and(|stage| stage == expected) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn actual_node_switch_quotes_names_preserves_null_role_and_owns_two_statements() {
    let (supervisor, root) = switch_supervisor("healthy").await;
    assert!(
        supervisor
            .request(
                "bad-impact",
                "switchSchema",
                switch_payload("OTHER"),
                Impact::Read
            )
            .is_err()
    );
    assert!(
        supervisor
            .request("bad-read", "context", json!({}), Impact::ContextChange)
            .is_err()
    );
    let schema = "Sales.\"雪; --";
    let mut op = supervisor
        .request(
            "switch",
            "switchSchema",
            switch_payload(schema),
            Impact::ContextChange,
        )
        .unwrap();
    let report = op.wait_bounded(LIMIT).await;
    assert!(report.ok, "{report:?}");
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    assert_eq!(report.phase, Phase::Acknowledged);
    assert_eq!(report.statements.len(), 2);
    assert_eq!(report.use_acknowledged, report.statements.first().cloned());
    let result = report.result.unwrap();
    assert_eq!(result["schema"], schema);
    assert_eq!(result["role"], serde_json::Value::Null);
    assert_eq!(result["warehouse"], serde_json::Value::Null);
    assert!(
        supervisor
            .shutdown()
            .await
            .cleanup
            .unwrap()
            .temporary_root_removed
    );
    assert!(!root.exists());
}
#[tokio::test]
async fn actual_node_switch_rejection_differs_from_unverified_acknowledged_use() {
    for scenario in ["use_denied", "readback_denied", "mismatch"] {
        let (supervisor, root) = switch_supervisor(scenario).await;
        let mut op = supervisor
            .request(
                "switch",
                "switchSchema",
                switch_payload("OTHER"),
                Impact::ContextChange,
            )
            .unwrap();
        let report = op.wait_bounded(LIMIT).await;
        assert!(!report.ok, "{scenario}: {report:?}");
        if scenario == "use_denied" {
            assert_eq!(report.certainty, Certainty::ServerTerminal);
            assert_eq!(report.error_code.as_deref(), Some("database"));
            assert!(report.use_acknowledged.is_none());
            assert_eq!(report.statements.len(), 1);
        } else {
            assert_eq!(report.certainty, Certainty::OutcomeUnknown);
            assert_eq!(report.error_code.as_deref(), Some("connectionClosed"));
            assert_eq!(report.use_acknowledged, report.statements.first().cloned());
            assert_eq!(report.statements.len(), 2);
            assert_eq!(report.phase, Phase::Dispatching);
        }
        assert!(supervisor.wait_exited().await);
        assert!(!root.exists());
    }
}
#[tokio::test]
async fn actual_node_switch_cancel_and_abandon_preserve_ack_evidence_and_reap() {
    for after_use in [false, true] {
        for abandon in [false, true] {
            let (supervisor, root) = switch_supervisor(if after_use {
                "readback_pending"
            } else {
                "use_pending"
            })
            .await;
            let mut op = supervisor
                .request(
                    "switch",
                    "switchSchema",
                    switch_payload("OTHER"),
                    Impact::ContextChange,
                )
                .unwrap();
            wait_switch_stage(&root, if after_use { "readback" } else { "use" }).await;
            timeout(LIMIT, async {
                while op.phase() != Phase::Dispatching
                    || (after_use && op.use_acknowledged().is_none())
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let mut receiver = op.terminal_receiver();
            if abandon {
                drop(op);
            } else {
                let mut cancel = supervisor.cancel(&op.token()).unwrap();
                assert!(cancel.wait_bounded(LIMIT).await.ok);
                assert_eq!(
                    op.wait_bounded(LIMIT).await.certainty,
                    Certainty::OutcomeUnknown
                );
            }
            let report = timeout(LIMIT, async {
                loop {
                    if let Some(report) = receiver.borrow_and_update().clone() {
                        break report;
                    }
                    receiver.changed().await.unwrap();
                }
            })
            .await
            .unwrap();
            assert_eq!(report.certainty, Certainty::OutcomeUnknown);
            assert_eq!(report.use_acknowledged.is_some(), after_use);
            assert_eq!(report.phase, Phase::Dispatching);
            assert!(supervisor.wait_exited().await);
            assert!(!root.exists());
        }
    }
}

#[tokio::test]
async fn graceful_close_atomically_rejects_new_work_during_its_ack_window() {
    let supervisor = spawn("wedged_close").await;
    let mut close = Box::pin(supervisor.close());
    assert!(
        timeout(Duration::from_millis(20), &mut close)
            .await
            .is_err()
    );
    assert!(supervisor.is_closed());
    assert!(
        supervisor
            .request("too-late", "query", json!({}), Impact::Mutating)
            .is_err()
    );
    assert!(close.await.is_none());
    assert!(supervisor.wait_exited().await);
}

#[tokio::test]
async fn abandoning_close_future_still_forces_and_reaps_wedged_child() {
    let supervisor = spawn("wedged_close").await;
    let mut close = Box::pin(supervisor.close());
    assert!(
        timeout(Duration::from_millis(20), &mut close)
            .await
            .is_err()
    );
    drop(close);
    assert!(
        supervisor.wait_exited().await,
        "abandoned close future stranded its child process"
    );
    assert!(supervisor.is_closed());
}

#[tokio::test]
async fn cancelled_handshake_reaps_owned_child_and_removes_parent_root() {
    let working = tempfile::tempdir().unwrap();
    std::fs::write(working.path().join("hold-hello"), b"hold").unwrap();
    let state = tempfile::tempdir().unwrap();
    let root = state.path().to_owned();
    let launch = Launch::new(
        PathBuf::from(env!("CARGO_BIN_EXE_snowflake-protocol-fixture")),
        None,
        working.path().to_owned(),
        state,
    )
    .unwrap();
    let cleanup = launch.cleanup_observer();
    let task = tokio::spawn(Supervisor::spawn(
        launch,
        GENERATION,
        Bounds {
            handshake: Duration::from_secs(60),
            ..bounds()
        },
    ));
    timeout(LIMIT, async {
        while !root.join("child.started").is_file() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(cleanup.wait(Duration::ZERO).await.is_none());
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let report = cleanup
        .wait(LIMIT)
        .await
        .expect("cancelled handshake cleanup receipt");
    assert!(report.reaped);
    assert!(report.temporary_root_removed);
    assert!(!root.exists());
}

#[tokio::test]
async fn forced_kill_reaps_and_removes_parent_root_without_claiming_logout() {
    let launch = launch();
    let root = launch.state_directory().to_owned();
    let supervisor = Supervisor::spawn(launch, GENERATION, bounds())
        .await
        .unwrap();
    let mut connect = supervisor
        .request(
            "connect",
            "connect",
            json!({"scenario":"wedged_close"}),
            Impact::Control,
        )
        .unwrap();
    assert!(connect.wait_bounded(LIMIT).await.ok);
    std::fs::write(root.join("owned-cache"), b"synthetic").unwrap();
    let report = supervisor.shutdown().await;
    assert!(!report.logout_acknowledged);
    assert_eq!(
        report.cleanup,
        Some(CleanupReport {
            child_started: true,
            reaped: true,
            temporary_root_removed: true
        })
    );
    assert!(!root.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn reaped_child_root_removal_can_be_retried_without_deleting_a_replacement() {
    let parent = tempfile::tempdir().unwrap();
    let working = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir_in(parent.path()).unwrap();
    let root = state.path().to_owned();
    let retained = parent.path().join("retained-original");
    let launch = Launch::new(
        PathBuf::from(env!("CARGO_BIN_EXE_snowflake-protocol-fixture")),
        None,
        working.path().to_owned(),
        state,
    )
    .unwrap();
    let cleanup = launch.cleanup_observer();
    let supervisor = Supervisor::spawn(launch, GENERATION, bounds())
        .await
        .unwrap();
    let mut connect = supervisor
        .request(
            "connect",
            "connect",
            json!({"scenario":"wedged_close"}),
            Impact::Control,
        )
        .unwrap();
    assert!(connect.wait_bounded(LIMIT).await.ok);
    std::fs::write(root.join("owned-cache"), b"synthetic").unwrap();
    std::fs::rename(&root, &retained).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("sentinel"), b"replacement").unwrap();

    let shutdown = supervisor.shutdown().await;
    assert!(!shutdown.logout_acknowledged);
    let failed = CleanupReport {
        child_started: true,
        reaped: true,
        temporary_root_removed: false,
    };
    assert_eq!(shutdown.cleanup, Some(failed));
    assert_eq!(cleanup.settle(LIMIT).await, Some(failed));
    assert_eq!(
        std::fs::read(root.join("sentinel")).unwrap(),
        b"replacement"
    );
    assert_eq!(
        std::fs::read(retained.join("owned-cache")).unwrap(),
        b"synthetic"
    );

    std::fs::remove_dir_all(&root).unwrap();
    std::fs::rename(&retained, &root).unwrap();
    assert_eq!(
        cleanup.settle(LIMIT).await,
        Some(CleanupReport {
            child_started: true,
            reaped: true,
            temporary_root_removed: true,
        })
    );
    assert!(!root.exists());
    assert!(
        !shutdown.logout_acknowledged,
        "local recovery cannot establish logout"
    );
}

#[tokio::test]
async fn handshake_timeout_returns_failure_and_observable_reaped_root_cleanup() {
    let working = tempfile::tempdir().unwrap();
    std::fs::write(working.path().join("hold-hello"), b"hold").unwrap();
    let state = tempfile::tempdir().unwrap();
    let root = state.path().to_owned();
    let launch = Launch::new(
        PathBuf::from(env!("CARGO_BIN_EXE_snowflake-protocol-fixture")),
        None,
        working.path().to_owned(),
        state,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let cleanup = launch.cleanup_observer();
    let result = Supervisor::spawn(
        launch,
        GENERATION,
        Bounds {
            handshake: Duration::from_millis(40),
            ..bounds()
        },
    )
    .await;
    assert!(matches!(result, Err("helper handshake failed")));
    assert_eq!(
        cleanup.wait(LIMIT).await,
        Some(CleanupReport {
            child_started: true,
            reaped: true,
            temporary_root_removed: true,
        })
    );
    assert!(!root.exists());
}

#[test]
fn abandoned_runtime_retains_root_without_claiming_reaping_or_cleanup() {
    let working = tempfile::tempdir().unwrap();
    std::fs::write(working.path().join("hold-hello"), b"hold").unwrap();
    let state = tempfile::tempdir().unwrap();
    let root = state.path().to_owned();
    let launch = Launch::new(
        PathBuf::from(env!("CARGO_BIN_EXE_snowflake-protocol-fixture")),
        None,
        working.path().to_owned(),
        state,
    )
    .unwrap();
    let cleanup = launch.cleanup_observer();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _attempt = runtime.spawn(Supervisor::spawn(
        launch,
        GENERATION,
        Bounds {
            handshake: Duration::from_secs(60),
            ..bounds()
        },
    ));
    runtime.block_on(async {
        timeout(LIMIT, async {
            while !root.join("child.started").is_file() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    let pid = std::fs::read_to_string(root.join("child.started")).unwrap();
    let pid: u32 = pid.parse().unwrap();
    // Simulate abrupt owned-runtime teardown instead of the required awaited shutdown.
    runtime.shutdown_timeout(LIMIT);
    assert!(
        root.is_dir(),
        "unconfirmed cleanup deleted the child's root"
    );
    let recovery = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(
        recovery
            .block_on(cleanup.wait(Duration::from_millis(20)))
            .is_none()
    );
    // Drive Tokio's orphan reaper and check only this fixture PID before explicit
    // test-owned cleanup. No receipt is fabricated for this independent recovery.
    recovery.block_on(async {
        tokio::process::Command::new("/usr/bin/true")
            .status()
            .await
            .unwrap();
        timeout(LIMIT, async {
            loop {
                let alive = std::process::Command::new("/bin/kill")
                    .args(["-0", &pid.to_string()])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success();
                if !alive {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    assert!(root.is_dir());
    std::fs::remove_dir_all(root).unwrap();
}
