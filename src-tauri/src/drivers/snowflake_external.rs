//! Stateful external-browser read adapter. Packaged account workflows and writes remain unqualified.
//! Each instance owns one helper connection; no query or connection is automatically recreated.
use super::snowflake_process::{
    Bounds, Certainty, CleanupObserver, Column, Impact, Launch, Operation, OperationToken, Report,
    ShutdownReport, Supervisor, TransactionAction,
};
use super::{
    BrowseRows, CompletionRelations, Driver, QuerySink, RowMutation,
    snowflake::Snowflake,
    snowflake_browsing::{self as browsing, BrowseRelation},
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

pub struct SnowflakeExternal {
    process: Arc<Supervisor>,
    profile: ConnectionProfile,
    session_id: String,
    identity: Value,
    lane: AsyncMutex<()>,
    active: Mutex<Option<Active>>,
    browsing: Mutex<BrowseState>,
    session_options: Mutex<SessionOptionsState>,
    transaction_authority: Mutex<TransactionAuthorityState>,
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
            session_options: Mutex::new(SessionOptionsState::default()),
            transaction_authority: Mutex::new(TransactionAuthorityState::default()),
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
                explicit_transaction_controls: true,
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
        *self.transaction_authority.lock() = TransactionAuthorityState::default();
        self.closed.cancel();
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
            self.require_session_identity(&observed)?;
            // An unset database is valid session state, but cannot identify this
            // database-scoped completion workflow. Treat losing it as global.
            observed.completion_identity()?;
            let changed = self
                .session_options
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
        generation
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

    fn publish_transaction_sample(
        &self,
        sample: &ObservationSample,
        generation: &str,
    ) -> AppResult<SessionRuntimeState> {
        let mut runtime = self.observed_session_state(sample)?;
        let mut state = self.transaction_authority.lock();
        if self.is_closed() || state.generation != generation {
            return Err(stale_transaction());
        }
        let revision = new_id();
        state.grant = Some(TransactionGrant {
            revision: revision.clone(),
            sample: sample.clone(),
        });
        state.recovery = Some(TransactionRecovery {
            revision: revision.clone(),
            restricted: false,
        });
        runtime.transaction_control_authority = Some(TransactionControlAuthority::Observed {
            revision,
            allowed_actions: self.transaction_actions(sample),
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
        revision
    }

    fn invalidate_session_options_revision(&self, revision: &str) {
        let mut options = self.session_options.lock();
        if options.revision == revision {
            *options = SessionOptionsState::default();
        }
    }

    fn require_session_options_revision(&self, revision: &str) -> AppResult<()> {
        if self.session_options.lock().revision != revision {
            return Err(stale_session_options());
        }
        Ok(())
    }

    /// Called with the SDK lane held. Five fixed, bounded reads form one sample;
    /// they do not change parameters, start a transaction or repair an observation.
    async fn session_sample(&self) -> AppResult<ObservationSample> {
        let context = self
            .metadata_read(metadata::CONTEXT_SQL, 1, session::OBSERVATION_MAX_BYTES)
            .await?;
        let policy = self
            .metadata_read(
                metadata::QUOTED_IDENTIFIER_POLICY_SQL,
                1,
                session::OBSERVATION_MAX_BYTES,
            )
            .await?;
        let observed = metadata::parse_context(&context, &policy)?;
        self.require_session_identity(&observed)?;
        if observed.database.as_deref() != Some(self.profile.database.as_str()) {
            return Err(stale_session_options());
        }
        let autocommit = self
            .metadata_read(session::AUTOCOMMIT_SQL, 1, session::OBSERVATION_MAX_BYTES)
            .await?;
        let abort_on_error = self
            .metadata_read(
                session::ABORT_ON_ERROR_SQL,
                1,
                session::OBSERVATION_MAX_BYTES,
            )
            .await?;
        let transaction = self
            .metadata_read(
                session::CURRENT_TRANSACTION_SQL,
                1,
                session::OBSERVATION_MAX_BYTES,
            )
            .await?;
        let sample = session::parse_sample(
            &context,
            &policy,
            &autocommit,
            &abort_on_error,
            &transaction,
        )?;
        Ok(sample)
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
            work.await
                .inspect_err(|error| self.observer_failed(&generation, error))
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
                metadata::quote_identifier(&request.database)?;
                metadata::quote_identifier(&request.schema)?;
                if request.context_fingerprint != snapshot.sample.context.fingerprint()?
                    || request.database != self.profile.database
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
                    .metadata_read(
                        &metadata::schemas_sql(&before.context)?,
                        metadata::SCHEMA_LIMIT + 1,
                        metadata::RELATION_MAX_BYTES,
                    )
                    .await?;
                let available = metadata::parse_schemas(&before.context, &rows)?;
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
                    .metadata_read(
                        &metadata::schemas_sql(&before.context)?,
                        metadata::SCHEMA_LIMIT + 1,
                        metadata::RELATION_MAX_BYTES,
                    )
                    .await?;
                let available = metadata::parse_schemas(&before.context, &rows)?;
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
let sequence = 0, contextReads = 0, transactionReads = 0, previousSettings = '', currentSchema = 'PUBLIC', switched = false, boundarySeen = false, currentTransaction = null;
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
      if (settingsText !== previousSettings) { previousSettings = settingsText; contextReads = 0; transactionReads = 0; }
      const settings = JSON.parse(settingsText);
      const value = (key, fallback) => Object.hasOwn(settings, key) ? settings[key] : fallback;
      const context = sql === normalizedContextSql;
      const unsupportedContext = sql === oldContextSql;
      const schemas = sql.includes('.INFORMATION_SCHEMA.SCHEMATA ');
      const use = sql.startsWith('USE SCHEMA ');
      const boundary = ['BEGIN TRANSACTION','COMMIT','ROLLBACK'].includes(sql);
      const readback = sql.startsWith('SELECT CURRENT_USER(),') && switched;
      let names = ['VALUE'], rows = [['1']], parameter;
      if (boundary) {
        if (!settings.rejectBoundary) {
          currentTransaction = sql === 'BEGIN TRANSACTION' ? 'opaque-TX-雪' : null;
          boundarySeen = true;
        }
        names = []; rows = [];
      } else if (use) {
        const parts = /^USE SCHEMA "((?:[^"]|"")*)"\."((?:[^"]|"")*)"$/u.exec(sql);
        if (!parts || parts[1] !== 'DB') throw new Error('unexpected fixture switch SQL');
        if (!settings.failUse) { currentSchema = parts[2].replaceAll('""','"'); switched = true; }
        names = []; rows = [];
      } else if (sql.startsWith('SELECT CURRENT_USER(),')) {
        names = ['U','R','D','S','W','ID'];
        rows = [['SERVER_USER',value('role','ROLE'),'DB',settings.badReadback ? 'WRONG' : currentSchema,value('warehouse','WH'),'SESSION']];
      } else if (sql === 'SELECT CURRENT_VERSION()') {
        rows = [['synthetic-version']];
      } else if (unsupportedContext) {
        names = []; rows = [];
      } else if (context) {
        contextReads++;
        names = ['PRINCIPAL','ROLE_NAME','DATABASE_NAME','SCHEMA_NAME','WAREHOUSE_NAME','SESSION_ID','SEARCH_PATH','SECONDARY_ROLES'];
        rows = [[value('principal','SERVER_USER'),(switched || boundarySeen) ? value('afterRole',value('role','ROLE')) : value('role','ROLE'),(switched || boundarySeen) ? value('afterDatabase',value('database','DB')) : value('database','DB'),(switched || boundarySeen) ? value('afterSchema',currentSchema) : value('schema',currentSchema),(switched || boundarySeen) ? value('afterWarehouse',value('warehouse','WH')) : settings.contextDrift && contextReads >= 2 ? 'CHANGED' : value('warehouse','WH'),(switched || boundarySeen) ? value('afterSession',value('session','SESSION')) : value('session','SESSION'),value('searchPath',JSON.stringify(['DB.' + currentSchema])),(switched || boundarySeen) ? value('afterSecondaryRoles','{"roles":"","value":"ALL"}') : '{"roles":"","value":"ALL"}']];
      } else if (sql.startsWith('SHOW PARAMETERS LIKE ')) {
        parameter = sql.split("'")[1];
        names = ['key','value','default','level'];
        const observed = parameter === 'AUTOCOMMIT' ? (switched || boundarySeen) ? value('afterAutocommit',value('autocommit','true')) : value('autocommit','true') : parameter === 'TRANSACTION_ABORT_ON_ERROR' ? (switched || boundarySeen) ? value('afterAbortOnError',value('abortOnError','false')) : value('abortOnError','false') : (switched || boundarySeen) ? value('afterPolicy','false') : 'false';
        rows = [[parameter,observed,observed === 'true' ? 'false' : 'true','ACCOUNT']];
      } else if (sql === 'SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID') {
        transactionReads++;
        names = ['TRANSACTION_ID'];
        rows = [[boundarySeen ? value('afterBoundaryTransactionId',currentTransaction) : switched ? value('afterTransactionId',value('transactionId',currentTransaction)) : settings.transactionDrift && transactionReads >= 2 ? 'different' : value('transactionId',currentTransaction)]];
      } else if (schemas) {
        names = ['CATALOG_NAME','SCHEMA_NAME'];
        rows = value('schemas',['OTHER','PUBLIC']).map(name => ['DB',name]);
      } else if (sql !== 'SELECT 1') {
        throw new Error('unexpected fixture SQL');
      }
      let cancelled = false;
      const statement = {
        getRequestId: () => uuid(id), getQueryId: () => uuid(id + 1000),
        getColumns: () => names.map(name => ({ getName: () => name, getType: () => settings.wrongTransactionType && name === 'TRANSACTION_ID' ? 'FIXED' : 'TEXT' })),
        streamRows() {
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
      const denied = boundary && settings.rejectBoundary || boundarySeen && settings.failBoundaryObserver === parameter && parameter !== undefined || unsupportedContext || settings.failParameter === parameter && parameter !== undefined || switched && settings.failFinalParameter === parameter && parameter !== undefined || settings.failSchemas && schemas || settings.failUse && use || settings.failReadback && readback;
      setImmediate(() => {
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
process.stdin.on('data', chunk => { try { for (const frame of decoder.push(chunk)) helper.receive(frame); } catch { retire(); } });
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
            let script = FIXTURE
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
            let launch = Launch::new(
                node,
                Some(path),
                scripts.path().to_owned(),
                tempfile::tempdir().unwrap(),
            )
            .unwrap();
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
                    ExternalBrowserOptions::default(),
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
            // Switch tests separately assert their one exact quoted USE. All
            // other statements remain fixed observer/list reads or SELECT 1.
            assert!(self.queries().iter().all(|sql| {
                sql.starts_with("SELECT ")
                    || [
                        metadata::QUOTED_IDENTIFIER_POLICY_SQL,
                        session::AUTOCOMMIT_SQL,
                        session::ABORT_ON_ERROR_SQL,
                    ]
                    .contains(&sql.as_str())
                    || sql.starts_with("USE SCHEMA \"DB\".")
                    || ["BEGIN TRANSACTION", "COMMIT", "ROLLBACK"].contains(&sql.as_str())
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
}
