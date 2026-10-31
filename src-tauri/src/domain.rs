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
    /// Extended schema object explorer, DDL inspection, and object dependencies.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub object_explorer: bool,
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
    #[serde(default)]
    pub insertable: bool,
    #[serde(default)]
    pub has_default: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DatabaseObjectKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    Function,
    Procedure,
    Sequence,
    Trigger,
    Index,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseObjectRef {
    pub schema: String,
    pub name: String,
    pub kind: DatabaseObjectKind,
    /// Disambiguates overloaded PostgreSQL routines (identity argument types)
    /// or table-scoped triggers and indexes (`owner_table`).
    #[serde(default)]
    pub identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseObjectSummary {
    pub object: DatabaseObjectRef,
    pub owner_table: Option<String>,
    pub signature: Option<String>,
    pub estimated_rows: Option<f64>,
    pub total_bytes: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub method: String,
    pub unique: bool,
    pub primary: bool,
    pub expressions: Vec<String>,
    pub predicate: Option<String>,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintKind {
    PrimaryKey,
    ForeignKey,
    Unique,
    Check,
    Exclusion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintInfo {
    pub name: String,
    pub kind: ConstraintKind,
    pub columns: Vec<String>,
    pub target: Option<TableRef>,
    pub target_columns: Vec<String>,
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TriggerInfo {
    pub name: String,
    pub table_name: String,
    pub timing: String,
    pub events: Vec<String>,
    pub orientation: String,
    pub enabled: bool,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RoutineInfo {
    pub identity_arguments: String,
    pub arguments: String,
    pub return_type: Option<String>,
    pub language: String,
    pub volatility: Option<String>,
    pub security_definer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SequenceInfo {
    pub data_type: String,
    pub start_value: String,
    pub min_value: String,
    pub max_value: String,
    pub increment_by: String,
    pub cache_size: String,
    pub cycle: bool,
    pub last_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEdge {
    pub object: DatabaseObjectRef,
    pub relation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ObjectDdlStatus {
    Available { ddl: String, truncated: bool },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDetails {
    pub object: DatabaseObjectRef,
    pub owner_table: Option<TableRef>,
    pub comment: Option<String>,
    pub ddl: ObjectDdlStatus,
    pub indexes: Vec<IndexInfo>,
    pub constraints: Vec<ConstraintInfo>,
    pub triggers: Vec<TriggerInfo>,
    pub routine: Option<RoutineInfo>,
    pub sequence: Option<SequenceInfo>,
    pub uses: Vec<DependencyEdge>,
    pub used_by: Vec<DependencyEdge>,
}

impl ObjectDdlStatus {
    pub fn capped(self, max_bytes: usize) -> Self {
        match self {
            Self::Available { mut ddl, truncated } => {
                if ddl.len() <= max_bytes {
                    return Self::Available { ddl, truncated };
                }
                let mut end = max_bytes;
                while end > 0 && !ddl.is_char_boundary(end) {
                    end -= 1;
                }
                ddl.truncate(end);
                Self::Available {
                    ddl,
                    truncated: true,
                }
            }
            other => other,
        }
    }
}

pub fn finalize_dependency_edges(
    self_object: &DatabaseObjectRef,
    edges: impl IntoIterator<Item = DependencyEdge>,
    max_edges: usize,
) -> Vec<DependencyEdge> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for edge in edges {
        if edge.object == *self_object {
            continue;
        }
        if seen.insert(edge.object.clone()) {
            result.push(edge);
            if result.len() >= max_edges {
                break;
            }
        }
    }
    result
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
    SessionState {
        state: SessionRuntimeState,
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
pub struct ResultDeletes {
    pub result_id: String,
    pub rows: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct InsertValue {
    pub column: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RowInsert {
    pub values: Vec<InsertValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TableChanges {
    #[serde(default)]
    pub result_id: Option<String>,
    #[serde(default)]
    pub updates: Vec<ResultChanges>,
    #[serde(default)]
    pub deletes: Vec<ResultDeletes>,
    #[serde(default)]
    pub inserts: Vec<RowInsert>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedQuery {
    pub id: String,
    pub title: String,
    pub sql: String,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[ts(type = "number")]
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedQueryInput {
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    pub sql: String,
    #[serde(default)]
    pub connection_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BookmarkTarget {
    Object {
        connection_id: String,
        object: DatabaseObjectRef,
    },
    SavedQuery {
        saved_query_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedBookmark {
    pub id: String,
    pub label: String,
    pub target: BookmarkTarget,
    #[ts(type = "number")]
    pub created_at_ms: u64,
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
    #[serde(default)]
    pub saved_queries: Vec<SavedQuery>,
    #[serde(default)]
    pub bookmarks: Vec<SavedBookmark>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryDraft {
    pub id: String,
    pub connection_id: String,
    pub title: String,
    pub sql: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedTableLayout {
    pub connection_id: String,
    pub table: TableRef,
    pub column_order: Vec<String>,
    pub hidden_columns: Vec<String>,
    pub column_widths: std::collections::BTreeMap<String, u32>,
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
    #[serde(default)]
    pub table_layouts: Vec<SavedTableLayout>,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TransactionMode {
    #[default]
    AutoCommit,
    Manual,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TransactionState {
    #[default]
    Idle,
    InTransaction,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SessionContextKind {
    #[default]
    None,
    Schema,
    Database,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntimeState {
    pub session_id: String,
    pub transaction_mode: TransactionMode,
    pub transaction_state: TransactionState,
    pub context_kind: SessionContextKind,
    pub active_context: Option<String>,
    pub database: String,
    pub role: String,
    pub search_path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionContextOptions {
    pub state: SessionRuntimeState,
    pub available: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Csv,
    Tsv,
    Json,
    Ndjson,
    SqlInsert,
    Markdown,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Json => "json",
            Self::Ndjson => "ndjson",
            Self::SqlInsert => "sql",
            Self::Markdown => "md",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
            Self::Json => "JSON",
            Self::Ndjson => "NDJSON",
            Self::SqlInsert => "SQL INSERT",
            Self::Markdown => "Markdown",
        }
    }
}

/// Capability token representing a user-selected native file destination or source.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileSelection {
    pub token: String,
    pub file_name: String,
    pub display_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenedSqlFile {
    pub token: String,
    pub file_name: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SavedSqlFile {
    pub token: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportSource {
    Result {
        result_id: String,
        #[serde(default)]
        column_ids: Vec<String>,
    },
    Table {
        session_id: String,
        table: TableRef,
        #[serde(default)]
        query: BrowseQuery,
        #[serde(default)]
        approval: Option<BrowseApproval>,
        #[serde(default)]
        column_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub destination_token: String,
    pub format: ExportFormat,
    pub source: ExportSource,
    #[serde(default = "default_true")]
    pub include_headers: bool,
    #[serde(default)]
    pub engine: Option<EngineKind>,
    #[serde(default)]
    pub target_table: Option<TableRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportEvent {
    Started {
        export_id: String,
        file_name: String,
        source_truncated: bool,
    },
    Progress {
        export_id: String,
        #[ts(type = "number")]
        rows_written: u64,
        #[ts(type = "number")]
        bytes_written: u64,
    },
    Finished {
        export_id: String,
        status: RunStatus,
        #[ts(type = "number")]
        rows_written: u64,
        #[ts(type = "number")]
        bytes_written: u64,
        output_file_name: Option<String>,
        partial_file_name: Option<String>,
        error: Option<AppError>,
        elapsed_ms: u32,
    },
}

fn default_null_token() -> String {
    "\\N".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CsvDelimiter {
    Comma,
    Tab,
    Semicolon,
    Pipe,
}

impl CsvDelimiter {
    pub fn byte(self) -> u8 {
        match self {
            Self::Comma => b',',
            Self::Tab => b'\t',
            Self::Semicolon => b';',
            Self::Pipe => b'|',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Comma => "Comma (,)",
            Self::Tab => "Tab (\\t)",
            Self::Semicolon => "Semicolon (;)",
            Self::Pipe => "Pipe (|)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportOptions {
    pub delimiter: CsvDelimiter,
    #[serde(default = "default_true")]
    pub has_header: bool,
    #[serde(default = "default_null_token")]
    pub null_token: String,
}

impl Default for CsvImportOptions {
    fn default() -> Self {
        Self {
            delimiter: CsvDelimiter::Comma,
            has_header: true,
            null_token: default_null_token(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CsvColumnMapping {
    pub csv_index: usize,
    pub target_column: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportIssue {
    pub line: usize,
    pub column: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportPreview {
    pub file_name: String,
    pub headers: Vec<String>,
    pub mappings: Vec<CsvColumnMapping>,
    pub unmapped_defaults: Vec<String>,
    pub unmapped_nulls: Vec<String>,
    pub sample_rows: Vec<Vec<Option<String>>>,
    pub total_rows: usize,
    pub batch_size: usize,
    pub batch_count: usize,
    pub issues: Vec<CsvImportIssue>,
    pub total_issues: usize,
    pub can_prepare: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CompareSide {
    #[serde(rename_all = "camelCase")]
    Snapshot {
        result_ids: Vec<String>,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        truncated_hint: bool,
    },
    #[serde(rename_all = "camelCase")]
    LiveTable {
        session_id: String,
        table: TableRef,
        #[serde(default)]
        query: BrowseQuery,
        #[serde(default)]
        approval: Option<BrowseApproval>,
        #[serde(default)]
        label: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CompareRequest {
    pub left: CompareSide,
    pub right: CompareSide,
    #[serde(default)]
    pub key_columns: Vec<String>,
    #[serde(default)]
    pub ignore_column_case: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AlignedCompareColumn {
    pub name: String,
    pub left_name: String,
    pub right_name: String,
    pub left_type: String,
    pub right_type: String,
    pub kind_mismatch: bool,
    pub is_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CellDiff {
    pub column: String,
    pub left: Option<String>,
    pub right: Option<String>,
    pub type_mismatch: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RowDiffKind {
    Different,
    OnlyLeft,
    OnlyRight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ComparedRowDiff {
    pub kind: RowDiffKind,
    pub key_values: Vec<String>,
    pub cells: Vec<CellDiff>,
    pub left_row: Option<Vec<Option<String>>>,
    pub right_row: Option<Vec<Option<String>>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateKeySample {
    pub key_values: Vec<String>,
    pub occurrences: usize,
    pub counterpart_occurrences: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CompareSideAnomalies {
    pub null_key_count: usize,
    pub null_key_samples: Vec<Vec<Option<String>>>,
    pub duplicate_key_rows: usize,
    pub duplicate_distinct_keys: usize,
    pub excluded_counterpart_rows: usize,
    pub duplicate_key_samples: Vec<DuplicateKeySample>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DataCompareReport {
    pub left_label: String,
    pub right_label: String,
    pub left_truncated: bool,
    pub right_truncated: bool,
    pub key_columns: Vec<String>,
    pub aligned_columns: Vec<AlignedCompareColumn>,
    pub unaligned_left_columns: Vec<String>,
    pub unaligned_right_columns: Vec<String>,
    pub left_total_rows: usize,
    pub right_total_rows: usize,
    pub identical_count: usize,
    pub different_count: usize,
    pub only_left_count: usize,
    pub only_right_count: usize,
    pub left_anomalies: CompareSideAnomalies,
    pub right_anomalies: CompareSideAnomalies,
    pub diffs: Vec<ComparedRowDiff>,
    pub diffs_truncated: bool,
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

    #[test]
    fn ddl_capping_preserves_utf8_boundaries_and_marks_truncation() {
        let ddl = ObjectDdlStatus::Available {
            ddl: "CREATE VIEW v AS SELECT 'héllo世界';".into(),
            truncated: false,
        };
        let intact = ddl.clone().capped(1024);
        assert_eq!(intact, ddl);

        // Cut right inside multi-byte 'é' / '世'
        let cut = "SELECT 'é世界';";
        let capped = ObjectDdlStatus::Available {
            ddl: cut.into(),
            truncated: false,
        }
        .capped(9);
        match capped {
            ObjectDdlStatus::Available { ddl, truncated } => {
                assert!(truncated);
                assert_eq!(ddl, "SELECT '");
            }
            ObjectDdlStatus::Unavailable { .. } => panic!("expected available DDL"),
        }
    }

    #[test]
    fn dependency_edges_filter_self_cycles_deduplicate_and_bound() {
        let self_view = DatabaseObjectRef {
            schema: "public".into(),
            name: "v_cycle_a".into(),
            kind: DatabaseObjectKind::View,
            identity: None,
        };
        let mutual_view = DatabaseObjectRef {
            schema: "public".into(),
            name: "v_cycle_b".into(),
            kind: DatabaseObjectKind::View,
            identity: None,
        };
        let fn_int = DatabaseObjectRef {
            schema: "public".into(),
            name: "calc".into(),
            kind: DatabaseObjectKind::Function,
            identity: Some("integer".into()),
        };
        let fn_num = DatabaseObjectRef {
            schema: "public".into(),
            name: "calc".into(),
            kind: DatabaseObjectKind::Function,
            identity: Some("numeric, integer".into()),
        };
        let edges = finalize_dependency_edges(
            &self_view,
            vec![
                DependencyEdge {
                    object: self_view.clone(),
                    relation: "rewrite self".into(),
                },
                DependencyEdge {
                    object: mutual_view.clone(),
                    relation: "view".into(),
                },
                DependencyEdge {
                    object: mutual_view.clone(),
                    relation: "duplicate".into(),
                },
                DependencyEdge {
                    object: fn_int.clone(),
                    relation: "function".into(),
                },
                DependencyEdge {
                    object: fn_num.clone(),
                    relation: "function".into(),
                },
            ],
            2,
        );
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].object, mutual_view);
        assert_eq!(edges[0].relation, "view");
        assert_eq!(edges[1].object, fn_int);
    }
}
