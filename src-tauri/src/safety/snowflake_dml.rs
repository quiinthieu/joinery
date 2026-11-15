//! Pure admission for the first, deliberately closed Snowflake DML grammar.
//!
//! These values establish syntax and canonical parameters, not permission to
//! execute, table identity/kind, absence of policies, or remote effects. The
//! driver must still qualify the target, apply policy/authority, and bound the
//! complete helper envelope. No function (including a user-written CAST) is
//! admitted. Trusted parameter compilation may introduce its fixed casts.

use crate::{
    domain::{
        AppError, AppResult, EngineKind, ErrorCode, QueryParamInput, SnowflakeDmlKind,
        StatementKind,
    },
    params::{self, BoundStatement},
    sql_text::RewriteMap,
};
use sqlparser::{
    ast::{
        AssignmentTarget, BinaryOperator, Expr, FromTable, Ident, ObjectName, Query, SetExpr,
        Statement, TableFactor, TableObject, TableWithJoins, UnaryOperator, Value, Visit,
    },
    dialect::SnowflakeDialect,
    parser::Parser,
    tokenizer::{Token, Tokenizer},
};
use std::collections::HashSet;

pub const MAX_SQL_BYTES: usize = 256 * 1024;
pub const MAX_PARSE_TOKENS: usize = 4096;
pub const MAX_PARAMETER_OCCURRENCES: usize = 1000;
pub const MAX_VALUES_ROWS: usize = 1000;
pub const MAX_COLUMNS: usize = 256;
pub const MAX_EXPRESSION_NODES: usize = 4096;
pub const MAX_EXPRESSION_DEPTH: usize = 64;
// Reserve room for the future closed operation envelope/identity. This is a
// conservative canonical-data cap, not proof that a complete helper frame fits.
pub const MAX_CANONICAL_BYTES: usize = 1024 * 1024 - 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    kind: SnowflakeDmlKind,
    qualified_target: [String; 3],
    missing_where: bool,
    original_sql: String,
}

impl Candidate {
    pub fn kind(&self) -> SnowflakeDmlKind {
        self.kind
    }
    pub fn qualified_target(&self) -> &[String; 3] {
        &self.qualified_target
    }
    pub fn missing_where(&self) -> bool {
        self.missing_where
    }
    pub fn original_sql(&self) -> &str {
        &self.original_sql
    }
}

#[derive(Debug, Clone)]
pub struct Canonical {
    candidate: Candidate,
    bound_statement: BoundStatement,
    validated_inputs: Vec<QueryParamInput>,
}

impl Canonical {
    pub fn candidate(&self) -> &Candidate {
        &self.candidate
    }
    pub fn kind(&self) -> SnowflakeDmlKind {
        self.candidate.kind()
    }
    pub fn qualified_target(&self) -> &[String; 3] {
        self.candidate.qualified_target()
    }
    pub fn missing_where(&self) -> bool {
        self.candidate.missing_where()
    }
    pub fn original_sql(&self) -> &str {
        self.candidate.original_sql()
    }
    pub fn bound_statement(&self) -> &BoundStatement {
        &self.bound_statement
    }
    pub fn validated_inputs(&self) -> &[QueryParamInput] {
        &self.validated_inputs
    }

    /// Only canonical SQL and ordered, fully validated TypedBind fields affect
    /// dispatch. RewriteMap is local error-location metadata, skipped on wire.
    pub fn verify_bound(&self, claimed: &BoundStatement) -> AppResult<()> {
        if claimed.rewritten_sql != self.bound_statement.rewritten_sql
            || claimed.binds != self.bound_statement.binds
        {
            return Err(AppError::invalid(
                "DML parameters do not match canonical compilation",
            ));
        }
        Ok(())
    }
}

fn unsupported() -> AppError {
    AppError::new(
        ErrorCode::Unsupported,
        "This SQL is outside the supported single-table Snowflake DML grammar",
    )
}

fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Snowflake DML exceeds the local review limits",
    )
}

fn check_sql(sql: &str) -> AppResult<()> {
    if sql.is_empty() || sql.contains('\0') {
        return Err(unsupported());
    }
    if sql.len() > MAX_SQL_BYTES {
        return Err(limit());
    }
    Ok(())
}

/// Parse original SQL after placeholder-only rewriting. Parser errors never
/// fall back to a permissive classification or to execution of original text.
pub fn analyze(original_sql: &str, pinned_database: &str) -> AppResult<Candidate> {
    check_sql(original_sql)?;
    validate_identifier_value(pinned_database)?;
    let slots = params::extract_query_params(EngineKind::Snowflake, original_sql)?;
    if slots.iter().map(|slot| slot.occurrences).sum::<usize>() > MAX_PARAMETER_OCCURRENCES {
        return Err(limit());
    }
    let rewritten = params::rewrite_for_classification(EngineKind::Snowflake, original_sql)?;
    check_sql(&rewritten.sql)?;
    let dialect = SnowflakeDialect {};
    // Bound flat trees before AST allocation as well as recursive parsing. The
    // parser recursion limit alone does not bound a long left-associated tree.
    let tokens = Tokenizer::new(&dialect, &rewritten.sql)
        .tokenize()
        .map_err(|_| unsupported())?;
    if tokens
        .iter()
        .filter(|token| !matches!(token, Token::Whitespace(_) | Token::EOF))
        .count()
        > MAX_PARSE_TOKENS
    {
        return Err(limit());
    }
    let statements = Parser::new(&dialect)
        .with_recursion_limit(MAX_EXPRESSION_DEPTH)
        .try_with_sql(&rewritten.sql)
        .and_then(|mut parser| parser.parse_statements())
        .map_err(|_| unsupported())?;
    let [statement] = statements.as_slice() else {
        return Err(unsupported());
    };
    let mut expressions = Expressions {
        remaining: MAX_EXPRESSION_NODES,
    };
    let (kind, target, missing_where) = match statement {
        Statement::Insert(insert) => {
            if !insert.into
                || !insert.optimizer_hints.is_empty()
                || insert.or.is_some()
                || insert.ignore
                || insert.table_alias.is_some()
                || insert.overwrite
                || !insert.assignments.is_empty()
                || insert.partitioned.is_some()
                || !insert.after_columns.is_empty()
                || insert.has_table_keyword
                || insert.on.is_some()
                || insert.returning.is_some()
                || insert.output.is_some()
                || insert.replace_into
                || insert.priority.is_some()
                || insert.insert_alias.is_some()
                || insert.settings.is_some()
                || insert.format_clause.is_some()
                || insert.multi_table_insert_type.is_some()
                || !insert.multi_table_into_clauses.is_empty()
                || !insert.multi_table_when_clauses.is_empty()
                || insert.multi_table_else_clause.is_some()
            {
                return Err(unsupported());
            }
            let TableObject::TableName(name) = &insert.table else {
                return Err(unsupported());
            };
            unique_columns(&insert.columns)?;
            let source = insert.source.as_ref().ok_or_else(unsupported)?;
            plain_query(source)?;
            let SetExpr::Values(values) = source.body.as_ref() else {
                return Err(unsupported());
            };
            if values.explicit_row || values.value_keyword || values.rows.is_empty() {
                return Err(unsupported());
            }
            if values.rows.len() > MAX_VALUES_ROWS {
                return Err(limit());
            }
            for row in &values.rows {
                if row.content.len() != insert.columns.len() {
                    return Err(unsupported());
                }
                for expr in &row.content {
                    expressions.scalar(expr, false, 0)?;
                }
            }
            (
                SnowflakeDmlKind::Insert,
                target(name, pinned_database)?,
                false,
            )
        }
        Statement::Update(update) => {
            if !update.optimizer_hints.is_empty()
                || update.from.is_some()
                || update.returning.is_some()
                || update.output.is_some()
                || update.or.is_some()
                || !update.order_by.is_empty()
                || update.limit.is_some()
            {
                return Err(unsupported());
            }
            if update.assignments.is_empty() {
                return Err(unsupported());
            }
            if update.assignments.len() > MAX_COLUMNS {
                return Err(limit());
            }
            let mut seen = HashSet::new();
            for assignment in &update.assignments {
                let AssignmentTarget::ColumnName(name) = &assignment.target else {
                    return Err(unsupported());
                };
                if !seen.insert(column(name)?) {
                    return Err(unsupported());
                }
                expressions.scalar(&assignment.value, true, 0)?;
            }
            if let Some(expr) = &update.selection {
                expressions.predicate(expr, 0)?;
            }
            (
                SnowflakeDmlKind::Update,
                table_target(&update.table, pinned_database)?,
                super::unrestricted(update.selection.as_ref()),
            )
        }
        Statement::Delete(delete) => {
            if !delete.optimizer_hints.is_empty()
                || !delete.tables.is_empty()
                || delete.using.is_some()
                || delete.returning.is_some()
                || delete.output.is_some()
                || !delete.order_by.is_empty()
                || delete.limit.is_some()
            {
                return Err(unsupported());
            }
            let FromTable::WithFromKeyword(tables) = &delete.from else {
                return Err(unsupported());
            };
            let [table] = tables.as_slice() else {
                return Err(unsupported());
            };
            if let Some(expr) = &delete.selection {
                expressions.predicate(expr, 0)?;
            }
            (
                SnowflakeDmlKind::Delete,
                table_target(table, pinned_database)?,
                super::unrestricted(delete.selection.as_ref()),
            )
        }
        _ => return Err(unsupported()),
    };
    // Keep the existing nested-statement/function veto. Run it only after the
    // closed traversal has bounded expression depth and excluded subqueries.
    let mut classification = super::Classification {
        engine: Some(EngineKind::Snowflake),
        ..Default::default()
    };
    let _ = statement.visit(&mut classification);
    if classification.kind != Some(StatementKind::Write) {
        return Err(unsupported());
    }
    Ok(Candidate {
        kind,
        qualified_target: target,
        missing_where,
        original_sql: original_sql.to_owned(),
    })
}

/// Rebuild from original SQL + public typed inputs, never from supplied
/// TypedBind metadata. All supported kinds (including typed NULL) use the
/// existing canonical validator and exact ordered compiler output.
pub fn compile(
    original_sql: &str,
    inputs: Option<&[QueryParamInput]>,
    pinned_database: &str,
) -> AppResult<Canonical> {
    let candidate = analyze(original_sql, pinned_database)?;
    let slots = params::extract_query_params(EngineKind::Snowflake, original_sql)?;
    let (validated_inputs, typed) = params::validate_inputs(&slots, inputs)?;
    let bound_statement =
        params::compile_statement(EngineKind::Snowflake, original_sql, &typed, &mut 0)?
            .unwrap_or_else(|| BoundStatement {
                rewritten_sql: original_sql.to_owned(),
                binds: Vec::new(),
                rewrite_map: RewriteMap::default(),
            });
    check_sql(&bound_statement.rewritten_sql)?;
    let bytes = serde_json::to_vec(&(&bound_statement, &validated_inputs)).map_err(|_| limit())?;
    if bytes.len() > MAX_CANONICAL_BYTES {
        return Err(limit());
    }
    Ok(Canonical {
        candidate,
        bound_statement,
        validated_inputs,
    })
}

fn validate_identifier_value(value: &str) -> AppResult<()> {
    if value.is_empty() || value.chars().count() > 255 || value.contains('\0') {
        return Err(unsupported());
    }
    Ok(())
}

fn identifier(ident: &Ident) -> AppResult<String> {
    validate_identifier_value(&ident.value)?;
    match ident.quote_style {
        Some('"') => Ok(ident.value.clone()),
        None => {
            let mut bytes = ident.value.bytes();
            if !bytes
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                || !bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'))
            {
                return Err(unsupported());
            }
            Ok(ident.value.to_ascii_uppercase())
        }
        _ => Err(unsupported()),
    }
}

fn column(name: &ObjectName) -> AppResult<String> {
    let [part] = name.0.as_slice() else {
        return Err(unsupported());
    };
    identifier(part.as_ident().ok_or_else(unsupported)?)
}

fn unique_columns(columns: &[ObjectName]) -> AppResult<()> {
    if columns.is_empty() {
        return Err(unsupported());
    }
    if columns.len() > MAX_COLUMNS {
        return Err(limit());
    }
    let mut seen = HashSet::new();
    for name in columns {
        if !seen.insert(column(name)?) {
            return Err(unsupported());
        }
    }
    Ok(())
}

fn target(name: &ObjectName, database: &str) -> AppResult<[String; 3]> {
    let [db, schema, table] = name.0.as_slice() else {
        return Err(unsupported());
    };
    let parts = [
        identifier(db.as_ident().ok_or_else(unsupported)?)?,
        identifier(schema.as_ident().ok_or_else(unsupported)?)?,
        identifier(table.as_ident().ok_or_else(unsupported)?)?,
    ];
    if parts[0] != database {
        return Err(unsupported());
    }
    Ok(parts)
}

fn table_target(table: &TableWithJoins, database: &str) -> AppResult<[String; 3]> {
    if !table.joins.is_empty() {
        return Err(unsupported());
    }
    let TableFactor::Table {
        name,
        alias,
        args,
        with_hints,
        version,
        with_ordinality,
        partitions,
        json_path,
        sample,
        index_hints,
    } = &table.relation
    else {
        return Err(unsupported());
    };
    if alias.is_some()
        || args.is_some()
        || !with_hints.is_empty()
        || version.is_some()
        || *with_ordinality
        || !partitions.is_empty()
        || json_path.is_some()
        || sample.is_some()
        || !index_hints.is_empty()
    {
        return Err(unsupported());
    }
    target(name, database)
}

fn plain_query(query: &Query) -> AppResult<()> {
    if query.with.is_some()
        || query.order_by.is_some()
        || query.limit_clause.is_some()
        || query.fetch.is_some()
        || !query.locks.is_empty()
        || query.for_clause.is_some()
        || query.settings.is_some()
        || query.format_clause.is_some()
        || !query.pipe_operators.is_empty()
    {
        return Err(unsupported());
    }
    Ok(())
}

struct Expressions {
    remaining: usize,
}
impl Expressions {
    fn node(&mut self, depth: usize) -> AppResult<()> {
        if depth > MAX_EXPRESSION_DEPTH || self.remaining == 0 {
            return Err(limit());
        }
        self.remaining -= 1;
        Ok(())
    }

    fn scalar(&mut self, expr: &Expr, columns: bool, depth: usize) -> AppResult<()> {
        self.node(depth)?;
        match expr {
            Expr::Value(value) => match &value.value {
                Value::Null | Value::Boolean(_) | Value::SingleQuotedString(_) => Ok(()),
                Value::DollarQuotedString(string) if string.tag.is_none() => Ok(()),
                Value::Number(number, false) if number.len() <= 128 => Ok(()),
                Value::Placeholder(p) if p == "?" => Ok(()),
                _ => Err(unsupported()),
            },
            Expr::Identifier(ident) if columns => {
                // DEFAULT is represented as an identifier by this parser. It
                // may invoke an unreviewed default/sequence rather than read a
                // column; the quoted column named "DEFAULT" remains ordinary.
                if ident.quote_style.is_none() && ident.value.eq_ignore_ascii_case("DEFAULT") {
                    return Err(unsupported());
                }
                identifier(ident)?;
                Ok(())
            }
            Expr::Nested(inner) => self.scalar(inner, columns, depth + 1),
            Expr::UnaryOp {
                op: UnaryOperator::Plus | UnaryOperator::Minus,
                expr,
            } => self.scalar(expr, columns, depth + 1),
            Expr::BinaryOp {
                left,
                op:
                    BinaryOperator::Plus
                    | BinaryOperator::Minus
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
                    | BinaryOperator::Modulo,
                right,
            } if columns => {
                self.scalar(left, columns, depth + 1)?;
                self.scalar(right, columns, depth + 1)
            }
            _ => Err(unsupported()),
        }
    }

    fn predicate(&mut self, expr: &Expr, depth: usize) -> AppResult<()> {
        self.node(depth)?;
        match expr {
            Expr::Nested(inner) => self.predicate(inner, depth + 1),
            Expr::UnaryOp {
                op: UnaryOperator::Not,
                expr,
            } => self.predicate(expr, depth + 1),
            Expr::BinaryOp {
                left,
                op: BinaryOperator::And | BinaryOperator::Or,
                right,
            } => {
                self.predicate(left, depth + 1)?;
                self.predicate(right, depth + 1)
            }
            Expr::BinaryOp {
                left,
                op:
                    BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq
                    | BinaryOperator::Gt
                    | BinaryOperator::GtEq,
                right,
            } => {
                self.scalar(left, true, depth + 1)?;
                self.scalar(right, true, depth + 1)
            }
            Expr::IsNull(inner)
            | Expr::IsNotNull(inner)
            | Expr::IsTrue(inner)
            | Expr::IsNotTrue(inner)
            | Expr::IsFalse(inner)
            | Expr::IsNotFalse(inner) => self.scalar(inner, true, depth + 1),
            Expr::InList { expr, list, .. }
                if !list.is_empty() && list.len() <= MAX_PARAMETER_OCCURRENCES =>
            {
                self.scalar(expr, true, depth + 1)?;
                for item in list {
                    self.scalar(item, true, depth + 1)?;
                }
                Ok(())
            }
            Expr::Between {
                expr, low, high, ..
            } => {
                self.scalar(expr, true, depth + 1)?;
                self.scalar(low, true, depth + 1)?;
                self.scalar(high, true, depth + 1)
            }
            Expr::Value(value) if matches!(value.value, Value::Boolean(_) | Value::Null) => Ok(()),
            _ => Err(unsupported()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::QueryParamKind;

    fn input(key: &str, kind: QueryParamKind, value: Option<&str>) -> QueryParamInput {
        QueryParamInput {
            key: key.into(),
            kind,
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn accepts_only_the_three_fixed_grammar_roots_and_preserves_original_text() {
        for (sql, kind) in [
            (
                "-- original\nINSERT INTO db.sc.t (a,b) VALUES (1,'text'),(-2,$$dollar;:not_a_bind$$);",
                SnowflakeDmlKind::Insert,
            ),
            (
                "UPDATE DB.SC.T SET A = A + :delta, B = NULL WHERE NOT (A < ? OR B IS NOT NULL)",
                SnowflakeDmlKind::Update,
            ),
            (
                "DELETE FROM DB.SC.T WHERE A BETWEEN -2 AND 10 AND B IN ('x','y')",
                SnowflakeDmlKind::Delete,
            ),
        ] {
            let candidate = analyze(sql, "DB").unwrap();
            assert_eq!(candidate.kind(), kind);
            assert_eq!(candidate.qualified_target(), &["DB", "SC", "T"]);
            assert_eq!(candidate.original_sql(), sql);
            assert!(!candidate.missing_where());
        }
    }

    #[test]
    fn quoted_components_are_not_split_or_casefolded() {
        let sql = "UPDATE \"Db.with.dot\".\"sc\".\"t\"\"name\" SET \"col.with.dot\" = \"DEFAULT\" WHERE \"x\" = 1";
        let candidate = analyze(sql, "Db.with.dot").unwrap();
        assert_eq!(
            candidate.qualified_target(),
            &["Db.with.dot", "sc", "t\"name"]
        );
        assert!(analyze(sql, "DB.WITH.DOT").is_err());
        assert!(analyze("DELETE FROM db.sc.t", "db").is_err());
        let name = "雪".repeat(255);
        assert!(analyze(&format!("DELETE FROM DB.SC.\"{name}\""), "DB").is_ok());
        assert!(analyze(&format!("DELETE FROM DB.SC.\"{name}雪\""), "DB").is_err());
    }

    #[test]
    fn rejects_unqualified_foreign_dynamic_or_malformed_targets() {
        for target in [
            "T",
            "SC.T",
            "OTHER.SC.T",
            "DB..T",
            "DB.SC.T.EXTRA",
            "IDENTIFIER('DB.SC.T')",
            "DB.SC.IDENTIFIER('T')",
            "\"\".SC.T",
            "DB.\"\".T",
            "DB.SC.\"\"",
            "`DB`.`SC`.`T`",
            "'DB'.SC.T",
        ] {
            assert!(
                analyze(&format!("DELETE FROM {target}"), "DB").is_err(),
                "{target}"
            );
        }
        for database in ["", "DB\0", &"x".repeat(256)] {
            assert!(analyze("DELETE FROM DB.SC.T", database).is_err());
        }
    }

    #[test]
    fn retains_existing_unrestricted_write_warnings() {
        for sql in [
            "UPDATE DB.SC.T SET A=1",
            "DELETE FROM DB.SC.T",
            "DELETE FROM DB.SC.T WHERE TRUE",
            "UPDATE DB.SC.T SET A=1 WHERE ((1=1))",
        ] {
            assert!(analyze(sql, "DB").unwrap().missing_where(), "{sql}");
        }
        assert!(
            !analyze("DELETE FROM DB.SC.T WHERE A=1", "DB")
                .unwrap()
                .missing_where()
        );
    }

    #[test]
    fn insert_columns_and_assignment_targets_are_explicit_simple_and_unique() {
        for sql in [
            "INSERT INTO DB.SC.T VALUES (1)",
            "INSERT INTO DB.SC.T (A,A) VALUES (1,2)",
            "INSERT INTO DB.SC.T (a,\"A\") VALUES (1,2)",
            "INSERT INTO DB.SC.T (T.A) VALUES (1)",
            "INSERT INTO DB.SC.T (A,B) VALUES (1)",
            "INSERT INTO DB.SC.T (A) VALUES (1,2)",
            "INSERT INTO DB.SC.T (A) VALUES (), (1)",
            "UPDATE DB.SC.T SET A=1,a=2",
            "UPDATE DB.SC.T SET T.A=1",
            "UPDATE DB.SC.T SET (A,B)=(1,2)",
        ] {
            assert!(analyze(sql, "DB").is_err(), "{sql}");
        }
        assert!(analyze("INSERT INTO DB.SC.T (a,\"a\") VALUES (1,2)", "DB").is_ok());
    }

    #[test]
    fn rejects_parsed_modifiers_joins_sources_and_returning() {
        for sql in [
            "INSERT INTO DB.SC.T (A) SELECT 1",
            "INSERT OVERWRITE INTO DB.SC.T (A) VALUES (1)",
            "INSERT INTO DB.SC.T (A) VALUES (1) RETURNING A",
            "INSERT ALL INTO DB.SC.T (A) VALUES (1) SELECT 1",
            "UPDATE DB.SC.T AS X SET A=1",
            "UPDATE DB.SC.T SET A=1 FROM DB.SC.U",
            "UPDATE DB.SC.T JOIN DB.SC.U ON TRUE SET A=1",
            "UPDATE DB.SC.T SET A=1 RETURNING A",
            "DELETE FROM DB.SC.T AS X WHERE A=1",
            "DELETE FROM DB.SC.T USING DB.SC.U WHERE A=1",
            "DELETE FROM DB.SC.T, DB.SC.U",
            "DELETE FROM DB.SC.T RETURNING A",
        ] {
            assert!(
                Parser::parse_sql(&SnowflakeDialect {}, sql).is_ok(),
                "test must reach an AST: {sql}"
            );
            assert!(analyze(sql, "DB").is_err(), "{sql}");
        }
    }

    #[test]
    fn rejects_all_other_roots_scripts_batches_and_parser_fallback() {
        for sql in [
            "",
            "-- comment only",
            "not a statement",
            "DELETE FROM DB.SC.T WHERE (",
            "SELECT * INTO DB.SC.T FROM DB.SC.U",
            "SELECT 1",
            "SHOW TABLES",
            "USE SC",
            "ALTER SESSION SET AUTOCOMMIT=FALSE",
            "BEGIN TRANSACTION",
            "COMMIT",
            "ROLLBACK",
            "CALL DB.SC.P()",
            "EXECUTE IMMEDIATE 'DELETE FROM DB.SC.T'",
            "COPY INTO DB.SC.T FROM @STAGE",
            "PUT file:///tmp/x @STAGE",
            "GET @STAGE file:///tmp/x",
            "MERGE INTO DB.SC.T USING DB.SC.U ON TRUE WHEN MATCHED THEN DELETE",
            "DELETE FROM DB.SC.T; DELETE FROM DB.SC.T",
            "DELETE FROM DB.SC.T; SELECT 1",
            "WITH X AS (SELECT 1) DELETE FROM DB.SC.T",
            "BEGIN DELETE FROM DB.SC.T; END;",
            "INSERT INTO DB.SC.T (A) VALUES (1) UNION SELECT 2",
            "DELETE FROM DB.SC.T\0",
        ] {
            assert!(analyze(sql, "DB").is_err(), "{sql}");
        }
    }

    #[test]
    fn expression_whitelist_rejects_functions_subqueries_sequences_and_modifiers() {
        for expression in [
            "ABS(A)",
            "DB.SC.UDF(A)",
            "\"ABS\"(A)",
            "COUNT(*) OVER ()",
            "SUM(A) FILTER (WHERE TRUE)",
            "PARSE_JSON('{}')",
            "CAST(1 AS NUMBER)",
            "1::NUMBER",
            "TRY_CAST('x' AS NUMBER)",
            "CURRENT_TIMESTAMP",
            "DEFAULT",
            "S.NEXTVAL",
            "(SELECT 1)",
            "EXISTS(SELECT 1)",
            "A:PATH",
            "A[0]",
            "ARRAY[1,2]",
            "CASE WHEN TRUE THEN 1 ELSE 2 END",
            "DATE '2026-11-10'",
            "{fn ABS(1)}",
            "(A,B)",
            "A || B",
            "A COLLATE 'en'",
            "*",
        ] {
            assert!(
                analyze(&format!("UPDATE DB.SC.T SET A={expression}"), "DB").is_err(),
                "{expression}"
            );
        }
        for predicate in [
            "EXISTS(SELECT 1)",
            "A IN (SELECT A FROM DB.SC.U)",
            "A=DB.SC.F()",
            "A > ANY(SELECT A FROM DB.SC.U)",
            "T.A=1",
            "A LIKE '%'",
            "A=1 AND F()=2",
        ] {
            assert!(
                analyze(&format!("DELETE FROM DB.SC.T WHERE {predicate}"), "DB").is_err(),
                "{predicate}"
            );
        }
        assert!(analyze("INSERT INTO DB.SC.T (A) VALUES (A)", "DB").is_err());
        assert!(analyze("INSERT INTO DB.SC.T (A) VALUES (1+2)", "DB").is_err());
    }

    #[test]
    fn canonical_compilation_preserves_repeated_named_and_ordered_positional_binds() {
        let sql = "INSERT INTO DB.SC.T (A,B,C,D) VALUES (:n,?,:n,?)";
        let inputs = [
            input("?2", QueryParamKind::Text, Some("tail")),
            input("n", QueryParamKind::Integer, Some(" +0042 ")),
            input("?1", QueryParamKind::Boolean, Some("yes")),
        ];
        let canonical = compile(sql, Some(&inputs), "DB").unwrap();
        assert_eq!(canonical.kind(), SnowflakeDmlKind::Insert);
        assert_eq!(canonical.qualified_target(), &["DB", "SC", "T"]);
        assert_eq!(canonical.original_sql(), sql);
        assert!(!canonical.missing_where());
        assert_eq!(
            canonical.bound_statement().rewritten_sql,
            "INSERT INTO DB.SC.T (A,B,C,D) VALUES (CAST(? AS NUMBER(38,0)),CAST(? AS BOOLEAN),CAST(? AS NUMBER(38,0)),?)"
        );
        let binds = &canonical.bound_statement().binds;
        assert_eq!(
            binds.iter().map(|b| b.key.as_str()).collect::<Vec<_>>(),
            ["n", "?1", "n", "?2"]
        );
        assert_eq!(binds[0], binds[2]);
        assert_eq!(binds[0].value.as_deref(), Some("42"));
        assert_eq!(binds[1].value.as_deref(), Some("true"));
        assert_eq!(
            canonical
                .validated_inputs()
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>(),
            ["n", "?1", "?2"]
        );
        canonical.verify_bound(canonical.bound_statement()).unwrap();
    }

    #[test]
    fn supports_all_canonical_typed_binds_without_permitting_user_functions() {
        let inputs = [
            input(
                "t",
                QueryParamKind::Text,
                Some("'); DELETE FROM OTHER.SC.T; --"),
            ),
            input("i", QueryParamKind::Integer, Some("9007199254740993")),
            input("d", QueryParamKind::Decimal, Some("123.450")),
            input("b", QueryParamKind::Boolean, None),
            input(
                "u",
                QueryParamKind::Uuid,
                Some("00000000-0000-4000-8000-000000000000"),
            ),
            input("date", QueryParamKind::Date, Some("2026-11-10")),
            input(
                "ts",
                QueryParamKind::Timestamp,
                Some("2026-11-10T12:00:00Z"),
            ),
            input("j", QueryParamKind::Json, Some("{\"n\":9007199254740993}")),
        ];
        let canonical = compile(
            "INSERT INTO DB.SC.T (A,B,C,D,E,F,G,H) VALUES (:t,:i,:d,:b,:u,:date,:ts,:j)",
            Some(&inputs),
            "DB",
        )
        .unwrap();
        let bound = canonical.bound_statement();
        assert_eq!(bound.binds.len(), 8);
        assert_eq!(bound.binds[1].value.as_deref(), Some("9007199254740993"));
        assert_eq!(bound.binds[3].value, None);
        assert!(bound.rewritten_sql.contains("CAST(? AS BOOLEAN)"));
        assert!(bound.rewritten_sql.contains("PARSE_JSON(?)"));
        assert!(!bound.rewritten_sql.contains("DELETE FROM OTHER"));
        assert!(analyze(&bound.rewritten_sql, "DB").is_err());
        let unbound = compile("DELETE FROM DB.SC.T WHERE A=1", None, "DB").unwrap();
        assert!(unbound.bound_statement().binds.is_empty());
        assert!(unbound.validated_inputs().is_empty());
    }

    #[test]
    fn rejects_forged_sql_bind_order_and_every_typed_metadata_field() {
        let canonical = compile(
            "UPDATE DB.SC.T SET A=:a WHERE B=:b",
            Some(&[
                input("a", QueryParamKind::Integer, Some("7")),
                input("b", QueryParamKind::Text, None),
            ]),
            "DB",
        )
        .unwrap();
        let original = canonical.bound_statement();
        let mut forged = original.clone();
        forged.rewritten_sql = "DELETE FROM DB.SC.T".into();
        assert!(canonical.verify_bound(&forged).is_err());
        let mut forged = original.clone();
        forged.binds.swap(0, 1);
        assert!(canonical.verify_bound(&forged).is_err());
        for field in 0..6 {
            let mut forged = original.clone();
            let bind = &mut forged.binds[1];
            match field {
                0 => bind.key = "wrong".into(),
                1 => bind.kind = QueryParamKind::Json,
                2 => bind.value = Some("null".into()),
                3 => bind.decimal_dims = Some((38, 0)),
                4 => bind.mysql_datetime = Some("private forged metadata".into()),
                _ => bind.timestamp_has_tz = true,
            }
            assert!(canonical.verify_bound(&forged).is_err(), "field {field}");
        }
    }

    #[test]
    fn rejects_missing_extra_duplicate_or_malformed_parameter_inputs() {
        let sql = "INSERT INTO DB.SC.T (A) VALUES (:x)";
        assert!(compile(sql, None, "DB").is_err());
        assert!(compile(sql, Some(&[]), "DB").is_err());
        let valid = input("x", QueryParamKind::Text, Some("x"));
        assert!(compile(sql, Some(&[valid.clone(), valid.clone()]), "DB").is_err());
        assert!(
            compile(
                sql,
                Some(&[valid, input("unknown", QueryParamKind::Text, None)]),
                "DB"
            )
            .is_err()
        );
        for (kind, value) in [
            (QueryParamKind::Integer, "9223372036854775808"),
            (QueryParamKind::Boolean, "maybe"),
            (QueryParamKind::Json, "["),
            (QueryParamKind::Date, "2026-02-30"),
            (QueryParamKind::Timestamp, "invalid"),
            (QueryParamKind::Uuid, "invalid"),
            (QueryParamKind::Text, "a\0b"),
            (
                QueryParamKind::Decimal,
                "123456789012345678901234567890123456789",
            ),
        ] {
            assert!(
                compile(sql, Some(&[input("x", kind, Some(value))]), "DB").is_err(),
                "{kind:?}"
            );
        }
        for kind in [
            QueryParamKind::Text,
            QueryParamKind::Integer,
            QueryParamKind::Decimal,
            QueryParamKind::Boolean,
            QueryParamKind::Uuid,
            QueryParamKind::Date,
            QueryParamKind::Timestamp,
            QueryParamKind::Json,
        ] {
            assert!(
                compile(sql, Some(&[input("x", kind, None)]), "DB").is_ok(),
                "{kind:?}"
            );
        }
        assert!(analyze("INSERT INTO :target (A) VALUES (1)", "DB").is_err());
        assert!(analyze("UPDATE DB.SC.T SET :column=1", "DB").is_err());
    }

    #[test]
    fn enforces_occurrence_row_column_node_and_depth_budgets() {
        let rows = std::iter::repeat_n("(:x)", 1000)
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("INSERT INTO DB.SC.T (A) VALUES {rows}");
        let canonical =
            compile(&sql, Some(&[input("x", QueryParamKind::Text, None)]), "DB").unwrap();
        assert_eq!(canonical.bound_statement().binds.len(), 1000);
        assert!(analyze(&format!("{sql},(:x)"), "DB").is_err());
        let rows = std::iter::repeat_n("(1)", 1001)
            .collect::<Vec<_>>()
            .join(",");
        assert!(analyze(&format!("INSERT INTO DB.SC.T (A) VALUES {rows}"), "DB").is_err());
        let columns = (0..257)
            .map(|i| format!("A{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(analyze(&format!("INSERT INTO DB.SC.T ({columns}) VALUES (1)"), "DB").is_err());
        let deep = format!(
            "DELETE FROM DB.SC.T WHERE {}TRUE{}",
            "(".repeat(100),
            ")".repeat(100)
        );
        assert!(analyze(&deep, "DB").is_err());
        // Large flat lists are bounded independently of recursive depth.
        let row = format!(
            "({})",
            std::iter::repeat_n("1", 256).collect::<Vec<_>>().join(",")
        );
        let columns = (0..256)
            .map(|i| format!("A{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let rows = std::iter::repeat_n(row, 17).collect::<Vec<_>>().join(",");
        assert!(
            analyze(
                &format!("INSERT INTO DB.SC.T ({columns}) VALUES {rows}"),
                "DB"
            )
            .is_err()
        );
    }

    #[test]
    fn bounds_original_wire_and_serialized_canonical_data_independently() {
        assert!(analyze(&" ".repeat(MAX_SQL_BYTES + 1), "DB").is_err());
        let sql = format!(
            "INSERT INTO DB.SC.T (A) VALUES (:x) --{}",
            "x".repeat(MAX_SQL_BYTES - 50)
        );
        assert!(analyze(&sql, "DB").is_ok());
        assert!(
            compile(
                &sql,
                Some(&[input("x", QueryParamKind::Integer, None)]),
                "DB"
            )
            .is_err()
        );
        // SQL fits, but JSON escaping expands each control byte sixfold.
        let sql = format!(
            "INSERT INTO DB.SC.T (A) VALUES ($${}$$)",
            "\u{0001}".repeat(MAX_SQL_BYTES - 100)
        );
        assert!(analyze(&sql, "DB").is_ok());
        assert!(compile(&sql, None, "DB").is_err());
        let value = "x".repeat(params::MAX_PARAM_VALUE_BYTES + 1);
        assert!(
            compile(
                "INSERT INTO DB.SC.T (A) VALUES (:x)",
                Some(&[input("x", QueryParamKind::Text, Some(&value))]),
                "DB"
            )
            .is_err()
        );
    }

    #[test]
    fn bounds_flat_trees_before_the_parser_and_shares_expression_budget() {
        let chain = std::iter::repeat_n("A=1", MAX_PARSE_TOKENS)
            .collect::<Vec<_>>()
            .join(" OR ");
        assert_eq!(
            analyze(&format!("DELETE FROM DB.SC.T WHERE {chain}"), "DB")
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
        let expression = Expr::Value(Value::Boolean(true).with_empty_span());
        let mut expressions = Expressions { remaining: 1 };
        expressions.predicate(&expression, 0).unwrap();
        assert!(expressions.predicate(&expression, 0).is_err());
    }
}
