#[cfg(all(unix, feature = "test-harness"))]
use crate::drivers::snowflake_qualification::QualificationAttachment;
#[cfg(feature = "desktop")]
use crate::file_capability::NativeFileDialog;
use crate::{
    connection_attempts::ConnectionAttempts,
    domain::*,
    drivers::{
        ColumnOrigin, Driver, EditIdentity, EditTarget, MysqlColumnOrigin, QuerySink, RowMutation,
        mysql::Mysql,
        postgres::{Postgres, mutation_sql},
        redis::Redis,
        snowflake::Snowflake,
    },
    export::{ExportFileSession, ExportWriterOptions, resolve_export_columns},
    file_capability::{DisabledFileDialog, FileCapabilities, FileDialogProvider},
    persistence::{HistoryCompletion, HistoryOwner, Persistence},
    results::{RESULT_BUDGET, ROW_LIMIT, ResultStore},
    safety,
    secrets::{OsSecrets, SecretStore},
    ssh::{SshManager, Tunnel, host_setting},
};
#[cfg(unix)]
use crate::{
    drivers::{
        snowflake_edit_snapshot::PreparedSnowflakeEdits,
        snowflake_external::{ExternalBrowserOptions, SnowflakeExternal},
    },
    snowflake_cleanup::SnowflakeCleanup,
    snowflake_runtime::SnowflakeRuntime,
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

#[cfg(test)]
mod connection_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(all(test, unix, feature = "test-harness"))]
mod snowflake_connection_tests;

pub type EventHandler = Arc<dyn Fn(QueryEvent) -> bool + Send + Sync>;
pub type ExportEventHandler = Arc<dyn Fn(ExportEvent) -> bool + Send + Sync>;

#[cfg(unix)]
mod batch_outcome_recording;
mod history_receipts;
mod outcome_memory;
mod outcome_recording;
mod recovery_activity;
mod recovery_files;
mod recovery_jobs;
mod snowflake_console;
#[cfg(all(test, unix, feature = "test-harness"))]
mod snowflake_console_tests;
mod snowflake_ddl;
mod snowflake_dml;
#[cfg(unix)]
mod snowflake_grid;
#[cfg(all(test, unix, feature = "test-harness"))]
mod snowflake_grid_tests;
mod snowflake_receipts;
mod transaction_controls;

#[derive(Default)]
struct MetadataGate {
    revision: String,
    held: bool,
    catalog_required: bool,
}

impl MetadataGate {
    fn hold(&mut self) {
        self.revision = new_id();
        self.held = true;
    }
    fn hold_catalog(&mut self) {
        self.hold();
        self.catalog_required = true;
    }
    fn check(&self, revision: Option<&str>, resume: bool) -> AppResult<()> {
        if revision.is_some_and(|revision| revision != self.revision) || (self.held && !resume) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Metadata is paused after a reviewed write or transaction action; explicitly refresh the session to resume",
            ));
        }
        Ok(())
    }
}

struct Session {
    rollback_memory: Mutex<Option<Arc<outcome_memory::SafetySlot>>>,
    metadata_gate: Mutex<MetadataGate>,
    info: SessionInfo,
    profile: ConnectionProfile,
    driver: Arc<dyn Driver>,
    #[cfg(unix)]
    snowflake_external: Option<Arc<SnowflakeExternal>>,
    lane: tokio::sync::Mutex<()>,
    cancellation_gate: tokio::sync::Mutex<()>,
    active_run: Mutex<Option<String>>,
    closed: AtomicBool,
    tunnel: Option<Arc<Tunnel>>,
    completion: Mutex<Option<CompletionCatalog>>,
    observed_runtime: Mutex<Option<SessionRuntimeState>>,
}

struct ExplainSource {
    sql: String,
    prefix: String,
}

impl ExplainSource {
    fn map_error(&self, error: &mut AppError) {
        if let Some(location) = error.location.as_mut() {
            location.span = crate::sql_text::span_without_generated_prefix(
                &self.sql,
                &self.prefix,
                location.span,
            );
        }
    }
}

enum Action {
    Sql {
        bound: Vec<Option<crate::params::BoundStatement>>,
        error_source: Option<ExplainSource>,
        original_sql: Option<String>,
    },
    Browse {
        table: TableRef,
        query: BrowseQuery,
    },
    Changes {
        schema: TableSchema,
        identity: Option<EditIdentity>,
        rows: Vec<RowMutation>,
        import_preview: Option<(String, String)>,
        anchor_id: Option<String>,
        updates: Vec<ResultChanges>,
        deletes: Vec<ResultDeletes>,
        has_inserts: bool,
    },
    CancelProcess {
        id: String,
        identity: String,
    },
    TransactionControl {
        request: TransactionControlRequest,
    },
    SnowflakeDml {
        request: SnowflakeDmlRequest,
        bound: crate::params::BoundStatement,
        database: String,
        mode: SafeMode,
    },
    #[cfg(unix)]
    SnowflakeConsole {
        request: SnowflakeConsoleRequest,
        compiled: safety::snowflake_script::CompiledScript,
        mode: SafeMode,
        timeout_seconds: u32,
    },
    SnowflakeDdl {
        request: SnowflakeDdlRequest,
        preflight: SnowflakeDdlPreflightResponse,
        mode: SafeMode,
    },
    #[cfg(unix)]
    SnowflakeGrid {
        target: snowflake_grid::SnowflakeGridPlanTarget,
        edits: PreparedSnowflakeEdits,
        digest: String,
        mode: SafeMode,
    },
}

enum StatementProvenance {
    None,
    Candidate {
        candidate: safety::SingleTableSelectCandidate,
        live_schema: Box<TableSchema>,
        identity: Option<EditIdentity>,
        safe_mode: SafeMode,
    },
    ReadOnly {
        reason: String,
    },
}

fn import_config_digest(
    table: &TableRef,
    options: &CsvImportOptions,
    mappings: &[CsvColumnMapping],
) -> AppResult<String> {
    let bytes = serde_json::to_vec(&(table, options, mappings))
        .map_err(|_| AppError::internal("Could not fingerprint import target"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

struct ImportTarget {
    can_prepare: bool,
    requested_config_digest: String,
    config_digest: String,
    target: EditTarget,
    created: Instant,
}

struct StoredPlan {
    plan: PreparedPlan,
    action: Action,
    deadline: Instant,
    approval: Option<String>,
    digest: String,
    bytes: usize,
}

struct Job {
    session_id: String,
    token: CancellationToken,
    cancellation_sent: Arc<AtomicBool>,
}

struct RunRecording {
    cell: Arc<outcome_recording::OutcomeCell>,
    events: EventHandler,
    completion: Arc<Mutex<Option<HistoryCompletion>>>,
}

struct ExportJob {
    session_id: Option<String>,
    token: CancellationToken,
}

struct PreparedTableExport {
    session: Arc<Session>,
    schema: TableSchema,
    query: BrowseQuery,
    column_indices: Vec<usize>,
    single_page_only: bool,
    approved_digest: Option<String>,
}

enum PreparedExportSource {
    Result {
        result_id: String,
        batch_count: usize,
        column_indices: Vec<usize>,
        source_truncated: bool,
    },
    Table(Box<PreparedTableExport>),
}

struct PreparedLiveCompare {
    session: Arc<Session>,
    schema: TableSchema,
    query: BrowseQuery,
    label: String,
    single_page_only: bool,
    approval_plan_id: Option<String>,
    approved_digest: Option<String>,
}

enum PreparedCompareSide {
    Snapshot {
        label: String,
        columns: Vec<ColumnInfo>,
        result_ids: Vec<String>,
        truncated: bool,
    },
    LiveTable(Box<PreparedLiveCompare>),
}

pub struct Studio {
    persistence: Persistence,
    history_owner: Arc<HistoryOwner>,
    outcome_memory: outcome_memory::OutcomeMemory,
    secrets: Arc<dyn SecretStore>,
    file_dialog: Arc<dyn FileDialogProvider>,
    pub file_capabilities: FileCapabilities,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    plans: Mutex<HashMap<String, StoredPlan>>,
    ddl_preflights: Mutex<HashMap<String, snowflake_ddl::Preflight>>,
    pending_snowflake_receipts: Mutex<HashMap<String, snowflake_receipts::PendingReceipt>>,
    pending_history: Mutex<HashMap<String, history_receipts::PendingHistory>>,
    console: snowflake_console::State,
    #[cfg(unix)]
    batch_outcomes: batch_outcome_recording::State,
    #[cfg(unix)]
    snowflake_grid: snowflake_grid::State,
    jobs: Mutex<HashMap<String, Job>>,
    exports: Mutex<HashMap<String, ExportJob>>,
    result_sessions: Mutex<HashMap<String, String>>,
    import_targets: Mutex<HashMap<(String, String, String), ImportTarget>>,
    mysql_guard_policy: crate::drivers::mysql::guard::GuardPolicy,
    pub results: ResultStore,
    connection_attempts: ConnectionAttempts,
    dml_tasks: tokio_util::task::TaskTracker,
    configuration_gate: Mutex<()>,
    recovery_active: AtomicBool,
    recovery_activity: AtomicUsize,
    recovery_jobs: Mutex<recovery_jobs::RecoveryJobs>,
    recovery_finished: tokio::sync::Notify,
    tunnels: SshManager,
    #[cfg(unix)]
    snowflake_runtime: Option<Arc<SnowflakeRuntime>>,
    #[cfg(unix)]
    snowflake_cleanup: SnowflakeCleanup,
}

impl Studio {
    /// Native acceptance evidence only; does not authenticate or qualify a server.
    #[cfg(feature = "native-smoke")]
    pub fn snowflake_runtime_available(&self) -> bool {
        #[cfg(unix)]
        {
            self.snowflake_runtime.is_some()
        }
        #[cfg(not(unix))]
        false
    }

    pub fn open(root: &Path) -> AppResult<Arc<Self>> {
        #[cfg(feature = "desktop")]
        let dialog: Arc<dyn FileDialogProvider> = Arc::new(NativeFileDialog);
        #[cfg(not(feature = "desktop"))]
        let dialog: Arc<dyn FileDialogProvider> = Arc::new(DisabledFileDialog);
        Self::with_secrets_and_dialog(root, OsSecrets::initialize(), dialog)
    }
    pub fn with_secrets(root: &Path, secrets: Arc<dyn SecretStore>) -> AppResult<Arc<Self>> {
        Self::with_secrets_and_dialog(root, secrets, Arc::new(DisabledFileDialog))
    }
    pub fn with_secrets_and_dialog(
        root: &Path,
        secrets: Arc<dyn SecretStore>,
        file_dialog: Arc<dyn FileDialogProvider>,
    ) -> AppResult<Arc<Self>> {
        Self::with_mysql_guard_policy(root, secrets, file_dialog, Default::default())
    }

    /// Rust-only fixture injection for bounded expiry/lifecycle qualification.
    /// No IPC, profile or persisted setting can change the production policy.
    #[doc(hidden)]
    pub fn with_mysql_guard_policy(
        root: &Path,
        secrets: Arc<dyn SecretStore>,
        file_dialog: Arc<dyn FileDialogProvider>,
        mysql_guard_policy: crate::drivers::mysql::guard::GuardPolicy,
    ) -> AppResult<Arc<Self>> {
        Self::with_components(
            root,
            secrets,
            file_dialog,
            mysql_guard_policy,
            #[cfg(unix)]
            None,
        )
    }

    /// Native-only selection; resources cannot be supplied by a profile or IPC.
    #[cfg(unix)]
    pub fn with_snowflake_runtime(
        root: &Path,
        secrets: Arc<dyn SecretStore>,
        file_dialog: Arc<dyn FileDialogProvider>,
        runtime: Option<Arc<SnowflakeRuntime>>,
    ) -> AppResult<Arc<Self>> {
        Self::with_components(root, secrets, file_dialog, Default::default(), runtime)
    }

    fn with_components(
        root: &Path,
        secrets: Arc<dyn SecretStore>,
        file_dialog: Arc<dyn FileDialogProvider>,
        mysql_guard_policy: crate::drivers::mysql::guard::GuardPolicy,
        #[cfg(unix)] snowflake_runtime: Option<Arc<SnowflakeRuntime>>,
    ) -> AppResult<Arc<Self>> {
        let persistence = Persistence::open(root)?;
        let history_owner = Arc::new(persistence.claim_history_owner()?);
        let results = ResultStore::default();
        results.set_temporary_disk_enabled(
            persistence.setting("temporary-result-storage")?.as_deref() == Some("true"),
        );
        Ok(Arc::new(Self {
            persistence,
            history_owner,
            outcome_memory: outcome_memory::OutcomeMemory::default(),
            secrets,
            file_dialog,
            file_capabilities: FileCapabilities::default(),
            sessions: Mutex::new(HashMap::new()),
            plans: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
            exports: Mutex::new(HashMap::new()),
            result_sessions: Mutex::new(HashMap::new()),
            import_targets: Mutex::new(HashMap::new()),
            mysql_guard_policy,
            results,
            connection_attempts: ConnectionAttempts::default(),
            dml_tasks: tokio_util::task::TaskTracker::new(),
            ddl_preflights: Mutex::new(HashMap::new()),
            pending_snowflake_receipts: Mutex::new(HashMap::new()),
            pending_history: Mutex::new(HashMap::new()),
            configuration_gate: Mutex::new(()),
            recovery_active: AtomicBool::new(false),
            recovery_activity: AtomicUsize::new(0),
            recovery_jobs: Mutex::new(recovery_jobs::RecoveryJobs::default()),
            recovery_finished: tokio::sync::Notify::new(),
            console: snowflake_console::State::default(),
            #[cfg(unix)]
            batch_outcomes: batch_outcome_recording::State::default(),
            #[cfg(unix)]
            snowflake_grid: snowflake_grid::State::default(),
            tunnels: SshManager::default(),
            #[cfg(unix)]
            snowflake_runtime,
            #[cfg(unix)]
            snowflake_cleanup: SnowflakeCleanup::default(),
        }))
    }

    pub fn bootstrap(&self) -> AppResult<Bootstrap> {
        Ok(Bootstrap {
            profiles: self.persistence.profiles()?,
            groups: self.persistence.groups()?,
            passphrase_configured: self
                .persistence
                .setting("passphrase-configured")?
                .as_deref()
                == Some("true"),
            supported_engines: vec![
                EngineKind::Postgres,
                EngineKind::Redis,
                EngineKind::Mysql,
                EngineKind::Snowflake,
            ],
            result_budget_bytes: RESULT_BUDGET,
            row_limit: ROW_LIMIT,
            temporary_result_storage: self.results.temporary_disk_enabled(),
            workspace: self.persistence.workspace()?,
            saved_queries: self.persistence.saved_queries()?,
            bookmarks: self.persistence.bookmarks()?,
            sql_format_settings: self.persistence.sql_format_settings()?,
            sql_snippets: self.persistence.sql_snippets()?,
            virtual_foreign_keys: self.persistence.virtual_foreign_keys()?,
        })
    }

    pub fn set_temporary_result_storage(&self, enabled: bool) -> AppResult<bool> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.set_setting(
            "temporary-result-storage",
            if enabled { "true" } else { "false" },
        )?;
        self.results.set_temporary_disk_enabled(enabled);
        Ok(enabled)
    }

    pub fn save_sql_format_settings(
        &self,
        settings: SqlFormatSettings,
    ) -> AppResult<SqlFormatSettings> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.save_sql_format_settings(settings)
    }

    pub fn save_sql_snippet(&self, input: SqlSnippetInput) -> AppResult<SqlSnippet> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.save_sql_snippet(input)
    }

    pub fn delete_sql_snippet(&self, id: &str) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.delete_sql_snippet(id)
    }

    pub fn save_virtual_foreign_key(
        &self,
        input: VirtualForeignKeyInput,
    ) -> AppResult<VirtualForeignKey> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.save_virtual_foreign_key(input)
    }

    pub fn delete_virtual_foreign_key(&self, id: &str) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.delete_virtual_foreign_key(id).map(|_| ())
    }

    pub fn local_history_entries(
        &self,
        tab_key: Option<&str>,
    ) -> AppResult<Vec<LocalHistoryEntry>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.persistence.local_history_entries(tab_key)
    }

    pub fn record_local_history(
        &self,
        input: LocalHistoryInput,
    ) -> AppResult<Option<LocalHistoryEntry>> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.record_local_history(input)
    }

    pub fn delete_local_history_entry(&self, id: &str) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.delete_local_history_entry(id)
    }

    pub fn clear_local_history(&self, tab_key: Option<&str>) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.clear_local_history(tab_key)
    }

    pub fn save_saved_query(&self, input: SavedQueryInput) -> AppResult<SavedQuery> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.save_saved_query(input)
    }

    pub fn delete_saved_query(&self, id: &str) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.delete_saved_query(id)
    }

    pub fn save_bookmark(&self, label: &str, target: BookmarkTarget) -> AppResult<SavedBookmark> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.save_bookmark(label, target)
    }

    pub fn delete_bookmark(&self, id: &str) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.delete_bookmark(id)
    }

    pub fn metadata_backups(&self) -> AppResult<Vec<MetadataBackup>> {
        self.persistence.metadata_backups()
    }

    pub fn save_workspace(&self, workspace: SavedWorkspace) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.save_workspace(workspace)
    }

    pub fn create_metadata_backup(&self) -> AppResult<MetadataBackup> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.persistence.create_metadata_backup()
    }

    pub fn delete_metadata_backup(&self, id: &str, digest: &str) -> AppResult<()> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.persistence.delete_metadata_backup(id, digest)
    }

    pub fn restore_metadata_backup(
        &self,
        id: &str,
        digest: &str,
        confirmation: &str,
        passphrase: Option<&str>,
    ) -> AppResult<MetadataBackup> {
        if confirmation != "RESTORE" {
            return Err(AppError::invalid("Type RESTORE to replace local metadata"));
        }
        let _configuration = self.configuration_mutation_guard()?;
        if self.connection_attempts.has_pending() {
            return Err(AppError::invalid(
                "Cancel connection setup before restoring metadata",
            ));
        }
        if !self.sessions.lock().is_empty()
            || !self.jobs.lock().is_empty()
            || !self.exports.lock().is_empty()
            || self.results.usage().1 > 0
        {
            return Err(AppError::invalid(
                "Disconnect every connection and close all result tabs before restoring metadata",
            ));
        }
        if self
            .persistence
            .setting("passphrase-configured")?
            .as_deref()
            == Some("true")
        {
            self.verify_passphrase(passphrase)?;
        }
        self.flush_pending_history()?;
        self.flush_pending_snowflake_receipts()?;
        self.flush_console_recordings()?;
        #[cfg(unix)]
        {
            self.flush_snowflake_grid_outcomes()?;
            if !self.batch_outcomes.is_empty() || !self.snowflake_grid.is_empty() {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "A Snowflake edit owner has not settled locally; finish or disconnect it before restoring metadata",
                ));
            }
        }
        let undo = self.persistence.restore_metadata_backup(id, digest)?;
        self.console.revoke();
        self.plans.lock().clear();
        self.ddl_preflights.lock().clear();
        // Restoration replaces settings without restarting this Studio. Fail
        // closed if the restored preference cannot be read.
        self.results.set_temporary_disk_enabled(false);
        self.results.set_temporary_disk_enabled(
            self.persistence
                .setting("temporary-result-storage")?
                .as_deref()
                == Some("true"),
        );
        Ok(undo)
    }

    pub fn save_profile(
        &self,
        mut profile: ConnectionProfile,
        passphrase: Option<&str>,
    ) -> AppResult<ConnectionProfile> {
        let _configuration = self.configuration_mutation_guard()?;
        if profile.id.is_empty() {
            profile.id = new_id();
        }
        uuid::Uuid::parse_str(&profile.id)
            .map_err(|_| AppError::invalid("Invalid connection identifier"))?;
        if profile.name.trim().is_empty()
            || profile.name.len() > 100
            || profile.host.trim().is_empty()
            || profile.host.len() > 253
            || profile.port == 0
            || profile.username.is_empty()
            || profile.username.len() > 1024
            || profile.database.is_empty()
            || profile.database.len() > 1024
            || profile
                .ca_cert_path
                .as_ref()
                .is_some_and(|p| p.len() > 4096)
        {
            return Err(AppError::invalid(
                "Provide a name, host, port, database, and username",
            ));
        }
        let profiles = self.persistence.profiles()?;
        if !profiles.iter().any(|p| p.id == profile.id) && profiles.len() >= 256 {
            return Err(AppError::invalid(
                "At most 256 connection profiles may be stored",
            ));
        }
        if let Some(ssh) = &profile.ssh
            && (ssh.host.trim().is_empty()
                || ssh.host.len() > 253
                || ssh.port == 0
                || ssh.username.is_empty()
                || ssh.username.len() > 255
                || ssh
                    .private_key_path
                    .as_ref()
                    .is_some_and(|p| p.len() > 4096))
        {
            return Err(AppError::invalid(
                "Provide a valid SSH host, port, username, and key path",
            ));
        }
        let groups = self.persistence.groups()?;
        let group = groups
            .iter()
            .find(|g| g.id == profile.group_id)
            .ok_or_else(|| AppError::invalid("Choose an existing connection group"))?;
        profile.environment = group.environment;
        if profile.environment == Environment::Production && profile.safe_mode != SafeMode::ReadOnly
        {
            profile.safe_mode = SafeMode::PasswordGated;
        }
        if profile.tls_mode == TlsMode::Disable && profile.environment == Environment::Production {
            return Err(AppError::invalid(
                "Production connections require verified TLS",
            ));
        }
        if profile.engine == EngineKind::Redis {
            Redis::validate_profile(&profile)?;
        }
        if profile.engine == EngineKind::Snowflake {
            Snowflake::validate_profile(&profile)?;
        }
        if let Some(previous) = profiles
            .iter()
            .find(|p| p.id == profile.id && p.environment == Environment::Production)
        {
            let previous_json = serde_json::to_value(previous)
                .map_err(|_| AppError::internal("Could not compare connection settings"))?;
            let proposed_json = serde_json::to_value(&profile)
                .map_err(|_| AppError::internal("Could not compare connection settings"))?;
            if previous_json != proposed_json {
                self.verify_passphrase(passphrase)?;
            }
        }
        self.console.revoke();
        self.connection_attempts.cancel_profile(&profile.id);
        self.close_profile_sessions(&profile.id);
        if profiles
            .iter()
            .any(|p| p.id == profile.id && p.remember_password)
            && !profile.remember_password
        {
            self.secrets.delete(&format!("database:{}", profile.id))?;
        }
        self.persistence.save_profile(&profile)?;
        Ok(profile)
    }

    pub fn save_group(&self, mut group: ConnectionGroup) -> AppResult<ConnectionGroup> {
        let _configuration = self.configuration_mutation_guard()?;
        let groups = self.persistence.groups()?;
        let previous = groups.iter().find(|g| g.id == group.id);
        if previous.is_none() && (uuid::Uuid::parse_str(&group.id).is_err() || groups.len() >= 256)
        {
            return Err(AppError::invalid(
                "Use a valid group identifier; at most 256 groups may be stored",
            ));
        }
        group.name = group.name.trim().to_string();
        if group.name.is_empty() || group.name.len() > 100 {
            return Err(AppError::invalid("Use a group name of 1 to 100 bytes"));
        }
        let mut current = group.parent_id.as_deref();
        let mut depth = 0;
        while let Some(id) = current {
            if id == group.id || depth >= 7 {
                return Err(AppError::invalid(
                    "Groups cannot form a cycle or exceed eight levels",
                ));
            }
            let parent = groups
                .iter()
                .find(|g| g.id == id)
                .ok_or_else(|| AppError::invalid("Choose an existing parent group"))?;
            if depth == 0 {
                group.environment = parent.environment;
            }
            current = parent.parent_id.as_deref();
            depth += 1;
        }
        if previous.is_some_and(|old| old.environment != group.environment) {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "A group's environment cannot change. Create a new group in the desired environment, then move profiles individually.",
            ));
        }
        // Count the deepest descendant too when moving an existing subtree.
        for descendant in &groups {
            let mut id = descendant.parent_id.as_deref();
            let mut below = 0;
            while let Some(parent) = id {
                below += 1;
                if parent == group.id && depth + below >= 8 {
                    return Err(AppError::invalid(
                        "This move would exceed eight group levels",
                    ));
                }
                if below > 256 {
                    return Err(AppError::invalid("The stored group hierarchy is invalid"));
                }
                id = groups
                    .iter()
                    .find(|g| g.id == parent)
                    .and_then(|g| g.parent_id.as_deref());
            }
        }
        self.console.revoke();
        self.persistence.save_group(&group)?;
        Ok(group)
    }

    pub fn delete_group(&self, id: &str) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        let groups = self.persistence.groups()?;
        if ["production", "staging", "local"].contains(&id) {
            return Err(AppError::invalid(
                "The default environment groups cannot be removed",
            ));
        }
        if !groups.iter().any(|g| g.id == id) {
            return Err(AppError::new(ErrorCode::NotFound, "Group no longer exists"));
        }
        if groups.iter().any(|g| g.parent_id.as_deref() == Some(id))
            || self
                .persistence
                .profiles()?
                .iter()
                .any(|p| p.group_id == id)
        {
            return Err(AppError::invalid(
                "Move this group's connections and child groups before removing it",
            ));
        }
        self.console.revoke();
        self.persistence.delete_group(id)
    }

    pub fn delete_profile(&self, id: &str) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        let profile = self.persistence.profile(id)?;
        self.console.revoke();
        self.connection_attempts.cancel_profile(id);
        self.close_profile_sessions(id);
        if profile.remember_password {
            self.secrets.delete(&format!("database:{id}"))?;
        }
        self.persistence.delete_profile(id)
    }

    /// Reserve capacity and issue the only identifier accepted by connect_prepared.
    /// The identifier becomes the session ID if setup wins publication.
    pub fn prepare_connection(&self, profile_id: &str) -> AppResult<String> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.profile(profile_id)?;
        let (live_total, live_for_profile) = {
            let sessions = self.sessions.lock();
            (
                sessions.len(),
                sessions
                    .values()
                    .filter(|session| session.profile.id == profile_id)
                    .count(),
            )
        };
        self.connection_attempts
            .reserve(profile_id, live_total, live_for_profile)
    }

    pub fn cancel_connection_attempt(&self, attempt_id: &str) {
        // Credential persistence and publication share this gate. Cancellation
        // cannot acknowledge while either can still complete after its check.
        let _configuration = self.configuration_gate.lock();
        self.connection_attempts.cancel(attempt_id);
        // Publication may already have won while its response was lost. The
        // reservation ID names exactly that session, never another by profile.
        self.disconnect(attempt_id);
    }

    pub async fn cancel_connection_attempt_and_wait(&self, attempt_id: &str) -> AppResult<()> {
        self.cancel_connection_attempt(attempt_id);
        #[cfg(unix)]
        self.snowflake_cleanup
            .wait_for(attempt_id, Duration::from_secs(10))
            .await?;
        Ok(())
    }

    pub async fn disconnect_and_wait(&self, session_id: &str) -> AppResult<()> {
        self.disconnect(session_id);
        #[cfg(unix)]
        self.snowflake_cleanup
            .wait_for(session_id, Duration::from_secs(10))
            .await?;
        Ok(())
    }

    /// Compatibility wrapper: every legacy request still has a tracked owner.
    pub async fn connect(&self, id: &str, secrets: ConnectionSecrets) -> AppResult<SessionInfo> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let attempt_id = self.prepare_connection(id)?;
        self.connect_prepared(&attempt_id, secrets).await
    }

    pub async fn connect_prepared(
        &self,
        attempt_id: &str,
        secrets: ConnectionSecrets,
    ) -> AppResult<SessionInfo> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.connect_prepared_attached(
            attempt_id,
            secrets,
            #[cfg(all(unix, feature = "test-harness"))]
            None,
        )
        .await
    }

    /// Rust-only harness connection. The eventual fixed runner must establish
    /// its execution envelope before calling; this method grants no account scope.
    #[cfg(all(unix, feature = "test-harness"))]
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "Internal qualification entry; tested without exposing desktop IPC"
        )
    )]
    pub(crate) async fn connect_prepared_qualification(
        &self,
        attempt_id: &str,
        secrets: ConnectionSecrets,
        qualification: QualificationAttachment,
    ) -> AppResult<SessionInfo> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.connect_prepared_attached(attempt_id, secrets, Some(qualification))
            .await
    }

    async fn connect_prepared_attached(
        &self,
        attempt_id: &str,
        secrets: ConnectionSecrets,
        #[cfg(all(unix, feature = "test-harness"))] qualification: Option<QualificationAttachment>,
    ) -> AppResult<SessionInfo> {
        let result = self
            .connect_prepared_inner(
                attempt_id,
                secrets,
                #[cfg(all(unix, feature = "test-harness"))]
                qualification,
            )
            .await;
        #[cfg(unix)]
        if result.is_err() {
            self.snowflake_cleanup
                .wait_for(attempt_id, Duration::from_secs(10))
                .await?;
        }
        result
    }

    async fn connect_prepared_inner(
        &self,
        attempt_id: &str,
        secrets: ConnectionSecrets,
        #[cfg(all(unix, feature = "test-harness"))] qualification: Option<QualificationAttachment>,
    ) -> AppResult<SessionInfo> {
        let mut attempt = self.connection_attempts.claim(attempt_id)?;
        let id = attempt.profile_id().to_owned();
        let session_id = attempt.id().to_owned();
        let cancellation = attempt.cancellation();
        #[cfg(unix)]
        let mut sdk_preparation = None;
        #[cfg(all(unix, feature = "test-harness"))]
        let qualification_check = qualification
            .as_ref()
            .map(|attachment| attachment.transport.clone());
        #[cfg(all(test, unix, feature = "test-harness"))]
        let qualification_before_publication = qualification
            .as_ref()
            .and_then(|attachment| attachment.before_publication.clone());
        let (profile, password, known_host) = {
            let _configuration = self.configuration_mutation_guard()?;
            if cancellation.is_cancelled() {
                return Err(connection_cancelled());
            }
            let profile = self.persistence.profile(&id)?;
            #[cfg(all(unix, feature = "test-harness"))]
            if qualification.is_some()
                && (profile.engine != EngineKind::Snowflake
                    || profile.snowflake.as_ref().is_none_or(|config| {
                        config.authentication != SnowflakeAuthentication::ExternalBrowser
                    })
                    || profile.ssh.is_some()
                    || profile.remember_password
                    || secrets.password.is_some()
                    || secrets.ssh_password.is_some()
                    || secrets.key_passphrase.is_some())
            {
                return Err(AppError::new(
                    ErrorCode::PolicyBlocked,
                    "Qualification requires a Snowflake external-browser connection without password storage, supplied secrets or a tunnel",
                ));
            }
            let password = match secrets.password {
                Some(value) => Some(Zeroizing::new(value)),
                None if profile.remember_password => self
                    .secrets
                    .get(&format!("database:{id}"))?
                    .map(Zeroizing::new),
                None => None,
            };
            let known_host = profile
                .ssh
                .as_ref()
                .map(|ssh| self.persistence.setting(&host_setting(ssh)))
                .transpose()?
                .flatten();
            if profile.engine == EngineKind::Snowflake
                && profile.snowflake.as_ref().is_some_and(|config| {
                    config.authentication == SnowflakeAuthentication::ExternalBrowser
                })
            {
                if password.is_some() {
                    return Err(AppError::invalid(
                        "Snowflake browser sign-in does not use a password",
                    ));
                }
                #[cfg(unix)]
                {
                    let runtime = self
                        .snowflake_runtime
                        .clone()
                        .ok_or_else(snowflake_runtime_unavailable)?;
                    let permit = self.snowflake_cleanup.reserve(attempt_id)?;
                    sdk_preparation = Some((runtime, permit));
                }
                #[cfg(not(unix))]
                return Err(snowflake_runtime_unavailable());
            }
            (profile, password, known_host)
        };
        #[cfg(unix)]
        let mut sdk_launch = match sdk_preparation {
            Some((runtime, permit)) => Some(
                runtime
                    .new_launch_tracked(permit)
                    .map_err(|_| snowflake_runtime_unavailable())?,
            ),
            None => None,
        };
        // Hashing/allocation happen outside the gate, with a retained pending
        // cleanup permit. Recheck before any authentication future is polled.
        {
            let _configuration = self.configuration_mutation_guard()?;
            if cancellation.is_cancelled() {
                return Err(connection_cancelled());
            }
            if self
                .persistence
                .profile(&id)
                .ok()
                .and_then(|p| serde_json::to_string(&p).ok())
                != serde_json::to_string(&profile).ok()
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "This connection profile changed while connecting. Connect again using its current settings.",
                ));
            }
        }
        // Neither the configuration gate nor a global connection lane is held
        // while SSH, database authentication, or browser sign-in is pending.
        let rollback_memory = self.outcome_memory.safety()?;
        let setup = async {
            let tunnel = if profile.ssh.is_some() {
                Some(
                    self.tunnels
                        .acquire(
                            &profile,
                            known_host,
                            secrets.ssh_password.map(Zeroizing::new),
                            secrets.key_passphrase.map(Zeroizing::new),
                        )
                        .await?,
                )
            } else {
                None
            };
            #[cfg(unix)]
            let mut console_owner = None;
            #[cfg(not(unix))]
            let console_owner = ();
            #[cfg(unix)]
            let mut snowflake_external = None;
            #[cfg(not(unix))]
            let snowflake_external = ();
            let (driver, info): (Arc<dyn Driver>, SessionInfo) = match profile.engine {
                EngineKind::Redis => {
                    let (driver, info) = Redis::open(
                        &profile,
                        password.as_deref().map(String::as_str),
                        &session_id,
                        tunnel.as_ref().map(|t| t.port),
                    )
                    .await?;
                    (driver, info)
                }
                EngineKind::Mysql => {
                    let (driver, info) = Mysql::open_with_guard_policy(
                        &profile,
                        password.as_deref().map(String::as_str),
                        &session_id,
                        tunnel.as_ref().map(|t| t.port),
                        self.mysql_guard_policy.clone(),
                    )
                    .await?;
                    (driver, info)
                }
                EngineKind::Postgres => {
                    let (d, i) = Postgres::open(
                        &profile,
                        password.as_deref().map(String::as_str),
                        &session_id,
                        tunnel.as_ref().map(|t| t.port),
                    )
                    .await?;
                    (d, i)
                }
                EngineKind::Snowflake => {
                    if password.is_some() {
                        return Err(AppError::invalid(
                            "Snowflake browser sign-in does not use a password",
                        ));
                    }
                    match profile
                        .snowflake
                        .as_ref()
                        .map(|config| config.authentication)
                        .unwrap_or_default()
                    {
                        SnowflakeAuthentication::LocalApplicationOauth => {
                            let (d, i) = Snowflake::open(&profile, &session_id).await?;
                            (d as Arc<dyn Driver>, i)
                        }
                        SnowflakeAuthentication::ExternalBrowser => {
                            #[cfg(unix)]
                            {
                                let launch = sdk_launch
                                    .take()
                                    .ok_or_else(snowflake_runtime_unavailable)?;
                                let options = ExternalBrowserOptions::default();
                                #[cfg(feature = "test-harness")]
                                let (launch, options) = {
                                    let mut launch = launch;
                                    let mut options = options;
                                    if let Some(qualification) = qualification {
                                        launch.attach_transport(qualification.transport.clone())?;
                                        options.metadata_observer =
                                            Some(qualification.metadata.clone());
                                    }
                                    (launch, options)
                                };
                                let (d, mut i) =
                                    SnowflakeExternal::open(&profile, &session_id, launch, options)
                                        .await?;
                                snowflake_external = Some(d.clone());
                                if profile
                                    .snowflake
                                    .as_ref()
                                    .is_some_and(|c| c.reviewed_console)
                                {
                                    self.flush_pending_snowflake_receipts()?;
                                    self.flush_console_recordings()?;
                                    console_owner = Some(d.claim_reviewed_console().await?);
                                    i.console_mode = true;
                                }
                                (d as Arc<dyn Driver>, i)
                            }
                            #[cfg(not(unix))]
                            return Err(snowflake_runtime_unavailable());
                        }
                    }
                }
            };
            Ok::<_, AppError>((driver, info, tunnel, console_owner, snowflake_external))
        };
        let (driver, info, tunnel, console_owner, snowflake_external) = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(connection_cancelled()),
            outcome = setup => outcome?,
        };
        #[cfg(all(test, unix, feature = "test-harness"))]
        if let Some(before_publication) = qualification_before_publication {
            before_publication();
        }
        let _configuration = self.configuration_mutation_guard()?;
        if cancellation.is_cancelled() {
            driver.close();
            return Err(connection_cancelled());
        }
        if self
            .persistence
            .profile(&id)
            .ok()
            .and_then(|current| serde_json::to_string(&current).ok())
            != serde_json::to_string(&profile).ok()
        {
            driver.close();
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This connection profile changed while connecting. Connect again using its current settings.",
            ));
        }
        if profile.remember_password
            && let Some(password) = password.as_ref()
            && let Err(error) = self.secrets.set(&format!("database:{id}"), password)
        {
            driver.close();
            return Err(error);
        }
        let result = attempt.try_publish_checked(|| {
            let mut sessions = self.sessions.lock();
            #[cfg(all(unix, feature = "test-harness"))]
            if let Some(check) = &qualification_check {
                check.check()?;
                if driver.is_closed() {
                    return Err(AppError::new(
                        ErrorCode::ConnectionClosed,
                        "The Snowflake helper exited before the connection could be published",
                    ));
                }
            }
            #[cfg(unix)]
            if let Some(owner) = console_owner {
                self.console
                    .owners
                    .lock()
                    .insert(session_id.clone(), Arc::new(tokio::sync::Mutex::new(owner)));
            }
            #[cfg(not(unix))]
            let _ = (console_owner, snowflake_external);
            sessions.insert(
                session_id,
                Arc::new(Session {
                    rollback_memory: Mutex::new(Some(rollback_memory)),
                    info: info.clone(),
                    profile,
                    driver: driver.clone(),
                    #[cfg(unix)]
                    snowflake_external,
                    lane: tokio::sync::Mutex::new(()),
                    completion: Mutex::new(None),
                    metadata_gate: Mutex::new(MetadataGate::default()),
                    observed_runtime: Mutex::new(None),
                    cancellation_gate: tokio::sync::Mutex::new(()),
                    active_run: Mutex::new(None),
                    closed: AtomicBool::new(false),
                    tunnel,
                }),
            );
            Ok(info)
        });
        if result.is_err() {
            driver.close();
        }
        result
    }

    fn session(&self, id: &str) -> AppResult<Arc<Session>> {
        let session = self.sessions.lock().get(id).cloned().ok_or_else(|| {
            AppError::new(ErrorCode::ConnectionClosed, "Connect to a database first")
        })?;
        if session.closed.load(Ordering::Acquire)
            || session.driver.is_closed()
            || session.tunnel.as_ref().is_some_and(|t| t.is_closed())
        {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        Ok(session)
    }

    pub async fn probe_ssh(&self, profile_id: &str) -> AppResult<SshHostKey> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.probe_ssh_with_attempt(profile_id, None).await
    }

    pub async fn probe_ssh_with_attempt(
        &self,
        profile_id: &str,
        attempt_id: Option<&str>,
    ) -> AppResult<SshHostKey> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let prepared = attempt_id.is_some();
        let id = match attempt_id {
            Some(id) => id.to_owned(),
            None => self.prepare_connection(profile_id)?,
        };
        let attempt = self.connection_attempts.claim(&id)?;
        let cancellation = attempt.cancellation();
        let (ssh, known) = {
            let _configuration = self.configuration_mutation_guard()?;
            if cancellation.is_cancelled() {
                return Err(connection_cancelled());
            }
            if attempt.profile_id() != profile_id {
                return Err(AppError::invalid(
                    "This connection request belongs to a different profile",
                ));
            }
            let ssh = self
                .persistence
                .profile(profile_id)?
                .ssh
                .ok_or_else(|| AppError::invalid("SSH is not configured"))?;
            let known = self.persistence.setting(&host_setting(&ssh))?;
            (ssh, known)
        };
        // The losing probe future drops its transport before the claim owner
        // is released, so shutdown cannot drain past a live preflight socket.
        let key = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(connection_cancelled()),
            outcome = self.tunnels.probe(&ssh, known) => outcome?,
        };
        let _configuration = self.configuration_mutation_guard()?;
        if cancellation.is_cancelled() {
            return Err(connection_cancelled());
        }
        if prepared {
            attempt.return_to_reserved()?;
        }
        Ok(key)
    }

    pub fn trust_ssh_key(&self, challenge_id: &str, fingerprint: &str) -> AppResult<()> {
        let (host, line) = self.tunnels.accept(challenge_id, fingerprint)?;
        let setting = host_setting(&host);
        if self
            .persistence
            .setting(&setting)?
            .is_some_and(|previous| previous != line)
        {
            return Err(AppError::new(
                ErrorCode::SshHostKeyChanged,
                "A different host key is already trusted. Connection blocked.",
            ));
        }
        self.persistence.set_setting(&setting, &line)?;
        self.persistence
            .audit("ssh", "trust-host", fingerprint, "accepted")
    }

    pub fn disconnect(&self, id: &str) {
        #[cfg(unix)]
        {
            self.console.owners.lock().remove(id);
            self.console.forget_session(id);
            self.snowflake_grid.close_session_helpers(id);
        }
        if let Some(session) = self.sessions.lock().remove(id) {
            session.closed.store(true, Ordering::Release);
            session.driver.close();
        }
        self.plans.lock().retain(|_, p| p.plan.session_id != id);
        self.ddl_preflights.lock().remove(id);
        self.import_targets
            .lock()
            .retain(|(session_id, _, _), _| session_id != id);
        for job in self.jobs.lock().values().filter(|j| j.session_id == id) {
            job.token.cancel();
        }
        for export in self
            .exports
            .lock()
            .values()
            .filter(|e| e.session_id.as_deref() == Some(id))
        {
            export.token.cancel();
        }
        let results = self
            .result_sessions
            .lock()
            .iter()
            .filter(|(result, s)| s.as_str() == id && !self.console.owns_result(result))
            .map(|(r, _)| r.clone())
            .collect::<Vec<_>>();
        for result in results {
            self.release_result(&result);
        }
    }

    fn close_profile_sessions(&self, id: &str) {
        let ids = self
            .sessions
            .lock()
            .iter()
            .filter(|(_, s)| s.profile.id == id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.disconnect(&id);
        }
        #[cfg(unix)]
        self.snowflake_grid.close_profile_helpers(id);
    }

    pub fn set_passphrase(&self, passphrase: &str, previous: Option<&str>) -> AppResult<()> {
        let _configuration = self.configuration_mutation_guard()?;
        if self
            .persistence
            .setting("passphrase-configured")?
            .as_deref()
            == Some("true")
        {
            self.verify_passphrase(previous)?;
        }
        if passphrase.chars().count() < 12 || passphrase.len() > 1024 {
            return Err(AppError::invalid(
                "Use a passphrase of 12 to 1,024 characters",
            ));
        }
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(passphrase.as_bytes(), &salt)
            .map_err(|_| AppError::internal("Could not create passphrase verifier"))?
            .to_string();
        // Rotation can change the secret even if the following SQLite write
        // fails. Revoke queued and consumed review authority before either mutation.
        self.console.revoke();
        self.plans.lock().clear();
        self.ddl_preflights.lock().clear();
        self.secrets.set("write-passphrase", &hash)?;
        self.persistence
            .set_setting("passphrase-configured", "true")?;
        Ok(())
    }

    fn verify_passphrase(&self, passphrase: Option<&str>) -> AppResult<()> {
        let passphrase = passphrase.filter(|p| p.len() <= 1024).ok_or_else(|| {
            AppError::new(
                ErrorCode::PassphraseRequired,
                "Enter your application write passphrase",
            )
        })?;
        let hash=self.secrets.get("write-passphrase")?.ok_or_else(||AppError::new(ErrorCode::PassphraseRequired,"Configure a write passphrase in Settings before changing protected connections or running production writes"))?;
        let hash = PasswordHash::new(&hash)
            .map_err(|_| AppError::internal("The passphrase verifier is invalid"))?;
        Argon2::default()
            .verify_password(passphrase.as_bytes(), &hash)
            .map_err(|_| {
                AppError::new(
                    ErrorCode::InvalidPassphrase,
                    "The write passphrase is incorrect",
                )
            })
    }

    pub fn prepare_execution(&self, session_id: &str, sql: &str) -> AppResult<PreparedPlan> {
        self.prepare_execution_with_params(session_id, sql, None)
    }

    pub fn prepare_execution_with_params(
        &self,
        session_id: &str,
        sql: &str,
        params: Option<Vec<QueryParamInput>>,
    ) -> AppResult<PreparedPlan> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.prepare_execution_inner(session_id, sql, params, None)
    }

    fn prepare_execution_inner(
        &self,
        session_id: &str,
        sql: &str,
        params: Option<Vec<QueryParamInput>>,
        error_source: Option<ExplainSource>,
    ) -> AppResult<PreparedPlan> {
        let session = self.session(session_id)?;
        if session.info.console_mode {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Use the whole-script review for this retained Snowflake console",
            ));
        }
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        if error_source.is_none() && snowflake_dml::sdk_profile(&session.profile) {
            let database = session.observed_runtime.lock().as_ref().map_or_else(
                || session.profile.database.clone(),
                |state| state.database.clone(),
            );
            if safety::snowflake_dml::analyze(sql, &database).is_ok() {
                return self.prepare_snowflake_dml(session_id, sql, params.as_deref());
            }
        }
        let source_bytes = error_source
            .as_ref()
            .map_or(0, |source| source.sql.len() + source.prefix.len());
        let slots = crate::params::extract_query_params(session.profile.engine, sql)?;
        if slots.is_empty() {
            crate::params::validate_inputs(&slots, params.as_deref())?;
            let plan = safety::prepare(session_id, &session.profile, sql)?;
            snowflake_dml::require_sdk_read_plan(&session.profile, &plan)?;
            return self.store_plan(
                plan,
                Action::Sql {
                    bound: Vec::new(),
                    error_source,
                    original_sql: Some(sql.to_owned()),
                },
                sql.len() + source_bytes,
            );
        }
        let (bound_parameters, validated) =
            crate::params::validate_inputs(&slots, params.as_deref())?;
        let classification =
            crate::params::rewrite_for_classification(session.profile.engine, sql)?;
        let mut plan =
            safety::prepare_parameterized(session_id, &session.profile, sql, &classification)?;
        snowflake_dml::require_sdk_read_plan(&session.profile, &plan)?;
        let mut question_offset = 0usize;
        let mut bound = Vec::with_capacity(plan.statements.len());
        let mut remaining_bytes =
            crate::params::MAX_BOUND_PLAN_BYTES.saturating_sub(sql.len() + source_bytes);
        for stmt in &plan.statements {
            bound.push(crate::params::compile_statement_with_budget(
                session.profile.engine,
                &stmt.sql,
                &validated,
                &mut question_offset,
                &mut remaining_bytes,
            )?);
        }
        let param_bytes = bound_parameters
            .iter()
            .map(|p| p.key.len() + p.value.as_ref().map_or(4, String::len))
            .sum::<usize>();
        plan.binding_digest = Some(hex::encode(Sha256::digest(
            serde_json::to_vec(&bound_parameters)
                .map_err(|_| AppError::internal("Could not fingerprint bound parameters"))?,
        )));
        plan.bound_parameters = bound_parameters;
        self.store_plan(
            plan,
            Action::Sql {
                bound,
                error_source,
                original_sql: Some(sql.to_owned()),
            },
            (crate::params::MAX_BOUND_PLAN_BYTES - remaining_bytes).saturating_add(param_bytes),
        )
    }

    pub fn prepare_explain(
        &self,
        session_id: &str,
        sql: &str,
        analyze: bool,
    ) -> AppResult<PreparedPlan> {
        self.prepare_explain_with_params(session_id, sql, analyze, None)
    }

    pub fn prepare_explain_with_params(
        &self,
        session_id: &str,
        sql: &str,
        analyze: bool,
        params: Option<Vec<QueryParamInput>>,
    ) -> AppResult<PreparedPlan> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        if analyze && !session.info.capabilities.explain_analyze {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "EXPLAIN ANALYZE is not supported by this session's engine capabilities",
            ));
        }
        let classification =
            crate::params::rewrite_for_classification(session.profile.engine, sql)?;
        let explained = crate::query_plan::explain_sql(
            session.profile.engine,
            &classification.sql,
            analyze,
            session.info.capabilities.actual_plan,
        )
        .map_err(|mut error| {
            if let Some(location) = error.location.as_mut() {
                location.span = classification.map.span_from_rewritten_line_char_col(
                    sql,
                    &classification.sql,
                    u64::from(location.span.line),
                    u64::from(location.span.column),
                );
            }
            error
        })?;
        let prefix_len = explained.len() - classification.sql.len();
        let source = ExplainSource {
            sql: sql.to_owned(),
            prefix: explained[..prefix_len].to_owned(),
        };
        self.prepare_execution_inner(
            session_id,
            &format!("{}{}", source.prefix, sql),
            params,
            Some(source),
        )
        .map_err(|mut error| {
            ExplainSource {
                sql: sql.to_owned(),
                prefix: explained[..prefix_len].to_owned(),
            }
            .map_error(&mut error);
            error
        })
    }

    fn store_plan(
        &self,
        plan: PreparedPlan,
        mut action: Action,
        mut bytes: usize,
    ) -> AppResult<PreparedPlan> {
        if let Action::Sql { original_sql, .. } = &mut action {
            let text = original_sql.get_or_insert_with(|| {
                plan.statements
                    .iter()
                    .map(|s| s.sql.as_str())
                    .collect::<Vec<_>>()
                    .join(";\n")
            });
            bytes = bytes.saturating_add(text.len());
        }
        self.console.replace_preview(&plan.session_id);
        let mut plans = self.plans.lock();
        plans.retain(|_, p| p.deadline > Instant::now());
        // A newer preview replaces earlier previews for the same session.
        plans.retain(|_, p| p.plan.session_id != plan.session_id);
        if plans.len() >= 32
            || plans
                .values()
                .map(|p| p.bytes)
                .sum::<usize>()
                .saturating_add(bytes)
                > 8 * 1024 * 1024
        {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Prepared changes exceed the 8 MiB review budget",
            ));
        }
        let mut fingerprint = Sha256::new();
        fingerprint.update(
            serde_json::to_vec(&plan)
                .map_err(|_| AppError::internal("Could not fingerprint query plan"))?,
        );
        match &action {
            #[cfg(unix)]
            Action::SnowflakeConsole {
                request,
                compiled,
                mode,
                timeout_seconds,
            } => fingerprint.update(
                serde_json::to_vec(&(
                    request,
                    compiled
                        .units()
                        .iter()
                        .map(|u| u.bound())
                        .collect::<Vec<_>>(),
                    mode,
                    timeout_seconds,
                ))
                .map_err(|_| AppError::internal("Could not fingerprint reviewed console"))?,
            ),
            Action::Browse { table, query } => fingerprint.update(
                serde_json::to_vec(&(table, query))
                    .map_err(|_| AppError::internal("Could not fingerprint table filter"))?,
            ),
            Action::Changes {
                schema,
                identity,
                rows,
                import_preview,
                ..
            } => fingerprint.update(
                serde_json::to_vec(&(schema, identity, rows, import_preview))
                    .map_err(|_| AppError::internal("Could not fingerprint bound changes"))?,
            ),
            Action::CancelProcess { id, identity } => fingerprint.update(
                serde_json::to_vec(&(id, identity))
                    .map_err(|_| AppError::internal("Could not fingerprint process target"))?,
            ),
            Action::Sql {
                bound,
                original_sql,
                ..
            } => fingerprint.update(
                serde_json::to_vec(&(bound, original_sql))
                    .map_err(|_| AppError::internal("Could not fingerprint bound parameters"))?,
            ),
            Action::SnowflakeDml {
                request,
                bound,
                database,
                mode,
            } => fingerprint.update(
                serde_json::to_vec(&(request, bound, database, mode))
                    .map_err(|_| AppError::internal("Could not fingerprint reviewed DML"))?,
            ),
            Action::TransactionControl { request } => fingerprint.update(
                serde_json::to_vec(request)
                    .map_err(|_| AppError::internal("Could not fingerprint transaction action"))?,
            ),
            Action::SnowflakeDdl {
                request,
                preflight,
                mode,
            } => fingerprint.update(
                serde_json::to_vec(&(request, preflight, mode))
                    .map_err(|_| AppError::internal("Could not fingerprint reviewed DDL"))?,
            ),
            #[cfg(unix)]
            Action::SnowflakeGrid {
                target,
                digest,
                mode,
                ..
            } => fingerprint
                .update(serde_json::to_vec(&(target, digest, mode)).map_err(|_| {
                    AppError::internal("Could not fingerprint reviewed grid edits")
                })?),
        }
        let digest = hex::encode(fingerprint.finalize());
        plans.insert(
            plan.id.clone(),
            StoredPlan {
                plan: plan.clone(),
                action,
                deadline: Instant::now() + Duration::from_secs(60),
                approval: None,
                digest,
                bytes,
            },
        );
        Ok(plan)
    }

    pub fn authorize(
        &self,
        plan_id: &str,
        passphrase: Option<&str>,
        confirmation: Option<&str>,
    ) -> AppResult<String> {
        self.authorize_plan(plan_id, passphrase, confirmation, None, false)
    }

    fn authorize_plan(
        &self,
        plan_id: &str,
        passphrase: Option<&str>,
        confirmation: Option<&str>,
        target_confirmation: Option<&str>,
        ddl: bool,
    ) -> AppResult<String> {
        let _configuration = self.configuration_mutation_guard()?;
        let mut plans = self.plans.lock();
        let stored = plans.get_mut(plan_id).ok_or_else(expired)?;
        if stored.deadline <= Instant::now() {
            return Err(expired());
        }
        let plan = &stored.plan;
        if ddl != matches!(stored.action, Action::SnowflakeDdl { .. }) {
            return Err(AppError::new(
                ErrorCode::ApprovalRequired,
                "Use the dedicated Snowflake DDL review to confirm this exact target",
            ));
        }
        if plan
            .target_confirmation_text
            .as_deref()
            .is_some_and(|expected| Some(expected) != target_confirmation)
        {
            return Err(AppError::new(
                ErrorCode::ApprovalRequired,
                "Type the exact fully qualified target to confirm this destructive operation",
            ));
        }
        if plan.requires_passphrase {
            self.verify_passphrase(passphrase)?;
        }
        if plan
            .confirmation_text
            .as_deref()
            .is_some_and(|expected| Some(expected) != confirmation)
        {
            return Err(AppError::new(
                ErrorCode::ApprovalRequired,
                "Type the exact connection name to confirm this operation",
            ));
        }
        self.session(&plan.session_id)?;
        let id = new_id();
        stored.approval = Some(id.clone());
        Ok(id)
    }

    pub fn execute(
        self: &Arc<Self>,
        plan_id: &str,
        approval: Option<&str>,
        events: EventHandler,
    ) -> AppResult<String> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let _configuration = self.configuration_mutation_guard()?;
        let recovery_activity = self.retain_recovery_activity()?;
        if self.dml_tasks.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "Joinery is closing; no query was started",
            ));
        }
        let stored = self.plans.lock().remove(plan_id).ok_or_else(expired)?;
        if stored.deadline <= Instant::now() {
            return Err(expired());
        }
        if matches!(
            stored.action,
            Action::Browse { .. }
                | Action::TransactionControl { .. }
                | Action::SnowflakeDml { .. }
                | Action::SnowflakeDdl { .. }
        ) {
            return Err(AppError::invalid(
                "Use the dedicated typed command to execute this prepared action",
            ));
        }
        #[cfg(unix)]
        if matches!(
            stored.action,
            Action::SnowflakeGrid { .. } | Action::SnowflakeConsole { .. }
        ) {
            return Err(AppError::invalid(
                "Use the dedicated typed command to execute this prepared action",
            ));
        }
        if stored.plan.requires_approval
            && (stored.approval.is_none() || stored.approval.as_deref() != approval)
        {
            return Err(AppError::new(
                ErrorCode::ApprovalRequired,
                "Review and authorize this exact query plan before running it",
            ));
        }
        let session = self.session(&stored.plan.session_id)?;
        let run_id = new_id();
        let token = CancellationToken::new();
        let delivery_token = token.clone();
        let events: EventHandler = Arc::new(move |event| {
            let delivered =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| events(event)))
                    .unwrap_or(false);
            if !delivered {
                delivery_token.cancel();
            }
            delivered
        });
        if self.jobs.lock().len() >= 32 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Too many query jobs are already running",
            ));
        }
        self.flush_pending_history()?;
        self.flush_pending_snowflake_receipts()?;
        #[cfg(unix)]
        self.flush_snowflake_grid_outcomes()?;
        let memory = self.outcome_memory.ordinary()?;
        let profile = self.persistence.profile(&session.profile.id)?;
        let seed = HistoryEntry {
            provenance: HistoryProvenance::Admitted,
            execution_summary: None,
            snowflake_dml: None,
            snowflake_ddl: None,
            id: run_id.clone(),
            connection_id: profile.id.clone(),
            connection_name: profile.name.clone(),
            started_at: now_ms(),
            sql: match &stored.action {
                Action::Sql { original_sql, .. } if profile.store_query_text => {
                    original_sql.clone()
                }
                _ => None,
            },
            statement_count: stored.plan.statements.len(),
            status: None,
            elapsed_ms: None,
        };
        let cell = self.admit_outcome(
            memory,
            Some(&seed),
            crate::persistence::OutcomeDescriptor {
                operation_id: run_id.clone(),
                connection_id: profile.id.clone(),
                session_id: session.info.id.clone(),
                started_at: seed.started_at,
                digest: stored.digest.clone(),
                kind: crate::persistence::OutcomeKind::Execute {
                    planned: seed.statement_count,
                },
            },
            crate::persistence::OutcomePool::Ordinary,
        )?;
        let cancellation_sent = Arc::new(AtomicBool::new(false));
        self.jobs.lock().insert(
            run_id.clone(),
            Job {
                session_id: session.info.id.clone(),
                token: token.clone(),
                cancellation_sent: cancellation_sent.clone(),
            },
        );
        let studio = self.clone();
        let run = run_id.clone();
        // Build before spawning: even an unpolled future owns its drop guard.
        let completion = Arc::new(Mutex::new(None));
        let mut owner = history_receipts::RunOwner::new(
            studio.clone(),
            session.clone(),
            cell.clone(),
            events.clone(),
            completion.clone(),
        );
        self.dml_tasks.spawn(async move {
            let _recovery_activity = recovery_activity;
            studio
                .run_plan(
                    run,
                    session,
                    stored,
                    token,
                    cancellation_sent,
                    RunRecording {
                        events,
                        cell,
                        completion,
                    },
                )
                .await;
            owner.disarm();
        });
        Ok(run_id)
    }

    async fn run_plan(
        self: Arc<Self>,
        run_id: String,
        session: Arc<Session>,
        stored: StoredPlan,
        token: CancellationToken,
        cancellation_sent: Arc<AtomicBool>,
        recording: RunRecording,
    ) {
        let RunRecording {
            events,
            cell,
            completion: observed_completion,
        } = recording;
        let started = Instant::now();
        let mut status = RunStatus::Completed;
        let mut execution_summary =
            matches!(&stored.action, Action::Sql { .. }).then(|| ExecutionSummary {
                planned: stored.plan.statements.len(),
                attempted: 0,
                completed: 0,
                not_attempted: stored.plan.statements.len(),
                stop_reason: None,
            });
        let serialized = !matches!(&stored.action, Action::CancelProcess { .. });
        let _lane = if serialized {
            Some(session.lane.lock().await)
        } else {
            None
        };
        let active_started = Instant::now();
        let queue_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let mut first_row_ms = None;
        if serialized {
            *session.completion.lock() = None;
            let _gate = session.cancellation_gate.lock().await;
            *session.active_run.lock() = Some(run_id.clone());
        }
        if !events(QueryEvent::Started {
            run_id: run_id.clone(),
        }) {
            token.cancel();
        }
        // Observe on the same serialized editor lane as the plan itself. An
        // unavailable observation must never fall back to frontend cached state.
        if matches!(stored.action, Action::Sql { .. })
            && stored.plan.statements.len() == 1
            && ([
                "EXPLAIN (FORMAT JSON)\n",
                "EXPLAIN (ANALYZE, FORMAT JSON)\n",
                "EXPLAIN FORMAT=JSON\n",
                "ANALYZE FORMAT=JSON\n",
                "EXPLAIN ANALYZE FORMAT=TREE\n",
            ]
            .iter()
            .any(|prefix| stored.plan.statements[0].sql.starts_with(prefix))
                || (stored.plan.statements[0]
                    .sql
                    .starts_with("EXPLAIN USING JSON\n")
                    && snowflake_dml::sdk_profile(&session.profile)))
        {
            let state = if token.is_cancelled() || session.closed.load(Ordering::Acquire) {
                None
            } else {
                session.driver.session_state(&session.info.id).await.ok()
            };
            if let Some(state) = &state {
                self.publish_observed_runtime(&session, state);
            }
            if !events(QueryEvent::PlanContext {
                run_id: run_id.clone(),
                state,
            }) {
                token.cancel();
            }
        }
        match &stored.action {
            Action::Sql {
                bound,
                error_source,
                ..
            } => {
                for (index, statement) in stored.plan.statements.iter().enumerate() {
                    if token.is_cancelled() || session.closed.load(Ordering::Acquire) {
                        status = RunStatus::Cancelled;
                        break;
                    }
                    if !events(QueryEvent::StatementStarted {
                        run_id: run_id.clone(),
                        index,
                    }) {
                        status = RunStatus::Cancelled;
                        break;
                    }
                    let clock = Instant::now();
                    if session.profile.engine == EngineKind::Mysql
                        && matches!(statement.kind, StatementKind::Ddl | StatementKind::Unknown)
                    {
                        // Approved schema and unclassified operations (including
                        // dialect DDL the parser cannot classify) must not wait
                        // on leases retained by another application console.
                        let drivers = self
                            .sessions
                            .lock()
                            .values()
                            .filter(|s| s.profile.engine == EngineKind::Mysql)
                            .map(|s| s.driver.clone())
                            .collect::<Vec<_>>();
                        let releases = futures_util::future::join_all(
                            drivers
                                .iter()
                                .map(|driver| driver.invalidate_edit_targets()),
                        )
                        .await;
                        if let Some(error) = releases.into_iter().find_map(Result::err) {
                            // SQL has not been submitted: this is not an unknown
                            // write outcome, even if lease teardown lost a socket.
                            status = if error.code == ErrorCode::Cancelled {
                                RunStatus::Cancelled
                            } else {
                                RunStatus::Failed
                            };
                            events(QueryEvent::Error {
                                error,
                                statement_index: index,
                            });
                            break;
                        }
                        if token.is_cancelled() || session.closed.load(Ordering::Acquire) {
                            status = RunStatus::Cancelled;
                            break;
                        }
                    }
                    let provenance = self.resolve_statement_provenance(&session, statement).await;
                    let mut sink = Output {
                        studio: &self,
                        session_id: &session.info.id,
                        run_id: &run_id,
                        index,
                        provenance,
                        current: None,
                        truncated: false,
                        viewer_closed: false,
                        active_started,
                        first_row_ms: &mut first_row_ms,
                        events: &events,
                    };
                    // StatementStarted precedes local preflight. Count only
                    // driver invocations; this does not prove remote submission.
                    let summary = execution_summary.as_mut().expect("SQL execution summary");
                    summary.attempted += 1;
                    summary.not_attempted -= 1;
                    let outcome = if let Some(b) = bound.get(index).and_then(Option::as_ref) {
                        session
                            .driver
                            .execute_bound(&statement.sql, statement.kind, b, &mut sink)
                            .await
                    } else {
                        session
                            .driver
                            .execute(&statement.sql, statement.kind, &mut sink)
                            .await
                    };
                    match outcome {
                        Ok(affected) => {
                            summary.completed += 1;
                            events(QueryEvent::StatementCompleted {
                                run_id: run_id.clone(),
                                index,
                                affected_rows: affected.to_string(),
                                elapsed_ms: clock.elapsed().as_millis() as u32,
                            });
                        }
                        Err(mut error) => {
                            sink.fail();
                            if matches!(
                                error.code,
                                ErrorCode::ConnectionClosed | ErrorCode::ConnectionFailed
                            ) && safety::needs_authorization(statement)
                            {
                                status = RunStatus::OutcomeUnknown;
                                let mut unknown = AppError::new(
                                    ErrorCode::OutcomeUnknown,
                                    "The connection was lost during a write. Verify its outcome before retrying.",
                                );
                                if let Some(stmt_span) = statement.span {
                                    unknown.location = Some(Box::new(ErrorLocation {
                                        span: stmt_span,
                                        statement_index: Some(index),
                                        sql_state: None,
                                        detail: None,
                                        hint: None,
                                    }));
                                }
                                if let Some(source) = error_source {
                                    source.map_error(&mut unknown);
                                }
                                events(QueryEvent::Error {
                                    error: unknown,
                                    statement_index: index,
                                });
                            } else {
                                status = if error.code == ErrorCode::OutcomeUnknown {
                                    RunStatus::OutcomeUnknown
                                } else if error.code == ErrorCode::Cancelled {
                                    RunStatus::Cancelled
                                } else {
                                    RunStatus::Failed
                                };
                                if error.code != ErrorCode::Cancelled {
                                    if let Some(mut loc) = error.location.take() {
                                        if let Some(stmt_span) = statement.span {
                                            loc.span = crate::sql_text::shift_child_span(
                                                stmt_span, loc.span,
                                            );
                                        }
                                        loc.statement_index = Some(index);
                                        error.location = Some(loc);
                                    } else if let Some(stmt_span) = statement.span {
                                        error.location = Some(Box::new(ErrorLocation {
                                            span: stmt_span,
                                            statement_index: Some(index),
                                            sql_state: None,
                                            detail: None,
                                            hint: None,
                                        }));
                                    }
                                }
                                if let Some(source) = error_source {
                                    source.map_error(&mut error);
                                }
                                events(QueryEvent::Error {
                                    error,
                                    statement_index: index,
                                });
                            }
                            break;
                        }
                    }
                    if sink.truncated && statement.kind == StatementKind::Read {
                        summary.stop_reason = Some(ExecutionStopReason::ResultLimit);
                        if summary.not_attempted > 0 {
                            status = RunStatus::Stopped;
                        }
                        break;
                    }
                }
            }
            Action::Changes {
                schema,
                identity,
                rows,
                anchor_id,
                updates,
                deletes,
                has_inserts,
                ..
            } => {
                if token.is_cancelled()
                    || session.closed.load(Ordering::Acquire)
                    || session.driver.is_closed()
                {
                    status = RunStatus::Cancelled;
                } else {
                    match session
                        .driver
                        .apply_target(schema, identity.as_ref(), rows)
                        .await
                    {
                        Ok(()) => {
                            for result in self.results.apply_committed_changes(
                                anchor_id.as_deref(),
                                updates,
                                deletes,
                                *has_inserts,
                            ) {
                                events(QueryEvent::Result { result });
                            }
                        }
                        Err(error) => {
                            status = if error.code == ErrorCode::Cancelled {
                                RunStatus::Cancelled
                            } else if error.code == ErrorCode::OutcomeUnknown {
                                RunStatus::OutcomeUnknown
                            } else {
                                RunStatus::Failed
                            };
                            events(QueryEvent::Error {
                                error,
                                statement_index: 0,
                            });
                        }
                    }
                }
            }
            Action::CancelProcess { id, identity } => {
                if let Err(error) = session.driver.cancel_process(id, identity).await {
                    status = RunStatus::Failed;
                    events(QueryEvent::Error {
                        error,
                        statement_index: 0,
                    });
                }
            }
            #[cfg(unix)]
            Action::SnowflakeGrid { .. } | Action::SnowflakeConsole { .. } => {}
            Action::Browse { .. }
            | Action::TransactionControl { .. }
            | Action::SnowflakeDml { .. }
            | Action::SnowflakeDdl { .. } => {}
        }
        if let Some(summary) = &mut execution_summary {
            // Preserve the actual stop cause, including failures/cancellation
            // before any driver invocation and unknown outcomes after a prefix.
            if summary.stop_reason.is_none() {
                summary.stop_reason = match status {
                    RunStatus::Cancelled => Some(ExecutionStopReason::Cancelled),
                    RunStatus::Failed => Some(ExecutionStopReason::Failed),
                    RunStatus::OutcomeUnknown => Some(ExecutionStopReason::OutcomeUnknown),
                    RunStatus::Completed | RunStatus::Stopped => None,
                };
            }
        }
        let ended = Instant::now();
        let elapsed_ms = ended
            .duration_since(started)
            .as_millis()
            .min(u32::MAX as u128) as u32;
        let completion = HistoryCompletion {
            provenance: HistoryProvenance::Recorded,
            execution_summary: execution_summary.clone(),
            snowflake_dml: None,
            snowflake_ddl: None,
            status: Some(status),
            elapsed_ms: Some(elapsed_ms),
        };
        let recording_result = self.finish_outcome_history(&cell, &completion);
        *observed_completion.lock() = Some(completion);
        if let Err(error) = recording_result {
            events(QueryEvent::Notice {
                message: error.message,
            });
        }
        if serialized {
            let observed_state = if matches!(stored.action, Action::Sql { .. })
                && !token.is_cancelled()
                && !cancellation_sent.load(Ordering::Acquire)
                && !session.driver.is_closed()
                && !session.closed.load(Ordering::Acquire)
            {
                session.driver.session_state(&session.info.id).await.ok()
            } else {
                None
            };
            let _gate = session.cancellation_gate.lock().await;
            // PostgreSQL's cancel protocol has no acknowledgement. Discard a session
            // after an explicit cancellation so a late request cannot hit the next run.
            let mut reason = None;
            if cancellation_sent.load(Ordering::Acquire) {
                session.closed.store(true, Ordering::Release);
                session.driver.close();
                reason = Some(if status == RunStatus::Completed {
                    "The operation completed before cancellation took effect. Its reported outcome still applies. Reconnect before running another query."
                } else {
                    "Cancellation was requested. Review the reported statement outcomes before retrying; reconnect before running another query."
                });
            } else if session.driver.is_closed() {
                session.closed.store(true, Ordering::Release);
                reason = Some("This database session closed. Reconnect to run another query.");
            }
            if let Some(reason) = reason {
                self.sessions.lock().remove(&session.info.id);
                self.import_targets
                    .lock()
                    .retain(|(session_id, _, _), _| session_id != &session.info.id);
                self.plans
                    .lock()
                    .retain(|_, p| p.plan.session_id != session.info.id);
                events(QueryEvent::SessionClosed {
                    session_id: session.info.id.clone(),
                    reason: reason.into(),
                });
            } else if let Some(state) = observed_state {
                self.publish_observed_runtime(&session, &state);
                events(QueryEvent::SessionState { state });
            }
            *session.active_run.lock() = None;
        }
        events(QueryEvent::RunMetrics {
            run_id: run_id.clone(),
            queue_ms,
            active_ms: ended
                .duration_since(active_started)
                .as_millis()
                .min(u32::MAX as u128) as u32,
            first_row_ms,
            total_ms: elapsed_ms,
        });
        self.jobs.lock().remove(&run_id);
        events(QueryEvent::Finished {
            execution_summary,
            run_id,
            status,
            elapsed_ms,
        });
    }

    pub async fn cancel(&self, run_id: &str) -> AppResult<()> {
        let (session_id, cancellation_sent) = {
            let jobs = self.jobs.lock();
            let job = jobs.get(run_id).ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "This query has already finished")
            })?;
            job.token.cancel();
            (job.session_id.clone(), job.cancellation_sent.clone())
        };
        if self.console.is_run(run_id) {
            // The retained owner cancels only the original helper operation and records its ACK.
            return Ok(());
        }
        let session = self.session(&session_id)?;
        let _gate = session.cancellation_gate.lock().await;
        if session.active_run.lock().as_deref() != Some(run_id) {
            return Ok(());
        }
        cancellation_sent.store(true, Ordering::Release);
        session.driver.cancel().await
    }

    pub async fn session_contexts(&self, session_id: &str) -> AppResult<SessionContextOptions> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.session_contexts_with_refresh(session_id, false).await
    }

    pub async fn session_contexts_with_refresh(
        &self,
        session_id: &str,
        resume_metadata: bool,
    ) -> AppResult<SessionContextOptions> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        let revision = session.metadata_gate.lock().revision.clone();
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Wait for the running query to finish first",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        session
            .metadata_gate
            .lock()
            .check(Some(&revision), resume_metadata)?;
        if resume_metadata && session.metadata_gate.lock().catalog_required {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "DDL metadata remains paused; explicitly reconcile the catalog to resume",
            )
            .with_completion_invalidated());
        }
        #[cfg(unix)]
        if resume_metadata {
            let _ = self.flush_snowflake_grid_outcomes();
        }
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let result = self.build_session_contexts(&session).await;
        if resume_metadata
            && snowflake_dml::sdk_profile(&session.profile)
            && result.as_ref().is_ok_and(|options| {
                !matches!(
                    options.state.transaction_observation,
                    Some(TransactionObservation::Observed { .. })
                ) || !matches!(
                    options.state.transaction_control_authority,
                    Some(TransactionControlAuthority::Observed { .. })
                )
            })
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The session could not be coherently observed; metadata remains paused",
            )
            .with_completion_invalidated());
        }
        if result.is_ok() && resume_metadata {
            session.metadata_gate.lock().held = false;
        }
        result
    }

    pub async fn switch_session_context(
        &self,
        session_id: &str,
        target: &str,
    ) -> AppResult<SessionContextOptions> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Wait for the running query to finish first",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        // A failed/lost response can still leave a changed server context.
        // Revoke every prepared target before dispatch, including on failure.
        *session.completion.lock() = None;
        session.driver.invalidate_relation_catalog();
        self.plans
            .lock()
            .retain(|_, p| p.plan.session_id != session.info.id);
        session.driver.switch_context(target).await?;
        let _ = self
            .persistence
            .audit(&session.profile.id, "switch-context", target, "completed");
        self.build_session_contexts(&session).await
    }

    pub async fn switch_session_schema(
        &self,
        session_id: &str,
        request: &SwitchSchemaRequest,
    ) -> AppResult<SessionContextOptions> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Wait for the running query to finish first",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        // Revoke targets before dispatch: an interrupted USE can change context.
        *session.observed_runtime.lock() = None;
        *session.completion.lock() = None;
        session.driver.invalidate_relation_catalog();
        self.plans
            .lock()
            .retain(|_, plan| plan.plan.session_id != session.info.id);
        let options = session.driver.switch_schema(session_id, request).await?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        if options.state.session_id != session.info.id || options.revision.is_none() {
            return Err(AppError::internal("Invalid schema switch observation"));
        }
        self.publish_observed_runtime(&session, &options.state);
        let _ = self.persistence.audit(
            &session.profile.id,
            "switch-schema",
            &request.schema,
            "completed",
        );
        // The driver already verified and published coherent options. A second
        // options read would add eleven SQL executions and revoke this revision.
        Ok(options)
    }

    pub async fn snowflake_session_contexts(
        &self,
        session_id: &str,
        resume_metadata: bool,
    ) -> AppResult<SnowflakeContextCatalog> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        let revision = session.metadata_gate.lock().revision.clone();
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Wait for the running query to finish first",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        session
            .metadata_gate
            .lock()
            .check(Some(&revision), resume_metadata)?;
        if resume_metadata && session.metadata_gate.lock().catalog_required {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "DDL metadata remains paused; explicitly reconcile the catalog to resume",
            )
            .with_completion_invalidated());
        }
        #[cfg(unix)]
        if resume_metadata {
            let _ = self.flush_snowflake_grid_outcomes();
        }
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        self.ddl_preflights.lock().remove(&session.info.id);
        let catalog = session
            .driver
            .snowflake_session_contexts(&session.info.id)
            .await;
        if catalog.is_err() && session.info.capabilities.relation_browsing {
            *session.observed_runtime.lock() = None;
            *session.completion.lock() = None;
            session.driver.invalidate_relation_catalog();
            self.plans
                .lock()
                .retain(|_, plan| plan.plan.session_id != session.info.id);
        }
        let catalog = catalog.inspect_err(|_| {
            if session.driver.is_closed() {
                self.disconnect(&session.info.id);
            }
        })?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        if resume_metadata
            && (!matches!(
                catalog.state.transaction_observation,
                Some(TransactionObservation::Observed { .. })
            ) || (!catalog.state.database.is_empty()
                && !matches!(
                    catalog.state.transaction_control_authority,
                    Some(TransactionControlAuthority::Observed { .. })
                )))
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The session could not be coherently observed; metadata remains paused",
            )
            .with_completion_invalidated());
        }
        if resume_metadata {
            session.metadata_gate.lock().held = false;
        }
        self.publish_observed_runtime(&session, &catalog.state);
        Ok(catalog)
    }

    pub async fn switch_snowflake_context(
        &self,
        session_id: &str,
        request: &SwitchSnowflakeContextRequest,
    ) -> AppResult<SnowflakeContextCatalog> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        let gate_revision = session.metadata_gate.lock().revision.clone();
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Wait for the running query to finish first",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        session
            .metadata_gate
            .lock()
            .check(Some(&gate_revision), false)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        self.ddl_preflights.lock().remove(&session.info.id);
        *session.observed_runtime.lock() = None;
        *session.completion.lock() = None;
        session.driver.invalidate_relation_catalog();
        self.plans
            .lock()
            .retain(|_, plan| plan.plan.session_id != session.info.id);
        let catalog = session
            .driver
            .switch_snowflake_context(session_id, request)
            .await
            .inspect_err(|_| {
                if session.driver.is_closed() {
                    self.disconnect(&session.info.id);
                }
            })?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        if catalog.state.session_id != session.info.id || catalog.revision.is_empty() {
            return Err(AppError::internal(
                "Invalid Snowflake context switch observation",
            ));
        }
        self.publish_observed_runtime(&session, &catalog.state);
        let audit_target = match &request.target {
            SnowflakeContextTarget::Role { role } => format!("role:{role}"),
            SnowflakeContextTarget::Warehouse { warehouse } => format!("warehouse:{warehouse}"),
            SnowflakeContextTarget::Database { database, schema } => match schema {
                Some(schema) => format!("database:{database}.{schema}"),
                None => format!("database:{database}"),
            },
            SnowflakeContextTarget::Schema { database, schema } => {
                format!("schema:{database}.{schema}")
            }
        };
        let _ = self.persistence.audit(
            &session.profile.id,
            "switch-snowflake-context",
            &audit_target,
            "completed",
        );
        Ok(catalog)
    }

    pub async fn set_transaction_mode(
        &self,
        session_id: &str,
        mode: TransactionMode,
    ) -> AppResult<SessionRuntimeState> {
        let _recovery_activity = self.begin_recovery_activity()?;
        if mode == TransactionMode::Unknown {
            return Err(AppError::invalid(
                "Unknown is not a requested transaction mode",
            ));
        }
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        if !session.info.capabilities.transactions {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Transactions are unavailable for this engine",
            ));
        }
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Wait for the running query to finish first",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        session.driver.set_transaction_mode(mode).await?;
        let label = match mode {
            TransactionMode::AutoCommit => "autoCommit",
            TransactionMode::Manual => "manual",
            TransactionMode::Unknown => {
                return Err(AppError::invalid(
                    "Unknown is not a requested transaction mode",
                ));
            }
        };
        let _ = self
            .persistence
            .audit(&session.profile.id, "transaction-mode", label, "completed");
        session.driver.session_state(&session.info.id).await
    }

    pub async fn commit_transaction(&self, session_id: &str) -> AppResult<SessionRuntimeState> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id).inspect_err(|error| {
            if error.code == ErrorCode::ConnectionClosed {
                self.disconnect(session_id);
            }
        })?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        if !session.info.capabilities.transactions {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Transactions are unavailable for this engine",
            ));
        }
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Wait for the running query to finish first",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            self.disconnect(&session.info.id);
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        session
            .driver
            .commit_transaction()
            .await
            .map_err(|error| self.transaction_control_error(&session, error))?;
        *session.completion.lock() = None;
        self.plans
            .lock()
            .retain(|_, p| p.plan.session_id != session.info.id);
        let _ = self.persistence.audit(
            &session.profile.id,
            "commit-transaction",
            &session.info.id,
            "completed",
        );
        session.driver.session_state(&session.info.id).await.map_err(|_| {
            self.disconnect(&session.info.id);
            AppError::new(
                ErrorCode::ConnectionClosed,
                "Transaction committed, but session context could not be refreshed. Reconnect before continuing.",
            )
        })
    }

    pub async fn rollback_transaction(&self, session_id: &str) -> AppResult<SessionRuntimeState> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id).inspect_err(|error| {
            if error.code == ErrorCode::ConnectionClosed {
                self.disconnect(session_id);
            }
        })?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        if !session.info.capabilities.transactions {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Transactions are unavailable for this engine",
            ));
        }
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Wait for the running query to finish first",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_secs(5), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            self.disconnect(&session.info.id);
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        session
            .driver
            .rollback_transaction()
            .await
            .map_err(|error| self.transaction_control_error(&session, error))?;
        *session.completion.lock() = None;
        self.plans
            .lock()
            .retain(|_, p| p.plan.session_id != session.info.id);
        let _ = self.persistence.audit(
            &session.profile.id,
            "rollback-transaction",
            &session.info.id,
            "completed",
        );
        session.driver.session_state(&session.info.id).await.map_err(|_| {
            self.disconnect(&session.info.id);
            AppError::new(
                ErrorCode::ConnectionClosed,
                "Transaction rolled back, but session context could not be refreshed. Reconnect before continuing.",
            )
        })
    }

    fn transaction_control_error(&self, session: &Session, mut error: AppError) -> AppError {
        // A deferred constraint failure can end the transaction and undo SET
        // LOCAL context even though the connection remains usable.
        if error.code == ErrorCode::Database {
            *session.completion.lock() = None;
            self.plans
                .lock()
                .retain(|_, p| p.plan.session_id != session.info.id);
        }
        if matches!(
            error.code,
            ErrorCode::ConnectionClosed | ErrorCode::ConnectionFailed | ErrorCode::OutcomeUnknown
        ) {
            self.disconnect(&session.info.id);
            if error.code == ErrorCode::ConnectionFailed {
                error.code = ErrorCode::ConnectionClosed;
            }
        }
        error
    }

    async fn build_session_contexts(
        &self,
        session: &Arc<Session>,
    ) -> AppResult<SessionContextOptions> {
        #[cfg(unix)]
        self.snowflake_grid
            .require_ordinary_session(&session.info.id)?;
        self.ddl_preflights.lock().remove(&session.info.id);
        let options = session
            .driver
            .session_context_options(&session.info.id)
            .await;
        if options.is_err() && session.info.capabilities.relation_browsing {
            *session.observed_runtime.lock() = None;
            *session.completion.lock() = None;
            session.driver.invalidate_relation_catalog();
            self.plans
                .lock()
                .retain(|_, plan| plan.plan.session_id != session.info.id);
        }
        let options = options?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        self.publish_observed_runtime(session, &options.state);
        Ok(options)
    }

    /// Called on the serialized editor lane before exposing SDK observations.
    fn publish_observed_runtime(&self, session: &Arc<Session>, state: &SessionRuntimeState) {
        if state.transaction_observation.is_none() {
            return;
        }
        let mut previous = session.observed_runtime.lock();
        let mut observed = state.clone();
        observed.transaction_control_authority = None;
        let previous_observed = previous.as_ref().map(|state| {
            let mut state = state.clone();
            state.transaction_control_authority = None;
            state
        });
        if previous_observed.as_ref() != Some(&observed)
            || matches!(
                state.transaction_observation,
                Some(TransactionObservation::Unavailable)
            )
        {
            *session.completion.lock() = None;
            session.driver.invalidate_relation_catalog();
            self.plans
                .lock()
                .retain(|_, plan| plan.plan.session_id != session.info.id);
        }
        *previous = Some(state.clone());
    }

    /// Read a bounded context snapshot on the existing editor execution lane.
    pub async fn completion_catalog(&self, session_id: &str) -> AppResult<CompletionCatalog> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        self.ddl_preflights.lock().remove(session_id);
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The session is busy; retry after execution finishes",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_millis(1500), session.lane.lock())
            .await
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Conflict,
                    "The session is busy; retry suggestions",
                )
            })?;
        session.metadata_gate.lock().check(None, false)?;
        let previous = session.completion.lock().take();
        let identity = session.driver.completion_identity().await?;
        let relations = session.driver.completion_catalog(&identity).await?;
        let tables = relations.tables;
        let unqualified_tables = relations.unqualified_tables;
        let revision = previous
            .filter(|old| {
                old.identity == identity
                    && old.unqualified_tables.len() == unqualified_tables.len()
                    && old
                        .unqualified_tables
                        .iter()
                        .zip(&unqualified_tables)
                        .all(|(a, b)| a.schema == b.schema && a.name == b.name)
                    && old.tables.len() == tables.len()
                    && old.tables.iter().zip(&tables).all(|(a, b)| {
                        a.table.schema == b.table.schema
                            && a.table.name == b.table.name
                            && a.kind == b.kind
                    })
            })
            .map(|old| old.revision)
            .unwrap_or_else(new_id);
        let snapshot = CompletionCatalog {
            revision,
            identity,
            tables,
            unqualified_tables,
        };
        if serde_json::to_vec(&snapshot)
            .map_err(|_| AppError::internal("Cannot encode completion metadata"))?
            .len()
            > 2 * 1024 * 1024
        {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Completion metadata exceeds its byte budget",
            ));
        }
        if session.closed.load(Ordering::Acquire) {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "The session closed",
            ));
        }
        *session.completion.lock() = Some(snapshot.clone());
        Ok(snapshot)
    }
    pub async fn completion_schema(
        &self,
        session_id: &str,
        revision: &str,
        table: &TableRef,
    ) -> AppResult<TableSchema> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.editor_schema(session_id, revision, table, false).await
    }

    pub async fn inspection_schema(
        &self,
        session_id: &str,
        revision: &str,
        table: &TableRef,
    ) -> AppResult<TableSchema> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.editor_schema(session_id, revision, table, true).await
    }

    async fn editor_schema(
        &self,
        session_id: &str,
        revision: &str,
        table: &TableRef,
        require_complete: bool,
    ) -> AppResult<TableSchema> {
        let session = self
            .session(session_id)
            .map_err(AppError::with_completion_invalidated)?;
        #[cfg(unix)]
        self.snowflake_grid
            .require_ordinary_session(session_id)
            .map_err(AppError::with_completion_invalidated)?;
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The session is busy; retry after execution finishes",
            ));
        }
        let _lane = tokio::time::timeout(Duration::from_millis(1500), session.lane.lock())
            .await
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Conflict,
                    "The session is busy; retry suggestions",
                )
            })?;
        session.metadata_gate.lock().check(None, false)?;
        let snapshot = session
            .completion
            .lock()
            .clone()
            .filter(|s| s.revision == revision)
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::Conflict,
                    "Completion context changed; refresh suggestions",
                )
                .with_completion_invalidated()
            })?;
        let observed = session
            .driver
            .completion_identity()
            .await
            .map_err(|error| {
                *session.completion.lock() = None;
                error.with_completion_invalidated()
            })?;
        if observed != snapshot.identity {
            *session.completion.lock() = None;
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Completion context changed; refresh suggestions",
            )
            .with_completion_invalidated());
        }
        if !snapshot
            .tables
            .iter()
            .any(|t| t.table.schema == table.schema && t.table.name == table.name)
        {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "The table is not visible in this completion context",
            ));
        }
        let mut schema = if require_complete {
            session.driver.inspection_schema(table).await
        } else {
            session.driver.completion_schema(table).await
        }
        .inspect_err(|error| {
            if error.completion_invalidated == Some(true) {
                *session.completion.lock() = None;
            }
        })?;
        // A driver may need several reads for columns. Never publish their
        // result under an identity that changed while those reads ran.
        let observed = session
            .driver
            .completion_identity()
            .await
            .map_err(|error| {
                *session.completion.lock() = None;
                error.with_completion_invalidated()
            })?;
        if observed != snapshot.identity {
            *session.completion.lock() = None;
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Completion context changed; refresh suggestions",
            )
            .with_completion_invalidated());
        }
        if let Ok(vfks) = self.persistence.virtual_foreign_keys() {
            merge_virtual_fks_into_table_schema(&mut schema, &vfks, &session.profile.id, true);
        }
        if schema.columns.len() > 4096
            || serde_json::to_vec(&schema)
                .map_err(|_| AppError::internal("Cannot encode completion columns"))?
                .len()
                > 1024 * 1024
        {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Completion columns exceed their budget",
            ));
        }
        if session.closed.load(Ordering::Acquire) {
            *session.completion.lock() = None;
            return Err(
                AppError::new(ErrorCode::ConnectionClosed, "The session closed")
                    .with_completion_invalidated(),
            );
        }
        Ok(schema)
    }
    async fn relation_lane(session: &Session) -> AppResult<tokio::sync::MutexGuard<'_, ()>> {
        if !session.info.capabilities.relation_browsing {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Session relation browsing is unavailable for this engine",
            ));
        }
        let lane = tokio::time::timeout(Duration::from_millis(1500), session.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            session.driver.invalidate_relation_catalog();
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "This database session is disconnected",
            ));
        }
        if session.active_run.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Wait for the running query to finish first",
            ));
        }
        session.metadata_gate.lock().check(None, false)?;
        Ok(lane)
    }

    fn publish_relation<T>(session: &Session, result: AppResult<T>) -> AppResult<T> {
        if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
            session.driver.invalidate_relation_catalog();
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "The database session closed while reading metadata",
            ));
        }
        result
    }

    pub async fn relation_catalog(&self, session_id: &str) -> AppResult<BrowseCatalog> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        self.ddl_preflights.lock().remove(session_id);
        // Refresh revokes targets even if the outer console lane is busy.
        // Never let a failed refresh leave a previously granted target usable.
        session.driver.invalidate_relation_catalog();
        let _lane = Self::relation_lane(&session).await?;
        Self::publish_relation(&session, session.driver.relation_catalog().await)
    }

    pub async fn inspect_relation(
        &self,
        session_id: &str,
        request: &InspectRelationRequest,
    ) -> AppResult<RelationInspection> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let _lane = Self::relation_lane(&session).await?;
        Self::publish_relation(&session, session.driver.inspect_relation(request).await)
    }

    pub async fn define_relation(
        &self,
        session_id: &str,
        request: &DefineRelationRequest,
    ) -> AppResult<RelationDefinition> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let _lane = Self::relation_lane(&session).await?;
        Self::publish_relation(&session, session.driver.define_relation(request).await)
    }

    pub async fn inspect_relation_constraints(
        &self,
        session_id: &str,
        request: &InspectRelationConstraintsRequest,
    ) -> AppResult<RelationConstraints> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let _lane = Self::relation_lane(&session).await?;
        Self::publish_relation(
            &session,
            session.driver.inspect_relation_constraints(request).await,
        )
    }

    pub async fn inspect_relation_check_constraints(
        &self,
        session_id: &str,
        request: &InspectRelationCheckConstraintsRequest,
    ) -> AppResult<RelationCheckConstraints> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let _lane = Self::relation_lane(&session).await?;
        Self::publish_relation(
            &session,
            session
                .driver
                .inspect_relation_check_constraints(request)
                .await,
        )
    }

    pub async fn inspect_relation_constraint_mappings(
        &self,
        session_id: &str,
        request: &InspectRelationConstraintMappingsRequest,
    ) -> AppResult<RelationConstraintMappings> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let _lane = Self::relation_lane(&session).await?;
        Self::publish_relation(
            &session,
            session
                .driver
                .inspect_relation_constraint_mappings(request)
                .await,
        )
    }

    pub async fn preview_relation(
        &self,
        session_id: &str,
        request: &PreviewRelationRequest,
    ) -> AppResult<RelationPreview> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let _lane = Self::relation_lane(&session).await?;
        Self::publish_relation(&session, session.driver.preview_relation(request).await)
    }

    pub async fn catalog(&self, session_id: &str) -> AppResult<Vec<TableInfo>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let _lane = if snowflake_dml::sdk_profile(&session.profile) {
            Some(Self::relation_lane(&session).await?)
        } else {
            None
        };
        session.driver.catalog().await
    }
    pub async fn database_objects(
        &self,
        session_id: &str,
    ) -> AppResult<Vec<DatabaseObjectSummary>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let _lane = if snowflake_dml::sdk_profile(&session.profile) {
            Some(Self::relation_lane(&session).await?)
        } else {
            None
        };
        let result = session.driver.database_objects().await;
        let mut objects = if snowflake_dml::sdk_profile(&session.profile) {
            Self::publish_relation(&session, result)?
        } else {
            result?
        };
        if objects.len() > 10_000 {
            objects.truncate(10_000);
        }
        Ok(objects)
    }
    pub async fn object_details(
        &self,
        session_id: &str,
        object: &DatabaseObjectRef,
    ) -> AppResult<ObjectDetails> {
        let _recovery_activity = self.begin_recovery_activity()?;
        const MAX_OBJECT_DDL_BYTES: usize = 256 * 1024;
        const MAX_OBJECT_DEPS: usize = 200;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let _lane = if snowflake_dml::sdk_profile(&session.profile) {
            Some(Self::relation_lane(&session).await?)
        } else {
            None
        };
        let result = session.driver.object_details(object).await;
        let mut details = if snowflake_dml::sdk_profile(&session.profile) {
            Self::publish_relation(&session, result)?
        } else {
            result?
        };
        details.ddl = details.ddl.capped(MAX_OBJECT_DDL_BYTES);
        details.uses = finalize_dependency_edges(&details.object, details.uses, MAX_OBJECT_DEPS);
        details.used_by =
            finalize_dependency_edges(&details.object, details.used_by, MAX_OBJECT_DEPS);
        Ok(details)
    }
    pub async fn schema(&self, session_id: &str, table: &TableRef) -> AppResult<TableSchema> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let mut schema = self.schema_for_session(&session, table).await?;
        if let Ok(vfks) = self.persistence.virtual_foreign_keys() {
            merge_virtual_fks_into_table_schema(&mut schema, &vfks, &session.profile.id, true);
        }
        Ok(schema)
    }
    async fn resolve_statement_provenance(
        &self,
        session: &Session,
        statement: &StatementPlan,
    ) -> StatementProvenance {
        if !matches!(
            session.profile.engine,
            EngineKind::Postgres | EngineKind::Mysql
        ) || !session.info.capabilities.table_editing
        {
            return StatementProvenance::None;
        }
        if statement.kind != StatementKind::Read {
            return StatementProvenance::ReadOnly {
                reason: "Statement is not a SELECT query".into(),
            };
        }
        const UNQUALIFIED_SENTINEL: &str = "\0joinery_unqualified\0";
        let mut candidate = match safety::analyze_single_table_select(
            session.profile.engine,
            &statement.sql,
            Some(UNQUALIFIED_SENTINEL),
        ) {
            Ok(candidate) => candidate,
            Err(reason) => return StatementProvenance::ReadOnly { reason },
        };
        let safe_mode = safety::effective_mode(&session.profile);
        if safe_mode == SafeMode::ReadOnly {
            return StatementProvenance::ReadOnly {
                reason: "Connection is in read-only mode".into(),
            };
        }
        // The active schema is not necessarily the relation selected by SQL:
        // PostgreSQL searches later schemas, and temporary tables can shadow
        // permanent tables on both engines. Resolve on the editor connection
        // while its execution lane is held; grid metadata uses another login.
        candidate.table = match session.driver.resolve_query_edit_table(
            &candidate.table,
            candidate.table.schema == UNQUALIFIED_SENTINEL,
        ).await {
            Ok(table) => table,
            Err(_) => return StatementProvenance::ReadOnly {
                reason: "Query selects a temporary relation or its base-table identity cannot be verified".into(),
            },
        };
        match session.driver.edit_target(&candidate.table).await {
            Ok(target) => StatementProvenance::Candidate {
                candidate,
                live_schema: Box::new(target.schema),
                identity: target.identity,
                safe_mode,
            },
            Err(_) => StatementProvenance::ReadOnly {
                reason: format!(
                    "Table \"{}.{}\" is not an editable base table or its schema is unavailable",
                    candidate.table.schema, candidate.table.name
                ),
            },
        }
    }
    pub async fn referencing_foreign_keys(
        &self,
        session_id: &str,
        table: &TableRef,
    ) -> AppResult<Vec<InboundForeignKey>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        let mut keys = session.driver.referencing_keys(table).await?;
        if let Ok(vfks) = self.persistence.virtual_foreign_keys() {
            merge_virtual_fks_into_inbound(&mut keys, &vfks, &session.profile.id, table);
        }
        Ok(keys)
    }
    pub async fn schema_diagram(&self, session_id: &str, schema: &str) -> AppResult<SchemaDiagram> {
        let _recovery_activity = self.begin_recovery_activity()?;
        if schema.is_empty() || schema.len() > 1024 || schema.contains('\0') {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Choose a schema to diagram",
            ));
        }
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        if !session.info.capabilities.schema_diagram {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Schema diagrams are not supported for this database engine",
            ));
        }
        let _lane = if snowflake_dml::sdk_profile(&session.profile) {
            Some(Self::relation_lane(&session).await?)
        } else {
            None
        };
        let result = session.driver.schema_diagram(schema).await;
        let mut diagram = if snowflake_dml::sdk_profile(&session.profile) {
            Self::publish_relation(&session, result)?
        } else {
            result?
        };
        if let Ok(vfks) = self.persistence.virtual_foreign_keys() {
            merge_virtual_fks_into_diagram(&mut diagram, &vfks, &session.profile.id);
        }
        Ok(diagram)
    }
    pub async fn schema_snapshot(
        &self,
        session_id: &str,
        schema: &str,
    ) -> AppResult<SchemaSnapshot> {
        let _recovery_activity = self.begin_recovery_activity()?;
        if schema.is_empty() || schema.len() > 1024 || schema.contains('\0') {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Choose a schema to snapshot",
            ));
        }
        let session = self.session(session_id)?;
        #[cfg(unix)]
        self.snowflake_grid.require_ordinary_session(session_id)?;
        if !session.info.capabilities.schema_diff {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Schema snapshots and comparison are not supported for this database engine",
            ));
        }
        let _lane = if snowflake_dml::sdk_profile(&session.profile) {
            Some(Self::relation_lane(&session).await?)
        } else {
            None
        };
        let result = session.driver.schema_snapshot(schema).await;
        if snowflake_dml::sdk_profile(&session.profile) {
            Self::publish_relation(&session, result)
        } else {
            result
        }
    }
    pub fn compare_schemas(
        &self,
        left: SchemaSnapshot,
        right: SchemaSnapshot,
    ) -> AppResult<SchemaDiffReport> {
        if left.engine != right.engine {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Compare schemas from the same database engine",
            ));
        }
        Ok(crate::schema_diff::compare_schema_snapshots(left, right))
    }
    pub fn generate_schema_ddl(
        &self,
        snapshot: SchemaSnapshot,
        target_schema: Option<&str>,
    ) -> AppResult<SchemaDdlPlan> {
        if let Some(target) = target_schema
            && (target.len() > 1024 || target.contains('\0'))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Choose a valid target schema name",
            ));
        }
        if snapshot.schema.trim().is_empty()
            && target_schema.map(str::trim).unwrap_or_default().is_empty()
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Choose a schema to generate DDL for",
            ));
        }
        crate::ddl::generate_schema_ddl(&snapshot, target_schema)
    }
    pub fn generate_migration_ddl(
        &self,
        left: SchemaSnapshot,
        right: SchemaSnapshot,
        target_schema: Option<&str>,
    ) -> AppResult<SchemaDdlPlan> {
        if let Some(target) = target_schema
            && (target.len() > 1024 || target.contains('\0'))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Choose a valid target schema name",
            ));
        }
        crate::ddl::generate_migration_ddl(&left, &right, target_schema)
    }
    pub fn preview_index_constraint_ddl(
        &self,
        engine: EngineKind,
        action: IndexConstraintAction,
    ) -> AppResult<IndexConstraintPreview> {
        crate::ddl::build_index_constraint_ddl(engine, &action)
    }
    pub fn prepare_index_constraint_ddl(
        &self,
        session_id: &str,
        action: IndexConstraintAction,
    ) -> AppResult<PreparedPlan> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        if !session.info.capabilities.ddl_editing {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Structured DDL editing is not supported for this database engine",
            ));
        }
        let preview = crate::ddl::build_index_constraint_ddl(session.profile.engine, &action)?;
        let sql = preview.statement.sql;
        let statement = StatementPlan {
            span: Some(crate::sql_text::span_for_byte_range(&sql, 0, sql.len())),
            sql: sql.clone(),
            kind: StatementKind::Ddl,
            missing_where: false,
        };
        let mut plan = safety::plan_for_statements(session_id, &session.profile, vec![statement])?;
        plan.requires_approval = true;
        for warning in preview.warnings {
            if !plan.warnings.contains(&warning) {
                plan.warnings.push(warning);
            }
        }
        let bytes = sql.len();
        self.store_plan(
            plan,
            Action::Sql {
                bound: Vec::new(),
                error_source: None,
                original_sql: None,
            },
            bytes,
        )
    }
    pub fn preview_table_object_ddl(
        &self,
        engine: EngineKind,
        action: TableObjectAction,
    ) -> AppResult<TableObjectPreview> {
        crate::ddl::build_table_object_ddl(engine, &action)
    }
    pub fn prepare_table_object_ddl(
        &self,
        session_id: &str,
        action: TableObjectAction,
    ) -> AppResult<PreparedPlan> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        if !session.info.capabilities.ddl_editing {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Structured DDL editing is not supported for this database engine",
            ));
        }
        let preview = crate::ddl::build_table_object_ddl(session.profile.engine, &action)?;
        let mut statements = Vec::with_capacity(preview.statements.len());
        let mut bytes = 0usize;
        for stmt in &preview.statements {
            bytes = bytes.saturating_add(stmt.sql.len());
            statements.push(StatementPlan {
                span: Some(crate::sql_text::span_for_byte_range(
                    &stmt.sql,
                    0,
                    stmt.sql.len(),
                )),
                sql: stmt.sql.clone(),
                kind: StatementKind::Ddl,
                missing_where: false,
            });
        }
        let mut plan = safety::plan_for_statements(session_id, &session.profile, statements)?;
        plan.requires_approval = true;
        for warning in preview.warnings {
            if !plan.warnings.contains(&warning) {
                plan.warnings.push(warning);
            }
        }
        self.store_plan(
            plan,
            Action::Sql {
                bound: Vec::new(),
                error_source: None,
                original_sql: None,
            },
            bytes,
        )
    }
    pub fn preview_table_maintenance(
        &self,
        engine: EngineKind,
        action: TableMaintenanceAction,
    ) -> AppResult<TableMaintenancePreview> {
        crate::ddl::build_table_maintenance_sql(engine, &action)
    }
    pub fn prepare_table_maintenance(
        &self,
        session_id: &str,
        action: TableMaintenanceAction,
    ) -> AppResult<PreparedPlan> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        let preview = crate::ddl::build_table_maintenance_sql(session.profile.engine, &action)?;
        let sql = preview.statement.sql;
        let statement = StatementPlan {
            span: Some(crate::sql_text::span_for_byte_range(&sql, 0, sql.len())),
            sql: sql.clone(),
            kind: StatementKind::Ddl,
            missing_where: false,
        };
        let mut plan = safety::plan_for_statements(session_id, &session.profile, vec![statement])?;
        plan.requires_approval = true;
        for warning in preview.warnings {
            if !plan.warnings.contains(&warning) {
                plan.warnings.push(warning);
            }
        }
        let bytes = sql.len();
        self.store_plan(
            plan,
            Action::Sql {
                bound: Vec::new(),
                error_source: None,
                original_sql: None,
            },
            bytes,
        )
    }
    pub async fn prepare_migration_ddl(
        &self,
        session_id: &str,
        expected_snapshot: SchemaSnapshot,
        statements: Vec<crate::ddl::DdlStatement>,
    ) -> AppResult<PreparedPlan> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        if !matches!(
            session.profile.engine,
            EngineKind::Postgres | EngineKind::Mysql
        ) || !session.info.capabilities.ddl_editing
            || !session.info.capabilities.schema_diff
        {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Schema migration execution is available for PostgreSQL and MySQL/MariaDB",
            ));
        }
        if session.profile.engine != expected_snapshot.engine {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Target session engine does not match the migration snapshot engine",
            ));
        }
        let target_schema = expected_snapshot.schema.trim();
        if target_schema.is_empty() || target_schema.len() > 1024 || target_schema.contains('\0') {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Choose a valid target schema to migrate",
            ));
        }
        if statements.is_empty() || statements.len() > 500 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Select between 1 and 500 migration DDL statements to prepare",
            ));
        }
        for stmt in &statements {
            if stmt.sql.trim().is_empty() || stmt.sql.contains('\0') {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "Each migration DDL statement must contain non-empty SQL",
                ));
            }
        }

        let mut statement_plans = Vec::with_capacity(statements.len());
        let mut bytes = 0usize;
        for stmt in &statements {
            bytes = bytes.saturating_add(stmt.sql.len());
            statement_plans.push(StatementPlan {
                sql: stmt.sql.clone(),
                kind: StatementKind::Ddl,
                missing_where: false,
                span: Some(crate::sql_text::span_for_byte_range(
                    &stmt.sql,
                    0,
                    stmt.sql.len(),
                )),
            });
        }
        let mut plan = safety::plan_for_statements(session_id, &session.profile, statement_plans)?;

        let live_snapshot = session.driver.schema_snapshot(target_schema).await?;
        let drift = crate::schema_diff::compare_schema_snapshots(
            live_snapshot.clone(),
            expected_snapshot.clone(),
        );
        if !drift.entries.is_empty()
            || live_snapshot.truncated != expected_snapshot.truncated
            || live_snapshot.restricted != expected_snapshot.restricted
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                format!(
                    "Target schema \"{}\" has drifted since the comparison snapshot was captured ({} difference(s) detected). Re-compare schemas before applying migration DDL.",
                    expected_snapshot.schema,
                    drift.entries.len()
                ),
            ));
        }

        plan.requires_approval = true;

        let destructive_count = statements.iter().filter(|s| s.destructive).count();
        if destructive_count > 0 {
            let warning = format!(
                "{destructive_count} destructive statement(s) (DROP) selected; review carefully before executing."
            );
            if !plan.warnings.contains(&warning) {
                plan.warnings.push(warning);
            }
        }
        match session.profile.engine {
            EngineKind::Mysql => {
                let warning = "Migration DDL on MySQL/MariaDB causes an implicit commit of any open transaction on this session before and after each DDL statement.".to_string();
                if !plan.warnings.contains(&warning) {
                    plan.warnings.push(warning);
                }
            }
            EngineKind::Postgres => {
                let warning = "Migration DDL on PostgreSQL acquires ACCESS EXCLUSIVE locks on altered or dropped objects and may block concurrent queries until completion.".to_string();
                if !plan.warnings.contains(&warning) {
                    plan.warnings.push(warning);
                }
            }
            _ => {}
        }

        self.store_plan(
            plan,
            Action::Sql {
                bound: Vec::new(),
                error_source: None,
                original_sql: None,
            },
            bytes,
        )
    }
    async fn schema_for_session(
        &self,
        session: &Session,
        table: &TableRef,
    ) -> AppResult<TableSchema> {
        let mut schema = session.driver.schema(table).await?;
        if safety::effective_mode(&session.profile) == SafeMode::ReadOnly {
            schema.editable = false;
            for c in &mut schema.columns {
                c.editable = false;
                c.insertable = false;
            }
        }
        Ok(schema)
    }
    pub async fn redis_scan(
        &self,
        session_id: &str,
        pattern: &str,
        cursor: Option<&str>,
    ) -> AppResult<RedisScanPage> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        let _lane = session.lane.lock().await;
        tokio::time::timeout(
            Duration::from_secs(15),
            session.driver.redis_scan(pattern, cursor),
        )
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::ConnectionFailed,
                "The Redis browser timed out. Narrow the pattern and restart the scan.",
            )
        })?
    }
    pub async fn redis_inspect(
        &self,
        session_id: &str,
        argument: &str,
        cursor: Option<&str>,
    ) -> AppResult<RedisValuePreview> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        let _lane = session.lane.lock().await;
        tokio::time::timeout(
            Duration::from_secs(15),
            session.driver.redis_inspect(argument, cursor),
        )
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::ConnectionFailed,
                "The Redis value preview timed out. Retry with a smaller range in the console.",
            )
        })?
    }
    pub async fn health(&self, session_id: &str) -> AppResult<u32> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        // SDK health sends SQL and revokes observer authority. Serialize and
        // recheck the native hold before entering the driver at all.
        let _lane = if snowflake_dml::sdk_profile(&session.profile) {
            let lane = tokio::time::timeout(Duration::from_millis(1500), session.lane.lock())
                .await
                .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
            session.metadata_gate.lock().check(None, false)?;
            Some(lane)
        } else {
            None
        };
        session.driver.health().await
    }
    pub async fn processes(&self, session_id: &str) -> AppResult<Vec<ProcessInfo>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.session(session_id)?.driver.processes().await
    }
    pub async fn lock_waits(&self, session_id: &str) -> AppResult<LockWaitSnapshot> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        if !session.info.capabilities.lock_inspection {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Lock inspection is unavailable for this engine",
            ));
        }
        session.driver.lock_waits().await
    }

    pub async fn browse(
        &self,
        session_id: &str,
        table: &TableRef,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<TablePage> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.browse_with(session_id, table, after, &BrowseQuery::default())
            .await
    }

    pub async fn prepare_browse(
        &self,
        session_id: &str,
        table: &TableRef,
        query: &BrowseQuery,
    ) -> AppResult<Option<PreparedPlan>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let query = query.normalized();
        if !query.has_raw_clauses() {
            return Ok(None);
        }
        let session = self.session(session_id)?;
        let _lane = session.lane.lock().await;
        let schema = self.schema_for_session(&session, table).await?;
        let sql = browse_sql_for_engine(session.profile.engine, &schema, &query, None)?;
        let (mut plan, needs_review) = classify_browse_sql(session_id, &session.profile, &sql)?;
        if !needs_review {
            return Ok(None);
        }
        plan.requires_approval = true;
        plan.warnings.push(
            "This table filter or order clause uses an unclassified or side-effecting expression and executes a single bounded page."
                .into(),
        );
        let bytes = sql.len();
        let stored = self.store_plan(
            plan,
            Action::Browse {
                table: table.clone(),
                query,
            },
            bytes,
        )?;
        Ok(Some(stored))
    }

    /// Table page with server-side sort/filter. Typed values stay bound; raw
    /// `WHERE` / `ORDER BY` clauses are classified through `safety::prepare`.
    pub async fn browse_with(
        &self,
        session_id: &str,
        table: &TableRef,
        after: Option<Vec<Option<String>>>,
        query: &BrowseQuery,
    ) -> AppResult<TablePage> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.browse_with_approval(session_id, table, after, query, None)
            .await
    }

    pub async fn browse_with_approval(
        &self,
        session_id: &str,
        table: &TableRef,
        after: Option<Vec<Option<String>>>,
        query: &BrowseQuery,
        approval: Option<&BrowseApproval>,
    ) -> AppResult<TablePage> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let query = query.normalized();
        let session = self.session(session_id)?;
        let _lane = session.lane.lock().await;
        let schema = self.schema_for_session(&session, table).await?;
        let mut approved_digest: Option<String> = None;
        if query.has_raw_clauses() {
            let sql =
                browse_sql_for_engine(session.profile.engine, &schema, &query, after.as_deref())?;
            let (_, needs_review) = classify_browse_sql(session_id, &session.profile, &sql)?;
            if needs_review {
                if after.is_some() {
                    return Err(AppError::new(
                        ErrorCode::ApprovalRequired,
                        "Approved table clauses execute a single bounded page",
                    ));
                }
                let token = approval.ok_or_else(|| {
                    AppError::new(
                        ErrorCode::ApprovalRequired,
                        "Review and authorize this table filter or order clause before running it",
                    )
                })?;
                let stored = self
                    .plans
                    .lock()
                    .remove(&token.plan_id)
                    .ok_or_else(expired)?;
                if stored.deadline <= Instant::now() {
                    return Err(expired());
                }
                if stored.plan.session_id != session_id
                    || stored.approval.as_deref() != Some(token.approval.as_str())
                    || !matches!(
                        &stored.action,
                        Action::Browse { table: t, query: q } if t == table && q == &query
                    )
                {
                    return Err(AppError::new(
                        ErrorCode::ApprovalRequired,
                        "Review and authorize this exact table filter or order clause before running it",
                    ));
                }
                self.persistence
                    .audit(&session.profile.id, "browse", &stored.digest, "started")?;
                approved_digest = Some(stored.digest);
            }
        }
        let (mut page, target) = match session.driver.browse_target(&schema, &query, after).await {
            Ok((page, target)) => {
                if let Some(digest) = &approved_digest {
                    let _ =
                        self.persistence
                            .audit(&session.profile.id, "browse", digest, "Completed");
                }
                (page, target)
            }
            Err(error) => {
                if let Some(digest) = &approved_digest {
                    let _ = self.persistence.audit(
                        &session.profile.id,
                        "browse",
                        digest,
                        if error.code == ErrorCode::OutcomeUnknown {
                            "OutcomeUnknown"
                        } else {
                            "Failed"
                        },
                    );
                }
                if session.driver.is_closed() {
                    self.disconnect(session_id);
                }
                return Err(error);
            }
        };
        if approved_digest.is_some() {
            page.has_more = false;
            page.next_key = None;
        }
        let mut display_schema = target.schema.clone();
        if let Ok(vfks) = self.persistence.virtual_foreign_keys() {
            merge_virtual_fks_into_table_schema(
                &mut display_schema,
                &vfks,
                &session.profile.id,
                false,
            );
        }
        let result = self.results.create(
            &format!("table:{session_id}"),
            0,
            display_schema.columns,
            Some(target.schema),
        )?;
        self.results
            .set_edit_identity(&result.id, target.identity)?;
        self.result_sessions
            .lock()
            .insert(result.id.clone(), session_id.into());
        let retained = self.results.append(&result.id, page.rows)?;
        if !retained {
            self.release_result(&result.id);
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Close a result tab to free memory before loading this page",
            ));
        }
        let result = self.results.finish(&result.id, page.truncated)?;
        Ok(TablePage {
            result,
            next_key: page.next_key,
            has_more: page.has_more,
        })
    }

    pub fn prepare_changes(
        &self,
        session_id: &str,
        result_id: &str,
        changes: Vec<CellChange>,
    ) -> AppResult<ChangePreview> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.prepare_changes_batch(
            session_id,
            vec![ResultChanges {
                result_id: result_id.into(),
                changes,
            }],
        )
    }

    pub fn prepare_changes_batch(
        &self,
        session_id: &str,
        inputs: Vec<ResultChanges>,
    ) -> AppResult<ChangePreview> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.prepare_table_changes(
            session_id,
            TableChanges {
                result_id: inputs.first().map(|input| input.result_id.clone()),
                updates: inputs,
                deletes: Vec::new(),
                inserts: Vec::new(),
            },
        )
    }

    fn verify_result_provenance(
        &self,
        session_id: &str,
        result_id: &str,
        expected: &TableSchema,
        identity: Option<&EditIdentity>,
    ) -> AppResult<()> {
        if self
            .result_sessions
            .lock()
            .get(result_id)
            .map(String::as_str)
            != Some(session_id)
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This result belongs to a different session",
            ));
        }
        let source = self.results.provenance(result_id)?;
        if self.results.edit_identity(result_id)?.as_ref() != identity
            || source.table != expected.table
            || serde_json::to_string(&source.columns).ok()
                != serde_json::to_string(&expected.columns).ok()
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "All edits must belong to the same unchanged table schema",
            ));
        }
        Ok(())
    }

    pub fn prepare_table_changes(
        &self,
        session_id: &str,
        changes: TableChanges,
    ) -> AppResult<ChangePreview> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        let anchor_id = changes
            .result_id
            .as_deref()
            .or_else(|| changes.updates.first().map(|u| u.result_id.as_str()))
            .or_else(|| changes.deletes.first().map(|d| d.result_id.as_str()))
            .ok_or_else(|| AppError::invalid("There are no pending edits"))?;
        if self
            .result_sessions
            .lock()
            .get(anchor_id)
            .map(String::as_str)
            != Some(session_id)
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This result belongs to a different session",
            ));
        }
        let schema = self.results.provenance(anchor_id)?;
        let identity = self.results.edit_identity(anchor_id)?;
        let retained_anchor = Some(anchor_id.to_owned());
        let retained_updates = changes.updates.clone();
        let retained_deletes = changes.deletes.clone();
        let update_cells = changes
            .updates
            .iter()
            .map(|i| i.changes.len())
            .sum::<usize>();
        let delete_rows = changes.deletes.iter().map(|d| d.rows.len()).sum::<usize>();
        let insert_cells = changes
            .inserts
            .iter()
            .map(|r| r.values.len())
            .sum::<usize>();
        let total_cells = update_cells + insert_cells;
        if !schema.editable
            || (update_cells == 0 && delete_rows == 0 && changes.inserts.is_empty())
            || total_cells > 5000
        {
            return Err(AppError::invalid(
                "Select editable cells in a table with a primary key",
            ));
        }

        if matches!(
            session.profile.engine,
            EngineKind::Postgres | EngineKind::Mysql
        ) && identity.is_none()
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "The original table identity is unavailable. Read the table again before editing.",
            ));
        }

        let mut keyed_mutations: Vec<(TextRow, RowMutation)> = Vec::new();
        let mut bytes = 0usize;

        for input in changes.deletes {
            self.verify_result_provenance(
                session_id,
                &input.result_id,
                &schema,
                identity.as_ref(),
            )?;
            let mut seen_rows = std::collections::HashSet::new();
            for row_index in input.rows {
                if !seen_rows.insert(row_index) {
                    return Err(AppError::invalid(
                        "A row was marked for deletion twice in one change set",
                    ));
                }
                let original = self.results.row(&input.result_id, row_index)?;
                bytes += original.iter().flatten().map(String::len).sum::<usize>();
                if bytes > 8 * 1024 * 1024 {
                    return Err(AppError::new(
                        ErrorCode::ResultLimitReached,
                        "This change set exceeds the 8 MiB review budget",
                    ));
                }
                keyed_mutations.push((original.clone(), RowMutation::Delete { original }));
            }
        }

        for input in changes.updates {
            self.verify_result_provenance(
                session_id,
                &input.result_id,
                &schema,
                identity.as_ref(),
            )?;
            let mut rows: HashMap<usize, Vec<(usize, Option<String>)>> = HashMap::new();
            for change in input.changes {
                if change.value.as_ref().is_some_and(|v| v.len() > 1024 * 1024) {
                    return Err(AppError::invalid(
                        "An edited value must be smaller than 1 MiB",
                    ));
                }
                let index = schema
                    .columns
                    .iter()
                    .position(|c| c.id == change.column)
                    .ok_or_else(|| AppError::invalid("Unknown edited column"))?;
                let column = &schema.columns[index];
                if !column.editable || column.primary_key {
                    return Err(AppError::new(
                        ErrorCode::PolicyBlocked,
                        "This column is read-only",
                    ));
                }
                if !column.nullable && change.value.is_none() {
                    return Err(AppError::invalid("This column does not allow NULL"));
                }
                if column.kind == ColumnKind::Json
                    && let Some(value) = &change.value
                {
                    serde_json::from_str::<serde_json::Value>(value).map_err(|_| {
                        AppError::invalid("Enter valid JSON before applying this edit")
                    })?;
                }
                let cells = rows.entry(change.row).or_default();
                if cells.iter().any(|(i, _)| *i == index) {
                    return Err(AppError::invalid(
                        "A cell was included twice in one change set",
                    ));
                }
                cells.push((index, change.value));
            }
            for (index, row_changes) in rows {
                let original = self.results.row(&input.result_id, index)?;
                bytes += original.iter().flatten().map(String::len).sum::<usize>()
                    + row_changes
                        .iter()
                        .filter_map(|(_, v)| v.as_ref())
                        .map(String::len)
                        .sum::<usize>();
                if bytes > 8 * 1024 * 1024 {
                    return Err(AppError::new(
                        ErrorCode::ResultLimitReached,
                        "This change set exceeds the 8 MiB review budget",
                    ));
                }
                keyed_mutations.push((
                    original.clone(),
                    RowMutation::Update {
                        original,
                        changes: row_changes,
                    },
                ));
            }
        }

        let key_indices = schema
            .primary_key
            .iter()
            .filter_map(|key| schema.columns.iter().position(|c| &c.name == key))
            .collect::<Vec<_>>();
        keyed_mutations.sort_by(|a, b| {
            key_indices
                .iter()
                .map(|i| &a.0[*i])
                .cmp(key_indices.iter().map(|i| &b.0[*i]))
        });
        for pair in keyed_mutations.windows(2) {
            if key_indices.iter().all(|i| pair[0].0[*i] == pair[1].0[*i]) {
                return Err(AppError::invalid(
                    "A primary key was included twice in this change set",
                ));
            }
        }

        let mut mutations: Vec<RowMutation> = keyed_mutations.into_iter().map(|(_, m)| m).collect();

        for row in changes.inserts {
            let mut cols: Vec<(usize, Option<String>)> = Vec::new();
            for entry in row.values {
                if entry.value.as_ref().is_some_and(|v| v.len() > 1024 * 1024) {
                    return Err(AppError::invalid(
                        "An edited value must be smaller than 1 MiB",
                    ));
                }
                let index = schema
                    .columns
                    .iter()
                    .position(|c| c.id == entry.column)
                    .ok_or_else(|| AppError::invalid("Unknown inserted column"))?;
                let column = &schema.columns[index];
                if !column.insertable {
                    return Err(AppError::new(
                        ErrorCode::PolicyBlocked,
                        format!("Column \"{}\" cannot be set on insert", column.name),
                    ));
                }
                if !column.nullable && entry.value.is_none() {
                    return Err(AppError::invalid(format!(
                        "Column \"{}\" does not allow NULL",
                        column.name
                    )));
                }
                if column.kind == ColumnKind::Json
                    && let Some(value) = &entry.value
                {
                    serde_json::from_str::<serde_json::Value>(value).map_err(|_| {
                        AppError::invalid("Enter valid JSON before applying this insert")
                    })?;
                }
                if cols.iter().any(|(i, _)| *i == index) {
                    return Err(AppError::invalid(
                        "A column was included twice in one inserted row",
                    ));
                }
                bytes += entry.value.as_ref().map_or(0, String::len);
                if bytes > 8 * 1024 * 1024 {
                    return Err(AppError::new(
                        ErrorCode::ResultLimitReached,
                        "This change set exceeds the 8 MiB review budget",
                    ));
                }
                cols.push((index, entry.value));
            }
            cols.sort_by_key(|(i, _)| *i);
            for (index, column) in schema.columns.iter().enumerate() {
                if !column.nullable && !column.has_default && !cols.iter().any(|(i, _)| *i == index)
                {
                    return Err(AppError::invalid(format!(
                        "Column \"{}\" requires a value (NOT NULL without a default)",
                        column.name
                    )));
                }
            }
            mutations.push(RowMutation::Insert { values: cols });
        }

        if mutations.len() > 500 {
            return Err(AppError::invalid("Apply at most 500 edited rows at a time"));
        }

        let generated = mutations
            .iter()
            .map(|m| {
                if session.profile.engine == EngineKind::Mysql {
                    crate::drivers::mysql::mutation_sql(&schema, m)
                } else {
                    mutation_sql(&schema, m)
                }
            })
            .collect::<AppResult<Vec<_>>>()?;
        let sql = generated
            .iter()
            .map(|(sql, _)| sql.clone())
            .collect::<Vec<_>>();
        let bindings = generated
            .into_iter()
            .map(|(_, values)| values)
            .collect::<Vec<_>>();
        let statements = sql
            .iter()
            .map(|sql| StatementPlan {
                sql: sql.clone(),
                kind: StatementKind::Write,
                missing_where: false,
                span: None,
            })
            .collect();
        let mut plan = safety::plan_for_statements(session_id, &session.profile, statements)?;
        plan.requires_approval = true;
        plan.warnings.push("Grid edits use the login role in a separate transaction. SQL-editor transactions and session settings are independent.".into());
        bytes += sql.iter().map(String::len).sum::<usize>();
        let has_inserts = mutations
            .iter()
            .any(|mutation| matches!(mutation, RowMutation::Insert { .. }));
        let row_count = mutations.len();
        let plan = self.store_plan(
            plan,
            Action::Changes {
                schema,
                identity,
                rows: mutations,
                import_preview: None,
                anchor_id: retained_anchor,
                updates: retained_updates,
                deletes: retained_deletes,
                has_inserts,
            },
            bytes,
        )?;
        Ok(ChangePreview {
            plan,
            row_count,
            sql,
            bindings,
        })
    }
    pub fn prepare_process_cancel(
        &self,
        session_id: &str,
        id: &str,
        identity: &str,
    ) -> AppResult<PreparedPlan> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let session = self.session(session_id)?;
        let mut plan = safety::prepare(
            session_id,
            &session.profile,
            "SELECT pg_catalog.pg_cancel_backend($1)",
        )?;
        plan.requires_approval = true;
        self.store_plan(
            plan,
            Action::CancelProcess {
                id: id.into(),
                identity: identity.into(),
            },
            id.len() + identity.len(),
        )
    }

    pub fn range(&self, result_id: &str, start: usize, count: usize) -> AppResult<Vec<u8>> {
        self.results.range(result_id, start, count)
    }
    pub fn release_result(&self, id: &str) {
        self.result_sessions.lock().remove(id);
        self.results.release(id);
        self.console.release_result(id);
    }
    pub fn history(&self, search: &str) -> AppResult<Vec<HistoryEntry>> {
        self.persistence.history(search)
    }
    pub fn history_page(&self, request: &HistoryPageRequest) -> AppResult<HistoryPage> {
        self.persistence.history_page(request)
    }
    pub fn history_entry(&self, id: &str) -> AppResult<Option<HistoryEntry>> {
        self.persistence.history_entry(id)
    }
    pub fn clear_history(&self) -> AppResult<()> {
        let _recovery_activity = self.begin_recovery_activity()?;
        self.persistence.clear_history()
    }
    pub fn audit_log(&self) -> AppResult<Vec<AuditEntry>> {
        self.persistence.audit_log()
    }
    pub async fn pick_export_destination(
        &self,
        suggested_name: &str,
        format: ExportFormat,
    ) -> AppResult<Option<FileSelection>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let mut sanitized = FileCapabilities::validate_suggested_name(suggested_name)?;
        let ext_suffix = format!(".{}", format.extension());
        if !sanitized.to_ascii_lowercase().ends_with(&ext_suffix) {
            sanitized.push_str(&ext_suffix);
        }
        let Some(path) = self.file_dialog.pick_save_file(&sanitized, format).await? else {
            return Ok(None);
        };
        Ok(Some(self.file_capabilities.mint_write_grant(path)?))
    }

    pub async fn start_export(
        self: &Arc<Self>,
        request: ExportRequest,
        events: ExportEventHandler,
    ) -> AppResult<String> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let recovery_activity = self.retain_recovery_activity()?;
        self.file_capabilities
            .verify_write_grant(&request.destination_token)?;
        if self.exports.lock().len() >= 16 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Too many exports are already running",
            ));
        }

        let (prepared, selected_columns, engine, target_table, owner_session_id) = match request
            .source
        {
            ExportSource::Result {
                result_id,
                column_ids,
            } => {
                let info = self.results.info(&result_id)?;
                let (selected_columns, column_indices) =
                    resolve_export_columns(&info.columns, &column_ids)?;
                let batch_count = self.results.batch_count(&result_id)?;
                let session_id = self.result_sessions.lock().get(&result_id).cloned();
                let engine = request.engine.or_else(|| {
                    session_id
                        .as_deref()
                        .and_then(|sid| self.sessions.lock().get(sid).map(|s| s.profile.engine))
                });
                let target_table = request
                    .target_table
                    .clone()
                    .or_else(|| self.results.provenance(&result_id).ok().map(|s| s.table));
                let source_truncated = info.status == RetentionStatus::Truncated;
                (
                    PreparedExportSource::Result {
                        result_id,
                        batch_count,
                        column_indices,
                        source_truncated,
                    },
                    selected_columns,
                    engine,
                    target_table,
                    session_id,
                )
            }
            ExportSource::Table {
                session_id,
                table,
                query,
                approval,
                column_ids,
            } => {
                let query = query.normalized();
                let session = self.session(&session_id)?;
                if session.profile.engine == EngineKind::Redis {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "Table streaming export is unavailable for Redis sessions",
                    ));
                }
                let schema = {
                    let _lane = session.lane.lock().await;
                    self.schema_for_session(&session, &table).await?
                };
                let mut approved_digest: Option<String> = None;
                if query.has_raw_clauses() {
                    let sql = browse_sql_for_engine(session.profile.engine, &schema, &query, None)?;
                    let (_, needs_review) =
                        classify_browse_sql(&session_id, &session.profile, &sql)?;
                    if needs_review {
                        let token = approval.as_ref().ok_or_else(|| {
                                AppError::new(
                                    ErrorCode::ApprovalRequired,
                                    "Review and authorize this table filter or order clause before exporting",
                                )
                            })?;
                        let stored = self
                            .plans
                            .lock()
                            .remove(&token.plan_id)
                            .ok_or_else(expired)?;
                        if stored.deadline <= Instant::now() {
                            return Err(expired());
                        }
                        if stored.plan.session_id != session_id
                            || stored.approval.as_deref() != Some(token.approval.as_str())
                            || !matches!(
                                &stored.action,
                                Action::Browse { table: t, query: q } if t == &table && q == &query
                            )
                        {
                            return Err(AppError::new(
                                ErrorCode::ApprovalRequired,
                                "Review and authorize this exact table filter or order clause before exporting",
                            ));
                        }
                        self.persistence.audit(
                            &session.profile.id,
                            "browse",
                            &stored.digest,
                            "started",
                        )?;
                        approved_digest = Some(stored.digest);
                    }
                }
                let (selected_columns, column_indices) =
                    resolve_export_columns(&schema.columns, &column_ids)?;
                let engine = request.engine.or(Some(session.profile.engine));
                let target_table = request.target_table.clone().or_else(|| Some(table.clone()));
                let single_page_only = approved_digest.is_some();
                (
                    PreparedExportSource::Table(Box::new(PreparedTableExport {
                        session,
                        schema,
                        query,
                        column_indices,
                        single_page_only,
                        approved_digest,
                    })),
                    selected_columns,
                    engine,
                    target_table,
                    Some(session_id),
                )
            }
        };

        let destination = self
            .file_capabilities
            .consume_write_grant(&request.destination_token)?;
        let options = ExportWriterOptions::new(
            request.format,
            selected_columns,
            request.include_headers,
            engine,
            target_table,
        );
        let file_session = ExportFileSession::create(destination, options)?;

        let export_id = new_id();
        let token = CancellationToken::new();
        self.exports.lock().insert(
            export_id.clone(),
            ExportJob {
                session_id: owner_session_id,
                token: token.clone(),
            },
        );
        let studio = self.clone();
        let id = export_id.clone();
        tokio::spawn(async move {
            let _recovery_activity = recovery_activity;
            studio
                .run_export(id, prepared, file_session, token, events)
                .await;
        });
        Ok(export_id)
    }

    async fn run_export(
        self: Arc<Self>,
        export_id: String,
        source: PreparedExportSource,
        mut file_session: ExportFileSession,
        token: CancellationToken,
        events: ExportEventHandler,
    ) {
        let started = Instant::now();
        let source_truncated = match &source {
            PreparedExportSource::Result {
                source_truncated, ..
            } => *source_truncated,
            PreparedExportSource::Table(_) => false,
        };
        if !events(ExportEvent::Started {
            export_id: export_id.clone(),
            file_name: file_session.file_name().to_owned(),
            source_truncated,
        }) {
            token.cancel();
        }

        let finish_aborted = |studio: &Studio,
                              file_session: ExportFileSession,
                              status: RunStatus,
                              reason: &str,
                              error: Option<AppError>| {
            let (rows_written, bytes_written, partial_file_name) = file_session.abort(reason);
            studio.exports.lock().remove(&export_id);
            events(ExportEvent::Finished {
                export_id: export_id.clone(),
                status,
                rows_written,
                bytes_written,
                output_file_name: None,
                partial_file_name: Some(partial_file_name),
                error,
                elapsed_ms: started.elapsed().as_millis() as u32,
            });
        };

        match source {
            PreparedExportSource::Result {
                result_id,
                batch_count,
                column_indices,
                ..
            } => {
                for batch_index in 0..batch_count {
                    if token.is_cancelled() {
                        finish_aborted(
                            &self,
                            file_session,
                            RunStatus::Cancelled,
                            "cancelled",
                            None,
                        );
                        return;
                    }
                    let rows =
                        match self
                            .results
                            .batch_rows(&result_id, batch_index, &column_indices)
                        {
                            Ok(Some(rows)) => rows,
                            Ok(None) => break,
                            Err(error) => {
                                let reason = error.message.clone();
                                finish_aborted(
                                    &self,
                                    file_session,
                                    RunStatus::Failed,
                                    &reason,
                                    Some(error),
                                );
                                return;
                            }
                        };
                    if let Err(error) = file_session.write_rows(&rows) {
                        let reason = error.message.clone();
                        finish_aborted(
                            &self,
                            file_session,
                            RunStatus::Failed,
                            &reason,
                            Some(error),
                        );
                        return;
                    }
                    if !events(ExportEvent::Progress {
                        export_id: export_id.clone(),
                        rows_written: file_session.rows_written(),
                        bytes_written: file_session.bytes_written(),
                    }) {
                        token.cancel();
                    }
                    tokio::task::yield_now().await;
                }
            }
            PreparedExportSource::Table(table_export) => {
                let PreparedTableExport {
                    session,
                    schema,
                    query,
                    column_indices,
                    single_page_only,
                    approved_digest,
                } = *table_export;
                let mut after: Option<Vec<Option<String>>> = None;
                let mut audited_completion = false;
                loop {
                    if token.is_cancelled() || session.closed.load(Ordering::Acquire) {
                        finish_aborted(
                            &self,
                            file_session,
                            RunStatus::Cancelled,
                            "cancelled",
                            None,
                        );
                        return;
                    }
                    let page = tokio::select! {
                        biased;
                        _ = token.cancelled() => {
                            finish_aborted(
                                &self,
                                file_session,
                                RunStatus::Cancelled,
                                "cancelled",
                                None,
                            );
                            return;
                        }
                        outcome = session.driver.browse_query(&schema, &query, after.take()) => outcome,
                    };
                    let page = match page {
                        Ok(page) => {
                            if !audited_completion && let Some(digest) = &approved_digest {
                                let _ = self.persistence.audit(
                                    &session.profile.id,
                                    "browse",
                                    digest,
                                    "Completed",
                                );
                                audited_completion = true;
                            }
                            page
                        }
                        Err(error) => {
                            if !audited_completion && let Some(digest) = &approved_digest {
                                let _ = self.persistence.audit(
                                    &session.profile.id,
                                    "browse",
                                    digest,
                                    "Failed",
                                );
                            }
                            let cancelled =
                                token.is_cancelled() || error.code == ErrorCode::Cancelled;
                            let status = if cancelled {
                                RunStatus::Cancelled
                            } else {
                                RunStatus::Failed
                            };
                            let reason = if cancelled {
                                "cancelled".to_owned()
                            } else {
                                error.message.clone()
                            };
                            finish_aborted(
                                &self,
                                file_session,
                                status,
                                &reason,
                                if cancelled { None } else { Some(error) },
                            );
                            return;
                        }
                    };
                    if let Err(error) =
                        file_session.write_projected_rows(&page.rows, &column_indices)
                    {
                        let reason = error.message.clone();
                        finish_aborted(
                            &self,
                            file_session,
                            RunStatus::Failed,
                            &reason,
                            Some(error),
                        );
                        return;
                    }
                    if !events(ExportEvent::Progress {
                        export_id: export_id.clone(),
                        rows_written: file_session.rows_written(),
                        bytes_written: file_session.bytes_written(),
                    }) {
                        token.cancel();
                    }
                    if !single_page_only && !page.has_more && page.truncated {
                        let error = AppError::new(
                            ErrorCode::ResultLimitReached,
                            "This table or custom ORDER BY cannot stream beyond one 500-row page without keyset pagination. Run a SQL query and export its result, or clear the custom ORDER BY.",
                        );
                        let reason = error.message.clone();
                        finish_aborted(
                            &self,
                            file_session,
                            RunStatus::Failed,
                            &reason,
                            Some(error),
                        );
                        return;
                    }
                    if single_page_only || !page.has_more || page.next_key.is_none() {
                        break;
                    }
                    after = page.next_key;
                    tokio::task::yield_now().await;
                }
            }
        }

        if token.is_cancelled() {
            finish_aborted(&self, file_session, RunStatus::Cancelled, "cancelled", None);
            return;
        }

        match file_session.complete() {
            Ok((rows_written, bytes_written, output_file_name)) => {
                self.exports.lock().remove(&export_id);
                events(ExportEvent::Finished {
                    export_id,
                    status: RunStatus::Completed,
                    rows_written,
                    bytes_written,
                    output_file_name: Some(output_file_name),
                    partial_file_name: None,
                    error: None,
                    elapsed_ms: started.elapsed().as_millis() as u32,
                });
            }
            Err(error) => {
                self.exports.lock().remove(&export_id);
                events(ExportEvent::Finished {
                    export_id,
                    status: RunStatus::Failed,
                    rows_written: 0,
                    bytes_written: 0,
                    output_file_name: None,
                    partial_file_name: None,
                    error: Some(error),
                    elapsed_ms: started.elapsed().as_millis() as u32,
                });
            }
        }
    }

    pub fn cancel_export(&self, export_id: &str) -> AppResult<()> {
        let exports = self.exports.lock();
        let job = exports.get(export_id).ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, "This export has already finished")
        })?;
        job.token.cancel();
        Ok(())
    }

    pub async fn pick_import_source(&self) -> AppResult<Option<FileSelection>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let Some(path) = self.file_dialog.pick_open_csv().await? else {
            return Ok(None);
        };
        self.file_capabilities
            .mint_read_grant(path, crate::import::MAX_IMPORT_BYTES)
            .map(Some)
    }

    pub async fn open_sql_file(&self) -> AppResult<Option<OpenedSqlFile>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let Some(path) = self.file_dialog.pick_open_sql().await? else {
            return Ok(None);
        };
        let read_path = path.clone();
        let (file_name, content) =
            tokio::task::spawn_blocking(move || FileCapabilities::read_sql_file_text(&read_path))
                .await
                .map_err(|_| AppError::internal("SQL file read task could not complete"))??;
        let token = self.file_capabilities.mint_sql_file_grant(path)?;
        Ok(Some(OpenedSqlFile {
            token,
            file_name,
            content,
        }))
    }

    pub async fn save_sql_file(
        &self,
        token: Option<&str>,
        suggested_name: &str,
        content: &str,
    ) -> AppResult<Option<SavedSqlFile>> {
        let _recovery_activity = self.begin_recovery_activity()?;
        FileCapabilities::validate_sql_content(content)?;
        if let Some(existing_token) = token.map(str::trim).filter(|t| !t.is_empty()) {
            let path = self
                .file_capabilities
                .resolve_sql_file_grant(existing_token)?;
            let owned = content.to_owned();
            let file_name = tokio::task::spawn_blocking(move || {
                FileCapabilities::write_sql_file_atomic(&path, &owned)
            })
            .await
            .map_err(|_| AppError::internal("SQL file save task could not complete"))??;
            return Ok(Some(SavedSqlFile {
                token: existing_token.to_owned(),
                file_name,
            }));
        }

        let suggested = FileCapabilities::validate_sql_suggested_name(suggested_name)?;
        let Some(path) = self.file_dialog.pick_save_sql(&suggested).await? else {
            return Ok(None);
        };
        let new_token = self.file_capabilities.mint_sql_file_grant(path.clone())?;
        let owned = content.to_owned();
        let write_result = tokio::task::spawn_blocking(move || {
            FileCapabilities::write_sql_file_atomic(&path, &owned)
        })
        .await
        .map_err(|_| AppError::internal("SQL file save task could not complete"))?;
        match write_result {
            Ok(file_name) => Ok(Some(SavedSqlFile {
                token: new_token,
                file_name,
            })),
            Err(err) => {
                self.file_capabilities.release_sql_file_grant(&new_token);
                Err(err)
            }
        }
    }

    pub fn release_sql_file_grant(&self, token: &str) {
        self.file_capabilities.release_sql_file_grant(token);
    }

    async fn verify_import_target(
        &self,
        session_id: &str,
        table: &TableRef,
    ) -> AppResult<(Arc<Session>, TableSchema)> {
        let session = self.session(session_id)?;
        if safety::effective_mode(&session.profile) == SafeMode::ReadOnly {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This connection is read-only. CSV import is blocked.",
            ));
        }
        if !matches!(
            session.profile.engine,
            EngineKind::Postgres | EngineKind::Mysql
        ) {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "CSV import is only available for writable PostgreSQL and MySQL/MariaDB tables",
            ));
        }
        let schema = self.schema_for_session(&session, table).await?;
        if !schema.editable || !schema.columns.iter().any(|c| c.insertable) {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "CSV import requires a writable transactional table with a primary key",
            ));
        }
        Ok((session, schema))
    }

    pub async fn preview_csv_import(
        &self,
        session_id: &str,
        source_token: &str,
        table: &TableRef,
        options: CsvImportOptions,
        mappings: Vec<CsvColumnMapping>,
    ) -> AppResult<CsvImportPreview> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let (session, _schema) = self.verify_import_target(session_id, table).await?;
        let target = session.driver.edit_target(table).await?;
        let schema = target.schema.clone();
        let requested_config_digest = import_config_digest(table, &options, &mappings)?;
        let preview_options = options.clone();
        let (path, max_bytes) = self.file_capabilities.resolve_read_grant(source_token)?;
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("import.csv")
            .to_owned();
        let engine = session.profile.engine;
        let mut preview = tokio::task::spawn_blocking(move || {
            crate::import::build_csv_import_preview(
                &path, max_bytes, file_name, engine, &schema, &options, &mappings,
            )
        })
        .await
        .map_err(|_| AppError::internal("CSV import preview task could not complete"))??;
        let config_digest = import_config_digest(table, &preview_options, &preview.mappings)?;
        let mut targets = self.import_targets.lock();
        targets.retain(|_, entry| entry.created.elapsed() < Duration::from_secs(300));
        if target.identity.is_some() {
            if session.closed.load(Ordering::Acquire) {
                return Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "The import session closed during preview",
                ));
            }
            if targets.len() >= 100 {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "Close an import or wait for its preview to expire before opening another",
                ));
            }
            let preview_token = uuid::Uuid::new_v4().to_string();
            targets.insert(
                (
                    session_id.to_owned(),
                    source_token.to_owned(),
                    preview_token.clone(),
                ),
                ImportTarget {
                    can_prepare: preview.can_prepare,
                    requested_config_digest,
                    config_digest,
                    target,
                    created: Instant::now(),
                },
            );
            preview.preview_token = Some(preview_token);
        }
        Ok(preview)
    }

    pub fn release_csv_import_preview(
        &self,
        session_id: &str,
        source_token: &str,
        preview_token: &str,
    ) {
        let mut targets = self.import_targets.lock();
        targets.remove(&(
            session_id.to_owned(),
            source_token.to_owned(),
            preview_token.to_owned(),
        ));
        // Executing plans have already left this map. Releasing a dialog only
        // discards its own unconsumed approval and never cancels submitted DML.
        self.plans.lock().retain(|_, stored| {
            stored.plan.session_id != session_id
                || !matches!(&stored.action, Action::Changes { import_preview: Some((source, token)), .. }
                    if source == source_token && token == preview_token)
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_csv_import_batch(
        &self,
        session_id: &str,
        source_token: &str,
        table: &TableRef,
        options: CsvImportOptions,
        mappings: Vec<CsvColumnMapping>,
        batch_index: usize,
        preview_token: &str,
    ) -> AppResult<ChangePreview> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let (session, mut schema) = self.verify_import_target(session_id, table).await?;
        let (identity, import_preview_key) = if matches!(
            session.profile.engine,
            EngineKind::Postgres | EngineKind::Mysql
        ) {
            let config_digest = import_config_digest(table, &options, &mappings)?;
            let targets = self.import_targets.lock();
            // Accept the exact preview request or its resolved mapping response. Both
            // refer to the same original target; unrelated configuration still expires it.
            let key = (
                session_id.to_owned(),
                source_token.to_owned(),
                preview_token.to_owned(),
            );
            let entry = targets.get(&key)
                .filter(|entry| {
                    (entry.config_digest == config_digest || entry.requested_config_digest == config_digest)
                        && entry.created.elapsed() < Duration::from_secs(300)
                })
                .ok_or_else(|| AppError::new(ErrorCode::Conflict, "The original import target preview expired or changed. Preview and review the import again."))?;
            if !entry.can_prepare {
                return Err(AppError::invalid(
                    "The CSV preview contains invalid values. Correct them and preview again before preparing a batch.",
                ));
            }
            schema = entry.target.schema.clone();
            (entry.target.identity.clone(), Some(key.clone()))
        } else {
            (None, None)
        };
        let (path, max_bytes) = self.file_capabilities.resolve_read_grant(source_token)?;
        let engine = session.profile.engine;
        let schema_for_parse = schema.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            crate::import::prepare_csv_batch_mutations(
                &path,
                max_bytes,
                engine,
                &schema_for_parse,
                &options,
                &mappings,
                batch_index,
            )
        })
        .await
        .map_err(|_| AppError::internal("CSV import preparation task could not complete"))??;

        let generated = prepared
            .mutations
            .iter()
            .map(|m| {
                if engine == EngineKind::Mysql {
                    crate::drivers::mysql::mutation_sql(&schema, m)
                } else {
                    mutation_sql(&schema, m)
                }
            })
            .collect::<AppResult<Vec<_>>>()?;
        let sql = generated
            .iter()
            .map(|(stmt, _)| stmt.clone())
            .collect::<Vec<_>>();
        let bindings = generated
            .into_iter()
            .map(|(_, values)| values)
            .collect::<Vec<_>>();
        let statements = sql
            .iter()
            .map(|stmt| StatementPlan {
                sql: stmt.clone(),
                kind: StatementKind::Write,
                missing_where: false,
                span: None,
            })
            .collect();

        let mut plan = safety::plan_for_statements(session_id, &session.profile, statements)?;
        plan.requires_approval = true;
        let row_count = prepared.mutations.len();
        plan.warnings.push(format!(
            "CSV import batch {} of {} ({} row{} in one transaction). Any constraint error rolls back this batch atomically without automatic retries.",
            batch_index + 1,
            prepared.batch_count,
            row_count,
            if row_count == 1 { "" } else { "s" }
        ));
        let total_bytes = prepared
            .approx_bytes
            .saturating_add(sql.iter().map(String::len).sum::<usize>());
        // Serialize publication with exact-preview release. A closed dialog's
        // parsing task must not resurrect a plan holding its original lease.
        let targets = self.import_targets.lock();
        if import_preview_key
            .as_ref()
            .is_some_and(|key| !targets.contains_key(key))
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The original import preview was released. Preview and review the import again.",
            ));
        }
        let plan = self.store_plan(
            plan,
            Action::Changes {
                schema,
                identity,
                import_preview: import_preview_key.map(|(_, source, token)| (source, token)),
                rows: prepared.mutations,
                anchor_id: None,
                updates: Vec::new(),
                deletes: Vec::new(),
                has_inserts: false,
            },
            total_bytes,
        )?;
        Ok(ChangePreview {
            plan,
            row_count,
            sql,
            bindings,
        })
    }

    async fn prepare_compare_side(
        &self,
        side: CompareSide,
        side_name: &'static str,
    ) -> AppResult<PreparedCompareSide> {
        match side {
            CompareSide::Snapshot {
                result_ids,
                label,
                truncated_hint,
            } => {
                if result_ids.is_empty() {
                    return Err(AppError::invalid(format!(
                        "{side_name} must select a retained result snapshot"
                    )));
                }
                let first = self.results.info(&result_ids[0])?;
                if first.columns.is_empty() {
                    return Err(AppError::invalid(format!(
                        "{side_name} has no columns to compare"
                    )));
                }
                let mut columns = first.columns.clone();
                if let Ok(prov) = self.results.provenance(&result_ids[0])
                    && prov.columns.len() == columns.len()
                {
                    for (col, src) in columns.iter_mut().zip(prov.columns.iter()) {
                        col.primary_key |= src.primary_key;
                    }
                }
                let mut truncated = truncated_hint || first.status == RetentionStatus::Truncated;
                for rid in result_ids.iter().skip(1) {
                    let info = self.results.info(rid)?;
                    if info.columns.len() != columns.len()
                        || info
                            .columns
                            .iter()
                            .zip(columns.iter())
                            .any(|(a, b)| a.name != b.name)
                    {
                        return Err(AppError::invalid(format!(
                            "{side_name} snapshot pages do not share an identical column schema"
                        )));
                    }
                    truncated |= info.status == RetentionStatus::Truncated;
                }
                let resolved_label = label.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
                    first
                        .table
                        .as_ref()
                        .map(|t| format!("{}.{} (snapshot)", t.schema, t.name))
                        .unwrap_or_else(|| format!("Result #{}", first.statement_index + 1))
                });
                Ok(PreparedCompareSide::Snapshot {
                    label: resolved_label,
                    columns,
                    result_ids,
                    truncated,
                })
            }
            CompareSide::LiveTable {
                session_id,
                table,
                query,
                approval,
                label,
            } => {
                let query = query.normalized();
                let session = self.session(&session_id)?;
                if !matches!(
                    session.profile.engine,
                    EngineKind::Postgres | EngineKind::Mysql
                ) {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "Live table comparison is available for PostgreSQL and MySQL/MariaDB. Compare retained query or preview snapshots for other engines.",
                    ));
                }
                let schema = {
                    let _lane = session.lane.lock().await;
                    self.schema_for_session(&session, &table).await?
                };
                if schema.columns.is_empty() {
                    return Err(AppError::invalid(format!(
                        "{side_name} table has no columns to compare"
                    )));
                }
                let mut approval_plan_id: Option<String> = None;
                if query.has_raw_clauses() {
                    let sql = browse_sql_for_engine(session.profile.engine, &schema, &query, None)?;
                    let (_, needs_review) =
                        classify_browse_sql(&session_id, &session.profile, &sql)?;
                    if needs_review {
                        let token = approval.as_ref().ok_or_else(|| {
                            AppError::new(
                                ErrorCode::ApprovalRequired,
                                format!(
                                    "Review and authorize the SQL clause on {} before comparing",
                                    side_name.to_ascii_lowercase()
                                ),
                            )
                        })?;
                        let plans = self.plans.lock();
                        let stored = plans.get(&token.plan_id).ok_or_else(expired)?;
                        if stored.deadline <= Instant::now() {
                            return Err(expired());
                        }
                        if stored.plan.session_id != session_id
                            || stored.approval.as_deref() != Some(token.approval.as_str())
                            || !matches!(
                                &stored.action,
                                Action::Browse { table: t, query: q } if t == &table && q == &query
                            )
                        {
                            return Err(AppError::new(
                                ErrorCode::ApprovalRequired,
                                format!(
                                    "Review and authorize the exact SQL clause on {} before comparing",
                                    side_name.to_ascii_lowercase()
                                ),
                            ));
                        }
                        approval_plan_id = Some(token.plan_id.clone());
                    }
                }
                let resolved_label = label.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
                    format!("{} · {}.{}", session.profile.name, table.schema, table.name)
                });
                let single_page_only = approval_plan_id.is_some();
                Ok(PreparedCompareSide::LiveTable(Box::new(
                    PreparedLiveCompare {
                        session,
                        schema,
                        query,
                        label: resolved_label,
                        single_page_only,
                        approval_plan_id,
                        approved_digest: None,
                    },
                )))
            }
        }
    }

    fn consume_compare_approval(&self, side: &mut PreparedCompareSide) -> AppResult<()> {
        if let PreparedCompareSide::LiveTable(live) = side
            && let Some(plan_id) = live.approval_plan_id.take()
        {
            let stored = self.plans.lock().remove(&plan_id).ok_or_else(expired)?;
            if stored.deadline <= Instant::now() {
                return Err(expired());
            }
            self.persistence.audit(
                &live.session.profile.id,
                "browse",
                &stored.digest,
                "started",
            )?;
            live.approved_digest = Some(stored.digest);
        }
        Ok(())
    }

    async fn materialize_compare_side(
        &self,
        side: PreparedCompareSide,
        side_name: &'static str,
    ) -> AppResult<crate::compare::CompareDataset> {
        match side {
            PreparedCompareSide::Snapshot {
                label,
                columns,
                result_ids,
                truncated,
            } => {
                let col_indices: Vec<usize> = (0..columns.len()).collect();
                let mut builder = crate::compare::CompareDatasetBuilder::new(
                    side_name, label, columns, truncated,
                );
                for rid in &result_ids {
                    let batches = self.results.batch_count(rid)?;
                    for batch_idx in 0..batches {
                        if let Some(rows) = self.results.batch_rows(rid, batch_idx, &col_indices)? {
                            builder.append_rows(rows)?;
                        }
                    }
                }
                Ok(builder.finish())
            }
            PreparedCompareSide::LiveTable(live) => {
                let PreparedLiveCompare {
                    session,
                    schema,
                    query,
                    label,
                    single_page_only,
                    approval_plan_id: _,
                    approved_digest,
                } = *live;
                let mut builder = crate::compare::CompareDatasetBuilder::new(
                    side_name,
                    label,
                    schema.columns.clone(),
                    false,
                );
                let mut after: Option<Vec<Option<String>>> = None;
                let mut audited_completion = false;
                loop {
                    if session.closed.load(Ordering::Acquire) {
                        return Err(AppError::new(
                            ErrorCode::ConnectionClosed,
                            "The database connection closed during comparison",
                        ));
                    }
                    let page = match session
                        .driver
                        .browse_query(&schema, &query, after.take())
                        .await
                    {
                        Ok(page) => {
                            if !audited_completion && let Some(digest) = &approved_digest {
                                let _ = self.persistence.audit(
                                    &session.profile.id,
                                    "browse",
                                    digest,
                                    "Completed",
                                );
                                audited_completion = true;
                            }
                            page
                        }
                        Err(err) => {
                            if !audited_completion && let Some(digest) = &approved_digest {
                                let _ = self.persistence.audit(
                                    &session.profile.id,
                                    "browse",
                                    digest,
                                    "Failed",
                                );
                            }
                            return Err(err);
                        }
                    };
                    builder.append_rows(page.rows)?;
                    if page.truncated && (!page.has_more || single_page_only) {
                        return Err(AppError::new(
                            ErrorCode::ResultLimitReached,
                            format!(
                                "{side_name} matched more rows than can be streamed without keyset pagination. Add a narrower filter or compare a SQL query snapshot."
                            ),
                        ));
                    }
                    if single_page_only || !page.has_more || page.next_key.is_none() {
                        break;
                    }
                    after = page.next_key;
                    tokio::task::yield_now().await;
                }
                Ok(builder.finish())
            }
        }
    }

    pub async fn compare_data(&self, request: CompareRequest) -> AppResult<DataCompareReport> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let mut left_prepared = self
            .prepare_compare_side(request.left, "Left dataset")
            .await?;
        let mut right_prepared = self
            .prepare_compare_side(request.right, "Right dataset")
            .await?;

        if let (PreparedCompareSide::LiveTable(l), PreparedCompareSide::LiveTable(r)) =
            (&left_prepared, &right_prepared)
            && let (Some(lp), Some(rp)) = (&l.approval_plan_id, &r.approval_plan_id)
            && lp == rp
        {
            return Err(AppError::new(
                ErrorCode::ApprovalRequired,
                "Each live table SQL clause requires its own one-use approval token",
            ));
        }

        self.consume_compare_approval(&mut left_prepared)?;
        self.consume_compare_approval(&mut right_prepared)?;

        let left_ds = self
            .materialize_compare_side(left_prepared, "Left dataset")
            .await?;
        let right_ds = self
            .materialize_compare_side(right_prepared, "Right dataset")
            .await?;
        let key_columns = request.key_columns;
        let ignore_case = request.ignore_column_case;

        tokio::task::spawn_blocking(move || {
            crate::compare::compare_datasets(left_ds, right_ds, &key_columns, ignore_case)
        })
        .await
        .map_err(|_| AppError::internal("Data comparison task could not complete"))?
    }

    pub async fn run_workspace_sql(
        &self,
        request: WorkspaceSqlRequest,
    ) -> AppResult<WorkspaceSqlResponse> {
        let _recovery_activity = self.begin_recovery_activity()?;
        let snapshots =
            crate::workspace_sql::materialize_mounts(&self.results, &request.mounts, |rid| {
                self.console.owns_result(rid)
            })?;
        let sql = request.sql;
        let out = tokio::task::spawn_blocking(move || {
            crate::workspace_sql::execute_workspace_query(snapshots, &sql)
        })
        .await
        .map_err(|_| AppError::internal("Workspace SQL task could not complete"))??;

        let created = self.results.create_with_editability(
            &format!("workspace-sql-{}", new_id()),
            0,
            out.columns,
            None,
            Some(ResultEditability::ReadOnly {
                reason: "Cross-source workspace query result".into(),
            }),
        )?;
        let mut truncated = out.truncated;
        for chunk in out.rows.chunks(512) {
            match self.results.append(&created.id, chunk.to_vec()) {
                Ok(true) => {}
                Ok(false) => {
                    truncated = true;
                    break;
                }
                Err(err) => {
                    self.results.release(&created.id);
                    return Err(err);
                }
            }
        }
        let finished = match self.results.finish(&created.id, truncated) {
            Ok(info) => info,
            Err(err) => {
                self.results.release(&created.id);
                return Err(err);
            }
        };
        Ok(WorkspaceSqlResponse {
            result: finished,
            elapsed_ms: out.elapsed_ms,
            mounted_rows: out.mounted_rows,
            truncated,
        })
    }

    pub async fn search_database_data(
        &self,
        request: DataSearchRequest,
    ) -> AppResult<DataSearchReport> {
        let _recovery_activity = self.begin_recovery_activity()?;
        crate::data_search::validate_search_request(&request)?;
        let session = self.session(&request.session_id)?;
        if session.info.console_mode {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Full-text data search is disabled on reviewed console sessions",
            ));
        }
        #[cfg(unix)]
        self.snowflake_grid
            .require_ordinary_session(&request.session_id)?;
        if snowflake_dml::sdk_profile(&session.profile) {
            let _lane = Self::relation_lane(&session).await?;
            let result = session.driver.search_data(&request).await;
            Self::publish_relation(&session, result)
        } else {
            let _lane = tokio::time::timeout(Duration::from_secs(3), session.lane.lock())
                .await
                .map_err(|_| AppError::new(ErrorCode::Conflict, "The database session is busy"))?;
            if session.closed.load(Ordering::Acquire) || session.driver.is_closed() {
                return Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "This database session is disconnected",
                ));
            }
            if session.active_run.lock().is_some() {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Wait for the running query to finish first",
                ));
            }
            session.metadata_gate.lock().check(None, false)?;
            session.driver.search_data(&request).await
        }
    }

    pub fn usage(&self) -> ResourceUsage {
        let (retained_bytes, result_count) = self.results.usage();
        let grid_runs = {
            #[cfg(unix)]
            {
                self.snowflake_grid.active_run_count()
            }
            #[cfg(not(unix))]
            {
                0
            }
        };
        ResourceUsage {
            retained_bytes,
            temporary_result_bytes: self.results.disk_usage(),
            result_count,
            session_count: self.sessions.lock().len(),
            active_runs: self.jobs.lock().len() + self.exports.lock().len() + grid_runs,
        }
    }
    /// Retire fixture work admitted before the configuration-gate snapshot.
    /// New work may proceed afterward; a real shutdown remains irreversible.
    #[cfg(any(test, feature = "test-harness"))]
    pub async fn reset_fixtures(&self) -> AppResult<()> {
        let _recovery_activity = self.begin_recovery_activity()?;
        #[cfg(unix)]
        let cleanup;
        let pending = {
            // Publication shares this gate: a setup either enters the session
            // snapshot or is cancelled before it can publish.
            let _configuration = self.configuration_mutation_guard()?;
            let pending = self.connection_attempts.cancel_fixture_snapshot();
            for export in self.exports.lock().values() {
                export.token.cancel();
            }
            self.file_capabilities.clear();
            let ids = self.sessions.lock().keys().cloned().collect::<Vec<_>>();
            for id in ids {
                self.disconnect(&id);
            }
            #[cfg(unix)]
            {
                self.snowflake_grid.close_all_helpers();
                cleanup = self.snowflake_cleanup.snapshot();
            }
            pending
        };
        // Setup owners need the configuration gate to unwind; never await them
        // while holding it. Leave the registry's shutdown barrier untouched.
        self.connection_attempts
            .wait_fixture_snapshot(&pending)
            .await;
        #[cfg(unix)]
        self.snowflake_cleanup
            .wait_snapshot(&cleanup, Duration::from_secs(10))
            .await?;
        Ok(())
    }

    pub fn shutdown(&self) {
        // Publication holds the same gate: either it wins and enters this
        // snapshot, or the irreversible attempt barrier rejects publication.
        let _configuration = self.configuration_gate.lock();
        self.connection_attempts.shutdown();
        self.cancel_recovery_for_shutdown();
        self.dml_tasks.close();
        for export in self.exports.lock().values() {
            export.token.cancel();
        }
        self.file_capabilities.clear();
        let ids = self.sessions.lock().keys().cloned().collect::<Vec<_>>();
        for id in ids {
            self.disconnect(&id);
        }
        #[cfg(unix)]
        self.snowflake_grid.close_all_helpers();
    }

    /// Await local setup owners after closing admission and existing sessions.
    /// Driver close currently requests logical shutdown; this is not a receipt
    /// of remote rollback or completed asynchronous engine cleanup.
    pub async fn shutdown_and_wait(&self) -> AppResult<()> {
        self.shutdown();
        tokio::time::timeout(Duration::from_secs(10), self.wait_metadata_recovery_idle())
            .await.map_err(|_| AppError::new(ErrorCode::Conflict,
                "Local metadata recovery is still finishing; keep Joinery open and try closing again"))?;
        self.connection_attempts.wait_idle().await;
        // Child reaping does not establish that its Studio owner has persisted
        // retained acknowledgements. A timeout keeps the window open and the
        // owner alive; a later shutdown can await the same tracker again.
        tokio::time::timeout(Duration::from_secs(10), self.dml_tasks.wait())
            .await
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Conflict,
                    "A database write is still recording its outcome; keep Joinery open and try closing again",
                )
            })?;
        self.flush_pending_snowflake_receipts()?;
        self.flush_pending_history()?;
        // Ordinary admission permits concurrent in-flight cells. After the
        // task drain, any remaining cell means its guard did not settle it.
        if !self.pending_history.lock().is_empty()
            || !self.pending_snowflake_receipts.lock().is_empty()
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "An admitted operation has not settled locally; keep Joinery open and inspect pending recording before closing",
            ));
        }
        self.flush_console_recordings()?;
        #[cfg(unix)]
        self.snowflake_cleanup
            .wait_all(Duration::from_secs(10))
            .await?;
        #[cfg(unix)]
        {
            // Reaping is an exact local retirement witness, never remote rollback.
            self.flush_snowflake_grid_outcomes()?;
            if !self.batch_outcomes.is_empty() || !self.snowflake_grid.is_empty() {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "A Snowflake edit outcome or owner is still recording locally; keep Joinery open and try closing again",
                ));
            }
        }
        self.console.release_journal();
        Ok(())
    }
}

fn snowflake_runtime_unavailable() -> AppError {
    AppError::new(
        ErrorCode::Unsupported,
        "The verified Snowflake SDK runtime is unavailable. Use a macOS ARM64 build with its prepared Snowflake resources.",
    )
}

fn connection_cancelled() -> AppError {
    AppError::new(
        ErrorCode::Cancelled,
        "This connection request was cancelled or the app is closing",
    )
}

fn browse_sql_for_engine(
    engine: EngineKind,
    schema: &TableSchema,
    query: &BrowseQuery,
    after: Option<&[Option<String>]>,
) -> AppResult<String> {
    match engine {
        EngineKind::Postgres => {
            crate::drivers::postgres::browse_sql(schema, query, after).map(|(sql, _)| sql)
        }
        EngineKind::Mysql => {
            crate::drivers::mysql::browse_sql(schema, query, after).map(|(sql, _)| sql)
        }
        EngineKind::Snowflake | EngineKind::Redis => Err(AppError::new(
            ErrorCode::Unsupported,
            "Sorting and filtering table data is unavailable for this engine",
        )),
    }
}

fn classify_browse_sql(
    session_id: &str,
    profile: &ConnectionProfile,
    sql: &str,
) -> AppResult<(PreparedPlan, bool)> {
    let plan = safety::prepare(session_id, profile, sql)?;
    if plan.statements.len() != 1 {
        return Err(AppError::invalid(
            "Table filter and order clauses must form a single SELECT query",
        ));
    }
    let non_read = plan.statements[0].kind != StatementKind::Read;
    if safety::effective_mode(profile) == SafeMode::ReadOnly && non_read {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "This connection is read-only. Table filter and order clauses must be pure read expressions.",
        ));
    }
    let needs_review = non_read || plan.requires_approval;
    Ok((plan, needs_review))
}

#[cfg(not(unix))]
impl Studio {
    pub async fn prepare_snowflake_grid_plan(
        &self,
        _request: &PrepareSnowflakeGridEditsRequest,
    ) -> AppResult<ChangePreview> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Snowflake generated edits are unavailable on this platform",
        ))
    }

    pub async fn execute_snowflake_grid_plan(
        self: &Arc<Self>,
        _plan_id: &str,
        _approval: Option<&str>,
    ) -> AppResult<SnowflakeGridResult> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Snowflake generated edits are unavailable on this platform",
        ))
    }

    pub async fn cancel_snowflake_grid_plan(&self, _plan_id: &str) -> AppResult<Option<String>> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Snowflake generated edits are unavailable on this platform",
        ))
    }

    pub async fn preview_snowflake_grid_transaction_relation(
        &self,
        _transaction_id: &str,
        _request: &PreviewRelationRequest,
    ) -> AppResult<RelationPreview> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Snowflake generated edits are unavailable on this platform",
        ))
    }

    pub async fn execute_snowflake_grid_finish(
        self: &Arc<Self>,
        _transaction_id: &str,
        _action: SnowflakeGridFinishAction,
    ) -> AppResult<SnowflakeGridResult> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Snowflake generated edits are unavailable on this platform",
        ))
    }
}

fn expired() -> AppError {
    AppError::new(
        ErrorCode::ApprovalExpired,
        "The query preview expired or was replaced. Review it again before running.",
    )
}

struct Output<'a> {
    studio: &'a Studio,
    session_id: &'a str,
    run_id: &'a str,
    index: usize,
    provenance: StatementProvenance,
    current: Option<String>,
    truncated: bool,
    viewer_closed: bool,
    active_started: Instant,
    first_row_ms: &'a mut Option<u32>,
    events: &'a EventHandler,
}
impl Output<'_> {
    fn discard(&mut self) {
        self.viewer_closed = true;
        if let Some(id) = self.current.take() {
            self.studio.release_result(&id);
        }
    }
    fn fail(&mut self) {
        if let Some(id) = self.current.take()
            && let Ok(result) = self.studio.results.finish(&id, true)
        {
            (self.events)(QueryEvent::Result { result });
        }
    }
}
impl QuerySink for Output<'_> {
    fn limit(&mut self, reason: &str) {
        self.truncated = true;
        (self.events)(QueryEvent::Notice {
            message: reason.into(),
        });
    }
    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()> {
        self.describe_actual(columns, None, None)
    }
    fn describe_with_origins(
        &mut self,
        columns: Vec<ColumnInfo>,
        origins: Vec<ColumnOrigin>,
    ) -> AppResult<()> {
        self.describe_actual(columns, Some(origins), None)
    }
    fn describe_mysql_with_origins(
        &mut self,
        columns: Vec<ColumnInfo>,
        origins: Vec<MysqlColumnOrigin>,
    ) -> AppResult<()> {
        self.describe_actual(columns, None, Some(origins))
    }
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        self.append_rows(rows)
    }
    fn complete(&mut self, affected: u64) -> AppResult<()> {
        self.complete_result(affected)
    }
}
impl Output<'_> {
    fn describe_actual(
        &mut self,
        columns: Vec<ColumnInfo>,
        origins: Option<Vec<ColumnOrigin>>,
        mysql_origins: Option<Vec<MysqlColumnOrigin>>,
    ) -> AppResult<()> {
        if self.viewer_closed {
            return Ok(());
        }
        let (resolved_columns, resolved_schema, resolved_editability) = if columns.is_empty() {
            (columns, None, None)
        } else {
            match &self.provenance {
                StatementProvenance::None => (columns, None, None),
                StatementProvenance::ReadOnly { reason } => (
                    columns,
                    None,
                    Some(ResultEditability::ReadOnly {
                        reason: reason.clone(),
                    }),
                ),
                StatementProvenance::Candidate {
                    candidate,
                    live_schema,
                    identity,
                    safe_mode,
                } => {
                    if identity.as_ref().is_some_and(|identity| !match identity {
                        EditIdentity::Postgres { .. } => origins
                            .as_ref()
                            .is_some_and(|origins| identity.matches_projection(&columns, origins)),
                        EditIdentity::Mysql { .. } => {
                            mysql_origins.as_ref().is_some_and(|origins| {
                                identity.matches_mysql_projection(&columns, origins)
                            })
                        }
                    }) {
                        (columns, None, Some(ResultEditability::ReadOnly { reason: "The actual result does not match the original table identity. Read the table again before editing.".into() }))
                    } else {
                        match safety::verify_result_provenance(
                            candidate,
                            live_schema,
                            &columns,
                            *safe_mode,
                        ) {
                            safety::ProvenanceResolution::Editable {
                                schema,
                                columns: enriched,
                                editability,
                            } => (enriched, Some(*schema), Some(editability)),
                            safety::ProvenanceResolution::ReadOnly { reason } => {
                                (columns, None, Some(ResultEditability::ReadOnly { reason }))
                            }
                        }
                    }
                }
            }
        };
        let identity = if resolved_schema.is_some() {
            match &self.provenance {
                StatementProvenance::Candidate { identity, .. } => identity.clone(),
                _ => None,
            }
        } else {
            None
        };
        let info = match self.studio.results.create_with_editability(
            self.run_id,
            self.index,
            resolved_columns,
            resolved_schema,
            resolved_editability,
        ) {
            Ok(info) => info,
            Err(error) => {
                self.limit(&error.message);
                self.discard();
                return Ok(());
            }
        };
        self.studio.results.set_edit_identity(&info.id, identity)?;
        self.studio
            .result_sessions
            .lock()
            .insert(info.id.clone(), self.session_id.into());
        self.current = Some(info.id.clone());
        self.truncated = false;
        if !(self.events)(QueryEvent::Result { result: info }) {
            self.discard();
        }
        Ok(())
    }
    fn append_rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        if self.viewer_closed {
            return Ok(false);
        }
        let id = self
            .current
            .as_deref()
            .ok_or_else(|| AppError::internal("Rows arrived before column metadata"))?;
        if !rows.is_empty() && self.first_row_ms.is_none() {
            *self.first_row_ms = Some(
                self.active_started
                    .elapsed()
                    .as_millis()
                    .min(u32::MAX as u128) as u32,
            );
        }
        let retained = match self.studio.results.append(id, rows) {
            Ok(value) => value,
            Err(error) => {
                self.limit(&error.message);
                // An append is atomic. Keep the valid retained prefix fetchable
                // and mark it truncated. The driver remains responsible for SQL
                // cancellation/terminal outcomes; cache retention proves neither.
                return Ok(false);
            }
        };
        self.truncated |= !retained;
        let info = self.studio.results.info(id)?;
        if !(self.events)(QueryEvent::Progress {
            result_id: id.into(),
            retained_rows: info.retained_rows,
        }) {
            self.discard();
            return Ok(false);
        }
        Ok(retained)
    }
    fn complete_result(&mut self, _affected: u64) -> AppResult<()> {
        if self.viewer_closed {
            return Ok(());
        }
        if self.current.is_none() {
            self.describe(Vec::new())?;
        }
        if self.viewer_closed {
            return Ok(());
        }
        let id = self.current.take().unwrap();
        let result = self.studio.results.finish(&id, self.truncated)?;
        (self.events)(QueryEvent::Result { result });
        Ok(())
    }
}

fn merge_virtual_fks_into_table_schema(
    schema: &mut TableSchema,
    vfks: &[VirtualForeignKey],
    connection_id: &str,
    include_inbound: bool,
) {
    let col_names: std::collections::HashSet<&str> =
        schema.columns.iter().map(|c| c.name.as_str()).collect();
    let mut newly_fk_columns: std::collections::HashSet<String> = std::collections::HashSet::new();
    for vfk in vfks {
        if vfk.connection_id != connection_id {
            continue;
        }
        if vfk.source == schema.table && vfk.columns.iter().all(|c| col_names.contains(c.as_str()))
        {
            let duplicate = schema.foreign_keys.iter().any(|fk| {
                fk.target == vfk.target
                    && fk.columns == vfk.columns
                    && fk.target_columns == vfk.target_columns
            });
            if !duplicate {
                for col in &vfk.columns {
                    newly_fk_columns.insert(col.clone());
                }
                schema.foreign_keys.push(ForeignKey {
                    name: vfk.name.clone(),
                    columns: vfk.columns.clone(),
                    target: vfk.target.clone(),
                    target_columns: vfk.target_columns.clone(),
                    virtual_relationship: true,
                });
            }
        }
        if include_inbound
            && vfk.target == schema.table
            && vfk
                .target_columns
                .iter()
                .all(|c| col_names.contains(c.as_str()))
        {
            let duplicate = schema.inbound_foreign_keys.iter().any(|ifk| {
                ifk.source == vfk.source
                    && ifk.columns == vfk.columns
                    && ifk.target_columns == vfk.target_columns
            });
            if !duplicate {
                schema.inbound_foreign_keys.push(InboundForeignKey {
                    name: vfk.name.clone(),
                    source: vfk.source.clone(),
                    columns: vfk.columns.clone(),
                    target_columns: vfk.target_columns.clone(),
                    virtual_relationship: true,
                });
            }
        }
    }
    if !newly_fk_columns.is_empty() {
        for col in &mut schema.columns {
            if newly_fk_columns.contains(&col.name) {
                col.foreign_key = true;
            }
        }
    }
}

fn merge_virtual_fks_into_inbound(
    keys: &mut Vec<InboundForeignKey>,
    vfks: &[VirtualForeignKey],
    connection_id: &str,
    table: &TableRef,
) {
    for vfk in vfks {
        if vfk.connection_id != connection_id || &vfk.target != table {
            continue;
        }
        let duplicate = keys.iter().any(|ifk| {
            ifk.source == vfk.source
                && ifk.columns == vfk.columns
                && ifk.target_columns == vfk.target_columns
        });
        if !duplicate {
            keys.push(InboundForeignKey {
                name: vfk.name.clone(),
                source: vfk.source.clone(),
                columns: vfk.columns.clone(),
                target_columns: vfk.target_columns.clone(),
                virtual_relationship: true,
            });
        }
    }
}

fn merge_virtual_fks_into_diagram(
    diagram: &mut SchemaDiagram,
    vfks: &[VirtualForeignKey],
    connection_id: &str,
) {
    let mut table_columns: std::collections::HashMap<&str, std::collections::HashSet<&str>> =
        std::collections::HashMap::with_capacity(diagram.tables.len());
    for node in &diagram.tables {
        table_columns.insert(
            node.table.name.as_str(),
            node.columns.iter().map(|c| c.name.as_str()).collect(),
        );
    }
    for vfk in vfks {
        if vfk.connection_id != connection_id || vfk.source.schema != diagram.schema {
            continue;
        }
        let Some(src_cols) = table_columns.get(vfk.source.name.as_str()) else {
            continue;
        };
        if !vfk.columns.iter().all(|c| src_cols.contains(c.as_str())) {
            continue;
        }
        if vfk.target.schema == diagram.schema {
            let Some(dst_cols) = table_columns.get(vfk.target.name.as_str()) else {
                continue;
            };
            if !vfk
                .target_columns
                .iter()
                .all(|c| dst_cols.contains(c.as_str()))
            {
                continue;
            }
        }
        let duplicate = diagram.foreign_keys.iter().any(|fk| {
            fk.source == vfk.source
                && fk.target == vfk.target
                && fk.columns == vfk.columns
                && fk.target_columns == vfk.target_columns
        });
        if !duplicate {
            diagram.foreign_keys.push(DiagramForeignKey {
                name: vfk.name.clone(),
                source: vfk.source.clone(),
                columns: vfk.columns.clone(),
                target: vfk.target.clone(),
                target_columns: vfk.target_columns.clone(),
                virtual_relationship: true,
            });
        }
    }
}

#[cfg(test)]
mod browse_classification_tests {
    use super::*;

    #[test]
    fn result_append_failure_keeps_fetchable_truncated_prefix() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let columns = schema(EngineKind::Postgres).columns;
        let info = studio
            .results
            .create("retention-failure", 0, columns, None)
            .unwrap();
        let original = vec![Some("1".into()), Some("2".into()), Some("prefix".into())];
        assert!(
            studio
                .results
                .append(&info.id, vec![original.clone()])
                .unwrap()
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let handler: EventHandler = Arc::new(move |event| {
            captured.lock().push(event);
            true
        });
        let mut first_row_ms = None;
        let mut output = Output {
            studio: &studio,
            session_id: "fixture",
            run_id: "retention-failure",
            index: 0,
            provenance: StatementProvenance::None,
            current: Some(info.id.clone()),
            truncated: false,
            viewer_closed: false,
            active_started: Instant::now(),
            first_row_ms: &mut first_row_ms,
            events: &handler,
        };
        assert!(
            !output
                .append_rows(vec![vec![Some("invalid width".into())]])
                .unwrap()
        );
        output.complete_result(0).unwrap();
        assert_eq!(studio.results.row(&info.id, 0).unwrap(), original);
        assert_eq!(
            studio.results.info(&info.id).unwrap().status,
            RetentionStatus::Truncated
        );
        let events = events.lock();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, QueryEvent::Notice { .. }))
        );
        assert!(events.iter().any(|event| matches!(event, QueryEvent::Result { result } if result.id == info.id && result.status == RetentionStatus::Truncated)));
    }

    fn profile(engine: EngineKind, safe_mode: SafeMode) -> ConnectionProfile {
        ConnectionProfile {
            id: new_id(),
            name: "BrowseTest".into(),
            group_id: "local".into(),
            engine,
            host: "127.0.0.1".into(),
            port: 5432,
            database: "joinery".into(),
            username: "joinery".into(),
            environment: Environment::Local,
            safe_mode,
            tls_mode: TlsMode::Disable,
            ca_cert_path: None,
            ssh: None,
            remember_password: false,
            store_query_text: false,
            store_drafts: false,
            snowflake: None,
        }
    }

    fn schema(engine: EngineKind) -> TableSchema {
        let (int_type, text_type) = if engine == EngineKind::Mysql {
            ("bigint", "varchar(64)")
        } else {
            ("bigint", "text")
        };
        TableSchema {
            table: TableRef {
                schema: if engine == EngineKind::Mysql {
                    "joinery".into()
                } else {
                    "public".into()
                },
                name: "orders".into(),
            },
            columns: vec![
                ColumnInfo {
                    id: "c0".into(),
                    name: "id".into(),
                    database_type: int_type.into(),
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
                    name: "bucket".into(),
                    database_type: int_type.into(),
                    kind: ColumnKind::Integer,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c2".into(),
                    name: "label".into(),
                    database_type: text_type.into(),
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
        }
    }

    #[test]
    fn pure_read_raw_clauses_with_typed_filters_need_no_approval_on_pg_and_mysql() {
        for engine in [EngineKind::Postgres, EngineKind::Mysql] {
            let schema = schema(engine);
            let query = BrowseQuery {
                sort: vec![SortKey {
                    column: "bucket".into(),
                    descending: true,
                }],
                filters: vec![
                    ColumnFilter {
                        column: "label".into(),
                        op: FilterOp::Contains,
                        value: Some("ok".into()),
                    },
                    ColumnFilter {
                        column: "bucket".into(),
                        op: FilterOp::Gte,
                        value: Some("1".into()),
                    },
                ],
                where_clause: Some("bucket IS NOT NULL AND LOWER(label) <> 'skip'".into()),
                order_clause: None,
            };
            let sql = browse_sql_for_engine(engine, &schema, &query, None).unwrap();
            for mode in [SafeMode::ReadOnly, SafeMode::Confirm, SafeMode::Silent] {
                let (_, needs_review) =
                    classify_browse_sql("s", &profile(engine, mode), &sql).unwrap();
                assert!(!needs_review, "{engine:?} {mode:?}: {sql}");
            }

            let custom_order = BrowseQuery {
                where_clause: Some("id > 0".into()),
                order_clause: Some("COALESCE(bucket, 0) DESC, id ASC".into()),
                ..Default::default()
            };
            let sql = browse_sql_for_engine(engine, &schema, &custom_order, None).unwrap();
            let (_, needs_review) =
                classify_browse_sql("s", &profile(engine, SafeMode::ReadOnly), &sql).unwrap();
            assert!(!needs_review, "{engine:?} custom order: {sql}");
        }
    }

    #[test]
    fn non_read_raw_clauses_are_blocked_in_read_only_and_reviewed_in_writable_modes() {
        for (engine, expr) in [
            (EngineKind::Postgres, "pg_advisory_lock(1) IS NULL"),
            (EngineKind::Mysql, "GET_LOCK('joinery', 0) = 1"),
        ] {
            let schema = schema(engine);
            let query = BrowseQuery {
                where_clause: Some(expr.into()),
                ..Default::default()
            };
            let sql = browse_sql_for_engine(engine, &schema, &query, None).unwrap();
            assert_eq!(
                classify_browse_sql("s", &profile(engine, SafeMode::ReadOnly), &sql)
                    .unwrap_err()
                    .code,
                ErrorCode::PolicyBlocked
            );
            for mode in [SafeMode::Silent, SafeMode::Confirm, SafeMode::PasswordGated] {
                let (_, needs_review) =
                    classify_browse_sql("s", &profile(engine, mode), &sql).unwrap();
                assert!(needs_review, "{engine:?} {mode:?} must require review");
            }
        }
    }

    #[derive(Default)]
    struct RecordingDriver {
        calls: Mutex<Vec<(String, crate::params::BoundStatement)>>,
        transaction_error: Option<ErrorCode>,
        closed: AtomicBool,
        context_enabled: bool,
        contexts_error: Option<ErrorCode>,
        switch_error: Option<ErrorCode>,
        relation_calls: std::sync::atomic::AtomicUsize,
        relation_result: Mutex<Option<AppResult<BrowseCatalog>>>,
        definition_calls: Mutex<Vec<DefineRelationRequest>>,
        constraints_calls: Mutex<Vec<InspectRelationConstraintsRequest>>,
        check_constraints_calls: Mutex<Vec<InspectRelationCheckConstraintsRequest>>,
        mapping_calls: Mutex<Vec<InspectRelationConstraintMappingsRequest>>,
        relation_invalidated: AtomicBool,
        panic_during_invalidation: AtomicBool,
        panic_during_close: AtomicBool,
        switch_saw_invalidation: AtomicBool,
        close_during_relation: bool,
        mode_calls: std::sync::atomic::AtomicUsize,
        runtime_override: Mutex<Option<SessionRuntimeState>>,
        close_during_context: bool,
        state_calls: std::sync::atomic::AtomicUsize,
        schema_switch_calls: Mutex<Vec<SwitchSchemaRequest>>,
        schema_switch_options: Mutex<Option<SessionContextOptions>>,
        close_during_schema_switch: bool,
        dml_calls: Mutex<Vec<SnowflakeDmlRequest>>,
        dml_result: Mutex<Option<SnowflakeDmlResult>>,
        ddl_calls: Mutex<Vec<SnowflakeDdlRequest>>,
        ddl_preflight: Mutex<Option<(SnowflakeDdlPreflightRequest, SnowflakeDdlPreflightResponse)>>,
        ddl_result: Mutex<Option<SnowflakeDdlResult>>,
        boundary_calls: Mutex<Vec<TransactionControlRequest>>,
        boundary_result: Mutex<Option<TransactionControlResult>>,
        boundary_predispatch_error: Mutex<Option<ErrorCode>>,
        boundary_barrier: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    }

    #[async_trait::async_trait]
    impl Driver for RecordingDriver {
        fn invalidate_relation_catalog(&self) {
            assert!(
                !self.panic_during_invalidation.load(Ordering::Acquire),
                "synthetic invalidation panic after admission"
            );
            self.relation_invalidated.store(true, Ordering::Release);
        }
        async fn relation_catalog(&self) -> AppResult<BrowseCatalog> {
            self.relation_calls.fetch_add(1, Ordering::Relaxed);
            if self.close_during_relation {
                self.close();
            }
            if let Some(result) = self.relation_result.lock().clone() {
                return result;
            }
            Ok(BrowseCatalog {
                revision: "r".into(),
                database: "fixture".into(),
                relations: vec![],
                identity: CompletionIdentity {
                    database: "fixture".into(),
                    role: "reader".into(),
                    search_path: vec!["public".into()],
                    case_sensitive_table_names: Some(true),
                    context_fingerprint: None,
                },
            })
        }
        async fn define_relation(
            &self,
            request: &DefineRelationRequest,
        ) -> AppResult<RelationDefinition> {
            self.definition_calls.lock().push(request.clone());
            if self.close_during_relation {
                self.close();
            }
            Ok(RelationDefinition {
                revision: request.revision.clone(),
                target_id: request.target_id.clone(),
                database: "fixture".into(),
                table: TableRef {
                    schema: "public".into(),
                    name: "item".into(),
                },
                ddl: "create or replace table item (id number);".into(),
            })
        }
        async fn inspect_relation_constraints(
            &self,
            request: &InspectRelationConstraintsRequest,
        ) -> AppResult<RelationConstraints> {
            self.constraints_calls.lock().push(request.clone());
            if self.close_during_relation {
                self.close();
            }
            Ok(RelationConstraints {
                revision: request.revision.clone(),
                target_id: request.target_id.clone(),
                database: "fixture".into(),
                table: TableRef {
                    schema: "public".into(),
                    name: "item".into(),
                },
                constraints: vec![],
            })
        }
        async fn inspect_relation_check_constraints(
            &self,
            request: &InspectRelationCheckConstraintsRequest,
        ) -> AppResult<RelationCheckConstraints> {
            self.check_constraints_calls.lock().push(request.clone());
            if self.close_during_relation {
                self.close();
            }
            Ok(RelationCheckConstraints {
                revision: request.revision.clone(),
                target_id: request.target_id.clone(),
                database: "fixture".into(),
                table: TableRef {
                    schema: "public".into(),
                    name: "item".into(),
                },
                constraints: vec![],
            })
        }
        async fn inspect_relation_constraint_mappings(
            &self,
            request: &InspectRelationConstraintMappingsRequest,
        ) -> AppResult<RelationConstraintMappings> {
            self.mapping_calls.lock().push(request.clone());
            if self.close_during_relation {
                self.close();
            }
            Ok(RelationConstraintMappings {
                revision: request.revision.clone(),
                target_id: request.target_id.clone(),
                database: "fixture".into(),
                table: TableRef {
                    schema: "public".into(),
                    name: "item".into(),
                },
                mappings: vec![],
            })
        }
        async fn session_state(&self, session_id: &str) -> AppResult<SessionRuntimeState> {
            self.state_calls.fetch_add(1, Ordering::Relaxed);
            if !self.context_enabled {
                return Err(AppError::new(ErrorCode::Unsupported, "No fixture context"));
            }
            if self.close_during_context {
                self.close();
            }
            if let Some(state) = self.runtime_override.lock().clone() {
                return Ok(SessionRuntimeState {
                    session_id: session_id.into(),
                    ..state
                });
            }
            Ok(SessionRuntimeState {
                session_id: session_id.into(),
                transaction_mode: TransactionMode::AutoCommit,
                transaction_state: TransactionState::Idle,
                context_kind: SessionContextKind::Schema,
                active_context: Some("public".into()),
                database: "fixture".into(),
                role: Some("reader".into()),
                warehouse: None,
                search_path: vec!["public".into()],
                context_fingerprint: None,
                transaction_observation: None,
                transaction_control_authority: None,
            })
        }
        async fn list_contexts(&self) -> AppResult<Vec<String>> {
            match self.contexts_error {
                Some(code) => Err(AppError::new(code, "Fixture context list denied")),
                None => Ok(vec!["public".into()]),
            }
        }
        async fn switch_context(&self, _target: &str) -> AppResult<()> {
            self.switch_saw_invalidation.store(
                self.relation_invalidated.load(Ordering::Acquire),
                Ordering::Release,
            );
            self.switch_error.map_or(Ok(()), |code| {
                Err(AppError::new(code, "Fixture switch failed"))
            })
        }
        async fn switch_schema(
            &self,
            _session_id: &str,
            request: &SwitchSchemaRequest,
        ) -> AppResult<SessionContextOptions> {
            self.schema_switch_calls.lock().push(request.clone());
            self.switch_saw_invalidation.store(
                self.relation_invalidated.load(Ordering::Acquire),
                Ordering::Release,
            );
            if self.close_during_schema_switch {
                self.close();
            }
            if let Some(code) = self.switch_error {
                return Err(AppError::new(code, "Fixture schema switch failed"));
            }
            self.schema_switch_options
                .lock()
                .clone()
                .ok_or_else(|| AppError::internal("Missing fixture schema switch result"))
        }
        async fn execute(
            &self,
            _sql: &str,
            _kind: StatementKind,
            sink: &mut dyn QuerySink,
        ) -> AppResult<u64> {
            sink.complete(1)?;
            Ok(1)
        }
        async fn execute_bound(
            &self,
            sql: &str,
            _kind: StatementKind,
            bound: &crate::params::BoundStatement,
            sink: &mut dyn QuerySink,
        ) -> AppResult<u64> {
            self.calls.lock().push((sql.to_owned(), bound.clone()));
            sink.complete(1)?;
            Ok(1)
        }
        async fn cancel(&self) -> AppResult<()> {
            Ok(())
        }
        fn validate_transaction_control(
            &self,
            request: &TransactionControlRequest,
        ) -> AppResult<()> {
            let state = self.runtime_override.lock();
            let authority = state
                .as_ref()
                .and_then(|state| state.transaction_control_authority.as_ref());
            let valid = match authority {
                Some(TransactionControlAuthority::Observed {
                    revision,
                    allowed_actions,
                }) => revision == &request.revision && allowed_actions.contains(&request.action),
                Some(TransactionControlAuthority::RollbackOnly { revision }) => {
                    revision == &request.revision
                        && request.action == ExplicitTransactionAction::Rollback
                }
                None => false,
            };
            if valid {
                Ok(())
            } else {
                Err(AppError::new(
                    ErrorCode::Conflict,
                    "Stale fixture authority",
                ))
            }
        }
        async fn transaction_control(
            &self,
            _session_id: &str,
            request: &TransactionControlRequest,
        ) -> AppResult<TransactionControlResult> {
            if let Some(code) = *self.boundary_predispatch_error.lock() {
                return Err(AppError::new(
                    code,
                    "Fixture rejected before remote submission",
                ));
            }
            self.boundary_calls.lock().push(request.clone());
            let result = self.boundary_result.lock().clone().unwrap();
            if let Some((started, release)) = &self.boundary_barrier {
                started.notify_one();
                release.notified().await;
            }
            Ok(result)
        }
        fn validate_snowflake_dml(&self, request: &SnowflakeDmlRequest) -> AppResult<()> {
            let state = self.runtime_override.lock();
            let state = state.as_ref().unwrap();
            if !matches!(&state.transaction_control_authority, Some(TransactionControlAuthority::Observed { revision, .. }) if revision == &request.revision)
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Stale fixture DML authority",
                ));
            }
            safety::snowflake_dml::compile(
                &request.sql,
                Some(&request.parameters),
                &state.database,
            )?;
            Ok(())
        }
        async fn snowflake_dml(
            &self,
            _session_id: &str,
            request: &SnowflakeDmlRequest,
        ) -> AppResult<SnowflakeDmlResult> {
            self.dml_calls.lock().push(request.clone());
            let result = self.dml_result.lock().clone().unwrap();
            if let Some((started, release)) = &self.boundary_barrier {
                started.notify_one();
                release.notified().await;
            }
            Ok(result)
        }
        async fn preflight_snowflake_ddl(
            &self,
            _session_id: &str,
            request: &SnowflakeDdlPreflightRequest,
        ) -> AppResult<SnowflakeDdlPreflightResponse> {
            let preflight = self.ddl_preflight.lock();
            let (expected, response) = preflight.as_ref().unwrap();
            if expected != request {
                return Err(AppError::invalid("Fixture preflight mismatch"));
            }
            Ok(response.clone())
        }
        fn validate_snowflake_ddl(&self, request: &SnowflakeDdlRequest) -> AppResult<()> {
            let preflight = self.ddl_preflight.lock();
            let (expected, response) = preflight.as_ref().ok_or_else(expired)?;
            if request.action != expected.action
                || request.revision != response.revision
                || request.sql != response.sql
            {
                return Err(expired());
            }
            Ok(())
        }
        async fn snowflake_ddl(
            &self,
            _session_id: &str,
            request: &SnowflakeDdlRequest,
        ) -> AppResult<SnowflakeDdlResult> {
            self.ddl_calls.lock().push(request.clone());
            let result = self.ddl_result.lock().clone().unwrap();
            if let Some((started, release)) = &self.boundary_barrier {
                started.notify_one();
                release.notified().await;
            }
            Ok(result)
        }
        async fn commit_transaction(&self) -> AppResult<()> {
            self.transaction_error
                .map_or(Ok(()), |code| Err(AppError::new(code, "control failure")))
        }
        async fn set_transaction_mode(&self, _mode: TransactionMode) -> AppResult<()> {
            self.mode_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        async fn rollback_transaction(&self) -> AppResult<()> {
            self.commit_transaction().await
        }
        async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
            Ok(Vec::new())
        }
        async fn schema(&self, table: &TableRef) -> AppResult<TableSchema> {
            Ok(TableSchema {
                table: table.clone(),
                columns: Vec::new(),
                primary_key: Vec::new(),
                foreign_keys: Vec::new(),
                inbound_foreign_keys: Vec::new(),
                editable: false,
            })
        }
        async fn browse(
            &self,
            _schema: &TableSchema,
            _after: Option<Vec<Option<String>>>,
        ) -> AppResult<crate::drivers::BrowseRows> {
            Ok(crate::drivers::BrowseRows {
                rows: Vec::new(),
                next_key: None,
                has_more: false,
                truncated: false,
            })
        }
        async fn apply(&self, _schema: &TableSchema, _rows: &[RowMutation]) -> AppResult<()> {
            Ok(())
        }
        async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
            Ok(Vec::new())
        }
        async fn cancel_process(&self, _id: &str, _identity: &str) -> AppResult<()> {
            Ok(())
        }
        async fn health(&self) -> AppResult<u32> {
            Ok(1)
        }
        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::Acquire)
        }
        fn close(&self) {
            self.closed.store(true, Ordering::Release);
            assert!(
                !self.panic_during_close.load(Ordering::Acquire),
                "synthetic close panic after returned original"
            );
        }
    }

    fn relation_test_session(
        studio: &Studio,
        driver: Arc<RecordingDriver>,
        enabled: bool,
    ) -> Arc<Session> {
        let profile = profile(EngineKind::Postgres, SafeMode::ReadOnly);
        let session = Arc::new(Session {
            rollback_memory: Mutex::new(None),
            #[cfg(unix)]
            snowflake_external: None,
            info: SessionInfo { console_mode: false, id: new_id(), connection_id: profile.id.clone(), engine: EngineKind::Postgres, server_version: "fixture".into(), latency_ms: 0, encrypted: false, capabilities: serde_json::from_value(serde_json::json!({"sql":true,"transactions":false,"cancellation":true,"tableBrowsing":false,"relationBrowsing":enabled,"tableEditing":false,"tableFiltering":false,"processes":false})).unwrap() },
            profile, driver, lane: tokio::sync::Mutex::new(()), cancellation_gate: tokio::sync::Mutex::new(()), active_run: Mutex::new(None), closed: AtomicBool::new(false), tunnel: None, completion: Mutex::new(None), metadata_gate: Mutex::new(MetadataGate::default()), observed_runtime: Mutex::new(None),
        });
        studio
            .sessions
            .lock()
            .insert(session.info.id.clone(), session.clone());
        session
    }

    fn explicit_control_fixture(
        mode: SafeMode,
        action: ExplicitTransactionAction,
        barrier: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    ) -> (
        tempfile::TempDir,
        Arc<Studio>,
        Arc<RecordingDriver>,
        Arc<Session>,
        TransactionControlRequest,
    ) {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let profile = profile(EngineKind::Snowflake, mode);
        // Persistence/policy fixture only; actual SDK profile validation is
        // covered separately, and no database connection is created here.
        studio.persistence.save_profile(&profile).unwrap();
        let id = new_id();
        let request = TransactionControlRequest {
            action,
            revision: new_id(),
        };
        let state = SessionRuntimeState {
            session_id: id.clone(),
            transaction_mode: TransactionMode::AutoCommit,
            transaction_state: TransactionState::Unknown,
            context_kind: SessionContextKind::Schema,
            active_context: Some("PUBLIC".into()),
            database: "DB".into(),
            role: None,
            warehouse: None,
            search_path: vec!["DB.PUBLIC".into()],
            context_fingerprint: Some("context".into()),
            transaction_observation: Some(TransactionObservation::Unavailable),
            transaction_control_authority: Some(if action == ExplicitTransactionAction::Rollback {
                TransactionControlAuthority::RollbackOnly {
                    revision: request.revision.clone(),
                }
            } else {
                TransactionControlAuthority::Observed {
                    revision: request.revision.clone(),
                    allowed_actions: vec![action],
                }
            }),
        };
        let driver = Arc::new(RecordingDriver {
            runtime_override: Mutex::new(Some(state.clone())),
            boundary_result: Mutex::new(Some(TransactionControlResult {
                action,
                boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
                state,
                retired: false,
                error: None,
            })),
            boundary_barrier: barrier,
            ..Default::default()
        });
        let session = Arc::new(Session {
            rollback_memory: Mutex::new(None),
            #[cfg(unix)]
            snowflake_external: None,
            info: SessionInfo { console_mode: false, id: id.clone(), connection_id: profile.id.clone(), engine: EngineKind::Snowflake,
                server_version: "synthetic".into(), latency_ms: 0, encrypted: true,
                capabilities: serde_json::from_value(serde_json::json!({"sql":true,"transactions":false,"explicitTransactionControls":true,"cancellation":true,"tableBrowsing":false,"tableEditing":false,"tableFiltering":false,"processes":false})).unwrap() },
            profile, driver: driver.clone(), lane: tokio::sync::Mutex::new(()),
            cancellation_gate: tokio::sync::Mutex::new(()), active_run: Mutex::new(None),
            closed: AtomicBool::new(false), tunnel: None, completion: Mutex::new(None), metadata_gate: Mutex::new(MetadataGate::default()), observed_runtime: Mutex::new(None),
        });
        studio.sessions.lock().insert(id, session.clone());
        (root, studio, driver, session, request)
    }

    #[cfg(test)]
    mod script_accounting_tests {
        include!("execution/script_accounting_tests.rs");
    }

    include!("execution/snowflake_dml_tests.rs");
    include!("execution/snowflake_ddl_tests.rs");
    include!("execution/snowflake_receipt_tests.rs");

    #[tokio::test]
    async fn explicit_boundary_policy_review_and_one_use_are_enforced_without_extra_observers() {
        for mode in [
            SafeMode::ReadOnly,
            SafeMode::Confirm,
            SafeMode::Guard,
            SafeMode::Silent,
        ] {
            for action in [
                ExplicitTransactionAction::Begin,
                ExplicitTransactionAction::Commit,
                ExplicitTransactionAction::Rollback,
            ] {
                let (_root, studio, driver, session, request) =
                    explicit_control_fixture(mode, action, None);
                let prepared = studio.prepare_transaction_control(&session.info.id, &request);
                if mode == SafeMode::ReadOnly && action != ExplicitTransactionAction::Rollback {
                    assert_eq!(prepared.unwrap_err().code, ErrorCode::PolicyBlocked);
                    assert!(driver.boundary_calls.lock().is_empty());
                    continue;
                }
                let plan = prepared.unwrap();
                assert_eq!(plan.statements[0].sql, action.sql());
                let approval = if plan.requires_approval {
                    assert_eq!(
                        studio
                            .execute_transaction_control(&plan.id, None)
                            .await
                            .unwrap_err()
                            .code,
                        ErrorCode::ApprovalRequired
                    );
                    assert!(driver.boundary_calls.lock().is_empty());
                    let next = studio
                        .prepare_transaction_control(&session.info.id, &request)
                        .unwrap();
                    let approval = studio.authorize(&next.id, None, None).unwrap();
                    Some((next, approval))
                } else {
                    None
                };
                let (plan, token) = approval
                    .as_ref()
                    .map_or((&plan, None), |(plan, token)| (plan, Some(token.as_str())));
                let result = studio
                    .execute_transaction_control(&plan.id, token)
                    .await
                    .unwrap();
                assert_eq!(
                    result.boundary_outcome,
                    TransactionBoundaryOutcome::Acknowledged
                );
                assert_eq!(*driver.boundary_calls.lock(), vec![request]);
                assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
                assert!(
                    studio
                        .execute_transaction_control(&plan.id, token)
                        .await
                        .is_err()
                );
                assert_eq!(driver.boundary_calls.lock().len(), 1);
            }
        }
    }

    #[tokio::test]
    async fn explicit_boundary_generic_route_and_changed_persisted_policy_never_dispatch() {
        for scenario in ["generic", "policy", "stale", "foreign-plan"] {
            let (_root, studio, driver, session, request) =
                explicit_control_fixture(SafeMode::Silent, ExplicitTransactionAction::Begin, None);
            let plan = studio
                .prepare_transaction_control(&session.info.id, &request)
                .unwrap();
            if scenario == "generic" {
                assert!(studio.execute(&plan.id, None, Arc::new(|_| true)).is_err());
            } else {
                if scenario == "policy" {
                    let mut profile = session.profile.clone();
                    profile.safe_mode = SafeMode::ReadOnly;
                    studio.persistence.save_profile(&profile).unwrap();
                } else if scenario == "stale" {
                    driver
                        .runtime_override
                        .lock()
                        .as_mut()
                        .unwrap()
                        .transaction_control_authority = None;
                } else {
                    let other = studio
                        .prepare_execution(&session.info.id, "SELECT 1")
                        .unwrap();
                    assert!(
                        studio
                            .execute_transaction_control(&other.id, None)
                            .await
                            .is_err()
                    );
                }
                assert!(
                    studio
                        .execute_transaction_control(&plan.id, None)
                        .await
                        .is_err()
                );
            }
            assert!(driver.boundary_calls.lock().is_empty(), "{scenario}");
        }
    }

    #[tokio::test]
    async fn explicit_boundary_unknown_outcome_cannot_publish_live_authority() {
        let (_root, studio, driver, session, request) =
            explicit_control_fixture(SafeMode::Silent, ExplicitTransactionAction::Begin, None);
        driver
            .boundary_result
            .lock()
            .as_mut()
            .unwrap()
            .boundary_outcome = TransactionBoundaryOutcome::Unknown;
        let plan = studio
            .prepare_transaction_control(&session.info.id, &request)
            .unwrap();
        let result = studio
            .execute_transaction_control(&plan.id, None)
            .await
            .unwrap();
        assert_eq!(result.boundary_outcome, TransactionBoundaryOutcome::Unknown);
        assert!(result.retired);
        assert!(result.state.transaction_control_authority.is_none());
        assert!(studio.session(&session.info.id).is_err());
        assert!(driver.is_closed());
        assert_eq!(driver.boundary_calls.lock().len(), 1);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn explicit_boundary_late_disconnect_keeps_ack_and_removes_authority() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (_root, studio, driver, session, request) = explicit_control_fixture(
            SafeMode::Silent,
            ExplicitTransactionAction::Begin,
            Some((entered.clone(), release.clone())),
        );
        let plan = studio
            .prepare_transaction_control(&session.info.id, &request)
            .unwrap();
        let owner = studio.clone();
        let task =
            tokio::spawn(async move { owner.execute_transaction_control(&plan.id, None).await });
        entered.notified().await;
        studio.disconnect(&session.info.id);
        release.notify_one();
        let result = task.await.unwrap().unwrap();
        assert_eq!(
            result.boundary_outcome,
            TransactionBoundaryOutcome::Acknowledged
        );
        assert!(result.retired);
        assert!(result.state.transaction_control_authority.is_none());
        assert_eq!(driver.boundary_calls.lock().len(), 1);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn explicit_boundary_audit_failure_cannot_trap_rollback_recovery() {
        for action in [
            ExplicitTransactionAction::Begin,
            ExplicitTransactionAction::Rollback,
        ] {
            let (root, studio, driver, session, request) =
                explicit_control_fixture(SafeMode::Silent, action, None);
            let plan = studio
                .prepare_transaction_control(&session.info.id, &request)
                .unwrap();
            let database = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
            database.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit_v8 BEGIN SELECT RAISE(ABORT, 'synthetic audit failure'); END;").unwrap();
            let result = studio.execute_transaction_control(&plan.id, None).await;
            assert_eq!(
                driver.boundary_calls.lock().len(),
                usize::from(action == ExplicitTransactionAction::Rollback)
            );
            if action == ExplicitTransactionAction::Rollback {
                assert_eq!(
                    result.unwrap().boundary_outcome,
                    TransactionBoundaryOutcome::Acknowledged
                );
            } else {
                assert!(result.is_err());
            }
        }
    }

    #[tokio::test]
    async fn explicit_boundary_queued_passphrase_change_revokes_prepared_action() {
        let (_root, studio, driver, session, request) =
            explicit_control_fixture(SafeMode::Silent, ExplicitTransactionAction::Begin, None);
        let plan = studio
            .prepare_transaction_control(&session.info.id, &request)
            .unwrap();
        let lane = session.lane.lock().await;
        let owner = studio.clone();
        let task =
            tokio::spawn(async move { owner.execute_transaction_control(&plan.id, None).await });
        tokio::task::yield_now().await;
        studio
            .set_passphrase("synthetic-long-passphrase", None)
            .unwrap();
        drop(lane);
        assert!(task.await.unwrap().is_err());
        assert!(driver.boundary_calls.lock().is_empty());
    }

    #[tokio::test]
    async fn explicit_boundary_production_requires_exact_plan_passphrase_authorization() {
        let (_root, studio, driver, session, request) =
            explicit_control_fixture(SafeMode::Silent, ExplicitTransactionAction::Begin, None);
        studio
            .set_passphrase("synthetic-protected-boundary", None)
            .unwrap();
        let mut profile = session.profile.clone();
        profile.environment = Environment::Production;
        studio.persistence.save_profile(&profile).unwrap();
        let plan = studio
            .prepare_transaction_control(&session.info.id, &request)
            .unwrap();
        assert!(plan.requires_approval && plan.requires_passphrase);
        assert!(
            studio
                .authorize(&plan.id, Some("wrong-passphrase"), None)
                .is_err()
        );
        assert!(driver.boundary_calls.lock().is_empty());
        let token = studio
            .authorize(&plan.id, Some("synthetic-protected-boundary"), None)
            .unwrap();
        let other = studio
            .prepare_transaction_control(&session.info.id, &request)
            .unwrap();
        assert_eq!(
            studio
                .execute_transaction_control(&other.id, Some(&token))
                .await
                .unwrap_err()
                .code,
            ErrorCode::ApprovalRequired
        );
        assert!(driver.boundary_calls.lock().is_empty());
        let plan = studio
            .prepare_transaction_control(&session.info.id, &request)
            .unwrap();
        let token = studio
            .authorize(&plan.id, Some("synthetic-protected-boundary"), None)
            .unwrap();
        let result = studio
            .execute_transaction_control(&plan.id, Some(&token))
            .await
            .unwrap();
        assert_eq!(
            result.boundary_outcome,
            TransactionBoundaryOutcome::Acknowledged
        );
        assert_eq!(driver.boundary_calls.lock().len(), 1);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn typed_schema_switch_returns_driver_options_without_observer_reads() {
        for outcome in [
            "success",
            "error",
            "closed",
            "wrong-session",
            "no-revision",
            "busy",
        ] {
            let root = tempfile::tempdir().unwrap();
            let studio = Studio::with_secrets(
                root.path(),
                Arc::new(crate::secrets::MemorySecrets::default()),
            )
            .unwrap();
            let driver = Arc::new(RecordingDriver {
                context_enabled: true,
                switch_error: (outcome == "error").then_some(ErrorCode::OutcomeUnknown),
                close_during_schema_switch: outcome == "closed",
                ..Default::default()
            });
            let session = relation_test_session(&studio, driver.clone(), true);
            let mut state = driver.session_state(&session.info.id).await.unwrap();
            state.active_context = Some("Quoted \"Schema".into());
            state.context_fingerprint = Some("verified-after-switch".into());
            state.transaction_observation = Some(TransactionObservation::Observed {
                transaction_id: None,
                abort_on_error: false,
                usability: TransactionUsability::Unknown,
            });
            if outcome == "wrong-session" {
                state.session_id = "another-session".into();
            }
            let expected = SessionContextOptions {
                state,
                available: vec!["Quoted \"Schema".into()],
                revision: (outcome != "no-revision").then(|| "fresh-driver-authority".into()),
            };
            *driver.schema_switch_options.lock() = Some(expected.clone());
            *session.completion.lock() = Some(CompletionCatalog {
                revision: "old".into(),
                identity: driver.relation_catalog().await.unwrap().identity,
                tables: vec![],
                unqualified_tables: vec![],
            });
            let plan = studio
                .prepare_execution(&session.info.id, "SELECT 1")
                .unwrap();
            driver.relation_invalidated.store(false, Ordering::Release);
            driver.state_calls.store(0, Ordering::Relaxed);
            if outcome == "busy" {
                *session.active_run.lock() = Some("active-query".into());
            }
            let request = SwitchSchemaRequest {
                revision: "original-options".into(),
                database: "fixture".into(),
                schema: "Quoted \"Schema".into(),
                context_fingerprint: "original-fingerprint".into(),
            };
            let result = studio
                .switch_session_schema(&session.info.id, &request)
                .await;
            assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0, "{outcome}");
            if outcome == "busy" {
                assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
                assert!(driver.schema_switch_calls.lock().is_empty());
                assert!(session.completion.lock().is_some());
                assert!(studio.plans.lock().contains_key(&plan.id));
                continue;
            }
            assert_eq!(&*driver.schema_switch_calls.lock(), &[request]);
            assert!(driver.switch_saw_invalidation.load(Ordering::Acquire));
            assert!(session.completion.lock().is_none());
            assert!(!studio.plans.lock().contains_key(&plan.id));
            if outcome == "success" {
                let actual = result.unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
                assert!(session.observed_runtime.lock().is_some());
            } else {
                assert_eq!(
                    result.unwrap_err().code,
                    match outcome {
                        "error" => ErrorCode::OutcomeUnknown,
                        "closed" => ErrorCode::ConnectionClosed,
                        _ => ErrorCode::Internal,
                    }
                );
                assert!(session.observed_runtime.lock().is_none());
            }
        }
    }

    #[tokio::test]
    async fn sdk_observation_drift_revokes_plans_and_completion_but_stable_state_preserves_them() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let driver = Arc::new(RecordingDriver {
            context_enabled: true,
            ..Default::default()
        });
        let session = relation_test_session(&studio, driver.clone(), true);
        let mut observed = driver.session_state(&session.info.id).await.unwrap();
        observed.context_fingerprint = Some("A".into());
        observed.transaction_observation = Some(TransactionObservation::Observed {
            transaction_id: None,
            abort_on_error: false,
            usability: TransactionUsability::Unknown,
        });
        *driver.runtime_override.lock() = Some(observed.clone());
        studio.session_contexts(&session.info.id).await.unwrap();
        *session.completion.lock() = Some(CompletionCatalog {
            revision: "old".into(),
            identity: driver.relation_catalog().await.unwrap().identity,
            tables: vec![],
            unqualified_tables: vec![],
        });
        let plan = studio
            .prepare_execution(&session.info.id, "SELECT 1")
            .unwrap();
        driver.relation_invalidated.store(false, Ordering::Release);
        studio.session_contexts(&session.info.id).await.unwrap();
        assert!(session.completion.lock().is_some());
        assert!(studio.plans.lock().contains_key(&plan.id));
        assert!(!driver.relation_invalidated.load(Ordering::Acquire));
        observed.transaction_control_authority = Some(TransactionControlAuthority::Observed {
            revision: new_id(),
            allowed_actions: vec![ExplicitTransactionAction::Begin],
        });
        *driver.runtime_override.lock() = Some(observed.clone());
        studio.session_contexts(&session.info.id).await.unwrap();
        assert!(session.completion.lock().is_some());
        assert!(studio.plans.lock().contains_key(&plan.id));
        assert!(!driver.relation_invalidated.load(Ordering::Acquire));
        observed.context_fingerprint = Some("B".into());
        observed.role = None;
        *driver.runtime_override.lock() = Some(observed);
        studio.session_contexts(&session.info.id).await.unwrap();
        assert!(session.completion.lock().is_none());
        assert!(!studio.plans.lock().contains_key(&plan.id));
        assert!(driver.relation_invalidated.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn a_session_closed_during_context_observation_cannot_publish_options() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let driver = Arc::new(RecordingDriver {
            context_enabled: true,
            close_during_context: true,
            ..Default::default()
        });
        let session = relation_test_session(&studio, driver, true);
        assert_eq!(
            studio
                .session_contexts(&session.info.id)
                .await
                .unwrap_err()
                .code,
            ErrorCode::ConnectionClosed
        );
        assert!(session.observed_runtime.lock().is_none());
    }

    #[tokio::test]
    async fn unknown_mode_is_rejected_before_driver_admission() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let driver = Arc::new(RecordingDriver::default());
        let session = relation_test_session(&studio, driver.clone(), true);
        assert_eq!(
            studio
                .set_transaction_mode(&session.info.id, TransactionMode::Unknown)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(driver.mode_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn context_list_denial_is_not_published_as_empty_success() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let driver = Arc::new(RecordingDriver {
            context_enabled: true,
            contexts_error: Some(ErrorCode::PermissionDenied),
            ..Default::default()
        });
        let session = relation_test_session(&studio, driver, true);
        assert_eq!(
            studio
                .session_contexts(&session.info.id)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
    }

    #[tokio::test]
    async fn failed_switch_revokes_targets_and_plans_before_dispatch() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let driver = Arc::new(RecordingDriver {
            context_enabled: true,
            switch_error: Some(ErrorCode::OutcomeUnknown),
            ..Default::default()
        });
        let session = relation_test_session(&studio, driver.clone(), true);
        *session.completion.lock() = Some(CompletionCatalog {
            revision: "old".into(),
            identity: driver.relation_catalog().await.unwrap().identity,
            tables: vec![],
            unqualified_tables: vec![],
        });
        let plan = studio
            .prepare_execution(&session.info.id, "SELECT 1")
            .unwrap();
        assert_eq!(
            studio
                .switch_session_context(&session.info.id, "next")
                .await
                .unwrap_err()
                .code,
            ErrorCode::OutcomeUnknown
        );
        assert!(driver.switch_saw_invalidation.load(Ordering::Acquire));
        assert!(session.completion.lock().is_none());
        assert!(!studio.plans.lock().contains_key(&plan.id));
    }

    #[tokio::test]
    async fn relation_admission_rejects_unavailable_busy_and_closed_sessions() {
        for state in [
            "unsupported",
            "running",
            "closed",
            "closed-during-read",
            "ready",
        ] {
            let root = tempfile::tempdir().unwrap();
            let studio = Studio::with_secrets(
                root.path(),
                Arc::new(crate::secrets::MemorySecrets::default()),
            )
            .unwrap();
            let driver = Arc::new(RecordingDriver {
                close_during_relation: state == "closed-during-read",
                ..Default::default()
            });
            let session = relation_test_session(&studio, driver.clone(), state != "unsupported");
            if state == "running" {
                *session.active_run.lock() = Some("query".into());
            }
            if state == "closed" {
                session.closed.store(true, Ordering::Release);
            }
            let result = studio.relation_catalog(&session.info.id).await;
            if state == "ready" {
                assert_eq!(result.unwrap().revision, "r");
            } else {
                assert_eq!(
                    result.unwrap_err().code,
                    match state {
                        "unsupported" => ErrorCode::Unsupported,
                        "running" => ErrorCode::Conflict,
                        _ => ErrorCode::ConnectionClosed,
                    }
                );
            }
            assert_eq!(
                driver.relation_calls.load(Ordering::Relaxed),
                usize::from(matches!(state, "ready" | "closed-during-read"))
            );
        }
    }

    #[tokio::test]
    async fn relation_definition_admission_forwards_only_retained_requests_on_available_sessions() {
        for state in [
            "unsupported",
            "running",
            "closed",
            "closed-during-read",
            "ready",
        ] {
            let root = tempfile::tempdir().unwrap();
            let studio = Studio::with_secrets(
                root.path(),
                Arc::new(crate::secrets::MemorySecrets::default()),
            )
            .unwrap();
            let driver = Arc::new(RecordingDriver {
                close_during_relation: state == "closed-during-read",
                ..Default::default()
            });
            let session = relation_test_session(&studio, driver.clone(), state != "unsupported");
            if state == "running" {
                *session.active_run.lock() = Some("query".into());
            }
            if state == "closed" {
                session.closed.store(true, Ordering::Release);
            }
            let request = DefineRelationRequest {
                revision: "catalog-revision".into(),
                target_id: "opaque-target".into(),
            };
            let result = studio.define_relation(&session.info.id, &request).await;
            if state == "ready" {
                let result = result.unwrap();
                assert_eq!(result.revision, request.revision);
                assert_eq!(result.target_id, request.target_id);
                assert_eq!(result.ddl, "create or replace table item (id number);");
                assert!(studio.session(&session.info.id).is_ok());
                assert!(!session.closed.load(Ordering::Acquire));
            } else {
                assert_eq!(
                    result.unwrap_err().code,
                    match state {
                        "unsupported" => ErrorCode::Unsupported,
                        "running" => ErrorCode::Conflict,
                        _ => ErrorCode::ConnectionClosed,
                    }
                );
            }
            let calls = driver.definition_calls.lock();
            assert_eq!(
                calls.len(),
                usize::from(matches!(state, "ready" | "closed-during-read"))
            );
            if let Some(call) = calls.first() {
                assert_eq!(call.revision, request.revision);
                assert_eq!(call.target_id, request.target_id);
            }
            assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
        }
    }

    #[tokio::test]
    async fn relation_constraints_admission_forwards_only_retained_requests_on_available_sessions()
    {
        for state in [
            "unsupported",
            "running",
            "lane-held",
            "metadata-held",
            "closed",
            "closed-during-read",
            "ready",
        ] {
            let root = tempfile::tempdir().unwrap();
            let studio = Studio::with_secrets(
                root.path(),
                Arc::new(crate::secrets::MemorySecrets::default()),
            )
            .unwrap();
            let driver = Arc::new(RecordingDriver {
                close_during_relation: state == "closed-during-read",
                ..Default::default()
            });
            let session = relation_test_session(&studio, driver.clone(), state != "unsupported");
            if state == "running" {
                *session.active_run.lock() = Some("query".into());
            }
            if state == "closed" {
                session.closed.store(true, Ordering::Release);
            }
            let lane = if state == "lane-held" {
                Some(session.lane.lock().await)
            } else {
                None
            };
            if state == "metadata-held" {
                session.metadata_gate.lock().hold();
            }
            let request = InspectRelationConstraintsRequest {
                revision: "catalog-revision".into(),
                target_id: "opaque-target".into(),
            };
            let result = studio
                .inspect_relation_constraints(&session.info.id, &request)
                .await;
            drop(lane);
            if state == "ready" {
                let result = result.unwrap();
                assert_eq!(result.revision, request.revision);
                assert_eq!(result.target_id, request.target_id);
                assert!(result.constraints.is_empty());
                assert!(studio.session(&session.info.id).is_ok());
                assert!(!session.closed.load(Ordering::Acquire));
            } else {
                assert_eq!(
                    result.unwrap_err().code,
                    match state {
                        "unsupported" => ErrorCode::Unsupported,
                        "running" | "lane-held" | "metadata-held" => ErrorCode::Conflict,
                        _ => ErrorCode::ConnectionClosed,
                    }
                );
            }
            {
                let calls = driver.constraints_calls.lock();
                assert_eq!(
                    calls.len(),
                    usize::from(matches!(state, "ready" | "closed-during-read"))
                );
                if let Some(call) = calls.first() {
                    assert_eq!(call.revision, request.revision);
                    assert_eq!(call.target_id, request.target_id);
                }
            }
            assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
            assert!(driver.definition_calls.lock().is_empty());
            assert_eq!(
                studio
                    .inspect_relation_constraints("missing", &request)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::ConnectionClosed
            );
        }
    }

    #[tokio::test]
    async fn relation_constraint_mappings_admission_forwards_only_retained_requests_on_available_sessions()
     {
        for state in [
            "unsupported",
            "running",
            "lane-held",
            "metadata-held",
            "closed",
            "closed-during-read",
            "ready",
        ] {
            let root = tempfile::tempdir().unwrap();
            let studio = Studio::with_secrets(
                root.path(),
                Arc::new(crate::secrets::MemorySecrets::default()),
            )
            .unwrap();
            let driver = Arc::new(RecordingDriver {
                close_during_relation: state == "closed-during-read",
                ..Default::default()
            });
            let session = relation_test_session(&studio, driver.clone(), state != "unsupported");
            if state == "running" {
                *session.active_run.lock() = Some("query".into());
            }
            if state == "closed" {
                session.closed.store(true, Ordering::Release);
            }
            let lane = if state == "lane-held" {
                Some(session.lane.lock().await)
            } else {
                None
            };
            if state == "metadata-held" {
                session.metadata_gate.lock().hold();
            }
            let request = InspectRelationConstraintMappingsRequest {
                revision: "catalog-revision".into(),
                target_id: "opaque-target".into(),
            };
            let result = studio
                .inspect_relation_constraint_mappings(&session.info.id, &request)
                .await;
            drop(lane);
            if state == "ready" {
                let result = result.unwrap();
                assert_eq!(result.revision, request.revision);
                assert_eq!(result.target_id, request.target_id);
                assert!(result.mappings.is_empty());
                assert!(studio.session(&session.info.id).is_ok());
                assert!(!session.closed.load(Ordering::Acquire));
            } else {
                assert_eq!(
                    result.unwrap_err().code,
                    match state {
                        "unsupported" => ErrorCode::Unsupported,
                        "running" | "lane-held" | "metadata-held" => ErrorCode::Conflict,
                        _ => ErrorCode::ConnectionClosed,
                    }
                );
            }
            {
                let calls = driver.mapping_calls.lock();
                assert_eq!(
                    calls.len(),
                    usize::from(matches!(state, "ready" | "closed-during-read"))
                );
                if let Some(call) = calls.first() {
                    assert_eq!(call.revision, request.revision);
                    assert_eq!(call.target_id, request.target_id);
                }
            }
            assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
            assert!(driver.definition_calls.lock().is_empty());
            assert_eq!(
                studio
                    .inspect_relation_constraint_mappings("missing", &request)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::ConnectionClosed
            );
        }
    }

    #[tokio::test]
    async fn relation_check_constraints_admission_forwards_only_retained_requests_on_available_sessions()
     {
        for state in [
            "unsupported",
            "running",
            "lane-held",
            "metadata-held",
            "closed",
            "closed-during-read",
            "ready",
        ] {
            let root = tempfile::tempdir().unwrap();
            let studio = Studio::with_secrets(
                root.path(),
                Arc::new(crate::secrets::MemorySecrets::default()),
            )
            .unwrap();
            let driver = Arc::new(RecordingDriver {
                close_during_relation: state == "closed-during-read",
                ..Default::default()
            });
            let session = relation_test_session(&studio, driver.clone(), state != "unsupported");
            if state == "running" {
                *session.active_run.lock() = Some("query".into());
            }
            if state == "closed" {
                session.closed.store(true, Ordering::Release);
            }
            let lane = if state == "lane-held" {
                Some(session.lane.lock().await)
            } else {
                None
            };
            if state == "metadata-held" {
                session.metadata_gate.lock().hold();
            }
            let request = InspectRelationCheckConstraintsRequest {
                revision: "catalog-revision".into(),
                target_id: "opaque-target".into(),
            };
            let result = studio
                .inspect_relation_check_constraints(&session.info.id, &request)
                .await;
            drop(lane);
            if state == "ready" {
                let result = result.unwrap();
                assert_eq!(result.revision, request.revision);
                assert_eq!(result.target_id, request.target_id);
                assert!(result.constraints.is_empty());
                assert!(studio.session(&session.info.id).is_ok());
                assert!(!session.closed.load(Ordering::Acquire));
            } else {
                assert_eq!(
                    result.unwrap_err().code,
                    match state {
                        "unsupported" => ErrorCode::Unsupported,
                        "running" | "lane-held" | "metadata-held" => ErrorCode::Conflict,
                        _ => ErrorCode::ConnectionClosed,
                    }
                );
            }
            {
                let calls = driver.check_constraints_calls.lock();
                assert_eq!(
                    calls.len(),
                    usize::from(matches!(state, "ready" | "closed-during-read"))
                );
                if let Some(call) = calls.first() {
                    assert_eq!(call.revision, request.revision);
                    assert_eq!(call.target_id, request.target_id);
                }
            }
            assert_eq!(driver.relation_calls.load(Ordering::Relaxed), 0);
            assert!(driver.definition_calls.lock().is_empty());
            assert_eq!(
                studio
                    .inspect_relation_check_constraints("missing", &request)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::ConnectionClosed
            );
        }
    }

    #[tokio::test]
    async fn transaction_controls_preserve_acknowledgement_and_retire_only_terminal_failures() {
        for engine in [EngineKind::Postgres, EngineKind::Mysql] {
            for rollback in [false, true] {
                for (control_error, preclosed) in [
                    (None, false),
                    (Some(ErrorCode::Database), false),
                    (Some(ErrorCode::Conflict), false),
                    (Some(ErrorCode::ConnectionClosed), false),
                    (Some(ErrorCode::OutcomeUnknown), false),
                    (Some(ErrorCode::ConnectionClosed), true),
                ] {
                    let root = tempfile::tempdir().unwrap();
                    let studio = Studio::with_secrets(
                        root.path(),
                        Arc::new(crate::secrets::MemorySecrets::default()),
                    )
                    .unwrap();
                    let saved = studio
                        .save_profile(profile(engine, SafeMode::Silent), None)
                        .unwrap();
                    let driver = Arc::new(RecordingDriver {
                        transaction_error: control_error,
                        ..Default::default()
                    });
                    let id = new_id();
                    studio.sessions.lock().insert(
                        id.clone(),
                        Arc::new(Session {
                            rollback_memory: Mutex::new(None),
                            #[cfg(unix)]
                            snowflake_external: None,
                            info: SessionInfo {
                                console_mode: false,
                                id: id.clone(),
                                connection_id: saved.id.clone(),
                                engine,
                                server_version: "fixture".into(),
                                latency_ms: 0,
                                encrypted: false,
                                capabilities: EngineCapabilities {
                                    sql: true,
                                    transactions: true,
                                    explicit_transaction_controls: false,
                                    cancellation: true,
                                    table_browsing: false,
                                    relation_browsing: false,
                                    table_editing: false,
                                    table_filtering: false,
                                    processes: false,
                                    object_explorer: false,
                                    schema_diagram: false,
                                    schema_diff: false,
                                    ddl_editing: false,
                                    snowflake_reviewed_ddl: false,
                                    snowflake_grid_editing: false,
                                    lock_inspection: false,
                                    explain_analyze: false,
                                    oauth_browser: false,
                                    actual_plan: None,
                                },
                            },
                            profile: saved,
                            driver: driver.clone(),
                            lane: tokio::sync::Mutex::new(()),
                            cancellation_gate: tokio::sync::Mutex::new(()),
                            active_run: Mutex::new(None),
                            closed: AtomicBool::new(false),
                            tunnel: None,
                            completion: Mutex::new(None),
                            metadata_gate: Mutex::new(MetadataGate::default()),
                            observed_runtime: Mutex::new(None),
                        }),
                    );
                    let pending = studio.prepare_execution(&id, "SELECT 1").unwrap();
                    if preclosed {
                        driver.close();
                    }
                    let error = if rollback {
                        studio.rollback_transaction(&id).await
                    } else {
                        studio.commit_transaction(&id).await
                    }
                    .unwrap_err();
                    let ordinary_error = matches!(
                        control_error,
                        Some(ErrorCode::Database | ErrorCode::Conflict)
                    );
                    if let Some(code) = control_error {
                        assert_eq!(error.code, code);
                    } else {
                        assert_eq!(error.code, ErrorCode::ConnectionClosed);
                        let action = if rollback { "rolled back" } else { "committed" };
                        assert_eq!(
                            error.message,
                            format!(
                                "Transaction {action}, but session context could not be refreshed. Reconnect before continuing."
                            )
                        );
                    }
                    assert_eq!(studio.sessions.lock().contains_key(&id), ordinary_error);
                    assert_eq!(
                        studio.plans.lock().contains_key(&pending.id),
                        control_error == Some(ErrorCode::Conflict)
                    );
                    assert_eq!(driver.is_closed(), !ordinary_error);
                }
            }
        }
    }

    #[tokio::test]
    async fn parameterized_plans_hash_bound_values_and_never_persist_parameter_secrets() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let mut prof = profile(EngineKind::Postgres, SafeMode::Confirm);
        prof.store_query_text = true;
        let saved = studio.save_profile(prof, None).unwrap();

        let recorder = Arc::new(RecordingDriver {
            calls: Mutex::new(Vec::new()),
            ..Default::default()
        });
        let session_id = new_id();
        studio.sessions.lock().insert(
            session_id.clone(),
            Arc::new(Session {
                rollback_memory: Mutex::new(None),
                #[cfg(unix)]
                snowflake_external: None,
                info: SessionInfo {
                    console_mode: false,
                    id: session_id.clone(),
                    connection_id: saved.id.clone(),
                    engine: EngineKind::Postgres,
                    server_version: "17.0".into(),
                    latency_ms: 1,
                    encrypted: false,
                    capabilities: EngineCapabilities {
                        sql: true,
                        actual_plan: None,
                        transactions: true,
                        explicit_transaction_controls: false,
                        cancellation: true,
                        table_browsing: true,
                        relation_browsing: false,
                        table_editing: true,
                        table_filtering: true,
                        processes: true,
                        object_explorer: true,
                        schema_diagram: false,
                        schema_diff: false,
                        ddl_editing: false,
                        snowflake_reviewed_ddl: false,
                        snowflake_grid_editing: false,
                        lock_inspection: false,
                        explain_analyze: false,
                        oauth_browser: false,
                    },
                },
                profile: saved,
                driver: recorder.clone(),
                lane: tokio::sync::Mutex::new(()),
                cancellation_gate: tokio::sync::Mutex::new(()),
                active_run: Mutex::new(None),
                closed: AtomicBool::new(false),
                tunnel: None,
                completion: Mutex::new(None),
                metadata_gate: Mutex::new(MetadataGate::default()),
                observed_runtime: Mutex::new(None),
            }),
        );

        let template_sql = "UPDATE orders SET note = :secret WHERE id = $1";
        let digest_for = |secret: Option<&str>, secret_kind: QueryParamKind, id_val: &str| {
            let plan = studio
                .prepare_execution_with_params(
                    &session_id,
                    template_sql,
                    Some(vec![
                        QueryParamInput {
                            key: "secret".into(),
                            kind: secret_kind,
                            value: secret.map(str::to_owned),
                        },
                        QueryParamInput {
                            key: "$1".into(),
                            kind: QueryParamKind::Integer,
                            value: Some(id_val.into()),
                        },
                    ]),
                )
                .unwrap();
            let digest = studio.plans.lock().get(&plan.id).unwrap().digest.clone();
            (plan, digest)
        };

        let (_, d_alpha) = digest_for(Some("100"), QueryParamKind::Text, "7");
        let (_, d_beta) = digest_for(Some("200"), QueryParamKind::Text, "7");
        let (_, d_int) = digest_for(Some("100"), QueryParamKind::Integer, "7");
        let (_, d_null) = digest_for(None, QueryParamKind::Text, "7");
        assert_ne!(d_alpha, d_beta);
        assert_ne!(d_alpha, d_int);
        assert_ne!(d_alpha, d_null);

        let secret_marker = "top-secret-param-value-999";
        let (plan, _) = digest_for(Some(secret_marker), QueryParamKind::Text, "42");
        assert!(plan.requires_approval);
        assert_eq!(plan.bound_parameters.len(), 2);
        assert_eq!(
            plan.bound_parameters[0].value.as_deref(),
            Some(secret_marker)
        );

        let approval = studio.authorize(&plan.id, None, None).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        studio
            .execute(
                &plan.id,
                Some(&approval),
                Arc::new(move |e| tx.send(e).is_ok()),
            )
            .unwrap();
        while let Some(ev) = rx.recv().await {
            if let QueryEvent::Finished { status, .. } = ev {
                assert_eq!(status, RunStatus::Completed);
                break;
            }
        }

        let calls = recorder.calls.lock();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, template_sql);
        assert_eq!(
            calls[0].1.rewritten_sql,
            "UPDATE orders SET note = $1 WHERE id = $2"
        );
        assert_eq!(calls[0].1.binds[0].value.as_deref(), Some(secret_marker));
        assert_eq!(calls[0].1.binds[1].value.as_deref(), Some("42"));
        drop(calls);

        let history = studio.history("").unwrap();
        let audit = studio.audit_log().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].sql.as_deref(), Some(template_sql));
        assert!(
            !serde_json::to_string(&(studio.bootstrap().unwrap(), &history, &audit))
                .unwrap()
                .contains(secret_marker),
            "bootstrap/history/audit must never persist bound parameter values"
        );
        let mut checked_files = 0usize;
        for entry in std::fs::read_dir(root.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                checked_files += 1;
                let db_bytes = std::fs::read(&path).unwrap();
                assert!(
                    !String::from_utf8_lossy(&db_bytes).contains(secret_marker),
                    "SQLite file {} must never contain bound parameter values",
                    path.display()
                );
            }
        }
        assert!(checked_files >= 1);
    }

    #[tokio::test]
    async fn prepare_migration_ddl_rejects_disconnected_session() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let snap = crate::schema_diff::SchemaSnapshot {
            engine: EngineKind::Postgres,
            server_version: "PostgreSQL 17".into(),
            database: "joinery".into(),
            schema: "public".into(),
            role: "joinery".into(),
            tables: vec![],
            routines: vec![],
            sequences: vec![],
            unsupported_objects: vec![],
            truncated: false,
            restricted: false,
        };
        let err = studio
            .prepare_migration_ddl(
                "missing-session",
                snap,
                vec![crate::ddl::DdlStatement {
                    sql: "DROP TABLE \"public\".\"t\";".into(),
                    description: "Drop table t".into(),
                    object_kind: DatabaseObjectKind::Table,
                    object_name: "t".into(),
                    destructive: true,
                }],
            )
            .await
            .expect_err("disconnected session should fail");
        assert_eq!(err.code, ErrorCode::ConnectionClosed);
    }

    #[tokio::test]
    async fn granular_engine_capabilities_gate_unsupported_operations() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let saved = studio
            .save_profile(profile(EngineKind::Postgres, SafeMode::Silent), None)
            .unwrap();
        let recorder = Arc::new(RecordingDriver {
            calls: Mutex::new(Vec::new()),
            ..Default::default()
        });
        let session_id = new_id();
        studio.sessions.lock().insert(
            session_id.clone(),
            Arc::new(Session {
                rollback_memory: Mutex::new(None),
                #[cfg(unix)]
                snowflake_external: None,
                info: SessionInfo {
                    console_mode: false,
                    id: session_id.clone(),
                    connection_id: saved.id.clone(),
                    engine: EngineKind::Postgres,
                    server_version: "17.0".into(),
                    latency_ms: 1,
                    encrypted: false,
                    capabilities: EngineCapabilities {
                        sql: true,
                        actual_plan: None,
                        transactions: true,
                        explicit_transaction_controls: false,
                        cancellation: true,
                        table_browsing: false,
                        relation_browsing: false,
                        table_editing: false,
                        table_filtering: false,
                        processes: false,
                        object_explorer: false,
                        schema_diagram: false,
                        schema_diff: false,
                        ddl_editing: false,
                        snowflake_reviewed_ddl: false,
                        snowflake_grid_editing: false,
                        lock_inspection: false,
                        explain_analyze: false,
                        oauth_browser: false,
                    },
                },
                profile: saved,
                driver: recorder,
                lane: tokio::sync::Mutex::new(()),
                cancellation_gate: tokio::sync::Mutex::new(()),
                active_run: Mutex::new(None),
                closed: AtomicBool::new(false),
                tunnel: None,
                completion: Mutex::new(None),
                metadata_gate: Mutex::new(MetadataGate::default()),
                observed_runtime: Mutex::new(None),
            }),
        );

        let diag_err = studio
            .schema_diagram(&session_id, "public")
            .await
            .expect_err(
                "schema_diagram should be rejected when schema_diagram capability is false",
            );
        assert_eq!(diag_err.code, ErrorCode::Unsupported);

        let snap_err = studio
            .schema_snapshot(&session_id, "public")
            .await
            .expect_err("schema_snapshot should be rejected when schema_diff capability is false");
        assert_eq!(snap_err.code, ErrorCode::Unsupported);

        let ddl_err = studio
            .prepare_table_object_ddl(
                &session_id,
                TableObjectAction::DropObject {
                    object: DatabaseObjectRef {
                        schema: "public".into(),
                        name: "items".into(),
                        kind: DatabaseObjectKind::Table,
                        identity: None,
                    },
                    cascade: false,
                },
            )
            .expect_err(
                "prepare_table_object_ddl should be rejected when ddl_editing capability is false",
            );
        assert_eq!(ddl_err.code, ErrorCode::Unsupported);

        let lock_err = studio
            .lock_waits(&session_id)
            .await
            .expect_err("lock_waits should be rejected when lock_inspection capability is false");
        assert_eq!(lock_err.code, ErrorCode::Unsupported);

        let analyze_err = studio
            .prepare_explain(&session_id, "SELECT 1", true)
            .expect_err("prepare_explain(analyze=true) should be rejected when explain_analyze capability is false");
        assert_eq!(analyze_err.code, ErrorCode::Unsupported);
    }

    #[test]
    fn virtual_foreign_keys_merge_into_schemas_inbound_and_diagrams_without_duplicates() {
        let conn_id = new_id();
        let orders = TableRef {
            schema: "public".into(),
            name: "orders".into(),
        };
        let customers = TableRef {
            schema: "public".into(),
            name: "customers".into(),
        };
        let audit_events = TableRef {
            schema: "audit".into(),
            name: "events".into(),
        };
        let vfks = vec![
            VirtualForeignKey {
                id: new_id(),
                connection_id: conn_id.clone(),
                name: "vfk_orders_customer".into(),
                source: orders.clone(),
                columns: vec!["customer_id".into()],
                target: customers.clone(),
                target_columns: vec!["id".into()],
                updated_at_ms: 1,
            },
            // Nonexistent columns should be ignored on schema/diagram merge.
            VirtualForeignKey {
                id: new_id(),
                connection_id: conn_id.clone(),
                name: "vfk_missing_col".into(),
                source: orders.clone(),
                columns: vec!["ghost_id".into()],
                target: customers.clone(),
                target_columns: vec!["ghost_target_id".into()],
                updated_at_ms: 2,
            },
            // Cross-schema target in diagram should be included.
            VirtualForeignKey {
                id: new_id(),
                connection_id: conn_id.clone(),
                name: "vfk_orders_audit".into(),
                source: orders.clone(),
                columns: vec!["id".into()],
                target: audit_events.clone(),
                target_columns: vec!["entity_id".into()],
                updated_at_ms: 3,
            },
        ];

        let mut orders_schema = TableSchema {
            table: orders.clone(),
            columns: vec![
                ColumnInfo {
                    id: "id".into(),
                    name: "id".into(),
                    database_type: "bigint".into(),
                    kind: ColumnKind::Integer,
                    nullable: false,
                    primary_key: true,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "customer_id".into(),
                    name: "customer_id".into(),
                    database_type: "bigint".into(),
                    kind: ColumnKind::Integer,
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
        merge_virtual_fks_into_table_schema(&mut orders_schema, &vfks, &conn_id, true);
        assert_eq!(orders_schema.foreign_keys.len(), 2);
        assert!(
            orders_schema
                .foreign_keys
                .iter()
                .all(|fk| fk.virtual_relationship)
        );
        assert!(
            orders_schema
                .columns
                .iter()
                .find(|c| c.name == "customer_id")
                .unwrap()
                .foreign_key
        );
        // Re-merging is idempotent (deduplicates).
        merge_virtual_fks_into_table_schema(&mut orders_schema, &vfks, &conn_id, true);
        assert_eq!(orders_schema.foreign_keys.len(), 2);

        let mut customers_schema = TableSchema {
            table: customers.clone(),
            columns: vec![ColumnInfo {
                id: "id".into(),
                name: "id".into(),
                database_type: "bigint".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            }],
            primary_key: vec!["id".into()],
            foreign_keys: vec![],
            inbound_foreign_keys: vec![],
            editable: true,
        };
        merge_virtual_fks_into_table_schema(&mut customers_schema, &vfks, &conn_id, true);
        assert_eq!(customers_schema.inbound_foreign_keys.len(), 1);
        assert!(customers_schema.inbound_foreign_keys[0].virtual_relationship);
        assert_eq!(customers_schema.inbound_foreign_keys[0].source, orders);

        let mut inbound = Vec::new();
        merge_virtual_fks_into_inbound(&mut inbound, &vfks, &conn_id, &customers);
        assert_eq!(inbound.len(), 2);
        assert!(inbound.iter().all(|k| k.virtual_relationship));

        let mut diagram = SchemaDiagram {
            schema: "public".into(),
            tables: vec![
                DiagramTable {
                    table: orders.clone(),
                    kind: "r".into(),
                    columns: vec![
                        DiagramColumn {
                            name: "id".into(),
                            database_type: "bigint".into(),
                            nullable: false,
                            primary_key: true,
                        },
                        DiagramColumn {
                            name: "customer_id".into(),
                            database_type: "bigint".into(),
                            nullable: false,
                            primary_key: false,
                        },
                    ],
                },
                DiagramTable {
                    table: customers.clone(),
                    kind: "r".into(),
                    columns: vec![DiagramColumn {
                        name: "id".into(),
                        database_type: "bigint".into(),
                        nullable: false,
                        primary_key: true,
                    }],
                },
            ],
            foreign_keys: vec![],
            truncated: false,
            restricted: false,
        };
        merge_virtual_fks_into_diagram(&mut diagram, &vfks, &conn_id);
        assert_eq!(diagram.foreign_keys.len(), 2);
        assert!(
            diagram
                .foreign_keys
                .iter()
                .all(|fk| fk.virtual_relationship)
        );
    }
}
