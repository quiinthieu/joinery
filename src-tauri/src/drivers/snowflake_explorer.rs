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
    DatabaseObjectSummary, DependencyEdge, ErrorCode, ObjectDdlStatus, ObjectDetails, RoutineInfo,
    SequenceInfo, TableRef, TableStorageStats,
};
use crate::relation_inspection::{
    RelationAvailability, RelationCheckConstraint, RelationConstraintMapping,
    RelationKeyConstraint, RelationKeyConstraintKind, RelationTraits,
};
use sqlparser::{
    ast::{Visit, Visitor},
    dialect::SnowflakeDialect,
    parser::Parser,
};
use std::collections::HashSet;
use std::ops::ControlFlow;

pub(crate) const OBJECT_LIMIT: usize = 10_000;
pub(crate) const OBJECT_MAX_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const DETAIL_MAX_BYTES: usize = 1024 * 1024;

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
        let kind = match required_cell(&row[3])? {
            "MATERIALIZED VIEW" => DatabaseObjectKind::MaterializedView,
            "VIEW" => DatabaseObjectKind::View,
            "BASE TABLE" | "TEMPORARY TABLE" | "EXTERNAL TABLE" | "EVENT TABLE" => {
                DatabaseObjectKind::Table
            }
            _ => continue,
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
            signature: None,
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

    let Ok(statements) = Parser::new(&SnowflakeDialect {})
        .with_recursion_limit(32)
        .try_with_sql(ddl)
        .and_then(|mut parser| parser.parse_statements())
    else {
        return Vec::new();
    };
    let mut visitor = RelationVisitor {
        refs: Vec::new(),
        ctes: HashSet::new(),
    };
    for stmt in &statements {
        if let sqlparser::ast::Statement::CreateView(create) = stmt {
            let _ = create.query.visit(&mut visitor);
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
        uses: Vec::new(),
        used_by: Vec::new(),
    }
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
}
