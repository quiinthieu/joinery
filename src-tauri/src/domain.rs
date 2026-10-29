use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum EngineKind {
    Postgres,
    Mysql,
    Redis,
    Snowflake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Environment {
    Production,
    Staging,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SafeMode {
    Silent,
    Confirm,
    Guard,
    PasswordGated,
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TlsMode {
    VerifyFull,
    Disable,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionGroup {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub environment: Environment,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: SshAuthentication,
    pub private_key_path: Option<String>,
    #[serde(default)]
    pub agent_socket_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SshAuthentication {
    Password,
    PrivateKey,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SshHostKey {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
    pub challenge_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub group_id: String,
    pub engine: EngineKind,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub environment: Environment,
    pub safe_mode: SafeMode,
    pub tls_mode: TlsMode,
    pub ca_cert_path: Option<String>,
    pub ssh: Option<SshConfig>,
    pub remember_password: bool,
    pub store_query_text: bool,
    #[serde(default)]
    pub store_drafts: bool,
    #[serde(default)]
    pub snowflake: Option<SnowflakeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnowflakeConfig {
    pub role: String,
    pub warehouse: String,
    pub schema: String,
}

// Secret inputs deliberately do not implement Debug and are never persisted with profiles.
#[derive(Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSecrets {
    pub password: Option<String>,
    pub ssh_password: Option<String>,
    pub key_passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub connection_id: String,
    pub engine: EngineKind,
    pub server_version: String,
    pub latency_ms: u32,
    pub encrypted: bool,
    pub capabilities: EngineCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EngineCapabilities {
    pub sql: bool,
    pub transactions: bool,
    pub cancellation: bool,
    pub table_browsing: bool,
    pub table_editing: bool,
    pub processes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StatementKind {
    Read,
    Write,
    Ddl,
    Transaction,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StatementPlan {
    pub sql: String,
    pub kind: StatementKind,
    pub missing_where: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreparedPlan {
    pub id: String,
    pub session_id: String,
    pub statements: Vec<StatementPlan>,
    pub requires_approval: bool,
    pub requires_passphrase: bool,
    pub confirmation_text: Option<String>,
    pub warnings: Vec<String>,
    #[ts(type = "number")]
    pub expires_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ColumnKind {
    Text,
    Integer,
    Decimal,
    Boolean,
    Temporal,
    Json,
    Binary,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub id: String,
    pub name: String,
    pub database_type: String,
    pub kind: ColumnKind,
    pub nullable: bool,
    pub primary_key: bool,
    pub foreign_key: bool,
    pub editable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TableRef {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub table: TableRef,
    pub kind: String,
    pub estimated_rows: Option<f64>,
    pub total_bytes: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKey {
    pub column: String,
    pub target: TableRef,
    pub target_column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TableSchema {
    pub table: TableRef,
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Vec<String>,
    pub foreign_keys: Vec<ForeignKey>,
    pub editable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RetentionStatus {
    Streaming,
    Complete,
    Truncated,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResultInfo {
    pub id: String,
    pub run_id: String,
    pub statement_index: usize,
    pub columns: Vec<ColumnInfo>,
    pub retained_rows: usize,
    pub status: RetentionStatus,
    pub table: Option<TableRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TablePage {
    pub result: ResultInfo,
    pub next_key: Option<Vec<Option<String>>>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Completed,
    Failed,
    Cancelled,
    OutcomeUnknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum QueryEvent {
    Started {
        run_id: String,
    },
    StatementStarted {
        run_id: String,
        index: usize,
    },
    Result {
        result: ResultInfo,
    },
    Progress {
        result_id: String,
        retained_rows: usize,
    },
    StatementCompleted {
        run_id: String,
        index: usize,
        affected_rows: String,
        elapsed_ms: u32,
    },
    Notice {
        message: String,
    },
    SessionClosed {
        session_id: String,
        reason: String,
    },
    Error {
        error: AppError,
        statement_index: usize,
    },
    Finished {
        run_id: String,
        status: RunStatus,
        elapsed_ms: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CellChange {
    pub row: usize,
    pub column: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResultChanges {
    pub result_id: String,
    pub changes: Vec<CellChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChangePreview {
    pub plan: PreparedPlan,
    pub row_count: usize,
    pub sql: Vec<String>,
    pub bindings: Vec<Vec<Option<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub id: String,
    pub identity: String,
    pub user: String,
    pub database: String,
    pub state: String,
    pub duration_seconds: f64,
    pub query: Option<String>,
    pub own_session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub connection_id: String,
    pub connection_name: String,
    #[ts(type = "number")]
    pub started_at: u64,
    pub sql: Option<String>,
    pub statement_count: usize,
    pub status: RunStatus,
    pub elapsed_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: String,
    #[ts(type = "number")]
    pub occurred_at: u64,
    pub connection_id: String,
    pub action: String,
    pub digest: String,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MetadataBackup {
    pub id: String,
    #[ts(type = "number")]
    pub created_at: u64,
    #[ts(type = "number")]
    pub size_bytes: u64,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub profiles: Vec<ConnectionProfile>,
    pub groups: Vec<ConnectionGroup>,
    pub passphrase_configured: bool,
    pub supported_engines: Vec<EngineKind>,
    pub result_budget_bytes: usize,
    pub row_limit: usize,
    pub workspace: Option<SavedWorkspace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryDraft {
    pub id: String,
    pub connection_id: String,
    pub title: String,
    pub sql: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedWorkspace {
    pub theme: String,
    pub density: String,
    pub sidebar_visible: bool,
    pub inspector_visible: bool,
    pub active_draft_id: Option<String>,
    pub drafts: Vec<QueryDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUsage {
    pub retained_bytes: usize,
    pub result_count: usize,
    pub session_count: usize,
    pub active_runs: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    InvalidInput,
    ConnectionFailed,
    ConnectionClosed,
    Database,
    PermissionDenied,
    Unsupported,
    PolicyBlocked,
    ApprovalRequired,
    ApprovalExpired,
    PassphraseRequired,
    InvalidPassphrase,
    Conflict,
    ResultLimitReached,
    NotFound,
    Cancelled,
    OutcomeUnknown,
    CredentialStore,
    Internal,
    SshHostKeyUnknown,
    SshHostKeyChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

pub type AppResult<T> = Result<T, AppError>;
pub type TextRow = Vec<Option<String>>;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RedisKeyInfo {
    pub key: String,
    pub argument: String,
    pub kind: String,
    pub ttl_ms: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RedisScanPage {
    pub keys: Vec<RedisKeyInfo>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RedisValuePreview {
    pub key: RedisKeyInfo,
    pub entries: Vec<RedisValueEntry>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RedisValueEntry {
    pub field: Option<String>,
    pub value: Option<String>,
    pub field_binary: bool,
    pub value_binary: bool,
}

/// Effective identity observed on the editor connection, never saved settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CompletionIdentity {
    pub database: String,
    pub role: String,
    pub search_path: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CompletionCatalog {
    pub revision: String,
    pub identity: CompletionIdentity,
    pub tables: Vec<TableInfo>,
    /// Selectable relations that PostgreSQL actually resolves without qualification.
    pub unqualified_tables: Vec<TableRef>,
}
