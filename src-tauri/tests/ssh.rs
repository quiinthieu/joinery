use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets, ssh::SshManager};
use russh::{
    keys::{Algorithm, PrivateKey, PublicKey, ssh_key::LineEnding},
    server::{self, Server as _},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use zeroize::Zeroizing;

#[derive(Clone)]
struct Bastion {
    identity: PublicKey,
    target_port: u16,
    connections: Arc<AtomicUsize>,
}
impl server::Server for Bastion {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.connections.fetch_add(1, Ordering::SeqCst);
        self.clone()
    }
}
impl server::Handler for Bastion {
    type Error = russh::Error;
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "fixture" && password == "ssh-test-only" {
            server::Auth::Accept
        } else {
            server::Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            }
        })
    }
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "fixture" && key == &self.identity {
            server::Auth::Accept
        } else {
            server::Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            }
        })
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: russh::Channel<server::Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if !["database.fixture.test", "wrong.fixture.test"].contains(&host)
            || port != self.target_port as u32
        {
            return Ok(());
        }
        let mut socket = TcpStream::connect(("127.0.0.1", self.target_port)).await?;
        reply.accept().await;
        tokio::spawn(async move {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut socket).await;
        });
        Ok(())
    }
}
struct Fixture {
    port: u16,
    key: PrivateKey,
    connections: Arc<AtomicUsize>,
    task: tokio::task::AbortHandle,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn bastion(
    target_port: u16,
    identity: &PrivateKey,
    host_key: Option<PrivateKey>,
    port: u16,
) -> Fixture {
    let key = host_key
        .unwrap_or_else(|| PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap());
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(server::Config {
        keys: vec![key.clone()],
        auth_rejection_time: Duration::from_millis(1),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let connections = Arc::new(AtomicUsize::new(0));
    let mut server = Bastion {
        identity: identity.public_key().clone(),
        target_port,
        connections: connections.clone(),
    };
    let task = tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    Fixture {
        port,
        key,
        connections,
        task: task.abort_handle(),
    }
}
fn profile(
    ssh_port: u16,
    target_port: u16,
    authentication: SshAuthentication,
) -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "SSH fixture".into(),
        group_id: "local".into(),
        engine: EngineKind::Postgres,
        host: "database.fixture.test".into(),
        port: target_port,
        database: "joinery".into(),
        username: "joinery".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::Silent,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: Some(SshConfig {
            host: "127.0.0.1".into(),
            port: ssh_port,
            username: "fixture".into(),
            authentication,
            private_key_path: None,
            agent_socket_path: None,
        }),
        remember_password: false,
        store_query_text: false,
    }
}
async fn echo(port: u16) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(b"round trip").await.unwrap();
    let mut bytes = [0; 10];
    stream.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"round trip");
}
async fn echo_target() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (mut read, mut write) = stream.into_split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    (port, task)
}

#[tokio::test]
async fn unknown_hosts_require_trust_and_shared_tunnels_release_their_ports() {
    let (target, echo_task) = echo_target().await;
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let server = bastion(target, &identity, None, 0).await;
    let manager = SshManager::default();
    let profile = profile(server.port, target, SshAuthentication::Password);
    assert!(
        matches!(manager.acquire(&profile, None, None, None).await, Err(e) if e.code == ErrorCode::SshHostKeyUnknown)
    );
    let host = manager
        .probe(profile.ssh.as_ref().unwrap(), None)
        .await
        .unwrap();
    let challenge = host.challenge_id.as_ref().unwrap();
    let (_, line) = manager.accept(challenge, &host.fingerprint).unwrap();
    assert!(manager.accept(challenge, &host.fingerprint).is_err());
    let first = manager
        .acquire(
            &profile,
            Some(line.clone()),
            Some(Zeroizing::new("ssh-test-only".into())),
            None,
        )
        .await
        .unwrap();
    let second = manager
        .acquire(&profile, Some(line), None, None)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(server.connections.load(Ordering::SeqCst), 2); // unauthenticated probe + one shared session
    let port = first.port;
    echo(port).await;
    drop(first);
    echo(port).await;
    drop(second);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    echo_task.abort();
}

#[tokio::test]
async fn changed_host_keys_are_rejected_before_authentication() {
    let (target, echo_task) = echo_target().await;
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let first = bastion(target, &identity, None, 0).await;
    let manager = SshManager::default();
    let profile = profile(first.port, target, SshAuthentication::Password);
    let known = first.key.public_key().to_openssh().unwrap();
    let port = first.port;
    first.task.abort();
    drop(first);
    tokio::time::sleep(Duration::from_millis(30)).await;
    let _replacement = bastion(target, &identity, None, port).await;
    let seen = manager
        .probe(profile.ssh.as_ref().unwrap(), Some(known.clone()))
        .await
        .unwrap();
    assert_ne!(Some(seen.fingerprint), seen.previous_fingerprint);
    assert!(seen.challenge_id.is_none());
    assert!(
        matches!(manager.acquire(&profile, Some(known), Some(Zeroizing::new("ssh-test-only".into())), None).await, Err(e) if e.code == ErrorCode::SshHostKeyChanged)
    );
    echo_task.abort();
}

#[tokio::test]
async fn encrypted_private_keys_authenticate_without_persisting_the_passphrase() {
    let (target, echo_task) = echo_target().await;
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let server = bastion(target, &identity, None, 0).await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity");
    identity
        .encrypt(&mut rand::rng(), "fixture-key-passphrase")
        .unwrap()
        .write_openssh_file(&path, LineEnding::LF)
        .unwrap();
    let mut profile = profile(server.port, target, SshAuthentication::PrivateKey);
    profile.ssh.as_mut().unwrap().private_key_path = Some(path.to_string_lossy().into());
    let manager = SshManager::default();
    let known = server.key.public_key().to_openssh().unwrap();
    let tunnel = manager
        .acquire(
            &profile,
            Some(known),
            None,
            Some(Zeroizing::new("fixture-key-passphrase".into())),
        )
        .await
        .unwrap();
    echo(tunnel.port).await;
    echo_task.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn a_task_owned_openssh_agent_can_sign_without_agent_forwarding() {
    struct Agent(std::process::Child);
    impl Drop for Agent {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let (target, echo_task) = echo_target().await;
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let server = bastion(target, &identity, None, 0).await;
    let directory = tempfile::tempdir().unwrap();
    let key_path = directory.path().join("identity");
    let socket = directory.path().join("agent.sock");
    identity
        .write_openssh_file(&key_path, LineEnding::LF)
        .unwrap();
    let _agent = Agent(
        std::process::Command::new("ssh-agent")
            .args(["-D", "-a"])
            .arg(&socket)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        std::process::Command::new("ssh-add")
            .arg(&key_path)
            .env("SSH_AUTH_SOCK", &socket)
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut profile = profile(server.port, target, SshAuthentication::Agent);
    profile.ssh.as_mut().unwrap().agent_socket_path = Some(socket.to_string_lossy().into());
    let manager = SshManager::default();
    let tunnel = manager
        .acquire(
            &profile,
            Some(server.key.public_key().to_openssh().unwrap()),
            None,
            None,
        )
        .await
        .unwrap();
    echo(tunnel.port).await;
    echo_task.abort();
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn postgres_queries_and_cancel_requests_cross_the_bastion() {
    let target = std::env::var("JOINERY_TEST_PORT")
        .unwrap_or_else(|_| "55432".into())
        .parse()
        .unwrap();
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let server = bastion(target, &identity, None, 0).await;
    let directory = tempfile::tempdir().unwrap();
    let studio =
        Studio::with_secrets(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
    let saved = studio
        .save_profile(
            profile(server.port, target, SshAuthentication::Password),
            None,
        )
        .unwrap();
    let key = studio.probe_ssh(&saved.id).await.unwrap();
    studio
        .trust_ssh_key(key.challenge_id.as_ref().unwrap(), &key.fingerprint)
        .unwrap();
    let session = studio
        .connect(
            &saved.id,
            ConnectionSecrets {
                password: Some("joinery-local-only".into()),
                ssh_password: Some("ssh-test-only".into()),
                key_passphrase: None,
            },
        )
        .await
        .unwrap();
    let plan = studio
        .prepare_execution(&session.id, "SELECT 9223372036854775807::bigint")
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    studio
        .execute(
            &plan.id,
            None,
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .unwrap();
    let mut id = None;
    while let Some(event) = rx.recv().await {
        match event {
            QueryEvent::Result { result } => id = Some(result.id),
            QueryEvent::Finished { status, .. } => {
                assert_eq!(status, RunStatus::Completed);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(
        studio.results.row(&id.unwrap(), 0).unwrap()[0].as_deref(),
        Some("9223372036854775807")
    );
    let plan = studio
        .prepare_execution(&session.id, "SELECT pg_sleep(10)")
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let run_id = studio
        .execute(
            &plan.id,
            None,
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .unwrap();
    while !matches!(
        rx.recv().await.unwrap(),
        QueryEvent::StatementStarted { .. }
    ) {}
    tokio::time::sleep(Duration::from_millis(100)).await;
    studio.cancel(&run_id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(event) = rx.recv().await {
            if let QueryEvent::Finished { status, .. } = event {
                assert_eq!(status, RunStatus::Cancelled);
                break;
            }
        }
    })
    .await
    .unwrap();
    studio.shutdown();
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL service in compose.yaml"]
async fn tls_through_ssh_verifies_the_database_hostname_instead_of_loopback() {
    let target = std::env::var("JOINERY_TEST_PORT")
        .unwrap_or_else(|_| "55432".into())
        .parse::<u16>()
        .unwrap();
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
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let tls_port = listener.local_addr().unwrap().port();
    let names = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = names.clone();
    let proxy = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = connections.join_next(), if !connections.is_empty() => {},
                socket = listener.accept() => {
                    let (mut socket, _) = socket.unwrap(); let acceptor = acceptor.clone(); let captured = captured.clone();
                    connections.spawn(async move {
                        let mut header = [0; 8]; if socket.read_exact(&mut header).await.is_err() { return; }
                        if header != [0,0,0,8,4,210,22,47] { return; } // PostgreSQL SSLRequest
                        if socket.write_all(b"S").await.is_err() { return; }
                        if let Ok(mut tls) = acceptor.accept(socket).await {
                            captured.lock().unwrap().push(tls.get_ref().1.server_name().unwrap_or("").to_string());
                            let mut upstream = TcpStream::connect(("127.0.0.1", target)).await.unwrap();
                            let _ = tokio::io::copy_bidirectional(&mut tls, &mut upstream).await;
                        }
                    });
                }
            }
        }
    });
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let server = bastion(tls_port, &identity, None, 0).await;
    let directory = tempfile::tempdir().unwrap();
    let ca = directory.path().join("root.pem");
    std::fs::write(&ca, certificate.cert.pem()).unwrap();
    let studio =
        Studio::with_secrets(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
    let mut connection = profile(server.port, tls_port, SshAuthentication::Password);
    connection.tls_mode = TlsMode::VerifyFull;
    connection.ca_cert_path = Some(ca.to_string_lossy().into());
    let saved = studio.save_profile(connection.clone(), None).unwrap();
    let key = studio.probe_ssh(&saved.id).await.unwrap();
    studio
        .trust_ssh_key(key.challenge_id.as_ref().unwrap(), &key.fingerprint)
        .unwrap();
    let credentials = || ConnectionSecrets {
        password: Some("joinery-local-only".into()),
        ssh_password: Some("ssh-test-only".into()),
        key_passphrase: None,
    };
    let session = studio.connect(&saved.id, credentials()).await.unwrap();
    assert!(session.encrypted);
    assert!(
        names
            .lock()
            .unwrap()
            .iter()
            .any(|name| name == "database.fixture.test")
    );
    studio.disconnect(&session.id);
    connection.id = new_id();
    connection.host = "wrong.fixture.test".into();
    let wrong = studio.save_profile(connection, None).unwrap();
    assert_eq!(
        studio
            .connect(&wrong.id, credentials())
            .await
            .unwrap_err()
            .code,
        ErrorCode::ConnectionFailed
    );
    assert!(
        !names
            .lock()
            .unwrap()
            .iter()
            .any(|name| name == "wrong.fixture.test")
    );
    studio.shutdown();
    proxy.abort();
}
