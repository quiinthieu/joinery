//! Synthetic pipes exercise admission races and actual receiving/writing paths.
use super::*;
use crate::domain::{AppError, AppResult};
use crate::drivers::snowflake_qualification_transport::{
    Disposition, LocalTerminal, LocalTerminalOrigin, PhysicalRequest, PhysicalTerminal, Stream,
    TransportLimits, TransportPolicy,
};
use std::{future::Future, pin::Pin, sync::atomic::AtomicUsize};
use tokio::sync::Notify;

type ObservedRequest = (String, u64, String, Vec<u8>);
type ObservedTerminal = (String, u64, String, String, Certainty);

#[derive(Default)]
struct LocalReportPolicy {
    reports: Mutex<Vec<Report>>,
    local: Mutex<Vec<ObservedLocal>>,
}
#[derive(Debug)]
struct ObservedLocal {
    id: String,
    ticket: u64,
    wire_id: Option<String>,
    candidate: Option<String>,
    target: Option<(String, Option<u64>)>,
    payload: Value,
    origin: LocalTerminalOrigin,
}
fn local_metadata(terminal: &LocalTerminal<'_>) -> ObservedLocal {
    assert!(valid_generation(terminal.generation));
    assert!(valid_transaction_impact(
        terminal.operation,
        terminal.payload,
        terminal.impact
    ));
    ObservedLocal {
        id: terminal.operation_id.into(),
        ticket: terminal.ticket,
        wire_id: terminal.wire_id.map(str::to_owned),
        candidate: terminal.candidate_wire_id.map(str::to_owned),
        target: terminal.target.map(|(id, ticket)| (id.into(), ticket)),
        payload: terminal.payload.clone(),
        origin: terminal.origin,
    }
}
impl TransportPolicy for LocalReportPolicy {
    fn admit<'a>(
        &'a self,
        _: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
    fn terminal(&self, terminal: PhysicalTerminal<'_>) -> AppResult<()> {
        self.reports.lock().unwrap().push(terminal.report.clone());
        Ok(())
    }
    fn local_terminal(&self, terminal: LocalTerminal<'_>) -> AppResult<()> {
        self.local.lock().unwrap().push(local_metadata(&terminal));
        self.reports.lock().unwrap().push(terminal.report.clone());
        Ok(())
    }
}

#[tokio::test]
async fn qualification_local_original_queued_cancel_is_observed_once_after_prior_failure() {
    let policy = Arc::new(LocalReportPolicy::default());
    let transport = TransportAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let (inner, _normal, _control) = isolated(transport.clone());
    let mut operation = inner
        .request(
            "queued",
            "query",
            json!({"sql":"never submitted"}),
            Impact::Read,
        )
        .unwrap();
    transport.fail(AppError::invalid("prior qualification denial"));
    let acknowledgement = inner.cancel(&operation.token()).unwrap();
    assert!(acknowledgement.report().unwrap().ok);
    let original = operation.wait().await;
    assert_eq!(original.certainty, Certainty::NotSubmitted);
    assert_eq!(original.error_code.as_deref(), Some("cancelled"));
    inner.retire(Retirement::Requested);
    let observed = policy.reports.lock().unwrap();
    assert_eq!(
        observed.len(),
        1,
        "the original local report must reach its owner"
    );
    assert_eq!(format!("{:?}", observed[0]), format!("{original:?}"));
    let local = policy.local.lock().unwrap();
    assert_eq!(local[0].id, "queued");
    assert_eq!(local[0].ticket, operation.token().ticket);
    assert_eq!(local[0].origin, LocalTerminalOrigin::QueuedCancellation);
    assert!(local[0].wire_id.is_none() && local[0].candidate.is_none());
    assert!(local[0].target.is_none());
    assert_eq!(local[0].payload, json!({"sql":"never submitted"}));
}

#[tokio::test]
async fn qualification_local_original_retirement_retains_all_typed_acks_once() {
    for (operation_name, payload, acknowledgement, impact) in [
        (
            "dml",
            json!({"kind":"insert"}),
            json!({"type":"dmlAcknowledged","kind":"insert"}),
            Impact::Mutating,
        ),
        (
            "ddl",
            json!({"kind":"createTable"}),
            json!({"type":"ddlAcknowledged","kind":"createTable"}),
            Impact::Mutating,
        ),
        (
            "transaction",
            json!({"action":"begin"}),
            json!({"type":"boundaryAcknowledged","action":"begin"}),
            Impact::Begin,
        ),
    ] {
        for reason in [
            Retirement::Eof,
            Retirement::Protocol,
            Retirement::Write,
            Retirement::Requested,
            Retirement::Deadline,
            Retirement::Dropped,
        ] {
            let policy = Arc::new(LocalReportPolicy::default());
            let transport =
                TransportAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
            let (inner, _normal, _control) = isolated(transport);
            let mut operation = inner
                .request("write", operation_name, payload.clone(), impact)
                .unwrap();
            {
                let mut state = inner.state.lock().unwrap();
                let pending = state.pending.get_mut("write").unwrap();
                pending.phase = Phase::Sent;
                pending.wire_id = Some("1".into());
                state.wire_ids.insert("1".into(), "write".into());
            }
            let sdk_id = "00000000-0000-4000-8000-000000000011";
            let query_id = "00000000-0000-4000-8000-000000000012";
            for extra in [
                json!({"type":"accepted"}),
                json!({"type":"phase","phase":"dispatching"}),
                json!({"type":"statement","sdkRequestId":sdk_id}),
                acknowledgement.clone(),
                json!({"type":"statement","sdkRequestId":sdk_id,"queryId":query_id}),
            ] {
                let mut event =
                    json!({"v":1,"generation":inner.generation,"id":"1","sdkRequestId":sdk_id});
                event
                    .as_object_mut()
                    .unwrap()
                    .extend(extra.as_object().unwrap().clone());
                assert!(inner.event(event));
            }
            inner.retire(reason);
            inner.retire(reason);
            let original = operation.wait().await;
            assert_eq!(original.certainty, Certainty::StatementSucceeded);
            assert_eq!(original.retirement, Some(reason));
            assert_eq!(original.statements[0].query_id.as_deref(), Some(query_id));
            let observed = policy.reports.lock().unwrap();
            assert_eq!(
                observed.len(),
                1,
                "retirement must retain one original owner report"
            );
            assert_eq!(format!("{:?}", observed[0]), format!("{original:?}"));
            let local = policy.local.lock().unwrap();
            assert_eq!(local[0].wire_id.as_deref(), Some("1"));
            assert!(local[0].candidate.is_none());
            assert_eq!(local[0].origin, LocalTerminalOrigin::Retirement);
        }
    }
}

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
    local: Mutex<Vec<ObservedLocal>>,
}
impl TransportPolicy for Policy {
    fn local_terminal(&self, terminal: LocalTerminal<'_>) -> AppResult<()> {
        self.local.lock().unwrap().push(local_metadata(&terminal));
        Ok(())
    }
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
        finalizations: watch::channel(0).0,
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
    for (input, expected_bytes, payload_bytes, frames, disposition) in [
        (frame(b"{}"), 6, 2, 1, Disposition::Eof),
        (frame(b"no"), 6, 2, 1, Disposition::Malformed),
        (vec![0, 0], 2, 0, 0, Disposition::Truncated),
        (vec![0, 0, 0, 4, b'{'], 5, 1, 0, Disposition::Truncated),
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
        if first.is_err() {
            let before = transport.report();
            let replay = frame(b"{}");
            let mut replay_reader = replay.as_slice();
            assert!(
                transport
                    .read_frame(&mut replay_reader, Duration::from_secs(1))
                    .await
                    .is_err()
            );
            assert_eq!(
                replay_reader.len(),
                6,
                "failed framing cannot resume or replay"
            );
            assert_eq!(transport.report().stdout_bytes, before.stdout_bytes);
        }
        drop(guard);
        let report = transport.wait(Duration::from_secs(1)).await.unwrap();
        assert_eq!(report.stdout_bytes, expected_bytes);
        assert_eq!(report.payload_bytes, payload_bytes);
        assert_eq!(report.complete_frames, frames);
        assert_eq!(report.stdout, disposition);
    }
}
#[test]
fn qualification_transport_payload_limit_constructor_bounds() {
    let defaults = TransportLimits::default();
    assert_eq!(defaults.payload_bytes, 32 * 1024 * 1024);
    assert_eq!(defaults.stdout_bytes, 64 * 1024 * 1024);
    for payload_bytes in [0, defaults.payload_bytes + 1, u64::MAX] {
        assert!(
            TransportAttachment::new(
                Arc::new(Policy::default()),
                TransportLimits {
                    payload_bytes,
                    ..defaults
                }
            )
            .is_err()
        );
    }
    for payload_bytes in [1, defaults.payload_bytes] {
        assert!(
            TransportAttachment::new(
                Arc::new(Policy::default()),
                TransportLimits {
                    payload_bytes,
                    ..defaults
                }
            )
            .is_ok()
        );
    }
}

#[tokio::test]
async fn qualification_transport_payload_exact_boundary_and_header_only_overflow() {
    for overflow in [false, true] {
        let transport = attachment(
            Arc::new(Policy::default()),
            TransportLimits {
                payload_bytes: 5,
                ..TransportLimits::default()
            },
        );
        transport.claim().unwrap();
        let guard = transport.start(Stream::Stdout);
        transport.seal();
        let mut input = [frame(b"{}"), frame(b"[0]")].concat();
        if overflow {
            // No body exists: rejection must follow the header, not a body read.
            input.extend(1_u32.to_be_bytes());
        }
        let mut reader = input.as_slice();
        assert_eq!(
            transport
                .read_frame(&mut reader, Duration::from_secs(1))
                .await
                .unwrap(),
            Some(json!({}))
        );
        assert_eq!(
            transport
                .read_frame(&mut reader, Duration::from_secs(1))
                .await
                .unwrap(),
            Some(json!([0]))
        );
        let result = transport
            .read_frame(&mut reader, Duration::from_secs(1))
            .await;
        if overflow {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_none());
        }
        assert!(reader.is_empty());
        drop(guard);
        let report = transport.wait(Duration::from_secs(1)).await.unwrap();
        assert_eq!(report.payload_bytes, 5);
        assert_eq!(report.stdout_bytes, if overflow { 17 } else { 13 });
        assert_eq!(report.complete_frames, 2);
        assert_eq!(
            report.stdout,
            if overflow {
                Disposition::Limit
            } else {
                Disposition::Eof
            }
        );
        assert_eq!(report.failed, overflow);
    }
    let transport = attachment(
        Arc::new(Policy::default()),
        TransportLimits {
            payload_bytes: 1,
            ..TransportLimits::default()
        },
    );
    let _guard = transport.start(Stream::Stdout);
    let input = frame(b"{}");
    let mut reader = input.as_slice();
    assert!(
        transport
            .read_frame(&mut reader, Duration::from_secs(1))
            .await
            .is_err()
    );
    assert_eq!(reader, b"{}", "over-budget body is not read");
    assert_eq!(transport.report().payload_bytes, 0);
    assert_eq!(transport.report().stdout_bytes, 4);
}

#[tokio::test]
async fn qualification_transport_cancelled_body_keeps_reservation_and_actual_bytes() {
    let transport = attachment(
        Arc::new(Policy::default()),
        TransportLimits {
            payload_bytes: 4,
            ..TransportLimits::default()
        },
    );
    transport.claim().unwrap();
    let guard = transport.start(Stream::Stdout);
    transport.seal();
    let (mut writer, mut reader) = tokio::io::duplex(16);
    writer.write_all(&[0, 0, 0, 4, b'[']).await.unwrap();
    {
        let mut read = Box::pin(transport.read_frame(&mut reader, Duration::from_secs(1)));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(read.as_mut().poll(&mut context).is_pending());
        assert_eq!(transport.report().payload_bytes, 1);
    }
    writer.write_all(b"0] ").await.unwrap();
    assert!(
        transport
            .read_frame(&mut reader, Duration::from_secs(1))
            .await
            .is_err()
    );
    assert_eq!(transport.report().payload_bytes, 1);
    assert_eq!(transport.report().stdout_bytes, 5);
    let mut unread = [0; 3];
    reader.read_exact(&mut unread).await.unwrap();
    assert_eq!(&unread, b"0] ");
    drop(guard);
    let report = transport.wait(Duration::from_secs(1)).await.unwrap();
    assert_eq!(report.stdout, Disposition::Interrupted);
    assert_eq!(report.complete_frames, 0);
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
    assert_eq!(
        wire["id"], "2",
        "the canceled operation's admitted candidate is consumed"
    );
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
    {
        let local = policy.local.lock().unwrap();
        let original = &local[0];
        assert_eq!(original.id, cancel.id);
        assert_eq!(original.ticket, cancel.token().ticket);
        assert_eq!(
            original.origin,
            LocalTerminalOrigin::InactiveControl { admitted: true }
        );
        assert!(original.wire_id.is_none());
        assert_eq!(original.candidate.as_deref(), Some("1"));
        assert_eq!(
            original.target,
            Some(("target".into(), Some(target.token().ticket)))
        );
        assert_eq!(original.payload, json!({"target":"target"}));
    }
    assert_eq!(inner.state.lock().unwrap().next_wire_id, 1);
    assert_eq!(transport.report().started_writes, 0);
    let next = inner
        .request("next", "query", json!({"sql":"next"}), Impact::Read)
        .unwrap();
    let wire = timeout(
        Duration::from_secs(1),
        read_frame(&mut reader, Duration::from_secs(1)),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(
        wire["id"], "2",
        "a consumed intent candidate cannot be reused"
    );
    inner.retire(Retirement::Requested);
    task.await.unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty());
    assert_eq!(transport.report().started_writes, 1);
    drop(next);
}

#[tokio::test]
async fn qualification_local_original_inactive_control_before_admission_has_no_candidate() {
    let policy = Arc::new(Policy::default());
    let transport = attachment(policy.clone(), TransportLimits::default());
    let (inner, normal, control) = isolated(transport.clone());
    let target = inner
        .request("target", "query", json!({}), Impact::Read)
        .unwrap();
    {
        let mut state = inner.state.lock().unwrap();
        let pending = state.pending.get_mut("target").unwrap();
        pending.phase = Phase::Sent;
        pending.wire_id = Some("existing".into());
    }
    let mut discard = inner.discard(&target.token()).unwrap();
    let pending = inner
        .state
        .lock()
        .unwrap()
        .pending
        .remove("target")
        .unwrap();
    pending.terminal.send_replace(Some(local_report(json!({}))));
    let (writer, mut reader) = tokio::io::duplex(1024);
    let task = tokio::spawn(write_loop(
        inner.clone(),
        writer,
        normal,
        control,
        Duration::from_secs(1),
    ));
    assert!(discard.wait_bounded(Duration::from_secs(1)).await.ok);
    {
        let local = policy.local.lock().unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(
            local[0].origin,
            LocalTerminalOrigin::InactiveControl { admitted: false }
        );
        assert!(local[0].wire_id.is_none() && local[0].candidate.is_none());
        assert_eq!(
            local[0].target,
            Some(("target".into(), Some(target.token().ticket)))
        );
        assert_eq!(
            local[0].payload,
            json!({"target":"target","batches":0,"discard":true})
        );
    }
    assert_eq!(transport.report().admitted_writes, 0);
    assert_eq!(inner.state.lock().unwrap().next_wire_id, 0);
    inner.retire(Retirement::Requested);
    task.await.unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn qualification_local_original_close_drains_queued_work_before_shutdown() {
    let policy = Arc::new(LocalReportPolicy::default());
    let transport = TransportAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let (inner, _normal, _control) = isolated(transport);
    inner.state.lock().unwrap().connect_sent = true;
    let mut query = inner
        .request("queued", "query", json!({}), Impact::Read)
        .unwrap();
    let process = Supervisor {
        inner,
        pid: 0,
        bounds: Bounds {
            exit: Duration::from_millis(10),
            ..Bounds::default()
        },
    };
    process.close().await;
    let report = query.wait().await;
    assert_eq!(report.certainty, Certainty::NotSubmitted);
    assert_eq!(report.error_code.as_deref(), Some("connectionClosed"));
    let local = policy.local.lock().unwrap();
    let matching = local
        .iter()
        .filter(|row| row.id == "queued")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].origin, LocalTerminalOrigin::QueuedClose);
    assert!(matching[0].wire_id.is_none() && matching[0].candidate.is_none());
}

struct BlockingPublicationPolicy {
    inner: Mutex<Option<std::sync::Weak<Inner>>>,
    entered: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    block_helper: bool,
    local_calls: AtomicUsize,
}
impl TransportPolicy for BlockingPublicationPolicy {
    fn admit<'a>(
        &'a self,
        _: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
    fn terminal(&self, _: PhysicalTerminal<'_>) -> AppResult<()> {
        if self.block_helper {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        }
        Ok(())
    }
    fn local_terminal(&self, terminal: LocalTerminal<'_>) -> AppResult<()> {
        let inner = self
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        {
            let state = inner
                .state
                .try_lock()
                .expect("local observer outside process State");
            assert!(
                state
                    .finalizing
                    .contains_key(&(terminal.operation_id.into(), terminal.ticket))
            );
            assert!(
                state
                    .pending
                    .get(terminal.operation_id)
                    .is_none_or(|p| p.ticket != terminal.ticket)
            );
        }
        self.local_calls.fetch_add(1, Ordering::SeqCst);
        if !self.block_helper {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            return Err(AppError::invalid(
                "local publication denied after original capture",
            ));
        }
        Ok(())
    }
}

#[tokio::test]
async fn qualification_local_original_inflight_callback_blocks_settlement_and_preserves_ack() {
    let (entered, entered_rx) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let policy = Arc::new(BlockingPublicationPolicy {
        inner: Mutex::new(None),
        entered,
        release: Mutex::new(release_rx),
        block_helper: false,
        local_calls: AtomicUsize::new(0),
    });
    let transport = TransportAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let guard = transport.start(Stream::Stdout);
    transport.seal();
    let (inner, _normal, _control) = isolated(transport.clone());
    *policy.inner.lock().unwrap() = Some(Arc::downgrade(&inner));
    let mut operation = inner
        .request("write", "dml", json!({"kind":"insert"}), Impact::Mutating)
        .unwrap();
    {
        let mut state = inner.state.lock().unwrap();
        let pending = state.pending.get_mut("write").unwrap();
        pending.phase = Phase::Dispatching;
        pending.wire_id = Some("1".into());
        let statement = StatementIdentity {
            sdk_request_id: "original-sdk-id".into(),
            query_id: Some("original-query-id".into()),
        };
        pending.statements.push(statement.clone());
        pending.dml_acknowledged = Some(DmlAcknowledgement {
            kind: SnowflakeDmlKind::Insert,
            statement,
        });
    }
    let owner = inner.clone();
    let thread = std::thread::spawn(move || owner.retire(Retirement::Eof));
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    drop(guard);
    assert!(
        !transport.report().settled,
        "local callback is not a pipe task but still owns settlement"
    );
    assert!(operation.report().is_none());
    assert_eq!(
        operation
            .dml_acknowledged()
            .unwrap()
            .statement
            .query_id
            .as_deref(),
        Some("original-query-id")
    );
    assert_eq!(operation.phase(), Phase::Dispatching);
    inner.retire(Retirement::Requested);
    release.send(()).unwrap();
    thread.join().unwrap();
    let original = operation.wait().await;
    assert_eq!(original.certainty, Certainty::StatementSucceeded);
    assert_eq!(original.retirement, Some(Retirement::Eof));
    assert!(original.dml_acknowledged.is_some());
    assert_eq!(policy.local_calls.load(Ordering::SeqCst), 1);
    assert!(
        transport
            .wait(Duration::from_secs(1))
            .await
            .unwrap()
            .settled
    );
    assert!(transport.first_error().is_some());
    assert!(inner.state.lock().unwrap().finalizing.is_empty());
}

#[test]
fn qualification_local_original_dropped_mutation_is_retained_without_a_caller_receiver() {
    let policy = Arc::new(LocalReportPolicy::default());
    let transport = TransportAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let (inner, _normal, _control) = isolated(transport);
    let operation = inner
        .request("dropped", "dml", json!({"kind":"insert"}), Impact::Mutating)
        .unwrap();
    {
        let mut state = inner.state.lock().unwrap();
        let pending = state.pending.get_mut("dropped").unwrap();
        pending.phase = Phase::Dispatching;
        pending.wire_id = Some("1".into());
        let statement = StatementIdentity {
            sdk_request_id: "original-sdk-id".into(),
            query_id: Some("original-query-id".into()),
        };
        pending.statements.push(statement.clone());
        pending.dml_acknowledged = Some(DmlAcknowledgement {
            kind: SnowflakeDmlKind::Insert,
            statement,
        });
    }
    drop(operation);
    let reports = policy.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].retirement, Some(Retirement::Dropped));
    assert_eq!(reports[0].certainty, Certainty::StatementSucceeded);
    assert_eq!(
        reports[0].dml_acknowledged.as_ref().unwrap().statement,
        reports[0].statements[0]
    );
    assert!(inner.state.lock().unwrap().finalizing.is_empty());
}

#[tokio::test]
async fn qualification_local_original_inactive_waits_for_exact_helper_publication() {
    use std::task::{Context, Poll, Waker};
    let (entered, entered_rx) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let policy = Arc::new(BlockingPublicationPolicy {
        inner: Mutex::new(None),
        entered,
        release: Mutex::new(release_rx),
        block_helper: true,
        local_calls: AtomicUsize::new(0),
    });
    let transport = TransportAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    transport.seal();
    let (inner, normal, control) = isolated(transport.clone());
    *policy.inner.lock().unwrap() = Some(Arc::downgrade(&inner));
    let target = inner
        .request("target", "query", json!({}), Impact::Read)
        .unwrap();
    {
        let mut state = inner.state.lock().unwrap();
        let pending = state.pending.get_mut("target").unwrap();
        pending.phase = Phase::Acknowledged;
        pending.wire_id = Some("1".into());
        state.wire_ids.insert("1".into(), "target".into());
    }
    let mut discard = inner.discard(&target.token()).unwrap();
    let owner = inner.clone();
    let event = json!({"v":1,"generation":inner.generation,"id":"1","type":"terminal","ok":true,"result":{}});
    let thread = std::thread::spawn(move || assert!(owner.event(event)));
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let (writer, mut reader) = tokio::io::duplex(1024);
    let write = write_loop(
        inner.clone(),
        writer,
        normal,
        control,
        Duration::from_secs(1),
    );
    tokio::pin!(write);
    assert!(matches!(
        write.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    assert_eq!(
        policy.local_calls.load(Ordering::SeqCst),
        0,
        "local control cannot precede target helper observation"
    );
    assert!(discard.report().is_none());
    assert!(!transport.report().settled);
    release.send(()).unwrap();
    thread.join().unwrap();
    assert!(matches!(
        write.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    assert!(discard.wait().await.ok);
    assert_eq!(policy.local_calls.load(Ordering::SeqCst), 1);
    inner.retire(Retirement::Requested);
    write.await;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty());
    assert!(transport.report().settled);
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
async fn qualification_transport_real_child_payload_cap_preserves_prior_ack_and_settles() {
    let generation = "00000000-0000-4000-8000-000000000001";
    let request_id = "00000000-0000-4000-8000-000000000011";
    let hello = json!({"v":1,"type":"hello","protocol":"joinery-snowflake","sdkVersion":"3.4.1","limits":{"frameBytes":MAX_FRAME,"batchRows":128,"batchBytes":MAX_BATCH,"queued":8,"credits":32}});
    let connect_events = [
        json!({"type":"accepted"}),
        json!({"type":"terminal","ok":true,"result":{}}),
    ];
    let dml_events = [
        json!({"type":"accepted"}),
        json!({"type":"phase","phase":"dispatching"}),
        json!({"type":"statement","sdkRequestId":request_id}),
        json!({"type":"dmlAcknowledged","sdkRequestId":request_id,"kind":"insert"}),
    ];
    let mut payload_bytes = serde_json::to_vec(&hello).unwrap().len() as u64;
    for (wire_id, events) in [
        ("1", connect_events.as_slice()),
        ("2", dml_events.as_slice()),
    ] {
        for extra in events {
            let mut event = json!({"v":1,"generation":generation,"id":wire_id});
            event
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            payload_bytes += serde_json::to_vec(&event).unwrap().len() as u64;
        }
    }
    let policy = Arc::new(LocalReportPolicy::default());
    let transport = TransportAttachment::new(
        policy.clone(),
        TransportLimits {
            payload_bytes,
            ..TransportLimits::default()
        },
    )
    .unwrap();
    // This is an actual child and production Supervisor reader, with scripted wire
    // events rather than the account SDK. The final header has deliberately no body.
    let script = format!(
        r#"{HELLO_SCRIPT}
const connectEvents={connect_events};const dmlEvents={dml_events};
let input=Buffer.alloc(0);process.stdin.on('data',chunk=>{{
 input=Buffer.concat([input,chunk]);
 while(input.length>=4&&input.length>=4+input.readUInt32BE(0)){{
  const length=input.readUInt32BE(0);const request=JSON.parse(input.subarray(4,4+length));input=input.subarray(4+length);
  const events=request.op==='connect'?connectEvents:request.op==='dml'?dmlEvents:null;
  if(!events)throw Error('unexpected request');
  for(const event of events)emit({{v:1,generation:request.generation,id:request.id,...event}});
  if(request.op==='dml'){{const prefix=Buffer.alloc(4);prefix.writeUInt32BE(1);process.stdout.write(prefix);}}
 }}
}});setInterval(()=>{{}},1000);
"#,
        connect_events = serde_json::to_string(&connect_events).unwrap(),
        dml_events = serde_json::to_string(&dml_events).unwrap()
    );
    let (_root, launch) = scripted_launch(&script, transport.clone());
    let cleanup = launch.cleanup_observer();
    let supervisor = Supervisor::spawn(launch, generation, Bounds::default())
        .await
        .unwrap();
    let mut connect = supervisor
        .request("connect", "connect", json!({}), Impact::Control)
        .unwrap();
    let connected = connect.wait_bounded(Duration::from_secs(3)).await;
    assert!(
        connected.ok,
        "connect={connected:?}, transport={:?}",
        transport.report()
    );
    let mut operation = supervisor
        .request("write", "dml", json!({"kind":"insert"}), Impact::Mutating)
        .unwrap();
    let original = operation.wait_bounded(Duration::from_secs(3)).await;
    assert_eq!(original.certainty, Certainty::StatementSucceeded);
    assert_eq!(original.retirement, Some(Retirement::Protocol));
    assert_eq!(
        original
            .dml_acknowledged
            .as_ref()
            .unwrap()
            .statement
            .sdk_request_id,
        request_id
    );
    let report = transport.wait(Duration::from_secs(3)).await.unwrap();
    assert_eq!(report.payload_bytes, payload_bytes);
    assert_eq!(report.stdout_bytes, payload_bytes + 8 * 4);
    assert_eq!(report.complete_frames, 7);
    assert_eq!(report.stdout, Disposition::Limit);
    assert!(report.failed && report.settled);
    assert!(supervisor.is_closed());
    assert!(cleanup.wait(Duration::from_secs(3)).await.unwrap().reaped);
    let reports = policy.reports.lock().unwrap();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[1].certainty, original.certainty);
    assert_eq!(reports[1].dml_acknowledged, original.dml_acknowledged);
    assert_eq!(policy.local.lock().unwrap().len(), 1);
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
    for prior_failure in [false, true] {
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
        if prior_failure {
            transport.fail(AppError::invalid("prior policy failure"));
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
        if prior_failure {
            assert_eq!(
                transport.first_error().unwrap().message,
                "prior policy failure"
            );
        }
    }
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
