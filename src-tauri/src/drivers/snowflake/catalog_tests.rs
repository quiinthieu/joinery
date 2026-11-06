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
            let status = if response.get("sqlState").is_some() {
                "422 Unprocessable Entity"
            } else {
                "200 OK"
            };
            let body = response.to_string();
            stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let driver = Snowflake {
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        base: Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
        database: Mutex::new("DB".into()),
        schema: Mutex::new("PUBLIC".into()),
        role: Mutex::new("READ_ONLY".into()),
        warehouse: Mutex::new("WH".into()),
        query_tag: "Joinery/catalog-test".into(),
        auth: AsyncMutex::new(AuthState {
            tokens: Tokens {
                access: Zeroizing::new("synthetic-token".into()),
                refresh: None,
                expires_in: 600,
            },
            expires_at: Instant::now() + Duration::from_secs(600),
        }),
        auth_method: SnowflakeAuthMethod::OAuthBrowser,
        key_pair: None,
        username: "TEST_USER".into(),
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

fn pk_result(columns: &[(&str, usize)]) -> Value {
    let rows: Vec<Value> = columns
        .iter()
        .map(|(name, seq)| {
            json!([
                "2026-01-01",
                "DB",
                "PUBLIC",
                "T",
                name,
                seq.to_string(),
                "SYS_PK",
                "false",
                null
            ])
        })
        .collect();
    json!({
        "resultSetMetaData": {
            "numRows": rows.len(),
            "rowType": [
                {"name": "created_on", "type": "text"},
                {"name": "database_name", "type": "text"},
                {"name": "schema_name", "type": "text"},
                {"name": "table_name", "type": "text"},
                {"name": "column_name", "type": "text"},
                {"name": "key_sequence", "type": "fixed"},
                {"name": "constraint_name", "type": "text"},
                {"name": "rely", "type": "text"},
                {"name": "comment", "type": "text"}
            ],
            "partitionInfo": [{}]
        },
        "data": rows
    })
}

fn completion_context(path: Value) -> Value {
    json!({"resultSetMetaData":{"numRows":1},"data":[["FIXTURE","READ_ONLY","DB","PUBLIC","WH",path.to_string()]]})
}

fn quoted_case(value: &str) -> Value {
    json!({"resultSetMetaData":{"numRows":1},"data":[["QUOTED_IDENTIFIERS_IGNORE_CASE",value]]})
}

#[tokio::test]
async fn completion_verifies_request_context_and_binds_only_the_first_schema() {
    let (driver, server) = fixture(vec![
        completion_context(json!(["DB.PRIVATE", "DB.PUBLIC"])),
        quoted_case("false"),
        json!({"resultSetMetaData":{"numRows":2},"data":[
            ["PRIVATE","ORDERS","BASE TABLE",null,null],
            ["PUBLIC","ORDERS","BASE TABLE",null,null]
        ]}),
    ])
    .await;
    let identity = driver.completion_identity().await.unwrap();
    assert_eq!(identity.database, "DB");
    assert_eq!(identity.role, "FIXTURE (READ_ONLY)");
    assert_eq!(identity.search_path, ["DB.PRIVATE", "DB.PUBLIC"]);
    let catalog = driver.completion_catalog(&identity).await.unwrap();
    assert_eq!(catalog.tables.len(), 2);
    assert_eq!(
        catalog.unqualified_tables,
        vec![TableRef {
            schema: "PRIVATE".into(),
            name: "ORDERS".into()
        }]
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    let statement = requests[0].1["statement"].as_str().unwrap();
    assert_eq!(
        statement,
        "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SCHEMAS()"
    );
    assert!(!statement.contains("TO_JSON"));
    assert!(
        requests[1].1["statement"]
            .as_str()
            .unwrap()
            .starts_with("SHOW PARAMETERS")
    );
    for (_, request) in requests {
        assert_eq!(request["database"], "DB");
        assert_eq!(request["schema"], "PUBLIC");
        assert_eq!(request["warehouse"], "WH");
        assert_eq!(request["role"], "READ_ONLY");
        assert_eq!(request["parameters"]["QUERY_TAG"], "Joinery/catalog-test");
    }

    let (driver, server) = fixture(vec![
        json!({"resultSetMetaData":{"numRows":1},"data":[["FIXTURE","read_only","db","public","wh","[DB.PRIVATE, DB.PUBLIC]"]]}),
        quoted_case("false"),
    ])
    .await;
    let identity = driver.completion_identity().await.unwrap();
    assert_eq!(identity.database, "db");
    assert_eq!(identity.search_path, ["DB.PRIVATE", "DB.PUBLIC"]);
    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test]
async fn completion_keeps_qualified_names_when_the_search_path_is_out_of_scope() {
    for path in [
        json!(["OTHER.PUBLIC", "DB.PUBLIC"]),
        json!(["DB.\"Mixed Schema\""]),
        json!([]),
    ] {
        let (driver, server) = fixture(vec![
            completion_context(path), quoted_case("false"),
            json!({"resultSetMetaData":{"numRows":1},"data":[["PUBLIC","ORDERS","BASE TABLE",null,null]]}),
        ]).await;
        let identity = driver.completion_identity().await.unwrap();
        let catalog = driver.completion_catalog(&identity).await.unwrap();
        assert_eq!(catalog.tables.len(), 1);
        assert!(catalog.unqualified_tables.is_empty());
        assert_eq!(server.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn completion_rejects_unverified_or_unsupported_snowflake_context() {
    let mut wrong_role = completion_context(json!(["DB.PUBLIC"]));
    wrong_role["data"][0][1] = json!("OTHER_ROLE");
    let mut malformed_path = completion_context(json!([]));
    malformed_path["data"][0][5] = json!("not JSON");
    for responses in [
        vec![wrong_role],
        vec![malformed_path],
        vec![
            completion_context(json!(["DB.PUBLIC"])),
            quoted_case("true"),
        ],
        vec![json!({"resultSetMetaData":{"numRows":1},"data":[["x".repeat(70*1024)]]})],
    ] {
        let expected = responses.len();
        let (driver, server) = fixture(responses).await;
        assert!(driver.completion_identity().await.is_err());
        assert_eq!(server.await.unwrap().len(), expected);
    }
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
async fn schema_and_preview_quote_identifiers_and_discover_primary_keys() {
    let mut page = description();
    page["resultSetMetaData"]["numRows"] = json!(2);
    page["data"] = json!([["9223372036854775808", null], ["2", ""]]);
    let (driver, server) = fixture(vec![description(), pk_result(&[("ID", 1)]), page]).await;
    let table = TableRef {
        schema: "Mixed Schema".into(),
        name: "Odd\"Table; DROP TABLE x".into(),
    };
    let schema = driver.schema(&table).await.unwrap();
    assert!(schema.editable);
    assert_eq!(schema.primary_key, vec!["ID"]);
    assert!(schema.columns[0].primary_key);
    assert!(!schema.columns[0].editable);
    assert!(!schema.columns[1].primary_key);
    assert!(schema.columns[1].editable);
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
        "SHOW PRIMARY KEYS IN TABLE \"DB\".\"Mixed Schema\".\"Odd\"\"Table; DROP TABLE x\""
    );
    assert_eq!(
        requests[2].1["statement"],
        "SELECT \"ID\",\"Odd\"\"Name\" FROM \"DB\".\"Mixed Schema\".\"Odd\"\"Table; DROP TABLE x\" LIMIT 501"
    );
}

#[tokio::test]
async fn schema_falls_back_to_unique_keys_when_no_primary_keys() {
    let (driver, server) = fixture(vec![
        description(),
        pk_result(&[]),
        pk_result(&[("Odd\"Name", 1)]),
    ])
    .await;
    let table = TableRef {
        schema: "PUBLIC".into(),
        name: "USERS".into(),
    };
    let schema = driver.schema(&table).await.unwrap();
    assert!(schema.editable);
    assert_eq!(schema.primary_key, vec!["Odd\"Name"]);
    assert!(!schema.columns[0].primary_key);
    assert!(schema.columns[0].editable);
    assert!(schema.columns[1].primary_key);
    assert!(!schema.columns[1].editable);
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1].1["statement"]
            .as_str()
            .unwrap()
            .starts_with("SHOW PRIMARY KEYS")
    );
    assert!(
        requests[2].1["statement"]
            .as_str()
            .unwrap()
            .starts_with("SHOW UNIQUE KEYS")
    );
}

#[tokio::test]
async fn schema_falls_back_to_inferred_non_variant_columns_when_no_declared_keys() {
    let (driver, server) = fixture(vec![description(), pk_result(&[]), pk_result(&[])]).await;
    let table = TableRef {
        schema: "PUBLIC".into(),
        name: "EVENTS".into(),
    };
    let schema = driver.schema(&table).await.unwrap();
    assert!(schema.editable);
    assert_eq!(schema.primary_key, vec!["ID", "Odd\"Name"]);
    // For inferred columns, column.primary_key must remain false so cells can be edited in grid
    assert!(!schema.columns[0].primary_key);
    assert!(schema.columns[0].editable);
    assert!(!schema.columns[1].primary_key);
    assert!(schema.columns[1].editable);
    assert_eq!(server.await.unwrap().len(), 3);
}

#[tokio::test]
async fn snowflake_apply_submits_scripting_procedural_block_and_detects_conflict() {
    let (driver, server) = fixture(vec![
        json!({
            "message": "Uncaught exception of type 'CONFLICT_ERR': A table change did not affect exactly one row. No edits were committed.",
            "sqlState": "P0000"
        })
    ])
    .await;
    let schema = TableSchema {
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "CUSTOMERS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "id".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "name".into(),
                database_type: "VARCHAR".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["id".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };
    let mutation = RowMutation::Update {
        original: vec![Some("10".into()), Some("Old Corp".into())],
        changes: vec![(1, Some("New Corp".into()))],
    };
    let err = driver.apply(&schema, &[mutation]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert!(
        err.message
            .contains("A table change did not affect exactly one row")
    );
    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();
    assert!(sql.contains("EXECUTE IMMEDIATE $_JOINERY_MUTATION_BLOCK_"));
    assert!(sql.contains("SQLROWCOUNT != 1"));
    assert!(sql.contains("RAISE conflict_err"));
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
        let (driver, server) =
            fixture(vec![description(), pk_result(&[("ID", 1)]), response]).await;
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
        assert_eq!(server.await.unwrap().len(), 3);
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
        let (driver, server) =
            fixture(vec![description(), pk_result(&[("ID", 1)]), response]).await;
        let schema = driver
            .schema(&TableRef {
                schema: "PUBLIC".into(),
                name: "T".into(),
            })
            .await
            .unwrap();
        assert!(driver.browse(&schema, None).await.is_err());
        assert_eq!(server.await.unwrap().len(), 3);
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
    let (driver, server) = fixture(vec![
        description(),
        pk_result(&[("ID", 1)]),
        first,
        json!({"data":[["2",""]]}),
    ])
    .await;
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
    assert_eq!(server.await.unwrap().len(), 4);
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

#[tokio::test]
async fn schema_context_switch_is_stateless_and_verifies_resolution() {
    let (driver, server) = fixture(vec![
        json!({"resultSetMetaData":{"numRows":3},"data":[
            ["ANALYTICS"],
            ["PUBLIC"],
            ["mixed_case_ignored"]
        ]}),
        json!({"resultSetMetaData":{"numRows":1},"data":[[null]]}),
        json!({"resultSetMetaData":{"numRows":1},"data":[["ANALYTICS"]]}),
    ])
    .await;

    let schemas = driver.list_contexts().await.unwrap();
    assert_eq!(schemas, vec!["ANALYTICS", "PUBLIC"]);

    // Lowercase or quoted names are rejected locally without contacting the server.
    assert_eq!(
        driver.switch_context("analytics").await.unwrap_err().code,
        ErrorCode::InvalidInput
    );

    // If Snowflake does not resolve CURRENT_SCHEMA() to the target, state stays unchanged.
    assert_eq!(
        driver.switch_context("MISSING").await.unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        driver
            .session_state("s-1")
            .await
            .unwrap()
            .active_context
            .as_deref(),
        Some("PUBLIC")
    );

    // Verified switch updates request-scoped schema without issuing USE SCHEMA.
    driver.switch_context("ANALYTICS").await.unwrap();
    let state = driver.session_state("s-1").await.unwrap();
    assert_eq!(state.active_context.as_deref(), Some("ANALYTICS"));
    assert_eq!(state.search_path, vec!["DB.ANALYTICS"]);

    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].1["schema"], "MISSING");
    assert_eq!(requests[2].1["schema"], "ANALYTICS");
    assert_eq!(requests[2].1["statement"], "SELECT CURRENT_SCHEMA()");
}

#[tokio::test]
async fn completion_catalog_handles_null_table_types_and_falls_back_to_active_schema_on_oversized_databases()
 {
    let (driver, server) = fixture(vec![
        completion_context(json!(["DB.PUBLIC"])),
        quoted_case("false"),
        // Database-wide INFORMATION_SCHEMA query exceeds 10,000 rows:
        json!({"resultSetMetaData":{"numRows":10001},"data":[]}),
        // Active-schema fallback returns tables including a Dynamic Table with null TABLE_TYPE:
        json!({"resultSetMetaData":{"numRows":1},"data":[
            ["PUBLIC","FACT_PO_TRACKER_7",null,"42","1024"]
        ]}),
    ])
    .await;
    let identity = driver.completion_identity().await.unwrap();
    let catalog = driver.completion_catalog(&identity).await.unwrap();
    assert_eq!(catalog.tables.len(), 1);
    assert_eq!(catalog.tables[0].table.schema, "PUBLIC");
    assert_eq!(catalog.tables[0].table.name, "FACT_PO_TRACKER_7");
    assert_eq!(catalog.tables[0].kind, "BASE TABLE");
    assert_eq!(
        catalog.unqualified_tables,
        vec![TableRef {
            schema: "PUBLIC".into(),
            name: "FACT_PO_TRACKER_7".into(),
        }]
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 4);
    assert!(
        requests[3].1["statement"]
            .as_str()
            .unwrap()
            .contains("WHERE TABLE_SCHEMA = 'PUBLIC'")
    );
}

// =========================================================================
// Adversarial Empirical Tests: Concurrency, Mutations & Escaping
// =========================================================================

#[tokio::test]
async fn test_adversarial_apply_detects_conflict_on_zero_rows_and_multiple_rows() {
    let (driver, server) = fixture(vec![
        json!({
            "message": "Uncaught exception of type 'CONFLICT_ERR' on line 4: A table change did not affect exactly one row. No edits were committed.",
            "sqlState": "P0000"
        })
    ]).await;

    let schema = TableSchema {
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "CUSTOMERS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "id".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "name".into(),
                database_type: "VARCHAR".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["id".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };

    let mutation = RowMutation::Update {
        original: vec![Some("10".into()), Some("Old Corp".into())],
        changes: vec![(1, Some("New Corp".into()))],
    };

    let err = driver.apply(&schema, &[mutation]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert!(
        err.message
            .contains("A table change did not affect exactly one row")
    );

    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();
    // Confirms SQLROWCOUNT != 1 checks both 0 rows affected (concurrent modification)
    // and > 1 rows affected (duplicate/non-unique match)
    assert!(sql.contains("IF (SQLROWCOUNT != 1) THEN RAISE conflict_err; END IF;"));
}

#[tokio::test]
async fn test_adversarial_apply_enforces_atomic_transaction_rollback() {
    let (driver, server) =
        fixture(vec![json!({"resultSetMetaData":{"numRows":0},"data":[]})]).await;

    let schema = TableSchema {
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "ACCOUNTS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "id".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "balance".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Decimal,
                nullable: false,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["id".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };

    // Multi-row batch mutation: 2 updates
    let mutations = vec![
        RowMutation::Update {
            original: vec![Some("1".into()), Some("100.00".into())],
            changes: vec![(1, Some("150.00".into()))],
        },
        RowMutation::Update {
            original: vec![Some("2".into()), Some("200.00".into())],
            changes: vec![(1, Some("250.00".into()))],
        },
    ];

    driver.apply(&schema, &mutations).await.unwrap();
    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();

    let has_explicit_transaction = sql.contains("BEGIN TRANSACTION;");
    let has_rollback_handler = sql.contains("ROLLBACK;");
    let has_explicit_commit = sql.contains("COMMIT;");
    let has_exception_handler = sql.contains("EXCEPTION") && sql.contains("WHEN OTHER THEN");
    let has_reraise = sql.contains("RAISE;");

    assert!(
        has_explicit_transaction
            && has_rollback_handler
            && has_explicit_commit
            && has_exception_handler
            && has_reraise,
        "apply procedural block must enforce BEGIN TRANSACTION, COMMIT, and EXCEPTION WHEN OTHER THEN ROLLBACK; RAISE; for atomicity!"
    );
}

#[tokio::test]
async fn test_adversarial_apply_dollar_quote_breakout_vulnerability() {
    let (driver, server) =
        fixture(vec![json!({"resultSetMetaData":{"numRows":0},"data":[]})]).await;

    let schema = TableSchema {
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "DOCUMENTS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "id".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "body".into(),
                database_type: "VARCHAR".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["id".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };

    // Adversarial input: dollar signs inside value attempting to escape EXECUTE IMMEDIATE $$ ... $$
    let payload = "text with $$; DROP TABLE CRITICAL_DATA; --";
    let mutation = RowMutation::Update {
        original: vec![Some("1".into()), Some("old text".into())],
        changes: vec![(1, Some(payload.into()))],
    };

    driver.apply(&schema, &[mutation]).await.unwrap();
    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();

    // Outer wrapper uses tagged dollar quote delimiter:
    assert!(sql.starts_with("EXECUTE IMMEDIATE $_JOINERY_MUTATION_BLOCK_$"));
    assert!(sql.ends_with("$_JOINERY_MUTATION_BLOCK_$;"));

    // The inner body safely retains $$ without colliding with the outer delimiter:
    assert!(
        sql.contains("'text with $$; DROP TABLE CRITICAL_DATA; --'"),
        "Inner $$ safely preserved inside string literal without colliding with outer delimiter"
    );
}

#[tokio::test]
async fn test_adversarial_apply_dynamic_tag_avoids_delimiter_collision() {
    let (driver, server) =
        fixture(vec![json!({"resultSetMetaData":{"numRows":0},"data":[]})]).await;

    let schema = TableSchema {
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "DOCUMENTS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "id".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "body".into(),
                database_type: "VARCHAR".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["id".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };

    // Adversarial input contains default tag $_JOINERY_MUTATION_BLOCK_$ to force collision:
    let payload = "malicious $_JOINERY_MUTATION_BLOCK_$; DROP TABLE PROD_USERS; --";
    let mutation = RowMutation::Update {
        original: vec![Some("1".into()), Some("old text".into())],
        changes: vec![(1, Some(payload.into()))],
    };

    driver.apply(&schema, &[mutation]).await.unwrap();
    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();

    // The outer wrapper dynamically incremented tag to avoid collision:
    assert!(sql.starts_with("EXECUTE IMMEDIATE $_JOINERY_MUTATION_BLOCK_1_$"));
    assert!(sql.ends_with("$_JOINERY_MUTATION_BLOCK_1_$;"));
    assert!(sql.contains("'malicious $_JOINERY_MUTATION_BLOCK_$; DROP TABLE PROD_USERS; --'"));
}

#[tokio::test]
async fn test_adversarial_apply_multi_level_delimiter_collisions_and_markdown() {
    let (driver, server) =
        fixture(vec![json!({"resultSetMetaData":{"numRows":0},"data":[]})]).await;

    let schema = TableSchema {
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "DOCUMENTS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "id".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "body".into(),
                database_type: "VARCHAR".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["id".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };

    // Adversarial input contains $$, $_JOINERY_MUTATION_BLOCK_$, $_JOINERY_MUTATION_BLOCK_1_$, and $_JOINERY_MUTATION_BLOCK_2_$
    let payload = "formula: $$E=mc^2$$ and tags: $_JOINERY_MUTATION_BLOCK_$ and $_JOINERY_MUTATION_BLOCK_1_$ and $_JOINERY_MUTATION_BLOCK_2_$";
    let mutation = RowMutation::Update {
        original: vec![Some("1".into()), Some("old text".into())],
        changes: vec![(1, Some(payload.into()))],
    };

    driver.apply(&schema, &[mutation]).await.unwrap();
    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();

    // The outer wrapper dynamically incremented tag to _JOINERY_MUTATION_BLOCK_3_ to avoid all collisions:
    assert!(sql.starts_with("EXECUTE IMMEDIATE $_JOINERY_MUTATION_BLOCK_3_$"));
    assert!(sql.ends_with("$_JOINERY_MUTATION_BLOCK_3_$;"));
    assert!(sql.contains("formula: $$E=mc^2$$ and tags: $_JOINERY_MUTATION_BLOCK_$ and $_JOINERY_MUTATION_BLOCK_1_$ and $_JOINERY_MUTATION_BLOCK_2_$"));
}

#[tokio::test]
async fn test_adversarial_apply_multi_row_atomicity_and_conflict_re_raise() {
    // Server simulates exception re-raised by EXCEPTION WHEN OTHER THEN ROLLBACK; RAISE;
    let (driver, server) = fixture(vec![
        json!({
            "message": "Uncaught exception of type 'CONFLICT_ERR' on line 14: A table change did not affect exactly one row. No edits were committed.",
            "sqlState": "P0000"
        })
    ]).await;

    let schema = TableSchema {
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "ACCOUNTS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "id".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "balance".into(),
                database_type: "NUMBER(18,2)".into(),
                kind: ColumnKind::Decimal,
                nullable: false,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["id".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };

    let mutations = vec![
        RowMutation::Update {
            original: vec![Some("1".into()), Some("100.00".into())],
            changes: vec![(1, Some("110.00".into()))],
        },
        RowMutation::Update {
            original: vec![Some("2".into()), Some("200.00".into())],
            changes: vec![(1, Some("210.00".into()))],
        },
        RowMutation::Update {
            original: vec![Some("3".into()), Some("300.00".into())],
            changes: vec![(1, Some("310.00".into()))],
        },
    ];

    let err = driver.apply(&schema, &mutations).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(
        err.message,
        "A table change did not affect exactly one row. No edits were committed."
    );

    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();

    // Verify transaction block order and structure
    let begin_txn_idx = sql
        .find("BEGIN TRANSACTION;")
        .expect("BEGIN TRANSACTION; present");
    let commit_idx = sql.find("COMMIT;").expect("COMMIT; present");
    let exception_idx = sql
        .find("WHEN OTHER THEN")
        .expect("WHEN OTHER THEN block present");
    let rollback_idx = sql.find("ROLLBACK;").expect("ROLLBACK; present");
    let reraise_idx = sql.find("RAISE;").expect("RAISE; present");

    assert!(
        begin_txn_idx < commit_idx,
        "BEGIN TRANSACTION must precede COMMIT"
    );
    assert!(
        commit_idx < exception_idx,
        "COMMIT must precede EXCEPTION handler"
    );
    assert!(
        exception_idx < rollback_idx,
        "EXCEPTION handler must contain ROLLBACK"
    );
    assert!(
        rollback_idx < reraise_idx,
        "ROLLBACK must precede RAISE in exception handler"
    );

    // All 3 update statements must be between BEGIN TRANSACTION and COMMIT
    let update_1_idx = sql
        .find("UPDATE \"DB\".\"PUBLIC\".\"ACCOUNTS\" SET \"balance\" = 110.00 WHERE \"id\" = 1")
        .expect("update 1");
    let update_2_idx = sql
        .find("UPDATE \"DB\".\"PUBLIC\".\"ACCOUNTS\" SET \"balance\" = 210.00 WHERE \"id\" = 2")
        .expect("update 2");
    let update_3_idx = sql
        .find("UPDATE \"DB\".\"PUBLIC\".\"ACCOUNTS\" SET \"balance\" = 310.00 WHERE \"id\" = 3")
        .expect("update 3");

    assert!(begin_txn_idx < update_1_idx);
    assert!(update_1_idx < update_2_idx);
    assert!(update_2_idx < update_3_idx);
    assert!(update_3_idx < commit_idx);
}

#[tokio::test]
async fn test_adversarial_apply_insert_empty_values_generates_default_values() {
    let (driver, server) =
        fixture(vec![json!({"resultSetMetaData":{"numRows":0},"data":[]})]).await;

    let schema = TableSchema {
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "AUDIT_LOG".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "id".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: true,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "created_at".into(),
                database_type: "TIMESTAMP_NTZ".into(),
                kind: ColumnKind::Temporal,
                nullable: false,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: true,
            },
        ],
        primary_key: vec!["id".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };

    let mutation = RowMutation::Insert { values: vec![] };

    driver.apply(&schema, &[mutation]).await.unwrap();
    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();

    // Must generate DEFAULT VALUES, never empty () VALUES ()
    assert!(sql.contains("INSERT INTO \"DB\".\"PUBLIC\".\"AUDIT_LOG\" DEFAULT VALUES;"));
    assert!(!sql.contains("() VALUES ()"));
    assert!(sql.contains("IF (SQLROWCOUNT != 1) THEN RAISE conflict_err; END IF;"));
}

#[tokio::test]
async fn test_adversarial_apply_composite_pks_and_special_identifiers() {
    let (driver, server) =
        fixture(vec![json!({"resultSetMetaData":{"numRows":0},"data":[]})]).await;

    let schema = TableSchema {
        table: TableRef {
            schema: "SPECIAL SCHEMA".into(),
            name: "TABLE WITH SPACES".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "PK 1".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "PK 2".into(),
                database_type: "VARCHAR".into(),
                kind: ColumnKind::Text,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c2".into(),
                name: "SELECT".into(), // SQL keyword
                database_type: "VARCHAR".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["PK 1".into(), "PK 2".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };

    let mutation = RowMutation::Update {
        original: vec![Some("100".into()), Some("DEPT_A".into()), Some("v1".into())],
        changes: vec![(2, Some("v2".into()))],
    };

    driver.apply(&schema, &[mutation]).await.unwrap();
    let requests = server.await.unwrap();
    let sql = requests[0].1["statement"].as_str().unwrap();

    // Verifies fully qualified table and all composite PKs and reserved columns are safely quoted
    assert!(sql.contains("UPDATE \"DB\".\"SPECIAL SCHEMA\".\"TABLE WITH SPACES\""));
    assert!(sql.contains("\"SELECT\" = 'v2'"));
    assert!(sql.contains("\"PK 1\" = 100"));
    assert!(sql.contains("\"PK 2\" = 'DEPT_A'"));
    assert!(sql.contains("EQUAL_NULL(\"SELECT\", 'v1')"));
}

#[tokio::test]
async fn test_adversarial_variant_only_table_safely_blocked_without_panic() {
    let (driver, server) = fixture(vec![
        // 1. SELECT * LIMIT 0 -> only VARIANT column
        json!({"resultSetMetaData":{"numRows":0,"rowType":[
            {"name":"RAW_JSON","type":"variant","nullable":true}
        ]},"data":[]}),
        // 2. SHOW PRIMARY KEYS -> 0 rows
        json!({"resultSetMetaData":{"numRows":0,"rowType":[
            {"name":"column_name","type":"text"},
            {"name":"key_sequence","type":"fixed"}
        ]},"data":[]}),
        // 3. SHOW UNIQUE KEYS -> 0 rows
        json!({"resultSetMetaData":{"numRows":0,"rowType":[
            {"name":"column_name","type":"text"},
            {"name":"key_sequence","type":"fixed"}
        ]},"data":[]}),
    ])
    .await;

    let table_ref = TableRef {
        schema: "PUBLIC".into(),
        name: "VARIANT_LOGS".into(),
    };

    let schema = driver.table_schema(&table_ref).await.unwrap();
    assert!(!schema.editable);
    assert!(schema.primary_key.is_empty());
    assert!(!schema.columns[0].editable);
    assert!(!schema.columns[0].insertable);

    // Attempting to apply mutations must immediately fail with PolicyBlocked without network call
    let mutation = RowMutation::Update {
        original: vec![Some("{}".into())],
        changes: vec![(0, Some("{\"x\": 1}".into()))],
    };
    let err = driver.apply(&schema, &[mutation]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PolicyBlocked);
    assert_eq!(err.message, "This table cannot be edited safely");

    // All 3 catalog queries were performed, 0 apply queries were sent
    assert_eq!(server.await.unwrap().len(), 3);
}

fn make_metadata_response(columns: &[&str], rows: Vec<Vec<Option<&str>>>) -> Value {
    let row_type: Vec<Value> = columns
        .iter()
        .map(|c| json!({"name": c, "type": "text"}))
        .collect();
    let data: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let row_vals: Vec<Value> = r
                .into_iter()
                .map(|v| match v {
                    Some(s) => json!(s),
                    None => Value::Null,
                })
                .collect();
            Value::Array(row_vals)
        })
        .collect();
    json!({
        "resultSetMetaData": {
            "numRows": data.len(),
            "rowType": row_type,
            "partitionInfo": [{}]
        },
        "data": data
    })
}

#[tokio::test]
async fn test_adversarial_catalog_objects_partial_failure_continues() {
    let stages_resp = make_metadata_response(
        &["STAGE_SCHEMA", "STAGE_NAME", "STAGE_TYPE", "COMMENT"],
        vec![
            vec![
                Some("PUBLIC"),
                Some("RAW_STAGE"),
                Some("INTERNAL"),
                Some("raw files"),
            ],
            vec![Some("STAGING"), Some("S3_STAGE"), Some("EXTERNAL"), None],
        ],
    );
    // Task query fails with 422 sqlState
    let tasks_resp = json!({
        "message": "Insufficient privileges on task",
        "sqlState": "02000"
    });
    let streams_resp = make_metadata_response(
        &[
            "STREAM_SCHEMA",
            "STREAM_NAME",
            "TABLE_NAME",
            "MODE",
            "COMMENT",
        ],
        vec![vec![
            Some("PUBLIC"),
            Some("DELTA_STREAM"),
            Some("ORDERS"),
            Some("DEFAULT"),
            None,
        ]],
    );
    // Pipes query fails with 422 sqlState
    let pipes_resp = json!({
        "message": "Pipe operation requires MONITOR privilege",
        "sqlState": "02000"
    });
    let procs_resp = make_metadata_response(
        &[
            "PROCEDURE_SCHEMA",
            "PROCEDURE_NAME",
            "ARGUMENT_SIGNATURE",
            "DATA_TYPE",
            "COMMENT",
        ],
        vec![vec![
            Some("PUBLIC"),
            Some("CALC_TAX"),
            Some("(ID NUMBER)"),
            Some("NUMBER"),
            None,
        ]],
    );
    let funcs_resp = make_metadata_response(
        &[
            "FUNCTION_SCHEMA",
            "FUNCTION_NAME",
            "ARGUMENT_SIGNATURE",
            "DATA_TYPE",
            "COMMENT",
        ],
        vec![vec![
            Some("PUBLIC"),
            Some("ADD_ONE"),
            Some("(X NUMBER)"),
            Some("NUMBER"),
            None,
        ]],
    );
    let seqs_resp = make_metadata_response(
        &["SEQUENCE_SCHEMA", "SEQUENCE_NAME", "DATA_TYPE", "COMMENT"],
        vec![vec![
            Some("PUBLIC"),
            Some("ORDER_SEQ"),
            Some("NUMBER(38,0)"),
            None,
        ]],
    );
    let formats_resp = make_metadata_response(
        &[
            "FILE_FORMAT_SCHEMA",
            "FILE_FORMAT_NAME",
            "FILE_FORMAT_TYPE",
            "COMMENT",
        ],
        vec![vec![Some("PUBLIC"), Some("CSV_FORMAT"), Some("CSV"), None]],
    );

    let (driver, server) = fixture(vec![
        stages_resp,
        tasks_resp,
        streams_resp,
        pipes_resp,
        procs_resp,
        funcs_resp,
        seqs_resp,
        formats_resp,
    ])
    .await;

    let objects = driver.catalog_objects().await.unwrap();
    // 2 stages + 0 tasks + 1 stream + 0 pipes + 1 proc + 1 func + 1 seq + 1 format = 7 objects
    assert_eq!(objects.len(), 7);

    assert_eq!(objects[0].object.kind, DatabaseObjectKind::Stage);
    assert_eq!(objects[0].object.name, "RAW_STAGE");
    assert_eq!(objects[0].signature.as_deref(), Some("INTERNAL"));

    assert_eq!(objects[1].object.kind, DatabaseObjectKind::Stage);
    assert_eq!(objects[1].object.name, "S3_STAGE");

    assert_eq!(objects[2].object.kind, DatabaseObjectKind::Stream);
    assert_eq!(objects[2].object.name, "DELTA_STREAM");
    assert_eq!(objects[2].owner_table.as_deref(), Some("ORDERS"));
    assert_eq!(objects[2].signature.as_deref(), Some("DEFAULT"));

    assert_eq!(objects[3].object.kind, DatabaseObjectKind::Procedure);
    assert_eq!(objects[3].object.name, "CALC_TAX");
    assert_eq!(objects[3].object.identity.as_deref(), Some("(ID NUMBER)"));
    assert_eq!(
        objects[3].signature.as_deref(),
        Some("(ID NUMBER) -> NUMBER")
    );

    assert_eq!(objects[4].object.kind, DatabaseObjectKind::Function);
    assert_eq!(objects[4].object.name, "ADD_ONE");
    assert_eq!(objects[4].object.identity.as_deref(), Some("(X NUMBER)"));
    assert_eq!(
        objects[4].signature.as_deref(),
        Some("(X NUMBER) -> NUMBER")
    );

    assert_eq!(objects[5].object.kind, DatabaseObjectKind::Sequence);
    assert_eq!(objects[5].object.name, "ORDER_SEQ");
    assert_eq!(objects[5].signature.as_deref(), Some("NUMBER(38,0)"));

    assert_eq!(objects[6].object.kind, DatabaseObjectKind::FileFormat);
    assert_eq!(objects[6].object.name, "CSV_FORMAT");
    assert_eq!(objects[6].signature.as_deref(), Some("CSV"));

    assert_eq!(server.await.unwrap().len(), 8);
}

#[tokio::test]
async fn test_adversarial_catalog_objects_all_categories_fail() {
    let err_resp = json!({
        "message": "User not authorized",
        "sqlState": "02000"
    });
    let (driver, server) = fixture(vec![
        err_resp.clone(),
        err_resp.clone(),
        err_resp.clone(),
        err_resp.clone(),
        err_resp.clone(),
        err_resp.clone(),
        err_resp.clone(),
        err_resp.clone(),
    ])
    .await;

    let objects = driver.catalog_objects().await.unwrap();
    assert_eq!(objects.len(), 0);
    assert_eq!(server.await.unwrap().len(), 8);
}

#[tokio::test]
async fn test_adversarial_fetch_snowflake_ddl_special_chars_and_quotes() {
    let ddl_text = "CREATE TABLE \"DB\".\"SPECIAL SCHEMA\".\"O'Reilly\"\"s Table\" (ID INT);";
    let (driver, server) = fixture(vec![make_metadata_response(
        &["GET_DDL"],
        vec![vec![Some(ddl_text)]],
    )])
    .await;

    let obj = DatabaseObjectRef {
        schema: "SPECIAL SCHEMA".into(),
        name: "O'Reilly\"s Table".into(),
        kind: DatabaseObjectKind::Table,
        identity: None,
    };

    let details = driver.get_object_details(&obj).await.unwrap();
    match details.ddl {
        ObjectDdlStatus::Available { ddl, truncated } => {
            assert_eq!(ddl, ddl_text);
            assert!(!truncated);
        }
        _ => panic!("Expected Available DDL"),
    }

    let reqs = server.await.unwrap();
    let sent_sql = reqs[0].1["statement"].as_str().unwrap();
    assert_eq!(
        sent_sql,
        "SELECT GET_DDL('TABLE', '\"DB\".\"SPECIAL SCHEMA\".\"O''Reilly\"\"s Table\"')"
    );
}

#[tokio::test]
async fn test_adversarial_fetch_snowflake_ddl_routine_overloads_with_signature() {
    let ddl_text = "CREATE PROCEDURE \"DB\".\"PUBLIC\".\"CALC\"(ID NUMBER, OPT NUMBER) RETURNS NUMBER AS 'begin return 1; end;'";
    let (driver, server) = fixture(vec![make_metadata_response(
        &["GET_DDL"],
        vec![vec![Some(ddl_text)]],
    )])
    .await;

    let obj = DatabaseObjectRef {
        schema: "PUBLIC".into(),
        name: "CALC".into(),
        kind: DatabaseObjectKind::Procedure,
        identity: Some("(ID NUMBER, OPT NUMBER)".into()),
    };

    let details = driver.get_object_details(&obj).await.unwrap();
    match details.ddl {
        ObjectDdlStatus::Available { ddl, .. } => {
            assert!(ddl.ends_with(';'));
        }
        _ => panic!("Expected Available DDL"),
    }

    let reqs = server.await.unwrap();
    let sent_sql = reqs[0].1["statement"].as_str().unwrap();
    assert_eq!(
        sent_sql,
        "SELECT GET_DDL('PROCEDURE', '\"DB\".\"PUBLIC\".\"CALC\"(ID NUMBER, OPT NUMBER)')"
    );
}

#[tokio::test]
async fn test_adversarial_fetch_snowflake_ddl_routine_fallback_chain() {
    // 1. Primary target with db and signature fails (422)
    // 2. Schema-qualified target with signature fails (422)
    // 3. Fallback to no-signature succeeds (200)
    let ddl_text = "CREATE PROCEDURE \"DB\".\"PUBLIC\".\"RUN_JOB\"() AS 'begin null; end;'";
    let (driver, server) = fixture(vec![
        json!({"message": "Invalid signature", "sqlState": "42000"}),
        json!({"message": "Invalid signature schema", "sqlState": "42000"}),
        make_metadata_response(&["GET_DDL"], vec![vec![Some(ddl_text)]]),
    ])
    .await;

    let obj = DatabaseObjectRef {
        schema: "PUBLIC".into(),
        name: "RUN_JOB".into(),
        kind: DatabaseObjectKind::Procedure,
        identity: Some("(JOB_ID VARCHAR)".into()),
    };

    let details = driver.get_object_details(&obj).await.unwrap();
    match details.ddl {
        ObjectDdlStatus::Available { ddl, .. } => {
            assert_eq!(ddl, format!("{ddl_text};"));
        }
        _ => panic!("Expected Available DDL via fallback"),
    }

    let reqs = server.await.unwrap();
    assert_eq!(reqs.len(), 3);
    assert_eq!(
        reqs[0].1["statement"],
        "SELECT GET_DDL('PROCEDURE', '\"DB\".\"PUBLIC\".\"RUN_JOB\"(JOB_ID VARCHAR)')"
    );
    assert_eq!(
        reqs[1].1["statement"],
        "SELECT GET_DDL('PROCEDURE', '\"PUBLIC\".\"RUN_JOB\"(JOB_ID VARCHAR)')"
    );
    assert_eq!(
        reqs[2].1["statement"],
        "SELECT GET_DDL('PROCEDURE', '\"DB\".\"PUBLIC\".\"RUN_JOB\"')"
    );
}

#[tokio::test]
async fn test_adversarial_fetch_snowflake_ddl_insufficient_privileges_returns_unavailable() {
    let (driver, server) = fixture(vec![
        json!({"message": "Insufficient privileges to operate on task 'SECRET_TASK'", "sqlState": "02000"}),
        json!({"message": "Insufficient privileges to operate on task 'SECRET_TASK'", "sqlState": "02000"}),
    ])
    .await;

    let obj = DatabaseObjectRef {
        schema: "PUBLIC".into(),
        name: "SECRET_TASK".into(),
        kind: DatabaseObjectKind::Task,
        identity: None,
    };

    let details = driver.get_object_details(&obj).await.unwrap();
    match details.ddl {
        ObjectDdlStatus::Unavailable { reason } => {
            assert!(reason.contains("Insufficient privileges"));
        }
        _ => panic!("Expected Unavailable DDL"),
    }

    let reqs = server.await.unwrap();
    assert_eq!(reqs.len(), 2);
}

#[tokio::test]
async fn test_adversarial_fetch_snowflake_ddl_empty_ddl_returns_unavailable() {
    let (driver, server) =
        fixture(vec![make_metadata_response(&["GET_DDL"], vec![vec![None]])]).await;

    let obj = DatabaseObjectRef {
        schema: "PUBLIC".into(),
        name: "EMPTY_OBJ".into(),
        kind: DatabaseObjectKind::Table,
        identity: None,
    };

    let details = driver.get_object_details(&obj).await.unwrap();
    match details.ddl {
        ObjectDdlStatus::Unavailable { reason } => {
            assert_eq!(reason, "Snowflake returned empty DDL");
        }
        _ => panic!("Expected Unavailable DDL"),
    }

    let reqs = server.await.unwrap();
    assert_eq!(reqs.len(), 1);
}

#[tokio::test]
async fn test_adversarial_fetch_snowflake_ddl_unsupported_kinds_no_network() {
    let (driver, server) = fixture(vec![]).await;

    let obj = DatabaseObjectRef {
        schema: "PUBLIC".into(),
        name: "MY_IDX".into(),
        kind: DatabaseObjectKind::Index,
        identity: None,
    };

    let details = driver.get_object_details(&obj).await.unwrap();
    match details.ddl {
        ObjectDdlStatus::Unavailable { reason } => {
            assert!(reason.contains("Snowflake does not support DDL for Index"));
        }
        _ => panic!("Expected Unavailable DDL"),
    }

    let reqs = server.await.unwrap();
    assert_eq!(reqs.len(), 0);
}

#[tokio::test]
async fn test_adversarial_fetch_snowflake_ddl_invalid_identifiers() {
    let (driver, server) = fixture(vec![]).await;

    // Empty object name
    let obj = DatabaseObjectRef {
        schema: "PUBLIC".into(),
        name: "".into(),
        kind: DatabaseObjectKind::Table,
        identity: None,
    };

    let details = driver.get_object_details(&obj).await.unwrap();
    match details.ddl {
        ObjectDdlStatus::Unavailable { reason } => {
            assert!(reason.contains("Invalid Snowflake object identifier"));
        }
        _ => panic!("Expected Unavailable DDL"),
    }

    // Object name with null byte
    let obj_null = DatabaseObjectRef {
        schema: "PUBLIC".into(),
        name: "BAD\0NAME".into(),
        kind: DatabaseObjectKind::Table,
        identity: None,
    };

    let details_null = driver.get_object_details(&obj_null).await.unwrap();
    match details_null.ddl {
        ObjectDdlStatus::Unavailable { reason } => {
            assert!(reason.contains("Invalid Snowflake object identifier"));
        }
        _ => panic!("Expected Unavailable DDL"),
    }

    let reqs = server.await.unwrap();
    assert_eq!(reqs.len(), 0);
}

#[test]
fn test_adversarial_parse_snowflake_use_whitespace_comments_and_quotes() {
    use super::{SnowflakeUseTarget, parse_snowflake_use_statements};

    // 1. Quoted identifier with spaces and special chars
    let sql1 = r#"USE WAREHOUSE "Analytics WH";"#;
    let targets1 = parse_snowflake_use_statements(sql1);
    assert_eq!(
        targets1,
        vec![SnowflakeUseTarget::Warehouse("Analytics WH".into())]
    );

    // 2. Unquoted identifier: normalized to uppercase
    let sql2 = "use role admin_role;";
    let targets2 = parse_snowflake_use_statements(sql2);
    assert_eq!(
        targets2,
        vec![SnowflakeUseTarget::Role("ADMIN_ROLE".into())]
    );

    // 3. Comments before, after, and inside
    let sql3 = r#"/* leading comment */
    USE DATABASE "PROD_DB"; -- trailing comment"#;
    let targets3 = parse_snowflake_use_statements(sql3);
    assert_eq!(
        targets3,
        vec![SnowflakeUseTarget::Database("PROD_DB".into())]
    );

    // 4. USE SCHEMA <db>.<schema>
    let sql4 = r#"USE SCHEMA "MY_DB"."PUBLIC";"#;
    let targets4 = parse_snowflake_use_statements(sql4);
    assert_eq!(
        targets4,
        vec![SnowflakeUseTarget::Schema {
            database: Some("MY_DB".into()),
            schema: "PUBLIC".into(),
        }]
    );

    // 5. USE SCHEMA <schema> (unqualified)
    let sql5 = "USE SCHEMA staging;";
    let targets5 = parse_snowflake_use_statements(sql5);
    assert_eq!(
        targets5,
        vec![SnowflakeUseTarget::Schema {
            database: None,
            schema: "STAGING".into(),
        }]
    );

    // 6. Shorthand USE <db>.<schema>
    let sql6 = r#"USE "ANALYTICS_DB"."REPORTING";"#;
    let targets6 = parse_snowflake_use_statements(sql6);
    assert_eq!(
        targets6,
        vec![SnowflakeUseTarget::DatabaseAndSchema {
            database: "ANALYTICS_DB".into(),
            schema: "REPORTING".into(),
        }]
    );

    // 7. Shorthand USE <db>
    let sql7 = r#"USE "RAW_DATA";"#;
    let targets7 = parse_snowflake_use_statements(sql7);
    assert_eq!(
        targets7,
        vec![SnowflakeUseTarget::Database("RAW_DATA".into())]
    );

    // 8. USE SECONDARY ROLES (unsupported)
    let sql8 = "USE SECONDARY ROLES ALL; USE SECONDARY ROLES NONE;";
    let targets8 = parse_snowflake_use_statements(sql8);
    assert_eq!(
        targets8,
        vec![
            SnowflakeUseTarget::Unsupported,
            SnowflakeUseTarget::Unsupported
        ]
    );

    // 9. Multi-statement chained batch with mixed casing & formatting
    let sql9 = r#"
        -- Step 1: switch role
        USE ROLE "DataEngineer";
        /* Step 2 */ USE WAREHOUSE COMPUTE_WH;
        USE DATABASE PROD_WAREHOUSE;
        USE SCHEMA "PROD_WAREHOUSE"."ANALYTICS";
    "#;
    let targets9 = parse_snowflake_use_statements(sql9);
    assert_eq!(
        targets9,
        vec![
            SnowflakeUseTarget::Role("DataEngineer".into()),
            SnowflakeUseTarget::Warehouse("COMPUTE_WH".into()),
            SnowflakeUseTarget::Database("PROD_WAREHOUSE".into()),
            SnowflakeUseTarget::Schema {
                database: Some("PROD_WAREHOUSE".into()),
                schema: "ANALYTICS".into(),
            },
        ]
    );

    // 10. Interleaved non-USE statements: only USE statements are extracted
    let sql10 = "SELECT 1; USE ROLE AUDITOR; SELECT * FROM t; USE WAREHOUSE QUERY_WH;";
    let targets10 = parse_snowflake_use_statements(sql10);
    assert_eq!(
        targets10,
        vec![
            SnowflakeUseTarget::Role("AUDITOR".into()),
            SnowflakeUseTarget::Warehouse("QUERY_WH".into()),
        ]
    );

    // 11. Malformed syntax, empty or incomplete USE
    assert!(parse_snowflake_use_statements("").is_empty());
    assert!(parse_snowflake_use_statements("   \n\t  ").is_empty());
    assert!(parse_snowflake_use_statements("USE;").is_empty());
    assert!(parse_snowflake_use_statements("USE ROLE;").is_empty());
    assert!(parse_snowflake_use_statements("USE WAREHOUSE;").is_empty());
    assert!(parse_snowflake_use_statements("USE DATABASE;").is_empty());
    assert!(parse_snowflake_use_statements("USE SCHEMA;").is_empty());
}

#[tokio::test]
async fn test_adversarial_parse_and_sync_snowflake_use_driver_state() {
    use super::parse_and_sync_snowflake_use;

    let (driver, server) = fixture(vec![]).await;
    assert_eq!(driver.current_role(), "READ_ONLY");
    assert_eq!(driver.current_warehouse(), "WH");
    assert_eq!(driver.current_database(), "DB");
    assert_eq!(driver.current_schema(), "PUBLIC");

    // Execute multi-statement USE sync
    let sync_sql = r#"
        /* Comment */ USE WAREHOUSE "Heavy Compute";
        USE ROLE "SecurityAdmin";
        USE DATABASE "ANALYTICS_PROD";
        USE SCHEMA "CORE";
    "#;
    parse_and_sync_snowflake_use(&driver, sync_sql);

    assert_eq!(driver.current_warehouse(), "Heavy Compute");
    assert_eq!(driver.current_role(), "SecurityAdmin");
    assert_eq!(driver.current_database(), "ANALYTICS_PROD");
    assert_eq!(driver.current_schema(), "CORE");

    // Shorthand USE DB.SCHEMA
    parse_and_sync_snowflake_use(&driver, r#"USE "NEW_DB"."NEW_SCHEMA";"#);
    assert_eq!(driver.current_database(), "NEW_DB");
    assert_eq!(driver.current_schema(), "NEW_SCHEMA");

    // Unsupported USE SECONDARY ROLES does not change existing state
    parse_and_sync_snowflake_use(&driver, "USE SECONDARY ROLES ALL;");
    assert_eq!(driver.current_database(), "NEW_DB");
    assert_eq!(driver.current_schema(), "NEW_SCHEMA");
    assert_eq!(driver.current_role(), "SecurityAdmin");

    assert!(server.await.unwrap().is_empty());
}

#[tokio::test]
async fn test_adversarial_switch_session_context_4d_deadlock_and_lock_sequencing() {
    let (driver, server) = fixture(vec![
        // 1. USE WAREHOUSE
        json!({"resultSetMetaData":{"numRows":0},"data":[]}),
        // 2. USE ROLE
        json!({"resultSetMetaData":{"numRows":0},"data":[]}),
        // 3. USE DATABASE
        json!({"resultSetMetaData":{"numRows":0},"data":[]}),
        // 4. USE SCHEMA
        json!({"resultSetMetaData":{"numRows":0},"data":[]}),
        // 5. list_warehouses (SHOW WAREHOUSES)
        json!({"resultSetMetaData":{"numRows":1,"rowType":[{"name":"name","type":"text"}]},"data":[["WH_NEW"]]}),
        // 6. list_roles (SHOW ROLES)
        json!({"resultSetMetaData":{"numRows":1,"rowType":[{"name":"name","type":"text"}]},"data":[["ROLE_NEW"]]}),
        // 7. list_databases (SHOW DATABASES)
        json!({"resultSetMetaData":{"numRows":1,"rowType":[{"name":"name","type":"text"}]},"data":[["DB_NEW"]]}),
        // 8. list_schemas (SELECT SCHEMA_NAME FROM ...)
        json!({"resultSetMetaData":{"numRows":1,"rowType":[{"name":"SCHEMA_NAME","type":"text"}]},"data":[["SCHEMA_NEW"]]}),
    ]).await;

    // Switch all 4 dimensions simultaneously:
    let result = driver
        .switch_session_context(
            None,
            Some("WH_NEW"),
            Some("ROLE_NEW"),
            Some("DB_NEW"),
            Some("SCHEMA_NEW"),
        )
        .await
        .expect("4-D context switch must succeed without deadlock");

    assert_eq!(result.active_warehouse.as_deref(), Some("WH_NEW"));
    assert_eq!(result.active_role.as_deref(), Some("ROLE_NEW"));
    assert_eq!(result.active_database.as_deref(), Some("DB_NEW"));
    assert_eq!(result.active_schema.as_deref(), Some("SCHEMA_NEW"));

    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 8);
    assert_eq!(requests[0].1["statement"], "USE WAREHOUSE \"WH_NEW\"");
    assert_eq!(requests[1].1["statement"], "USE ROLE \"ROLE_NEW\"");
    assert_eq!(requests[2].1["statement"], "USE DATABASE \"DB_NEW\"");
    assert_eq!(requests[3].1["statement"], "USE SCHEMA \"SCHEMA_NEW\"");
}

#[tokio::test]
async fn test_adversarial_switch_session_context_partial_failure_preserves_atomic_guarantees() {
    let (driver, server) = fixture(vec![
        // 1. USE WAREHOUSE succeeds
        json!({"resultSetMetaData":{"numRows":0},"data":[]}),
        // 2. USE ROLE fails (insufficient privileges)
        json!({
            "message": "Role 'FORBIDDEN_ROLE' does not exist or not authorized",
            "sqlState": "42000"
        }),
    ])
    .await;

    let initial_role = driver.current_role();
    let err = driver
        .switch_session_context(None, Some("WH_NEW"), Some("FORBIDDEN_ROLE"), None, None)
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::Database);
    assert!(err.message.contains("Role 'FORBIDDEN_ROLE' does not exist"));

    // Warehouse succeeded, so warehouse was updated:
    assert_eq!(driver.current_warehouse(), "WH_NEW");
    // Role failed, so role was NOT updated:
    assert_eq!(driver.current_role(), initial_role);

    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test]
async fn test_adversarial_switch_session_context_quoted_identifiers_escaping() {
    let (driver, server) = fixture(vec![
        // 1. USE WAREHOUSE with quotes and spaces
        json!({"resultSetMetaData":{"numRows":0},"data":[]}),
        // 2. SHOW WAREHOUSES
        json!({"resultSetMetaData":{"numRows":1,"rowType":[{"name":"name","type":"text"}]},"data":[["WH \"PROD\" 1"]]}),
        // 3. SHOW ROLES
        json!({"resultSetMetaData":{"numRows":0,"data":[]}}),
        // 4. SHOW DATABASES
        json!({"resultSetMetaData":{"numRows":0,"data":[]}}),
        // 5. SHOW SCHEMAS
        json!({"resultSetMetaData":{"numRows":0,"data":[]}}),
    ]).await;

    let tricky_wh = "WH \"PROD\" 1";
    let res = driver
        .switch_session_context(None, Some(tricky_wh), None, None, None)
        .await
        .unwrap();

    assert_eq!(res.active_warehouse.as_deref(), Some(tricky_wh));
    let requests = server.await.unwrap();
    // Identifier must escape internal double quotes by doubling them
    assert_eq!(
        requests[0].1["statement"],
        "USE WAREHOUSE \"WH \"\"PROD\"\" 1\""
    );
}

#[tokio::test]
async fn test_adversarial_catalog_objects_bounding_at_10000() {
    // Generate 2000 rows for 6 categories (total 12,000 objects > 10,000 bound)
    let make_rows = |prefix: &'static str, count: usize| -> Vec<Vec<Option<&str>>> {
        (0..count)
            .map(|i| {
                vec![
                    Some("PUBLIC"),
                    Some(Box::leak(format!("{prefix}_{i}").into_boxed_str()) as &str),
                    Some("TEXT"),
                    None,
                ]
            })
            .collect()
    };

    let stages_resp = make_metadata_response(
        &["STAGE_SCHEMA", "STAGE_NAME", "STAGE_TYPE", "COMMENT"],
        make_rows("STAGE", 2000),
    );
    let tasks_resp = make_metadata_response(
        &["TASK_SCHEMA", "TASK_NAME", "SCHEDULE", "STATE", "COMMENT"],
        (0..2000)
            .map(|i| {
                vec![
                    Some("PUBLIC"),
                    Some(Box::leak(format!("TASK_{i}").into_boxed_str()) as &str),
                    Some("USING CRON 0 * * * *"),
                    Some("STARTED"),
                    None,
                ]
            })
            .collect(),
    );
    let streams_resp = make_metadata_response(
        &[
            "STREAM_SCHEMA",
            "STREAM_NAME",
            "TABLE_NAME",
            "MODE",
            "COMMENT",
        ],
        (0..2000)
            .map(|i| {
                vec![
                    Some("PUBLIC"),
                    Some(Box::leak(format!("STREAM_{i}").into_boxed_str()) as &str),
                    Some("T"),
                    Some("DEFAULT"),
                    None,
                ]
            })
            .collect(),
    );
    let pipes_resp = make_metadata_response(
        &["PIPE_SCHEMA", "PIPE_NAME", "COMMENT"],
        (0..2000)
            .map(|i| {
                vec![
                    Some("PUBLIC"),
                    Some(Box::leak(format!("PIPE_{i}").into_boxed_str()) as &str),
                    None,
                ]
            })
            .collect(),
    );
    let procs_resp = make_metadata_response(
        &[
            "PROCEDURE_SCHEMA",
            "PROCEDURE_NAME",
            "ARGUMENT_SIGNATURE",
            "DATA_TYPE",
            "COMMENT",
        ],
        (0..2000)
            .map(|i| {
                vec![
                    Some("PUBLIC"),
                    Some(Box::leak(format!("PROC_{i}").into_boxed_str()) as &str),
                    Some("()"),
                    Some("VARCHAR"),
                    None,
                ]
            })
            .collect(),
    );
    let funcs_resp = make_metadata_response(
        &[
            "FUNCTION_SCHEMA",
            "FUNCTION_NAME",
            "ARGUMENT_SIGNATURE",
            "DATA_TYPE",
            "COMMENT",
        ],
        (0..2000)
            .map(|i| {
                vec![
                    Some("PUBLIC"),
                    Some(Box::leak(format!("FUNC_{i}").into_boxed_str()) as &str),
                    Some("()"),
                    Some("VARCHAR"),
                    None,
                ]
            })
            .collect(),
    );
    let seqs_resp = make_metadata_response(
        &["SEQUENCE_SCHEMA", "SEQUENCE_NAME", "DATA_TYPE", "COMMENT"],
        vec![],
    );
    let formats_resp = make_metadata_response(
        &[
            "FILE_FORMAT_SCHEMA",
            "FILE_FORMAT_NAME",
            "FILE_FORMAT_TYPE",
            "COMMENT",
        ],
        vec![],
    );

    let (driver, server) = fixture(vec![
        stages_resp,
        tasks_resp,
        streams_resp,
        pipes_resp,
        procs_resp,
        funcs_resp,
        seqs_resp,
        formats_resp,
    ])
    .await;

    let objects = driver.catalog_objects().await.unwrap();
    // Must be capped at exactly 10,000
    assert_eq!(objects.len(), 10_000);
    assert_eq!(server.await.unwrap().len(), 8);
}
