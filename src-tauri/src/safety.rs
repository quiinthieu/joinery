use crate::domain::*;
use sqlparser::{
    ast::{Query, SetExpr, Statement, Visit, Visitor},
    dialect::{Dialect, MySqlDialect, PostgreSqlDialect, SnowflakeDialect},
    parser::Parser,
    tokenizer::{Location, Token, Tokenizer},
};
use std::ops::ControlFlow;

pub(crate) fn has_mysql_executable_comment(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                    } else if bytes[i] == quote {
                        if bytes.get(i + 1) == Some(&quote) {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'`' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'`' {
                        if bytes.get(i + 1) == Some(&b'`') {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'#' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'-' if bytes.get(i + 1) == Some(&b'-')
                && bytes
                    .get(i + 2)
                    .is_none_or(|b| b.is_ascii_whitespace() || b.is_ascii_control()) =>
            {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                if bytes.get(i + 2) == Some(&b'!')
                    || (matches!(bytes.get(i + 2), Some(b'M' | b'm'))
                        && bytes.get(i + 3) == Some(&b'!'))
                {
                    return true;
                }
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    false
}

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
    prepare_inner(session_id, profile, sql, None)
}

pub fn prepare_parameterized(
    session_id: &str,
    profile: &ConnectionProfile,
    original_sql: &str,
    rewritten: &crate::params::ClassificationRewrite,
) -> AppResult<PreparedPlan> {
    prepare_inner(session_id, profile, original_sql, Some(rewritten))
}

fn prepare_inner(
    session_id: &str,
    profile: &ConnectionProfile,
    original_sql: &str,
    rewritten: Option<&crate::params::ClassificationRewrite>,
) -> AppResult<PreparedPlan> {
    let parse_sql = rewritten.map_or(original_sql, |r| r.sql.as_str());
    if original_sql.trim().is_empty() {
        return Err(AppError::invalid("Write a query before running it"));
    }
    if original_sql.len() > 1024 * 1024 {
        return Err(AppError::invalid(
            "A query batch must be smaller than 1 MiB",
        ));
    }
    if profile.engine == EngineKind::Redis {
        return crate::drivers::redis::commands::prepare(session_id, profile, original_sql);
    }
    // sqlparser does not support Snowflake's USING JSON syntax. Recognize
    // only our exact wrapper, validate the entire inner query, then apply the
    // existing read classification. Never whitelist an EXPLAIN prefix alone.
    if profile.engine == EngineKind::Snowflake
        && let Some(query) = original_sql.strip_prefix("EXPLAIN USING JSON\n")
    {
        let query_span = crate::sql_text::span_for_byte_range(
            original_sql,
            original_sql.len() - query.len(),
            original_sql.len(),
        );
        let shift_error = |mut error: AppError| {
            if let Some(location) = error.location.as_mut() {
                location.span = crate::sql_text::shift_child_span(query_span, location.span);
            }
            error
        };
        let inner_rewrite = crate::params::rewrite_for_classification(profile.engine, query)?;
        crate::query_plan::explain_sql(profile.engine, &inner_rewrite.sql, false, None).map_err(
            |mut error| {
                if let Some(location) = error.location.as_mut() {
                    location.span = inner_rewrite.map.span_from_rewritten_line_char_col(
                        query,
                        &inner_rewrite.sql,
                        u64::from(location.span.line),
                        u64::from(location.span.column),
                    );
                }
                shift_error(error)
            },
        )?;
        let inner = prepare_parameterized(session_id, profile, query, &inner_rewrite)
            .map_err(shift_error)?;
        if inner.statements.len() != 1 || inner.statements[0].kind != StatementKind::Read {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Explain requires one read-only query",
            ));
        }
        return plan_for_statements(
            session_id,
            profile,
            vec![StatementPlan {
                sql: original_sql.to_owned(),
                kind: StatementKind::Read,
                missing_where: false,
                span: Some(crate::sql_text::span_for_byte_range(
                    original_sql,
                    0,
                    original_sql.len(),
                )),
            }],
        );
    }
    let mut mysql_analysis = false;
    if profile.engine == EngineKind::Mysql {
        if has_mysql_executable_comment(original_sql) {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "MySQL executable comments are not supported. Submit their SQL explicitly for review.",
            ));
        }
        let tokens = Tokenizer::new(&MySqlDialect {}, parse_sql)
            .tokenize()
            .map_err(|e| {
                let err = tokenizer_error(
                    parse_sql,
                    "The SQL contains an unterminated token",
                    e.location,
                );
                if let Some(rew) = rewritten {
                    let b = byte_offset(parse_sql, e.location);
                    let span = rew.map.span_at_rewritten_byte(original_sql, b);
                    AppError::invalid("The SQL contains an unterminated token").with_location(
                        ErrorLocation {
                            span,
                            statement_index: Some(0),
                            sql_state: None,
                            detail: None,
                            hint: None,
                        },
                    )
                } else {
                    err
                }
            })?;
        mysql_analysis = mysql_analysis_tokens(&tokens);
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
    let mut executes_analysis = mysql_analysis;
    let mut parser_error_location: Option<ErrorLocation> = None;
    let statements = match Parser::parse_sql(dialect.as_ref(), parse_sql) {
        Ok(ast) => {
            let ranges = split_sql_byte_ranges(dialect.as_ref(), parse_sql)?;
            if ast.is_empty() {
                return Err(AppError::invalid("The query contains only comments"));
            }
            if ranges.len() != ast.len() {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "This procedural batch cannot be split safely. Execute supported statements separately.",
                ));
            }
            ast.iter()
                .zip(ranges)
                .map(|(statement, (rew_start, rew_end))| {
                    let (orig_start, orig_end) = if let Some(rew) = rewritten {
                        (
                            rew.to_original_byte(original_sql.len(), rew_start),
                            rew.to_original_end_byte(original_sql.len(), rew_end),
                        )
                    } else {
                        (rew_start, rew_end)
                    };
                    let mut classification = Classification {
                        engine: Some(profile.engine),
                        ..Default::default()
                    };
                    let _ = statement.visit(&mut classification);
                    executes_analysis |= classification.executes_analysis;
                    StatementPlan {
                        sql: original_sql[orig_start..orig_end].to_string(),
                        kind: classification.kind.unwrap_or(StatementKind::Unknown),
                        missing_where: classification.missing_where,
                        span: Some(crate::sql_text::span_for_byte_range(
                            original_sql,
                            orig_start,
                            orig_end,
                        )),
                    }
                })
                .collect::<Vec<_>>()
        }
        Err(err) => {
            let err_text = err.to_string();
            let mapped_span = if let Some(rew) = rewritten {
                crate::sql_text::span_from_sqlparser_error(parse_sql, &err_text).map(|s| {
                    let b = byte_offset(
                        parse_sql,
                        Location {
                            line: u64::from(s.line),
                            column: u64::from(s.column),
                        },
                    );
                    rew.map.span_at_rewritten_byte(original_sql, b)
                })
            } else {
                crate::sql_text::span_from_sqlparser_error(original_sql, &err_text)
            };
            if let Some(span) = mapped_span {
                parser_error_location = Some(ErrorLocation {
                    span,
                    statement_index: Some(0),
                    sql_state: None,
                    detail: Some(err_text),
                    hint: None,
                });
            }
            vec![StatementPlan {
                sql: original_sql.to_string(),
                kind: StatementKind::Unknown,
                missing_where: false,
                // Parser fallback submits the complete original buffer, including
                // leading whitespace. Its span must share that same origin.
                span: Some(crate::sql_text::span_for_byte_range(
                    original_sql,
                    0,
                    original_sql.len(),
                )),
            }]
        }
    };
    if statements.len() > 100 {
        return Err(AppError::invalid("Run at most 100 statements in one batch"));
    }
    let mut plan = plan_for_statements(session_id, profile, statements).map_err(|mut err| {
        if err.location.is_none()
            && let Some(loc) = parser_error_location.take()
        {
            err.location = Some(Box::new(loc));
        }
        err
    })?;
    if executes_analysis {
        // This gate also applies to SQL entered directly, including Silent
        // profiles. Set it before the plan is stored and fingerprinted.
        plan.requires_approval = true;
        plan.warnings.push(
            if mysql_analysis {
                "ANALYZE executes the statement. Side effects occur normally; execution may add profiling overhead."
            } else {
                "EXPLAIN ANALYZE executes the query. Side effects occur normally; execution may add profiling overhead."
            }.into(),
        );
    }
    Ok(plan)
}

/// MySQL may return a partial analysis result rather than an error after KILL.
/// Share the conservative pre-parser gate with its cancellation checkpoint.
pub(crate) fn mysql_executes_analysis(sql: &str) -> bool {
    Tokenizer::new(&MySqlDialect {}, sql)
        .tokenize()
        .is_ok_and(|tokens| mysql_analysis_tokens(&tokens))
}

fn mysql_analysis_tokens(tokens: &[Token]) -> bool {
    // Includes MariaDB syntax and EXPLAIN's DESC/DESCRIBE aliases, even when
    // sqlparser cannot parse options such as FOR SCHEMA. Quoted text is inert.
    let mut start = true;
    let mut explain = false;
    for token in tokens {
        match token {
            Token::Whitespace(_) | Token::EOF => {}
            Token::SemiColon => {
                start = true;
                explain = false;
            }
            Token::Word(word) if word.quote_style.is_none() => {
                if word.value.eq_ignore_ascii_case("ANALYZE") && (start || explain) {
                    return true;
                }
                if start {
                    explain = ["EXPLAIN", "DESC", "DESCRIBE"]
                        .iter()
                        .any(|alias| word.value.eq_ignore_ascii_case(alias));
                }
                start = false;
            }
            _ => start = false,
        }
    }
    false
}

pub(crate) fn plan_for_statements(
    session_id: &str,
    profile: &ConnectionProfile,
    statements: Vec<StatementPlan>,
) -> AppResult<PreparedPlan> {
    let mode = effective_mode(profile);
    let first_unsafe = statements
        .iter()
        .enumerate()
        .find(|(_, s)| needs_authorization(s));
    let unsafe_batch = first_unsafe.is_some();
    if mode == SafeMode::ReadOnly
        && let Some((idx, stmt)) = first_unsafe
    {
        let mut err = AppError::new(
            ErrorCode::PolicyBlocked,
            "This connection is read-only. The batch contains a write, a session change, or SQL that could not be classified.",
        );
        if stmt.kind != StatementKind::Unknown
            && let Some(span) = stmt.span
        {
            err.location = Some(Box::new(ErrorLocation {
                span,
                statement_index: Some(idx),
                sql_state: None,
                detail: None,
                hint: None,
            }));
        }
        return Err(err);
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
        bound_parameters: Vec::new(),
        binding_digest: None,
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
    Ok(split_sql_spanned(dialect, sql)?
        .into_iter()
        .map(|(s, _)| s)
        .collect())
}

pub fn split_sql_spanned(dialect: &dyn Dialect, sql: &str) -> AppResult<Vec<(String, SqlSpan)>> {
    Ok(split_sql_byte_ranges(dialect, sql)?
        .into_iter()
        .map(|(s, e)| {
            (
                sql[s..e].to_string(),
                crate::sql_text::span_for_byte_range(sql, s, e),
            )
        })
        .collect())
}

fn split_sql_byte_ranges(dialect: &dyn Dialect, sql: &str) -> AppResult<Vec<(usize, usize)>> {
    let tokens = Tokenizer::new(dialect, sql)
        .tokenize_with_location()
        .map_err(|e| {
            tokenizer_error(
                sql,
                "The SQL contains an unterminated string or comment",
                e.location,
            )
        })?;
    let mut parts = Vec::new();
    let mut start = 0;
    let mut content = false;
    for token in tokens {
        match token.token {
            Token::SemiColon => {
                let end = byte_offset(sql, token.span.start);
                if content && let Some(range) = trimmed_range(sql, start, end) {
                    parts.push(range);
                }
                start = end + 1;
                content = false;
            }
            Token::Whitespace(_) | Token::EOF => {}
            _ => content = true,
        }
    }
    if content && let Some(range) = trimmed_range(sql, start, sql.len()) {
        parts.push(range);
    }
    Ok(parts)
}

fn trimmed_range(sql: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let slice = &sql[start..end];
    let leading = slice.len() - slice.trim_start().len();
    let trimmed = slice.trim();
    if trimmed.is_empty() {
        return None;
    }
    let trimmed_start = start + leading;
    let trimmed_end = trimmed_start + trimmed.len();
    Some((trimmed_start, trimmed_end))
}

fn tokenizer_error(sql: &str, message: &str, location: Location) -> AppError {
    let span = crate::sql_text::span_from_line_char_col(sql, location.line, location.column);
    AppError::invalid(message).with_location(ErrorLocation {
        span,
        statement_index: Some(0),
        sql_state: None,
        detail: None,
        hint: None,
    })
}

pub(crate) fn byte_offset(sql: &str, location: Location) -> usize {
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
    executes_analysis: bool,
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
            Statement::Explain {
                analyze, options, ..
            } => {
                let executes = *analyze
                    || options.as_deref().unwrap_or_default().iter().any(|option| {
                        option.name.value.eq_ignore_ascii_case("analyze")
                            && !option.arg.as_ref().is_some_and(|arg| {
                                matches!(
                                    arg.to_string().to_ascii_lowercase().as_str(),
                                    "false" | "off" | "0" | "'false'" | "'off'" | "'0'"
                                )
                            })
                    });
                self.executes_analysis |= executes;
                self.raise(if executes {
                    StatementKind::Unknown
                } else {
                    StatementKind::Read
                });
            }
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

    #[test]
    fn serialized_policy_error_exposes_unicode_parser_location() {
        let sql = "SELECT '🧪';\n  SELEC broken";
        let error =
            prepare("s", &profile(SafeMode::ReadOnly, Environment::Local), sql).unwrap_err();
        let json = serde_json::to_value(error).unwrap();
        assert_eq!(json["code"], "policyBlocked");
        assert_eq!(json["location"]["span"]["line"], 2);
        assert_eq!(json["location"]["span"]["column"], 3);
        assert_eq!(json["location"]["span"]["startUtf16"], 15);
        assert_eq!(json["location"]["span"]["endUtf16"], 20);
    }
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
            store_drafts: false,
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
    fn generated_snowflake_explain_is_read_only_without_allowing_a_batch_escape() {
        let mut p = profile(SafeMode::ReadOnly, Environment::Local);
        p.engine = EngineKind::Snowflake;
        let sql = "EXPLAIN USING JSON\n/* preserve */ SELECT 'é;' AS note, id FROM lab.public.events QUALIFY ROW_NUMBER() OVER (ORDER BY id)=1; -- end";
        let plan = prepare("s", &p, sql).unwrap();
        assert_eq!(plan.statements.len(), 1);
        assert_eq!(plan.statements[0].sql, sql);
        assert_eq!(plan.statements[0].kind, StatementKind::Read);
        assert!(!plan.requires_approval);
        for query in [
            "SELECT 1; DELETE FROM t",
            "SELECT 1; SELECT 2",
            "DELETE FROM t",
            "EXPLAIN SELECT 1",
            "SELECT '",
            "SELECT 1 INTO new_table",
        ] {
            assert!(
                prepare("s", &p, &format!("EXPLAIN USING JSON\n{query}")).is_err(),
                "{query}"
            );
        }
    }

    #[test]
    fn direct_snowflake_wrapper_shifts_parser_and_policy_locations_once() {
        let mut p = profile(SafeMode::ReadOnly, Environment::Local);
        p.engine = EngineKind::Snowflake;
        for (query, expected, line) in [
            ("/* 🧪 */\nSELEC 1", "SELEC", 3),
            (
                "/* 🧪 */ SELECT 1 INTO new_table",
                "/* 🧪 */ SELECT 1 INTO new_table",
                2,
            ),
        ] {
            let sql = format!("EXPLAIN USING JSON\n{query}");
            let error = prepare("s", &p, &sql).unwrap_err();
            let location = error.location.expect("wrapper error location");
            let units: Vec<u16> = sql.encode_utf16().collect();
            assert_eq!(
                String::from_utf16(&units[location.span.start_utf16..location.span.end_utf16])
                    .unwrap(),
                expected
            );
            assert_eq!(location.span.line, line);
        }
    }

    #[test]
    fn parser_fallback_retains_the_submitted_buffer_origin() {
        let sql = " \n  SELEC '🧪'";
        let plan = prepare("s", &profile(SafeMode::Silent, Environment::Local), sql).unwrap();
        assert_eq!(plan.statements.len(), 1);
        assert_eq!(plan.statements[0].sql, sql);
        let origin = plan.statements[0].span.unwrap();
        assert_eq!(origin.start_utf16, 0);
        let child = crate::sql_text::span_from_line_char_col(sql, 2, 3);
        assert_eq!(crate::sql_text::shift_child_span(origin, child), child);
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

    #[test]
    fn estimated_plans_keep_existing_nested_statement_and_function_guards() {
        let readonly = profile(SafeMode::ReadOnly, Environment::Local);
        let simple =
            crate::query_plan::explain_sql(EngineKind::Postgres, "SELECT 1", false, None).unwrap();
        let plan = prepare("s", &readonly, &simple).unwrap();
        assert!(!plan.requires_approval);
        for sql in [
            "SELECT pg_terminate_backend(1)",
            "WITH gone AS (DELETE FROM a RETURNING *) SELECT * FROM gone",
            "SELECT * FROM a FOR UPDATE",
        ] {
            let explained =
                crate::query_plan::explain_sql(EngineKind::Postgres, sql, false, None).unwrap();
            assert!(prepare("s", &readonly, &explained).is_err(), "{sql}");
            let production = profile(SafeMode::Silent, Environment::Production);
            let plan = prepare("s", &production, &explained).unwrap();
            assert!(plan.requires_approval && plan.requires_passphrase, "{sql}");
        }
    }

    #[test]
    fn mysql_and_mariadb_analysis_requires_review_even_when_parsing_falls_back() {
        let mut p = profile(SafeMode::Silent, Environment::Local);
        p.engine = EngineKind::Mysql;
        for sql in [
            "ANALYZE FORMAT=JSON SELECT 1",
            "/* analysis */ ANALYZE FORMAT = JSON SELECT 'é; value'",
            "SELECT 1; -- next\n ANALYZE FORMAT=JSON SELECT 2",
            "ANALYZE SELECT 1",
            "EXPLAIN ANALYZE FORMAT=JSON SELECT 1",
            "EXPLAIN ANALYZE FORMAT=TREE SELECT 1",
            "DESC ANALYZE FORMAT=TREE SELECT 1",
            "DESC ANALYZE FORMAT=TREE FOR SCHEMA joinery SELECT 1",
            "DESCRIBE ANALYZE FORMAT=TREE FOR DATABASE joinery SELECT 1",
            "SELECT 1; DESCRIBE /* review */ ANALYZE FORMAT=TREE SELECT 1",
        ] {
            let plan = prepare("s", &p, sql).unwrap();
            assert!(plan.requires_approval, "{sql}");
            assert!(
                plan.warnings.iter().any(|w| w.contains("executes")),
                "{sql}"
            );
            p.safe_mode = SafeMode::ReadOnly;
            assert_eq!(
                prepare("s", &p, sql).unwrap_err().code,
                ErrorCode::PolicyBlocked
            );
            p.safe_mode = SafeMode::Silent;
            p.environment = Environment::Production;
            let production = prepare("s", &p, sql).unwrap();
            assert!(production.requires_approval && production.requires_passphrase);
            p.environment = Environment::Local;
        }
        for sql in [
            "SELECT 'ANALYZE FORMAT=JSON'",
            "SELECT 1 /* ANALYZE SELECT 2 */",
            "SELECT `analyze` FROM sample",
            "DESC `analyze`",
        ] {
            assert!(!prepare("s", &p, sql).unwrap().requires_approval, "{sql}");
        }
    }

    #[test]
    fn executing_explain_requires_review_even_in_silent_mode() {
        for sql in [
            "EXPLAIN ANALYZE SELECT 1",
            "EXPLAIN (ANALYZE, FORMAT JSON) SELECT 1",
            "EXPLAIN (ANALYZE TRUE, FORMAT JSON) SELECT 1",
            "EXPLAIN (ANALYZE 'on') SELECT 1",
            "SELECT 1; EXPLAIN (ANALYZE 1) SELECT 2",
        ] {
            let plan = prepare("s", &profile(SafeMode::Silent, Environment::Local), sql).unwrap();
            assert!(plan.requires_approval, "{sql}");
            assert!(!plan.requires_passphrase, "{sql}");
            assert!(
                plan.warnings
                    .iter()
                    .any(|w| w.contains("executes the query")),
                "{sql}"
            );
            assert_eq!(
                prepare("s", &profile(SafeMode::ReadOnly, Environment::Local), sql)
                    .unwrap_err()
                    .code,
                ErrorCode::PolicyBlocked,
                "{sql}"
            );
            let production = prepare(
                "s",
                &profile(SafeMode::Silent, Environment::Production),
                sql,
            )
            .unwrap();
            assert!(
                production.requires_approval && production.requires_passphrase,
                "{sql}"
            );
        }
        for sql in [
            "EXPLAIN (ANALYZE FALSE, FORMAT JSON) SELECT 1",
            "EXPLAIN (ANALYZE 'off') SELECT 1",
            "EXPLAIN (ANALYZE 0) SELECT 1",
            "EXPLAIN SELECT 'ANALYZE'",
        ] {
            let plan = prepare("s", &profile(SafeMode::ReadOnly, Environment::Local), sql).unwrap();
            assert!(!plan.requires_approval, "{sql}");
        }
    }
    #[test]
    fn attaches_utf16_statement_spans_and_error_locations_on_syntax_and_policy_errors() {
        let batch = "SELECT '🧪';\n  SELECT 42";
        let plan = prepare("s", &profile(SafeMode::Silent, Environment::Local), batch).unwrap();
        assert_eq!(plan.statements.len(), 2);
        let units: Vec<u16> = batch.encode_utf16().collect();
        let s0 = plan.statements[0].span.unwrap();
        let s1 = plan.statements[1].span.unwrap();
        assert_eq!(
            String::from_utf16(&units[s0.start_utf16..s0.end_utf16]).unwrap(),
            "SELECT '🧪'"
        );
        assert_eq!(
            String::from_utf16(&units[s1.start_utf16..s1.end_utf16]).unwrap(),
            "SELECT 42"
        );
        assert_eq!((s1.line, s1.column), (2, 3));

        // MySQL unterminated string reports exact UTF-16 tokenizer location
        let mut mysql_prof = profile(SafeMode::Silent, Environment::Local);
        mysql_prof.engine = EngineKind::Mysql;
        let bad_quote = "SELECT 1;\nSELECT 'unterminated";
        let quote_err = prepare("s", &mysql_prof, bad_quote).unwrap_err();
        assert_eq!(quote_err.code, ErrorCode::InvalidInput);
        let quote_loc = quote_err.location.expect("tokenizer error location");
        assert_eq!(quote_loc.span.line, 2);

        // ReadOnly syntax error maps sqlparser Line/Column to offending token in UTF-16
        let ro_syntax_sql = "SELECT '🧪';\n  SELEC broken FROM orders";
        let ro_syntax_err = prepare(
            "s",
            &profile(SafeMode::ReadOnly, Environment::Local),
            ro_syntax_sql,
        )
        .unwrap_err();
        assert_eq!(ro_syntax_err.code, ErrorCode::PolicyBlocked);
        let ro_syntax_loc = ro_syntax_err.location.expect("readonly syntax location");
        assert_eq!((ro_syntax_loc.span.line, ro_syntax_loc.span.column), (2, 3));
        let syntax_units: Vec<u16> = ro_syntax_sql.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(
                &syntax_units[ro_syntax_loc.span.start_utf16..ro_syntax_loc.span.end_utf16]
            )
            .unwrap(),
            "SELEC"
        );

        // ReadOnly write failure on statement index 1 reports statement_index and exact statement span
        let ro_write_sql = "SELECT '🧪';\n  DELETE FROM orders WHERE id = 1";
        let ro_write_err = prepare(
            "s",
            &profile(SafeMode::ReadOnly, Environment::Local),
            ro_write_sql,
        )
        .unwrap_err();
        assert_eq!(ro_write_err.code, ErrorCode::PolicyBlocked);
        let ro_write_loc = ro_write_err.location.expect("readonly policy location");
        assert_eq!(ro_write_loc.statement_index, Some(1));
        assert_eq!((ro_write_loc.span.line, ro_write_loc.span.column), (2, 3));
        let write_units: Vec<u16> = ro_write_sql.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(
                &write_units[ro_write_loc.span.start_utf16..ro_write_loc.span.end_utf16]
            )
            .unwrap(),
            "DELETE FROM orders WHERE id = 1"
        );
    }
}
