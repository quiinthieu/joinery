//! Stateful external-browser adapter with separately admitted reviewed DML and DDL.
//! Packaged account workflows and actual mutation effects remain unqualified.
//! Each instance owns one helper connection; no query or connection is automatically recreated.
use super::snowflake_process::{
    Bounds, Certainty, CleanupObserver, Column, Impact, Launch, Operation, OperationToken, Report,
    ShutdownReport, Supervisor, TransactionAction,
};
#[cfg(all(unix, feature = "test-harness"))]
use super::snowflake_qualification_metadata::{
    MetadataAttachment, MetadataSite, RelationObservation, SchemaObservation,
};
use super::{
    BrowseRows, CompletionRelations, Driver, QuerySink, RowMutation,
    snowflake::Snowflake,
    snowflake_browsing::{self as browsing, BrowseRelation},
    snowflake_explorer as explorer,
    snowflake_metadata::{self as metadata, LiveContext, MetadataResult},
    snowflake_session::{self as session, ObservationSample, SampleConsistency},
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

pub(crate) mod console;
mod edit_snapshots;
pub(crate) mod grid;
mod grid_freshness;

const SQL_BYTES: usize = 262_144;
const DDL_GRANT_TTL: Duration = Duration::from_secs(60);

/// LOGIN_NAME need not equal CURRENT_USER. Only enforce a principal when explicitly configured.
#[derive(Default)]
pub struct ExternalBrowserOptions {
    pub expected_server_user: Option<String>,
    #[cfg(all(unix, feature = "test-harness"))]
    pub(crate) metadata_observer: Option<MetadataAttachment>,
}

impl ExternalBrowserOptions {
    pub fn with_expected_server_user(mut self, user: String) -> Self {
        self.expected_server_user = Some(user);
        self
    }
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
    preview_revision: Option<String>,
}

#[derive(Default)]
struct SessionOptionsState {
    revision: String,
    snapshot: Option<SessionOptionsSnapshot>,
}
struct SessionOptionsSnapshot {
    sample: ObservationSample,
    available: Vec<String>,
}

#[derive(Default)]
struct ContextCatalogState {
    revision: String,
    snapshot: Option<ContextCatalogSnapshot>,
}
struct ContextCatalogSnapshot {
    sample: ObservationSample,
    roles: Vec<String>,
    warehouses: Vec<String>,
    databases: Vec<String>,
    schemas: Vec<String>,
}

#[derive(Default)]
struct TransactionAuthorityState {
    generation: String,
    grant: Option<TransactionGrant>,
    // Independent of options/catalog generations. The helper pins this same
    // principal/session at startup; ordinary failed reads cannot erase recovery.
    recovery: Option<TransactionRecovery>,
}
struct TransactionGrant {
    revision: String,
    sample: ObservationSample,
}
struct TransactionRecovery {
    revision: String,
    restricted: bool,
}
struct TransactionLease<'a> {
    driver: &'a SnowflakeExternal,
    operation: Option<Operation>,
    settled: bool,
}
impl Drop for TransactionLease<'_> {
    fn drop(&mut self) {
        if self.operation.is_some() && !self.settled {
            self.driver.start_shutdown();
        }
    }
}

// Keep the switch ticket, and its independent USE receipt, alive through the
// final observation and publication. Dropping any admitted workflow closes it.
struct SchemaSwitchLease<'a> {
    driver: &'a SnowflakeExternal,
    operation: Option<Operation>,
    published: bool,
}
impl Drop for SchemaSwitchLease<'_> {
    fn drop(&mut self) {
        if self.operation.is_some() && !self.published {
            self.driver.start_shutdown();
        }
    }
}

#[derive(Default)]
struct DdlAuthorityState {
    generation: String,
    grant: Option<DdlGrant>,
}
type CompiledDdlAction = (
    crate::safety::snowflake_ddl::SnowflakeCanonical,
    Option<(LiveContext, BrowseRelation)>,
    Option<Vec<crate::relation_inspection::ColumnInspection>>,
);

struct DdlGrant {
    revision: String,
    issued_at: Instant,
    transaction_generation: String,
    sample: ObservationSample,
    action: SnowflakeDdlAction,
    canonical: crate::safety::snowflake_ddl::SnowflakeCanonical,
    target: Option<BrowseRelation>,
    inspected_columns: Option<Vec<crate::relation_inspection::ColumnInspection>>,
}

pub struct SnowflakeExternal {
    process: Arc<Supervisor>,
    profile: ConnectionProfile,
    session_id: String,
    identity: Value,
    server_version: Mutex<String>,
    lane: Arc<AsyncMutex<()>>,
    grid: grid::State,
    // Permanent once claimed; releasing a run never reenables ordinary probes.
    console_mode: Mutex<Option<String>>,
    active: Mutex<Option<Active>>,
    active_database: Mutex<Option<String>>,
    browsing: Mutex<BrowseState>,
    edit_snapshots: Arc<super::snowflake_edit_snapshot::SnapshotStore>,
    session_options: Mutex<SessionOptionsState>,
    context_catalog: Mutex<ContextCatalogState>,
    transaction_authority: Mutex<TransactionAuthorityState>,
    ddl_authority: Mutex<DdlAuthorityState>,
    closed: CancellationToken,
    shutdown_started: AtomicBool,
    shutdown: watch::Sender<Option<Arc<ShutdownReport>>>,
    runtime: tokio::runtime::Handle,
    #[cfg(all(unix, feature = "test-harness"))]
    metadata_observer: Option<MetadataAttachment>,
}

impl SnowflakeExternal {
    #[cfg(all(unix, feature = "test-harness"))]
    fn check_metadata_observer(&self) -> AppResult<()> {
        self.metadata_observer
            .as_ref()
            .map_or(Ok(()), MetadataAttachment::check)
    }

    #[cfg(all(unix, feature = "test-harness"))]
    fn observe_schemas(
        &self,
        site: MetadataSite,
        context: &LiveContext,
        rows: &MetadataResult,
        parsed_names: &[String],
    ) -> AppResult<()> {
        if let Some(observer) = &self.metadata_observer {
            // Only called after the production parser accepted the complete
            // result. Its sorted output must not erase original wire order.
            let wire_names = rows
                .rows
                .iter()
                .map(|row| {
                    row.get(1)
                        .and_then(Option::as_ref)
                        .cloned()
                        .ok_or_else(protocol_error)
                })
                .collect::<AppResult<Vec<_>>>()?;
            observer.observe_schemas(SchemaObservation {
                site,
                context,
                wire_names: &wire_names,
                parsed_names,
            })?;
        }
        Ok(())
    }

    #[cfg(all(unix, feature = "test-harness"))]
    fn observe_relations(
        &self,
        site: MetadataSite,
        context: &LiveContext,
        schema: Option<&str>,
        relations: &[BrowseRelation],
    ) -> AppResult<()> {
        if let Some(observer) = &self.metadata_observer {
            observer.observe_relations(RelationObservation {
                site,
                context,
                schema,
                relations,
            })?;
        }
        Ok(())
    }

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
        #[cfg(all(unix, feature = "test-harness"))]
        if let Some(observer) = &options.metadata_observer {
            observer.check()?;
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
        let mut connect_payload = json!({
            "account": account, "host": host, "username": profile.username,
            "role": config.role, "warehouse": config.warehouse,
            "database": profile.database, "schema": config.schema,
        });
        // This closed scalar is present only on permanently reviewed-console
        // profiles. Ordinary and fixed qualification transport remains unchanged.
        if config.reviewed_console {
            connect_payload["reviewedConsoleTimeoutSeconds"] =
                json!(config.effective_console_timeout_seconds()?);
        }
        let mut connect = process
            .request("connect", "connect", connect_payload, Impact::Control)
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
            server_version: Mutex::new(String::new()),
            lane: Arc::new(AsyncMutex::new(())),
            grid: grid::State::default(),
            console_mode: Mutex::new(None),
            active: Mutex::new(None),
            active_database: Mutex::new(Some(profile.database.clone())),
            browsing: Mutex::new(BrowseState::default()),
            edit_snapshots: edit_snapshots::shared_store(),
            session_options: Mutex::new(SessionOptionsState::default()),
            context_catalog: Mutex::new(ContextCatalogState::default()),
            transaction_authority: Mutex::new(TransactionAuthorityState::default()),
            ddl_authority: Mutex::new(DdlAuthorityState::default()),
            closed: CancellationToken::new(),
            shutdown_started: AtomicBool::new(false),
            shutdown,
            runtime: tokio::runtime::Handle::current(),
            #[cfg(all(unix, feature = "test-harness"))]
            metadata_observer: options.metadata_observer,
        });
        // Obtain the version on the same SDK session rather than infer it from the local package.
        let mut version = ScalarSink::default();
        let version_read = metadata::version_read();
        // Fixed internal metadata SQL is not an owner-supplied classification hint.
        tokio::time::timeout(
            Duration::from_secs(65),
            driver.stream_bounded(
                version_read.sql(),
                version_read.binds().to_vec(),
                &mut version,
                version_read.max_rows(),
                version_read.max_bytes(),
            ),
        )
        .await
        .map_err(|_| closed_error())??;
        let server_version = version
            .value
            .filter(|v| !v.is_empty())
            .ok_or_else(protocol_error)?;
        *driver.server_version.lock() = server_version.clone();
        let info = SessionInfo {
            console_mode: false,
            id: session_id.into(),
            connection_id: profile.id.clone(),
            engine: EngineKind::Snowflake,
            server_version,
            latency_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
            encrypted: true,
            capabilities: EngineCapabilities {
                sql: true,
                transactions: false,
                explicit_transaction_controls: true,
                cancellation: true,
                table_browsing: false,
                relation_browsing: true,
                table_editing: false,
                table_filtering: false,
                processes: false,
                object_explorer: !config.reviewed_console,
                schema_diagram: !config.reviewed_console,
                schema_diff: !config.reviewed_console,
                ddl_editing: false,
                snowflake_reviewed_ddl: config.reviewed_ddl
                    && profile.safe_mode != SafeMode::ReadOnly,
                snowflake_grid_editing: config.reviewed_dml
                    && !config.reviewed_console
                    && profile.safe_mode != SafeMode::ReadOnly,
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
    /// Capture before disconnecting; observing reaping must not retain this driver.
    pub fn retirement_observer(&self) -> super::snowflake_process::HelperRetirementObserver {
        self.process.retirement_observer()
    }
    /// A logical close is immediate; only this receipt/cleanup observer proves logout/reap.
    pub fn shutdown_receiver(&self) -> watch::Receiver<Option<Arc<ShutdownReport>>> {
        self.shutdown.subscribe()
    }
    fn start_shutdown(&self) {
        {
            // Serialize local revocation with the console's final dispatch check.
            let _mode = self.console_mode.lock();
            self.closed.cancel();
        }
        // Revocation and preview publication share browsing -> store ordering.
        // Repeat even after an earlier shutdown started; no authority survives.
        {
            let _browsing = self.browsing.lock();
            self.revoke_edit_snapshots();
        }
        *self.transaction_authority.lock() = TransactionAuthorityState::default();
        *self.ddl_authority.lock() = DdlAuthorityState::default();
        self.invalidate_session_options();
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
        self.metadata_read_bound(sql, vec![], max_rows, max_bytes)
            .await
    }

    async fn metadata_read_bound(
        &self,
        sql: &str,
        binds: Vec<Value>,
        max_rows: usize,
        max_bytes: usize,
    ) -> AppResult<MetadataResult> {
        let mut sink = MetadataSink::new(max_rows, max_bytes);
        self.stream_bounded(sql, binds, &mut sink, max_rows as u64, max_bytes as u64)
            .await?;
        Ok(sink.data)
    }

    async fn read_spec(&self, spec: &metadata::ReadSpec) -> AppResult<MetadataResult> {
        let rows = usize::try_from(spec.max_rows()).map_err(|_| metadata_limit())?;
        let bytes = usize::try_from(spec.max_bytes()).map_err(|_| metadata_limit())?;
        let mut sink = MetadataSink::new(rows, bytes);
        self.stream_bounded(
            spec.sql(),
            spec.binds().to_vec(),
            &mut sink,
            spec.max_rows(),
            spec.max_bytes(),
        )
        .await?;
        Ok(sink.data)
    }

    async fn live_metadata_context(&self) -> AppResult<LiveContext> {
        async {
            let [context_read, policy_read] = metadata::context_reads();
            let context = self.read_spec(&context_read).await?;
            let policy = self.read_spec(&policy_read).await?;
            let observed = metadata::parse_context(&context, &policy)?;
            self.require_session_identity(&observed)?;
            // An unset database is valid session state, but cannot identify this
            // database-scoped completion workflow. Treat losing it as global.
            observed.completion_identity()?;
            let changed = self
                .session_options
                .lock()
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.sample.context != observed)
                || self
                    .context_catalog
                    .lock()
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.sample.context != observed);
            if changed {
                self.invalidate_session_options();
            }
            Ok(observed)
        }
        .await
        .map_err(|error: AppError| {
            self.invalidate_session_options();
            error.with_completion_invalidated()
        })
    }

    fn require_session_identity(&self, context: &LiveContext) -> AppResult<()> {
        if self.identity.get("session").and_then(Value::as_str) != Some(context.session_id.as_str())
            || self.identity.get("user").and_then(Value::as_str) != Some(context.principal.as_str())
        {
            self.start_shutdown();
            return Err(closed_error());
        }
        Ok(())
    }

    fn require_unrestricted(&self) -> AppResult<()> {
        self.require_ordinary_mode()?;
        #[cfg(all(unix, feature = "test-harness"))]
        self.check_metadata_observer()?;
        if self.is_closed() {
            return Err(closed_error());
        }
        if self
            .transaction_authority
            .lock()
            .recovery
            .as_ref()
            .is_some_and(|r| r.restricted)
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake transaction state is unavailable; use the reviewed rollback control or disconnect",
            ));
        }
        Ok(())
    }

    fn revoke_transaction_grant(&self) -> String {
        let mut state = self.transaction_authority.lock();
        let generation = new_id();
        state.generation = generation.clone();
        state.grant = None;
        drop(state);
        self.revoke_ddl_grant();
        generation
    }

    fn revoke_ddl_grant(&self) -> String {
        let mut state = self.ddl_authority.lock();
        let generation = new_id();
        state.generation = generation.clone();
        state.grant = None;
        generation
    }

    fn require_ddl_generation(&self, generation: &str) -> AppResult<()> {
        if self.is_closed() {
            return Err(closed_error());
        }
        if self.ddl_authority.lock().generation != generation {
            return Err(stale_ddl());
        }
        Ok(())
    }

    fn require_transaction_generation(&self, generation: &str) -> AppResult<()> {
        if self.is_closed() {
            return Err(closed_error());
        }
        if self.transaction_authority.lock().generation != generation {
            return Err(stale_transaction());
        }
        Ok(())
    }

    fn restrict_transaction_recovery(&self, generation: &str) -> bool {
        let mut state = self.transaction_authority.lock();
        if state.generation != generation || self.is_closed() {
            return false;
        }
        state.grant = None;
        if let Some(recovery) = &mut state.recovery {
            // Preserve an already exposed recovery revision through any number
            // of failed refreshes; each admitted rollback consumes it once.
            recovery.restricted = true;
            return true;
        }
        false
    }

    fn transaction_authority(&self) -> Option<TransactionControlAuthority> {
        let state = self.transaction_authority.lock();
        if let Some(recovery) = &state.recovery
            && recovery.restricted
        {
            return Some(TransactionControlAuthority::RollbackOnly {
                revision: recovery.revision.clone(),
            });
        }
        state
            .grant
            .as_ref()
            .map(|grant| TransactionControlAuthority::Observed {
                revision: grant.revision.clone(),
                allowed_actions: self.transaction_actions(&grant.sample),
            })
    }

    fn transaction_actions(&self, sample: &ObservationSample) -> Vec<ExplicitTransactionAction> {
        let mut actions = vec![ExplicitTransactionAction::Rollback];
        if self.profile.safe_mode != SafeMode::ReadOnly {
            actions.insert(
                0,
                if sample.transaction_id.is_some() {
                    ExplicitTransactionAction::Commit
                } else {
                    ExplicitTransactionAction::Begin
                },
            );
        }
        actions
    }

    fn validate_transaction_locked(
        &self,
        state: &TransactionAuthorityState,
        request: &TransactionControlRequest,
    ) -> AppResult<()> {
        self.require_ordinary_mode()?;
        if self.is_closed() {
            return Err(closed_error());
        }
        if request.action != ExplicitTransactionAction::Rollback
            && self.profile.safe_mode == SafeMode::ReadOnly
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This Snowflake profile allows read-only operations",
            ));
        }
        if let Some(recovery) = &state.recovery
            && recovery.restricted
        {
            return if request.action == ExplicitTransactionAction::Rollback
                && request.revision == recovery.revision
            {
                Ok(())
            } else {
                Err(stale_transaction())
            };
        }
        let grant = state
            .grant
            .as_ref()
            .filter(|grant| grant.revision == request.revision)
            .ok_or_else(stale_transaction)?;
        if !self
            .transaction_actions(&grant.sample)
            .contains(&request.action)
        {
            return Err(stale_transaction());
        }
        Ok(())
    }

    fn validate_dml_locked(
        &self,
        state: &TransactionAuthorityState,
        request: &SnowflakeDmlRequest,
    ) -> AppResult<crate::safety::snowflake_dml::Canonical> {
        self.require_ordinary_mode()?;
        if self.is_closed() {
            return Err(closed_error());
        }
        if self.profile.safe_mode == SafeMode::ReadOnly
            || !self.profile.snowflake.as_ref().is_some_and(|config| {
                config.authentication == SnowflakeAuthentication::ExternalBrowser
                    && config.reviewed_dml
            })
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake DML requires an explicitly enabled external-browser profile with writes allowed",
            ));
        }
        if state
            .recovery
            .as_ref()
            .is_some_and(|recovery| recovery.restricted)
            || state
                .grant
                .as_ref()
                .is_none_or(|grant| grant.revision != request.revision)
        {
            return Err(stale_transaction());
        }
        let active_database = self.require_active_database()?;
        crate::safety::snowflake_dml::compile(
            &request.sql,
            Some(&request.parameters),
            &active_database,
        )
    }

    fn require_active_database(&self) -> AppResult<String> {
        self.active_database
            .lock()
            .clone()
            .ok_or_else(stale_session_options)
    }

    fn require_ddl_policy(&self) -> AppResult<()> {
        self.require_ordinary_mode()?;
        if self.is_closed() {
            return Err(closed_error());
        }
        if self.profile.safe_mode == SafeMode::ReadOnly
            || !self.profile.snowflake.as_ref().is_some_and(|config| {
                config.authentication == SnowflakeAuthentication::ExternalBrowser
                    && config.reviewed_ddl
            })
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake DDL requires an explicitly enabled external-browser profile with writes allowed",
            ));
        }
        Ok(())
    }

    fn require_ddl_session_sample(&self, sample: &ObservationSample) -> AppResult<()> {
        let Some(active_database) = self.active_database.lock().clone() else {
            return Err(stale_ddl());
        };
        if sample.context.database.as_deref() != Some(active_database.as_str()) {
            return Err(stale_ddl());
        }
        if !sample.autocommit || sample.transaction_id.is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Snowflake reviewed DDL requires an idle session with AUTOCOMMIT enabled; commit or roll back active transactions first",
            ));
        }
        Ok(())
    }

    fn compile_ddl_action(&self, action: &SnowflakeDdlAction) -> AppResult<CompiledDdlAction> {
        let active_database = self.active_database.lock().clone().ok_or_else(stale_ddl)?;
        match action {
            SnowflakeDdlAction::CreateTable {
                database, schema, ..
            }
            | SnowflakeDdlAction::CreateView {
                database, schema, ..
            } => {
                if schema == "INFORMATION_SCHEMA" {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "Reviewed Snowflake DDL cannot create objects in INFORMATION_SCHEMA",
                    ));
                }
                let canonical = crate::safety::snowflake_ddl::compile_snowflake_action(
                    action,
                    None,
                    &active_database,
                )?;
                if database != &active_database {
                    return Err(stale_ddl());
                }
                Ok((canonical, None, None))
            }
            SnowflakeDdlAction::AddColumn {
                browse_revision,
                target_id,
                inspection_revision,
                column,
            } => {
                let (browse_ctx, target) = self.browse_target(browse_revision, target_id)?;
                require_plain_base_table(&target)?;
                let inspected_columns =
                    self.browse_columns(browse_revision, target_id, inspection_revision)?;
                if inspected_columns.len() >= browsing::COLUMN_LIMIT {
                    return Err(metadata_limit());
                }
                let canonical = crate::safety::snowflake_ddl::compile_snowflake_action(
                    action,
                    Some(&target.table),
                    &active_database,
                )?;
                if target.database != active_database {
                    return Err(stale_ddl());
                }
                if inspected_columns
                    .iter()
                    .any(|existing| existing.name == column.name)
                {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "A column with this exact name already exists on the inspected table",
                    ));
                }
                Ok((
                    canonical,
                    Some((browse_ctx, target)),
                    Some(inspected_columns),
                ))
            }
            SnowflakeDdlAction::AlterColumnNullability {
                browse_revision,
                target_id,
                inspection_revision,
                column_name,
                nullable,
            } => {
                let (browse_ctx, target) = self.browse_target(browse_revision, target_id)?;
                require_plain_base_table(&target)?;
                let inspected_columns =
                    self.browse_columns(browse_revision, target_id, inspection_revision)?;
                let canonical = crate::safety::snowflake_ddl::compile_snowflake_action(
                    action,
                    Some(&target.table),
                    &active_database,
                )?;
                if target.database != active_database {
                    return Err(stale_ddl());
                }
                let Some(existing) = inspected_columns
                    .iter()
                    .find(|existing| existing.name == *column_name)
                else {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "The selected column no longer exists on the inspected table",
                    ));
                };
                if existing.kind != crate::relation_inspection::InspectedColumnKind::Column {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "Virtual or partition columns cannot change nullability",
                    ));
                }
                if existing.nullable == *nullable {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "The selected column already has the requested nullability",
                    ));
                }
                Ok((
                    canonical,
                    Some((browse_ctx, target)),
                    Some(inspected_columns),
                ))
            }
            SnowflakeDdlAction::RenameColumn {
                browse_revision,
                target_id,
                inspection_revision,
                column_name,
                new_name,
            } => {
                let (browse_ctx, target) = self.browse_target(browse_revision, target_id)?;
                require_plain_base_table(&target)?;
                let inspected_columns =
                    self.browse_columns(browse_revision, target_id, inspection_revision)?;
                let canonical = crate::safety::snowflake_ddl::compile_snowflake_action(
                    action,
                    Some(&target.table),
                    &active_database,
                )?;
                if target.database != active_database {
                    return Err(stale_ddl());
                }
                if !inspected_columns
                    .iter()
                    .any(|existing| existing.name == *column_name)
                {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "The selected column no longer exists on the inspected table",
                    ));
                }
                if inspected_columns
                    .iter()
                    .any(|existing| existing.name == *new_name)
                {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "A column with the new exact name already exists on the inspected table",
                    ));
                }
                Ok((
                    canonical,
                    Some((browse_ctx, target)),
                    Some(inspected_columns),
                ))
            }
            SnowflakeDdlAction::DropColumn {
                browse_revision,
                target_id,
                inspection_revision,
                column_name,
            } => {
                let (browse_ctx, target) = self.browse_target(browse_revision, target_id)?;
                require_plain_base_table(&target)?;
                let inspected_columns =
                    self.browse_columns(browse_revision, target_id, inspection_revision)?;
                if inspected_columns.len() <= 1 {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "Snowflake tables cannot drop their last remaining column; drop the table instead",
                    ));
                }
                let canonical = crate::safety::snowflake_ddl::compile_snowflake_action(
                    action,
                    Some(&target.table),
                    &active_database,
                )?;
                if target.database != active_database {
                    return Err(stale_ddl());
                }
                if !inspected_columns
                    .iter()
                    .any(|existing| existing.name == *column_name)
                {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "The selected column no longer exists on the inspected table",
                    ));
                }
                Ok((
                    canonical,
                    Some((browse_ctx, target)),
                    Some(inspected_columns),
                ))
            }
            SnowflakeDdlAction::DropTable {
                browse_revision,
                target_id,
            } => {
                let (browse_ctx, target) = self.browse_target(browse_revision, target_id)?;
                require_plain_base_table(&target)?;
                let canonical = crate::safety::snowflake_ddl::compile_snowflake_action(
                    action,
                    Some(&target.table),
                    &active_database,
                )?;
                if target.database != active_database {
                    return Err(stale_ddl());
                }
                Ok((canonical, Some((browse_ctx, target)), None))
            }
            SnowflakeDdlAction::DropView {
                browse_revision,
                target_id,
            } => {
                let (browse_ctx, target) = self.browse_target(browse_revision, target_id)?;
                require_plain_view(&target)?;
                let canonical = crate::safety::snowflake_ddl::compile_snowflake_action(
                    action,
                    Some(&target.table),
                    &active_database,
                )?;
                if target.database != active_database {
                    return Err(stale_ddl());
                }
                Ok((canonical, Some((browse_ctx, target)), None))
            }
        }
    }

    async fn verify_ddl_catalog_state(
        &self,
        context: &LiveContext,
        action: &SnowflakeDdlAction,
        target: Option<&BrowseRelation>,
        inspected_columns: Option<&[crate::relation_inspection::ColumnInspection]>,
    ) -> AppResult<()> {
        let active_database = self.active_database.lock().clone().ok_or_else(stale_ddl)?;
        match action {
            SnowflakeDdlAction::CreateTable {
                database,
                schema,
                name,
                ..
            }
            | SnowflakeDdlAction::CreateView {
                database,
                schema,
                name,
                ..
            } => {
                if database != &active_database
                    || context.database.as_deref() != Some(database.as_str())
                {
                    return Err(stale_ddl());
                }
                if schema == "INFORMATION_SCHEMA" {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "Reviewed Snowflake DDL cannot create objects in INFORMATION_SCHEMA",
                    ));
                }
                let schema_rows = self.read_spec(&metadata::schemas_read(context)?).await?;
                let schemas = browsing::parse_schemas(context, &schema_rows)?;
                #[cfg(all(unix, feature = "test-harness"))]
                self.observe_schemas(
                    MetadataSite::CreateTargetCheck,
                    context,
                    &schema_rows,
                    &schemas,
                )?;
                if !schemas.iter().any(|existing| existing == schema) {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "The target schema is not available in the current Snowflake database; refresh schemas and relations first",
                    ));
                }
                let relation_rows = self
                    .read_spec(&metadata::relations_read(context, schema)?)
                    .await?;
                let relations = browsing::parse_relations(context, schema, &relation_rows)?;
                #[cfg(all(unix, feature = "test-harness"))]
                self.observe_relations(
                    MetadataSite::CreateTargetCheck,
                    context,
                    Some(schema),
                    &relations,
                )?;
                if relations
                    .iter()
                    .any(|relation| relation.table.name == *name)
                {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "A relation with this exact name already exists in the target schema",
                    ));
                }
                Ok(())
            }
            SnowflakeDdlAction::AddColumn { column, .. } => {
                let target = target.ok_or_else(stale_ddl)?;
                let expected_columns = inspected_columns.ok_or_else(stale_ddl)?;
                require_plain_base_table(target)?;
                self.require_browse_relation(context, target).await?;
                let [columns_read, describe_read, resolved_read] =
                    metadata::inspection_reads(target)?;
                let columns = self.read_spec(&columns_read).await?;
                let describe = self.read_spec(&describe_read).await?;
                let resolved = self.read_spec(&resolved_read).await?;
                let live_columns =
                    browsing::parse_inspection(target, &columns, &describe, &resolved)?;
                if live_columns.as_slice() != expected_columns {
                    return Err(stale_ddl());
                }
                if live_columns
                    .iter()
                    .any(|existing| existing.name == column.name)
                {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "A column with this exact name already exists on the inspected table",
                    ));
                }
                self.require_browse_relation(context, target).await
            }
            SnowflakeDdlAction::AlterColumnNullability {
                column_name,
                nullable,
                ..
            } => {
                let target = target.ok_or_else(stale_ddl)?;
                let expected_columns = inspected_columns.ok_or_else(stale_ddl)?;
                require_plain_base_table(target)?;
                self.require_browse_relation(context, target).await?;
                let [columns_read, describe_read, resolved_read] =
                    metadata::inspection_reads(target)?;
                let columns = self.read_spec(&columns_read).await?;
                let describe = self.read_spec(&describe_read).await?;
                let resolved = self.read_spec(&resolved_read).await?;
                let live_columns =
                    browsing::parse_inspection(target, &columns, &describe, &resolved)?;
                if live_columns.as_slice() != expected_columns {
                    return Err(stale_ddl());
                }
                let Some(existing) = live_columns
                    .iter()
                    .find(|existing| existing.name == *column_name)
                else {
                    return Err(stale_ddl());
                };
                if existing.nullable == *nullable {
                    return Err(stale_ddl());
                }
                self.require_browse_relation(context, target).await
            }
            SnowflakeDdlAction::RenameColumn {
                column_name,
                new_name,
                ..
            } => {
                let target = target.ok_or_else(stale_ddl)?;
                let expected_columns = inspected_columns.ok_or_else(stale_ddl)?;
                require_plain_base_table(target)?;
                self.require_browse_relation(context, target).await?;
                let [columns_read, describe_read, resolved_read] =
                    metadata::inspection_reads(target)?;
                let columns = self.read_spec(&columns_read).await?;
                let describe = self.read_spec(&describe_read).await?;
                let resolved = self.read_spec(&resolved_read).await?;
                let live_columns =
                    browsing::parse_inspection(target, &columns, &describe, &resolved)?;
                if live_columns.as_slice() != expected_columns
                    || !live_columns.iter().any(|c| c.name == *column_name)
                    || live_columns.iter().any(|c| c.name == *new_name)
                {
                    return Err(stale_ddl());
                }
                self.require_browse_relation(context, target).await
            }
            SnowflakeDdlAction::DropColumn { column_name, .. } => {
                let target = target.ok_or_else(stale_ddl)?;
                let expected_columns = inspected_columns.ok_or_else(stale_ddl)?;
                require_plain_base_table(target)?;
                self.require_browse_relation(context, target).await?;
                let [columns_read, describe_read, resolved_read] =
                    metadata::inspection_reads(target)?;
                let columns = self.read_spec(&columns_read).await?;
                let describe = self.read_spec(&describe_read).await?;
                let resolved = self.read_spec(&resolved_read).await?;
                let live_columns =
                    browsing::parse_inspection(target, &columns, &describe, &resolved)?;
                if live_columns.as_slice() != expected_columns
                    || live_columns.len() <= 1
                    || !live_columns.iter().any(|c| c.name == *column_name)
                {
                    return Err(stale_ddl());
                }
                self.require_browse_relation(context, target).await
            }
            SnowflakeDdlAction::DropTable { .. } => {
                let target = target.ok_or_else(stale_ddl)?;
                require_plain_base_table(target)?;
                self.require_browse_relation(context, target).await
            }
            SnowflakeDdlAction::DropView { .. } => {
                let target = target.ok_or_else(stale_ddl)?;
                require_plain_view(target)?;
                self.require_browse_relation(context, target).await
            }
        }
    }

    fn validate_ddl_locked(
        &self,
        tx: &TransactionAuthorityState,
        state: &DdlAuthorityState,
        request: &SnowflakeDdlRequest,
    ) -> AppResult<()> {
        self.require_ddl_policy()?;
        if tx
            .recovery
            .as_ref()
            .is_some_and(|recovery| recovery.restricted)
        {
            return Err(stale_ddl());
        }
        let grant = state
            .grant
            .as_ref()
            .filter(|grant| {
                grant.revision == request.revision && grant.issued_at.elapsed() < DDL_GRANT_TTL
            })
            .ok_or_else(stale_ddl)?;
        if grant.transaction_generation != tx.generation
            || tx
                .grant
                .as_ref()
                .is_none_or(|tx_grant| tx_grant.sample != grant.sample)
            || grant.action != request.action
            || grant.canonical.sql() != request.sql
        {
            return Err(stale_ddl());
        }
        let (canonical, browse_target, inspected_columns) =
            self.compile_ddl_action(&request.action)?;
        if canonical != grant.canonical
            || browse_target.as_ref().map(|(_, target)| target) != grant.target.as_ref()
            || browse_target
                .as_ref()
                .is_some_and(|(browse_ctx, _)| *browse_ctx != grant.sample.context)
            || inspected_columns != grant.inspected_columns
        {
            return Err(stale_ddl());
        }
        Ok(())
    }

    fn publish_transaction_sample(
        &self,
        sample: &ObservationSample,
        generation: &str,
    ) -> AppResult<SessionRuntimeState> {
        #[cfg(all(unix, feature = "test-harness"))]
        self.check_metadata_observer()?;
        let mut runtime = self.observed_session_state(sample)?;
        let state = self.transaction_authority.lock();
        if self.is_closed() || state.generation != generation {
            return Err(stale_transaction());
        }
        // A shared failure can arrive while this publication waits for state.
        #[cfg(all(unix, feature = "test-harness"))]
        self.check_metadata_observer()?;
        let revision = new_id();
        runtime.transaction_control_authority = Some(TransactionControlAuthority::Observed {
            revision: revision.clone(),
            allowed_actions: self.transaction_actions(sample),
        });
        #[cfg(all(unix, feature = "test-harness"))]
        let state = if let Some(observer) = &self.metadata_observer {
            // Observers cannot reenter the adapter. Still release authority
            // state around callbacks, and revalidate after reacquiring it.
            drop(state);
            observer.observe_runtime(&runtime)?;
            let state = self.transaction_authority.lock();
            if self.is_closed() || state.generation != generation {
                return Err(stale_transaction());
            }
            self.check_metadata_observer()?;
            state
        } else {
            state
        };
        let mut state = state;
        state.grant = Some(TransactionGrant {
            revision: revision.clone(),
            sample: sample.clone(),
        });
        state.recovery = Some(TransactionRecovery {
            revision,
            restricted: false,
        });
        Ok(runtime)
    }

    fn observer_failed(&self, generation: &str, error: &AppError) {
        if error.code == ErrorCode::Database {
            self.restrict_transaction_recovery(generation);
        }
    }

    /// Revocation precedes lane acquisition. A failed or abandoned refresh can
    /// never leave a previous options revision authoritative.
    fn invalidate_session_options(&self) -> String {
        let revision = uuid::Uuid::new_v4().to_string();
        *self.session_options.lock() = SessionOptionsState {
            revision: revision.clone(),
            snapshot: None,
        };
        *self.context_catalog.lock() = ContextCatalogState {
            revision: revision.clone(),
            snapshot: None,
        };
        self.revoke_ddl_grant();
        revision
    }

    fn invalidate_session_options_revision(&self, revision: &str) {
        let mut options = self.session_options.lock();
        if options.revision == revision {
            *options = SessionOptionsState::default();
        }
    }

    fn invalidate_context_catalog_revision(&self, revision: &str) {
        let mut catalog = self.context_catalog.lock();
        if catalog.revision == revision {
            *catalog = ContextCatalogState::default();
        }
    }

    fn require_session_options_revision(&self, revision: &str) -> AppResult<()> {
        if self.session_options.lock().revision != revision {
            return Err(stale_session_options());
        }
        Ok(())
    }

    fn require_context_catalog_revision(&self, revision: &str) -> AppResult<()> {
        if self.context_catalog.lock().revision != revision {
            return Err(stale_session_options());
        }
        Ok(())
    }

    /// Called with the SDK lane held. Five fixed, bounded reads form one sample;
    /// they do not change parameters, start a transaction or repair an observation.
    async fn session_sample(&self) -> AppResult<ObservationSample> {
        let active_database = self.require_active_database()?;
        let [
            context_read,
            policy_read,
            autocommit_read,
            abort_read,
            transaction_read,
        ] = session::sample_reads();
        let context = self.read_spec(&context_read).await?;
        let policy = self.read_spec(&policy_read).await?;
        let observed = metadata::parse_context(&context, &policy)?;
        self.require_session_identity(&observed)?;
        if observed.database.as_deref() != Some(active_database.as_str()) {
            return Err(stale_session_options());
        }
        let autocommit = self.read_spec(&autocommit_read).await?;
        let abort_on_error = self.read_spec(&abort_read).await?;
        let transaction = self.read_spec(&transaction_read).await?;
        let sample = session::parse_sample(
            &context,
            &policy,
            &autocommit,
            &abort_on_error,
            &transaction,
        )?;
        Ok(sample)
    }

    async fn snowflake_context_sample(&self) -> AppResult<ObservationSample> {
        let expected_database = self.active_database.lock().clone();
        let [
            context_read,
            policy_read,
            autocommit_read,
            abort_read,
            transaction_read,
        ] = session::sample_reads();
        let context = self.read_spec(&context_read).await?;
        let policy = self.read_spec(&policy_read).await?;
        let observed = metadata::parse_context(&context, &policy)?;
        self.require_session_identity(&observed)?;
        if observed.database != expected_database {
            return Err(stale_session_options());
        }
        let autocommit = self.read_spec(&autocommit_read).await?;
        let abort_on_error = self.read_spec(&abort_read).await?;
        let transaction = self.read_spec(&transaction_read).await?;
        session::parse_sample(
            &context,
            &policy,
            &autocommit,
            &abort_on_error,
            &transaction,
        )
    }

    async fn read_context_lists(
        &self,
        context: &LiveContext,
    ) -> AppResult<(Vec<String>, Vec<String>, Vec<String>, Vec<String>)> {
        let roles_rows = self
            .metadata_read(
                metadata::SHOW_ROLES_SQL,
                metadata::SHOW_CONTEXT_LIMIT + 1,
                metadata::SHOW_CONTEXT_MAX_BYTES,
            )
            .await?;
        let mut roles = metadata::parse_show_names(&roles_rows)?;
        if let Some(active_role) = &context.role
            && !roles.contains(active_role)
        {
            roles.push(active_role.clone());
            roles.sort();
        }
        let warehouses_rows = self
            .metadata_read(
                metadata::SHOW_WAREHOUSES_SQL,
                metadata::SHOW_CONTEXT_LIMIT + 1,
                metadata::SHOW_CONTEXT_MAX_BYTES,
            )
            .await?;
        let mut warehouses = metadata::parse_show_names(&warehouses_rows)?;
        if let Some(active_wh) = &context.warehouse
            && !warehouses.contains(active_wh)
        {
            warehouses.push(active_wh.clone());
            warehouses.sort();
        }
        let databases_rows = self
            .metadata_read(
                metadata::SHOW_DATABASES_SQL,
                metadata::SHOW_CONTEXT_LIMIT + 1,
                metadata::SHOW_CONTEXT_MAX_BYTES,
            )
            .await?;
        let mut databases = metadata::parse_show_names(&databases_rows)?;
        if let Some(active_db) = &context.database
            && !databases.contains(active_db)
        {
            databases.push(active_db.clone());
            databases.sort();
        }
        let mut schemas = if context.database.is_some() {
            let rows = self.read_spec(&metadata::schemas_read(context)?).await?;
            metadata::parse_schemas(context, &rows)?
        } else {
            Vec::new()
        };
        if let Some(active_schema) = &context.schema
            && active_schema != "INFORMATION_SCHEMA"
            && !schemas.contains(active_schema)
        {
            schemas.push(active_schema.clone());
            schemas.sort();
        }
        Ok((roles, warehouses, databases, schemas))
    }

    fn observed_session_state(&self, sample: &ObservationSample) -> AppResult<SessionRuntimeState> {
        Ok(SessionRuntimeState {
            session_id: self.session_id.clone(),
            transaction_mode: if sample.autocommit {
                TransactionMode::AutoCommit
            } else {
                TransactionMode::Manual
            },
            transaction_state: if sample.transaction_id.is_some() {
                TransactionState::InTransaction
            } else {
                TransactionState::Idle
            },
            context_kind: SessionContextKind::Schema,
            active_context: sample.context.schema.clone(),
            database: sample
                .context
                .database
                .clone()
                .ok_or_else(stale_session_options)?,
            role: sample.context.role.clone(),
            warehouse: sample.context.warehouse.clone(),
            search_path: sample
                .context
                .search_path
                .iter()
                .map(|path| path.raw.clone())
                .collect(),
            context_fingerprint: Some(sample.context.fingerprint()?),
            transaction_control_authority: None,
            transaction_observation: Some(TransactionObservation::Observed {
                transaction_id: sample.transaction_id.clone(),
                abort_on_error: sample.abort_on_error,
                usability: TransactionUsability::Unknown,
            }),
        })
    }

    fn observed_context_catalog_state(
        &self,
        sample: &ObservationSample,
    ) -> AppResult<SessionRuntimeState> {
        Ok(SessionRuntimeState {
            session_id: self.session_id.clone(),
            transaction_mode: if sample.autocommit {
                TransactionMode::AutoCommit
            } else {
                TransactionMode::Manual
            },
            transaction_state: if sample.transaction_id.is_some() {
                TransactionState::InTransaction
            } else {
                TransactionState::Idle
            },
            context_kind: SessionContextKind::Schema,
            active_context: sample.context.schema.clone(),
            database: sample.context.database.clone().unwrap_or_default(),
            role: sample.context.role.clone(),
            warehouse: sample.context.warehouse.clone(),
            search_path: sample
                .context
                .search_path
                .iter()
                .map(|path| path.raw.clone())
                .collect(),
            context_fingerprint: Some(sample.context.fingerprint()?),
            transaction_control_authority: None,
            transaction_observation: Some(TransactionObservation::Observed {
                transaction_id: sample.transaction_id.clone(),
                abort_on_error: sample.abort_on_error,
                usability: TransactionUsability::Unknown,
            }),
        })
    }

    fn unavailable_session_state(&self) -> SessionRuntimeState {
        SessionRuntimeState {
            session_id: self.session_id.clone(),
            transaction_mode: TransactionMode::Unknown,
            transaction_state: TransactionState::Unknown,
            context_kind: SessionContextKind::Schema,
            active_context: None,
            database: String::new(),
            role: None,
            warehouse: None,
            search_path: vec![],
            context_fingerprint: None,
            transaction_observation: Some(TransactionObservation::Unavailable),
            transaction_control_authority: self.transaction_authority().filter(|authority| {
                matches!(authority, TransactionControlAuthority::RollbackOnly { .. })
            }),
        }
    }

    async fn metadata_operation<T: Send>(
        &self,
        work: impl std::future::Future<Output = AppResult<T>> + Send,
    ) -> AppResult<T> {
        self.require_unrestricted()?;
        self.revoke_transaction_grant();
        self.metadata_operation_inner(async {
            self.require_unrestricted()?;
            let generation = self.revoke_transaction_grant();
            let result = work.await;
            // This wrapper serves reads and schema switching. Typed DML/DDL/TX
            // use the inner wrapper and retain their independent ACK mappings.
            #[cfg(all(unix, feature = "test-harness"))]
            let result = result.and_then(|value| {
                self.check_metadata_observer()
                    .map_err(AppError::with_completion_invalidated)?;
                Ok(value)
            });
            result.inspect_err(|error| self.observer_failed(&generation, error))
        })
        .await
    }

    async fn metadata_operation_inner<T: Send>(
        &self,
        work: impl std::future::Future<Output = AppResult<T>> + Send,
    ) -> AppResult<T> {
        let _lane = tokio::time::timeout(Duration::from_millis(1500), self.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The Snowflake session is busy"))?;
        self.require_ordinary_mode()?;
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
            self.revoke_edit_snapshots();
        }
        drop(state);
        self.revoke_ddl_grant();
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
        let active_database = self.active_database.lock().clone();
        if active_database.is_none()
            || context.database != active_database
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
            .read_spec(&metadata::exact_relation_read(context, target)?)
            .await?;
        browsing::require_relation(context, target, &result)
            .map_err(AppError::with_completion_invalidated)?;
        #[cfg(all(unix, feature = "test-harness"))]
        // require_relation established full equality with this parsed target,
        // including creation/DDL markers; do not drop them into a public DTO.
        self.observe_relations(
            MetadataSite::ExactRelationCheck,
            context,
            Some(&target.table.schema),
            std::slice::from_ref(target),
        )?;
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
        self.require_unrestricted()?;
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
        self.revoke_transaction_grant();
        self.invalidate_session_options();
        let _lane = self.lane.lock().await;
        self.require_unrestricted()?;
        let generation = self.revoke_transaction_grant();
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
                result.inspect_err(|error| self.observer_failed(&generation, error))
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
        self.stream_bounded(
            wire,
            binds,
            sink,
            metadata::STANDARD_QUERY_MAX_ROWS,
            metadata::STANDARD_QUERY_MAX_BYTES,
        )
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
        self.require_ordinary_mode()?;
        #[cfg(all(unix, feature = "test-harness"))]
        self.check_metadata_observer()?;
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
        let mut boolean_columns = Vec::new();
        loop {
            let available_columns = operation.columns.borrow().clone();
            if !described && let Some(columns) = available_columns {
                let infos: Vec<ColumnInfo> =
                    columns.into_iter().enumerate().map(column_info).collect();
                boolean_columns = infos
                    .iter()
                    .map(|c| c.kind == ColumnKind::Boolean)
                    .collect();
                if let Err(error) = sink.describe(infos) {
                    sink_error = Some(error);
                    retaining = false;
                    self.discard(&token, &mut operation).await?;
                }
                described = true;
            }
            tokio::select! {
                biased;
                changed = operation.columns.changed(), if !described && columns_open => { columns_open = changed.is_ok(); }
                batch = operation.rows.recv() => {
                    let Some(mut batch) = batch else { break; };
                    if !described { self.start_shutdown(); return Err(protocol_error()); }
                    let mut rows = std::mem::take(&mut batch.rows);
                    drop(batch); // Return the supervisor's bounded queue credit before granting more.
                    if retaining {
                        normalize_wire_booleans(&boolean_columns, &mut rows);
                        match sink.rows(rows) {
                            Ok(true) => {},
                            Ok(false) => { retaining = false; sink.limit("Result retention limit reached"); },
                            Err(error) => { retaining = false; sink_error = Some(error); }
                        }
                        if !retaining { self.discard(&token, &mut operation).await?; }
                    }
                    if retaining && operation.report().is_none() {
                        match self.process.credit(&token, 1) {
                            Ok(mut credit) => {
                                let report = credit.wait_bounded(Duration::from_secs(3)).await;
                                if operation.report().is_none() { check_report(&report)?; }
                            },
                            Err(_) if operation.report().is_some() => {},
                            Err(super::snowflake_process::INACTIVE_OPERATION)
                                if operation.wait_if_finalizing().await.is_some() => {},
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
        #[cfg(all(unix, feature = "test-harness"))]
        self.check_metadata_observer()?;
        if result.get("truncated").and_then(Value::as_bool) == Some(true) {
            sink.limit("Snowflake helper result retention limit reached");
        }
        sink.complete(count)?;
        Ok(count)
    }
    async fn discard(
        &self,
        token: &OperationToken,
        operation: &mut super::snowflake_process::Operation,
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
            Err(super::snowflake_process::INACTIVE_OPERATION)
                if operation.wait_if_finalizing().await.is_some() =>
            {
                Ok(())
            }
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
    fn validate_snowflake_dml(&self, request: &SnowflakeDmlRequest) -> AppResult<()> {
        self.validate_dml_locked(&self.transaction_authority.lock(), request)
            .map(|_| ())
    }

    async fn snowflake_dml(
        &self,
        session_id: &str,
        request: &SnowflakeDmlRequest,
    ) -> AppResult<SnowflakeDmlResult> {
        if session_id != self.session_id {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        // Recompile the original SQL and scalar parameters at the private
        // adapter boundary. Public callers cannot supply a trusted wire query.
        // This is reviewed SQL-name execution, not an atomic object binding:
        // concurrent DDL, temporary shadowing and indirect effects remain gates.
        let (canonical, baseline, generation) = {
            let mut state = self.transaction_authority.lock();
            let canonical = self.validate_dml_locked(&state, request)?;
            let baseline = state.grant.take().ok_or_else(stale_transaction)?.sample;
            let generation = new_id();
            state.generation = generation.clone();
            if let Some(recovery) = &mut state.recovery {
                recovery.revision = new_id();
            }
            (canonical, baseline, generation)
        };
        let kind = canonical.kind();
        let wire = canonical.bound_statement();
        let binds = binds_json(&wire.binds)?;
        self.invalidate_session_options();
        self.invalidate_relation_catalog();
        let mut lease = TransactionLease {
            driver: self,
            operation: None,
            settled: false,
        };
        let mut summary = SnowflakeDmlSummary::unavailable();
        let result = self.metadata_operation_inner(async {
            self.require_transaction_generation(&generation)?;
            let before = self.session_sample().await?;
            if before != baseline {
                self.restrict_transaction_recovery(&generation);
                return Err(stale_transaction());
            }
            {
                let state = self.transaction_authority.lock();
                if state.generation != generation || self.is_closed() {
                    return Err(stale_transaction());
                }
                #[cfg(all(unix, feature = "test-harness"))]
                self.check_metadata_observer()?;
                lease.operation = Some(self.process.request(
                    "reviewed-dml", "dml",
                    json!({"kind": kind, "sql": wire.rewritten_sql, "binds": binds,
                        "expected": {"user": self.identity["user"], "session": self.identity["session"]}}),
                    Impact::Mutating,
                ).map_err(control_error)?);
            }
            let report = {
                let operation = lease.operation.as_mut().ok_or_else(protocol_error)?;
                *self.active.lock() = Some(Active {
                    token: operation.token(),
                    terminal: operation.terminal_receiver(),
                });
                let _active = ActiveLease(&self.active);
                operation.wait().await
            };
            // Retain available count evidence before the final observer can
            // fail. rowsSeen describes status rows and is never affected rows.
            if report.dml_acknowledged.as_ref().is_some_and(|ack| ack.kind == kind) {
                summary = dml_summary(kind, report.result.as_ref());
            }
            check_report(&report)?;
            if report.dml_acknowledged.as_ref().is_none_or(|ack| ack.kind != kind) {
                return Err(protocol_error());
            }
            self.require_transaction_generation(&generation)?;
            let after = self.session_sample().await?;
            let mut comparable = after.clone();
            comparable.transaction_id = baseline.transaction_id.clone();
            if comparable != baseline
                || baseline.transaction_id.as_ref().is_some_and(|id| after.transaction_id.as_ref() != Some(id))
                || (baseline.autocommit && baseline.transaction_id.is_none() && after.transaction_id.is_some())
            {
                return Err(stale_transaction());
            }
            self.require_transaction_generation(&generation)?;
            self.publish_transaction_sample(&after, &generation)
        }).await;
        match result {
            Ok(state) => {
                lease.settled = true;
                Ok(SnowflakeDmlResult {
                    kind,
                    statement_outcome: SnowflakeDmlOutcome::Succeeded,
                    summary,
                    state,
                    retired: false,
                    error: None,
                })
            }
            Err(error) => {
                self.observer_failed(&generation, &error);
                let Some(operation) = &lease.operation else {
                    return Err(error.with_completion_invalidated());
                };
                let acknowledged = operation
                    .dml_acknowledged()
                    .is_some_and(|ack| ack.kind == kind);
                let report = operation.report();
                let rejected = !acknowledged
                    && report.as_ref().is_some_and(|report| {
                        !report.ok
                            && report.retirement.is_none()
                            && report.error_code.as_deref() == Some("database")
                            && report.certainty == Certainty::ServerTerminal
                    });
                let recoverable = ((acknowledged
                    && report.as_ref().is_some_and(|report| report.ok)
                    && error.code == ErrorCode::Database)
                    || rejected)
                    && self.restrict_transaction_recovery(&generation);
                if recoverable {
                    lease.settled = true;
                } else {
                    self.start_shutdown();
                }
                let error = if acknowledged && !recoverable {
                    AppError::new(
                        ErrorCode::ConnectionClosed,
                        "Snowflake acknowledged the DML statement, but the final session state could not be verified. The session was closed; reconnect before continuing.",
                    )
                } else if !acknowledged && !rejected {
                    AppError::new(
                        ErrorCode::OutcomeUnknown,
                        "Snowflake DML outcome is unknown; inspect query history before retrying",
                    )
                } else {
                    error
                };
                Ok(SnowflakeDmlResult {
                    kind,
                    statement_outcome: if acknowledged {
                        SnowflakeDmlOutcome::Succeeded
                    } else if rejected {
                        SnowflakeDmlOutcome::Rejected
                    } else {
                        SnowflakeDmlOutcome::Unknown
                    },
                    summary,
                    state: self.unavailable_session_state(),
                    retired: !recoverable,
                    error: Some(error.with_completion_invalidated()),
                })
            }
        }
    }

    async fn preflight_snowflake_ddl(
        &self,
        session_id: &str,
        request: &SnowflakeDdlPreflightRequest,
    ) -> AppResult<SnowflakeDdlPreflightResponse> {
        if session_id != self.session_id || request.session_id != self.session_id {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        self.require_ddl_policy()?;
        self.require_unrestricted()?;
        let (canonical, browse_target, inspected_columns) =
            self.compile_ddl_action(&request.action)?;
        let tx_generation = self.revoke_transaction_grant();
        let ddl_generation = self.revoke_ddl_grant();
        self.metadata_operation_inner(async {
            self.require_ddl_policy()?;
            self.require_unrestricted()?;
            self.require_transaction_generation(&tx_generation)?;
            self.require_ddl_generation(&ddl_generation)?;
            let (current_canonical, current_target, current_columns) =
                self.compile_ddl_action(&request.action)?;
            if current_canonical != canonical
                || current_target != browse_target
                || current_columns != inspected_columns
            {
                return Err(stale_ddl());
            }
            let before = self.session_sample().await?;
            self.require_ddl_session_sample(&before)?;
            if let Some((ref browse_ctx, ref target)) = browse_target {
                if *browse_ctx != before.context {
                    return Err(stale_ddl());
                }
                self.verify_ddl_catalog_state(
                    &before.context,
                    &request.action,
                    Some(target),
                    inspected_columns.as_deref(),
                )
                .await?;
            } else {
                self.verify_ddl_catalog_state(&before.context, &request.action, None, None)
                    .await?;
            }
            let after = self.session_sample().await?;
            if session::compare_samples(Some(&before), Some(&after)) != SampleConsistency::Stable {
                return Err(stale_ddl());
            }
            self.require_ddl_session_sample(&after)?;
            let (final_canonical, final_target, final_columns) =
                self.compile_ddl_action(&request.action)?;
            if final_canonical != canonical
                || final_target != browse_target
                || final_columns != inspected_columns
            {
                return Err(stale_ddl());
            }
            self.require_transaction_generation(&tx_generation)?;
            self.require_ddl_generation(&ddl_generation)?;
            let _ = self.publish_transaction_sample(&after, &tx_generation)?;
            let components = canonical.qualified_target().clone();
            let qualified_target = qualified_ddl_target(&components)?;
            let kind = canonical.kind();
            let warnings = ddl_warnings(
                kind,
                &qualified_target,
                browse_target.as_ref().map(|(_, target)| target),
            );
            let sql = canonical.sql().to_owned();
            let revision = new_id();
            {
                let mut ddl = self.ddl_authority.lock();
                if self.is_closed() || ddl.generation != ddl_generation {
                    return Err(stale_ddl());
                }
                #[cfg(all(unix, feature = "test-harness"))]
                self.check_metadata_observer()?;
                ddl.grant = Some(DdlGrant {
                    revision: revision.clone(),
                    issued_at: Instant::now(),
                    transaction_generation: tx_generation.clone(),
                    sample: after,
                    action: request.action.clone(),
                    canonical,
                    target: browse_target.map(|(_, target)| target),
                    inspected_columns,
                });
            }
            Ok(SnowflakeDdlPreflightResponse {
                revision,
                kind,
                database: components[0].clone(),
                schema: components[1].clone(),
                name: components[2].clone(),
                qualified_target,
                sql,
                warnings,
            })
        })
        .await
        .inspect_err(|error| {
            self.observer_failed(&tx_generation, error);
            self.revoke_ddl_grant();
        })
    }

    fn validate_snowflake_ddl(&self, request: &SnowflakeDdlRequest) -> AppResult<()> {
        // Match dispatch's transaction -> DDL lock order. Validation must never
        // acquire transaction authority while already holding DDL authority.
        let tx = self.transaction_authority.lock();
        self.validate_ddl_locked(&tx, &self.ddl_authority.lock(), request)
    }

    async fn snowflake_ddl(
        &self,
        session_id: &str,
        request: &SnowflakeDdlRequest,
    ) -> AppResult<SnowflakeDdlResult> {
        if session_id != self.session_id {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        let grant = {
            let tx = self.transaction_authority.lock();
            let mut ddl = self.ddl_authority.lock();
            self.validate_ddl_locked(&tx, &ddl, request)?;
            ddl.grant.take().ok_or_else(stale_ddl)?
        };
        let kind = grant.canonical.kind();
        let qualified_target = qualified_ddl_target(grant.canonical.qualified_target())?;
        self.invalidate_session_options();
        self.invalidate_relation_catalog();
        let tx_generation = self.revoke_transaction_grant();
        let ddl_generation = self.revoke_ddl_grant();
        let mut lease = TransactionLease {
            driver: self,
            operation: None,
            settled: false,
        };
        let mut summary_status = SnowflakeDdlSummaryStatus::Unavailable;
        let mut rejected_error: Option<AppError> = None;
        let result = self
            .metadata_operation_inner(async {
                self.require_ddl_policy()?;
                self.require_unrestricted()?;
                self.require_transaction_generation(&tx_generation)?;
                self.require_ddl_generation(&ddl_generation)?;
                let before = self.session_sample().await?;
                if before != grant.sample {
                    return Err(stale_ddl());
                }
                self.require_ddl_session_sample(&before)?;
                self.verify_ddl_catalog_state(
                    &before.context,
                    &grant.action,
                    grant.target.as_ref(),
                    grant.inspected_columns.as_deref(),
                )
                .await?;
                // Catalog reads are observations too. A fresh final idle sample
                // must still match the reviewed principal/session and options
                // before any statement can be queued for SDK dispatch.
                let final_sample = self.session_sample().await?;
                if final_sample != grant.sample {
                    return Err(stale_ddl());
                }
                self.require_ddl_session_sample(&final_sample)?;
                {
                    let tx = self.transaction_authority.lock();
                    let ddl = self.ddl_authority.lock();
                    if tx.generation != tx_generation
                        || ddl.generation != ddl_generation
                        || self.is_closed()
                        || grant.issued_at.elapsed() >= DDL_GRANT_TTL
                    {
                        return Err(stale_ddl());
                    }
                    #[cfg(all(unix, feature = "test-harness"))]
                    self.check_metadata_observer()?;
                    lease.operation = Some(
                        self.process
                            .request(
                                "reviewed-ddl",
                                "ddl",
                                json!({
                                    "kind": kind.as_str(),
                                    "sql": grant.canonical.sql(),
                                    "expected": {
                                        "user": self.identity["user"],
                                        "session": self.identity["session"]
                                    }
                                }),
                                Impact::Mutating,
                            )
                            .map_err(control_error)?,
                    );
                }
                let report = {
                    let operation = lease.operation.as_mut().ok_or_else(protocol_error)?;
                    *self.active.lock() = Some(Active {
                        token: operation.token(),
                        terminal: operation.terminal_receiver(),
                    });
                    let _active = ActiveLease(&self.active);
                    operation.wait().await
                };
                if report
                    .ddl_acknowledged
                    .as_ref()
                    .is_some_and(|ack| ack.kind == kind)
                {
                    summary_status = ddl_summary_status(kind, report.result.as_ref());
                }
                let acknowledged = report
                    .ddl_acknowledged
                    .as_ref()
                    .is_some_and(|ack| ack.kind == kind);
                let rejected = !acknowledged
                    && !report.ok
                    && report.retirement.is_none()
                    && report.error_code.as_deref() == Some("database")
                    && report.certainty == Certainty::ServerTerminal;
                if rejected {
                    let db_error = check_report(&report).err().unwrap_or_else(|| {
                        AppError::new(ErrorCode::Database, "Snowflake rejected the DDL statement")
                    });
                    rejected_error = Some(db_error.clone());
                    let post_reject = async {
                        self.require_transaction_generation(&tx_generation)?;
                        self.require_ddl_generation(&ddl_generation)?;
                        let after = self.session_sample().await?;
                        if after != grant.sample {
                            return Err(stale_ddl());
                        }
                        self.require_ddl_session_sample(&after)?;
                        self.require_transaction_generation(&tx_generation)?;
                        self.require_ddl_generation(&ddl_generation)?;
                        self.publish_transaction_sample(&after, &tx_generation)
                    }
                    .await;
                    return match post_reject {
                        Ok(state) => Ok((
                            SnowflakeDdlOutcome::Rejected,
                            state,
                            false,
                            Some(db_error.with_completion_invalidated()),
                        )),
                        Err(_) => {
                            self.start_shutdown();
                            Ok((
                                SnowflakeDdlOutcome::Rejected,
                                self.unavailable_session_state(),
                                true,
                                Some(db_error.with_completion_invalidated()),
                            ))
                        }
                    };
                }
                check_report(&report)?;
                if !acknowledged || summary_status != SnowflakeDdlSummaryStatus::Acknowledged {
                    return Err(protocol_error());
                }
                self.require_transaction_generation(&tx_generation)?;
                self.require_ddl_generation(&ddl_generation)?;
                let after = self.session_sample().await?;
                if after != grant.sample {
                    return Err(stale_ddl());
                }
                self.require_ddl_session_sample(&after)?;
                self.require_transaction_generation(&tx_generation)?;
                self.require_ddl_generation(&ddl_generation)?;
                let state = self.publish_transaction_sample(&after, &tx_generation)?;
                Ok((SnowflakeDdlOutcome::Succeeded, state, false, None))
            })
            .await;
        match result {
            Ok((statement_outcome, state, retired, error)) => {
                if !retired {
                    lease.settled = true;
                }
                Ok(SnowflakeDdlResult {
                    kind,
                    qualified_target,
                    statement_outcome,
                    summary_status,
                    catalog_reconciliation_required: true,
                    state,
                    retired,
                    error,
                })
            }
            Err(error) => {
                let Some(operation) = &lease.operation else {
                    self.observer_failed(&tx_generation, &error);
                    return Err(error.with_completion_invalidated());
                };
                // Snowflake DDL is not transactional and cannot be rolled back;
                // never offer rollback-only recovery after dispatching DDL.
                self.start_shutdown();
                let acknowledged = operation
                    .ddl_acknowledged()
                    .is_some_and(|ack| ack.kind == kind);
                if let Some(rejected) = rejected_error {
                    return Ok(SnowflakeDdlResult {
                        kind,
                        qualified_target,
                        statement_outcome: SnowflakeDdlOutcome::Rejected,
                        summary_status: SnowflakeDdlSummaryStatus::Unavailable,
                        catalog_reconciliation_required: true,
                        state: self.unavailable_session_state(),
                        retired: true,
                        error: Some(rejected.with_completion_invalidated()),
                    });
                }
                let error = if acknowledged {
                    AppError::new(
                        ErrorCode::ConnectionClosed,
                        "Snowflake acknowledged the DDL statement, but the final session state could not be verified. The session was closed; reconnect before continuing.",
                    )
                } else {
                    AppError::new(
                        ErrorCode::OutcomeUnknown,
                        "Snowflake DDL outcome is unknown; inspect query history and refresh the catalog before retrying",
                    )
                };
                Ok(SnowflakeDdlResult {
                    kind,
                    qualified_target,
                    statement_outcome: if acknowledged {
                        SnowflakeDdlOutcome::Succeeded
                    } else {
                        SnowflakeDdlOutcome::Unknown
                    },
                    summary_status,
                    catalog_reconciliation_required: true,
                    state: self.unavailable_session_state(),
                    retired: true,
                    error: Some(error.with_completion_invalidated()),
                })
            }
        }
    }

    fn validate_transaction_control(&self, request: &TransactionControlRequest) -> AppResult<()> {
        self.validate_transaction_locked(&self.transaction_authority.lock(), request)
    }

    async fn transaction_control(
        &self,
        session_id: &str,
        request: &TransactionControlRequest,
    ) -> AppResult<TransactionControlResult> {
        if session_id != self.session_id {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        // Consume once synchronously; neither a queued request nor a dropped
        // future can replay the public revision against another operation.
        let (baseline, generation) = {
            let mut state = self.transaction_authority.lock();
            self.validate_transaction_locked(&state, request)?;
            let baseline = state.grant.take().map(|grant| grant.sample);
            let generation = new_id();
            state.generation = generation.clone();
            if let Some(recovery) = &mut state.recovery {
                recovery.revision = new_id();
            }
            (baseline, generation)
        };
        self.invalidate_session_options();
        self.invalidate_relation_catalog();
        let action = match request.action {
            ExplicitTransactionAction::Begin => TransactionAction::Begin,
            ExplicitTransactionAction::Commit => TransactionAction::Commit,
            ExplicitTransactionAction::Rollback => TransactionAction::Rollback,
        };
        let mut lease = TransactionLease {
            driver: self,
            operation: None,
            settled: false,
        };
        let result = self.metadata_operation_inner(async {
            self.require_transaction_generation(&generation)?;
            if action != TransactionAction::Rollback {
                let expected = baseline.as_ref().ok_or_else(stale_transaction)?;
                let before = self.session_sample().await?;
                if &before != expected || (action == TransactionAction::Begin) != before.transaction_id.is_none() {
                    self.restrict_transaction_recovery(&generation);
                    return Err(stale_transaction());
                }
            }
            {
                let state = self.transaction_authority.lock();
                if state.generation != generation || self.is_closed() { return Err(stale_transaction()); }
                #[cfg(all(unix, feature = "test-harness"))]
                self.check_metadata_observer()?;
                lease.operation = Some(self.process.request("transaction-control", "transaction",
                    json!({"action": action.as_str(), "expected": {"user": self.identity["user"], "session": self.identity["session"]}}),
                    action.impact()).map_err(control_error)?);
            }
            let report = {
                let operation = lease.operation.as_mut().ok_or_else(protocol_error)?;
                *self.active.lock() = Some(Active { token: operation.token(), terminal: operation.terminal_receiver() });
                let _active = ActiveLease(&self.active);
                operation.wait().await
            };
            check_report(&report)?;
            if report.boundary_acknowledged.as_ref().is_none_or(|ack| ack.action != action) { return Err(protocol_error()); }
            self.require_transaction_generation(&generation)?;
            let after = self.session_sample().await?;
            if let Some(before) = &baseline {
                let mut comparable = after.clone();
                comparable.transaction_id = before.transaction_id.clone();
                if comparable != *before { return Err(stale_transaction()); }
            }
            if (action == TransactionAction::Begin) != after.transaction_id.is_some() { return Err(stale_transaction()); }
            self.require_transaction_generation(&generation)?;
            self.publish_transaction_sample(&after, &generation)
        }).await;
        match result {
            Ok(state) => {
                lease.settled = true;
                Ok(TransactionControlResult {
                    action: request.action,
                    boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
                    state,
                    retired: false,
                    error: None,
                })
            }
            Err(error) => {
                self.observer_failed(&generation, &error);
                let Some(operation) = &lease.operation else {
                    return Err(error.with_completion_invalidated());
                };
                let acknowledged = operation.boundary_acknowledged().is_some();
                let report = operation.report();
                let rejected = !acknowledged
                    && report.as_ref().is_some_and(|report| {
                        !report.ok
                            && report.retirement.is_none()
                            && report.error_code.as_deref() == Some("database")
                            && report.certainty == Certainty::ServerTerminal
                    });
                // A completed boundary plus ordinary observer denial leaves the
                // exact same helper available solely for reviewed rollback.
                let recoverable = ((acknowledged
                        && report.as_ref().is_some_and(|r| r.ok)
                        && error.code == ErrorCode::Database)
                        || rejected)
                    // This lock is the recovery publication point. An observer
                    // admitted after the earlier check cannot revive old work.
                    && self.restrict_transaction_recovery(&generation);
                if recoverable {
                    lease.settled = true;
                } else {
                    self.start_shutdown();
                }
                let error = if acknowledged && !recoverable {
                    AppError::new(
                        ErrorCode::ConnectionClosed,
                        "Snowflake acknowledged the transaction command, but its final session state could not be verified. The session was closed; reconnect before continuing.",
                    )
                } else if !acknowledged && !rejected {
                    AppError::new(
                        ErrorCode::OutcomeUnknown,
                        "Snowflake transaction command outcome is unknown; inspect query history before retrying",
                    )
                } else {
                    error
                };
                Ok(TransactionControlResult {
                    action: request.action,
                    boundary_outcome: if acknowledged {
                        TransactionBoundaryOutcome::Acknowledged
                    } else if rejected {
                        TransactionBoundaryOutcome::Rejected
                    } else {
                        TransactionBoundaryOutcome::Unknown
                    },
                    state: self.unavailable_session_state(),
                    retired: !recoverable,
                    error: Some(error.with_completion_invalidated()),
                })
            }
        }
    }

    async fn switch_schema(
        &self,
        session_id: &str,
        request: &SwitchSchemaRequest,
    ) -> AppResult<SessionContextOptions> {
        self.require_unrestricted()?;
        if session_id != self.session_id {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        // Claim and consume the public revision before the first await. A stale
        // caller cannot consume newer authority, and ABA never restores it.
        let (snapshot, generation) = {
            let mut options = self.session_options.lock();
            if options.revision != request.revision {
                return Err(stale_session_options().with_completion_invalidated());
            }
            let snapshot = options.snapshot.take().ok_or_else(stale_session_options)?;
            let generation = uuid::Uuid::new_v4().to_string();
            options.revision = generation.clone();
            *self.context_catalog.lock() = ContextCatalogState {
                revision: generation.clone(),
                snapshot: None,
            };
            (snapshot, generation)
        };
        self.invalidate_relation_catalog();
        let mut lease = SchemaSwitchLease {
            driver: self,
            operation: None,
            published: false,
        };
        let result = self
            .metadata_operation(async {
                let active_database = self.require_active_database()?;
                metadata::quote_identifier(&request.database)?;
                metadata::quote_identifier(&request.schema)?;
                if request.context_fingerprint != snapshot.sample.context.fingerprint()?
                    || request.database != active_database
                    || snapshot.sample.context.database.as_deref()
                        != Some(request.database.as_str())
                    || snapshot.sample.transaction_id.is_some()
                    || !snapshot.available.contains(&request.schema)
                {
                    return Err(stale_session_options());
                }
                self.require_session_options_revision(&generation)?;
                let before = self.session_sample().await?;
                if before != snapshot.sample || before.transaction_id.is_some() {
                    return Err(stale_session_options());
                }
                let rows = self
                    .read_spec(&metadata::schemas_read(&before.context)?)
                    .await?;
                let available = metadata::parse_schemas(&before.context, &rows)?;
                #[cfg(all(unix, feature = "test-harness"))]
                self.observe_schemas(
                    MetadataSite::SchemaSwitch,
                    &before.context,
                    &rows,
                    &available,
                )?;
                if !available.contains(&request.schema) {
                    return Err(stale_session_options());
                }
                {
                    // Admission is synchronous while generation ownership is locked.
                    // A superseding refresh cannot slip between this check and USE.
                    let options = self.session_options.lock();
                    if options.revision != generation || self.is_closed() {
                        return Err(stale_session_options());
                    }
                    #[cfg(all(unix, feature = "test-harness"))]
                    self.check_metadata_observer()?;
                    lease.operation = Some(
                        self.process
                            .request(
                                "switch-schema",
                                "switchSchema",
                                json!({"database":request.database,"schema":request.schema,
                        "expected":{"user":before.context.principal,"role":before.context.role,
                        "warehouse":before.context.warehouse,"session":before.context.session_id}}),
                                Impact::ContextChange,
                            )
                            .map_err(control_error)?,
                    );
                }
                let report = {
                    let operation = lease.operation.as_mut().ok_or_else(protocol_error)?;
                    *self.active.lock() = Some(Active {
                        token: operation.token(),
                        terminal: operation.terminal_receiver(),
                    });
                    let _active = ActiveLease(&self.active);
                    operation.wait().await
                };
                check_report(&report)?;
                if report.use_acknowledged.is_none() {
                    return Err(protocol_error());
                }
                let readback = report.result.as_ref().ok_or_else(protocol_error)?;
                for (field, expected) in [
                    ("user", json!(before.context.principal)),
                    ("role", json!(before.context.role)),
                    ("database", json!(request.database)),
                    ("schema", json!(request.schema)),
                    ("warehouse", json!(before.context.warehouse)),
                    ("session", json!(before.context.session_id)),
                ] {
                    if readback.get(field) != Some(&expected) {
                        return Err(protocol_error());
                    }
                }
                self.require_session_options_revision(&generation)?;
                let after = self.session_sample().await?;
                let mut comparable = after.clone();
                // USE legitimately changes the active schema and CURRENT_SCHEMAS.
                // Every other observed field, including transaction/settings, pins.
                comparable.context.schema = before.context.schema.clone();
                comparable.context.search_path = before.context.search_path.clone();
                if after.context.schema.as_deref() != Some(request.schema.as_str())
                    || comparable != before
                    || after.transaction_id.is_some()
                {
                    return Err(stale_session_options());
                }
                let revision = uuid::Uuid::new_v4().to_string();
                let published = SessionContextOptions {
                    state: self.observed_session_state(&after)?,
                    available: available.clone(),
                    revision: Some(revision.clone()),
                };
                let mut options = self.session_options.lock();
                if options.revision != generation || self.is_closed() {
                    return Err(stale_session_options());
                }
                #[cfg(all(unix, feature = "test-harness"))]
                self.check_metadata_observer()?;
                *options = SessionOptionsState {
                    revision,
                    snapshot: Some(SessionOptionsSnapshot {
                        sample: after,
                        available,
                    }),
                };
                Ok(published)
            })
            .await;
        match result {
            Ok(options) => {
                lease.published = true;
                Ok(options)
            }
            Err(error) => {
                self.invalidate_session_options_revision(&generation);
                if let Some(operation) = &lease.operation {
                    let acknowledged = operation.use_acknowledged().is_some();
                    self.start_shutdown();
                    if acknowledged {
                        return Err(AppError::new(ErrorCode::ConnectionClosed,
                            "Snowflake acknowledged the schema change, but its final session context could not be verified. The session was closed; reconnect before continuing.")
                            .with_completion_invalidated());
                    }
                }
                Err(error.with_completion_invalidated())
            }
        }
    }

    async fn session_state(&self, session_id: &str) -> AppResult<SessionRuntimeState> {
        self.require_ordinary_mode()?;
        if session_id != self.session_id {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        let revision = self.invalidate_session_options();
        let generation = self.revoke_transaction_grant();
        let result = self
            .metadata_operation_inner(async {
                self.require_session_options_revision(&revision)?;
                self.require_transaction_generation(&generation)?;
                let before = self.session_sample().await?;
                let after = self.session_sample().await?;
                if session::compare_samples(Some(&before), Some(&after))
                    != SampleConsistency::Stable
                {
                    return Err(stale_session_options());
                }
                self.require_session_options_revision(&revision)?;
                self.publish_transaction_sample(&after, &generation)
            })
            .await;
        match result {
            Ok(state) => Ok(state),
            Err(error) => {
                self.observer_failed(&generation, &error);
                self.invalidate_session_options_revision(&revision);
                self.invalidate_relation_catalog();
                if self.is_closed()
                    || matches!(
                        error.code,
                        ErrorCode::ConnectionClosed | ErrorCode::OutcomeUnknown
                    )
                {
                    Err(error.with_completion_invalidated())
                } else {
                    // This independent observer cannot replace an acknowledged
                    // console statement's success or definitive database error.
                    Ok(self.unavailable_session_state())
                }
            }
        }
    }

    async fn session_context_options(&self, session_id: &str) -> AppResult<SessionContextOptions> {
        self.require_ordinary_mode()?;
        if session_id != self.session_id {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        let revision = self.invalidate_session_options();
        let generation = self.revoke_transaction_grant();
        let result = self
            .metadata_operation_inner(async {
                self.require_session_options_revision(&revision)?;
                self.require_transaction_generation(&generation)?;
                let before = self.session_sample().await?;
                let rows = self
                    .read_spec(&metadata::schemas_read(&before.context)?)
                    .await?;
                let available = metadata::parse_schemas(&before.context, &rows)?;
                #[cfg(all(unix, feature = "test-harness"))]
                self.observe_schemas(
                    MetadataSite::SessionContextOptions,
                    &before.context,
                    &rows,
                    &available,
                )?;
                let after = self.session_sample().await?;
                if session::compare_samples(Some(&before), Some(&after))
                    != SampleConsistency::Stable
                {
                    return Err(stale_session_options());
                }
                let mut options = self.session_options.lock();
                if options.revision != revision {
                    return Err(stale_session_options());
                }
                #[cfg(all(unix, feature = "test-harness"))]
                self.check_metadata_observer()?;
                options.snapshot = Some(SessionOptionsSnapshot {
                    sample: after,
                    available,
                });
                let snapshot = options
                    .snapshot
                    .as_ref()
                    .ok_or_else(stale_session_options)?;
                Ok(SessionContextOptions {
                    state: self.publish_transaction_sample(&snapshot.sample, &generation)?,
                    available: snapshot.available.clone(),
                    revision: Some(revision.clone()),
                })
            })
            .await
            .inspect_err(|_| {
                self.invalidate_session_options_revision(&revision);
                self.invalidate_relation_catalog();
            })
            .map_err(AppError::with_completion_invalidated);
        if let Err(error) = &result {
            self.observer_failed(&generation, error);
            if error.code == ErrorCode::Database
                && !self.is_closed()
                && matches!(
                    self.transaction_authority(),
                    Some(TransactionControlAuthority::RollbackOnly { .. })
                )
            {
                return Ok(SessionContextOptions {
                    state: self.unavailable_session_state(),
                    available: vec![],
                    revision: None,
                });
            }
        }
        result
    }

    async fn snowflake_session_contexts(
        &self,
        session_id: &str,
    ) -> AppResult<SnowflakeContextCatalog> {
        self.require_ordinary_mode()?;
        if session_id != self.session_id {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        let revision = self.invalidate_session_options();
        let generation = self.revoke_transaction_grant();
        let result = self
            .metadata_operation_inner(async {
                self.require_context_catalog_revision(&revision)?;
                self.require_transaction_generation(&generation)?;
                let before = self.snowflake_context_sample().await?;
                let (roles, warehouses, databases, schemas) =
                    self.read_context_lists(&before.context).await?;
                let after = self.snowflake_context_sample().await?;
                if session::compare_samples(Some(&before), Some(&after))
                    != SampleConsistency::Stable
                {
                    return Err(stale_session_options());
                }
                let state = if after.context.database.is_some() {
                    self.publish_transaction_sample(&after, &generation)?
                } else {
                    self.observed_context_catalog_state(&after)?
                };
                let mut catalog = self.context_catalog.lock();
                if catalog.revision != revision || self.is_closed() {
                    return Err(stale_session_options());
                }
                #[cfg(all(unix, feature = "test-harness"))]
                self.check_metadata_observer()?;
                catalog.snapshot = Some(ContextCatalogSnapshot {
                    sample: after.clone(),
                    roles: roles.clone(),
                    warehouses: warehouses.clone(),
                    databases: databases.clone(),
                    schemas: schemas.clone(),
                });
                if after.context.database.is_some() {
                    *self.session_options.lock() = SessionOptionsState {
                        revision: revision.clone(),
                        snapshot: Some(SessionOptionsSnapshot {
                            sample: after,
                            available: schemas.clone(),
                        }),
                    };
                }
                Ok(SnowflakeContextCatalog {
                    state,
                    available_roles: roles,
                    available_warehouses: warehouses,
                    available_databases: databases,
                    available_schemas: schemas,
                    revision: revision.clone(),
                })
            })
            .await
            .inspect_err(|_| {
                self.invalidate_context_catalog_revision(&revision);
                self.invalidate_session_options_revision(&revision);
                self.invalidate_relation_catalog();
            })
            .map_err(AppError::with_completion_invalidated);
        if let Err(error) = &result {
            self.observer_failed(&generation, error);
        }
        result
    }

    async fn switch_snowflake_context(
        &self,
        session_id: &str,
        request: &SwitchSnowflakeContextRequest,
    ) -> AppResult<SnowflakeContextCatalog> {
        self.require_unrestricted()?;
        if session_id != self.session_id {
            return Err(AppError::invalid("Invalid Snowflake session"));
        }
        let (snapshot, generation) = {
            let mut catalog = self.context_catalog.lock();
            if catalog.revision != request.revision {
                return Err(stale_session_options().with_completion_invalidated());
            }
            let snapshot = catalog.snapshot.take().ok_or_else(stale_session_options)?;
            let generation = uuid::Uuid::new_v4().to_string();
            catalog.revision = generation.clone();
            *self.session_options.lock() = SessionOptionsState {
                revision: generation.clone(),
                snapshot: None,
            };
            (snapshot, generation)
        };
        self.revoke_ddl_grant();
        self.invalidate_relation_catalog();
        let mut lease = SchemaSwitchLease {
            driver: self,
            operation: None,
            published: false,
        };
        let result = self
            .metadata_operation(async {
                if request.context_fingerprint != snapshot.sample.context.fingerprint()?
                    || snapshot.sample.transaction_id.is_some()
                {
                    return Err(stale_session_options());
                }
                match &request.target {
                    SnowflakeContextTarget::Role { role } => {
                        metadata::quote_identifier(role)?;
                        if !snapshot.roles.contains(role) {
                            return Err(stale_session_options());
                        }
                    }
                    SnowflakeContextTarget::Warehouse { warehouse } => {
                        metadata::quote_identifier(warehouse)?;
                        if !snapshot.warehouses.contains(warehouse) {
                            return Err(stale_session_options());
                        }
                    }
                    SnowflakeContextTarget::Database { database, schema } => {
                        metadata::quote_identifier(database)?;
                        if let Some(schema) = schema {
                            metadata::quote_identifier(schema)?;
                        }
                        if !snapshot.databases.contains(database) {
                            return Err(stale_session_options());
                        }
                    }
                    SnowflakeContextTarget::Schema { database, schema } => {
                        metadata::quote_identifier(database)?;
                        metadata::quote_identifier(schema)?;
                        if snapshot.sample.context.database.as_deref() != Some(database.as_str())
                            || !snapshot.schemas.contains(schema)
                        {
                            return Err(stale_session_options());
                        }
                    }
                }
                self.require_context_catalog_revision(&generation)?;
                let before = self.snowflake_context_sample().await?;
                if before != snapshot.sample || before.transaction_id.is_some() {
                    return Err(stale_session_options());
                }
                match &request.target {
                    SnowflakeContextTarget::Role { role } => {
                        let rows = self
                            .metadata_read(
                                metadata::SHOW_ROLES_SQL,
                                metadata::SHOW_CONTEXT_LIMIT + 1,
                                metadata::SHOW_CONTEXT_MAX_BYTES,
                            )
                            .await?;
                        let mut live_roles = metadata::parse_show_names(&rows)?;
                        if let Some(active) = &before.context.role
                            && !live_roles.contains(active)
                        {
                            live_roles.push(active.clone());
                        }
                        if !live_roles.contains(role) {
                            return Err(stale_session_options());
                        }
                    }
                    SnowflakeContextTarget::Warehouse { warehouse } => {
                        let rows = self
                            .metadata_read(
                                metadata::SHOW_WAREHOUSES_SQL,
                                metadata::SHOW_CONTEXT_LIMIT + 1,
                                metadata::SHOW_CONTEXT_MAX_BYTES,
                            )
                            .await?;
                        let mut live_warehouses = metadata::parse_show_names(&rows)?;
                        if let Some(active) = &before.context.warehouse
                            && !live_warehouses.contains(active)
                        {
                            live_warehouses.push(active.clone());
                        }
                        if !live_warehouses.contains(warehouse) {
                            return Err(stale_session_options());
                        }
                    }
                    SnowflakeContextTarget::Database { database, .. } => {
                        let rows = self
                            .metadata_read(
                                metadata::SHOW_DATABASES_SQL,
                                metadata::SHOW_CONTEXT_LIMIT + 1,
                                metadata::SHOW_CONTEXT_MAX_BYTES,
                            )
                            .await?;
                        let mut live_databases = metadata::parse_show_names(&rows)?;
                        if let Some(active) = &before.context.database
                            && !live_databases.contains(active)
                        {
                            live_databases.push(active.clone());
                        }
                        if !live_databases.contains(database) {
                            return Err(stale_session_options());
                        }
                    }
                    SnowflakeContextTarget::Schema { schema, .. } => {
                        let rows = self
                            .read_spec(&metadata::schemas_read(&before.context)?)
                            .await?;
                        let live_schemas = metadata::parse_schemas(&before.context, &rows)?;
                        if !live_schemas.contains(schema) {
                            return Err(stale_session_options());
                        }
                    }
                }
                let expected = json!({
                    "user": before.context.principal,
                    "role": before.context.role,
                    "database": before.context.database,
                    "schema": before.context.schema,
                    "warehouse": before.context.warehouse,
                    "session": before.context.session_id,
                });
                let payload = match &request.target {
                    SnowflakeContextTarget::Role { role } => {
                        json!({ "kind": "role", "role": role, "expected": expected })
                    }
                    SnowflakeContextTarget::Warehouse { warehouse } => {
                        json!({ "kind": "warehouse", "warehouse": warehouse, "expected": expected })
                    }
                    SnowflakeContextTarget::Database { database, schema } => {
                        json!({ "kind": "database", "database": database, "schema": schema, "expected": expected })
                    }
                    SnowflakeContextTarget::Schema { database, schema } => {
                        json!({ "kind": "schema", "database": database, "schema": schema, "expected": expected })
                    }
                };
                {
                    let catalog = self.context_catalog.lock();
                    if catalog.revision != generation || self.is_closed() {
                        return Err(stale_session_options());
                    }
                    #[cfg(all(unix, feature = "test-harness"))]
                    self.check_metadata_observer()?;
                    lease.operation = Some(
                        self.process
                            .request(
                                "switch-context",
                                "switchContext",
                                payload,
                                Impact::ContextChange,
                            )
                            .map_err(control_error)?,
                    );
                }
                let report = {
                    let operation = lease.operation.as_mut().ok_or_else(protocol_error)?;
                    *self.active.lock() = Some(Active {
                        token: operation.token(),
                        terminal: operation.terminal_receiver(),
                    });
                    let _active = ActiveLease(&self.active);
                    operation.wait().await
                };
                if !report.ok
                    && report.use_acknowledged.is_none()
                    && report.certainty == Certainty::ServerTerminal
                    && report.error_code.as_deref() == Some("database")
                {
                    lease.published = true;
                    return Err(AppError::new(
                        ErrorCode::Database,
                        "Snowflake rejected the context switch; check privileges and refresh context options",
                    ));
                }
                check_report(&report)?;
                if report.use_acknowledged.is_none() {
                    return Err(protocol_error());
                }
                let readback = report.result.as_ref().ok_or_else(protocol_error)?;
                if readback.get("user") != Some(&json!(before.context.principal))
                    || readback.get("session") != Some(&json!(before.context.session_id))
                {
                    return Err(protocol_error());
                }
                match &request.target {
                    SnowflakeContextTarget::Role { role } => {
                        if readback.get("role") != Some(&json!(role)) {
                            return Err(protocol_error());
                        }
                    }
                    SnowflakeContextTarget::Warehouse { warehouse } => {
                        if readback.get("role") != Some(&json!(before.context.role))
                            || readback.get("database") != Some(&json!(before.context.database))
                            || readback.get("schema") != Some(&json!(before.context.schema))
                            || readback.get("warehouse") != Some(&json!(warehouse))
                        {
                            return Err(protocol_error());
                        }
                    }
                    SnowflakeContextTarget::Database { database, schema } => {
                        if readback.get("role") != Some(&json!(before.context.role))
                            || readback.get("database") != Some(&json!(database))
                            || schema
                                .as_ref()
                                .is_some_and(|s| readback.get("schema") != Some(&json!(s)))
                            || readback.get("warehouse") != Some(&json!(before.context.warehouse))
                        {
                            return Err(protocol_error());
                        }
                    }
                    SnowflakeContextTarget::Schema { database, schema } => {
                        if readback.get("role") != Some(&json!(before.context.role))
                            || readback.get("database") != Some(&json!(database))
                            || readback.get("schema") != Some(&json!(schema))
                            || readback.get("warehouse") != Some(&json!(before.context.warehouse))
                        {
                            return Err(protocol_error());
                        }
                    }
                }
                let next_database = match readback.get("database") {
                    Some(Value::String(db)) => Some(db.clone()),
                    Some(Value::Null) => None,
                    _ => return Err(protocol_error()),
                };
                *self.active_database.lock() = next_database;
                self.require_context_catalog_revision(&generation)?;
                let after = self.snowflake_context_sample().await?;
                if after.transaction_id.is_some()
                    || after.autocommit != before.autocommit
                    || after.abort_on_error != before.abort_on_error
                {
                    return Err(stale_session_options());
                }
                match &request.target {
                    SnowflakeContextTarget::Role { role } => {
                        if after.context.role.as_deref() != Some(role.as_str()) {
                            return Err(stale_session_options());
                        }
                    }
                    SnowflakeContextTarget::Warehouse { warehouse } => {
                        if after.context.warehouse.as_deref() != Some(warehouse.as_str()) {
                            return Err(stale_session_options());
                        }
                    }
                    SnowflakeContextTarget::Database { database, schema } => {
                        if after.context.database.as_deref() != Some(database.as_str())
                            || schema.as_ref().is_some_and(|s| {
                                after.context.schema.as_deref() != Some(s.as_str())
                            })
                        {
                            return Err(stale_session_options());
                        }
                    }
                    SnowflakeContextTarget::Schema { database, schema } => {
                        if after.context.database.as_deref() != Some(database.as_str())
                            || after.context.schema.as_deref() != Some(schema.as_str())
                        {
                            return Err(stale_session_options());
                        }
                    }
                }
                let (roles, warehouses, databases, schemas) =
                    self.read_context_lists(&after.context).await?;
                let verified = self.snowflake_context_sample().await?;
                if session::compare_samples(Some(&after), Some(&verified))
                    != SampleConsistency::Stable
                {
                    return Err(stale_session_options());
                }
                let tx_generation = self.revoke_transaction_grant();
                let state = if verified.context.database.is_some() {
                    self.publish_transaction_sample(&verified, &tx_generation)?
                } else {
                    self.observed_context_catalog_state(&verified)?
                };
                let revision = uuid::Uuid::new_v4().to_string();
                let mut catalog = self.context_catalog.lock();
                if catalog.revision != generation || self.is_closed() {
                    return Err(stale_session_options());
                }
                #[cfg(all(unix, feature = "test-harness"))]
                self.check_metadata_observer()?;
                *catalog = ContextCatalogState {
                    revision: revision.clone(),
                    snapshot: Some(ContextCatalogSnapshot {
                        sample: verified.clone(),
                        roles: roles.clone(),
                        warehouses: warehouses.clone(),
                        databases: databases.clone(),
                        schemas: schemas.clone(),
                    }),
                };
                if verified.context.database.is_some() {
                    *self.session_options.lock() = SessionOptionsState {
                        revision: revision.clone(),
                        snapshot: Some(SessionOptionsSnapshot {
                            sample: verified,
                            available: schemas.clone(),
                        }),
                    };
                }
                Ok(SnowflakeContextCatalog {
                    state,
                    available_roles: roles,
                    available_warehouses: warehouses,
                    available_databases: databases,
                    available_schemas: schemas,
                    revision,
                })
            })
            .await;
        match result {
            Ok(catalog) => {
                lease.published = true;
                Ok(catalog)
            }
            Err(error) => {
                self.invalidate_context_catalog_revision(&generation);
                self.invalidate_session_options_revision(&generation);
                if let Some(operation) = &lease.operation
                    && !lease.published
                {
                    let acknowledged = operation.use_acknowledged().is_some();
                    self.start_shutdown();
                    if acknowledged {
                        return Err(AppError::new(
                            ErrorCode::ConnectionClosed,
                            "Snowflake acknowledged the context change, but its final session context could not be verified. The session was closed; reconnect before continuing.",
                        )
                        .with_completion_invalidated());
                    }
                }
                Err(error.with_completion_invalidated())
            }
        }
    }

    fn invalidate_relation_catalog(&self) {
        {
            let mut state = self.browsing.lock();
            *state = BrowseState::default();
            self.revoke_edit_snapshots();
        }
        self.revoke_ddl_grant();
    }

    async fn relation_catalog(&self) -> AppResult<BrowseCatalog> {
        self.grid.require_ordinary()?;
        let revision = uuid::Uuid::new_v4().to_string();
        // Refresh owns a new generation before its first await, even if it fails
        // to acquire the lane or the caller abandons it.
        {
            let mut state = self.browsing.lock();
            *state = BrowseState {
                revision: revision.clone(),
                snapshot: None,
            };
            self.revoke_edit_snapshots();
        }
        self.browse_operation(&revision, async {
            if self.browsing.lock().revision != revision {
                return Err(stale_browse());
            }
            let context = self.browse_context(None).await?;
            let rows = self.read_spec(&metadata::schemas_read(&context)?).await?;
            let schemas = browsing::parse_schemas(&context, &rows)?;
            #[cfg(all(unix, feature = "test-harness"))]
            self.observe_schemas(
                MetadataSite::RelationCatalogBatch,
                &context,
                &rows,
                &schemas,
            )?;
            let mut relations = Vec::new();
            let mut bytes = 0usize;
            for schema in &schemas {
                let rows = self
                    .read_spec(&metadata::relations_read(&context, schema)?)
                    .await?;
                let batch = browsing::parse_relations(&context, schema, &rows)?;
                #[cfg(all(unix, feature = "test-harness"))]
                self.observe_relations(
                    MetadataSite::RelationCatalogBatch,
                    &context,
                    Some(schema),
                    &batch,
                )?;
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
            #[cfg(all(unix, feature = "test-harness"))]
            self.observe_relations(
                MetadataSite::RelationCatalogSnapshot,
                &context,
                None,
                &relations,
            )?;
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
                database: context.database.clone().ok_or_else(stale_browse)?,
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
            #[cfg(all(unix, feature = "test-harness"))]
            self.check_metadata_observer()?;
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
        self.grid.require_ordinary()?;
        let (context, target) = self.browse_target(&request.revision, &request.target_id)?;
        let inspection = uuid::Uuid::new_v4().to_string();
        {
            let mut state = self.browsing.lock();
            if state.revision != request.revision {
                return Err(stale_browse());
            }
            self.edit_snapshots.revoke_target(
                &self.session_id,
                &self.process.helper_generation(),
                &request.target_id,
            );
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
                        preview_revision: None,
                    },
                );
        }
        self.browse_operation(&request.revision, async {
            self.require_inspection_attempt(&request.revision, &request.target_id, &inspection)?;
            self.browse_context(Some(&context)).await?;
            self.require_browse_relation(&context, &target).await?;
            let [columns_read, describe_read, resolved_read] = metadata::inspection_reads(&target)?;
            let columns = self.read_spec(&columns_read).await?;
            let describe = self.read_spec(&describe_read).await?;
            let resolved = self.read_spec(&resolved_read).await?;
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
            #[cfg(all(unix, feature = "test-harness"))]
            self.check_metadata_observer()?;
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

    async fn define_relation(
        &self,
        request: &DefineRelationRequest,
    ) -> AppResult<RelationDefinition> {
        self.grid.require_ordinary()?;
        let (context, target) = self.browse_target(&request.revision, &request.target_id)?;
        let read = metadata::definition_read(&target)?;
        self.browse_operation(&request.revision, async {
            // Recheck after the metadata lane wait; a refresh may have replaced
            // the retained target before this operation acquired the lane.
            self.browse_target(&request.revision, &request.target_id)?;
            self.browse_context(Some(&context)).await?;
            self.require_browse_relation(&context, &target).await?;
            let ddl = browsing::parse_definition(&self.read_spec(&read).await?)?;
            self.require_browse_relation(&context, &target).await?;
            self.browse_context(Some(&context)).await?;
            self.browse_target(&request.revision, &request.target_id)?;
            #[cfg(all(unix, feature = "test-harness"))]
            self.check_metadata_observer()?;
            let result = RelationDefinition {
                revision: request.revision.clone(),
                target_id: request.target_id.clone(),
                database: target.database,
                table: target.table,
                ddl,
            };
            if serde_json::to_vec(&result)
                .map_err(|_| protocol_error())?
                .len()
                > browsing::DEFINITION_MAX_BYTES
            {
                return Err(metadata_limit());
            }
            Ok(result)
        })
        .await
    }

    async fn inspect_relation_constraints(
        &self,
        request: &InspectRelationConstraintsRequest,
    ) -> AppResult<RelationConstraints> {
        self.grid.require_ordinary()?;
        let (context, target) = self.browse_target(&request.revision, &request.target_id)?;
        let read = metadata::constraints_read(&target)?;
        self.browse_operation(&request.revision, async {
            self.browse_target(&request.revision, &request.target_id)?;
            self.browse_context(Some(&context)).await?;
            self.require_browse_relation(&context, &target).await?;
            let constraints = browsing::parse_constraints(&target, &self.read_spec(&read).await?)?;
            self.require_browse_relation(&context, &target).await?;
            self.browse_context(Some(&context)).await?;
            let result = RelationConstraints {
                revision: request.revision.clone(),
                target_id: request.target_id.clone(),
                database: target.database,
                table: target.table,
                constraints,
            };
            if serde_json::to_vec(&result)
                .map_err(|_| protocol_error())?
                .len()
                > browsing::CONSTRAINT_MAX_BYTES
            {
                return Err(metadata_limit());
            }
            self.browse_target(&request.revision, &request.target_id)?;
            #[cfg(all(unix, feature = "test-harness"))]
            self.check_metadata_observer()?;
            Ok(result)
        })
        .await
    }

    async fn inspect_relation_check_constraints(
        &self,
        request: &InspectRelationCheckConstraintsRequest,
    ) -> AppResult<RelationCheckConstraints> {
        self.grid.require_ordinary()?;
        let (context, target) = self.browse_target(&request.revision, &request.target_id)?;
        let read = metadata::check_constraints_read(&target)?;
        self.browse_operation(&request.revision, async {
            self.browse_target(&request.revision, &request.target_id)?;
            self.browse_context(Some(&context)).await?;
            self.require_browse_relation(&context, &target).await?;
            let constraints =
                browsing::parse_check_constraints(&target, &self.read_spec(&read).await?)?;
            self.require_browse_relation(&context, &target).await?;
            self.browse_context(Some(&context)).await?;
            let result = RelationCheckConstraints {
                revision: request.revision.clone(),
                target_id: request.target_id.clone(),
                database: target.database,
                table: target.table,
                constraints,
            };
            if serde_json::to_vec(&result)
                .map_err(|_| protocol_error())?
                .len()
                > browsing::CHECK_MAX_BYTES
            {
                return Err(metadata_limit());
            }
            self.browse_target(&request.revision, &request.target_id)?;
            #[cfg(all(unix, feature = "test-harness"))]
            self.check_metadata_observer()?;
            Ok(result)
        })
        .await
    }

    async fn inspect_relation_constraint_mappings(
        &self,
        request: &InspectRelationConstraintMappingsRequest,
    ) -> AppResult<RelationConstraintMappings> {
        self.grid.require_ordinary()?;
        let (context, target) = self.browse_target(&request.revision, &request.target_id)?;
        // Unknown traits are rejected by browse_target before SQL. The explicit
        // unsupported result below is only for an admitted dynamic relation.
        let declarations_read = metadata::constraints_read(&target)?;
        self.browse_operation(&request.revision, async {
            self.browse_target(&request.revision, &request.target_id)?;
            self.browse_context(Some(&context)).await?;
            self.require_browse_relation(&context, &target).await?;
            let declarations =
                browsing::parse_constraints(&target, &self.read_spec(&declarations_read).await?)?;
            let mappings = if target.traits.dynamic == Some(false) {
                let reads = metadata::mapping_reads(&target)?;
                let primary = self.read_spec(&reads[0]).await?;
                let unique = self.read_spec(&reads[1]).await?;
                let foreign = self.read_spec(&reads[2]).await?;
                browsing::parse_mappings(&target, &declarations, [&primary, &unique, &foreign])?
            } else {
                browsing::unsupported_mappings(&declarations)
            };
            self.require_browse_relation(&context, &target).await?;
            self.browse_context(Some(&context)).await?;
            let result = RelationConstraintMappings {
                revision: request.revision.clone(),
                target_id: request.target_id.clone(),
                database: target.database,
                table: target.table,
                mappings,
            };
            if serde_json::to_vec(&result)
                .map_err(|_| protocol_error())?
                .len()
                > browsing::CONSTRAINT_MAX_BYTES
            {
                return Err(metadata_limit());
            }
            self.browse_target(&request.revision, &request.target_id)?;
            #[cfg(all(unix, feature = "test-harness"))]
            self.check_metadata_observer()?;
            Ok(result)
        })
        .await
    }

    async fn preview_relation(
        &self,
        request: &PreviewRelationRequest,
    ) -> AppResult<RelationPreview> {
        self.grid.require_ordinary()?;
        let (context, target) = self.browse_target(&request.revision, &request.target_id)?;
        let columns = self.browse_columns(
            &request.revision,
            &request.target_id,
            &request.inspection_revision,
        )?;
        let slice = browsing::PreviewSlice::from_request(request);
        let (sql, binds) = browsing::preview_query(&target, &columns, &slice)?;
        let preview_revision = self.begin_edit_preview(request)?;
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
                .metadata_read_bound(
                    &sql,
                    binds,
                    browsing::PREVIEW_LIMIT + 1,
                    browsing::PREVIEW_MAX_BYTES,
                )
                .await?;
            let preview = browsing::parse_preview_with_offset(&columns, &rows, slice.offset)?;
            self.require_browse_relation(&context, &target).await?;
            self.browse_context(Some(&context)).await?;
            self.browse_columns(
                &request.revision,
                &request.target_id,
                &request.inspection_revision,
            )?;
            self.retain_edit_preview(request, &preview_revision, preview)
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
            let schemas = self.read_spec(&metadata::schemas_read(&context)?).await?;
            let mut relations = Vec::new();
            let mut relation_bytes = 0usize;
            let parsed_schemas = metadata::parse_schemas(&context, &schemas)?;
            #[cfg(all(unix, feature = "test-harness"))]
            self.observe_schemas(
                MetadataSite::CompletionCatalog,
                &context,
                &schemas,
                &parsed_schemas,
            )?;
            for schema in parsed_schemas {
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

    async fn inspection_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.completion_schema(table).await
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
        {
            let _mode = self.console_mode.lock();
            self.closed.cancel();
        }
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
    async fn database_objects(&self) -> AppResult<Vec<DatabaseObjectSummary>> {
        self.grid.require_ordinary()?;
        self.metadata_operation(async {
            let context = self.live_metadata_context().await?;
            let schema_rows = self.read_spec(&metadata::schemas_read(&context)?).await?;
            let schemas = metadata::parse_schemas(&context, &schema_rows)?;
            let mut objects = Vec::new();
            let mut total_bytes = 0usize;
            for schema in &schemas {
                let table_rows = self
                    .read_spec(&metadata::explorer_tables_read(&context, schema)?)
                    .await?;
                let seq_rows = self
                    .read_spec(&metadata::explorer_sequences_read(&context, schema)?)
                    .await?;
                let fn_rows = self
                    .read_spec(&metadata::explorer_functions_read(&context, schema)?)
                    .await?;
                let proc_rows = self
                    .read_spec(&metadata::explorer_procedures_read(&context, schema)?)
                    .await?;
                for batch in [
                    explorer::parse_tables(&context, schema, &table_rows)?,
                    explorer::parse_sequences(&context, schema, &seq_rows)?,
                    explorer::parse_routines(&context, schema, false, &fn_rows)?,
                    explorer::parse_routines(&context, schema, true, &proc_rows)?,
                ] {
                    total_bytes = total_bytes
                        .checked_add(
                            serde_json::to_vec(&batch)
                                .map_err(|_| protocol_error())?
                                .len(),
                        )
                        .ok_or_else(metadata_limit)?;
                    if objects.len().saturating_add(batch.len()) > explorer::OBJECT_LIMIT
                        || total_bytes > explorer::OBJECT_MAX_BYTES
                    {
                        return Err(metadata_limit());
                    }
                    objects.extend(batch);
                }
            }
            self.recheck_metadata_context(&context).await?;
            Ok(objects)
        })
        .await
    }
    async fn object_details(&self, object: &DatabaseObjectRef) -> AppResult<ObjectDetails> {
        self.grid.require_ordinary()?;
        if !matches!(
            object.kind,
            DatabaseObjectKind::Table
                | DatabaseObjectKind::View
                | DatabaseObjectKind::MaterializedView
                | DatabaseObjectKind::Sequence
                | DatabaseObjectKind::Function
                | DatabaseObjectKind::Procedure
        ) {
            return Err(unsupported());
        }
        self.metadata_operation(async {
            let context = self.live_metadata_context().await?;
            let database = context.database.clone().ok_or_else(|| {
                AppError::new(
                    ErrorCode::Unsupported,
                    "Select an active Snowflake database before inspecting object details",
                )
            })?;
            let mut details = explorer::empty_details(object.clone());
            match object.kind {
                DatabaseObjectKind::Table
                | DatabaseObjectKind::View
                | DatabaseObjectKind::MaterializedView => {
                    let exact_read =
                        metadata::explorer_exact_table_read(&context, &object.schema, &object.name)?;
                    let exact = self.read_spec(&exact_read).await?;
                    let (target, comment, storage) = explorer::parse_exact_table(
                        &context,
                        &object.schema,
                        &object.name,
                        &exact,
                    )?;
                    let kind_matches = match object.kind {
                        DatabaseObjectKind::MaterializedView => {
                            target.table_type == "MATERIALIZED VIEW"
                        }
                        DatabaseObjectKind::View => target.table_type == "VIEW",
                        DatabaseObjectKind::Table => matches!(
                            target.table_type.as_str(),
                            "BASE TABLE" | "TEMPORARY TABLE" | "EXTERNAL TABLE" | "EVENT TABLE"
                        ),
                        _ => false,
                    };
                    if !kind_matches {
                        return Err(AppError::new(
                            ErrorCode::Conflict,
                            "Snowflake relation kind changed; refresh schema objects",
                        ));
                    }
                    details.comment = comment.clone();
                    if object.kind == DatabaseObjectKind::Table {
                        details.storage = Some(storage.clone());
                    }
                    let ddl_read = metadata::definition_read(&target)?;
                    details.ddl = match self.read_spec(&ddl_read).await {
                        Ok(result) => ObjectDdlStatus::Available {
                            ddl: explorer::parse_ddl(&result)?,
                            truncated: false,
                        },
                        Err(err) if err.code == ErrorCode::Database && !self.is_closed() => {
                            ObjectDdlStatus::Unavailable {
                                reason: err.message,
                            }
                        }
                        Err(err) => return Err(err),
                    };
                    if object.kind == DatabaseObjectKind::Table {
                        let key_rows = self.read_spec(&metadata::constraints_read(&target)?).await?;
                        let check_rows = self
                            .read_spec(&metadata::check_constraints_read(&target)?)
                            .await?;
                        let keys = browsing::parse_constraints(&target, &key_rows)?;
                        let mappings = if target.traits.dynamic == Some(false) {
                            let reads = metadata::mapping_reads(&target)?;
                            let primary = self.read_spec(&reads[0]).await?;
                            let unique = self.read_spec(&reads[1]).await?;
                            let foreign = self.read_spec(&reads[2]).await?;
                            browsing::parse_mappings(
                                &target,
                                &keys,
                                [&primary, &unique, &foreign],
                            )?
                        } else {
                            browsing::unsupported_mappings(&keys)
                        };
                        let checks = browsing::parse_check_constraints(&target, &check_rows)?;
                        for mapping in &mappings {
                            if mapping.kind
                                == crate::relation_inspection::RelationKeyConstraintKind::ForeignKey
                                && let Some(ref_table) = &mapping.referenced_table
                                && ref_table.database == database
                            {
                                details.uses.push(DependencyEdge {
                                    object: DatabaseObjectRef {
                                        schema: ref_table.table.schema.clone(),
                                        name: ref_table.table.name.clone(),
                                        kind: DatabaseObjectKind::Table,
                                        identity: None,
                                    },
                                    relation: "foreign key".into(),
                                });
                            }
                        }
                        details.constraints =
                            explorer::build_table_constraints(&database, &keys, &mappings, &checks);
                    } else if let ObjectDdlStatus::Available { ddl, .. } = &details.ddl {
                        let schema_rows = self.read_spec(&metadata::schemas_read(&context)?).await?;
                        let schemas = metadata::parse_schemas(&context, &schema_rows)?;
                        let mut known = Vec::new();
                        for schema in schemas.iter().take(64) {
                            let rows = self
                                .read_spec(&metadata::explorer_tables_read(&context, schema)?)
                                .await?;
                            let batch = explorer::parse_tables(&context, schema, &rows)?;
                            if known.len().saturating_add(batch.len()) > explorer::OBJECT_LIMIT {
                                break;
                            }
                            known.extend(batch);
                        }
                        details.uses = explorer::extract_view_dependencies(
                            &database,
                            &object.schema,
                            &object.name,
                            ddl,
                            &known,
                        );
                    }
                    let verify = self.read_spec(&exact_read).await?;
                    let after = explorer::parse_exact_table(
                        &context,
                        &object.schema,
                        &object.name,
                        &verify,
                    )?;
                    if after != (target, comment, storage) {
                        return Err(AppError::new(
                            ErrorCode::Conflict,
                            "Snowflake relation changed while inspecting details; refresh and retry",
                        ));
                    }
                }
                DatabaseObjectKind::Sequence => {
                    let exact_read = metadata::explorer_exact_sequence_read(
                        &context,
                        &object.schema,
                        &object.name,
                    )?;
                    let exact = self.read_spec(&exact_read).await?;
                    let (sequence, comment) = explorer::parse_exact_sequence(
                        &context,
                        &object.schema,
                        &object.name,
                        &exact,
                    )?;
                    details.comment = comment.clone();
                    details.sequence = Some(sequence.clone());
                    let ddl_read =
                        metadata::explorer_sequence_ddl_read(&database, &object.schema, &object.name)?;
                    details.ddl = match self.read_spec(&ddl_read).await {
                        Ok(result) => ObjectDdlStatus::Available {
                            ddl: explorer::parse_ddl(&result)?,
                            truncated: false,
                        },
                        Err(err) if err.code == ErrorCode::Database && !self.is_closed() => {
                            ObjectDdlStatus::Unavailable {
                                reason: err.message,
                            }
                        }
                        Err(err) => return Err(err),
                    };
                    let verify = self.read_spec(&exact_read).await?;
                    let after = explorer::parse_exact_sequence(
                        &context,
                        &object.schema,
                        &object.name,
                        &verify,
                    )?;
                    if after.0.data_type != sequence.data_type
                        || after.0.start_value != sequence.start_value
                        || after.0.increment_by != sequence.increment_by
                        || after.0.cycle != sequence.cycle
                        || after.1 != comment
                    {
                        return Err(AppError::new(
                            ErrorCode::Conflict,
                            "Snowflake sequence changed while inspecting details; refresh and retry",
                        ));
                    }
                }
                DatabaseObjectKind::Function | DatabaseObjectKind::Procedure => {
                    let is_procedure = object.kind == DatabaseObjectKind::Procedure;
                    let exact_read = metadata::explorer_exact_routine_read(
                        &context,
                        &object.schema,
                        &object.name,
                        is_procedure,
                    )?;
                    let exact = self.read_spec(&exact_read).await?;
                    let (routine, identity_args, comment) = explorer::parse_exact_routine(
                        &context,
                        &object.schema,
                        &object.name,
                        is_procedure,
                        object.identity.as_deref(),
                        &exact,
                    )?;
                    details.object.identity = Some(identity_args.clone());
                    details.comment = comment.clone();
                    details.routine = Some(routine.clone());
                    let ddl_read = metadata::explorer_routine_ddl_read(
                        &database,
                        &object.schema,
                        &object.name,
                        is_procedure,
                        &identity_args,
                    )?;
                    details.ddl = match self.read_spec(&ddl_read).await {
                        Ok(result) => ObjectDdlStatus::Available {
                            ddl: explorer::parse_ddl(&result)?,
                            truncated: false,
                        },
                        Err(err) if err.code == ErrorCode::Database && !self.is_closed() => {
                            ObjectDdlStatus::Unavailable {
                                reason: err.message,
                            }
                        }
                        Err(err) => return Err(err),
                    };
                    let verify = self.read_spec(&exact_read).await?;
                    let after = explorer::parse_exact_routine(
                        &context,
                        &object.schema,
                        &object.name,
                        is_procedure,
                        Some(&identity_args),
                        &verify,
                    )?;
                    if after.0.arguments != routine.arguments
                        || after.0.return_type != routine.return_type
                        || after.0.language != routine.language
                        || after.0.volatility != routine.volatility
                        || after.2 != comment
                    {
                        return Err(AppError::new(
                            ErrorCode::Conflict,
                            "Snowflake routine changed while inspecting details; refresh and retry",
                        ));
                    }
                }
                _ => return Err(unsupported()),
            }
            self.recheck_metadata_context(&context).await?;
            Ok(details)
        })
        .await
    }
    async fn schema_diagram(&self, schema: &str) -> AppResult<SchemaDiagram> {
        use super::snowflake_explorer as explorer;
        self.grid.require_ordinary()?;
        self.require_ordinary_mode()?;
        explorer::validate_user_schema(schema)?;
        let schema_name = schema.to_owned();
        self.metadata_operation(async {
            let context = self.live_metadata_context().await?;
            if context.database.is_none() {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "Select an active Snowflake database before opening a schema diagram",
                ));
            }
            let schema_rows = self.read_spec(&metadata::schemas_read(&context)?).await?;
            let schemas = metadata::parse_schemas(&context, &schema_rows)?;
            if !schemas.iter().any(|s| s == &schema_name) {
                return Err(AppError::new(
                    ErrorCode::NotFound,
                    "The schema is missing or unavailable in the current Snowflake database",
                ));
            }
            let relations = self
                .read_spec(&metadata::explorer_schema_relations_read(
                    &context,
                    &schema_name,
                )?)
                .await?;
            let columns = self
                .read_spec(&metadata::explorer_schema_columns_read(
                    &context,
                    &schema_name,
                )?)
                .await?;
            let mappings = metadata::explorer_schema_mapping_reads(&context, &schema_name)?;
            let primary_keys = self.read_spec(&mappings[0]).await?;
            let foreign_keys = self.read_spec(&mappings[2]).await?;
            let diagram = explorer::parse_schema_diagram(
                &context,
                &schema_name,
                &relations,
                &columns,
                &primary_keys,
                &foreign_keys,
            )?;
            self.recheck_metadata_context(&context).await?;
            Ok(diagram)
        })
        .await
    }
    async fn schema_snapshot(&self, schema: &str) -> AppResult<SchemaSnapshot> {
        use super::snowflake_explorer as explorer;
        self.grid.require_ordinary()?;
        self.require_ordinary_mode()?;
        explorer::validate_user_schema(schema)?;
        let schema_name = schema.to_owned();
        let server_version = self.server_version.lock().clone();
        self.metadata_operation(async {
            let context = self.live_metadata_context().await?;
            let Some(database) = context.database.clone() else {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "Select an active Snowflake database before capturing a schema snapshot",
                ));
            };
            let schema_rows = self.read_spec(&metadata::schemas_read(&context)?).await?;
            let schemas = metadata::parse_schemas(&context, &schema_rows)?;
            if !schemas.iter().any(|s| s == &schema_name) {
                return Err(AppError::new(
                    ErrorCode::NotFound,
                    "The schema is missing or unavailable in the current Snowflake database",
                ));
            }
            let relations = self
                .read_spec(&metadata::explorer_schema_relations_read(
                    &context,
                    &schema_name,
                )?)
                .await?;
            let columns = self
                .read_spec(&metadata::explorer_schema_columns_read(
                    &context,
                    &schema_name,
                )?)
                .await?;
            let constraints = self
                .read_spec(&metadata::explorer_schema_constraints_read(
                    &context,
                    &schema_name,
                )?)
                .await?;
            let checks = self
                .read_spec(&metadata::explorer_schema_checks_read(
                    &context,
                    &schema_name,
                )?)
                .await?;
            let mappings = metadata::explorer_schema_mapping_reads(&context, &schema_name)?;
            let primary_keys = self.read_spec(&mappings[0]).await?;
            let unique_keys = self.read_spec(&mappings[1]).await?;
            let foreign_keys = self.read_spec(&mappings[2]).await?;
            let sequences = self
                .read_spec(&metadata::explorer_sequences_read(&context, &schema_name)?)
                .await?;
            let functions = self
                .read_spec(&metadata::explorer_functions_read(&context, &schema_name)?)
                .await?;
            let procedures = self
                .read_spec(&metadata::explorer_procedures_read(&context, &schema_name)?)
                .await?;

            let mut plan = explorer::parse_schema_snapshot_base(
                &context,
                &server_version,
                &schema_name,
                &relations,
                &columns,
                &constraints,
                &checks,
                &primary_keys,
                &unique_keys,
                &foreign_keys,
                &sequences,
                &functions,
                &procedures,
            )?;

            let mut ddl_budget = explorer::SCHEMA_SNAPSHOT_DDL_LIMIT;
            for target in &plan.view_targets {
                if ddl_budget == 0 {
                    plan.snapshot.truncated = true;
                    break;
                }
                ddl_budget -= 1;
                let ddl_read = metadata::definition_read(target)?;
                let view_ddl = match self.read_spec(&ddl_read).await {
                    Ok(res) => Some(explorer::strip_active_database_qualifier(
                        &database,
                        &explorer::parse_ddl(&res)?,
                    )),
                    Err(err)
                        if matches!(
                            err.code,
                            ErrorCode::Database | ErrorCode::PermissionDenied
                        ) && !self.is_closed() =>
                    {
                        plan.snapshot.restricted = true;
                        None
                    }
                    Err(err) => return Err(err),
                };
                if let Some(slot) = plan.snapshot.tables.iter_mut().find(|t| {
                    t.name == target.table.name
                        && matches!(
                            t.kind,
                            DatabaseObjectKind::View | DatabaseObjectKind::MaterializedView
                        )
                }) {
                    slot.view_definition = view_ddl;
                }
            }

            for routine in &mut plan.snapshot.routines {
                if ddl_budget == 0 {
                    plan.snapshot.truncated = true;
                    break;
                }
                ddl_budget -= 1;
                let ddl_read = metadata::explorer_routine_ddl_read(
                    &database,
                    &schema_name,
                    &routine.name,
                    routine.kind == DatabaseObjectKind::Procedure,
                    &routine.identity_arguments,
                )?;
                routine.definition = match self.read_spec(&ddl_read).await {
                    Ok(res) => Some(explorer::strip_active_database_qualifier(
                        &database,
                        &explorer::parse_ddl(&res)?,
                    )),
                    Err(err)
                        if matches!(
                            err.code,
                            ErrorCode::Database | ErrorCode::PermissionDenied
                        ) && !self.is_closed() =>
                    {
                        plan.snapshot.restricted = true;
                        None
                    }
                    Err(err) => return Err(err),
                };
            }

            plan.snapshot.canonicalize();
            self.recheck_metadata_context(&context).await?;
            Ok(plan.snapshot)
        })
        .await
    }
    async fn search_data(&self, request: &DataSearchRequest) -> AppResult<DataSearchReport> {
        use super::snowflake_explorer as explorer;
        use crate::data_search::{
            MAX_SEARCH_REPORT_BYTES, MAX_SEARCH_ROWS_PER_TABLE, MAX_SEARCH_TOTAL_ROW_HITS,
            build_snowflake_search_sql, build_table_match, estimate_table_match_bytes,
            filter_candidate_tables, validate_search_request,
        };
        self.grid.require_ordinary()?;
        self.require_ordinary_mode()?;
        let max_tables = validate_search_request(request)?;
        let started = std::time::Instant::now();

        self.metadata_operation(async {
            let context = self.live_metadata_context().await?;
            let Some(database) = context.database.clone() else {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "Select an active Snowflake database before searching table data",
                ));
            };
            let schema_rows = self.read_spec(&metadata::schemas_read(&context)?).await?;
            let schemas = metadata::parse_schemas(&context, &schema_rows)?;
            let mut discovered = Vec::new();
            let mut skipped_tables = 0usize;
            for s in &schemas {
                if self.is_closed() {
                    return Err(closed_error());
                }
                if !request.schemas.is_empty() && !request.schemas.iter().any(|wanted| wanted == s)
                {
                    continue;
                }
                if !request.tables.is_empty()
                    && !request.tables.iter().any(|wanted| &wanted.schema == s)
                {
                    continue;
                }
                let table_rows = match self
                    .read_spec(&metadata::explorer_tables_read(&context, s)?)
                    .await
                {
                    Ok(rows) => rows,
                    Err(err)
                        if matches!(
                            err.code,
                            ErrorCode::Database | ErrorCode::PermissionDenied
                        ) && !self.is_closed() =>
                    {
                        skipped_tables += 1;
                        continue;
                    }
                    Err(err) => return Err(err),
                };
                for obj in explorer::parse_tables(&context, s, &table_rows)? {
                    discovered.push(TableRef {
                        schema: obj.object.schema,
                        name: obj.object.name,
                    });
                }
            }
            let (candidates, mut truncated) =
                filter_candidate_tables(discovered, request, max_tables);

            let mut scanned_tables = 0usize;
            let mut total_row_hits = 0usize;
            let mut report_bytes = 0usize;
            let mut matches = Vec::new();

            for table in &candidates {
                if self.is_closed() {
                    return Err(closed_error());
                }
                if total_row_hits >= MAX_SEARCH_TOTAL_ROW_HITS
                    || report_bytes >= MAX_SEARCH_REPORT_BYTES
                {
                    truncated = true;
                    break;
                }
                let col_sql = match metadata::resolved_columns_sql(&context, table) {
                    Ok(s) => s,
                    Err(_) => {
                        skipped_tables += 1;
                        continue;
                    }
                };
                let col_rows = match self
                    .metadata_read(&col_sql, 0, metadata::COLUMN_MAX_BYTES)
                    .await
                {
                    Ok(r) => r,
                    Err(err)
                        if matches!(
                            err.code,
                            ErrorCode::Database
                                | ErrorCode::PermissionDenied
                                | ErrorCode::Unsupported
                        ) && !self.is_closed() =>
                    {
                        skipped_tables += 1;
                        continue;
                    }
                    Err(err) => return Err(err),
                };
                let schema = match metadata::parse_resolved_columns(table, col_rows) {
                    Ok(s) => s,
                    Err(_) => {
                        skipped_tables += 1;
                        continue;
                    }
                };
                let Some((sql, binds, searchable)) = build_snowflake_search_sql(
                    &database,
                    &schema,
                    &request.query,
                    request.match_mode,
                    request.case_sensitive,
                    MAX_SEARCH_ROWS_PER_TABLE,
                ) else {
                    skipped_tables += 1;
                    continue;
                };
                scanned_tables += 1;
                let res = match self
                    .metadata_read_bound(
                        &sql,
                        binds,
                        MAX_SEARCH_ROWS_PER_TABLE + 1,
                        browsing::PREVIEW_MAX_BYTES,
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(err)
                        if matches!(
                            err.code,
                            ErrorCode::Database
                                | ErrorCode::PermissionDenied
                                | ErrorCode::Unsupported
                        ) && !self.is_closed() =>
                    {
                        skipped_tables += 1;
                        continue;
                    }
                    Err(err) => return Err(err),
                };
                let allowed_rows = MAX_SEARCH_TOTAL_ROW_HITS
                    .saturating_sub(total_row_hits)
                    .min(MAX_SEARCH_ROWS_PER_TABLE);
                if let Some(m) = build_table_match(
                    &schema,
                    searchable,
                    res.rows,
                    &request.query,
                    request.match_mode,
                    request.case_sensitive,
                    allowed_rows,
                ) {
                    let bytes = estimate_table_match_bytes(&m);
                    if report_bytes.saturating_add(bytes) > MAX_SEARCH_REPORT_BYTES
                        && !matches.is_empty()
                    {
                        truncated = true;
                        break;
                    }
                    report_bytes += bytes;
                    total_row_hits += m.rows.len();
                    if m.has_more_in_table && allowed_rows < MAX_SEARCH_ROWS_PER_TABLE {
                        truncated = true;
                    }
                    matches.push(m);
                }
            }

            self.recheck_metadata_context(&context).await?;
            Ok(DataSearchReport {
                query: request.query.clone(),
                match_mode: request.match_mode,
                case_sensitive: request.case_sensitive,
                database: Some(database),
                scanned_tables,
                skipped_tables,
                total_row_hits,
                truncated,
                elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
                matches,
            })
        })
        .await
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

fn ddl_summary_status(kind: SnowflakeDdlKind, result: Option<&Value>) -> SnowflakeDdlSummaryStatus {
    let valid = (|| -> Option<bool> {
        let result = result?;
        if result.get("kind")?.as_str()? != kind.as_str() {
            return None;
        }
        let columns = result.get("columns")?.as_array()?;
        if columns.len() > 2 {
            return None;
        }
        for column in columns {
            let name = column.get("name")?.as_str()?;
            let data_type = column.get("type")?.as_str()?;
            if name.is_empty() || data_type.is_empty() {
                return None;
            }
        }
        let rows: Vec<TextRow> = serde_json::from_value(result.get("values")?.clone()).ok()?;
        if rows.len() > 1
            || rows.iter().any(|row| row.len() != columns.len())
            || result.get("rowsSeen")?.as_u64()? != rows.len() as u64
            || result.get("rowsRetained")?.as_u64()? != 0
            || result.get("truncated")?.as_bool()?
        {
            return None;
        }
        Some(true)
    })();
    if valid == Some(true) {
        SnowflakeDdlSummaryStatus::Acknowledged
    } else {
        SnowflakeDdlSummaryStatus::Unavailable
    }
}

fn require_plain_base_table(target: &BrowseRelation) -> AppResult<()> {
    if target.availability != crate::relation_inspection::RelationAvailability::Available
        || target.table_type != "BASE TABLE"
        || target.traits.transient.is_none()
        || target.traits
            != (crate::relation_inspection::RelationTraits {
                transient: target.traits.transient,
                temporary: Some(false),
                iceberg: Some(false),
                dynamic: Some(false),
                hybrid: Some(false),
                immutable: Some(false),
            })
    {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Reviewed Snowflake DDL requires an available permanent or transient BASE TABLE with known non-temporary traits",
        ));
    }
    Ok(())
}

fn require_plain_view(target: &BrowseRelation) -> AppResult<()> {
    if target.availability != crate::relation_inspection::RelationAvailability::Available
        || target.table_type != "VIEW"
        || target.traits
            != (crate::relation_inspection::RelationTraits {
                transient: target.traits.transient,
                temporary: Some(false),
                iceberg: Some(false),
                dynamic: Some(false),
                hybrid: Some(false),
                immutable: Some(false),
            })
    {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Reviewed Snowflake DDL requires an available standard VIEW with known non-temporary traits",
        ));
    }
    Ok(())
}

fn qualified_ddl_target(components: &[String; 3]) -> AppResult<String> {
    Ok(format!(
        "{}.{}.{}",
        metadata::quote_identifier(&components[0])?,
        metadata::quote_identifier(&components[1])?,
        metadata::quote_identifier(&components[2])?,
    ))
}

fn ddl_warnings(
    kind: SnowflakeDdlKind,
    qualified_target: &str,
    target: Option<&BrowseRelation>,
) -> Vec<String> {
    let mut warnings = vec![
        "Snowflake DDL is not transactional and cannot be rolled back. Running this statement implicitly commits any open transaction work on the session.".into(),
        format!(
            "This operation targets the exact quoted SQL name shown above ({qualified_target}). Concurrent DDL or temporary objects can change name resolution, and dependent views, tasks, streams, or policies are not enumerated locally."
        ),
        "Catalog, relation browser, and SQL completion caches require an explicit post-DDL reconciliation refresh before continuing.".into(),
    ];
    if kind == SnowflakeDdlKind::CreateTable {
        warnings.push(
            "A table created in a transient schema or database is transient. Transient tables have only 0 or 1 day of Time Travel retention and no Fail-safe recovery period.".into(),
        );
    } else if target.is_some_and(|target| target.traits.transient == Some(true)) {
        warnings.push(
            "The reviewed target is a transient table: Time Travel retention is limited to 0 or 1 day and there is no Fail-safe recovery period. Recovery after a drop is not guaranteed.".into(),
        );
    }
    if kind == SnowflakeDdlKind::DropTable {
        warnings.push(
            "Dropping a Snowflake table immediately removes the table from active schemas. Time Travel recovery depends on remote object retention settings not verified here.".into(),
        );
    }
    if kind == SnowflakeDdlKind::DropColumn {
        warnings.push(
            "Dropping a Snowflake column permanently removes column visibility from the active table schema and breaks dependent views or queries referencing the column.".into(),
        );
    }
    if kind == SnowflakeDdlKind::DropView {
        warnings.push(
            "Dropping a Snowflake view immediately removes the view definition and invalidates downstream views or tasks referencing it.".into(),
        );
    }
    warnings
}

fn dml_summary(kind: SnowflakeDmlKind, result: Option<&Value>) -> SnowflakeDmlSummary {
    let parse = || -> Option<SnowflakeDmlSummary> {
        let result = result?;
        let columns = result.get("columns")?.as_array()?;
        if columns.len() > 2 {
            return None;
        }
        let columns = columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                Some(column_info((
                    index,
                    Column {
                        name: column.get("name")?.as_str()?.to_owned(),
                        data_type: column.get("type")?.as_str()?.to_owned(),
                    },
                )))
            })
            .collect::<Option<Vec<_>>>()?;
        let rows: Vec<TextRow> = serde_json::from_value(result.get("values")?.clone()).ok()?;
        if result.get("rowsSeen")?.as_u64()? != rows.len() as u64 {
            return None;
        }
        Some(super::snowflake_dml_summary::parse(
            kind,
            &columns,
            &rows,
            result.get("truncated")?.as_bool()?,
        ))
    };
    parse().unwrap_or_else(SnowflakeDmlSummary::unavailable)
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
fn normalize_wire_booleans(boolean_columns: &[bool], rows: &mut [TextRow]) {
    if !boolean_columns.iter().any(|&is_bool| is_bool) {
        return;
    }
    for row in rows {
        for (cell, &is_bool) in row.iter_mut().zip(boolean_columns) {
            if is_bool && let Some(value) = cell.as_mut() {
                if value == "TRUE" {
                    *value = "true".to_owned();
                } else if value == "FALSE" {
                    *value = "false".to_owned();
                }
            }
        }
    }
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
fn stale_transaction() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake transaction authority changed; refresh session state before reviewing another control",
    )
}

fn stale_session_options() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake session observation changed; refresh session context",
    )
}

fn stale_ddl() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake DDL context or catalog state changed; refresh session state and relations before reviewing again",
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

#[cfg(all(test, unix, feature = "test-harness"))]
mod session_tests {
    use super::*;
    use std::path::PathBuf;

    mod console_tests {
        include!("snowflake_external/console_tests.rs");
    }
    mod edit_snapshots_tests {
        include!("snowflake_external/edit_snapshots_tests.rs");
    }
    mod grid_freshness_tests {
        include!("snowflake_external/grid_freshness_tests.rs");
    }
    mod grid_adapter_tests {
        include!("snowflake_external/grid_adapter_tests.rs");
    }

    mod qualification_metadata_tests {
        include!("snowflake_qualification_metadata_adapter_tests.rs");
    }
    mod qualification_runtime_tests {
        include!("snowflake_qualification_runtime_adapter_tests.rs");
    }
    mod read_spec_tests {
        include!("snowflake_read_spec_adapter_tests.rs");
    }
    mod reference_navigation_tests {
        include!("snowflake_reference_navigation_tests.rs");
    }
    mod relation_definition_tests {
        include!("snowflake_relation_definition_adapter_tests.rs");
    }
    mod relation_mapping_tests {
        include!("snowflake_constraint_mappings_adapter_tests.rs");
    }
    mod check_constraints_tests {
        include!("snowflake_check_constraints_adapter_tests.rs");
    }
    mod relation_constraints_tests {
        include!("snowflake_relation_constraints_adapter_tests.rs");
    }
    mod qualification_finalizing_tests {
        include!("snowflake_qualification_finalizing_adapter_tests.rs");
    }

    // The real framed helper runs in a child with a synthetic SDK. No SDK import,
    // external-browser login, socket or account is used by this fixture.
    const FIXTURE: &str = r#"
import { Readable } from 'node:stream';
import { readFileSync, appendFileSync, existsSync, writeFileSync } from 'node:fs';
const { Helper } = await import(HELPER_URL);
const { Decoder, writer } = await import(FRAMING_URL);
const file = name => new URL(name, import.meta.url);
// Independent expected SQL: a fixture success must not mask unsupported implicit
// coercion in the production template. This models routing, not Snowflake typing.
const normalizedContextSql = 'SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES';
const oldContextSql = normalizedContextSql.replace('TO_VARCHAR(CURRENT_SCHEMAS())', 'TO_JSON(CURRENT_SCHEMAS())');
let sequence = 0, contextReads = 0, transactionReads = 0, previousSettings = '', currentRole = 'ROLE', currentWarehouse = 'WH', currentDatabase = 'DB', currentSchema = 'PUBLIC', switched = false, boundarySeen = false, dmlSeen = false, ddlSeen = false, definitionSeen = false, constraintsSeen = false, checksSeen = false, mappingSeen = false, currentTransaction = null;
const uuid = n => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const sdk = { createConnection() {
  appendFileSync(file('connections'), '1');
  return {
    connectAsync(callback) { queueMicrotask(() => callback(null)); return Promise.resolve(); },
    destroy(callback) { callback(null); },
    execute(options) {
      const sql = options.sqlText, id = ++sequence;
      appendFileSync(file('queries'), JSON.stringify(sql) + '\n');
      const settingsText = existsSync(file('settings')) ? readFileSync(file('settings'), 'utf8') : '{}';
      if (settingsText !== previousSettings) { previousSettings = settingsText; contextReads = 0; transactionReads = 0; definitionSeen = false; constraintsSeen = false; checksSeen = false; mappingSeen = false; }
      const settings = JSON.parse(settingsText);
      const value = (key, fallback) => Object.hasOwn(settings, key) ? settings[key] : fallback;
      // Opt-in navigation fixture; ordinary observer/qualification fixtures keep
      // their original inventories and routing unchanged.
      const navigation = settings.referenceNavigation === true;
      const navigationSchema = `Ref."Schema'雪`;
      const navigationTable = `Shared."Table'雪`;
      const predicate = column => {
        const match = new RegExp(column + " = '((?:[^']|'')*)'", 'u').exec(sql);
        return match ? match[1].replaceAll("''", "'") : null;
      };

      const context = sql === normalizedContextSql;
      const unsupportedContext = sql === oldContextSql;
      const schemas = sql.includes('.INFORMATION_SCHEMA.SCHEMATA ');
      const explorerExactTable = sql.includes('ROW_COUNT::VARCHAR AS ROW_COUNT') && sql.includes('IS_TRANSIENT');
      const explorerTables = sql.includes('ROW_COUNT::VARCHAR AS ROW_COUNT') && !sql.includes('IS_TRANSIENT');
      const explorerSequences = sql.includes('.INFORMATION_SCHEMA.SEQUENCES ');
      const explorerFunctions = sql.includes('.INFORMATION_SCHEMA.FUNCTIONS ');
      const explorerProcedures = sql.includes('.INFORMATION_SCHEMA.PROCEDURES ');
      const explorerSchemaRelations = sql.includes('.INFORMATION_SCHEMA.TABLES ') && sql.includes('CLUSTERING_KEY');
      const explorerSchemaColumns = sql.includes('.INFORMATION_SCHEMA.COLUMNS AS SC ');
      const explorerSchemaConstraints = sql.includes('.INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS STC ');
      const explorerSchemaChecks = sql.includes('.INFORMATION_SCHEMA.CHECK_CONSTRAINTS AS SCC ');
      const schemaMappingKind = sql.startsWith('SHOW PRIMARY KEYS IN SCHEMA ') ? 'Primary' : sql.startsWith('SHOW UNIQUE KEYS IN SCHEMA ') ? 'Unique' : sql.startsWith('SHOW IMPORTED KEYS IN SCHEMA ') ? 'Foreign' : null;
      const relations = sql.includes('.INFORMATION_SCHEMA.TABLES ') && !sql.includes('ROW_COUNT::VARCHAR') && !explorerSchemaRelations;
      const columnMetadata = sql.includes('.INFORMATION_SCHEMA.COLUMNS AS C ');
      const describe = sql.startsWith('DESCRIBE TABLE ') || sql.startsWith('DESCRIBE VIEW ') || sql.startsWith('DESCRIBE MATERIALIZED VIEW ');
      const zeroRows = sql.startsWith('SELECT * FROM "DB".') && sql.endsWith(' LIMIT 0');
      const definition = sql.startsWith('SELECT GET_DDL(');
      const constraints = sql.includes('.INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS TC ');
      const checks = sql.includes('.INFORMATION_SCHEMA.CHECK_CONSTRAINTS ') && !explorerSchemaChecks;
      const mappingKind = sql.startsWith('SHOW PRIMARY KEYS IN TABLE ') ? 'Primary' : sql.startsWith('SHOW UNIQUE KEYS IN TABLE ') ? 'Unique' : sql.startsWith('SHOW IMPORTED KEYS IN TABLE ') ? 'Foreign' : null;
      const showRoles = sql === 'SHOW ROLES';
      const showWarehouses = sql === 'SHOW WAREHOUSES';
      const showDatabases = sql === 'SHOW DATABASES';
      const use = /^(USE SCHEMA|USE ROLE|USE WAREHOUSE|USE DATABASE) /u.test(sql);
      const boundary = ['BEGIN TRANSACTION','COMMIT','ROLLBACK'].includes(sql);
      const dml = /^(INSERT INTO|UPDATE|DELETE FROM) /u.test(sql);
      const ddl = /^(CREATE TABLE|ALTER TABLE|DROP TABLE|CREATE VIEW|DROP VIEW) /u.test(sql);
      const readback = sql.startsWith('SELECT CURRENT_USER(),') && switched;
      let names = ['VALUE'], rows = [['1']], parameter;
      if (ddl) {
        ddlSeen = true;
        names = ['status'];
        rows = [[value('ddlStatus', 'Statement executed successfully.')]];
      } else if (dml) {
        dmlSeen = true;
        appendFileSync(file('dml-binds'), JSON.stringify(options.binds) + '\n');
        if (!settings.rejectDml && value('autocommit','true') === 'false') currentTransaction = value('transactionId','implicit-DML-TX');
        names = [sql.startsWith('INSERT') ? 'number of rows inserted' : sql.startsWith('UPDATE') ? 'number of rows updated' : 'number of rows deleted'];
        rows = [[value('affectedRows','9007199254740993')]];
        if (settings.multiJoined !== undefined) { names.push('number of multi-joined rows updated'); rows[0].push(settings.multiJoined); }
      } else if (boundary) {
        if (!settings.rejectBoundary) {
          currentTransaction = sql === 'BEGIN TRANSACTION' ? 'opaque-TX-雪' : null;
          boundarySeen = true;
        }
        names = []; rows = [];
      } else if (showRoles) {
        names = ['created_on', 'name'];
        rows = value('roles', ['ROLE', 'ANALYST']).map(r => ['2026-01-01', r]);
      } else if (showWarehouses) {
        names = ['name', 'state'];
        rows = value('warehouses', ['WH', 'LARGE_WH']).map(w => [w, 'STARTED']);
      } else if (showDatabases) {
        names = ['created_on', 'name'];
        rows = value('databases', ['DB', 'ANALYTICS_DB']).map(d => ['2026-01-01', d]);
      } else if (use) {
        if (sql.startsWith('USE SCHEMA ')) {
          const parts = /^USE SCHEMA "((?:[^"]|"")*)"\."((?:[^"]|"")*)"$/u.exec(sql);
          if (!parts || parts[1].replaceAll('""','"') !== currentDatabase) throw new Error('unexpected fixture switch SQL');
          if (!settings.failUse) { currentSchema = parts[2].replaceAll('""','"'); switched = true; }
        } else if (sql.startsWith('USE ROLE ')) {
          const parts = /^USE ROLE "((?:[^"]|"")*)"$/u.exec(sql);
          if (!parts) throw new Error('unexpected fixture USE ROLE SQL');
          if (!settings.failUse) {
            currentRole = parts[1].replaceAll('""','"');
            if (settings.roleClearsDatabase) { currentDatabase = null; currentSchema = null; }
            switched = true;
          }
        } else if (sql.startsWith('USE WAREHOUSE ')) {
          const parts = /^USE WAREHOUSE "((?:[^"]|"")*)"$/u.exec(sql);
          if (!parts) throw new Error('unexpected fixture USE WAREHOUSE SQL');
          if (!settings.failUse) { currentWarehouse = parts[1].replaceAll('""','"'); switched = true; }
        } else if (sql.startsWith('USE DATABASE ')) {
          const parts = /^USE DATABASE "((?:[^"]|"")*)"$/u.exec(sql);
          if (!parts) throw new Error('unexpected fixture USE DATABASE SQL');
          if (!settings.failUse) {
            currentDatabase = parts[1].replaceAll('""','"');
            currentSchema = value('databaseDefaultSchema', 'PUBLIC');
            switched = true;
          }
        }
        names = []; rows = [];
      } else if (sql.startsWith('SELECT CURRENT_USER(),')) {
        names = ['U','R','D','S','W','ID'];
        rows = [['SERVER_USER',value('role',currentRole),currentDatabase,settings.badReadback ? 'WRONG' : currentSchema,value('warehouse',currentWarehouse),'SESSION']];
      } else if (sql === 'SELECT CURRENT_VERSION()') {
        rows = [['synthetic-version']];
      } else if (unsupportedContext) {
        names = []; rows = [];
      } else if (context) {
        contextReads++;
        names = ['PRINCIPAL','ROLE_NAME','DATABASE_NAME','SCHEMA_NAME','WAREHOUSE_NAME','SESSION_ID','SEARCH_PATH','SECONDARY_ROLES'];
        rows = [[value('principal','SERVER_USER'),(switched || boundarySeen || dmlSeen || ddlSeen) ? value('afterRole',value('role',currentRole)) : value('role',currentRole),(switched || boundarySeen || dmlSeen || ddlSeen) ? value('afterDatabase',value('database',currentDatabase)) : value('database',currentDatabase),(switched || boundarySeen || dmlSeen || ddlSeen) ? value('afterSchema',currentSchema) : value('schema',currentSchema),(switched || boundarySeen || dmlSeen || ddlSeen) ? value('afterWarehouse',value('warehouse',currentWarehouse)) : settings.contextDrift && contextReads >= 2 ? 'CHANGED' : value('warehouse',currentWarehouse),(switched || boundarySeen || dmlSeen || ddlSeen) ? value('afterSession',value('session','SESSION')) : value('session','SESSION'),value('searchPath',currentDatabase && currentSchema ? JSON.stringify([currentDatabase + '.' + currentSchema]) : '[]'),(switched || boundarySeen || dmlSeen || ddlSeen) ? value('afterSecondaryRoles','{"roles":"","value":"ALL"}') : '{"roles":"","value":"ALL"}']];
      } else if (sql.startsWith('SHOW PARAMETERS LIKE ')) {
        parameter = sql.split("'")[1];
        names = ['key','value','default','level'];
        const observed = parameter === 'AUTOCOMMIT' ? (switched || boundarySeen || ddlSeen) ? value('afterAutocommit',value('autocommit','true')) : value(contextReads >= 2 ? 'finalAutocommit' : 'autocommit',value('autocommit','true')) : parameter === 'TRANSACTION_ABORT_ON_ERROR' ? (switched || boundarySeen || ddlSeen) ? value('afterAbortOnError',value('abortOnError','false')) : value('abortOnError','false') : (switched || boundarySeen || ddlSeen) ? value('afterPolicy','false') : 'false';
        rows = [[parameter,observed,observed === 'true' ? 'false' : 'true','ACCOUNT']];
      } else if (sql === 'SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID') {
        transactionReads++;
        names = ['TRANSACTION_ID'];
        rows = [[boundarySeen ? value('afterBoundaryTransactionId',currentTransaction) : (switched || dmlSeen || ddlSeen) ? value('afterTransactionId',value('transactionId',currentTransaction)) : settings.transactionDrift && transactionReads >= 2 ? 'different' : value('transactionId',currentTransaction)]];
      } else if (mappingKind) {
        mappingSeen = true;
        names = mappingKind === 'Foreign' ? ['FK_DATABASE_NAME','FK_SCHEMA_NAME','FK_TABLE_NAME','FK_COLUMN_NAME','KEY_SEQUENCE','FK_NAME','PK_DATABASE_NAME','PK_SCHEMA_NAME','PK_TABLE_NAME','PK_COLUMN_NAME','PK_NAME'] : ['DATABASE_NAME','SCHEMA_NAME','TABLE_NAME','COLUMN_NAME','KEY_SEQUENCE','CONSTRAINT_NAME'];
        const key = mappingKind === 'Primary' ? 'ORDERS_PK' : 'ORDERS_UQ';
        rows = value('mapping' + mappingKind + 'Rows', mappingKind === 'Foreign' ? [
          ['DB','PUBLIC','ORDERS','PARENT_B','2','ORDERS_PARENT_FK','DB',navigation ? navigationSchema : 'PUBLIC',navigation ? navigationTable : 'PARENTS','B','PARENT_PK'],
          ['DB','PUBLIC','ORDERS','PARENT_A','1','ORDERS_PARENT_FK','DB',navigation ? navigationSchema : 'PUBLIC',navigation ? navigationTable : 'PARENTS','A','PARENT_PK'],
        ] : [['DB','PUBLIC','ORDERS','B','2',key],['DB','PUBLIC','ORDERS','A','1',key]]);
        if (settings.mappingColumnName !== undefined) names[0] = settings.mappingColumnName;
      } else if (checks) {
        checksSeen = true;
        appendFileSync(file('check-binds'), JSON.stringify(options.binds) + '\n');
        names = ['CONSTRAINT_CATALOG','CONSTRAINT_SCHEMA','CONSTRAINT_TABLE','CONSTRAINT_NAME','CHECK_CLAUSE'];
        const schema = navigation ? options.binds[1] : 'PUBLIC';
        const table = navigation ? options.binds[2] : 'ORDERS';
        if (navigation && (!['PUBLIC',navigationSchema].includes(schema) || !['ORDERS',navigationTable].includes(table))) throw new Error('unexpected navigation CHECK target');
        rows = value('checkRows', [['DB',schema,table,'POSITIVE_ID','ID > 0'],['DB',schema,table,'NOTES_LENGTH','"NOTES" IS NULL OR\nLENGTH("NOTES") <= 100 /* 雪 */']]);
        if (settings.checkColumnName !== undefined) names[0] = settings.checkColumnName;
      } else if (constraints) {
        constraintsSeen = true;
        appendFileSync(file('constraint-binds'), JSON.stringify(options.binds) + '\n');
        names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','CONSTRAINT_CATALOG','CONSTRAINT_SCHEMA','CONSTRAINT_NAME','CONSTRAINT_TYPE','IS_DEFERRABLE','INITIALLY_DEFERRED','ENFORCED','RELY','COMMENT','FK_CONSTRAINT_CATALOG','FK_CONSTRAINT_SCHEMA','FK_CONSTRAINT_NAME','UNIQUE_CONSTRAINT_CATALOG','UNIQUE_CONSTRAINT_SCHEMA','UNIQUE_CONSTRAINT_NAME','MATCH_OPTION','UPDATE_RULE','DELETE_RULE'];
        const key = (name, kind) => ['DB','PUBLIC','ORDERS','DB','PUBLIC',name,kind,'NO','YES','NO','NO',null,...Array(9).fill(null)];
        const foreign = key('ORDERS_PARENT_FK','FOREIGN KEY');
        foreign.splice(12,9,'DB','PUBLIC','ORDERS_PARENT_FK','DB',navigation ? navigationSchema : 'PUBLIC','PARENT_PK','FULL','NO ACTION','NO ACTION');
        rows = value('constraintRows',[key('ORDERS_PK','PRIMARY KEY'),key('ORDERS_UQ','UNIQUE'),foreign]);
        if (settings.constraintColumnName !== undefined) names[0] = settings.constraintColumnName;
      } else if (definition) {
        definitionSeen = true;
        names = value('definitionColumns',['DDL']);
        const defaultDdl = sql.includes('ACTIVE_ORDERS')
          ? 'create or replace view "DB"."PUBLIC"."ACTIVE_ORDERS" as select ID, NOTES from "DB"."PUBLIC"."ORDERS";'
          : sql.includes('CALC_TAX(NUMBER, FLOAT)')
            ? 'CREATE OR REPLACE FUNCTION "DB"."PUBLIC"."CALC_TAX"(AMOUNT NUMBER, RATE FLOAT)\nRETURNS FLOAT\nLANGUAGE SQL\nAS \'AMOUNT * RATE\';'
            : sql.includes('CALC_TAX(NUMBER)')
              ? 'CREATE OR REPLACE FUNCTION "DB"."PUBLIC"."CALC_TAX"(AMOUNT NUMBER)\nRETURNS NUMBER(18, 2)\nLANGUAGE SQL\nAS \'AMOUNT * 0.1\';'
              : sql.includes('SYNC_ORDERS')
                ? 'CREATE OR REPLACE PROCEDURE "DB"."PUBLIC"."SYNC_ORDERS"(BATCH_ID VARCHAR)\nRETURNS VARCHAR\nLANGUAGE SQL\nAS $$ BEGIN RETURN BATCH_ID; END; $$;'
                : 'create or replace table "DB"."PUBLIC"."ORDERS" (ID NUMBER);';
        rows = value('definitionRows',[[value('definition',defaultDdl)]]);
      } else if (explorerSchemaRelations) {
        names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','TABLE_TYPE','IS_TRANSIENT','IS_TEMPORARY','IS_ICEBERG','IS_DYNAMIC','IS_HYBRID','IS_IMMUTABLE','CLUSTERING_KEY','COMMENT'];
        const schema = predicate('TABLE_SCHEMA') ?? 'PUBLIC';
        rows = value('schemaRelationRows', schema === 'PUBLIC' ? [
          ['DB','PUBLIC','ACTIVE_ORDERS','VIEW','NO','NO','NO','NO','NO','NO',null,'Active view'],
          ['DB','PUBLIC','ORDERS','BASE TABLE','NO','NO','NO','NO','NO','NO','LINEAR(ID)','Order ledger'],
        ] : [
          ['DB',schema,'PARENTS','BASE TABLE','NO','NO','NO','NO','NO','NO',null,null],
        ]);
      } else if (explorerSchemaColumns) {
        names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','COLUMN_NAME','ORDINAL_POSITION','COLUMN_DEFAULT','IS_NULLABLE','DATA_TYPE','CHARACTER_MAXIMUM_LENGTH','NUMERIC_PRECISION','NUMERIC_SCALE','IS_IDENTITY','IDENTITY_START','IDENTITY_INCREMENT','EXPRESSION','KIND'];
        const schema = predicate('TABLE_SCHEMA') ?? 'PUBLIC';
        rows = value('schemaColumnRows', schema === 'PUBLIC' ? [
          ['DB','PUBLIC','ACTIVE_ORDERS','ID','1',null,'YES','NUMBER',null,'38','0','NO',null,null,null,'COLUMN'],
          ['DB','PUBLIC','ACTIVE_ORDERS','NOTES','2',null,'YES','TEXT','100',null,null,'NO',null,null,null,'COLUMN'],
          ['DB','PUBLIC','ORDERS','ID','1','42','NO','NUMBER',null,'38','0','NO',null,null,null,'COLUMN'],
          ['DB','PUBLIC','ORDERS','PARENT_A','2',null,'NO','NUMBER',null,'38','0','NO',null,null,null,'COLUMN'],
          ['DB','PUBLIC','ORDERS','PARENT_B','3',null,'NO','NUMBER',null,'38','0','NO',null,null,null,'COLUMN'],
          ['DB','PUBLIC','ORDERS','NOTES','4',null,'YES','TEXT','100',null,null,'NO',null,null,null,'COLUMN'],
        ] : [
          ['DB',schema,'PARENTS','A','1',null,'NO','NUMBER',null,'38','0','NO',null,null,null,'COLUMN'],
          ['DB',schema,'PARENTS','B','2',null,'NO','NUMBER',null,'38','0','NO',null,null,null,'COLUMN'],
        ]);
      } else if (explorerSchemaConstraints) {
        names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','CONSTRAINT_NAME','CONSTRAINT_TYPE','ENFORCED','RELY','UNIQUE_CONSTRAINT_CATALOG','UNIQUE_CONSTRAINT_SCHEMA','UPDATE_RULE','DELETE_RULE'];
        const schema = options.binds ? options.binds[1] : 'PUBLIC';
        rows = value('schemaConstraintRows', schema === 'PUBLIC' ? [
          ['DB','PUBLIC','ORDERS','ORDERS_PARENT_FK','FOREIGN KEY','NO','YES','DB','OTHER','NO ACTION','NO ACTION'],
          ['DB','PUBLIC','ORDERS','ORDERS_PK','PRIMARY KEY','NO','YES',null,null,null,null],
          ['DB','PUBLIC','ORDERS','ORDERS_UQ','UNIQUE','NO','NO',null,null,null,null],
        ] : [
          ['DB',schema,'PARENTS','PARENT_PK','PRIMARY KEY','NO','YES',null,null,null,null],
        ]);
      } else if (explorerSchemaChecks) {
        names = ['CONSTRAINT_CATALOG','CONSTRAINT_SCHEMA','CONSTRAINT_TABLE','CONSTRAINT_NAME','CHECK_CLAUSE'];
        const schema = options.binds ? options.binds[1] : 'PUBLIC';
        rows = value('schemaCheckRows', schema === 'PUBLIC' ? [
          ['DB','PUBLIC','ORDERS','POSITIVE_ID','ID > 0'],
        ] : []);
      } else if (schemaMappingKind) {
        const schema = sql.includes('"DB"."OTHER"') ? 'OTHER' : 'PUBLIC';
        names = schemaMappingKind === 'Foreign'
          ? ['FK_DATABASE_NAME','FK_SCHEMA_NAME','FK_TABLE_NAME','FK_COLUMN_NAME','KEY_SEQUENCE','FK_NAME','PK_DATABASE_NAME','PK_SCHEMA_NAME','PK_TABLE_NAME','PK_COLUMN_NAME','PK_NAME']
          : ['DATABASE_NAME','SCHEMA_NAME','TABLE_NAME','COLUMN_NAME','KEY_SEQUENCE','CONSTRAINT_NAME'];
        rows = value('schemaMapping' + schemaMappingKind + 'Rows', schema === 'PUBLIC' ? (schemaMappingKind === 'Foreign' ? [
          ['DB','PUBLIC','ORDERS','PARENT_A','1','ORDERS_PARENT_FK','DB','OTHER','PARENTS','A','PARENT_PK'],
          ['DB','PUBLIC','ORDERS','PARENT_B','2','ORDERS_PARENT_FK','DB','OTHER','PARENTS','B','PARENT_PK'],
        ] : schemaMappingKind === 'Primary' ? [
          ['DB','PUBLIC','ORDERS','ID','1','ORDERS_PK'],
        ] : [
          ['DB','PUBLIC','ORDERS','PARENT_A','1','ORDERS_UQ'],
          ['DB','PUBLIC','ORDERS','PARENT_B','2','ORDERS_UQ'],
        ]) : (schemaMappingKind === 'Primary' ? [
          ['DB',schema,'PARENTS','A','1','PARENT_PK'],
          ['DB',schema,'PARENTS','B','2','PARENT_PK'],
        ] : []));
      } else if (schemas) {
        names = ['CATALOG_NAME','SCHEMA_NAME'];
        rows = value('schemas',navigation ? ['PUBLIC',navigationSchema] : ['OTHER','PUBLIC']).map(name => [currentDatabase ?? 'DB',name]);
      } else if (explorerTables) {
        names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','TABLE_TYPE','ROW_COUNT','BYTES','COMMENT'];
        const schema = predicate('TABLE_SCHEMA') ?? 'PUBLIC';
        rows = value('explorerTableRows', schema === 'PUBLIC' ? [['DB','PUBLIC','ORDERS','BASE TABLE','42','8192','Order ledger'],['DB','PUBLIC','ACTIVE_ORDERS','VIEW',null,null,'Active view']] : [['DB',schema,'Mixed.Name','BASE TABLE','5','1024',null]]);
      } else if (explorerExactTable) {
        names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','TABLE_TYPE','IS_TRANSIENT','IS_TEMPORARY','IS_ICEBERG','IS_DYNAMIC','IS_HYBRID','IS_IMMUTABLE','ROW_COUNT','BYTES','COMMENT'];
        const schema = predicate('TABLE_SCHEMA') ?? 'PUBLIC';
        const table = predicate('TABLE_NAME') ?? 'ORDERS';
        rows = value('explorerExactTableRows', table === 'ACTIVE_ORDERS' ? [['DB',schema,'ACTIVE_ORDERS','VIEW','NO','NO','NO','NO','NO','NO',null,null,'Active view']] : [['DB',schema,table,value('tableType','BASE TABLE'),'NO','NO','NO','NO','NO','NO','42','8192','Order ledger']]);
      } else if (explorerSequences) {
        names = ['SEQUENCE_CATALOG','SEQUENCE_SCHEMA','SEQUENCE_NAME','DATA_TYPE','START_VALUE','MINIMUM_VALUE','MAXIMUM_VALUE','INCREMENT_BY','CYCLE_OPTION','COMMENT'];
        const schema = predicate('SEQUENCE_SCHEMA') ?? 'PUBLIC';
        const seqName = predicate('SEQUENCE_NAME');
        rows = value('explorerSequenceRows', schema === 'PUBLIC' && (!seqName || seqName === 'ORDER_SEQ') ? [['DB','PUBLIC','ORDER_SEQ','NUMBER','1','1','999999999','1','NO','Order sequence']] : []);
      } else if (explorerFunctions) {
        names = ['FUNCTION_CATALOG','FUNCTION_SCHEMA','FUNCTION_NAME','ARGUMENT_SIGNATURE','DATA_TYPE','FUNCTION_LANGUAGE','IS_VOLATILE','COMMENT'];
        const schema = predicate('FUNCTION_SCHEMA') ?? 'PUBLIC';
        const fnName = predicate('FUNCTION_NAME');
        rows = value('explorerFunctionRows', schema === 'PUBLIC' && (!fnName || fnName === 'CALC_TAX') ? [['DB','PUBLIC','CALC_TAX','(AMOUNT NUMBER)','NUMBER(18,2)','SQL','NO','Tax helper'],['DB','PUBLIC','CALC_TAX','(AMOUNT NUMBER, RATE FLOAT)','FLOAT','SQL','YES','Overloaded tax helper']] : []);
      } else if (explorerProcedures) {
        names = ['PROCEDURE_CATALOG','PROCEDURE_SCHEMA','PROCEDURE_NAME','ARGUMENT_SIGNATURE','DATA_TYPE','PROCEDURE_LANGUAGE','COMMENT'];
        const schema = predicate('PROCEDURE_SCHEMA') ?? 'PUBLIC';
        const procName = predicate('PROCEDURE_NAME');
        rows = value('explorerProcedureRows', schema === 'PUBLIC' && (!procName || procName === 'SYNC_ORDERS') ? [['DB','PUBLIC','SYNC_ORDERS','(BATCH_ID VARCHAR)','VARCHAR','SQL','Sync batch']] : []);
      } else if (relations) {
        names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','TABLE_TYPE','IS_TRANSIENT','IS_TEMPORARY','IS_ICEBERG','IS_DYNAMIC','IS_HYBRID','IS_IMMUTABLE','CREATED','LAST_DDL'];
        const schema = sql.includes("TABLE_SCHEMA = 'OTHER'") ? 'OTHER' : 'PUBLIC';
        const table = schema === 'OTHER' ? 'Mixed.Name' : value('tableName','ORDERS');
        const kind = value('tableType','BASE TABLE');
        const transient = value('isTransient','NO');
        const lastDdl = checksSeen ? value('afterCheckLastDdl',value('lastDdl','last-ddl')) : mappingSeen ? value('afterMappingLastDdl',value('lastDdl','last-ddl')) : constraintsSeen ? value('afterConstraintsLastDdl',value('lastDdl','last-ddl')) : definitionSeen ? value('afterDefinitionLastDdl',value('lastDdl','last-ddl')) : ddlSeen ? value('afterLastDdl','last-ddl') : value('lastDdl','last-ddl');
        rows = settings.emptyRelations ? [] : [['DB',schema,table,kind,transient,'NO','NO',value('isDynamic','NO'),'NO','NO','created',lastDdl]];
        if (navigation) {
          const exactSchema = predicate('TABLE_SCHEMA');
          const exactTable = predicate('TABLE_NAME');
          if (!['PUBLIC', navigationSchema].includes(exactSchema)) throw new Error('unexpected navigation schema');
          rows = settings.emptyRelations ? [] : ['ORDERS',navigationTable]
            .filter(name => exactTable === null || name === exactTable)
            .map(name => ['DB',exactSchema,name,kind,transient,'NO','NO',value('isDynamic','NO'),'NO','NO','created',lastDdl]);
        }

      } else if (columnMetadata) {
        names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','COLUMN_NAME','ORDINAL_POSITION','COLUMN_DEFAULT','IS_NULLABLE','DATA_TYPE','DATA_TYPE_ALIAS','CHARACTER_MAXIMUM_LENGTH','CHARACTER_OCTET_LENGTH','NUMERIC_PRECISION','NUMERIC_PRECISION_RADIX','NUMERIC_SCALE','IS_IDENTITY','IDENTITY_GENERATION','IDENTITY_START','IDENTITY_INCREMENT','IDENTITY_ORDERED','COMMENT','EXPRESSION','KIND'];
        const table = Object.hasOwn(settings,'tableName') ? settings.tableName : (predicate('TABLE_NAME') ?? 'ORDERS');
        rows = [['DB','PUBLIC',table,'ID','1','42','NO','NUMBER','BIGINT',null,null,'38','10','0','NO',null,null,null,null,'identifier',null,'COLUMN'],['DB','PUBLIC',table,'NOTES','3',null,'YES','TEXT',null,'100','400',null,null,null,'NO',null,null,null,null,null,null,'COLUMN']];
        if (navigation) {
          const exactSchema = predicate('TABLE_SCHEMA'), exactTable = predicate('TABLE_NAME');
          if (!['PUBLIC',navigationSchema].includes(exactSchema) || !['ORDERS',navigationTable].includes(exactTable)) throw new Error('unexpected navigation columns target');
          rows = rows.map(row => ['DB',exactSchema,exactTable,...row.slice(3)]);
        }

      } else if (describe) {
        names = ['name','type','kind','null?','default','expression','comment'];
        rows = [['ID','NUMBER(38,0)',value('descriptorKind','COLUMN'),'N','42',null,'identifier'],['NOTES','VARCHAR(100)','COLUMN','Y',null,null,null]];
      } else if (zeroRows) {
        names = ['ID','NOTES'];
        rows = [];
      } else if (sql !== 'SELECT 1') {
        throw new Error('unexpected fixture SQL');
      }
      let cancelled = false;
      const statement = {
        getRequestId: () => uuid(id), getQueryId: () => uuid(id + 1000),
        getColumns: () => names.map(name => ({ getName: () => settings.badDmlName && dml ? 'unexpected' : name, getType: () => checks ? value('checkType','TEXT') : mappingKind ? value('mappingType','TEXT') : constraints ? value('constraintType','TEXT') : definition ? value('definitionType','TEXT') : (zeroRows && name === 'ID') ? 'FIXED' : dml ? value('dmlType','FIXED') : settings.wrongTransactionType && name === 'TRANSACTION_ID' ? 'FIXED' : 'TEXT' })),
        streamRows() {
          if (checks && settings.holdChecks) {
            writeFileSync(file('held-checks'), '');
            return Readable.from((async function*() {
              while (!existsSync(file('release')) && !cancelled) await new Promise(resolve => setTimeout(resolve,5));
              if (cancelled) throw { sqlState: '57014' };
              yield* rows;
            })());
          }
          if (mappingKind && settings.holdMappings) {
            writeFileSync(file('held-mappings'), '');
            return Readable.from((async function*() {
              while (!existsSync(file('release')) && !cancelled) await new Promise(resolve => setTimeout(resolve,5));
              if (cancelled) throw { sqlState: '57014' };
              yield* rows;
            })());
          }
          if (constraints && settings.holdConstraints) {
            writeFileSync(file('held-constraints'), '');
            return Readable.from((async function*() {
              while (!existsSync(file('release')) && !cancelled) await new Promise(resolve => setTimeout(resolve,5));
              if (cancelled) throw { sqlState: '57014' };
              yield* rows;
            })());
          }
          if (definition && settings.holdDefinition) {
            writeFileSync(file('held-definition'), '');
            return Readable.from((async function*() {
              while (!existsSync(file('release')) && !cancelled) await new Promise(resolve => setTimeout(resolve,5));
              if (cancelled) throw { sqlState: '57014' };
              yield* rows;
            })());
          }
          if (dml && settings.protocolDml || ddl && settings.protocolDdl) {
            send.flush().then(() => process.stdout.write(Buffer.alloc(4)));
            return Readable.from((async function*() { await new Promise(() => {}); })());
          }
          if (dml && settings.holdDmlDrain || context && dmlSeen && settings.holdDmlObserver || ddl && settings.holdDdlDrain || context && ddlSeen && settings.holdDdlObserver) {
            writeFileSync(file(ddl ? 'held-ddl-drain' : context && ddlSeen ? 'held-ddl-observer' : dml ? 'held-dml-drain' : 'held-dml-observer'), '');
            return Readable.from((async function*() {
              while (!existsSync(file('release')) && !cancelled) await new Promise(resolve => setTimeout(resolve,5));
              if (cancelled) throw { sqlState: '57014' };
              yield* rows;
            })());
          }
          if (boundary && settings.protocolBoundary) {
            send.flush().then(() => process.stdout.write(Buffer.alloc(4)));
            return Readable.from((async function*() { await new Promise(() => {}); })());
          }
          if (context && (settings.holdContext && contextReads === 1 || (switched || boundarySeen) && settings.holdAfterContext) || schemas && settings.holdSchemas || boundary && settings.holdBoundaryDrain) {
            writeFileSync(file(boundary ? 'held-boundary-drain' : (switched || boundarySeen) && context ? 'held-after-context' : schemas ? 'held-schemas' : 'held'), '');
            return Readable.from((async function*() {
              while (!existsSync(file('release')) && !cancelled) await new Promise(resolve => setTimeout(resolve,5));
              if (cancelled) throw { sqlState: '57014' };
              yield* rows;
            })());
          }
          return Readable.from(rows);
        },
        cancel(callback) { cancelled = true; callback(null); },
      };
      const denied = checks && settings.rejectChecks || mappingKind && settings.rejectMappings || constraints && settings.rejectConstraints || definition && settings.rejectDefinition || definition && Array.isArray(settings.failGetDdlFor) && settings.failGetDdlFor.some(token => sql.includes(token)) || ddl && settings.rejectDdl || ddlSeen && settings.failDdlObserver === parameter && parameter !== undefined || dml && settings.rejectDml || dmlSeen && settings.failDmlObserver === parameter && parameter !== undefined || boundary && settings.rejectBoundary || boundarySeen && settings.failBoundaryObserver === parameter && parameter !== undefined || unsupportedContext || settings.failParameter === parameter && parameter !== undefined || switched && settings.failFinalParameter === parameter && parameter !== undefined || settings.failSchemas && schemas || settings.failUse && use || settings.failReadback && readback;
      setImmediate(() => {
        if (ddl && settings.holdDdl) { writeFileSync(file('held-ddl'), ''); return; }
        if (dml && settings.holdDml) { writeFileSync(file('held-dml'), ''); return; }
        if (boundary && settings.holdBoundary || use && settings.holdUse || readback && settings.holdReadback) {
          writeFileSync(file(boundary ? 'held-boundary' : use ? 'held-use' : 'held-readback'), '');
          return;
        }
        options.complete(denied ? {sqlState:unsupportedContext ? '42883' : '42501'} : null, statement);
      });
      return statement;
    },
  };
}};
const send = writer(process.stdout.write.bind(process.stdout), () => process.exit(2));
let stopping = false;
const retire = () => { if (stopping) return; stopping = true; send.flush().then(() => process.exit(0)); };
const helper = new Helper(sdk, send, retire, { clientConfigFile:'/nonexistent/synthetic', closeTimeoutMs:200 });
const decoder = new Decoder();
process.stdin.on('data', chunk => { try { for (const frame of decoder.push(chunk)) {
  if (frame.op === 'query') appendFileSync(file('query-specs'), JSON.stringify(frame.payload) + '\n');
  helper.receive(frame);
} } catch { retire(); } });
process.stdin.on('end', retire);
"#;

    struct Harness {
        driver: Arc<SnowflakeExternal>,
        cleanup: CleanupObserver,
        scripts: tempfile::TempDir,
    }

    impl Harness {
        async fn open() -> Self {
            Self::open_mode(SafeMode::ReadOnly).await
        }

        async fn open_mode(safe_mode: SafeMode) -> Self {
            Self::open_policy(safe_mode, false).await
        }

        async fn open_policy(safe_mode: SafeMode, reviewed_dml: bool) -> Self {
            Self::open_policy_with(safe_mode, reviewed_dml, false).await
        }

        async fn open_ddl_policy(safe_mode: SafeMode, reviewed_ddl: bool) -> Self {
            Self::open_policy_with(safe_mode, false, reviewed_ddl).await
        }

        async fn open_policy_with(
            safe_mode: SafeMode,
            reviewed_dml: bool,
            reviewed_ddl: bool,
        ) -> Self {
            Self::open_policy_and_observer(safe_mode, reviewed_dml, reviewed_ddl, None).await
        }

        async fn open_policy_and_observer(
            safe_mode: SafeMode,
            reviewed_dml: bool,
            reviewed_ddl: bool,
            metadata_observer: Option<MetadataAttachment>,
        ) -> Self {
            Self::open_policy_and_attachments(
                safe_mode,
                reviewed_dml,
                reviewed_ddl,
                metadata_observer,
                None,
            )
            .await
        }

        async fn open_policy_and_attachments(
            safe_mode: SafeMode,
            reviewed_dml: bool,
            reviewed_ddl: bool,
            metadata_observer: Option<MetadataAttachment>,
            transport: Option<
                crate::drivers::snowflake_qualification_transport::TransportAttachment,
            >,
        ) -> Self {
            Self::open_fixture(
                safe_mode,
                reviewed_dml,
                reviewed_ddl,
                false,
                metadata_observer,
                transport,
                FIXTURE,
            )
            .await
        }

        async fn open_fixture(
            safe_mode: SafeMode,
            reviewed_dml: bool,
            reviewed_ddl: bool,
            reviewed_console: bool,
            metadata_observer: Option<MetadataAttachment>,
            transport: Option<
                crate::drivers::snowflake_qualification_transport::TransportAttachment,
            >,
            fixture: &str,
        ) -> Self {
            let scripts = tempfile::tempdir().unwrap();
            let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_owned();
            let source_url = |name| {
                url::Url::from_file_path(repo.join("snowflake-helper/src").join(name))
                    .unwrap()
                    .to_string()
            };
            let script = fixture
                .replace(
                    "HELPER_URL",
                    &serde_json::to_string(&source_url("helper.mjs")).unwrap(),
                )
                .replace(
                    "FRAMING_URL",
                    &serde_json::to_string(&source_url("framing.mjs")).unwrap(),
                );
            let path = scripts.path().join("synthetic-sdk.mjs");
            std::fs::write(&path, script).unwrap();
            let node = PathBuf::from(
                std::env::var_os("JOINERY_TEST_NODE").expect("explicit JOINERY_TEST_NODE required"),
            );
            assert!(node.is_absolute() && node.is_file());
            let mut launch = Launch::new(
                node,
                Some(path),
                scripts.path().to_owned(),
                tempfile::tempdir().unwrap(),
            )
            .unwrap();
            if let Some(transport) = transport {
                launch.attach_transport(transport).unwrap();
            }
            let cleanup = launch.cleanup_observer();
            let profile = ConnectionProfile {
                id: new_id(),
                name: "Synthetic SDK observer".into(),
                group_id: "test".into(),
                engine: EngineKind::Snowflake,
                host: "org-account.snowflakecomputing.com".into(),
                port: 443,
                database: "DB".into(),
                username: "different-login@example.test".into(),
                environment: Environment::Local,
                safe_mode,
                tls_mode: TlsMode::VerifyFull,
                ca_cert_path: None,
                ssh: None,
                remember_password: false,
                store_query_text: false,
                store_drafts: false,
                snowflake: Some(SnowflakeConfig {
                    authentication: SnowflakeAuthentication::ExternalBrowser,
                    reviewed_dml,
                    reviewed_ddl,
                    reviewed_console,
                    reviewed_console_timeout_seconds: None,
                    role: "ROLE".into(),
                    warehouse: "WH".into(),
                    schema: "PUBLIC".into(),
                }),
            };
            let (driver, _) = tokio::time::timeout(
                Duration::from_secs(10),
                SnowflakeExternal::open(
                    &profile,
                    "observer-session",
                    launch,
                    ExternalBrowserOptions {
                        metadata_observer,
                        ..Default::default()
                    },
                ),
            )
            .await
            .unwrap()
            .unwrap();
            Self {
                driver,
                cleanup,
                scripts,
            }
        }

        fn settings(&self, mut value: Value) {
            value
                .as_object_mut()
                .unwrap()
                .insert("generation".into(), json!(new_id()));
            std::fs::write(
                self.scripts.path().join("settings"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
        }

        fn queries(&self) -> Vec<String> {
            std::fs::read_to_string(self.scripts.path().join("queries"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }

        async fn prepare_create_ddl(&self) -> SnowflakeDdlRequest {
            let action = SnowflakeDdlAction::CreateTable {
                database: "DB".into(),
                schema: "PUBLIC".into(),
                name: "NEW_ORDERS".into(),
                columns: vec![SnowflakeDdlColumnDraft {
                    name: "ID".into(),
                    data_type: "NUMBER(38, 0)".into(),
                    nullable: false,
                }],
            };
            let preflight = self
                .driver
                .preflight_snowflake_ddl(
                    "observer-session",
                    &SnowflakeDdlPreflightRequest {
                        session_id: "observer-session".into(),
                        action: action.clone(),
                    },
                )
                .await
                .unwrap();
            SnowflakeDdlRequest {
                revision: preflight.revision,
                action,
                sql: preflight.sql,
            }
        }

        async fn wait_until(&self, ready: impl Fn() -> bool) {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !ready() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        }

        async fn finish(self) {
            self.driver.close();
            let report = self.cleanup.wait(Duration::from_secs(10)).await.unwrap();
            assert!(report.reaped);
            assert!(report.temporary_root_removed);
            assert_eq!(
                std::fs::read_to_string(self.scripts.path().join("connections")).unwrap(),
                "1"
            );
            // Mapping tests use only this retained fixture target. Admit its
            // three complete closed factory strings, never arbitrary SHOW SQL.
            let mapping_reads = metadata::mapping_reads(&browsing::BrowseRelation {
                database: "DB".into(),
                table: TableRef {
                    schema: "PUBLIC".into(),
                    name: "ORDERS".into(),
                },
                table_type: "BASE TABLE".into(),
                traits: RelationTraits {
                    transient: Some(false),
                    temporary: Some(false),
                    iceberg: Some(false),
                    dynamic: Some(false),
                    hybrid: Some(false),
                    immutable: Some(false),
                },
                availability: RelationAvailability::Available,
                created: None,
                last_ddl: None,
            })
            .unwrap();
            let sample_ctx = metadata::LiveContext {
                principal: "SERVER_USER".into(),
                role: Some("ROLE".into()),
                database: Some("DB".into()),
                schema: Some("PUBLIC".into()),
                warehouse: Some("WH".into()),
                session_id: "SESSION".into(),
                search_path: Vec::new(),
                secondary_roles: metadata::SecondaryRoles {
                    roles: String::new(),
                    value: "ALL".into(),
                },
                quoted_identifiers_ignore_case: false,
            };
            let schema_mapping_reads: Vec<_> = ["PUBLIC", "OTHER"]
                .into_iter()
                .flat_map(|s| metadata::explorer_schema_mapping_reads(&sample_ctx, s).unwrap())
                .collect();
            // Switch tests separately assert their one exact quoted USE.
            assert!(self.queries().iter().all(|sql| {
                sql.starts_with("SELECT ")
                    || [
                        metadata::QUOTED_IDENTIFIER_POLICY_SQL,
                        metadata::SHOW_ROLES_SQL,
                        metadata::SHOW_WAREHOUSES_SQL,
                        metadata::SHOW_DATABASES_SQL,
                        session::AUTOCOMMIT_SQL,
                        session::ABORT_ON_ERROR_SQL,
                    ]
                    .contains(&sql.as_str())
                    || mapping_reads.iter().any(|read| sql == read.sql())
                    || schema_mapping_reads.iter().any(|read| sql == read.sql())
                    || [
                        "USE SCHEMA \"",
                        "USE ROLE \"",
                        "USE WAREHOUSE \"",
                        "USE DATABASE \"",
                    ]
                    .iter()
                    .any(|prefix| sql.starts_with(prefix))
                    || sql.starts_with("DESCRIBE TABLE ")
                    || sql.starts_with("DESCRIBE VIEW ")
                    || sql.starts_with("DESCRIBE MATERIALIZED VIEW ")
                    || ["BEGIN TRANSACTION", "COMMIT", "ROLLBACK"].contains(&sql.as_str())
                    || [
                        "INSERT INTO ",
                        "UPDATE ",
                        "DELETE FROM ",
                        "CREATE TABLE ",
                        "ALTER TABLE ",
                        "DROP TABLE ",
                        "CREATE VIEW ",
                        "DROP VIEW ",
                    ]
                    .iter()
                    .any(|prefix| sql.starts_with(prefix))
            }));
        }
    }

    fn sample_sql() -> Vec<String> {
        [
            metadata::CONTEXT_SQL,
            metadata::QUOTED_IDENTIFIER_POLICY_SQL,
            session::AUTOCOMMIT_SQL,
            session::ABORT_ON_ERROR_SQL,
            session::CURRENT_TRANSACTION_SQL,
        ]
        .map(str::to_owned)
        .to_vec()
    }

    #[tokio::test]
    async fn context_normalization_routes_exact_text_sql_and_rejects_old_expression() {
        const EXPECTED: &str = "SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES";
        let h = Harness::open().await;
        let baseline = h.queries().len();
        let lane = h.driver.lane.lock().await;
        let observed = h.driver.live_metadata_context().await.unwrap();
        assert_eq!(observed.search_path[0].raw, "DB.PUBLIC");
        assert_eq!(observed.secondary_roles.roles, "");
        assert_eq!(observed.secondary_roles.value, "ALL");
        assert_eq!(
            h.queries()[baseline..],
            [EXPECTED, metadata::QUOTED_IDENTIFIER_POLICY_SQL]
        );
        // An actual helper child passes the exact old SQL to the synthetic SDK,
        // whose explicit rejection must remain a definitive database outcome.
        let old = EXPECTED.replace(
            "TO_VARCHAR(CURRENT_SCHEMAS())",
            "TO_JSON(CURRENT_SCHEMAS())",
        );
        let error = h
            .driver
            .metadata_read(&old, 1, metadata::CONTEXT_MAX_BYTES)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Database);
        assert_eq!(h.queries()[baseline + 2..], [old]);
        drop(lane);
        h.finish().await;
    }

    #[tokio::test]
    async fn transaction_observation_issues_pure_one_use_authority() {
        let h = Harness::open().await;
        let state = h.driver.session_state("observer-session").await.unwrap();
        let Some(TransactionControlAuthority::Observed {
            revision,
            allowed_actions,
        }) = state.transaction_control_authority
        else {
            panic!("a complete stable observation must issue transaction authority");
        };
        assert!(allowed_actions.contains(&ExplicitTransactionAction::Rollback));
        let request = TransactionControlRequest {
            action: ExplicitTransactionAction::Rollback,
            revision,
        };
        let baseline = h.queries().len();
        h.driver.validate_transaction_control(&request).unwrap();
        h.driver.validate_transaction_control(&request).unwrap();
        assert_eq!(
            h.queries().len(),
            baseline,
            "validation neither queries nor consumes"
        );
        h.finish().await;
    }

    fn transaction_request(
        state: &SessionRuntimeState,
        action: ExplicitTransactionAction,
    ) -> TransactionControlRequest {
        let revision = match state.transaction_control_authority.as_ref().unwrap() {
            TransactionControlAuthority::Observed { revision, .. }
            | TransactionControlAuthority::RollbackOnly { revision } => revision.clone(),
        };
        TransactionControlRequest { action, revision }
    }

    fn dml_request(state: &SessionRuntimeState, sql: &str) -> SnowflakeDmlRequest {
        SnowflakeDmlRequest {
            sql: sql.into(),
            parameters: vec![],
            revision: transaction_request(state, ExplicitTransactionAction::Rollback).revision,
        }
    }

    fn dml_count(h: &Harness) -> usize {
        h.queries()
            .iter()
            .filter(|sql| {
                ["INSERT INTO ", "UPDATE ", "DELETE FROM "]
                    .iter()
                    .any(|prefix| sql.starts_with(prefix))
            })
            .count()
    }

    #[tokio::test]
    async fn dml_uses_exact_eleven_statements_and_decimal_counts_not_status_rows() {
        for (sql, settings) in [
            ("INSERT INTO DB.PUBLIC.T (V) VALUES (1)", json!({})),
            (
                "UPDATE DB.PUBLIC.T SET V = 2 WHERE V = 1",
                json!({"multiJoined":"7"}),
            ),
            (
                "DELETE FROM DB.PUBLIC.T WHERE V = 1",
                json!({"affectedRows":"0"}),
            ),
        ] {
            let h = Harness::open_policy(SafeMode::Confirm, true).await;
            h.settings(settings.clone());
            let state = h.driver.session_state("observer-session").await.unwrap();
            let request = dml_request(&state, sql);
            let baseline = h.queries().len();
            h.driver.validate_snowflake_dml(&request).unwrap();
            h.driver.validate_snowflake_dml(&request).unwrap();
            assert_eq!(h.queries().len(), baseline);
            let result = h
                .driver
                .snowflake_dml("observer-session", &request)
                .await
                .unwrap();
            assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
            assert_eq!(result.summary.status, SnowflakeDmlSummaryStatus::Validated);
            assert_eq!(
                result.summary.affected_rows.as_deref(),
                Some(if settings.get("affectedRows").is_some() {
                    "0"
                } else {
                    "9007199254740993"
                })
            );
            assert_eq!(
                result.summary.multi_joined_rows.as_deref(),
                settings.get("multiJoined").and_then(Value::as_str)
            );
            assert!(!result.retired && result.error.is_none());
            let mut expected = sample_sql();
            expected.push(sql.into());
            expected.extend(sample_sql());
            assert_eq!(h.queries()[baseline..], expected);
            assert!(
                h.driver
                    .snowflake_dml("observer-session", &request)
                    .await
                    .is_err()
            );
            assert_eq!(dml_count(&h), 1);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn dml_recompiles_bound_original_and_preserves_scalar_precision() {
        let h = Harness::open_policy(SafeMode::Confirm, true).await;
        let state = h.driver.session_state("observer-session").await.unwrap();
        let mut request = dml_request(&state, "INSERT INTO DB.PUBLIC.T (V) VALUES (:value)");
        request.parameters.push(QueryParamInput {
            key: "value".into(),
            kind: QueryParamKind::Integer,
            value: Some("9007199254740993".into()),
        });
        let result = h
            .driver
            .snowflake_dml("observer-session", &request)
            .await
            .unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
        let canonical =
            crate::safety::snowflake_dml::compile(&request.sql, Some(&request.parameters), "DB")
                .unwrap();
        assert!(
            h.queries()
                .contains(&canonical.bound_statement().rewritten_sql)
        );
        let binds = std::fs::read_to_string(h.scripts.path().join("dml-binds")).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(binds.trim()).unwrap(),
            json!(["9007199254740993"])
        );
        h.finish().await;
    }

    #[tokio::test]
    async fn dml_profile_grammar_revision_and_rollback_only_denials_submit_nothing() {
        for (safe_mode, enabled) in [(SafeMode::ReadOnly, false), (SafeMode::Confirm, false)] {
            let h = Harness::open_policy(safe_mode, enabled).await;
            let state = h.driver.session_state("observer-session").await.unwrap();
            let request = dml_request(&state, "DELETE FROM DB.PUBLIC.T WHERE V = 1");
            let baseline = h.queries().len();
            assert_eq!(
                h.driver
                    .snowflake_dml("observer-session", &request)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::PolicyBlocked
            );
            assert_eq!(h.queries().len(), baseline);
            h.finish().await;
        }
        let h = Harness::open_policy(SafeMode::Confirm, true).await;
        let state = h.driver.session_state("observer-session").await.unwrap();
        let baseline = h.queries().len();
        let mut sink = ScalarSink::default();
        for kind in [StatementKind::Read, StatementKind::Write] {
            assert!(
                h.driver
                    .execute("DELETE FROM DB.PUBLIC.T WHERE V = 1", kind, &mut sink)
                    .await
                    .is_err()
            );
        }
        for sql in [
            "DELETE FROM T",
            "DELETE FROM OTHER.PUBLIC.T",
            "DELETE FROM DB.PUBLIC.T; DELETE FROM DB.PUBLIC.T",
            "UPDATE DB.PUBLIC.T SET V = EXTERNAL_FUNCTION(V)",
        ] {
            assert!(
                h.driver
                    .snowflake_dml("observer-session", &dml_request(&state, sql))
                    .await
                    .is_err()
            );
        }
        let mut request = dml_request(&state, "DELETE FROM DB.PUBLIC.T WHERE V = 1");
        request.revision = "foreign".into();
        assert!(
            h.driver
                .snowflake_dml("observer-session", &request)
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), baseline);
        let generation = h.driver.transaction_authority.lock().generation.clone();
        h.driver.restrict_transaction_recovery(&generation);
        let revision = match h.driver.transaction_authority().unwrap() {
            TransactionControlAuthority::RollbackOnly { revision } => revision,
            _ => panic!("restricted authority"),
        };
        request.revision = revision;
        assert!(
            h.driver
                .snowflake_dml("observer-session", &request)
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), baseline);
        h.finish().await;
    }

    #[tokio::test]
    async fn dml_accepts_pending_and_implicit_transactions_without_boundaries() {
        for settings in [
            json!({"transactionId":"pending-opaque-TX"}),
            json!({"autocommit":"false"}),
        ] {
            let h = Harness::open_policy(SafeMode::Confirm, true).await;
            h.settings(settings.clone());
            let state = h.driver.session_state("observer-session").await.unwrap();
            let request = dml_request(&state, "UPDATE DB.PUBLIC.T SET V = 2 WHERE V = 1");
            let baseline = h.queries().len();
            let result = h
                .driver
                .snowflake_dml("observer-session", &request)
                .await
                .unwrap();
            assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
            assert_eq!(
                result.state.transaction_state,
                TransactionState::InTransaction
            );
            let expected = if settings.get("transactionId").is_some() {
                "pending-opaque-TX"
            } else {
                "implicit-DML-TX"
            };
            assert!(
                matches!(result.state.transaction_observation, Some(TransactionObservation::Observed { transaction_id: Some(ref id), usability: TransactionUsability::Unknown, .. }) if id == expected)
            );
            assert_eq!(h.queries().len() - baseline, 11);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn dml_ack_and_count_survive_denied_final_observer_with_rollback_only_recovery() {
        let h = Harness::open_policy(SafeMode::Confirm, true).await;
        h.settings(json!({"failDmlObserver":"AUTOCOMMIT"}));
        let state = h.driver.session_state("observer-session").await.unwrap();
        let request = dml_request(&state, "DELETE FROM DB.PUBLIC.T WHERE V = 1");
        let result = h
            .driver
            .snowflake_dml("observer-session", &request)
            .await
            .unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
        assert_eq!(
            result.summary.affected_rows.as_deref(),
            Some("9007199254740993")
        );
        assert!(!result.retired);
        assert_eq!(result.error.unwrap().code, ErrorCode::Database);
        assert!(matches!(
            result.state.transaction_control_authority,
            Some(TransactionControlAuthority::RollbackOnly { .. })
        ));
        assert!(
            h.driver
                .snowflake_dml(
                    "observer-session",
                    &dml_request(&result.state, &request.sql)
                )
                .await
                .is_err()
        );
        h.settings(json!({}));
        let rollback = transaction_request(&result.state, ExplicitTransactionAction::Rollback);
        let baseline = h.queries().len();
        h.driver
            .transaction_control("observer-session", &rollback)
            .await
            .unwrap();
        assert_eq!(h.queries()[baseline], "ROLLBACK");
        assert_eq!(h.queries().len() - baseline, 6);
        h.finish().await;
    }

    #[tokio::test]
    async fn dml_database_rejection_is_definitive_and_unknown_delivery_never_replays() {
        let h = Harness::open_policy(SafeMode::Confirm, true).await;
        h.settings(json!({"rejectDml":true}));
        let state = h.driver.session_state("observer-session").await.unwrap();
        let request = dml_request(&state, "DELETE FROM DB.PUBLIC.T WHERE V = 1");
        let baseline = h.queries().len();
        let result = h
            .driver
            .snowflake_dml("observer-session", &request)
            .await
            .unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Rejected);
        assert_eq!(result.summary, SnowflakeDmlSummary::unavailable());
        assert!(!result.retired);
        assert_eq!(h.queries().len() - baseline, 6);
        assert!(matches!(
            result.state.transaction_control_authority,
            Some(TransactionControlAuthority::RollbackOnly { .. })
        ));
        h.finish().await;

        for (settings, barrier, expected) in [
            (
                json!({"holdDml":true}),
                "held-dml",
                SnowflakeDmlOutcome::Unknown,
            ),
            (
                json!({"holdDmlDrain":true}),
                "held-dml-drain",
                SnowflakeDmlOutcome::Succeeded,
            ),
            (
                json!({"holdDmlObserver":true}),
                "held-dml-observer",
                SnowflakeDmlOutcome::Succeeded,
            ),
        ] {
            let h = Harness::open_policy(SafeMode::Confirm, true).await;
            h.settings(settings);
            let state = h.driver.session_state("observer-session").await.unwrap();
            let request = dml_request(&state, "DELETE FROM DB.PUBLIC.T WHERE V = 1");
            let driver = h.driver.clone();
            let task =
                tokio::spawn(
                    async move { driver.snowflake_dml("observer-session", &request).await },
                );
            h.wait_until(|| h.scripts.path().join(barrier).exists())
                .await;
            let _ = h.driver.cancel().await;
            let result = task.await.unwrap().unwrap();
            assert_eq!(result.statement_outcome, expected);
            assert!(result.retired && h.driver.is_closed());
            if barrier == "held-dml-observer" {
                assert_eq!(
                    result.summary.affected_rows.as_deref(),
                    Some("9007199254740993")
                );
            }
            assert_eq!(dml_count(&h), 1);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn dml_unavailable_summary_does_not_erase_success() {
        for settings in [
            json!({"dmlType":"TEXT"}),
            json!({"badDmlName":true}),
            json!({"affectedRows":"-1"}),
            json!({"affectedRows":null}),
        ] {
            let h = Harness::open_policy(SafeMode::Confirm, true).await;
            h.settings(settings);
            let state = h.driver.session_state("observer-session").await.unwrap();
            let request = dml_request(&state, "DELETE FROM DB.PUBLIC.T WHERE V = 1");
            let result = h
                .driver
                .snowflake_dml("observer-session", &request)
                .await
                .unwrap();
            assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
            assert_eq!(result.summary, SnowflakeDmlSummary::unavailable());
            assert!(!result.retired);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn dml_post_ack_protocol_and_foreign_context_keep_ack_and_retire() {
        for settings in [
            json!({"protocolDml":true}),
            json!({"afterSession":"FOREIGN"}),
            json!({"afterRole":"CHANGED"}),
            json!({"afterTransactionId":"foreign-TX"}),
        ] {
            let h = Harness::open_policy(SafeMode::Confirm, true).await;
            h.settings(settings.clone());
            let state = h.driver.session_state("observer-session").await.unwrap();
            let request = dml_request(&state, "DELETE FROM DB.PUBLIC.T WHERE V = 1");
            let result = h
                .driver
                .snowflake_dml("observer-session", &request)
                .await
                .unwrap();
            assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
            assert!(result.retired && h.driver.is_closed());
            if settings.get("protocolDml").is_none() {
                assert_eq!(
                    result.summary.affected_rows.as_deref(),
                    Some("9007199254740993")
                );
            }
            assert_eq!(dml_count(&h), 1);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn dml_consumes_authority_before_busy_lane_and_rejects_changed_baseline() {
        let h = Harness::open_policy(SafeMode::Confirm, true).await;
        let state = h.driver.session_state("observer-session").await.unwrap();
        let request = dml_request(&state, "DELETE FROM DB.PUBLIC.T WHERE V = 1");
        let baseline = h.queries().len();
        let lane = h.driver.lane.lock().await;
        assert_eq!(
            h.driver
                .snowflake_dml("observer-session", &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        drop(lane);
        assert!(h.driver.validate_snowflake_dml(&request).is_err());
        assert_eq!(h.queries().len(), baseline);
        let state = h.driver.session_state("observer-session").await.unwrap();
        let request = dml_request(&state, &request.sql);
        h.settings(json!({"warehouse":"CHANGED"}));
        assert_eq!(
            h.driver
                .snowflake_dml("observer-session", &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(dml_count(&h), 0);
        h.finish().await;
    }

    #[tokio::test]
    async fn dml_abandoned_after_ack_reaps_and_revokes_authority() {
        for (settings, barrier) in [
            (json!({"holdDmlDrain":true}), "held-dml-drain"),
            (json!({"holdDmlObserver":true}), "held-dml-observer"),
        ] {
            let h = Harness::open_policy(SafeMode::Confirm, true).await;
            h.settings(settings);
            let state = h.driver.session_state("observer-session").await.unwrap();
            let request = dml_request(&state, "DELETE FROM DB.PUBLIC.T WHERE V = 1");
            let driver = h.driver.clone();
            let task =
                tokio::spawn(
                    async move { driver.snowflake_dml("observer-session", &request).await },
                );
            h.wait_until(|| h.scripts.path().join(barrier).exists())
                .await;
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(h.driver.is_closed());
            assert_eq!(dml_count(&h), 1);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn transaction_begin_commit_and_rollback_use_exact_fixed_counts_with_null_role() {
        let h = Harness::open_mode(SafeMode::Confirm).await;
        h.settings(json!({"role":null,"warehouse":null,"abortOnError":"true"}));
        let state = h.driver.session_state("observer-session").await.unwrap();
        let begin = transaction_request(&state, ExplicitTransactionAction::Begin);
        let baseline = h.queries().len();
        let begun = h
            .driver
            .transaction_control("observer-session", &begin)
            .await
            .unwrap();
        assert_eq!(
            begun.boundary_outcome,
            TransactionBoundaryOutcome::Acknowledged
        );
        assert!(!begun.retired);
        assert!(begun.error.is_none());
        assert_eq!(begun.state.role, None);
        assert!(
            matches!(begun.state.transaction_observation, Some(TransactionObservation::Observed { transaction_id:Some(ref id), .. }) if id == "opaque-TX-雪")
        );
        let mut expected = sample_sql();
        expected.push("BEGIN TRANSACTION".into());
        expected.extend(sample_sql());
        assert_eq!(h.queries()[baseline..], expected);
        let no_replay = h.queries().len();
        assert!(
            h.driver
                .transaction_control("observer-session", &begin)
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), no_replay);
        let commit = transaction_request(&begun.state, ExplicitTransactionAction::Commit);
        let baseline = h.queries().len();
        let committed = h
            .driver
            .transaction_control("observer-session", &commit)
            .await
            .unwrap();
        let mut expected = sample_sql();
        expected.push("COMMIT".into());
        expected.extend(sample_sql());
        assert_eq!(h.queries()[baseline..], expected);
        assert_eq!(committed.state.transaction_state, TransactionState::Idle);
        let rollback = transaction_request(&committed.state, ExplicitTransactionAction::Rollback);
        let baseline = h.queries().len();
        let rolled = h
            .driver
            .transaction_control("observer-session", &rollback)
            .await
            .unwrap();
        let mut expected = vec!["ROLLBACK".to_owned()];
        expected.extend(sample_sql());
        assert_eq!(h.queries()[baseline..], expected);
        assert_eq!(
            rolled.boundary_outcome,
            TransactionBoundaryOutcome::Acknowledged
        );
        assert_eq!(rolled.state.transaction_state, TransactionState::Idle);
        h.finish().await;
    }

    #[tokio::test]
    async fn transaction_known_begin_ack_and_repeated_denied_observers_keep_zero_preread_rollback()
    {
        let h = Harness::open_mode(SafeMode::Confirm).await;
        let state = h.driver.session_state("observer-session").await.unwrap();
        let begin = transaction_request(&state, ExplicitTransactionAction::Begin);
        h.settings(json!({"failBoundaryObserver":"AUTOCOMMIT"}));
        let begun = h
            .driver
            .transaction_control("observer-session", &begin)
            .await
            .unwrap();
        assert_eq!(
            begun.boundary_outcome,
            TransactionBoundaryOutcome::Acknowledged
        );
        assert_eq!(begun.error.as_ref().unwrap().code, ErrorCode::Database);
        assert!(!begun.retired);
        assert_unavailable(&begun.state);
        let rollback = transaction_request(&begun.state, ExplicitTransactionAction::Rollback);
        let count = h.queries().len();
        let mut sink = ScalarSink::default();
        assert_eq!(
            h.driver
                .execute("SELECT 1", StatementKind::Read, &mut sink)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PolicyBlocked
        );
        assert!(h.driver.completion_identity().await.is_err());
        assert!(h.driver.relation_catalog().await.is_err());
        assert!(
            h.driver
                .validate_transaction_control(&TransactionControlRequest {
                    action: ExplicitTransactionAction::Begin,
                    revision: rollback.revision.clone()
                })
                .is_err()
        );
        assert_eq!(h.queries().len(), count);
        for _ in 0..2 {
            let failed = h
                .driver
                .session_context_options("observer-session")
                .await
                .unwrap();
            assert_unavailable(&failed.state);
            assert!(failed.available.is_empty() && failed.revision.is_none());
            assert_eq!(
                transaction_request(&failed.state, ExplicitTransactionAction::Rollback),
                rollback
            );
            let failed = h.driver.session_state("observer-session").await.unwrap();
            assert_eq!(
                transaction_request(&failed, ExplicitTransactionAction::Rollback),
                rollback
            );
        }
        h.settings(json!({}));
        let baseline = h.queries().len();
        let recovered = h
            .driver
            .transaction_control("observer-session", &rollback)
            .await
            .unwrap();
        let mut expected = vec!["ROLLBACK".to_owned()];
        expected.extend(sample_sql());
        assert_eq!(h.queries()[baseline..], expected);
        assert_eq!(
            recovered.boundary_outcome,
            TransactionBoundaryOutcome::Acknowledged
        );
        assert!(!recovered.retired);
        assert_eq!(recovered.state.transaction_state, TransactionState::Idle);
        assert!(
            h.driver
                .transaction_control("observer-session", &rollback)
                .await
                .is_err()
        );
        h.finish().await;
    }

    #[tokio::test]
    async fn transaction_stale_cross_session_readonly_and_public_boundaries_dispatch_nothing() {
        let h = Harness::open().await;
        let state = h.driver.session_state("observer-session").await.unwrap();
        let rollback = transaction_request(&state, ExplicitTransactionAction::Rollback);
        let baseline = h.queries().len();
        assert_eq!(
            h.driver
                .validate_transaction_control(&TransactionControlRequest {
                    action: ExplicitTransactionAction::Begin,
                    revision: rollback.revision.clone()
                })
                .unwrap_err()
                .code,
            ErrorCode::PolicyBlocked
        );
        assert!(
            h.driver
                .transaction_control("different-session", &rollback)
                .await
                .is_err()
        );
        assert!(
            h.driver
                .transaction_control(
                    "observer-session",
                    &TransactionControlRequest {
                        action: ExplicitTransactionAction::Rollback,
                        revision: new_id()
                    }
                )
                .await
                .is_err()
        );
        for sql in ["BEGIN TRANSACTION", "COMMIT", "ROLLBACK"] {
            let mut sink = ScalarSink::default();
            assert!(
                h.driver
                    .execute(sql, StatementKind::Read, &mut sink)
                    .await
                    .is_err()
            );
        }
        assert_eq!(h.queries().len(), baseline);
        h.driver.validate_transaction_control(&rollback).unwrap();
        let mut sink = ScalarSink::default();
        h.driver
            .execute("SELECT 1", StatementKind::Read, &mut sink)
            .await
            .unwrap();
        assert!(h.driver.validate_transaction_control(&rollback).is_err());
        h.finish().await;
    }

    #[tokio::test]
    async fn transaction_baseline_full_sample_drift_and_queued_closure_never_submit_boundary() {
        for changed in [
            json!({"warehouse":"OTHER"}),
            json!({"role":null}),
            json!({"autocommit":"false"}),
            json!({"abortOnError":"true"}),
            json!({"transactionId":"external-TX"}),
            json!({"searchPath":"[\"DB.OTHER\"]"}),
        ] {
            let h = Harness::open_mode(SafeMode::Confirm).await;
            let state = h.driver.session_state("observer-session").await.unwrap();
            h.settings(changed);
            let request = transaction_request(&state, ExplicitTransactionAction::Begin);
            let baseline = h.queries().len();
            assert_eq!(
                h.driver
                    .transaction_control("observer-session", &request)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            assert_eq!(h.queries()[baseline..], sample_sql());
            assert!(h.driver.validate_transaction_control(&request).is_err());
            h.finish().await;
        }
        let h = Harness::open_mode(SafeMode::Confirm).await;
        let state = h.driver.session_state("observer-session").await.unwrap();
        let request = transaction_request(&state, ExplicitTransactionAction::Begin);
        let lane = h.driver.lane.lock().await;
        let baseline = h.queries().len();
        let driver = h.driver.clone();
        let task = tokio::spawn(async move {
            driver
                .transaction_control("observer-session", &request)
                .await
        });
        h.wait_until(|| h.driver.transaction_authority.lock().grant.is_none())
            .await;
        h.driver.close();
        drop(lane);
        assert!(task.await.unwrap().is_err());
        assert_eq!(h.queries().len(), baseline);
        h.finish().await;
    }

    #[tokio::test]
    async fn transaction_definitive_rejection_retains_recovery_without_replay() {
        let h = Harness::open_mode(SafeMode::Confirm).await;
        let state = h.driver.session_state("observer-session").await.unwrap();
        let request = transaction_request(&state, ExplicitTransactionAction::Begin);
        h.settings(json!({"rejectBoundary":true}));
        let baseline = h.queries().len();
        let rejected = h
            .driver
            .transaction_control("observer-session", &request)
            .await
            .unwrap();
        assert_eq!(
            rejected.boundary_outcome,
            TransactionBoundaryOutcome::Rejected
        );
        assert!(!rejected.retired);
        let mut expected = sample_sql();
        expected.push("BEGIN TRANSACTION".into());
        assert_eq!(h.queries()[baseline..], expected);
        assert!(matches!(
            rejected.state.transaction_control_authority,
            Some(TransactionControlAuthority::RollbackOnly { .. })
        ));
        h.finish().await;
    }

    #[tokio::test]
    async fn transaction_known_ack_final_drift_retires_without_losing_ack() {
        for settings in [
            json!({"afterWarehouse":"OTHER"}),
            json!({"afterRole":null}),
            json!({"afterAutocommit":"false"}),
            json!({"afterBoundaryTransactionId":null}),
        ] {
            let h = Harness::open_mode(SafeMode::Confirm).await;
            let state = h.driver.session_state("observer-session").await.unwrap();
            h.settings(settings);
            let result = h
                .driver
                .transaction_control(
                    "observer-session",
                    &transaction_request(&state, ExplicitTransactionAction::Begin),
                )
                .await
                .unwrap();
            assert_eq!(
                result.boundary_outcome,
                TransactionBoundaryOutcome::Acknowledged
            );
            assert!(result.retired && h.driver.is_closed());
            assert_eq!(result.error.unwrap().code, ErrorCode::ConnectionClosed);
            assert!(result.state.transaction_control_authority.is_none());
            assert_eq!(
                h.queries()
                    .iter()
                    .filter(|sql| *sql == "BEGIN TRANSACTION")
                    .count(),
                1
            );
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn transaction_lost_ack_is_unknown_and_known_ack_drain_cancel_is_acknowledged() {
        for acknowledged in [false, true] {
            let h = Harness::open_mode(SafeMode::Confirm).await;
            let state = h.driver.session_state("observer-session").await.unwrap();
            h.settings(if acknowledged {
                json!({"holdBoundaryDrain":true})
            } else {
                json!({"holdBoundary":true})
            });
            let request = transaction_request(&state, ExplicitTransactionAction::Begin);
            let driver = h.driver.clone();
            let task = tokio::spawn(async move {
                driver
                    .transaction_control("observer-session", &request)
                    .await
            });
            h.wait_until(|| {
                h.scripts
                    .path()
                    .join(if acknowledged {
                        "held-boundary-drain"
                    } else {
                        "held-boundary"
                    })
                    .exists()
            })
            .await;
            if acknowledged {
                assert_eq!(
                    h.driver.cancel().await.unwrap_err().code,
                    ErrorCode::ConnectionClosed
                );
            } else {
                h.driver.close();
            }
            let result = task.await.unwrap().unwrap();
            assert_eq!(
                result.boundary_outcome,
                if acknowledged {
                    TransactionBoundaryOutcome::Acknowledged
                } else {
                    TransactionBoundaryOutcome::Unknown
                }
            );
            assert!(result.retired);
            assert_eq!(
                result.error.unwrap().code,
                if acknowledged {
                    ErrorCode::ConnectionClosed
                } else {
                    ErrorCode::OutcomeUnknown
                }
            );
            assert_eq!(
                h.queries()
                    .iter()
                    .filter(|sql| *sql == "BEGIN TRANSACTION")
                    .count(),
                1
            );
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn transaction_post_ack_drop_and_supersession_retire_original_owner() {
        for supersede in [false, true] {
            let h = Harness::open_mode(SafeMode::Confirm).await;
            let state = h.driver.session_state("observer-session").await.unwrap();
            h.settings(json!({"holdAfterContext":true}));
            let request = transaction_request(&state, ExplicitTransactionAction::Begin);
            let driver = h.driver.clone();
            let task = tokio::spawn(async move {
                driver
                    .transaction_control("observer-session", &request)
                    .await
            });
            h.wait_until(|| h.scripts.path().join("held-after-context").exists())
                .await;
            if supersede {
                h.driver.revoke_transaction_grant();
                std::fs::write(h.scripts.path().join("release"), "").unwrap();
                let result = task.await.unwrap().unwrap();
                assert_eq!(
                    result.boundary_outcome,
                    TransactionBoundaryOutcome::Acknowledged
                );
                assert!(result.retired);
            } else {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            h.wait_until(|| h.driver.is_closed()).await;
            assert_eq!(
                h.queries()
                    .iter()
                    .filter(|sql| *sql == "BEGIN TRANSACTION")
                    .count(),
                1
            );
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn transaction_known_ack_protocol_failure_never_becomes_unknown_or_rejected() {
        let h = Harness::open_mode(SafeMode::Confirm).await;
        let state = h.driver.session_state("observer-session").await.unwrap();
        h.settings(json!({"protocolBoundary":true}));
        let result = h
            .driver
            .transaction_control(
                "observer-session",
                &transaction_request(&state, ExplicitTransactionAction::Begin),
            )
            .await
            .unwrap();
        assert_eq!(
            result.boundary_outcome,
            TransactionBoundaryOutcome::Acknowledged
        );
        assert!(result.retired && h.driver.is_closed());
        assert_eq!(result.error.unwrap().code, ErrorCode::ConnectionClosed);
        assert!(result.state.transaction_control_authority.is_none());
        assert_eq!(
            h.queries()
                .iter()
                .filter(|sql| *sql == "BEGIN TRANSACTION")
                .count(),
            1
        );
        h.finish().await;
    }

    #[tokio::test]
    async fn transaction_authority_is_independent_of_options_and_catalog_but_rejects_aba() {
        let h = Harness::open_mode(SafeMode::Confirm).await;
        let first = h.driver.session_state("observer-session").await.unwrap();
        let old = transaction_request(&first, ExplicitTransactionAction::Begin);
        h.driver.invalidate_session_options();
        h.driver.invalidate_relation_catalog();
        h.driver.validate_transaction_control(&old).unwrap();
        let current = h.driver.session_state("observer-session").await.unwrap();
        let fresh = transaction_request(&current, ExplicitTransactionAction::Begin);
        assert_ne!(old.revision, fresh.revision);
        assert!(
            h.driver
                .unavailable_session_state()
                .transaction_control_authority
                .is_none()
        );
        let baseline = h.queries().len();
        assert!(
            h.driver
                .transaction_control("observer-session", &old)
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), baseline);
        h.driver.validate_transaction_control(&fresh).unwrap();
        h.finish().await;
    }

    fn switch_request(options: &SessionContextOptions, schema: &str) -> SwitchSchemaRequest {
        SwitchSchemaRequest {
            revision: options.revision.clone().unwrap(),
            database: options.state.database.clone(),
            schema: schema.into(),
            context_fingerprint: options.state.context_fingerprint.clone().unwrap(),
        }
    }

    #[tokio::test]
    async fn switch_uses_exactly_thirteen_executions_and_publishes_fresh_null_role_options() {
        let h = Harness::open().await;
        let schema = "Sales.\"雪; --";
        h.settings(json!({"role":null,"warehouse":null,"schemas":["PUBLIC",schema]}));
        let options = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        let request = switch_request(&options, schema);
        let baseline = h.queries().len();
        let switched = h
            .driver
            .switch_schema("observer-session", &request)
            .await
            .unwrap();
        let queries = h.queries();
        assert_eq!(queries.len() - baseline, 13);
        assert_eq!(queries[baseline..baseline + 5], sample_sql());
        assert_eq!(
            queries[baseline + 5],
            metadata::schemas_sql(
                &h.driver
                    .session_options
                    .lock()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .sample
                    .context
            )
            .unwrap()
        );
        assert_eq!(
            queries[baseline + 6],
            "USE SCHEMA \"DB\".\"Sales.\"\"雪; --\""
        );
        assert_eq!(
            queries[baseline + 7],
            "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SESSION()"
        );
        assert_eq!(queries[baseline + 8..], sample_sql());
        assert_eq!(switched.state.active_context.as_deref(), Some(schema));
        assert_eq!(switched.state.role, None);
        assert_eq!(switched.state.transaction_state, TransactionState::Idle);
        assert_ne!(switched.revision, options.revision);
        assert_ne!(
            switched.state.context_fingerprint,
            options.state.context_fingerprint
        );
        assert!(h.driver.session_options.lock().snapshot.is_some());
        assert!(!h.driver.is_closed());
        h.finish().await;
    }

    fn use_count(h: &Harness) -> usize {
        h.queries()
            .iter()
            .filter(|sql| sql.starts_with("USE "))
            .count()
    }

    #[tokio::test]
    async fn switch_rejects_forged_or_malformed_authority_without_sql_and_consumes_matching_revision()
     {
        let h = Harness::open().await;
        for change in 0..7 {
            let options = h
                .driver
                .session_context_options("observer-session")
                .await
                .unwrap();
            let mut request = switch_request(&options, "OTHER");
            match change {
                0 => request.context_fingerprint = "forged".into(),
                1 => request.database = "OTHER_DB".into(),
                2 => request.schema = "unlisted".into(),
                3 => request.schema.clear(),
                4 => request.schema = "x".repeat(256),
                5 => request.schema = "a\0b".into(),
                _ => request.database = "x".repeat(256),
            }
            let baseline = h.queries().len();
            let error = h
                .driver
                .switch_schema("observer-session", &request)
                .await
                .unwrap_err();
            assert!(matches!(
                error.code,
                ErrorCode::Conflict | ErrorCode::InvalidInput
            ));
            assert_eq!(h.queries().len(), baseline);
            assert!(h.driver.session_options.lock().snapshot.is_none());
            assert_eq!(
                h.driver
                    .switch_schema("observer-session", &switch_request(&options, "OTHER"))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            assert_eq!(h.queries().len(), baseline);
        }
        let old = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        let fresh = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        let baseline = h.queries().len();
        assert_eq!(
            h.driver
                .switch_schema("observer-session", &switch_request(&old, "OTHER"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(h.queries().len(), baseline);
        assert_eq!(
            h.driver.session_options.lock().revision,
            fresh.revision.unwrap()
        );
        assert!(h.driver.session_options.lock().snapshot.is_some());
        assert_eq!(use_count(&h), 0);
        h.finish().await;
    }

    #[tokio::test]
    async fn switch_requires_cached_and_fresh_idle_matching_context_and_fresh_schema_membership() {
        let h = Harness::open().await;
        h.settings(json!({"transactionId":"open-opaque-TX"}));
        let options = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        let baseline = h.queries().len();
        assert_eq!(
            h.driver
                .switch_schema("observer-session", &switch_request(&options, "OTHER"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(h.queries().len(), baseline);
        for settings in [
            json!({"transactionId":"new-TX"}),
            json!({"warehouse":"CHANGED"}),
            json!({"schemas":["PUBLIC"]}),
            json!({"failSchemas":true}),
        ] {
            h.settings(json!({}));
            let options = h
                .driver
                .session_context_options("observer-session")
                .await
                .unwrap();
            h.settings(settings);
            let error = h
                .driver
                .switch_schema("observer-session", &switch_request(&options, "OTHER"))
                .await
                .unwrap_err();
            assert!(matches!(
                error.code,
                ErrorCode::Conflict | ErrorCode::Database
            ));
            assert!(h.driver.session_options.lock().snapshot.is_none());
            assert!(!h.driver.is_closed());
            assert_eq!(use_count(&h), 0);
        }
        h.finish().await;
    }

    #[tokio::test]
    async fn switch_superseded_during_fresh_schema_read_never_admits_use() {
        let h = Harness::open().await;
        let options = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        h.settings(json!({"holdSchemas":true}));
        let driver = h.driver.clone();
        let request = switch_request(&options, "OTHER");
        let task =
            tokio::spawn(async move { driver.switch_schema("observer-session", &request).await });
        h.wait_until(|| h.scripts.path().join("held-schemas").exists())
            .await;
        let replacement = h.driver.invalidate_session_options();
        std::fs::write(h.scripts.path().join("release"), "").unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(h.driver.session_options.lock().revision, replacement);
        assert_eq!(use_count(&h), 0);
        assert!(!h.driver.is_closed());
        h.finish().await;
    }

    #[tokio::test]
    async fn switch_busy_admission_consumes_revision_while_wrong_session_and_public_use_do_not_dispatch()
     {
        let h = Harness::open().await;
        let options = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        let request = switch_request(&options, "OTHER");
        let baseline = h.queries().len();
        assert_eq!(
            h.driver
                .switch_schema("other-session", &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        assert!(h.driver.session_options.lock().snapshot.is_some());
        assert_eq!(
            h.driver.switch_context("OTHER").await.unwrap_err().code,
            ErrorCode::Unsupported
        );
        let mut sink = ScalarSink::default();
        assert_eq!(
            h.driver
                .execute(
                    "USE SCHEMA \"DB\".\"OTHER\"",
                    StatementKind::Read,
                    &mut sink
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::PolicyBlocked
        );
        let lane = h.driver.lane.lock().await;
        assert_eq!(
            h.driver
                .switch_schema("observer-session", &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        drop(lane);
        assert_eq!(h.queries().len(), baseline);
        assert!(h.driver.session_options.lock().snapshot.is_none());
        assert!(!h.driver.is_closed());
        h.finish().await;
    }

    #[tokio::test]
    async fn switch_accepts_observed_manual_idle_but_rejects_changed_initial_session_before_use() {
        let h = Harness::open().await;
        h.settings(json!({"autocommit":"false","abortOnError":"true"}));
        let options = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        let switched = h
            .driver
            .switch_schema("observer-session", &switch_request(&options, "OTHER"))
            .await
            .unwrap();
        assert_eq!(switched.state.transaction_mode, TransactionMode::Manual);
        assert_eq!(switched.state.transaction_state, TransactionState::Idle);
        assert_eq!(use_count(&h), 1);
        h.finish().await;
        for settings in [
            json!({"principal":"different"}),
            json!({"session":"different"}),
        ] {
            let h = Harness::open().await;
            let options = h
                .driver
                .session_context_options("observer-session")
                .await
                .unwrap();
            h.settings(settings);
            assert_eq!(
                h.driver
                    .switch_schema("observer-session", &switch_request(&options, "OTHER"))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::ConnectionClosed
            );
            assert_eq!(use_count(&h), 0);
            assert!(h.driver.is_closed());
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn switch_use_rejection_is_definitive_database_error_but_retires_session() {
        let h = Harness::open().await;
        let options = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        h.settings(json!({"failUse":true}));
        let error = h
            .driver
            .switch_schema("observer-session", &switch_request(&options, "OTHER"))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Database);
        assert!(!error.message.contains("acknowledged"));
        assert!(h.driver.is_closed());
        assert_eq!(use_count(&h), 1);
        h.finish().await;
    }

    #[tokio::test]
    async fn switch_preserves_use_acknowledgement_on_readback_or_final_observation_failure() {
        for settings in [
            json!({"failReadback":true}),
            json!({"badReadback":true}),
            json!({"failFinalParameter":"AUTOCOMMIT"}),
            json!({"afterRole":"CHANGED"}),
            json!({"afterDatabase":"CHANGED"}),
            json!({"afterWarehouse":"CHANGED"}),
            json!({"afterSession":"CHANGED"}),
            json!({"afterSecondaryRoles":"{\"roles\":\"\",\"value\":\"NONE\"}"}),
            json!({"afterAutocommit":"false"}),
            json!({"afterAbortOnError":"true"}),
            json!({"afterTransactionId":"new-TX"}),
            json!({"afterPolicy":"true"}),
            json!({"afterSchema":"CHANGED"}),
        ] {
            let h = Harness::open().await;
            h.settings(settings);
            let options = h
                .driver
                .session_context_options("observer-session")
                .await
                .unwrap();
            let error = h
                .driver
                .switch_schema("observer-session", &switch_request(&options, "OTHER"))
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::ConnectionClosed);
            assert_eq!(
                error.message,
                "Snowflake acknowledged the schema change, but its final session context could not be verified. The session was closed; reconnect before continuing."
            );
            assert!(error.completion_invalidated == Some(true));
            assert!(h.driver.is_closed());
            assert!(h.driver.session_options.lock().snapshot.is_none());
            assert_eq!(use_count(&h), 1);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn abandoned_switch_after_use_ack_or_during_final_sample_reaps_without_publication() {
        for (settings, barrier) in [
            (json!({"holdReadback":true}), "held-readback"),
            (json!({"holdAfterContext":true}), "held-after-context"),
        ] {
            let h = Harness::open().await;
            let options = h
                .driver
                .session_context_options("observer-session")
                .await
                .unwrap();
            h.settings(settings);
            let driver = h.driver.clone();
            let request = switch_request(&options, "OTHER");
            let task =
                tokio::spawn(
                    async move { driver.switch_schema("observer-session", &request).await },
                );
            h.wait_until(|| h.scripts.path().join(barrier).exists())
                .await;
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(h.driver.is_closed());
            assert!(h.driver.session_options.lock().snapshot.is_none());
            assert_eq!(use_count(&h), 1);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn cancelled_switch_distinguishes_unacknowledged_use_from_known_use_success() {
        for (settings, barrier, expected) in [
            (
                json!({"holdUse":true}),
                "held-use",
                ErrorCode::OutcomeUnknown,
            ),
            (
                json!({"holdReadback":true}),
                "held-readback",
                ErrorCode::ConnectionClosed,
            ),
            (
                json!({"holdAfterContext":true}),
                "held-after-context",
                ErrorCode::ConnectionClosed,
            ),
        ] {
            let h = Harness::open().await;
            let options = h
                .driver
                .session_context_options("observer-session")
                .await
                .unwrap();
            h.settings(settings);
            let driver = h.driver.clone();
            let request = switch_request(&options, "OTHER");
            let task =
                tokio::spawn(
                    async move { driver.switch_schema("observer-session", &request).await },
                );
            h.wait_until(|| h.scripts.path().join(barrier).exists())
                .await;
            let _ = h.driver.cancel().await;
            let error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, expected);
            assert_eq!(
                error.message.contains("acknowledged the schema change"),
                expected == ErrorCode::ConnectionClosed
            );
            assert!(h.driver.is_closed());
            assert_eq!(use_count(&h), 1);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn superseded_switch_after_use_ack_cannot_publish_and_closes() {
        let h = Harness::open().await;
        let options = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        h.settings(json!({"holdAfterContext":true}));
        let driver = h.driver.clone();
        let request = switch_request(&options, "OTHER");
        let task =
            tokio::spawn(async move { driver.switch_schema("observer-session", &request).await });
        h.wait_until(|| h.scripts.path().join("held-after-context").exists())
            .await;
        h.driver.invalidate_session_options();
        std::fs::write(h.scripts.path().join("release"), "").unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.code, ErrorCode::ConnectionClosed);
        assert!(error.message.contains("acknowledged the schema change"));
        assert!(h.driver.is_closed());
        assert!(h.driver.session_options.lock().snapshot.is_none());
        assert_eq!(use_count(&h), 1);
        h.finish().await;
    }

    fn assert_unavailable(state: &SessionRuntimeState) {
        assert_eq!(state.transaction_mode, TransactionMode::Unknown);
        assert_eq!(state.transaction_state, TransactionState::Unknown);
        assert!(state.database.is_empty());
        assert!(
            state.active_context.is_none() && state.role.is_none() && state.search_path.is_empty()
        );
        assert!(state.context_fingerprint.is_none());
        assert_eq!(
            state.transaction_observation,
            Some(TransactionObservation::Unavailable)
        );
    }

    #[tokio::test]
    async fn state_uses_exactly_two_samples_and_preserves_null_as_observed_idle() {
        let h = Harness::open().await;
        let baseline = h.queries().len();
        let state = h.driver.session_state("observer-session").await.unwrap();
        assert_eq!(
            h.queries()[baseline..],
            [sample_sql(), sample_sql()].concat()
        );
        assert_eq!(state.transaction_mode, TransactionMode::AutoCommit);
        assert_eq!(state.transaction_state, TransactionState::Idle);
        assert_eq!(state.role.as_deref(), Some("ROLE"));
        assert_eq!(
            state.transaction_observation,
            Some(TransactionObservation::Observed {
                transaction_id: None,
                abort_on_error: false,
                usability: TransactionUsability::Unknown,
            })
        );
        h.finish().await;
    }

    #[tokio::test]
    async fn opaque_open_transaction_and_null_role_never_imply_usability_or_profile_fallback() {
        let h = Harness::open().await;
        h.settings(
            json!({"role":null,"transactionId":"opaque-TX-9007199254740993","abortOnError":"true"}),
        );
        let state = h.driver.session_state("observer-session").await.unwrap();
        assert_eq!(state.transaction_mode, TransactionMode::AutoCommit);
        assert_eq!(state.transaction_state, TransactionState::InTransaction);
        assert_eq!(state.role, None);
        assert_eq!(
            state.transaction_observation,
            Some(TransactionObservation::Observed {
                transaction_id: Some("opaque-TX-9007199254740993".into()),
                abort_on_error: true,
                usability: TransactionUsability::Unknown,
            })
        );
        h.finish().await;
    }

    #[tokio::test]
    async fn options_bracket_one_list_with_two_samples_and_never_insert_omitted_current_schema() {
        let h = Harness::open().await;
        h.settings(json!({"schemas":["OTHER"]}));
        let baseline = h.queries().len();
        let options = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        let queries = h.queries();
        assert_eq!(queries.len() - baseline, 11);
        assert_eq!(queries[baseline..baseline + 5], sample_sql());
        assert!(queries[baseline + 5].contains(".INFORMATION_SCHEMA.SCHEMATA "));
        assert_eq!(queries[baseline + 6..], sample_sql());
        assert_eq!(options.available, ["OTHER"]);
        assert_eq!(options.state.active_context.as_deref(), Some("PUBLIC"));
        let revision = options.revision.unwrap();
        assert!(uuid::Uuid::parse_str(&revision).is_ok());
        assert_eq!(h.driver.session_options.lock().revision, revision);
        let next = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        assert_ne!(next.revision.as_deref(), Some(revision.as_str()));
        h.finish().await;
    }

    #[tokio::test]
    async fn failed_and_inconsistent_observations_clear_options_and_publish_only_unknown() {
        let h = Harness::open().await;
        for settings in [
            json!({"failParameter":"AUTOCOMMIT"}),
            json!({"contextDrift":true}),
            json!({"transactionDrift":true}),
            json!({"wrongTransactionType":true}),
            json!({"autocommit":"not-a-boolean"}),
            json!({"database":"OTHER"}),
        ] {
            h.settings(json!({}));
            h.driver
                .session_context_options("observer-session")
                .await
                .unwrap();
            h.settings(settings);
            assert_unavailable(&h.driver.session_state("observer-session").await.unwrap());
            assert!(h.driver.session_options.lock().snapshot.is_none());
            assert!(!h.driver.is_closed());
        }
        h.finish().await;
    }

    #[tokio::test]
    async fn list_failure_projects_recovery_and_user_execution_revokes_prior_options() {
        let h = Harness::open().await;
        h.driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        h.settings(json!({"failSchemas":true}));
        let failed = h
            .driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        assert_unavailable(&failed.state);
        assert!(failed.available.is_empty());
        assert!(failed.revision.is_none());
        assert!(matches!(
            failed.state.transaction_control_authority,
            Some(TransactionControlAuthority::RollbackOnly { .. })
        ));
        assert!(h.driver.session_options.lock().snapshot.is_none());
        h.settings(json!({}));
        h.driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        let mut sink = ScalarSink::default();
        assert_eq!(
            h.driver
                .execute("SELECT 1", StatementKind::Read, &mut sink)
                .await
                .unwrap(),
            1
        );
        assert_eq!(sink.value.as_deref(), Some("1"));
        assert!(h.driver.session_options.lock().snapshot.is_none());
        // A later failed observer must not rewrite this acknowledged result.
        h.settings(json!({"failParameter":"AUTOCOMMIT"}));
        assert_unavailable(&h.driver.session_state("observer-session").await.unwrap());
        assert_eq!(sink.value.as_deref(), Some("1"));
        h.finish().await;
    }

    #[tokio::test]
    async fn superseded_refresh_cannot_publish_or_erase_replacement_authority() {
        let h = Harness::open().await;
        h.settings(json!({"holdContext":true}));
        let driver = h.driver.clone();
        let first =
            tokio::spawn(async move { driver.session_context_options("observer-session").await });
        h.wait_until(|| h.scripts.path().join("held").exists())
            .await;
        let first_revision = h.driver.session_options.lock().revision.clone();
        let driver = h.driver.clone();
        let second =
            tokio::spawn(async move { driver.session_context_options("observer-session").await });
        h.wait_until(|| h.driver.session_options.lock().revision != first_revision)
            .await;
        std::fs::write(h.scripts.path().join("release"), "").unwrap();
        assert_eq!(first.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        let replacement = second.await.unwrap().unwrap();
        assert_eq!(
            h.driver.session_options.lock().revision,
            replacement.revision.unwrap()
        );
        assert!(h.driver.session_options.lock().snapshot.is_some());
        h.finish().await;
    }

    #[tokio::test]
    async fn busy_refresh_revokes_before_lane_wait_and_wrong_session_never_dispatches() {
        let h = Harness::open().await;
        h.driver
            .session_context_options("observer-session")
            .await
            .unwrap();
        let count = h.queries().len();
        assert!(h.driver.session_state("wrong-session").await.is_err());
        assert!(
            h.driver
                .session_context_options("wrong-session")
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), count);
        let lane = h.driver.lane.lock().await;
        let driver = h.driver.clone();
        let refresh =
            tokio::spawn(async move { driver.session_context_options("observer-session").await });
        h.wait_until(|| h.driver.session_options.lock().snapshot.is_none())
            .await;
        assert_eq!(
            refresh.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(h.queries().len(), count);
        drop(lane);
        h.finish().await;
    }

    #[tokio::test]
    async fn lost_session_generation_retires_instead_of_reporting_unknown_as_success() {
        let h = Harness::open().await;
        h.settings(json!({"session":"REPLACEMENT"}));
        assert_eq!(
            h.driver
                .session_state("observer-session")
                .await
                .unwrap_err()
                .code,
            ErrorCode::ConnectionClosed
        );
        assert!(h.driver.is_closed());
        assert!(h.driver.session_options.lock().snapshot.is_none());
        h.finish().await;
    }

    #[tokio::test]
    async fn ddl_create_add_column_and_drop_preflight_validate_and_execute_with_single_use_grant() {
        // 1. CreateTable
        let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
        let create_action = SnowflakeDdlAction::CreateTable {
            database: "DB".into(),
            schema: "PUBLIC".into(),
            name: "NEW_ORDERS".into(),
            columns: vec![
                SnowflakeDdlColumnDraft {
                    name: "ID".into(),
                    data_type: "NUMBER(38, 0)".into(),
                    nullable: false,
                },
                SnowflakeDdlColumnDraft {
                    name: "LABEL".into(),
                    data_type: "VARCHAR(120)".into(),
                    nullable: true,
                },
            ],
        };
        let preflight = h
            .driver
            .preflight_snowflake_ddl(
                "observer-session",
                &SnowflakeDdlPreflightRequest {
                    session_id: "observer-session".into(),
                    action: create_action.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(preflight.kind, SnowflakeDdlKind::CreateTable);
        assert_eq!(
            preflight.qualified_target,
            "\"DB\".\"PUBLIC\".\"NEW_ORDERS\""
        );
        assert_eq!(
            preflight.sql,
            "CREATE TABLE \"DB\".\"PUBLIC\".\"NEW_ORDERS\" (\"ID\" NUMBER(38, 0) NOT NULL, \"LABEL\" VARCHAR(120))"
        );
        let req = SnowflakeDdlRequest {
            revision: preflight.revision.clone(),
            action: create_action,
            sql: preflight.sql.clone(),
        };
        h.driver.validate_snowflake_ddl(&req).unwrap();
        h.driver.validate_snowflake_ddl(&req).unwrap();
        let result = h
            .driver
            .snowflake_ddl("observer-session", &req)
            .await
            .unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
        assert_eq!(
            result.summary_status,
            SnowflakeDdlSummaryStatus::Acknowledged
        );
        assert!(result.catalog_reconciliation_required);
        assert!(!result.retired && result.error.is_none());
        assert!(
            h.driver
                .snowflake_ddl("observer-session", &req)
                .await
                .is_err()
        );
        h.finish().await;

        // 2. AddColumn & DropTable
        for is_add_column in [true, false] {
            let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
            let catalog = h.driver.relation_catalog().await.unwrap();
            let target_id = catalog
                .relations
                .iter()
                .find(|r| r.table.schema == "PUBLIC" && r.table.name == "ORDERS")
                .unwrap()
                .target_id
                .clone();
            let action = if is_add_column {
                let inspection = h
                    .driver
                    .inspect_relation(&InspectRelationRequest {
                        revision: catalog.revision.clone(),
                        target_id: target_id.clone(),
                    })
                    .await
                    .unwrap();
                SnowflakeDdlAction::AddColumn {
                    browse_revision: catalog.revision.clone(),
                    target_id: target_id.clone(),
                    inspection_revision: inspection.revision,
                    column: SnowflakeDdlColumnDraft {
                        name: "STATUS".into(),
                        data_type: "VARCHAR(32)".into(),
                        nullable: true,
                    },
                }
            } else {
                SnowflakeDdlAction::DropTable {
                    browse_revision: catalog.revision.clone(),
                    target_id,
                }
            };
            let preflight = h
                .driver
                .preflight_snowflake_ddl(
                    "observer-session",
                    &SnowflakeDdlPreflightRequest {
                        session_id: "observer-session".into(),
                        action: action.clone(),
                    },
                )
                .await
                .unwrap();
            let req = SnowflakeDdlRequest {
                revision: preflight.revision,
                action,
                sql: preflight.sql,
            };
            let result = h
                .driver
                .snowflake_ddl("observer-session", &req)
                .await
                .unwrap();
            assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
            assert_eq!(
                result.summary_status,
                SnowflakeDdlSummaryStatus::Acknowledged
            );
            assert!(result.catalog_reconciliation_required);
            assert!(!result.retired && result.error.is_none());
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn ddl_pre_ack_database_rejection_keeps_session_open_while_post_ack_failure_retires_without_rollback_only()
     {
        // Pre-ACK database rejection keeps session open if post-rejection sample is intact
        let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
        let action = SnowflakeDdlAction::CreateTable {
            database: "DB".into(),
            schema: "PUBLIC".into(),
            name: "NEW_ORDERS".into(),
            columns: vec![SnowflakeDdlColumnDraft {
                name: "ID".into(),
                data_type: "NUMBER(38, 0)".into(),
                nullable: false,
            }],
        };
        let preflight = h
            .driver
            .preflight_snowflake_ddl(
                "observer-session",
                &SnowflakeDdlPreflightRequest {
                    session_id: "observer-session".into(),
                    action: action.clone(),
                },
            )
            .await
            .unwrap();
        h.settings(json!({"rejectDdl":true}));
        let result = h
            .driver
            .snowflake_ddl(
                "observer-session",
                &SnowflakeDdlRequest {
                    revision: preflight.revision,
                    action: action.clone(),
                    sql: preflight.sql,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Rejected);
        assert_eq!(
            result.summary_status,
            SnowflakeDdlSummaryStatus::Unavailable
        );
        assert!(result.catalog_reconciliation_required);
        assert!(!result.retired && !h.driver.is_closed());
        assert_eq!(result.error.unwrap().code, ErrorCode::Database);
        assert!(!matches!(
            result.state.transaction_control_authority,
            Some(TransactionControlAuthority::RollbackOnly { .. })
        ));
        h.finish().await;

        // Post-ACK observer failure retires session and never exposes RollbackOnly
        let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
        let preflight = h
            .driver
            .preflight_snowflake_ddl(
                "observer-session",
                &SnowflakeDdlPreflightRequest {
                    session_id: "observer-session".into(),
                    action: action.clone(),
                },
            )
            .await
            .unwrap();
        h.settings(json!({"failDdlObserver":"AUTOCOMMIT"}));
        let result = h
            .driver
            .snowflake_ddl(
                "observer-session",
                &SnowflakeDdlRequest {
                    revision: preflight.revision,
                    action,
                    sql: preflight.sql,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
        assert_eq!(
            result.summary_status,
            SnowflakeDdlSummaryStatus::Acknowledged
        );
        assert!(result.retired && h.driver.is_closed());
        assert!(result.state.transaction_control_authority.is_none());
        h.finish().await;
    }

    #[tokio::test]
    async fn ddl_final_idle_observation_blocks_drift_after_catalog_recheck() {
        for settings in [
            json!({"contextDrift":true}),
            json!({"finalAutocommit":"false"}),
            json!({"transactionDrift":true}),
        ] {
            let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
            let request = h.prepare_create_ddl().await;
            let baseline = h.queries().len();
            h.settings(settings);
            let error = h
                .driver
                .snowflake_ddl("observer-session", &request)
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            let queries = h.queries();
            assert_eq!(
                queries[baseline..]
                    .iter()
                    .filter(|sql| sql.as_str() == metadata::CONTEXT_SQL)
                    .count(),
                2
            );
            assert!(!queries[baseline..].iter().any(|sql| sql == &request.sql));
            assert!(!h.driver.is_closed());
            assert!(h.driver.validate_snowflake_ddl(&request).is_err());
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn ddl_expired_while_queued_cannot_dispatch_or_reuse_grant() {
        let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
        let request = h.prepare_create_ddl().await;
        h.driver
            .ddl_authority
            .lock()
            .grant
            .as_mut()
            .unwrap()
            .issued_at = Instant::now() - (DDL_GRANT_TTL - Duration::from_millis(100));
        let lane = h.driver.lane.lock().await;
        let driver = h.driver.clone();
        let submitted = request.clone();
        let task =
            tokio::spawn(async move { driver.snowflake_ddl("observer-session", &submitted).await });
        h.wait_until(|| h.driver.ddl_authority.lock().grant.is_none())
            .await;
        tokio::time::sleep(Duration::from_millis(110)).await;
        drop(lane);
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(!h.queries().iter().any(|sql| sql == &request.sql));
        assert!(!h.driver.is_closed());
        assert!(h.driver.validate_snowflake_ddl(&request).is_err());
        h.finish().await;
    }

    #[tokio::test]
    async fn ddl_grant_binds_exact_action_sql_expiry_and_metadata_generation() {
        let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
        let request = h.prepare_create_ddl().await;
        let count = h.queries().len();
        let mut changed = request.clone();
        changed.sql.push(' ');
        assert!(h.driver.validate_snowflake_ddl(&changed).is_err());
        let mut changed = request.clone();
        let SnowflakeDdlAction::CreateTable { name, .. } = &mut changed.action else {
            unreachable!();
        };
        *name = "OTHER_TARGET".into();
        assert!(h.driver.validate_snowflake_ddl(&changed).is_err());
        h.driver.validate_snowflake_ddl(&request).unwrap();
        assert_eq!(h.queries().len(), count);
        h.driver
            .ddl_authority
            .lock()
            .grant
            .as_mut()
            .unwrap()
            .issued_at = Instant::now() - DDL_GRANT_TTL;
        assert!(h.driver.validate_snowflake_ddl(&request).is_err());
        let request = h.prepare_create_ddl().await;
        // Model the interval between transaction revocation and its dependent
        // DDL revocation. Independently matching names/samples are insufficient.
        h.driver.transaction_authority.lock().generation = new_id();
        assert!(h.driver.validate_snowflake_ddl(&request).is_err());
        let request = h.prepare_create_ddl().await;
        h.driver.relation_catalog().await.unwrap();
        assert!(h.driver.validate_snowflake_ddl(&request).is_err());
        assert!(!h.queries().iter().any(|sql| sql == &request.sql));
        h.finish().await;
    }

    #[tokio::test]
    async fn ddl_catalog_collision_after_review_is_not_submitted() {
        let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
        let request = h.prepare_create_ddl().await;
        h.settings(json!({"tableName":"NEW_ORDERS"}));
        let error = h
            .driver
            .snowflake_ddl("observer-session", &request)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(!h.queries().iter().any(|sql| sql == &request.sql));
        assert!(!h.driver.is_closed());
        h.finish().await;
    }

    #[tokio::test]
    async fn ddl_permanent_and_transient_targets_bind_traits_and_warn_about_recovery() {
        for transient in [false, true] {
            let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
            h.settings(json!({"isTransient":if transient { "YES" } else { "NO" }}));
            let catalog = h.driver.relation_catalog().await.unwrap();
            let relation = catalog
                .relations
                .iter()
                .find(|relation| relation.table.schema == "PUBLIC")
                .unwrap();
            let (_, target) = h
                .driver
                .browse_target(&catalog.revision, &relation.target_id)
                .unwrap();
            require_plain_base_table(&target).unwrap();
            let mut unknown = target.clone();
            unknown.traits.transient = None;
            assert!(require_plain_base_table(&unknown).is_err());
            let mut unknown = target.clone();
            unknown.traits.temporary = None;
            assert!(require_plain_base_table(&unknown).is_err());
            let mut temporary = target.clone();
            temporary.traits.temporary = Some(true);
            assert!(require_plain_base_table(&temporary).is_err());
            let action = SnowflakeDdlAction::DropTable {
                browse_revision: catalog.revision,
                target_id: relation.target_id.clone(),
            };
            let preflight = h
                .driver
                .preflight_snowflake_ddl(
                    "observer-session",
                    &SnowflakeDdlPreflightRequest {
                        session_id: "observer-session".into(),
                        action: action.clone(),
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                preflight.warnings.iter().any(|warning| {
                    warning.contains("reviewed target is a transient")
                        && warning.contains("0 or 1 day")
                        && warning.contains("no Fail-safe")
                }),
                transient
            );
            let result = h
                .driver
                .snowflake_ddl(
                    "observer-session",
                    &SnowflakeDdlRequest {
                        revision: preflight.revision,
                        action,
                        sql: preflight.sql,
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn ddl_preflight_requires_explicit_policy_and_idle_automatic_commit() {
        let source = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
        let request = source.prepare_create_ddl().await;
        source.finish().await;
        for (safe_mode, reviewed_ddl, settings, expected) in [
            (
                SafeMode::ReadOnly,
                false,
                json!({}),
                ErrorCode::PolicyBlocked,
            ),
            (
                SafeMode::Confirm,
                false,
                json!({}),
                ErrorCode::PolicyBlocked,
            ),
            (
                SafeMode::Confirm,
                true,
                json!({"autocommit":"false"}),
                ErrorCode::Conflict,
            ),
            (
                SafeMode::Confirm,
                true,
                json!({"transactionId":"active-TX"}),
                ErrorCode::Conflict,
            ),
        ] {
            let h = Harness::open_ddl_policy(safe_mode, reviewed_ddl).await;
            h.settings(settings);
            let error = h
                .driver
                .preflight_snowflake_ddl(
                    "observer-session",
                    &SnowflakeDdlPreflightRequest {
                        session_id: "observer-session".into(),
                        action: request.action.clone(),
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, expected);
            assert!(h.driver.ddl_authority.lock().grant.is_none());
            assert!(!h.queries().iter().any(|sql| sql == &request.sql));
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn ddl_cancel_and_protocol_loss_preserve_early_ack_without_replay() {
        for (settings, barrier, expected, summary) in [
            (
                json!({"holdDdl":true}),
                "held-ddl",
                SnowflakeDdlOutcome::Unknown,
                SnowflakeDdlSummaryStatus::Unavailable,
            ),
            (
                json!({"holdDdlDrain":true}),
                "held-ddl-drain",
                SnowflakeDdlOutcome::Succeeded,
                SnowflakeDdlSummaryStatus::Unavailable,
            ),
            (
                json!({"holdDdlObserver":true}),
                "held-ddl-observer",
                SnowflakeDdlOutcome::Succeeded,
                SnowflakeDdlSummaryStatus::Acknowledged,
            ),
            (
                json!({"protocolDdl":true}),
                "",
                SnowflakeDdlOutcome::Succeeded,
                SnowflakeDdlSummaryStatus::Unavailable,
            ),
        ] {
            let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
            let request = h.prepare_create_ddl().await;
            h.settings(settings);
            let driver = h.driver.clone();
            let submitted = request.clone();
            let task =
                tokio::spawn(
                    async move { driver.snowflake_ddl("observer-session", &submitted).await },
                );
            if !barrier.is_empty() {
                h.wait_until(|| h.scripts.path().join(barrier).exists())
                    .await;
                let _ = h.driver.cancel().await;
            }
            let result = tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(result.statement_outcome, expected, "{barrier}");
            assert_eq!(result.summary_status, summary, "{barrier}");
            assert!(result.retired && h.driver.is_closed());
            assert!(result.state.transaction_control_authority.is_none());
            assert_eq!(
                h.queries()
                    .iter()
                    .filter(|sql| *sql == &request.sql)
                    .count(),
                1
            );
            assert!(h.driver.validate_snowflake_ddl(&request).is_err());
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn ddl_dropped_waiter_or_shutdown_retires_without_replay() {
        for abort_waiter in [false, true] {
            let h = Harness::open_ddl_policy(SafeMode::Confirm, true).await;
            let request = h.prepare_create_ddl().await;
            h.settings(json!({"holdDdlDrain":true}));
            let driver = h.driver.clone();
            let submitted = request.clone();
            let task =
                tokio::spawn(
                    async move { driver.snowflake_ddl("observer-session", &submitted).await },
                );
            h.wait_until(|| h.scripts.path().join("held-ddl-drain").exists())
                .await;
            if abort_waiter {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                h.driver.close();
                let result = task.await.unwrap().unwrap();
                assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
                assert!(result.retired);
            }
            assert!(h.driver.is_closed());
            assert_eq!(
                h.queries()
                    .iter()
                    .filter(|sql| *sql == &request.sql)
                    .count(),
                1
            );
            assert!(h.driver.validate_snowflake_ddl(&request).is_err());
            h.finish().await;
        }
    }

    #[tokio::test]
    async fn snowflake_session_contexts_and_switch_context_cover_role_warehouse_database_schema_and_pre_ack_rejection()
     {
        let h = Harness::open().await;
        let catalog = h
            .driver
            .snowflake_session_contexts("observer-session")
            .await
            .unwrap();
        assert_eq!(catalog.available_roles, vec!["ANALYST", "ROLE"]);
        assert_eq!(catalog.available_warehouses, vec!["LARGE_WH", "WH"]);
        assert_eq!(catalog.available_databases, vec!["ANALYTICS_DB", "DB"]);
        assert_eq!(catalog.available_schemas, vec!["OTHER", "PUBLIC"]);
        assert_eq!(catalog.state.warehouse.as_deref(), Some("WH"));

        // Switch warehouse with one-use revision.
        let after_wh = h
            .driver
            .switch_snowflake_context(
                "observer-session",
                &SwitchSnowflakeContextRequest {
                    revision: catalog.revision.clone(),
                    context_fingerprint: catalog.state.context_fingerprint.clone().unwrap(),
                    target: SnowflakeContextTarget::Warehouse {
                        warehouse: "LARGE_WH".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(after_wh.state.warehouse.as_deref(), Some("LARGE_WH"));
        // Replaying the consumed revision is rejected.
        assert_eq!(
            h.driver
                .switch_snowflake_context(
                    "observer-session",
                    &SwitchSnowflakeContextRequest {
                        revision: catalog.revision,
                        context_fingerprint: catalog.state.context_fingerprint.unwrap(),
                        target: SnowflakeContextTarget::Warehouse {
                            warehouse: "WH".into(),
                        },
                    },
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );

        // Pre-ACK USE ROLE rejection preserves the external-browser SSO session.
        h.settings(json!({"failUse": true}));
        let rejected = h
            .driver
            .switch_snowflake_context(
                "observer-session",
                &SwitchSnowflakeContextRequest {
                    revision: after_wh.revision,
                    context_fingerprint: after_wh.state.context_fingerprint.unwrap(),
                    target: SnowflakeContextTarget::Role {
                        role: "ANALYST".into(),
                    },
                },
            )
            .await
            .unwrap_err();
        assert_eq!(rejected.code, ErrorCode::Database);
        assert!(!h.driver.is_closed());

        // Refresh after pre-ACK rejection, then switch role that clears active database,
        // and recover by switching database and schema.
        h.settings(json!({"roleClearsDatabase": true}));
        let refreshed = h
            .driver
            .snowflake_session_contexts("observer-session")
            .await
            .unwrap();
        let after_role = h
            .driver
            .switch_snowflake_context(
                "observer-session",
                &SwitchSnowflakeContextRequest {
                    revision: refreshed.revision,
                    context_fingerprint: refreshed.state.context_fingerprint.unwrap(),
                    target: SnowflakeContextTarget::Role {
                        role: "ANALYST".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(after_role.state.role.as_deref(), Some("ANALYST"));
        assert_eq!(after_role.state.database, "");
        assert!(after_role.available_schemas.is_empty());

        let after_db = h
            .driver
            .switch_snowflake_context(
                "observer-session",
                &SwitchSnowflakeContextRequest {
                    revision: after_role.revision,
                    context_fingerprint: after_role.state.context_fingerprint.unwrap(),
                    target: SnowflakeContextTarget::Database {
                        database: "ANALYTICS_DB".into(),
                        schema: None,
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(after_db.state.database, "ANALYTICS_DB");
        assert_eq!(after_db.state.active_context.as_deref(), Some("PUBLIC"));

        let after_schema = h
            .driver
            .switch_snowflake_context(
                "observer-session",
                &SwitchSnowflakeContextRequest {
                    revision: after_db.revision,
                    context_fingerprint: after_db.state.context_fingerprint.unwrap(),
                    target: SnowflakeContextTarget::Schema {
                        database: "ANALYTICS_DB".into(),
                        schema: "OTHER".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(after_schema.state.database, "ANALYTICS_DB");
        assert_eq!(after_schema.state.active_context.as_deref(), Some("OTHER"));
        assert!(!h.driver.is_closed());
        let use_queries: Vec<_> = h
            .queries()
            .into_iter()
            .filter(|sql| sql.starts_with("USE "))
            .collect();
        assert_eq!(
            use_queries,
            vec![
                "USE WAREHOUSE \"LARGE_WH\"",
                "USE ROLE \"ANALYST\"",
                "USE ROLE \"ANALYST\"",
                "USE DATABASE \"ANALYTICS_DB\"",
                "USE SCHEMA \"ANALYTICS_DB\".\"OTHER\"",
            ]
        );
        h.finish().await;
    }

    #[tokio::test]
    async fn object_explorer_and_inspector_cover_tables_views_sequences_and_overloaded_routines() {
        let h = Harness::open().await;
        let objects = h.driver.database_objects().await.unwrap();
        assert!(objects.iter().any(|o| {
            o.object.schema == "PUBLIC"
                && o.object.name == "ORDERS"
                && o.object.kind == DatabaseObjectKind::Table
                && o.estimated_rows == Some(42.0)
                && o.total_bytes == Some(8192.0)
        }));
        assert!(objects.iter().any(|o| {
            o.object.schema == "PUBLIC"
                && o.object.name == "ACTIVE_ORDERS"
                && o.object.kind == DatabaseObjectKind::View
        }));
        assert!(objects.iter().any(|o| {
            o.object.schema == "PUBLIC"
                && o.object.name == "ORDER_SEQ"
                && o.object.kind == DatabaseObjectKind::Sequence
        }));
        let overloaded_fns: Vec<_> = objects
            .iter()
            .filter(|o| {
                o.object.schema == "PUBLIC"
                    && o.object.name == "CALC_TAX"
                    && o.object.kind == DatabaseObjectKind::Function
            })
            .collect();
        assert_eq!(overloaded_fns.len(), 2);
        assert!(overloaded_fns[0].object.identity.is_some());
        assert_ne!(
            overloaded_fns[0].object.identity,
            overloaded_fns[1].object.identity
        );

        let table_details = h
            .driver
            .object_details(&DatabaseObjectRef {
                schema: "PUBLIC".into(),
                name: "ORDERS".into(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            })
            .await
            .unwrap();
        assert_eq!(table_details.comment.as_deref(), Some("Order ledger"));
        assert_eq!(
            table_details
                .storage
                .as_ref()
                .and_then(|s| s.estimated_rows),
            Some(42.0)
        );
        assert!(matches!(
            table_details.ddl,
            ObjectDdlStatus::Available { .. }
        ));
        assert!(table_details.constraints.iter().any(|c| {
            c.name == "ORDERS_PARENT_FK"
                && c.columns == vec!["PARENT_A", "PARENT_B"]
                && c.target
                    == Some(TableRef {
                        schema: "PUBLIC".into(),
                        name: "PARENTS".into(),
                    })
                && c.target_columns == vec!["A", "B"]
        }));
        assert!(
            table_details
                .constraints
                .iter()
                .any(|c| c.name == "POSITIVE_ID")
        );
        assert!(table_details.uses.iter().any(|u| {
            u.object.schema == "PUBLIC" && u.object.name == "PARENTS" && u.relation == "foreign key"
        }));

        let view_details = h
            .driver
            .object_details(&DatabaseObjectRef {
                schema: "PUBLIC".into(),
                name: "ACTIVE_ORDERS".into(),
                kind: DatabaseObjectKind::View,
                identity: None,
            })
            .await
            .unwrap();
        assert_eq!(view_details.comment.as_deref(), Some("Active view"));
        assert!(matches!(
            view_details.ddl,
            ObjectDdlStatus::Available { .. }
        ));
        assert!(view_details.uses.iter().any(|u| {
            u.object.schema == "PUBLIC"
                && u.object.name == "ORDERS"
                && u.object.kind == DatabaseObjectKind::Table
        }));

        let seq_details = h
            .driver
            .object_details(&DatabaseObjectRef {
                schema: "PUBLIC".into(),
                name: "ORDER_SEQ".into(),
                kind: DatabaseObjectKind::Sequence,
                identity: None,
            })
            .await
            .unwrap();
        assert_eq!(
            seq_details.sequence.as_ref().map(|s| s.data_type.as_str()),
            Some("NUMBER")
        );

        let fn_details = h
            .driver
            .object_details(&overloaded_fns[0].object)
            .await
            .unwrap();
        assert!(fn_details.routine.is_some());
        assert!(matches!(fn_details.ddl, ObjectDdlStatus::Available { .. }));

        let proc_ref = objects
            .iter()
            .find(|o| o.object.kind == DatabaseObjectKind::Procedure)
            .unwrap()
            .object
            .clone();
        h.settings(json!({ "rejectDefinition": true }));
        let proc_details = h.driver.object_details(&proc_ref).await.unwrap();
        assert!(proc_details.routine.is_some());
        assert!(matches!(
            proc_details.ddl,
            ObjectDdlStatus::Unavailable { .. }
        ));
        h.finish().await;
    }

    #[tokio::test]
    async fn reviewed_ddl_executes_column_migrations_and_view_lifecycle() {
        let h = Harness::open_ddl_policy(SafeMode::Guard, true).await;
        let create_view = SnowflakeDdlAction::CreateView {
            database: "DB".into(),
            schema: "PUBLIC".into(),
            name: "ACTIVE_ORDERS_V".into(),
            query_sql: "SELECT \"ID\", \"NOTES\" FROM \"DB\".\"PUBLIC\".\"ORDERS\"".into(),
        };
        let view_preflight = h
            .driver
            .preflight_snowflake_ddl(
                "observer-session",
                &SnowflakeDdlPreflightRequest {
                    session_id: "observer-session".into(),
                    action: create_view.clone(),
                },
            )
            .await
            .unwrap();
        let view_res = h
            .driver
            .snowflake_ddl(
                "observer-session",
                &SnowflakeDdlRequest {
                    revision: view_preflight.revision,
                    action: create_view,
                    sql: view_preflight.sql,
                },
            )
            .await
            .unwrap();
        assert_eq!(view_res.kind, SnowflakeDdlKind::CreateView);
        assert_eq!(view_res.statement_outcome, SnowflakeDdlOutcome::Succeeded);

        let catalog = h.driver.relation_catalog().await.unwrap();
        let target_id = catalog
            .relations
            .iter()
            .find(|r| r.table.schema == "PUBLIC" && r.table.name == "ORDERS")
            .unwrap()
            .target_id
            .clone();
        let inspected = h
            .driver
            .inspect_relation(&InspectRelationRequest {
                revision: catalog.revision.clone(),
                target_id: target_id.clone(),
            })
            .await
            .unwrap();
        let rename_action = SnowflakeDdlAction::RenameColumn {
            browse_revision: catalog.revision.clone(),
            target_id: target_id.clone(),
            inspection_revision: inspected.revision.clone(),
            column_name: "NOTES".into(),
            new_name: "MEMO".into(),
        };
        let rename_preflight = h
            .driver
            .preflight_snowflake_ddl(
                "observer-session",
                &SnowflakeDdlPreflightRequest {
                    session_id: "observer-session".into(),
                    action: rename_action.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            rename_preflight.sql,
            "ALTER TABLE \"DB\".\"PUBLIC\".\"ORDERS\" RENAME COLUMN \"NOTES\" TO \"MEMO\""
        );
        let rename_res = h
            .driver
            .snowflake_ddl(
                "observer-session",
                &SnowflakeDdlRequest {
                    revision: rename_preflight.revision,
                    action: rename_action,
                    sql: rename_preflight.sql,
                },
            )
            .await
            .unwrap();
        assert_eq!(rename_res.kind, SnowflakeDdlKind::RenameColumn);
        assert_eq!(rename_res.statement_outcome, SnowflakeDdlOutcome::Succeeded);

        h.settings(json!({ "tableName": "ACTIVE_ORDERS_V", "tableType": "VIEW" }));
        let view_catalog = h.driver.relation_catalog().await.unwrap();
        let view_target_id = view_catalog
            .relations
            .iter()
            .find(|r| r.table.schema == "PUBLIC" && r.table.name == "ACTIVE_ORDERS_V")
            .unwrap()
            .target_id
            .clone();
        let drop_view = SnowflakeDdlAction::DropView {
            browse_revision: view_catalog.revision,
            target_id: view_target_id,
        };
        let drop_view_preflight = h
            .driver
            .preflight_snowflake_ddl(
                "observer-session",
                &SnowflakeDdlPreflightRequest {
                    session_id: "observer-session".into(),
                    action: drop_view.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            drop_view_preflight.sql,
            "DROP VIEW \"DB\".\"PUBLIC\".\"ACTIVE_ORDERS_V\""
        );
        let drop_view_res = h
            .driver
            .snowflake_ddl(
                "observer-session",
                &SnowflakeDdlRequest {
                    revision: drop_view_preflight.revision,
                    action: drop_view,
                    sql: drop_view_preflight.sql,
                },
            )
            .await
            .unwrap();
        assert_eq!(drop_view_res.kind, SnowflakeDdlKind::DropView);
        assert_eq!(
            drop_view_res.statement_outcome,
            SnowflakeDdlOutcome::Succeeded
        );
        h.finish().await;
    }

    #[tokio::test]
    async fn schema_diagram_and_schema_snapshot_round_trip_through_synthetic_sdk() {
        let h = Harness::open().await;
        let diagram = h.driver.schema_diagram("PUBLIC").await.unwrap();
        assert_eq!(diagram.schema, "PUBLIC");
        assert!(!diagram.truncated);
        assert!(!diagram.restricted);
        assert_eq!(diagram.tables.len(), 2);
        let orders = diagram
            .tables
            .iter()
            .find(|n| n.table.name == "ORDERS")
            .unwrap();
        assert_eq!(orders.kind, "BASE TABLE");
        assert!(
            orders
                .columns
                .iter()
                .any(|c| c.name == "ID" && c.primary_key && c.database_type == "NUMBER(38, 0)")
        );
        assert_eq!(diagram.foreign_keys.len(), 1);
        assert_eq!(diagram.foreign_keys[0].name, "ORDERS_PARENT_FK");
        assert_eq!(diagram.foreign_keys[0].target.schema, "OTHER");
        assert_eq!(diagram.foreign_keys[0].target.name, "PARENTS");
        assert_eq!(
            diagram.foreign_keys[0].columns,
            vec!["PARENT_A", "PARENT_B"]
        );
        assert_eq!(diagram.foreign_keys[0].target_columns, vec!["A", "B"]);

        let missing = h.driver.schema_diagram("MISSING").await.unwrap_err();
        assert_eq!(missing.code, ErrorCode::NotFound);
        let reserved = h
            .driver
            .schema_diagram("INFORMATION_SCHEMA")
            .await
            .unwrap_err();
        assert_eq!(reserved.code, ErrorCode::Unsupported);

        let public_snap = h.driver.schema_snapshot("PUBLIC").await.unwrap();
        assert_eq!(public_snap.engine, EngineKind::Snowflake);
        assert_eq!(public_snap.server_version, "synthetic-version");
        assert_eq!(public_snap.schema, "PUBLIC");
        assert!(!public_snap.truncated);
        assert!(!public_snap.restricted);
        assert_eq!(public_snap.tables.len(), 2);
        assert_eq!(public_snap.sequences.len(), 1);
        assert_eq!(public_snap.routines.len(), 3);
        let orders_snap = public_snap
            .tables
            .iter()
            .find(|t| t.name == "ORDERS")
            .unwrap();
        assert_eq!(orders_snap.partition_key.as_deref(), Some("ID"));
        let active_view = public_snap
            .tables
            .iter()
            .find(|t| t.name == "ACTIVE_ORDERS")
            .unwrap();
        assert_eq!(
            active_view.view_definition.as_deref(),
            Some(
                "create or replace view \"PUBLIC\".\"ACTIVE_ORDERS\" as select ID, NOTES from \"PUBLIC\".\"ORDERS\";"
            )
        );

        let other_snap = h.driver.schema_snapshot("OTHER").await.unwrap();
        let diff =
            crate::schema_diff::compare_schema_snapshots(other_snap.clone(), public_snap.clone());
        assert!(!diff.entries.is_empty());
        let migration =
            crate::ddl::generate_migration_ddl(&other_snap, &public_snap, Some("OTHER")).unwrap();
        assert!(
            migration
                .statements
                .iter()
                .any(|s| s.sql.contains("CREATE TABLE \"OTHER\".\"ORDERS\""))
        );

        h.settings(json!({
            "failGetDdlFor": ["ACTIVE_ORDERS"]
        }));
        let restricted_snap = h.driver.schema_snapshot("PUBLIC").await.unwrap();
        assert!(restricted_snap.restricted);
        let restricted_view = restricted_snap
            .tables
            .iter()
            .find(|t| t.name == "ACTIVE_ORDERS")
            .unwrap();
        assert!(restricted_view.view_definition.is_none());

        h.finish().await;
    }
}

#[cfg(test)]
mod wire_boolean_tests {
    use super::normalize_wire_booleans;

    #[test]
    fn wire_boolean_normalization_lowercases_boolean_columns_only() {
        let mut rows = vec![
            vec![Some("TRUE".to_owned()), Some("TRUE".to_owned()), None],
            vec![
                Some("FALSE".to_owned()),
                Some("FALSE".to_owned()),
                Some("true".to_owned()),
            ],
        ];
        normalize_wire_booleans(&[true, false, true], &mut rows);
        assert_eq!(
            rows,
            vec![
                vec![Some("true".to_owned()), Some("TRUE".to_owned()), None],
                vec![
                    Some("false".to_owned()),
                    Some("FALSE".to_owned()),
                    Some("true".to_owned()),
                ],
            ]
        );
    }
}
