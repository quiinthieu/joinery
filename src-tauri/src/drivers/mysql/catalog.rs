use super::values::{cast, identifier, key_predicate, projection, qualified};
use super::*;

impl Mysql {
    pub(super) async fn live_completion_catalog(&self) -> AppResult<CompletionRelations> {
        self.bounded(async {
            let mut conn = self.editor.connection.lock().await;
            let was_in_trans = observe_status_flags(&mut conn)
                .await?
                .contains(StatusFlags::SERVER_STATUS_IN_TRANS);
            let rows = fetch_text(&mut conn,"SELECT TABLE_SCHEMA,TABLE_NAME,TABLE_TYPE,IF(@@lower_case_table_names=0,BINARY TABLE_SCHEMA=BINARY DATABASE(),LOWER(TABLE_SCHEMA)=LOWER(DATABASE())) FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA NOT IN ('mysql','performance_schema','sys') ORDER BY TABLE_SCHEMA,TABLE_NAME LIMIT 10001", 10000, 2*1024*1024).await;
            Self::close_phantom_transaction(&mut conn, was_in_trans).await;
            let rows = rows?;
            let tables:Vec<TableInfo> = rows.iter().map(|r| TableInfo {
                table: TableRef { schema: cell(r,0).into(),name: cell(r,1).into() },
                kind: cell(r,2).into(),estimated_rows:None,total_bytes:None,
            }).collect();
            let unqualified_tables = rows.iter().zip(&tables).filter(|(r,_)|cell(r,3)=="1").map(|(_,t)|t.table.clone()).collect();
            Ok(CompletionRelations { tables, unqualified_tables })
        }).await
    }
    pub(super) async fn live_completion_schema(
        &self,
        table: &TableRef,
        require_complete: bool,
    ) -> AppResult<TableSchema> {
        identifier(&table.schema)?;
        identifier(&table.name)?;
        self.bounded(async {
            let mut conn = self.editor.connection.lock().await;
            let was_in_trans = observe_status_flags(&mut conn)
                .await?
                .contains(StatusFlags::SERVER_STATUS_IN_TRANS);
            // SHOW resolves session-local temporary shadows; INFORMATION_SCHEMA
            // would describe the permanent table instead. Never execute SELECTs
            // against user objects to discover their columns.
            let qualified_table = qualified(table);
            let outcome = async {
                let rows = fetch_text(
                    &mut conn,
                    &format!("SHOW FULL COLUMNS FROM {qualified_table}"),
                    4096,
                    1024 * 1024,
                )
                .await?;
                if require_complete {
                    // SHOW COLUMNS can hide columns with no privileges. Compare
                    // against the full server DDL before claiming any absence.
                    // Unsupported DDL or SHOW CREATE permissions fail closed.
                    let create = fetch_text(&mut conn, &format!("SHOW CREATE TABLE {qualified_table}"), 1, 1024 * 1024).await?;
                    let ddl = create.first().map(|row| cell(row, 1)).unwrap_or_default();
                    let complete = sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::MySqlDialect {}, ddl)
                        .ok().filter(|statements| statements.len() == 1)
                        .and_then(|mut statements| match statements.remove(0) {
                            sqlparser::ast::Statement::CreateTable(table) => Some(table.columns),
                            _ => None,
                        });
                    let visible: std::collections::HashSet<&str> = rows.iter()
                        .filter(|row| cell(row, 7).split(',').any(|p| p.eq_ignore_ascii_case("select")))
                        .map(|row| cell(row, 0)).collect();
                    if !complete.is_some_and(|columns| !columns.is_empty() && columns.len() == visible.len() && columns.iter().all(|column| visible.contains(column.name.value.as_str()))) {
                        return Err(AppError::new(ErrorCode::PermissionDenied, "Complete selectable column metadata is unavailable; no column claim was made"));
                    }
                }
                let mut columns: Vec<ColumnInfo> = rows
                    .iter()
                    .filter(|r| {
                        cell(r, 7)
                            .split(',')
                            .any(|p| p.eq_ignore_ascii_case("select"))
                    })
                    .enumerate()
                    .map(|(i, r)| ColumnInfo {
                        id: format!("c{i}"),
                        name: cell(r, 0).into(),
                        database_type: cell(r, 1).into(),
                        kind: ColumnKind::Unknown,
                        nullable: cell(r, 3) == "YES",
                        primary_key: cell(r, 4) == "PRI",
                        foreign_key: false,
                        editable: false,
                        insertable: false,
                        has_default: false,
                    })
                    .collect();
                if columns.is_empty() {
                    return Err(AppError::new(
                        ErrorCode::PermissionDenied,
                        "No selectable columns are available in this completion context",
                    ));
                }
                let (foreign_keys, inbound_foreign_keys) =
                    completion_fks_on(&mut conn, table, &qualified_table, &columns).await;
                for c in &mut columns {
                    c.foreign_key = foreign_keys
                        .iter()
                        .any(|fk| fk.columns.iter().any(|col| col == &c.name));
                }
                let primary_key = columns
                    .iter()
                    .filter(|c| c.primary_key)
                    .map(|c| c.name.clone())
                    .collect();
                Ok(TableSchema {
                    table: table.clone(),
                    columns,
                    primary_key,
                    foreign_keys,
                    inbound_foreign_keys,
                    editable: false,
                })
            }
            .await;
            Self::close_phantom_transaction(&mut conn, was_in_trans).await;
            outcome
        })
        .await
    }
    pub(super) async fn catalog_tables(&self) -> AppResult<Vec<TableInfo>> {
        let rows=self.metadata(|conn|Box::pin(fetch(conn,
            "SELECT TABLE_SCHEMA,TABLE_NAME,TABLE_TYPE,TABLE_ROWS,DATA_LENGTH+INDEX_LENGTH FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA=DATABASE() AND TABLE_TYPE<>'SEQUENCE' ORDER BY TABLE_NAME LIMIT 10001",
            vec![],10000,8*1024*1024))).await?;
        Ok(rows
            .into_iter()
            .map(|r| TableInfo {
                table: TableRef {
                    schema: cell(&r, 0).into(),
                    name: cell(&r, 1).into(),
                },
                kind: cell(&r, 2).into(),
                estimated_rows: r[3]
                    .as_ref()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0.0),
                total_bytes: r[4]
                    .as_ref()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0.0),
            })
            .collect())
    }
    pub(super) async fn table_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        identifier(&table.schema)?;
        identifier(&table.name)?;
        let table = table.clone();
        self.metadata(move |conn| Box::pin(async move { schema_on(conn, &table).await }))
            .await
    }
    pub(super) async fn referencing_foreign_keys(
        &self,
        table: &TableRef,
    ) -> AppResult<Vec<InboundForeignKey>> {
        identifier(&table.schema)?;
        identifier(&table.name)?;
        let table = table.clone();
        self.metadata(move |conn| Box::pin(async move { referencing_keys_on(conn, &table).await }))
            .await
    }
    pub(super) async fn relationship_diagram(&self, schema: &str) -> AppResult<SchemaDiagram> {
        identifier(schema)?;
        let schema = schema.to_owned();
        self.metadata(move |conn| Box::pin(async move { schema_diagram_on(conn, &schema).await }))
            .await
    }
    pub(super) async fn browse_rows(
        &self,
        schema: &TableSchema,
        query: BrowseQuery,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        let schema = schema.clone();
        let raw = query.has_raw_clauses();
        let submitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let submitted_query = submitted.clone();
        let outcome = self.metadata(move |conn| Box::pin(async move {
            let current = schema_on(conn, &schema.table).await?;
            unchanged(&schema, &current)?;
            let (sql, built) = browse_sql(&schema, &query, after.as_deref())?;
            let params = built.binds.iter().cloned().map(bind).collect::<Vec<_>>();
            submitted_query.store(true, std::sync::atomic::Ordering::Release);
            let mut query = conn.exec_iter(sql, Params::Positional(params)).await.map_err(mysql_error)?;
            let names = query.columns_ref().iter().map(|column| column.name_str().into_owned()).collect::<Vec<_>>();
            if names != schema.columns.iter().map(|column| column.name.clone()).collect::<Vec<_>>() {
                return Err(AppError::new(ErrorCode::Conflict, "The table schema changed. Refresh before browsing."));
            }
            let mut rows = Vec::new();
            let mut bytes = 0;
            let mut limited = false;
            let mut display_error = None;
            // The SQL itself is bounded to 501 rows. Retain a bounded prefix,
            // but always observe the terminal server response: approved raw
            // functions can fail (or commit effects) after the retained prefix.
            while let Some(row) = query.next().await.map_err(mysql_error)? {
                if limited { continue; }
                let row = match text_row(row) {
                    Ok(row) => row,
                    Err(error) => { display_error = Some(error); limited = true; continue; }
                };
                let size = row.iter().map(|value| value.as_ref().map_or(5, |value| value.len() + 5)).sum::<usize>();
                if rows.len() == 500 || bytes + size > 2 * 1024 * 1024 {
                    if rows.is_empty() {
                        display_error = Some(AppError::new(ErrorCode::ResultLimitReached, "A table row exceeds the 2 MiB page budget. Use a smaller projection."));
                    }
                    limited = true;
                    continue;
                }
                bytes += size;
                rows.push(row);
            }
            if let Some(error) = display_error { return Err(error); }
            let has_more = limited && built.pageable;
            let next_key = if has_more { rows.last().map(|row| built.cursor(row)) } else { None };
            Ok(BrowseRows { rows, next_key, has_more, truncated: limited && !has_more })
        })).await;
        if raw
            && submitted.load(std::sync::atomic::Ordering::Acquire)
            && outcome.as_ref().is_err_and(|error| {
                matches!(
                    error.code,
                    ErrorCode::ConnectionClosed | ErrorCode::ConnectionFailed
                )
            })
        {
            self.closed.cancel();
            self.guards.close();
            return Err(AppError::new(
                ErrorCode::OutcomeUnknown,
                "The table browse connection closed before its outcome could be observed. Verify any function effects before retrying this operation.",
            ));
        }
        outcome
    }
    pub(super) async fn search_rows(
        &self,
        request: DataSearchRequest,
    ) -> AppResult<DataSearchReport> {
        use crate::data_search::{
            MAX_SEARCH_REPORT_BYTES, MAX_SEARCH_ROWS_PER_TABLE, MAX_SEARCH_TOTAL_ROW_HITS,
            build_mysql_search_sql, build_table_match, estimate_table_match_bytes,
            filter_candidate_tables, validate_search_request,
        };
        let max_tables = validate_search_request(&request)?;
        let started = std::time::Instant::now();
        let all_tables: Vec<TableRef> = if !request.schemas.is_empty() || !request.tables.is_empty()
        {
            let rows = self
                    .metadata(|conn| {
                        Box::pin(fetch(
                            conn,
                            "SELECT TABLE_SCHEMA,TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA NOT IN ('information_schema','mysql','performance_schema','sys') AND TABLE_TYPE<>'SEQUENCE' ORDER BY TABLE_SCHEMA,TABLE_NAME LIMIT 10001",
                            vec![],
                            10_000,
                            4 * 1024 * 1024,
                        ))
                    })
                    .await?;
            rows.into_iter()
                .map(|r| TableRef {
                    schema: cell(&r, 0).into(),
                    name: cell(&r, 1).into(),
                })
                .collect()
        } else {
            self.catalog().await?.into_iter().map(|t| t.table).collect()
        };
        let (candidates, mut truncated) = filter_candidate_tables(all_tables, &request, max_tables);

        self.metadata(move |conn| {
            Box::pin(async move {
                let mut scanned_tables = 0usize;
                let mut skipped_tables = 0usize;
                let mut total_row_hits = 0usize;
                let mut report_bytes = 0usize;
                let mut matches = Vec::new();

                for table in &candidates {
                    if total_row_hits >= MAX_SEARCH_TOTAL_ROW_HITS
                        || report_bytes >= MAX_SEARCH_REPORT_BYTES
                    {
                        truncated = true;
                        break;
                    }
                    let schema = match schema_on(conn, table).await {
                        Ok(s) => s,
                        Err(err)
                            if matches!(
                                err.code,
                                ErrorCode::Database
                                    | ErrorCode::PermissionDenied
                                    | ErrorCode::Unsupported
                            ) =>
                        {
                            skipped_tables += 1;
                            continue;
                        }
                        Err(err) => return Err(err),
                    };
                    let Some((sql, raw_binds, searchable)) = build_mysql_search_sql(
                        &schema,
                        &request.query,
                        request.match_mode,
                        request.case_sensitive,
                        MAX_SEARCH_ROWS_PER_TABLE,
                    ) else {
                        skipped_tables += 1;
                        continue;
                    };
                    scanned_tables += 1;
                    let params = raw_binds
                        .into_iter()
                        .map(|s| bind(Some(s)))
                        .collect::<Vec<_>>();
                    let mut result_iter =
                        match conn.exec_iter(sql, Params::Positional(params)).await {
                            Ok(it) => it,
                            Err(err) => {
                                let mapped = mysql_error(err);
                                if matches!(
                                    mapped.code,
                                    ErrorCode::Database
                                        | ErrorCode::PermissionDenied
                                        | ErrorCode::Unsupported
                                ) {
                                    skipped_tables += 1;
                                    continue;
                                }
                                return Err(mapped);
                            }
                        };
                    let mut raw_rows = Vec::new();
                    let mut row_failed = false;
                    let mut fatal_err: Option<AppError> = None;
                    // Always drain all bounded rows (<= 11) so `conn` stays in clean idle state.
                    while let Some(row_res) = result_iter.next().await.transpose() {
                        if fatal_err.is_some() || row_failed {
                            if let Err(err) = row_res {
                                let mapped = mysql_error(err);
                                if !matches!(
                                    mapped.code,
                                    ErrorCode::Database
                                        | ErrorCode::PermissionDenied
                                        | ErrorCode::Unsupported
                                ) {
                                    fatal_err = Some(mapped);
                                }
                            }
                            continue;
                        }
                        match row_res.map_err(mysql_error).and_then(text_row) {
                            Ok(row) => {
                                if raw_rows.len() <= MAX_SEARCH_ROWS_PER_TABLE {
                                    raw_rows.push(row);
                                }
                            }
                            Err(err)
                                if matches!(
                                    err.code,
                                    ErrorCode::Database
                                        | ErrorCode::PermissionDenied
                                        | ErrorCode::Unsupported
                                ) =>
                            {
                                row_failed = true;
                            }
                            Err(err) => {
                                fatal_err = Some(err);
                            }
                        }
                    }
                    if let Some(err) = fatal_err {
                        return Err(err);
                    }
                    if row_failed {
                        skipped_tables += 1;
                        continue;
                    }
                    let allowed_rows = MAX_SEARCH_TOTAL_ROW_HITS
                        .saturating_sub(total_row_hits)
                        .min(MAX_SEARCH_ROWS_PER_TABLE);
                    if let Some(m) = build_table_match(
                        &schema,
                        searchable,
                        raw_rows,
                        &request.query,
                        request.match_mode,
                        request.case_sensitive,
                        allowed_rows,
                    ) {
                        let bytes = estimate_table_match_bytes(&m);
                        if report_bytes.saturating_add(bytes) > MAX_SEARCH_REPORT_BYTES
                            && !matches.is_empty()
                        {
                            truncated = true;
                            break;
                        }
                        report_bytes += bytes;
                        total_row_hits += m.rows.len();
                        if m.has_more_in_table && allowed_rows < MAX_SEARCH_ROWS_PER_TABLE {
                            truncated = true;
                        }
                        matches.push(m);
                    }
                }

                Ok(DataSearchReport {
                    query: request.query.clone(),
                    match_mode: request.match_mode,
                    case_sensitive: request.case_sensitive,
                    scanned_tables,
                    skipped_tables,
                    total_row_hits,
                    truncated,
                    elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
                    matches,
                })
            })
        })
        .await
    }
    pub(super) async fn apply_rows(
        &self,
        schema: &TableSchema,
        rows: &[RowMutation],
        guard: &guard::MysqlGuardHandle,
    ) -> AppResult<()> {
        if !schema.editable || self.read_only {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Only supported InnoDB tables with primary keys can be edited",
            ));
        }
        let outcome = self
            .bounded(async {
                if self.cancel_requested.is_cancelled() {
                    return Err(AppError::new(
                        ErrorCode::Cancelled,
                        "The edit was cancelled before submission",
                    ));
                }
                let conn = self.auxiliary().await?;
                let id = conn.id();
                let lease = Arc::new(LiveConnection {
                    connection: Mutex::new(conn),
                    id,
                });
                *self.edit.lock() = Some(lease.clone());
                let mut conn = lease.connection.lock().await;
                apply_on(&mut conn, schema, rows, guard, self).await
            })
            .await;
        self.edit.lock().take();
        match outcome {
            Err(e)
                if matches!(
                    e.code,
                    ErrorCode::ConnectionClosed | ErrorCode::ConnectionFailed
                ) =>
            {
                Err(AppError::new(
                    ErrorCode::OutcomeUnknown,
                    "The grid connection closed. Verify the database state before retrying these edits.",
                ))
            }
            other => other,
        }
    }
}

pub(crate) fn browse_sql(
    schema: &TableSchema,
    query: &BrowseQuery,
    after: Option<&[Option<String>]>,
) -> AppResult<(String, browse_query::Built)> {
    identifier(&schema.table.schema)?;
    identifier(&schema.table.name)?;
    let built = browse_query::build(schema, query, after, &MysqlDialect)?;
    let sql = format!(
        "SELECT {} FROM {}{}{} LIMIT 501",
        projection(schema),
        qualified(&schema.table),
        built.where_sql,
        built.order_sql
    );
    Ok((sql, built))
}

pub(super) fn unchanged(expected: &TableSchema, current: &TableSchema) -> AppResult<()> {
    // Read-only Safe Mode masks editability on the expected schema. Masking
    // may narrow, never widen, what the current table allows.
    let narrowed = |expected: bool, current: bool| !expected || current;
    let columns = expected.columns.len() == current.columns.len()
        && expected.columns.iter().zip(&current.columns).all(|(e, c)| {
            narrowed(e.editable, c.editable)
                && narrowed(e.insertable, c.insertable)
                && *e
                    == ColumnInfo {
                        editable: e.editable,
                        insertable: e.insertable,
                        ..c.clone()
                    }
        });
    if !columns
        || expected.primary_key != current.primary_key
        || !narrowed(expected.editable, current.editable)
        || serde_json::to_string(&expected.foreign_keys).ok()
            != serde_json::to_string(&current.foreign_keys).ok()
    {
        Err(AppError::new(
            ErrorCode::Conflict,
            "The table schema changed. Refresh before continuing.",
        ))
    } else {
        Ok(())
    }
}

fn unchanged_for_apply(expected: &TableSchema, current: &TableSchema) -> AppResult<()> {
    let narrowed = |expected: bool, current: bool| !expected || current;
    let columns = !expected.columns.is_empty()
        && expected.columns.iter().all(|e| {
            current.columns.iter().any(|c| {
                e.name.eq_ignore_ascii_case(&c.name)
                    && e.database_type == c.database_type
                    && e.kind == c.kind
                    && e.nullable == c.nullable
                    && e.primary_key == c.primary_key
                    && e.foreign_key == c.foreign_key
                    && e.has_default == c.has_default
                    && narrowed(e.editable, c.editable)
                    && narrowed(e.insertable, c.insertable)
            })
        });
    if !columns
        || expected.primary_key != current.primary_key
        || !narrowed(expected.editable, current.editable)
        || serde_json::to_string(&expected.foreign_keys).ok()
            != serde_json::to_string(&current.foreign_keys).ok()
    {
        Err(AppError::new(
            ErrorCode::Conflict,
            "The table schema changed. Refresh before continuing.",
        ))
    } else {
        Ok(())
    }
}

async fn referencing_keys_on(
    conn: &mut Conn,
    table: &TableRef,
) -> AppResult<Vec<InboundForeignKey>> {
    identifier(&table.schema)?;
    identifier(&table.name)?;
    let params = || {
        vec![
            bind(Some(table.schema.clone())),
            bind(Some(table.name.clone())),
        ]
    };
    let metadata = fetch(
        conn,
        "SELECT 1 FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA=? AND TABLE_NAME=?",
        params(),
        1,
        1024,
    )
    .await?;
    if metadata.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The table is missing or its metadata is unavailable",
        ));
    }
    let rows = fetch(
        conn,
        "SELECT CONSTRAINT_NAME, TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, REFERENCED_COLUMN_NAME
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE
         WHERE REFERENCED_TABLE_SCHEMA = ? AND REFERENCED_TABLE_NAME = ?
         ORDER BY TABLE_SCHEMA, TABLE_NAME, CONSTRAINT_NAME, ORDINAL_POSITION
         LIMIT 500",
        params(),
        500,
        2 * 1024 * 1024,
    )
    .await?;
    Ok(InboundForeignKey::group(rows.iter().map(|r| {
        (
            cell(r, 0).into(),
            TableRef {
                schema: cell(r, 1).into(),
                name: cell(r, 2).into(),
            },
            cell(r, 3).into(),
            cell(r, 4).into(),
        )
    })))
}

/// Reads relations, columns and outbound foreign keys for one database.
/// INFORMATION_SCHEMA already hides tables the account cannot access.
async fn schema_diagram_on(conn: &mut Conn, schema: &str) -> AppResult<SchemaDiagram> {
    let param = || vec![bind(Some(schema.to_owned()))];
    let visible = fetch(
        conn,
        "SELECT 1 FROM INFORMATION_SCHEMA.SCHEMATA WHERE SCHEMA_NAME=?",
        param(),
        1,
        1024,
    )
    .await?;
    if visible.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The database is missing or unavailable",
        ));
    }
    let tables = fetch(
        conn,
        "SELECT TABLE_NAME, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES
         WHERE TABLE_SCHEMA=? AND TABLE_TYPE<>'SEQUENCE'
         ORDER BY TABLE_NAME LIMIT 501",
        param(),
        MAX_DIAGRAM_TABLES + 1,
        1024 * 1024,
    )
    .await?;
    let columns = fetch(
        conn,
        "SELECT TABLE_NAME, COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY
         FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA=?
         ORDER BY TABLE_NAME, ORDINAL_POSITION LIMIT 20001",
        param(),
        MAX_DIAGRAM_COLUMNS + 1,
        8 * 1024 * 1024,
    )
    .await?;
    let keys = fetch(
        conn,
        "SELECT CONSTRAINT_NAME, TABLE_NAME, REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME,
                COLUMN_NAME, REFERENCED_COLUMN_NAME
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE
         WHERE TABLE_SCHEMA=? AND REFERENCED_TABLE_NAME IS NOT NULL
         ORDER BY TABLE_NAME, CONSTRAINT_NAME, ORDINAL_POSITION LIMIT 20001",
        param(),
        MAX_DIAGRAM_KEY_ROWS + 1,
        8 * 1024 * 1024,
    )
    .await?;
    Ok(SchemaDiagram::assemble(
        schema,
        tables
            .iter()
            .map(|r| (cell(r, 0).into(), cell(r, 1).into()))
            .collect(),
        columns
            .iter()
            .map(|r| {
                (
                    cell(r, 0).into(),
                    DiagramColumn {
                        name: cell(r, 1).into(),
                        database_type: cell(r, 2).into(),
                        nullable: cell(r, 3) == "YES",
                        primary_key: cell(r, 4) == "PRI",
                    },
                )
            })
            .collect(),
        keys.iter()
            .map(|r| DiagramKeyRow {
                name: cell(r, 0).into(),
                source_table: cell(r, 1).into(),
                target: TableRef {
                    schema: cell(r, 2).into(),
                    name: cell(r, 3).into(),
                },
                column: cell(r, 4).into(),
                target_column: cell(r, 5).into(),
                expected: None,
            })
            .collect(),
    ))
}

pub(super) async fn schema_on(conn: &mut Conn, table: &TableRef) -> AppResult<TableSchema> {
    identifier(&table.schema)?;
    identifier(&table.name)?;
    let params = || {
        vec![
            bind(Some(table.schema.clone())),
            bind(Some(table.name.clone())),
        ]
    };
    let metadata=fetch(conn,"SELECT TABLE_TYPE,COALESCE(ENGINE,''),TABLE_SCHEMA,TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA=? AND TABLE_NAME=? AND (@@lower_case_table_names<>0 OR (BINARY TABLE_SCHEMA=BINARY ? AND BINARY TABLE_NAME=BINARY ?))",[params(),params()].concat(),1,4096).await?;
    let table_info = metadata.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The table is missing or its metadata is unavailable",
        )
    })?;
    // Use server spelling for wire-origin comparisons and registry reuse.
    let canonical = TableRef {
        schema: cell(table_info, 2).into(),
        name: cell(table_info, 3).into(),
    };
    let table = &canonical;
    let params = || {
        vec![
            bind(Some(table.schema.clone())),
            bind(Some(table.name.clone())),
        ]
    };
    let transactional =
        cell(table_info, 0) == "BASE TABLE" && cell(table_info, 1).eq_ignore_ascii_case("InnoDB");
    let rows=fetch(conn,"SELECT COLUMN_NAME,COLUMN_TYPE,DATA_TYPE,IS_NULLABLE,EXTRA,COLLATION_NAME,CASE WHEN COLUMN_DEFAULT IS NOT NULL AND (IS_NULLABLE='NO' OR COLUMN_DEFAULT<>'NULL') THEN '1' ELSE '0' END FROM INFORMATION_SCHEMA.COLUMNS WHERE BINARY TABLE_SCHEMA=BINARY ? AND BINARY TABLE_NAME=BINARY ? ORDER BY ORDINAL_POSITION LIMIT 4097",params(),4096,2*1024*1024).await?;
    if rows.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "No column metadata is available",
        ));
    }
    let keys=fetch(conn,"SELECT COLUMN_NAME FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE WHERE BINARY TABLE_SCHEMA=BINARY ? AND BINARY TABLE_NAME=BINARY ? AND CONSTRAINT_NAME='PRIMARY' ORDER BY ORDINAL_POSITION",params(),4096,1024*1024).await?;
    let primary_key = keys
        .iter()
        .map(|r| cell(r, 0).to_owned())
        .collect::<Vec<_>>();
    let foreign=fetch(conn,"SELECT CONSTRAINT_NAME,COLUMN_NAME,REFERENCED_TABLE_SCHEMA,REFERENCED_TABLE_NAME,REFERENCED_COLUMN_NAME FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE WHERE BINARY TABLE_SCHEMA=BINARY ? AND BINARY TABLE_NAME=BINARY ? AND REFERENCED_TABLE_NAME IS NOT NULL ORDER BY CONSTRAINT_NAME,ORDINAL_POSITION LIMIT 4097",params(),4096,2*1024*1024).await?;
    let foreign_keys = ForeignKey::group(foreign.iter().map(|r| {
        (
            cell(r, 0).into(),
            cell(r, 1).into(),
            TableRef {
                schema: cell(r, 2).into(),
                name: cell(r, 3).into(),
            },
            cell(r, 4).into(),
        )
    }));
    let mut columns =
        rows.iter()
            .enumerate()
            .map(|(i, r)| {
                let name = cell(r, 0);
                let typ = cell(r, 2).to_ascii_lowercase();
                let extra = cell(r, 4).to_ascii_lowercase();
                let generated =
                    extra.contains("virtual generated") || extra.contains("stored generated");
                let auto_increment = extra.contains("auto_increment");
                let kind = match typ.as_str() {
                    "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint"
                    | "year" => ColumnKind::Integer,
                    "decimal" | "numeric" | "float" | "double" | "real" => ColumnKind::Decimal,
                    "date" | "datetime" | "timestamp" | "time" => ColumnKind::Temporal,
                    "json" => ColumnKind::Json,
                    "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob"
                    | "bit" | "geometry" => ColumnKind::Binary,
                    "char" | "varchar" | "tinytext" | "text" | "mediumtext" | "longtext"
                    | "enum" | "set" => ColumnKind::Text,
                    _ => ColumnKind::Unknown,
                };
                let mut c = ColumnInfo {
                    id: format!("c{i}"),
                    name: name.into(),
                    database_type: if cell(r, 5).is_empty() {
                        cell(r, 1).into()
                    } else {
                        format!("{} COLLATE {}", cell(r, 1), cell(r, 5))
                    },
                    kind,
                    nullable: cell(r, 3) == "YES",
                    primary_key: primary_key.iter().any(|k| k == name),
                    foreign_key: foreign_keys
                        .iter()
                        .any(|f| f.columns.iter().any(|c| c == name)),
                    editable: false,
                    insertable: false,
                    has_default: cell(r, 6) == "1" || auto_increment || generated,
                };
                let castable = cast(&c).is_some();
                c.editable =
                    transactional && !c.primary_key && !generated && !auto_increment && castable;
                c.insertable = transactional && !generated && castable;
                c
            })
            .collect::<Vec<_>>();
    let editable = transactional
        && !primary_key.is_empty()
        && columns
            .iter()
            .filter(|c| c.primary_key)
            .all(|c| cast(c).is_some() && !c.nullable);
    if !editable {
        for c in &mut columns {
            c.editable = false;
            c.insertable = false;
        }
    }
    Ok(TableSchema {
        table: table.clone(),
        columns,
        primary_key,
        foreign_keys,
        inbound_foreign_keys: Vec::new(),
        editable,
    })
}

fn hex_utf8_literal(text: &str) -> String {
    let mut hex = String::with_capacity(text.len() * 2);
    for byte in text.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    format!("CONVERT(UNHEX('{hex}') USING utf8mb4)")
}

async fn completion_fks_on(
    conn: &mut Conn,
    table: &TableRef,
    qualified_table: &str,
    columns: &[ColumnInfo],
) -> (Vec<ForeignKey>, Vec<InboundForeignKey>) {
    // Detect session-local TEMPORARY TABLE shadows; views without SHOW VIEW or
    // temporary tables never expose permanent-table INFORMATION_SCHEMA FKs.
    let Ok(create_rows) = fetch_text(
        conn,
        &format!("SHOW CREATE TABLE {qualified_table}"),
        4,
        1024 * 1024,
    )
    .await
    else {
        return (Vec::new(), Vec::new());
    };
    let Some(first) = create_rows.first() else {
        return (Vec::new(), Vec::new());
    };
    let ddl = cell(first, 1).trim_start().to_ascii_uppercase();
    if ddl.starts_with("CREATE TEMPORARY TABLE ")
        || ddl.starts_with("CREATE OR REPLACE TEMPORARY TABLE ")
        || !ddl.starts_with("CREATE TABLE ")
    {
        return (Vec::new(), Vec::new());
    }

    let selectable: std::collections::HashSet<&str> =
        columns.iter().map(|c| c.name.as_str()).collect();
    let schema_lit = hex_utf8_literal(&table.schema);
    let table_lit = hex_utf8_literal(&table.name);

    let outbound_sql = format!(
        "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME,
                EXISTS(
                  SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS pc
                  WHERE pc.TABLE_SCHEMA = k.REFERENCED_TABLE_SCHEMA
                    AND pc.TABLE_NAME = k.REFERENCED_TABLE_NAME
                    AND pc.COLUMN_NAME = k.REFERENCED_COLUMN_NAME
                    AND FIND_IN_SET('select', LOWER(pc.PRIVILEGES)) > 0
                )
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE k
         WHERE k.TABLE_SCHEMA = {schema_lit}
           AND k.TABLE_NAME = {table_lit}
           AND k.REFERENCED_TABLE_NAME IS NOT NULL
         ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION
         LIMIT 500"
    );
    let outbound = if let Ok(rows) = fetch_text(conn, &outbound_sql, 500, 2 * 1024 * 1024).await {
        let mut denied: std::collections::HashSet<(String, String, String)> =
            std::collections::HashSet::new();
        let mut grouped = ForeignKey::group(rows.iter().map(|r| {
            let name = cell(r, 0).to_owned();
            let col = cell(r, 1).to_owned();
            let target = TableRef {
                schema: cell(r, 2).to_owned(),
                name: cell(r, 3).to_owned(),
            };
            let target_col = cell(r, 4).to_owned();
            if !selectable.contains(col.as_str()) || cell(r, 5) != "1" {
                denied.insert((target.schema.clone(), target.name.clone(), name.clone()));
            }
            (name, col, target, target_col)
        }));
        if rows.len() == 500 {
            grouped.pop();
        }
        grouped
            .into_iter()
            .filter(|k| {
                !denied.contains(&(
                    k.target.schema.clone(),
                    k.target.name.clone(),
                    k.name.clone(),
                ))
            })
            .collect()
    } else {
        Vec::new()
    };

    let inbound_sql = format!(
        "SELECT k.CONSTRAINT_NAME, k.TABLE_SCHEMA, k.TABLE_NAME, k.COLUMN_NAME, k.REFERENCED_COLUMN_NAME,
                EXISTS(
                  SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS pc
                  WHERE pc.TABLE_SCHEMA = k.TABLE_SCHEMA
                    AND pc.TABLE_NAME = k.TABLE_NAME
                    AND pc.COLUMN_NAME = k.COLUMN_NAME
                    AND FIND_IN_SET('select', LOWER(pc.PRIVILEGES)) > 0
                )
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE k
         WHERE k.REFERENCED_TABLE_SCHEMA = {schema_lit}
           AND k.REFERENCED_TABLE_NAME = {table_lit}
         ORDER BY k.TABLE_SCHEMA, k.TABLE_NAME, k.CONSTRAINT_NAME, k.ORDINAL_POSITION
         LIMIT 500"
    );
    let inbound = if let Ok(rows) = fetch_text(conn, &inbound_sql, 500, 2 * 1024 * 1024).await {
        let mut denied: std::collections::HashSet<(String, String, String)> =
            std::collections::HashSet::new();
        let mut grouped = InboundForeignKey::group(rows.iter().map(|r| {
            let name = cell(r, 0).to_owned();
            let source = TableRef {
                schema: cell(r, 1).to_owned(),
                name: cell(r, 2).to_owned(),
            };
            let col = cell(r, 3).to_owned();
            let target_col = cell(r, 4).to_owned();
            if !selectable.contains(target_col.as_str()) || cell(r, 5) != "1" {
                denied.insert((source.schema.clone(), source.name.clone(), name.clone()));
            }
            (name, source, col, target_col)
        }));
        if rows.len() == 500 {
            grouped.pop();
        }
        grouped
            .into_iter()
            .filter(|k| {
                !denied.contains(&(
                    k.source.schema.clone(),
                    k.source.name.clone(),
                    k.name.clone(),
                ))
            })
            .collect()
    } else {
        Vec::new()
    };

    (outbound, inbound)
}

async fn apply_on(
    conn: &mut Connection,
    schema: &TableSchema,
    rows: &[RowMutation],
    guard: &guard::MysqlGuardHandle,
    driver: &Mysql,
) -> AppResult<()> {
    let cancel = &driver.cancel_requested;
    if cancel.is_cancelled() {
        return Err(AppError::new(
            ErrorCode::Cancelled,
            "The edit was cancelled before submission",
        ));
    }
    let lock_timeout = driver.guards.policy().writer_lock_timeout;
    // Row-lock timeouts do not bound metadata locks. This disposable writer
    // needs both the server MDL timeout and an independent client-side bound.
    conn.query_drop(format!(
        "SET SESSION lock_wait_timeout={}",
        lock_timeout.as_secs().max(1)
    ))
    .await
    .map_err(mysql_error)?;
    conn.query_drop("START TRANSACTION")
        .await
        .map_err(mysql_error)?;
    let mut interrupted_lock = false;
    let outcome = async {
        // Acquire the table metadata lock before rechecking engine and schema;
        // concurrent ALTER TABLE cannot switch the engine during this commit.
        let lock = conn.query_drop(format!(
            "SELECT {} FROM {} WHERE 1=0 FOR UPDATE",
            projection(schema),
            qualified(&schema.table)
        ));
        match tokio::time::timeout(lock_timeout + Duration::from_secs(1), lock).await {
            Ok(result) => result.map_err(mysql_error)?,
            Err(_) => {
                interrupted_lock = true;
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "The table is busy. Refresh before retrying these edits.",
                ));
            }
        }
        // The writer owns MDL now. Fresh I/O must prove this exact retained
        // generation still protects the original table before any DML.
        guard
            .verify(&driver.session_id, &schema.table, cancel, &driver.closed)
            .await?;
        unchanged_for_apply(schema, &schema_on(conn, &schema.table).await?)?;
        for row in rows {
            if cancel.is_cancelled() {
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "The edits were cancelled and rolled back",
                ));
            }
            if let RowMutation::Update { original, .. } | RowMutation::Delete { original } = row {
                let (predicate, keys) = key_predicate(schema, original)?;
                let locked = fetch(
                    conn,
                    &format!(
                        "SELECT {} FROM {} WHERE {predicate} LIMIT 2 FOR UPDATE",
                        projection(schema),
                        qualified(&schema.table)
                    ),
                    keys.into_iter().map(bind).collect(),
                    2,
                    2 * 1024 * 1024,
                )
                .await?;
                if locked.len() != 1 || &locked[0] != original {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "A target row changed or was deleted. No edits were committed.",
                    ));
                }
            }
            let (sql, values) = mutation_sql(schema, row)?;
            conn.exec_drop(
                sql,
                Params::Positional(values.into_iter().map(bind).collect()),
            )
            .await
            .map_err(mysql_error)?;
            if conn.affected_rows() != 1 {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "A table change did not affect exactly one row. No edits were committed.",
                ));
            }
            if conn.get_warnings() != 0 {
                return Err(AppError::invalid(
                    "A value required a lossy conversion. No edits were committed.",
                ));
            }
        }
        if cancel.is_cancelled() {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "The edits were cancelled and rolled back",
            ));
        }
        Ok(())
    }
    .await;
    if let Err(error) = outcome {
        if interrupted_lock {
            if let Some(conn) = conn.0.take() {
                let _ =
                    tokio::time::timeout(driver.guards.policy().close_timeout, conn.disconnect())
                        .await;
            }
            return Err(error);
        }
        let _ = conn.query_drop("ROLLBACK").await;
        return Err(error);
    }
    match conn.query_drop("COMMIT").await {
        Ok(()) => Ok(()),
        Err(mysql_async::Error::Server(error)) => {
            let _ = conn.query_drop("ROLLBACK").await;
            Err(mysql_error(mysql_async::Error::Server(error)))
        }
        Err(_) => Err(AppError::new(
            ErrorCode::OutcomeUnknown,
            "The connection was lost while committing. Verify the database before retrying these edits.",
        )),
    }
}
