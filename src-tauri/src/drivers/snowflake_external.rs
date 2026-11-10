//! Stateful external-browser read adapter. Packaged account workflows and writes remain unqualified.
//! Each instance owns one helper connection; no query or connection is automatically recreated.
use super::snowflake_process::{
    Bounds, Certainty, CleanupObserver, Column, Impact, Launch, OperationToken, Report,
    ShutdownReport, Supervisor,
};
use super::{
    BrowseRows, CompletionRelations, Driver, QuerySink, RowMutation,
    snowflake::Snowflake,
    snowflake_browsing::{self as browsing, BrowseRelation},
    snowflake_metadata::{self as metadata, LiveContext, MetadataResult},
};
use crate::{
    domain::*,
    params::{BoundStatement, TypedBind},
};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
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

#[derive(Default)]
struct BrowseState {
    revision: String,
    snapshot: Option<BrowseSnapshot>,
}
struct BrowseSnapshot {
    context: LiveContext,
    targets: HashMap<String, BrowseRelation>,
    inspections: HashMap<String, InspectionSlot>,
}
struct InspectionSlot {
    revision: String,
    columns: Option<Vec<ColumnInspection>>,
}

pub struct SnowflakeExternal {
    process: Arc<Supervisor>,
    profile: ConnectionProfile,
    session_id: String,
    identity: Value,
    lane: AsyncMutex<()>,
    active: Mutex<Option<Active>>,
    browsing: Mutex<BrowseState>,
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
            browsing: Mutex::new(BrowseState::default()),
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
                relation_browsing: true,
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

    /// Called only under `lane`, with fixed builders owned by this adapter.
    /// Metadata cannot change session settings or create an independent connection.
    async fn metadata_read(
        &self,
        sql: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> AppResult<MetadataResult> {
        let mut sink = MetadataSink::new(max_rows, max_bytes);
        self.stream_bounded(sql, vec![], &mut sink, max_rows as u64, max_bytes as u64)
            .await?;
        Ok(sink.data)
    }

    async fn live_metadata_context(&self) -> AppResult<LiveContext> {
        async {
            let context = self
                .metadata_read(metadata::CONTEXT_SQL, 1, metadata::CONTEXT_MAX_BYTES)
                .await?;
            let policy = self
                .metadata_read(
                    metadata::QUOTED_IDENTIFIER_POLICY_SQL,
                    1,
                    metadata::CONTEXT_MAX_BYTES,
                )
                .await?;
            let observed = metadata::parse_context(&context, &policy)?;
            if self.identity.get("session").and_then(Value::as_str)
                != Some(observed.session_id.as_str())
                || self.identity.get("user").and_then(Value::as_str)
                    != Some(observed.principal.as_str())
            {
                self.start_shutdown();
                return Err(closed_error());
            }
            // An unset database is valid session state, but cannot identify this
            // database-scoped completion workflow. Treat losing it as global.
            observed.completion_identity()?;
            Ok(observed)
        }
        .await
        .map_err(AppError::with_completion_invalidated)
    }

    async fn metadata_operation<T: Send>(
        &self,
        work: impl std::future::Future<Output = AppResult<T>> + Send,
    ) -> AppResult<T> {
        let _lane = tokio::time::timeout(Duration::from_millis(1500), self.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The Snowflake session is busy"))?;
        if self.is_closed() {
            return Err(closed_error().with_completion_invalidated());
        }
        match tokio::time::timeout(Duration::from_secs(65), work).await {
            Ok(result) => {
                if result.as_ref().is_err_and(|error| {
                    matches!(
                        error.code,
                        ErrorCode::ConnectionClosed | ErrorCode::OutcomeUnknown
                    )
                }) {
                    self.start_shutdown();
                }
                if self.is_closed() && result.is_ok() {
                    return Err(closed_error().with_completion_invalidated());
                }
                result.map_err(|error| {
                    if self.is_closed()
                        || matches!(
                            error.code,
                            ErrorCode::ConnectionClosed | ErrorCode::OutcomeUnknown
                        )
                    {
                        error.with_completion_invalidated()
                    } else {
                        error
                    }
                })
            }
            Err(_) => {
                self.start_shutdown();
                Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "Snowflake metadata deadline expired; reconnect before continuing",
                )
                .with_completion_invalidated())
            }
        }
    }

    async fn recheck_metadata_context(&self, before: &LiveContext) -> AppResult<()> {
        if self.live_metadata_context().await? != *before {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Snowflake context changed; refresh suggestions",
            )
            .with_completion_invalidated());
        }
        Ok(())
    }

    fn invalidate_browse_revision(&self, revision: &str) {
        let mut state = self.browsing.lock();
        if state.revision == revision {
            *state = BrowseState::default();
        }
    }

    fn browse_target(
        &self,
        revision: &str,
        target_id: &str,
    ) -> AppResult<(LiveContext, BrowseRelation)> {
        let state = self.browsing.lock();
        if state.revision != revision {
            return Err(stale_browse());
        }
        let snapshot = state.snapshot.as_ref().ok_or_else(stale_browse)?;
        let target = snapshot.targets.get(target_id).ok_or_else(stale_browse)?;
        // This also rejects unavailable/colliding kinds before any helper SQL.
        browsing::relation_sql(&snapshot.context, target)?;
        Ok((snapshot.context.clone(), target.clone()))
    }

    fn browse_columns(
        &self,
        revision: &str,
        target_id: &str,
        inspection: &str,
    ) -> AppResult<Vec<ColumnInspection>> {
        let state = self.browsing.lock();
        if state.revision != revision {
            return Err(stale_browse());
        }
        let slot = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.inspections.get(target_id))
            .filter(|slot| slot.revision == inspection)
            .ok_or_else(stale_browse)?;
        slot.columns.clone().ok_or_else(stale_browse)
    }

    fn require_inspection_attempt(
        &self,
        revision: &str,
        target_id: &str,
        inspection: &str,
    ) -> AppResult<()> {
        let state = self.browsing.lock();
        if state.revision != revision
            || !state.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .inspections
                    .get(target_id)
                    .is_some_and(|slot| slot.revision == inspection)
            })
        {
            return Err(stale_browse());
        }
        Ok(())
    }

    async fn browse_operation<T: Send>(
        &self,
        revision: &str,
        work: impl std::future::Future<Output = AppResult<T>> + Send,
    ) -> AppResult<T> {
        self.metadata_operation(work).await.inspect_err(|error| {
            if error.completion_invalidated == Some(true) {
                self.invalidate_browse_revision(revision);
            }
        })
    }

    async fn browse_context(&self, expected: Option<&LiveContext>) -> AppResult<LiveContext> {
        let context = self.live_metadata_context().await?;
        if context.database.as_deref() != Some(self.profile.database.as_str())
            || expected.is_some_and(|expected| *expected != context)
        {
            return Err(stale_browse().with_completion_invalidated());
        }
        Ok(context)
    }

    async fn require_browse_relation(
        &self,
        context: &LiveContext,
        target: &BrowseRelation,
    ) -> AppResult<()> {
        let result = self
            .metadata_read(
                &browsing::relation_sql(context, target)?,
                3,
                browsing::CATALOG_MAX_BYTES,
            )
            .await?;
        browsing::require_relation(context, target, &result)
            .map_err(AppError::with_completion_invalidated)
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
        self.stream_bounded(wire, binds, sink, QUERY_ROWS, QUERY_BYTES)
            .await
    }

    async fn stream_bounded(
        &self,
        wire: &str,
        binds: Vec<Value>,
        sink: &mut dyn QuerySink,
        max_rows: u64,
        max_bytes: u64,
    ) -> AppResult<u64> {
        let mut operation = self
            .process
            .request(
                "query",
                "query",
                json!({"sql":wire,"binds":binds,
            "classification":"read","credits":1,"maxRows":max_rows,"maxBytes":max_bytes}),
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
    fn invalidate_relation_catalog(&self) {
        *self.browsing.lock() = BrowseState::default();
    }

    async fn relation_catalog(&self) -> AppResult<BrowseCatalog> {
        let revision = uuid::Uuid::new_v4().to_string();
        // Refresh owns a new generation before its first await, even if it fails
        // to acquire the lane or the caller abandons it.
        *self.browsing.lock() = BrowseState {
            revision: revision.clone(),
            snapshot: None,
        };
        self.browse_operation(&revision, async {
            if self.browsing.lock().revision != revision {
                return Err(stale_browse());
            }
            let context = self.browse_context(None).await?;
            let rows = self
                .metadata_read(
                    &browsing::schemas_sql(&context)?,
                    browsing::SCHEMA_LIMIT + 1,
                    browsing::CATALOG_MAX_BYTES,
                )
                .await?;
            let schemas = browsing::parse_schemas(&context, &rows)?;
            let mut relations = Vec::new();
            let mut bytes = 0usize;
            for schema in &schemas {
                let rows = self
                    .metadata_read(
                        &browsing::relations_sql(&context, schema)?,
                        browsing::RELATION_LIMIT + 1,
                        browsing::CATALOG_MAX_BYTES,
                    )
                    .await?;
                let batch = browsing::parse_relations(&context, schema, &rows)?;
                bytes = bytes
                    .checked_add(
                        serde_json::to_vec(&batch)
                            .map_err(|_| protocol_error())?
                            .len(),
                    )
                    .ok_or_else(metadata_limit)?;
                if relations.len().saturating_add(batch.len()) > browsing::RELATION_LIMIT
                    || bytes > browsing::CATALOG_MAX_BYTES
                {
                    return Err(metadata_limit());
                }
                relations.extend(batch);
            }
            browsing::validate_catalog(&context, &schemas, &mut relations)?;
            self.browse_context(Some(&context)).await?;
            let targets: HashMap<_, _> = relations
                .into_iter()
                .map(|relation| (uuid::Uuid::new_v4().to_string(), relation))
                .collect();
            let mut summaries: Vec<_> = targets
                .iter()
                .map(|(id, target)| target.summary(id.clone()))
                .collect();
            summaries.sort_by(|left, right| {
                (
                    &left.table.schema,
                    &left.table.name,
                    &left.table_type,
                    &left.target_id,
                )
                    .cmp(&(
                        &right.table.schema,
                        &right.table.name,
                        &right.table_type,
                        &right.target_id,
                    ))
            });
            let catalog = BrowseCatalog {
                revision: revision.clone(),
                identity: context.completion_identity()?,
                database: self.profile.database.clone(),
                relations: summaries,
            };
            if serde_json::to_vec(&catalog)
                .map_err(|_| protocol_error())?
                .len()
                > browsing::CATALOG_MAX_BYTES
            {
                return Err(metadata_limit());
            }
            let mut state = self.browsing.lock();
            if state.revision != revision {
                return Err(stale_browse());
            }
            state.snapshot = Some(BrowseSnapshot {
                context,
                targets,
                inspections: HashMap::new(),
            });
            Ok(catalog)
        })
        .await
    }

    async fn inspect_relation(
        &self,
        request: &InspectRelationRequest,
    ) -> AppResult<RelationInspection> {
        let (context, target) = self.browse_target(&request.revision, &request.target_id)?;
        let inspection = uuid::Uuid::new_v4().to_string();
        {
            let mut state = self.browsing.lock();
            if state.revision != request.revision {
                return Err(stale_browse());
            }
            state
                .snapshot
                .as_mut()
                .ok_or_else(stale_browse)?
                .inspections
                .insert(
                    request.target_id.clone(),
                    InspectionSlot {
                        revision: inspection.clone(),
                        columns: None,
                    },
                );
        }
        self.browse_operation(&request.revision, async {
            self.require_inspection_attempt(&request.revision, &request.target_id, &inspection)?;
            self.browse_context(Some(&context)).await?;
            self.require_browse_relation(&context, &target).await?;
            let columns = self
                .metadata_read(
                    &browsing::columns_sql(&target)?,
                    browsing::COLUMN_LIMIT + 1,
                    browsing::COLUMN_MAX_BYTES,
                )
                .await?;
            let describe = self
                .metadata_read(
                    &browsing::describe_sql(&target)?,
                    browsing::COLUMN_LIMIT + 1,
                    browsing::COLUMN_MAX_BYTES,
                )
                .await?;
            let resolved = self
                .metadata_read(
                    &browsing::resolved_columns_sql(&target)?,
                    0,
                    browsing::COLUMN_MAX_BYTES,
                )
                .await?;
            let columns = browsing::parse_inspection(&target, &columns, &describe, &resolved)?;
            self.require_browse_relation(&context, &target).await?;
            self.browse_context(Some(&context)).await?;
            let result = RelationInspection {
                revision: inspection.clone(),
                target_id: request.target_id.clone(),
                database: target.database,
                table: target.table,
                columns: columns.clone(),
                constraints: ConstraintInspectionStatus::NotFetched,
            };
            if serde_json::to_vec(&result)
                .map_err(|_| protocol_error())?
                .len()
                > browsing::COLUMN_MAX_BYTES
            {
                return Err(metadata_limit());
            }
            let mut state = self.browsing.lock();
            if state.revision != request.revision {
                return Err(stale_browse());
            }
            let slot = state
                .snapshot
                .as_mut()
                .and_then(|snapshot| snapshot.inspections.get_mut(&request.target_id))
                .filter(|slot| slot.revision == inspection)
                .ok_or_else(stale_browse)?;
            slot.columns = Some(columns);
            Ok(result)
        })
        .await
    }

    async fn preview_relation(
        &self,
        request: &PreviewRelationRequest,
    ) -> AppResult<RelationPreview> {
        let (context, target) = self.browse_target(&request.revision, &request.target_id)?;
        let columns = self.browse_columns(
            &request.revision,
            &request.target_id,
            &request.inspection_revision,
        )?;
        self.browse_operation(&request.revision, async {
            // A request may have waited behind a refresh or replacement inspection.
            self.browse_columns(
                &request.revision,
                &request.target_id,
                &request.inspection_revision,
            )?;
            self.browse_context(Some(&context)).await?;
            self.require_browse_relation(&context, &target).await?;
            let rows = self
                .metadata_read(
                    &browsing::preview_sql(&target, &columns)?,
                    browsing::PREVIEW_LIMIT + 1,
                    browsing::PREVIEW_MAX_BYTES,
                )
                .await?;
            let preview = browsing::parse_preview(&columns, &rows)?;
            self.require_browse_relation(&context, &target).await?;
            self.browse_context(Some(&context)).await?;
            self.browse_columns(
                &request.revision,
                &request.target_id,
                &request.inspection_revision,
            )?;
            Ok(preview)
        })
        .await
    }

    async fn completion_identity(&self) -> AppResult<CompletionIdentity> {
        self.metadata_operation(async {
            let context = self.live_metadata_context().await?;
            let identity = context.completion_identity()?;
            self.recheck_metadata_context(&context).await?;
            Ok(identity)
        })
        .await
    }

    async fn completion_catalog(
        &self,
        identity: &CompletionIdentity,
    ) -> AppResult<CompletionRelations> {
        self.metadata_operation(async {
            let context = self.live_metadata_context().await?;
            if context.completion_identity()? != *identity {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Snowflake context changed; refresh suggestions",
                ));
            }
            let schemas = self
                .metadata_read(
                    &metadata::schemas_sql(&context)?,
                    metadata::SCHEMA_LIMIT + 1,
                    metadata::RELATION_MAX_BYTES,
                )
                .await?;
            let mut relations = Vec::new();
            let mut relation_bytes = 0usize;
            for schema in metadata::parse_schemas(&context, &schemas)? {
                let rows = self
                    .metadata_read(
                        &metadata::relations_sql(&context, &schema)?,
                        metadata::RELATION_LIMIT + 1,
                        metadata::RELATION_MAX_BYTES,
                    )
                    .await?;
                let batch = metadata::parse_relations(&context, &schema, &rows)?;
                relation_bytes = relation_bytes
                    .checked_add(
                        serde_json::to_vec(&batch)
                            .map_err(|_| protocol_error())?
                            .len(),
                    )
                    .ok_or_else(metadata_limit)?;
                // Bound accumulation before retaining another schema's results.
                if relations.len().saturating_add(batch.len()) > metadata::RELATION_LIMIT
                    || relation_bytes > metadata::RELATION_MAX_BYTES
                {
                    return Err(metadata_limit());
                }
                relations.extend(batch);
            }
            let result = metadata::completion_relations(&context, relations)?;
            self.recheck_metadata_context(&context).await?;
            Ok(result)
        })
        .await
    }

    async fn completion_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.metadata_operation(async {
            let context = self.live_metadata_context().await?;
            let rows = self
                .metadata_read(
                    &metadata::resolved_columns_sql(&context, table)?,
                    0,
                    metadata::COLUMN_MAX_BYTES,
                )
                .await?;
            let schema = metadata::parse_resolved_columns(table, rows)?;
            self.recheck_metadata_context(&context).await?;
            Ok(schema)
        })
        .await
    }

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

fn stale_browse() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake relation context changed; refresh relations and inspect again",
    )
}

struct MetadataSink {
    data: MetadataResult,
    max_rows: usize,
    max_bytes: usize,
    bytes: usize,
}
impl MetadataSink {
    fn new(max_rows: usize, max_bytes: usize) -> Self {
        Self {
            data: MetadataResult {
                columns: vec![],
                rows: vec![],
                rows_seen: 0,
                truncated: false,
            },
            max_rows,
            max_bytes,
            bytes: 0,
        }
    }
    fn retain_bytes(&mut self, bytes: usize) -> AppResult<()> {
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(metadata_limit)?;
        if self.bytes > self.max_bytes {
            return Err(metadata_limit());
        }
        Ok(())
    }
}
fn metadata_limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Snowflake metadata exceeds its complete-result budget",
    )
}
impl QuerySink for MetadataSink {
    fn limit(&mut self, _: &str) {
        self.data.truncated = true;
    }
    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()> {
        if columns.len() > metadata::COLUMN_LIMIT {
            return Err(metadata_limit());
        }
        self.retain_bytes(
            serde_json::to_vec(&columns)
                .map_err(|_| protocol_error())?
                .len(),
        )?;
        self.data.columns = columns;
        Ok(())
    }
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        if self.data.rows.len().saturating_add(rows.len()) > self.max_rows {
            return Err(metadata_limit());
        }
        if rows.iter().any(|row| row.len() != self.data.columns.len()) {
            return Err(protocol_error());
        }
        self.retain_bytes(
            serde_json::to_vec(&rows)
                .map_err(|_| protocol_error())?
                .len(),
        )?;
        self.data.rows.extend(rows);
        Ok(true)
    }
    fn complete(&mut self, rows_seen: u64) -> AppResult<()> {
        self.data.rows_seen = rows_seen;
        if self.data.truncated || rows_seen != self.data.rows.len() as u64 {
            return Err(metadata_limit());
        }
        Ok(())
    }
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
