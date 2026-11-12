//! Pure, bounded metadata for the SDK connection. Callers own execution, lanes,
//! cancellation, terminal success and context rechecks. These helpers neither
//! establish another session nor provide authoritative table inspection.

use super::CompletionRelations;
use crate::domain::{
    AppError, AppResult, ColumnInfo, ColumnKind, CompletionIdentity, ErrorCode, TableInfo,
    TableRef, TableSchema, TextRow,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{self, Write};

pub(crate) const CONTEXT_MAX_BYTES: usize = 64 * 1024;
pub(crate) const SCHEMA_LIMIT: usize = 1000;
pub(crate) const RELATION_LIMIT: usize = 10_000;
pub(crate) const RELATION_MAX_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const COLUMN_LIMIT: usize = 1000;
pub(crate) const COLUMN_MAX_BYTES: usize = 1024 * 1024;
pub(crate) const STANDARD_QUERY_MAX_ROWS: u64 = 1_000_000;
pub(crate) const STANDARD_QUERY_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum ReadKind {
    Context,
    QuotedIdentifierPolicy,
    Autocommit,
    AbortOnError,
    CurrentTransaction,
    Version,
    Schemas,
    Relations,
    ExactRelation,
    Definition,
    Constraints,
    CheckConstraints,
    PrimaryKeyMapping,
    UniqueKeyMapping,
    ForeignKeyMapping,
    Columns,
    Describe,
    ZeroRowResolution,
}

/// An immutable production read. Only closed factories below construct it;
/// callers cannot supply SQL, binds, or independently chosen retention bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadSpec {
    kind: ReadKind,
    sql: String,
    binds: Vec<serde_json::Value>,
    max_rows: u64,
    max_bytes: u64,
}

impl ReadSpec {
    fn bounded(kind: ReadKind, sql: impl Into<String>, rows: u64, bytes: u64) -> Self {
        Self {
            kind,
            sql: sql.into(),
            binds: vec![],
            max_rows: rows,
            max_bytes: bytes,
        }
    }
    #[cfg(feature = "test-harness")]
    pub(crate) fn kind(&self) -> ReadKind {
        self.kind
    }
    pub(crate) fn sql(&self) -> &str {
        &self.sql
    }
    pub(crate) fn max_rows(&self) -> u64 {
        self.max_rows
    }
    pub(crate) fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    pub(crate) fn binds(&self) -> &[serde_json::Value] {
        &self.binds
    }
}

pub(crate) fn context_reads() -> [ReadSpec; 2] {
    [
        ReadSpec::bounded(ReadKind::Context, CONTEXT_SQL, 1, CONTEXT_MAX_BYTES as u64),
        ReadSpec::bounded(
            ReadKind::QuotedIdentifierPolicy,
            QUOTED_IDENTIFIER_POLICY_SQL,
            1,
            CONTEXT_MAX_BYTES as u64,
        ),
    ]
}

pub(crate) fn sample_reads() -> [ReadSpec; 5] {
    use super::snowflake_session as session;
    let bytes = session::OBSERVATION_MAX_BYTES as u64;
    [
        ReadSpec::bounded(ReadKind::Context, CONTEXT_SQL, 1, bytes),
        ReadSpec::bounded(
            ReadKind::QuotedIdentifierPolicy,
            QUOTED_IDENTIFIER_POLICY_SQL,
            1,
            bytes,
        ),
        ReadSpec::bounded(ReadKind::Autocommit, session::AUTOCOMMIT_SQL, 1, bytes),
        ReadSpec::bounded(
            ReadKind::AbortOnError,
            session::ABORT_ON_ERROR_SQL,
            1,
            bytes,
        ),
        ReadSpec::bounded(
            ReadKind::CurrentTransaction,
            session::CURRENT_TRANSACTION_SQL,
            1,
            bytes,
        ),
    ]
}

pub(crate) fn version_read() -> ReadSpec {
    ReadSpec::bounded(
        ReadKind::Version,
        super::snowflake_sql_contract::VERSION_SQL,
        STANDARD_QUERY_MAX_ROWS,
        STANDARD_QUERY_MAX_BYTES,
    )
}

pub(crate) fn schemas_read(context: &LiveContext) -> AppResult<ReadSpec> {
    use super::snowflake_browsing as browsing;
    Ok(ReadSpec::bounded(
        ReadKind::Schemas,
        browsing::schemas_sql(context)?,
        (browsing::SCHEMA_LIMIT + 1) as u64,
        browsing::CATALOG_MAX_BYTES as u64,
    ))
}

pub(crate) fn relations_read(context: &LiveContext, schema: &str) -> AppResult<ReadSpec> {
    use super::snowflake_browsing as browsing;
    Ok(ReadSpec::bounded(
        ReadKind::Relations,
        browsing::relations_sql(context, schema)?,
        (browsing::RELATION_LIMIT + 1) as u64,
        browsing::CATALOG_MAX_BYTES as u64,
    ))
}

pub(crate) fn exact_relation_read(
    context: &LiveContext,
    target: &super::snowflake_browsing::BrowseRelation,
) -> AppResult<ReadSpec> {
    use super::snowflake_browsing as browsing;
    Ok(ReadSpec::bounded(
        ReadKind::ExactRelation,
        browsing::relation_sql(context, target)?,
        3,
        browsing::CATALOG_MAX_BYTES as u64,
    ))
}

pub(crate) fn definition_read(
    target: &super::snowflake_browsing::BrowseRelation,
) -> AppResult<ReadSpec> {
    use super::snowflake_browsing as browsing;
    Ok(ReadSpec::bounded(
        ReadKind::Definition,
        browsing::definition_sql(target)?,
        1,
        browsing::DEFINITION_MAX_BYTES as u64,
    ))
}

pub(crate) fn constraints_read(
    target: &super::snowflake_browsing::BrowseRelation,
) -> AppResult<ReadSpec> {
    use super::snowflake_browsing as browsing;
    let mut read = ReadSpec::bounded(
        ReadKind::Constraints,
        browsing::constraints_sql(target)?,
        (browsing::CONSTRAINT_LIMIT + 1) as u64,
        browsing::CONSTRAINT_MAX_BYTES as u64,
    );
    read.binds = vec![
        target.database.clone().into(),
        target.table.schema.clone().into(),
        target.table.name.clone().into(),
    ];
    Ok(read)
}

pub(crate) fn check_constraints_read(
    target: &super::snowflake_browsing::BrowseRelation,
) -> AppResult<ReadSpec> {
    use super::snowflake_browsing as browsing;
    let mut read = ReadSpec::bounded(
        ReadKind::CheckConstraints,
        browsing::check_constraints_sql(target)?,
        (browsing::CHECK_LIMIT + 1) as u64,
        browsing::CHECK_MAX_BYTES as u64,
    );
    read.binds = vec![
        target.database.clone().into(),
        target.table.schema.clone().into(),
        target.table.name.clone().into(),
    ];
    Ok(read)
}

pub(crate) fn mapping_reads(
    target: &super::snowflake_browsing::BrowseRelation,
) -> AppResult<[ReadSpec; 3]> {
    use super::snowflake_browsing as browsing;
    use crate::relation_inspection::RelationKeyConstraintKind as Kind;
    let make = |kind, tag| -> AppResult<ReadSpec> {
        Ok(ReadSpec::bounded(
            tag,
            browsing::mapping_sql(target, kind)?,
            (browsing::CONSTRAINT_LIMIT + 1) as u64,
            browsing::CONSTRAINT_MAX_BYTES as u64,
        ))
    };
    Ok([
        make(Kind::PrimaryKey, ReadKind::PrimaryKeyMapping)?,
        make(Kind::Unique, ReadKind::UniqueKeyMapping)?,
        make(Kind::ForeignKey, ReadKind::ForeignKeyMapping)?,
    ])
}

pub(crate) fn inspection_reads(
    target: &super::snowflake_browsing::BrowseRelation,
) -> AppResult<[ReadSpec; 3]> {
    use super::snowflake_browsing as browsing;
    let rows = (browsing::COLUMN_LIMIT + 1) as u64;
    let bytes = browsing::COLUMN_MAX_BYTES as u64;
    Ok([
        ReadSpec::bounded(
            ReadKind::Columns,
            browsing::columns_sql(target)?,
            rows,
            bytes,
        ),
        ReadSpec::bounded(
            ReadKind::Describe,
            browsing::describe_sql(target)?,
            rows,
            bytes,
        ),
        ReadSpec::bounded(
            ReadKind::ZeroRowResolution,
            browsing::resolved_columns_sql(target)?,
            0,
            bytes,
        ),
    ])
}

// CURRENT_SESSION is VARCHAR, not a numeric identity. CURRENT_SECONDARY_ROLES
// returns JSON with string fields `roles` and `value`; do not split their commas.
// https://docs.snowflake.com/en/sql-reference/functions/current_session
// https://docs.snowflake.com/en/sql-reference/functions/current_secondary_roles
// Normalize search-path output to text without TO_JSON's VARIANT-only input
// contract. TO_VARCHAR preserves string text and renders ARRAY values as JSON.
// https://docs.snowflake.com/en/sql-reference/functions/to_char
pub(crate) const CONTEXT_SQL: &str = "SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES";
pub(crate) const QUOTED_IDENTIFIER_POLICY_SQL: &str =
    "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION";
const CONTEXT_COLUMNS: [&str; 8] = [
    "PRINCIPAL",
    "ROLE_NAME",
    "DATABASE_NAME",
    "SCHEMA_NAME",
    "WAREHOUSE_NAME",
    "SESSION_ID",
    "SEARCH_PATH",
    "SECONDARY_ROLES",
];
const SCHEMA_COLUMNS: [&str; 2] = ["CATALOG_NAME", "SCHEMA_NAME"];
const RELATION_COLUMNS: [&str; 10] = [
    "TABLE_CATALOG",
    "TABLE_SCHEMA",
    "TABLE_NAME",
    "TABLE_TYPE",
    "IS_TRANSIENT",
    "IS_TEMPORARY",
    "IS_ICEBERG",
    "IS_DYNAMIC",
    "IS_HYBRID",
    "IS_IMMUTABLE",
];

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MetadataResult {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<TextRow>,
    /// Total rows reported by the terminal stream summary, including discarded
    /// rows. A collector must not replace this with its retained row count.
    pub rows_seen: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SecondaryRoles {
    pub roles: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ResolvedSchema {
    pub raw: String,
    // Unrecognized/ambiguous spelling is retained in the fingerprint but never
    // authorizes an unqualified suggestion.
    pub database: Option<String>,
    pub schema: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LiveContext {
    pub principal: String,
    pub role: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub warehouse: Option<String>,
    pub session_id: String,
    pub search_path: Vec<ResolvedSchema>,
    pub secondary_roles: SecondaryRoles,
    pub quoted_identifiers_ignore_case: bool,
}

impl LiveContext {
    pub(crate) fn fingerprint(&self) -> AppResult<String> {
        // Struct serialization fixes key order; options retain null vs present,
        // and ordered search-path entries are not sorted or deduplicated.
        let canonical = serde_json::to_vec(self).map_err(|_| malformed())?;
        if canonical.len() > CONTEXT_MAX_BYTES {
            return Err(limit());
        }
        Ok(format!("{:x}", Sha256::digest(canonical)))
    }

    pub(crate) fn completion_identity(&self) -> AppResult<CompletionIdentity> {
        require_supported_policy(self)?;
        Ok(CompletionIdentity {
            database: current_database(self)?.to_owned(),
            role: self.role.clone().unwrap_or_default(),
            search_path: self
                .search_path
                .iter()
                .map(|entry| entry.raw.clone())
                .collect(),
            case_sensitive_table_names: Some(true),
            context_fingerprint: Some(self.fingerprint()?),
        })
    }
}

pub(crate) fn parse_context(
    result: &MetadataResult,
    policy: &MetadataResult,
) -> AppResult<LiveContext> {
    exact_columns(result, &CONTEXT_COLUMNS, 1, CONTEXT_MAX_BYTES)?;
    let row = only_row(result)?;
    let principal = required_identifier(&row[0])?.to_owned();
    let role = optional_identifier(&row[1])?;
    let database = optional_identifier(&row[2])?;
    let schema = optional_identifier(&row[3])?;
    let warehouse = optional_identifier(&row[4])?;
    let session_id = bounded_text(required(&row[5])?, 1024, false)?.to_owned();
    let path_json = required(&row[6])?;
    let paths: Vec<String> = serde_json::from_str(path_json).map_err(|_| malformed())?;
    if paths.len() > 64 {
        return Err(limit());
    }
    let search_path = paths
        .into_iter()
        .map(|raw| {
            bounded_text(&raw, 1024, false)?;
            let parsed = parse_resolved_path(&raw);
            Ok(ResolvedSchema {
                raw,
                database: parsed.as_ref().map(|(database, _)| database.clone()),
                schema: parsed.map(|(_, schema)| schema),
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let secondary_roles: SecondaryRoles =
        serde_json::from_str(required(&row[7])?).map_err(|_| malformed())?;
    bounded_text(&secondary_roles.roles, 16 * 1024, true)?;
    bounded_text(&secondary_roles.value, 16 * 1024, true)?;
    let context = LiveContext {
        principal,
        role,
        database,
        schema,
        warehouse,
        session_id,
        search_path,
        secondary_roles,
        quoted_identifiers_ignore_case: parse_policy(policy)?,
    };
    require_supported_policy(&context)?;
    context.fingerprint()?;
    Ok(context)
}

fn parse_policy(result: &MetadataResult) -> AppResult<bool> {
    validate_result(result, 1, CONTEXT_MAX_BYTES)?;
    let row = only_row(result)?;
    let find = |name: &str| -> AppResult<usize> {
        let mut indices = result
            .columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.name.eq_ignore_ascii_case(name));
        let (index, column) = indices.next().ok_or_else(malformed)?;
        if indices.next().is_some() || column.kind != ColumnKind::Text {
            return Err(malformed());
        }
        Ok(index)
    };
    let key = find("key")?;
    let value = find("value")?;
    if required(&row[key])? != "QUOTED_IDENTIFIERS_IGNORE_CASE" {
        return Err(malformed());
    }
    match required(&row[value])? {
        value if value.eq_ignore_ascii_case("true") => Ok(true),
        value if value.eq_ignore_ascii_case("false") => Ok(false),
        _ => Err(malformed()),
    }
}

/// One component, never a caller-supplied SQL expression or dotted path.
pub(crate) fn quote_identifier(value: &str) -> AppResult<String> {
    validate_identifier(value)?;
    Ok(format!("\"{}\"", value.replace('"', "\"\"")))
}

pub(crate) fn schemas_sql(context: &LiveContext) -> AppResult<String> {
    let database = current_database(context)?;
    Ok(format!(
        "SELECT CATALOG_NAME, SCHEMA_NAME FROM {}.INFORMATION_SCHEMA.SCHEMATA WHERE CATALOG_NAME = {} AND SCHEMA_NAME <> 'INFORMATION_SCHEMA' ORDER BY SCHEMA_NAME LIMIT {}",
        quote_identifier(database)?,
        sql_literal(database)?,
        SCHEMA_LIMIT + 1,
    ))
}

pub(crate) fn parse_schemas(
    context: &LiveContext,
    result: &MetadataResult,
) -> AppResult<Vec<String>> {
    let database = current_database(context)?;
    exact_columns(result, &SCHEMA_COLUMNS, SCHEMA_LIMIT, RELATION_MAX_BYTES)?;
    let mut schemas = Vec::with_capacity(result.rows.len());
    let mut names = HashSet::new();
    for row in &result.rows {
        if required_identifier(&row[0])? != database {
            return Err(malformed());
        }
        let schema = required_identifier(&row[1])?;
        if schema == "INFORMATION_SCHEMA" || !names.insert(schema) {
            return Err(malformed());
        }
        schemas.push(schema.to_owned());
    }
    schemas.sort();
    Ok(schemas)
}

pub(crate) fn relations_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let database = current_database(context)?;
    validate_identifier(schema)?;
    if schema == "INFORMATION_SCHEMA" {
        return Err(AppError::invalid(
            "System schemas are outside completion scope",
        ));
    }
    // TABLES' documented flags preserve distinctions TABLE_TYPE alone loses.
    // A per-schema predicate supplies selectivity; LIMIT alone does not.
    // https://docs.snowflake.com/en/sql-reference/info-schema/tables
    Ok(format!(
        "SELECT {} FROM {}.INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = {} AND TABLE_SCHEMA = {} ORDER BY TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, IS_ICEBERG, IS_DYNAMIC, IS_HYBRID, IS_IMMUTABLE LIMIT {}",
        RELATION_COLUMNS.join(", "),
        quote_identifier(database)?,
        sql_literal(database)?,
        sql_literal(schema)?,
        RELATION_LIMIT + 1,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Relation {
    pub database: String,
    pub table: TableRef,
    pub table_type: String,
    pub transient: Option<bool>,
    pub temporary: Option<bool>,
    pub iceberg: Option<bool>,
    pub dynamic: Option<bool>,
    pub hybrid: Option<bool>,
    pub immutable: Option<bool>,
}

pub(crate) fn parse_relations(
    context: &LiveContext,
    schema: &str,
    result: &MetadataResult,
) -> AppResult<Vec<Relation>> {
    let database = current_database(context)?;
    validate_identifier(schema)?;
    exact_columns(
        result,
        &RELATION_COLUMNS,
        RELATION_LIMIT,
        RELATION_MAX_BYTES,
    )?;
    let mut relations = Vec::with_capacity(result.rows.len());
    let mut names = HashSet::new();
    for row in &result.rows {
        if required_identifier(&row[0])? != database
            || required_identifier(&row[1])? != schema
            || schema == "INFORMATION_SCHEMA"
        {
            return Err(malformed());
        }
        let name = required_identifier(&row[2])?;
        if !names.insert(name) {
            // Temporary and permanent relations can share a fully qualified
            // spelling. Visibility cannot pick an authoritative object target.
            return Err(ambiguous());
        }
        relations.push(Relation {
            database: database.to_owned(),
            table: TableRef {
                schema: schema.to_owned(),
                name: name.to_owned(),
            },
            table_type: bounded_text(required(&row[3])?, 128, false)?.to_owned(),
            transient: yes_no(&row[4])?,
            temporary: yes_no(&row[5])?,
            iceberg: yes_no(&row[6])?,
            dynamic: yes_no(&row[7])?,
            hybrid: yes_no(&row[8])?,
            immutable: yes_no(&row[9])?,
        });
    }
    Ok(relations)
}

pub(crate) fn completion_relations(
    context: &LiveContext,
    mut relations: Vec<Relation>,
) -> AppResult<CompletionRelations> {
    let database = current_database(context)?;
    if relations.len() > RELATION_LIMIT {
        return Err(limit());
    }
    bounded_json(&relations, RELATION_MAX_BYTES)?;
    relations
        .sort_by(|a, b| (&a.table.schema, &a.table.name).cmp(&(&b.table.schema, &b.table.name)));
    let first = context
        .search_path
        .first()
        .filter(|entry| entry.database.as_deref() == Some(database))
        .and_then(|entry| entry.schema.as_deref());
    let mut names = HashSet::new();
    let mut tables = Vec::with_capacity(relations.len());
    let mut unqualified_tables = Vec::new();
    for relation in relations {
        validate_identifier(&relation.table.schema)?;
        validate_identifier(&relation.table.name)?;
        bounded_text(&relation.table_type, 128, false)?;
        if relation.database != database || relation.table.schema == "INFORMATION_SCHEMA" {
            return Err(malformed());
        }
        if !names.insert(relation.table.clone()) {
            return Err(ambiguous());
        }
        if first == Some(relation.table.schema.as_str()) {
            unqualified_tables.push(relation.table.clone());
        }
        let flag = |value: Option<bool>| match value {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        };
        tables.push(TableInfo {
            table: relation.table,
            // The shared DTO has only a kind string; retain all distinctions in
            // that string so flags also participate in revision comparisons.
            kind: format!(
                "{}; transient={}; temporary={}; iceberg={}; dynamic={}; hybrid={}; immutable={}",
                relation.table_type,
                flag(relation.transient),
                flag(relation.temporary),
                flag(relation.iceberg),
                flag(relation.dynamic),
                flag(relation.hybrid),
                flag(relation.immutable)
            ),
            estimated_rows: None,
            total_bytes: None,
        });
    }
    bounded_json(&(&tables, &unqualified_tables), RELATION_MAX_BYTES)?;
    Ok(CompletionRelations {
        tables,
        unqualified_tables,
    })
}

pub(crate) fn resolved_columns_sql(context: &LiveContext, table: &TableRef) -> AppResult<String> {
    Ok(format!(
        "SELECT * FROM {}.{}.{} LIMIT 0",
        quote_identifier(current_database(context)?)?,
        quote_identifier(&table.schema)?,
        quote_identifier(&table.name)?
    ))
}

/// Column names/types come from the relation actually resolved by this session.
/// Other fields are completion placeholders, never table-inspection evidence.
pub(crate) fn parse_resolved_columns(
    table: &TableRef,
    result: MetadataResult,
) -> AppResult<TableSchema> {
    validate_identifier(&table.schema)?;
    validate_identifier(&table.name)?;
    validate_result(&result, 0, COLUMN_MAX_BYTES)?;
    if result.columns.is_empty() {
        return Err(malformed());
    }
    let mut names = HashSet::new();
    let mut columns = result.columns;
    for (index, column) in columns.iter_mut().enumerate() {
        validate_identifier(&column.name).map_err(|_| malformed())?;
        bounded_text(&column.database_type, 1024, false)?;
        if !names.insert(column.name.clone()) {
            return Err(ambiguous());
        }
        column.id = format!("c{index}");
        column.nullable = true;
        column.primary_key = false;
        column.foreign_key = false;
        column.editable = false;
        column.insertable = false;
        column.has_default = false;
    }
    let schema = TableSchema {
        table: table.clone(),
        columns,
        primary_key: Vec::new(),
        foreign_keys: Vec::new(),
        inbound_foreign_keys: Vec::new(),
        editable: false,
    };
    bounded_json(&schema, COLUMN_MAX_BYTES)?;
    Ok(schema)
}

fn current_database(context: &LiveContext) -> AppResult<&str> {
    require_supported_policy(context)?;
    let database = context.database.as_deref().ok_or_else(|| {
        AppError::new(
            ErrorCode::Unsupported,
            "Snowflake completion requires an observed current database",
        )
    })?;
    validate_identifier(database)?;
    Ok(database)
}

fn require_supported_policy(context: &LiveContext) -> AppResult<()> {
    if context.quoted_identifiers_ignore_case {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Snowflake completion requires QUOTED_IDENTIFIERS_IGNORE_CASE=FALSE",
        ));
    }
    Ok(())
}

fn validate_identifier(value: &str) -> AppResult<()> {
    if value.is_empty() || value.chars().count() > 255 || value.contains('\0') {
        return Err(AppError::invalid("Invalid Snowflake identifier"));
    }
    Ok(())
}

fn sql_literal(value: &str) -> AppResult<String> {
    validate_identifier(value)?;
    // Snowflake single-quoted literals recognize backslash escapes as well as
    // doubled apostrophes. Escape backslashes before escaping apostrophes.
    Ok(format!(
        "'{}'",
        value.replace('\\', "\\\\").replace('\'', "''")
    ))
}

fn required(value: &Option<String>) -> AppResult<&str> {
    value.as_deref().ok_or_else(malformed)
}

fn required_identifier(value: &Option<String>) -> AppResult<&str> {
    let value = required(value)?;
    validate_identifier(value).map_err(|_| malformed())?;
    Ok(value)
}

fn optional_identifier(value: &Option<String>) -> AppResult<Option<String>> {
    value
        .as_ref()
        .map(|value| {
            validate_identifier(value).map_err(|_| malformed())?;
            Ok(value.clone())
        })
        .transpose()
}

fn bounded_text(value: &str, maximum: usize, empty_allowed: bool) -> AppResult<&str> {
    if value.len() > maximum {
        return Err(limit());
    }
    if (!empty_allowed && value.is_empty()) || value.contains('\0') {
        return Err(malformed());
    }
    Ok(value)
}

fn yes_no(value: &Option<String>) -> AppResult<Option<bool>> {
    match value.as_deref() {
        Some("YES") => Ok(Some(true)),
        Some("NO") => Ok(Some(false)),
        None => Ok(None),
        _ => Err(malformed()),
    }
}

fn only_row(result: &MetadataResult) -> AppResult<&TextRow> {
    if result.rows.len() != 1 {
        return Err(malformed());
    }
    Ok(&result.rows[0])
}

fn exact_columns(
    result: &MetadataResult,
    expected: &[&str],
    max_rows: usize,
    max_bytes: usize,
) -> AppResult<()> {
    validate_result(result, max_rows, max_bytes)?;
    if result.columns.len() != expected.len()
        || result
            .columns
            .iter()
            .zip(expected)
            .any(|(column, name)| column.name != *name || column.kind != ColumnKind::Text)
    {
        return Err(malformed());
    }
    Ok(())
}

fn validate_result(result: &MetadataResult, max_rows: usize, max_bytes: usize) -> AppResult<()> {
    if result.truncated
        || result.columns.len() > COLUMN_LIMIT
        || result.rows.len() > max_rows
        || result.rows_seen != result.rows.len() as u64
        || result
            .rows
            .iter()
            .any(|row| row.len() != result.columns.len())
    {
        return Err(limit());
    }
    bounded_json(result, max_bytes)
}

struct BudgetWriter {
    remaining: usize,
}
impl Write for BudgetWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.remaining = self
            .remaining
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("Snowflake metadata byte limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bounded_json(value: &impl Serialize, maximum: usize) -> AppResult<()> {
    serde_json::to_writer(BudgetWriter { remaining: maximum }, value).map_err(|_| limit())
}

fn parse_resolved_path(raw: &str) -> Option<(String, String)> {
    fn component(input: &str) -> Option<(String, &str)> {
        if let Some(mut tail) = input.strip_prefix('"') {
            let mut value = String::new();
            loop {
                let end = tail.find('"')?;
                value.push_str(&tail[..end]);
                tail = &tail[end + 1..];
                if let Some(rest) = tail.strip_prefix('"') {
                    value.push('"');
                    tail = rest;
                } else {
                    validate_identifier(&value).ok()?;
                    return Some((value, tail));
                }
            }
        }
        let end = input.find('.').unwrap_or(input.len());
        let value = &input[..end];
        let mut chars = value.chars();
        if !chars
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
            || !chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '$')
        {
            return None;
        }
        validate_identifier(value).ok()?;
        Some((value.to_owned(), &input[end..]))
    }
    let (database, tail) = component(raw)?;
    let (schema, tail) = component(tail.strip_prefix('.')?)?;
    if !tail.is_empty() {
        return None;
    }
    Some((database, schema))
}

fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Snowflake metadata is incomplete, malformed or exceeds its bounded limit",
    )
}
fn malformed() -> AppError {
    limit()
}
fn ambiguous() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake completion cannot disambiguate relations with the same qualified name",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str) -> ColumnInfo {
        ColumnInfo {
            id: "wire-id".into(),
            name: name.into(),
            database_type: "VARCHAR".into(),
            kind: ColumnKind::Text,
            nullable: false,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        }
    }

    fn result(names: &[&str], rows: Vec<Vec<Option<String>>>) -> MetadataResult {
        MetadataResult {
            columns: names.iter().map(|name| column(name)).collect(),
            rows_seen: rows.len() as u64,
            rows,
            truncated: false,
        }
    }

    fn text_row(values: &[&str]) -> TextRow {
        values.iter().map(|value| Some((*value).into())).collect()
    }

    fn context_result() -> MetadataResult {
        result(
            &CONTEXT_COLUMNS,
            vec![text_row(&[
                "PERSON_NAME",
                "ANALYST",
                "DB",
                "PUBLIC",
                "WH",
                "opaque-A7/session",
                r#"["DB.PUBLIC","DB.OTHER"]"#,
                r#"{"roles":"R1,R2","value":"ALL"}"#,
            ])],
        )
    }

    fn policy_result() -> MetadataResult {
        result(
            &["value", "level", "key"],
            vec![text_row(&[
                "false",
                "SESSION",
                "QUOTED_IDENTIFIERS_IGNORE_CASE",
            ])],
        )
    }

    fn context() -> LiveContext {
        parse_context(&context_result(), &policy_result()).unwrap()
    }

    fn relation_result(schema: &str, name: &str) -> MetadataResult {
        result(
            &RELATION_COLUMNS,
            vec![text_row(&[
                "DB",
                schema,
                name,
                "BASE TABLE",
                "NO",
                "NO",
                "NO",
                "NO",
                "NO",
                "NO",
            ])],
        )
    }

    fn relation(schema: &str, name: &str) -> Relation {
        parse_relations(&context(), schema, &relation_result(schema, name))
            .unwrap()
            .remove(0)
    }

    fn assert_code<T>(value: AppResult<T>, code: ErrorCode) {
        assert_eq!(value.err().expect("expected rejection").code, code);
    }

    #[test]
    fn context_observes_opaque_session_structured_roles_and_nullable_state() {
        let mut input = context_result();
        for index in [1, 2, 3, 4] {
            input.rows[0][index] = None;
        }
        let observed = parse_context(&input, &policy_result()).unwrap();
        assert_eq!(observed.principal, "PERSON_NAME");
        assert_eq!(observed.session_id, "opaque-A7/session");
        assert_eq!(observed.secondary_roles.roles, "R1,R2");
        assert_eq!(observed.secondary_roles.value, "ALL");
        assert_eq!(
            (
                observed.role.as_ref(),
                observed.database.as_ref(),
                observed.schema.as_ref(),
                observed.warehouse.as_ref()
            ),
            (None, None, None, None)
        );
        assert_code(observed.completion_identity(), ErrorCode::Unsupported);
        assert_code(schemas_sql(&observed), ErrorCode::Unsupported);
    }

    #[test]
    fn context_json_text_preserves_documents_without_double_decoding() {
        let mut input = context_result();
        input.rows[0][6] = Some(r#" [ "DB.PUBLIC", "\"D.B\".\"S\"\"Q\"" ] "#.into());
        input.rows[0][7] = Some(r#" { "value": "ALL", "roles": "" } "#.into());
        let observed = parse_context(&input, &policy_result()).unwrap();
        assert_eq!(observed.search_path[0].raw, "DB.PUBLIC");
        assert_eq!(observed.search_path[1].database.as_deref(), Some("D.B"));
        assert_eq!(observed.search_path[1].schema.as_deref(), Some("S\"Q"));
        assert_eq!(observed.secondary_roles.roles, "");
        assert_eq!(observed.secondary_roles.value, "ALL");
        // Text normalization must not invite a permissive second JSON decode.
        for invalid in [
            serde_json::to_string(r#"["DB.PUBLIC"]"#).unwrap(),
            "null".into(),
            "{}".into(),
            "[null]".into(),
            "[1]".into(),
        ] {
            input.rows[0][6] = Some(invalid);
            assert_code(
                parse_context(&input, &policy_result()),
                ErrorCode::ResultLimitReached,
            );
        }
    }

    #[test]
    fn fingerprint_tracks_every_context_field_without_ambiguous_concatenation() {
        let original = context();
        let fingerprint = original.fingerprint().unwrap();
        assert_eq!(fingerprint.len(), 64);
        let mut variants = Vec::new();
        let mut changed = original.clone();
        changed.principal.push('2');
        variants.push(changed);
        let mut changed = original.clone();
        changed.role = None;
        variants.push(changed);
        let mut changed = original.clone();
        changed.database = Some("OTHER".into());
        variants.push(changed);
        // An explicit search path can stay the same across a schema switch.
        let mut changed = original.clone();
        changed.schema = Some("OTHER".into());
        variants.push(changed);
        let mut changed = original.clone();
        changed.warehouse = None;
        variants.push(changed);
        let mut changed = original.clone();
        changed.session_id.push('2');
        variants.push(changed);
        let mut changed = original.clone();
        changed.search_path.reverse();
        variants.push(changed);
        let mut changed = original.clone();
        changed.search_path[0].schema = None;
        variants.push(changed);
        let mut changed = original.clone();
        changed.secondary_roles.roles.push('2');
        variants.push(changed);
        let mut changed = original.clone();
        changed.secondary_roles.value = "NONE".into();
        variants.push(changed);
        let mut changed = original.clone();
        changed.quoted_identifiers_ignore_case = true;
        variants.push(changed);
        for changed in variants {
            assert_ne!(changed.fingerprint().unwrap(), fingerprint);
        }
        let mut reordered = context_result();
        reordered.rows[0][7] = Some(r#"{ "value": "ALL", "roles": "R1,R2" }"#.into());
        assert_eq!(
            parse_context(&reordered, &policy_result())
                .unwrap()
                .fingerprint()
                .unwrap(),
            fingerprint
        );
        let identity = original.completion_identity().unwrap();
        assert_eq!(identity.role, "ANALYST");
        assert_eq!(identity.context_fingerprint, Some(fingerprint));
    }

    #[test]
    fn context_rejects_missing_or_mistyped_authoritative_fields() {
        for index in [0, 5, 6, 7] {
            let mut input = context_result();
            input.rows[0][index] = None;
            assert_code(
                parse_context(&input, &policy_result()),
                ErrorCode::ResultLimitReached,
            );
        }
        for invalid in [
            r#"{"roles":[],"value":"ALL"}"#,
            r#"{"roles":"","value":null}"#,
            r#"{"roles":"","value":"ALL","extra":1}"#,
            r#"{"roles":"","roles":"R","value":"ALL"}"#,
            r#"["R"]"#,
        ] {
            let mut input = context_result();
            input.rows[0][7] = Some(invalid.into());
            assert_code(
                parse_context(&input, &policy_result()),
                ErrorCode::ResultLimitReached,
            );
        }
        let mut input = context_result();
        input.columns[5].kind = ColumnKind::Integer;
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
        let mut input = context_result();
        input.columns[5].name = "CURRENT_SESSION()".into();
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
        let mut input = context_result();
        input.rows[0][2] = Some(String::new());
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
    }

    #[test]
    fn policy_requires_one_unique_typed_observation_and_rejects_case_folding() {
        let mut input = policy_result();
        input.rows[0][0] = Some("TRUE".into());
        assert_code(
            parse_context(&context_result(), &input),
            ErrorCode::Unsupported,
        );
        let mut input = policy_result();
        input.columns.push(column("value"));
        input.rows[0].push(Some("false".into()));
        assert_code(
            parse_context(&context_result(), &input),
            ErrorCode::ResultLimitReached,
        );
        let mut input = policy_result();
        input.rows[0][2] = Some("OTHER_PARAMETER".into());
        assert_code(
            parse_context(&context_result(), &input),
            ErrorCode::ResultLimitReached,
        );
        let mut input = policy_result();
        input.rows_seen = 2;
        assert_code(
            parse_context(&context_result(), &input),
            ErrorCode::ResultLimitReached,
        );
    }

    #[test]
    fn context_path_and_byte_limits_fail_closed() {
        let mut input = context_result();
        input.rows[0][6] = Some(serde_json::to_string(&vec!["DB.PUBLIC"; 64]).unwrap());
        assert_eq!(
            parse_context(&input, &policy_result())
                .unwrap()
                .search_path
                .len(),
            64
        );
        input.rows[0][6] = Some(serde_json::to_string(&vec!["DB.PUBLIC"; 65]).unwrap());
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
        input.rows[0][6] = Some(serde_json::to_string(&vec!["x".repeat(1025)]).unwrap());
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
        let mut input = context_result();
        input.rows[0][5] = Some("s".repeat(1025));
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
        let mut input = context_result();
        input.rows[0][7] = Some(" ".repeat(CONTEXT_MAX_BYTES));
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
        let mut input = context_result();
        input.truncated = true;
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
        let mut input = context_result();
        input.rows_seen = 2;
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
        let mut input = context_result();
        input.rows[0].pop();
        assert_code(
            parse_context(&input, &policy_result()),
            ErrorCode::ResultLimitReached,
        );
    }

    #[test]
    fn identifier_components_preserve_quotes_dots_unicode_and_sql_punctuation() {
        assert_eq!(quote_identifier("My.DB").unwrap(), "\"My.DB\"");
        assert_eq!(
            quote_identifier("a\"; SELECT 9 --").unwrap(),
            "\"a\"\"; SELECT 9 --\""
        );
        assert!(quote_identifier(&"界".repeat(255)).is_ok());
        for invalid in [String::new(), "x".repeat(256), "bad\0name".into()] {
            assert_code(quote_identifier(&invalid), ErrorCode::InvalidInput);
        }
        let mut observed = context();
        observed.database = Some("My.DB".into());
        let table = TableRef {
            schema: "My.Schema".into(),
            name: "Table\".1".into(),
        };
        assert_eq!(
            resolved_columns_sql(&observed, &table).unwrap(),
            "SELECT * FROM \"My.DB\".\"My.Schema\".\"Table\"\".1\" LIMIT 0"
        );
    }

    #[test]
    fn metadata_sql_uses_current_database_selective_schema_and_overflow_sentinels() {
        assert_eq!(
            schemas_sql(&context()).unwrap(),
            "SELECT CATALOG_NAME, SCHEMA_NAME FROM \"DB\".INFORMATION_SCHEMA.SCHEMATA WHERE CATALOG_NAME = 'DB' AND SCHEMA_NAME <> 'INFORMATION_SCHEMA' ORDER BY SCHEMA_NAME LIMIT 1001"
        );
        let sql = relations_sql(&context(), "PUBLIC").unwrap();
        assert!(sql.starts_with("SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, IS_ICEBERG, IS_DYNAMIC, IS_HYBRID, IS_IMMUTABLE FROM \"DB\".INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = 'DB' AND TABLE_SCHEMA = 'PUBLIC' ORDER BY "));
        assert!(sql.ends_with("IS_IMMUTABLE LIMIT 10001"));
        // Both literal escaping mechanisms matter: a single escaped apostrophe
        // must not terminate the schema predicate or change its target value.
        assert_eq!(
            sql_literal("x\\'; SELECT 9 --").unwrap(),
            "'x\\\\''; SELECT 9 --'"
        );
        assert!(
            relations_sql(&context(), "x\\'; SELECT 9 --")
                .unwrap()
                .contains("TABLE_SCHEMA = 'x\\\\''; SELECT 9 --' ORDER BY")
        );
        assert_code(
            relations_sql(&context(), "INFORMATION_SCHEMA"),
            ErrorCode::InvalidInput,
        );
    }

    #[test]
    fn schema_parser_requires_current_database_unique_complete_names() {
        let input = result(
            &SCHEMA_COLUMNS,
            vec![text_row(&["DB", "Z"]), text_row(&["DB", "A"])],
        );
        assert_eq!(parse_schemas(&context(), &input).unwrap(), vec!["A", "Z"]);
        for rows in [
            vec![text_row(&["OTHER", "A"])],
            vec![text_row(&["DB", "INFORMATION_SCHEMA"])],
            vec![text_row(&["DB", "A"]), text_row(&["DB", "A"])],
        ] {
            assert_code(
                parse_schemas(&context(), &result(&SCHEMA_COLUMNS, rows)),
                ErrorCode::ResultLimitReached,
            );
        }
        let overflow = result(
            &SCHEMA_COLUMNS,
            (0..=SCHEMA_LIMIT)
                .map(|index| text_row(&["DB", &format!("S{index}")]))
                .collect(),
        );
        assert_code(
            parse_schemas(&context(), &overflow),
            ErrorCode::ResultLimitReached,
        );
        assert!(
            parse_schemas(&context(), &result(&SCHEMA_COLUMNS, Vec::new()))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn relation_parser_preserves_documented_flags_and_unknown_values() {
        let mut input = relation_result("PUBLIC", "T");
        input.rows[0][4] = Some("YES".into());
        input.rows[0][6] = Some("YES".into());
        input.rows[0][7] = Some("YES".into());
        input.rows[0][8] = None;
        let rows = parse_relations(&context(), "PUBLIC", &input).unwrap();
        assert_eq!(
            (
                rows[0].transient,
                rows[0].temporary,
                rows[0].iceberg,
                rows[0].dynamic,
                rows[0].hybrid
            ),
            (Some(true), Some(false), Some(true), Some(true), None)
        );
        let completed = completion_relations(&context(), rows).unwrap();
        assert!(
            completed.tables[0]
                .kind
                .contains("iceberg=yes; dynamic=yes; hybrid=unknown")
        );
        input.rows[0][8] = Some("MAYBE".into());
        assert_code(
            parse_relations(&context(), "PUBLIC", &input),
            ErrorCode::ResultLimitReached,
        );
    }

    #[test]
    fn relations_reject_scope_mismatch_shape_truncation_and_hidden_discard() {
        let observed = context();
        let mut cases = Vec::new();
        let mut input = relation_result("PUBLIC", "T");
        input.rows[0][0] = Some("OTHER".into());
        cases.push(input);
        let mut input = relation_result("PUBLIC", "T");
        input.rows[0][1] = Some("OTHER".into());
        cases.push(input);
        let mut input = relation_result("PUBLIC", "T");
        input.rows[0].pop();
        cases.push(input);
        let mut input = relation_result("PUBLIC", "T");
        input.rows_seen = 2;
        cases.push(input);
        let mut input = relation_result("PUBLIC", "T");
        input.truncated = true;
        cases.push(input);
        let mut input = relation_result("PUBLIC", "T");
        input.columns[1].name = "SCHEMA".into();
        cases.push(input);
        let mut input = relation_result("PUBLIC", "T");
        input.columns[1].kind = ColumnKind::Boolean;
        cases.push(input);
        for input in cases {
            assert_code(
                parse_relations(&observed, "PUBLIC", &input),
                ErrorCode::ResultLimitReached,
            );
        }
        let mut overflow = relation_result("PUBLIC", "T");
        overflow.rows = (0..=RELATION_LIMIT)
            .map(|_| overflow.rows[0].clone())
            .collect();
        overflow.rows_seen = overflow.rows.len() as u64;
        assert_code(
            parse_relations(&observed, "PUBLIC", &overflow),
            ErrorCode::ResultLimitReached,
        );
    }

    #[test]
    fn duplicate_shadowing_relations_never_collapse_into_one_target() {
        let mut input = relation_result("PUBLIC", "T");
        let mut temporary = input.rows[0].clone();
        temporary[3] = Some("TEMPORARY TABLE".into());
        temporary[5] = Some("YES".into());
        input.rows.push(temporary);
        input.rows_seen = 2;
        assert_code(
            parse_relations(&context(), "PUBLIC", &input),
            ErrorCode::Conflict,
        );
        assert_code(
            completion_relations(
                &context(),
                vec![relation("PUBLIC", "T"), relation("PUBLIC", "T")],
            ),
            ErrorCode::Conflict,
        );
    }

    #[test]
    fn only_first_proven_resolved_schema_gets_unqualified_suggestions() {
        let rows = vec![relation("OTHER", "SAME"), relation("PUBLIC", "SAME")];
        let completed = completion_relations(&context(), rows.clone()).unwrap();
        assert_eq!(
            completed.unqualified_tables,
            vec![TableRef {
                schema: "PUBLIC".into(),
                name: "SAME".into()
            }]
        );
        assert_eq!(completed.tables[0].table.schema, "OTHER");
        for path in [
            r#"["OTHER_DB.PUBLIC","DB.PUBLIC"]"#,
            r#"["DB.PART.ONE","DB.PUBLIC"]"#,
            r#"["db.public","DB.PUBLIC"]"#,
            r#"[]"#,
        ] {
            let mut input = context_result();
            input.rows[0][6] = Some(path.into());
            let observed = parse_context(&input, &policy_result()).unwrap();
            assert!(
                completion_relations(&observed, rows.clone())
                    .unwrap()
                    .unqualified_tables
                    .is_empty()
            );
        }
        let mut input = context_result();
        input.rows[0][2] = Some("My.DB".into());
        input.rows[0][6] = Some(r#"["\"My.DB\".\"My.Schema\""]"#.into());
        let observed = parse_context(&input, &policy_result()).unwrap();
        let mut row = relation("My.Schema", "T");
        row.database = "My.DB".into();
        assert_eq!(
            completion_relations(&observed, vec![row])
                .unwrap()
                .unqualified_tables
                .len(),
            1
        );
        assert_eq!(
            parse_resolved_path("\"a\"\"b\".\"c.d\""),
            Some(("a\"b".into(), "c.d".into()))
        );
    }

    #[test]
    fn aggregate_relations_enforce_count_and_actual_serialized_snapshot_bytes() {
        let row = relation("PUBLIC", "T");
        assert_code(
            completion_relations(&context(), vec![row.clone(); RELATION_LIMIT + 1]),
            ErrorCode::ResultLimitReached,
        );
        let wide: Vec<_> = (0..4000)
            .map(|index| {
                let mut row = row.clone();
                row.table.schema = "s".repeat(255);
                row.table.name = format!("{index:05}{}", "t".repeat(250));
                row
            })
            .collect();
        assert_code(
            completion_relations(&context(), wide),
            ErrorCode::ResultLimitReached,
        );
        let mut foreign = row.clone();
        foreign.database = "OTHER".into();
        assert_code(
            completion_relations(&context(), vec![foreign]),
            ErrorCode::ResultLimitReached,
        );
    }

    #[test]
    fn lazy_columns_are_zero_row_resolved_completion_placeholders() {
        let table = TableRef {
            schema: "PUBLIC".into(),
            name: "T".into(),
        };
        let mut input = result(&["id", "Payload"], Vec::new());
        input.columns[0].kind = ColumnKind::Integer;
        input.columns[0].database_type = "FIXED".into();
        input.columns[0].editable = true;
        input.columns[0].insertable = true;
        input.columns[0].primary_key = true;
        input.columns[0].has_default = true;
        let schema = parse_resolved_columns(&table, input).unwrap();
        assert_eq!(schema.table, table);
        assert_eq!(schema.columns[0].kind, ColumnKind::Integer);
        assert_eq!(schema.columns[0].database_type, "FIXED");
        assert_eq!(schema.columns[0].id, "c0");
        assert!(schema.columns.iter().all(|column| column.nullable
            && !column.editable
            && !column.insertable
            && !column.primary_key
            && !column.foreign_key
            && !column.has_default));
        assert!(!schema.editable);
        assert!(
            schema.primary_key.is_empty()
                && schema.foreign_keys.is_empty()
                && schema.inbound_foreign_keys.is_empty()
        );
        assert_eq!(
            resolved_columns_sql(&context(), &table).unwrap(),
            "SELECT * FROM \"DB\".\"PUBLIC\".\"T\" LIMIT 0"
        );
    }

    #[test]
    fn lazy_columns_reject_rows_duplicates_incomplete_and_oversized_schemas() {
        let table = TableRef {
            schema: "PUBLIC".into(),
            name: "T".into(),
        };
        assert_code(
            parse_resolved_columns(&table, result(&[], Vec::new())),
            ErrorCode::ResultLimitReached,
        );
        assert_code(
            parse_resolved_columns(&table, result(&["X", "X"], Vec::new())),
            ErrorCode::Conflict,
        );
        assert_code(
            parse_resolved_columns(&table, result(&["X"], vec![text_row(&["unexpected"])])),
            ErrorCode::ResultLimitReached,
        );
        let mut input = result(&["X"], Vec::new());
        input.rows_seen = 1;
        assert_code(
            parse_resolved_columns(&table, input),
            ErrorCode::ResultLimitReached,
        );
        let mut input = result(&["X"], Vec::new());
        input.truncated = true;
        assert_code(
            parse_resolved_columns(&table, input),
            ErrorCode::ResultLimitReached,
        );
        let mut input = result(&[], Vec::new());
        input.columns = (0..=COLUMN_LIMIT)
            .map(|index| column(&format!("C{index}")))
            .collect();
        assert_code(
            parse_resolved_columns(&table, input),
            ErrorCode::ResultLimitReached,
        );
        let mut input = result(&[], Vec::new());
        input.columns = (0..COLUMN_LIMIT)
            .map(|index| {
                let mut column = column(&format!("C{index}"));
                column.database_type = "t".repeat(1024);
                column
            })
            .collect();
        assert_code(
            parse_resolved_columns(&table, input),
            ErrorCode::ResultLimitReached,
        );
    }
}
