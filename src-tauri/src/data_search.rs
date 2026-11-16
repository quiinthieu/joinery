//! Bounded, parameterized full-text data search helpers across database tables.
//!
//! Identifiers are always quoted from live catalog/schema metadata; search terms
//! are strictly bound as parameters (never interpolated) with explicit LIKE escaping.

use crate::domain::*;
use std::collections::HashSet;

pub const DEFAULT_SEARCH_MAX_TABLES: usize = 25;
pub const MAX_SEARCH_TABLES: usize = 50;
pub const MAX_SEARCH_COLUMNS_PER_TABLE: usize = 64;
pub const MAX_SEARCH_ROWS_PER_TABLE: usize = 10;
pub const MAX_SEARCH_TOTAL_ROW_HITS: usize = 200;
pub const MAX_SEARCH_REPORT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SEARCH_QUERY_BYTES: usize = 512;
pub const LIKE_ESCAPE_CHAR: char = '!';

pub fn validate_search_request(request: &DataSearchRequest) -> AppResult<usize> {
    if request.query.is_empty() || request.query.contains('\0') {
        return Err(AppError::invalid("Enter a non-empty search value"));
    }
    if request.query.len() > MAX_SEARCH_QUERY_BYTES {
        return Err(AppError::invalid(
            "Search value is limited to 512 UTF-8 bytes",
        ));
    }
    let max_tables = request.max_tables.unwrap_or(DEFAULT_SEARCH_MAX_TABLES);
    if !(1..=MAX_SEARCH_TABLES).contains(&max_tables) {
        return Err(AppError::invalid(format!(
            "Search table limit must be between 1 and {MAX_SEARCH_TABLES}"
        )));
    }
    for schema in &request.schemas {
        if schema.trim().is_empty() || schema.len() > 256 || schema.contains('\0') {
            return Err(AppError::invalid("Invalid schema filter"));
        }
    }
    for table in &request.tables {
        if table.schema.trim().is_empty()
            || table.schema.len() > 256
            || table.schema.contains('\0')
            || table.name.trim().is_empty()
            || table.name.len() > 256
            || table.name.contains('\0')
        {
            return Err(AppError::invalid("Invalid table filter"));
        }
    }
    Ok(max_tables)
}

pub fn filter_candidate_tables(
    all_tables: Vec<TableRef>,
    request: &DataSearchRequest,
    max_tables: usize,
) -> (Vec<TableRef>, bool) {
    let schema_filter: HashSet<&str> = request.schemas.iter().map(String::as_str).collect();
    let table_filter: HashSet<(&str, &str)> = request
        .tables
        .iter()
        .map(|t| (t.schema.as_str(), t.name.as_str()))
        .collect();

    let mut matched: Vec<TableRef> = all_tables
        .into_iter()
        .filter(|t| {
            if !table_filter.is_empty() {
                return table_filter.contains(&(t.schema.as_str(), t.name.as_str()));
            }
            if !schema_filter.is_empty() {
                return schema_filter.contains(t.schema.as_str());
            }
            true
        })
        .collect();
    matched.sort_by(|a, b| a.schema.cmp(&b.schema).then_with(|| a.name.cmp(&b.name)));
    matched.dedup();
    let truncated = matched.len() > max_tables;
    if truncated {
        matched.truncate(max_tables);
    }
    (matched, truncated)
}

pub fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '%' | '_') || ch == LIKE_ESCAPE_CHAR {
            escaped.push(LIKE_ESCAPE_CHAR);
        }
        escaped.push(ch);
    }
    escaped
}

pub fn like_bind_value(query: &str, mode: DataSearchMatchMode) -> String {
    match mode {
        DataSearchMatchMode::Exact => query.to_owned(),
        DataSearchMatchMode::Contains => format!("%{}%", escape_like_pattern(query)),
        DataSearchMatchMode::StartsWith => format!("{}%", escape_like_pattern(query)),
    }
}

pub fn searchable_columns(schema: &TableSchema) -> Vec<&ColumnInfo> {
    schema
        .columns
        .iter()
        .filter(|c| c.kind != ColumnKind::Binary)
        .take(MAX_SEARCH_COLUMNS_PER_TABLE)
        .collect()
}

fn quote_double(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn quote_backtick(ident: &str) -> String {
    format!("`{}`", ident.replace('`', "``"))
}

pub fn build_postgres_search_sql(
    schema: &TableSchema,
    query: &str,
    mode: DataSearchMatchMode,
    case_sensitive: bool,
    row_limit: usize,
) -> Option<(String, String, Vec<String>)> {
    let searchable = searchable_columns(schema);
    if searchable.is_empty() || schema.columns.is_empty() {
        return None;
    }
    let table_sql = format!(
        "{}.{}",
        quote_double(&schema.table.schema),
        quote_double(&schema.table.name)
    );
    let projection = schema
        .columns
        .iter()
        .enumerate()
        .map(|(idx, col)| {
            let q = quote_double(&col.name);
            if col.kind == ColumnKind::Binary {
                format!(
                    "CASE WHEN {table_sql}.{q} IS NULL THEN NULL ELSE '0x' || pg_catalog.encode({table_sql}.{q}, 'hex') END AS __joinery_c{idx}"
                )
            } else {
                format!("{table_sql}.{q}::pg_catalog.text AS __joinery_c{idx}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let predicates = searchable
        .iter()
        .map(|col| {
            let expr = format!("{table_sql}.{}::pg_catalog.text", quote_double(&col.name));
            match (mode, case_sensitive) {
                (DataSearchMatchMode::Exact, true) => format!("{expr} = $1::pg_catalog.text"),
                (DataSearchMatchMode::Exact, false) => {
                    format!("pg_catalog.lower({expr}) = pg_catalog.lower($1::pg_catalog.text)")
                }
                (_, true) => format!("{expr} LIKE $1::pg_catalog.text ESCAPE '{LIKE_ESCAPE_CHAR}'"),
                (_, false) => {
                    format!("{expr} ILIKE $1::pg_catalog.text ESCAPE '{LIKE_ESCAPE_CHAR}'")
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" OR ");

    let sql = format!(
        "SELECT {projection} FROM {table_sql} WHERE ({predicates}) LIMIT {}",
        row_limit.saturating_add(1)
    );
    let bind = like_bind_value(query, mode);
    let names = searchable.into_iter().map(|c| c.name.clone()).collect();
    Some((sql, bind, names))
}

pub fn build_mysql_search_sql(
    schema: &TableSchema,
    query: &str,
    mode: DataSearchMatchMode,
    case_sensitive: bool,
    row_limit: usize,
) -> Option<(String, Vec<String>, Vec<String>)> {
    let searchable = searchable_columns(schema);
    if searchable.is_empty() || schema.columns.is_empty() {
        return None;
    }
    let table_sql = format!(
        "{}.{}",
        quote_backtick(&schema.table.schema),
        quote_backtick(&schema.table.name)
    );
    let projection = schema
        .columns
        .iter()
        .map(|col| quote_backtick(&col.name))
        .collect::<Vec<_>>()
        .join(", ");

    let bind_val = like_bind_value(query, mode);
    let mut binds = Vec::with_capacity(searchable.len());
    let mut predicates = Vec::with_capacity(searchable.len());
    for col in &searchable {
        let base = format!(
            "CAST({} AS CHAR CHARACTER SET utf8mb4)",
            quote_backtick(&col.name)
        );
        let pred = match (mode, case_sensitive) {
            (DataSearchMatchMode::Exact, true) => format!("{base} COLLATE utf8mb4_bin = ?"),
            (DataSearchMatchMode::Exact, false) => format!("LOWER({base}) = LOWER(?)"),
            (_, true) => {
                format!("{base} COLLATE utf8mb4_bin LIKE ? ESCAPE '{LIKE_ESCAPE_CHAR}'")
            }
            (_, false) => format!("LOWER({base}) LIKE LOWER(?) ESCAPE '{LIKE_ESCAPE_CHAR}'"),
        };
        predicates.push(pred);
        binds.push(bind_val.clone());
    }

    let sql = format!(
        "SELECT {projection} FROM {table_sql} WHERE ({}) LIMIT {}",
        predicates.join(" OR "),
        row_limit.saturating_add(1)
    );
    let names = searchable.into_iter().map(|c| c.name.clone()).collect();
    Some((sql, binds, names))
}

pub fn snowflake_search_expr(col: &ColumnInfo) -> String {
    let q = quote_double(&col.name);
    let upper = col.database_type.trim().to_ascii_uppercase();
    if upper.starts_with("OBJECT") || upper.starts_with("ARRAY") {
        format!("TO_JSON({q})")
    } else if upper.starts_with("VARIANT") {
        format!("IFF(IS_OBJECT({q}) OR IS_ARRAY({q}), TO_JSON({q}), TO_VARCHAR({q}))")
    } else if upper.starts_with("GEOGRAPHY") || upper.starts_with("GEOMETRY") {
        format!("ST_ASEWKT({q})")
    } else {
        format!("TO_VARCHAR({q})")
    }
}

pub fn build_snowflake_search_sql(
    database: &str,
    schema: &TableSchema,
    query: &str,
    mode: DataSearchMatchMode,
    case_sensitive: bool,
    row_limit: usize,
) -> Option<(String, Vec<serde_json::Value>, Vec<String>)> {
    let searchable = searchable_columns(schema);
    if searchable.is_empty() || schema.columns.is_empty() {
        return None;
    }
    let table_sql = format!(
        "{}.{}.{}",
        quote_double(database),
        quote_double(&schema.table.schema),
        quote_double(&schema.table.name)
    );
    let projection = schema
        .columns
        .iter()
        .map(|col| {
            let q = quote_double(&col.name);
            let upper = col.database_type.trim().to_ascii_uppercase();
            if upper.starts_with("GEOGRAPHY") || upper.starts_with("GEOMETRY") {
                format!("ST_ASEWKT({q}) AS {q}")
            } else {
                q
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let mut binds = Vec::with_capacity(searchable.len());
    let mut predicates = Vec::with_capacity(searchable.len());
    for col in &searchable {
        let expr = snowflake_search_expr(col);
        let pred = match (mode, case_sensitive) {
            (DataSearchMatchMode::Exact, true) => format!("{expr} = ?"),
            (DataSearchMatchMode::Exact, false) => format!("LOWER({expr}) = LOWER(?)"),
            (DataSearchMatchMode::Contains, true) => format!("CONTAINS({expr}, ?)"),
            (DataSearchMatchMode::Contains, false) => {
                format!("CONTAINS(LOWER({expr}), LOWER(?))")
            }
            (DataSearchMatchMode::StartsWith, true) => format!("STARTSWITH({expr}, ?)"),
            (DataSearchMatchMode::StartsWith, false) => {
                format!("STARTSWITH(LOWER({expr}), LOWER(?))")
            }
        };
        predicates.push(pred);
        binds.push(serde_json::Value::String(query.to_owned()));
    }

    let sql = format!(
        "SELECT {projection} FROM {table_sql} WHERE ({}) LIMIT {}",
        predicates.join(" OR "),
        row_limit.saturating_add(1)
    );
    let names = searchable.into_iter().map(|c| c.name.clone()).collect();
    Some((sql, binds, names))
}

pub fn cell_matches_query(
    cell: &str,
    query: &str,
    mode: DataSearchMatchMode,
    case_sensitive: bool,
) -> bool {
    if case_sensitive {
        match mode {
            DataSearchMatchMode::Contains => cell.contains(query),
            DataSearchMatchMode::Exact => cell == query,
            DataSearchMatchMode::StartsWith => cell.starts_with(query),
        }
    } else {
        let cell_lower = cell.to_lowercase();
        let query_lower = query.to_lowercase();
        match mode {
            DataSearchMatchMode::Contains => cell_lower.contains(&query_lower),
            DataSearchMatchMode::Exact => cell_lower == query_lower,
            DataSearchMatchMode::StartsWith => cell_lower.starts_with(&query_lower),
        }
    }
}

pub fn build_table_match(
    schema: &TableSchema,
    searchable_columns: Vec<String>,
    mut raw_rows: Vec<TextRow>,
    query: &str,
    mode: DataSearchMatchMode,
    case_sensitive: bool,
    row_limit: usize,
) -> Option<DataSearchTableMatch> {
    if raw_rows.is_empty() {
        return None;
    }
    let has_more_in_table = raw_rows.len() > row_limit;
    if has_more_in_table {
        raw_rows.truncate(row_limit);
    }
    let searchable_set: HashSet<&str> = searchable_columns.iter().map(String::as_str).collect();
    let rows = raw_rows
        .into_iter()
        .map(|row| {
            let matched_columns = schema
                .columns
                .iter()
                .zip(row.iter())
                .filter_map(|(col, cell)| {
                    if !searchable_set.contains(col.name.as_str()) {
                        return None;
                    }
                    let text = cell.as_deref()?;
                    let matched = cell_matches_query(text, query, mode, case_sensitive)
                        || (col.kind == ColumnKind::Json
                            && serde_json::from_str::<serde_json::Value>(text)
                                .ok()
                                .map(|v| v.to_string())
                                .is_some_and(|compact| {
                                    cell_matches_query(&compact, query, mode, case_sensitive)
                                }));
                    matched.then(|| col.name.clone())
                })
                .collect();
            DataSearchRowHit {
                matched_columns,
                row,
            }
        })
        .collect();

    Some(DataSearchTableMatch {
        table: schema.table.clone(),
        columns: schema.columns.clone(),
        searchable_columns,
        rows,
        has_more_in_table,
    })
}

pub fn estimate_table_match_bytes(m: &DataSearchTableMatch) -> usize {
    let col_bytes: usize = m.columns.iter().map(|c| 64 + c.name.len()).sum();
    let row_bytes: usize = m
        .rows
        .iter()
        .map(|r| {
            32 + r
                .row
                .iter()
                .map(|c| 8 + c.as_ref().map_or(0, String::len))
                .sum::<usize>()
                + r.matched_columns.iter().map(String::len).sum::<usize>()
        })
        .sum();
    128 + col_bytes + row_bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_schema() -> TableSchema {
        TableSchema {
            table: TableRef {
                schema: "public".into(),
                name: "customers".into(),
            },
            columns: vec![
                ColumnInfo {
                    id: "id".into(),
                    name: "id".into(),
                    database_type: "bigint".into(),
                    kind: ColumnKind::Integer,
                    nullable: false,
                    primary_key: true,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "email".into(),
                    name: "email".into(),
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
                    id: "avatar".into(),
                    name: "avatar".into(),
                    database_type: "bytea".into(),
                    kind: ColumnKind::Binary,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: false,
                    insertable: false,
                    has_default: false,
                },
            ],
            primary_key: vec!["id".into()],
            foreign_keys: vec![],
            inbound_foreign_keys: vec![],
            editable: true,
        }
    }

    #[test]
    fn escapes_like_metacharacters_and_skips_binary_predicates_across_all_dialects() {
        let schema = sample_schema();
        let (pg_sql, pg_bind, searchable) = build_postgres_search_sql(
            &schema,
            "100%_vip!",
            DataSearchMatchMode::Contains,
            false,
            10,
        )
        .unwrap();
        assert_eq!(searchable, vec!["id", "email"]);
        assert_eq!(pg_bind, "%100!%!_vip!!%");
        assert!(pg_sql.contains("ILIKE $1::pg_catalog.text ESCAPE '!'"));
        assert!(!pg_sql.contains("\"avatar\"::pg_catalog.text ILIKE"));

        let (my_sql, my_binds, _) = build_mysql_search_sql(
            &schema,
            "100%_vip!",
            DataSearchMatchMode::Contains,
            true,
            10,
        )
        .unwrap();
        assert_eq!(my_binds.len(), 2);
        assert!(my_sql.contains("COLLATE utf8mb4_bin LIKE ? ESCAPE '!'"));

        let mut sf_schema = schema.clone();
        sf_schema.columns.push(ColumnInfo {
            id: "payload".into(),
            name: "payload".into(),
            database_type: "OBJECT".into(),
            kind: ColumnKind::Json,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        });
        sf_schema.columns.push(ColumnInfo {
            id: "coords".into(),
            name: "coords".into(),
            database_type: "GEOGRAPHY".into(),
            kind: ColumnKind::Unknown,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        });

        let (sf_sql, sf_binds, _) = build_snowflake_search_sql(
            "ANALYTICS",
            &sf_schema,
            "vip",
            DataSearchMatchMode::StartsWith,
            false,
            10,
        )
        .unwrap();
        assert!(sf_sql.starts_with(
            "SELECT \"id\", \"email\", \"avatar\", \"payload\", ST_ASEWKT(\"coords\") AS \"coords\" FROM \"ANALYTICS\".\"public\".\"customers\""
        ));
        assert!(sf_sql.contains("STARTSWITH(LOWER(TO_VARCHAR(\"email\")), LOWER(?))"));
        assert!(sf_sql.contains("STARTSWITH(LOWER(TO_JSON(\"payload\")), LOWER(?))"));
        assert!(sf_sql.contains("STARTSWITH(LOWER(ST_ASEWKT(\"coords\")), LOWER(?))"));
        assert_eq!(sf_binds.len(), 4);

        let variant_col = ColumnInfo {
            id: "doc".into(),
            name: "doc".into(),
            database_type: "VARIANT".into(),
            kind: ColumnKind::Json,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        };
        assert_eq!(
            snowflake_search_expr(&variant_col),
            "IFF(IS_OBJECT(\"doc\") OR IS_ARRAY(\"doc\"), TO_JSON(\"doc\"), TO_VARCHAR(\"doc\"))"
        );

        let spatial_match = build_table_match(
            &sf_schema,
            vec![
                "id".into(),
                "email".into(),
                "payload".into(),
                "coords".into(),
            ],
            vec![vec![
                Some("1".into()),
                Some("ada@example.com".into()),
                None,
                Some("{\n  \"tier\": \"VIP\"\n}".into()),
                Some("SRID=4326;POINT(-122.4194 37.7749)".into()),
            ]],
            "POINT(-122.4194",
            DataSearchMatchMode::Contains,
            false,
            10,
        )
        .unwrap();
        assert_eq!(spatial_match.rows[0].matched_columns, vec!["coords"]);

        let json_match = build_table_match(
            &sf_schema,
            vec![
                "id".into(),
                "email".into(),
                "payload".into(),
                "coords".into(),
            ],
            vec![vec![
                Some("1".into()),
                Some("ada@example.com".into()),
                None,
                Some("{\n  \"tier\": \"VIP\"\n}".into()),
                None,
            ]],
            "{\"tier\":\"VIP\"",
            DataSearchMatchMode::Contains,
            true,
            10,
        )
        .unwrap();
        assert_eq!(json_match.rows[0].matched_columns, vec!["payload"]);
    }

    #[test]
    fn identifies_matched_columns_and_truncates_extra_probe_row() {
        let schema = sample_schema();
        let m = build_table_match(
            &schema,
            vec!["id".into(), "email".into()],
            vec![
                vec![Some("1".into()), Some("Ada@Example.com".into()), None],
                vec![Some("2".into()), Some("Grace@Example.com".into()), None],
            ],
            "ada@",
            DataSearchMatchMode::Contains,
            false,
            1,
        )
        .unwrap();
        assert!(m.has_more_in_table);
        assert_eq!(m.rows.len(), 1);
        assert_eq!(m.rows[0].matched_columns, vec!["email"]);
    }
}
