use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use crate::console_contract::*;
pub use crate::relation_inspection::*;

pub use crate::ddl::{
    DdlStatement, IndexConstraintAction, IndexConstraintPreview, SchemaDdlPlan, TableColumnDraft,
    TableMaintenanceAction, TableMaintenancePreview, TableObjectAction, TableObjectPreview,
};
pub use crate::schema_diff::{
    MAX_SNAPSHOT_COLUMNS, MAX_SNAPSHOT_CONSTRAINTS, MAX_SNAPSHOT_INDEXES, MAX_SNAPSHOT_RELATIONS,
    MAX_SNAPSHOT_ROUTINES, MAX_SNAPSHOT_SEQUENCES, MAX_SNAPSHOT_TRIGGERS, MAX_SNAPSHOT_UNSUPPORTED,
    SchemaDiffKind, SchemaDiffReport, SchemaMemberCategory, SchemaMemberDiff, SchemaObjectDiff,
    SchemaSnapshot, SnapshotColumn, SnapshotIndex, SnapshotRoutine, SnapshotSequence,
    SnapshotTable, UnsupportedSchemaObject,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
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
    #[serde(default)]
    pub authentication: SnowflakeAuthentication,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub reviewed_dml: bool,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub reviewed_ddl: bool,
    /// Separate consent for exact owner-authored console SQL. Generated DML/DDL
    /// permissions and legacy profiles must never implicitly grant this.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub reviewed_console: bool,
    /// Per-unit execution/result-delivery budget; absent legacy profiles retain 65 seconds.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_console_timeout"
    )]
    #[ts(optional)]
    pub reviewed_console_timeout_seconds: Option<u32>,
    pub role: String,
    pub warehouse: String,
    pub schema: String,
}

impl SnowflakeConfig {
    pub fn effective_console_timeout_seconds(&self) -> AppResult<u32> {
        let seconds = self.reviewed_console_timeout_seconds.unwrap_or(65);
        if !(1..=86_400).contains(&seconds) {
            return Err(AppError::invalid(
                "Reviewed console timeout must be between 1 and 86400 seconds",
            ));
        }
        Ok(seconds)
    }
}

fn deserialize_console_timeout<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u32>, D::Error> {
    let seconds = u32::deserialize(deserializer)?;
    if !(1..=86_400).contains(&seconds) {
        return Err(serde::de::Error::custom("Invalid reviewed console timeout"));
    }
    Ok(Some(seconds))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeAuthentication {
    #[default]
    LocalApplicationOauth,
    ExternalBrowser,
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
    /// Permanent reviewed-console ownership; background SQL must stay paused.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub console_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EngineCapabilities {
    pub sql: bool,
    pub transactions: bool,
    /// Reviewed explicit boundaries, independently of AUTOCOMMIT mode changes.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub explicit_transaction_controls: bool,
    pub cancellation: bool,
    pub table_browsing: bool,
    /// Session-bound relation inspection and bounded read-only previews.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub relation_browsing: bool,
    pub table_editing: bool,
    /// Server-side typed sorting and filtering while browsing tables.
    pub table_filtering: bool,
    pub processes: bool,
    /// Extended schema object explorer, DDL inspection, and object dependencies.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub object_explorer: bool,
    /// Per-schema entity-relationship diagram generation.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub schema_diagram: bool,
    /// Structural schema snapshots and schema diff comparison.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub schema_diff: bool,
    /// Structured table, column, index, constraint, and migration DDL editing.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub ddl_editing: bool,
    /// Narrow reviewed generated table DDL on the Snowflake SDK route.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub snowflake_reviewed_ddl: bool,
    /// Reviewed native Snowflake relation grid editing and manual batch transactions.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub snowflake_grid_editing: bool,
    /// Active lock-wait and blocker chain inspection.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub lock_inspection: bool,
    /// Executing query plan analysis (EXPLAIN ANALYZE / ANALYZE FORMAT=JSON).
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub explain_analyze: bool,
    /// External system-browser OAuth sign-in flow.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub oauth_browser: bool,
    /// Executing plan format available to this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub actual_plan: Option<ActualPlanFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ActualPlanFormat {
    PostgresJson,
    MariaDbJson,
    MysqlTree,
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

/// Half-open UTF-16 code-unit range `[start_utf16, end_utf16)` plus 1-based
/// Unicode scalar line and column coordinates within the submitted SQL buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SqlSpan {
    pub start_utf16: usize,
    pub end_utf16: usize,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ErrorLocation {
    pub span: SqlSpan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub statement_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sql_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum QueryParamKind {
    Text,
    Integer,
    Decimal,
    Boolean,
    Uuid,
    Date,
    Timestamp,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueryParamSlot {
    pub key: String,
    pub positional: bool,
    pub occurrences: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryParamInput {
    pub key: String,
    pub kind: QueryParamKind,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StatementPlan {
    pub sql: String,
    pub kind: StatementKind,
    pub missing_where: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub span: Option<SqlSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreparedPlan {
    /// Dedicated typed execution route; never a generic row-count job.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub reviewed_snowflake_dml: bool,
    /// Dedicated typed reviewed Snowflake DDL route; never a generic SQL job.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub reviewed_snowflake_ddl: bool,
    /// Dedicated typed reviewed Snowflake relation grid route; never a generic SQL job.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub reviewed_snowflake_grid: bool,
    pub id: String,
    pub session_id: String,
    pub statements: Vec<StatementPlan>,
    #[serde(default)]
    #[ts(optional, as = "Option<Vec<QueryParamInput>>")]
    pub bound_parameters: Vec<QueryParamInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub binding_digest: Option<String>,
    pub requires_approval: bool,
    pub requires_passphrase: bool,
    pub confirmation_text: Option<String>,
    /// Separate exact-target confirmation (e.g. DROP TABLE), independent of policy confirmation_text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target_confirmation_text: Option<String>,
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

/// Bounds for one schema relationship diagram.
pub const MAX_DIAGRAM_TABLES: usize = 500;
pub const MAX_DIAGRAM_COLUMNS: usize = 20_000;
pub const MAX_DIAGRAM_KEY_ROWS: usize = 20_000;

/// One column shown in a schema diagram node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiagramColumn {
    pub name: String,
    pub database_type: String,
    pub nullable: bool,
    pub primary_key: bool,
}

/// One relation in a schema diagram. Columns are in ordinal order and only
/// include columns the session may read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiagramTable {
    pub table: TableRef,
    pub kind: String,
    pub columns: Vec<DiagramColumn>,
}

/// One foreign-key constraint whose source is in the diagram schema. The
/// target may be in another schema. `columns[i]` references `target_columns[i]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiagramForeignKey {
    pub name: String,
    pub source: TableRef,
    pub columns: Vec<String>,
    pub target: TableRef,
    pub target_columns: Vec<String>,
}

/// Read-only relationship graph for one schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiagram {
    pub schema: String,
    pub tables: Vec<DiagramTable>,
    pub foreign_keys: Vec<DiagramForeignKey>,
    /// Tables, columns or constraints were omitted by the diagram bounds.
    pub truncated: bool,
    /// Constraints were omitted because some of their columns are not readable.
    pub restricted: bool,
}

/// One catalog row per foreign-key column, in source table, constraint and
/// ordinal order. `expected` is the constraint width when the engine reports it.
pub struct DiagramKeyRow {
    pub name: String,
    pub source_table: String,
    pub target: TableRef,
    pub column: String,
    pub target_column: String,
    pub expected: Option<usize>,
}

impl SchemaDiagram {
    /// Builds a bounded diagram from catalog rows. Each input may hold one
    /// row beyond its bound, which marks the diagram as truncated.
    pub fn assemble(
        schema: &str,
        tables: Vec<(String, String)>,
        columns: Vec<(String, DiagramColumn)>,
        keys: Vec<DiagramKeyRow>,
    ) -> Self {
        let mut truncated = false;
        let mut tables = tables;
        if tables.len() > MAX_DIAGRAM_TABLES {
            tables.truncate(MAX_DIAGRAM_TABLES);
            truncated = true;
        }
        let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut nodes: Vec<DiagramTable> = Vec::with_capacity(tables.len());
        for (name, kind) in tables {
            if index.contains_key(&name) {
                continue;
            }
            index.insert(name.clone(), nodes.len());
            nodes.push(DiagramTable {
                table: TableRef {
                    schema: schema.to_owned(),
                    name,
                },
                kind,
                columns: Vec::new(),
            });
        }
        let mut columns = columns;
        if columns.len() > MAX_DIAGRAM_COLUMNS {
            columns.truncate(MAX_DIAGRAM_COLUMNS);
            truncated = true;
        }
        for (table, column) in columns {
            if let Some(&i) = index.get(&table) {
                nodes[i].columns.push(column);
            }
        }
        let mut keys = keys;
        let key_rows_limited = keys.len() > MAX_DIAGRAM_KEY_ROWS;
        if key_rows_limited {
            keys.truncate(MAX_DIAGRAM_KEY_ROWS);
            truncated = true;
        }
        let mut grouped: Vec<(DiagramForeignKey, Option<usize>)> = Vec::new();
        for row in keys {
            match grouped.last_mut() {
                Some((key, _)) if key.name == row.name && key.source.name == row.source_table => {
                    key.columns.push(row.column);
                    key.target_columns.push(row.target_column);
                }
                _ => grouped.push((
                    DiagramForeignKey {
                        name: row.name,
                        source: TableRef {
                            schema: schema.to_owned(),
                            name: row.source_table,
                        },
                        columns: vec![row.column],
                        target: row.target,
                        target_columns: vec![row.target_column],
                    },
                    row.expected,
                )),
            }
        }
        // The last constraint may be cut by the row bound.
        if key_rows_limited {
            grouped.pop();
        }
        let mut restricted = false;
        let mut foreign_keys = Vec::with_capacity(grouped.len());
        for (key, expected) in grouped {
            if expected.is_some_and(|width| width != key.columns.len()) {
                restricted = true;
                continue;
            }
            let source_known = index.contains_key(&key.source.name);
            let target_known = key.target.schema != schema || index.contains_key(&key.target.name);
            if !source_known || !target_known {
                truncated = true;
                continue;
            }
            foreign_keys.push(key);
        }
        Self {
            schema: schema.to_owned(),
            tables: nodes,
            foreign_keys,
            truncated,
            restricted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TableSchema {
    pub table: TableRef,
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Vec<String>,
    pub foreign_keys: Vec<ForeignKey>,
    #[serde(default)]
    #[ts(optional, as = "Option<Vec<InboundForeignKey>>")]
    pub inbound_foreign_keys: Vec<InboundForeignKey>,
    pub editable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub method: String,
    pub unique: bool,
    pub primary: bool,
    pub expressions: Vec<String>,
    pub predicate: Option<String>,
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub size_bytes: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scans: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tuples_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tuples_fetched: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cardinality: Option<f64>,
}

/// Read-only storage and planner/collector activity estimates for a relation.
/// Row and tuple counts come from catalog/statistics views only — Joinery never
/// runs automatic `COUNT(*)` or `ANALYZE`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TableStorageStats {
    pub estimated_rows: Option<f64>,
    pub table_bytes: Option<f64>,
    pub index_bytes: Option<f64>,
    pub free_bytes: Option<f64>,
    pub total_bytes: Option<f64>,
    pub live_tuples: Option<f64>,
    pub dead_tuples: Option<f64>,
    pub sequential_scans: Option<f64>,
    pub index_scans: Option<f64>,
    pub last_analyze: Option<String>,
    pub last_vacuum: Option<String>,
    #[serde(default)]
    pub stats_stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub restricted_reason: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub storage: Option<TableStorageStats>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ResultEditability {
    Editable {
        table: TableRef,
        primary_key: Vec<String>,
    },
    ReadOnly {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResultInfo {
    pub id: String,
    pub run_id: String,
    pub statement_index: usize,
    pub columns: Vec<ColumnInfo>,
    pub retained_rows: usize,
    pub status: RetentionStatus,
    pub table: Option<TableRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub editability: Option<ResultEditability>,
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
    Stopped,
    Failed,
    Cancelled,
    OutcomeUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionStopReason {
    ResultLimit,
    Cancelled,
    Failed,
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSummary {
    pub planned: usize,
    /// Local driver invocations, not proof of remote SQL submission.
    pub attempted: usize,
    /// Successful driver returns, not proof of commit or complete retention.
    pub completed: usize,
    pub not_attempted: usize,
    pub stop_reason: Option<ExecutionStopReason>,
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
    RunMetrics {
        run_id: String,
        queue_ms: u32,
        active_ms: u32,
        first_row_ms: Option<u32>,
        total_ms: u32,
    },
    PlanContext {
        run_id: String,
        state: Option<SessionRuntimeState>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        execution_summary: Option<ExecutionSummary>,
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

pub const MAX_LOCK_WAITS: usize = 500;

/// One server session waiting on a lock, with the sessions that block it.
/// Process identifiers match `ProcessInfo::id` for the same engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LockWait {
    pub waiting_id: String,
    pub user: String,
    pub wait_seconds: f64,
    pub lock_type: String,
    pub lock_mode: String,
    pub object: Option<TableRef>,
    /// Engine wait classification when reported, e.g. `Lock:transactionid`.
    pub wait_event: Option<String>,
    pub query: Option<String>,
    pub blocking_ids: Vec<String>,
    pub own_session: bool,
}

impl LockWait {
    /// Merges rows that describe the same waiting session, keeping the first
    /// lock description and every distinct blocker. Longest waits come first.
    /// Returns whether the list was cut by `MAX_LOCK_WAITS`.
    pub fn merge(rows: impl IntoIterator<Item = LockWait>) -> (Vec<LockWait>, bool) {
        let mut merged: Vec<LockWait> = Vec::new();
        let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for row in rows {
            match index.get(&row.waiting_id) {
                Some(&i) => {
                    for id in row.blocking_ids {
                        if !merged[i].blocking_ids.contains(&id) {
                            merged[i].blocking_ids.push(id);
                        }
                    }
                }
                None => {
                    index.insert(row.waiting_id.clone(), merged.len());
                    let mut row = row;
                    let mut seen = std::collections::HashSet::new();
                    row.blocking_ids.retain(|id| seen.insert(id.clone()));
                    merged.push(row);
                }
            }
        }
        merged.sort_by(|a, b| {
            b.wait_seconds
                .total_cmp(&a.wait_seconds)
                .then_with(|| a.waiting_id.cmp(&b.waiting_id))
        });
        let truncated = merged.len() > MAX_LOCK_WAITS;
        merged.truncate(MAX_LOCK_WAITS);
        (merged, truncated)
    }
}

/// Lock waits observed in one read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LockWaitSnapshot {
    pub waits: Vec<LockWait>,
    /// More waiting sessions existed than `MAX_LOCK_WAITS`.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HistoryProvenance {
    #[default]
    Recorded,
    Admitted,
    Interrupted,
    RestoreRecovered,
}

// Unit-enum Deserialize also accepts {"variant":null}. Persisted lifecycle
// provenance has a closed string representation shared with summary projections.
impl<'de> Deserialize<'de> for HistoryProvenance {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "recorded" => Ok(Self::Recorded),
            "admitted" => Ok(Self::Admitted),
            "interrupted" => Ok(Self::Interrupted),
            "restoreRecovered" => Ok(Self::RestoreRecovered),
            _ => Err(serde::de::Error::custom("Invalid history provenance")),
        }
    }
}

fn validate_history_lifecycle(
    provenance: HistoryProvenance,
    status: Option<RunStatus>,
    elapsed_ms: Option<u32>,
    has_outcome_evidence: bool,
) -> AppResult<()> {
    if (provenance == HistoryProvenance::Recorded && status.is_none())
        || (status.is_none() && elapsed_ms.is_some())
        || (provenance == HistoryProvenance::Admitted
            && (status.is_some() || elapsed_ms.is_some() || has_outcome_evidence))
    {
        return Err(AppError::invalid("History lifecycle is inconsistent"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// Legacy entries predate lifecycle provenance and contain recorded outcomes.
    #[serde(default)]
    pub provenance: HistoryProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub execution_summary: Option<ExecutionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snowflake_dml: Option<SnowflakeDmlEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snowflake_ddl: Option<SnowflakeDdlEvidence>,
    pub id: String,
    pub connection_id: String,
    pub connection_name: String,
    #[ts(type = "number")]
    pub started_at: u64,
    pub sql: Option<String>,
    pub statement_count: usize,
    pub status: Option<RunStatus>,
    pub elapsed_ms: Option<u32>,
}

impl HistoryEntry {
    pub(crate) fn validate_lifecycle(&self) -> AppResult<()> {
        validate_history_lifecycle(
            self.provenance,
            self.status,
            self.elapsed_ms,
            self.execution_summary.is_some()
                || self.snowflake_dml.is_some()
                || self.snowflake_ddl.is_some(),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryCursor {
    #[ts(type = "number")]
    pub started_at: u64,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryPageRequest {
    pub search: String,
    pub cursor: Option<HistoryCursor>,
    pub limit: u32,
}

/// Bounded SQL preview; opening a row explicitly retrieves its current detail.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistorySummary {
    #[serde(default)]
    pub provenance: HistoryProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub execution_summary: Option<ExecutionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snowflake_dml: Option<SnowflakeDmlEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snowflake_ddl: Option<SnowflakeDdlEvidence>,
    pub id: String,
    pub connection_id: String,
    pub connection_name: String,
    #[ts(type = "number")]
    pub started_at: u64,
    pub sql_preview: Option<String>,
    pub sql_stored: bool,
    pub statement_count: usize,
    pub status: Option<RunStatus>,
    pub elapsed_ms: Option<u32>,
}

impl HistorySummary {
    pub(crate) fn validate_lifecycle(&self) -> AppResult<()> {
        validate_history_lifecycle(
            self.provenance,
            self.status,
            self.elapsed_ms,
            self.execution_summary.is_some()
                || self.snowflake_dml.is_some()
                || self.snowflake_ddl.is_some(),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub entries: Vec<HistorySummary>,
    pub next_cursor: Option<HistoryCursor>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryBackupFormat {
    LegacySqlite,
    EncryptedV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryBackup {
    pub metadata: MetadataBackup,
    pub format: RecoveryBackupFormat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryPhase {
    Staging,
    Validating,
    Encrypting,
    Decrypting,
    Merging,
    CreatingUndo,
    Replacing,
    Finalizing,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryProgress {
    pub phase: RecoveryPhase,
    #[ts(type = "number")]
    pub completed_bytes: u64,
    #[ts(type = "number | null")]
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryOperationKind {
    Create,
    Restore,
    Import,
    Export,
}

/// Secret input travels separately and is never included in operation status.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRequest {
    pub kind: RecoveryOperationKind,
    pub backup_id: Option<String>,
    pub digest: Option<String>,
    pub confirmation: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryOperationState {
    Running,
    Succeeded,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryOperation {
    pub id: String,
    /// Absent when reconciling an identifier whose process-local status was lost.
    pub kind: Option<RecoveryOperationKind>,
    pub state: RecoveryOperationState,
    pub progress: RecoveryProgress,
    pub cancellation_requested: bool,
    pub backup: Option<RecoveryBackup>,
    pub error: Option<AppError>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SqlKeywordCase {
    Preserve,
    #[default]
    Upper,
    Lower,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SqlFormatSettings {
    pub keyword_case: SqlKeywordCase,
    pub indent_width: u8,
    pub lines_between_queries: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dialects: Option<SqlFormatDialects>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SqlFormatStyle {
    pub keyword_case: SqlKeywordCase,
    pub indent_width: u8,
    pub lines_between_queries: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SqlFormatDialects {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub postgres: Option<SqlFormatStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mysql: Option<SqlFormatStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snowflake: Option<SqlFormatStyle>,
}

impl Default for SqlFormatSettings {
    fn default() -> Self {
        Self {
            keyword_case: SqlKeywordCase::Upper,
            indent_width: 2,
            lines_between_queries: 1,
            dialects: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SqlSnippetPlace {
    #[default]
    Any,
    Statement,
    From,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SqlSnippet {
    pub id: String,
    pub trigger: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub engine: Option<EngineKind>,
    /// Completion site. Missing values from older metadata stay available everywhere.
    #[serde(default)]
    pub place: SqlSnippetPlace,
    #[ts(type = "number")]
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SqlSnippetInput {
    #[serde(default)]
    pub id: Option<String>,
    pub trigger: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub engine: Option<EngineKind>,
    /// Completion site. Missing values from older metadata stay available everywhere.
    #[serde(default)]
    pub place: SqlSnippetPlace,
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
    /// Opt-in anonymous temporary result storage; applies to new results.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub temporary_result_storage: bool,
    pub workspace: Option<SavedWorkspace>,
    #[serde(default)]
    pub saved_queries: Vec<SavedQuery>,
    #[serde(default)]
    pub bookmarks: Vec<SavedBookmark>,
    #[serde(default)]
    pub sql_format_settings: SqlFormatSettings,
    #[serde(default)]
    pub sql_snippets: Vec<SqlSnippet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryDraft {
    pub id: String,
    pub connection_id: String,
    pub title: String,
    pub sql: String,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub pinned: bool,
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
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    pub sidebar_collapsed: bool,
    pub inspector_visible: bool,
    pub active_draft_id: Option<String>,
    pub drafts: Vec<QueryDraft>,
    #[serde(default)]
    pub table_layouts: Vec<SavedTableLayout>,
    #[serde(default)]
    #[ts(optional, as = "Option<std::collections::BTreeMap<String, String>>")]
    pub shortcuts: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUsage {
    pub retained_bytes: usize,
    #[serde(default)]
    #[ts(optional, as = "Option<usize>")]
    pub temporary_result_bytes: usize,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<ErrorLocation>")]
    pub location: Option<Box<ErrorLocation>>,
    /// The observed editor context is no longer valid. This is distinct from
    /// an error reading one table, which must preserve other cached columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub completion_invalidated: Option<bool>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            location: None,
            completion_invalidated: None,
        }
    }
    pub fn with_location(mut self, location: ErrorLocation) -> Self {
        self.location = Some(Box::new(location));
        self
    }
    pub fn with_completion_invalidated(mut self) -> Self {
        self.completion_invalidated = Some(true);
        self
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
    /// Engine-specific canonical identity beyond the common display fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub context_fingerprint: Option<String>,
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
    /// Observation only; never a valid requested transaction mode.
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TransactionState {
    #[default]
    Idle,
    InTransaction,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TransactionUsability {
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ExplicitTransactionAction {
    Begin,
    Commit,
    Rollback,
}

impl ExplicitTransactionAction {
    pub fn sql(self) -> &'static str {
        match self {
            Self::Begin => crate::drivers::snowflake_sql_contract::BEGIN_SQL,
            Self::Commit => crate::drivers::snowflake_sql_contract::COMMIT_SQL,
            Self::Rollback => crate::drivers::snowflake_sql_contract::ROLLBACK_SQL,
        }
    }
}

/// Opaque one-use authority; never a claim about transaction usability/effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum TransactionControlAuthority {
    Observed {
        revision: String,
        #[serde(rename = "allowedActions")]
        allowed_actions: Vec<ExplicitTransactionAction>,
    },
    RollbackOnly {
        revision: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransactionControlRequest {
    pub action: ExplicitTransactionAction,
    pub revision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TransactionBoundaryOutcome {
    Acknowledged,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeDmlKind {
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeDmlRequest {
    pub sql: String,
    #[serde(default)]
    pub parameters: Vec<QueryParamInput>,
    /// One-use coherent observed authority; rollback-only authority never admits DML.
    pub revision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeDmlOutcome {
    Succeeded,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeDmlSummaryStatus {
    Validated,
    Unavailable,
}

/// Counts are decimal strings, never rounded JavaScript numbers or status-row counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnowflakeDmlSummary {
    pub status: SnowflakeDmlSummaryStatus,
    pub affected_rows: Option<String>,
    pub multi_joined_rows: Option<String>,
}

impl SnowflakeDmlSummary {
    pub fn unavailable() -> Self {
        Self {
            status: SnowflakeDmlSummaryStatus::Unavailable,
            affected_rows: None,
            multi_joined_rows: None,
        }
    }
}

/// Durable statement evidence; excludes parameters and transient session authority.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnowflakeDmlEvidence {
    pub kind: SnowflakeDmlKind,
    pub statement_outcome: SnowflakeDmlOutcome,
    pub summary: SnowflakeDmlSummary,
    pub retired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnowflakeDmlResult {
    pub kind: SnowflakeDmlKind,
    pub statement_outcome: SnowflakeDmlOutcome,
    pub summary: SnowflakeDmlSummary,
    pub state: SessionRuntimeState,
    pub retired: bool,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeDdlKind {
    CreateTable,
    AddColumn,
    DropTable,
}

impl SnowflakeDdlKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreateTable => "createTable",
            Self::AddColumn => "addColumn",
            Self::DropTable => "dropTable",
        }
    }
}

impl From<crate::safety::snowflake_ddl::Kind> for SnowflakeDdlKind {
    fn from(kind: crate::safety::snowflake_ddl::Kind) -> Self {
        match kind {
            crate::safety::snowflake_ddl::Kind::CreateTable => Self::CreateTable,
            crate::safety::snowflake_ddl::Kind::AddColumn => Self::AddColumn,
            crate::safety::snowflake_ddl::Kind::DropTable => Self::DropTable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeDdlColumnDraft {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

impl SnowflakeDdlColumnDraft {
    pub fn to_table_column_draft(&self) -> TableColumnDraft {
        TableColumnDraft {
            name: self.name.clone(),
            data_type: self.data_type.clone(),
            nullable: self.nullable,
            default_expression: None,
            auto_increment: false,
            primary_key: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SnowflakeDdlAction {
    CreateTable {
        database: String,
        schema: String,
        name: String,
        columns: Vec<SnowflakeDdlColumnDraft>,
    },
    AddColumn {
        browse_revision: String,
        target_id: String,
        inspection_revision: String,
        column: SnowflakeDdlColumnDraft,
    },
    DropTable {
        browse_revision: String,
        target_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeDdlPreflightRequest {
    pub session_id: String,
    pub action: SnowflakeDdlAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeDdlPreflightResponse {
    pub revision: String,
    pub kind: SnowflakeDdlKind,
    pub database: String,
    pub schema: String,
    pub name: String,
    pub qualified_target: String,
    pub sql: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeDdlRequest {
    pub revision: String,
    pub action: SnowflakeDdlAction,
    pub sql: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeDdlOutcome {
    Succeeded,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeDdlSummaryStatus {
    Acknowledged,
    Unavailable,
}

/// Durable statement evidence for reviewed Snowflake DDL; never claims affected rows or rollbackability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnowflakeDdlEvidence {
    pub kind: SnowflakeDdlKind,
    pub qualified_target: String,
    pub statement_outcome: SnowflakeDdlOutcome,
    pub summary_status: SnowflakeDdlSummaryStatus,
    pub catalog_reconciliation_required: bool,
    pub retired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnowflakeDdlResult {
    pub kind: SnowflakeDdlKind,
    pub qualified_target: String,
    pub statement_outcome: SnowflakeDdlOutcome,
    pub summary_status: SnowflakeDdlSummaryStatus,
    pub catalog_reconciliation_required: bool,
    pub state: SessionRuntimeState,
    pub retired: bool,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SnowflakeGridCellInput {
    Default,
    Null,
    Text { value: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnowflakeGridColumnValue {
    pub column_index: usize,
    pub value: SnowflakeGridCellInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SnowflakeGridMutation {
    Insert {
        values: Vec<SnowflakeGridColumnValue>,
    },
    Update {
        #[serde(rename = "rowIndex")]
        row_index: usize,
        changes: Vec<SnowflakeGridColumnValue>,
    },
    Delete {
        #[serde(rename = "rowIndex")]
        row_index: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeGridMode {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PrepareSnowflakeGridEditsRequest {
    pub session_id: String,
    #[serde(default)]
    pub transaction_id: Option<String>,
    pub preview: PreviewRelationRequest,
    pub snapshot_id: String,
    pub mutations: Vec<SnowflakeGridMutation>,
    pub mode: SnowflakeGridMode,
    #[serde(default)]
    pub rollback_prior: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeGridFinishAction {
    Commit,
    Rollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeGridBatchOutcome {
    Committed,
    Pending,
    RolledBack,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnowflakeGridResult {
    pub session_id: String,
    pub operation_id: String,
    pub outcome: SnowflakeGridBatchOutcome,
    pub transaction_id: Option<String>,
    pub recording_error: Option<String>,
    pub metadata_refresh_required: bool,
    pub retired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TransactionControlResult {
    pub action: ExplicitTransactionAction,
    pub boundary_outcome: TransactionBoundaryOutcome,
    pub state: SessionRuntimeState,
    pub retired: bool,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum TransactionObservation {
    Unavailable,
    Observed {
        #[serde(rename = "transactionId")]
        transaction_id: Option<String>,
        #[serde(rename = "abortOnError")]
        abort_on_error: bool,
        usability: TransactionUsability,
    },
}

impl<'de> Deserialize<'de> for TransactionObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
        enum Wire {
            Unavailable {},
            Observed {
                #[serde(
                    rename = "transactionId",
                    deserialize_with = "deserialize_required_nullable"
                )]
                transaction_id: Option<String>,
                #[serde(rename = "abortOnError")]
                abort_on_error: bool,
                usability: TransactionUsability,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Unavailable {} => Self::Unavailable,
            Wire::Observed {
                transaction_id,
                abort_on_error,
                usability,
            } => Self::Observed {
                transaction_id,
                abort_on_error,
                usability,
            },
        })
    }
}

fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
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
    pub role: Option<String>,
    pub search_path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub context_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub transaction_observation: Option<TransactionObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub transaction_control_authority: Option<TransactionControlAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionContextOptions {
    pub state: SessionRuntimeState,
    pub available: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revision: Option<String>,
}

/// One-use authority for a schema selected from this session's observed options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwitchSchemaRequest {
    pub revision: String,
    pub database: String,
    pub schema: String,
    pub context_fingerprint: String,
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
    #[serde(default)]
    pub preview_token: Option<String>,
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

    #[test]
    fn schema_switch_requires_exact_string_authority_fields() {
        let request = serde_json::json!({
            "revision":"opaque-revision", "database":"DB",
            "schema":"Selected.Schema", "contextFingerprint":"opaque-fingerprint"
        });
        let parsed: SwitchSchemaRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(parsed.schema, "Selected.Schema");
        for key in ["revision", "database", "schema", "contextFingerprint"] {
            let mut missing = request.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<SwitchSchemaRequest>(missing).is_err());
            for invalid in [
                serde_json::Value::Null,
                serde_json::json!(1),
                serde_json::json!({}),
            ] {
                let mut wrong_type = request.clone();
                wrong_type[key] = invalid;
                assert!(serde_json::from_value::<SwitchSchemaRequest>(wrong_type).is_err());
            }
        }
        let mut extra = request;
        extra["sql"] = serde_json::json!("USE SCHEMA OTHER");
        assert!(serde_json::from_value::<SwitchSchemaRequest>(extra).is_err());
    }

    #[test]
    fn transaction_observation_requires_complete_tagged_evidence() {
        let observed = serde_json::json!({
            "status":"observed", "transactionId":null,
            "abortOnError":true, "usability":"unknown"
        });
        assert_eq!(
            serde_json::from_value::<TransactionObservation>(observed.clone()).unwrap(),
            TransactionObservation::Observed {
                transaction_id: None,
                abort_on_error: true,
                usability: TransactionUsability::Unknown
            }
        );
        for key in ["transactionId", "abortOnError", "usability"] {
            let mut incomplete = observed.clone();
            incomplete.as_object_mut().unwrap().remove(key);
            assert!(
                serde_json::from_value::<TransactionObservation>(incomplete).is_err(),
                "{key}"
            );
        }
        assert!(
            serde_json::from_value::<TransactionObservation>(
                serde_json::json!({"status":"unavailable","transactionId":null})
            )
            .is_err()
        );
        let opaque = TransactionObservation::Observed {
            transaction_id: Some("18446744073709551616:opaque".into()),
            abort_on_error: false,
            usability: TransactionUsability::Unknown,
        };
        assert_eq!(
            serde_json::from_value::<TransactionObservation>(
                serde_json::to_value(&opaque).unwrap()
            )
            .unwrap(),
            opaque
        );
    }

    fn diagram_column(name: &str, primary_key: bool) -> DiagramColumn {
        DiagramColumn {
            name: name.into(),
            database_type: "bigint".into(),
            nullable: !primary_key,
            primary_key,
        }
    }

    fn key_row(
        name: &str,
        source: &str,
        target: (&str, &str),
        columns: (&str, &str),
        expected: Option<usize>,
    ) -> DiagramKeyRow {
        DiagramKeyRow {
            name: name.into(),
            source_table: source.into(),
            target: TableRef {
                schema: target.0.into(),
                name: target.1.into(),
            },
            column: columns.0.into(),
            target_column: columns.1.into(),
            expected,
        }
    }

    #[test]
    fn lock_waits_merge_blockers_per_waiter_and_sort_by_wait() {
        let wait = |id: &str, seconds: f64, blockers: &[&str]| LockWait {
            waiting_id: id.into(),
            user: "app".into(),
            wait_seconds: seconds,
            lock_type: "RECORD".into(),
            lock_mode: "X".into(),
            object: None,
            wait_event: None,
            query: Some("UPDATE t SET v = 1".into()),
            blocking_ids: blockers.iter().map(|b| b.to_string()).collect(),
            own_session: false,
        };
        let (merged, truncated) = LockWait::merge([
            wait("7", 1.0, &["3"]),
            wait("9", 5.0, &["7", "7"]),
            wait("7", 1.0, &["4"]),
            wait("7", 1.0, &["3"]),
        ]);
        assert!(!truncated);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].waiting_id, "9");
        assert_eq!(merged[0].blocking_ids, ["7"]);
        assert_eq!(merged[1].blocking_ids, ["3", "4"]);

        let (bounded, truncated) =
            LockWait::merge((0..=MAX_LOCK_WAITS).map(|i| wait(&i.to_string(), i as f64, &["x"])));
        assert!(truncated);
        assert_eq!(bounded.len(), MAX_LOCK_WAITS);
        assert_eq!(bounded[0].waiting_id, MAX_LOCK_WAITS.to_string());
    }

    #[test]
    fn schema_diagram_groups_composite_keys_and_keeps_cross_schema_targets() {
        let diagram = SchemaDiagram::assemble(
            "public",
            vec![
                ("line_notes".into(), "r".into()),
                ("order_lines".into(), "r".into()),
                ("orders".into(), "r".into()),
            ],
            vec![
                ("orders".into(), diagram_column("id", true)),
                ("order_lines".into(), diagram_column("order_id", true)),
                ("order_lines".into(), diagram_column("line_no", true)),
                ("line_notes".into(), diagram_column("note_order", false)),
                ("line_notes".into(), diagram_column("note_line", false)),
                ("missing".into(), diagram_column("ignored", false)),
            ],
            vec![
                key_row(
                    "line_notes_line_fk",
                    "line_notes",
                    ("public", "order_lines"),
                    ("note_order", "order_id"),
                    Some(2),
                ),
                key_row(
                    "line_notes_line_fk",
                    "line_notes",
                    ("public", "order_lines"),
                    ("note_line", "line_no"),
                    Some(2),
                ),
                key_row(
                    "order_lines_order_id_fkey",
                    "order_lines",
                    ("public", "orders"),
                    ("order_id", "id"),
                    Some(1),
                ),
                key_row(
                    "orders_audit_fk",
                    "orders",
                    ("audit", "events"),
                    ("id", "order_id"),
                    None,
                ),
            ],
        );
        assert!(!diagram.truncated);
        assert!(!diagram.restricted);
        assert_eq!(diagram.tables.len(), 3);
        assert_eq!(
            diagram.tables[1]
                .columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["order_id", "line_no"]
        );
        assert_eq!(diagram.foreign_keys.len(), 3);
        assert_eq!(diagram.foreign_keys[0].columns, ["note_order", "note_line"]);
        assert_eq!(
            diagram.foreign_keys[0].target_columns,
            ["order_id", "line_no"]
        );
        assert_eq!(diagram.foreign_keys[2].target.schema, "audit");
    }

    #[test]
    fn schema_diagram_withholds_partially_readable_keys_and_marks_bounds() {
        let restricted = SchemaDiagram::assemble(
            "public",
            vec![("a".into(), "r".into()), ("b".into(), "r".into())],
            vec![],
            vec![key_row("a_b", "a", ("public", "b"), ("x", "y"), Some(2))],
        );
        assert!(restricted.restricted);
        assert!(!restricted.truncated);
        assert!(restricted.foreign_keys.is_empty());

        let tables = (0..=MAX_DIAGRAM_TABLES)
            .map(|i| (format!("t{i:04}"), "r".to_string()))
            .collect();
        let bounded = SchemaDiagram::assemble(
            "public",
            tables,
            vec![],
            vec![key_row(
                "late",
                &format!("t{MAX_DIAGRAM_TABLES:04}"),
                ("public", "t0000"),
                ("x", "y"),
                Some(1),
            )],
        );
        assert!(bounded.truncated);
        assert_eq!(bounded.tables.len(), MAX_DIAGRAM_TABLES);
        assert!(bounded.foreign_keys.is_empty());

        let mut rows: Vec<DiagramKeyRow> = (0..MAX_DIAGRAM_KEY_ROWS)
            .map(|i| key_row(&format!("k{i}"), "a", ("public", "b"), ("x", "y"), None))
            .collect();
        rows.push(key_row("cut", "a", ("public", "b"), ("x", "y"), None));
        let keys = SchemaDiagram::assemble(
            "public",
            vec![("a".into(), "r".into()), ("b".into(), "r".into())],
            vec![],
            rows,
        );
        assert!(keys.truncated);
        assert_eq!(keys.foreign_keys.len(), MAX_DIAGRAM_KEY_ROWS - 1);
    }

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

    #[test]
    fn snowflake_ddl_dtos_are_recursively_closed() {
        let create = serde_json::json!({
            "sessionId": "session-1",
            "action": {
                "kind": "createTable",
                "database": "DB",
                "schema": "PUBLIC",
                "name": "ORDERS",
                "columns": [
                    {"name": "ID", "dataType": "NUMBER(38,0)", "nullable": false}
                ]
            }
        });
        assert!(serde_json::from_value::<SnowflakeDdlPreflightRequest>(create.clone()).is_ok());
        let mut extra_outer = create.clone();
        extra_outer["sql"] = serde_json::json!("CREATE TABLE T (ID INT)");
        assert!(serde_json::from_value::<SnowflakeDdlPreflightRequest>(extra_outer).is_err());

        let mut extra_action = create.clone();
        extra_action["action"]["comment"] = serde_json::json!("not allowed");
        assert!(serde_json::from_value::<SnowflakeDdlPreflightRequest>(extra_action).is_err());

        let mut extra_column = create;
        extra_column["action"]["columns"][0]["defaultExpression"] = serde_json::json!("1");
        assert!(serde_json::from_value::<SnowflakeDdlPreflightRequest>(extra_column).is_err());

        let request = serde_json::json!({
            "revision": "rev-1",
            "action": {
                "kind": "dropTable",
                "browseRevision": "browse-1",
                "targetId": "target-1"
            },
            "sql": "DROP TABLE \"DB\".\"PUBLIC\".\"ORDERS\" RESTRICT"
        });
        assert!(serde_json::from_value::<SnowflakeDdlRequest>(request.clone()).is_ok());
        let mut cascade = request;
        cascade["action"]["cascade"] = serde_json::json!(false);
        assert!(serde_json::from_value::<SnowflakeDdlRequest>(cascade).is_err());
    }
}
