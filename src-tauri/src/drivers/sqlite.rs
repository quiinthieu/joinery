//! First-class local SQLite database driver (`rusqlite` bundled SQLite 3).
//!
//! Supports `:memory:` and local `.sqlite` / `.db` files with OS-level
//! `SQLITE_OPEN_READ_ONLY` + `PRAGMA query_only = ON` enforcement in `SafeMode::ReadOnly`.

use crate::data_search::{
    MAX_SEARCH_REPORT_BYTES, MAX_SEARCH_ROWS_PER_TABLE, MAX_SEARCH_TOTAL_ROW_HITS,
    build_table_match, escape_like_pattern, estimate_table_match_bytes, filter_candidate_tables,
    validate_search_request,
};
use crate::domain::*;
use crate::drivers::{
    BrowseRows, CompletionRelations, Driver, QuerySink, RowMutation, browse_query,
};
use crate::params::{BoundStatement, TypedBind};
use crate::results::ROW_LIMIT;
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{
    Connection, InterruptHandle, OpenFlags,
    config::DbConfig,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::{Value as SqliteValue, ValueRef},
};
use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const PAGE_SIZE: usize = 200;
const DDL_PREVIEW_LIMIT: usize = 256 * 1024;
type SqliteFkGroupMap = BTreeMap<i64, (String, Vec<(i64, String, String)>)>;

pub fn quote_sqlite_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quote_sqlite_table(table: &TableRef) -> String {
    let schema = if table.schema.trim().is_empty() {
        "main"
    } else {
        table.schema.trim()
    };
    format!(
        "{}.{}",
        quote_sqlite_ident(schema),
        quote_sqlite_ident(&table.name)
    )
}

fn classify_sqlite_type(raw_type: &str) -> ColumnKind {
    let upper = raw_type.trim().to_ascii_uppercase();
    if upper.is_empty() {
        return ColumnKind::Text;
    }
    if upper.contains("BOOL") {
        ColumnKind::Boolean
    } else if upper.contains("INT") {
        ColumnKind::Integer
    } else if upper.contains("REAL")
        || upper.contains("FLOA")
        || upper.contains("DOUB")
        || upper.contains("NUMERIC")
        || upper.contains("DECIMAL")
    {
        ColumnKind::Decimal
    } else if upper.contains("BLOB") || upper.contains("BINARY") {
        ColumnKind::Binary
    } else if upper.contains("JSON") {
        ColumnKind::Json
    } else if upper.contains("DATE") || upper.contains("TIME") {
        ColumnKind::Temporal
    } else {
        ColumnKind::Text
    }
}

fn format_value_ref(val: ValueRef<'_>, col_kind: ColumnKind) -> Option<String> {
    match val {
        ValueRef::Null => None,
        ValueRef::Integer(n) => {
            if col_kind == ColumnKind::Boolean {
                Some(if n != 0 {
                    "true".into()
                } else {
                    "false".into()
                })
            } else {
                Some(n.to_string())
            }
        }
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => {
            let mut hex = String::with_capacity(2 + bytes.len() * 2);
            hex.push_str("0x");
            for b in bytes {
                use std::fmt::Write;
                let _ = write!(&mut hex, "{b:02x}");
            }
            Some(hex)
        }
    }
}

fn decode_hex_blob(raw: &str) -> Option<Vec<u8>> {
    let trimmed = raw.trim();
    let hex_str = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .or_else(|| trimmed.strip_prefix("\\x"))
        .or_else(|| trimmed.strip_prefix("\\X"))?;
    if hex_str.len().is_multiple_of(2) && hex_str.bytes().all(|b| b.is_ascii_hexdigit()) {
        hex::decode(hex_str).ok()
    } else {
        None
    }
}

fn parse_sqlite_param(raw: &Option<String>, col_kind: ColumnKind) -> SqliteValue {
    let Some(text) = raw.as_deref() else {
        return SqliteValue::Null;
    };
    match col_kind {
        ColumnKind::Binary => decode_hex_blob(text)
            .map(SqliteValue::Blob)
            .unwrap_or_else(|| SqliteValue::Blob(text.as_bytes().to_vec())),
        ColumnKind::Boolean => {
            let lower = text.trim().to_ascii_lowercase();
            match lower.as_str() {
                "true" | "t" | "1" | "yes" => SqliteValue::Integer(1),
                "false" | "f" | "0" | "no" => SqliteValue::Integer(0),
                _ => SqliteValue::Text(text.to_owned()),
            }
        }
        ColumnKind::Integer => text
            .trim()
            .parse::<i64>()
            .map(SqliteValue::Integer)
            .unwrap_or_else(|_| SqliteValue::Text(text.to_owned())),
        ColumnKind::Decimal => {
            let trimmed = text.trim();
            if let Ok(int_val) = trimmed.parse::<i64>() {
                return SqliteValue::Integer(int_val);
            }
            let sig_digits = trimmed.bytes().filter(u8::is_ascii_digit).count();
            if sig_digits <= 15
                && let Ok(f) = trimmed.parse::<f64>()
                && f.is_finite()
                && f.to_string() == trimmed
            {
                return SqliteValue::Real(f);
            }
            SqliteValue::Text(text.to_owned())
        }
        _ => SqliteValue::Text(text.to_owned()),
    }
}

fn run_blocking_in_place<T, F: FnOnce() -> T>(f: F) -> T {
    if tokio::runtime::Handle::try_current()
        .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
    {
        tokio::task::block_in_place(f)
    } else {
        f()
    }
}

fn leading_sql_keyword(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i..i + 2) == Some(b"--") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"/*") {
            i += 2;
            while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        break;
    }
    let start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        i += 1;
    }
    sql[start..i].to_ascii_uppercase()
}

fn should_auto_begin_sqlite(sql: &str, kind: StatementKind) -> bool {
    if kind == StatementKind::Transaction {
        return false;
    }
    !matches!(
        leading_sql_keyword(sql).as_str(),
        "" | "BEGIN"
            | "COMMIT"
            | "END"
            | "ROLLBACK"
            | "SAVEPOINT"
            | "RELEASE"
            | "VACUUM"
            | "PRAGMA"
            | "ATTACH"
            | "DETACH"
            | "EXPLAIN"
    )
}

fn is_explain_query_plan_sql(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    trimmed
        .get(..18)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("EXPLAIN QUERY PLAN"))
}

fn typed_bind_to_sqlite(bind: &TypedBind) -> SqliteValue {
    let Some(raw) = bind.value.as_deref() else {
        return SqliteValue::Null;
    };
    match bind.kind {
        QueryParamKind::Boolean => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "t" | "yes" => SqliteValue::Integer(1),
            _ => SqliteValue::Integer(0),
        },
        QueryParamKind::Integer => raw
            .trim()
            .parse::<i64>()
            .map(SqliteValue::Integer)
            .unwrap_or_else(|_| SqliteValue::Text(raw.to_owned())),
        QueryParamKind::Decimal => raw
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(SqliteValue::Real)
            .unwrap_or_else(|| SqliteValue::Text(raw.to_owned())),
        QueryParamKind::Text
        | QueryParamKind::Uuid
        | QueryParamKind::Date
        | QueryParamKind::Timestamp
        | QueryParamKind::Json => SqliteValue::Text(raw.to_owned()),
    }
}

struct SqliteBrowseDialect;

impl browse_query::Dialect for SqliteBrowseDialect {
    fn engine(&self) -> EngineKind {
        EngineKind::Sqlite
    }

    fn column(&self, name: &str) -> String {
        quote_sqlite_ident(name)
    }

    fn typed_bind(&self, column: &ColumnInfo, _index: usize) -> Option<String> {
        match column.kind {
            ColumnKind::Binary => None,
            ColumnKind::Integer => Some("CAST(? AS INTEGER)".into()),
            ColumnKind::Decimal => Some("CAST(? AS REAL)".into()),
            ColumnKind::Boolean => {
                Some("CASE WHEN LOWER(?) IN ('1','true','t','yes') THEN 1 ELSE 0 END".into())
            }
            ColumnKind::Text | ColumnKind::Json | ColumnKind::Temporal | ColumnKind::Unknown => {
                Some("CAST(? AS TEXT)".into())
            }
        }
    }

    fn like(&self, column: &ColumnInfo, _index: usize) -> String {
        format!(
            "LOWER(CAST({} AS TEXT)) LIKE LOWER(?){}",
            quote_sqlite_ident(&column.name),
            browse_query::like_escape_clause()
        )
    }

    fn order_term(&self, expression: &str, descending: bool) -> String {
        if descending {
            format!("{expression} DESC NULLS FIRST")
        } else {
            format!("{expression} ASC NULLS LAST")
        }
    }
}

pub(crate) fn browse_sql(
    schema: &TableSchema,
    query: &BrowseQuery,
    after: Option<&[Option<String>]>,
) -> AppResult<(String, browse_query::Built)> {
    let built = browse_query::build(schema, query, after, &SqliteBrowseDialect)?;
    let cols = if schema.columns.is_empty() {
        "1".to_owned()
    } else {
        schema
            .columns
            .iter()
            .map(|c| quote_sqlite_ident(&c.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let sql = format!(
        "SELECT {cols} FROM {}{}{} LIMIT {}",
        quote_sqlite_table(&schema.table),
        built.where_sql,
        built.order_sql,
        PAGE_SIZE + 1
    );
    Ok((sql, built))
}

pub fn mutation_sql(
    schema: &TableSchema,
    mutation: &RowMutation,
) -> AppResult<(String, Vec<Option<String>>)> {
    let table_sql = quote_sqlite_table(&schema.table);
    let full_row_where =
        |original: &TextRow, binds: &mut Vec<Option<String>>| -> AppResult<String> {
            if schema.primary_key.is_empty() {
                return Err(AppError::invalid(
                    "SQLite grid edits require a table primary key",
                ));
            }
            if original.len() != schema.columns.len() {
                return Err(AppError::invalid(
                    "Original SQLite row length does not match table schema",
                ));
            }
            let mut parts = Vec::with_capacity(schema.columns.len());
            for (col, val) in schema.columns.iter().zip(original.iter()) {
                let col_sql = quote_sqlite_ident(&col.name);
                match val {
                    Some(v) => {
                        parts.push(format!("{col_sql} IS ?"));
                        binds.push(Some(v.clone()));
                    }
                    None => parts.push(format!("{col_sql} IS NULL")),
                }
            }
            Ok(parts.join(" AND "))
        };

    match mutation {
        RowMutation::Update { original, changes } => {
            if changes.is_empty() {
                return Err(AppError::invalid("No column changes supplied"));
            }
            let mut binds = Vec::new();
            let mut sets = Vec::with_capacity(changes.len());
            for (col_idx, new_val) in changes {
                let col = schema
                    .columns
                    .get(*col_idx)
                    .ok_or_else(|| AppError::invalid("Invalid column index"))?;
                let col_sql = quote_sqlite_ident(&col.name);
                match new_val {
                    Some(v) => {
                        sets.push(format!("{col_sql} = ?"));
                        binds.push(Some(v.clone()));
                    }
                    None => sets.push(format!("{col_sql} = NULL")),
                }
            }
            let where_sql = full_row_where(original, &mut binds)?;
            Ok((
                format!(
                    "UPDATE {table_sql} SET {} WHERE {where_sql};",
                    sets.join(", ")
                ),
                binds,
            ))
        }
        RowMutation::Delete { original } => {
            let mut binds = Vec::new();
            let where_sql = full_row_where(original, &mut binds)?;
            Ok((format!("DELETE FROM {table_sql} WHERE {where_sql};"), binds))
        }
        RowMutation::Insert { values } => {
            if values.is_empty() {
                return Ok((
                    format!("INSERT INTO {table_sql} DEFAULT VALUES;"),
                    Vec::new(),
                ));
            }
            let mut cols = Vec::with_capacity(values.len());
            let mut vals = Vec::with_capacity(values.len());
            let mut binds = Vec::new();
            for (col_idx, v) in values {
                let col = schema
                    .columns
                    .get(*col_idx)
                    .ok_or_else(|| AppError::invalid("Invalid column index"))?;
                cols.push(quote_sqlite_ident(&col.name));
                match v {
                    Some(s) => {
                        vals.push("?".to_owned());
                        binds.push(Some(s.clone()));
                    }
                    None => vals.push("NULL".to_owned()),
                }
            }
            Ok((
                format!(
                    "INSERT INTO {table_sql} ({}) VALUES ({});",
                    cols.join(", "),
                    vals.join(", ")
                ),
                binds,
            ))
        }
    }
}

pub struct Sqlite {
    conn: Arc<Mutex<Option<Connection>>>,
    interrupt: Arc<InterruptHandle>,
    closed: Arc<AtomicBool>,
    tx_mode: Arc<Mutex<TransactionMode>>,
    database_label: String,
    server_version: String,
    read_only: bool,
}

fn resolve_sqlite_candidate_path(raw: &Path, depth: usize) -> AppResult<PathBuf> {
    if depth > 16 {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "SQLite path symlink chain is too deep",
        ));
    }
    let abs = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| AppError::internal("Could not resolve current working directory"))?
            .join(raw)
    };

    let mut acc = PathBuf::new();
    let components: Vec<Component<'_>> = abs.components().collect();
    for (idx, comp) in components.iter().enumerate() {
        match comp {
            Component::Prefix(p) => acc.push(p.as_os_str()),
            Component::RootDir => acc.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                acc.pop();
                if acc.as_os_str().is_empty() {
                    acc.push(Path::new("/"));
                } else if let Ok(canon) = std::fs::canonicalize(&acc) {
                    acc = canon;
                }
            }
            Component::Normal(seg) => {
                acc.push(seg);
                if let Ok(canon) = std::fs::canonicalize(&acc) {
                    acc = canon;
                } else if let Ok(meta) = std::fs::symlink_metadata(&acc)
                    && meta.file_type().is_symlink()
                    && let Ok(link_target) = std::fs::read_link(&acc)
                {
                    let mut base = acc.clone();
                    base.pop();
                    let mut expanded = if link_target.is_absolute() {
                        link_target
                    } else {
                        base.join(link_target)
                    };
                    for rem in &components[idx + 1..] {
                        expanded.push(rem.as_os_str());
                    }
                    return resolve_sqlite_candidate_path(&expanded, depth + 1);
                }
            }
        }
    }
    Ok(acc)
}

fn is_forbidden_joinery_sqlite_path(resolved: &Path, forbidden_roots: &[PathBuf]) -> bool {
    if let Some(name) = resolved.file_name().and_then(|n| n.to_str()) {
        let lower = name.to_ascii_lowercase();
        if lower == "joinery.sqlite"
            || lower.starts_with("joinery.sqlite-")
            || lower.starts_with("joinery.sqlite.")
        {
            return true;
        }
    }
    for root in forbidden_roots {
        let canon_root = resolve_sqlite_candidate_path(root, 0).unwrap_or_else(|_| root.clone());
        if resolved.starts_with(&canon_root) {
            return true;
        }
    }
    let path_lower = resolved.to_string_lossy().to_ascii_lowercase();
    if path_lower.contains("/com.joinery.desktop/")
        || path_lower.ends_with("/com.joinery.desktop")
        || path_lower.contains("/application support/joinery/")
        || path_lower.ends_with("/application support/joinery")
        || path_lower.contains("/snowflake-state/")
        || path_lower.ends_with("/snowflake-state")
    {
        return true;
    }
    false
}

fn is_forbidden_sqlite_function(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "load_extension" | "fts3_tokenizer" | "readfile" | "writefile" | "edit"
    )
}

fn sqlite_readonly_authorizer(ctx: AuthContext<'_>) -> Authorization {
    match ctx.action {
        AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => {
            Authorization::Allow
        }
        AuthAction::Function { function_name } => {
            if is_forbidden_sqlite_function(function_name) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } => {
            let lower = pragma_name.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "writable_schema"
                    | "trusted_schema"
                    | "temp_store_directory"
                    | "data_store_directory"
                    | "wal_checkpoint"
                    | "optimize"
                    | "incremental_vacuum"
                    | "shrink_memory"
            ) {
                return Authorization::Deny;
            }
            if pragma_value.is_some()
                && matches!(
                    lower.as_str(),
                    "query_only"
                        | "journal_mode"
                        | "locking_mode"
                        | "synchronous"
                        | "auto_vacuum"
                        | "user_version"
                        | "application_id"
                        | "schema_version"
                        | "page_size"
                        | "max_page_count"
                        | "foreign_keys"
                )
            {
                return Authorization::Deny;
            }
            Authorization::Allow
        }
        _ => Authorization::Deny,
    }
}

fn make_sqlite_writable_authorizer(
    forbidden_roots: Vec<PathBuf>,
) -> impl FnMut(AuthContext<'_>) -> Authorization + Send + 'static {
    move |ctx: AuthContext<'_>| sqlite_writable_authorizer(ctx, &forbidden_roots)
}

fn sqlite_writable_authorizer(ctx: AuthContext<'_>, forbidden_roots: &[PathBuf]) -> Authorization {
    match ctx.action {
        AuthAction::Attach { filename } => {
            let trimmed = filename.trim();
            if trimmed.is_empty() || trimmed == ":memory:" {
                return Authorization::Allow;
            }
            if Sqlite::validate_target_path(trimmed, forbidden_roots).is_err() {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }
        AuthAction::Function { function_name } => {
            if is_forbidden_sqlite_function(function_name) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } => {
            let lower = pragma_name.to_ascii_lowercase();
            if matches!(lower.as_str(), "writable_schema")
                || (pragma_value.is_some()
                    && matches!(
                        lower.as_str(),
                        "trusted_schema" | "temp_store_directory" | "data_store_directory"
                    ))
            {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }
        _ => Authorization::Allow,
    }
}

impl Sqlite {
    pub fn validate_target_path(raw: &str, forbidden_roots: &[PathBuf]) -> AppResult<()> {
        let db = raw.trim();
        if db.is_empty() || db.len() > 4096 || db.contains('\0') {
            return Err(AppError::invalid(
                "Provide a SQLite database file path or :memory:",
            ));
        }
        if db == ":memory:" {
            return Ok(());
        }
        if db.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("file:")) {
            return Err(AppError::invalid(
                "Provide a plain filesystem path or :memory: instead of a SQLite file: URI",
            ));
        }
        let resolved = resolve_sqlite_candidate_path(Path::new(db), 0)?;
        if is_forbidden_joinery_sqlite_path(&resolved, forbidden_roots) {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Opening Joinery internal application metadata or state files as a user SQLite database is blocked",
            ));
        }
        Ok(())
    }

    pub fn validate_profile(profile: &ConnectionProfile) -> AppResult<()> {
        Self::validate_target_path(&profile.database, &[])?;
        if profile.ssh.is_some() {
            return Err(AppError::invalid(
                "SSH tunneling does not apply to local SQLite database files",
            ));
        }
        if profile.snowflake.is_some() {
            return Err(AppError::invalid(
                "Snowflake settings do not apply to SQLite connections",
            ));
        }
        Ok(())
    }

    pub async fn open(
        profile: &ConnectionProfile,
        session_id: &str,
        forbidden_roots: Vec<PathBuf>,
    ) -> AppResult<(Arc<dyn Driver>, SessionInfo)> {
        Self::validate_profile(profile)?;
        let db_target = profile.database.trim().to_owned();
        Self::validate_target_path(&db_target, &forbidden_roots)?;
        let read_only = crate::safety::effective_mode(profile) == SafeMode::ReadOnly;
        let sid = session_id.to_owned();
        let connection_id = profile.id.clone();

        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            Self::validate_target_path(&db_target, &forbidden_roots)?;
            let conn = if db_target == ":memory:" {
                Connection::open_in_memory().map_err(map_sqlite_err)?
            } else {
                let path = Path::new(&db_target);
                let flags = if read_only {
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
                } else {
                    if let Some(parent) = path.parent()
                        && !parent.as_os_str().is_empty()
                        && !parent.exists()
                    {
                        return Err(AppError::new(
                            ErrorCode::ConnectionFailed,
                            format!("Directory does not exist: {}", parent.display()),
                        ));
                    }
                    OpenFlags::SQLITE_OPEN_READ_WRITE
                        | OpenFlags::SQLITE_OPEN_CREATE
                        | OpenFlags::SQLITE_OPEN_NO_MUTEX
                };
                let opened = Connection::open_with_flags(path, flags).map_err(|err| {
                    AppError::new(
                        ErrorCode::ConnectionFailed,
                        format!("Could not open SQLite database: {err}"),
                    )
                })?;
                if let Ok(canon_after_open) = std::fs::canonicalize(path)
                    && is_forbidden_joinery_sqlite_path(&canon_after_open, &forbidden_roots)
                {
                    return Err(AppError::new(
                        ErrorCode::PolicyBlocked,
                        "Opening Joinery internal application metadata or state files as a user SQLite database is blocked",
                    ));
                }
                opened
            };

            let _ = conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true);
            conn.execute_batch(
                "PRAGMA busy_timeout = 5000; PRAGMA trusted_schema = OFF; PRAGMA foreign_keys = ON;",
            )
            .map_err(map_sqlite_err)?;

            let raw_ver: String = conn
                .query_row("SELECT sqlite_version()", [], |r| r.get(0))
                .map_err(map_sqlite_err)?;
            let server_version = format!("SQLite {raw_ver}");

            if read_only {
                let _ = conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0);
                conn.execute_batch("PRAGMA query_only = ON;")
                    .map_err(map_sqlite_err)?;
                conn.authorizer(Some(sqlite_readonly_authorizer))
                    .map_err(map_sqlite_err)?;
            } else {
                conn.authorizer(Some(make_sqlite_writable_authorizer(forbidden_roots)))
                    .map_err(map_sqlite_err)?;
            }

            let latency_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            let interrupt = Arc::new(conn.get_interrupt_handle());

            let label = if db_target == ":memory:" {
                ":memory:".to_owned()
            } else {
                Path::new(&db_target)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| db_target.clone())
            };

            let driver = Arc::new(Self {
                conn: Arc::new(Mutex::new(Some(conn))),
                interrupt,
                closed: Arc::new(AtomicBool::new(false)),
                tx_mode: Arc::new(Mutex::new(TransactionMode::AutoCommit)),
                database_label: label,
                server_version: server_version.clone(),
                read_only,
            });

            let info = SessionInfo {
                id: sid,
                connection_id,
                engine: EngineKind::Sqlite,
                server_version,
                latency_ms,
                encrypted: false,
                capabilities: EngineCapabilities {
                    sql: true,
                    transactions: !read_only,
                    explicit_transaction_controls: false,
                    cancellation: true,
                    table_browsing: true,
                    relation_browsing: false,
                    table_editing: !read_only,
                    table_filtering: true,
                    processes: false,
                    object_explorer: true,
                    schema_diagram: true,
                    schema_diff: true,
                    ddl_editing: !read_only,
                    snowflake_reviewed_ddl: false,
                    snowflake_grid_editing: false,
                    lock_inspection: false,
                    explain_analyze: false,
                    oauth_browser: false,
                    actual_plan: None,
                },
                console_mode: false,
            };

            Ok((driver as Arc<dyn Driver>, info))
        })
        .await
        .map_err(|e| AppError::internal(format!("SQLite worker join failed: {e}")))?
    }

    fn with_conn<T, F>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce(&Connection) -> AppResult<T>,
    {
        if self.closed.load(Ordering::Acquire) {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "SQLite connection is closed",
            ));
        }
        let guard = self.conn.lock();
        let conn = guard.as_ref().ok_or_else(|| {
            AppError::new(ErrorCode::ConnectionClosed, "SQLite connection is closed")
        })?;
        f(conn)
    }

    fn load_schema_sync(
        conn: &Connection,
        table: &TableRef,
        read_only: bool,
    ) -> AppResult<TableSchema> {
        let schema_name = if table.schema.trim().is_empty() {
            "main"
        } else {
            table.schema.trim()
        };
        if !schema_name.eq_ignore_ascii_case("main") {
            return Err(AppError::new(
                ErrorCode::NotFound,
                format!("SQLite schema {schema_name:?} does not exist (expected \"main\")"),
            ));
        }
        let obj_type: Option<String> = conn
            .query_row(
                "SELECT type FROM sqlite_schema WHERE type IN ('table', 'view') AND name = ?1",
                [&table.name],
                |row| row.get(0),
            )
            .ok();
        let Some(obj_type) = obj_type else {
            return Err(AppError::new(
                ErrorCode::NotFound,
                format!("SQLite relation {:?} was not found", table.name),
            ));
        };
        let is_table = obj_type == "table";

        // Foreign keys via PRAGMA foreign_key_list
        let fk_sql = format!(
            "PRAGMA main.foreign_key_list({})",
            quote_sqlite_ident(&table.name)
        );
        let mut fk_stmt = conn.prepare(&fk_sql).map_err(map_sqlite_err)?;
        let mut fk_groups: SqliteFkGroupMap = BTreeMap::new();
        let mut fk_cols_set = HashSet::new();
        let fk_rows = fk_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                ))
            })
            .map_err(map_sqlite_err)?;
        for item in fk_rows {
            let (id, seq, target_table, from_col, to_col) = item.map_err(map_sqlite_err)?;
            fk_cols_set.insert(from_col.clone());
            let entry = fk_groups
                .entry(id)
                .or_insert_with(|| (target_table, Vec::new()));
            entry.1.push((seq, from_col, to_col));
        }
        let mut foreign_keys = Vec::with_capacity(fk_groups.len());
        for (id, (target_table, mut cols)) in fk_groups {
            cols.sort_by_key(|(seq, _, _)| *seq);
            let from_cols: Vec<String> = cols.iter().map(|(_, f, _)| f.clone()).collect();
            let mut to_cols: Vec<String> = cols.into_iter().map(|(_, _, t)| t).collect();
            if to_cols.iter().any(String::is_empty) {
                let parent_pk_sql = format!(
                    "PRAGMA main.table_info({})",
                    quote_sqlite_ident(&target_table)
                );
                if let Ok(mut p_stmt) = conn.prepare(&parent_pk_sql)
                    && let Ok(rows) =
                        p_stmt.query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
                {
                    let mut pks: Vec<(i64, String)> =
                        rows.flatten().filter(|(pk, _)| *pk > 0).collect();
                    pks.sort_by_key(|(pk, _)| *pk);
                    if pks.len() == from_cols.len() {
                        to_cols = pks.into_iter().map(|(_, n)| n).collect();
                    }
                }
            }
            foreign_keys.push(ForeignKey {
                name: format!("{}_fk_{id}", table.name),
                columns: from_cols,
                target: TableRef {
                    schema: "main".into(),
                    name: target_table,
                },
                target_columns: to_cols,
                virtual_relationship: false,
            });
        }

        // Columns via PRAGMA table_xinfo
        let info_sql = format!(
            "PRAGMA main.table_xinfo({})",
            quote_sqlite_ident(&table.name)
        );
        let mut stmt = conn.prepare(&info_sql).map_err(map_sqlite_err)?;
        let raw_cols = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })
            .map_err(map_sqlite_err)?;

        let mut pk_pairs: Vec<(i64, String)> = Vec::new();
        let mut columns = Vec::new();
        for item in raw_cols {
            let (name, db_type, not_null, dflt, pk_ord, hidden) = item.map_err(map_sqlite_err)?;
            if hidden == 1 {
                continue;
            }
            let is_generated = hidden == 2 || hidden == 3;
            if pk_ord > 0 {
                pk_pairs.push((pk_ord, name.clone()));
            }
            let display_type = if db_type.trim().is_empty() {
                "ANY".to_owned()
            } else {
                db_type.clone()
            };
            let kind = classify_sqlite_type(&db_type);
            let can_write = is_table && !read_only && !is_generated && kind != ColumnKind::Binary;
            columns.push(ColumnInfo {
                id: name.clone(),
                name: name.clone(),
                database_type: display_type,
                kind,
                nullable: !not_null && pk_ord == 0,
                primary_key: pk_ord > 0,
                foreign_key: fk_cols_set.contains(&name),
                editable: can_write,
                insertable: can_write,
                has_default: dflt.is_some() || (pk_ord == 1 && kind == ColumnKind::Integer),
            });
        }
        pk_pairs.sort_by_key(|(ord, _)| *ord);
        let primary_key: Vec<String> = pk_pairs.into_iter().map(|(_, n)| n).collect();
        let table_editable = is_table && !read_only && !primary_key.is_empty();
        if !table_editable {
            for col in &mut columns {
                col.editable = false;
                col.insertable = false;
            }
        }

        let inbound = Self::referencing_keys_sync(conn, &table.name)?;

        Ok(TableSchema {
            table: TableRef {
                schema: "main".into(),
                name: table.name.clone(),
            },
            columns,
            primary_key,
            foreign_keys,
            inbound_foreign_keys: inbound,
            editable: table_editable,
        })
    }

    fn referencing_keys_sync(
        conn: &Connection,
        target_table_name: &str,
    ) -> AppResult<Vec<InboundForeignKey>> {
        let mut tbl_stmt = conn
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(map_sqlite_err)?;
        let table_names: Vec<String> = tbl_stmt
            .query_map([], |r| r.get(0))
            .map_err(map_sqlite_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite_err)?;

        let mut inbound = Vec::new();
        for src_table in table_names {
            let fk_sql = format!(
                "PRAGMA main.foreign_key_list({})",
                quote_sqlite_ident(&src_table)
            );
            let Ok(mut fk_stmt) = conn.prepare(&fk_sql) else {
                continue;
            };
            let mut groups: SqliteFkGroupMap = BTreeMap::new();
            if let Ok(rows) = fk_stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                ))
            }) {
                for (id, seq, ref_tbl, from_col, to_col) in rows.flatten() {
                    if ref_tbl.eq_ignore_ascii_case(target_table_name) {
                        let entry = groups.entry(id).or_insert_with(|| (ref_tbl, Vec::new()));
                        entry.1.push((seq, from_col, to_col));
                    }
                }
            }
            for (id, (_, mut cols)) in groups {
                cols.sort_by_key(|(seq, _, _)| *seq);
                inbound.push(InboundForeignKey {
                    name: format!("{src_table}_fk_{id}"),
                    source: TableRef {
                        schema: "main".into(),
                        name: src_table.clone(),
                    },
                    columns: cols.iter().map(|(_, f, _)| f.clone()).collect(),
                    target_columns: cols.into_iter().map(|(_, _, t)| t).collect(),
                    virtual_relationship: false,
                });
            }
        }
        Ok(inbound)
    }

    fn execute_sync(
        conn: &Connection,
        closed: &AtomicBool,
        sql: &str,
        params: &[SqliteValue],
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        let mut stmt = conn.prepare(sql).map_err(map_sqlite_err)?;
        let col_count = stmt.column_count();
        if col_count == 0 {
            let affected = stmt
                .execute(rusqlite::params_from_iter(params.iter()))
                .map_err(map_sqlite_err)? as u64;
            sink.describe(Vec::new())?;
            sink.complete(affected)?;
            return Ok(affected);
        }

        let col_names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
        if is_explain_query_plan_sql(sql)
            && col_count == 4
            && col_names
                .iter()
                .map(|s| s.to_ascii_lowercase())
                .eq(["id", "parent", "notused", "detail"])
        {
            let mut rows_iter = stmt
                .query(rusqlite::params_from_iter(params.iter()))
                .map_err(map_sqlite_err)?;
            let mut steps = Vec::new();
            while let Some(row) = rows_iter.next().map_err(map_sqlite_err)? {
                let id: i64 = row.get(0).unwrap_or(0);
                let parent: i64 = row.get(1).unwrap_or(0);
                let detail: String = row.get(3).unwrap_or_default();
                steps.push(serde_json::json!({
                    "id": id,
                    "parent": parent,
                    "detail": detail,
                }));
            }
            let payload = serde_json::json!({ "sqliteQueryPlan": steps }).to_string();
            sink.describe(vec![ColumnInfo {
                id: "query_plan".into(),
                name: "QUERY PLAN".into(),
                database_type: "JSON".into(),
                kind: ColumnKind::Json,
                nullable: false,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: false,
            }])?;
            let _ = sink.rows(vec![vec![Some(payload)]])?;
            sink.complete(1)?;
            return Ok(1);
        }

        let columns: Vec<ColumnInfo> = col_names
            .iter()
            .enumerate()
            .map(|(i, name)| ColumnInfo {
                id: format!("c{i}_{name}"),
                name: name.clone(),
                database_type: "ANY".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: false,
            })
            .collect();
        sink.describe(columns.clone())?;

        let mut rows_iter = stmt
            .query(rusqlite::params_from_iter(params.iter()))
            .map_err(map_sqlite_err)?;
        let mut batch = Vec::with_capacity(128);
        let mut seen = 0u64;

        while let Some(row) = rows_iter.next().map_err(map_sqlite_err)? {
            if closed.load(Ordering::Acquire) {
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "SQLite query cancelled",
                ));
            }
            if seen as usize >= ROW_LIMIT {
                sink.limit("Result preview limit reached");
                break;
            }
            let mut cells = Vec::with_capacity(col_count);
            for (i, col) in columns.iter().enumerate() {
                let v = row.get_ref(i).map_err(map_sqlite_err)?;
                cells.push(format_value_ref(v, col.kind));
            }
            batch.push(cells);
            seen += 1;
            if batch.len() >= 128 && !sink.rows(std::mem::take(&mut batch))? {
                sink.limit("Result retention budget reached");
                break;
            }
        }
        if !batch.is_empty() && !sink.rows(batch)? {
            sink.limit("Result retention budget reached");
        }
        sink.complete(seen)?;
        Ok(seen)
    }
}

fn map_sqlite_err(err: rusqlite::Error) -> AppError {
    match &err {
        rusqlite::Error::SqliteFailure(ffi_err, msg) => {
            if ffi_err.code == rusqlite::ErrorCode::OperationInterrupted {
                return AppError::new(ErrorCode::Cancelled, "SQLite query was cancelled");
            }
            if ffi_err.code == rusqlite::ErrorCode::ReadOnly {
                return AppError::new(
                    ErrorCode::PermissionDenied,
                    msg.as_deref()
                        .unwrap_or("SQLite database is opened in read-only mode"),
                );
            }
            if ffi_err.code == rusqlite::ErrorCode::AuthorizationForStatementDenied {
                return AppError::new(
                    ErrorCode::PolicyBlocked,
                    msg.as_deref()
                        .unwrap_or("SQLite operation blocked by security policy"),
                );
            }
            AppError::new(
                ErrorCode::Database,
                msg.as_deref().unwrap_or(&err.to_string()),
            )
        }
        _ => AppError::new(ErrorCode::Database, err.to_string()),
    }
}

fn parse_sqlite_index_ddl(
    idx_ddl: Option<&str>,
    pragma_cols: &[Option<String>],
    quote_plain_cols: bool,
) -> (Vec<String>, Option<String>) {
    let mut ddl_exprs: Vec<String> = Vec::new();
    let mut predicate: Option<String> = None;

    if let Some(ddl) = idx_ddl {
        let bytes = ddl.as_bytes();
        let mut i = 0usize;
        let mut depth = 0usize;
        let mut open_paren: Option<usize> = None;
        let mut close_paren: Option<usize> = None;
        let mut item_start = 0usize;

        while i < bytes.len() {
            match bytes[i] {
                b'\'' | b'"' | b'`' => {
                    let q = bytes[i];
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == q {
                            if bytes.get(i + 1) == Some(&q) {
                                i += 2;
                            } else {
                                i += 1;
                                break;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    continue;
                }
                b'[' => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != b']' {
                        i += 1;
                    }
                }
                b'(' => {
                    if depth == 0 && open_paren.is_none() {
                        open_paren = Some(i);
                        item_start = i + 1;
                    }
                    depth += 1;
                }
                b',' if depth == 1 && close_paren.is_none() => {
                    let seg = ddl[item_start..i].trim();
                    if !seg.is_empty() {
                        ddl_exprs.push(seg.to_owned());
                    }
                    item_start = i + 1;
                }
                b')' => {
                    if depth == 1 && close_paren.is_none() {
                        let seg = ddl[item_start..i].trim();
                        if !seg.is_empty() {
                            ddl_exprs.push(seg.to_owned());
                        }
                        close_paren = Some(i);
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            i += 1;
        }

        if let Some(end_idx) = close_paren {
            let tail = ddl[end_idx + 1..].trim().trim_end_matches(';').trim();
            if tail
                .get(..5)
                .is_some_and(|w| w.eq_ignore_ascii_case("WHERE"))
                && tail
                    .as_bytes()
                    .get(5)
                    .is_some_and(|b| b.is_ascii_whitespace() || *b == b'(')
            {
                let pred = tail[5..].trim();
                if !pred.is_empty() {
                    predicate = Some(pred.to_owned());
                }
            }
        }
    }

    let has_expr_cols = pragma_cols.iter().any(Option::is_none);
    let expressions = if has_expr_cols && !ddl_exprs.is_empty() {
        ddl_exprs
    } else if !pragma_cols.is_empty() {
        pragma_cols
            .iter()
            .enumerate()
            .map(|(i, col_opt)| match col_opt {
                Some(c) if quote_plain_cols => quote_sqlite_ident(c),
                Some(c) => c.clone(),
                None => ddl_exprs
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| "<expr>".to_owned()),
            })
            .collect()
    } else {
        ddl_exprs
    };

    (expressions, predicate)
}

fn extract_sqlite_generated_expr(table_ddl: Option<&str>, col_name: &str) -> Option<String> {
    let ddl = table_ddl?;
    let bytes = ddl.as_bytes();
    let mut open_paren = None;
    for (idx, b) in bytes.iter().enumerate() {
        if *b == b'(' {
            open_paren = Some(idx);
            break;
        }
    }
    let start = open_paren? + 1;
    // Split top-level column/constraint items inside CREATE TABLE (...)
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut item_start = start;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == q {
                        if bytes.get(i + 1) == Some(&q) {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                if depth == 0 {
                    items.push(ddl[item_start..i].trim());
                    break;
                }
                depth -= 1;
            }
            b',' if depth == 0 => {
                items.push(ddl[item_start..i].trim());
                item_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }

    for item in items {
        let trimmed = item.trim();
        // Match leading column identifier (quoted or unquoted)
        let (ident, rest) = if let Some(quote_ch) = trimmed
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '`' | '\''))
        {
            let q_bytes = trimmed.as_bytes();
            let mut pos = 1usize;
            let mut unescaped = String::new();
            while pos < q_bytes.len() {
                let ch = trimmed[pos..].chars().next().unwrap();
                pos += ch.len_utf8();
                if ch == quote_ch {
                    if trimmed[pos..].starts_with(quote_ch) {
                        unescaped.push(quote_ch);
                        pos += quote_ch.len_utf8();
                    } else {
                        break;
                    }
                } else {
                    unescaped.push(ch);
                }
            }
            (unescaped, &trimmed[pos..])
        } else {
            let end = trimmed
                .find(|c: char| c.is_ascii_whitespace() || c == '(')
                .unwrap_or(trimmed.len());
            (trimmed[..end].to_owned(), &trimmed[end..])
        };

        if !ident.eq_ignore_ascii_case(col_name) {
            continue;
        }
        // Look for `AS (...)` in `rest`
        let upper = rest.to_ascii_uppercase();
        let mut search_from = 0usize;
        while let Some(rel) = upper[search_from..].find("AS") {
            let pos = search_from + rel;
            let before_ok = pos == 0
                || rest.as_bytes()[pos - 1].is_ascii_whitespace()
                || rest.as_bytes()[pos - 1] == b')';
            let after_slice = rest[pos + 2..].trim_start();
            if before_ok && after_slice.starts_with('(') {
                let ab = after_slice.as_bytes();
                let mut d = 0usize;
                let mut k = 0usize;
                while k < ab.len() {
                    match ab[k] {
                        b'\'' | b'"' | b'`' => {
                            let q = ab[k];
                            k += 1;
                            while k < ab.len() {
                                if ab[k] == q {
                                    if ab.get(k + 1) == Some(&q) {
                                        k += 2;
                                    } else {
                                        k += 1;
                                        break;
                                    }
                                } else {
                                    k += 1;
                                }
                            }
                            continue;
                        }
                        b'(' => d += 1,
                        b')' => {
                            d -= 1;
                            if d == 0 {
                                return Some(after_slice[1..k].trim().to_owned());
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
            }
            search_from = pos + 2;
        }
    }
    None
}

fn load_sqlite_triggers_sync(conn: &Connection, table_name: &str) -> Vec<TriggerInfo> {
    let mut triggers = Vec::new();
    if let Ok(mut trg_stmt) = conn.prepare(
        "SELECT name, sql FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = ?1 ORDER BY name",
    ) && let Ok(trg_rows) = trg_stmt.query_map([table_name], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    }) {
        for (trg_name, trg_sql) in trg_rows.flatten() {
            let def = trg_sql.unwrap_or_default();
            let upper = def.to_ascii_uppercase();
            let timing = if upper.contains("BEFORE ") {
                "BEFORE"
            } else if upper.contains("INSTEAD OF ") {
                "INSTEAD OF"
            } else {
                "AFTER"
            };
            let mut events = Vec::new();
            for ev in ["INSERT", "UPDATE", "DELETE"] {
                if upper.contains(ev) {
                    events.push(ev.to_owned());
                }
            }
            triggers.push(TriggerInfo {
                name: trg_name,
                table_name: table_name.to_owned(),
                timing: timing.into(),
                events,
                orientation: "ROW".into(),
                enabled: true,
                definition: Some(def),
            });
        }
    }
    triggers
}

#[async_trait]
impl Driver for Sqlite {
    async fn execute(
        &self,
        sql: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        run_blocking_in_place(|| {
            let auto_begin = !self.read_only
                && *self.tx_mode.lock() == TransactionMode::Manual
                && should_auto_begin_sqlite(sql, kind);
            self.with_conn(|conn| {
                if auto_begin && conn.is_autocommit() {
                    conn.execute_batch("BEGIN DEFERRED TRANSACTION")
                        .map_err(map_sqlite_err)?;
                }
                Self::execute_sync(conn, &self.closed, sql, &[], sink)
            })
        })
    }

    async fn execute_bound(
        &self,
        sql: &str,
        kind: StatementKind,
        bound: &BoundStatement,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        let params: Vec<SqliteValue> = bound.binds.iter().map(typed_bind_to_sqlite).collect();
        run_blocking_in_place(|| {
            let auto_begin = !self.read_only
                && *self.tx_mode.lock() == TransactionMode::Manual
                && should_auto_begin_sqlite(sql, kind);
            self.with_conn(|conn| {
                if auto_begin && conn.is_autocommit() {
                    conn.execute_batch("BEGIN DEFERRED TRANSACTION")
                        .map_err(map_sqlite_err)?;
                }
                Self::execute_sync(conn, &self.closed, &bound.rewritten_sql, &params, sink)
            })
        })
    }

    async fn cancel(&self) -> AppResult<()> {
        self.interrupt.interrupt();
        Ok(())
    }

    async fn completion_identity(&self) -> AppResult<CompletionIdentity> {
        Ok(CompletionIdentity {
            database: self.database_label.clone(),
            role: "local".into(),
            search_path: vec!["main".into()],
            case_sensitive_table_names: Some(false),
            context_fingerprint: Some(format!("sqlite:{}:main", self.database_label)),
        })
    }

    async fn resolve_query_edit_table(
        &self,
        table: &TableRef,
        unqualified: bool,
    ) -> AppResult<TableRef> {
        let req_table = table.clone();
        let ro = self.read_only;
        self.with_conn(|conn| {
            if unqualified {
                let shadowed_by_temp: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM sqlite_temp_schema WHERE type IN ('table', 'view') AND LOWER(name) = LOWER(?1) LIMIT 1",
                        [&req_table.name],
                        |r| r.get(0),
                    )
                    .ok();
                if shadowed_by_temp.is_some() {
                    return Err(AppError::invalid(
                        "Query selects a temporary SQLite relation",
                    ));
                }
            } else if !req_table.schema.trim().eq_ignore_ascii_case("main") {
                return Err(AppError::new(
                    ErrorCode::NotFound,
                    format!("SQLite schema {:?} was not found", req_table.schema),
                ));
            }

            let canonical_name: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_schema WHERE type = 'table' AND LOWER(name) = LOWER(?1) LIMIT 1",
                    [&req_table.name],
                    |r| r.get(0),
                )
                .ok();
            let Some(canonical_name) = canonical_name else {
                return Err(AppError::new(
                    ErrorCode::NotFound,
                    format!("SQLite base table {:?} was not found", req_table.name),
                ));
            };
            let schema = Self::load_schema_sync(
                conn,
                &TableRef {
                    schema: "main".into(),
                    name: canonical_name,
                },
                ro,
            )?;
            Ok(schema.table)
        })
    }

    async fn completion_catalog(
        &self,
        _identity: &CompletionIdentity,
    ) -> AppResult<CompletionRelations> {
        let tables = self.catalog().await?;
        let unqualified_tables = tables.iter().map(|t| t.table.clone()).collect();
        Ok(CompletionRelations {
            tables,
            unqualified_tables,
        })
    }

    async fn completion_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.schema(table).await
    }

    async fn inspection_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.schema(table).await
    }

    async fn session_state(&self, session_id: &str) -> AppResult<SessionRuntimeState> {
        let in_tx = self.with_conn(|conn| Ok(!conn.is_autocommit()))?;
        let mode = *self.tx_mode.lock();
        Ok(SessionRuntimeState {
            session_id: session_id.to_owned(),
            transaction_mode: mode,
            transaction_state: if in_tx {
                TransactionState::InTransaction
            } else {
                TransactionState::Idle
            },
            context_kind: SessionContextKind::Schema,
            active_context: Some("main".into()),
            database: self.database_label.clone(),
            role: Some("local".into()),
            warehouse: None,
            search_path: vec!["main".into()],
            context_fingerprint: Some(format!("sqlite:{}:main", self.database_label)),
            transaction_observation: None,
            transaction_control_authority: None,
        })
    }

    async fn list_contexts(&self) -> AppResult<Vec<String>> {
        Ok(vec!["main".into()])
    }

    async fn set_transaction_mode(&self, mode: TransactionMode) -> AppResult<()> {
        if mode == TransactionMode::Unknown {
            return Err(AppError::invalid(
                "Unknown is not a requested transaction mode",
            ));
        }
        if self.read_only {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Transactions are disabled on read-only SQLite connections",
            ));
        }
        self.with_conn(|conn| {
            if mode == TransactionMode::AutoCommit && !conn.is_autocommit() {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Commit or roll back the open SQLite transaction before switching to auto-commit",
                ));
            }
            *self.tx_mode.lock() = mode;
            Ok(())
        })
    }

    async fn commit_transaction(&self) -> AppResult<()> {
        self.with_conn(|conn| {
            if conn.is_autocommit() {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "No transaction is currently open",
                ));
            }
            conn.execute_batch("COMMIT").map_err(map_sqlite_err)
        })
    }

    async fn rollback_transaction(&self) -> AppResult<()> {
        self.with_conn(|conn| {
            if conn.is_autocommit() {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "No transaction is currently open",
                ));
            }
            conn.execute_batch("ROLLBACK").map_err(map_sqlite_err)
        })
    }

    async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT name, type FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )
                .map_err(map_sqlite_err)?;
            let rows = stmt
                .query_map([], |row| {
                    let name: String = row.get(0)?;
                    let obj_type: String = row.get(1)?;
                    Ok(TableInfo {
                        table: TableRef {
                            schema: "main".into(),
                            name,
                        },
                        kind: if obj_type == "view" {
                            "view".into()
                        } else {
                            "table".into()
                        },
                        estimated_rows: None,
                        total_bytes: None,
                    })
                })
                .map_err(map_sqlite_err)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(map_sqlite_err)
        })
    }

    async fn database_objects(&self) -> AppResult<Vec<DatabaseObjectSummary>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT name, type, tbl_name FROM sqlite_schema WHERE type IN ('table', 'view', 'index', 'trigger') AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
                )
                .map_err(map_sqlite_err)?;
            let rows = stmt
                .query_map([], |row| {
                    let name: String = row.get(0)?;
                    let obj_type: String = row.get(1)?;
                    let tbl_name: String = row.get(2)?;
                    let (kind, owner_table, identity) = match obj_type.as_str() {
                        "view" => (DatabaseObjectKind::View, None, None),
                        "index" => (
                            DatabaseObjectKind::Index,
                            Some(tbl_name.clone()),
                            Some(tbl_name),
                        ),
                        "trigger" => (
                            DatabaseObjectKind::Trigger,
                            Some(tbl_name.clone()),
                            Some(tbl_name),
                        ),
                        _ => (DatabaseObjectKind::Table, None, None),
                    };
                    Ok(DatabaseObjectSummary {
                        object: DatabaseObjectRef {
                            schema: "main".into(),
                            name,
                            kind,
                            identity,
                        },
                        owner_table,
                        estimated_rows: None,
                        total_bytes: None,
                        signature: None,
                    })
                })
                .map_err(map_sqlite_err)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(map_sqlite_err)
        })
    }

    async fn object_details(&self, object: &DatabaseObjectRef) -> AppResult<ObjectDetails> {
        let obj = object.clone();
        let read_only = self.read_only;
        self.with_conn(|conn| {
            let row: Option<(String, String, Option<String>)> = conn
                .query_row(
                    "SELECT type, tbl_name, sql FROM sqlite_schema WHERE name = ?1 AND name NOT LIKE 'sqlite_%'",
                    [&obj.name],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok();
            let Some((raw_type, tbl_name, ddl_sql)) = row else {
                return Err(AppError::new(
                    ErrorCode::NotFound,
                    format!("SQLite object {:?} was not found", obj.name),
                ));
            };

            let ddl = match ddl_sql {
                Some(s) if !s.trim().is_empty() => {
                    let formatted = if s.trim_end().ends_with(';') {
                        s
                    } else {
                        format!("{s};")
                    };
                    ObjectDdlStatus::Available {
                        ddl: formatted,
                        truncated: false,
                    }
                    .capped(DDL_PREVIEW_LIMIT)
                }
                _ => ObjectDdlStatus::Unavailable {
                    reason: "SQLite auto-generated internal object has no user DDL".into(),
                },
            };

            let owner_table = if matches!(raw_type.as_str(), "index" | "trigger") {
                Some(TableRef {
                    schema: "main".into(),
                    name: tbl_name.clone(),
                })
            } else {
                None
            };

            let mut indexes = Vec::new();
            let mut constraints = Vec::new();
            let mut triggers = Vec::new();
            let mut uses = Vec::new();
            let mut used_by = Vec::new();

            if raw_type == "table" {
                let table_ref = TableRef {
                    schema: "main".into(),
                    name: obj.name.clone(),
                };
                let schema = Self::load_schema_sync(conn, &table_ref, read_only)?;
                if !schema.primary_key.is_empty() {
                    constraints.push(ConstraintInfo {
                        name: format!("{}_pk", obj.name),
                        kind: ConstraintKind::PrimaryKey,
                        columns: schema.primary_key.clone(),
                        target: None,
                        target_columns: Vec::new(),
                        definition: format!("PRIMARY KEY ({})", schema.primary_key.join(", ")),
                    });
                }
                for fk in &schema.foreign_keys {
                    constraints.push(ConstraintInfo {
                        name: fk.name.clone(),
                        kind: ConstraintKind::ForeignKey,
                        columns: fk.columns.clone(),
                        target: Some(fk.target.clone()),
                        target_columns: fk.target_columns.clone(),
                        definition: format!(
                            "FOREIGN KEY ({}) REFERENCES {}({})",
                            fk.columns.join(", "),
                            fk.target.name,
                            fk.target_columns.join(", ")
                        ),
                    });
                    uses.push(DependencyEdge {
                        object: DatabaseObjectRef {
                            schema: "main".into(),
                            name: fk.target.name.clone(),
                            kind: DatabaseObjectKind::Table,
                            identity: None,
                        },
                        relation: "foreign key".into(),
                    });
                }
                for in_fk in &schema.inbound_foreign_keys {
                    used_by.push(DependencyEdge {
                        object: DatabaseObjectRef {
                            schema: "main".into(),
                            name: in_fk.source.name.clone(),
                            kind: DatabaseObjectKind::Table,
                            identity: None,
                        },
                        relation: "referenced by FK".into(),
                    });
                }

                let idx_list_sql = format!(
                    "PRAGMA main.index_list({})",
                    quote_sqlite_ident(&obj.name)
                );
                if let Ok(mut idx_stmt) = conn.prepare(&idx_list_sql)
                    && let Ok(idx_rows) = idx_stmt.query_map([], |r| {
                        Ok((
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)? != 0,
                            r.get::<_, String>(3)?,
                        ))
                    })
                {
                    for (idx_name, unique, origin) in idx_rows.flatten() {
                        let cols_sql = format!(
                            "PRAGMA main.index_info({})",
                            quote_sqlite_ident(&idx_name)
                        );
                        let mut raw_cols = Vec::new();
                        if let Ok(mut c_stmt) = conn.prepare(&cols_sql)
                            && let Ok(c_rows) =
                                c_stmt.query_map([], |cr| cr.get::<_, Option<String>>(2))
                        {
                            for col_opt in c_rows.flatten() {
                                raw_cols.push(col_opt);
                            }
                        }
                        let idx_ddl: Option<String> = conn
                            .query_row(
                                "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1",
                                [&idx_name],
                                |r| r.get(0),
                            )
                            .ok()
                            .flatten();
                        let (parsed_exprs, predicate) =
                            parse_sqlite_index_ddl(idx_ddl.as_deref(), &raw_cols, false);
                        if origin == "u" && unique {
                            let u_cols: Vec<String> =
                                raw_cols.iter().filter_map(|c| c.clone()).collect();
                            if !u_cols.is_empty() {
                                let quoted_u_cols = u_cols
                                    .iter()
                                    .map(|c| quote_sqlite_ident(c))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                constraints.push(ConstraintInfo {
                                    name: idx_name.clone(),
                                    kind: ConstraintKind::Unique,
                                    columns: u_cols,
                                    target: None,
                                    target_columns: Vec::new(),
                                    definition: format!("UNIQUE ({quoted_u_cols})"),
                                });
                            }
                        }
                        indexes.push(IndexInfo {
                            name: idx_name,
                            method: "btree".into(),
                            unique,
                            primary: origin == "pk",
                            expressions: parsed_exprs,
                            predicate,
                            definition: Some(idx_ddl.unwrap_or_else(|| "AUTOINDEX".into())),
                            size_bytes: None,
                            scans: None,
                            tuples_read: None,
                            tuples_fetched: None,
                            cardinality: None,
                        });
                    }
                }

                triggers = load_sqlite_triggers_sync(conn, &obj.name);
            }

            Ok(ObjectDetails {
                object: obj.clone(),
                owner_table,
                comment: None,
                ddl,
                indexes,
                constraints,
                triggers,
                routine: None,
                sequence: None,
                storage: None,
                snowflake_table: None,
                uses: finalize_dependency_edges(&obj, uses, 64),
                used_by: finalize_dependency_edges(&obj, used_by, 64),
            })
        })
    }

    async fn schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        let t = table.clone();
        let ro = self.read_only;
        self.with_conn(|conn| Self::load_schema_sync(conn, &t, ro))
    }

    async fn referencing_keys(&self, table: &TableRef) -> AppResult<Vec<InboundForeignKey>> {
        let name = table.name.clone();
        self.with_conn(|conn| Self::referencing_keys_sync(conn, &name))
    }

    async fn schema_diagram(&self, schema_name: &str) -> AppResult<SchemaDiagram> {
        if !schema_name.trim().is_empty() && !schema_name.eq_ignore_ascii_case("main") {
            return Err(AppError::new(
                ErrorCode::NotFound,
                format!("SQLite schema {schema_name:?} was not found"),
            ));
        }
        let tables = self.catalog().await?;
        let mut nodes = Vec::with_capacity(tables.len());
        let mut foreign_keys = Vec::new();
        for t in tables {
            let ts = self.schema(&t.table).await?;
            for fk in &ts.foreign_keys {
                foreign_keys.push(DiagramForeignKey {
                    name: fk.name.clone(),
                    source: ts.table.clone(),
                    columns: fk.columns.clone(),
                    target: fk.target.clone(),
                    target_columns: fk.target_columns.clone(),
                    virtual_relationship: false,
                });
            }
            nodes.push(DiagramTable {
                table: ts.table,
                kind: t.kind,
                columns: ts
                    .columns
                    .into_iter()
                    .map(|c| DiagramColumn {
                        name: c.name,
                        database_type: c.database_type,
                        nullable: c.nullable,
                        primary_key: c.primary_key,
                    })
                    .collect(),
            });
        }
        Ok(SchemaDiagram {
            schema: "main".into(),
            tables: nodes,
            foreign_keys,
            truncated: false,
            restricted: false,
        })
    }

    async fn schema_snapshot(&self, schema_name: &str) -> AppResult<SchemaSnapshot> {
        if !schema_name.trim().is_empty() && !schema_name.eq_ignore_ascii_case("main") {
            return Err(AppError::new(
                ErrorCode::NotFound,
                format!("SQLite schema {schema_name:?} was not found"),
            ));
        }
        let cat = self.catalog().await?;
        let mut snap_tables = Vec::with_capacity(cat.len());
        for t in cat {
            let ts = self.schema(&t.table).await?;
            let is_view = t.kind == "view";
            let (relation_sql, xinfo_cols, snap_unique_constraints, snap_indexes, snap_triggers) =
                self.with_conn(|conn| {
                    let obj_type = if is_view { "view" } else { "table" };
                    let rel_sql: Option<String> = conn
                        .query_row(
                            "SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2",
                            rusqlite::params![obj_type, &t.table.name],
                            |r| r.get(0),
                        )
                        .ok()
                        .flatten();

                    let info_sql = format!(
                        "PRAGMA main.table_xinfo({})",
                        quote_sqlite_ident(&t.table.name)
                    );
                    let mut col_meta = Vec::new();
                    if let Ok(mut c_stmt) = conn.prepare(&info_sql)
                        && let Ok(rows) = c_stmt.query_map([], |row| {
                            Ok((
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, i64>(3)? != 0,
                                row.get::<_, Option<String>>(4)?,
                                row.get::<_, i64>(5)?,
                                row.get::<_, i64>(6)?,
                            ))
                        })
                    {
                        for item in rows.flatten() {
                            if item.5 != 1 {
                                col_meta.push(item);
                            }
                        }
                    }

                    let mut unique_cons = Vec::new();
                    let mut indexes = Vec::new();
                    if !is_view {
                        let idx_list_sql = format!(
                            "PRAGMA main.index_list({})",
                            quote_sqlite_ident(&t.table.name)
                        );
                        if let Ok(mut idx_stmt) = conn.prepare(&idx_list_sql)
                            && let Ok(idx_rows) = idx_stmt.query_map([], |r| {
                                Ok((
                                    r.get::<_, String>(1)?,
                                    r.get::<_, i64>(2)? != 0,
                                    r.get::<_, String>(3)?,
                                ))
                            })
                        {
                            for (idx_name, unique, origin) in idx_rows.flatten() {
                                let cols_sql = format!(
                                    "PRAGMA main.index_info({})",
                                    quote_sqlite_ident(&idx_name)
                                );
                                let mut raw_cols = Vec::new();
                                if let Ok(mut ic_stmt) = conn.prepare(&cols_sql)
                                    && let Ok(ic_rows) =
                                        ic_stmt.query_map([], |cr| cr.get::<_, Option<String>>(2))
                                {
                                    for col_opt in ic_rows.flatten() {
                                        raw_cols.push(col_opt);
                                    }
                                }
                                let idx_ddl: Option<String> = conn
                                    .query_row(
                                        "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1",
                                        [&idx_name],
                                        |r| r.get(0),
                                    )
                                    .ok()
                                    .flatten();
                                let (exprs, predicate) =
                                    parse_sqlite_index_ddl(idx_ddl.as_deref(), &raw_cols, true);
                                if origin == "u" && unique {
                                    let u_cols: Vec<String> =
                                        raw_cols.iter().filter_map(|c| c.clone()).collect();
                                    if !u_cols.is_empty() {
                                        let quoted_u_cols = u_cols
                                            .iter()
                                            .map(|c| quote_sqlite_ident(c))
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        unique_cons.push(ConstraintInfo {
                                            name: idx_name.clone(),
                                            kind: ConstraintKind::Unique,
                                            columns: u_cols,
                                            target: None,
                                            target_columns: Vec::new(),
                                            definition: format!("UNIQUE ({quoted_u_cols})"),
                                        });
                                    }
                                }
                                if origin == "c" {
                                    indexes.push(SnapshotIndex {
                                        name: idx_name,
                                        method: "btree".into(),
                                        unique,
                                        primary: false,
                                        expressions: exprs,
                                        predicate,
                                        definition: idx_ddl,
                                    });
                                }
                            }
                        }
                    }

                    let triggers = load_sqlite_triggers_sync(conn, &t.table.name);
                    Ok((rel_sql, col_meta, unique_cons, indexes, triggers))
                })?;

            let mut snap_constraints = Vec::new();
            if !ts.primary_key.is_empty() {
                let quoted_pk = ts
                    .primary_key
                    .iter()
                    .map(|c| quote_sqlite_ident(c))
                    .collect::<Vec<_>>()
                    .join(", ");
                snap_constraints.push(ConstraintInfo {
                    name: format!("{}_pk", t.table.name),
                    kind: ConstraintKind::PrimaryKey,
                    columns: ts.primary_key.clone(),
                    target: None,
                    target_columns: Vec::new(),
                    definition: format!("PRIMARY KEY ({quoted_pk})"),
                });
            }
            snap_constraints.extend(snap_unique_constraints);
            for fk in &ts.foreign_keys {
                let quoted_src = fk
                    .columns
                    .iter()
                    .map(|c| quote_sqlite_ident(c))
                    .collect::<Vec<_>>()
                    .join(", ");
                let quoted_dst = fk
                    .target_columns
                    .iter()
                    .map(|c| quote_sqlite_ident(c))
                    .collect::<Vec<_>>()
                    .join(", ");
                snap_constraints.push(ConstraintInfo {
                    name: fk.name.clone(),
                    kind: ConstraintKind::ForeignKey,
                    columns: fk.columns.clone(),
                    target: Some(fk.target.clone()),
                    target_columns: fk.target_columns.clone(),
                    definition: format!(
                        "FOREIGN KEY ({quoted_src}) REFERENCES {} ({quoted_dst})",
                        quote_sqlite_ident(&fk.target.name)
                    ),
                });
            }

            let snap_columns: Vec<SnapshotColumn> = if !xinfo_cols.is_empty() {
                xinfo_cols
                    .into_iter()
                    .enumerate()
                    .map(|(idx, (name, db_type, not_null, dflt, pk_ord, hidden))| {
                        let data_type = if db_type.trim().is_empty() {
                            "ANY".to_owned()
                        } else {
                            db_type
                        };
                        let (generated, default_expression) = match hidden {
                            2 => (
                                Some("VIRTUAL".to_owned()),
                                dflt.or_else(|| {
                                    extract_sqlite_generated_expr(relation_sql.as_deref(), &name)
                                }),
                            ),
                            3 => (
                                Some("STORED".to_owned()),
                                dflt.or_else(|| {
                                    extract_sqlite_generated_expr(relation_sql.as_deref(), &name)
                                }),
                            ),
                            _ => (None, dflt),
                        };
                        SnapshotColumn {
                            name,
                            data_type,
                            nullable: !not_null && pk_ord == 0,
                            default_expression,
                            identity: None,
                            generated,
                            ordinal_position: (idx + 1) as u32,
                        }
                    })
                    .collect()
            } else {
                ts.columns
                    .into_iter()
                    .enumerate()
                    .map(|(idx, c)| SnapshotColumn {
                        name: c.name,
                        data_type: c.database_type,
                        nullable: c.nullable,
                        default_expression: None,
                        identity: None,
                        generated: None,
                        ordinal_position: (idx + 1) as u32,
                    })
                    .collect()
            };

            snap_tables.push(SnapshotTable {
                name: t.table.name,
                kind: if is_view {
                    DatabaseObjectKind::View
                } else {
                    DatabaseObjectKind::Table
                },
                comment: None,
                partition_key: None,
                view_definition: if is_view { relation_sql } else { None },
                columns: snap_columns,
                constraints: snap_constraints,
                indexes: snap_indexes,
                triggers: snap_triggers,
            });
        }
        Ok(SchemaSnapshot {
            engine: EngineKind::Sqlite,
            server_version: self.server_version.clone(),
            database: self.database_label.clone(),
            schema: "main".into(),
            role: "local".into(),
            tables: snap_tables,
            routines: Vec::new(),
            sequences: Vec::new(),
            unsupported_objects: Vec::new(),
            truncated: false,
            restricted: false,
        })
    }

    async fn search_data(&self, request: &DataSearchRequest) -> AppResult<DataSearchReport> {
        let started = Instant::now();
        let max_tables = validate_search_request(request)?;
        let all_tables: Vec<TableRef> =
            self.catalog().await?.into_iter().map(|t| t.table).collect();
        let (candidates, mut truncated) = filter_candidate_tables(all_tables, request, max_tables);

        let mut scanned_tables = 0usize;
        let mut skipped_tables = 0usize;
        let mut total_row_hits = 0usize;
        let mut report_bytes = 0usize;
        let mut matches = Vec::new();
        let needle = request.query.as_str();

        for table in candidates {
            if self.is_closed() {
                return Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "SQLite connection closed during data search",
                ));
            }
            if total_row_hits >= MAX_SEARCH_TOTAL_ROW_HITS
                || report_bytes >= MAX_SEARCH_REPORT_BYTES
            {
                truncated = true;
                break;
            }
            let ts = match self.schema(&table).await {
                Ok(s) => s,
                Err(_) => {
                    skipped_tables += 1;
                    continue;
                }
            };
            let searchable_cols: Vec<&ColumnInfo> = ts
                .columns
                .iter()
                .filter(|c| c.kind != ColumnKind::Binary)
                .collect();
            if searchable_cols.is_empty() {
                skipped_tables += 1;
                continue;
            }
            scanned_tables += 1;

            let bind_val = match (request.match_mode, request.case_sensitive) {
                (DataSearchMatchMode::Exact, _) => needle.to_owned(),
                (_, true) => needle.to_owned(),
                (DataSearchMatchMode::Contains, false) => {
                    format!("%{}%", escape_like_pattern(needle))
                }
                (DataSearchMatchMode::StartsWith, false) => {
                    format!("{}%", escape_like_pattern(needle))
                }
            };

            let preds: Vec<String> = searchable_cols
                .iter()
                .map(|c| {
                    let col_expr = format!("CAST({} AS TEXT)", quote_sqlite_ident(&c.name));
                    match (request.match_mode, request.case_sensitive) {
                        (DataSearchMatchMode::Exact, true) => format!("{col_expr} = ?1"),
                        (DataSearchMatchMode::Exact, false) => {
                            format!("LOWER({col_expr}) = LOWER(?1)")
                        }
                        (DataSearchMatchMode::Contains, true) => {
                            format!("INSTR({col_expr}, ?1) > 0")
                        }
                        (DataSearchMatchMode::StartsWith, true) => {
                            format!("SUBSTR({col_expr}, 1, LENGTH(?1)) = ?1")
                        }
                        (_, false) => format!("LOWER({col_expr}) LIKE LOWER(?1) ESCAPE '!'"),
                    }
                })
                .collect();

            let proj = ts
                .columns
                .iter()
                .map(|c| quote_sqlite_ident(&c.name))
                .collect::<Vec<_>>()
                .join(", ");
            let remaining =
                (MAX_SEARCH_TOTAL_ROW_HITS - total_row_hits).min(MAX_SEARCH_ROWS_PER_TABLE);
            let sql = format!(
                "SELECT {proj} FROM {} WHERE ({}) LIMIT {}",
                quote_sqlite_table(&ts.table),
                preds.join(" OR "),
                remaining + 1
            );

            let fetched = run_blocking_in_place(|| {
                self.with_conn(|conn| {
                    let mut stmt = conn.prepare(&sql).map_err(map_sqlite_err)?;
                    let mut rows_iter = stmt.query([&bind_val]).map_err(map_sqlite_err)?;
                    let mut rows = Vec::new();
                    while let Some(row) = rows_iter.next().map_err(map_sqlite_err)? {
                        let mut cells = Vec::with_capacity(ts.columns.len());
                        for (ci, col) in ts.columns.iter().enumerate() {
                            let v = row.get_ref(ci).map_err(map_sqlite_err)?;
                            cells.push(format_value_ref(v, col.kind));
                        }
                        rows.push(cells);
                    }
                    Ok(rows)
                })
            });

            let Ok(rows) = fetched else {
                skipped_tables += 1;
                continue;
            };
            let searchable_names: Vec<String> =
                searchable_cols.iter().map(|c| c.name.clone()).collect();
            if let Some(hit) = build_table_match(
                &ts,
                searchable_names,
                rows,
                needle,
                request.match_mode,
                request.case_sensitive,
                remaining,
            ) {
                if hit.has_more_in_table {
                    truncated = true;
                }
                report_bytes = report_bytes.saturating_add(estimate_table_match_bytes(&hit));
                total_row_hits += hit.rows.len();
                matches.push(hit);
            }
        }

        Ok(DataSearchReport {
            query: request.query.clone(),
            match_mode: request.match_mode,
            case_sensitive: request.case_sensitive,
            database: Some(self.database_label.clone()),
            scanned_tables,
            skipped_tables,
            total_row_hits,
            truncated,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
            matches,
        })
    }

    async fn browse(
        &self,
        schema: &TableSchema,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        self.browse_query(schema, &BrowseQuery::default(), after)
            .await
    }

    async fn browse_query(
        &self,
        schema: &TableSchema,
        query: &BrowseQuery,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        let (sql, built) = browse_sql(schema, query, after.as_deref())?;
        let params: Vec<SqliteValue> = built
            .binds
            .iter()
            .map(|b| match b {
                Some(s) => SqliteValue::Text(s.clone()),
                None => SqliteValue::Null,
            })
            .collect();
        let cols = schema.columns.clone();
        run_blocking_in_place(|| {
            self.with_conn(|conn| {
                let mut stmt = conn.prepare(&sql).map_err(map_sqlite_err)?;
                let mut rows_iter = stmt
                    .query(rusqlite::params_from_iter(params.iter()))
                    .map_err(map_sqlite_err)?;
                let mut rows = Vec::with_capacity(PAGE_SIZE + 1);
                while let Some(row) = rows_iter.next().map_err(map_sqlite_err)? {
                    let mut cells = Vec::with_capacity(cols.len());
                    for (i, col) in cols.iter().enumerate() {
                        let v = row.get_ref(i).map_err(map_sqlite_err)?;
                        cells.push(format_value_ref(v, col.kind));
                    }
                    rows.push(cells);
                }
                let has_more = rows.len() > PAGE_SIZE && built.pageable;
                let truncated = rows.len() > PAGE_SIZE && !built.pageable;
                if rows.len() > PAGE_SIZE {
                    rows.truncate(PAGE_SIZE);
                }
                let next_key = if has_more {
                    rows.last().map(|r| built.cursor(r))
                } else {
                    None
                };
                Ok(BrowseRows {
                    rows,
                    next_key,
                    has_more,
                    truncated,
                })
            })
        })
    }

    async fn apply(&self, schema: &TableSchema, rows: &[RowMutation]) -> AppResult<()> {
        if self.read_only {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "SQLite connection is in read-only mode",
            ));
        }
        if schema.primary_key.is_empty()
            && rows
                .iter()
                .any(|r| matches!(r, RowMutation::Update { .. } | RowMutation::Delete { .. }))
        {
            return Err(AppError::invalid(
                "SQLite row updates and deletes require a primary key",
            ));
        }
        let table_sql = quote_sqlite_table(&schema.table);
        let build_row_conflict_where =
            |original: &TextRow, params: &mut Vec<SqliteValue>| -> AppResult<String> {
                if original.len() != schema.columns.len() {
                    return Err(AppError::invalid(
                        "Original SQLite row length does not match table schema",
                    ));
                }
                let mut where_parts = Vec::with_capacity(schema.columns.len());
                for (col, orig_val) in schema.columns.iter().zip(original.iter()) {
                    let col_sql = quote_sqlite_ident(&col.name);
                    match orig_val.as_deref() {
                        None => where_parts.push(format!("{col_sql} IS NULL")),
                        Some(v) if col.kind == ColumnKind::Boolean => {
                            let lower = v.trim().to_ascii_lowercase();
                            if matches!(lower.as_str(), "true" | "t" | "1" | "yes") {
                                where_parts.push(format!(
                                    "({col_sql} IS 1 OR LOWER(CAST({col_sql} AS TEXT)) = 'true')"
                                ));
                            } else if matches!(lower.as_str(), "false" | "f" | "0" | "no") {
                                where_parts.push(format!(
                                    "({col_sql} IS 0 OR LOWER(CAST({col_sql} AS TEXT)) = 'false')"
                                ));
                            } else {
                                where_parts.push(format!("{col_sql} IS ?"));
                                params.push(SqliteValue::Text(v.to_owned()));
                            }
                        }
                        Some(_) => {
                            where_parts.push(format!("{col_sql} IS ?"));
                            params.push(parse_sqlite_param(orig_val, col.kind));
                        }
                    }
                }
                Ok(where_parts.join(" AND "))
            };

        run_blocking_in_place(|| {
            self.with_conn(|conn| {
                if !conn.is_autocommit() {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "Commit or roll back the open SQLite editor transaction before applying table grid edits",
                    ));
                }
                conn.execute_batch("SAVEPOINT joinery_grid_apply")
                    .map_err(map_sqlite_err)?;
                let outcome: AppResult<()> = (|| {
                    for mutation in rows {
                        match mutation {
                            RowMutation::Update { original, changes } => {
                                let mut set_parts = Vec::with_capacity(changes.len());
                                let mut params: Vec<SqliteValue> = Vec::new();
                                for (col_idx, new_val) in changes {
                                    let col = schema
                                        .columns
                                        .get(*col_idx)
                                        .ok_or_else(|| AppError::invalid("Invalid column index"))?;
                                    set_parts
                                        .push(format!("{} = ?", quote_sqlite_ident(&col.name)));
                                    params.push(parse_sqlite_param(new_val, col.kind));
                                }
                                let where_clause =
                                    build_row_conflict_where(original, &mut params)?;
                                let sql = format!(
                                    "UPDATE {table_sql} SET {} WHERE {where_clause}",
                                    set_parts.join(", ")
                                );
                                let affected = conn
                                    .execute(&sql, rusqlite::params_from_iter(params.iter()))
                                    .map_err(map_sqlite_err)?;
                                if affected != 1 {
                                    return Err(AppError::new(
                                        ErrorCode::Conflict,
                                        "Row changed or disappeared before SQLite grid edit could be applied",
                                    ));
                                }
                            }
                            RowMutation::Delete { original } => {
                                let mut params: Vec<SqliteValue> = Vec::new();
                                let where_clause =
                                    build_row_conflict_where(original, &mut params)?;
                                let sql = format!("DELETE FROM {table_sql} WHERE {where_clause}");
                                let affected = conn
                                    .execute(&sql, rusqlite::params_from_iter(params.iter()))
                                    .map_err(map_sqlite_err)?;
                                if affected != 1 {
                                    return Err(AppError::new(
                                        ErrorCode::Conflict,
                                        "Row changed or disappeared before SQLite row delete could be applied",
                                    ));
                                }
                            }
                        RowMutation::Insert { values } => {
                            if values.is_empty() {
                                let sql = format!("INSERT INTO {table_sql} DEFAULT VALUES");
                                conn.execute(&sql, []).map_err(map_sqlite_err)?;
                            } else {
                                let mut cols = Vec::with_capacity(values.len());
                                let mut placeholders = Vec::with_capacity(values.len());
                                let mut params: Vec<SqliteValue> = Vec::with_capacity(values.len());
                                for (col_idx, v) in values {
                                    let col = schema
                                        .columns
                                        .get(*col_idx)
                                        .ok_or_else(|| AppError::invalid("Invalid column index"))?;
                                    cols.push(quote_sqlite_ident(&col.name));
                                    placeholders.push("?");
                                    params.push(parse_sqlite_param(v, col.kind));
                                }
                                let sql = format!(
                                    "INSERT INTO {table_sql} ({}) VALUES ({})",
                                    cols.join(", "),
                                    placeholders.join(", ")
                                );
                                conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
                                    .map_err(map_sqlite_err)?;
                            }
                        }
                    }
                }
                Ok(())
            })();

            match outcome {
                Ok(()) => {
                    conn.execute_batch("RELEASE SAVEPOINT joinery_grid_apply")
                        .map_err(map_sqlite_err)?;
                    Ok(())
                }
                Err(err) => {
                    let _ = conn.execute_batch(
                        "ROLLBACK TO SAVEPOINT joinery_grid_apply; RELEASE SAVEPOINT joinery_grid_apply;",
                    );
                    Err(err)
                }
            }
            })
        })
    }

    async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
        Ok(Vec::new())
    }

    async fn cancel_process(&self, _id: &str, _identity: &str) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "SQLite is an embedded local engine without remote server processes",
        ))
    }

    async fn health(&self) -> AppResult<u32> {
        let started = Instant::now();
        self.with_conn(|conn| {
            let one: i64 = conn
                .query_row("SELECT 1", [], |r| r.get(0))
                .map_err(map_sqlite_err)?;
            let _ = one;
            Ok(started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32)
        })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.interrupt.interrupt();
            let mut guard = self.conn.lock();
            let _ = guard.take();
        }
    }
}
