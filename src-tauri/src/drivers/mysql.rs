use super::*;
use futures_util::future::BoxFuture;
use mysql_async::{
    Conn, Opts, OptsBuilder, Params, Row, SslOpts, Value, consts::StatusFlags, prelude::Queryable,
};
use std::{
    future::Future,
    ops::{Deref, DerefMut},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

mod catalog;
mod explorer;
mod values;
pub(crate) use catalog::browse_sql;
pub(crate) use values::MysqlDialect;
pub use values::mutation_sql;
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

struct MysqlIdentityCache {
    database: String,
    role: String,
    dirty: bool,
}

pub struct Mysql {
    editor: Arc<LiveConnection>,
    opts: Opts,
    own_id: u32,
    edit: parking_lot::Mutex<Option<Arc<LiveConnection>>>,
    identity_cache: parking_lot::Mutex<MysqlIdentityCache>,
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
        // COM_STMT_PREPARE checks syntax/support without executing the SELECT.
        // Never collect actual measurements during connection or change the
        // user's optimizer/JSON-format settings to enable a plan action.
        let (probe, format) = if maria {
            (
                "ANALYZE FORMAT=JSON SELECT 1",
                ActualPlanFormat::MariaDbJson,
            )
        } else {
            (
                "EXPLAIN ANALYZE FORMAT=TREE SELECT 1",
                ActualPlanFormat::MysqlTree,
            )
        };
        let actual_plan = match conn.prep(probe).await {
            Ok(statement) => {
                conn.close(statement).await.map_err(mysql_error)?;
                Some(format)
            }
            Err(_) => None,
        };
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
        let identity_rows = fetch_text(
            &mut conn,
            "SELECT COALESCE(DATABASE(),''),CURRENT_USER(),COALESCE(CURRENT_ROLE(),'NONE')",
            1,
            64 * 1024,
        )
        .await?;
        let identity_row = identity_rows
            .first()
            .ok_or_else(|| AppError::internal("Missing session identity"))?;
        let initial_cache = MysqlIdentityCache {
            database: cell(identity_row, 0).to_owned(),
            role: format!("{} ({})", cell(identity_row, 1), cell(identity_row, 2)),
            dirty: false,
        };
        Ok((
            Arc::new(Self {
                editor: Arc::new(LiveConnection {
                    connection: Mutex::new(conn),
                    id: own_id,
                }),
                opts,
                own_id,
                edit: parking_lot::Mutex::new(None),
                identity_cache: parking_lot::Mutex::new(initial_cache),
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
                    object_explorer: true,
                    actual_plan,
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

    pub(super) async fn close_phantom_transaction(conn: &mut Conn, was_in_trans: bool) {
        if was_in_trans {
            return;
        }
        if let Ok(post_flags) = observe_status_flags(conn).await
            && !post_flags.contains(StatusFlags::SERVER_STATUS_AUTOCOMMIT)
            && post_flags.contains(StatusFlags::SERVER_STATUS_IN_TRANS)
        {
            let _ = conn.query_drop("ROLLBACK").await;
        }
    }

    async fn refresh_identity_if_needed(&self, conn: &mut Conn) -> AppResult<(String, String)> {
        {
            let cache = self.identity_cache.lock();
            if !cache.dirty {
                return Ok((cache.database.clone(), cache.role.clone()));
            }
        }
        let was_in_trans = observe_status_flags(conn)
            .await?
            .contains(StatusFlags::SERVER_STATUS_IN_TRANS);
        let rows = fetch_text(
            conn,
            "SELECT COALESCE(DATABASE(),''),CURRENT_USER(),COALESCE(CURRENT_ROLE(),'NONE')",
            1,
            64 * 1024,
        )
        .await;
        Self::close_phantom_transaction(conn, was_in_trans).await;
        let rows = rows?;
        let row = rows
            .first()
            .ok_or_else(|| AppError::internal("Missing session identity"))?;
        let database = cell(row, 0).to_owned();
        let role = format!("{} ({})", cell(row, 1), cell(row, 2));
        let mut cache = self.identity_cache.lock();
        cache.database = database.clone();
        cache.role = role.clone();
        cache.dirty = false;
        Ok((database, role))
    }
    async fn execute_editor_sql(
        &self,
        original_sql: &str,
        wire_sql: &str,
        positional: Option<Vec<Value>>,
        rewrite_map: Option<&crate::sql_text::RewriteMap>,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        let mut editor = self.editor.connection.lock().await;
        let conn = &mut *editor;
        let mut result = self
            .bounded(async {
                if self.cancel_requested.is_cancelled() {
                    return Err(AppError::new(
                        ErrorCode::Cancelled,
                        "The query was cancelled before submission",
                    ));
                }
                conn.query_drop("SET NAMES utf8mb4; SET SESSION sql_mode='STRICT_ALL_TABLES,NO_ENGINE_SUBSTITUTION'; SET SESSION time_zone='+00:00'")
                    .await
                    .map_err(mysql_error)?;
                if self.cancel_requested.is_cancelled() {
                    return Err(AppError::new(
                        ErrorCode::Cancelled,
                        "The query was cancelled before submission",
                    ));
                }
                if let Some(values) = positional {
                    let result = conn
                        .exec_iter(wire_sql, Params::Positional(values))
                        .await
                        .map_err(|e| mysql_sql_error_mapped(e, original_sql, wire_sql, rewrite_map))?;
                    self.stream_mysql_result(result, original_sql, wire_sql, rewrite_map, kind, sink)
                        .await
                } else {
                    let result = conn
                        .query_iter(wire_sql)
                        .await
                        .map_err(|e| mysql_sql_error_mapped(e, original_sql, wire_sql, rewrite_map))?;
                    self.stream_mysql_result(result, original_sql, wire_sql, rewrite_map, kind, sink)
                        .await
                }
            })
            .await;
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
        if mysql_may_change_identity(original_sql) {
            self.identity_cache.lock().dirty = true;
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

    async fn stream_mysql_result<'a, 't, P: mysql_async::prelude::Protocol + Send>(
        &self,
        mut result: mysql_async::QueryResult<'a, 't, P>,
        original_sql: &str,
        wire_sql: &str,
        rewrite_map: Option<&crate::sql_text::RewriteMap>,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        if (kind == StatementKind::Read || crate::safety::mysql_executes_analysis(original_sql))
            && self.cancel_requested.is_cancelled()
        {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "The query was cancelled",
            ));
        }
        let mut total = 0u64;
        // next() returns None at a result-set boundary and advances to the
        // next set. Always consume every set, including write RETURNING rows.
        loop {
            let columns = result.columns_ref().to_vec();
            let affected = if columns.is_empty() {
                result.affected_rows()
            } else {
                0
            };
            if !columns.is_empty() {
                sink.describe(
                    columns
                        .iter()
                        .enumerate()
                        .map(|(i, c)| column(i, c))
                        .collect(),
                )?;
            }
            let mut pending = Vec::new();
            let mut bytes = 0;
            let mut retaining = true;
            while let Some(row) = result
                .next()
                .await
                .map_err(|e| mysql_sql_error_mapped(e, original_sql, wire_sql, rewrite_map))?
            {
                if !retaining {
                    continue;
                }
                let row = text_row(row)?;
                let size = row
                    .iter()
                    .map(|v| v.as_ref().map_or(5, |v| v.len() + 5))
                    .sum::<usize>();
                if size > 2 * 1024 * 1024 {
                    sink.limit(
                        "A returned row exceeds the 2 MiB viewer limit. Use a smaller projection.",
                    );
                    retaining = false;
                } else {
                    bytes += size;
                    pending.push(row);
                }
                if (pending.len() >= 500 || bytes >= 256 * 1024 || !retaining)
                    && !pending.is_empty()
                {
                    retaining = sink.rows(std::mem::take(&mut pending))? && retaining;
                    bytes = 0;
                }
                if !retaining && kind == StatementKind::Read {
                    sink.complete(total)?;
                    self.closed.cancel();
                    return Ok(total);
                }
            }
            if !pending.is_empty() {
                let _ = sink.rows(pending)?;
            }
            total = total.saturating_add(affected);
            sink.complete(affected)?;
            if result.is_empty() {
                break;
            }
        }
        result
            .drop_result()
            .await
            .map_err(|e| mysql_sql_error_mapped(e, original_sql, wire_sql, rewrite_map))?;
        if (kind == StatementKind::Read || crate::safety::mysql_executes_analysis(original_sql))
            && self.cancel_requested.is_cancelled()
        {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "The query was cancelled",
            ));
        }
        Ok(total)
    }
}

fn mysql_bind_value(bind: &crate::params::TypedBind) -> AppResult<Value> {
    let Some(raw) = bind.value.as_deref() else {
        return Ok(Value::NULL);
    };
    match bind.kind {
        QueryParamKind::Integer => Ok(Value::Int(raw.parse::<i64>().map_err(|_| {
            AppError::invalid(format!("Parameter {} must be a 64-bit integer", bind.key))
        })?)),
        QueryParamKind::Boolean => Ok(Value::Int(if raw == "true" { 1 } else { 0 })),
        QueryParamKind::Timestamp => Ok(Value::Bytes(
            bind.mysql_datetime
                .as_deref()
                .unwrap_or(raw)
                .as_bytes()
                .to_vec(),
        )),
        QueryParamKind::Text
        | QueryParamKind::Decimal
        | QueryParamKind::Uuid
        | QueryParamKind::Date
        | QueryParamKind::Json => Ok(Value::Bytes(raw.as_bytes().to_vec())),
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
        self.execute_editor_sql(sql, sql, None, None, kind, sink)
            .await
    }

    async fn execute_bound(
        &self,
        sql: &str,
        kind: StatementKind,
        bound: &crate::params::BoundStatement,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        if bound.binds.is_empty() {
            return self.execute(sql, kind, sink).await;
        }
        let values = bound
            .binds
            .iter()
            .map(mysql_bind_value)
            .collect::<AppResult<Vec<_>>>()?;
        self.execute_editor_sql(
            sql,
            &bound.rewritten_sql,
            Some(values),
            Some(&bound.rewrite_map),
            kind,
            sink,
        )
        .await
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
    async fn database_objects(&self) -> AppResult<Vec<DatabaseObjectSummary>> {
        self.explorer_objects().await
    }
    async fn object_details(&self, object: &DatabaseObjectRef) -> AppResult<ObjectDetails> {
        self.explorer_object_details(object).await
    }
    async fn completion_identity(&self) -> AppResult<CompletionIdentity> {
        self.bounded(async {
            let mut conn = self.editor.connection.lock().await;
            let was_in_trans = observe_status_flags(&mut conn)
                .await?
                .contains(StatusFlags::SERVER_STATUS_IN_TRANS);
            let rows = fetch_text(&mut conn, "SELECT COALESCE(DATABASE(),''),CURRENT_USER(),COALESCE(CURRENT_ROLE(),'NONE'),@@lower_case_table_names", 1, 64*1024).await;
            Self::close_phantom_transaction(&mut conn, was_in_trans).await;
            let rows = rows?;
            let row = rows.first().ok_or_else(|| AppError::internal("Missing session identity"))?;
            let case_sensitive = match cell(row,3) {
                "0" => true,
                "1" | "2" => false,
                _ => return Err(AppError::new(ErrorCode::Unsupported,"Unknown server identifier case rules")),
            };
            let database = cell(row,0).to_owned();
            let role = format!("{} ({})",cell(row,1),cell(row,2));
            {
                let mut cache = self.identity_cache.lock();
                cache.database = database.clone();
                cache.role = role.clone();
                cache.dirty = false;
            }
            Ok(CompletionIdentity {
                search_path: if database.is_empty() { vec![] } else { vec![database.clone()] },
                database,
                role,
                case_sensitive_table_names: Some(case_sensitive),
            })
        }).await
    }
    async fn session_state(&self, session_id: &str) -> AppResult<SessionRuntimeState> {
        let session_id = session_id.to_owned();
        self.bounded(async move {
            let mut conn = self.editor.connection.lock().await;
            let (database, role) = self.refresh_identity_if_needed(&mut conn).await?;
            let flags = observe_status_flags(&mut conn).await?;
            let transaction_mode = if flags.contains(StatusFlags::SERVER_STATUS_AUTOCOMMIT) {
                TransactionMode::AutoCommit
            } else {
                TransactionMode::Manual
            };
            let transaction_state = if flags.contains(StatusFlags::SERVER_STATUS_IN_TRANS) {
                TransactionState::InTransaction
            } else {
                TransactionState::Idle
            };
            let active_context = (!database.is_empty()).then(|| database.clone());
            let search_path = active_context.clone().into_iter().collect();
            Ok(SessionRuntimeState {
                session_id,
                transaction_mode,
                transaction_state,
                context_kind: SessionContextKind::Database,
                active_context,
                database,
                role,
                search_path,
            })
        })
        .await
    }
    async fn list_contexts(&self) -> AppResult<Vec<String>> {
        let rows = self
            .metadata(|conn| {
                Box::pin(fetch(
                    conn,
                    "SELECT SCHEMA_NAME
                     FROM INFORMATION_SCHEMA.SCHEMATA
                     WHERE LOWER(SCHEMA_NAME) NOT IN ('mysql', 'performance_schema', 'sys', 'information_schema')
                     ORDER BY SCHEMA_NAME
                     LIMIT 1000",
                    vec![],
                    1000,
                    256 * 1024,
                ))
            })
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| cell(&r, 0).to_owned())
            .filter(|s| !s.is_empty())
            .collect())
    }
    async fn switch_context(&self, target: &str) -> AppResult<()> {
        values::identifier(target)?;
        let quoted = values::quote(target);
        self.bounded(async {
            let mut conn = self.editor.connection.lock().await;
            conn.query_drop(format!("USE {quoted}"))
                .await
                .map_err(|error| {
                    let mapped = mysql_error(error);
                    if mapped.code == ErrorCode::Database
                        && mapped
                            .message
                            .to_ascii_lowercase()
                            .contains("unknown database")
                    {
                        AppError::new(ErrorCode::NotFound, mapped.message)
                    } else {
                        mapped
                    }
                })?;
            self.identity_cache.lock().dirty = true;
            let _ = self.refresh_identity_if_needed(&mut conn).await?;
            Ok(())
        })
        .await
    }
    async fn set_transaction_mode(&self, mode: TransactionMode) -> AppResult<()> {
        self.bounded(async {
            let mut conn = self.editor.connection.lock().await;
            let in_trans = observe_status_flags(&mut conn)
                .await?
                .contains(StatusFlags::SERVER_STATUS_IN_TRANS);
            if mode == TransactionMode::AutoCommit && in_trans {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Commit or roll back the open transaction before switching to auto-commit",
                ));
            }
            let sql = match mode {
                TransactionMode::AutoCommit => "SET SESSION autocommit = 1",
                TransactionMode::Manual => "SET SESSION autocommit = 0",
            };
            conn.query_drop(sql).await.map_err(mysql_error)
        })
        .await
    }
    async fn commit_transaction(&self) -> AppResult<()> {
        self.bounded(async {
            let mut conn = self.editor.connection.lock().await;
            if !observe_status_flags(&mut conn)
                .await?
                .contains(StatusFlags::SERVER_STATUS_IN_TRANS)
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "No transaction is currently open",
                ));
            }
            conn.query_drop("COMMIT").await.map_err(mysql_error)
        })
        .await
    }
    async fn rollback_transaction(&self) -> AppResult<()> {
        self.bounded(async {
            let mut conn = self.editor.connection.lock().await;
            if !observe_status_flags(&mut conn)
                .await?
                .contains(StatusFlags::SERVER_STATUS_IN_TRANS)
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "No transaction is currently open",
                ));
            }
            conn.query_drop("ROLLBACK").await.map_err(mysql_error)
        })
        .await
    }
    async fn completion_catalog(
        &self,
        _identity: &CompletionIdentity,
    ) -> AppResult<CompletionRelations> {
        self.live_completion_catalog().await
    }
    async fn completion_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.live_completion_schema(table, false).await
    }
    async fn inspection_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.live_completion_schema(table, true).await
    }
    async fn schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.table_schema(table).await
    }
    async fn referencing_keys(&self, table: &TableRef) -> AppResult<Vec<InboundForeignKey>> {
        self.referencing_foreign_keys(table).await
    }
    async fn schema_diagram(&self, schema: &str) -> AppResult<SchemaDiagram> {
        self.relationship_diagram(schema).await
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
    async fn lock_waits(&self) -> AppResult<LockWaitSnapshot> {
        let sql = if self.maria {
            // MariaDB 11.4 truncates these ENUM columns over the binary
            // protocol ("RE" for RECORD); an explicit text cast keeps them whole.
            "SELECT r.trx_mysql_thread_id, COALESCE(p.USER, ''),
                    COALESCE(TIMESTAMPDIFF(SECOND, r.trx_wait_started, NOW()), 0),
                    COALESCE(CAST(l.lock_type AS CHAR(32)), ''),
                    COALESCE(CAST(l.lock_mode AS CHAR(32)), ''), NULL, l.lock_table,
                    LEFT(r.trx_query, 4096), b.trx_mysql_thread_id
             FROM INFORMATION_SCHEMA.INNODB_LOCK_WAITS w
             JOIN INFORMATION_SCHEMA.INNODB_TRX r ON r.trx_id = w.requesting_trx_id
             JOIN INFORMATION_SCHEMA.INNODB_TRX b ON b.trx_id = w.blocking_trx_id
             LEFT JOIN INFORMATION_SCHEMA.INNODB_LOCKS l ON l.lock_id = w.requested_lock_id
             LEFT JOIN INFORMATION_SCHEMA.PROCESSLIST p ON p.ID = r.trx_mysql_thread_id
             ORDER BY 3 DESC, 1, 9 LIMIT 2001"
        } else {
            "SELECT r.trx_mysql_thread_id, COALESCE(p.USER, ''),
                    COALESCE(TIMESTAMPDIFF(SECOND, r.trx_wait_started, NOW()), 0),
                    COALESCE(dl.LOCK_TYPE, ''), COALESCE(dl.LOCK_MODE, ''),
                    dl.OBJECT_SCHEMA, dl.OBJECT_NAME,
                    LEFT(r.trx_query, 4096), b.trx_mysql_thread_id
             FROM performance_schema.data_lock_waits w
             JOIN INFORMATION_SCHEMA.INNODB_TRX r ON r.trx_id = w.REQUESTING_ENGINE_TRANSACTION_ID
             JOIN INFORMATION_SCHEMA.INNODB_TRX b ON b.trx_id = w.BLOCKING_ENGINE_TRANSACTION_ID
             LEFT JOIN performance_schema.data_locks dl ON dl.ENGINE_LOCK_ID = w.REQUESTING_ENGINE_LOCK_ID
             LEFT JOIN INFORMATION_SCHEMA.PROCESSLIST p ON p.ID = r.trx_mysql_thread_id
             ORDER BY 3 DESC, 1, 9 LIMIT 2001"
        };
        let maria = self.maria;
        let rows = self
            .metadata(move |conn| Box::pin(fetch(conn, sql, vec![], 2001, 4 * 1024 * 1024)))
            .await
            .map_err(|error| {
                if error.code == ErrorCode::PermissionDenied {
                    AppError::new(
                        ErrorCode::PermissionDenied,
                        if maria {
                            "Lock inspection needs the PROCESS privilege."
                        } else {
                            "Lock inspection needs the PROCESS privilege and SELECT on performance_schema."
                        },
                    )
                } else {
                    error
                }
            })?;
        let (waits, truncated) = LockWait::merge(rows.iter().map(|r| {
            let id = cell(r, 0).to_owned();
            let object = if maria {
                r[6].as_deref().and_then(parse_quoted_table)
            } else {
                r[5].clone()
                    .zip(r[6].clone())
                    .map(|(schema, name)| TableRef { schema, name })
            };
            LockWait {
                own_session: id == self.own_id.to_string(),
                waiting_id: id,
                user: cell(r, 1).into(),
                wait_seconds: cell(r, 2).parse().unwrap_or(0.0),
                lock_type: cell(r, 3).into(),
                lock_mode: cell(r, 4).into(),
                object,
                wait_event: None,
                query: r[7].clone(),
                blocking_ids: vec![cell(r, 8).into()],
            }
        }));
        Ok(LockWaitSnapshot { waits, truncated })
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
/// Parses MariaDB's backtick-quoted `` `schema`.`table` `` lock target.
/// Returns `None` for any other shape rather than guessing.
fn parse_quoted_table(text: &str) -> Option<TableRef> {
    let mut parts = Vec::new();
    let mut chars = text.chars().peekable();
    while chars.peek().is_some() {
        if chars.next()? != '`' {
            return None;
        }
        let mut part = String::new();
        loop {
            match chars.next()? {
                '`' if chars.peek() == Some(&'`') => {
                    chars.next();
                    part.push('`');
                }
                '`' => break,
                c => part.push(c),
            }
        }
        parts.push(part);
        match chars.next() {
            Some('.') => continue,
            None => break,
            Some(_) => return None,
        }
    }
    match <[String; 2]>::try_from(parts) {
        Ok([schema, name]) => Some(TableRef { schema, name }),
        Err(_) => None,
    }
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
async fn observe_status_flags(conn: &mut Conn) -> AppResult<StatusFlags> {
    if let Some(ok) = conn.last_ok_packet() {
        return Ok(ok.status_flags());
    }
    conn.ping().await.map_err(mysql_error)?;
    Ok(conn
        .last_ok_packet()
        .map(|ok| ok.status_flags())
        .unwrap_or_else(StatusFlags::empty))
}

fn mysql_may_change_identity(sql: &str) -> bool {
    sql.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|token| {
            token.eq_ignore_ascii_case("USE")
                || token.eq_ignore_ascii_case("SET")
                || token.eq_ignore_ascii_case("DROP")
        })
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

fn mysql_sql_error_mapped(
    error: mysql_async::Error,
    original_sql: &str,
    wire_sql: &str,
    rewrite_map: Option<&crate::sql_text::RewriteMap>,
) -> AppError {
    let mut error = mysql_sql_error(error, wire_sql);
    if let (Some(map), Some(location)) = (rewrite_map, error.location.as_mut()) {
        location.span = map.map_span(original_sql, wire_sql, location.span);
    }
    error
}

pub(crate) fn mysql_sql_error(error: mysql_async::Error, sql: &str) -> AppError {
    let sql_state = match &error {
        mysql_async::Error::Server(server) if !server.state.is_empty() => {
            Some(server.state.clone())
        }
        _ => None,
    };
    let is_server = matches!(&error, mysql_async::Error::Server(_));
    let base = mysql_error(error);
    if !is_server || base.code == ErrorCode::Cancelled {
        return base;
    }
    // MySQL and MariaDB count syntax-error lines after stripping leading
    // whitespace from the submitted statement. Preserve the editor's origin.
    let body = sql.trim_start();
    let origin = crate::sql_text::span_for_byte_range(sql, sql.len() - body.len(), sql.len());
    let span = crate::sql_text::span_from_mysql_error(body, &base.message)
        .map(|span| crate::sql_text::shift_child_span(origin, span))
        .unwrap_or_else(|| crate::sql_text::trimmed_span(sql));
    base.with_location(ErrorLocation {
        span,
        statement_index: None,
        sql_state,
        detail: None,
        hint: None,
    })
}

#[cfg(test)]
mod lock_target_tests {
    use super::parse_quoted_table;

    #[test]
    fn mariadb_lock_targets_parse_only_two_quoted_parts() {
        let parsed = parse_quoted_table("`joinery`.`browse_sort`").unwrap();
        assert_eq!(
            (parsed.schema.as_str(), parsed.name.as_str()),
            ("joinery", "browse_sort")
        );
        let quoted = parse_quoted_table("`a``b`.`c.d`").unwrap();
        assert_eq!(
            (quoted.schema.as_str(), quoted.name.as_str()),
            ("a`b", "c.d")
        );
        for invalid in [
            "",
            "joinery.browse_sort",
            "`only`",
            "`a`.`b`.`c`",
            "`a`x`b`",
            "`open",
        ] {
            assert!(parse_quoted_table(invalid).is_none(), "{invalid}");
        }
    }
}
