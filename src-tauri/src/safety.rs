use crate::domain::*;
use sqlparser::{
    ast::{Query, SetExpr, Statement, Visit, Visitor},
    dialect::{Dialect, MySqlDialect, PostgreSqlDialect, SnowflakeDialect},
    parser::Parser,
    tokenizer::{Location, Token, Tokenizer},
};
use std::ops::ControlFlow;

pub fn effective_mode(profile: &ConnectionProfile) -> SafeMode {
    if profile.environment == Environment::Production && profile.safe_mode != SafeMode::ReadOnly {
        SafeMode::PasswordGated
    } else {
        profile.safe_mode
    }
}

pub fn prepare(
    session_id: &str,
    profile: &ConnectionProfile,
    sql: &str,
) -> AppResult<PreparedPlan> {
    if sql.trim().is_empty() {
        return Err(AppError::invalid("Write a query before running it"));
    }
    if sql.len() > 1024 * 1024 {
        return Err(AppError::invalid(
            "A query batch must be smaller than 1 MiB",
        ));
    }
    if profile.engine == EngineKind::Mysql {
        let tokens = Tokenizer::new(&MySqlDialect {}, sql)
            .tokenize()
            .map_err(|_| AppError::invalid("The SQL contains an unterminated token"))?;
        if tokens.iter().any(|token|matches!(token,Token::Whitespace(sqlparser::tokenizer::Whitespace::MultiLineComment(comment)) if comment.starts_with('!')||comment.to_ascii_uppercase().starts_with("M!"))) {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "MySQL executable comments are not supported. Submit their SQL explicitly for review.",
            ));
        }
        let words = tokens
            .iter()
            .filter_map(|t| {
                if let Token::Word(w) = t {
                    Some(w.value.to_ascii_uppercase())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if words.iter().any(|w| w == "SET")
            && (words.iter().any(|w| {
                [
                    "SQL_MODE",
                    "CHARACTER_SET_CLIENT",
                    "CHARACTER_SET_CONNECTION",
                    "CHARACTER_SET_RESULTS",
                    "COLLATION_CONNECTION",
                ]
                .contains(&w.as_str())
            }) || words
                .windows(2)
                .any(|w| w == ["SET", "NAMES"] || w == ["SET", "CHARACTER"]))
        {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Joinery manages MySQL SQL mode and character encoding to keep query review and execution consistent.",
            ));
        }
    }
    let dialect: Box<dyn Dialect> = match profile.engine {
        EngineKind::Postgres => Box::new(PostgreSqlDialect {}),
        EngineKind::Mysql => Box::new(MySqlDialect {}),
        EngineKind::Snowflake => Box::new(SnowflakeDialect {}),
        EngineKind::Redis => {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Use the Redis command console for this connection",
            ));
        }
    };
    let statements = match Parser::parse_sql(dialect.as_ref(), sql) {
        Ok(ast) => {
            let parts = split_sql(dialect.as_ref(), sql)?;
            if ast.is_empty() {
                return Err(AppError::invalid("The query contains only comments"));
            }
            if parts.len() != ast.len() {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "This procedural batch cannot be split safely. Execute supported statements separately.",
                ));
            }
            ast.iter()
                .zip(parts)
                .map(|(statement, sql)| {
                    let mut classification = Classification {
                        engine: Some(profile.engine),
                        ..Default::default()
                    };
                    let _ = statement.visit(&mut classification);
                    StatementPlan {
                        sql,
                        kind: classification.kind.unwrap_or(StatementKind::Unknown),
                        missing_where: classification.missing_where,
                    }
                })
                .collect::<Vec<_>>()
        }
        Err(_) => vec![StatementPlan {
            sql: sql.to_string(),
            kind: StatementKind::Unknown,
            missing_where: false,
        }],
    };
    if statements.len() > 100 {
        return Err(AppError::invalid("Run at most 100 statements in one batch"));
    }
    let mode = effective_mode(profile);
    let unsafe_batch = statements.iter().any(needs_authorization);
    if mode == SafeMode::ReadOnly && unsafe_batch {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "This connection is read-only. The batch contains a write, a session change, or SQL that could not be classified.",
        ));
    }
    let mut warnings = Vec::new();
    if statements.iter().any(|s| s.missing_where) {
        warnings
            .push("An UPDATE or DELETE has no WHERE clause or an unrestricted predicate.".into());
    }
    if statements.iter().any(|s| s.kind == StatementKind::Ddl) {
        warnings
            .push("This batch changes database structure. Some DDL cannot be rolled back.".into());
    }
    if statements.iter().any(|s| s.kind == StatementKind::Unknown) {
        warnings.push("Part of this batch could not be classified. Review its complete effects before continuing.".into());
    }
    let confirmation_text = if matches!(mode, SafeMode::Guard | SafeMode::PasswordGated)
        && statements
            .iter()
            .any(|s| s.missing_where || s.kind == StatementKind::Ddl)
    {
        Some(profile.name.clone())
    } else {
        None
    };
    Ok(PreparedPlan {
        id: new_id(),
        session_id: session_id.into(),
        statements,
        requires_approval: unsafe_batch && mode != SafeMode::Silent,
        requires_passphrase: unsafe_batch && mode == SafeMode::PasswordGated,
        confirmation_text,
        warnings,
        expires_at: now_ms() + 60_000,
    })
}

pub fn needs_authorization(statement: &StatementPlan) -> bool {
    let rollback = statement.kind == StatementKind::Transaction
        && statement
            .sql
            .split_whitespace()
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("ROLLBACK"));
    statement.kind != StatementKind::Read && !rollback
}

pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub fn split_sql(dialect: &dyn Dialect, sql: &str) -> AppResult<Vec<String>> {
    let tokens = Tokenizer::new(dialect, sql)
        .tokenize_with_location()
        .map_err(|_| AppError::invalid("The SQL contains an unterminated string or comment"))?;
    let mut parts = Vec::new();
    let mut start = 0;
    let mut content = false;
    for token in tokens {
        match token.token {
            Token::SemiColon => {
                let end = byte_offset(sql, token.span.start);
                if content {
                    parts.push(sql[start..end].trim().to_string());
                }
                start = end + 1;
                content = false;
            }
            Token::Whitespace(_) | Token::EOF => {}
            _ => content = true,
        }
    }
    if content {
        parts.push(sql[start..].trim().to_string());
    }
    Ok(parts)
}

fn byte_offset(sql: &str, location: Location) -> usize {
    let line = location.line.saturating_sub(1) as usize;
    let start = sql
        .split_inclusive('\n')
        .take(line)
        .map(str::len)
        .sum::<usize>();
    start
        + sql[start..]
            .char_indices()
            .nth(location.column.saturating_sub(1) as usize)
            .map(|(i, _)| i)
            .unwrap_or(sql.len() - start)
}

#[derive(Default)]
struct Classification {
    engine: Option<EngineKind>,
    kind: Option<StatementKind>,
    missing_where: bool,
}
impl Classification {
    fn raise(&mut self, kind: StatementKind) {
        fn rank(k: StatementKind) -> u8 {
            match k {
                StatementKind::Read => 0,
                StatementKind::Transaction => 1,
                StatementKind::Write => 2,
                StatementKind::Ddl => 3,
                StatementKind::Unknown => 4,
            }
        }
        if self.kind.is_none_or(|current| rank(kind) > rank(current)) {
            self.kind = Some(kind);
        }
    }
}

impl Visitor for Classification {
    type Break = ();
    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<()> {
        match statement {
            Statement::Query(_) => self.raise(StatementKind::Read),
            Statement::Update(update) => {
                self.raise(StatementKind::Write);
                self.missing_where |= unrestricted(update.selection.as_ref());
            }
            Statement::Delete(delete) => {
                self.raise(StatementKind::Write);
                self.missing_where |= unrestricted(delete.selection.as_ref());
            }
            Statement::Insert(_) => self.raise(StatementKind::Write),
            Statement::StartTransaction { .. }
            | Statement::Commit { .. }
            | Statement::Rollback { .. } => self.raise(StatementKind::Transaction),
            Statement::ShowVariable { .. }
            | Statement::ShowVariables { .. }
            | Statement::ShowTables { .. }
            | Statement::ShowColumns { .. }
            | Statement::ShowSchemas { .. }
            | Statement::ShowDatabases { .. }
            | Statement::ShowCreate { .. }
            | Statement::ShowProcessList { .. } => self.raise(StatementKind::Read),
            Statement::Explain { analyze: false, .. } => self.raise(StatementKind::Read),
            _ => {
                let first = statement
                    .to_string()
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_uppercase();
                self.raise(
                    if ["CREATE", "ALTER", "DROP", "TRUNCATE", "GRANT", "REVOKE"]
                        .contains(&first.as_str())
                    {
                        StatementKind::Ddl
                    } else {
                        StatementKind::Unknown
                    },
                );
            }
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        if let SetExpr::Select(select) = query.body.as_ref()
            && select.into.is_some()
        {
            self.raise(StatementKind::Write);
        }
        if !query.locks.is_empty() {
            self.raise(StatementKind::Unknown);
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_expr(&mut self, expression: &sqlparser::ast::Expr) -> ControlFlow<()> {
        if let sqlparser::ast::Expr::Function(function) = expression {
            let name = function.name.to_string().to_lowercase();
            let builtin = if self.engine == Some(EngineKind::Postgres) {
                name.strip_prefix("pg_catalog.").unwrap_or(&name)
            } else {
                &name
            };
            if ![
                "count",
                "sum",
                "min",
                "max",
                "avg",
                "coalesce",
                "nullif",
                "lower",
                "upper",
                "length",
                "char_length",
                "concat",
                "substring",
                "trim",
                "round",
                "abs",
                "floor",
                "ceil",
                "now",
                "date_trunc",
                "extract",
                "to_char",
                "array_agg",
                "string_agg",
                "json_agg",
                "jsonb_agg",
                "row_number",
                "rank",
                "dense_rank",
                "lag",
                "lead",
                "pg_sleep",
                "generate_series",
                "version",
                "current_database",
                "current_schema",
            ]
            .contains(&builtin)
                && !(self.engine == Some(EngineKind::Mysql)
                    && [
                        "database",
                        "current_user",
                        "connection_id",
                        "sleep",
                        "hex",
                        "json_extract",
                        "json_unquote",
                        "date_format",
                        "utc_timestamp",
                    ]
                    .contains(&builtin))
            {
                self.raise(StatementKind::Unknown);
            }
        }
        ControlFlow::Continue(())
    }
}

fn unrestricted(selection: Option<&sqlparser::ast::Expr>) -> bool {
    selection.is_none_or(|expr| {
        matches!(
            expr.to_string()
                .to_uppercase()
                .replace([' ', '(', ')'], "")
                .as_str(),
            "TRUE" | "1=1" | "0=0"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn profile(mode: SafeMode, environment: Environment) -> ConnectionProfile {
        ConnectionProfile {
            id: new_id(),
            name: "Test".into(),
            group_id: "local".into(),
            engine: EngineKind::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            database: "test".into(),
            username: "test".into(),
            environment,
            safe_mode: mode,
            tls_mode: TlsMode::Disable,
            ca_cert_path: None,
            ssh: None,
            remember_password: false,
            store_query_text: false,
            snowflake: None,
        }
    }
    #[test]
    fn splitter_preserves_strings_comments_dollar_quotes_and_unicode() {
        let sql = "SELECT 'é;'; -- keep ;\nSELECT $$x;y$$; /* trailing ; */";
        let p = prepare("s", &profile(SafeMode::Silent, Environment::Local), sql).unwrap();
        assert_eq!(p.statements.len(), 2);
        assert_eq!(p.statements[0].sql, "SELECT 'é;'");
        assert!(p.statements[1].sql.contains("$$x;y$$"));
    }
    #[test]
    fn production_cannot_silently_bypass_password_policy() {
        let p = prepare(
            "s",
            &profile(SafeMode::Silent, Environment::Production),
            "UPDATE accounts SET active=false WHERE id=1",
        )
        .unwrap();
        assert!(p.requires_approval && p.requires_passphrase);
    }
    #[test]
    fn detects_nested_writes_and_real_where_clauses() {
        let p = prepare(
            "s",
            &profile(SafeMode::Guard, Environment::Staging),
            "WITH gone AS (DELETE FROM a RETURNING *) SELECT * FROM gone",
        )
        .unwrap();
        assert!(p.requires_approval);
        assert!(p.statements[0].missing_where);
        let p = prepare(
            "s",
            &profile(SafeMode::Guard, Environment::Staging),
            "UPDATE a SET x=(SELECT x FROM b WHERE id=1)",
        )
        .unwrap();
        assert!(p.statements[0].missing_where);
    }
    #[test]
    fn read_only_blocks_unparsed_and_select_into() {
        for sql in [
            "SELECT 1 INTO new_table",
            "SOME UNKNOWN COMMAND",
            "DELETE FROM a WHERE id=1",
        ] {
            assert_eq!(
                prepare("s", &profile(SafeMode::ReadOnly, Environment::Local), sql)
                    .unwrap_err()
                    .code,
                ErrorCode::PolicyBlocked
            );
        }
    }
    #[test]
    fn snowflake_read_only_blocks_writes_and_unknown_sql() {
        let mut p = profile(SafeMode::ReadOnly, Environment::Production);
        p.engine = EngineKind::Snowflake;
        assert_eq!(
            prepare("s", &p, "SELECT 1").unwrap().statements[0].kind,
            StatementKind::Read
        );
        for sql in [
            "DELETE FROM T WHERE ID = 1",
            "CREATE TABLE T (ID INT)",
            "EXECUTE IMMEDIATE 'DELETE FROM T'",
        ] {
            assert_eq!(
                prepare("s", &p, sql).unwrap_err().code,
                ErrorCode::PolicyBlocked
            );
        }
    }
    #[test]
    fn comments_do_not_turn_a_write_into_a_read() {
        let p = prepare(
            "s",
            &profile(SafeMode::Confirm, Environment::Staging),
            "/* SELECT */ DELETE FROM a WHERE TRUE",
        )
        .unwrap();
        assert!(p.requires_approval && p.statements[0].missing_where);
    }
    #[test]
    fn unparsed_rollback_prefix_and_side_effecting_functions_require_approval() {
        for sql in [
            "ROLLBACK; SOME UNKNOWN COMMAND; DELETE FROM a",
            "SELECT pg_terminate_backend(1)",
        ] {
            let p = prepare(
                "s",
                &profile(SafeMode::Silent, Environment::Production),
                sql,
            )
            .unwrap();
            assert!(p.requires_approval && p.requires_passphrase);
        }
    }
}
