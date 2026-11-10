//! Real loopback RESP handshakes exercise the production connect path. No
//! profile, IPC field, or production driver is replaced by a test connector.
use super::*;
use crate::secrets::MemorySecrets;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

const BOUND: Duration = Duration::from_secs(3);

fn studio() -> (Arc<Studio>, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    (studio, root)
}

fn profile(port: u16) -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Loopback setup".into(),
        group_id: "local".into(),
        engine: EngineKind::Redis,
        host: "127.0.0.1".into(),
        port,
        database: "0".into(),
        username: "default".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::ReadOnly,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: None,
    }
}

fn secrets() -> ConnectionSecrets {
    ConnectionSecrets {
        password: Some("loopback-only".into()),
        ..Default::default()
    }
}

async fn line(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        bytes.push(stream.read_u8().await.unwrap());
        assert!(bytes.len() < 1024);
        if bytes.ends_with(b"\r\n") {
            bytes.truncate(bytes.len() - 2);
            return bytes;
        }
    }
}

async fn command(stream: &mut TcpStream) -> Vec<Vec<u8>> {
    let header = line(stream).await;
    assert_eq!(header[0], b'*');
    let count: usize = std::str::from_utf8(&header[1..]).unwrap().parse().unwrap();
    assert!(count <= 3);
    let mut arguments = Vec::new();
    for _ in 0..count {
        let header = line(stream).await;
        assert_eq!(header[0], b'$');
        let size: usize = std::str::from_utf8(&header[1..]).unwrap().parse().unwrap();
        assert!(size < 1024);
        let mut value = vec![0; size + 2];
        stream.read_exact(&mut value).await.unwrap();
        assert!(value.ends_with(b"\r\n"));
        value.truncate(size);
        arguments.push(value);
    }
    arguments
}

fn assert_peer_closed(result: std::io::Result<usize>) {
    assert!(
        matches!(result, Ok(0))
            || result.is_err_and(|error| {
                matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                )
            })
    );
}

struct Endpoint {
    port: u16,
    auth: oneshot::Receiver<()>,
    release: Option<oneshot::Sender<()>>,
    closed: oneshot::Receiver<()>,
    handshake: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

impl Endpoint {
    async fn new() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (auth_tx, auth) = oneshot::channel();
        let (release, ready) = oneshot::channel();
        let (closed_tx, closed) = oneshot::channel();
        let (handshake_tx, handshake) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert_eq!(command(&mut stream).await[0], b"AUTH");
            auth_tx.send(()).unwrap();
            let mut byte = [0];
            tokio::select! {
                _ = ready => {
                    let handshake_result = async {
                        stream.write_all(b"+OK\r\n").await?;
                        let mut ping = [0; 14];
                        stream.read_exact(&mut ping).await?;
                        assert_eq!(&ping, b"*1\r\n$4\r\nPING\r\n");
                        stream.write_all(b"+PONG\r\n").await
                    }.await;
                    if let Err(error) = handshake_result {
                        assert!(matches!(error.kind(), std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe));
                        let _ = closed_tx.send(());
                        return;
                    }
                    let _ = handshake_tx.send(());
                }
                count = stream.read(&mut byte) => {
                    assert_peer_closed(count);
                    let _ = closed_tx.send(());
                    return;
                }
            }
            assert_peer_closed(stream.read(&mut byte).await);
            let _ = closed_tx.send(());
        });
        Self {
            port,
            auth,
            release: Some(release),
            closed,
            handshake,
            task,
        }
    }
    fn allow(&mut self) {
        let _ = self.release.take().unwrap().send(());
    }
    async fn dispatched(&mut self) {
        tokio::time::timeout(BOUND, &mut self.auth)
            .await
            .unwrap()
            .unwrap();
    }
    async fn authenticated(&mut self) {
        tokio::time::timeout(BOUND, &mut self.handshake)
            .await
            .unwrap()
            .unwrap();
    }
    async fn reaped(&mut self) {
        tokio::time::timeout(BOUND, &mut self.closed)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(BOUND, &mut self.task)
            .await
            .unwrap()
            .unwrap();
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn connect(studio: &Arc<Studio>, id: String) -> tokio::task::JoinHandle<AppResult<SessionInfo>> {
    let studio = studio.clone();
    tokio::spawn(async move { studio.connect_prepared(&id, secrets()).await })
}
async fn outcome(task: tokio::task::JoinHandle<AppResult<SessionInfo>>) -> AppResult<SessionInfo> {
    tokio::time::timeout(BOUND, task).await.unwrap().unwrap()
}

async fn hold_configuration(
    studio: &Arc<Studio>,
) -> (std::sync::mpsc::Sender<()>, tokio::task::JoinHandle<()>) {
    let studio = studio.clone();
    let (acquired_tx, acquired_rx) = oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let owner = tokio::task::spawn_blocking(move || {
        let _configuration = studio.configuration_gate.lock();
        acquired_tx.send(()).unwrap();
        release_rx.recv_timeout(BOUND).unwrap();
    });
    tokio::time::timeout(BOUND, acquired_rx)
        .await
        .unwrap()
        .unwrap();
    (release_tx, owner)
}

#[tokio::test]
async fn cancelled_or_caller_chosen_reservations_never_dispatch() {
    let (studio, _root) = studio();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let profile = studio
        .save_profile(profile(listener.local_addr().unwrap().port()), None)
        .unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    assert_eq!(uuid::Uuid::parse_str(&id).unwrap().get_version_num(), 4);
    studio.cancel_connection_attempt(&id);
    for id in [id, new_id()] {
        assert_eq!(
            studio
                .connect_prepared(&id, secrets())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    assert!(!studio.connection_attempts.has_pending());
}

#[tokio::test]
async fn prepared_request_claims_once_and_publishes_the_reserved_session_id() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    let task = connect(&studio, id.clone());
    endpoint.dispatched().await;
    assert_eq!(
        studio
            .connect_prepared(&id, secrets())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    endpoint.allow();
    let info = outcome(task).await.unwrap();
    assert_eq!(info.id, id);
    assert_eq!(info.connection_id, profile.id);
    assert_eq!(studio.usage().session_count, 1);
    assert_eq!(
        studio
            .connect_prepared(&id, secrets())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    studio.disconnect(&id);
    endpoint.reaped().await;
}

#[tokio::test]
async fn live_and_pending_sessions_share_per_profile_capacity() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    let live = studio.prepare_connection(&profile.id).unwrap();
    endpoint.allow();
    outcome(connect(&studio, live.clone())).await.unwrap();
    let pending = (0..3)
        .map(|_| studio.prepare_connection(&profile.id).unwrap())
        .collect::<Vec<_>>();
    assert!(studio.prepare_connection(&profile.id).is_err());
    studio.cancel_connection_attempt(&pending[0]);
    assert!(studio.prepare_connection(&profile.id).is_ok());
    studio.shutdown_and_wait().await.unwrap();
    endpoint.reaped().await;
    assert!(!studio.connection_attempts.has_pending());
}

#[test]
fn prepared_requests_share_global_capacity_across_profiles() {
    let (studio, _root) = studio();
    let mut ids = Vec::new();
    for _ in 0..8 {
        let profile = studio.save_profile(profile(9), None).unwrap();
        for _ in 0..4 {
            ids.push(studio.prepare_connection(&profile.id).unwrap());
        }
    }
    let profile = studio.save_profile(profile(9), None).unwrap();
    assert!(studio.prepare_connection(&profile.id).is_err());
    studio.cancel_connection_attempt(&ids[0]);
    assert!(studio.prepare_connection(&profile.id).is_ok());
    studio.shutdown();
    assert!(!studio.connection_attempts.has_pending());
}

#[tokio::test]
async fn rejected_auth_releases_the_attempt_and_never_publishes_a_session() {
    let (studio, _root) = studio();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let profile = studio
        .save_profile(profile(listener.local_addr().unwrap().port()), None)
        .unwrap();
    let endpoint = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert_eq!(command(&mut stream).await[0], b"AUTH");
        stream.write_all(b"-ERR rejected\r\n").await.unwrap();
        assert_peer_closed(stream.read(&mut [0]).await);
    });
    let id = studio.prepare_connection(&profile.id).unwrap();
    assert_eq!(
        outcome(connect(&studio, id)).await.unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    tokio::time::timeout(BOUND, endpoint)
        .await
        .unwrap()
        .unwrap();
    assert!(!studio.connection_attempts.has_pending());
    assert_eq!(studio.usage().session_count, 0);
    assert!(studio.prepare_connection(&profile.id).is_ok());
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn pending_auth_does_not_block_profile_mutation_or_independent_setup() {
    let (studio, _root) = studio();
    let mut first = Endpoint::new().await;
    let mut second = Endpoint::new().await;
    let profile_a = studio.save_profile(profile(first.port), None).unwrap();
    let mut profile_b = studio.save_profile(profile(second.port), None).unwrap();
    let id = studio.prepare_connection(&profile_a.id).unwrap();
    let pending = connect(&studio, id.clone());
    first.dispatched().await;
    profile_b.name = "Edited while another authentication waits".into();
    let save_studio = studio.clone();
    let profile_b = tokio::time::timeout(
        BOUND,
        tokio::task::spawn_blocking(move || save_studio.save_profile(profile_b, None)),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    let second_id = studio.prepare_connection(&profile_b.id).unwrap();
    second.allow();
    outcome(connect(&studio, second_id.clone())).await.unwrap();
    assert!(!pending.is_finished());
    studio.cancel_connection_attempt(&id);
    assert_eq!(
        outcome(pending).await.unwrap_err().code,
        ErrorCode::Cancelled
    );
    first.reaped().await;
    studio.disconnect(&second_id);
    second.reaped().await;
}

#[tokio::test]
async fn changed_and_unchanged_saves_invalidate_pending_auth_and_prepared_requests() {
    for changed in [false, true] {
        let (studio, _root) = studio();
        let mut endpoint = Endpoint::new().await;
        let mut profile = studio.save_profile(profile(endpoint.port), None).unwrap();
        let id = studio.prepare_connection(&profile.id).unwrap();
        let unused = studio.prepare_connection(&profile.id).unwrap();
        let pending = connect(&studio, id);
        endpoint.dispatched().await;
        if changed {
            profile.name = "Changed during auth".into();
        }
        studio.save_profile(profile, None).unwrap();
        assert_eq!(
            outcome(pending).await.unwrap_err().code,
            ErrorCode::Cancelled
        );
        endpoint.reaped().await;
        assert_eq!(
            studio
                .connect_prepared(&unused, secrets())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        assert_eq!(studio.usage().session_count, 0);
        assert!(!studio.connection_attempts.has_pending());
    }
}

#[tokio::test]
async fn delete_cancels_auth_without_recreating_the_profile_or_session() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    let pending = connect(&studio, id);
    endpoint.dispatched().await;
    studio.delete_profile(&profile.id).unwrap();
    assert_eq!(
        outcome(pending).await.unwrap_err().code,
        ErrorCode::Cancelled
    );
    endpoint.reaped().await;
    assert!(studio.persistence.profile(&profile.id).is_err());
    assert_eq!(studio.usage().session_count, 0);
    assert!(studio.prepare_connection(&profile.id).is_err());
}

#[tokio::test]
async fn restore_refuses_prepared_claimed_and_cancelled_but_not_yet_dropped_auth() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    let snapshot = studio.create_metadata_backup().unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    let restore =
        || studio.restore_metadata_backup(&snapshot.id, &snapshot.digest, "RESTORE", None);
    assert!(restore().is_err());
    let pending = connect(&studio, id.clone());
    endpoint.dispatched().await;
    assert!(restore().is_err());
    studio.cancel_connection_attempt(&id);
    // The task has not had a scheduling turn to drop its claimed owner yet.
    assert!(restore().is_err());
    assert_eq!(
        outcome(pending).await.unwrap_err().code,
        ErrorCode::Cancelled
    );
    endpoint.reaped().await;
    restore().unwrap();
}

#[tokio::test]
async fn shutdown_closes_admission_cancels_auth_and_waits_for_its_owner() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    let pending = connect(&studio, id);
    endpoint.dispatched().await;
    tokio::time::timeout(BOUND, studio.shutdown_and_wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        outcome(pending).await.unwrap_err().code,
        ErrorCode::Cancelled
    );
    endpoint.reaped().await;
    assert_eq!(studio.usage().session_count, 0);
    assert!(!studio.connection_attempts.has_pending());
    assert_eq!(
        studio.prepare_connection(&profile.id).unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        studio
            .connect(&profile.id, secrets())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn abandoned_connect_future_releases_capacity_and_closes_its_socket() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    let pending = connect(&studio, id.clone());
    endpoint.dispatched().await;
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    endpoint.reaped().await;
    assert!(!studio.connection_attempts.has_pending());
    assert_eq!(
        studio
            .connect_prepared(&id, secrets())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(studio.prepare_connection(&profile.id).is_ok());
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_queued_at_ready_auth_retires_session_after_configuration_gate() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    let pending = connect(&studio, id.clone());
    endpoint.dispatched().await;
    let (release_configuration, owner) = hold_configuration(&studio).await;
    endpoint.allow();
    endpoint.authenticated().await;
    let cancelling = studio.clone();
    let cancelled = tokio::task::spawn_blocking(move || cancelling.cancel_connection_attempt(&id));
    release_configuration.send(()).unwrap();
    owner.await.unwrap();
    tokio::time::timeout(BOUND, cancelled)
        .await
        .unwrap()
        .unwrap();
    if let Err(error) = outcome(pending).await {
        assert_eq!(error.code, ErrorCode::Cancelled);
    }
    endpoint.reaped().await;
    assert_eq!(studio.usage().session_count, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_racing_ready_auth_either_prevents_or_snapshots_publication() {
    for _ in 0..8 {
        let (studio, _root) = studio();
        let mut endpoint = Endpoint::new().await;
        let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
        let id = studio.prepare_connection(&profile.id).unwrap();
        let pending = connect(&studio, id);
        endpoint.dispatched().await;
        let (release_configuration, owner) = hold_configuration(&studio).await;
        endpoint.allow();
        endpoint.authenticated().await;
        let closing = studio.clone();
        let shutdown = tokio::task::spawn_blocking(move || closing.shutdown());
        release_configuration.send(()).unwrap();
        owner.await.unwrap();
        let connected = outcome(pending).await;
        tokio::time::timeout(BOUND, shutdown)
            .await
            .unwrap()
            .unwrap();
        if let Err(error) = connected {
            assert_eq!(error.code, ErrorCode::Cancelled);
        }
        assert_eq!(studio.usage().session_count, 0);
        assert!(!studio.connection_attempts.has_pending());
        assert!(studio.prepare_connection(&profile.id).is_err());
        endpoint.reaped().await;
    }
}

#[tokio::test]
async fn legacy_connect_uses_the_same_tracked_publication_and_disconnect_path() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    endpoint.allow();
    let info = tokio::time::timeout(BOUND, studio.connect(&profile.id, secrets()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.connection_id, profile.id);
    assert!(!studio.connection_attempts.has_pending());
    assert!(studio.session(&info.id).is_ok());
    studio.disconnect(&info.id);
    endpoint.reaped().await;
}

#[tokio::test]
async fn legacy_setup_inflight_cannot_publish_after_synchronous_shutdown() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    let connecting = studio.clone();
    let pending = tokio::spawn(async move { connecting.connect(&profile.id, secrets()).await });
    endpoint.dispatched().await;
    studio.shutdown();
    endpoint.allow();
    assert_eq!(
        outcome(pending).await.unwrap_err().code,
        ErrorCode::Cancelled
    );
    endpoint.reaped().await;
    assert_eq!(studio.usage().session_count, 0);
}

#[tokio::test]
async fn fixture_reset_twice_retires_resources_and_allows_new_connections() {
    let (studio, root) = studio();
    let file = root.path().join("fixture.csv");
    std::fs::write(&file, b"fixture\n").unwrap();
    for _ in 0..2 {
        let mut endpoint = Endpoint::new().await;
        let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
        let live = studio.prepare_connection(&profile.id).unwrap();
        let stale = studio.prepare_connection(&profile.id).unwrap();
        endpoint.allow();
        outcome(connect(&studio, live.clone())).await.unwrap();
        let grant = studio
            .file_capabilities
            .mint_read_grant(file.clone(), 1024)
            .unwrap();
        // Stored-result exports need not have a session to be retired.
        let export_id = new_id();
        let export_token = CancellationToken::new();
        studio.exports.lock().insert(
            export_id.clone(),
            ExportJob {
                session_id: None,
                token: export_token.clone(),
            },
        );

        tokio::time::timeout(BOUND, studio.reset_fixtures())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(studio.usage().session_count, 0);
        assert!(studio.session(&live).is_err());
        assert!(!studio.connection_attempts.has_pending());
        assert!(export_token.is_cancelled());
        assert!(
            studio
                .file_capabilities
                .resolve_read_grant(&grant.token)
                .is_err()
        );
        assert_eq!(
            studio
                .connect_prepared(&stale, secrets())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        endpoint.reaped().await;
        studio.exports.lock().remove(&export_id);
        let next = studio.prepare_connection(&profile.id).unwrap();
        studio.cancel_connection_attempt(&next);
    }
}

#[tokio::test]
async fn fixture_reset_drains_stalled_auth_and_prevents_late_publication() {
    let (studio, _root) = studio();
    let mut endpoint = Endpoint::new().await;
    let profile = studio.save_profile(profile(endpoint.port), None).unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    let pending = connect(&studio, id.clone());
    endpoint.dispatched().await;

    tokio::time::timeout(BOUND, studio.reset_fixtures())
        .await
        .unwrap()
        .unwrap();
    endpoint.allow();
    assert_eq!(
        outcome(pending).await.unwrap_err().code,
        ErrorCode::Cancelled
    );
    endpoint.reaped().await;
    assert!(!studio.connection_attempts.has_pending());
    assert_eq!(studio.usage().session_count, 0);
    assert_eq!(
        studio
            .connect_prepared(&id, secrets())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    let next = studio.prepare_connection(&profile.id).unwrap();
    studio.cancel_connection_attempt(&next);
}

#[tokio::test]
async fn fixture_reset_waits_only_for_owners_in_its_retirement_snapshot() {
    let (studio, _root) = studio();
    let profile = studio.save_profile(profile(9), None).unwrap();
    let old_id = studio.prepare_connection(&profile.id).unwrap();
    let old_owner = studio.connection_attempts.claim(&old_id).unwrap();
    let cancelled = old_owner.cancellation();
    let resetting = studio.clone();
    let mut reset = tokio::spawn(async move { resetting.reset_fixtures().await });
    tokio::time::timeout(BOUND, cancelled.cancelled())
        .await
        .unwrap();

    // Acquiring the configuration gate here places this reservation after the
    // reset snapshot, while the old claimed owner still prevents its drain.
    let fresh_id = studio.prepare_connection(&profile.id).unwrap();
    drop(old_owner);
    let result = tokio::time::timeout(BOUND, &mut reset).await;
    if result.is_err() {
        reset.abort();
    }
    result
        .expect("fresh work must not block the fixture retirement snapshot")
        .unwrap()
        .unwrap();
    let fresh_owner = studio.connection_attempts.claim(&fresh_id).unwrap();
    assert!(!fresh_owner.cancellation().is_cancelled());
    drop(fresh_owner);

    studio.shutdown_and_wait().await.unwrap();
    studio.reset_fixtures().await.unwrap();
    assert_eq!(
        studio.prepare_connection(&profile.id).unwrap_err().code,
        ErrorCode::Cancelled
    );
}

#[tokio::test]
async fn fixture_reset_never_reopens_real_shutdown_admission() {
    let (studio, _root) = studio();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let profile = studio
        .save_profile(profile(listener.local_addr().unwrap().port()), None)
        .unwrap();
    let stale = studio.prepare_connection(&profile.id).unwrap();
    studio.shutdown_and_wait().await.unwrap();
    for _ in 0..2 {
        studio.reset_fixtures().await.unwrap();
        assert_eq!(
            studio.prepare_connection(&profile.id).unwrap_err().code,
            ErrorCode::Cancelled
        );
        assert_eq!(
            studio
                .connect_prepared(&stale, secrets())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_ack_waits_for_inflight_remembered_credential_write() {
    struct BarrierSecrets {
        inner: MemorySecrets,
        entered: Mutex<Option<oneshot::Sender<()>>>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        written: AtomicBool,
    }
    impl SecretStore for BarrierSecrets {
        fn get(&self, name: &str) -> AppResult<Option<String>> {
            self.inner.get(name)
        }
        fn set(&self, name: &str, secret: &str) -> AppResult<()> {
            if let Some(entered) = self.entered.lock().take() {
                entered.send(()).unwrap();
                self.release.lock().recv_timeout(BOUND).unwrap();
            }
            self.inner.set(name, secret)?;
            self.written.store(true, Ordering::Release);
            Ok(())
        }
        fn delete(&self, name: &str) -> AppResult<()> {
            self.inner.delete(name)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let secrets_store = Arc::new(BarrierSecrets {
        inner: MemorySecrets::default(),
        entered: Mutex::new(Some(entered_tx)),
        release: Mutex::new(release_rx),
        written: AtomicBool::new(false),
    });
    let studio = Studio::with_secrets(root.path(), secrets_store.clone()).unwrap();
    let mut endpoint = Endpoint::new().await;
    let mut profile = profile(endpoint.port);
    profile.remember_password = true;
    let profile = studio.save_profile(profile, None).unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    endpoint.allow();
    let pending = connect(&studio, id.clone());
    tokio::time::timeout(BOUND, entered_rx)
        .await
        .unwrap()
        .unwrap();
    let cancelling = studio.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    let cancelled = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        cancelling.cancel_connection_attempt(&id);
        ack_tx.send(()).unwrap();
    });
    started_rx.recv_timeout(BOUND).unwrap();
    assert!(
        matches!(
            ack_rx.recv_timeout(Duration::from_millis(30)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "cancellation must not acknowledge while a remembered credential write is pending"
    );
    assert!(!secrets_store.written.load(Ordering::Acquire));
    release_tx.send(()).unwrap();
    ack_rx.recv_timeout(BOUND).unwrap();
    cancelled.join().unwrap();
    assert!(secrets_store.written.load(Ordering::Acquire));
    // Publication may win the serialized race; cancellation must still retire
    // the exact reserved session before acknowledging, even without its reply.
    assert_eq!(studio.usage().session_count, 0);
    let _ = outcome(pending).await;
    endpoint.reaped().await;
}

#[tokio::test]
async fn cancellation_retires_published_session_without_consuming_its_connect_response() {
    let (studio, _root) = studio();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let profile = studio
        .save_profile(profile(listener.local_addr().unwrap().port()), None)
        .unwrap();
    let peer = tokio::spawn(async move {
        let mut clients = tokio::task::JoinSet::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            clients.spawn(async move {
                assert_eq!(command(&mut stream).await[0], b"AUTH");
                stream.write_all(b"+OK\r\n").await.unwrap();
                assert_eq!(command(&mut stream).await, [b"PING".to_vec()]);
                stream.write_all(b"+PONG\r\n").await.unwrap();
                assert_peer_closed(stream.read(&mut [0]).await);
            });
        }
        while let Some(client) = clients.join_next().await {
            client.unwrap();
        }
    });
    let lost_id = studio.prepare_connection(&profile.id).unwrap();
    let lost_response = connect(&studio, lost_id.clone());
    tokio::time::timeout(BOUND, async {
        while studio.session(&lost_id).is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let newer_id = studio.prepare_connection(&profile.id).unwrap();
    outcome(connect(&studio, newer_id.clone())).await.unwrap();
    studio.cancel_connection_attempt(&lost_id);
    studio.cancel_connection_attempt(&lost_id);
    assert!(studio.session(&lost_id).is_err());
    assert!(
        studio.session(&newer_id).is_ok(),
        "cancellation must not match another session by profile"
    );
    assert_eq!(studio.usage().session_count, 1);
    drop(lost_response); // Deliberately discard the reply rather than reconcile in the UI.
    studio.disconnect(&newer_id);
    tokio::time::timeout(BOUND, peer).await.unwrap().unwrap();
}
