//! Pure compilation for reviewed Snowflake consoles. This establishes exact SQL
//! units and scalar bindings, never execution permission or remote effects.
//! The retained console owner must still enforce profile policy, recovery holds,
//! one-use approval, session identity and per-submission statement count.

use crate::{
    domain::{AppError, AppResult, EngineKind, ErrorCode, QueryParamInput, SqlSpan},
    params::{self, BoundStatement},
    sql_text::{RewriteMap, span_for_byte_range},
};
use sqlparser::{dialect::SnowflakeDialect, parser::Parser, tokenizer::Tokenizer};

pub const MAX_SCRIPT_BYTES: usize = 1024 * 1024;
pub const MAX_UNITS: usize = 100;
pub const MAX_UNIT_BYTES: usize = 256 * 1024;
const MAX_PARSE_TOKENS: usize = 65_536;
const MAX_UNIT_CANONICAL_BYTES: usize = 1024 * 1024 - 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The complete batch must parse and every split must independently parse
    /// as exactly one statement. Procedural bodies are never split naively.
    ParsedStatements,
    /// Preserve the exact selection as one submission. Snowflake validates it
    /// at execution with helper-owned MULTI_STATEMENT_COUNT=1. It may contain a
    /// scripting body; this is not proof of its number of internal statements.
    SelectedUnit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validation {
    LocalSyntax,
    ServerAtExecution,
}

#[derive(Clone)]
pub struct Unit {
    original_sql: String,
    span: SqlSpan,
    bound: BoundStatement,
    validation: Validation,
}

impl Unit {
    pub fn original_sql(&self) -> &str {
        &self.original_sql
    }
    pub fn span(&self) -> SqlSpan {
        self.span
    }
    pub fn bound(&self) -> &BoundStatement {
        &self.bound
    }
    pub fn validation(&self) -> Validation {
        self.validation
    }
}

#[derive(Clone)]
pub struct CompiledScript {
    original_sql: String,
    mode: Mode,
    units: Vec<Unit>,
    inputs: Vec<QueryParamInput>,
}

impl CompiledScript {
    pub fn original_sql(&self) -> &str {
        &self.original_sql
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn units(&self) -> &[Unit] {
        &self.units
    }
    /// Transient review data only. Never put bound values into history/audit.
    pub fn inputs(&self) -> &[QueryParamInput] {
        &self.inputs
    }
    /// Recompile from original inputs at the trusted admission boundary. Callers
    /// cannot substitute wire SQL, casts, order or typed bind metadata.
    pub fn verify(&self, claimed: &[BoundStatement]) -> AppResult<()> {
        let canonical = compile(&self.original_sql, self.mode, Some(&self.inputs))?;
        if canonical.units.len() != claimed.len()
            || canonical.units.iter().zip(claimed).any(|(unit, wire)| {
                unit.bound.rewritten_sql != wire.rewritten_sql || unit.bound.binds != wire.binds
            })
        {
            return Err(AppError::invalid(
                "Reviewed script differs from canonical compilation",
            ));
        }
        Ok(())
    }
}

fn unsupported(message: &str) -> AppError {
    AppError::new(ErrorCode::Unsupported, message)
}

/// Match the pinned SDK's trim/first-three-ASCII-letter file-transfer dispatch.
/// This is only a client filesystem branch veto, not SQL classification. The
/// helper repeats its authoritative check on the exact unchanged wire string.
fn client_file_transfer(sql: &str) -> bool {
    let prefix = sql.trim_start_matches(|c| {
        matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}'
            | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
            | '\u{205f}' | '\u{3000}' | '\u{feff}')
    });
    prefix
        .get(..3)
        .is_some_and(|p| p.eq_ignore_ascii_case("PUT") || p.eq_ignore_ascii_case("GET"))
}

pub fn compile(
    sql: &str,
    mode: Mode,
    inputs: Option<&[QueryParamInput]>,
) -> AppResult<CompiledScript> {
    if sql.trim().is_empty() || sql.len() > MAX_SCRIPT_BYTES || sql.contains('\0') {
        return Err(AppError::invalid(
            "Provide nonempty SQL of at most 1 MiB without NUL bytes",
        ));
    }
    let slots = params::extract_query_params(EngineKind::Snowflake, sql)?;
    let (inputs, validated) = params::validate_inputs(&slots, inputs)?;
    let parts = match mode {
        Mode::SelectedUnit => vec![(sql.to_owned(), span_for_byte_range(sql, 0, sql.len()))],
        Mode::ParsedStatements => {
            let rewrite = params::rewrite_for_classification(EngineKind::Snowflake, sql)?;
            let dialect = SnowflakeDialect {};
            let tokens = Tokenizer::new(&dialect, &rewrite.sql)
                .tokenize()
                .map_err(|_| AppError::invalid("SQL contains an unterminated token"))?;
            if tokens.len() > MAX_PARSE_TOKENS {
                return Err(AppError::invalid(
                    "The parsed script exceeds its token budget",
                ));
            }
            let ast = Parser::parse_sql(&dialect, &rewrite.sql).map_err(|_| {
                unsupported("This dialect syntax needs an explicit selected server-validated unit")
            })?;
            let ranges = super::split_sql_byte_ranges(&dialect, &rewrite.sql)?;
            if ast.is_empty() || ast.len() != ranges.len() {
                return Err(unsupported(
                    "This script cannot be segmented safely; select one server-validated unit",
                ));
            }
            let mut parts = Vec::with_capacity(ranges.len());
            for (start, end) in ranges {
                if Parser::parse_sql(&dialect, &rewrite.sql[start..end])
                    .map_or(true, |unit| unit.len() != 1)
                {
                    return Err(unsupported(
                        "This script cannot be segmented safely; select one server-validated unit",
                    ));
                }
                let start = rewrite.to_original_byte(sql.len(), start);
                let end = rewrite.to_original_end_byte(sql.len(), end);
                parts.push((
                    sql[start..end].to_owned(),
                    span_for_byte_range(sql, start, end),
                ));
            }
            parts
        }
    };
    if parts.len() > MAX_UNITS {
        return Err(AppError::invalid(
            "Review at most 100 execution units at once",
        ));
    }
    let mut units = Vec::with_capacity(parts.len());
    let mut question_offset = 0;
    let mut remaining = params::MAX_BOUND_PLAN_BYTES;
    let mut canonical_bytes = 0usize;
    for (original_sql, span) in parts {
        if original_sql.len() > MAX_UNIT_BYTES || client_file_transfer(&original_sql) {
            return Err(unsupported(
                "A SQL unit exceeds 256 KiB or requires the separate file-transfer workflow",
            ));
        }
        let bound = params::compile_statement_with_budget(
            EngineKind::Snowflake,
            &original_sql,
            &validated,
            &mut question_offset,
            &mut remaining,
        )?
        .unwrap_or_else(|| BoundStatement {
            rewritten_sql: original_sql.clone(),
            binds: Vec::new(),
            rewrite_map: RewriteMap::identity(&original_sql),
        });
        if bound.rewritten_sql.len() > MAX_UNIT_BYTES || bound.binds.len() > 1000 {
            return Err(AppError::invalid(
                "Compiled SQL unit exceeds the helper SQL/bind budget",
            ));
        }
        let bytes = serde_json::to_vec(&bound)
            .map_err(|_| AppError::invalid("Cannot encode reviewed SQL"))?
            .len();
        if bytes > MAX_UNIT_CANONICAL_BYTES {
            return Err(AppError::invalid(
                "Compiled SQL unit exceeds the helper frame budget",
            ));
        }
        canonical_bytes = canonical_bytes
            .checked_add(bytes)
            .filter(|total| *total <= params::MAX_BOUND_PLAN_BYTES)
            .ok_or_else(|| AppError::invalid("Encoded script exceeds the 8 MiB review budget"))?;
        remaining = remaining
            .checked_sub(bound.rewritten_sql.len())
            .ok_or_else(|| AppError::invalid("Compiled script exceeds the 8 MiB review budget"))?;
        units.push(Unit {
            original_sql,
            span,
            bound,
            validation: match mode {
                Mode::ParsedStatements => Validation::LocalSyntax,
                Mode::SelectedUnit => Validation::ServerAtExecution,
            },
        });
    }
    Ok(CompiledScript {
        original_sql: sql.to_owned(),
        mode,
        units,
        inputs,
    })
}

#[cfg(test)]
mod tests;
