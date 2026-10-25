use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

// This server uses loopback and synthetic tokens only. It never opens SSO or a database.
async fn fixture(responses: Vec<Value>) -> (Snowflake, JoinHandle<Vec<(String, Value)>>) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            let (end, length) = loop {
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
                        break (end, length);
                    }
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let body = if length == 0 {
                Value::Null
            } else {
                serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap()
            };
            requests.push((headers.lines().next().unwrap().to_string(), body));
            let body = response.to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let driver = Snowflake {
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        base: Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
        database: "DB".into(),
        schema: "PUBLIC".into(),
        role: "READ_ONLY".into(),
        warehouse: "WH".into(),
        query_tag: "Joinery/catalog-test".into(),
        auth: AsyncMutex::new(AuthState {
            tokens: Tokens {
                access: Zeroizing::new("synthetic-token".into()),
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
    (driver, server)
}

fn description() -> Value {
    json!({"resultSetMetaData":{"numRows":0,"rowType":[
        {"name":"ID","type":"fixed","scale":0,"nullable":false},
        {"name":"Odd\"Name","type":"text","nullable":true}
    ],"partitionInfo":[{}]},"data":[]})
}

#[tokio::test]
async fn catalog_discovers_tables_across_partitions_without_touching_query_cancellation() {
    let handle = uuid::Uuid::new_v4().to_string();
    let active = uuid::Uuid::new_v4().to_string();
    let (driver, server) = fixture(vec![
        json!({"statementHandle":handle,"resultSetMetaData":{"numRows":2,"partitionInfo":[{},{}]},"data":[["PUBLIC","ORDERS","BASE TABLE","123","456"]]}),
        json!({"data":[["Mixed Schema","Quoted\"View","VIEW",null,null]]})
    ]).await;
    *driver.active_handle.lock() = Some(active.clone());
    driver.cancel_requested.cancel();
    let tables = driver.catalog().await.unwrap();
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].estimated_rows, Some(123.0));
    assert_eq!(tables[0].total_bytes, Some(456.0));
    assert_eq!(tables[1].table.name, "Quoted\"View");
    assert_eq!(tables[1].estimated_rows, None);
    assert_eq!(
        driver.active_handle.lock().as_deref(),
        Some(active.as_str())
    );
    assert!(driver.cancel_requested.is_cancelled());
    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();
    assert!(sql.contains("\"DB\".INFORMATION_SCHEMA.TABLES"));
    assert!(sql.contains("LIMIT 10001"));
    assert!(!sql.contains("COUNT(*)"));
    assert!(requests[1].0.contains(&format!("/{handle}?partition=1")));
}

#[tokio::test]
async fn schema_and_preview_quote_identifiers_and_keep_every_column_read_only() {
    let mut page = description();
    page["resultSetMetaData"]["numRows"] = json!(2);
    page["data"] = json!([["9223372036854775808", null], ["2", ""]]);
    let (driver, server) = fixture(vec![description(), page]).await;
    let table = TableRef {
        schema: "Mixed Schema".into(),
        name: "Odd\"Table; DROP TABLE x".into(),
    };
    let schema = driver.schema(&table).await.unwrap();
    assert!(!schema.editable);
    assert!(schema.primary_key.is_empty());
    assert!(schema.columns.iter().all(|c| !c.editable && !c.primary_key));
    assert!(matches!(schema.columns[0].kind, ColumnKind::Integer));
    let page = driver.browse(&schema, None).await.unwrap();
    assert_eq!(page.rows[0][0].as_deref(), Some("9223372036854775808"));
    assert_eq!(page.rows[0][1], None);
    assert_eq!(page.rows[1][1].as_deref(), Some(""));
    assert!(!page.has_more);
    assert!(page.next_key.is_none());
    let requests = server.await.unwrap();
    assert_eq!(
        requests[0].1["statement"],
        "SELECT * FROM \"DB\".\"Mixed Schema\".\"Odd\"\"Table; DROP TABLE x\" LIMIT 0"
    );
    assert_eq!(
        requests[1].1["statement"],
        "SELECT \"ID\",\"Odd\"\"Name\" FROM \"DB\".\"Mixed Schema\".\"Odd\"\"Table; DROP TABLE x\" LIMIT 501"
    );
}

#[tokio::test]
async fn catalog_fails_instead_of_silently_showing_an_incomplete_database() {
    for response in [
        json!({"resultSetMetaData":{"numRows":10001},"data":[]}),
        json!({"resultSetMetaData":{"numRows":2},"data":[["PUBLIC","T","BASE TABLE",null,null]]}),
        json!({"resultSetMetaData":{"numRows":2,"partitionInfo":[{},{}]},"data":[["PUBLIC","T","BASE TABLE",null,null]]}),
    ] {
        let (driver, server) = fixture(vec![response]).await;
        assert!(driver.catalog().await.is_err());
        assert_eq!(server.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn preview_stops_at_row_and_byte_limits_and_rejects_unstable_continuation() {
    for (rows, expected) in [
        (
            (0..501)
                .map(|n| json!([n.to_string(), null]))
                .collect::<Vec<_>>(),
            500,
        ),
        (
            vec![
                json!(["1", "x".repeat(1024 * 1024)]),
                json!(["2", "y".repeat(1024 * 1024)]),
            ],
            1,
        ),
    ] {
        let mut response = description();
        response["resultSetMetaData"]["numRows"] = json!(rows.len());
        response["data"] = json!(rows);
        let (driver, server) = fixture(vec![description(), response]).await;
        let schema = driver
            .schema(&TableRef {
                schema: "PUBLIC".into(),
                name: "T".into(),
            })
            .await
            .unwrap();
        let page = driver.browse(&schema, None).await.unwrap();
        assert_eq!(page.rows.len(), expected);
        assert!(page.truncated);
        assert!(!page.has_more);
        assert!(
            driver
                .browse(&schema, Some(vec![Some("1".into())]))
                .await
                .is_err()
        );
        assert_eq!(server.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn a_single_oversized_row_and_changed_column_types_are_rejected() {
    let mut oversized = description();
    oversized["resultSetMetaData"]["numRows"] = json!(1);
    oversized["data"] = json!([["1", "x".repeat(2 * 1024 * 1024)]]);
    let mut changed = description();
    changed["resultSetMetaData"]["rowType"][0]["type"] = json!("TEXT");
    for response in [oversized, changed] {
        let (driver, server) = fixture(vec![description(), response]).await;
        let schema = driver
            .schema(&TableRef {
                schema: "PUBLIC".into(),
                name: "T".into(),
            })
            .await
            .unwrap();
        assert!(driver.browse(&schema, None).await.is_err());
        assert_eq!(server.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn preview_uses_all_partitions_and_rejects_writes_before_network_access() {
    let mut first = description();
    let handle = uuid::Uuid::new_v4().to_string();
    first["statementHandle"] = json!(handle);
    first["resultSetMetaData"]["partitionInfo"] = json!([{}, {}]);
    first["resultSetMetaData"]["numRows"] = json!(2);
    first["data"] = json!([["1", null]]);
    let (driver, server) = fixture(vec![description(), first, json!({"data":[["2",""]]})]).await;
    let schema = driver
        .schema(&TableRef {
            schema: "PUBLIC".into(),
            name: "T".into(),
        })
        .await
        .unwrap();
    let page = driver.browse(&schema, None).await.unwrap();
    assert_eq!(page.rows.len(), 2);
    assert!(!page.truncated);
    assert!(driver.apply(&schema, &[]).await.is_err());
    assert!(
        driver
            .execute(
                "DELETE FROM T",
                StatementKind::Write,
                &mut super::tests::Sink::default()
            )
            .await
            .is_err()
    );
    assert_eq!(server.await.unwrap().len(), 3);
}

#[tokio::test]
async fn invalid_object_identifiers_never_reach_the_server() {
    let (driver, server) = fixture(vec![]).await;
    for name in [String::new(), "bad\0name".into(), "x".repeat(256)] {
        assert!(
            driver
                .schema(&TableRef {
                    schema: "PUBLIC".into(),
                    name
                })
                .await
                .is_err()
        );
    }
    assert!(server.await.unwrap().is_empty());
}
