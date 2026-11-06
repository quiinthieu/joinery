use super::*;

fn parse_current_schemas(raw: &str) -> Option<Vec<String>> {
    if let Ok(paths) = serde_json::from_str::<Vec<String>>(raw) {
        return Some(paths);
    }
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    let mut paths = Vec::new();
    for item in inner.split(',') {
        let entry = item.trim();
        if entry.is_empty()
            || !entry.contains('.')
            || entry.starts_with('"')
            || entry.ends_with('"')
            || entry.chars().any(char::is_control)
        {
            return None;
        }
        paths.push(entry.to_string());
    }
    Some(paths)
}

impl Snowflake {
    pub(super) async fn live_completion_identity(&self) -> AppResult<CompletionIdentity> {
        let expected_schema = self.current_schema();
        let rows = self
            .context_rows("SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SCHEMAS()")
            .await?;
        let invalid = || {
            AppError::new(
                ErrorCode::Database,
                "Snowflake returned invalid completion context",
            )
        };
        let row = rows.first().filter(|r| r.len() == 6).ok_or_else(invalid)?;
        let user = row[0]
            .as_deref()
            .filter(|v| !v.is_empty())
            .ok_or_else(invalid)?;
        if !same(&row[1], &self.current_role())
            || !same(&row[2], &self.current_database())
            || !same(&row[3], &expected_schema)
            || !same(&row[4], &self.current_warehouse())
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake completion context differs from the configured SQL request",
            ));
        }
        let role = row[1].as_deref().ok_or_else(invalid)?;
        let database = row[2].clone().ok_or_else(invalid)?;
        let search_path = row[5]
            .as_deref()
            .and_then(parse_current_schemas)
            .ok_or_else(invalid)?;
        if search_path.len() > 64 || search_path.iter().any(|s| s.is_empty() || s.len() > 1024) {
            return Err(invalid());
        }
        // SQL API requests inherit account/user defaults. Never assume that
        // double quotes preserve case when the effective setting says otherwise.
        let settings = self
            .context_rows("SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION")
            .await?;
        let setting = settings
            .first()
            .filter(|r| r.len() >= 2)
            .ok_or_else(invalid)?;
        if !same(&setting[0], "QUOTED_IDENTIFIERS_IGNORE_CASE") {
            return Err(invalid());
        }
        match setting[1].as_deref() {
            Some(v) if v.eq_ignore_ascii_case("false") => (),
            Some(v) if v.eq_ignore_ascii_case("true") => {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "Completion requires case-preserving quoted Snowflake identifiers",
                ));
            }
            _ => return Err(invalid()),
        }
        Ok(CompletionIdentity {
            database,
            role: format!("{user} ({role})"),
            search_path,
            case_sensitive_table_names: None,
        })
    }

    pub(super) async fn live_completion_catalog(
        &self,
        identity: &CompletionIdentity,
    ) -> AppResult<CompletionRelations> {
        let tables = match self.catalog_tables().await {
            Ok(tables) => tables,
            Err(_) => {
                let schema = self.current_schema();
                self.catalog_tables_in_schema(&schema).await?
            }
        };
        // Catalog visibility cannot prove the absence of an inaccessible table
        // in an earlier schema. Bind known names in the first resolved schema
        // only; later schemas and cross-database paths retain qualified names.
        let first_schema = identity
            .search_path
            .first()
            .and_then(|p| {
                let (db, schema) = p.split_once('.')?;
                db.eq_ignore_ascii_case(&identity.database)
                    .then_some(schema)
            })
            .filter(|s| simple_identifier(s));
        let unqualified_tables = tables
            .iter()
            .filter(|t| first_schema.is_some_and(|s| t.table.schema.eq_ignore_ascii_case(s)))
            .map(|t| t.table.clone())
            .collect();
        Ok(CompletionRelations {
            tables,
            unqualified_tables,
        })
    }
}
