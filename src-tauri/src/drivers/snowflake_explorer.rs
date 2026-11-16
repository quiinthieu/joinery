//! Pure, bounded current-database Snowflake Object Explorer & Inspector helpers.
//!
//! Callers own session lanes, ordinary-mode enforcement, and context-drift
//! bracketing. Every identifier is validated and double-quoted without trimming
//! or splitting embedded dots; string literals use closed single-quote escaping
//! after identifier validation.

use super::snowflake_browsing::{self as browsing, BrowseRelation};
use super::snowflake_metadata::{self as metadata, LiveContext, MetadataResult};
use crate::domain::{
    AppError, AppResult, ConstraintInfo, ConstraintKind, DatabaseObjectKind, DatabaseObjectRef,
    DatabaseObjectSummary, DependencyEdge, DiagramColumn, DiagramKeyRow, EngineKind, ErrorCode,
    ObjectDdlStatus, ObjectDetails, RoutineInfo, SchemaDiagram, SchemaSnapshot, SequenceInfo,
    SnapshotColumn, SnapshotRoutine, SnapshotSequence, SnapshotTable, SnowflakeOperatorProfile,
    SnowflakeOperatorStat, SnowflakeQueryHistoryEntry, SnowflakeTableSubtype, TableRef,
    TableStorageStats, UnsupportedSchemaObject,
};
use crate::relation_inspection::{
    RelationAvailability, RelationCheckConstraint, RelationConstraintMapping,
    RelationKeyConstraint, RelationKeyConstraintKind, RelationTraits,
};
use crate::schema_diff::{
    MAX_SNAPSHOT_COLUMNS, MAX_SNAPSHOT_CONSTRAINTS, MAX_SNAPSHOT_RELATIONS, MAX_SNAPSHOT_ROUTINES,
    MAX_SNAPSHOT_SEQUENCES, MAX_SNAPSHOT_UNSUPPORTED, quote_engine_identifier,
};
use sqlparser::{
    ast::{Visit, Visitor},
    dialect::SnowflakeDialect,
    parser::Parser,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::ControlFlow;

pub(crate) const OBJECT_LIMIT: usize = 10_000;
pub(crate) const OBJECT_MAX_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const DETAIL_MAX_BYTES: usize = 1024 * 1024;
pub(crate) const SCHEMA_RELATION_LIMIT: usize = 1000;
pub(crate) const SCHEMA_COLUMN_LIMIT: usize = 20_000;
pub(crate) const SCHEMA_KEY_LIMIT: usize = 20_000;
pub(crate) const SCHEMA_CONSTRAINT_LIMIT: usize = 10_000;
pub(crate) const SCHEMA_CHECK_LIMIT: usize = 5_000;
pub(crate) const SCHEMA_SNAPSHOT_DDL_LIMIT: usize = 200;
pub(crate) const SCHEMA_MAX_BYTES: usize = 8 * 1024 * 1024;

const EXPLORER_TABLE_COLUMNS: [&str; 7] = [
    "TABLE_CATALOG",
    "TABLE_SCHEMA",
    "TABLE_NAME",
    "TABLE_TYPE",
    "ROW_COUNT",
    "BYTES",
    "COMMENT",
];

const EXACT_TABLE_COLUMNS: [&str; 13] = [
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
    "ROW_COUNT",
    "BYTES",
    "COMMENT",
];

const SEQUENCE_COLUMNS: [&str; 10] = [
    "SEQUENCE_CATALOG",
    "SEQUENCE_SCHEMA",
    "SEQUENCE_NAME",
    "DATA_TYPE",
    "START_VALUE",
    "MINIMUM_VALUE",
    "MAXIMUM_VALUE",
    "INCREMENT_BY",
    "CYCLE_OPTION",
    "COMMENT",
];

const FUNCTION_COLUMNS: [&str; 8] = [
    "FUNCTION_CATALOG",
    "FUNCTION_SCHEMA",
    "FUNCTION_NAME",
    "ARGUMENT_SIGNATURE",
    "DATA_TYPE",
    "FUNCTION_LANGUAGE",
    "IS_VOLATILE",
    "COMMENT",
];

const PROCEDURE_COLUMNS: [&str; 7] = [
    "PROCEDURE_CATALOG",
    "PROCEDURE_SCHEMA",
    "PROCEDURE_NAME",
    "ARGUMENT_SIGNATURE",
    "DATA_TYPE",
    "PROCEDURE_LANGUAGE",
    "COMMENT",
];

fn malformed() -> AppError {
    AppError::internal("Invalid Snowflake object explorer metadata")
}

fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Snowflake object metadata exceeds the 10,000-object explorer limit",
    )
}

fn database(context: &LiveContext) -> AppResult<&str> {
    let db = context.database.as_deref().ok_or_else(|| {
        AppError::new(
            ErrorCode::Unsupported,
            "Select an active Snowflake database before exploring schema objects",
        )
    })?;
    metadata::quote_identifier(db)?;
    Ok(db)
}

fn user_schema(schema: &str) -> AppResult<()> {
    metadata::quote_identifier(schema)?;
    if schema.eq_ignore_ascii_case("INFORMATION_SCHEMA") {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "INFORMATION_SCHEMA cannot be inspected through Object Explorer",
        ));
    }
    Ok(())
}

fn literal(value: &str) -> AppResult<String> {
    if value.is_empty() || value.len() > 1024 || value.contains('\0') {
        return Err(malformed());
    }
    Ok(format!(
        "'{}'",
        value.replace('\\', "\\\\").replace('\'', "''")
    ))
}

fn check_columns(result: &MetadataResult, expected: &[&str], max_rows: usize) -> AppResult<()> {
    if result.truncated || result.rows_seen > max_rows as u64 || result.rows.len() > max_rows {
        return Err(limit());
    }
    if result.columns.len() != expected.len()
        || result
            .columns
            .iter()
            .zip(expected.iter())
            .any(|(col, exp)| col.name != *exp)
        || result.rows.iter().any(|row| row.len() != expected.len())
    {
        return Err(malformed());
    }
    Ok(())
}

fn required_cell(cell: &Option<String>) -> AppResult<&str> {
    let value = cell.as_deref().ok_or_else(malformed)?;
    if value.is_empty() || value.contains('\0') {
        return Err(malformed());
    }
    Ok(value)
}

fn optional_metric(cell: &Option<String>) -> AppResult<Option<f64>> {
    let Some(raw) = cell.as_deref() else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let parsed = raw.parse::<f64>().map_err(|_| malformed())?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(malformed());
    }
    Ok(Some(parsed))
}

fn yes_no(cell: &Option<String>) -> AppResult<Option<bool>> {
    match cell.as_deref() {
        Some("YES") => Ok(Some(true)),
        Some("NO") => Ok(Some(false)),
        None => Ok(None),
        _ => Err(malformed()),
    }
}

pub(crate) fn tables_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    Ok(format!(
        "SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, ROW_COUNT::VARCHAR AS ROW_COUNT, BYTES::VARCHAR AS BYTES, COMMENT FROM {}.INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = {} AND TABLE_SCHEMA = {} ORDER BY TABLE_NAME, TABLE_TYPE LIMIT {}",
        metadata::quote_identifier(db)?,
        literal(db)?,
        literal(schema)?,
        OBJECT_LIMIT + 1,
    ))
}

pub(crate) fn parse_tables(
    context: &LiveContext,
    schema: &str,
    result: &MetadataResult,
) -> AppResult<Vec<DatabaseObjectSummary>> {
    let db = database(context)?;
    user_schema(schema)?;
    check_columns(result, &EXPLORER_TABLE_COLUMNS, OBJECT_LIMIT)?;
    let mut out = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let name = required_cell(&row[2])?;
        metadata::quote_identifier(name)?;
        let table_type = required_cell(&row[3])?;
        let kind = match table_type {
            "MATERIALIZED VIEW" => DatabaseObjectKind::MaterializedView,
            "VIEW" => DatabaseObjectKind::View,
            "BASE TABLE" | "TEMPORARY TABLE" | "EXTERNAL TABLE" | "EVENT TABLE" => {
                DatabaseObjectKind::Table
            }
            _ => continue,
        };
        let signature = match table_type {
            "EXTERNAL TABLE" | "EVENT TABLE" => Some(table_type.to_owned()),
            _ => None,
        };
        let estimated_rows = optional_metric(&row[4])?;
        let total_bytes = optional_metric(&row[5])?;
        out.push(DatabaseObjectSummary {
            object: DatabaseObjectRef {
                schema: schema.to_owned(),
                name: name.to_owned(),
                kind,
                identity: None,
            },
            owner_table: None,
            signature,
            estimated_rows,
            total_bytes,
        });
    }
    Ok(out)
}

pub(crate) fn sequences_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    Ok(format!(
        "SELECT SEQUENCE_CATALOG, SEQUENCE_SCHEMA, SEQUENCE_NAME, DATA_TYPE, START_VALUE::VARCHAR AS START_VALUE, MINIMUM_VALUE::VARCHAR AS MINIMUM_VALUE, MAXIMUM_VALUE::VARCHAR AS MAXIMUM_VALUE, \"INCREMENT\"::VARCHAR AS INCREMENT_BY, CYCLE_OPTION, COMMENT FROM {}.INFORMATION_SCHEMA.SEQUENCES WHERE SEQUENCE_CATALOG = {} AND SEQUENCE_SCHEMA = {} ORDER BY SEQUENCE_NAME LIMIT {}",
        metadata::quote_identifier(db)?,
        literal(db)?,
        literal(schema)?,
        OBJECT_LIMIT + 1,
    ))
}

pub(crate) fn parse_sequences(
    context: &LiveContext,
    schema: &str,
    result: &MetadataResult,
) -> AppResult<Vec<DatabaseObjectSummary>> {
    let db = database(context)?;
    user_schema(schema)?;
    check_columns(result, &SEQUENCE_COLUMNS, OBJECT_LIMIT)?;
    let mut out = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let name = required_cell(&row[2])?;
        metadata::quote_identifier(name)?;
        out.push(DatabaseObjectSummary {
            object: DatabaseObjectRef {
                schema: schema.to_owned(),
                name: name.to_owned(),
                kind: DatabaseObjectKind::Sequence,
                identity: None,
            },
            owner_table: None,
            signature: None,
            estimated_rows: None,
            total_bytes: None,
        });
    }
    Ok(out)
}

pub(crate) fn functions_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    Ok(format!(
        "SELECT FUNCTION_CATALOG, FUNCTION_SCHEMA, FUNCTION_NAME, ARGUMENT_SIGNATURE, DATA_TYPE, FUNCTION_LANGUAGE, IS_VOLATILE, COMMENT FROM {}.INFORMATION_SCHEMA.FUNCTIONS WHERE FUNCTION_CATALOG = {} AND FUNCTION_SCHEMA = {} ORDER BY FUNCTION_NAME, ARGUMENT_SIGNATURE LIMIT {}",
        metadata::quote_identifier(db)?,
        literal(db)?,
        literal(schema)?,
        OBJECT_LIMIT + 1,
    ))
}

pub(crate) fn procedures_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    Ok(format!(
        "SELECT PROCEDURE_CATALOG, PROCEDURE_SCHEMA, PROCEDURE_NAME, ARGUMENT_SIGNATURE, DATA_TYPE, PROCEDURE_LANGUAGE, COMMENT FROM {}.INFORMATION_SCHEMA.PROCEDURES WHERE PROCEDURE_CATALOG = {} AND PROCEDURE_SCHEMA = {} ORDER BY PROCEDURE_NAME, ARGUMENT_SIGNATURE LIMIT {}",
        metadata::quote_identifier(db)?,
        literal(db)?,
        literal(schema)?,
        OBJECT_LIMIT + 1,
    ))
}

/// Extract canonical parameter type list (without parameter names or outer parens)
/// from Snowflake's `ARGUMENT_SIGNATURE` (e.g. `(X NUMBER, Y VARCHAR)` -> `NUMBER, VARCHAR`).
/// Rejects signatures containing quotes, semicolons, comments, or unbalanced parens.
pub(crate) fn normalize_argument_types(signature: &str) -> AppResult<String> {
    let trimmed = signature.trim();
    if trimmed.len() > 4096
        || !trimmed.is_ascii()
        || trimmed
            .bytes()
            .any(|b| b.is_ascii_control() || matches!(b, b'\'' | b'"' | b';' | b'-' | b'/' | b'\\'))
    {
        return Err(malformed());
    }
    let inner = trimmed
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(malformed)?
        .trim();
    if inner.is_empty() {
        return Ok(String::new());
    }
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, ch) in inner.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Err(malformed());
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                parts.push(&inner[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(malformed());
    }
    parts.push(&inner[start..]);
    let mut types = Vec::with_capacity(parts.len());
    for raw_param in parts {
        let param = raw_param.trim();
        if param.is_empty() {
            return Err(malformed());
        }
        // In Snowflake ARGUMENT_SIGNATURE, parameters are formatted as `NAME TYPE`
        // (or `TYPE` when unnamed). Split only at top-level whitespace outside `(...)`.
        let mut split_at = None;
        let mut p_depth = 0usize;
        for (idx, ch) in param.char_indices() {
            match ch {
                '(' => p_depth += 1,
                ')' => p_depth = p_depth.saturating_sub(1),
                c if c.is_ascii_whitespace() && p_depth == 0 => {
                    split_at = Some(idx);
                    break;
                }
                _ => {}
            }
        }
        let ty = match split_at {
            Some(idx) => {
                let candidate = param[idx..].trim();
                // Handle multi-word unnamed types like `TIMESTAMP_NTZ` vs `DOUBLE PRECISION`
                if param[..idx].eq_ignore_ascii_case("DOUBLE")
                    && candidate.eq_ignore_ascii_case("PRECISION")
                {
                    "FLOAT"
                } else if param[..idx].eq_ignore_ascii_case("CHARACTER")
                    && candidate.to_ascii_uppercase().starts_with("VARYING")
                {
                    param
                } else {
                    candidate
                }
            }
            None => param,
        };
        if ty.is_empty()
            || !ty
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'(' | b')' | b',' | b' '))
        {
            return Err(malformed());
        }
        types.push(ty.to_ascii_uppercase());
    }
    Ok(types.join(", "))
}

pub(crate) fn parse_routines(
    context: &LiveContext,
    schema: &str,
    is_procedure: bool,
    result: &MetadataResult,
) -> AppResult<Vec<DatabaseObjectSummary>> {
    let db = database(context)?;
    user_schema(schema)?;
    let expected: &[&str] = if is_procedure {
        &PROCEDURE_COLUMNS
    } else {
        &FUNCTION_COLUMNS
    };
    check_columns(result, expected, OBJECT_LIMIT)?;
    let mut out = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let name = required_cell(&row[2])?;
        metadata::quote_identifier(name)?;
        let arg_sig = required_cell(&row[3])?;
        let return_type = required_cell(&row[4])?;
        let identity_types = normalize_argument_types(arg_sig)?;
        out.push(DatabaseObjectSummary {
            object: DatabaseObjectRef {
                schema: schema.to_owned(),
                name: name.to_owned(),
                kind: if is_procedure {
                    DatabaseObjectKind::Procedure
                } else {
                    DatabaseObjectKind::Function
                },
                identity: Some(identity_types),
            },
            owner_table: None,
            signature: Some(format!("{arg_sig} -> {return_type}")),
            estimated_rows: None,
            total_bytes: None,
        });
    }
    Ok(out)
}

pub(crate) fn exact_table_sql(
    context: &LiveContext,
    schema: &str,
    name: &str,
) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    metadata::quote_identifier(name)?;
    Ok(format!(
        "SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, IS_ICEBERG, IS_DYNAMIC, IS_HYBRID, IS_IMMUTABLE, ROW_COUNT::VARCHAR AS ROW_COUNT, BYTES::VARCHAR AS BYTES, COMMENT FROM {}.INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = {} AND TABLE_SCHEMA = {} AND TABLE_NAME = {} ORDER BY TABLE_TYPE, IS_TEMPORARY DESC LIMIT 3",
        metadata::quote_identifier(db)?,
        literal(db)?,
        literal(schema)?,
        literal(name)?,
    ))
}

pub(crate) fn parse_exact_table(
    context: &LiveContext,
    schema: &str,
    name: &str,
    result: &MetadataResult,
) -> AppResult<(BrowseRelation, Option<String>, TableStorageStats)> {
    let db = database(context)?;
    user_schema(schema)?;
    metadata::quote_identifier(name)?;
    check_columns(result, &EXACT_TABLE_COLUMNS, 2)?;
    if result.rows.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The requested Snowflake relation is no longer visible in the current database",
        ));
    }
    if result.rows.len() > 1 {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "Multiple Snowflake relations share this name (such as a temporary shadow); use Relation Browser to inspect disambiguated targets",
        ));
    }
    let row = &result.rows[0];
    if required_cell(&row[0])? != db
        || required_cell(&row[1])? != schema
        || required_cell(&row[2])? != name
    {
        return Err(malformed());
    }
    let table_type = required_cell(&row[3])?.to_owned();
    let traits = RelationTraits {
        transient: yes_no(&row[4])?,
        temporary: yes_no(&row[5])?,
        iceberg: yes_no(&row[6])?,
        dynamic: yes_no(&row[7])?,
        hybrid: yes_no(&row[8])?,
        immutable: yes_no(&row[9])?,
    };
    let estimated_rows = optional_metric(&row[10])?;
    let total_bytes = optional_metric(&row[11])?;
    let comment = row[12].clone().filter(|s| !s.is_empty());
    let relation = BrowseRelation {
        database: db.to_owned(),
        table: TableRef {
            schema: schema.to_owned(),
            name: name.to_owned(),
        },
        table_type,
        traits,
        availability: RelationAvailability::Available,
        created: None,
        last_ddl: None,
    };
    let storage = TableStorageStats {
        estimated_rows,
        table_bytes: total_bytes,
        total_bytes,
        ..TableStorageStats::default()
    };
    Ok((relation, comment, storage))
}

pub(crate) fn exact_sequence_sql(
    context: &LiveContext,
    schema: &str,
    name: &str,
) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    metadata::quote_identifier(name)?;
    Ok(format!(
        "SELECT SEQUENCE_CATALOG, SEQUENCE_SCHEMA, SEQUENCE_NAME, DATA_TYPE, START_VALUE::VARCHAR AS START_VALUE, MINIMUM_VALUE::VARCHAR AS MINIMUM_VALUE, MAXIMUM_VALUE::VARCHAR AS MAXIMUM_VALUE, \"INCREMENT\"::VARCHAR AS INCREMENT_BY, CYCLE_OPTION, COMMENT FROM {}.INFORMATION_SCHEMA.SEQUENCES WHERE SEQUENCE_CATALOG = {} AND SEQUENCE_SCHEMA = {} AND SEQUENCE_NAME = {} LIMIT 2",
        metadata::quote_identifier(db)?,
        literal(db)?,
        literal(schema)?,
        literal(name)?,
    ))
}

pub(crate) fn parse_exact_sequence(
    context: &LiveContext,
    schema: &str,
    name: &str,
    result: &MetadataResult,
) -> AppResult<(SequenceInfo, Option<String>)> {
    let db = database(context)?;
    user_schema(schema)?;
    metadata::quote_identifier(name)?;
    check_columns(result, &SEQUENCE_COLUMNS, 2)?;
    if result.rows.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The requested Snowflake sequence is no longer visible in the current database",
        ));
    }
    if result.rows.len() > 1 {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "Multiple Snowflake sequences matched this object name",
        ));
    }
    let row = &result.rows[0];
    if required_cell(&row[0])? != db
        || required_cell(&row[1])? != schema
        || required_cell(&row[2])? != name
    {
        return Err(malformed());
    }
    let info = SequenceInfo {
        data_type: required_cell(&row[3])?.to_owned(),
        start_value: required_cell(&row[4])?.to_owned(),
        min_value: required_cell(&row[5])?.to_owned(),
        max_value: required_cell(&row[6])?.to_owned(),
        increment_by: required_cell(&row[7])?.to_owned(),
        cache_size: "—".into(),
        cycle: yes_no(&row[8])?.unwrap_or(false),
        last_value: None,
    };
    let comment = row[9].clone().filter(|s| !s.is_empty());
    Ok((info, comment))
}

pub(crate) fn exact_routine_sql(
    context: &LiveContext,
    schema: &str,
    name: &str,
    is_procedure: bool,
) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    metadata::quote_identifier(name)?;
    if is_procedure {
        Ok(format!(
            "SELECT PROCEDURE_CATALOG, PROCEDURE_SCHEMA, PROCEDURE_NAME, ARGUMENT_SIGNATURE, DATA_TYPE, PROCEDURE_LANGUAGE, COMMENT FROM {}.INFORMATION_SCHEMA.PROCEDURES WHERE PROCEDURE_CATALOG = {} AND PROCEDURE_SCHEMA = {} AND PROCEDURE_NAME = {} ORDER BY ARGUMENT_SIGNATURE LIMIT 65",
            metadata::quote_identifier(db)?,
            literal(db)?,
            literal(schema)?,
            literal(name)?,
        ))
    } else {
        Ok(format!(
            "SELECT FUNCTION_CATALOG, FUNCTION_SCHEMA, FUNCTION_NAME, ARGUMENT_SIGNATURE, DATA_TYPE, FUNCTION_LANGUAGE, IS_VOLATILE, COMMENT FROM {}.INFORMATION_SCHEMA.FUNCTIONS WHERE FUNCTION_CATALOG = {} AND FUNCTION_SCHEMA = {} AND FUNCTION_NAME = {} ORDER BY ARGUMENT_SIGNATURE LIMIT 65",
            metadata::quote_identifier(db)?,
            literal(db)?,
            literal(schema)?,
            literal(name)?,
        ))
    }
}

pub(crate) fn parse_exact_routine(
    context: &LiveContext,
    schema: &str,
    name: &str,
    is_procedure: bool,
    identity: Option<&str>,
    result: &MetadataResult,
) -> AppResult<(RoutineInfo, String, Option<String>)> {
    let db = database(context)?;
    user_schema(schema)?;
    metadata::quote_identifier(name)?;
    let expected: &[&str] = if is_procedure {
        &PROCEDURE_COLUMNS
    } else {
        &FUNCTION_COLUMNS
    };
    check_columns(result, expected, 64)?;
    let mut matches = Vec::new();
    for row in &result.rows {
        if required_cell(&row[0])? != db
            || required_cell(&row[1])? != schema
            || required_cell(&row[2])? != name
        {
            return Err(malformed());
        }
        let arg_sig = required_cell(&row[3])?;
        let norm = normalize_argument_types(arg_sig)?;
        if identity.is_none_or(|wanted| wanted == norm) {
            matches.push((row, norm));
        }
    }
    if matches.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The requested Snowflake routine is no longer visible in the current database",
        ));
    }
    if matches.len() > 1 {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "Multiple overloaded Snowflake routines share this name; select a specific signature in Database Explorer",
        ));
    }
    let (row, identity_arguments) = &matches[0];
    let arguments = required_cell(&row[3])?.to_owned();
    let return_type = Some(required_cell(&row[4])?.to_owned());
    let language = row[5]
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("SQL")
        .to_owned();
    let (volatility, comment) = if is_procedure {
        (None, row[6].clone().filter(|s| !s.is_empty()))
    } else {
        let vol = match yes_no(&row[6])? {
            Some(true) => Some("VOLATILE".to_owned()),
            Some(false) => Some("IMMUTABLE".to_owned()),
            None => None,
        };
        (vol, row[7].clone().filter(|s| !s.is_empty()))
    };
    let info = RoutineInfo {
        identity_arguments: identity_arguments.clone(),
        arguments,
        return_type,
        language,
        volatility,
        security_definer: false,
    };
    Ok((info, identity_arguments.clone(), comment))
}

pub(crate) fn sequence_ddl_sql(database: &str, schema: &str, name: &str) -> AppResult<String> {
    let qualified = format!(
        "{}.{}.{}",
        metadata::quote_identifier(database)?,
        metadata::quote_identifier(schema)?,
        metadata::quote_identifier(name)?,
    );
    Ok(format!(
        "SELECT GET_DDL('SEQUENCE', {}, TRUE) AS DDL",
        literal(&qualified)?
    ))
}

pub(crate) fn routine_ddl_sql(
    database: &str,
    schema: &str,
    name: &str,
    is_procedure: bool,
    identity_arguments: &str,
) -> AppResult<String> {
    let normalized = normalize_argument_types(&format!("({identity_arguments})"))?;
    let qualified = format!(
        "{}.{}.{}({})",
        metadata::quote_identifier(database)?,
        metadata::quote_identifier(schema)?,
        metadata::quote_identifier(name)?,
        normalized,
    );
    let kind = if is_procedure {
        "PROCEDURE"
    } else {
        "FUNCTION"
    };
    Ok(format!(
        "SELECT GET_DDL('{kind}', {}, TRUE) AS DDL",
        literal(&qualified)?
    ))
}

pub(crate) fn parse_ddl(result: &MetadataResult) -> AppResult<String> {
    browsing::parse_definition(result)
}

pub(crate) fn build_table_constraints(
    active_database: &str,
    keys: &[RelationKeyConstraint],
    mappings: &[RelationConstraintMapping],
    checks: &[RelationCheckConstraint],
) -> Vec<ConstraintInfo> {
    let mut out = Vec::with_capacity(keys.len() + checks.len());
    for key in keys {
        let kind = match key.kind {
            RelationKeyConstraintKind::PrimaryKey => ConstraintKind::PrimaryKey,
            RelationKeyConstraintKind::Unique => ConstraintKind::Unique,
            RelationKeyConstraintKind::ForeignKey => ConstraintKind::ForeignKey,
        };
        let label = match key.kind {
            RelationKeyConstraintKind::PrimaryKey => "PRIMARY KEY",
            RelationKeyConstraintKind::Unique => "UNIQUE",
            RelationKeyConstraintKind::ForeignKey => "FOREIGN KEY",
        };
        let enforcement = match (key.enforced, key.rely) {
            (true, _) => "ENFORCED",
            (false, true) => "NOT ENFORCED RELY",
            (false, false) => "NOT ENFORCED",
        };
        let mapping = mappings
            .iter()
            .find(|m| m.schema == key.schema && m.name == key.name && m.kind == key.kind);
        let columns: Vec<String> = mapping
            .map(|m| m.columns.iter().map(|c| c.column_name.clone()).collect())
            .unwrap_or_default();
        let target_columns: Vec<String> = mapping
            .map(|m| {
                m.columns
                    .iter()
                    .filter_map(|c| c.referenced_column_name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let target = mapping
            .and_then(|m| m.referenced_table.as_ref())
            .filter(|r| r.database == active_database)
            .map(|r| r.table.clone());
        let definition = match (
            mapping.and_then(|m| m.referenced_table.as_ref()),
            &key.reference,
        ) {
            (Some(ref_table), Some(reference)) => format!(
                "{label} ({enforcement}) -> {}.{}.{} (MATCH {}, ON UPDATE {}, ON DELETE {})",
                ref_table.database,
                ref_table.table.schema,
                ref_table.table.name,
                reference.match_option,
                reference.update_rule,
                reference.delete_rule,
            ),
            (Some(ref_table), None) => format!(
                "{label} ({enforcement}) -> {}.{}.{}",
                ref_table.database, ref_table.table.schema, ref_table.table.name,
            ),
            (None, Some(reference)) => format!(
                "{label} ({enforcement}) -> constraint {}.{}.{} (MATCH {}, ON UPDATE {}, ON DELETE {})",
                reference.catalog,
                reference.schema,
                reference.name,
                reference.match_option,
                reference.update_rule,
                reference.delete_rule,
            ),
            (None, None) => format!("{label} ({enforcement})"),
        };
        out.push(ConstraintInfo {
            name: key.name.clone(),
            kind,
            columns,
            target,
            target_columns,
            definition,
        });
    }
    for check in checks {
        out.push(ConstraintInfo {
            name: check.name.clone(),
            kind: ConstraintKind::Check,
            columns: Vec::new(),
            target: None,
            target_columns: Vec::new(),
            definition: if check.check_clause.is_empty() {
                "CHECK (clause unavailable)".into()
            } else {
                check.check_clause.clone()
            },
        });
    }
    out
}

/// Conservatively extract referenced relations from a Snowflake `CREATE VIEW` /
/// `CREATE MATERIALIZED VIEW` DDL definition using `SnowflakeDialect`, matching
/// only known relations in the active database.
pub(crate) fn extract_view_dependencies(
    active_database: &str,
    view_schema: &str,
    view_name: &str,
    ddl: &str,
    known_relations: &[DatabaseObjectSummary],
) -> Vec<DependencyEdge> {
    struct RelationVisitor {
        refs: Vec<Vec<String>>,
        ctes: HashSet<String>,
    }
    impl Visitor for RelationVisitor {
        type Break = ();
        fn pre_visit_query(&mut self, query: &sqlparser::ast::Query) -> ControlFlow<Self::Break> {
            if let Some(with) = &query.with {
                for cte in &with.cte_tables {
                    self.ctes.insert(cte.alias.name.value.clone());
                }
            }
            ControlFlow::Continue(())
        }
        fn pre_visit_relation(
            &mut self,
            relation: &sqlparser::ast::ObjectName,
        ) -> ControlFlow<Self::Break> {
            let parts: Vec<String> = relation
                .0
                .iter()
                .filter_map(|part| part.as_ident().map(|ident| ident.value.clone()))
                .collect();
            if !parts.is_empty() && parts.len() == relation.0.len() {
                self.refs.push(parts);
            }
            ControlFlow::Continue(())
        }
    }

    fn slice_query_after_top_level_as(ddl: &str) -> Option<&str> {
        use sqlparser::keywords::Keyword;
        use sqlparser::tokenizer::{Token, Tokenizer};
        let tokens = Tokenizer::new(&SnowflakeDialect {}, ddl)
            .tokenize_with_location()
            .ok()?;
        let non_ws: Vec<_> = tokens
            .iter()
            .filter(|t| !matches!(t.token, Token::Whitespace(_)))
            .collect();
        let mut depth = 0i32;
        for (idx, tok) in non_ws.iter().enumerate() {
            match &tok.token {
                Token::LParen => depth += 1,
                Token::RParen => depth = (depth - 1).max(0),
                Token::Word(w)
                    if depth == 0 && w.quote_style.is_none() && w.keyword == Keyword::AS =>
                {
                    let next = non_ws.get(idx + 1)?;
                    let starts_query = match &next.token {
                        Token::Word(nw) if nw.quote_style.is_none() => {
                            matches!(nw.keyword, Keyword::SELECT | Keyword::WITH)
                        }
                        Token::LParen => true,
                        _ => false,
                    };
                    if starts_query {
                        let target_line = next.span.start.line as usize;
                        let target_col = next.span.start.column as usize;
                        if target_line == 0 || target_col == 0 {
                            return None;
                        }
                        let mut cur_line = 1usize;
                        let mut line_byte_start = 0usize;
                        for (b_idx, ch) in ddl.char_indices() {
                            if cur_line == target_line {
                                line_byte_start = b_idx;
                                break;
                            }
                            if ch == '\n' {
                                cur_line += 1;
                            }
                        }
                        if cur_line != target_line {
                            return None;
                        }
                        let col_offset = ddl[line_byte_start..]
                            .char_indices()
                            .nth(target_col - 1)
                            .map(|(off, _)| off)?;
                        return ddl.get(line_byte_start + col_offset..);
                    }
                }
                _ => {}
            }
        }
        None
    }

    let statements = match Parser::new(&SnowflakeDialect {})
        .with_recursion_limit(32)
        .try_with_sql(ddl)
        .and_then(|mut parser| parser.parse_statements())
    {
        Ok(stmts) => stmts,
        Err(_) => {
            let Some(query_sql) = slice_query_after_top_level_as(ddl) else {
                return Vec::new();
            };
            let Ok(stmts) = Parser::new(&SnowflakeDialect {})
                .with_recursion_limit(32)
                .try_with_sql(query_sql)
                .and_then(|mut parser| parser.parse_statements())
            else {
                return Vec::new();
            };
            stmts
        }
    };
    let mut visitor = RelationVisitor {
        refs: Vec::new(),
        ctes: HashSet::new(),
    };
    for stmt in &statements {
        match stmt {
            sqlparser::ast::Statement::CreateView(create) => {
                let _ = create.query.visit(&mut visitor);
            }
            sqlparser::ast::Statement::CreateTable(create) => {
                if let Some(query) = &create.query {
                    let _ = query.visit(&mut visitor);
                }
            }
            sqlparser::ast::Statement::Query(query) => {
                let _ = query.visit(&mut visitor);
            }
            _ => {}
        }
    }
    let mut edges = Vec::new();
    for parts in visitor.refs {
        let (schema, name) = match parts.as_slice() {
            [single] => {
                if visitor.ctes.contains(single) {
                    continue;
                }
                (view_schema, single.as_str())
            }
            [schema, name] => (schema.as_str(), name.as_str()),
            [db, schema, name] if db == active_database => (schema.as_str(), name.as_str()),
            _ => continue,
        };
        if schema == view_schema && name == view_name {
            continue;
        }
        if let Some(found) = known_relations.iter().find(|item| {
            matches!(
                item.object.kind,
                DatabaseObjectKind::Table
                    | DatabaseObjectKind::View
                    | DatabaseObjectKind::MaterializedView
            ) && item.object.schema == schema
                && item.object.name == name
        }) {
            edges.push(DependencyEdge {
                object: found.object.clone(),
                relation: "references".into(),
            });
        }
    }
    edges
}

pub(crate) fn empty_details(object: DatabaseObjectRef) -> ObjectDetails {
    ObjectDetails {
        object,
        owner_table: None,
        comment: None,
        ddl: ObjectDdlStatus::Unavailable {
            reason: "DDL unavailable".into(),
        },
        indexes: Vec::new(),
        constraints: Vec::new(),
        triggers: Vec::new(),
        routine: None,
        sequence: None,
        storage: None,
        snowflake_table: None,
        uses: Vec::new(),
        used_by: Vec::new(),
    }
}

pub(crate) fn classify_table_subtype(target: &BrowseRelation) -> SnowflakeTableSubtype {
    match target.table_type.as_str() {
        "EXTERNAL TABLE" => SnowflakeTableSubtype::External,
        "EVENT TABLE" => SnowflakeTableSubtype::Event,
        "TEMPORARY TABLE" => SnowflakeTableSubtype::Temporary,
        _ if target.traits.dynamic == Some(true) => SnowflakeTableSubtype::Dynamic,
        _ if target.traits.hybrid == Some(true) => SnowflakeTableSubtype::Hybrid,
        _ if target.traits.iceberg == Some(true) => SnowflakeTableSubtype::Iceberg,
        _ if target.traits.transient == Some(true) => SnowflakeTableSubtype::Transient,
        _ => SnowflakeTableSubtype::Base,
    }
}

fn escape_show_like_exact(name: &str) -> AppResult<String> {
    metadata::quote_identifier(name)?;
    let mut escaped = String::with_capacity(name.len() + 4);
    for ch in name.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        if ch == '\'' {
            escaped.push('\'');
        }
        escaped.push(ch);
    }
    Ok(format!("'{escaped}'"))
}

pub(crate) fn dynamic_table_meta_sql(
    context: &LiveContext,
    schema: &str,
    name: &str,
) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    Ok(format!(
        "SHOW DYNAMIC TABLES LIKE {} IN SCHEMA {}.{}",
        escape_show_like_exact(name)?,
        metadata::quote_identifier(db)?,
        metadata::quote_identifier(schema)?,
    ))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DynamicTableMeta {
    pub target_lag: Option<String>,
    pub warehouse: Option<String>,
    pub refresh_mode: Option<String>,
    pub scheduling_state: Option<String>,
}

pub(crate) fn parse_dynamic_table_meta(
    expected_name: &str,
    result: &MetadataResult,
) -> DynamicTableMeta {
    let col_idx = |wanted: &str| {
        result
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(wanted))
    };
    let Some(name_idx) = col_idx("name") else {
        return DynamicTableMeta::default();
    };
    let lag_idx = col_idx("target_lag");
    let wh_idx = col_idx("warehouse");
    let mode_idx = col_idx("refresh_mode");
    let sched_idx = col_idx("scheduling_state");

    for row in &result.rows {
        if row.get(name_idx).and_then(Option::as_deref) == Some(expected_name) {
            let pick = |idx: Option<usize>| {
                idx.and_then(|i| row.get(i))
                    .and_then(Clone::clone)
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
            };
            return DynamicTableMeta {
                target_lag: pick(lag_idx),
                warehouse: pick(wh_idx),
                refresh_mode: pick(mode_idx),
                scheduling_state: pick(sched_idx),
            };
        }
    }
    DynamicTableMeta::default()
}

pub(crate) fn external_table_meta_sql(
    context: &LiveContext,
    schema: &str,
    name: &str,
) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    Ok(format!(
        "SHOW EXTERNAL TABLES LIKE {} IN SCHEMA {}.{}",
        escape_show_like_exact(name)?,
        metadata::quote_identifier(db)?,
        metadata::quote_identifier(schema)?,
    ))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ExternalTableMeta {
    pub external_location: Option<String>,
    pub file_format: Option<String>,
}

pub(crate) fn parse_external_table_meta(
    expected_name: &str,
    result: &MetadataResult,
) -> ExternalTableMeta {
    let col_idx = |wanted: &str| {
        result
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(wanted))
    };
    let Some(name_idx) = col_idx("name") else {
        return ExternalTableMeta::default();
    };
    let loc_idx = col_idx("location").or_else(|| col_idx("stage"));
    let fmt_name_idx = col_idx("file_format_name");
    let fmt_type_idx = col_idx("file_format_type");

    for row in &result.rows {
        if row.get(name_idx).and_then(Option::as_deref) == Some(expected_name) {
            let pick = |idx: Option<usize>| {
                idx.and_then(|i| row.get(i))
                    .and_then(Clone::clone)
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
            };
            let file_format = pick(fmt_name_idx).or_else(|| pick(fmt_type_idx));
            return ExternalTableMeta {
                external_location: pick(loc_idx),
                file_format,
            };
        }
    }
    ExternalTableMeta::default()
}

pub const MAX_QUERY_HISTORY_LIMIT: u32 = 100;
pub const MAX_OPERATOR_STATS_ROWS: usize = 500;
const MAX_QUERY_TEXT_PREVIEW_BYTES: usize = 16 * 1024;

pub(crate) fn is_valid_query_id(query_id: &str) -> bool {
    let trimmed = query_id.trim();
    (8..=64).contains(&trimmed.len())
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub(crate) fn query_history_sql(limit: u32) -> AppResult<String> {
    if !(1..=MAX_QUERY_HISTORY_LIMIT).contains(&limit) {
        return Err(AppError::invalid(format!(
            "Snowflake query history limit must be between 1 and {MAX_QUERY_HISTORY_LIMIT}"
        )));
    }
    Ok(format!(
        "SELECT QUERY_ID, QUERY_TEXT, QUERY_TYPE, EXECUTION_STATUS, WAREHOUSE_NAME, WAREHOUSE_SIZE, DATABASE_NAME, SCHEMA_NAME, START_TIME::VARCHAR AS START_TIME, TOTAL_ELAPSED_TIME::VARCHAR AS TOTAL_ELAPSED_TIME, COMPILATION_TIME::VARCHAR AS COMPILATION_TIME, EXECUTION_TIME::VARCHAR AS EXECUTION_TIME, (COALESCE(QUEUED_PROVISIONING_TIME, 0) + COALESCE(QUEUED_REPAIR_TIME, 0) + COALESCE(QUEUED_OVERLOAD_TIME, 0))::VARCHAR AS QUEUED_TIME, BYTES_SCANNED::VARCHAR AS BYTES_SCANNED, PERCENTAGE_SCANNED_FROM_CACHE::VARCHAR AS CACHE_RATIO, BYTES_SPILLED_TO_LOCAL_STORAGE::VARCHAR AS SPILL_LOCAL, BYTES_SPILLED_TO_REMOTE_STORAGE::VARCHAR AS SPILL_REMOTE, PARTITIONS_SCANNED::VARCHAR AS PARTITIONS_SCANNED, PARTITIONS_TOTAL::VARCHAR AS PARTITIONS_TOTAL, ROWS_PRODUCED::VARCHAR AS ROWS_PRODUCED, ERROR_MESSAGE FROM TABLE(INFORMATION_SCHEMA.QUERY_HISTORY_BY_SESSION(RESULT_LIMIT => {limit})) ORDER BY START_TIME DESC"
    ))
}

fn opt_u64(cell: Option<&Option<String>>) -> Option<u64> {
    let raw = cell?.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    raw.parse::<u64>().ok().or_else(|| {
        raw.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v as u64)
    })
}

fn opt_f64(cell: Option<&Option<String>>) -> Option<f64> {
    let raw = cell?.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    raw.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn cap_utf8_str(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_owned();
    out.push('…');
    out
}

pub(crate) fn parse_query_history_entries(
    result: &MetadataResult,
    redact_sql: bool,
) -> AppResult<Vec<SnowflakeQueryHistoryEntry>> {
    if result.columns.len() != 21 || result.rows.len() > MAX_QUERY_HISTORY_LIMIT as usize {
        return Err(malformed());
    }
    let mut entries = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        if row.len() != 21 {
            return Err(malformed());
        }
        let query_id = required_cell(&row[0])?.trim().to_owned();
        if !is_valid_query_id(&query_id) {
            continue;
        }
        let raw_sql = row[1].as_deref().unwrap_or_default();
        let query_text = if redact_sql {
            "[SQL text hidden by connection privacy policy]".to_owned()
        } else {
            cap_utf8_str(raw_sql, MAX_QUERY_TEXT_PREVIEW_BYTES)
        };
        let clean_opt = |idx: usize| {
            row[idx]
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        entries.push(SnowflakeQueryHistoryEntry {
            query_id,
            query_text,
            query_type: clean_opt(2).unwrap_or_else(|| "UNKNOWN".into()),
            execution_status: clean_opt(3).unwrap_or_else(|| "UNKNOWN".into()),
            warehouse_name: clean_opt(4),
            warehouse_size: clean_opt(5),
            database_name: clean_opt(6),
            schema_name: clean_opt(7),
            start_time: clean_opt(8).unwrap_or_default(),
            total_elapsed_ms: opt_u64(row.get(9)),
            compilation_ms: opt_u64(row.get(10)),
            execution_ms: opt_u64(row.get(11)),
            queued_ms: opt_u64(row.get(12)),
            bytes_scanned: opt_u64(row.get(13)),
            cache_hit_ratio: opt_f64(row.get(14)),
            bytes_spilled_local: opt_u64(row.get(15)),
            bytes_spilled_remote: opt_u64(row.get(16)),
            partitions_scanned: opt_u64(row.get(17)),
            partitions_total: opt_u64(row.get(18)),
            rows_produced: opt_u64(row.get(19)),
            error_message: clean_opt(20),
        });
    }
    Ok(entries)
}

pub(crate) fn operator_profile_sql() -> &'static str {
    "SELECT STEP_ID::VARCHAR AS STEP_ID, OPERATOR_ID::VARCHAR AS OPERATOR_ID, TO_JSON(PARENT_OPERATORS) AS PARENT_OPERATORS, OPERATOR_TYPE, EXECUTION_TIME_BREAKDOWN:overall_percentage::VARCHAR AS OVERALL_PCT, OPERATOR_STATISTICS:input_rows::VARCHAR AS INPUT_ROWS, OPERATOR_STATISTICS:output_rows::VARCHAR AS OUTPUT_ROWS, OPERATOR_STATISTICS:io:bytes_scanned::VARCHAR AS BYTES_SCANNED, OPERATOR_STATISTICS:io:percentage_scanned_from_cache::VARCHAR AS CACHE_RATIO, OPERATOR_STATISTICS:pruning:partitions_scanned::VARCHAR AS PARTITIONS_SCANNED, OPERATOR_STATISTICS:pruning:partitions_total::VARCHAR AS PARTITIONS_TOTAL, OPERATOR_STATISTICS:spilling:bytes_spilled_local_storage::VARCHAR AS SPILL_LOCAL, OPERATOR_STATISTICS:spilling:bytes_spilled_remote_storage::VARCHAR AS SPILL_REMOTE, TO_JSON(OPERATOR_ATTRIBUTES) AS OPERATOR_ATTRIBUTES FROM TABLE(GET_QUERY_OPERATOR_STATS(?)) ORDER BY STEP_ID, OPERATOR_ID LIMIT 500"
}

fn parse_parent_operator_ids(raw: Option<&str>) -> Vec<u32> {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(text) {
        return arr
            .into_iter()
            .filter_map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u32>().ok()))
            })
            .collect();
    }
    Vec::new()
}

pub(crate) fn parse_operator_profile(
    query_id: &str,
    result: &MetadataResult,
) -> AppResult<SnowflakeOperatorProfile> {
    if !is_valid_query_id(query_id) {
        return Err(AppError::invalid("Provide a valid Snowflake query ID"));
    }
    if result.columns.len() != 14 || result.rows.len() > MAX_OPERATOR_STATS_ROWS {
        return Err(malformed());
    }
    let mut operators = Vec::with_capacity(result.rows.len());
    let mut total_spilled_local_bytes = 0u64;
    let mut total_spilled_remote_bytes = 0u64;
    let mut best_bottleneck: Option<(f64, u64, u32)> = None;

    for row in &result.rows {
        if row.len() != 14 {
            return Err(malformed());
        }
        let step_id = opt_u64(row.first())
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(1);
        let operator_id = opt_u64(row.get(1))
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(malformed)?;
        let parent_operators = parse_parent_operator_ids(row[2].as_deref());
        let operator_type = required_cell(&row[3])?.trim().to_owned();
        let overall_percentage = opt_f64(row.get(4));
        let input_rows = opt_u64(row.get(5));
        let output_rows = opt_u64(row.get(6));
        let bytes_scanned = opt_u64(row.get(7));
        let cache_hit_ratio = opt_f64(row.get(8));
        let partitions_scanned = opt_u64(row.get(9));
        let partitions_total = opt_u64(row.get(10));
        let bytes_spilled_local = opt_u64(row.get(11));
        let bytes_spilled_remote = opt_u64(row.get(12));
        let detail = row[13]
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "{}" && *s != "null")
            .map(|s| cap_utf8_str(s, 1024));

        let op_spill = bytes_spilled_local
            .unwrap_or(0)
            .saturating_add(bytes_spilled_remote.unwrap_or(0));
        total_spilled_local_bytes =
            total_spilled_local_bytes.saturating_add(bytes_spilled_local.unwrap_or(0));
        total_spilled_remote_bytes =
            total_spilled_remote_bytes.saturating_add(bytes_spilled_remote.unwrap_or(0));

        let pct = overall_percentage.unwrap_or(0.0);
        if pct >= 0.20 || op_spill > 0 {
            let score = (pct, op_spill, operator_id);
            if best_bottleneck.is_none_or(|prev| (score.0, score.1) > (prev.0, prev.1)) {
                best_bottleneck = Some(score);
            }
        }

        operators.push(SnowflakeOperatorStat {
            step_id,
            operator_id,
            parent_operators,
            operator_type,
            overall_percentage,
            input_rows,
            output_rows,
            bytes_scanned,
            cache_hit_ratio,
            partitions_scanned,
            partitions_total,
            bytes_spilled_local,
            bytes_spilled_remote,
            detail,
        });
    }

    Ok(SnowflakeOperatorProfile {
        query_id: query_id.trim().to_owned(),
        operators,
        total_spilled_local_bytes,
        total_spilled_remote_bytes,
        bottleneck_operator_id: best_bottleneck.map(|(_, _, id)| id),
    })
}

const SCHEMA_RELATION_COLUMNS: [&str; 12] = [
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
    "CLUSTERING_KEY",
    "COMMENT",
];

const SCHEMA_COLUMN_COLUMNS: [&str; 16] = [
    "TABLE_CATALOG",
    "TABLE_SCHEMA",
    "TABLE_NAME",
    "COLUMN_NAME",
    "ORDINAL_POSITION",
    "COLUMN_DEFAULT",
    "IS_NULLABLE",
    "DATA_TYPE",
    "CHARACTER_MAXIMUM_LENGTH",
    "NUMERIC_PRECISION",
    "NUMERIC_SCALE",
    "IS_IDENTITY",
    "IDENTITY_START",
    "IDENTITY_INCREMENT",
    "EXPRESSION",
    "KIND",
];

const SCHEMA_CONSTRAINT_COLUMNS: [&str; 11] = [
    "TABLE_CATALOG",
    "TABLE_SCHEMA",
    "TABLE_NAME",
    "CONSTRAINT_NAME",
    "CONSTRAINT_TYPE",
    "ENFORCED",
    "RELY",
    "UNIQUE_CONSTRAINT_CATALOG",
    "UNIQUE_CONSTRAINT_SCHEMA",
    "UPDATE_RULE",
    "DELETE_RULE",
];

const SCHEMA_CHECK_COLUMNS: [&str; 5] = [
    "CONSTRAINT_CATALOG",
    "CONSTRAINT_SCHEMA",
    "CONSTRAINT_TABLE",
    "CONSTRAINT_NAME",
    "CHECK_CLAUSE",
];

const SCHEMA_KEY_COLUMNS: [&str; 6] = [
    "DATABASE_NAME",
    "SCHEMA_NAME",
    "TABLE_NAME",
    "COLUMN_NAME",
    "KEY_SEQUENCE",
    "CONSTRAINT_NAME",
];

const SCHEMA_FOREIGN_COLUMNS: [&str; 11] = [
    "FK_DATABASE_NAME",
    "FK_SCHEMA_NAME",
    "FK_TABLE_NAME",
    "FK_COLUMN_NAME",
    "KEY_SEQUENCE",
    "FK_NAME",
    "PK_DATABASE_NAME",
    "PK_SCHEMA_NAME",
    "PK_TABLE_NAME",
    "PK_COLUMN_NAME",
    "PK_NAME",
];

pub(crate) fn validate_user_schema(schema: &str) -> AppResult<()> {
    user_schema(schema)
}

fn check_columns_truncatable(
    result: &MetadataResult,
    expected: &[&str],
    max_rows: usize,
) -> AppResult<bool> {
    if result.columns.len() != expected.len()
        || result
            .columns
            .iter()
            .zip(expected.iter())
            .any(|(col, exp)| col.name != *exp)
        || result.rows.iter().any(|row| row.len() != expected.len())
    {
        return Err(malformed());
    }
    Ok(result.truncated || result.rows_seen > max_rows as u64 || result.rows.len() > max_rows)
}

pub(crate) fn strip_active_database_qualifier(database: &str, sql: &str) -> String {
    let trimmed = sql.trim();
    if trimmed.is_empty() || database.is_empty() {
        return trimmed.to_owned();
    }
    let mut out = if let Ok(quoted_db) = metadata::quote_identifier(database) {
        trimmed
            .replace(&format!("'{quoted_db}."), "'")
            .replace(&format!("{quoted_db}."), "")
    } else {
        trimmed.to_owned()
    };
    if database
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'))
    {
        for prefix in [' ', '\n', '\t', '('] {
            let needle = format!("{prefix}{database}.");
            let replacement = prefix.to_string();
            out = out.replace(&needle, &replacement);
        }
        if let Some(rest) = out.strip_prefix(&format!("{database}.")) {
            out = rest.to_owned();
        }
    }
    out
}

pub(crate) fn format_snowflake_column_type(
    data_type: &str,
    char_max: Option<&str>,
    num_precision: Option<&str>,
    num_scale: Option<&str>,
) -> String {
    let upper = data_type.trim().to_ascii_uppercase();
    match upper.as_str() {
        "NUMBER" => match (
            num_precision.filter(|s| !s.is_empty()),
            num_scale.filter(|s| !s.is_empty()),
        ) {
            (Some(p), Some(s)) => format!("NUMBER({p}, {s})"),
            (Some(p), None) => format!("NUMBER({p})"),
            _ => "NUMBER".into(),
        },
        "TEXT" => match char_max.filter(|s| !s.is_empty() && *s != "16777216") {
            Some(len) => format!("VARCHAR({len})"),
            None => "VARCHAR".into(),
        },
        "BINARY" => match char_max.filter(|s| !s.is_empty() && *s != "8388608") {
            Some(len) => format!("BINARY({len})"),
            None => "BINARY".into(),
        },
        _ => upper,
    }
}

fn normalize_clustering_key(database: &str, raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }
    let stripped = strip_active_database_qualifier(database, value);
    let inner = if stripped.len() >= 8
        && stripped[..7].eq_ignore_ascii_case("LINEAR(")
        && stripped.ends_with(')')
    {
        stripped[7..stripped.len() - 1].trim()
    } else {
        stripped.trim()
    };
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_owned())
    }
}

pub(crate) fn schema_relations_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    Ok(format!(
        "SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, IS_ICEBERG, IS_DYNAMIC, IS_HYBRID, IS_IMMUTABLE, CLUSTERING_KEY, COMMENT FROM {}.INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = {} AND TABLE_SCHEMA = {} ORDER BY TABLE_NAME, TABLE_TYPE, IS_TEMPORARY DESC LIMIT {}",
        metadata::quote_identifier(db)?,
        literal(db)?,
        literal(schema)?,
        SCHEMA_RELATION_LIMIT + 1,
    ))
}

pub(crate) fn schema_columns_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    Ok(format!(
        "SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, ORDINAL_POSITION::VARCHAR AS ORDINAL_POSITION, COLUMN_DEFAULT, IS_NULLABLE, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH::VARCHAR AS CHARACTER_MAXIMUM_LENGTH, NUMERIC_PRECISION::VARCHAR AS NUMERIC_PRECISION, NUMERIC_SCALE::VARCHAR AS NUMERIC_SCALE, IS_IDENTITY, IDENTITY_START::VARCHAR AS IDENTITY_START, IDENTITY_INCREMENT::VARCHAR AS IDENTITY_INCREMENT, EXPRESSION, KIND FROM {}.INFORMATION_SCHEMA.COLUMNS AS SC WHERE TABLE_CATALOG = {} AND TABLE_SCHEMA = {} ORDER BY SC.TABLE_NAME, SC.ORDINAL_POSITION LIMIT {}",
        metadata::quote_identifier(db)?,
        literal(db)?,
        literal(schema)?,
        SCHEMA_COLUMN_LIMIT + 1,
    ))
}

pub(crate) fn schema_constraints_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    let quoted_db = metadata::quote_identifier(db)?;
    Ok(format!(
        "SELECT STC.TABLE_CATALOG AS TABLE_CATALOG, STC.TABLE_SCHEMA AS TABLE_SCHEMA, STC.TABLE_NAME AS TABLE_NAME, STC.CONSTRAINT_NAME AS CONSTRAINT_NAME, STC.CONSTRAINT_TYPE AS CONSTRAINT_TYPE, STC.ENFORCED AS ENFORCED, STC.RELY AS RELY, SRC.UNIQUE_CONSTRAINT_CATALOG AS UNIQUE_CONSTRAINT_CATALOG, SRC.UNIQUE_CONSTRAINT_SCHEMA AS UNIQUE_CONSTRAINT_SCHEMA, SRC.UPDATE_RULE AS UPDATE_RULE, SRC.DELETE_RULE AS DELETE_RULE FROM {quoted_db}.INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS STC LEFT JOIN {quoted_db}.INFORMATION_SCHEMA.REFERENTIAL_CONSTRAINTS AS SRC ON STC.CONSTRAINT_TYPE = 'FOREIGN KEY' AND STC.CONSTRAINT_CATALOG = SRC.CONSTRAINT_CATALOG AND STC.CONSTRAINT_SCHEMA = SRC.CONSTRAINT_SCHEMA AND STC.CONSTRAINT_NAME = SRC.CONSTRAINT_NAME WHERE STC.TABLE_CATALOG = ? AND STC.TABLE_SCHEMA = ? ORDER BY STC.TABLE_NAME, STC.CONSTRAINT_NAME, STC.CONSTRAINT_TYPE LIMIT {}",
        SCHEMA_CONSTRAINT_LIMIT + 1,
    ))
}

pub(crate) fn schema_checks_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    let quoted_db = metadata::quote_identifier(db)?;
    Ok(format!(
        "SELECT CONSTRAINT_CATALOG, CONSTRAINT_SCHEMA, CONSTRAINT_TABLE, CONSTRAINT_NAME, CHECK_CLAUSE FROM {quoted_db}.INFORMATION_SCHEMA.CHECK_CONSTRAINTS AS SCC WHERE SCC.CONSTRAINT_CATALOG = ? AND SCC.CONSTRAINT_SCHEMA = ? ORDER BY SCC.CONSTRAINT_TABLE, SCC.CONSTRAINT_NAME LIMIT {}",
        SCHEMA_CHECK_LIMIT + 1,
    ))
}

pub(crate) fn schema_mapping_sql(
    context: &LiveContext,
    schema: &str,
    kind: RelationKeyConstraintKind,
) -> AppResult<String> {
    let db = database(context)?;
    user_schema(schema)?;
    let qualified_schema = format!(
        "{}.{}",
        metadata::quote_identifier(db)?,
        metadata::quote_identifier(schema)?
    );
    let (command, columns, table_col, name_col, col_col) = match kind {
        RelationKeyConstraintKind::PrimaryKey => (
            "PRIMARY",
            SCHEMA_KEY_COLUMNS.as_slice(),
            "TABLE_NAME",
            "CONSTRAINT_NAME",
            "COLUMN_NAME",
        ),
        RelationKeyConstraintKind::Unique => (
            "UNIQUE",
            SCHEMA_KEY_COLUMNS.as_slice(),
            "TABLE_NAME",
            "CONSTRAINT_NAME",
            "COLUMN_NAME",
        ),
        RelationKeyConstraintKind::ForeignKey => (
            "IMPORTED",
            SCHEMA_FOREIGN_COLUMNS.as_slice(),
            "FK_TABLE_NAME",
            "FK_NAME",
            "FK_COLUMN_NAME",
        ),
    };
    let projection = columns
        .iter()
        .map(|column| {
            format!(
                "TO_VARCHAR(\"{}\") AS {column}",
                column.to_ascii_lowercase()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "SHOW {command} KEYS IN SCHEMA {qualified_schema} ->> SELECT {projection} FROM $1 ORDER BY {table_col}, {name_col}, \"key_sequence\", {col_col} LIMIT {}",
        SCHEMA_KEY_LIMIT + 1
    ))
}

struct ParsedSchemaRelation {
    name: String,
    table_type: String,
    traits: RelationTraits,
    clustering_key: Option<String>,
    comment: Option<String>,
}

fn parse_schema_relation_rows(
    db: &str,
    schema: &str,
    result: &MetadataResult,
) -> AppResult<(Vec<ParsedSchemaRelation>, HashSet<String>, bool)> {
    let truncated =
        check_columns_truncatable(result, &SCHEMA_RELATION_COLUMNS, SCHEMA_RELATION_LIMIT)?;
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    let mut parsed = Vec::with_capacity(result.rows.len().min(SCHEMA_RELATION_LIMIT));
    for row in result.rows.iter().take(SCHEMA_RELATION_LIMIT) {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let name = required_cell(&row[2])?.to_owned();
        metadata::quote_identifier(&name)?;
        let table_type = required_cell(&row[3])?.to_owned();
        let traits = RelationTraits {
            transient: yes_no(&row[4])?,
            temporary: yes_no(&row[5])?,
            iceberg: yes_no(&row[6])?,
            dynamic: yes_no(&row[7])?,
            hybrid: yes_no(&row[8])?,
            immutable: yes_no(&row[9])?,
        };
        let clustering_key = normalize_clustering_key(db, row[10].as_deref());
        let comment = row[11].clone().filter(|s| !s.is_empty());
        *name_counts.entry(name.clone()).or_insert(0) += 1;
        parsed.push(ParsedSchemaRelation {
            name,
            table_type,
            traits,
            clustering_key,
            comment,
        });
    }
    let collided: HashSet<String> = name_counts
        .into_iter()
        .filter_map(|(name, count)| (count > 1).then_some(name))
        .collect();
    Ok((parsed, collided, truncated))
}

type ConstraintColumnMap = HashMap<(String, String), Vec<String>>;

fn parse_schema_key_groups(
    db: &str,
    schema: &str,
    result: &MetadataResult,
) -> AppResult<(ConstraintColumnMap, bool, bool)> {
    let truncated = check_columns_truncatable(result, &SCHEMA_KEY_COLUMNS, SCHEMA_KEY_LIMIT)?;
    let mut raw_groups: HashMap<(String, String), BTreeMap<u32, String>> = HashMap::new();
    let mut restricted = false;
    for row in result.rows.iter().take(SCHEMA_KEY_LIMIT) {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let table = required_cell(&row[2])?.to_owned();
        let column = required_cell(&row[3])?.to_owned();
        let seq = required_cell(&row[4])?
            .parse::<u32>()
            .map_err(|_| malformed())?;
        let cname = required_cell(&row[5])?.to_owned();
        metadata::quote_identifier(&table)?;
        metadata::quote_identifier(&column)?;
        metadata::quote_identifier(&cname)?;
        if seq == 0 {
            return Err(malformed());
        }
        if raw_groups
            .entry((table, cname))
            .or_default()
            .insert(seq, column)
            .is_some()
        {
            return Err(malformed());
        }
    }
    let mut out = HashMap::with_capacity(raw_groups.len());
    for (key, cols) in raw_groups {
        if cols.keys().copied().ne(1..=cols.len() as u32) {
            restricted = true;
            continue;
        }
        out.insert(key, cols.into_values().collect());
    }
    Ok((out, truncated, restricted))
}

struct ParsedForeignGroup {
    columns: Vec<String>,
    target_database: String,
    target: TableRef,
    target_columns: Vec<String>,
}

type ForeignConstraintGroupMap = HashMap<(String, String), ParsedForeignGroup>;

fn parse_schema_foreign_groups(
    db: &str,
    schema: &str,
    result: &MetadataResult,
) -> AppResult<(ForeignConstraintGroupMap, bool, bool)> {
    let truncated = check_columns_truncatable(result, &SCHEMA_FOREIGN_COLUMNS, SCHEMA_KEY_LIMIT)?;
    struct RawFk {
        cols: BTreeMap<u32, (String, String)>,
        target_db: String,
        target_schema: String,
        target_table: String,
        ambiguous: bool,
    }
    let mut groups: HashMap<(String, String), RawFk> = HashMap::new();
    for row in result.rows.iter().take(SCHEMA_KEY_LIMIT) {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let fk_table = required_cell(&row[2])?.to_owned();
        let fk_col = required_cell(&row[3])?.to_owned();
        let seq = required_cell(&row[4])?
            .parse::<u32>()
            .map_err(|_| malformed())?;
        let fk_name = required_cell(&row[5])?.to_owned();
        let pk_db = required_cell(&row[6])?.to_owned();
        let pk_schema = required_cell(&row[7])?.to_owned();
        let pk_table = required_cell(&row[8])?.to_owned();
        let pk_col = required_cell(&row[9])?.to_owned();
        let _pk_name = required_cell(&row[10])?;
        for ident in [
            &fk_table, &fk_col, &fk_name, &pk_db, &pk_schema, &pk_table, &pk_col,
        ] {
            metadata::quote_identifier(ident)?;
        }
        if seq == 0 {
            return Err(malformed());
        }
        let entry = groups.entry((fk_table, fk_name)).or_insert_with(|| RawFk {
            cols: BTreeMap::new(),
            target_db: pk_db.clone(),
            target_schema: pk_schema.clone(),
            target_table: pk_table.clone(),
            ambiguous: false,
        });
        if entry.target_db != pk_db
            || entry.target_schema != pk_schema
            || entry.target_table != pk_table
            || entry.cols.insert(seq, (fk_col, pk_col)).is_some()
        {
            entry.ambiguous = true;
        }
    }
    let mut restricted = false;
    let mut out = HashMap::with_capacity(groups.len());
    for (key, raw) in groups {
        if raw.ambiguous || raw.cols.keys().copied().ne(1..=raw.cols.len() as u32) {
            restricted = true;
            continue;
        }
        let mut columns = Vec::with_capacity(raw.cols.len());
        let mut target_columns = Vec::with_capacity(raw.cols.len());
        for (src, dst) in raw.cols.into_values() {
            columns.push(src);
            target_columns.push(dst);
        }
        out.insert(
            key,
            ParsedForeignGroup {
                columns,
                target_database: raw.target_db,
                target: TableRef {
                    schema: raw.target_schema,
                    name: raw.target_table,
                },
                target_columns,
            },
        );
    }
    Ok((out, truncated, restricted))
}

pub(crate) fn parse_schema_diagram(
    context: &LiveContext,
    schema: &str,
    relations_res: &MetadataResult,
    columns_res: &MetadataResult,
    primary_keys_res: &MetadataResult,
    foreign_keys_res: &MetadataResult,
) -> AppResult<SchemaDiagram> {
    let db = database(context)?;
    user_schema(schema)?;
    let (relations, collided, rel_trunc) = parse_schema_relation_rows(db, schema, relations_res)?;
    let col_trunc =
        check_columns_truncatable(columns_res, &SCHEMA_COLUMN_COLUMNS, SCHEMA_COLUMN_LIMIT)?;
    let (pk_groups, pk_trunc, pk_restr) = parse_schema_key_groups(db, schema, primary_keys_res)?;
    let (fk_groups, fk_trunc, fk_restr) =
        parse_schema_foreign_groups(db, schema, foreign_keys_res)?;

    let mut extra_truncated = rel_trunc || col_trunc || pk_trunc || fk_trunc;
    let mut extra_restricted = !collided.is_empty() || pk_restr || fk_restr;

    let mut tables = Vec::new();
    for rel in &relations {
        if collided.contains(&rel.name) {
            continue;
        }
        if matches!(
            rel.table_type.as_str(),
            "BASE TABLE" | "TEMPORARY TABLE" | "VIEW" | "MATERIALIZED VIEW"
        ) {
            tables.push((rel.name.clone(), rel.table_type.clone()));
        }
    }

    let mut pk_columns: HashSet<(String, String)> = HashSet::new();
    for ((table, _cname), cols) in &pk_groups {
        for col in cols {
            pk_columns.insert((table.clone(), col.clone()));
        }
    }

    let mut readable_columns: HashSet<(String, String)> = HashSet::new();
    let mut diagram_columns = Vec::with_capacity(columns_res.rows.len().min(SCHEMA_COLUMN_LIMIT));
    for row in columns_res.rows.iter().take(SCHEMA_COLUMN_LIMIT) {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let table_name = required_cell(&row[2])?.to_owned();
        if collided.contains(&table_name) {
            continue;
        }
        let col_name = required_cell(&row[3])?.to_owned();
        metadata::quote_identifier(&table_name)?;
        metadata::quote_identifier(&col_name)?;
        let _ordinal = required_cell(&row[4])?
            .parse::<u32>()
            .map_err(|_| malformed())?;
        let nullable = yes_no(&row[6])?.ok_or_else(malformed)?;
        let database_type = format_snowflake_column_type(
            required_cell(&row[7])?,
            row[8].as_deref(),
            row[9].as_deref(),
            row[10].as_deref(),
        );
        readable_columns.insert((table_name.clone(), col_name.clone()));
        diagram_columns.push((
            table_name.clone(),
            DiagramColumn {
                primary_key: pk_columns.contains(&(table_name, col_name.clone())),
                name: col_name,
                database_type,
                nullable,
            },
        ));
    }

    let mut sorted_fks: Vec<_> = fk_groups.into_iter().collect();
    sorted_fks.sort_by(|a, b| a.0.cmp(&b.0));

    let mut diagram_keys = Vec::new();
    for ((source_table, fk_name), group) in sorted_fks {
        if collided.contains(&source_table) {
            extra_restricted = true;
            continue;
        }
        if group.target_database != db {
            extra_restricted = true;
            continue;
        }
        if group
            .columns
            .iter()
            .any(|col| !readable_columns.contains(&(source_table.clone(), col.clone())))
        {
            extra_restricted = true;
            continue;
        }
        let width = group.columns.len();
        for (src_col, dst_col) in group.columns.into_iter().zip(group.target_columns) {
            if diagram_keys.len() > crate::domain::MAX_DIAGRAM_KEY_ROWS {
                extra_truncated = true;
                break;
            }
            diagram_keys.push(DiagramKeyRow {
                name: fk_name.clone(),
                source_table: source_table.clone(),
                target: group.target.clone(),
                column: src_col,
                target_column: dst_col,
                expected: Some(width),
            });
        }
    }

    let mut diagram = SchemaDiagram::assemble(schema, tables, diagram_columns, diagram_keys);
    diagram.truncated |= extra_truncated;
    diagram.restricted |= extra_restricted;
    Ok(diagram)
}

pub(crate) struct SnapshotRelationPlan {
    pub snapshot: SchemaSnapshot,
    pub view_targets: Vec<BrowseRelation>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_schema_snapshot_base(
    context: &LiveContext,
    server_version: &str,
    schema: &str,
    relations_res: &MetadataResult,
    columns_res: &MetadataResult,
    constraints_res: &MetadataResult,
    checks_res: &MetadataResult,
    primary_keys_res: &MetadataResult,
    unique_keys_res: &MetadataResult,
    foreign_keys_res: &MetadataResult,
    sequences_res: &MetadataResult,
    functions_res: &MetadataResult,
    procedures_res: &MetadataResult,
) -> AppResult<SnapshotRelationPlan> {
    let db = database(context)?;
    user_schema(schema)?;
    let role = context
        .role
        .clone()
        .unwrap_or_else(|| context.principal.clone());

    let (relations, collided, rel_trunc) = parse_schema_relation_rows(db, schema, relations_res)?;
    let col_trunc =
        check_columns_truncatable(columns_res, &SCHEMA_COLUMN_COLUMNS, MAX_SNAPSHOT_COLUMNS)?;
    let con_trunc = check_columns_truncatable(
        constraints_res,
        &SCHEMA_CONSTRAINT_COLUMNS,
        SCHEMA_CONSTRAINT_LIMIT,
    )?;
    let chk_trunc =
        check_columns_truncatable(checks_res, &SCHEMA_CHECK_COLUMNS, SCHEMA_CHECK_LIMIT)?;
    let (pk_groups, pk_trunc, pk_restr) = parse_schema_key_groups(db, schema, primary_keys_res)?;
    let (uq_groups, uq_trunc, uq_restr) = parse_schema_key_groups(db, schema, unique_keys_res)?;
    let (fk_groups, fk_trunc, fk_restr) =
        parse_schema_foreign_groups(db, schema, foreign_keys_res)?;
    let seq_trunc =
        check_columns_truncatable(sequences_res, &SEQUENCE_COLUMNS, MAX_SNAPSHOT_SEQUENCES)?;
    let fn_trunc =
        check_columns_truncatable(functions_res, &FUNCTION_COLUMNS, MAX_SNAPSHOT_ROUTINES)?;
    let proc_trunc =
        check_columns_truncatable(procedures_res, &PROCEDURE_COLUMNS, MAX_SNAPSHOT_ROUTINES)?;

    let mut truncated = rel_trunc
        || col_trunc
        || con_trunc
        || chk_trunc
        || pk_trunc
        || uq_trunc
        || fk_trunc
        || seq_trunc
        || fn_trunc
        || proc_trunc;
    let mut restricted = pk_restr || uq_restr || fk_restr;
    let mut unsupported_objects: Vec<UnsupportedSchemaObject> = Vec::new();
    let push_unsupported = |out: &mut Vec<UnsupportedSchemaObject>,
                            trunc: &mut bool,
                            name: String,
                            object_type: &str,
                            reason: &str| {
        if out.len() < MAX_SNAPSHOT_UNSUPPORTED {
            out.push(UnsupportedSchemaObject {
                name,
                object_type: object_type.to_owned(),
                reason: reason.to_owned(),
            });
        } else {
            *trunc = true;
        }
    };

    for name in &collided {
        restricted = true;
        push_unsupported(
            &mut unsupported_objects,
            &mut truncated,
            name.clone(),
            "Shadowed relation",
            "Multiple Snowflake relations share this name (such as a temporary shadow) and cannot be snapshotted unambiguously.",
        );
    }

    let mut tables: Vec<SnapshotTable> = Vec::new();
    let mut table_index: HashMap<String, usize> = HashMap::new();
    let mut view_targets: Vec<BrowseRelation> = Vec::new();

    for rel in relations {
        if collided.contains(&rel.name) {
            continue;
        }
        match rel.table_type.as_str() {
            "EXTERNAL TABLE" => {
                push_unsupported(
                    &mut unsupported_objects,
                    &mut truncated,
                    rel.name,
                    "External table",
                    "External stage-backed Snowflake tables cannot be recreated by portable schema DDL.",
                );
                continue;
            }
            "EVENT TABLE" => {
                push_unsupported(
                    &mut unsupported_objects,
                    &mut truncated,
                    rel.name,
                    "Event table",
                    "Snowflake telemetry event tables are omitted from schema snapshots.",
                );
                continue;
            }
            "TEMPORARY TABLE" => {
                push_unsupported(
                    &mut unsupported_objects,
                    &mut truncated,
                    rel.name,
                    "Temporary table",
                    "Session-scoped Snowflake temporary tables are omitted from schema snapshots.",
                );
                continue;
            }
            "BASE TABLE" | "VIEW" | "MATERIALIZED VIEW" => {}
            _ => continue,
        }
        if rel.traits.dynamic == Some(true) {
            push_unsupported(
                &mut unsupported_objects,
                &mut truncated,
                rel.name,
                "Dynamic table",
                "Snowflake Dynamic Tables use declarative lag/warehouse refresh pipelines and are omitted from standard table DDL.",
            );
            continue;
        }
        if rel.traits.iceberg == Some(true) {
            push_unsupported(
                &mut unsupported_objects,
                &mut truncated,
                rel.name,
                "Iceberg table",
                "Snowflake Iceberg Tables depend on external volume/catalog bindings and are omitted from standard table DDL.",
            );
            continue;
        }
        if rel.traits.hybrid == Some(true) {
            push_unsupported(
                &mut unsupported_objects,
                &mut truncated,
                rel.name,
                "Hybrid table",
                "Snowflake Hybrid Tables are omitted from standard table DDL.",
            );
            continue;
        }
        if tables.len() >= MAX_SNAPSHOT_RELATIONS {
            truncated = true;
            break;
        }
        let kind = match rel.table_type.as_str() {
            "MATERIALIZED VIEW" => DatabaseObjectKind::MaterializedView,
            "VIEW" => DatabaseObjectKind::View,
            _ => DatabaseObjectKind::Table,
        };
        if matches!(
            kind,
            DatabaseObjectKind::View | DatabaseObjectKind::MaterializedView
        ) {
            view_targets.push(BrowseRelation {
                database: db.to_owned(),
                table: TableRef {
                    schema: schema.to_owned(),
                    name: rel.name.clone(),
                },
                table_type: rel.table_type.clone(),
                traits: rel.traits.clone(),
                availability: RelationAvailability::Available,
                created: None,
                last_ddl: None,
            });
        }
        let idx = tables.len();
        table_index.insert(rel.name.clone(), idx);
        tables.push(SnapshotTable {
            name: rel.name,
            kind,
            comment: rel.comment,
            partition_key: if kind == DatabaseObjectKind::Table {
                rel.clustering_key
            } else {
                None
            },
            view_definition: None,
            columns: Vec::new(),
            constraints: Vec::new(),
            indexes: Vec::new(),
            triggers: Vec::new(),
        });
    }

    let mut total_columns = 0usize;
    for row in columns_res.rows.iter().take(MAX_SNAPSHOT_COLUMNS + 1) {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let table_name = required_cell(&row[2])?;
        let Some(&t_idx) = table_index.get(table_name) else {
            continue;
        };
        if total_columns >= MAX_SNAPSHOT_COLUMNS {
            truncated = true;
            break;
        }
        let col_name = required_cell(&row[3])?.to_owned();
        metadata::quote_identifier(&col_name)?;
        let ordinal_position = required_cell(&row[4])?
            .parse::<u32>()
            .map_err(|_| malformed())?;
        let nullable = yes_no(&row[6])?.ok_or_else(malformed)?;
        let data_type = format_snowflake_column_type(
            required_cell(&row[7])?,
            row[8].as_deref(),
            row[9].as_deref(),
            row[10].as_deref(),
        );
        let is_identity = yes_no(&row[11])?.unwrap_or(false);
        let id_start = row[12].as_deref().filter(|s| !s.is_empty());
        let id_inc = row[13].as_deref().filter(|s| !s.is_empty());
        let expr = row[14].as_deref().filter(|s| !s.is_empty());
        let col_kind = required_cell(&row[15])?;

        let (identity, generated, default_expression) =
            if col_kind == "VIRTUAL_COLUMN" && expr.is_some() {
                (
                    None,
                    Some("VIRTUAL".to_owned()),
                    expr.map(|e| strip_active_database_qualifier(db, e)),
                )
            } else if is_identity {
                let id_spec = match (id_start, id_inc) {
                    (Some(s), Some(i)) => format!("START {s} INCREMENT {i}"),
                    _ => "IDENTITY".to_owned(),
                };
                (Some(id_spec), None, None)
            } else {
                (
                    None,
                    None,
                    row[5]
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(|d| strip_active_database_qualifier(db, d)),
                )
            };

        tables[t_idx].columns.push(SnapshotColumn {
            name: col_name,
            data_type,
            nullable,
            default_expression,
            identity,
            generated,
            ordinal_position,
        });
        total_columns += 1;
    }

    let mut total_constraints = 0usize;
    for row in constraints_res.rows.iter().take(SCHEMA_CONSTRAINT_LIMIT) {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let table_name = required_cell(&row[2])?;
        let Some(&t_idx) = table_index.get(table_name) else {
            continue;
        };
        if tables[t_idx].kind != DatabaseObjectKind::Table {
            continue;
        }
        if total_constraints >= MAX_SNAPSHOT_CONSTRAINTS {
            truncated = true;
            break;
        }
        let cname = required_cell(&row[3])?.to_owned();
        metadata::quote_identifier(&cname)?;
        let ctype = required_cell(&row[4])?;
        let key_lookup = (table_name.to_owned(), cname.clone());
        match ctype {
            "PRIMARY KEY" => {
                let Some(cols) = pk_groups.get(&key_lookup) else {
                    restricted = true;
                    continue;
                };
                let quoted_cols = cols
                    .iter()
                    .map(|c| quote_engine_identifier(EngineKind::Snowflake, c))
                    .collect::<Vec<_>>()
                    .join(", ");
                tables[t_idx].constraints.push(ConstraintInfo {
                    name: cname,
                    kind: ConstraintKind::PrimaryKey,
                    columns: cols.clone(),
                    target: None,
                    target_columns: Vec::new(),
                    definition: format!("PRIMARY KEY ({quoted_cols})"),
                });
                total_constraints += 1;
            }
            "UNIQUE" => {
                let Some(cols) = uq_groups.get(&key_lookup) else {
                    restricted = true;
                    continue;
                };
                let quoted_cols = cols
                    .iter()
                    .map(|c| quote_engine_identifier(EngineKind::Snowflake, c))
                    .collect::<Vec<_>>()
                    .join(", ");
                tables[t_idx].constraints.push(ConstraintInfo {
                    name: cname,
                    kind: ConstraintKind::Unique,
                    columns: cols.clone(),
                    target: None,
                    target_columns: Vec::new(),
                    definition: format!("UNIQUE ({quoted_cols})"),
                });
                total_constraints += 1;
            }
            "FOREIGN KEY" => {
                let Some(fk) = fk_groups.get(&key_lookup) else {
                    restricted = true;
                    continue;
                };
                if fk.target_database != db {
                    restricted = true;
                    push_unsupported(
                        &mut unsupported_objects,
                        &mut truncated,
                        format!("{table_name}.{cname}"),
                        "Cross-database foreign key",
                        "References another Snowflake database and cannot be retargeted within a single-database schema snapshot.",
                    );
                    continue;
                }
                let quoted_cols = fk
                    .columns
                    .iter()
                    .map(|c| quote_engine_identifier(EngineKind::Snowflake, c))
                    .collect::<Vec<_>>()
                    .join(", ");
                let quoted_ref_cols = fk
                    .target_columns
                    .iter()
                    .map(|c| quote_engine_identifier(EngineKind::Snowflake, c))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut def = format!(
                    "FOREIGN KEY ({quoted_cols}) REFERENCES {}.{} ({quoted_ref_cols})",
                    quote_engine_identifier(EngineKind::Snowflake, &fk.target.schema),
                    quote_engine_identifier(EngineKind::Snowflake, &fk.target.name),
                );
                if let Some(upd) = row[9]
                    .as_deref()
                    .filter(|r| !r.is_empty() && *r != "NO ACTION")
                {
                    def.push_str(&format!(" ON UPDATE {upd}"));
                }
                if let Some(del) = row[10]
                    .as_deref()
                    .filter(|r| !r.is_empty() && *r != "NO ACTION")
                {
                    def.push_str(&format!(" ON DELETE {del}"));
                }
                tables[t_idx].constraints.push(ConstraintInfo {
                    name: cname,
                    kind: ConstraintKind::ForeignKey,
                    columns: fk.columns.clone(),
                    target: Some(fk.target.clone()),
                    target_columns: fk.target_columns.clone(),
                    definition: def,
                });
                total_constraints += 1;
            }
            _ => {}
        }
    }

    for row in checks_res.rows.iter().take(SCHEMA_CHECK_LIMIT) {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let table_name = required_cell(&row[2])?;
        let Some(&t_idx) = table_index.get(table_name) else {
            continue;
        };
        if tables[t_idx].kind != DatabaseObjectKind::Table {
            continue;
        }
        if total_constraints >= MAX_SNAPSHOT_CONSTRAINTS {
            truncated = true;
            break;
        }
        let cname = required_cell(&row[3])?.to_owned();
        metadata::quote_identifier(&cname)?;
        let Some(raw_clause) = row[4].as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            restricted = true;
            continue;
        };
        let stripped = strip_active_database_qualifier(db, raw_clause);
        let definition = if stripped.len() >= 5 && stripped[..5].eq_ignore_ascii_case("CHECK") {
            stripped
        } else {
            format!("CHECK ({stripped})")
        };
        tables[t_idx].constraints.push(ConstraintInfo {
            name: cname,
            kind: ConstraintKind::Check,
            columns: Vec::new(),
            target: None,
            target_columns: Vec::new(),
            definition,
        });
        total_constraints += 1;
    }

    let mut sequences = Vec::with_capacity(sequences_res.rows.len().min(MAX_SNAPSHOT_SEQUENCES));
    for row in sequences_res.rows.iter().take(MAX_SNAPSHOT_SEQUENCES) {
        if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
            return Err(malformed());
        }
        let name = required_cell(&row[2])?.to_owned();
        metadata::quote_identifier(&name)?;
        sequences.push(SnapshotSequence {
            name,
            data_type: required_cell(&row[3])?.to_owned(),
            start_value: required_cell(&row[4])?.to_owned(),
            min_value: required_cell(&row[5])?.to_owned(),
            max_value: required_cell(&row[6])?.to_owned(),
            increment_by: required_cell(&row[7])?.to_owned(),
            cache_size: String::new(),
            cycle: yes_no(&row[8])?.unwrap_or(false),
        });
    }

    let mut routines = Vec::new();
    for (is_procedure, res) in [(false, functions_res), (true, procedures_res)] {
        for row in res.rows.iter() {
            if routines.len() >= MAX_SNAPSHOT_ROUTINES {
                truncated = true;
                break;
            }
            if required_cell(&row[0])? != db || required_cell(&row[1])? != schema {
                return Err(malformed());
            }
            let name = required_cell(&row[2])?.to_owned();
            metadata::quote_identifier(&name)?;
            let arg_sig = required_cell(&row[3])?;
            let identity_arguments = normalize_argument_types(arg_sig)?;
            let arguments = arg_sig
                .trim()
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or(arg_sig)
                .trim()
                .to_owned();
            let return_type = Some(required_cell(&row[4])?.to_owned());
            let language = row[5]
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("SQL")
                .to_owned();
            let (volatility, comment) = if is_procedure {
                (None, row[6].clone().filter(|s| !s.is_empty()))
            } else {
                let vol = match yes_no(&row[6])? {
                    Some(true) => Some("VOLATILE".to_owned()),
                    Some(false) => Some("IMMUTABLE".to_owned()),
                    None => None,
                };
                (vol, row[7].clone().filter(|s| !s.is_empty()))
            };
            routines.push(SnapshotRoutine {
                name,
                kind: if is_procedure {
                    DatabaseObjectKind::Procedure
                } else {
                    DatabaseObjectKind::Function
                },
                identity_arguments,
                arguments,
                return_type,
                language,
                volatility,
                security_definer: false,
                definition: None,
                comment,
            });
        }
    }

    Ok(SnapshotRelationPlan {
        snapshot: SchemaSnapshot {
            engine: EngineKind::Snowflake,
            server_version: server_version.to_owned(),
            database: db.to_owned(),
            schema: schema.to_owned(),
            role,
            tables,
            routines,
            sequences,
            unsupported_objects,
            truncated,
            restricted,
        },
        view_targets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ColumnInfo, ColumnKind};
    use crate::drivers::snowflake_metadata::{ResolvedSchema, SecondaryRoles};

    fn cols(names: &[&str]) -> Vec<ColumnInfo> {
        names
            .iter()
            .map(|name| ColumnInfo {
                id: (*name).to_owned(),
                name: (*name).to_owned(),
                database_type: "TEXT".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: false,
            })
            .collect()
    }

    fn sample_context() -> LiveContext {
        LiveContext {
            principal: "USER1".into(),
            role: Some("ANALYST".into()),
            database: Some("DB".into()),
            schema: Some("PUBLIC".into()),
            warehouse: Some("WH".into()),
            session_id: "101".into(),
            search_path: vec![ResolvedSchema {
                raw: "DB.PUBLIC".into(),
                database: Some("DB".into()),
                schema: Some("PUBLIC".into()),
            }],
            secondary_roles: SecondaryRoles {
                roles: String::new(),
                value: "ALL".into(),
            },
            quoted_identifiers_ignore_case: false,
        }
    }

    #[test]
    fn normalizes_overloaded_routine_signatures_and_rejects_injection() {
        assert_eq!(normalize_argument_types("()").unwrap(), "");
        assert_eq!(
            normalize_argument_types("(X NUMBER, Y VARCHAR(100), Z TIMESTAMP_LTZ(9))").unwrap(),
            "NUMBER, VARCHAR(100), TIMESTAMP_LTZ(9)"
        );
        assert_eq!(
            normalize_argument_types("(AMOUNT NUMBER(18, 2), PAYLOAD VARIANT)").unwrap(),
            "NUMBER(18, 2), VARIANT"
        );
        assert!(normalize_argument_types("('); DROP TABLE T; --)").is_err());
        assert!(normalize_argument_types("X NUMBER").is_err());
        assert_eq!(
            routine_ddl_sql("DB", "PUBLIC", "CALC", false, "NUMBER, VARCHAR").unwrap(),
            "SELECT GET_DDL('FUNCTION', '\"DB\".\"PUBLIC\".\"CALC\"(NUMBER, VARCHAR)', TRUE) AS DDL"
        );
    }

    #[test]
    fn parses_tables_sequences_routines_and_extracts_view_dependencies() {
        let ctx = sample_context();
        let tables = parse_tables(
            &ctx,
            "PUBLIC",
            &MetadataResult {
                columns: cols(&EXPLORER_TABLE_COLUMNS),
                rows: vec![
                    vec![
                        Some("DB".into()),
                        Some("PUBLIC".into()),
                        Some("ORDERS".into()),
                        Some("BASE TABLE".into()),
                        Some("42".into()),
                        Some("8192".into()),
                        Some("Order ledger".into()),
                    ],
                    vec![
                        Some("DB".into()),
                        Some("PUBLIC".into()),
                        Some("ACTIVE_ORDERS".into()),
                        Some("VIEW".into()),
                        None,
                        None,
                        None,
                    ],
                    vec![
                        Some("DB".into()),
                        Some("PUBLIC".into()),
                        Some("ORDER_ROLLUP".into()),
                        Some("MATERIALIZED VIEW".into()),
                        Some("10".into()),
                        Some("2048".into()),
                        None,
                    ],
                ],
                rows_seen: 3,
                truncated: false,
            },
        )
        .unwrap();
        assert_eq!(tables.len(), 3);
        assert_eq!(tables[0].object.kind, DatabaseObjectKind::Table);
        assert_eq!(tables[0].estimated_rows, Some(42.0));
        assert_eq!(tables[0].total_bytes, Some(8192.0));
        assert_eq!(tables[1].object.kind, DatabaseObjectKind::View);
        assert_eq!(tables[2].object.kind, DatabaseObjectKind::MaterializedView);

        let deps = extract_view_dependencies(
            "DB",
            "PUBLIC",
            "ACTIVE_ORDERS",
            "CREATE OR REPLACE VIEW \"DB\".\"PUBLIC\".\"ACTIVE_ORDERS\" AS WITH c AS (SELECT * FROM \"PUBLIC\".\"ORDERS\") SELECT * FROM c",
            &tables,
        );
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].object.name, "ORDERS");
    }

    #[test]
    fn parses_schema_diagram_and_snapshot_with_constraints_and_disclosures() {
        let ctx = sample_context();
        let row_of = |items: &[Option<&str>]| -> Vec<Option<String>> {
            items.iter().map(|s| s.map(str::to_owned)).collect()
        };
        let relations = MetadataResult {
            columns: cols(&SCHEMA_RELATION_COLUMNS),
            rows: vec![
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ORDERS"),
                    Some("BASE TABLE"),
                    Some("NO"),
                    Some("NO"),
                    Some("NO"),
                    Some("NO"),
                    Some("NO"),
                    Some("NO"),
                    Some("LINEAR(CREATED_AT)"),
                    Some("Order ledger"),
                ]),
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ACTIVE_ORDERS"),
                    Some("VIEW"),
                    Some("NO"),
                    Some("NO"),
                    Some("NO"),
                    Some("NO"),
                    Some("NO"),
                    Some("NO"),
                    None,
                    Some("Active orders view"),
                ]),
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("DYN_ORDERS"),
                    Some("BASE TABLE"),
                    Some("NO"),
                    Some("NO"),
                    Some("NO"),
                    Some("YES"),
                    Some("NO"),
                    Some("NO"),
                    None,
                    None,
                ]),
            ],
            rows_seen: 3,
            truncated: false,
        };
        let columns = MetadataResult {
            columns: cols(&SCHEMA_COLUMN_COLUMNS),
            rows: vec![
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ORDERS"),
                    Some("ID"),
                    Some("1"),
                    None,
                    Some("NO"),
                    Some("NUMBER"),
                    None,
                    Some("38"),
                    Some("0"),
                    Some("YES"),
                    Some("1"),
                    Some("1"),
                    None,
                    Some("COLUMN"),
                ]),
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ORDERS"),
                    Some("CUSTOMER_ID"),
                    Some("2"),
                    None,
                    Some("NO"),
                    Some("NUMBER"),
                    None,
                    Some("38"),
                    Some("0"),
                    Some("NO"),
                    None,
                    None,
                    None,
                    Some("COLUMN"),
                ]),
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ORDERS"),
                    Some("TOTAL_CENTS"),
                    Some("3"),
                    None,
                    Some("YES"),
                    Some("NUMBER"),
                    None,
                    Some("18"),
                    Some("0"),
                    Some("NO"),
                    None,
                    None,
                    Some("\"CUSTOMER_ID\" * 100"),
                    Some("VIRTUAL_COLUMN"),
                ]),
            ],
            rows_seen: 3,
            truncated: false,
        };
        let pks = MetadataResult {
            columns: cols(&SCHEMA_KEY_COLUMNS),
            rows: vec![row_of(&[
                Some("DB"),
                Some("PUBLIC"),
                Some("ORDERS"),
                Some("ID"),
                Some("1"),
                Some("ORDERS_PK"),
            ])],
            rows_seen: 1,
            truncated: false,
        };
        let uqs = MetadataResult {
            columns: cols(&SCHEMA_KEY_COLUMNS),
            rows: vec![],
            rows_seen: 0,
            truncated: false,
        };
        let fks = MetadataResult {
            columns: cols(&SCHEMA_FOREIGN_COLUMNS),
            rows: vec![
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ORDERS"),
                    Some("CUSTOMER_ID"),
                    Some("1"),
                    Some("ORDERS_CUSTOMER_FK"),
                    Some("DB"),
                    Some("CORE"),
                    Some("CUSTOMERS"),
                    Some("ID"),
                    Some("CUSTOMERS_PK"),
                ]),
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ORDERS"),
                    Some("CUSTOMER_ID"),
                    Some("1"),
                    Some("ORDERS_EXT_DB_FK"),
                    Some("OTHER_DB"),
                    Some("CORE"),
                    Some("CUSTOMERS"),
                    Some("ID"),
                    Some("CUSTOMERS_PK"),
                ]),
            ],
            rows_seen: 2,
            truncated: false,
        };
        let diagram = parse_schema_diagram(&ctx, "PUBLIC", &relations, &columns, &pks, &fks)
            .expect("diagram parses");
        assert!(!diagram.truncated);
        assert!(diagram.restricted, "cross-db FK marks restricted");
        assert_eq!(diagram.tables.len(), 3);
        assert_eq!(diagram.foreign_keys.len(), 1);
        assert_eq!(diagram.foreign_keys[0].target.schema, "CORE");
        assert_eq!(diagram.foreign_keys[0].target.name, "CUSTOMERS");

        let constraints = MetadataResult {
            columns: cols(&SCHEMA_CONSTRAINT_COLUMNS),
            rows: vec![
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ORDERS"),
                    Some("ORDERS_PK"),
                    Some("PRIMARY KEY"),
                    Some("NO"),
                    Some("YES"),
                    None,
                    None,
                    None,
                    None,
                ]),
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ORDERS"),
                    Some("ORDERS_CUSTOMER_FK"),
                    Some("FOREIGN KEY"),
                    Some("NO"),
                    Some("NO"),
                    Some("DB"),
                    Some("CORE"),
                    Some("NO ACTION"),
                    Some("NO ACTION"),
                ]),
                row_of(&[
                    Some("DB"),
                    Some("PUBLIC"),
                    Some("ORDERS"),
                    Some("ORDERS_EXT_DB_FK"),
                    Some("FOREIGN KEY"),
                    Some("NO"),
                    Some("NO"),
                    Some("OTHER_DB"),
                    Some("CORE"),
                    Some("NO ACTION"),
                    Some("NO ACTION"),
                ]),
            ],
            rows_seen: 3,
            truncated: false,
        };
        let checks = MetadataResult {
            columns: cols(&SCHEMA_CHECK_COLUMNS),
            rows: vec![row_of(&[
                Some("DB"),
                Some("PUBLIC"),
                Some("ORDERS"),
                Some("ORDERS_POS_CHK"),
                Some("\"ID\" > 0"),
            ])],
            rows_seen: 1,
            truncated: false,
        };
        let seqs = MetadataResult {
            columns: cols(&SEQUENCE_COLUMNS),
            rows: vec![row_of(&[
                Some("DB"),
                Some("PUBLIC"),
                Some("ORDER_SEQ"),
                Some("NUMBER"),
                Some("1"),
                Some("1"),
                Some("999999"),
                Some("1"),
                Some("NO"),
                None,
            ])],
            rows_seen: 1,
            truncated: false,
        };
        let fns = MetadataResult {
            columns: cols(&FUNCTION_COLUMNS),
            rows: vec![row_of(&[
                Some("DB"),
                Some("PUBLIC"),
                Some("CALC_TAX"),
                Some("(AMOUNT NUMBER)"),
                Some("NUMBER(18, 2)"),
                Some("SQL"),
                Some("NO"),
                Some("Tax helper"),
            ])],
            rows_seen: 1,
            truncated: false,
        };
        let procs = MetadataResult {
            columns: cols(&PROCEDURE_COLUMNS),
            rows: vec![],
            rows_seen: 0,
            truncated: false,
        };

        let mut plan = parse_schema_snapshot_base(
            &ctx,
            "8.42.0",
            "PUBLIC",
            &relations,
            &columns,
            &constraints,
            &checks,
            &pks,
            &uqs,
            &fks,
            &seqs,
            &fns,
            &procs,
        )
        .expect("snapshot base parses");
        plan.snapshot.canonicalize();
        assert_eq!(plan.view_targets.len(), 1);
        assert_eq!(plan.snapshot.tables.len(), 2);
        assert!(plan.snapshot.restricted);
        assert_eq!(plan.snapshot.unsupported_objects.len(), 2);
        let orders = plan
            .snapshot
            .tables
            .iter()
            .find(|t| t.name == "ORDERS")
            .unwrap();
        assert_eq!(orders.partition_key.as_deref(), Some("CREATED_AT"));
        assert_eq!(orders.columns.len(), 3);
        assert_eq!(
            orders.columns[0].identity.as_deref(),
            Some("START 1 INCREMENT 1")
        );
        assert_eq!(orders.columns[2].generated.as_deref(), Some("VIRTUAL"));
        assert_eq!(orders.constraints.len(), 3);
        assert_eq!(plan.snapshot.routines.len(), 1);
        assert_eq!(plan.snapshot.routines[0].arguments, "AMOUNT NUMBER");
        assert_eq!(
            strip_active_database_qualifier(
                "DB",
                "CREATE OR REPLACE VIEW \"DB\".\"PUBLIC\".\"ACTIVE_ORDERS\" AS SELECT * FROM DB.PUBLIC.ORDERS"
            ),
            "CREATE OR REPLACE VIEW \"PUBLIC\".\"ACTIVE_ORDERS\" AS SELECT * FROM PUBLIC.ORDERS"
        );
    }

    #[test]
    fn parses_dynamic_and_external_table_metadata_and_post_execution_operator_profile() {
        let dyn_res = MetadataResult {
            columns: vec![
                ColumnInfo {
                    id: "c0".into(),
                    name: "name".into(),
                    database_type: "TEXT".into(),
                    kind: ColumnKind::Text,
                    nullable: false,
                    primary_key: false,
                    foreign_key: false,
                    editable: false,
                    insertable: false,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c1".into(),
                    name: "target_lag".into(),
                    database_type: "TEXT".into(),
                    kind: ColumnKind::Text,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: false,
                    insertable: false,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c2".into(),
                    name: "warehouse".into(),
                    database_type: "TEXT".into(),
                    kind: ColumnKind::Text,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: false,
                    insertable: false,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c3".into(),
                    name: "refresh_mode".into(),
                    database_type: "TEXT".into(),
                    kind: ColumnKind::Text,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: false,
                    insertable: false,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c4".into(),
                    name: "scheduling_state".into(),
                    database_type: "TEXT".into(),
                    kind: ColumnKind::Text,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: false,
                    insertable: false,
                    has_default: false,
                },
            ],
            rows: vec![vec![
                Some("LIVE_ROLLUP".into()),
                Some("5 minutes".into()),
                Some("WH_PIPE".into()),
                Some("INCREMENTAL".into()),
                Some("ACTIVE".into()),
            ]],
            rows_seen: 1,
            truncated: false,
        };
        let dyn_meta = parse_dynamic_table_meta("LIVE_ROLLUP", &dyn_res);
        assert_eq!(dyn_meta.target_lag.as_deref(), Some("5 minutes"));
        assert_eq!(dyn_meta.warehouse.as_deref(), Some("WH_PIPE"));
        assert_eq!(dyn_meta.refresh_mode.as_deref(), Some("INCREMENTAL"));
        assert_eq!(dyn_meta.scheduling_state.as_deref(), Some("ACTIVE"));

        let op_cols = (0..14)
            .map(|i| ColumnInfo {
                id: format!("c{i}"),
                name: format!("C{i}"),
                database_type: "TEXT".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: false,
            })
            .collect();
        let op_res = MetadataResult {
            columns: op_cols,
            rows: vec![
                vec![
                    Some("1".into()),
                    Some("0".into()),
                    Some("[]".into()),
                    Some("Result".into()),
                    Some("0.05".into()),
                    Some("100".into()),
                    Some("100".into()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some("{}".into()),
                ],
                vec![
                    Some("1".into()),
                    Some("1".into()),
                    Some("[0]".into()),
                    Some("Join".into()),
                    Some("0.72".into()),
                    Some("500000".into()),
                    Some("100".into()),
                    Some("1048576".into()),
                    Some("0.85".into()),
                    Some("2".into()),
                    Some("16".into()),
                    Some("65536".into()),
                    Some("0".into()),
                    Some("{\"joinType\":\"INNER\"}".into()),
                ],
            ],
            rows_seen: 2,
            truncated: false,
        };
        let prof = parse_operator_profile("01b77a9e-0000-1111-2222-333344445555", &op_res).unwrap();
        assert_eq!(prof.operators.len(), 2);
        assert_eq!(prof.bottleneck_operator_id, Some(1));
        assert_eq!(prof.total_spilled_local_bytes, 65536);
        assert_eq!(prof.operators[1].parent_operators, vec![0]);
    }
}
