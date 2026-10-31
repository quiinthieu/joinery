use super::{BrowseRows, CompletionRelations, Driver, QuerySink, RowMutation};
use crate::domain::*;
use async_trait::async_trait;
use parking_lot::Mutex;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use url::Url;
use zeroize::Zeroizing;

use super::snowflake_auth::{self, Tokens};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PARTITIONS: usize = 64;

mod catalog;
#[cfg(test)]
mod catalog_tests;
mod completion;
#[cfg(test)]
mod compression_tests;
#[cfg(test)]
mod lifecycle_tests;

struct AuthState {
    tokens: Tokens,
    expires_at: Instant,
}

pub struct Snowflake {
    client: Client,
    base: Url,
    database: String,
    schema: String,
    role: String,
    warehouse: String,
    query_tag: String,
    auth: AsyncMutex<AuthState>,
    metadata_lane: AsyncMutex<()>,
    active_handle: Mutex<Option<String>>,
    // Cancellation retires the Studio session, so neither signal is reset.
    cancel_requested: CancellationToken,
    closed: CancellationToken,
}

fn account_url(host: &str) -> AppResult<Url> {
    let host = host.trim().to_ascii_lowercase();
    if host.len() > 253
        || !host.ends_with(".snowflakecomputing.com")
        || !host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        || host
            .split('.')
            .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
    {
        return Err(AppError::invalid(
            "Enter a Snowflake account hostname ending in .snowflakecomputing.com",
        ));
    }
    Url::parse(&format!("https://{host}/"))
        .map_err(|_| AppError::invalid("Invalid Snowflake account hostname"))
}

fn simple_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
        && value.len() <= 255
        && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

impl Snowflake {
    pub fn validate_profile(profile: &ConnectionProfile) -> AppResult<()> {
        let config = profile
            .snowflake
            .as_ref()
            .ok_or_else(|| AppError::invalid("Set the Snowflake role, warehouse, and schema"))?;
        if [
            config.role.as_str(),
            config.warehouse.as_str(),
            config.schema.as_str(),
            profile.database.as_str(),
        ]
        .iter()
        .any(|v| !simple_identifier(v))
        {
            return Err(AppError::invalid(
                "Use uppercase unquoted Snowflake role, warehouse, database, and schema names",
            ));
        }
        if profile.port != 443
            || profile.tls_mode != TlsMode::VerifyFull
            || profile.ssh.is_some()
            || profile.remember_password
            || profile.safe_mode != SafeMode::ReadOnly
            || profile.ca_cert_path.is_some()
        {
            return Err(AppError::invalid(
                "Snowflake requires read-only mode and direct verified HTTPS on port 443 without a stored password",
            ));
        }
        account_url(&profile.host)?;
        Ok(())
    }

    pub async fn open(
        profile: &ConnectionProfile,
        session_id: &str,
    ) -> AppResult<(Arc<Self>, SessionInfo)> {
        let started = Instant::now();
        Self::validate_profile(profile)?;
        let config = profile
            .snowflake
            .as_ref()
            .expect("validated Snowflake config");
        let base = account_url(&profile.host)?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(70))
            .build()
            .map_err(|_| AppError::internal("Could not initialize Snowflake HTTPS"))?;
        let tokens = snowflake_auth::sign_in(&client, &base, &config.role).await?;
        let expires_at = Instant::now() + Duration::from_secs(tokens.expires_in);
        let driver = Arc::new(Self {
            client,
            base,
            database: profile.database.clone(),
            schema: config.schema.clone(),
            role: config.role.clone(),
            warehouse: config.warehouse.clone(),
            query_tag: format!("Joinery/{session_id}"),
            auth: AsyncMutex::new(AuthState { tokens, expires_at }),
            metadata_lane: AsyncMutex::new(()),
            active_handle: Mutex::new(None),
            cancel_requested: CancellationToken::new(),
            closed: CancellationToken::new(),
        });
        let identity = driver.query_rows("SELECT CURRENT_VERSION(), CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE()", 1).await?;
        let row = identity.first().ok_or_else(|| {
            AppError::new(
                ErrorCode::ConnectionFailed,
                "Snowflake did not return session identity",
            )
        })?;
        if row.len() != 6
            || !same(&row[1], &profile.username)
            || !same(&row[2], &config.role)
            || !same(&row[3], &profile.database)
            || !same(&row[4], &config.schema)
            || !same(&row[5], &config.warehouse)
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake connected with a different user, role, database, schema, or warehouse",
            ));
        }
        let info = SessionInfo {
            id: session_id.into(),
            connection_id: profile.id.clone(),
            engine: EngineKind::Snowflake,
            server_version: row[0].clone().unwrap_or_else(|| "Snowflake".into()),
            latency_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
            encrypted: true,
            capabilities: EngineCapabilities {
                sql: true,
                transactions: false,
                cancellation: true,
                table_browsing: true,
                table_editing: false,
                table_filtering: false,
                processes: false,
                object_explorer: false,
            },
        };
        Ok((driver, info))
    }

    async fn access_token(&self) -> AppResult<Zeroizing<String>> {
        let mut auth = self.auth.lock().await;
        let margin = Duration::from_secs((auth.tokens.expires_in / 10).min(30));
        if auth.expires_at <= Instant::now() + margin {
            let Some(refresh) = auth.tokens.refresh.as_ref() else {
                if auth.expires_at > Instant::now() {
                    return Ok(Zeroizing::new(auth.tokens.access.to_string()));
                }
                self.close();
                return Err(Self::sign_in_expired());
            };
            let mut next = match snowflake_auth::refresh(&self.client, &self.base, refresh).await {
                Ok(next) => next,
                Err(error) if error.code == ErrorCode::PermissionDenied => {
                    self.close();
                    return Err(Self::sign_in_expired());
                }
                Err(error) => return Err(error),
            };
            if next.refresh.is_none() {
                next.refresh = auth.tokens.refresh.take();
            }
            let expires_at = Instant::now() + Duration::from_secs(next.expires_in);
            auth.tokens = next;
            auth.expires_at = expires_at;
        }
        Ok(Zeroizing::new(auth.tokens.access.to_string()))
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> AppResult<(StatusCode, Value)> {
        self.request_scoped(method, path, body, false).await
    }

    async fn request_scoped(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        query_submission: bool,
    ) -> AppResult<(StatusCode, Value)> {
        tokio::select! {
            biased;
            _ = self.closed.cancelled() => Err(Self::sign_in_expired()),
            result = self.request_inner(method, path, body, query_submission) => result,
        }
    }

    fn sign_in_expired() -> AppError {
        AppError::new(
            ErrorCode::ConnectionClosed,
            "Snowflake session closed or sign-in expired. Reconnect to sign in again.",
        )
    }

    fn cancelled() -> AppError {
        AppError::new(ErrorCode::Cancelled, "Snowflake query cancelled")
    }

    async fn query_request(&self, path: &str, track_query: bool) -> AppResult<(StatusCode, Value)> {
        tokio::select! {
            biased;
            _ = self.cancel_requested.cancelled(), if track_query => Err(Self::cancelled()),
            result = self.request(Method::GET, path, None) => result,
        }
    }

    async fn request_inner(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        query_submission: bool,
    ) -> AppResult<(StatusCode, Value)> {
        let url = self
            .base
            .join(path)
            .map_err(|_| AppError::internal("Invalid Snowflake request path"))?;
        let token = self.access_token().await?;
        if query_submission && self.cancel_requested.is_cancelled() {
            return Err(Self::cancelled());
        }
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(token.as_str())
            .header("X-Snowflake-Authorization-Token-Type", "OAUTH")
            .header("Accept", "application/json")
            .header("User-Agent", "Joinery/0.1");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| AppError::new(ErrorCode::ConnectionFailed, "Snowflake request failed"))?;
        let status = response.status();
        // Do not replay a submitted statement after an authentication failure.
        // The server's error body can contain account or token details.
        if status == StatusCode::UNAUTHORIZED {
            self.close();
            return Err(Self::sign_in_expired());
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Snowflake result partition exceeds the 16 MiB response limit",
            ));
        }
        // Reqwest decodes Snowflake's gzip partitions before yielding chunks,
        // so this bound applies to decompressed data, not just the wire size.
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            AppError::new(
                ErrorCode::ConnectionFailed,
                "Snowflake response was interrupted",
            )
        })? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "Snowflake result partition exceeds the 16 MiB response limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
            AppError::new(
                ErrorCode::Database,
                "Snowflake returned an invalid response",
            )
        })?;
        if !status.is_success() {
            let detail = value
                .get("message")
                .and_then(Value::as_str)
                .map(|message| {
                    message
                        .chars()
                        .filter(|c| !c.is_control() || *c == '\n')
                        .take(500)
                        .collect::<String>()
                })
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| "Check account access, role, warehouse, and SQL.".into());
            return Err(AppError::new(
                ErrorCode::Database,
                format!("Snowflake rejected the request: {detail}"),
            ));
        }
        Ok((status, value))
    }

    async fn submit(&self, sql: &str, track_query: bool) -> AppResult<Value> {
        let body = json!({"statement": sql, "timeout": 60, "database": self.database,
            "schema": self.schema, "warehouse": self.warehouse, "role": self.role,
            "parameters": {"QUERY_TAG": self.query_tag}});
        let (mut status, mut value) = self
            .request_scoped(
                Method::POST,
                "api/v2/statements?async=true",
                Some(body),
                track_query,
            )
            .await?;
        for _ in 0..120 {
            if let Some(handle) = value.get("statementHandle").and_then(Value::as_str) {
                let handle = uuid::Uuid::parse_str(handle)
                    .map_err(|_| {
                        AppError::new(
                            ErrorCode::Database,
                            "Snowflake returned an invalid statement handle",
                        )
                    })?
                    .to_string();
                if track_query {
                    *self.active_handle.lock() = Some(handle.clone());
                }
                if track_query && self.cancel_requested.is_cancelled() {
                    let _ = self
                        .request(
                            Method::POST,
                            &format!("api/v2/statements/{handle}/cancel"),
                            None,
                        )
                        .await;
                    return Err(Self::cancelled());
                }
                if status == StatusCode::ACCEPTED {
                    tokio::select! {
                        biased;
                        _ = self.cancel_requested.cancelled(), if track_query => return Err(Self::cancelled()),
                        _ = self.closed.cancelled() => return Err(Self::sign_in_expired()),
                        _ = tokio::time::sleep(Duration::from_millis(500)) => (),
                    }
                    (status, value) = self
                        .query_request(
                            &format!("api/v2/statements/{handle}?partition=0"),
                            track_query,
                        )
                        .await?;
                    continue;
                }
            }
            if status == StatusCode::OK {
                return Ok(value);
            }
            return Err(AppError::new(
                ErrorCode::Database,
                "Snowflake returned an unexpected query status",
            ));
        }
        Err(AppError::new(
            ErrorCode::Database,
            "Snowflake query exceeded the polling limit",
        ))
    }
}

fn same(value: &Option<String>, expected: &str) -> bool {
    value
        .as_deref()
        .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
}

fn decode_rows(value: &Value) -> AppResult<Vec<TextRow>> {
    let rows = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::new(ErrorCode::Database, "Snowflake result has no row data"))?;
    rows.iter()
        .map(|row| {
            let cells = row.as_array().ok_or_else(|| {
                AppError::new(ErrorCode::Database, "Snowflake returned a malformed row")
            })?;
            cells
                .iter()
                .map(|cell| match cell {
                    Value::Null => Ok(None),
                    Value::String(text) => Ok(Some(text.clone())),
                    Value::Bool(_) | Value::Number(_) => Ok(Some(cell.to_string())),
                    _ => Err(AppError::new(
                        ErrorCode::Database,
                        "Snowflake returned an unsupported result value",
                    )),
                })
                .collect()
        })
        .collect()
}

fn columns(value: &Value) -> AppResult<Vec<ColumnInfo>> {
    let Some(types) = value
        .pointer("/resultSetMetaData/rowType")
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };
    if types.len() > 1000 {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            "Snowflake returned too many columns",
        ));
    }
    types
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let name = ty.get("name").and_then(Value::as_str).ok_or_else(|| {
                AppError::new(ErrorCode::Database, "Snowflake result column has no name")
            })?;
            let database_type = ty
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN")
                .to_string();
            let kind = match database_type.to_ascii_uppercase().as_str() {
                "FIXED" if ty.get("scale").and_then(Value::as_i64).unwrap_or(0) == 0 => {
                    ColumnKind::Integer
                }
                "FIXED" | "REAL" => ColumnKind::Decimal,
                "BOOLEAN" => ColumnKind::Boolean,
                "DATE" | "TIME" | "TIMESTAMP_LTZ" | "TIMESTAMP_NTZ" | "TIMESTAMP_TZ" => {
                    ColumnKind::Temporal
                }
                "VARIANT" | "OBJECT" | "ARRAY" => ColumnKind::Json,
                "BINARY" => ColumnKind::Binary,
                _ => ColumnKind::Text,
            };
            Ok(ColumnInfo {
                id: format!("c{index}"),
                name: name.into(),
                database_type,
                kind,
                nullable: ty.get("nullable").and_then(Value::as_bool).unwrap_or(true),
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: false,
            })
        })
        .collect()
}

#[async_trait]
impl Driver for Snowflake {
    async fn execute(
        &self,
        sql: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        if kind != StatementKind::Read {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake connections currently run read-only queries only",
            ));
        }
        if self.cancel_requested.is_cancelled() {
            return Err(Self::cancelled());
        }
        let outcome = async {
            let first = self.submit(sql, true).await?;
            let columns = columns(&first)?;
            sink.describe(columns)?;
            let mut count = 0u64;
            let partitions = first
                .pointer("/resultSetMetaData/partitionInfo")
                .and_then(Value::as_array)
                .map_or(1, Vec::len);
            if partitions > MAX_PARTITIONS {
                sink.limit("Snowflake result has too many partitions");
            }
            for index in 0..partitions.min(MAX_PARTITIONS) {
                if self.cancel_requested.is_cancelled() {
                    return Err(Self::cancelled());
                }
                let value = if index == 0 {
                    first.clone()
                } else {
                    let handle = self.active_handle.lock().clone().ok_or_else(|| {
                        AppError::new(ErrorCode::Database, "Missing Snowflake result handle")
                    })?;
                    self.query_request(
                        &format!("api/v2/statements/{handle}?partition={index}"),
                        true,
                    )
                    .await?
                    .1
                };
                let rows = decode_rows(&value)?;
                count += rows.len() as u64;
                for chunk in rows.chunks(crate::results::BATCH_ROWS) {
                    if !sink.rows(chunk.to_vec())? {
                        sink.limit("Result retention limit reached");
                        sink.complete(count)?;
                        return Ok(count);
                    }
                }
            }
            sink.complete(count)?;
            Ok(count)
        }
        .await;
        *self.active_handle.lock() = None;
        if self.cancel_requested.is_cancelled() {
            Err(Self::cancelled())
        } else {
            outcome
        }
    }

    async fn cancel(&self) -> AppResult<()> {
        // Hold the handle lock while signaling, so the executor cannot clear a
        // known handle before this cancellation has captured its target.
        let handle = {
            let active = self.active_handle.lock();
            self.cancel_requested.cancel();
            active.clone()
        };
        if let Some(handle) = handle {
            self.request(
                Method::POST,
                &format!("api/v2/statements/{handle}/cancel"),
                None,
            )
            .await?;
        }
        Ok(())
    }
    async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
        self.catalog_tables().await
    }
    async fn completion_identity(&self) -> AppResult<CompletionIdentity> {
        self.live_completion_identity().await
    }
    async fn completion_catalog(
        &self,
        identity: &CompletionIdentity,
    ) -> AppResult<CompletionRelations> {
        self.live_completion_catalog(identity).await
    }
    async fn completion_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.table_schema(table).await
    }
    async fn schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.table_schema(table).await
    }
    async fn browse(
        &self,
        schema: &TableSchema,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        self.table_preview(schema, after).await
    }
    async fn apply(&self, _: &TableSchema, _: &[RowMutation]) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "Snowflake editing is disabled",
        ))
    }
    async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
        Ok(Vec::new())
    }
    async fn cancel_process(&self, _: &str, _: &str) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Snowflake process cancellation is not available",
        ))
    }
    async fn health(&self) -> AppResult<u32> {
        if self.is_closed() {
            Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "Snowflake session closed",
            ))
        } else {
            Ok(0)
        }
    }
    fn is_closed(&self) -> bool {
        self.closed.is_cancelled()
    }
    fn close(&self) {
        self.closed.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[derive(Default)]
    pub(super) struct Sink {
        columns: Vec<ColumnInfo>,
        pub(super) rows: Vec<TextRow>,
        pub(super) completed: bool,
    }
    impl QuerySink for Sink {
        fn limit(&mut self, _: &str) {}
        fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()> {
            self.columns = columns;
            Ok(())
        }
        fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
            self.rows.extend(rows);
            Ok(true)
        }
        fn complete(&mut self, _: u64) -> AppResult<()> {
            self.completed = true;
            Ok(())
        }
    }

    #[tokio::test]
    async fn async_query_reads_bounded_partitions_without_changing_numeric_text() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = uuid::Uuid::new_v4().to_string();
        let server = tokio::spawn(async move {
            for step in 0..3 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 1024];
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
                let request = String::from_utf8_lossy(&bytes);
                assert!(
                    request.contains("Authorization: Bearer test-token")
                        || request.contains("authorization: Bearer test-token")
                );
                if step == 0 {
                    assert!(request.contains("\"QUERY_TAG\":\"Joinery/test\""));
                    assert!(!request.contains("\"AUTOCOMMIT\""));
                    assert!(request.contains("\"role\":\"READ_ONLY\""));
                }
                let (status, body) = match step {
                    0 => ("202 Accepted", json!({"statementHandle":handle})),
                    1 => (
                        "200 OK",
                        json!({"statementHandle":handle,"resultSetMetaData":{"rowType":[{"name":"N","type":"FIXED","scale":0,"nullable":false}],"partitionInfo":[{},{}]},"data":[["123456789012345678901234567890"]]}),
                    ),
                    _ => ("200 OK", json!({"data":[["0.000000000000001"]]})),
                };
                let body = body.to_string();
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
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
            query_tag: "Joinery/test".into(),
            auth: AsyncMutex::new(AuthState {
                tokens: Tokens {
                    access: Zeroizing::new("test-token".into()),
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
        let mut sink = Sink::default();
        let count = driver
            .execute("SELECT 1", StatementKind::Read, &mut sink)
            .await
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(sink.columns[0].name, "N");
        assert_eq!(
            sink.rows[0][0].as_deref(),
            Some("123456789012345678901234567890")
        );
        assert_eq!(sink.rows[1][0].as_deref(), Some("0.000000000000001"));
        assert!(sink.completed);
        server.await.unwrap();
    }
    #[test]
    fn account_hosts_cannot_redirect_tokens() {
        assert!(account_url("org-account.snowflakecomputing.com").is_ok());
        assert!(account_url("org-account.snowflakecomputing.com.evil.test").is_err());
        assert!(account_url("localhost").is_err());
        assert!(account_url("org-account.snowflakecomputing.com/path").is_err());
    }
    #[test]
    fn identifiers_do_not_expand_oauth_scope() {
        assert!(simple_identifier("READ_ONLY"));
        assert!(!simple_identifier("READ_ONLY session:role:ADMIN"));
        assert!(!simple_identifier("read_only"));
        assert!(!simple_identifier("READ-ONLY"));
    }
    #[test]
    fn exact_values_stay_text() {
        let response =
            json!({"data":[["123456789012345678901234567890",null,"0.000000000000001"]]});
        let rows = decode_rows(&response).unwrap();
        assert_eq!(
            rows[0][0].as_deref(),
            Some("123456789012345678901234567890")
        );
        assert_eq!(rows[0][1], None);
        assert_eq!(rows[0][2].as_deref(), Some("0.000000000000001"));
    }
}
