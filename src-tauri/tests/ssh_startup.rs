use joinery_core::{domain::*, ssh::SshManager};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

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
                .write_all(b"SSH-2.0-startup-fixture\r\n")
                .await
                .unwrap();
            let packet_size = stream.read_u32().await.unwrap() as usize;
            assert!((2..=256 * 1024).contains(&packet_size));
            let mut packet = vec![0; packet_size];
            stream.read_exact(&mut packet).await.unwrap();
            assert_eq!(packet[1], 20, "the client must have started key exchange");
            started_tx.send(()).unwrap();
            // Keep the peer alive without completing key exchange. A closed
            // sender alone cannot stop russh's session while KEX is active.
            let mut remainder = Vec::new();
            match stream.read_to_end(&mut remainder).await {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
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
            .expect("SSH did not reach key exchange")
            .unwrap();
    }

    async fn closed(&mut self) {
        tokio::time::timeout(BOUND, &mut self.closed)
            .await
            .expect("cancelled SSH startup retained its peer socket")
            .unwrap();
        (&mut self.task).await.unwrap();
    }
}

impl Drop for StalledKeyExchange {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn profile(port: u16) -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "SSH startup fixture".into(),
        group_id: "local".into(),
        engine: EngineKind::Redis,
        host: "database.fixture.test".into(),
        port: 6379,
        database: "0".into(),
        username: "fixture".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::ReadOnly,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: Some(SshConfig {
            host: "127.0.0.1".into(),
            port,
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

#[tokio::test]
async fn cancelled_acquire_retires_socket_during_key_exchange() {
    let mut peer = StalledKeyExchange::new().await;
    let profile = profile(peer.port);
    let connecting = tokio::spawn(async move {
        SshManager::default()
            .acquire(&profile, Some("not-reached-before-KEX".into()), None, None)
            .await
    });
    peer.started().await;
    connecting.abort();
    assert!(matches!(connecting.await, Err(error) if error.is_cancelled()));
    peer.closed().await;
}

#[tokio::test]
async fn timed_out_acquire_retires_socket_during_key_exchange() {
    let mut peer = StalledKeyExchange::new().await;
    let profile = profile(peer.port);
    let connecting = tokio::spawn(async move {
        SshManager::default()
            .acquire(&profile, Some("not-reached-before-KEX".into()), None, None)
            .await
    });
    peer.started().await;
    let outcome = tokio::time::timeout(Duration::from_secs(12), connecting)
        .await
        .expect("SSH startup deadline did not complete")
        .unwrap();
    assert!(matches!(outcome, Err(error) if error.code == ErrorCode::ConnectionFailed));
    peer.closed().await;
}

#[tokio::test]
async fn cancelled_probe_retires_socket_during_key_exchange() {
    let mut peer = StalledKeyExchange::new().await;
    let ssh = profile(peer.port).ssh.unwrap();
    let probing = tokio::spawn(async move { SshManager::default().probe(&ssh, None).await });
    peer.started().await;
    probing.abort();
    assert!(probing.await.unwrap_err().is_cancelled());
    peer.closed().await;
}
