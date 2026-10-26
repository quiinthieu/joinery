use super::*;
use flate2::{Compression, write::GzEncoder};
use std::io::Write;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn query_with_partition(body: Vec<u8>) -> (AppResult<u64>, super::tests::Sink) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = uuid::Uuid::new_v4().to_string();
    let server = tokio::spawn(async move {
        for step in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let (encoding, payload) = if step == 0 {
                let rows: Vec<_> = (0..85).map(|n| vec![n.to_string()]).collect();
                (
                    "",
                    json!({"statementHandle":handle,"resultSetMetaData":{
                        "rowType":[{"name":"N","type":"FIXED","scale":0}],
                        "partitionInfo":[{"rowCount":85},{"rowCount":15}]},"data":rows
                    })
                    .to_string()
                    .into_bytes(),
                )
            } else {
                assert!(
                    String::from_utf8_lossy(&bytes)
                        .starts_with(&format!("GET /api/v2/statements/{handle}?partition=1 "))
                );
                ("Content-Encoding: gzip\r\n", body.clone())
            };
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{encoding}Content-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            // A bounded decoder may close the response before consuming it all.
            let _ = stream.write_all(&payload).await;
        }
    });
    let driver = Snowflake {
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        base: Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
        database: "DB".into(),
        schema: "PUBLIC".into(),
        role: "READ_ONLY".into(),
        warehouse: "WH".into(),
        query_tag: "Joinery/compression-test".into(),
        auth: AsyncMutex::new(AuthState {
            tokens: Tokens {
                access: Zeroizing::new("synthetic-access".into()),
                refresh: None,
                expires_in: 600,
            },
            expires_at: Instant::now() + Duration::from_secs(600),
        }),
        metadata_lane: AsyncMutex::new(()),
        active_handle: Mutex::new(None),
        cancel_requested: CancellationToken::new(),
        closed: CancellationToken::new(),
    };
    let mut sink = super::tests::Sink::default();
    let result = driver
        .execute("SELECT 1", StatementKind::Read, &mut sink)
        .await;
    server.await.unwrap();
    (result, sink)
}

fn gzip(body: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(body).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test]
async fn compressed_partition_completes_all_100_rows() {
    let mut rows: Vec<Vec<Option<String>>> = (85..100).map(|n| vec![Some(n.to_string())]).collect();
    rows[13][0] = None;
    rows[14][0] = Some("123456789012345678901234567890.000000001".into());
    let (result, sink) =
        query_with_partition(gzip(json!({"data":rows}).to_string().as_bytes())).await;
    assert_eq!(result.unwrap(), 100);
    assert_eq!(sink.rows.len(), 100);
    assert_eq!(&sink.rows[85..], rows.as_slice());
    assert!(sink.completed);
}

#[tokio::test]
async fn compressed_partition_enforces_decompressed_response_limit() {
    let body = gzip(&vec![b' '; MAX_RESPONSE_BYTES + 1]);
    assert!(body.len() < MAX_RESPONSE_BYTES);
    let (result, sink) = query_with_partition(body).await;
    assert_eq!(result.unwrap_err().code, ErrorCode::ResultLimitReached);
    assert_eq!(sink.rows.len(), 85);
    assert!(!sink.completed);
}

#[tokio::test]
async fn corrupt_compressed_partition_does_not_report_completion() {
    let mut body = gzip(b"{\"data\":[[\"secret-synthetic-value\"]]}");
    let checksum = body.len() - 8;
    body[checksum] ^= 0xff;
    let (result, sink) = query_with_partition(body).await;
    let error = result.unwrap_err();
    assert_eq!(error.code, ErrorCode::ConnectionFailed);
    assert_eq!(error.message, "Snowflake response was interrupted");
    assert_eq!(sink.rows.len(), 85);
    assert!(!sink.completed);
}
