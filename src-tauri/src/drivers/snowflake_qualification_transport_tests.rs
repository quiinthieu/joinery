//! Synthetic pipes exercise admission races and actual receiving/writing paths.
use super::*;
use crate::domain::{AppError, AppResult};
use crate::drivers::snowflake_qualification_transport::{
    Disposition, PhysicalRequest, PhysicalTerminal, Stream, TransportLimits, TransportPolicy,
};
use std::{future::Future, pin::Pin, sync::atomic::AtomicUsize};
use tokio::sync::Notify;

type ObservedRequest = (String, u64, String, Vec<u8>);
type ObservedTerminal = (String, u64, String, String, Certainty);

#[derive(Default)]
struct TerminalPolicy {
    inner: Mutex<Option<std::sync::Weak<Inner>>>,
    operation: Mutex<Option<std::sync::Weak<Operation>>>,
    receiver: Mutex<Option<watch::Receiver<Option<Report>>>>,
    observed: Mutex<Vec<ObservedTerminal>>,
    metadata_denied: AtomicBool,
    retire: AtomicBool,
    reuse: AtomicBool,
    replacement: Mutex<Option<Operation>>,
}
impl crate::drivers::snowflake_qualification_metadata::MetadataObserver for TerminalPolicy {
    fn check(&self) -> AppResult<()> {
        if self.metadata_denied.load(Ordering::SeqCst) {
            Err(AppError::invalid("synthetic metadata failure"))
        } else {
            Ok(())
        }
    }
    fn observe_schemas(
        &self,
        _: crate::drivers::snowflake_qualification_metadata::SchemaObservation<'_>,
    ) -> AppResult<()> {
        Ok(())
    }
    fn observe_relations(
        &self,
        _: crate::drivers::snowflake_qualification_metadata::RelationObservation<'_>,
    ) -> AppResult<()> {
        Ok(())
    }
}
impl TransportPolicy for TerminalPolicy {
    fn admit<'a>(
        &'a self,
        _request: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
    fn terminal(&self, terminal: PhysicalTerminal<'_>) -> AppResult<()> {
        let inner = self
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        let state = inner
            .state
            .try_lock()
            .expect("terminal callback must run outside process mutex");
        assert!(!state.pending.contains_key(terminal.operation_id));
        assert!(
            self.receiver
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .borrow()
                .is_none()
        );
        assert_eq!(terminal.generation, inner.generation);
        assert_eq!(terminal.impact, Impact::Mutating);
        drop(state);
        let operation = self
            .operation
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        let retire = self.retire.load(Ordering::SeqCst);
        let reuse = self.reuse.load(Ordering::SeqCst);
        // A second caller races the unpublished terminal, with no sleeps. The
        // callback cannot finish until this caller has inspected the outcome.
        let replacement = std::thread::spawn(move || {
            let replacement = reuse.then(|| {
                inner
                    .request("write", "query", json!({}), Impact::Read)
                    .unwrap()
            });
            if let Some(replacement) = &replacement {
                assert_ne!(operation.token.ticket, replacement.token.ticket);
                assert!(replacement.dml_acknowledged().is_none());
            }
            if retire {
                inner.retire(Retirement::Requested);
            }
            assert!(
                operation.dml_acknowledged().is_some(),
                "known ACK remains visible during callback"
            );
            assert_eq!(operation.phase(), Phase::Acknowledged);
            replacement
        })
        .join()
        .unwrap();
        *self.replacement.lock().unwrap() = replacement;
        self.observed.lock().unwrap().push((
            terminal.operation_id.into(),
            terminal.ticket,
            terminal.wire_id.into(),
            terminal.operation.into(),
            terminal.report.certainty,
        ));
        Err(AppError::invalid("synthetic terminal policy denial"))
    }
}

#[tokio::test]
async fn qualification_terminal_observation_precedes_publication_and_preserves_original_ack() {
    terminal_observation(false, false, false).await;
}

#[tokio::test]
async fn qualification_terminal_observation_survives_prior_policy_failure() {
    terminal_observation(true, false, false).await;
}

#[tokio::test]
async fn qualification_terminal_observation_preserves_ack_during_retirement_and_id_reuse() {
    terminal_observation(false, true, false).await;
    terminal_observation(false, false, true).await;
}

async fn terminal_observation(prior_failure: bool, retire: bool, reuse: bool) {
    let policy = Arc::new(TerminalPolicy::default());
    policy.retire.store(retire, Ordering::SeqCst);
    policy.reuse.store(reuse, Ordering::SeqCst);
    let attachment = crate::drivers::snowflake_qualification::QualificationAttachment::new(
        policy.clone(),
        TransportLimits::default(),
    )
    .unwrap();
    let transport = attachment.transport.clone();
    let (inner, _normal, _control) = isolated(transport.clone());
    *policy.inner.lock().unwrap() = Some(Arc::downgrade(&inner));
    let operation = Arc::new(
        inner
            .request("write", "dml", json!({"kind":"insert"}), Impact::Mutating)
            .unwrap(),
    );
    *policy.operation.lock().unwrap() = Some(Arc::downgrade(&operation));
    *policy.receiver.lock().unwrap() = Some(operation.terminal_receiver());
    if prior_failure {
        policy.metadata_denied.store(true, Ordering::SeqCst);
        assert!(attachment.metadata.check().is_err());
        assert!(transport.check().is_err());
    }
    let ticket = operation.token().ticket;
    {
        let mut state = inner.state.lock().unwrap();
        let pending = state.pending.get_mut("write").unwrap();
        pending.phase = Phase::Sent;
        pending.wire_id = Some("1".into());
        state.wire_ids.insert("1".into(), "write".into());
    }
    let request_id = "00000000-0000-4000-8000-000000000011";
    for extra in [
        json!({"type":"accepted"}),
        json!({"type":"phase","phase":"dispatching"}),
        json!({"type":"statement","sdkRequestId":request_id}),
        json!({"type":"dmlAcknowledged","sdkRequestId":request_id,"kind":"insert"}),
        json!({"type":"phase","phase":"acknowledged","sdkRequestId":request_id}),
        json!({"type":"terminal","ok":true,"sdkRequestId":request_id,"result":{
            "kind":"insert","sdkRequestId":request_id,"rowsSeen":1,"rowsRetained":0,
            "truncated":false,"columns":[{"name":"number of rows inserted","type":"fixed"}],"values":[["1"]]
        }}),
    ] {
        let mut event = json!({"v":1,"generation":inner.generation,"id":"1"});
        event
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        if event["type"] == "terminal" {
            let mut malformed = event.clone();
            malformed["result"]["kind"] = json!("delete");
            assert!(!inner.event(malformed));
            assert!(policy.observed.lock().unwrap().is_empty());
            assert!(operation.terminal_receiver().borrow().is_none());
        }
        assert!(inner.event(event));
    }
    // The callback has completed by the time even a nonblocking caller can see
    // its terminal. Its denial revokes future work, not the original known ACK.
    assert_eq!(
        *policy.observed.lock().unwrap(),
        vec![(
            "write".into(),
            ticket,
            "1".into(),
            "dml".into(),
            Certainty::StatementSucceeded
        )]
    );
    assert!(transport.check().is_err());
    let mut operation = Arc::try_unwrap(operation).ok().unwrap();
    let report = operation.wait().await;
    assert!(report.ok);
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    assert!(report.dml_acknowledged.is_some());
    assert!(report.retirement.is_none());
    assert!(inner.state.lock().unwrap().finalizing.is_empty());
}

#[derive(Default)]
struct Policy {
    deny: AtomicBool,
    revoked: AtomicBool,
    checked: Notify,
    calls: AtomicUsize,
    entered: Notify,
    release: Notify,
    pause: AtomicBool,
    seen: Mutex<Vec<ObservedRequest>>,
}
impl TransportPolicy for Policy {
    fn check(&self) -> AppResult<()> {
        let revoked = self.revoked.load(Ordering::SeqCst);
        if self.calls.load(Ordering::SeqCst) > 0 {
            self.checked.notify_one();
        }
        if revoked {
            Err(AppError::invalid("fixture policy revoked"))
        } else {
            Ok(())
        }
    }
    fn admit<'a>(
        &'a self,
        request: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            assert!(valid_generation(request.generation));
            assert_eq!(request.payload["op"], request.operation);
            assert!(valid_transaction_impact(
                request.operation,
                &request.payload["payload"],
                request.impact
            ));
            if let Some((target, ticket)) = request.target {
                assert!(!target.is_empty());
                assert!(ticket.is_some());
                assert!(
                    request.payload["payload"]["target"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty())
                );
            }
            assert_eq!(
                serde_json::from_slice::<Value>(request.wire_payload).unwrap(),
                *request.payload
            );
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push((
                request.operation_id.into(),
                request.ticket,
                request.operation.into(),
                request.wire_payload.to_vec(),
            ));
            self.entered.notify_one();
            if self.pause.swap(false, Ordering::SeqCst) {
                self.release.notified().await;
            }
            if self.deny.load(Ordering::SeqCst) {
                Err(AppError::invalid("private fixture denial"))
            } else {
                Ok(())
            }
        })
    }
}
fn attachment(policy: Arc<Policy>, limits: TransportLimits) -> TransportAttachment {
    TransportAttachment::new(policy, limits).unwrap()
}
fn isolated(
    transport: TransportAttachment,
) -> (
    Arc<Inner>,
    mpsc::Receiver<Outbound>,
    mpsc::Receiver<Outbound>,
) {
    let (normal, normal_rx) = mpsc::channel(8);
    let (control, control_rx) = mpsc::channel(16);
    let (stop, _) = watch::channel(false);
    let (_, receiver) = watch::channel(None);
    let inner = Arc::new(Inner {
        generation: "00000000-0000-4000-8000-000000000001".into(),
        state: Mutex::new(State {
            connected: true,
            ..State::default()
        }),
        normal,
        control,
        stop,
        exited: CleanupObserver {
            receiver,
            authority: None,
        },
        sequence: AtomicU64::new(0),
        buffered: Arc::new(AtomicUsize::new(0)),
        logout_acknowledged: AtomicBool::new(false),
        transport: Some(transport),
    });
    (inner, normal_rx, control_rx)
}
fn frame(bytes: &[u8]) -> Vec<u8> {
    let mut all = (bytes.len() as u32).to_be_bytes().to_vec();
    all.extend(bytes);
    all
}
#[tokio::test]
async fn qualification_transport_counts_complete_malformed_and_partial_physical_input() {
    for (input, expected_bytes, frames, disposition) in [
        (frame(b"{}"), 6, 1, Disposition::Eof),
        (frame(b"no"), 6, 1, Disposition::Malformed),
        (vec![0, 0], 2, 0, Disposition::Truncated),
        (vec![0, 0, 0, 4, b'{'], 5, 0, Disposition::Truncated),
    ] {
        let transport = attachment(Arc::new(Policy::default()), TransportLimits::default());
        transport.claim().unwrap();
        let guard = transport.start(Stream::Stdout);
        transport.seal();
        let mut reader = input.as_slice();
        let first = transport
            .read_frame(&mut reader, Duration::from_secs(1))
            .await;
        if first.is_ok() {
            assert!(
                transport
                    .read_frame(&mut reader, Duration::from_secs(1))
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        drop(guard);
        let report = transport.wait(Duration::from_secs(1)).await.unwrap();
        assert_eq!(report.stdout_bytes, expected_bytes);
        assert_eq!(report.complete_frames, frames);
        assert_eq!(report.stdout, disposition);
    }
}
#[tokio::test]
async fn qualification_transport_caps_before_frame_allocation_and_counts_stderr_sentinel() {
    for (limits, input, expected_bytes, expected_frames) in [
        (
            TransportLimits {
                frame_bytes: 2,
                ..TransportLimits::default()
            },
            vec![0, 16, 0, 0],
            4,
            0,
        ),
        (
            TransportLimits {
                stdout_bytes: 5,
                ..TransportLimits::default()
            },
            frame(b"{}"),
            4,
            0,
        ),
        (
            TransportLimits {
                frames: 1,
                ..TransportLimits::default()
            },
            [frame(b"{}"), frame(b"{}")].concat(),
            10,
            1,
        ),
    ] {
        let transport = attachment(Arc::new(Policy::default()), limits);
        let _guard = transport.start(Stream::Stdout);
        let mut reader = input.as_slice();
        while matches!(
            transport
                .read_frame(&mut reader, Duration::from_secs(1))
                .await,
            Ok(Some(_))
        ) {}
        let report = transport.report();
        assert_eq!(report.stdout_bytes, expected_bytes);
        assert_eq!(report.complete_frames, expected_frames);
        assert_eq!(report.stdout, Disposition::Limit);
        assert!(report.failed);
    }
    let transport = attachment(
        Arc::new(Policy::default()),
        TransportLimits {
            stderr_bytes: 3,
            ..TransportLimits::default()
        },
    );
    let _guard = transport.start(Stream::Stderr);
    let mut reader = &b"private error text"[..];
    assert!(transport.drain_stderr(&mut reader).await.is_err());
    let report = transport.report();
    assert_eq!(report.stderr_bytes, 4);
    assert_eq!(report.stderr, Disposition::Limit);
}
#[tokio::test]
async fn qualification_transport_distinguishes_eof_timeout_and_task_interruption() {
    let transport = attachment(Arc::new(Policy::default()), TransportLimits::default());
    transport.claim().unwrap();
    let guard = transport.start(Stream::Stdout);
    transport.seal();
    let (mut writer, mut reader) = tokio::io::duplex(8);
    writer.write_all(&[0]).await.unwrap();
    assert!(
        transport
            .read_frame(&mut reader, Duration::from_millis(10))
            .await
            .is_err()
    );
    drop(guard);
    assert_eq!(
        transport.wait(Duration::from_secs(1)).await.unwrap().stdout,
        Disposition::Deadline
    );
    let transport = attachment(Arc::new(Policy::default()), TransportLimits::default());
    transport.claim().unwrap();
    let guard = transport.start(Stream::Stdout);
    transport.seal();
    assert!(transport.wait(Duration::from_millis(5)).await.is_none());
    drop(guard);
    assert_eq!(
        transport.wait(Duration::from_secs(1)).await.unwrap().stdout,
        Disposition::Interrupted
    );
}
#[tokio::test]
async fn qualification_transport_denial_is_before_any_pipe_write_and_latches_first_error() {
    let policy = Arc::new(Policy::default());
    policy.deny.store(true, Ordering::SeqCst);
    let transport = attachment(policy.clone(), TransportLimits::default());
    let (inner, normal, control) = isolated(transport.clone());
    let mut op = inner
        .request(
            "denied",
            "dml",
            json!({"kind":"insert","sql":"fixed"}),
            Impact::Mutating,
        )
        .unwrap();
    let (writer, mut reader) = tokio::io::duplex(1024);
    write_loop(
        inner.clone(),
        writer,
        normal,
        control,
        Duration::from_secs(1),
    )
    .await;
    let report = op.wait().await;
    assert_eq!(report.certainty, Certainty::NotSubmitted);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty());
    let counters = transport.report();
    assert_eq!(counters.started_writes, 0);
    assert_eq!(counters.outbound_bytes, 0);
    assert_eq!(policy.calls.load(Ordering::SeqCst), 1);
    transport.fail(AppError::invalid("later failure"));
    assert_eq!(
        transport.first_error().unwrap().message,
        "private fixture denial"
    );
}
#[tokio::test]
async fn qualification_transport_awaited_admission_rechecks_canceled_and_replaced_ticket() {
    let policy = Arc::new(Policy::default());
    policy.pause.store(true, Ordering::SeqCst);
    let transport = attachment(policy.clone(), TransportLimits::default());
    let (inner, normal, control) = isolated(transport.clone());
    let mut old = inner
        .request("same", "query", json!({"sql":"old"}), Impact::Read)
        .unwrap();
    let (writer, mut reader) = tokio::io::duplex(4096);
    let task = tokio::spawn(write_loop(
        inner.clone(),
        writer,
        normal,
        control,
        Duration::from_secs(1),
    ));
    policy.entered.notified().await;
    let cancel = inner.cancel(&old.token()).unwrap();
    assert!(cancel.report().unwrap().ok);
    assert_eq!(old.wait().await.certainty, Certainty::NotSubmitted);
    let replacement = inner
        .request("same", "query", json!({"sql":"replacement"}), Impact::Read)
        .unwrap();
    policy.release.notify_one();
    let wire = timeout(
        Duration::from_secs(1),
        read_frame(&mut reader, Duration::from_secs(1)),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(wire["payload"]["sql"], "replacement");
    assert_eq!(wire["id"], "1");
    assert_eq!(transport.report().admitted_writes, 2);
    assert_eq!(transport.report().started_writes, 1);
    {
        let seen = policy.seen.lock().unwrap();
        assert_ne!(seen[0].1, seen[1].1);
    }
    inner.retire(Retirement::Requested);
    task.await.unwrap();
    drop(replacement);
}
#[tokio::test]
async fn qualification_transport_awaited_control_revalidates_terminal_target() {
    let policy = Arc::new(Policy::default());
    let transport = attachment(policy.clone(), TransportLimits::default());
    let (inner, normal, control) = isolated(transport.clone());
    let target = inner
        .request("target", "query", json!({"sql":"read"}), Impact::Read)
        .unwrap();
    {
        let mut state = inner.state.lock().unwrap();
        let pending = state.pending.get_mut("target").unwrap();
        pending.phase = Phase::Sent;
        pending.wire_id = Some("existing-wire".into());
    }
    let mut cancel = inner.cancel(&target.token()).unwrap();
    policy.pause.store(true, Ordering::SeqCst);
    let (writer, mut reader) = tokio::io::duplex(1024);
    let task = tokio::spawn(write_loop(
        inner.clone(),
        writer,
        normal,
        control,
        Duration::from_secs(1),
    ));
    policy.entered.notified().await;
    let pending = inner
        .state
        .lock()
        .unwrap()
        .pending
        .remove("target")
        .unwrap();
    pending.terminal.send_replace(Some(local_report(json!({}))));
    policy.release.notify_one();
    assert!(cancel.wait_bounded(Duration::from_secs(1)).await.ok);
    inner.retire(Retirement::Requested);
    task.await.unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty());
    assert_eq!(transport.report().started_writes, 0);
}
struct PartialWriter {
    remaining: usize,
}
impl AsyncWrite for PartialWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self.remaining == 0 {
            return std::task::Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        let count = self.remaining.min(bytes.len());
        self.remaining -= count;
        std::task::Poll::Ready(Ok(count))
    }
    fn poll_flush(
        self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}
#[tokio::test]
async fn qualification_transport_partial_write_is_consumed_once_and_outcome_unknown() {
    let policy = Arc::new(Policy::default());
    let transport = attachment(policy.clone(), TransportLimits::default());
    let (inner, normal, control) = isolated(transport.clone());
    let mut op = inner
        .request(
            "partial",
            "dml",
            json!({"kind":"insert","sql":"fixed"}),
            Impact::Mutating,
        )
        .unwrap();
    write_loop(
        inner,
        PartialWriter { remaining: 6 },
        normal,
        control,
        Duration::from_secs(1),
    )
    .await;
    let report = op.wait().await;
    assert_eq!(report.phase, Phase::Sending);
    assert_eq!(report.certainty, Certainty::OutcomeUnknown);
    let counters = transport.report();
    assert_eq!(counters.admitted_writes, 1);
    assert_eq!(counters.started_writes, 1);
    assert_eq!(counters.complete_writes, 0);
    assert_eq!(counters.outbound_bytes, 6);
    assert_eq!(policy.calls.load(Ordering::SeqCst), 1);
}
fn node() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".cache/toolchains/node-v24.22.2-darwin-arm64/bin/node")
}
fn scripted_launch(script: &str, transport: TransportAttachment) -> (tempfile::TempDir, Launch) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("fixture.mjs");
    std::fs::write(&file, script).unwrap();
    let mut launch = Launch::new(
        node(),
        Some(file),
        root.path().to_owned(),
        tempfile::tempdir_in(root.path()).unwrap(),
    )
    .unwrap();
    launch.attach_transport(transport).unwrap();
    (root, launch)
}
const HELLO_SCRIPT: &str = r#"const hello={v:1,type:'hello',protocol:'joinery-snowflake',sdkVersion:'3.4.1',limits:{frameBytes:1048576,batchRows:128,batchBytes:262144,queued:8,credits:32}};function emit(v){let b=Buffer.from(JSON.stringify(v));let p=Buffer.alloc(4);p.writeUInt32BE(b.length);process.stdout.write(Buffer.concat([p,b]));}emit(hello);"#;
#[tokio::test]
async fn qualification_transport_real_handshake_stderr_overflow_and_settlement_are_distinct_from_reap()
 {
    let policy = Arc::new(Policy::default());
    let transport = attachment(
        policy,
        TransportLimits {
            stderr_bytes: 8,
            ..TransportLimits::default()
        },
    );
    let script = format!(
        "{HELLO_SCRIPT}setTimeout(()=>process.stderr.write('123456789private'),30);setInterval(()=>{{}},1000);"
    );
    let (_root, launch) = scripted_launch(&script, transport.clone());
    let cleanup = launch.cleanup_observer();
    let supervisor = Supervisor::spawn(
        launch,
        "00000000-0000-4000-8000-000000000001",
        Bounds::default(),
    )
    .await
    .unwrap();
    let expected=frame(serde_json::to_string(&json!({"v":1,"type":"hello","protocol":"joinery-snowflake","sdkVersion":"3.4.1","limits":{"frameBytes":MAX_FRAME,"batchRows":128,"batchBytes":MAX_BATCH,"queued":8,"credits":32}})).unwrap().as_bytes()).len() as u64;
    let report = transport.wait(Duration::from_secs(3)).await.unwrap();
    assert!(supervisor.is_closed());
    assert_eq!(report.stdout_bytes, expected);
    assert_eq!(report.complete_frames, 1);
    assert_eq!(report.stderr_bytes, 9);
    assert_eq!(report.stderr, Disposition::Limit);
    assert!(report.failed);
    assert!(cleanup.wait(Duration::from_secs(3)).await.unwrap().reaped);
}
#[tokio::test]
async fn qualification_transport_cancelled_handshake_and_failed_spawn_settle_without_reuse() {
    let transport = attachment(Arc::new(Policy::default()), TransportLimits::default());
    let (_root, launch) = scripted_launch("setInterval(()=>{},1000)", transport.clone());
    let cleanup = launch.cleanup_observer();
    let task = tokio::spawn(Supervisor::spawn(
        launch,
        "00000000-0000-4000-8000-000000000001",
        Bounds::default(),
    ));
    tokio::time::sleep(Duration::from_millis(30)).await;
    task.abort();
    let _ = task.await;
    let report = transport.wait(Duration::from_secs(3)).await.unwrap();
    assert!(report.settled);
    assert_eq!(report.stdout, Disposition::Interrupted);
    assert!(cleanup.wait(Duration::from_secs(3)).await.unwrap().reaped);
    let root = tempfile::tempdir().unwrap();
    let mut launch = Launch::new(
        node(),
        None,
        root.path().to_owned(),
        tempfile::tempdir_in(root.path()).unwrap(),
    )
    .unwrap();
    assert!(launch.attach_transport(transport).is_err());
    let transport = attachment(Arc::new(Policy::default()), TransportLimits::default());
    launch.executable = root.path().join("missing");
    launch.attach_transport(transport.clone()).unwrap();
    assert!(
        Supervisor::spawn(
            launch,
            "00000000-0000-4000-8000-000000000001",
            Bounds::default()
        )
        .await
        .is_err()
    );
    assert!(
        transport
            .wait(Duration::from_secs(1))
            .await
            .unwrap()
            .settled
    );
}
#[tokio::test]
async fn qualification_transport_known_dml_ack_survives_later_inbound_limit() {
    let transport = attachment(
        Arc::new(Policy::default()),
        TransportLimits {
            frames: 4,
            ..TransportLimits::default()
        },
    );
    let (inner, _normal, _control) = isolated(transport.clone());
    let mut op = inner
        .request("write", "dml", json!({"kind":"insert"}), Impact::Mutating)
        .unwrap();
    {
        let mut state = inner.state.lock().unwrap();
        let p = state.pending.get_mut("write").unwrap();
        p.phase = Phase::Sent;
        p.wire_id = Some("1".into());
        state.wire_ids.insert("1".into(), "write".into());
    }
    let id = "00000000-0000-4000-8000-000000000011";
    let events = [
        json!({"type":"accepted"}),
        json!({"type":"phase","phase":"dispatching"}),
        json!({"type":"statement","sdkRequestId":id}),
        json!({"type":"dmlAcknowledged","sdkRequestId":id,"kind":"insert"}),
        json!({"type":"unexpected"}),
    ];
    let mut all = Vec::new();
    for extra in events {
        let mut e = json!({"v":1,"generation":inner.generation,"id":"1"});
        e.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        all.extend(frame(&serde_json::to_vec(&e).unwrap()));
    }
    let _guard = transport.start(Stream::Stdout);
    let mut reader = all.as_slice();
    loop {
        match observed_frame(&inner, &mut reader, Duration::from_secs(1)).await {
            Ok(Some(e)) => assert!(inner.event(e)),
            Err(_) => {
                inner.retire(Retirement::Protocol);
                break;
            }
            Ok(None) => panic!("expected cap"),
        }
    }
    let report = op.wait().await;
    assert!(report.dml_acknowledged.is_some());
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    assert_eq!(transport.report().complete_frames, 4);
    assert!(transport.report().failed);
}

struct WriteGate {
    opened: AtomicBool,
    entered: Notify,
    waker: Mutex<Option<std::task::Waker>>,
    bytes: AtomicUsize,
}
struct GatedWriter {
    gate: Arc<WriteGate>,
    before_pause: usize,
}
impl AsyncWrite for GatedWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self.before_pause == 0 && !self.gate.opened.load(Ordering::SeqCst) {
            *self.gate.waker.lock().unwrap() = Some(cx.waker().clone());
            self.gate.entered.notify_one();
            return std::task::Poll::Pending;
        }
        let count = if self.before_pause > 0 {
            self.before_pause.min(bytes.len())
        } else {
            bytes.len()
        };
        self.before_pause = self.before_pause.saturating_sub(count);
        self.gate.bytes.fetch_add(count, Ordering::SeqCst);
        std::task::Poll::Ready(Ok(count))
    }
    fn poll_flush(
        self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}
#[tokio::test]
async fn qualification_transport_final_policy_recheck_after_pending_write_preserves_zero_or_partial_certainty()
 {
    for prefix_bytes in [0, 2] {
        let policy = Arc::new(Policy::default());
        let transport = attachment(policy.clone(), TransportLimits::default());
        let (inner, normal, control) = isolated(transport.clone());
        let mut op = inner
            .request("blocked", "dml", json!({"kind":"insert"}), Impact::Mutating)
            .unwrap();
        let gate = Arc::new(WriteGate {
            opened: AtomicBool::new(false),
            entered: Notify::new(),
            waker: Mutex::new(None),
            bytes: AtomicUsize::new(0),
        });
        let mut task = tokio::spawn(write_loop(
            inner.clone(),
            GatedWriter {
                gate: gate.clone(),
                before_pause: prefix_bytes,
            },
            normal,
            control,
            Duration::from_secs(1),
        ));
        timeout(Duration::from_secs(1), gate.entered.notified())
            .await
            .unwrap();
        policy.revoked.store(true, Ordering::SeqCst);
        gate.opened.store(true, Ordering::SeqCst);
        if let Some(waker) = gate.waker.lock().unwrap().take() {
            waker.wake();
        }
        if timeout(Duration::from_millis(200), &mut task)
            .await
            .is_err()
        {
            inner.retire(Retirement::Requested);
            task.await.unwrap();
        }
        let report = op.wait().await;
        assert_eq!(
            gate.bytes.load(Ordering::SeqCst),
            prefix_bytes,
            "revocation must prevent all further bytes"
        );
        assert_eq!(
            report.certainty,
            if prefix_bytes == 0 {
                Certainty::NotSubmitted
            } else {
                Certainty::OutcomeUnknown
            }
        );
        assert_eq!(transport.report().outbound_bytes, prefix_bytes as u64);
        assert_eq!(policy.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_transport_final_policy_recheck_after_process_mutex_contention() {
    let policy = Arc::new(Policy::default());
    policy.pause.store(true, Ordering::SeqCst);
    let transport = attachment(policy.clone(), TransportLimits::default());
    let (inner, normal, control) = isolated(transport.clone());
    let mut op = inner
        .request(
            "contended",
            "dml",
            json!({"kind":"insert"}),
            Impact::Mutating,
        )
        .unwrap();
    let (writer, mut reader) = tokio::io::duplex(4096);
    let mut task = tokio::spawn(write_loop(
        inner.clone(),
        writer,
        normal,
        control,
        Duration::from_secs(1),
    ));
    policy.entered.notified().await;
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let (unlock_tx, unlock_rx) = std::sync::mpsc::channel();
    let owner = inner.clone();
    let holder = std::thread::spawn(move || {
        let _lock = owner.state.lock().unwrap();
        locked_tx.send(()).unwrap();
        unlock_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    });
    locked_rx.await.unwrap();
    policy.release.notify_one();
    policy.checked.notified().await;
    policy.revoked.store(true, Ordering::SeqCst);
    unlock_tx.send(()).unwrap();
    holder.join().unwrap();
    if timeout(Duration::from_millis(200), &mut task)
        .await
        .is_err()
    {
        inner.retire(Retirement::Requested);
        task.await.unwrap();
    }
    assert_eq!(op.wait().await.certainty, Certainty::NotSubmitted);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty());
    assert_eq!(transport.report().outbound_bytes, 0);
}
#[tokio::test]
async fn qualification_transport_unstarted_disposal_consumes_capability_and_live_guard_blocks_settlement()
 {
    let transport = attachment(Arc::new(Policy::default()), TransportLimits::default());
    transport.settle_unstarted().unwrap();
    let report = transport.wait(Duration::from_secs(1)).await.unwrap();
    assert_eq!(report.stdout, Disposition::NotStarted);
    assert!(transport.claim().is_err());
    let transport = attachment(Arc::new(Policy::default()), TransportLimits::default());
    transport.claim().unwrap();
    let guard = transport.start(Stream::Stdout);
    transport.seal();
    transport.stream_failure(Stream::Stdout, Disposition::Limit);
    assert!(!transport.report().settled);
    assert!(transport.wait(Duration::from_millis(5)).await.is_none());
    drop(guard);
    assert!(
        transport
            .wait(Duration::from_secs(1))
            .await
            .unwrap()
            .settled
    );
}
