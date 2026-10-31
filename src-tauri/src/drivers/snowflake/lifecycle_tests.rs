use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::{JoinHandle, JoinSet},
};

struct Request {
    headers: String,
    body: String,
    reply: oneshot::Sender<(u16, Value)>,
}

struct Server {
    requests: mpsc::UnboundedReceiver<Request>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn next(&mut self) -> Request {
        tokio::time::timeout(Duration::from_secs(3), self.requests.recv())
            .await
            .expect("expected synthetic HTTP request")
            .unwrap()
    }
}

impl Request {
    fn respond(self, status: u16, body: Value) {
        self.reply.send((status, body)).unwrap();
    }
}

// Loopback HTTP only, with synthetic credentials. Concurrent handlers let tests
// hold a request open while disconnecting or issuing a cancellation.
async fn fixture(refresh: bool, expired: bool) -> (Arc<Snowflake>, Server) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, requests) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted.unwrap();
                    let tx = tx.clone();
                    handlers.spawn(async move {
                        let mut bytes = Vec::new();
                        let mut chunk = [0u8; 4096];
                        let (end, length) = loop {
                            let n = stream.read(&mut chunk).await.unwrap();
                            if n == 0 { return; }
                            bytes.extend_from_slice(&chunk[..n]);
                            assert!(bytes.len() < 64 * 1024);
                            if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                                let headers = String::from_utf8_lossy(&bytes[..end]);
                                let length = headers.lines().find_map(|line| {
                                    line.to_ascii_lowercase().strip_prefix("content-length: ")
                                        .and_then(|s| s.parse::<usize>().ok())
                                }).unwrap_or(0);
                                if bytes.len() >= end + 4 + length { break (end, length); }
                            }
                        };
                        let (reply, response) = oneshot::channel();
                        tx.send(Request {
                            headers: String::from_utf8(bytes[..end].to_vec()).unwrap(),
                            body: String::from_utf8(bytes[end + 4..end + 4 + length].to_vec()).unwrap(),
                            reply,
                        }).unwrap();
                        if let Ok((status, body)) = response.await {
                            let body = body.to_string();
                            let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                        }
                    });
                }
                Some(result) = handlers.join_next(), if !handlers.is_empty() => { result.unwrap(); }
            }
        }
    });
    let driver = Arc::new(Snowflake {
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        base: Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
        database: "DB".into(),
        schema: Mutex::new("PUBLIC".into()),
        role: "READ_ONLY".into(),
        warehouse: "WH".into(),
        query_tag: "Joinery/lifecycle-test".into(),
        auth: AsyncMutex::new(AuthState {
            tokens: Tokens {
                access: Zeroizing::new("synthetic-access".into()),
                refresh: refresh.then(|| Zeroizing::new("synthetic-refresh".into())),
                expires_in: 600,
            },
            expires_at: Instant::now() + Duration::from_secs(if expired { 0 } else { 600 }),
        }),
        metadata_lane: AsyncMutex::new(()),
        active_handle: Mutex::new(None),
        cancel_requested: CancellationToken::new(),
        closed: CancellationToken::new(),
    });
    (driver, Server { requests, task })
}

fn execute(driver: &Arc<Snowflake>) -> JoinHandle<AppResult<u64>> {
    let driver = driver.clone();
    tokio::spawn(async move {
        driver
            .execute(
                "SELECT 1",
                StatementKind::Read,
                &mut super::tests::Sink::default(),
            )
            .await
    })
}

fn result() -> Value {
    json!({"statementHandle":uuid::Uuid::new_v4().to_string(),
        "resultSetMetaData":{"rowType":[{"name":"N","type":"FIXED","scale":0}],"partitionInfo":[{}]},
        "data":[["1"]]})
}

async fn finished<T>(task: JoinHandle<AppResult<T>>) -> AppResult<T> {
    tokio::time::timeout(Duration::from_millis(800), task)
        .await
        .expect("operation must finish without waiting for the HTTP timeout")
        .unwrap()
}

fn request(driver: &Arc<Snowflake>) -> JoinHandle<AppResult<(StatusCode, Value)>> {
    let driver = driver.clone();
    tokio::spawn(async move {
        driver
            .request(Method::GET, "api/v2/statements/test", None)
            .await
    })
}

#[tokio::test]
async fn cancellation_before_execute_never_submits_sql() {
    let (driver, mut server) = fixture(false, false).await;
    driver.cancel().await.unwrap();
    let task = execute(&driver);
    tokio::select! {
        request = server.next() => {
            request.respond(200, result());
            panic!("SQL submitted after cancellation");
        }
        outcome = finished(task) => assert_eq!(outcome.unwrap_err().code, ErrorCode::Cancelled),
    }
}

#[tokio::test]
async fn disconnect_interrupts_inflight_http() {
    let (driver, mut server) = fixture(false, false).await;
    let task = execute(&driver);
    let _held_request = server.next().await;
    driver.close();
    assert_eq!(
        finished(task).await.unwrap_err().code,
        ErrorCode::ConnectionClosed
    );
}

#[tokio::test]
async fn expired_sign_in_invalidates_session_without_submitting_sql() {
    let (driver, _server) = fixture(false, true).await;
    assert_eq!(
        finished(execute(&driver)).await.unwrap_err().code,
        ErrorCode::ConnectionClosed
    );
    assert!(driver.is_closed(), "expired session must require reconnect");
    assert_eq!(
        driver.health().await.unwrap_err().code,
        ErrorCode::ConnectionClosed
    );
}

#[tokio::test]
async fn unrefreshable_access_token_remains_usable_until_expiry() {
    let (driver, mut server) = fixture(false, false).await;
    driver.auth.lock().await.expires_at = Instant::now() + Duration::from_secs(20);
    let task = execute(&driver);
    let sql = server.next().await;
    assert!(sql.headers.starts_with("POST /api/v2/statements?"));
    sql.respond(200, result());
    assert_eq!(finished(task).await.unwrap(), 1);
    assert!(!driver.is_closed());
}

#[tokio::test]
async fn disconnect_interrupts_refresh_and_auth_lock_waiters() {
    let (driver, mut server) = fixture(true, true).await;
    let first = request(&driver);
    let held = server.next().await;
    assert!(held.headers.starts_with("POST /oauth/token-request "));
    let second = request(&driver);
    driver.close();
    for task in [first, second] {
        assert_eq!(
            finished(task).await.unwrap_err().code,
            ErrorCode::ConnectionClosed
        );
    }
    assert!(driver.is_closed());
}

#[tokio::test]
async fn concurrent_requests_share_refresh_and_retain_rotated_credentials() {
    let (driver, mut server) = fixture(true, true).await;
    let first = request(&driver);
    let second = request(&driver);
    let refresh = server.next().await;
    assert!(refresh.headers.starts_with("POST /oauth/token-request "));
    assert!(refresh.body.contains("refresh_token=synthetic-refresh"));
    refresh.respond(200, json!({"access_token":"rotated-access", "refresh_token":"rotated-refresh", "expires_in":600, "token_type":"Bearer"}));
    for _ in 0..2 {
        let sql = server.next().await;
        assert!(
            sql.headers
                .to_ascii_lowercase()
                .contains("authorization: bearer rotated-access")
        );
        sql.respond(200, json!({}));
    }
    finished(first).await.unwrap();
    finished(second).await.unwrap();
    driver.auth.lock().await.expires_at = Instant::now();
    let third = request(&driver);
    let refresh = server.next().await;
    assert!(refresh.body.contains("refresh_token=rotated-refresh"));
    // Providers may omit an unchanged refresh token on later refreshes.
    refresh.respond(
        200,
        json!({"access_token":"third-access", "expires_in":600, "token_type":"Bearer"}),
    );
    server.next().await.respond(200, json!({}));
    finished(third).await.unwrap();
    assert_eq!(
        driver
            .auth
            .lock()
            .await
            .tokens
            .refresh
            .as_deref()
            .map(String::as_str),
        Some("rotated-refresh")
    );
}

#[tokio::test]
async fn short_lived_access_token_is_not_immediately_refreshed_again() {
    let (driver, mut server) = fixture(true, true).await;
    let first = request(&driver);
    server.next().await.respond(
        200,
        json!({"access_token":"brief-access", "expires_in":10, "token_type":"Bearer"}),
    );
    server.next().await.respond(200, json!({}));
    finished(first).await.unwrap();
    let second = request(&driver);
    let sql = server.next().await;
    assert!(
        sql.headers.starts_with("GET /api/"),
        "usable short-lived token was refreshed again"
    );
    sql.respond(200, json!({}));
    finished(second).await.unwrap();
}

#[tokio::test]
async fn rejected_refresh_retires_session_and_never_replays_sql() {
    let (driver, mut server) = fixture(true, true).await;
    let task = execute(&driver);
    server.next().await.respond(
        400,
        json!({"error":"invalid_grant", "error_description":"synthetic-secret-must-not-surface"}),
    );
    let error = finished(task).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::ConnectionClosed);
    assert!(!error.message.contains("synthetic-secret"));
    assert!(driver.is_closed());
    assert_eq!(
        finished(execute(&driver)).await.unwrap_err().code,
        ErrorCode::ConnectionClosed
    );
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn transient_refresh_failure_preserves_session_for_explicit_retry() {
    let (driver, mut server) = fixture(true, true).await;
    let task = execute(&driver);
    server
        .next()
        .await
        .respond(503, json!({"error":"temporarily_unavailable"}));
    assert_eq!(
        finished(task).await.unwrap_err().code,
        ErrorCode::ConnectionFailed
    );
    assert!(!driver.is_closed());
    assert!(server.requests.try_recv().is_err());
    let retry = execute(&driver);
    let refresh = server.next().await;
    assert!(refresh.body.contains("refresh_token=synthetic-refresh"));
    refresh.respond(
        200,
        json!({"access_token":"retry-access", "expires_in":600, "token_type":"Bearer"}),
    );
    server.next().await.respond(200, result());
    assert_eq!(finished(retry).await.unwrap(), 1);
}

#[tokio::test]
async fn rejected_access_token_requires_reconnect_without_automatic_replay() {
    let (driver, mut server) = fixture(true, false).await;
    let task = execute(&driver);
    server
        .next()
        .await
        .respond(401, json!({"message":"synthetic-secret-must-not-surface"}));
    let error = finished(task).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::ConnectionClosed);
    assert!(!error.message.contains("synthetic-secret"));
    assert!(driver.is_closed());
    assert_eq!(
        finished(execute(&driver)).await.unwrap_err().code,
        ErrorCode::ConnectionClosed
    );
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn cancellation_during_submission_cancels_the_returned_handle() {
    let (driver, mut server) = fixture(false, false).await;
    let task = execute(&driver);
    let submitted = server.next().await;
    driver.cancel().await.unwrap();
    let handle = uuid::Uuid::new_v4().to_string();
    submitted.respond(202, json!({"statementHandle":handle}));
    let cancel = server.next().await;
    assert!(
        cancel
            .headers
            .starts_with(&format!("POST /api/v2/statements/{handle}/cancel "))
    );
    cancel.respond(200, json!({}));
    assert_eq!(finished(task).await.unwrap_err().code, ErrorCode::Cancelled);
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn cancellation_during_refresh_never_submits_sql() {
    let (driver, mut server) = fixture(true, true).await;
    let task = execute(&driver);
    let refresh = server.next().await;
    driver.cancel().await.unwrap();
    refresh.respond(
        200,
        json!({"access_token":"fresh-access", "expires_in":600, "token_type":"Bearer"}),
    );
    assert_eq!(finished(task).await.unwrap_err().code, ErrorCode::Cancelled);
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn cancellation_interrupts_partition_wait_and_targets_the_owned_handle() {
    let (driver, mut server) = fixture(false, false).await;
    let task = execute(&driver);
    let mut first = result();
    first["resultSetMetaData"]["partitionInfo"] = json!([{}, {}]);
    let handle = first["statementHandle"].as_str().unwrap().to_string();
    server.next().await.respond(200, first);
    let held_partition = server.next().await;
    assert!(held_partition.headers.contains("?partition=1"));
    let cancelling = {
        let driver = driver.clone();
        tokio::spawn(async move { driver.cancel().await })
    };
    let cancel = server.next().await;
    assert!(
        cancel
            .headers
            .starts_with(&format!("POST /api/v2/statements/{handle}/cancel "))
    );
    cancel.respond(200, json!({}));
    finished(cancelling).await.unwrap();
    assert_eq!(finished(task).await.unwrap_err().code, ErrorCode::Cancelled);
    drop(held_partition);
    assert!(server.requests.try_recv().is_err());
}
