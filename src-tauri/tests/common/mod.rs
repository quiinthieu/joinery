use russh::{
    keys::{Algorithm, PrivateKey, PublicKey},
    server::{self, Server as _},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};
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
pub struct Fixture {
    pub port: u16,
    pub key: PrivateKey,
    pub connections: Arc<AtomicUsize>,
    pub task: tokio::task::AbortHandle,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub async fn bastion(
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
