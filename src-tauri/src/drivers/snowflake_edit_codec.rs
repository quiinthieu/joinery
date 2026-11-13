//! Closed inspection-to-codec conversion, without snapshot or write authority.
//! Names remain mutable names; every retained column participates in stale-row
//! predicates. Unsupported engine types/default semantics are not inferred.
use super::{
    snowflake_browsing::BrowseRelation,
    snowflake_mutations::{
        self as mutations, CellValue, CompiledBatch, Mutation, MutationColumn, MutationTarget,
        OriginalRow, ScalarType, WriteMode,
    },
};
use crate::{
    domain::{AppError, AppResult, ColumnKind, ErrorCode, TextRow},
    relation_inspection::{
        ColumnInspection, InspectedColumnKind, RelationAvailability, WriteDefaultObservationStatus,
    },
};

pub(crate) struct InspectedMutationTarget {
    target: MutationTarget,
    text_limits: Vec<Option<TextLimit>>,
}
#[derive(Clone, Copy)]
struct TextLimit {
    characters: usize,
    octets: usize,
}

fn invalid() -> AppError {
    AppError::invalid("Snowflake edit inspection metadata or values are inconsistent")
}
fn unsupported() -> AppError {
    AppError::new(
        ErrorCode::Unsupported,
        "This Snowflake relation, type, generated column, or default observation is not supported for generated edits",
    )
}
fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Snowflake edit codec budget exceeded",
    )
}
fn unsigned(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 38
        && value.bytes().all(|b| b.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}
fn dimension(value: Option<&str>) -> AppResult<usize> {
    let value = value.ok_or_else(invalid)?;
    if !unsigned(value) {
        return Err(invalid());
    }
    value.parse().map_err(|_| invalid())
}
fn signed(value: Option<&str>) -> AppResult<&str> {
    let value = value.ok_or_else(invalid)?;
    let magnitude = value.strip_prefix('-').unwrap_or(value);
    if !unsigned(magnitude) || value == "-0" {
        return Err(invalid());
    }
    Ok(value)
}
fn family(value: &str) -> Option<u8> {
    match value {
        "TEXT" | "VARCHAR" | "CHAR" | "CHARACTER" | "STRING" => Some(0),
        "NUMBER" | "DECIMAL" | "NUMERIC" | "FIXED" | "INT" | "INTEGER" | "BIGINT" | "SMALLINT"
        | "TINYINT" | "BYTEINT" => Some(1),
        "BOOLEAN" => Some(2),
        _ => None,
    }
}
fn parameters(full: &str, expected: u8) -> AppResult<Vec<&str>> {
    let (base, tail) = full.split_once('(').ok_or_else(invalid)?;
    if family(base) != Some(expected) {
        return Err(invalid());
    }
    let arguments = tail.strip_suffix(')').ok_or_else(invalid)?;
    let values: Vec<_> = arguments.split(',').map(str::trim).collect();
    if values.iter().any(|v| !unsigned(v)) {
        return Err(invalid());
    }
    Ok(values)
}
fn codec(column: &ColumnInspection) -> AppResult<(ScalarType, Option<TextLimit>)> {
    let kind = family(&column.data_type).ok_or_else(unsupported)?;
    if let Some(alias) = column.data_type_alias.as_deref() {
        match family(alias) {
            None => return Err(unsupported()),
            Some(alias_kind) if alias_kind != kind => return Err(invalid()),
            _ => {}
        }
    }
    match kind {
        0 => {
            if column.result_kind != ColumnKind::Text
                || column.numeric_precision.is_some()
                || column.numeric_precision_radix.is_some()
                || column.numeric_scale.is_some()
            {
                return Err(invalid());
            }
            let chars = dimension(column.character_maximum_length.as_deref())?;
            let bytes = dimension(column.character_octet_length.as_deref())?;
            let args = parameters(&column.full_type, kind)?;
            if args.len() != 1
                || dimension(Some(args[0]))? != chars
                || chars == 0
                || bytes < chars
                || bytes > chars.checked_mul(4).ok_or_else(invalid)?
            {
                return Err(invalid());
            }
            Ok((
                ScalarType::Text {
                    max_bytes: bytes.min(mutations::MAX_CELL_BYTES),
                },
                Some(TextLimit {
                    characters: chars,
                    octets: bytes,
                }),
            ))
        }
        1 => {
            if column.character_maximum_length.is_some()
                || column.character_octet_length.is_some()
                || column.numeric_precision_radix.as_deref() != Some("10")
            {
                return Err(invalid());
            }
            let precision = dimension(column.numeric_precision.as_deref())?;
            let scale = dimension(column.numeric_scale.as_deref())?;
            let args = parameters(&column.full_type, kind)?;
            if args.len() != 2
                || dimension(Some(args[0]))? != precision
                || dimension(Some(args[1]))? != scale
                || !(1..=38).contains(&precision)
                || scale > precision
                || !matches!(
                    column.result_kind,
                    ColumnKind::Integer | ColumnKind::Decimal
                )
                || (scale != 0 && column.result_kind == ColumnKind::Integer)
            {
                return Err(invalid());
            }
            if scale > 37 {
                return Err(unsupported());
            }
            let integer_alias = |value: &str| {
                matches!(
                    value,
                    "INT" | "INTEGER" | "BIGINT" | "SMALLINT" | "TINYINT" | "BYTEINT"
                )
            };
            if (integer_alias(&column.data_type)
                || column.data_type_alias.as_deref().is_some_and(integer_alias))
                && (precision != 38 || scale != 0)
            {
                return Err(invalid());
            }
            Ok((
                ScalarType::Decimal {
                    precision: precision as u8,
                    scale: scale as u8,
                },
                None,
            ))
        }
        2 => {
            if column.full_type != "BOOLEAN"
                || column.result_kind != ColumnKind::Boolean
                || column.character_maximum_length.is_some()
                || column.character_octet_length.is_some()
                || column.numeric_precision.is_some()
                || column.numeric_precision_radix.is_some()
                || column.numeric_scale.is_some()
            {
                return Err(invalid());
            }
            Ok((ScalarType::Boolean, None))
        }
        _ => unreachable!(),
    }
}
fn mode(column: &ColumnInspection, scalar: ScalarType) -> AppResult<WriteMode> {
    let identity = &column.identity;
    match identity.is_identity {
        Some(false)
            if identity.generation.is_none()
                && identity.start.is_none()
                && identity.increment.is_none()
                && identity.ordered.is_none() => {}
        Some(true) => {
            if column.kind != InspectedColumnKind::Column
                || column.expression.is_some()
                || !matches!(scalar, ScalarType::Decimal { scale: 0, .. })
                || identity.generation.as_deref() != Some("BY DEFAULT")
                || identity.ordered.is_none()
            {
                return Err(invalid());
            }
            signed(identity.start.as_deref())?;
            if signed(identity.increment.as_deref())? == "0" {
                return Err(invalid());
            }
            return Ok(WriteMode::Generated);
        }
        Some(false) => return Err(invalid()),
        None => return Err(unsupported()),
    }
    match (column.kind, column.expression.as_deref()) {
        (InspectedColumnKind::Column, None) => Ok(WriteMode::Writable),
        (InspectedColumnKind::VirtualColumn, Some(value)) if !value.trim().is_empty() => {
            Ok(WriteMode::Generated)
        }
        (InspectedColumnKind::PartitionColumn, _) => Err(unsupported()),
        _ => Err(invalid()),
    }
}

impl InspectedMutationTarget {
    pub(crate) fn from_inspection(
        relation: &BrowseRelation,
        columns: &[ColumnInspection],
    ) -> AppResult<Self> {
        let t = &relation.traits;
        let temporary = match relation.table_type.as_str() {
            "BASE TABLE" => false,
            "TEMPORARY TABLE" => true,
            _ => return Err(unsupported()),
        };
        if relation.availability != RelationAvailability::Available
            || t.transient.is_none()
            || t.temporary != Some(temporary)
            || t.iceberg != Some(false)
            || t.dynamic != Some(false)
            || t.hybrid != Some(false)
            || t.immutable != Some(false)
            || relation.table.schema == "INFORMATION_SCHEMA"
        {
            return Err(unsupported());
        }
        if columns.is_empty() || columns.len() > mutations::MAX_COLUMNS {
            return Err(limit());
        }
        let mut prior = 0;
        let mut metadata_bytes = 0usize;
        let mut compiled = Vec::with_capacity(columns.len());
        let mut text_limits = Vec::with_capacity(columns.len());
        for column in columns {
            if column.ordinal <= prior || column.full_type.len() > 4096 {
                return Err(invalid());
            }
            prior = column.ordinal;
            for field in [
                Some(column.name.as_str()),
                Some(column.full_type.as_str()),
                Some(column.data_type.as_str()),
                column.data_type_alias.as_deref(),
                column.default_expression.as_deref(),
                column.descriptor_default_expression.as_deref(),
                column.write_default_expression.as_deref(),
                column.comment.as_deref(),
                column.expression.as_deref(),
                column.identity.generation.as_deref(),
                column.identity.start.as_deref(),
                column.identity.increment.as_deref(),
                column.character_maximum_length.as_deref(),
                column.character_octet_length.as_deref(),
                column.numeric_precision.as_deref(),
                column.numeric_precision_radix.as_deref(),
                column.numeric_scale.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if field.len() > 64 * 1024 || field.contains('\0') {
                    return Err(invalid());
                }
                metadata_bytes = metadata_bytes.checked_add(field.len()).ok_or_else(limit)?;
                if metadata_bytes > 1024 * 1024 {
                    return Err(limit());
                }
            }
            if column.default_expression != column.descriptor_default_expression {
                return Err(invalid());
            }
            if column
                .default_expression
                .as_deref()
                .is_some_and(|s| s.trim().is_empty())
            {
                return Err(unsupported());
            }
            match (
                column.write_default_status,
                &column.write_default_expression,
            ) {
                (
                    WriteDefaultObservationStatus::NotReported
                    | WriteDefaultObservationStatus::Available,
                    None,
                ) => {}
                (WriteDefaultObservationStatus::NotReported, Some(_)) => return Err(invalid()),
                (WriteDefaultObservationStatus::Available, Some(_)) => return Err(unsupported()),
            }
            let (scalar, limits) = codec(column)?;
            let mode = mode(column, scalar)?;
            if column.kind == InspectedColumnKind::VirtualColumn
                && column.default_expression.is_some()
            {
                return Err(unsupported());
            }
            compiled.push(MutationColumn::new(
                &column.name,
                scalar,
                column.nullable,
                mode,
                column.default_expression.is_some(),
            )?);
            text_limits.push(limits);
        }
        Ok(Self {
            target: MutationTarget::new(
                &relation.database,
                &relation.table.schema,
                &relation.table.name,
                compiled,
            )?,
            text_limits,
        })
    }
    #[cfg(test)]
    pub(crate) fn target(&self) -> &MutationTarget {
        &self.target
    }
    fn cell(&self, index: usize, value: &CellValue) -> AppResult<()> {
        let limits = self.text_limits.get(index).ok_or_else(invalid)?;
        if let CellValue::Text(text) = value {
            if text.len() > mutations::MAX_CELL_BYTES {
                return Err(limit());
            }
            if let Some(limits) = limits
                && (text.len() > limits.octets || text.chars().count() > limits.characters)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
    fn row(&self, row: &[CellValue]) -> AppResult<()> {
        if row.len() != self.text_limits.len() {
            return Err(invalid());
        }
        for (index, cell) in row.iter().enumerate() {
            self.cell(index, cell)?;
        }
        Ok(())
    }
    pub(crate) fn original(&self, row: &TextRow) -> AppResult<OriginalRow> {
        if row.len() != self.text_limits.len() {
            return Err(invalid());
        }
        let mut bytes = 0usize;
        for (index, value) in row.iter().enumerate() {
            if let Some(value) = value {
                bytes = bytes.checked_add(value.len()).ok_or_else(limit)?;
                if bytes > mutations::MAX_BATCH_BYTES || value.len() > mutations::MAX_CELL_BYTES {
                    return Err(limit());
                }
                if let Some(limits) = self.text_limits[index]
                    && (value.len() > limits.octets || value.chars().count() > limits.characters)
                {
                    return Err(invalid());
                }
            }
        }
        OriginalRow::new(
            &self.target,
            row.iter()
                .map(|v| {
                    v.as_ref()
                        .map_or(CellValue::Null, |s| CellValue::Text(s.clone()))
                })
                .collect(),
        )
    }
    pub(crate) fn compile(&self, mutations: &[Mutation]) -> AppResult<CompiledBatch> {
        if mutations.is_empty() || mutations.len() > mutations::MAX_ROWS {
            return Err(limit());
        }
        for mutation in mutations {
            match mutation {
                Mutation::Insert { values } => {
                    if values.is_empty() || values.len() > self.text_limits.len() {
                        return Err(invalid());
                    }
                    for (index, value) in values {
                        self.cell(*index, value)?;
                    }
                }
                Mutation::Update { original, changes } => {
                    if changes.is_empty() || changes.len() > self.text_limits.len() {
                        return Err(invalid());
                    }
                    self.row(original.values())?;
                    for (index, value) in changes {
                        self.cell(*index, value)?;
                    }
                }
                Mutation::Delete { original } => self.row(original.values())?,
            }
        }
        mutations::compile(&self.target, mutations)
    }
}

#[cfg(test)]
#[path = "snowflake_edit_codec_tests.rs"]
mod tests;
