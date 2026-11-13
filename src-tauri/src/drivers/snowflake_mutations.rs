//! Pure generated mutation plans, without execution or edit authority.
//!
//! A caller must retain and corroborate inspection/result revisions separately,
//! own the transaction, check each actual DML cardinality, and roll back on a
//! mismatch. Quoted names are mutable names, not immutable object identities.
//! Original-value fingerprints only detect locally recognizable overlaps;
//! duplicate server rows can still match the same predicate.
//!
//! This foundation supports UTF-8 text, exact fixed-point
//! decimals and Boolean text. Float, binary, date/time, semi-structured and
//! broader collation/transport qualification remain required work. Text stale
//! predicates compare explicit UTF-8 binary conversions, not SQL collation.
//! Nothing here enables editing. UPDATE DEFAULT and mixed batches with unknown INSERT postimages
//! are explicit compiler limits, rather than guessed server effects.
//! Quoted identifier resolution also requires corroborating the session's
//! QUOTED_IDENTIFIERS_IGNORE_CASE setting; quoting is not an identity proof.
use crate::domain::{AppError, AppResult, ErrorCode};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub const MAX_ROWS: usize = 500;
pub const MAX_COLUMNS: usize = 128;
pub const MAX_CELL_BYTES: usize = 1024 * 1024;
pub const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedType {
    Float,
    Binary,
    Date,
    Time,
    Timestamp,
    Variant,
    Array,
    Object,
    Geography,
    Geometry,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarType {
    Text { max_bytes: usize },
    Decimal { precision: u8, scale: u8 },
    Boolean,
    Unsupported(UnsupportedType),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    Writable,
    Generated,
}

#[derive(Clone, Debug)]
pub struct MutationColumn {
    name: String,
    quoted: String,
    data_type: ScalarType,
    nullable: bool,
    mode: WriteMode,
    has_default: bool,
}
impl MutationColumn {
    pub fn new(
        name: &str,
        data_type: ScalarType,
        nullable: bool,
        mode: WriteMode,
        has_default: bool,
    ) -> AppResult<Self> {
        validate_identifier(name)?;
        match data_type {
            ScalarType::Text { max_bytes } if max_bytes == 0 || max_bytes > MAX_CELL_BYTES => {
                return Err(invalid());
            }
            ScalarType::Decimal { precision, scale }
                if precision == 0 || precision > 38 || scale > precision || scale > 37 =>
            {
                return Err(invalid());
            }
            ScalarType::Unsupported(_) => return Err(unsupported()),
            _ => {}
        }
        Ok(Self {
            name: name.into(),
            quoted: quote(name),
            data_type,
            nullable,
            mode,
            has_default,
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn data_type(&self) -> ScalarType {
        self.data_type
    }
}

/// Validated compilation inputs only. This descriptor proves no permission,
/// immutable identity, snapshot freshness, key enforcement or row uniqueness.
#[derive(Clone, Debug)]
pub struct MutationTarget {
    qualified_name: String,
    columns: Vec<MutationColumn>,
}
impl MutationTarget {
    pub fn new(
        database: &str,
        schema: &str,
        table: &str,
        columns: Vec<MutationColumn>,
    ) -> AppResult<Self> {
        for name in [database, schema, table] {
            validate_identifier(name)?;
        }
        if columns.is_empty() || columns.len() > MAX_COLUMNS {
            return Err(invalid());
        }
        let mut names = HashSet::new();
        for column in &columns {
            if !names.insert(column.name.as_str()) {
                return Err(invalid());
            }
        }
        Ok(Self {
            qualified_name: format!("{}.{}.{}", quote(database), quote(schema), quote(table)),
            columns,
        })
    }
    pub fn columns(&self) -> &[MutationColumn] {
        &self.columns
    }
    pub fn qualified_name(&self) -> &str {
        &self.qualified_name
    }
}

/// Text is exact input for the column's closed scalar codec, never SQL syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellValue {
    Default,
    Null,
    Text(String),
}

/// Full ordered values of all target columns, including generated columns.
/// The future runtime must obtain originals from its own retained results.
#[derive(Clone, Debug)]
pub struct OriginalRow {
    values: Vec<CellValue>,
}
impl OriginalRow {
    pub fn new(target: &MutationTarget, values: Vec<CellValue>) -> AppResult<Self> {
        validate_original(target, &values)?;
        Ok(Self { values })
    }
    pub fn values(&self) -> &[CellValue] {
        &self.values
    }
}

#[derive(Clone, Debug)]
pub enum Mutation {
    Insert {
        values: Vec<(usize, CellValue)>,
    },
    Update {
        original: OriginalRow,
        changes: Vec<(usize, CellValue)>,
    },
    Delete {
        original: OriginalRow,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    Text,
    Decimal,
    Boolean,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    kind: BindingKind,
    value: Option<String>,
}
impl Binding {
    pub fn kind(&self) -> BindingKind {
        self.kind
    }
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationKind {
    Insert,
    Update,
    Delete,
}
#[derive(Debug)]
pub struct CompiledUnit {
    kind: MutationKind,
    sql: String,
    bindings: Vec<Binding>,
    expected_rows: u64,
}
impl CompiledUnit {
    pub fn kind(&self) -> MutationKind {
        self.kind
    }
    pub fn sql(&self) -> &str {
        &self.sql
    }
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }
    pub fn expected_rows(&self) -> u64 {
        self.expected_rows
    }
}
#[derive(Debug)]
pub struct CompiledBatch {
    units: Vec<CompiledUnit>,
    accounted_bytes: usize,
}
impl CompiledBatch {
    pub fn units(&self) -> &[CompiledUnit] {
        &self.units
    }
    /// Conservative logical input/output/SQL/identifier/binding accounting;
    /// this is not an allocator-capacity or whole-process RSS measurement.
    pub fn accounted_bytes(&self) -> usize {
        self.accounted_bytes
    }
    pub fn into_units(self) -> Vec<CompiledUnit> {
        self.units
    }
}

fn invalid() -> AppError {
    AppError::invalid("Generated Snowflake mutation inputs are invalid")
}
fn unsupported() -> AppError {
    AppError::new(
        ErrorCode::Unsupported,
        "This generated Snowflake mutation requires an unsupported exact codec or batch shape",
    )
}
fn overlap() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Generated mutations contain duplicate or overlapping original-row targets",
    )
}
fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Generated mutations exceed the bounded compiler budget",
    )
}
fn validate_identifier(name: &str) -> AppResult<()> {
    // Snowflake quoted identifiers allow at most 255 characters. The byte
    // ceiling rejects oversized input before scanning; UTF-8 uses <=4 bytes.
    if name.is_empty() || name.len() > 255 * 4 || name.chars().count() > 255
        || name.trim().is_empty()
        || name.chars().any(|c| c.is_control() || matches!(c, '\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{2028}'..='\u{202e}'|'\u{2066}'..='\u{2069}')) {
        return Err(invalid());
    }
    Ok(())
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn decimal_parts(text: &str, precision: u8, scale: u8) -> AppResult<(&str, &str, bool)> {
    let (negative, unsigned) = text.strip_prefix('-').map_or((false, text), |s| (true, s));
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if integer.is_empty()
        || !integer.bytes().all(|c| c.is_ascii_digit())
        || !fraction.bytes().all(|c| c.is_ascii_digit())
        || (unsigned.contains('.') && fraction.is_empty())
        || fraction.len() > usize::from(scale)
        || integer.trim_start_matches('0').len() > usize::from(precision - scale)
    {
        return Err(invalid());
    }
    Ok((integer, fraction, negative))
}
fn validate_cell(column: &MutationColumn, value: &CellValue, original: bool) -> AppResult<()> {
    match value {
        CellValue::Default if !original && (column.has_default || column.nullable) => Ok(()),
        CellValue::Default => Err(invalid()),
        CellValue::Null if column.nullable => Ok(()),
        CellValue::Null => Err(invalid()),
        CellValue::Text(text) => {
            if text.len() > MAX_CELL_BYTES {
                return Err(limit());
            }
            match column.data_type {
                ScalarType::Text { max_bytes } if text.len() <= max_bytes => Ok(()),
                ScalarType::Decimal { precision, scale } => {
                    decimal_parts(text, precision, scale).map(|_| ())
                }
                ScalarType::Boolean if matches!(text.as_str(), "true" | "false") => Ok(()),
                ScalarType::Unsupported(_) => Err(unsupported()),
                _ => Err(invalid()),
            }
        }
    }
}
fn validate_original(target: &MutationTarget, values: &[CellValue]) -> AppResult<()> {
    if values.len() != target.columns.len() {
        return Err(invalid());
    }
    for (column, value) in target.columns.iter().zip(values) {
        validate_cell(column, value, true)?;
    }
    Ok(())
}
fn validate_assignments(
    target: &MutationTarget,
    values: &[(usize, CellValue)],
    update: bool,
) -> AppResult<()> {
    if values.is_empty() || values.len() > target.columns.len() {
        return Err(invalid());
    }
    let mut seen = [false; MAX_COLUMNS];
    for (index, value) in values {
        let column = target.columns.get(*index).ok_or_else(invalid)?;
        if seen[*index] || column.mode != WriteMode::Writable {
            return Err(invalid());
        }
        seen[*index] = true;
        if update && *value == CellValue::Default {
            return Err(unsupported());
        }
        validate_cell(column, value, false)?;
    }
    if !update
        && target.columns.iter().enumerate().any(|(i, c)| {
            !seen[i] && c.mode == WriteMode::Writable && !c.nullable && !c.has_default
        })
    {
        return Err(invalid());
    }
    Ok(())
}

fn fingerprint<'a>(
    target: &MutationTarget,
    values: impl Iterator<Item = &'a CellValue>,
) -> AppResult<[u8; 32]> {
    let mut digest = Sha256::new();
    for (column, value) in target.columns.iter().zip(values) {
        match value {
            CellValue::Default => return Err(unsupported()),
            CellValue::Null => digest.update([0]),
            CellValue::Text(text) => {
                digest.update([1]);
                if let ScalarType::Decimal { precision, scale } = column.data_type {
                    let (integer, fraction, negative) = decimal_parts(text, precision, scale)?;
                    let integer = integer.trim_start_matches('0');
                    let fraction = fraction.trim_end_matches('0');
                    digest.update([u8::from(
                        negative && (!integer.is_empty() || !fraction.is_empty()),
                    )]);
                    digest.update((integer.len() as u64).to_be_bytes());
                    digest.update(integer.as_bytes());
                    digest.update((fraction.len() as u64).to_be_bytes());
                    digest.update(fraction.as_bytes());
                } else {
                    digest.update((text.len() as u64).to_be_bytes());
                    digest.update(text.as_bytes());
                }
            }
        }
    }
    Ok(digest.finalize().into())
}
struct Budget(usize);
impl Budget {
    fn add(&mut self, amount: usize) -> AppResult<()> {
        self.0 = self
            .0
            .checked_add(amount)
            .filter(|n| *n <= MAX_BATCH_BYTES)
            .ok_or_else(limit)?;
        Ok(())
    }
    fn cell(&mut self, value: &CellValue) -> AppResult<()> {
        // Input cell/index/Vec overhead plus one owned output binding. Payload
        // bytes are charged twice because compilation retains exact binds.
        self.add(96)?;
        if let CellValue::Text(text) = value {
            self.add(text.len().checked_mul(2).ok_or_else(limit)?)?;
        }
        Ok(())
    }
}

/// Compile a nonempty bounded batch. No query, transaction or capability is
/// created. Actual affected-row counts must equal expected_rows before commit.
pub fn compile(target: &MutationTarget, mutations: &[Mutation]) -> AppResult<CompiledBatch> {
    if mutations.is_empty() {
        return Err(invalid());
    }
    if mutations.len() > MAX_ROWS {
        return Err(limit());
    }
    let mut budget = Budget(0);
    budget.add(
        target.qualified_name.len()
            + target
                .columns
                .iter()
                .map(|c| c.name.len() + c.quoted.len() + 128)
                .sum::<usize>(),
    )?;
    let mut originals = HashSet::new();
    let mut original_by_unit = Vec::with_capacity(mutations.len());
    let mut postimages = Vec::with_capacity(mutations.len());
    let mut capacities = Vec::with_capacity(mutations.len());
    let mut unknown_insert = false;
    for mutation in mutations {
        let (original, assignments) = match mutation {
            Mutation::Insert { values } => {
                validate_assignments(target, values, false)?;
                (None, values.as_slice())
            }
            Mutation::Update { original, changes } => {
                validate_original(target, &original.values)?;
                validate_assignments(target, changes, true)?;
                (Some(original), changes.as_slice())
            }
            Mutation::Delete { original } => {
                validate_original(target, &original.values)?;
                (Some(original), &[][..])
            }
        };
        let mut assigned = [None; MAX_COLUMNS];
        for (index, value) in assignments {
            assigned[*index] = Some(value);
            budget.cell(value)?;
        }
        if let Some(original) = original {
            for value in &original.values {
                budget.cell(value)?;
            }
        }
        let original_key = original
            .map(|r| fingerprint(target, r.values.iter()))
            .transpose()?;
        if let Some(key) = original_key
            && !originals.insert(key)
        {
            return Err(overlap());
        }
        original_by_unit.push(original_key);
        let postimage = match mutation {
            Mutation::Delete { .. } => None,
            Mutation::Update { original, .. } => Some(fingerprint(
                target,
                original
                    .values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| assigned[i].unwrap_or(v)),
            )?),
            Mutation::Insert { .. }
                if (0..target.columns.len())
                    .all(|i| assigned[i].is_some_and(|v| *v != CellValue::Default)) =>
            {
                Some(fingerprint(
                    target,
                    (0..target.columns.len()).map(|i| assigned[i].expect("complete validated row")),
                )?)
            }
            Mutation::Insert { .. } => {
                unknown_insert = true;
                None
            }
        };
        postimages.push(postimage);
        // Conservative upper bound includes syntax, quoted identifiers, casts,
        // bind-vector storage and bounded overlap bookkeeping before allocation.
        let sql_capacity = 128
            + target.qualified_name.len()
            + assignments
                .iter()
                .map(|(i, _)| target.columns[*i].quoted.len() + 64)
                .sum::<usize>()
            + original.map_or(0, |_| {
                target
                    .columns
                    .iter()
                    .map(|c| c.quoted.len() + 128)
                    .sum::<usize>()
            });
        budget.add(sql_capacity + 256)?;
        capacities.push(sql_capacity);
    }
    if unknown_insert && !originals.is_empty() {
        return Err(unsupported());
    }
    for (i, postimage) in postimages.iter().enumerate() {
        if let Some(key) = postimage
            && originals.contains(key)
            && original_by_unit[i].as_ref() != Some(key)
        {
            return Err(overlap());
        }
    }
    let units = mutations
        .iter()
        .zip(capacities)
        .map(|(mutation, capacity)| emit(target, mutation, capacity))
        .collect();
    Ok(CompiledBatch {
        units,
        accounted_bytes: budget.0,
    })
}

fn cast(data_type: ScalarType) -> String {
    match data_type {
        ScalarType::Text { .. } => "VARCHAR".into(),
        ScalarType::Decimal { precision, scale } => format!("NUMBER({precision},{scale})"),
        ScalarType::Boolean => "BOOLEAN".into(),
        ScalarType::Unsupported(_) => unreachable!("validated target"),
    }
}
fn bind(sql: &mut String, bindings: &mut Vec<Binding>, column: &MutationColumn, value: &CellValue) {
    if *value == CellValue::Default {
        sql.push_str("DEFAULT");
        return;
    }
    sql.push_str("CAST(? AS ");
    sql.push_str(&cast(column.data_type));
    sql.push(')');
    let kind = match column.data_type {
        ScalarType::Text { .. } => BindingKind::Text,
        ScalarType::Decimal { .. } => BindingKind::Decimal,
        ScalarType::Boolean => BindingKind::Boolean,
        ScalarType::Unsupported(_) => unreachable!("validated target"),
    };
    bindings.push(Binding {
        kind,
        value: match value {
            CellValue::Text(text) => Some(text.clone()),
            CellValue::Null => None,
            CellValue::Default => unreachable!(),
        },
    });
}
fn emit(target: &MutationTarget, mutation: &Mutation, capacity: usize) -> CompiledUnit {
    let mut sql = String::with_capacity(capacity);
    let mut bindings = Vec::new();
    let (kind, original) = match mutation {
        Mutation::Insert { values } => {
            sql.push_str("INSERT INTO ");
            sql.push_str(&target.qualified_name);
            sql.push_str(" (");
            for (n, (i, _)) in values.iter().enumerate() {
                if n > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&target.columns[*i].quoted);
            }
            sql.push_str(") VALUES (");
            for (n, (i, v)) in values.iter().enumerate() {
                if n > 0 {
                    sql.push_str(", ");
                }
                bind(&mut sql, &mut bindings, &target.columns[*i], v);
            }
            sql.push(')');
            (MutationKind::Insert, None)
        }
        Mutation::Update { original, changes } => {
            sql.push_str("UPDATE ");
            sql.push_str(&target.qualified_name);
            sql.push_str(" SET ");
            for (n, (i, v)) in changes.iter().enumerate() {
                if n > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&target.columns[*i].quoted);
                sql.push_str(" = ");
                bind(&mut sql, &mut bindings, &target.columns[*i], v);
            }
            (MutationKind::Update, Some(original))
        }
        Mutation::Delete { original } => {
            sql.push_str("DELETE FROM ");
            sql.push_str(&target.qualified_name);
            (MutationKind::Delete, Some(original))
        }
    };
    if let Some(original) = original {
        sql.push_str(" WHERE ");
        for (n, (column, value)) in target.columns.iter().zip(&original.values).enumerate() {
            if n > 0 {
                sql.push_str(" AND ");
            }
            let binary_text = matches!(column.data_type, ScalarType::Text { .. });
            if binary_text {
                sql.push_str("TO_BINARY(");
            }
            sql.push_str(&column.quoted);
            if binary_text {
                sql.push_str(", 'UTF-8')");
            }
            sql.push_str(" IS NOT DISTINCT FROM ");
            if binary_text {
                sql.push_str("TO_BINARY(");
            }
            bind(&mut sql, &mut bindings, column, value);
            if binary_text {
                sql.push_str(", 'UTF-8')");
            }
        }
    }
    debug_assert!(sql.len() <= capacity);
    CompiledUnit {
        kind,
        sql,
        bindings,
        expected_rows: 1,
    }
}

#[cfg(test)]
#[path = "snowflake_mutations_tests.rs"]
mod tests;
