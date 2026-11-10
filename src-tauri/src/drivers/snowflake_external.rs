//! Stateful external-browser read adapter. Remote authentication and writes remain unqualified.
//! Each instance owns one helper connection; no query or connection is automatically recreated.
use super::snowflake_process::{
    Bounds, Certainty, CleanupObserver, Column, Impact, Launch, OperationToken, Report,
    ShutdownReport, Supervisor,
};
use super::{BrowseRows, Driver, QuerySink, RowMutation, snowflake::Snowflake};
use crate::{
    domain::*,
    params::{BoundStatement, TypedBind},
};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio_util::sync::CancellationToken;

const SQL_BYTES: usize = 262_144;
const QUERY_ROWS: u64 = 1_000_000;
const QUERY_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// LOGIN_NAME need not equal CURRENT_USER. Only enforce a principal when explicitly configured.
#[derive(Default)]
pub struct ExternalBrowserOptions {
    pub expected_server_user: Option<String>,
}

#[derive(Clone)]
struct Active {
    token: OperationToken,
    terminal: watch::Receiver<Option<Report>>,
}
struct ActiveLease<'a>(&'a Mutex<Option<Active>>);
impl Drop for ActiveLease<'_> {
    fn drop(&mut self) {
        self.0.lock().take();
    }
}

pub struct SnowflakeExternal {
    process: Arc<Supervisor>,
    profile: ConnectionProfile,
    session_id: String,
    identity: Value,
    lane: AsyncMutex<()>,
    active: Mutex<Option<Active>>,
    closed: CancellationToken,
    shutdown_started: AtomicBool,
    shutdown: watch::Sender<Option<Arc<ShutdownReport>>>,
    runtime: tokio::runtime::Handle,
}

impl SnowflakeExternal {
    /// Capture `launch.cleanup_observer()` before awaiting this method. Cancelling open
    /// drops its supervisor and requests child termination; the observer still proves reap.
    pub async fn open(
        profile: &ConnectionProfile,
        session_id: &str,
        launch: Launch,
        options: ExternalBrowserOptions,
    ) -> AppResult<(Arc<Self>, SessionInfo)> {
        Snowflake::validate_profile(profile)?;
        if profile.snowflake.as_ref().unwrap().authentication
            != SnowflakeAuthentication::ExternalBrowser
        {
            return Err(AppError::invalid(
                "This profile requires local-application OAuth",
            ));
        }
        if profile.engine != EngineKind::Snowflake || session_id.is_empty() {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        if options
            .expected_server_user
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 1024)
        {
            return Err(AppError::invalid("Invalid expected Snowflake principal"));
        }
        let started = Instant::now();
        let process = Arc::new(
            Supervisor::spawn(launch, &uuid::Uuid::new_v4().to_string(), Bounds::default())
                .await
                .map_err(|_| {
                    AppError::new(
                        ErrorCode::ConnectionFailed,
                        "Could not start the Snowflake helper",
                    )
                })?,
        );
        let config = profile
            .snowflake
            .as_ref()
            .expect("validated Snowflake profile");
        // Preserve the exact verified hostname, including regional and PrivateLink components.
        let host = profile.host.trim().to_ascii_lowercase();
        let account = host
            .strip_suffix(".snowflakecomputing.com")
            .ok_or_else(|| AppError::invalid("Invalid Snowflake account hostname"))?;
        let mut connect = process
            .request(
                "connect",
                "connect",
                json!({
                    "account": account, "host": host, "username": profile.username,
                    "role": config.role, "warehouse": config.warehouse,
                    "database": profile.database, "schema": config.schema,
                }),
                Impact::Control,
            )
            .map_err(control_error)?;
        check_report(&connect.wait_bounded(Duration::from_secs(125)).await)?;
        let mut context = process
            .request("identity", "context", json!({}), Impact::Read)
            .map_err(control_error)?;
        let report = context.wait_bounded(Duration::from_secs(65)).await;
        check_report(&report)?;
        let identity = report.result.ok_or_else(protocol_error)?;
        for (field, expected) in [
            ("role", config.role.as_str()),
            ("warehouse", config.warehouse.as_str()),
            ("database", profile.database.as_str()),
            ("schema", config.schema.as_str()),
        ] {
            if identity.get(field).and_then(Value::as_str) != Some(expected) {
                return Err(AppError::new(
                    ErrorCode::PolicyBlocked,
                    "Snowflake connected with a different session context",
                ));
            }
        }
        if identity
            .get("user")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            || identity
                .get("session")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || options
                .expected_server_user
                .as_ref()
                .is_some_and(|expected| {
                    identity.get("user").and_then(Value::as_str) != Some(expected.as_str())
                })
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake did not return the expected session identity",
            ));
        }
        let (shutdown, _) = watch::channel(None);
        let driver = Arc::new(Self {
            process,
            profile: profile.clone(),
            session_id: session_id.into(),
            identity,
            lane: AsyncMutex::new(()),
            active: Mutex::new(None),
            closed: CancellationToken::new(),
            shutdown_started: AtomicBool::new(false),
            shutdown,
            runtime: tokio::runtime::Handle::current(),
        });
        // Obtain the version on the same SDK session rather than infer it from the local package.
        let mut version = ScalarSink::default();
        // Fixed internal metadata SQL is not an owner-supplied classification hint.
        tokio::time::timeout(
            Duration::from_secs(65),
            driver.stream("SELECT CURRENT_VERSION()", vec![], &mut version),
        )
        .await
        .map_err(|_| closed_error())??;
        let server_version = version
            .value
            .filter(|v| !v.is_empty())
            .ok_or_else(protocol_error)?;
        let info = SessionInfo {
            id: session_id.into(),
            connection_id: profile.id.clone(),
            engine: EngineKind::Snowflake,
            server_version,
            latency_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
            encrypted: true,
            capabilities: EngineCapabilities {
                sql: true,
                transactions: false,
                cancellation: true,
                table_browsing: false,
                table_editing: false,
                table_filtering: false,
                processes: false,
                object_explorer: false,
                schema_diagram: false,
                schema_diff: false,
                ddl_editing: false,
                lock_inspection: false,
                explain_analyze: false,
                oauth_browser: false,
                actual_plan: None,
            },
        };
        Ok((driver, info))
    }

    pub fn verified_identity(&self) -> &Value {
        &self.identity
    }
    pub fn closed_signal(&self) -> CancellationToken {
        self.closed.clone()
    }
    pub fn cleanup_observer(&self) -> CleanupObserver {
        self.process.cleanup_observer()
    }
    /// A logical close is immediate; only this receipt/cleanup observer proves logout/reap.
    pub fn shutdown_receiver(&self) -> watch::Receiver<Option<Arc<ShutdownReport>>> {
        self.shutdown.subscribe()
    }
    fn start_shutdown(&self) {
        self.closed.cancel();
        if self.shutdown_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let process = self.process.clone();
        let sender = self.shutdown.clone();
        if tokio::runtime::Handle::try_current().is_err() {
            self.process.request_retirement();
        }
        // Cleanup runs on the runtime that owns the supervisor, even when close is
        // called from a synchronous thread while another Arc retains this driver.
        self.runtime.spawn(async move {
            sender.send_replace(Some(Arc::new(process.shutdown().await)));
        });
    }

    fn validate_read(
        &self,
        sql: &str,
        kind: StatementKind,
        bound: Option<&BoundStatement>,
    ) -> AppResult<()> {
        if kind != StatementKind::Read {
            return Err(unsupported());
        }
        let plan = if bound.is_some() {
            let rewrite = crate::params::rewrite_for_classification(EngineKind::Snowflake, sql)?;
            crate::safety::prepare_parameterized(&self.session_id, &self.profile, sql, &rewrite)?
        } else {
            crate::safety::prepare(&self.session_id, &self.profile, sql)?
        };
        if plan.statements.len() != 1 || plan.statements[0].kind != StatementKind::Read {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This Snowflake adapter accepts one read statement at a time",
            ));
        }
        Ok(())
    }

    async fn run(
        &self,
        sql: &str,
        wire: &str,
        kind: StatementKind,
        bound: Option<&BoundStatement>,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        self.validate_read(sql, kind, bound)?;
        // BoundStatement is public: accept generated casts/functions only when
        // recompiling the classified template produces exactly this wire query.
        if let Some(bound) = bound {
            if bound.binds.len() > crate::params::MAX_PARAM_OCCURRENCES {
                return Err(AppError::invalid("Too many Snowflake helper parameters"));
            }
            let unique: std::collections::HashMap<_, _> = bound
                .binds
                .iter()
                .map(|bind| (bind.key.clone(), bind.clone()))
                .collect();
            let slots: Vec<_> = unique
                .keys()
                .map(|key| QueryParamSlot {
                    key: key.clone(),
                    positional: key.starts_with('?'),
                    occurrences: 1,
                })
                .collect();
            let inputs: Vec<_> = unique
                .values()
                .map(|bind| QueryParamInput {
                    key: bind.key.clone(),
                    kind: bind.kind,
                    value: bind.value.clone(),
                })
                .collect();
            let (_, validated) = crate::params::validate_inputs(&slots, Some(&inputs))?;
            // Batch-wide anonymous placeholders retain their global numbering.
            let mut question_offset = bound
                .binds
                .iter()
                .find_map(|bind| bind.key.strip_prefix('?'))
                .and_then(|number| number.parse::<usize>().ok())
                .and_then(|number| number.checked_sub(1))
                .unwrap_or(0);
            question_offset
                .checked_add(bound.binds.len())
                .ok_or_else(|| AppError::invalid("Invalid Snowflake parameter numbering"))?;
            let compiled = crate::params::compile_statement(
                EngineKind::Snowflake,
                sql,
                &validated,
                &mut question_offset,
            )?;
            if compiled.as_ref().is_none_or(|compiled| {
                compiled.rewritten_sql != wire || compiled.binds != bound.binds
            }) {
                return Err(AppError::new(
                    ErrorCode::PolicyBlocked,
                    "Snowflake parameters do not match the classified query",
                ));
            }
        }
        if wire.len() > SQL_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Snowflake helper SQL limit is 256 KiB",
            ));
        }
        let binds = binds_json(bound.map_or(&[], |b| b.binds.as_slice()))?;
        let _lane = self.lane.lock().await;
        if self.is_closed() {
            return Err(closed_error());
        }
        let outcome =
            tokio::time::timeout(Duration::from_secs(65), self.stream(wire, binds, sink)).await;
        match outcome {
            Ok(result) => {
                if result.as_ref().is_err_and(|error| {
                    matches!(
                        error.code,
                        ErrorCode::ConnectionClosed | ErrorCode::OutcomeUnknown
                    )
                }) {
                    self.start_shutdown();
                }
                result
            }
            Err(_) => {
                self.start_shutdown();
                Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "Snowflake read deadline expired; reconnect before continuing",
                ))
            }
        }
    }

    async fn stream(
        &self,
        wire: &str,
        binds: Vec<Value>,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        let mut operation = self
            .process
            .request(
                "query",
                "query",
                json!({"sql":wire,"binds":binds,
            "classification":"read","credits":1,"maxRows":QUERY_ROWS,"maxBytes":QUERY_BYTES}),
                Impact::Read,
            )
            .map_err(control_error)?;
        let token = operation.token();
        *self.active.lock() = Some(Active {
            token: token.clone(),
            terminal: operation.terminal_receiver(),
        });
        let _active = ActiveLease(&self.active);
        let mut described = false;
        let mut columns_open = true;
        let mut retaining = true;
        let mut sink_error = None;
        loop {
            let available_columns = operation.columns.borrow().clone();
            if !described && let Some(columns) = available_columns {
                if let Err(error) =
                    sink.describe(columns.into_iter().enumerate().map(column_info).collect())
                {
                    sink_error = Some(error);
                    retaining = false;
                    self.discard(&token, &operation).await?;
                }
                described = true;
            }
            tokio::select! {
                biased;
                changed = operation.columns.changed(), if !described && columns_open => { columns_open = changed.is_ok(); }
                batch = operation.rows.recv() => {
                    let Some(mut batch) = batch else { break; };
                    if !described { self.start_shutdown(); return Err(protocol_error()); }
                    let rows = std::mem::take(&mut batch.rows);
                    drop(batch); // Return the supervisor's bounded queue credit before granting more.
                    if retaining {
                        match sink.rows(rows) {
                            Ok(true) => {},
                            Ok(false) => { retaining = false; sink.limit("Result retention limit reached"); },
                            Err(error) => { retaining = false; sink_error = Some(error); }
                        }
                        if !retaining { self.discard(&token, &operation).await?; }
                    }
                    if retaining && operation.report().is_none() {
                        match self.process.credit(&token, 1) {
                            Ok(mut credit) => {
                                let report = credit.wait_bounded(Duration::from_secs(3)).await;
                                if operation.report().is_none() { check_report(&report)?; }
                            },
                            Err(_) if operation.report().is_some() => {},
                            Err(_) => { self.start_shutdown(); return Err(closed_error()); }
                        }
                    }
                }
            }
        }
        let report = operation.wait().await;
        if let Err(error) = check_report(&report) {
            if matches!(
                error.code,
                ErrorCode::ConnectionClosed | ErrorCode::OutcomeUnknown
            ) {
                self.start_shutdown();
            }
            return Err(error);
        }
        if let Some(error) = sink_error {
            return Err(error);
        }
        if !described {
            self.start_shutdown();
            return Err(protocol_error());
        }
        let result = report.result.as_ref().ok_or_else(protocol_error)?;
        let count = result
            .get("rowsSeen")
            .and_then(Value::as_u64)
            .ok_or_else(protocol_error)?;
        if result.get("truncated").and_then(Value::as_bool) == Some(true) {
            sink.limit("Snowflake helper result retention limit reached");
        }
        sink.complete(count)?;
        Ok(count)
    }
    async fn discard(
        &self,
        token: &OperationToken,
        operation: &super::snowflake_process::Operation,
    ) -> AppResult<()> {
        match self.process.discard(token) {
            Ok(mut discard) => {
                let report = discard.wait_bounded(Duration::from_secs(3)).await;
                // A stale control rejection cannot replace the original terminal.
                if operation.report().is_some() {
                    Ok(())
                } else {
                    check_report(&report)
                }
            }
            Err(_) if operation.report().is_some() => Ok(()),
            Err(_) => {
                self.start_shutdown();
                Err(closed_error())
            }
        }
    }
}
impl Drop for SnowflakeExternal {
    fn drop(&mut self) {
        self.start_shutdown();
    }
}

#[async_trait]
impl Driver for SnowflakeExternal {
    async fn execute(
        &self,
        sql: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        self.run(sql, sql, kind, None, sink).await
    }
    async fn execute_bound(
        &self,
        sql: &str,
        kind: StatementKind,
        bound: &BoundStatement,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        self.run(sql, &bound.rewritten_sql, kind, Some(bound), sink)
            .await
    }
    async fn cancel(&self) -> AppResult<()> {
        self.closed.cancel();
        let active = self.active.lock().clone();
        let result = if let Some(mut active) = active {
            let already_terminal = active.terminal.borrow().clone();
            let terminal = if already_terminal.is_some() {
                already_terminal
            } else {
                if let Ok(mut cancel) = self.process.cancel(&active.token) {
                    // The ACK only confirms the cancel request. Original terminal evidence
                    // below remains authoritative even if this control races completion.
                    let _ = cancel.wait_bounded(Duration::from_secs(3)).await;
                }
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let report = active.terminal.borrow().clone();
                        if report.is_some() {
                            return report;
                        }
                        if active.terminal.changed().await.is_err() {
                            return None;
                        }
                    }
                })
                .await
                .unwrap_or(None)
            };
            match terminal {
                Some(report)
                    if report.error_code.as_deref() == Some("cancelled")
                        && report.certainty != Certainty::OutcomeUnknown
                        && report.retirement.is_none() =>
                {
                    Ok(())
                }
                Some(report) if report.ok => Err(AppError::new(
                    ErrorCode::Conflict,
                    "Snowflake query finished before cancellation was confirmed",
                )),
                Some(report) => check_report(&report),
                None => Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "Snowflake cancellation could not be confirmed; the session was closed",
                )),
            }
        } else {
            Ok(())
        };
        self.start_shutdown();
        result
    }

    async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
        Err(unsupported())
    }
    async fn schema(&self, _: &TableRef) -> AppResult<TableSchema> {
        Err(unsupported())
    }
    async fn browse(
        &self,
        _: &TableSchema,
        _: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        Err(unsupported())
    }
    async fn apply(&self, _: &TableSchema, _: &[RowMutation]) -> AppResult<()> {
        Err(unsupported())
    }
    async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
        Err(unsupported())
    }
    async fn cancel_process(&self, _: &str, _: &str) -> AppResult<()> {
        Err(unsupported())
    }
    async fn health(&self) -> AppResult<u32> {
        let start = Instant::now();
        let mut sink = ScalarSink::default();
        self.execute("SELECT 1", StatementKind::Read, &mut sink)
            .await?;
        Ok(start.elapsed().as_millis().min(u32::MAX as u128) as u32)
    }
    fn is_closed(&self) -> bool {
        self.closed.is_cancelled() || self.process.is_closed()
    }
    fn close(&self) {
        self.start_shutdown();
    }
}

fn binds_json(binds: &[TypedBind]) -> AppResult<Vec<Value>> {
    if binds.len() > 1000 {
        return Err(AppError::invalid("Too many Snowflake helper parameters"));
    }
    binds
        .iter()
        .map(|bind| match (&bind.kind, &bind.value) {
            (_, None) => Ok(Value::Null),
            (QueryParamKind::Boolean, Some(value)) => match value.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(AppError::invalid("Invalid canonical Boolean parameter")),
            },
            (_, Some(value)) if value.len() <= SQL_BYTES => Ok(Value::String(value.clone())),
            _ => Err(AppError::invalid("Snowflake helper parameter is too large")),
        })
        .collect()
}
fn column_info((index, column): (usize, Column)) -> ColumnInfo {
    let ty = column.data_type.to_ascii_uppercase();
    let kind = match ty.as_str() {
        "INTEGER" | "BIGINT" => ColumnKind::Integer,
        "FIXED" | "REAL" | "FLOAT" | "NUMBER" | "DECIMAL" => ColumnKind::Decimal,
        "BOOLEAN" => ColumnKind::Boolean,
        "DATE" | "TIME" | "TIMESTAMP" | "TIMESTAMP_NTZ" | "TIMESTAMP_LTZ" | "TIMESTAMP_TZ" => {
            ColumnKind::Temporal
        }
        "VARIANT" | "OBJECT" | "ARRAY" => ColumnKind::Json,
        "BINARY" => ColumnKind::Binary,
        _ => ColumnKind::Text,
    };
    ColumnInfo {
        id: format!("c{index}"),
        name: column.name,
        database_type: column.data_type,
        kind,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: false,
        insertable: false,
        has_default: false,
    }
}
fn check_report(report: &Report) -> AppResult<()> {
    if report.ok {
        return Ok(());
    }
    let code = if report.certainty == Certainty::OutcomeUnknown {
        ErrorCode::OutcomeUnknown
    } else if report.retirement.is_some() {
        ErrorCode::ConnectionClosed
    } else {
        match report.error_code.as_deref() {
            Some("cancelled") => ErrorCode::Cancelled,
            Some("outcomeUnknown") => ErrorCode::OutcomeUnknown,
            Some("connectionClosed") => ErrorCode::ConnectionClosed,
            Some("unsupported") => ErrorCode::Unsupported,
            Some("invalidInput") => ErrorCode::InvalidInput,
            Some("resultLimitReached") => ErrorCode::ResultLimitReached,
            _ => ErrorCode::Database,
        }
    };
    Err(AppError::new(
        code,
        match code {
            ErrorCode::OutcomeUnknown => {
                "Snowflake statement outcome is unknown; inspect query history before retrying"
            }
            ErrorCode::ConnectionClosed => "Snowflake session closed; reconnect before continuing",
            ErrorCode::Cancelled => "Snowflake query cancelled",
            _ => "Snowflake could not complete the operation",
        },
    ))
}
fn control_error(_: &str) -> AppError {
    closed_error()
}
fn closed_error() -> AppError {
    AppError::new(
        ErrorCode::ConnectionClosed,
        "Snowflake session closed; reconnect before continuing",
    )
}
fn protocol_error() -> AppError {
    AppError::new(
        ErrorCode::ConnectionClosed,
        "Invalid Snowflake helper response; reconnect before continuing",
    )
}
fn unsupported() -> AppError {
    AppError::new(
        ErrorCode::Unsupported,
        "This Snowflake external-browser increment supports read queries only",
    )
}
#[derive(Default)]
struct ScalarSink {
    value: Option<String>,
}
impl QuerySink for ScalarSink {
    fn limit(&mut self, _: &str) {}
    fn describe(&mut self, _: Vec<ColumnInfo>) -> AppResult<()> {
        Ok(())
    }
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        if self.value.is_none() {
            self.value = rows.first().and_then(|r| r.first()).cloned().flatten();
        }
        Ok(true)
    }
    fn complete(&mut self, _: u64) -> AppResult<()> {
        Ok(())
    }
}
