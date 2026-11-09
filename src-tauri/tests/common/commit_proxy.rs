//! A local, plaintext fixture relay that loses a response only after a real COMMIT.
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

#[derive(Clone, Copy)]
pub enum Protocol {
    Postgres,
    Mysql,
}
#[derive(Clone, Copy)]
pub enum Fault {
    CommitAcknowledgement,
    CommitFatalResponse,
    PostCommitRefresh,
}
#[derive(Default)]
struct State {
    armed: AtomicBool,
    phase: AtomicU8,
    fault: AtomicU8,
    commits: AtomicUsize,
    acknowledged: AtomicBool,
    triggered: AtomicBool,
}
pub struct CommitProxy {
    pub port: u16,
    state: Arc<State>,
    task: tokio::task::JoinHandle<()>,
}
impl CommitProxy {
    pub async fn start(protocol: Protocol, upstream_port: u16) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(State::default());
        let shared = state.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((client, _)) = accepted else { break };
                        let state = shared.clone();
                        connections.spawn(async move {
                            if let Ok(server) = TcpStream::connect(("127.0.0.1", upstream_port)).await {
                                relay(client, server, protocol, state).await;
                            }
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Self { port, state, task }
    }
    pub fn arm(&self, fault: Fault) {
        self.state.fault.store(
            match fault {
                Fault::CommitAcknowledgement => 1,
                Fault::PostCommitRefresh => 2,
                Fault::CommitFatalResponse => 3,
            },
            Ordering::SeqCst,
        );
        self.state.armed.store(true, Ordering::SeqCst);
    }
    pub fn commits(&self) -> usize {
        self.state.commits.load(Ordering::SeqCst)
    }
    pub fn acknowledged(&self) -> bool {
        self.state.acknowledged.load(Ordering::SeqCst)
    }
    pub fn triggered(&self) -> bool {
        self.state.triggered.load(Ordering::SeqCst)
    }
}
impl Drop for CommitProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn frame(reader: &mut (impl AsyncRead + Unpin), protocol: Protocol) -> io::Result<Vec<u8>> {
    let mut header = vec![
        0;
        match protocol {
            Protocol::Postgres => 5,
            Protocol::Mysql => 4,
        }
    ];
    reader.read_exact(&mut header).await?;
    let length = match protocol {
        Protocol::Postgres => u32::from_be_bytes(header[1..5].try_into().unwrap()) as usize,
        Protocol::Mysql => {
            (header[0] as usize) | ((header[1] as usize) << 8) | ((header[2] as usize) << 16)
        }
    };
    let body_length = match protocol {
        Protocol::Postgres => length.checked_sub(4),
        Protocol::Mysql => Some(length),
    }
    .filter(|n| *n <= 16 * 1024 * 1024)
    .ok_or_else(|| io::Error::other("invalid fixture protocol frame"))?;
    let offset = header.len();
    header.resize(offset + body_length, 0);
    reader.read_exact(&mut header[offset..]).await?;
    Ok(header)
}
async fn startup(
    reader: &mut (impl AsyncRead + Unpin),
    writer: &mut (impl AsyncWrite + Unpin),
) -> io::Result<()> {
    let length = reader.read_u32().await?;
    if !(8..=1024 * 1024).contains(&length) {
        return Err(io::Error::other("invalid fixture startup"));
    }
    let mut bytes = vec![0; length as usize];
    bytes[..4].copy_from_slice(&length.to_be_bytes());
    reader.read_exact(&mut bytes[4..]).await?;
    writer.write_all(&bytes).await
}
async fn relay(client: TcpStream, server: TcpStream, protocol: Protocol, state: Arc<State>) {
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = server.into_split();
    let committing = AtomicBool::new(false);
    let upstream = async {
        if matches!(protocol, Protocol::Postgres) {
            startup(&mut client_read, &mut server_write).await?;
        }
        loop {
            let bytes = frame(&mut client_read, protocol).await?;
            let commit = match protocol {
                Protocol::Postgres => {
                    bytes[0] == b'Q' && bytes[5..].eq_ignore_ascii_case(b"COMMIT\0")
                }
                Protocol::Mysql => {
                    bytes.get(4) == Some(&3) && bytes[5..].eq_ignore_ascii_case(b"COMMIT")
                }
            };
            if commit {
                state.commits.fetch_add(1, Ordering::SeqCst);
                if state.armed.swap(false, Ordering::SeqCst) {
                    committing.store(true, Ordering::SeqCst);
                }
            }
            server_write.write_all(&bytes).await?;
        }
        #[allow(unreachable_code)]
        Ok::<(), io::Error>(())
    };
    let downstream = async {
        let mut commit_complete = false;
        loop {
            let bytes = frame(&mut server_read, protocol).await?;
            if state.fault.load(Ordering::SeqCst) == 2
                && state
                    .phase
                    .compare_exchange(2, 3, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                state.triggered.store(true, Ordering::SeqCst);
                return Ok::<(), io::Error>(());
            }
            if committing.load(Ordering::SeqCst) {
                let complete = match protocol {
                    Protocol::Postgres => {
                        if bytes[0] == b'C' && bytes[5..] == *b"COMMIT\0" {
                            commit_complete = true;
                        }
                        commit_complete && bytes[0] == b'Z' && bytes.get(5) == Some(&b'I')
                    }
                    Protocol::Mysql => bytes.get(4) == Some(&0),
                };
                if complete {
                    state.acknowledged.store(true, Ordering::SeqCst);
                    committing.store(false, Ordering::SeqCst);
                    if state.fault.load(Ordering::SeqCst) == 1 {
                        state.triggered.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                    if state.fault.load(Ordering::SeqCst) == 3 {
                        // Model a terminal PostgreSQL ErrorResponse without a
                        // successful COMMIT acknowledgement reaching the client.
                        let body = b"SFATAL\0VFATAL\0C08006\0Mfixture connection terminated\0\0";
                        client_write.write_u8(b'E').await?;
                        client_write.write_u32((body.len() + 4) as u32).await?;
                        client_write.write_all(body).await?;
                        state.triggered.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                    state.phase.store(2, Ordering::SeqCst);
                }
                // Withhold the whole COMMIT response, including CommandComplete.
                if matches!(state.fault.load(Ordering::SeqCst), 1 | 3) {
                    continue;
                }
            }
            client_write.write_all(&bytes).await?;
        }
    };
    tokio::select! { _ = upstream => {}, _ = downstream => {} }
}
