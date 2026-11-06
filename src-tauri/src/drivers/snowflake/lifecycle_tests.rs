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
        database: Mutex::new("DB".into()),
        schema: Mutex::new("PUBLIC".into()),
        role: Mutex::new("READ_ONLY".into()),
        warehouse: Mutex::new("WH".into()),
        query_tag: "Joinery/lifecycle-test".into(),
        auth: AsyncMutex::new(AuthState {
            tokens: Tokens {
                access: Zeroizing::new("synthetic-access".into()),
                refresh: refresh.then(|| Zeroizing::new("synthetic-refresh".into())),
                expires_in: 600,
            },
            expires_at: Instant::now() + Duration::from_secs(if expired { 0 } else { 600 }),
        }),
        auth_method: SnowflakeAuthMethod::OAuthBrowser,
        key_pair: None,
        username: "TEST_USER".into(),
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

#[tokio::test]
async fn estimated_plan_submits_only_the_wrapper_and_preserves_raw_json() {
    let (driver, mut server) = fixture(false, false).await;
    let sql = crate::query_plan::explain_sql(
        EngineKind::Snowflake,
        "/* é */ SELECT ';' AS note",
        false,
        None,
    )
    .unwrap();
    let expected = sql.clone();
    let task = tokio::spawn(async move {
        let mut sink = super::tests::Sink::default();
        driver.execute(&sql, StatementKind::Read, &mut sink).await?;
        Ok(sink)
    });
    let request = server.next().await;
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(body["statement"], expected);
    assert_eq!(body["warehouse"], "WH");
    assert_eq!(body["role"], "READ_ONLY");
    assert_eq!(body["database"], "DB");
    assert_eq!(body["schema"], "PUBLIC");
    assert_eq!(body["parameters"]["QUERY_TAG"], "Joinery/lifecycle-test");
    let raw = "{\"GlobalStats\":{\"bytesAssigned\":9007199254740993123},\"Operations\":[[{\"id\":0,\"operation\":\"Result\"}]]}";
    request.respond(
        200,
        json!({"statementHandle":uuid::Uuid::new_v4().to_string(),
        "resultSetMetaData":{"rowType":[{"name":"content","type":"TEXT"}],"partitionInfo":[{}]},
        "data":[[raw]]}),
    );
    let sink = finished(task).await.unwrap();
    assert_eq!(sink.rows, vec![vec![Some(raw.into())]]);
    assert!(sink.completed);
    assert!(
        server.requests.try_recv().is_err(),
        "never execute the underlying query separately"
    );
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

#[tokio::test]
async fn test_switch_session_context_no_deadlock() {
    let (driver, mut server) = fixture(false, false).await;
    let handle = uuid::Uuid::new_v4().to_string();

    tokio::spawn(async move {
        // 1. Respond to USE WAREHOUSE statement
        let req1 = server.next().await;
        req1.respond(
            200,
            json!({
                "statementHandle": handle,
                "resultSetMetaData": {
                    "rowType": [{"name": "status", "type": "TEXT"}],
                    "partitionInfo": [{}]
                },
                "data": [["Statement executed successfully."]]
            }),
        );

        // 2. Respond to list_context_dimensions (SHOW WAREHOUSES, SHOW ROLES, SHOW DATABASES, SHOW SCHEMAS)
        for _ in 0..4 {
            let req = server.next().await;
            req.respond(
                200,
                json!({
                    "statementHandle": uuid::Uuid::new_v4().to_string(),
                    "resultSetMetaData": {
                        "rowType": [{"name": "name", "type": "TEXT"}],
                        "partitionInfo": [{}]
                    },
                    "data": [["ITEM_1"], ["ITEM_2"]]
                }),
            );
        }
    });

    let res = tokio::time::timeout(
        Duration::from_secs(3),
        driver.switch_session_context(None, Some("WH_NEW"), None, None, None),
    )
    .await
    .expect("Deadlock detected: switch_session_context timed out!")
    .expect("switch_session_context failed");

    assert_eq!(res.active_warehouse, Some("WH_NEW".into()));
    assert_eq!(driver.current_warehouse(), "WH_NEW");
    assert!(res.contexts.is_some());
}

#[tokio::test]
async fn test_parse_and_sync_snowflake_use_edge_cases() {
    let (driver, _server) = fixture(false, false).await;

    // 1. Quoted identifier with spaces
    parse_and_sync_snowflake_use(&driver, "USE ROLE \"MY ROLE\";");
    assert_eq!(driver.current_role(), "MY ROLE");

    // 2. Unquoted identifier normalized to uppercase
    parse_and_sync_snowflake_use(&driver, "USE ROLE analyst;");
    assert_eq!(driver.current_role(), "ANALYST");

    // 3. Quoted identifier preserving lowercase
    parse_and_sync_snowflake_use(&driver, "USE ROLE \"analyst\";");
    assert_eq!(driver.current_role(), "analyst");

    // 4. Quoted identifier with escaped quotes
    parse_and_sync_snowflake_use(&driver, "USE ROLE \"ANALYST\"\"S ROLE\";");
    assert_eq!(driver.current_role(), "ANALYST\"S ROLE");

    // 5. Quoted warehouse with spaces
    parse_and_sync_snowflake_use(&driver, "USE WAREHOUSE \"ETL WH\";");
    assert_eq!(driver.current_warehouse(), "ETL WH");

    // 6. Dot notation with double quotes
    parse_and_sync_snowflake_use(&driver, "USE \"PROD_DB\".\"FINANCE\";");
    assert_eq!(driver.current_database(), "PROD_DB");
    assert_eq!(driver.current_schema(), "FINANCE");

    // 7. Dot notation with USE SCHEMA and quotes
    parse_and_sync_snowflake_use(&driver, "USE SCHEMA \"ANALYTICS_DB\".\"STAGING\";");
    assert_eq!(driver.current_database(), "ANALYTICS_DB");
    assert_eq!(driver.current_schema(), "STAGING");

    // 8. USE SCHEMA single identifier without altering database
    parse_and_sync_snowflake_use(&driver, "USE SCHEMA \"PUBLIC\";");
    assert_eq!(driver.current_database(), "ANALYTICS_DB"); // preserved
    assert_eq!(driver.current_schema(), "PUBLIC");

    // 9. Preceding SQL single-line and multi-line comments
    parse_and_sync_snowflake_use(&driver, "-- switch role\nUSE ROLE SECURITYADMIN;");
    assert_eq!(driver.current_role(), "SECURITYADMIN");

    parse_and_sync_snowflake_use(&driver, "/* multi-line comment */ USE ROLE ACCOUNTADMIN;");
    assert_eq!(driver.current_role(), "ACCOUNTADMIN");

    // 10. Chained multi-statement batch
    parse_and_sync_snowflake_use(
        &driver,
        "USE ROLE \"DEV\"; USE WAREHOUSE \"WH_FAST\"; USE DATABASE \"RAW\";",
    );
    assert_eq!(driver.current_role(), "DEV");
    assert_eq!(driver.current_warehouse(), "WH_FAST");
    assert_eq!(driver.current_database(), "RAW");

    // 11. Multi-statement batch containing preceding and succeeding queries
    parse_and_sync_snowflake_use(&driver, "SELECT 1; USE ROLE \"BATCH_ROLE\"; SELECT 2;");
    assert_eq!(driver.current_role(), "BATCH_ROLE");

    // 12. Identifiers with dots inside quotes
    parse_and_sync_snowflake_use(&driver, "USE \"DB.WITH.DOTS\".\"SCHEMA.WITH.DOTS\";");
    assert_eq!(driver.current_database(), "DB.WITH.DOTS");
    assert_eq!(driver.current_schema(), "SCHEMA.WITH.DOTS");

    // 13. Unrelated statements containing word USE
    let prev_role = driver.current_role();
    parse_and_sync_snowflake_use(&driver, "SELECT use_count FROM table;");
    assert_eq!(driver.current_role(), prev_role);
}

#[tokio::test]
async fn test_switch_session_context_concurrent_stress() {
    let (driver, mut server) = fixture(false, false).await;

    // Server loop responding to any incoming requests with dummy 200 OK
    let server_task = tokio::spawn(async move {
        while let Some(req) = server.requests.recv().await {
            req.respond(
                200,
                json!({
                    "statementHandle": uuid::Uuid::new_v4().to_string(),
                    "resultSetMetaData": {
                        "rowType": [{"name": "name", "type": "TEXT"}],
                        "partitionInfo": [{}]
                    },
                    "data": [["PUBLIC"], ["ACCOUNTADMIN"], ["COMPUTE_WH"], ["DEMO_DB"]]
                }),
            );
        }
    });

    // Launch 10 concurrent tasks calling switch_session_context
    let mut handles = Vec::new();
    for i in 0..10 {
        let d = driver.clone();
        handles.push(tokio::spawn(async move {
            let wh = format!("WH_{i}");
            let role = format!("ROLE_{i}");
            let db = format!("DB_{i}");
            let sc = format!("SCHEMA_{i}");
            d.switch_session_context(None, Some(&wh), Some(&role), Some(&db), Some(&sc))
                .await
        }));
    }

    // Also run concurrent list_context_dimensions calls
    for _ in 0..5 {
        let d = driver.clone();
        handles.push(tokio::spawn(async move {
            let dims = d.list_context_dimensions().await?;
            assert_eq!(dims.len(), 4);
            Ok(SessionContextOptions {
                state: SessionRuntimeState {
                    session_id: String::new(),
                    transaction_mode: TransactionMode::AutoCommit,
                    transaction_state: TransactionState::Idle,
                    context_kind: SessionContextKind::Schema,
                    active_context: None,
                    database: String::new(),
                    role: String::new(),
                    warehouse: None,
                    search_path: vec![],
                },
                available: vec![],
                active_warehouse: None,
                active_role: None,
                active_database: None,
                active_schema: None,
                contexts: Some(dims),
            })
        }));
    }

    // Wait with timeout
    let results = tokio::time::timeout(Duration::from_secs(5), async {
        let mut res = Vec::new();
        for h in handles {
            res.push(h.await.unwrap());
        }
        res
    })
    .await
    .expect("Deadlock detected under concurrent switch_session_context execution!");

    for r in results {
        assert!(r.is_ok(), "Concurrent context switch failed: {:?}", r.err());
    }

    server_task.abort();
}

#[tokio::test]
async fn test_parse_and_sync_snowflake_use_adversarial_stress() {
    let (driver, _server) = fixture(false, false).await;

    // A. Empty, whitespace, semicolon storms
    parse_and_sync_snowflake_use(&driver, "");
    parse_and_sync_snowflake_use(&driver, "   \n\t  ");
    parse_and_sync_snowflake_use(&driver, ";;;;;");
    parse_and_sync_snowflake_use(&driver, "-- just a comment");
    parse_and_sync_snowflake_use(&driver, "/* block comment */");

    // B. Lowercase and mixed case keywords
    parse_and_sync_snowflake_use(&driver, "use warehouse \"my_wh\";");
    assert_eq!(driver.current_warehouse(), "my_wh");
    parse_and_sync_snowflake_use(&driver, "Use Role \"my_role\";");
    assert_eq!(driver.current_role(), "my_role");
    parse_and_sync_snowflake_use(&driver, "uSe DaTaBaSe \"my_db\";");
    assert_eq!(driver.current_database(), "my_db");
    parse_and_sync_snowflake_use(&driver, "USE schema \"my_sc\";");
    assert_eq!(driver.current_schema(), "my_sc");

    // C. Tabs and newlines between tokens
    parse_and_sync_snowflake_use(&driver, "\n\tUSE\n\tROLE\n\t\"TAB_ROLE\"\n;");
    assert_eq!(driver.current_role(), "TAB_ROLE");

    // D. Interleaved comments between keywords
    parse_and_sync_snowflake_use(
        &driver,
        "USE /* comment 1 */ ROLE /* comment 2 */ \"INTERLEAVED_ROLE\";",
    );
    assert_eq!(driver.current_role(), "INTERLEAVED_ROLE");

    // E. Special characters and emojis in quoted identifiers
    parse_and_sync_snowflake_use(&driver, "USE ROLE \"!@#$%^&*()_+-=\";");
    assert_eq!(driver.current_role(), "!@#$%^&*()_+-=");

    parse_and_sync_snowflake_use(&driver, "USE ROLE \"🚀_ROLE_データ\";");
    assert_eq!(driver.current_role(), "🚀_ROLE_データ");

    // F. Unquoted identifiers with lowercase normalized to uppercase
    parse_and_sync_snowflake_use(&driver, "USE ROLE dev_analyst;");
    assert_eq!(driver.current_role(), "DEV_ANALYST");

    // G. Unquoted dot notation
    parse_and_sync_snowflake_use(&driver, "USE analytics_db.staging_schema;");
    assert_eq!(driver.current_database(), "ANALYTICS_DB");
    assert_eq!(driver.current_schema(), "STAGING_SCHEMA");

    // H. Mixed unquoted and quoted dot notation
    parse_and_sync_snowflake_use(&driver, "USE \"prod_db\".finance_schema;");
    assert_eq!(driver.current_database(), "prod_db");
    assert_eq!(driver.current_schema(), "FINANCE_SCHEMA");

    // I. USE SCHEMA with unquoted and mixed dot notation
    parse_and_sync_snowflake_use(&driver, "USE SCHEMA reporting_db.summary_schema;");
    assert_eq!(driver.current_database(), "REPORTING_DB");
    assert_eq!(driver.current_schema(), "SUMMARY_SCHEMA");

    parse_and_sync_snowflake_use(&driver, "USE SCHEMA \"custom_db\".raw_schema;");
    assert_eq!(driver.current_database(), "custom_db");
    assert_eq!(driver.current_schema(), "RAW_SCHEMA");

    // J. USE SECONDARY ROLES (must not corrupt active primary role)
    let saved_role = driver.current_role();
    parse_and_sync_snowflake_use(&driver, "USE SECONDARY ROLES ALL;");
    assert_eq!(driver.current_role(), saved_role);
    parse_and_sync_snowflake_use(&driver, "USE SECONDARY ROLES NONE;");
    assert_eq!(driver.current_role(), saved_role);

    // K. Malformed USE queries (must not panic, must not corrupt valid state)
    parse_and_sync_snowflake_use(&driver, "USE;");
    parse_and_sync_snowflake_use(&driver, "USE ROLE;");
    parse_and_sync_snowflake_use(&driver, "USE WAREHOUSE;");
    parse_and_sync_snowflake_use(&driver, "USE DATABASE;");
    parse_and_sync_snowflake_use(&driver, "USE SCHEMA;");
    parse_and_sync_snowflake_use(&driver, "USE SOMETHING UNKNOWN;");
    assert_eq!(driver.current_role(), saved_role);

    // L. Queries containing USE in identifiers, literals, or CTEs
    parse_and_sync_snowflake_use(&driver, "SELECT 'USE ROLE HACK' AS query_text FROM users;");
    assert_eq!(driver.current_role(), saved_role);

    parse_and_sync_snowflake_use(
        &driver,
        "WITH use_data AS (SELECT 1) SELECT * FROM use_data;",
    );
    assert_eq!(driver.current_role(), saved_role);

    // M. 4-way multi-statement batch updating all dimensions atomically in one go
    parse_and_sync_snowflake_use(
        &driver,
        "USE ROLE \"R_FINAL\"; USE WAREHOUSE \"W_FINAL\"; USE DATABASE \"D_FINAL\"; USE SCHEMA \"S_FINAL\";",
    );
    assert_eq!(driver.current_role(), "R_FINAL");
    assert_eq!(driver.current_warehouse(), "W_FINAL");
    assert_eq!(driver.current_database(), "D_FINAL");
    assert_eq!(driver.current_schema(), "S_FINAL");

    // N. Escaped quotes with triple quote formatting
    parse_and_sync_snowflake_use(&driver, "USE ROLE \"\"\"ROLE_WITH_QUOTES\"\"\";");
    assert_eq!(driver.current_role(), "\"ROLE_WITH_QUOTES\"");

    // O. Concurrent execution of parse_and_sync_snowflake_use across threads
    let mut thread_handles = Vec::new();
    for i in 0..20 {
        let d = driver.clone();
        thread_handles.push(tokio::spawn(async move {
            let sql = format!("USE ROLE \"ROLE_{i}\"; USE WAREHOUSE \"WH_{i}\";");
            parse_and_sync_snowflake_use(&d, &sql);
        }));
    }
    for h in thread_handles {
        h.await.unwrap();
    }
    // Verify driver has a valid role and warehouse without corruption
    assert!(driver.current_role().starts_with("ROLE_"));
    assert!(driver.current_warehouse().starts_with("WH_"));
}
