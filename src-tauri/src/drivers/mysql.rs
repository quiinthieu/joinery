use super::*;
use futures_util::future::BoxFuture;
use mysql_async::{Conn, Opts, OptsBuilder, Params, Row, SslOpts, Value, prelude::Queryable};
use std::{
    future::Future,
    ops::{Deref, DerefMut},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

mod catalog;
mod values;
pub(crate) use values::MysqlDialect;
pub use values::update_sql;
use values::{column, text_row};

// Conn's normal Drop drains pending results in a background task. Explicit
// disconnect prevents a released, oversized viewer from draining unbounded rows.
struct Connection(Option<Conn>);
impl Deref for Connection {
    type Target = Conn;
    fn deref(&self) -> &Conn {
        self.0.as_ref().unwrap()
    }
}
impl DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut Conn {
        self.0.as_mut().unwrap()
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(conn) = self.0.take()
            && let Ok(handle) = tokio::runtime::Handle::try_current()
        {
            handle.spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(2), conn.disconnect()).await;
            });
        }
    }
}

struct LiveConnection {
    connection: Mutex<Connection>,
    id: u32,
}
pub struct Mysql {
    editor: Arc<LiveConnection>,
    opts: Opts,
    own_id: u32,
    edit: parking_lot::Mutex<Option<Arc<LiveConnection>>>,
    cancel_requested: CancellationToken,
    closed: CancellationToken,
    read_only: bool,
    maria: bool,
}

impl Mysql {
    pub async fn open(
        profile: &ConnectionProfile,
        password: Option<&str>,
        session_id: &str,
        forwarded_port: Option<u16>,
    ) -> AppResult<(Arc<Self>, SessionInfo)> {
        let started = Instant::now();
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut builder = OptsBuilder::default()
            .ip_or_hostname(profile.host.clone())
            .tcp_port(forwarded_port.unwrap_or(profile.port))
            .user(Some(profile.username.clone()))
            .pass(password.map(str::to_owned))
            .db_name(Some(profile.database.clone()))
            .prefer_socket(false)
            .client_found_rows(true)
            .max_allowed_packet(Some(16 * 1024 * 1024))
            .stmt_cache_size(32);
        if forwarded_port.is_some() {
            builder = builder.resolved_ips(Some(vec![std::net::Ipv4Addr::LOCALHOST.into()]));
        }
        if profile.tls_mode == TlsMode::VerifyFull {
            let mut roots = rustls_native_certs::load_native_certs()
                .certs
                .into_iter()
                .map(|c| c.as_ref().to_vec().into())
                .collect::<Vec<_>>();
            if let Some(path) = &profile.ca_cert_path {
                let bytes = crate::files::read_bounded(path, 1024 * 1024)?;
                let certificates = rustls_pemfile::certs(&mut std::io::Cursor::new(&bytes))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| AppError::invalid("Invalid CA certificate"))?;
                if certificates.is_empty() {
                    return Err(AppError::invalid(
                        "The selected file contains no PEM certificates",
                    ));
                }
                roots.push(bytes.to_vec().into());
            }
            builder = builder.ssl_opts(Some(SslOpts::default().with_root_certs(roots)));
        }
        let opts: Opts = builder.into();
        let mut conn = connect(&opts).await?;
        let version: Option<String> = conn
            .query_first("SELECT VERSION()")
            .await
            .map_err(mysql_error)?;
        let version =
            version.ok_or_else(|| AppError::internal("The server returned no version"))?;
        let maria = version.to_ascii_lowercase().contains("mariadb");
        initialize(&mut conn, profile.safe_mode == SafeMode::ReadOnly, maria).await?;
        if profile.tls_mode == TlsMode::VerifyFull {
            let cipher: Option<(String, String)> = conn
                .query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'")
                .await
                .map_err(mysql_error)?;
            if cipher.is_none_or(|(_, cipher)| cipher.is_empty()) {
                return Err(AppError::new(
                    ErrorCode::ConnectionFailed,
                    "The server did not establish the required verified TLS connection",
                ));
            }
        }
        let own_id = conn.id();
        Ok((
            Arc::new(Self {
                editor: Arc::new(LiveConnection {
                    connection: Mutex::new(conn),
                    id: own_id,
                }),
                opts,
                own_id,
                edit: parking_lot::Mutex::new(None),
                cancel_requested: CancellationToken::new(),
                closed: CancellationToken::new(),
                read_only: profile.safe_mode == SafeMode::ReadOnly,
                maria,
            }),
            SessionInfo {
                id: session_id.into(),
                connection_id: profile.id.clone(),
                engine: EngineKind::Mysql,
                server_version: version,
                latency_ms: started.elapsed().as_millis() as u32,
                encrypted: profile.tls_mode == TlsMode::VerifyFull,
                capabilities: EngineCapabilities {
                    sql: true,
                    transactions: true,
                    cancellation: true,
                    table_browsing: true,
                    table_editing: true,
                    table_filtering: true,
                    processes: true,
                },
            },
        ))
    }

    async fn bounded<T>(&self, work: impl Future<Output = AppResult<T>>) -> AppResult<T> {
        tokio::select! {
            biased;
            _=self.closed.cancelled()=>Err(closed()),
            outcome=tokio::time::timeout(Duration::from_secs(65),work)=>outcome.unwrap_or_else(|_| {
                self.closed.cancel();
                Err(AppError::new(ErrorCode::ConnectionClosed,"The database operation timed out. Reconnect and verify any write outcome before retrying."))
            }),
        }
    }
    async fn auxiliary(&self) -> AppResult<Connection> {
        let mut conn = connect(&self.opts).await?;
        initialize(&mut conn, self.read_only, self.maria).await?;
        Ok(conn)
    }
    async fn metadata<T>(
        &self,
        work: impl for<'a> FnOnce(&'a mut Conn) -> BoxFuture<'a, AppResult<T>>,
    ) -> AppResult<T> {
        self.bounded(async {
            let mut conn = self.auxiliary().await?;
            work(&mut conn).await
        })
        .await
    }
}

async fn connect(opts: &Opts) -> AppResult<Connection> {
    tokio::time::timeout(Duration::from_secs(10), Conn::new(opts.clone()))
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::ConnectionFailed,
                "The database connection timed out",
            )
        })?
        .map(|c| Connection(Some(c)))
        .map_err(mysql_error)
}
async fn initialize(conn: &mut Conn, read_only: bool, maria: bool) -> AppResult<()> {
    conn.query_drop("SET NAMES utf8mb4")
        .await
        .map_err(mysql_error)?;
    conn.query_drop("SET SESSION sql_mode='STRICT_ALL_TABLES,NO_ENGINE_SUBSTITUTION'")
        .await
        .map_err(mysql_error)?;
    conn.query_drop("SET SESSION time_zone='+00:00'")
        .await
        .map_err(mysql_error)?;
    conn.query_drop("SET SESSION innodb_lock_wait_timeout=5")
        .await
        .map_err(mysql_error)?;
    conn.query_drop(if maria {
        "SET SESSION max_statement_time=60"
    } else {
        "SET SESSION max_execution_time=60000"
    })
    .await
    .map_err(mysql_error)?;
    if read_only {
        conn.query_drop("SET SESSION TRANSACTION READ ONLY")
            .await
            .map_err(mysql_error)?;
    }
    Ok(())
}

#[async_trait]
impl Driver for Mysql {
    async fn execute(
        &self,
        sql: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        let mut editor = self.editor.connection.lock().await;
        let conn = &mut *editor;
        let mut result=self.bounded(async {
            if self.cancel_requested.is_cancelled() { return Err(AppError::new(ErrorCode::Cancelled,"The query was cancelled before submission")); }
            conn.query_drop("SET NAMES utf8mb4; SET SESSION sql_mode='STRICT_ALL_TABLES,NO_ENGINE_SUBSTITUTION'; SET SESSION time_zone='+00:00'").await.map_err(mysql_error)?;
            if self.cancel_requested.is_cancelled() { return Err(AppError::new(ErrorCode::Cancelled,"The query was cancelled before submission")); }
            let mut result=conn.query_iter(sql).await.map_err(mysql_error)?;
            if kind==StatementKind::Read && self.cancel_requested.is_cancelled() {return Err(AppError::new(ErrorCode::Cancelled,"The query was cancelled"));}
            let mut total=0u64;
            // next() returns None at a result-set boundary and advances to the
            // next set. Always consume every set, including write RETURNING rows.
            loop {
                let columns=result.columns_ref().to_vec();
                let affected=if columns.is_empty() {result.affected_rows()}else{0};
                if !columns.is_empty() { sink.describe(columns.iter().enumerate().map(|(i,c)|column(i,c)).collect())?; }
                let mut pending=Vec::new();let mut bytes=0;let mut retaining=true;
                while let Some(row)=result.next().await.map_err(mysql_error)? {
                    if !retaining {continue;}
                    let row=text_row(row)?;
                    let size=row.iter().map(|v|v.as_ref().map_or(5,|v|v.len()+5)).sum::<usize>();
                    if size>2*1024*1024 {
                        sink.limit("A returned row exceeds the 2 MiB viewer limit. Use a smaller projection.");retaining=false;
                    } else { bytes+=size;pending.push(row); }
                    if (pending.len()>=500 || bytes>=256*1024 || !retaining) && !pending.is_empty() {
                        retaining=sink.rows(std::mem::take(&mut pending))? && retaining; bytes=0;
                    }
                    if !retaining && kind==StatementKind::Read {
                        sink.complete(total)?;
                        self.closed.cancel();
                        return Ok(total);
                    }
                }
                if !pending.is_empty() { let _=sink.rows(pending)?; }
                total=total.saturating_add(affected);sink.complete(affected)?;
                if result.is_empty() {break;}
            }
            result.drop_result().await.map_err(mysql_error)?;
            if kind==StatementKind::Read && self.cancel_requested.is_cancelled() {return Err(AppError::new(ErrorCode::Cancelled,"The query was cancelled"));}
            Ok(total)
        }).await;
        if result.as_ref().is_err_and(|e| {
            matches!(
                e.code,
                ErrorCode::ResultLimitReached
                    | ErrorCode::Internal
                    | ErrorCode::NotFound
                    | ErrorCode::Unsupported
                    | ErrorCode::InvalidInput
            )
        }) {
            self.closed.cancel();
            if kind != StatementKind::Read {
                result = Err(AppError::new(
                    ErrorCode::OutcomeUnknown,
                    "The write result could not be fully observed. Verify the database state before retrying.",
                ));
            }
        }
        if self.closed.is_cancelled()
            || result.as_ref().is_err_and(|e| {
                matches!(
                    e.code,
                    ErrorCode::ConnectionClosed | ErrorCode::ConnectionFailed
                )
            })
        {
            self.closed.cancel();
        }
        result
    }
    async fn cancel(&self) -> AppResult<()> {
        // Hold the actual owning connection alive, including a short-lived grid
        // connection, until cancellation finishes. Never target metadata sessions.
        let lease = self
            .edit
            .lock()
            .clone()
            .unwrap_or_else(|| self.editor.clone());
        self.cancel_requested.cancel();
        let outcome = tokio::time::timeout(Duration::from_secs(10), async {
            let mut conn = connect(&self.opts).await?;
            loop {
                conn.query_drop(format!("KILL QUERY {}", lease.id))
                    .await
                    .map_err(mysql_error)?;
                if lease.connection.try_lock().is_ok() {
                    return Ok(());
                }
                // KILL QUERY can arrive during preflight, before the submitted
                // statement reaches the server. Retry only this still-owned
                // connection until execution observes its terminal outcome.
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            Err(AppError::new(
                ErrorCode::ConnectionFailed,
                "The query cancellation request timed out",
            ))
        });
        if outcome.is_err() {
            self.closed.cancel();
        }
        outcome
    }
    async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
        self.catalog_tables().await
    }
    async fn completion_identity(&self) -> AppResult<CompletionIdentity> {
        self.bounded(async {
            let mut conn = self.editor.connection.lock().await;
            let rows = fetch_text(&mut conn, "SELECT COALESCE(DATABASE(),''),CURRENT_USER(),COALESCE(CURRENT_ROLE(),'NONE'),@@lower_case_table_names", 1, 64*1024).await?;
            let row = rows.first().ok_or_else(|| AppError::internal("Missing session identity"))?;
            let case_sensitive = match cell(row,3) {
                "0" => true,
                "1" | "2" => false,
                _ => return Err(AppError::new(ErrorCode::Unsupported,"Unknown server identifier case rules")),
            };
            let database = cell(row,0).to_owned();
            Ok(CompletionIdentity {
                search_path: if database.is_empty() { vec![] } else { vec![database.clone()] },
                database,
                role: format!("{} ({})",cell(row,1),cell(row,2)),
                case_sensitive_table_names: Some(case_sensitive),
            })
        }).await
    }
    async fn completion_catalog(
        &self,
        _identity: &CompletionIdentity,
    ) -> AppResult<CompletionRelations> {
        self.live_completion_catalog().await
    }
    async fn completion_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.live_completion_schema(table).await
    }
    async fn schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.table_schema(table).await
    }
    async fn browse(
        &self,
        schema: &TableSchema,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        self.browse_rows(schema, BrowseQuery::default(), after)
            .await
    }
    async fn browse_query(
        &self,
        schema: &TableSchema,
        query: &BrowseQuery,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        self.browse_rows(schema, query.clone(), after).await
    }
    async fn apply(&self, schema: &TableSchema, rows: &[RowMutation]) -> AppResult<()> {
        self.apply_rows(schema, rows).await
    }
    async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
        let rows=self.metadata(|conn| Box::pin(fetch(conn,"SELECT ID,USER,COALESCE(DB,''),COMMAND,TIME,LEFT(INFO,4096) FROM INFORMATION_SCHEMA.PROCESSLIST ORDER BY TIME DESC LIMIT 500",vec![],500,3*1024*1024))).await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let id = cell(&r, 0).to_owned();
                ProcessInfo {
                    identity: format!("{}:{}", id, cell(&r, 4)),
                    own_session: id == self.own_id.to_string(),
                    id,
                    user: cell(&r, 1).into(),
                    database: cell(&r, 2).into(),
                    state: if cell(&r, 3) == "Query" {
                        "active"
                    } else {
                        "idle"
                    }
                    .into(),
                    duration_seconds: cell(&r, 4).parse().unwrap_or(0.0),
                    query: r[5].clone(),
                }
            })
            .collect())
    }
    async fn cancel_process(&self, _id: &str, _identity: &str) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "MySQL process inspection is read-only. Cancel an owned query from its SQL tab to preserve cancellation identity.",
        ))
    }
    async fn health(&self) -> AppResult<u32> {
        let started = Instant::now();
        if let Ok(mut guard) = self.editor.connection.try_lock() {
            let conn = &mut *guard;
            self.bounded(async { conn.ping().await.map_err(mysql_error) })
                .await?;
        } else {
            self.metadata(|c| Box::pin(async { c.ping().await.map_err(mysql_error) }))
                .await?;
        }
        Ok(started.elapsed().as_millis() as u32)
    }
    fn is_closed(&self) -> bool {
        self.closed.is_cancelled()
    }
    fn close(&self) {
        self.closed.cancel();
    }
}

async fn fetch(
    conn: &mut Conn,
    sql: &str,
    params: Vec<Value>,
    max_rows: usize,
    max_bytes: usize,
) -> AppResult<Vec<TextRow>> {
    let result = conn
        .exec_iter(sql, Params::Positional(params))
        .await
        .map_err(mysql_error)?;
    collect_metadata(result, max_rows, max_bytes).await
}

// Prepared statements can retain DATABASE() from their preparation context.
// Live metadata uses text protocol after USE, role and temporary-table changes.
async fn fetch_text(
    conn: &mut Conn,
    sql: &str,
    max_rows: usize,
    max_bytes: usize,
) -> AppResult<Vec<TextRow>> {
    let result = conn.query_iter(sql).await.map_err(mysql_error)?;
    collect_metadata(result, max_rows, max_bytes).await
}

async fn collect_metadata<P: mysql_async::prelude::Protocol + Send>(
    mut result: mysql_async::QueryResult<'_, '_, P>,
    max_rows: usize,
    max_bytes: usize,
) -> AppResult<Vec<TextRow>> {
    let mut rows = Vec::new();
    let mut bytes = 0;
    while let Some(row) = result.next().await.map_err(mysql_error)? {
        let row = text_row(row)?;
        bytes += row
            .iter()
            .map(|v| v.as_ref().map_or(5, |v| v.len() + 5))
            .sum::<usize>();
        if rows.len() >= max_rows || bytes > max_bytes {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "The metadata or row result exceeds its bounded buffer",
            ));
        }
        rows.push(row);
    }
    result.drop_result().await.map_err(mysql_error)?;
    Ok(rows)
}
fn cell(row: &TextRow, index: usize) -> &str {
    row[index].as_deref().unwrap_or("")
}
fn bind(value: Option<String>) -> Value {
    value.map_or(Value::NULL, |v| Value::Bytes(v.into_bytes()))
}
fn closed() -> AppError {
    AppError::new(
        ErrorCode::ConnectionClosed,
        "The database connection closed. Reconnect before running another query.",
    )
}
fn mysql_error(error: mysql_async::Error) -> AppError {
    match error {
        mysql_async::Error::Server(error) => AppError::new(
            match error.code {
                1317 | 3024 | 1969 => ErrorCode::Cancelled,
                1044 | 1045 | 1142 | 1227 => ErrorCode::PermissionDenied,
                _ => ErrorCode::Database,
            },
            error.message,
        ),
        _ => AppError::new(
            ErrorCode::ConnectionClosed,
            "The MySQL connection failed or closed. Check network, authentication, and verified TLS settings.",
        ),
    }
}
