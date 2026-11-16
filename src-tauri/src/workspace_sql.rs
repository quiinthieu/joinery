//! Sandboxed read-only in-memory SQLite engine for querying across retained result sets.
//!
//! Allows joining, filtering, and aggregating snapshots across PostgreSQL, MySQL/MariaDB,
//! and Snowflake sessions without mutating remote databases or touching external files.

use crate::domain::*;
use crate::results::ResultStore;
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::{ToSqlOutput, Value as SqlValue, ValueRef},
};
use sqlparser::{
    dialect::SQLiteDialect,
    tokenizer::{Token, Tokenizer},
};
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const MAX_WORKSPACE_MOUNTS: usize = 16;
pub const MAX_MOUNT_RESULT_IDS: usize = 200;
pub const MAX_TOTAL_MOUNTED_ROWS: usize = 100_000;
pub const MAX_TOTAL_MOUNTED_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_WORKSPACE_SQL_BYTES: usize = 128 * 1024;
pub const MAX_WORKSPACE_OUTPUT_ROWS: usize = 50_000;
pub const MAX_WORKSPACE_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const WORKSPACE_QUERY_TIMEOUT: Duration = Duration::from_secs(15);

pub struct MountedSnapshot {
    pub alias: String,
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<TextRow>,
}

pub struct WorkspaceQueryOutput {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<TextRow>,
    pub mounted_rows: usize,
    pub truncated: bool,
    pub elapsed_ms: u32,
}

pub fn valid_mount_alias(alias: &str) -> bool {
    let bytes = alias.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return false;
    }
    if !bytes[1..]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        return false;
    }
    !alias.to_ascii_lowercase().starts_with("sqlite_")
}

fn quote_sqlite_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn deduplicate_column_names(columns: &[ColumnInfo]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::with_capacity(columns.len());
    let mut names = Vec::with_capacity(columns.len());
    for (idx, col) in columns.iter().enumerate() {
        let base = {
            let trimmed = col.name.trim();
            if trimmed.is_empty() {
                format!("column_{}", idx + 1)
            } else {
                trimmed.replace('\0', "")
            }
        };
        let mut candidate = base.clone();
        let mut suffix = 2usize;
        while !seen.insert(candidate.to_ascii_lowercase()) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        names.push(candidate);
    }
    names
}

fn sqlite_column_affinity(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Integer | ColumnKind::Boolean => "INTEGER",
        ColumnKind::Decimal => "REAL",
        ColumnKind::Text
        | ColumnKind::Temporal
        | ColumnKind::Json
        | ColumnKind::Binary
        | ColumnKind::Unknown => "TEXT",
    }
}

fn cell_to_sqlite_value(cell: &Option<String>, kind: ColumnKind) -> SqlValue {
    let Some(text) = cell else {
        return SqlValue::Null;
    };
    match kind {
        ColumnKind::Integer => text
            .trim()
            .parse::<i64>()
            .map(SqlValue::Integer)
            .unwrap_or_else(|_| SqlValue::Text(text.clone())),
        ColumnKind::Decimal => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(SqlValue::Real)
            .unwrap_or_else(|| SqlValue::Text(text.clone())),
        ColumnKind::Boolean => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "t" | "1" => SqlValue::Integer(1),
            "false" | "f" | "0" => SqlValue::Integer(0),
            _ => SqlValue::Text(text.clone()),
        },
        _ => SqlValue::Text(text.clone()),
    }
}

fn ensure_single_select_like_statement(raw_sql: &str) -> AppResult<&str> {
    if raw_sql.contains('\0') {
        return Err(AppError::invalid("Workspace SQL cannot contain NUL bytes"));
    }
    if raw_sql.len() > MAX_WORKSPACE_SQL_BYTES {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            "Workspace SQL is limited to 128 KiB",
        ));
    }
    let trimmed = raw_sql.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid("Enter a SELECT or WITH query to run"));
    }
    let tokens = Tokenizer::new(&SQLiteDialect {}, trimmed)
        .tokenize()
        .map_err(|_| {
            AppError::invalid("Workspace SQL contains an unterminated string or comment")
        })?;
    let mut statements = 0usize;
    let mut current_has_tokens = false;
    let mut first_keyword: Option<String> = None;
    for token in tokens {
        match token {
            Token::Whitespace(_) | Token::EOF => {}
            Token::SemiColon => {
                if current_has_tokens {
                    statements += 1;
                    current_has_tokens = false;
                }
            }
            other => {
                if statements >= 1 {
                    return Err(AppError::invalid(
                        "Workspace SQL runs one read-only SELECT or WITH statement at a time",
                    ));
                }
                if first_keyword.is_none() {
                    if let Token::Word(w) = &other {
                        first_keyword = Some(w.value.to_ascii_uppercase());
                    }
                }
                current_has_tokens = true;
            }
        }
    }
    if current_has_tokens {
        statements += 1;
    }
    if statements == 0 {
        return Err(AppError::invalid("Enter a SELECT or WITH query to run"));
    }
    match first_keyword.as_deref() {
        Some("SELECT" | "WITH" | "VALUES" | "EXPLAIN") => Ok(trimmed),
        _ => Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "Workspace SQL permits read-only SELECT, WITH, VALUES, or EXPLAIN queries only",
        )),
    }
}

fn workspace_authorizer(ctx: AuthContext<'_>) -> Authorization {
    match ctx.action {
        AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => {
            Authorization::Allow
        }
        AuthAction::Function { function_name } => {
            let lower = function_name.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "load_extension" | "fts3_tokenizer" | "readfile" | "writefile" | "edit"
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }
        _ => Authorization::Deny,
    }
}

pub fn materialize_mounts(
    results: &ResultStore,
    mounts: &[WorkspaceResultMount],
    mut is_forbidden_result: impl FnMut(&str) -> bool,
) -> AppResult<Vec<MountedSnapshot>> {
    if mounts.is_empty() || mounts.len() > MAX_WORKSPACE_MOUNTS {
        return Err(AppError::invalid(format!(
            "Mount between 1 and {MAX_WORKSPACE_MOUNTS} result sets to query"
        )));
    }
    let mut seen_aliases = HashSet::with_capacity(mounts.len());
    let mut total_rows = 0usize;
    let mut total_bytes = 0usize;
    let mut out = Vec::with_capacity(mounts.len());

    for mount in mounts {
        let alias = mount.alias.trim();
        if !valid_mount_alias(alias) {
            return Err(AppError::invalid(format!(
                "Mount alias {alias:?} must match [A-Za-z_][A-Za-z0-9_]{{0,63}} and cannot start with sqlite_"
            )));
        }
        if !seen_aliases.insert(alias.to_ascii_lowercase()) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                format!("Duplicate mount alias {alias:?}"),
            ));
        }
        if mount.result_ids.is_empty() || mount.result_ids.len() > MAX_MOUNT_RESULT_IDS {
            return Err(AppError::invalid(format!(
                "Mount {alias:?} must reference between 1 and {MAX_MOUNT_RESULT_IDS} retained result pages"
            )));
        }
        for rid in &mount.result_ids {
            if is_forbidden_result(rid) {
                return Err(AppError::new(
                    ErrorCode::PolicyBlocked,
                    "Reviewed console-owned results cannot be mounted into Workspace SQL",
                ));
            }
        }
        let first = results.info(&mount.result_ids[0])?;
        if first.columns.is_empty() {
            return Err(AppError::invalid(format!(
                "Mounted result {alias:?} has no columns"
            )));
        }
        let columns = first.columns.clone();
        for rid in mount.result_ids.iter().skip(1) {
            let info = results.info(rid)?;
            if info.columns.len() != columns.len()
                || info
                    .columns
                    .iter()
                    .zip(columns.iter())
                    .any(|(a, b)| a.name != b.name)
            {
                return Err(AppError::invalid(format!(
                    "Result pages in mount {alias:?} do not share an identical column schema"
                )));
            }
        }

        let col_indices: Vec<usize> = (0..columns.len()).collect();
        let mut rows = Vec::new();
        for rid in &mount.result_ids {
            let batch_count = results.batch_count(rid)?;
            for batch_idx in 0..batch_count {
                if let Some(batch) = results.batch_rows(rid, batch_idx, &col_indices)? {
                    total_rows = total_rows.saturating_add(batch.len());
                    if total_rows > MAX_TOTAL_MOUNTED_ROWS {
                        return Err(AppError::new(
                            ErrorCode::ResultLimitReached,
                            format!(
                                "Mounted datasets exceed the {MAX_TOTAL_MOUNTED_ROWS}-row Workspace SQL limit"
                            ),
                        ));
                    }
                    let added_bytes: usize = batch
                        .iter()
                        .map(|r| {
                            24 + r
                                .iter()
                                .map(|c| 8 + c.as_ref().map_or(0, String::len))
                                .sum::<usize>()
                        })
                        .sum();
                    total_bytes = total_bytes.saturating_add(added_bytes);
                    if total_bytes > MAX_TOTAL_MOUNTED_BYTES {
                        return Err(AppError::new(
                            ErrorCode::ResultLimitReached,
                            "Mounted datasets exceed the 32 MiB Workspace SQL memory budget",
                        ));
                    }
                    rows.extend(batch);
                }
            }
        }
        out.push(MountedSnapshot {
            alias: alias.to_owned(),
            columns,
            rows,
        });
    }

    Ok(out)
}

pub fn execute_workspace_query(
    snapshots: Vec<MountedSnapshot>,
    raw_sql: &str,
) -> AppResult<WorkspaceQueryOutput> {
    execute_workspace_query_with_timeout(snapshots, raw_sql, WORKSPACE_QUERY_TIMEOUT)
}

pub fn execute_workspace_query_with_timeout(
    snapshots: Vec<MountedSnapshot>,
    raw_sql: &str,
    timeout: Duration,
) -> AppResult<WorkspaceQueryOutput> {
    let sql = ensure_single_select_like_statement(raw_sql)?;
    let started = Instant::now();

    let mut conn = Connection::open_in_memory().map_err(|e| {
        AppError::internal(format!("Could not open in-memory workspace engine: {e}"))
    })?;
    conn.set_limit(
        Limit::SQLITE_LIMIT_SQL_LENGTH,
        MAX_WORKSPACE_SQL_BYTES as i32,
    )
    .map_err(|_| AppError::internal("Could not configure workspace SQL length limit"))?;
    conn.set_limit(Limit::SQLITE_LIMIT_EXPR_DEPTH, 200)
        .map_err(|_| AppError::internal("Could not configure workspace expression depth"))?;
    conn.set_limit(Limit::SQLITE_LIMIT_COMPOUND_SELECT, 64)
        .map_err(|_| AppError::internal("Could not configure workspace compound-select limit"))?;
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)
        .map_err(|_| AppError::internal("Could not configure workspace attach limit"))?;
    conn.execute_batch("PRAGMA temp_store=MEMORY;")
        .map_err(|_| AppError::internal("Could not configure workspace memory temp store"))?;

    let mut mounted_rows = 0usize;
    {
        let tx = conn
            .transaction()
            .map_err(|_| AppError::internal("Could not stage mounted result tables"))?;
        for snap in &snapshots {
            let col_names = deduplicate_column_names(&snap.columns);
            let col_defs: Vec<String> = col_names
                .iter()
                .zip(snap.columns.iter())
                .map(|(name, col)| {
                    format!(
                        "{} {}",
                        quote_sqlite_ident(name),
                        sqlite_column_affinity(col.kind)
                    )
                })
                .collect();
            let create_sql = format!(
                "CREATE TABLE {} ({})",
                quote_sqlite_ident(&snap.alias),
                col_defs.join(", ")
            );
            tx.execute(&create_sql, []).map_err(|e| {
                AppError::new(
                    ErrorCode::Database,
                    format!("Could not create mounted table {:?}: {e}", snap.alias),
                )
            })?;
            if !snap.rows.is_empty() {
                let placeholders = std::iter::repeat_n("?", col_names.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let insert_sql = format!(
                    "INSERT INTO {} VALUES ({placeholders})",
                    quote_sqlite_ident(&snap.alias)
                );
                let mut insert_stmt = tx.prepare(&insert_sql).map_err(|e| {
                    AppError::internal(format!("Could not prepare staging insert: {e}"))
                })?;
                for row in &snap.rows {
                    let values: Vec<ToSqlOutput<'_>> = row
                        .iter()
                        .zip(snap.columns.iter())
                        .map(|(cell, col)| ToSqlOutput::Owned(cell_to_sqlite_value(cell, col.kind)))
                        .collect();
                    insert_stmt
                        .execute(rusqlite::params_from_iter(values))
                        .map_err(|e| {
                            AppError::new(
                                ErrorCode::Database,
                                format!("Could not populate mounted table {:?}: {e}", snap.alias),
                            )
                        })?;
                    mounted_rows += 1;
                }
            }
        }
        tx.commit()
            .map_err(|_| AppError::internal("Could not commit staged workspace tables"))?;
    }

    conn.execute_batch("PRAGMA query_only=ON;")
        .map_err(|_| AppError::internal("Could not lock workspace engine to read-only"))?;
    conn.authorizer(Some(workspace_authorizer))
        .map_err(|_| AppError::internal("Could not install workspace SQL authorizer"))?;
    let deadline_start = Instant::now();
    conn.progress_handler(1024, Some(move || deadline_start.elapsed() > timeout))
        .map_err(|_| AppError::internal("Could not install workspace query timeout"))?;

    let map_sqlite_err = |err: rusqlite::Error| -> AppError {
        match err {
            rusqlite::Error::SqliteFailure(inner, msg) => {
                if inner.code == rusqlite::ErrorCode::OperationInterrupted {
                    return AppError::new(
                        ErrorCode::Cancelled,
                        "Workspace SQL query timed out after 15 seconds. Add a narrower filter or LIMIT.",
                    );
                }
                if inner.code == rusqlite::ErrorCode::AuthorizationForStatementDenied {
                    return AppError::new(
                        ErrorCode::PolicyBlocked,
                        "Workspace SQL permits read-only SELECT queries only (DDL, DML, PRAGMA, and file operations are blocked)",
                    );
                }
                AppError::new(
                    ErrorCode::Database,
                    msg.unwrap_or_else(|| inner.to_string()),
                )
            }
            other => AppError::new(ErrorCode::Database, other.to_string()),
        }
    };

    let mut stmt = conn.prepare(sql).map_err(map_sqlite_err)?;
    let col_count = stmt.column_count();
    if col_count == 0 {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "Workspace SQL query must return at least one result column",
        ));
    }
    let raw_col_names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();

    let mut out_rows: Vec<TextRow> = Vec::new();
    let mut inferred_kinds = vec![ColumnKind::Unknown; col_count];
    let mut out_bytes = 0usize;
    let mut truncated = false;

    let mut cursor = stmt.query([]).map_err(map_sqlite_err)?;
    while let Some(row) = cursor.next().map_err(map_sqlite_err)? {
        if out_rows.len() >= MAX_WORKSPACE_OUTPUT_ROWS {
            truncated = true;
            break;
        }
        let mut text_row = Vec::with_capacity(col_count);
        let mut row_bytes = 24usize;
        for idx in 0..col_count {
            let val = row.get_ref(idx).map_err(map_sqlite_err)?;
            let rendered = match val {
                ValueRef::Null => None,
                ValueRef::Integer(i) => {
                    if inferred_kinds[idx] == ColumnKind::Unknown {
                        inferred_kinds[idx] = ColumnKind::Integer;
                    }
                    Some(i.to_string())
                }
                ValueRef::Real(f) => {
                    inferred_kinds[idx] = ColumnKind::Decimal;
                    Some(f.to_string())
                }
                ValueRef::Text(bytes) => {
                    inferred_kinds[idx] = ColumnKind::Text;
                    Some(String::from_utf8_lossy(bytes).into_owned())
                }
                ValueRef::Blob(bytes) => {
                    inferred_kinds[idx] = ColumnKind::Binary;
                    Some(format!("0x{}", hex::encode(bytes)))
                }
            };
            row_bytes = row_bytes.saturating_add(8 + rendered.as_ref().map_or(0, String::len));
            text_row.push(rendered);
        }
        if out_bytes.saturating_add(row_bytes) > MAX_WORKSPACE_OUTPUT_BYTES {
            truncated = true;
            break;
        }
        out_bytes += row_bytes;
        out_rows.push(text_row);
    }
    drop(cursor);
    drop(stmt);
    let _ = conn.progress_handler(0, None::<fn() -> bool>);
    let _ = conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);

    let columns: Vec<ColumnInfo> = raw_col_names
        .into_iter()
        .enumerate()
        .map(|(idx, name)| {
            let kind = match inferred_kinds[idx] {
                ColumnKind::Unknown => ColumnKind::Text,
                other => other,
            };
            let db_type = match kind {
                ColumnKind::Integer => "INTEGER",
                ColumnKind::Decimal => "REAL",
                ColumnKind::Binary => "BLOB",
                _ => "TEXT",
            };
            ColumnInfo {
                id: format!("wsql_{idx}_{name}"),
                name,
                database_type: db_type.into(),
                kind,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: false,
            }
        })
        .collect();

    let elapsed_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    Ok(WorkspaceQueryOutput {
        columns,
        rows: out_rows,
        mounted_rows,
        truncated,
        elapsed_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, kind: ColumnKind) -> ColumnInfo {
        ColumnInfo {
            id: name.into(),
            name: name.into(),
            database_type: "text".into(),
            kind,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        }
    }

    #[test]
    fn joins_and_aggregates_cross_source_snapshots_with_typed_numeric_affinity() {
        let pg_users = MountedSnapshot {
            alias: "pg_users".into(),
            columns: vec![
                col("id", ColumnKind::Integer),
                col("email", ColumnKind::Text),
            ],
            rows: vec![
                vec![Some("1".into()), Some("ada@example.com".into())],
                vec![Some("2".into()), Some("grace@example.com".into())],
            ],
        };
        let sf_orders = MountedSnapshot {
            alias: "sf_orders".into(),
            columns: vec![
                col("user_id", ColumnKind::Integer),
                col("amount", ColumnKind::Decimal),
            ],
            rows: vec![
                vec![Some("1".into()), Some("19.5".into())],
                vec![Some("1".into()), Some("30.5".into())],
                vec![Some("2".into()), Some("12.0".into())],
            ],
        };

        let out = execute_workspace_query(
            vec![pg_users, sf_orders],
            "SELECT u.email, COUNT(*) AS order_count, SUM(o.amount) AS total
             FROM pg_users u
             JOIN sf_orders o ON u.id = o.user_id
             GROUP BY u.email
             ORDER BY total DESC;",
        )
        .unwrap();

        assert_eq!(out.mounted_rows, 5);
        assert!(!out.truncated);
        assert_eq!(out.columns.len(), 3);
        assert_eq!(out.rows.len(), 2);
        assert_eq!(
            out.rows[0],
            vec![
                Some("ada@example.com".into()),
                Some("2".into()),
                Some("50".into())
            ]
        );
        assert_eq!(
            out.rows[1],
            vec![
                Some("grace@example.com".into()),
                Some("1".into()),
                Some("12".into())
            ]
        );
    }

    #[test]
    fn blocks_ddl_dml_pragma_attach_multi_statement_and_dangerous_functions() {
        let snap = || {
            vec![MountedSnapshot {
                alias: "t".into(),
                columns: vec![col("id", ColumnKind::Integer)],
                rows: vec![vec![Some("1".into())]],
            }]
        };

        for forbidden in [
            "ATTACH DATABASE ':memory:' AS other",
            "PRAGMA database_list",
            "CREATE TABLE evil (x INT)",
            "DELETE FROM t",
            "INSERT INTO t VALUES (2)",
            "SELECT 1; SELECT 2",
            "SELECT load_extension('evil')",
        ] {
            let err = execute_workspace_query(snap(), forbidden)
                .err()
                .unwrap_or_else(|| panic!("expected {forbidden} to be rejected"));
            assert!(
                matches!(
                    err.code,
                    ErrorCode::PolicyBlocked | ErrorCode::InvalidInput | ErrorCode::Database
                ),
                "unexpected error for {forbidden}: {err:?}"
            );
        }
    }

    #[test]
    fn interrupts_runaway_recursive_queries_via_progress_handler() {
        let snap = vec![MountedSnapshot {
            alias: "seed".into(),
            columns: vec![col("n", ColumnKind::Integer)],
            rows: vec![vec![Some("1".into())]],
        }];
        let err = execute_workspace_query_with_timeout(
            snap,
            "WITH RECURSIVE inf(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM inf)
             SELECT MAX(x) FROM inf",
            Duration::from_millis(25),
        )
        .err()
        .expect("runaway recursive query must time out");
        assert_eq!(err.code, ErrorCode::Cancelled);
    }
}
