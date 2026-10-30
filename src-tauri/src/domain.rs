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
    /// Server-side typed sorting and filtering while browsing tables.
    pub table_filtering: bool,
    pub processes: bool,
}

/// One table-browser ordering key; primary-key tie-breakers are implicit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SortKey {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FilterOp {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    Contains,
    StartsWith,
    IsNull,
    IsNotNull,
}

/// A typed predicate; `value` is always bound, never interpolated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ColumnFilter {
    pub column: String,
    pub op: FilterOp,
    pub value: Option<String>,
}

/// Server-side table-browser view. Filters are combined with AND.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BrowseQuery {
    #[serde(default)]
    pub sort: Vec<SortKey>,
    #[serde(default)]
    pub filters: Vec<ColumnFilter>,
    /// Optional SQL predicate fragment; classified through `safety::prepare`.
    #[serde(default)]
    pub where_clause: Option<String>,
    /// Optional SQL ORDER BY list; disables keyset continuation paging.
    #[serde(default)]
    pub order_clause: Option<String>,
}

impl BrowseQuery {
    pub fn normalized(&self) -> Self {
        Self {
            sort: self.sort.clone(),
            filters: self.filters.clone(),
            where_clause: self
                .where_clause
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            order_clause: self
                .order_clause
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        }
    }

    pub fn has_raw_clauses(&self) -> bool {
        self.where_clause
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            || self
                .order_clause
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.sort.is_empty() && self.filters.is_empty() && !self.has_raw_clauses()
    }
}

/// One-use authorization token for a non-`Read` raw table browse clause.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BrowseApproval {
    pub plan_id: String,
    pub approval: String,
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

/// One foreign-key constraint. `columns[i]` references `target_columns[i]`,
/// in constraint ordinal order, so composite keys keep their pairing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub target: TableRef,
    pub target_columns: Vec<String>,
}

impl ForeignKey {
    /// Folds per-column catalog rows `(constraint, column, target, target column)`
    /// into constraints. Rows must be ordered by constraint and ordinal position.
    pub fn group(rows: impl IntoIterator<Item = (String, String, TableRef, String)>) -> Vec<Self> {
        let mut keys: Vec<Self> = Vec::new();
        for (name, column, target, target_column) in rows {
            match keys.last_mut() {
                Some(key) if key.name == name && key.target == target => {
                    key.columns.push(column);
                    key.target_columns.push(target_column);
                }
                _ => keys.push(Self {
                    name,
                    columns: vec![column],
                    target,
                    target_columns: vec![target_column],
                }),
            }
        }
        keys
    }
}

/// One inbound foreign-key constraint referencing a target table.
/// `columns[i]` on `source` references `target_columns[i]` on the current table,
/// in constraint ordinal order, so composite keys keep their pairing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct InboundForeignKey {
    pub name: String,
    pub source: TableRef,
    pub columns: Vec<String>,
    pub target_columns: Vec<String>,
}

impl InboundForeignKey {
    /// Folds per-column catalog rows `(constraint, source, source column, target column)`
    /// into constraints. Rows must be ordered by source table, constraint, and ordinal position.
    pub fn group(rows: impl IntoIterator<Item = (String, TableRef, String, String)>) -> Vec<Self> {
        let mut keys: Vec<Self> = Vec::new();
        for (name, source, column, target_column) in rows {
            match keys.last_mut() {
                Some(key) if key.name == name && key.source == source => {
                    key.columns.push(column);
                    key.target_columns.push(target_column);
                }
                _ => keys.push(Self {
                    name,
                    source,
                    columns: vec![column],
                    target_columns: vec![target_column],
                }),
            }
        }
        keys
    }
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
    pub case_sensitive_table_names: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CompletionCatalog {
    pub revision: String,
    pub identity: CompletionIdentity,
    pub tables: Vec<TableInfo>,
    /// Relations that resolve in the current engine's unqualified namespace.
    pub unqualified_tables: Vec<TableRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        name: &str,
        column: &str,
        target: &str,
        target_column: &str,
    ) -> (String, String, TableRef, String) {
        (
            name.into(),
            column.into(),
            TableRef {
                schema: "public".into(),
                name: target.into(),
            },
            target_column.into(),
        )
    }

    #[test]
    fn foreign_keys_group_by_constraint_in_ordinal_order() {
        let keys = ForeignKey::group([
            row("line_fk", "order_id", "order_lines", "order_id"),
            row("line_fk", "line_no", "order_lines", "line_no"),
            row("reviewer_fk", "reviewer_id", "customers", "id"),
        ]);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].columns, ["order_id", "line_no"]);
        assert_eq!(keys[0].target_columns, ["order_id", "line_no"]);
        assert_eq!(keys[1].name, "reviewer_fk");
        assert_eq!(keys[1].columns, ["reviewer_id"]);
    }

    #[test]
    fn same_named_constraints_to_different_targets_stay_separate() {
        let keys = ForeignKey::group([
            row("fk", "a", "left_table", "id"),
            row("fk", "b", "right_table", "id"),
        ]);
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn inbound_foreign_keys_group_by_source_and_constraint_in_ordinal_order() {
        let line_notes = TableRef {
            schema: "public".into(),
            name: "line_notes".into(),
        };
        let other_notes = TableRef {
            schema: "alt".into(),
            name: "line_notes".into(),
        };
        let keys = InboundForeignKey::group([
            (
                "line_notes_line_fk".into(),
                line_notes.clone(),
                "note_order".into(),
                "order_id".into(),
            ),
            (
                "line_notes_line_fk".into(),
                line_notes.clone(),
                "note_line".into(),
                "line_no".into(),
            ),
            (
                "line_notes_line_fk".into(),
                other_notes.clone(),
                "note_order".into(),
                "order_id".into(),
            ),
        ]);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].source, line_notes);
        assert_eq!(keys[0].columns, ["note_order", "note_line"]);
        assert_eq!(keys[0].target_columns, ["order_id", "line_no"]);
        assert_eq!(keys[1].source, other_notes);
    }
}
