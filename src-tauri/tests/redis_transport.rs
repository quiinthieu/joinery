use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{mpsc, oneshot},
};
mod common;

async fn command<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    io: &mut BufReader<T>,
) -> Vec<Vec<u8>> {
    let mut line = String::new();
    io.read_line(&mut line).await.unwrap();
    let count: usize = line.trim().strip_prefix('*').unwrap().parse().unwrap();
    let mut result = Vec::new();
    for _ in 0..count {
        line.clear();
        io.read_line(&mut line).await.unwrap();
        let length: usize = line.trim().strip_prefix('$').unwrap().parse().unwrap();
        let mut bytes = vec![0; length + 2];
        io.read_exact(&mut bytes).await.unwrap();
        bytes.truncate(length);
        result.push(bytes);
    }
    result
}

#[tokio::test]
async fn verified_tls_over_ssh_preserves_the_redis_hostname_and_rejects_mismatch() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificate =
        rcgen::generate_simple_self_signed(vec!["database.fixture.test".into()]).unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der())
                .into(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = listener.local_addr().unwrap().port();
    let names = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = names.clone();
    let tls_server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _=connections.join_next(),if !connections.is_empty()=>{},
                connection=listener.accept()=>{
                    let (stream,_)=connection.unwrap();let acceptor=acceptor.clone();let names=captured.clone();
                    connections.spawn(async move {
                        let Ok(tls)=acceptor.accept(stream).await else{return};
                        names.lock().unwrap().push(tls.get_ref().1.server_name().unwrap().to_owned());
                        let mut io=BufReader::new(tls);
                        while !io.fill_buf().await.unwrap_or_default().is_empty() {
                            assert_eq!(command(&mut io).await[0],b"PING");io.get_mut().write_all(b"+PONG\r\n").await.unwrap();
                        }
                    });
                }
            }
        }
    });
    use russh::keys::{Algorithm, PrivateKey};
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let bastion = common::bastion(target, &identity, None, 0).await;
    let root = tempfile::tempdir().unwrap();
    let ca = root.path().join("redis-ca.pem");
    std::fs::write(&ca, certificate.cert.pem()).unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    for host in ["database.fixture.test", "wrong.fixture.test"] {
        let p = studio
            .save_profile(
                ConnectionProfile {
                    id: new_id(),
                    name: "Redis TLS fixture".into(),
                    group_id: "local".into(),
                    engine: EngineKind::Redis,
                    host: host.into(),
                    port: target,
                    database: "0".into(),
                    username: "default".into(),
                    environment: Environment::Local,
                    safe_mode: SafeMode::ReadOnly,
                    tls_mode: TlsMode::VerifyFull,
                    ca_cert_path: Some(ca.to_string_lossy().into()),
                    ssh: Some(SshConfig {
                        host: "127.0.0.1".into(),
                        port: bastion.port,
                        username: "fixture".into(),
                        authentication: SshAuthentication::Password,
                        private_key_path: None,
                        agent_socket_path: None,
                    }),
                    remember_password: false,
                    store_query_text: false,
                    store_drafts: false,
                    snowflake: None,
                },
                None,
            )
            .unwrap();
        let key = studio.probe_ssh(&p.id).await.unwrap();
        assert_eq!(
            key.fingerprint,
            bastion
                .key
                .public_key()
                .fingerprint(russh::keys::HashAlg::Sha256)
                .to_string()
        );
        if let Some(challenge) = key.challenge_id {
            studio.trust_ssh_key(&challenge, &key.fingerprint).unwrap();
        }
        let result = studio
            .connect(
                &p.id,
                ConnectionSecrets {
                    ssh_password: Some("ssh-test-only".into()),
                    ..Default::default()
                },
            )
            .await;
        if host == "wrong.fixture.test" {
            assert_eq!(result.unwrap_err().code, ErrorCode::ConnectionFailed);
        } else {
            let session = result.unwrap();
            assert!(session.encrypted);
            let (_, rx) = start(&studio, &session, "PING");
            let events = finish(rx).await;
            assert!(events.iter().any(|e| matches!(
                e,
                QueryEvent::Finished {
                    status: RunStatus::Completed,
                    ..
                }
            )));
            studio.disconnect(&session.id);
        }
    }
    assert!(
        bastion
            .connections
            .load(std::sync::atomic::Ordering::SeqCst)
            > 0
    );
    assert_eq!(names.lock().unwrap().as_slice(), ["database.fixture.test"]);
    studio.shutdown();
    tls_server.abort();
}
async fn setup_as(port: u16, username: &str) -> (tempfile::TempDir, Arc<Studio>, SessionInfo) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let p = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "RESP transport fixture".into(),
                group_id: "local".into(),
                engine: EngineKind::Redis,
                host: "127.0.0.1".into(),
                port,
                database: "0".into(),
                username: username.into(),
                environment: Environment::Local,
                safe_mode: SafeMode::Confirm,
                tls_mode: TlsMode::Disable,
                ca_cert_path: None,
                ssh: None,
                remember_password: false,
                store_query_text: false,
                store_drafts: false,
                snowflake: None,
            },
            None,
        )
        .unwrap();
    let s = studio
        .connect(&p.id, ConnectionSecrets::default())
        .await
        .unwrap();
    (root, studio, s)
}
fn start(
    studio: &Arc<Studio>,
    session: &SessionInfo,
    source: &str,
) -> (String, mpsc::UnboundedReceiver<QueryEvent>) {
    let plan = studio.prepare_execution(&session.id, source).unwrap();
    let approval = if plan.requires_approval {
        Some(studio.authorize(&plan.id, None, None).unwrap())
    } else {
        None
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let run = studio
        .execute(
            &plan.id,
            approval.as_deref(),
            Arc::new(move |e| tx.send(e).is_ok()),
        )
        .unwrap();
    (run, rx)
}
async fn finish(mut rx: mpsc::UnboundedReceiver<QueryEvent>) -> Vec<QueryEvent> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            let done = matches!(e, QueryEvent::Finished { .. });
            events.push(e);
            if done {
                break;
            }
        }
        events
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn cancelling_a_sent_read_closes_the_socket_and_retires_the_session() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sent_tx, sent_rx) = oneshot::channel();
    let (closed_tx, closed_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut io = BufReader::new(stream);
        assert_eq!(command(&mut io).await[0], b"PING");
        io.get_mut().write_all(b"+PONG\r\n").await.unwrap();
        assert_eq!(command(&mut io).await[0], b"GET");
        sent_tx.send(()).unwrap();
        let mut byte = [0];
        let closed = io.read(&mut byte).await.unwrap() == 0;
        closed_tx.send(closed).unwrap();
    });
    let (_root, studio, session) = setup_as(port, "default").await;
    let (run, rx) = start(&studio, &session, "GET pending");
    sent_rx.await.unwrap();
    studio.cancel(&run).await.unwrap();
    let events = finish(rx).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            QueryEvent::Finished {
                status: RunStatus::Cancelled,
                ..
            }
        )),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, QueryEvent::SessionClosed { .. }))
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(3), closed_rx)
            .await
            .unwrap()
            .unwrap()
    );
    server.await.unwrap();
    assert!(studio.prepare_execution(&session.id, "PING").is_err());
}

#[tokio::test]
async fn losing_a_write_reply_is_unknown_without_automatic_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut io = BufReader::new(stream);
        assert_eq!(command(&mut io).await[0], b"PING");
        io.get_mut().write_all(b"+PONG\r\n").await.unwrap();
        let request = command(&mut io).await;
        assert_eq!(request[0], b"SET");
        assert_eq!(request[1], b"one-attempt");
        // The peer accepts the command and disappears before its acknowledgement.
        drop(io);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let (_root, studio, session) = setup_as(port, "default").await;
    let (_, rx) = start(&studio, &session, "SET one-attempt value");
    let events = finish(rx).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            QueryEvent::Finished {
                status: RunStatus::OutcomeUnknown,
                ..
            }
        )),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, QueryEvent::SessionClosed { .. }))
    );
    server.await.unwrap();
}

#[tokio::test]
async fn oversized_bulk_and_array_headers_are_rejected_before_result_allocation() {
    for header in [b"$2147483647\r\n".as_slice(), b"*1000000000\r\n"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let header = header.to_vec();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut io = BufReader::new(stream);
            assert_eq!(command(&mut io).await[0], b"PING");
            io.get_mut().write_all(b"+PONG\r\n").await.unwrap();
            assert_eq!(command(&mut io).await[0], b"GET");
            io.get_mut().write_all(&header).await.unwrap();
            let mut byte = [0];
            assert_eq!(io.read(&mut byte).await.unwrap(), 0);
        });
        let (_root, studio, session) = setup_as(port, "default").await;
        let (_, rx) = start(&studio, &session, "GET oversized");
        let events = finish(rx).await;
        assert!(events.iter().any(|e|matches!(e,QueryEvent::Error{error,..}if error.code==ErrorCode::ResultLimitReached)),"{events:?}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, QueryEvent::SessionClosed { .. }))
        );
        assert_eq!(studio.usage().retained_bytes, 0);
        assert_eq!(studio.usage().result_count, 0);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn an_explicit_acl_username_never_falls_back_to_the_default_user() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut io = BufReader::new(stream);
        let auth = command(&mut io).await;
        assert_eq!(
            auth,
            vec![b"AUTH".to_vec(), b"read-only-user".to_vec(), vec![]]
        );
        io.get_mut().write_all(b"+OK\r\n").await.unwrap();
        assert_eq!(command(&mut io).await[0], b"PING");
        io.get_mut().write_all(b"+PONG\r\n").await.unwrap();
    });
    let (_root, studio, session) = setup_as(port, "read-only-user").await;
    studio.disconnect(&session.id);
    server.await.unwrap();
}

#[tokio::test]
async fn cancelling_a_sent_write_reports_unknown_instead_of_claiming_rollback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sent_tx, sent_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut io = BufReader::new(stream);
        assert_eq!(command(&mut io).await[0], b"PING");
        io.get_mut().write_all(b"+PONG\r\n").await.unwrap();
        assert_eq!(command(&mut io).await[0], b"SET");
        sent_tx.send(()).unwrap();
        let mut byte = [0];
        assert_eq!(io.read(&mut byte).await.unwrap(), 0);
    });
    let (_root, studio, session) = setup_as(port, "default").await;
    let (run, rx) = start(&studio, &session, "SET pending value");
    sent_rx.await.unwrap();
    studio.cancel(&run).await.unwrap();
    let events = finish(rx).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            QueryEvent::Finished {
                status: RunStatus::OutcomeUnknown,
                ..
            }
        )),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, QueryEvent::SessionClosed { .. }))
    );
    server.await.unwrap();
}
