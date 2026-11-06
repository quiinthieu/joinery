use super::*;

struct MetadataResult {
    columns: Vec<ColumnInfo>,
    rows: Vec<TextRow>,
    truncated: bool,
}

pub(crate) fn identifier(value: &str) -> AppResult<String> {
    if value.is_empty() || value.chars().count() > 255 || value.contains('\0') {
        return Err(AppError::invalid("Invalid Snowflake object identifier"));
    }
    Ok(format!("\"{}\"", value.replace('"', "\"\"")))
}

fn limit_error() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Snowflake metadata exceeds the browsing limit. Use a query with a narrower projection or filter.",
    )
}

impl Snowflake {
    pub(super) async fn context_rows(&self, sql: &str) -> AppResult<Vec<TextRow>> {
        Ok(self.metadata_result(sql, 1, 64 * 1024, false).await?.rows)
    }
    pub(super) async fn query_rows(&self, sql: &str, max_rows: usize) -> AppResult<Vec<TextRow>> {
        Ok(self
            .metadata_result(sql, max_rows, 8 * 1024 * 1024, false)
            .await?
            .rows)
    }
    pub(crate) fn qualified(&self, table: &TableRef) -> AppResult<String> {
        Ok(format!(
            "{}.{}.{}",
            identifier(&self.current_database())?,
            identifier(&table.schema)?,
            identifier(&table.name)?
        ))
    }

    // Metadata has a separate lane and a local statement handle. It must never
    // replace the editor run's handle or inherit a previous run's cancellation.
    async fn metadata_result(
        &self,
        sql: &str,
        max_rows: usize,
        max_bytes: usize,
        preview: bool,
    ) -> AppResult<MetadataResult> {
        let _lane = self.metadata_lane.lock().await;
        let first = self.submit(sql, false).await?;
        let description = columns(&first)?;
        let partitions = first
            .pointer("/resultSetMetaData/partitionInfo")
            .and_then(Value::as_array)
            .map_or(1, |p| p.len().max(1));
        let expected_rows = first
            .pointer("/resultSetMetaData/numRows")
            .and_then(Value::as_u64);
        if partitions > MAX_PARTITIONS
            || (!preview && expected_rows.is_some_and(|n| n > max_rows as u64))
        {
            return Err(limit_error());
        }
        let handle = first
            .get("statementHandle")
            .and_then(Value::as_str)
            .map(uuid::Uuid::parse_str)
            .transpose()
            .map_err(|_| AppError::new(ErrorCode::Database, "Invalid Snowflake metadata handle"))?;
        let mut result = MetadataResult {
            columns: description,
            rows: Vec::new(),
            truncated: false,
        };
        let mut bytes = 0usize;
        for index in 0..partitions {
            let partition;
            let value = if index == 0 {
                &first
            } else {
                let handle = handle.ok_or_else(|| {
                    AppError::new(ErrorCode::Database, "Missing Snowflake metadata handle")
                })?;
                partition = self
                    .request(
                        Method::GET,
                        &format!("api/v2/statements/{handle}?partition={index}"),
                        None,
                    )
                    .await?
                    .1;
                &partition
            };
            for row in decode_rows(value)? {
                if !result.columns.is_empty() && row.len() != result.columns.len() {
                    return Err(AppError::new(
                        ErrorCode::Database,
                        "Snowflake metadata column count changed",
                    ));
                }
                let size = std::mem::size_of::<TextRow>()
                    + row
                        .iter()
                        .map(|v| {
                            std::mem::size_of::<Option<String>>()
                                + v.as_ref().map_or(0, String::len)
                        })
                        .sum::<usize>();
                if result.rows.len() == max_rows || bytes.saturating_add(size) > max_bytes {
                    if !preview || result.rows.is_empty() {
                        return Err(limit_error());
                    }
                    result.truncated = true;
                    return Ok(result);
                }
                bytes += size;
                result.rows.push(row);
            }
        }
        if expected_rows.is_some_and(|n| n != result.rows.len() as u64) {
            return Err(AppError::new(
                ErrorCode::Database,
                "Snowflake returned incomplete metadata. Refresh and try again.",
            ));
        }
        Ok(result)
    }

    async fn list_show_names(&self, sql: &str) -> AppResult<Vec<String>> {
        let result = self
            .metadata_result(sql, 1000, 2 * 1024 * 1024, false)
            .await?;
        let name_idx = result
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case("name"))
            .unwrap_or(0);
        Ok(result
            .rows
            .into_iter()
            .filter_map(|row| row.get(name_idx).cloned().flatten())
            .filter(|name| !name.is_empty())
            .collect())
    }

    pub(super) async fn list_warehouses(&self) -> AppResult<Vec<String>> {
        self.list_show_names("SHOW WAREHOUSES").await
    }

    pub(super) async fn list_roles(&self) -> AppResult<Vec<String>> {
        self.list_show_names("SHOW ROLES").await
    }

    pub(super) async fn list_databases(&self) -> AppResult<Vec<String>> {
        self.list_show_names("SHOW DATABASES").await
    }

    pub(super) async fn list_schemas_in_database(&self, database: &str) -> AppResult<Vec<String>> {
        let sql = format!(
            "SELECT SCHEMA_NAME FROM {}.INFORMATION_SCHEMA.SCHEMATA WHERE SCHEMA_NAME <> 'INFORMATION_SCHEMA' ORDER BY SCHEMA_NAME LIMIT 1001",
            identifier(database)?
        );
        if let Ok(rows) = self.query_rows(&sql, 1000).await {
            let schemas: Vec<String> = rows
                .into_iter()
                .filter_map(|row| row.into_iter().next().flatten())
                .filter(|name| simple_identifier(name))
                .collect();
            if !schemas.is_empty() {
                return Ok(schemas);
            }
        }
        self.list_show_names(&format!(
            "SHOW SCHEMAS IN DATABASE {}",
            identifier(database)?
        ))
        .await
    }

    pub(super) async fn list_schemas(&self) -> AppResult<Vec<String>> {
        let current_db = self.current_database();
        self.list_schemas_in_database(&current_db).await
    }

    fn parse_table_rows(rows: Vec<TextRow>) -> AppResult<Vec<TableInfo>> {
        rows.into_iter()
            .map(|row| {
                if row.len() != 5 {
                    return Err(AppError::new(
                        ErrorCode::Database,
                        "Snowflake returned malformed table metadata",
                    ));
                }
                let required = |index: usize| {
                    row[index].clone().filter(|s| !s.is_empty()).ok_or_else(|| {
                        AppError::new(
                            ErrorCode::Database,
                            "Snowflake returned incomplete table metadata",
                        )
                    })
                };
                let estimate = |index: usize| {
                    row[index]
                        .as_ref()
                        .and_then(|v| v.parse::<f64>().ok())
                        .filter(|n| n.is_finite() && *n >= 0.0)
                };
                Ok(TableInfo {
                    table: TableRef {
                        schema: required(0)?,
                        name: required(1)?,
                    },
                    kind: row[2]
                        .clone()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "BASE TABLE".into()),
                    estimated_rows: estimate(3),
                    total_bytes: estimate(4),
                })
            })
            .collect()
    }

    pub(super) async fn catalog_tables_in_schema(&self, schema: &str) -> AppResult<Vec<TableInfo>> {
        if !simple_identifier(schema) {
            return Err(AppError::invalid("Invalid Snowflake schema identifier"));
        }
        let sql = format!(
            "SELECT TABLE_SCHEMA,TABLE_NAME,TABLE_TYPE,ROW_COUNT,BYTES FROM {}.INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = '{}' ORDER BY TABLE_SCHEMA,TABLE_NAME LIMIT 10001",
            identifier(&self.current_database())?,
            schema
        );
        let rows = self.query_rows(&sql, 10_000).await?;
        Self::parse_table_rows(rows)
    }

    pub(super) async fn catalog_tables(&self) -> AppResult<Vec<TableInfo>> {
        let sql = format!(
            "SELECT TABLE_SCHEMA,TABLE_NAME,TABLE_TYPE,ROW_COUNT,BYTES FROM {}.INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY TABLE_SCHEMA,TABLE_NAME LIMIT 10001",
            identifier(&self.current_database())?
        );
        match self.query_rows(&sql, 10_000).await {
            Ok(rows) => Self::parse_table_rows(rows),
            Err(err) if err.message.to_ascii_lowercase().contains("too much data") => {
                let schema = self.current_schema();
                self.catalog_tables_in_schema(&schema).await
            }
            Err(err) => Err(err),
        }
    }

    pub(super) async fn table_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        let qualified = self.qualified(table)?;
        let sql = format!("SELECT * FROM {qualified} LIMIT 0");
        let result = self.metadata_result(&sql, 0, 0, false).await?;
        if result.columns.is_empty() {
            return Err(AppError::new(
                ErrorCode::NotFound,
                "Snowflake table columns are unavailable",
            ));
        }

        // 1. Discover Primary Keys via SHOW PRIMARY KEYS
        let pk_sql = format!("SHOW PRIMARY KEYS IN TABLE {qualified}");
        let mut primary_key = Vec::new();
        let mut declared_pk = false;

        if let Ok(pk_meta) = self.metadata_result(&pk_sql, 100, 1024 * 1024, false).await {
            let col_idx = pk_meta
                .columns
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case("column_name"))
                .unwrap_or(4);
            let seq_idx = pk_meta
                .columns
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case("key_sequence"))
                .unwrap_or(5);
            let mut key_cols: Vec<(usize, String)> = pk_meta
                .rows
                .into_iter()
                .filter_map(|row| {
                    let col = row.get(col_idx)?.clone()?;
                    let seq = row
                        .get(seq_idx)?
                        .as_ref()?
                        .parse::<usize>()
                        .ok()
                        .unwrap_or(1);
                    Some((seq, col))
                })
                .collect();
            key_cols.sort_by_key(|(seq, _)| *seq);
            primary_key = key_cols.into_iter().map(|(_, name)| name).collect();
            if !primary_key.is_empty() {
                declared_pk = true;
            }
        }

        // 2. Fallback to SHOW UNIQUE KEYS
        if primary_key.is_empty() {
            let uq_sql = format!("SHOW UNIQUE KEYS IN TABLE {qualified}");
            if let Ok(uq_meta) = self.metadata_result(&uq_sql, 100, 1024 * 1024, false).await {
                let col_idx = uq_meta
                    .columns
                    .iter()
                    .position(|c| c.name.eq_ignore_ascii_case("column_name"))
                    .unwrap_or(4);
                let seq_idx = uq_meta
                    .columns
                    .iter()
                    .position(|c| c.name.eq_ignore_ascii_case("key_sequence"))
                    .unwrap_or(5);
                let mut key_cols: Vec<(usize, String)> = uq_meta
                    .rows
                    .into_iter()
                    .filter_map(|row| {
                        let col = row.get(col_idx)?.clone()?;
                        let seq = row
                            .get(seq_idx)?
                            .as_ref()?
                            .parse::<usize>()
                            .ok()
                            .unwrap_or(1);
                        Some((seq, col))
                    })
                    .collect();
                key_cols.sort_by_key(|(seq, _)| *seq);
                primary_key = key_cols.into_iter().map(|(_, name)| name).collect();
                if !primary_key.is_empty() {
                    declared_pk = true;
                }
            }
        }

        // 3. Fallback to Inferred Non-Variant Columns
        if primary_key.is_empty() {
            primary_key = result
                .columns
                .iter()
                .filter(|c| {
                    c.kind != ColumnKind::Json
                        && c.kind != ColumnKind::Binary
                        && c.kind != ColumnKind::Unknown
                })
                .map(|c| c.name.clone())
                .collect();
        }

        // 4. Update Column editability and primary_key status
        let mut columns = result.columns;
        let editable_table = !primary_key.is_empty();

        for col in &mut columns {
            let is_pk = declared_pk
                && primary_key
                    .iter()
                    .any(|k| k.eq_ignore_ascii_case(&col.name));
            col.primary_key = is_pk;
            col.editable = editable_table && !is_pk && col.kind != ColumnKind::Unknown;
            col.insertable = editable_table && col.kind != ColumnKind::Unknown;
        }

        Ok(TableSchema {
            table: table.clone(),
            columns,
            primary_key,
            foreign_keys: Vec::new(),
            inbound_foreign_keys: Vec::new(),
            editable: editable_table,
        })
    }

    pub(super) async fn table_preview(
        &self,
        schema: &TableSchema,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        if after.is_some() {
            return Err(AppError::invalid(
                "Snowflake previews do not support continuation. Use SQL for a larger or ordered result.",
            ));
        }
        if schema.columns.is_empty() {
            return Err(AppError::invalid("Snowflake table columns are unavailable"));
        }
        let projection = schema
            .columns
            .iter()
            .map(|c| identifier(&c.name))
            .collect::<AppResult<Vec<_>>>()?
            .join(",");
        let sql = format!(
            "SELECT {projection} FROM {} LIMIT 501",
            self.qualified(&schema.table)?
        );
        let result = self
            .metadata_result(&sql, 500, 2 * 1024 * 1024, true)
            .await?;
        let columns_changed = result.columns.len() != schema.columns.len()
            || result
                .columns
                .iter()
                .zip(&schema.columns)
                .any(|(actual, expected)| {
                    actual.name != expected.name
                        || actual.database_type != expected.database_type
                        || actual.kind != expected.kind
                        || actual.nullable != expected.nullable
                });
        if columns_changed {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The table schema changed. Refresh the table before continuing.",
            ));
        }
        Ok(BrowseRows {
            rows: result.rows,
            next_key: None,
            has_more: false,
            truncated: result.truncated,
        })
    }

    pub(super) async fn table_browse_query(
        &self,
        schema: &TableSchema,
        query: &BrowseQuery,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        let (sql, built) = browse_sql(schema, query, after.as_deref())?;
        let bindings = if built.binds.is_empty() {
            None
        } else {
            Some(super::snowflake_browse_bindings_json(&built.binds))
        };
        let _lane = self.metadata_lane.lock().await;
        let schema_name = self.current_schema();
        let first = self
            .submit_with_schema_and_bindings(&sql, &schema_name, bindings, false)
            .await?;
        let rows = decode_rows(&first)?;
        let has_more = rows.len() > 500;
        let mut rows = rows;
        if has_more {
            rows.truncate(500);
        }
        let next_key = if has_more && built.pageable {
            rows.last().map(|row| built.cursor(row))
        } else {
            None
        };
        Ok(BrowseRows {
            rows,
            next_key,
            has_more,
            truncated: has_more,
        })
    }
}

pub(crate) struct SnowflakeDialect<'a>(pub &'a TableRef);

impl super::super::browse_query::Dialect for SnowflakeDialect<'_> {
    fn engine(&self) -> EngineKind {
        EngineKind::Snowflake
    }
    fn column(&self, name: &str) -> String {
        format!(
            "{}.{}.{}",
            identifier(&self.0.schema).unwrap_or_default(),
            identifier(&self.0.name).unwrap_or_default(),
            identifier(name).unwrap_or_default()
        )
    }
    fn typed_bind(&self, column: &ColumnInfo, _index: usize) -> Option<String> {
        match column.kind {
            ColumnKind::Text => Some("?".into()),
            ColumnKind::Integer | ColumnKind::Decimal => Some("CAST(? AS NUMBER)".into()),
            ColumnKind::Boolean => Some("CAST(? AS BOOLEAN)".into()),
            ColumnKind::Temporal => Some(format!("CAST(? AS {})", column.database_type)),
            ColumnKind::Binary | ColumnKind::Json | ColumnKind::Unknown => None,
        }
    }
    fn like(&self, column: &ColumnInfo, _index: usize) -> String {
        format!(
            "{} ILIKE ?{}",
            self.column(&column.name),
            super::super::browse_query::like_escape_clause()
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
) -> AppResult<(String, super::super::browse_query::Built)> {
    let dialect = SnowflakeDialect(&schema.table);
    let built = super::super::browse_query::build(schema, query, after, &dialect)?;
    let qualified_table = format!(
        "{}.{}",
        identifier(&schema.table.schema)?,
        identifier(&schema.table.name)?
    );
    let projection = schema
        .columns
        .iter()
        .map(|c| identifier(&c.name))
        .collect::<AppResult<Vec<_>>>()?
        .join(", ");
    let sql = format!(
        "SELECT {projection} FROM {qualified_table}{}{} LIMIT 501",
        built.where_sql, built.order_sql
    );
    Ok((sql, built))
}

impl Snowflake {
    pub(super) async fn catalog_objects(&self) -> AppResult<Vec<DatabaseObjectSummary>> {
        let db = self.current_database();
        let db_id = identifier(&db)?;
        let mut objects = Vec::new();

        // 1. Stages
        let stage_sql = format!(
            "SELECT STAGE_SCHEMA, STAGE_NAME, STAGE_TYPE, COMMENT FROM {db_id}.INFORMATION_SCHEMA.STAGES WHERE STAGE_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY STAGE_SCHEMA, STAGE_NAME LIMIT 2001"
        );
        if let Ok(rows) = self.query_rows(&stage_sql, 2000).await {
            for r in rows {
                if let (Some(schema), Some(name)) =
                    (r.first().cloned().flatten(), r.get(1).cloned().flatten())
                {
                    let stage_type = r.get(2).cloned().flatten();
                    objects.push(DatabaseObjectSummary {
                        object: DatabaseObjectRef {
                            schema,
                            name,
                            kind: DatabaseObjectKind::Stage,
                            identity: None,
                        },
                        owner_table: None,
                        signature: stage_type,
                        estimated_rows: None,
                        total_bytes: None,
                    });
                }
            }
        }

        // 2. Tasks
        let task_sql = format!(
            "SELECT TASK_SCHEMA, TASK_NAME, SCHEDULE, STATE, COMMENT FROM {db_id}.INFORMATION_SCHEMA.TASKS WHERE TASK_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY TASK_SCHEMA, TASK_NAME LIMIT 2001"
        );
        if let Ok(rows) = self.query_rows(&task_sql, 2000).await {
            for r in rows {
                if let (Some(schema), Some(name)) =
                    (r.first().cloned().flatten(), r.get(1).cloned().flatten())
                {
                    let schedule = r.get(2).cloned().flatten();
                    objects.push(DatabaseObjectSummary {
                        object: DatabaseObjectRef {
                            schema,
                            name,
                            kind: DatabaseObjectKind::Task,
                            identity: None,
                        },
                        owner_table: None,
                        signature: schedule.or_else(|| Some("()".into())),
                        estimated_rows: None,
                        total_bytes: None,
                    });
                }
            }
        }

        // 3. Streams
        let stream_sql = format!(
            "SELECT STREAM_SCHEMA, STREAM_NAME, TABLE_NAME, MODE, COMMENT FROM {db_id}.INFORMATION_SCHEMA.STREAMS WHERE STREAM_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY STREAM_SCHEMA, STREAM_NAME LIMIT 2001"
        );
        if let Ok(rows) = self.query_rows(&stream_sql, 2000).await {
            for r in rows {
                if let (Some(schema), Some(name)) =
                    (r.first().cloned().flatten(), r.get(1).cloned().flatten())
                {
                    let table_name = r.get(2).cloned().flatten();
                    let mode = r.get(3).cloned().flatten();
                    objects.push(DatabaseObjectSummary {
                        object: DatabaseObjectRef {
                            schema,
                            name,
                            kind: DatabaseObjectKind::Stream,
                            identity: None,
                        },
                        owner_table: table_name,
                        signature: mode,
                        estimated_rows: None,
                        total_bytes: None,
                    });
                }
            }
        }

        // 4. Pipes
        let pipe_sql = format!(
            "SELECT PIPE_SCHEMA, PIPE_NAME, COMMENT FROM {db_id}.INFORMATION_SCHEMA.PIPES WHERE PIPE_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY PIPE_SCHEMA, PIPE_NAME LIMIT 2001"
        );
        if let Ok(rows) = self.query_rows(&pipe_sql, 2000).await {
            for r in rows {
                if let (Some(schema), Some(name)) =
                    (r.first().cloned().flatten(), r.get(1).cloned().flatten())
                {
                    objects.push(DatabaseObjectSummary {
                        object: DatabaseObjectRef {
                            schema,
                            name,
                            kind: DatabaseObjectKind::Pipe,
                            identity: None,
                        },
                        owner_table: None,
                        signature: None,
                        estimated_rows: None,
                        total_bytes: None,
                    });
                }
            }
        }

        // 5. Procedures
        let proc_sql = format!(
            "SELECT PROCEDURE_SCHEMA, PROCEDURE_NAME, ARGUMENT_SIGNATURE, DATA_TYPE, COMMENT FROM {db_id}.INFORMATION_SCHEMA.PROCEDURES WHERE PROCEDURE_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY PROCEDURE_SCHEMA, PROCEDURE_NAME LIMIT 2001"
        );
        if let Ok(rows) = self.query_rows(&proc_sql, 2000).await {
            for r in rows {
                if let (Some(schema), Some(name)) =
                    (r.first().cloned().flatten(), r.get(1).cloned().flatten())
                {
                    let sig = r.get(2).cloned().flatten();
                    let ret = r.get(3).cloned().flatten();
                    let display_sig = match (&sig, &ret) {
                        (Some(s), Some(rt)) => Some(format!("{s} -> {rt}")),
                        (Some(s), None) => Some(s.clone()),
                        _ => None,
                    };
                    objects.push(DatabaseObjectSummary {
                        object: DatabaseObjectRef {
                            schema,
                            name,
                            kind: DatabaseObjectKind::Procedure,
                            identity: sig.clone(),
                        },
                        owner_table: None,
                        signature: display_sig,
                        estimated_rows: None,
                        total_bytes: None,
                    });
                }
            }
        }

        // 6. Functions
        let func_sql = format!(
            "SELECT FUNCTION_SCHEMA, FUNCTION_NAME, ARGUMENT_SIGNATURE, DATA_TYPE, COMMENT FROM {db_id}.INFORMATION_SCHEMA.FUNCTIONS WHERE FUNCTION_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY FUNCTION_SCHEMA, FUNCTION_NAME LIMIT 2001"
        );
        if let Ok(rows) = self.query_rows(&func_sql, 2000).await {
            for r in rows {
                if let (Some(schema), Some(name)) =
                    (r.first().cloned().flatten(), r.get(1).cloned().flatten())
                {
                    let sig = r.get(2).cloned().flatten();
                    let ret = r.get(3).cloned().flatten();
                    let display_sig = match (&sig, &ret) {
                        (Some(s), Some(rt)) => Some(format!("{s} -> {rt}")),
                        (Some(s), None) => Some(s.clone()),
                        _ => None,
                    };
                    objects.push(DatabaseObjectSummary {
                        object: DatabaseObjectRef {
                            schema,
                            name,
                            kind: DatabaseObjectKind::Function,
                            identity: sig.clone(),
                        },
                        owner_table: None,
                        signature: display_sig,
                        estimated_rows: None,
                        total_bytes: None,
                    });
                }
            }
        }

        // 7. Sequences
        let seq_sql = format!(
            "SELECT SEQUENCE_SCHEMA, SEQUENCE_NAME, DATA_TYPE, COMMENT FROM {db_id}.INFORMATION_SCHEMA.SEQUENCES WHERE SEQUENCE_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY SEQUENCE_SCHEMA, SEQUENCE_NAME LIMIT 2001"
        );
        if let Ok(rows) = self.query_rows(&seq_sql, 2000).await {
            for r in rows {
                if let (Some(schema), Some(name)) =
                    (r.first().cloned().flatten(), r.get(1).cloned().flatten())
                {
                    let dt = r.get(2).cloned().flatten();
                    objects.push(DatabaseObjectSummary {
                        object: DatabaseObjectRef {
                            schema,
                            name,
                            kind: DatabaseObjectKind::Sequence,
                            identity: None,
                        },
                        owner_table: None,
                        signature: dt,
                        estimated_rows: None,
                        total_bytes: None,
                    });
                }
            }
        }

        // 8. File Formats
        let format_sql = format!(
            "SELECT FILE_FORMAT_SCHEMA, FILE_FORMAT_NAME, FILE_FORMAT_TYPE, COMMENT FROM {db_id}.INFORMATION_SCHEMA.FILE_FORMATS WHERE FILE_FORMAT_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY FILE_FORMAT_SCHEMA, FILE_FORMAT_NAME LIMIT 2001"
        );
        if let Ok(rows) = self.query_rows(&format_sql, 2000).await {
            for r in rows {
                if let (Some(schema), Some(name)) =
                    (r.first().cloned().flatten(), r.get(1).cloned().flatten())
                {
                    let format_type = r.get(2).cloned().flatten();
                    objects.push(DatabaseObjectSummary {
                        object: DatabaseObjectRef {
                            schema,
                            name,
                            kind: DatabaseObjectKind::FileFormat,
                            identity: None,
                        },
                        owner_table: None,
                        signature: format_type,
                        estimated_rows: None,
                        total_bytes: None,
                    });
                }
            }
        }

        if objects.len() > 10_000 {
            objects.truncate(10_000);
        }
        Ok(objects)
    }

    pub(super) async fn get_object_details(
        &self,
        requested: &DatabaseObjectRef,
    ) -> AppResult<ObjectDetails> {
        let obj_type = match requested.kind {
            DatabaseObjectKind::Table | DatabaseObjectKind::PartitionedTable => Some("TABLE"),
            DatabaseObjectKind::View | DatabaseObjectKind::MaterializedView => Some("VIEW"),
            DatabaseObjectKind::Procedure => Some("PROCEDURE"),
            DatabaseObjectKind::Function => Some("FUNCTION"),
            DatabaseObjectKind::Sequence => Some("SEQUENCE"),
            DatabaseObjectKind::Stage => Some("STAGE"),
            DatabaseObjectKind::Task => Some("TASK"),
            DatabaseObjectKind::Stream => Some("STREAM"),
            DatabaseObjectKind::Pipe => Some("PIPE"),
            DatabaseObjectKind::FileFormat => Some("FILE_FORMAT"),
            DatabaseObjectKind::Index | DatabaseObjectKind::Trigger => None,
        };

        let ddl_status = if let Some(kind_str) = obj_type {
            self.fetch_snowflake_ddl(kind_str, requested).await
        } else {
            ObjectDdlStatus::Unavailable {
                reason: format!("Snowflake does not support DDL for {:?}", requested.kind),
            }
        };

        let (constraints, storage) = if matches!(
            requested.kind,
            DatabaseObjectKind::Table | DatabaseObjectKind::PartitionedTable
        ) {
            let table_ref = TableRef {
                schema: requested.schema.clone(),
                name: requested.name.clone(),
            };
            if let Ok(schema) = self.table_schema(&table_ref).await {
                let mut c_list = Vec::new();
                if !schema.primary_key.is_empty() {
                    c_list.push(ConstraintInfo {
                        name: format!("PK_{}", requested.name),
                        kind: ConstraintKind::PrimaryKey,
                        columns: schema.primary_key,
                        target: None,
                        target_columns: Vec::new(),
                        definition: "PRIMARY KEY".into(),
                    });
                }
                (c_list, None)
            } else {
                (Vec::new(), None)
            }
        } else {
            (Vec::new(), None)
        };

        Ok(ObjectDetails {
            object: requested.clone(),
            owner_table: None,
            comment: None,
            ddl: ddl_status,
            indexes: Vec::new(),
            constraints,
            triggers: Vec::new(),
            routine: None,
            sequence: None,
            storage,
            uses: Vec::new(),
            used_by: Vec::new(),
        })
    }

    async fn fetch_snowflake_ddl(
        &self,
        kind_str: &str,
        requested: &DatabaseObjectRef,
    ) -> ObjectDdlStatus {
        let db = self.current_database();
        let db_id = match identifier(&db) {
            Ok(id) => id,
            Err(e) => return ObjectDdlStatus::Unavailable { reason: e.message },
        };
        let sc_id = match identifier(&requested.schema) {
            Ok(id) => id,
            Err(e) => return ObjectDdlStatus::Unavailable { reason: e.message },
        };
        let nm_id = match identifier(&requested.name) {
            Ok(id) => id,
            Err(e) => return ObjectDdlStatus::Unavailable { reason: e.message },
        };

        let primary_target = if matches!(
            requested.kind,
            DatabaseObjectKind::Procedure | DatabaseObjectKind::Function
        ) {
            if let Some(sig) = &requested.identity {
                format!("{db_id}.{sc_id}.{nm_id}{sig}")
            } else {
                format!("{db_id}.{sc_id}.{nm_id}")
            }
        } else {
            format!("{db_id}.{sc_id}.{nm_id}")
        };

        let sql = format!(
            "SELECT GET_DDL('{kind_str}', '{}')",
            primary_target.replace('\'', "''")
        );
        match self.query_rows(&sql, 1).await {
            Ok(rows) => {
                if let Some(ddl_raw) = rows
                    .into_iter()
                    .next()
                    .and_then(|r| r.into_iter().next())
                    .flatten()
                {
                    let mut ddl = ddl_raw.trim().to_string();
                    if !ddl.ends_with(';') {
                        ddl.push(';');
                    }
                    return ObjectDdlStatus::Available {
                        ddl,
                        truncated: false,
                    };
                }
            }
            Err(err) => {
                let schema_target = if let Some(sig) = &requested.identity {
                    format!("{sc_id}.{nm_id}{sig}")
                } else {
                    format!("{sc_id}.{nm_id}")
                };
                let fallback_sql = format!(
                    "SELECT GET_DDL('{kind_str}', '{}')",
                    schema_target.replace('\'', "''")
                );
                if let Some(ddl_raw) = self
                    .query_rows(&fallback_sql, 1)
                    .await
                    .ok()
                    .and_then(|rows| rows.into_iter().next()?.into_iter().next()?)
                {
                    let mut ddl = ddl_raw.trim().to_string();
                    if !ddl.ends_with(';') {
                        ddl.push(';');
                    }
                    return ObjectDdlStatus::Available {
                        ddl,
                        truncated: false,
                    };
                }

                if matches!(
                    requested.kind,
                    DatabaseObjectKind::Procedure | DatabaseObjectKind::Function
                ) && requested.identity.is_some()
                {
                    let nosig_target = format!("{db_id}.{sc_id}.{nm_id}");
                    let nosig_sql = format!(
                        "SELECT GET_DDL('{kind_str}', '{}')",
                        nosig_target.replace('\'', "''")
                    );
                    if let Some(ddl_raw) = self
                        .query_rows(&nosig_sql, 1)
                        .await
                        .ok()
                        .and_then(|rows| rows.into_iter().next()?.into_iter().next()?)
                    {
                        let mut ddl = ddl_raw.trim().to_string();
                        if !ddl.ends_with(';') {
                            ddl.push(';');
                        }
                        return ObjectDdlStatus::Available {
                            ddl,
                            truncated: false,
                        };
                    }
                }

                return ObjectDdlStatus::Unavailable {
                    reason: err.message,
                };
            }
        }

        ObjectDdlStatus::Unavailable {
            reason: "Snowflake returned empty DDL".into(),
        }
    }
}
