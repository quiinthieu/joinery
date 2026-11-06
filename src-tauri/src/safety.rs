use crate::domain::*;
use sqlparser::{
    ast::{
        Expr, GroupByExpr, Ident, Query, SelectItem, SelectItemQualifiedWildcardKind, SetExpr,
        Statement, TableFactor, Visit, Visitor, WildcardAdditionalOptions,
    },
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
                if profile.engine == EngineKind::Snowflake {
                    ranges
                        .into_iter()
                        .map(|(rew_start, rew_end)| {
                            let (orig_start, orig_end) = if let Some(rew) = rewritten {
                                (
                                    rew.to_original_byte(original_sql.len(), rew_start),
                                    rew.to_original_end_byte(original_sql.len(), rew_end),
                                )
                            } else {
                                (rew_start, rew_end)
                            };
                            let stmt_str = &parse_sql[rew_start..rew_end];
                            let kind = if let Ok(sub_ast) =
                                Parser::parse_sql(dialect.as_ref(), stmt_str)
                                && sub_ast.len() == 1
                            {
                                let mut classification = Classification {
                                    engine: Some(profile.engine),
                                    ..Default::default()
                                };
                                let _ = sub_ast[0].visit(&mut classification);
                                classification
                                    .kind
                                    .unwrap_or_else(|| classify_snowflake_statement(stmt_str))
                            } else {
                                classify_snowflake_statement(stmt_str)
                            };
                            StatementPlan {
                                sql: original_sql[orig_start..orig_end].to_string(),
                                kind,
                                missing_where: false,
                                span: Some(crate::sql_text::span_for_byte_range(
                                    original_sql,
                                    orig_start,
                                    orig_end,
                                )),
                            }
                        })
                        .collect::<Vec<_>>()
                } else {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "This procedural batch cannot be split safely. Execute supported statements separately.",
                    ));
                }
            } else {
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
        }
        Err(err) => {
            if profile.engine == EngineKind::Snowflake
                && let Ok(ranges) = split_sql_byte_ranges(dialect.as_ref(), parse_sql)
                && !ranges.is_empty()
            {
                ranges
                    .into_iter()
                    .map(|(rew_start, rew_end)| {
                        let (orig_start, orig_end) = if let Some(rew) = rewritten {
                            (
                                rew.to_original_byte(original_sql.len(), rew_start),
                                rew.to_original_end_byte(original_sql.len(), rew_end),
                            )
                        } else {
                            (rew_start, rew_end)
                        };
                        let stmt_str = &parse_sql[rew_start..rew_end];
                        let kind = classify_snowflake_statement(stmt_str);
                        StatementPlan {
                            sql: original_sql[orig_start..orig_end].to_string(),
                            kind,
                            missing_where: false,
                            span: Some(crate::sql_text::span_for_byte_range(
                                original_sql,
                                orig_start,
                                orig_end,
                            )),
                        }
                    })
                    .collect::<Vec<_>>()
            } else {
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

fn classify_snowflake_statement(sql: &str) -> StatementKind {
    let trimmed = sql.trim();
    let upper = trimmed.to_ascii_uppercase();
    let first = upper.split_whitespace().next().unwrap_or("");
    if first == "USE" {
        return StatementKind::Read;
    }
    if ["CREATE", "ALTER", "DROP", "TRUNCATE", "GRANT", "REVOKE"].contains(&first) {
        return StatementKind::Ddl;
    }
    if ["SELECT", "WITH", "SHOW", "DESCRIBE", "DESC"].contains(&first) {
        return StatementKind::Read;
    }
    if [
        "INSERT", "UPDATE", "DELETE", "MERGE", "CALL", "BEGIN", "DECLARE", "EXECUTE",
    ]
    .contains(&first)
    {
        if upper.contains("CREATE ")
            || upper.contains("DROP ")
            || upper.contains("ALTER ")
            || upper.contains("TRUNCATE ")
        {
            return StatementKind::Ddl;
        }
        return StatementKind::Write;
    }
    StatementKind::Unknown
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
    let mut begin_depth: usize = 0;
    let mut case_depth: usize = 0;
    let mut in_declare = false;

    let len = tokens.len();
    let mut i = 0;
    while i < len {
        let token = &tokens[i];
        match &token.token {
            Token::Word(w) if w.quote_style.is_none() => {
                let val = w.value.to_ascii_uppercase();
                match val.as_str() {
                    "CASE" => {
                        case_depth += 1;
                    }
                    "DECLARE" if begin_depth == 0 => {
                        in_declare = true;
                    }
                    "BEGIN" => {
                        let mut next_is_tx = false;
                        for j in (i + 1)..len {
                            match &tokens[j].token {
                                Token::Whitespace(_) => continue,
                                Token::SemiColon => {
                                    next_is_tx = true;
                                    break;
                                }
                                Token::Word(nw) if nw.quote_style.is_none() => {
                                    let nval = nw.value.to_ascii_uppercase();
                                    if nval == "TRANSACTION"
                                        || nval == "WORK"
                                        || nval == "DEFERRED"
                                        || nval == "IMMEDIATE"
                                        || nval == "EXCLUSIVE"
                                    {
                                        next_is_tx = true;
                                    }
                                    break;
                                }
                                _ => break,
                            }
                        }
                        if !next_is_tx {
                            begin_depth += 1;
                            in_declare = false;
                        }
                    }
                    "END" => {
                        let mut next_tok_idx = None;
                        for j in (i + 1)..len {
                            if !matches!(&tokens[j].token, Token::Whitespace(_)) {
                                next_tok_idx = Some(j);
                                break;
                            }
                        }

                        if let Some(next_idx) = next_tok_idx {
                            match &tokens[next_idx].token {
                                Token::Word(nw) if nw.quote_style.is_none() => {
                                    let next_val = nw.value.to_ascii_uppercase();
                                    match next_val.as_str() {
                                        "IF" | "LOOP" | "WHILE" | "REPEAT" => {
                                            // Closes compound control structure (END IF, END LOOP, etc.).
                                            // Neither begin_depth nor case_depth decrements.
                                            i = next_idx;
                                        }
                                        "FOR" => {
                                            let mut is_for_update = false;
                                            for k in (next_idx + 1)..len {
                                                match &tokens[k].token {
                                                    Token::Whitespace(_) => continue,
                                                    Token::Word(w2) if w2.quote_style.is_none() => {
                                                        let v2 = w2.value.to_ascii_uppercase();
                                                        if matches!(
                                                            v2.as_str(),
                                                            "UPDATE" | "SHARE" | "KEY" | "NO"
                                                        ) {
                                                            is_for_update = true;
                                                        }
                                                        break;
                                                    }
                                                    _ => break,
                                                }
                                            }
                                            if is_for_update {
                                                if case_depth > 0 {
                                                    case_depth -= 1;
                                                }
                                            } else {
                                                i = next_idx;
                                            }
                                        }
                                        "CASE" => {
                                            // Closes CASE statement: END CASE;
                                            if case_depth > 0 {
                                                case_depth -= 1;
                                            }
                                            i = next_idx;
                                        }
                                        _ => {
                                            if case_depth > 0 {
                                                case_depth -= 1;
                                            } else if begin_depth > 0 {
                                                begin_depth -= 1;
                                                // Check for block label followed by semicolon: END label;
                                                for k in (next_idx + 1)..len {
                                                    match &tokens[k].token {
                                                        Token::Whitespace(_) => continue,
                                                        Token::SemiColon => {
                                                            i = next_idx;
                                                            break;
                                                        }
                                                        _ => break,
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Token::SemiColon => {
                                    if case_depth > 0 {
                                        case_depth -= 1;
                                    } else if begin_depth > 0 {
                                        begin_depth -= 1;
                                    }
                                }
                                _ => {
                                    if case_depth > 0 {
                                        case_depth -= 1;
                                    } else if begin_depth > 0 {
                                        begin_depth -= 1;
                                    }
                                }
                            }
                        } else {
                            if case_depth > 0 {
                                case_depth -= 1;
                            } else if begin_depth > 0 {
                                begin_depth -= 1;
                            }
                        }
                    }
                    _ => {}
                }
                content = true;
            }
            Token::SemiColon => {
                if begin_depth == 0 && case_depth == 0 && !in_declare {
                    let end = byte_offset(sql, token.span.start);
                    if content && let Some(range) = trimmed_range(sql, start, end) {
                        parts.push(range);
                    }
                    start = end + 1;
                    content = false;
                }
            }
            Token::Whitespace(_) | Token::EOF => {}
            _ => content = true,
        }
        i += 1;
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
            | Statement::ShowProcessList { .. }
            | Statement::Use { .. } => self.raise(StatementKind::Read),
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
                self.raise(if first == "USE" {
                    StatementKind::Read
                } else if ["CREATE", "ALTER", "DROP", "TRUNCATE", "GRANT", "REVOKE"]
                    .contains(&first.as_str())
                {
                    StatementKind::Ddl
                } else {
                    StatementKind::Unknown
                });
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

/// Candidate single-table `SELECT` extracted from the SQL AST before live
/// catalog verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleTableSelectCandidate {
    pub engine: EngineKind,
    pub table: TableRef,
    pub projection: Vec<ProjectedSelectItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectedSelectItem {
    Wildcard,
    Column(Ident),
}

/// Outcome of verifying a `SingleTableSelectCandidate` against the live
/// `TableSchema` and result column metadata.
#[derive(Debug, Clone, PartialEq)]
pub enum ProvenanceResolution {
    Editable {
        schema: Box<TableSchema>,
        columns: Vec<ColumnInfo>,
        editability: ResultEditability,
    },
    ReadOnly {
        reason: String,
    },
}

fn dialect_for_provenance(engine: EngineKind) -> &'static dyn Dialect {
    static PG: PostgreSqlDialect = PostgreSqlDialect {};
    static MYSQL: MySqlDialect = MySqlDialect {};
    static SNOWFLAKE: SnowflakeDialect = SnowflakeDialect {};
    match engine {
        EngineKind::Postgres | EngineKind::Redis => &PG,
        EngineKind::Mysql => &MYSQL,
        EngineKind::Snowflake => &SNOWFLAKE,
    }
}

fn normalize_lookup_ident(engine: EngineKind, ident: &Ident) -> String {
    if engine == EngineKind::Postgres && ident.quote_style.is_none() {
        ident.value.to_ascii_lowercase()
    } else {
        ident.value.clone()
    }
}

fn ident_matches_target(engine: EngineKind, ident: &Ident, target: &str) -> bool {
    match engine {
        EngineKind::Postgres => {
            if ident.quote_style.is_some() {
                ident.value == target
            } else {
                ident.value.eq_ignore_ascii_case(target)
            }
        }
        EngineKind::Mysql => ident.value.eq_ignore_ascii_case(target),
        _ => false,
    }
}

fn ident_matches_ident(engine: EngineKind, left: &Ident, right: &Ident) -> bool {
    match engine {
        EngineKind::Postgres => match (left.quote_style.is_some(), right.quote_style.is_some()) {
            (true, true) => left.value == right.value,
            (true, false) => left.value == right.value.to_ascii_lowercase(),
            (false, true) => left.value.to_ascii_lowercase() == right.value,
            (false, false) => left.value.eq_ignore_ascii_case(&right.value),
        },
        EngineKind::Mysql => left.value.eq_ignore_ascii_case(&right.value),
        _ => false,
    }
}

fn wildcard_has_modifiers(opts: &WildcardAdditionalOptions) -> bool {
    opts.opt_ilike.is_some()
        || opts.opt_exclude.is_some()
        || opts.opt_except.is_some()
        || opts.opt_replace.is_some()
        || opts.opt_rename.is_some()
}

fn validate_relation_qualifier(
    engine: EngineKind,
    qualifier_parts: &[Ident],
    resolved_schema: &str,
    explicit_schema: Option<&Ident>,
    table_ident: &Ident,
    alias_ident: Option<&Ident>,
) -> Result<(), String> {
    if let Some(alias) = alias_ident {
        if qualifier_parts.len() == 1 && ident_matches_ident(engine, &qualifier_parts[0], alias) {
            return Ok(());
        }
        return Err(format!(
            "Column qualifier does not match table alias \"{}\"",
            alias.value
        ));
    }

    match qualifier_parts {
        [rel] => {
            if ident_matches_ident(engine, rel, table_ident) {
                Ok(())
            } else {
                Err(format!(
                    "Column qualifier \"{}\" does not match table \"{}\"",
                    rel.value, table_ident.value
                ))
            }
        }
        [sch, rel] => {
            let schema_ok = if let Some(exp_sch) = explicit_schema {
                ident_matches_ident(engine, sch, exp_sch)
            } else {
                ident_matches_target(engine, sch, resolved_schema)
            };
            if schema_ok && ident_matches_ident(engine, rel, table_ident) {
                Ok(())
            } else {
                Err(format!(
                    "Column qualifier \"{}.{}\" does not match target table",
                    sch.value, rel.value
                ))
            }
        }
        _ => Err("Projection uses an unsupported multi-part column qualifier".into()),
    }
}

fn extract_projected_column(
    engine: EngineKind,
    expr: &Expr,
    resolved_schema: &str,
    explicit_schema: Option<&Ident>,
    table_ident: &Ident,
    alias_ident: Option<&Ident>,
) -> Result<Ident, String> {
    match expr {
        Expr::Identifier(ident) => Ok(ident.clone()),
        Expr::CompoundIdentifier(parts) if parts.len() >= 2 => {
            let (qualifiers, col_ident) = parts.split_at(parts.len() - 1);
            validate_relation_qualifier(
                engine,
                qualifiers,
                resolved_schema,
                explicit_schema,
                table_ident,
                alias_ident,
            )?;
            Ok(col_ident[0].clone())
        }
        Expr::Function(_) => Err("Query uses GROUP BY or aggregate/function expressions".into()),
        _ => Err("Projection contains computed expressions or literals".into()),
    }
}

/// Pure, deterministic AST analyzer that checks whether `sql` is a single-table
/// `SELECT` statement eligible for provenance verification.
pub fn analyze_single_table_select(
    engine: EngineKind,
    sql: &str,
    default_schema: Option<&str>,
) -> Result<SingleTableSelectCandidate, String> {
    if !matches!(engine, EngineKind::Postgres | EngineKind::Mysql) {
        return Err("Engine does not support query result editing".into());
    }

    let statements = Parser::parse_sql(dialect_for_provenance(engine), sql)
        .map_err(|_| "Query could not be parsed for single-table provenance".to_string())?;
    if statements.len() != 1 {
        return Err("Statement is not a single SELECT query".into());
    }

    let Statement::Query(query) = &statements[0] else {
        return Err("Statement is not a SELECT query".into());
    };

    if query.with.is_some() {
        return Err("Query uses a WITH clause (CTE)".into());
    }
    if !query.locks.is_empty() {
        return Err("Query uses a locking clause".into());
    }

    let SetExpr::Select(select) = query.body.as_ref() else {
        return Err("Query uses set operations (UNION, INTERSECT, or EXCEPT)".into());
    };

    if select.distinct.is_some() {
        return Err("Query uses DISTINCT".into());
    }
    if select.into.is_some() {
        return Err("Query uses SELECT INTO".into());
    }
    match &select.group_by {
        GroupByExpr::Expressions(exprs, _) if exprs.is_empty() => {}
        _ => return Err("Query uses GROUP BY or aggregate expressions".into()),
    }
    if select.having.is_some() {
        return Err("Query uses GROUP BY or HAVING".into());
    }
    if select.qualify.is_some() {
        return Err("Query uses QUALIFY".into());
    }
    if !select.named_window.is_empty()
        || !select.lateral_views.is_empty()
        || !select.cluster_by.is_empty()
        || !select.distribute_by.is_empty()
        || !select.sort_by.is_empty()
        || !select.connect_by.is_empty()
    {
        return Err("Query uses window, lateral, or grouping clauses".into());
    }

    if select.from.is_empty() {
        return Err("Query does not select from a table".into());
    }
    if select.from.len() > 1 || !select.from[0].joins.is_empty() {
        return Err("Query joins multiple tables".into());
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
    } = &select.from[0].relation
    else {
        return Err("Query does not select directly from a base table".into());
    };

    if args.is_some()
        || !with_hints.is_empty()
        || version.is_some()
        || *with_ordinality
        || !partitions.is_empty()
        || json_path.is_some()
        || sample.is_some()
        || !index_hints.is_empty()
    {
        return Err("Query uses table functions, hints, or partition clauses".into());
    }

    let alias_ident = if let Some(table_alias) = alias {
        if !table_alias.columns.is_empty() {
            return Err("Table alias renames columns".into());
        }
        Some(&table_alias.name)
    } else {
        None
    };

    let mut name_parts = Vec::with_capacity(name.0.len());
    for part in &name.0 {
        let Some(ident) = part.as_ident() else {
            return Err("Table name is not a simple identifier".into());
        };
        name_parts.push(ident);
    }

    let (explicit_schema, table_ident, resolved_schema) = match name_parts.as_slice() {
        [tbl] => {
            let schema = default_schema
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| (engine == EngineKind::Postgres).then(|| "public".to_owned()))
                .ok_or_else(|| {
                    "Unqualified table requires an active schema or database".to_string()
                })?;
            (None, *tbl, schema)
        }
        [sch, tbl] => (Some(*sch), *tbl, normalize_lookup_ident(engine, sch)),
        _ => return Err("Cross-catalog table references are read-only".into()),
    };

    let resolved_table = TableRef {
        schema: resolved_schema.clone(),
        name: normalize_lookup_ident(engine, table_ident),
    };

    if select.projection.is_empty() {
        return Err("Projection has no columns".into());
    }

    let mut projection = Vec::with_capacity(select.projection.len());
    for item in &select.projection {
        match item {
            SelectItem::Wildcard(opts) => {
                if wildcard_has_modifiers(opts) {
                    return Err("Wildcard projection modifies or excludes columns".into());
                }
                projection.push(ProjectedSelectItem::Wildcard);
            }
            SelectItem::QualifiedWildcard(kind, opts) => {
                if wildcard_has_modifiers(opts) {
                    return Err("Wildcard projection modifies or excludes columns".into());
                }
                let SelectItemQualifiedWildcardKind::ObjectName(qualifier_name) = kind else {
                    return Err("Projection uses an unsupported wildcard qualifier".into());
                };
                let mut q_parts = Vec::with_capacity(qualifier_name.0.len());
                for part in &qualifier_name.0 {
                    let Some(ident) = part.as_ident() else {
                        return Err("Projection uses an unsupported wildcard qualifier".into());
                    };
                    q_parts.push(ident.clone());
                }
                validate_relation_qualifier(
                    engine,
                    &q_parts,
                    &resolved_schema,
                    explicit_schema,
                    table_ident,
                    alias_ident,
                )?;
                projection.push(ProjectedSelectItem::Wildcard);
            }
            SelectItem::UnnamedExpr(expr) => {
                let col = extract_projected_column(
                    engine,
                    expr,
                    &resolved_schema,
                    explicit_schema,
                    table_ident,
                    alias_ident,
                )?;
                projection.push(ProjectedSelectItem::Column(col));
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                let col = extract_projected_column(
                    engine,
                    expr,
                    &resolved_schema,
                    explicit_schema,
                    table_ident,
                    alias_ident,
                )?;
                if !ident_matches_ident(engine, &col, alias) {
                    return Err(format!(
                        "Projection renames column \"{}\" to \"{}\"",
                        col.value, alias.value
                    ));
                }
                projection.push(ProjectedSelectItem::Column(col));
            }
            SelectItem::ExprWithAliases { .. } => {
                return Err("Projection uses multiple aliases".into());
            }
        }
    }

    Ok(SingleTableSelectCandidate {
        engine,
        table: resolved_table,
        projection,
    })
}

/// Verifies a `SingleTableSelectCandidate` against the live `TableSchema` and
/// driver-reported result columns, producing a projected `TableSchema` and
/// enriched `Vec<ColumnInfo>` when proven editable.
pub fn verify_result_provenance(
    candidate: &SingleTableSelectCandidate,
    live_schema: &TableSchema,
    result_columns: &[ColumnInfo],
    safe_mode: SafeMode,
) -> ProvenanceResolution {
    if safe_mode == SafeMode::ReadOnly {
        return ProvenanceResolution::ReadOnly {
            reason: "Connection is in read-only mode".into(),
        };
    }

    if live_schema.primary_key.is_empty() {
        return ProvenanceResolution::ReadOnly {
            reason: format!(
                "Table \"{}.{}\" has no primary key",
                live_schema.table.schema, live_schema.table.name
            ),
        };
    }

    if !live_schema.editable {
        return ProvenanceResolution::ReadOnly {
            reason: format!(
                "Table \"{}.{}\" is read-only",
                live_schema.table.schema, live_schema.table.name
            ),
        };
    }

    let mut expanded_live_cols: Vec<&ColumnInfo> = Vec::with_capacity(result_columns.len());
    for item in &candidate.projection {
        match item {
            ProjectedSelectItem::Wildcard => {
                expanded_live_cols.extend(live_schema.columns.iter());
            }
            ProjectedSelectItem::Column(ident) => {
                let mut matches = live_schema
                    .columns
                    .iter()
                    .filter(|col| ident_matches_target(candidate.engine, ident, &col.name));
                let Some(found) = matches.next() else {
                    return ProvenanceResolution::ReadOnly {
                        reason: format!(
                            "Column \"{}\" was not found in table \"{}.{}\"",
                            ident.value, live_schema.table.schema, live_schema.table.name
                        ),
                    };
                };
                if matches.next().is_some() {
                    return ProvenanceResolution::ReadOnly {
                        reason: format!("Column reference \"{}\" is ambiguous", ident.value),
                    };
                }
                expanded_live_cols.push(found);
            }
        }
    }

    if expanded_live_cols.len() != result_columns.len() {
        return ProvenanceResolution::ReadOnly {
            reason: "Result column count does not match table schema".into(),
        };
    }

    let mut seen_names = std::collections::HashSet::with_capacity(expanded_live_cols.len());
    let mut enriched_columns = Vec::with_capacity(expanded_live_cols.len());

    for (index, (live_col, reported_col)) in expanded_live_cols
        .iter()
        .zip(result_columns.iter())
        .enumerate()
    {
        let same_name = if candidate.engine == EngineKind::Mysql {
            reported_col.name.eq_ignore_ascii_case(&live_col.name)
        } else {
            reported_col.name == live_col.name
        };
        if !same_name {
            return ProvenanceResolution::ReadOnly {
                reason: format!(
                    "Result column \"{}\" does not match table column \"{}\"",
                    reported_col.name, live_col.name
                ),
            };
        }

        let dedup_key = if candidate.engine == EngineKind::Mysql {
            live_col.name.to_ascii_lowercase()
        } else {
            live_col.name.clone()
        };
        if !seen_names.insert(dedup_key) {
            return ProvenanceResolution::ReadOnly {
                reason: format!("Projection repeats column \"{}\"", live_col.name),
            };
        }

        let mut col = (*live_col).clone();
        col.id = format!("c{index}");
        enriched_columns.push(col);
    }

    for pk in &live_schema.primary_key {
        let has_pk = enriched_columns.iter().any(|col| {
            col.primary_key
                && if candidate.engine == EngineKind::Mysql {
                    col.name.eq_ignore_ascii_case(pk)
                } else {
                    &col.name == pk
                }
        });
        if !has_pk {
            return ProvenanceResolution::ReadOnly {
                reason: format!("Projection omits primary key column \"{pk}\""),
            };
        }
    }

    if !enriched_columns.iter().any(|col| col.editable) {
        return ProvenanceResolution::ReadOnly {
            reason: "Projection contains no editable columns".into(),
        };
    }

    let omits_required_insert_column = live_schema.columns.iter().any(|table_col| {
        let is_projected = enriched_columns.iter().any(|proj_col| {
            if candidate.engine == EngineKind::Mysql {
                proj_col.name.eq_ignore_ascii_case(&table_col.name)
            } else {
                proj_col.name == table_col.name
            }
        });
        !is_projected && !table_col.nullable && !table_col.has_default
    });
    if omits_required_insert_column {
        for col in &mut enriched_columns {
            col.insertable = false;
        }
    }

    let projected_schema = TableSchema {
        table: live_schema.table.clone(),
        columns: enriched_columns.clone(),
        primary_key: live_schema.primary_key.clone(),
        foreign_keys: live_schema
            .foreign_keys
            .iter()
            .filter(|fk| {
                fk.columns
                    .iter()
                    .all(|fk_col| enriched_columns.iter().any(|c| &c.name == fk_col))
            })
            .cloned()
            .collect(),
        inbound_foreign_keys: live_schema.inbound_foreign_keys.clone(),
        editable: true,
    };

    let editability = ResultEditability::Editable {
        table: live_schema.table.clone(),
        primary_key: live_schema.primary_key.clone(),
    };

    ProvenanceResolution::Editable {
        schema: Box::new(projected_schema),
        columns: enriched_columns,
        editability,
    }
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

    fn sample_orders_schema() -> TableSchema {
        TableSchema {
            table: TableRef {
                schema: "public".into(),
                name: "orders".into(),
            },
            columns: vec![
                ColumnInfo {
                    id: "c0".into(),
                    name: "id".into(),
                    database_type: "bigint".into(),
                    kind: ColumnKind::Integer,
                    nullable: false,
                    primary_key: true,
                    foreign_key: false,
                    editable: false,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c1".into(),
                    name: "customer_id".into(),
                    database_type: "bigint".into(),
                    kind: ColumnKind::Integer,
                    nullable: false,
                    primary_key: false,
                    foreign_key: true,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c2".into(),
                    name: "status".into(),
                    database_type: "text".into(),
                    kind: ColumnKind::Text,
                    nullable: false,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c3".into(),
                    name: "notes".into(),
                    database_type: "text".into(),
                    kind: ColumnKind::Text,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
            ],
            primary_key: vec!["id".into()],
            foreign_keys: Vec::new(),
            inbound_foreign_keys: Vec::new(),
            editable: true,
        }
    }

    fn reported_cols(names: &[&str]) -> Vec<ColumnInfo> {
        names
            .iter()
            .enumerate()
            .map(|(i, &name)| ColumnInfo {
                id: format!("c{i}"),
                name: name.into(),
                database_type: "text".into(),
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

    #[test]
    fn single_table_select_provenance_accepts_wildcard_subset_and_alias() {
        let live = sample_orders_schema();

        let cand = analyze_single_table_select(
            EngineKind::Postgres,
            "SELECT * FROM orders WHERE status = 'pending' ORDER BY id DESC LIMIT 25",
            Some("public"),
        )
        .expect("wildcard select should be eligible");
        assert_eq!(
            cand.table,
            TableRef {
                schema: "public".into(),
                name: "orders".into()
            }
        );
        let res = verify_result_provenance(
            &cand,
            &live,
            &reported_cols(&["id", "customer_id", "status", "notes"]),
            SafeMode::Silent,
        );
        match res {
            ProvenanceResolution::Editable {
                schema,
                columns,
                editability,
            } => {
                assert_eq!(
                    editability,
                    ResultEditability::Editable {
                        table: live.table.clone(),
                        primary_key: vec!["id".into()],
                    }
                );
                assert_eq!(columns.len(), 4);
                assert!(columns[0].primary_key && !columns[0].editable && columns[0].insertable);
                assert!(schema.editable);
            }
            other => panic!("expected Editable, got {other:?}"),
        }

        let cand_subset = analyze_single_table_select(
            EngineKind::Postgres,
            "SELECT o.id, o.status AS status FROM public.orders AS o WHERE o.id = 1",
            Some("public"),
        )
        .expect("subset select with table alias should be eligible");
        let res_subset = verify_result_provenance(
            &cand_subset,
            &live,
            &reported_cols(&["id", "status"]),
            SafeMode::Confirm,
        );
        match res_subset {
            ProvenanceResolution::Editable {
                schema,
                columns,
                editability,
            } => {
                assert_eq!(
                    editability,
                    ResultEditability::Editable {
                        table: live.table.clone(),
                        primary_key: vec!["id".into()],
                    }
                );
                assert_eq!(
                    columns.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
                    vec!["c0", "c1"]
                );
                assert_eq!(
                    columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
                    vec!["id", "status"]
                );
                assert!(!columns[0].editable && columns[1].editable);
                assert!(columns.iter().all(|c| !c.insertable));
                assert_eq!(schema.columns.len(), 2);
            }
            other => panic!("expected Editable for subset, got {other:?}"),
        }
    }

    #[test]
    fn single_table_select_provenance_rejects_ineligible_ast_and_missing_pk() {
        let live = sample_orders_schema();

        for (sql, expected_fragment) in [
            (
                "SELECT o.id, c.name FROM orders o JOIN customers c ON c.id = o.customer_id",
                "joins multiple tables",
            ),
            (
                "WITH recent AS (SELECT * FROM orders) SELECT * FROM recent",
                "WITH clause",
            ),
            ("SELECT DISTINCT id, status FROM orders", "DISTINCT"),
            (
                "SELECT status, count(*) FROM orders GROUP BY status",
                "GROUP BY",
            ),
            (
                "SELECT id, upper(status) FROM orders",
                "aggregate/function expressions",
            ),
            (
                "SELECT id, status AS order_status FROM orders",
                "renames column",
            ),
            (
                "SELECT id FROM orders UNION SELECT id FROM orders",
                "set operations",
            ),
        ] {
            let err = analyze_single_table_select(EngineKind::Postgres, sql, Some("public"))
                .expect_err(sql);
            assert!(
                err.contains(expected_fragment),
                "sql={sql} expected '{expected_fragment}' in '{err}'"
            );
        }

        let no_pk_cand = analyze_single_table_select(
            EngineKind::Postgres,
            "SELECT status, notes FROM public.orders WHERE id = 1",
            Some("public"),
        )
        .unwrap();
        assert_eq!(
            verify_result_provenance(
                &no_pk_cand,
                &live,
                &reported_cols(&["status", "notes"]),
                SafeMode::Silent,
            ),
            ProvenanceResolution::ReadOnly {
                reason: "Projection omits primary key column \"id\"".into(),
            }
        );

        let dup_cand = analyze_single_table_select(
            EngineKind::Postgres,
            "SELECT id, status, status FROM public.orders",
            Some("public"),
        )
        .unwrap();
        assert_eq!(
            verify_result_provenance(
                &dup_cand,
                &live,
                &reported_cols(&["id", "status", "status"]),
                SafeMode::Silent,
            ),
            ProvenanceResolution::ReadOnly {
                reason: "Projection repeats column \"status\"".into(),
            }
        );
    }
}
