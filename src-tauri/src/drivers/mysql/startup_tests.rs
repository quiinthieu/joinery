use super::Mysql;
use crate::domain::{ConnectionProfile, EngineKind, Environment, SafeMode, TlsMode};
use std::{io, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::{JoinHandle, JoinSet},
};

const BOUND: Duration = Duration::from_secs(3);

struct PeerOutcome {
    saw_quit: bool,
    saw_eof: bool,
    observation: String,
}

struct MysqlPeer {
    port: u16,
    first_query: oneshot::Receiver<String>,
    outcome: oneshot::Receiver<PeerOutcome>,
    task: JoinHandle<()>,
}

impl MysqlPeer {
    async fn new() -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let (query_tx, first_query) = oneshot::channel();
        let (outcome_tx, outcome) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            write_packet(&mut stream, 0, &greeting()).await.unwrap();
            let (sequence, response) = read_packet(&mut stream).await.unwrap();
            assert_eq!(sequence, 1);
            assert!(!response.is_empty());
            write_packet(&mut stream, 2, &[0x00, 0, 0, 2, 0, 0, 0])
                .await
                .unwrap();

            let (sequence, command) = read_packet(&mut stream).await.unwrap();
            assert_eq!(sequence, 0);
            assert_eq!(command.first(), Some(&0x03), "expected COM_QUERY");
            let query = String::from_utf8(command[1..].to_vec()).unwrap();

            if query == "SELECT @@wait_timeout" {
                // Put the driver's internal settings query into a pending result
                // before stalling. Raw mysql_async::Conn Drop then drains this
                // result without a bound when startup is cancelled.
                write_packet(&mut stream, 1, &[1]).await.unwrap();
                write_packet(&mut stream, 2, &column("wait_timeout"))
                    .await
                    .unwrap();
                write_packet(&mut stream, 3, &[0xfe, 0, 0, 2, 0])
                    .await
                    .unwrap();
                // Let Conn::new consume the metadata and block waiting for the
                // row before the test cancels its startup future.
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let _ = query_tx.send(query);

            let (quit, eof, observation) =
                match tokio::time::timeout(BOUND, read_packet(&mut stream)).await {
                    Ok(Ok((sequence, payload))) if payload == [0x01] => {
                        let mut byte = [0];
                        let eof = tokio::time::timeout(BOUND, stream.read(&mut byte))
                            .await
                            .is_ok_and(|result| match result {
                                Ok(0) => true,
                                Err(error) => matches!(
                                    error.kind(),
                                    io::ErrorKind::UnexpectedEof
                                        | io::ErrorKind::ConnectionReset
                                        | io::ErrorKind::ConnectionAborted
                                        | io::ErrorKind::BrokenPipe
                                ),
                                _ => false,
                            });
                        (
                            true,
                            eof,
                            format!("COM_QUIT sequence {sequence}; eof={eof}"),
                        )
                    }
                    Ok(Ok((sequence, payload))) => (
                        false,
                        false,
                        format!("packet sequence {sequence}, payload {payload:?}"),
                    ),
                    Ok(Err(error)) => {
                        let eof = matches!(
                            error.kind(),
                            io::ErrorKind::UnexpectedEof
                                | io::ErrorKind::ConnectionReset
                                | io::ErrorKind::ConnectionAborted
                                | io::ErrorKind::BrokenPipe
                        );
                        (false, eof, format!("socket closed: {error}"))
                    }
                    Err(_) => (false, false, "timed out waiting for socket close".into()),
                };
            let _ = outcome_tx.send(PeerOutcome {
                saw_quit: quit,
                saw_eof: eof,
                observation,
            });
        });
        Self {
            port,
            first_query,
            outcome,
            task,
        }
    }
}

impl Drop for MysqlPeer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn profile(port: u16) -> ConnectionProfile {
    ConnectionProfile {
        id: "mysql-startup-cancel".into(),
        name: "MySQL startup cancellation fixture".into(),
        group_id: "local".into(),
        engine: EngineKind::Mysql,
        host: "127.0.0.1".into(),
        port,
        database: "joinery".into(),
        username: "joinery".into(),
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

fn greeting() -> Vec<u8> {
    const CAPABILITIES: u32 = 0x0008_0000 | 0x8000 | 0x2000 | 0x0200 | 0x000f;
    let mut payload = vec![0x0a];
    payload.extend_from_slice(b"8.0.36-Joinery-test\0");
    payload.extend_from_slice(&1_u32.to_le_bytes());
    payload.extend_from_slice(b"abcdefgh");
    payload.push(0);
    payload.extend_from_slice(&(CAPABILITIES as u16).to_le_bytes());
    payload.push(45);
    payload.extend_from_slice(&2_u16.to_le_bytes());
    payload.extend_from_slice(&((CAPABILITIES >> 16) as u16).to_le_bytes());
    payload.push(21);
    payload.extend_from_slice(&[0; 10]);
    payload.extend_from_slice(b"ijklmnopqrst\0");
    payload.extend_from_slice(b"mysql_native_password\0");
    payload
}

fn column(name: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    for value in [
        b"def".as_slice(),
        b"",
        b"",
        b"",
        name.as_bytes(),
        name.as_bytes(),
    ] {
        payload.push(value.len() as u8);
        payload.extend_from_slice(value);
    }
    payload.push(12);
    payload.extend_from_slice(&45_u16.to_le_bytes());
    payload.extend_from_slice(&64_u32.to_le_bytes());
    payload.push(0x08); // MYSQL_TYPE_LONGLONG
    payload.extend_from_slice(&0_u16.to_le_bytes());
    payload.push(0);
    payload.extend_from_slice(&0_u16.to_le_bytes());
    payload
}

async fn read_packet(stream: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0; 4];
    stream.read_exact(&mut header).await?;
    let length =
        usize::from(header[0]) | (usize::from(header[1]) << 8) | (usize::from(header[2]) << 16);
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).await?;
    Ok((header[3], payload))
}

async fn write_packet(stream: &mut TcpStream, sequence: u8, payload: &[u8]) -> io::Result<()> {
    let length = payload.len();
    let header = [
        length as u8,
        (length >> 8) as u8,
        (length >> 16) as u8,
        sequence,
    ];
    stream.write_all(&header).await?;
    stream.write_all(payload).await
}

#[tokio::test]
async fn cancelled_startup_skips_internal_settings_query_and_closes_socket() {
    let mut peer = MysqlPeer::new().await;
    let profile = profile(peer.port);
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut startup = JoinSet::new();
    startup.spawn(async move { Mysql::open(&profile, None, &session_id, None).await });

    let first_query = tokio::time::timeout(BOUND, &mut peer.first_query)
        .await
        .expect("MySQL startup should send its first query")
        .expect("wire peer should report the query");
    startup.abort_all();
    let cancelled = tokio::time::timeout(BOUND, startup.join_next())
        .await
        .expect("cancelled startup should unwind")
        .expect("startup task should be present");
    assert!(cancelled.is_err_and(|error| error.is_cancelled()));

    let outcome = tokio::time::timeout(BOUND * 2, &mut peer.outcome)
        .await
        .expect("cancelled startup should close the wire peer")
        .expect("wire peer should report disconnect");
    assert!(
        outcome.saw_eof,
        "client socket should close on cancellation (COM_QUIT observed: {}): {}",
        outcome.saw_quit, outcome.observation
    );
    assert_eq!(first_query, "SELECT VERSION()");
}
