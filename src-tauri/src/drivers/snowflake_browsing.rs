//! Pure current-database browsing. Callers retain session/context/generation
//! ownership, serialize operations and bracket reads against observed drift.
//! These checks do not make name-based reads atomic against external DDL.

use super::snowflake_metadata::{self as metadata, LiveContext, MetadataResult};
use crate::domain::{AppError, AppResult, ColumnInfo, ColumnKind, ErrorCode, TableRef};
use crate::relation_inspection::{
    ColumnIdentity, ColumnInspection, InspectedColumnKind, RelationAvailability, RelationPreview,
    RelationTargetSummary, RelationTraits, WriteDefaultObservationStatus,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

pub(crate) const SCHEMA_LIMIT: usize = 1000;
pub(crate) const RELATION_LIMIT: usize = 10_000;
pub(crate) const CATALOG_MAX_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const COLUMN_LIMIT: usize = 1000;
pub(crate) const COLUMN_MAX_BYTES: usize = 1024 * 1024;
pub(crate) const PREVIEW_LIMIT: usize = 500;
pub(crate) const PREVIEW_MAX_BYTES: usize = 2 * 1024 * 1024;

const RELATION_COLUMNS: [&str; 12] = [
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
    "CREATED",
    "LAST_DDL",
];
const COLUMN_COLUMNS: [&str; 22] = [
    "TABLE_CATALOG",
    "TABLE_SCHEMA",
    "TABLE_NAME",
    "COLUMN_NAME",
    "ORDINAL_POSITION",
    "COLUMN_DEFAULT",
    "IS_NULLABLE",
    "DATA_TYPE",
    "DATA_TYPE_ALIAS",
    "CHARACTER_MAXIMUM_LENGTH",
    "CHARACTER_OCTET_LENGTH",
    "NUMERIC_PRECISION",
    "NUMERIC_PRECISION_RADIX",
    "NUMERIC_SCALE",
    "IS_IDENTITY",
    "IDENTITY_GENERATION",
    "IDENTITY_START",
    "IDENTITY_INCREMENT",
    "IDENTITY_ORDERED",
    "COMMENT",
    "EXPRESSION",
    "KIND",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BrowseRelation {
    pub database: String,
    pub table: TableRef,
    pub table_type: String,
    pub traits: RelationTraits,
    pub availability: RelationAvailability,
    /// Best-effort drift indicators, never immutable object IDs.
    pub created: Option<String>,
    pub last_ddl: Option<String>,
}

impl BrowseRelation {
    pub(crate) fn summary(&self, target_id: String) -> RelationTargetSummary {
        RelationTargetSummary {
            target_id,
            database: self.database.clone(),
            table: self.table.clone(),
            table_type: self.table_type.clone(),
            traits: self.traits.clone(),
            availability: self.availability,
        }
    }
}

pub(crate) fn schemas_sql(context: &LiveContext) -> AppResult<String> {
    metadata::schemas_sql(context)
}

pub(crate) fn parse_schemas(
    context: &LiveContext,
    result: &MetadataResult,
) -> AppResult<Vec<String>> {
    metadata::parse_schemas(context, result)
}

pub(crate) fn relations_sql(context: &LiveContext, schema: &str) -> AppResult<String> {
    let database = database(context)?;
    user_schema(schema)?;
    Ok(format!(
        "SELECT {}, CREATED::VARCHAR AS CREATED, LAST_DDL::VARCHAR AS LAST_DDL FROM {}.INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = {} AND TABLE_SCHEMA = {} ORDER BY TABLE_NAME, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, IS_ICEBERG, IS_DYNAMIC, IS_HYBRID, IS_IMMUTABLE, CREATED, LAST_DDL LIMIT {}",
        RELATION_COLUMNS[..10].join(", "),
        metadata::quote_identifier(database)?,
        literal(database)?,
        literal(schema)?,
        RELATION_LIMIT + 1,
    ))
}

pub(crate) fn parse_relations(
    context: &LiveContext,
    schema: &str,
    result: &MetadataResult,
) -> AppResult<Vec<BrowseRelation>> {
    let database = database(context)?;
    user_schema(schema)?;
    exact(result, &RELATION_COLUMNS, RELATION_LIMIT, CATALOG_MAX_BYTES)?;
    let mut relations = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        if identifier(&row[0])? != database || identifier(&row[1])? != schema {
            return Err(malformed());
        }
        let table_type = text(required(&row[3])?, 128, false)?.to_owned();
        let traits = RelationTraits {
            transient: yes_no(&row[4])?,
            temporary: yes_no(&row[5])?,
            iceberg: yes_no(&row[6])?,
            dynamic: yes_no(&row[7])?,
            hybrid: yes_no(&row[8])?,
            immutable: yes_no(&row[9])?,
        };
        let availability = initial_availability(&table_type, &traits);
        relations.push(BrowseRelation {
            database: database.to_owned(),
            table: TableRef {
                schema: schema.to_owned(),
                name: identifier(&row[2])?.to_owned(),
            },
            table_type,
            traits,
            availability,
            created: optional_text(&row[10], 128)?,
            last_ddl: optional_text(&row[11], 128)?,
        });
    }
    classify_collisions(&mut relations)?;
    bounded(&relations, CATALOG_MAX_BYTES)?;
    Ok(relations)
}

pub(crate) fn relation_sql(context: &LiveContext, target: &BrowseRelation) -> AppResult<String> {
    usable(target)?;
    if database(context)? != target.database {
        return Err(ambiguous());
    }
    Ok(format!(
        "SELECT {}, CREATED::VARCHAR AS CREATED, LAST_DDL::VARCHAR AS LAST_DDL FROM {}.INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = {} AND TABLE_SCHEMA = {} AND TABLE_NAME = {} ORDER BY TABLE_TYPE, IS_TEMPORARY LIMIT 3",
        RELATION_COLUMNS[..10].join(", "),
        metadata::quote_identifier(&target.database)?,
        literal(&target.database)?,
        literal(&target.table.schema)?,
        literal(&target.table.name)?,
    ))
}

/// One exact resolved candidate is required both before and after inspection
/// or preview. Timestamps detect some changes, not atomic object identity.
pub(crate) fn require_relation(
    context: &LiveContext,
    target: &BrowseRelation,
    result: &MetadataResult,
) -> AppResult<()> {
    usable(target)?;
    if database(context)? != target.database {
        return Err(ambiguous());
    }
    complete(result, 2, CATALOG_MAX_BYTES)?;
    let observed = parse_relations(context, &target.table.schema, result)?;
    if observed.len() != 1 || observed[0] != *target {
        return Err(ambiguous());
    }
    Ok(())
}

/// Validate the complete aggregate, not just each per-schema response. Call only
/// after all schema reads succeeded; permission errors are never empty success.
pub(crate) fn validate_catalog(
    context: &LiveContext,
    schemas: &[String],
    relations: &mut [BrowseRelation],
) -> AppResult<()> {
    let database = database(context)?;
    if schemas.len() > SCHEMA_LIMIT || relations.len() > RELATION_LIMIT {
        return Err(limit());
    }
    let mut seen = HashSet::new();
    for schema in schemas {
        user_schema(schema)?;
        if !seen.insert(schema.as_str()) {
            return Err(malformed());
        }
    }
    for relation in relations.iter() {
        if relation.database != database || !seen.contains(relation.table.schema.as_str()) {
            return Err(malformed());
        }
        metadata::quote_identifier(&relation.table.name)?;
        text(&relation.table_type, 128, false)?;
    }
    classify_collisions(relations)?;
    bounded(&(schemas, relations), CATALOG_MAX_BYTES)
}

fn initial_availability(kind: &str, traits: &RelationTraits) -> RelationAvailability {
    if !matches!(
        kind,
        "BASE TABLE"
            | "TEMPORARY TABLE"
            | "EXTERNAL TABLE"
            | "EVENT TABLE"
            | "VIEW"
            | "MATERIALIZED VIEW"
    ) {
        return RelationAvailability::UnsupportedKind;
    }
    if [
        traits.transient,
        traits.temporary,
        traits.iceberg,
        traits.dynamic,
        traits.hybrid,
        traits.immutable,
    ]
    .contains(&None)
    {
        return RelationAvailability::UnknownTraits;
    }
    if (kind == "TEMPORARY TABLE") != (traits.temporary == Some(true)) {
        return RelationAvailability::Ambiguous;
    }
    RelationAvailability::Available
}

fn classify_collisions(relations: &mut [BrowseRelation]) -> AppResult<()> {
    let mut groups: HashMap<(&str, &str, &str), Vec<usize>> = HashMap::new();
    for (index, relation) in relations.iter().enumerate() {
        groups
            .entry((
                &relation.database,
                &relation.table.schema,
                &relation.table.name,
            ))
            .or_default()
            .push(index);
    }
    let collisions: Vec<Vec<usize>> = groups
        .into_values()
        .filter(|group| group.len() > 1)
        .collect();
    for group in collisions {
        // Identical rows cannot be assigned independent trustworthy targets.
        for (position, index) in group.iter().enumerate() {
            if group[..position]
                .iter()
                .any(|other| relations[*other] == relations[*index])
            {
                return Err(malformed());
            }
        }
        let has_temporary = group
            .iter()
            .any(|index| relations[*index].traits.temporary == Some(true));
        for index in group {
            relations[index].availability =
                if has_temporary && relations[index].traits.temporary == Some(false) {
                    RelationAvailability::Shadowed
                } else {
                    RelationAvailability::Ambiguous
                };
        }
    }
    Ok(())
}

fn usable(target: &BrowseRelation) -> AppResult<()> {
    if target.availability != RelationAvailability::Available
        || initial_availability(&target.table_type, &target.traits)
            != RelationAvailability::Available
    {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Snowflake relation inspection is unavailable or ambiguous",
        ));
    }
    metadata::quote_identifier(&target.database)?;
    user_schema(&target.table.schema)?;
    metadata::quote_identifier(&target.table.name)?;
    Ok(())
}

fn qualified(target: &BrowseRelation) -> AppResult<String> {
    usable(target)?;
    Ok(format!(
        "{}.{}.{}",
        metadata::quote_identifier(&target.database)?,
        metadata::quote_identifier(&target.table.schema)?,
        metadata::quote_identifier(&target.table.name)?
    ))
}

pub(crate) fn columns_sql(target: &BrowseRelation) -> AppResult<String> {
    usable(target)?;
    let projection = COLUMN_COLUMNS
        .iter()
        .map(|name| {
            if matches!(
                *name,
                "ORDINAL_POSITION"
                    | "CHARACTER_MAXIMUM_LENGTH"
                    | "CHARACTER_OCTET_LENGTH"
                    | "NUMERIC_PRECISION"
                    | "NUMERIC_PRECISION_RADIX"
                    | "NUMERIC_SCALE"
            ) {
                format!("{name}::VARCHAR AS {name}")
            } else {
                (*name).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    // The projected ordinal is VARCHAR; order by the numeric source, not its
    // alias (which would place column 10 before column 2).
    Ok(format!(
        "SELECT {projection} FROM {}.INFORMATION_SCHEMA.COLUMNS AS C WHERE TABLE_CATALOG = {} AND TABLE_SCHEMA = {} AND TABLE_NAME = {} ORDER BY C.ORDINAL_POSITION LIMIT {}",
        metadata::quote_identifier(&target.database)?,
        literal(&target.database)?,
        literal(&target.table.schema)?,
        literal(&target.table.name)?,
        COLUMN_LIMIT + 1
    ))
}

pub(crate) fn describe_sql(target: &BrowseRelation) -> AppResult<String> {
    let name = qualified(target)?;
    // Choose the documented relation-specific descriptor. No fallback replay.
    Ok(if target.traits.dynamic == Some(true) {
        format!("DESCRIBE DYNAMIC TABLE {name}")
    } else {
        match target.table_type.as_str() {
            "VIEW" => format!("DESCRIBE VIEW {name}"),
            "MATERIALIZED VIEW" => format!("DESCRIBE MATERIALIZED VIEW {name}"),
            "EXTERNAL TABLE" => format!("DESCRIBE EXTERNAL TABLE {name} TYPE = COLUMNS"),
            "EVENT TABLE" => format!("DESCRIBE EVENT TABLE {name}"),
            _ if target.traits.iceberg == Some(true) => format!("DESCRIBE ICEBERG TABLE {name}"),
            _ => format!("DESCRIBE TABLE {name} TYPE = COLUMNS"),
        }
    })
}

pub(crate) fn resolved_columns_sql(target: &BrowseRelation) -> AppResult<String> {
    Ok(format!("SELECT * FROM {} LIMIT 0", qualified(target)?))
}

pub(crate) fn parse_inspection(
    target: &BrowseRelation,
    columns: &MetadataResult,
    describe: &MetadataResult,
    resolved: &MetadataResult,
) -> AppResult<Vec<ColumnInspection>> {
    usable(target)?;
    exact(columns, &COLUMN_COLUMNS, COLUMN_LIMIT, COLUMN_MAX_BYTES)?;
    complete(describe, COLUMN_LIMIT, COLUMN_MAX_BYTES)?;
    complete(resolved, 0, COLUMN_MAX_BYTES)?;
    if columns.rows.is_empty()
        || columns.rows.len() != describe.rows.len()
        || columns.rows.len() != resolved.columns.len()
    {
        return Err(malformed());
    }
    let fields = [
        "name",
        "type",
        "kind",
        "null?",
        "default",
        "expression",
        "comment",
    ];
    let positions = named_fields(describe, &fields)?;
    // Optional across descriptor versions. Presence, even SQL NULL, is an
    // observation; absence must not be presented as an empty write default.
    let write_default_position = describe
        .columns
        .iter()
        .position(|column| column.name.eq_ignore_ascii_case("write default"));
    if let Some(position) = write_default_position {
        let column = &describe.columns[position];
        if column.name != "write default" || column.kind != ColumnKind::Text {
            return Err(malformed());
        }
    }
    let mut names = HashSet::new();
    let mut prior_ordinal = 0;
    let mut parsed = Vec::with_capacity(columns.rows.len());
    for (index, row) in columns.rows.iter().enumerate() {
        if identifier(&row[0])? != target.database
            || identifier(&row[1])? != target.table.schema
            || identifier(&row[2])? != target.table.name
        {
            return Err(malformed());
        }
        let name = identifier(&row[3])?;
        if !names.insert(name) {
            return Err(ambiguous());
        }
        let ordinal = unsigned(required(&row[4])?)?
            .parse::<u32>()
            .map_err(|_| malformed())?;
        if ordinal <= prior_ordinal {
            return Err(malformed());
        }
        prior_ordinal = ordinal;
        let nullable = required_yes_no(&row[6])?;
        let data_type = text(required(&row[7])?, 1024, false)?.to_owned();
        let kind = match required(&row[21])? {
            "COLUMN" => InspectedColumnKind::Column,
            "VIRTUAL_COLUMN" => InspectedColumnKind::VirtualColumn,
            "PARTITION_COLUMN" => InspectedColumnKind::PartitionColumn,
            _ => return Err(malformed()),
        };
        let described = &describe.rows[index];
        let at = |field: usize| &described[positions[field]];
        let full_type = text(required(at(1))?, 4096, false)?;
        let described_kind = match required(at(2))? {
            "COLUMN" => InspectedColumnKind::Column,
            "VIRTUAL" | "VIRTUAL_COLUMN" => InspectedColumnKind::VirtualColumn,
            "PARTITION_COLUMN" => InspectedColumnKind::PartitionColumn,
            _ => return Err(malformed()),
        };
        let described_nullable = match required(at(3))? {
            "Y" => true,
            "N" => false,
            _ => return Err(malformed()),
        };
        if required(at(0))? != name
            || described_kind != kind
            || described_nullable != nullable
            || (target.traits.iceberg != Some(true) && at(4) != &row[5])
            || at(5) != &row[20]
            || at(6) != &row[19]
        {
            return Err(ambiguous());
        }
        let wire = &resolved.columns[index];
        if wire.name != name
            || !compatible_types(&data_type, full_type, &wire.database_type)
            || !compatible_kind(&wire.database_type, wire.kind)
        {
            return Err(ambiguous());
        }
        let identity = ColumnIdentity {
            is_identity: yes_no(&row[14])?,
            generation: optional_text(&row[15], 128)?,
            start: optional_signed(&row[16])?,
            increment: optional_signed(&row[17])?,
            ordered: yes_no(&row[18])?,
        };
        if identity
            .generation
            .as_deref()
            .is_some_and(|value| value != "BY DEFAULT")
        {
            return Err(malformed());
        }
        if identity.is_identity == Some(true)
            && (identity.generation.as_deref() != Some("BY DEFAULT")
                || identity.start.is_none()
                || identity.increment.is_none()
                || identity.ordered.is_none())
        {
            return Err(malformed());
        }
        if identity.is_identity == Some(false)
            && (identity.generation.is_some()
                || identity.start.is_some()
                || identity.increment.is_some()
                || identity.ordered.is_some())
        {
            return Err(malformed());
        }
        if (kind == InspectedColumnKind::Column && row[20].is_some())
            || (kind == InspectedColumnKind::VirtualColumn
                && row[20].as_deref().is_none_or(str::is_empty))
        {
            return Err(malformed());
        }
        check_dimensions(&data_type, full_type, row)?;
        parsed.push(ColumnInspection {
            name: name.to_owned(),
            ordinal,
            full_type: full_type.to_owned(),
            data_type,
            data_type_alias: optional_text(&row[8], 1024)?,
            kind,
            result_kind: wire.kind,
            nullable,
            default_expression: optional_text(&row[5], 64 * 1024)?,
            // For Iceberg these are distinct observations: DESC default is
            // initial-default, while COLUMNS may report a Snowflake expression.
            // https://docs.snowflake.com/en/user-guide/tables-iceberg-default-values
            descriptor_default_expression: optional_text(at(4), 64 * 1024)?,
            write_default_expression: write_default_position
                .map(|position| optional_text(&described[position], 64 * 1024))
                .transpose()?
                .flatten(),
            write_default_status: if write_default_position.is_some() {
                WriteDefaultObservationStatus::Available
            } else {
                WriteDefaultObservationStatus::NotReported
            },
            comment: optional_text(&row[19], 64 * 1024)?,
            character_maximum_length: optional_unsigned(&row[9])?,
            character_octet_length: optional_unsigned(&row[10])?,
            numeric_precision: optional_unsigned(&row[11])?,
            numeric_precision_radix: optional_unsigned(&row[12])?,
            numeric_scale: optional_unsigned(&row[13])?,
            identity,
            expression: optional_text(&row[20], 64 * 1024)?,
        });
    }
    bounded(&parsed, COLUMN_MAX_BYTES)?;
    Ok(parsed)
}

/// Explicit family map. Unknown/new engine types fail closed rather than being
/// accepted because the adapter's generic display kind happens to be Text.
fn family(value: &str) -> Option<&'static str> {
    let upper = value.trim().to_ascii_uppercase();
    let base = upper.split(['(', '<', ' ']).next()?;
    Some(match base {
        "NUMBER" | "DECIMAL" | "NUMERIC" | "FIXED" | "INT" | "INTEGER" | "BIGINT" | "SMALLINT"
        | "TINYINT" | "BYTEINT" => "fixed",
        "FLOAT" | "FLOAT4" | "FLOAT8" | "DOUBLE" | "REAL" => "real",
        "VARCHAR" | "CHAR" | "CHARACTER" | "STRING" | "TEXT" => "text",
        "BOOLEAN" => "boolean",
        "DATE" => "date",
        "TIME" => "time",
        "TIMESTAMP_NTZ" | "DATETIME" => "timestamp_ntz",
        "TIMESTAMP_LTZ" => "timestamp_ltz",
        "TIMESTAMP_TZ" => "timestamp_tz",
        "BINARY" | "VARBINARY" => "binary",
        "VARIANT" => "variant",
        "OBJECT" => "object",
        "ARRAY" => "array",
        "GEOGRAPHY" => "geography",
        "GEOMETRY" => "geometry",
        "VECTOR" => "vector",
        _ => return None,
    })
}

fn compatible_types(metadata: &str, described: &str, wire: &str) -> bool {
    match (family(metadata), family(described), family(wire)) {
        (Some(a), Some(b), Some(c)) => a == b && b == c,
        _ => false,
    }
}

fn compatible_kind(wire: &str, kind: ColumnKind) -> bool {
    match family(wire) {
        Some("fixed") => matches!(kind, ColumnKind::Integer | ColumnKind::Decimal),
        Some("real") => kind == ColumnKind::Decimal,
        Some("text" | "geography" | "geometry" | "vector") => kind == ColumnKind::Text,
        Some("boolean") => kind == ColumnKind::Boolean,
        Some("date" | "time" | "timestamp_ntz" | "timestamp_ltz" | "timestamp_tz") => {
            kind == ColumnKind::Temporal
        }
        Some("variant" | "object" | "array") => kind == ColumnKind::Json,
        Some("binary") => kind == ColumnKind::Binary,
        _ => false,
    }
}

fn check_dimensions(data_type: &str, full_type: &str, row: &[Option<String>]) -> AppResult<()> {
    let parameters = || -> AppResult<Vec<&str>> {
        let (_, tail) = full_type.split_once('(').ok_or_else(malformed)?;
        let (arguments, _) = tail.split_once(')').ok_or_else(malformed)?;
        Ok(arguments.split(',').map(str::trim).collect())
    };
    match family(data_type) {
        Some("fixed") => {
            let args = parameters()?;
            if args.len() != 2
                || unsigned(args[0])? != required(&row[11])?
                || unsigned(args[1])? != required(&row[13])?
                || required(&row[12])? != "10"
            {
                return Err(ambiguous());
            }
        }
        Some("text") => {
            let args = parameters()?;
            if args.len() != 1 || unsigned(args[0])? != required(&row[9])? {
                return Err(ambiguous());
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn preview_sql(
    target: &BrowseRelation,
    columns: &[ColumnInspection],
) -> AppResult<String> {
    let name = qualified(target)?;
    verify_inspected_columns(columns)?;
    let projection = columns
        .iter()
        .map(|column| metadata::quote_identifier(&column.name))
        .collect::<AppResult<Vec<_>>>()?
        .join(", ");
    Ok(format!(
        "SELECT {projection} FROM {name} LIMIT {}",
        PREVIEW_LIMIT + 1
    ))
}

pub(crate) fn parse_preview(
    columns: &[ColumnInspection],
    result: &MetadataResult,
) -> AppResult<RelationPreview> {
    verify_inspected_columns(columns)?;
    // The collector must retain the sentinel. Byte truncation or discarded
    // rows are not evidence for a complete valid preview.
    complete(result, PREVIEW_LIMIT + 1, PREVIEW_MAX_BYTES)?;
    if result.columns.len() != columns.len() {
        return Err(malformed());
    }
    for (inspected, actual) in columns.iter().zip(&result.columns) {
        if inspected.name != actual.name
            || !compatible_types(
                &inspected.data_type,
                &inspected.full_type,
                &actual.database_type,
            )
            || inspected.result_kind != actual.kind
            || !compatible_kind(&actual.database_type, actual.kind)
        {
            return Err(ambiguous());
        }
    }
    let preview = RelationPreview {
        columns: columns
            .iter()
            .enumerate()
            .map(|(index, column)| ColumnInfo {
                id: format!("c{index}"),
                name: column.name.clone(),
                database_type: column.full_type.clone(),
                kind: column.result_kind,
                nullable: column.nullable,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: column.default_expression.is_some(),
            })
            .collect(),
        rows: result.rows.iter().take(PREVIEW_LIMIT).cloned().collect(),
        truncated: result.rows.len() > PREVIEW_LIMIT,
    };
    bounded(&preview, PREVIEW_MAX_BYTES)?;
    Ok(preview)
}

fn verify_inspected_columns(columns: &[ColumnInspection]) -> AppResult<()> {
    if columns.is_empty() || columns.len() > COLUMN_LIMIT {
        return Err(limit());
    }
    let mut names = HashSet::new();
    let mut prior = 0;
    for column in columns {
        metadata::quote_identifier(&column.name)?;
        if !names.insert(&column.name) || column.ordinal <= prior {
            return Err(malformed());
        }
        prior = column.ordinal;
        if family(&column.data_type).is_none()
            || family(&column.data_type) != family(&column.full_type)
        {
            return Err(malformed());
        }
    }
    bounded(&columns, COLUMN_MAX_BYTES)
}

fn database(context: &LiveContext) -> AppResult<&str> {
    // Reuse the supported-policy and current-database checks without altering
    // the live context or manufacturing transaction/autocommit observations.
    metadata::schemas_sql(context)?;
    context.database.as_deref().ok_or_else(malformed)
}
fn user_schema(schema: &str) -> AppResult<()> {
    metadata::quote_identifier(schema)?;
    if schema == "INFORMATION_SCHEMA" {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "System schema browsing is unavailable",
        ));
    }
    Ok(())
}
fn literal(value: &str) -> AppResult<String> {
    metadata::quote_identifier(value)?;
    Ok(format!(
        "'{}'",
        value.replace('\\', "\\\\").replace('\'', "''")
    ))
}
fn required(value: &Option<String>) -> AppResult<&str> {
    value.as_deref().ok_or_else(malformed)
}
fn text(value: &str, max: usize, empty: bool) -> AppResult<&str> {
    if value.len() > max {
        return Err(limit());
    }
    if (!empty && value.is_empty()) || value.contains('\0') {
        return Err(malformed());
    }
    Ok(value)
}
fn identifier(value: &Option<String>) -> AppResult<&str> {
    let value = required(value)?;
    metadata::quote_identifier(value).map_err(|_| malformed())?;
    Ok(value)
}
fn optional_text(value: &Option<String>, max: usize) -> AppResult<Option<String>> {
    value
        .as_deref()
        .map(|value| text(value, max, true).map(str::to_owned))
        .transpose()
}
fn yes_no(value: &Option<String>) -> AppResult<Option<bool>> {
    match value.as_deref() {
        Some("YES") => Ok(Some(true)),
        Some("NO") => Ok(Some(false)),
        None => Ok(None),
        _ => Err(malformed()),
    }
}
fn required_yes_no(value: &Option<String>) -> AppResult<bool> {
    yes_no(value)?.ok_or_else(malformed)
}
fn unsigned(value: &str) -> AppResult<&str> {
    if value.is_empty() || value.len() > 38 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(malformed());
    }
    // Metadata integers are bounded to Snowflake NUMBER(38,0), retained as text.
    Ok(value)
}
fn optional_unsigned(value: &Option<String>) -> AppResult<Option<String>> {
    value
        .as_deref()
        .map(|value| unsigned(value).map(str::to_owned))
        .transpose()
}
fn optional_signed(value: &Option<String>) -> AppResult<Option<String>> {
    value
        .as_deref()
        .map(|value| {
            unsigned(value.strip_prefix('-').unwrap_or(value))?;
            Ok(value.to_owned())
        })
        .transpose()
}
fn named_fields(result: &MetadataResult, fields: &[&str]) -> AppResult<Vec<usize>> {
    let mut seen = HashSet::new();
    for column in &result.columns {
        if !seen.insert(column.name.to_ascii_lowercase()) {
            return Err(malformed());
        }
    }
    fields
        .iter()
        .map(|field| {
            let index = result
                .columns
                .iter()
                .position(|column| column.name == *field)
                .ok_or_else(malformed)?;
            if result.columns[index].kind != ColumnKind::Text {
                return Err(malformed());
            }
            Ok(index)
        })
        .collect()
}
fn exact(result: &MetadataResult, names: &[&str], rows: usize, bytes: usize) -> AppResult<()> {
    complete(result, rows, bytes)?;
    if result.columns.len() != names.len()
        || result
            .columns
            .iter()
            .zip(names)
            .any(|(column, name)| column.name != *name || column.kind != ColumnKind::Text)
    {
        return Err(malformed());
    }
    Ok(())
}
fn complete(result: &MetadataResult, rows: usize, bytes: usize) -> AppResult<()> {
    if result.truncated
        || result.columns.len() > COLUMN_LIMIT
        || result.rows.len() > rows
        || result.rows_seen != result.rows.len() as u64
    {
        return Err(limit());
    }
    if result
        .rows
        .iter()
        .any(|row| row.len() != result.columns.len())
    {
        return Err(malformed());
    }
    bounded(result, bytes)
}
struct BudgetWriter(usize);
impl Write for BudgetWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("Snowflake browsing byte limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn bounded(value: &impl Serialize, bytes: usize) -> AppResult<()> {
    serde_json::to_writer(BudgetWriter(bytes), value).map_err(|_| limit())
}
fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Snowflake browsing result is incomplete or exceeds its bound",
    )
}
fn malformed() -> AppError {
    AppError::new(
        ErrorCode::Database,
        "Snowflake browsing metadata has an invalid shape or value",
    )
}
fn ambiguous() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake relation metadata changed or is ambiguous",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::TextRow;
    use crate::drivers::snowflake_metadata::SecondaryRoles;

    fn context() -> LiveContext {
        LiveContext {
            principal: "PERSON".into(),
            role: Some("ANALYST".into()),
            database: Some("DB".into()),
            schema: Some("PUBLIC".into()),
            warehouse: Some("WH".into()),
            session_id: "opaque-A7/session".into(),
            search_path: Vec::new(),
            secondary_roles: SecondaryRoles {
                roles: "ANALYST".into(),
                value: "ALL".into(),
            },
            quoted_identifiers_ignore_case: false,
        }
    }
    fn wire_column(name: &str, ty: &str, kind: ColumnKind) -> ColumnInfo {
        ColumnInfo {
            id: "wire".into(),
            name: name.into(),
            database_type: ty.into(),
            kind,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        }
    }
    fn result(names: &[&str], rows: Vec<TextRow>) -> MetadataResult {
        MetadataResult {
            columns: names
                .iter()
                .map(|name| wire_column(name, "TEXT", ColumnKind::Text))
                .collect(),
            rows_seen: rows.len() as u64,
            rows,
            truncated: false,
        }
    }
    fn row(values: &[&str]) -> TextRow {
        values.iter().map(|value| Some((*value).into())).collect()
    }
    fn relation_result() -> MetadataResult {
        result(
            &RELATION_COLUMNS,
            vec![row(&[
                "DB",
                "PUBLIC",
                "ITEM",
                "BASE TABLE",
                "NO",
                "NO",
                "NO",
                "NO",
                "NO",
                "NO",
                "created",
                "ddl",
            ])],
        )
    }
    fn target() -> BrowseRelation {
        parse_relations(&context(), "PUBLIC", &relation_result())
            .unwrap()
            .remove(0)
    }
    fn columns_result(names: &[&str], ordinals: &[u32]) -> MetadataResult {
        let rows = names
            .iter()
            .zip(ordinals)
            .map(|(name, ordinal)| {
                vec![
                    Some("DB".into()),
                    Some("PUBLIC".into()),
                    Some("ITEM".into()),
                    Some((*name).into()),
                    Some(ordinal.to_string()),
                    None,
                    Some("YES".into()),
                    Some("TEXT".into()),
                    None,
                    Some("20".into()),
                    Some("80".into()),
                    None,
                    None,
                    None,
                    Some("NO".into()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some("COLUMN".into()),
                ]
            })
            .collect();
        result(&COLUMN_COLUMNS, rows)
    }
    fn describe_result(names: &[&str]) -> MetadataResult {
        result(
            &[
                "name",
                "type",
                "kind",
                "null?",
                "default",
                "expression",
                "comment",
            ],
            names
                .iter()
                .map(|name| {
                    vec![
                        Some((*name).into()),
                        Some("VARCHAR(20)".into()),
                        Some("COLUMN".into()),
                        Some("Y".into()),
                        None,
                        None,
                        None,
                    ]
                })
                .collect(),
        )
    }
    fn resolved_result(names: &[&str]) -> MetadataResult {
        MetadataResult {
            columns: names
                .iter()
                .map(|name| wire_column(name, "TEXT", ColumnKind::Text))
                .collect(),
            rows: Vec::new(),
            rows_seen: 0,
            truncated: false,
        }
    }
    fn inspection() -> Vec<ColumnInspection> {
        parse_inspection(
            &target(),
            &columns_result(&["ID", "quoted\"name"], &[1, 10]),
            &describe_result(&["ID", "quoted\"name"]),
            &resolved_result(&["ID", "quoted\"name"]),
        )
        .unwrap()
    }

    #[test]
    fn builders_quote_components_escape_literals_and_order_numeric_source_ordinal() {
        let mut target = target();
        target.database = "D'\\\"B".into();
        target.table.schema = "S.with.dot".into();
        target.table.name = "T\"; DROP TABLE X;--".into();
        let sql = columns_sql(&target).unwrap();
        assert!(sql.contains("\"D'\\\"\"B\".INFORMATION_SCHEMA.COLUMNS AS C"));
        assert!(sql.contains("TABLE_CATALOG = 'D''\\\\\"B'"));
        assert!(sql.contains("ORDINAL_POSITION::VARCHAR AS ORDINAL_POSITION"));
        assert!(sql.ends_with("ORDER BY C.ORDINAL_POSITION LIMIT 1001"));
        assert!(
            resolved_columns_sql(&target)
                .unwrap()
                .ends_with(".\"T\"\"; DROP TABLE X;--\" LIMIT 0")
        );
        let sql = preview_sql(&target, &inspection()).unwrap();
        assert!(sql.starts_with("SELECT \"ID\", \"quoted\"\"name\" FROM "));
        assert!(sql.ends_with(" LIMIT 501"));
        target.table.name = "bad\0name".into();
        assert!(columns_sql(&target).is_err());
    }

    #[test]
    fn builders_require_supported_live_database_and_user_schema() {
        let mut context = context();
        context.database = None;
        assert!(relations_sql(&context, "PUBLIC").is_err());
        context.database = Some("DB".into());
        context.quoted_identifiers_ignore_case = true;
        assert!(relations_sql(&context, "PUBLIC").is_err());
        assert!(relations_sql(&super::tests::context(), "INFORMATION_SCHEMA").is_err());
    }

    #[test]
    fn catalog_preserves_traits_and_unknown_kinds_without_enabling_actions() {
        let mut result = relation_result();
        result.rows[0][3] = Some("FUTURE TABLE".into());
        result.rows[0][7] = Some("YES".into());
        let parsed = parse_relations(&context(), "PUBLIC", &result).unwrap();
        assert_eq!(
            parsed[0].availability,
            RelationAvailability::UnsupportedKind
        );
        assert_eq!(parsed[0].traits.dynamic, Some(true));
        assert!(columns_sql(&parsed[0]).is_err());
        result.rows[0][3] = Some("BASE TABLE".into());
        result.rows[0][9] = None;
        assert_eq!(
            parse_relations(&context(), "PUBLIC", &result).unwrap()[0].availability,
            RelationAvailability::UnknownTraits
        );
        result.rows[0][9] = Some("true".into());
        assert!(parse_relations(&context(), "PUBLIC", &result).is_err());
    }

    #[test]
    fn same_name_temporary_and_permanent_targets_remain_separate_unavailable() {
        let mut result = relation_result();
        let mut temporary = result.rows[0].clone();
        temporary[3] = Some("TEMPORARY TABLE".into());
        temporary[5] = Some("YES".into());
        result.rows.push(temporary);
        result.rows_seen = 2;
        let mut parsed = parse_relations(&context(), "PUBLIC", &result).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].availability, RelationAvailability::Shadowed);
        assert_eq!(parsed[1].availability, RelationAvailability::Ambiguous);
        assert!(
            parsed
                .iter()
                .all(|target| columns_sql(target).is_err() && describe_sql(target).is_err())
        );
        validate_catalog(&context(), &["PUBLIC".into()], &mut parsed).unwrap();
        assert_ne!(
            parsed[0].summary("a".into()).target_id,
            parsed[1].summary("b".into()).target_id
        );
        assert!(require_relation(&context(), &target(), &result).is_err());
    }

    #[test]
    fn duplicate_catalog_rows_wrong_database_and_permission_shapes_fail() {
        let mut result = relation_result();
        result.rows.push(result.rows[0].clone());
        result.rows_seen = 2;
        assert!(parse_relations(&context(), "PUBLIC", &result).is_err());
        result = relation_result();
        result.rows[0][0] = Some("OTHER_DB".into());
        assert!(parse_relations(&context(), "PUBLIC", &result).is_err());
        let error_shape = super::tests::result(&["error"], vec![row(&["access denied"])]);
        assert!(parse_relations(&context(), "PUBLIC", &error_shape).is_err());
    }

    #[test]
    fn exact_relation_read_rejects_disappearance_change_and_sentinel() {
        let target = target();
        assert!(
            relation_sql(&context(), &target)
                .unwrap()
                .ends_with("ORDER BY TABLE_TYPE, IS_TEMPORARY LIMIT 3")
        );
        require_relation(&context(), &target, &relation_result()).unwrap();
        let mut changed = relation_result();
        changed.rows[0][11] = Some("later-ddl".into());
        assert!(require_relation(&context(), &target, &changed).is_err());
        changed.rows.clear();
        changed.rows_seen = 0;
        assert!(require_relation(&context(), &target, &changed).is_err());
        changed = relation_result();
        changed.rows_seen = 3;
        assert_eq!(
            require_relation(&context(), &target, &changed)
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
    }

    #[test]
    fn aggregate_limits_and_complete_terminal_counts_are_enforced() {
        assert!(
            validate_catalog(
                &context(),
                &vec!["PUBLIC".into(); SCHEMA_LIMIT + 1],
                &mut []
            )
            .is_err()
        );
        let mut relations = vec![target(); RELATION_LIMIT + 1];
        assert!(validate_catalog(&context(), &["PUBLIC".into()], &mut relations).is_err());
        let mut result = relation_result();
        result.rows_seen += 1;
        assert!(parse_relations(&context(), "PUBLIC", &result).is_err());
        result = relation_result();
        result.truncated = true;
        assert!(parse_relations(&context(), "PUBLIC", &result).is_err());
        result = relation_result();
        result.columns[0].database_type = "x".repeat(CATALOG_MAX_BYTES);
        assert!(parse_relations(&context(), "PUBLIC", &result).is_err());
        assert!(validate_catalog(&context(), &["OTHER".into()], &mut [target()]).is_err());
    }

    #[test]
    fn iceberg_initial_and_write_defaults_remain_distinct_observations() {
        let mut target = target();
        target.traits.iceberg = Some(true);
        let mut columns = columns_result(&["A"], &[1]);
        columns.rows[0][5] = Some("2".into());
        let mut describe = describe_result(&["A"]);
        describe.rows[0][4] = Some("2".into());
        describe
            .columns
            .push(wire_column("write default", "TEXT", ColumnKind::Text));
        describe.rows[0].push(Some("3".into()));
        let parsed =
            parse_inspection(&target, &columns, &describe, &resolved_result(&["A"])).unwrap();
        let value = serde_json::to_value(&parsed[0]).unwrap();
        assert_eq!(value["defaultExpression"], "2");
        assert_eq!(value["descriptorDefaultExpression"], "2");
        assert_eq!(value["writeDefaultExpression"], "3");
        assert_eq!(value["writeDefaultStatus"], "available");
    }

    #[test]
    fn write_default_observation_distinguishes_missing_null_empty_and_expression() {
        let mut target = target();
        target.traits.iceberg = Some(true);
        let columns = columns_result(&["A"], &[1]);
        for value in [None, Some(""), Some("3")] {
            let mut describe = describe_result(&["A"]);
            describe
                .columns
                .push(wire_column("write default", "TEXT", ColumnKind::Text));
            describe.rows[0].push(value.map(str::to_owned));
            let parsed =
                parse_inspection(&target, &columns, &describe, &resolved_result(&["A"])).unwrap();
            assert_eq!(
                parsed[0].write_default_status,
                WriteDefaultObservationStatus::Available
            );
            assert_eq!(parsed[0].write_default_expression.as_deref(), value);
        }
        let parsed = parse_inspection(
            &target,
            &columns,
            &describe_result(&["A"]),
            &resolved_result(&["A"]),
        )
        .unwrap();
        assert_eq!(
            parsed[0].write_default_status,
            WriteDefaultObservationStatus::NotReported
        );
        assert_eq!(parsed[0].write_default_expression, None);

        // Snowflake-only expression defaults are independent of the Iceberg
        // initial/write defaults; do not evaluate, replace or compare them.
        let mut columns = columns;
        columns.rows[0][5] = Some("CURRENT_DATE()".into());
        columns.rows[0][7] = Some("DATE".into());
        columns.rows[0][9] = None;
        columns.rows[0][10] = None;
        let mut describe = describe_result(&["A"]);
        describe
            .columns
            .push(wire_column("write default", "TEXT", ColumnKind::Text));
        describe.rows[0].push(None);
        describe.rows[0][1] = Some("DATE".into());
        let mut resolved = resolved_result(&["A"]);
        resolved.columns[0].database_type = "DATE".into();
        resolved.columns[0].kind = ColumnKind::Temporal;
        let parsed = parse_inspection(&target, &columns, &describe, &resolved).unwrap();
        assert_eq!(
            parsed[0].default_expression.as_deref(),
            Some("CURRENT_DATE()")
        );
        assert_eq!(parsed[0].descriptor_default_expression, None);
        assert_eq!(parsed[0].write_default_expression, None);
        assert_eq!(
            parsed[0].write_default_status,
            WriteDefaultObservationStatus::Available
        );
        target.traits.iceberg = Some(false);
        assert!(parse_inspection(&target, &columns, &describe, &resolved).is_err());
    }

    #[test]
    fn descriptor_default_observations_reject_duplicate_nontext_and_oversized_fields() {
        let mut target = target();
        target.traits.iceberg = Some(true);
        let columns = columns_result(&["A"], &[1]);
        let resolved = resolved_result(&["A"]);
        let mut describe = describe_result(&["A"]);
        describe
            .columns
            .push(wire_column("write default", "TEXT", ColumnKind::Text));
        describe.rows[0].push(None);
        for kind in [ColumnKind::Decimal, ColumnKind::Json, ColumnKind::Unknown] {
            describe.columns.last_mut().unwrap().kind = kind;
            assert!(parse_inspection(&target, &columns, &describe, &resolved).is_err());
        }
        describe.columns.last_mut().unwrap().kind = ColumnKind::Text;
        describe.columns.last_mut().unwrap().name = "WRITE DEFAULT".into();
        assert!(parse_inspection(&target, &columns, &describe, &resolved).is_err());
        describe.columns.last_mut().unwrap().name = "write default".into();
        describe
            .columns
            .push(wire_column("WRITE DEFAULT", "TEXT", ColumnKind::Text));
        describe.rows[0].push(None);
        assert!(parse_inspection(&target, &columns, &describe, &resolved).is_err());
        describe.columns.pop();
        describe.rows[0].pop();
        for position in [4, 7] {
            for value in ["x".repeat(64 * 1024 + 1), "bad\0value".into()] {
                describe.rows[0][position] = Some(value);
                assert!(parse_inspection(&target, &columns, &describe, &resolved).is_err());
            }
            describe.rows[0][position] = None;
        }
        // Empty descriptor default text is preserved independently from SQL NULL.
        describe.rows[0][4] = Some(String::new());
        let parsed = parse_inspection(&target, &columns, &describe, &resolved).unwrap();
        assert_eq!(parsed[0].descriptor_default_expression.as_deref(), Some(""));
    }

    #[test]
    fn inspection_accepts_ordinal_gaps_and_preserves_null_empty_default_and_alias() {
        let mut columns = columns_result(&["A", "B"], &[2, 10]);
        let mut describe = describe_result(&["A", "B"]);
        columns.rows[1][5] = Some("''".into());
        describe.rows[1][4] = Some("''".into());
        columns.rows[1][8] = Some("VARCHAR".into());
        columns.rows[0][19] = Some(String::new());
        describe.rows[0][6] = Some(String::new());
        let parsed = parse_inspection(
            &target(),
            &columns,
            &describe,
            &resolved_result(&["A", "B"]),
        )
        .unwrap();
        assert_eq!(parsed[0].ordinal, 2);
        assert_eq!(parsed[1].ordinal, 10);
        assert_eq!(parsed[0].default_expression, None);
        assert_eq!(parsed[1].default_expression.as_deref(), Some("''"));
        assert_eq!(parsed[0].comment.as_deref(), Some(""));
        assert_eq!(parsed[0].data_type_alias, None);
        assert_eq!(parsed[1].data_type_alias.as_deref(), Some("VARCHAR"));
        assert_eq!(parsed[1].full_type, "VARCHAR(20)");
    }

    #[test]
    fn virtual_column_kinds_normalize_without_losing_expression() {
        let mut columns = columns_result(&["A"], &[1]);
        let mut describe = describe_result(&["A"]);
        columns.rows[0][21] = Some("VIRTUAL_COLUMN".into());
        columns.rows[0][20] = Some("GET(V, 'A')".into());
        describe.rows[0][2] = Some("VIRTUAL".into());
        describe.rows[0][5] = columns.rows[0][20].clone();
        let parsed =
            parse_inspection(&target(), &columns, &describe, &resolved_result(&["A"])).unwrap();
        assert_eq!(parsed[0].kind, InspectedColumnKind::VirtualColumn);
        assert_eq!(parsed[0].expression, columns.rows[0][20]);
        columns.rows[0][20] = None;
        describe.rows[0][5] = None;
        assert!(
            parse_inspection(&target(), &columns, &describe, &resolved_result(&["A"])).is_err()
        );
    }

    #[test]
    fn numeric_dimensions_and_identity_decimal_text_are_cross_checked() {
        let mut columns = columns_result(&["N"], &[1]);
        let mut describe = describe_result(&["N"]);
        columns.rows[0][7] = Some("NUMBER".into());
        columns.rows[0][9] = None;
        columns.rows[0][10] = None;
        columns.rows[0][11] = Some("38".into());
        columns.rows[0][12] = Some("10".into());
        columns.rows[0][13] = Some("0".into());
        columns.rows[0][14] = Some("YES".into());
        columns.rows[0][15] = Some("BY DEFAULT".into());
        columns.rows[0][16] = Some("9007199254740993".into());
        columns.rows[0][17] = Some("-2".into());
        columns.rows[0][18] = Some("NO".into());
        describe.rows[0][1] = Some("NUMBER(38,0)".into());
        let mut resolved = resolved_result(&["N"]);
        resolved.columns[0].database_type = "FIXED".into();
        resolved.columns[0].kind = ColumnKind::Decimal;
        let parsed = parse_inspection(&target(), &columns, &describe, &resolved).unwrap();
        assert_eq!(
            parsed[0].identity.start.as_deref(),
            Some("9007199254740993")
        );
        assert_eq!(parsed[0].identity.increment.as_deref(), Some("-2"));
        describe.rows[0][1] = Some("NUMBER(10,0)".into());
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
        describe.rows[0][1] = Some("NUMBER(38,0)".into());
        columns.rows[0][16] = Some("1e4".into());
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
    }

    #[test]
    fn inspection_rejects_bad_ordinals_duplicates_and_target_mismatch() {
        for ordinals in [[0, 2], [2, 2], [10, 2]] {
            assert!(
                parse_inspection(
                    &target(),
                    &columns_result(&["A", "B"], &ordinals),
                    &describe_result(&["A", "B"]),
                    &resolved_result(&["A", "B"])
                )
                .is_err()
            );
        }
        assert!(
            parse_inspection(
                &target(),
                &columns_result(&["A", "A"], &[1, 2]),
                &describe_result(&["A", "A"]),
                &resolved_result(&["A", "A"])
            )
            .is_err()
        );
        let mut columns = columns_result(&["A"], &[1]);
        columns.rows[0][0] = Some("OTHER_DB".into());
        assert!(
            parse_inspection(
                &target(),
                &columns,
                &describe_result(&["A"]),
                &resolved_result(&["A"])
            )
            .is_err()
        );
        columns.rows[0][0] = Some("DB".into());
        columns.rows[0][4] = Some("4294967296".into());
        assert!(
            parse_inspection(
                &target(),
                &columns,
                &describe_result(&["A"]),
                &resolved_result(&["A"])
            )
            .is_err()
        );
    }

    #[test]
    fn inspection_rejects_nullability_default_type_order_and_permission_shapes() {
        let columns = columns_result(&["A"], &[1]);
        let mut describe = describe_result(&["A"]);
        for (index, value) in [(3, "N"), (4, "'changed'"), (1, "VARCHAR(21)"), (0, "OTHER")] {
            describe.rows[0][index] = Some(value.into());
            assert!(
                parse_inspection(&target(), &columns, &describe, &resolved_result(&["A"])).is_err()
            );
            describe = describe_result(&["A"]);
        }
        let denied = result(&["error", "sqlState"], vec![row(&["denied", "42501"])]);
        assert!(parse_inspection(&target(), &columns, &denied, &resolved_result(&["A"])).is_err());
        let mut resolved = resolved_result(&["A"]);
        resolved.columns[0].database_type = "FIXED".into();
        resolved.columns[0].kind = ColumnKind::Decimal;
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
        resolved = resolved_result(&["A"]);
        resolved.columns[0].kind = ColumnKind::Unknown;
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
        assert!(!compatible_types(
            "FUTURE_TYPE",
            "FUTURE_TYPE",
            "FUTURE_TYPE"
        ));
    }

    #[test]
    fn descriptor_requires_unique_text_fields_but_tolerates_bounded_new_fields() {
        let columns = columns_result(&["A"], &[1]);
        let resolved = resolved_result(&["A"]);
        let mut describe = describe_result(&["A"]);
        describe
            .columns
            .push(wire_column("future_field", "TEXT", ColumnKind::Text));
        describe.rows[0].push(None);
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_ok());
        describe.columns.last_mut().unwrap().name = "NAME".into();
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
        describe = describe_result(&["A"]);
        describe.columns[1].kind = ColumnKind::Json;
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
    }

    #[test]
    fn inspection_rejects_discarded_nonzero_resolved_rows_and_oversized_metadata() {
        let mut columns = columns_result(&["A"], &[1]);
        let describe = describe_result(&["A"]);
        let mut resolved = resolved_result(&["A"]);
        resolved.rows_seen = 1;
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
        resolved.rows = vec![row(&["unexpected data"])];
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
        resolved = resolved_result(&["A"]);
        columns.rows[0][19] = Some("x".repeat(COLUMN_MAX_BYTES));
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
        columns = columns_result(&["A"], &[1]);
        columns.truncated = true;
        assert!(parse_inspection(&target(), &columns, &describe, &resolved).is_err());
        let too_many = vec!["A"; COLUMN_LIMIT + 1];
        assert!(
            parse_inspection(
                &target(),
                &columns_result(&too_many, &vec![1; too_many.len()]),
                &describe_result(&too_many),
                &resolved_result(&too_many)
            )
            .is_err()
        );
    }

    #[test]
    fn preview_retains_500_rows_uses_sentinel_and_clears_write_flags() {
        let columns = inspection();
        let mut result = resolved_result(&["ID", "quoted\"name"]);
        result.rows = (0..PREVIEW_LIMIT + 1)
            .map(|index| vec![Some(index.to_string()), None])
            .collect();
        result.rows_seen = result.rows.len() as u64;
        result.columns[0].editable = true;
        result.columns[0].insertable = true;
        result.columns[0].primary_key = true;
        let parsed = parse_preview(&columns, &result).unwrap();
        assert_eq!(parsed.rows.len(), PREVIEW_LIMIT);
        assert!(parsed.truncated);
        assert!(parsed.columns.iter().all(|column| !column.editable
            && !column.insertable
            && !column.primary_key
            && !column.foreign_key));
        result.rows.pop();
        result.rows_seen -= 1;
        assert!(!parse_preview(&columns, &result).unwrap().truncated);
    }

    #[test]
    fn preview_rejects_byte_truncation_oversized_first_row_shape_and_type_drift() {
        let columns = inspection();
        let mut result = resolved_result(&["ID", "quoted\"name"]);
        result.truncated = true;
        assert!(parse_preview(&columns, &result).is_err());
        result.truncated = false;
        result.rows_seen = 1;
        assert!(parse_preview(&columns, &result).is_err());
        result.rows = vec![vec![Some("x".repeat(PREVIEW_MAX_BYTES)), None]];
        assert!(parse_preview(&columns, &result).is_err());
        result.rows = vec![row(&["only one cell"])];
        assert!(parse_preview(&columns, &result).is_err());
        result = resolved_result(&["ID", "quoted\"name"]);
        result.columns[0].name = "OTHER".into();
        assert!(parse_preview(&columns, &result).is_err());
        result.columns[0].name = "ID".into();
        result.columns[0].database_type = "BINARY".into();
        result.columns[0].kind = ColumnKind::Binary;
        assert!(parse_preview(&columns, &result).is_err());
        assert!(preview_sql(&target(), &[]).is_err());
        let mut duplicate = columns.clone();
        duplicate[1].name = duplicate[0].name.clone();
        assert!(preview_sql(&target(), &duplicate).is_err());
    }
}
