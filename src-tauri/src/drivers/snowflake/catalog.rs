use super::*;

struct MetadataResult {
    columns: Vec<ColumnInfo>,
    rows: Vec<TextRow>,
    truncated: bool,
}

fn identifier(value: &str) -> AppResult<String> {
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
    fn qualified(&self, table: &TableRef) -> AppResult<String> {
        Ok(format!(
            "{}.{}.{}",
            identifier(&self.database)?,
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

    pub(super) async fn catalog_tables(&self) -> AppResult<Vec<TableInfo>> {
        let sql = format!(
            "SELECT TABLE_SCHEMA,TABLE_NAME,TABLE_TYPE,ROW_COUNT,BYTES FROM {}.INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY TABLE_SCHEMA,TABLE_NAME LIMIT 10001",
            identifier(&self.database)?
        );
        let rows = self.query_rows(&sql, 10_000).await?;
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
                    kind: required(2)?,
                    estimated_rows: estimate(3),
                    total_bytes: estimate(4),
                })
            })
            .collect()
    }

    pub(super) async fn table_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        let sql = format!("SELECT * FROM {} LIMIT 0", self.qualified(table)?);
        let result = self.metadata_result(&sql, 0, 0, false).await?;
        if result.columns.is_empty() {
            return Err(AppError::new(
                ErrorCode::NotFound,
                "Snowflake table columns are unavailable",
            ));
        }
        // Standard Snowflake PK/FK declarations are not enforced. No inferred
        // keys, keyset continuation, or edit provenance is exposed by this pilot.
        Ok(TableSchema {
            table: table.clone(),
            columns: result.columns,
            primary_key: Vec::new(),
            foreign_keys: Vec::new(),
            editable: false,
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
        if result.columns != schema.columns {
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
}
