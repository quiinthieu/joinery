//! Real loopback SSH preflight ownership through Studio, including shutdown.
use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use russh::keys::{Algorithm, HashAlg, PrivateKey};
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

mod common;

const BOUND: Duration = Duration::from_secs(3);

struct StalledKeyExchange {
    port: u16,
    started: oneshot::Receiver<()>,
    closed: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

impl StalledKeyExchange {
    async fn new() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (started_tx, started) = oneshot::channel();
        let (closed_tx, closed) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut identification = Vec::new();
            loop {
                let byte = stream.read_u8().await.unwrap();
                identification.push(byte);
                assert!(identification.len() <= 255);
                if byte == b'\n' {
                    break;
                }
            }
            assert!(identification.starts_with(b"SSH-2.0-"));
            stream
                .write_all(b"SSH-2.0-preflight-fixture\r\n")
                .await
                .unwrap();
            let size = stream.read_u32().await.unwrap() as usize;
            assert!((2..=256 * 1024).contains(&size));
            let mut packet = vec![0; size];
            stream.read_exact(&mut packet).await.unwrap();
            assert_eq!(packet[1], 20, "client must have started key exchange");
            started_tx.send(()).unwrap();
            // The peer stays open until client EOF. Cancelling the fixture or
            // reaching a timer must never count as proof of client cleanup.
            let mut remainder = Vec::new();
            match stream.read_to_end(&mut remainder).await {
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::BrokenPipe
                    ) => {}
                Err(error) => panic!("SSH peer failed before EOF: {error}"),
            }
            closed_tx.send(()).unwrap();
        });
        Self {
            port,
            started,
            closed,
            task,
        }
    }

    async fn started(&mut self) {
        tokio::time::timeout(BOUND, &mut self.started)
            .await
            .unwrap()
            .unwrap();
    }

    async fn closed(&mut self) {
        tokio::time::timeout(BOUND, &mut self.closed)
            .await
            .expect("preflight retained its SSH socket")
            .unwrap();
        (&mut self.task).await.unwrap();
    }
}

impl Drop for StalledKeyExchange {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn studio() -> (Arc<Studio>, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    (studio, root)
}

fn profile(ssh_port: u16, target_port: u16) -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "SSH preflight fixture".into(),
        group_id: "local".into(),
        engine: EngineKind::Redis,
        host: "database.fixture.test".into(),
        port: target_port,
        database: "0".into(),
        username: "default".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::ReadOnly,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: Some(SshConfig {
            host: "127.0.0.1".into(),
            port: ssh_port,
            username: "fixture".into(),
            authentication: SshAuthentication::Password,
            private_key_path: None,
            agent_socket_path: None,
        }),
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: None,
    }
}

fn secrets() -> ConnectionSecrets {
    ConnectionSecrets {
        password: None,
        ssh_password: Some("ssh-test-only".into()),
        key_passphrase: None,
    }
}

fn probe(
    studio: &Arc<Studio>,
    profile_id: &str,
    attempt_id: Option<String>,
) -> tokio::task::JoinHandle<AppResult<SshHostKey>> {
    let studio = studio.clone();
    let profile_id = profile_id.to_owned();
    tokio::spawn(async move {
        studio
            .probe_ssh_with_attempt(&profile_id, attempt_id.as_deref())
            .await
    })
}

async fn cancelled(pending: tokio::task::JoinHandle<AppResult<SshHostKey>>) {
    let error = tokio::time::timeout(BOUND, pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
}

#[tokio::test]
async fn prepared_probe_cancellation_closes_kex_and_cannot_revive_the_attempt() {
    let (studio, _root) = studio();
    let mut peer = StalledKeyExchange::new().await;
    let profile = studio.save_profile(profile(peer.port, 6379), None).unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    let pending = probe(&studio, &profile.id, Some(id.clone()));
    peer.started().await;
    assert_eq!(
        studio
            .connect_prepared(&id, secrets())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio
            .probe_ssh_with_attempt(&profile.id, Some(&id))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    studio.cancel_connection_attempt(&id);
    cancelled(pending).await;
    peer.closed().await;
    assert_eq!(
        studio
            .connect_prepared(&id, secrets())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(studio.prepare_connection(&profile.id).is_ok());
}

#[tokio::test]
async fn shutdown_drains_both_legacy_and_prepared_probe_sockets() {
    for prepared in [false, true] {
        let (studio, _root) = studio();
        let mut peer = StalledKeyExchange::new().await;
        let profile = studio.save_profile(profile(peer.port, 6379), None).unwrap();
        let id = prepared.then(|| studio.prepare_connection(&profile.id).unwrap());
        let pending = probe(&studio, &profile.id, id.clone());
        peer.started().await;
        tokio::time::timeout(BOUND, studio.shutdown_and_wait())
            .await
            .unwrap()
            .unwrap();
        cancelled(pending).await;
        peer.closed().await;
        if let Some(id) = id {
            assert_eq!(
                studio
                    .connect_prepared(&id, secrets())
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Cancelled
            );
        }
        assert_eq!(studio.usage().session_count, 0);
    }
}

#[tokio::test]
async fn shutdown_rejects_new_probes_before_any_tcp_connection() {
    let (studio, _root) = studio();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let profile = studio
        .save_profile(profile(listener.local_addr().unwrap().port(), 6379), None)
        .unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    studio.shutdown_and_wait().await.unwrap();
    assert_eq!(
        studio.probe_ssh(&profile.id).await.unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        studio
            .probe_ssh_with_attempt(&profile.id, Some(&id))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn mismatched_and_stale_probe_ids_cannot_open_tcp_or_leave_capacity_reserved() {
    let (studio, _root) = studio();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let first = studio.save_profile(profile(port, 6379), None).unwrap();
    let second = studio.save_profile(profile(port, 6379), None).unwrap();
    let id = studio.prepare_connection(&first.id).unwrap();
    assert_eq!(
        studio
            .probe_ssh_with_attempt(&second.id, Some(&id))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    for stale in [&id, "caller-chosen-id"] {
        assert_eq!(
            studio
                .probe_ssh_with_attempt(&first.id, Some(stale))
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
    }
    let cancelled_id = studio.prepare_connection(&first.id).unwrap();
    studio.cancel_connection_attempt(&cancelled_id);
    assert_eq!(
        studio
            .probe_ssh_with_attempt(&first.id, Some(&cancelled_id))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    for _ in 0..4 {
        studio.prepare_connection(&first.id).unwrap();
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn failed_probe_retires_its_reservation_instead_of_rearming_it() {
    let (studio, _root) = studio();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let profile = studio
        .save_profile(profile(listener.local_addr().unwrap().port(), 6379), None)
        .unwrap();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(b"SSH-1.0-unsupported\r\n").await.unwrap();
    });
    let id = studio.prepare_connection(&profile.id).unwrap();
    let error = tokio::time::timeout(BOUND, studio.probe_ssh_with_attempt(&profile.id, Some(&id)))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ConnectionFailed);
    peer.await.unwrap();
    assert_eq!(
        studio
            .connect_prepared(&id, secrets())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    for _ in 0..4 {
        studio.prepare_connection(&profile.id).unwrap();
    }
}

#[tokio::test]
async fn profile_save_and_delete_cancel_a_probe_without_rearming_it() {
    for mutation in ["unchanged", "changed", "delete"] {
        let (studio, _root) = studio();
        let mut peer = StalledKeyExchange::new().await;
        let mut profile = studio.save_profile(profile(peer.port, 6379), None).unwrap();
        let id = studio.prepare_connection(&profile.id).unwrap();
        let pending = probe(&studio, &profile.id, Some(id.clone()));
        peer.started().await;
        match mutation {
            "delete" => studio.delete_profile(&profile.id).unwrap(),
            _ => {
                if mutation == "changed" {
                    profile.name = "Updated preflight fixture".into();
                }
                studio.save_profile(profile.clone(), None).unwrap();
            }
        }
        cancelled(pending).await;
        peer.closed().await;
        assert_eq!(
            studio
                .connect_prepared(&id, secrets())
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        studio.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test]
async fn abandoned_preflight_future_closes_socket_and_releases_its_owner() {
    for prepared in [false, true] {
        let (studio, _root) = studio();
        let mut peer = StalledKeyExchange::new().await;
        let profile = studio.save_profile(profile(peer.port, 6379), None).unwrap();
        let id = prepared.then(|| studio.prepare_connection(&profile.id).unwrap());
        let pending = probe(&studio, &profile.id, id.clone());
        peer.started().await;
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        peer.closed().await;
        if let Some(id) = id {
            assert_eq!(
                studio
                    .connect_prepared(&id, secrets())
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Cancelled
            );
        }
        for _ in 0..4 {
            studio.prepare_connection(&profile.id).unwrap();
        }
        tokio::time::timeout(BOUND, studio.shutdown_and_wait())
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn successful_preflight_reuses_its_reservation_for_an_actual_tunnelled_connection() {
    let (studio, _root) = studio();
    let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let target_port = target.local_addr().unwrap().port();
    let database = tokio::spawn(async move {
        let (mut stream, _) = target.accept().await.unwrap();
        let mut ping = [0; 14];
        stream.read_exact(&mut ping).await.unwrap();
        assert_eq!(&ping, b"*1\r\n$4\r\nPING\r\n");
        stream.write_all(b"+PONG\r\n").await.unwrap();
        let mut remainder = Vec::new();
        match stream.read_to_end(&mut remainder).await {
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::BrokenPipe
                ) => {}
            Err(error) => panic!("tunnelled target failed before EOF: {error}"),
        }
    });
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let bastion = common::bastion(target_port, &identity, None, 0).await;
    let profile = studio
        .save_profile(profile(bastion.port, target_port), None)
        .unwrap();
    let id = studio.prepare_connection(&profile.id).unwrap();
    let key = tokio::time::timeout(BOUND, studio.probe_ssh_with_attempt(&profile.id, Some(&id)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        key.fingerprint,
        bastion
            .key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string()
    );
    studio
        .trust_ssh_key(key.challenge_id.as_deref().unwrap(), &key.fingerprint)
        .unwrap();
    let session = tokio::time::timeout(BOUND, studio.connect_prepared(&id, secrets()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.id, id);
    assert_eq!(studio.usage().session_count, 1);
    assert_eq!(bastion.connections.load(Ordering::SeqCst), 2);
    studio.disconnect(&id);
    tokio::time::timeout(BOUND, database)
        .await
        .unwrap()
        .unwrap();
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn successful_legacy_probes_do_not_leave_reserved_or_claimed_capacity() {
    let (studio, _root) = studio();
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let bastion = common::bastion(6379, &identity, None, 0).await;
    let profile = studio
        .save_profile(profile(bastion.port, 6379), None)
        .unwrap();
    for _ in 0..5 {
        tokio::time::timeout(BOUND, studio.probe_ssh(&profile.id))
            .await
            .unwrap()
            .unwrap();
    }
    for _ in 0..4 {
        studio.prepare_connection(&profile.id).unwrap();
    }
    assert!(studio.prepare_connection(&profile.id).is_err());
    studio.shutdown_and_wait().await.unwrap();
}
