use super::*;

impl Snowflake {
    pub(super) async fn live_completion_identity(&self) -> AppResult<CompletionIdentity> {
        let expected_schema = self.current_schema();
        let rows = self.context_rows("SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), TO_JSON(CURRENT_SCHEMAS())").await?;
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
        if row[1].as_deref() != Some(&self.role)
            || row[2].as_deref() != Some(&self.database)
            || row[3].as_deref() != Some(expected_schema.as_str())
            || row[4].as_deref() != Some(&self.warehouse)
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake completion context differs from the configured SQL request",
            ));
        }
        let search_path: Vec<String> =
            serde_json::from_str(row[5].as_deref().ok_or_else(invalid)?).map_err(|_| invalid())?;
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
        if setting[0].as_deref() != Some("QUOTED_IDENTIFIERS_IGNORE_CASE") {
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
            database: self.database.clone(),
            role: format!("{user} ({})", self.role),
            search_path,
            case_sensitive_table_names: None,
        })
    }

    pub(super) async fn live_completion_catalog(
        &self,
        identity: &CompletionIdentity,
    ) -> AppResult<CompletionRelations> {
        let tables = self.catalog_tables().await?;
        // Catalog visibility cannot prove the absence of an inaccessible table
        // in an earlier schema. Bind known names in the first resolved schema
        // only; later schemas and cross-database paths retain qualified names.
        let prefix = format!("{}.", self.database);
        let first_schema = identity
            .search_path
            .first()
            .and_then(|p| p.strip_prefix(&prefix))
            .filter(|s| simple_identifier(s));
        let unqualified_tables = tables
            .iter()
            .filter(|t| Some(t.table.schema.as_str()) == first_schema)
            .map(|t| t.table.clone())
            .collect();
        Ok(CompletionRelations {
            tables,
            unqualified_tables,
        })
    }
}
