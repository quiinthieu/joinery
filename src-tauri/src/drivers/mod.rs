use crate::domain::*;
use async_trait::async_trait;

pub mod browse_query;
pub mod edit_target;
pub use edit_target::{ColumnOrigin, EditIdentity, EditTarget, MysqlColumnOrigin};
pub mod mysql;
pub mod postgres;
pub mod redis;
pub mod snowflake;
pub mod snowflake_auth;
pub(crate) mod snowflake_batch;
#[cfg(unix)]
pub(crate) mod snowflake_browsing;
pub mod snowflake_dml_summary;
#[cfg(unix)]
pub(crate) mod snowflake_edit_codec;
#[cfg(unix)]
pub(crate) mod snowflake_edit_snapshot;
#[cfg(unix)]
pub mod snowflake_external;
#[cfg(unix)]
mod snowflake_metadata;
pub mod snowflake_mutations;
#[cfg(unix)]
pub mod snowflake_process;
#[cfg(all(unix, feature = "test-harness"))]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Internal qualification plumbing; exercised by harness tests until the fixed runner is implemented"
    )
)]
pub(crate) mod snowflake_qualification;
#[cfg(all(unix, feature = "test-harness"))]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Internal qualification metadata interface; no desktop entry point"
    )
)]
pub(crate) mod snowflake_qualification_metadata;
#[cfg(all(unix, feature = "test-harness"))]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Internal qualification transport interface; no desktop entry point"
    )
)]
pub(crate) mod snowflake_qualification_transport;
#[cfg(unix)]
mod snowflake_session;
pub(crate) mod snowflake_sql_contract;
#[cfg(all(unix, feature = "test-harness"))]
pub mod snowflake_table_qualification;

pub trait QuerySink: Send {
    fn limit(&mut self, reason: &str);
    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()>;
    fn describe_with_origins(
        &mut self,
        columns: Vec<ColumnInfo>,
        _origins: Vec<ColumnOrigin>,
    ) -> AppResult<()> {
        self.describe(columns)
    }
    fn describe_mysql_with_origins(
        &mut self,
        columns: Vec<ColumnInfo>,
        _origins: Vec<MysqlColumnOrigin>,
    ) -> AppResult<()> {
        self.describe(columns)
    }
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool>;
    fn complete(&mut self, affected: u64) -> AppResult<()>;
}

#[derive(Clone, serde::Serialize)]
pub enum RowMutation {
    Update {
        original: TextRow,
        changes: Vec<(usize, Option<String>)>,
    },
    Delete {
        original: TextRow,
    },
    Insert {
        values: Vec<(usize, Option<String>)>,
    },
}

pub struct BrowseRows {
    pub rows: Vec<TextRow>,
    pub next_key: Option<Vec<Option<String>>>,
    pub has_more: bool,
    pub truncated: bool,
}

pub struct CompletionRelations {
    pub tables: Vec<TableInfo>,
    pub unqualified_tables: Vec<TableRef>,
}

#[async_trait]
pub trait Driver: Send + Sync {
    /// Forget all server-held relation targets before attempting a context change.
    fn invalidate_relation_catalog(&self) {}
    async fn relation_catalog(&self) -> AppResult<BrowseCatalog> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Session relation browsing is unavailable for this engine",
        ))
    }
    async fn inspect_relation(
        &self,
        _request: &InspectRelationRequest,
    ) -> AppResult<RelationInspection> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Session relation inspection is unavailable for this engine",
        ))
    }
    async fn define_relation(
        &self,
        _request: &DefineRelationRequest,
    ) -> AppResult<RelationDefinition> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Session relation definitions are unavailable for this engine",
        ))
    }
    async fn inspect_relation_constraints(
        &self,
        _request: &InspectRelationConstraintsRequest,
    ) -> AppResult<RelationConstraints> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Session relation key constraints are unavailable for this engine",
        ))
    }
    async fn inspect_relation_check_constraints(
        &self,
        _request: &InspectRelationCheckConstraintsRequest,
    ) -> AppResult<RelationCheckConstraints> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Session relation CHECK constraints are unavailable for this engine",
        ))
    }
    async fn inspect_relation_constraint_mappings(
        &self,
        _request: &InspectRelationConstraintMappingsRequest,
    ) -> AppResult<RelationConstraintMappings> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Session relation key column mappings are unavailable for this engine",
        ))
    }
    async fn preview_relation(
        &self,
        _request: &PreviewRelationRequest,
    ) -> AppResult<RelationPreview> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Session relation preview is unavailable for this engine",
        ))
    }
    /// Revoke retained edit leases before approved schema changes. Drivers with
    /// metadata locks must await their bounded release before SQL submission.
    async fn invalidate_edit_targets(&self) -> AppResult<()> {
        Ok(())
    }
    async fn edit_target(&self, table: &TableRef) -> AppResult<EditTarget> {
        Ok(EditTarget {
            schema: self.schema(table).await?,
            identity: None,
        })
    }
    async fn browse_target(
        &self,
        schema: &TableSchema,
        query: &BrowseQuery,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<(BrowseRows, EditTarget)> {
        Ok((
            self.browse_query(schema, query, after).await?,
            EditTarget {
                schema: schema.clone(),
                identity: None,
            },
        ))
    }
    async fn apply_target(
        &self,
        schema: &TableSchema,
        identity: Option<&EditIdentity>,
        rows: &[RowMutation],
    ) -> AppResult<()> {
        if identity.is_some() {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "The edit target identity cannot be verified by this driver",
            ));
        }
        self.apply(schema, rows).await
    }
    async fn execute(
        &self,
        sql: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64>;
    async fn execute_bound(
        &self,
        sql: &str,
        kind: StatementKind,
        bound: &crate::params::BoundStatement,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        if bound.binds.is_empty() {
            return self.execute(sql, kind, sink).await;
        }
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Typed query parameters are unavailable for this engine",
        ))
    }
    async fn cancel(&self) -> AppResult<()>;
    async fn completion_identity(&self) -> AppResult<CompletionIdentity> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Live completion context is unavailable for this engine",
        ))
    }
    /// Resolve the relation on the SQL editor connection before granting edit
    /// provenance. Metadata connections cannot see session-local table shadows.
    async fn resolve_query_edit_table(
        &self,
        _table: &TableRef,
        _unqualified: bool,
    ) -> AppResult<TableRef> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Query result table identity cannot be verified for this engine",
        ))
    }
    async fn completion_catalog(
        &self,
        _identity: &CompletionIdentity,
    ) -> AppResult<CompletionRelations> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Live completion metadata is unavailable for this engine",
        ))
    }
    async fn completion_schema(&self, _table: &TableRef) -> AppResult<TableSchema> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Live completion metadata is unavailable for this engine",
        ))
    }
    async fn inspection_schema(&self, _table: &TableRef) -> AppResult<TableSchema> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Complete column inspection metadata is unavailable for this engine",
        ))
    }
    async fn session_state(&self, _session_id: &str) -> AppResult<SessionRuntimeState> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Session runtime context is unavailable for this engine",
        ))
    }
    async fn list_contexts(&self) -> AppResult<Vec<String>> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Database or schema switching is unavailable for this engine",
        ))
    }
    async fn session_context_options(&self, session_id: &str) -> AppResult<SessionContextOptions> {
        let state = self.session_state(session_id).await?;
        let mut available = self.list_contexts().await?;
        if let Some(active) = &state.active_context
            && !active.is_empty()
            && !available.iter().any(|item| item == active)
        {
            available.insert(0, active.clone());
        }
        Ok(SessionContextOptions {
            state,
            available,
            revision: None,
        })
    }
    async fn switch_context(&self, _target: &str) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Database or schema switching is unavailable for this engine",
        ))
    }
    async fn switch_schema(
        &self,
        _session_id: &str,
        _request: &SwitchSchemaRequest,
    ) -> AppResult<SessionContextOptions> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Revision-bound schema switching is unavailable for this engine",
        ))
    }
    /// Pure validation only; this cannot consume authority or submit SQL.
    fn validate_snowflake_dml(&self, _request: &SnowflakeDmlRequest) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Reviewed Snowflake SQL writes are unavailable for this engine",
        ))
    }
    async fn snowflake_dml(
        &self,
        _session_id: &str,
        _request: &SnowflakeDmlRequest,
    ) -> AppResult<SnowflakeDmlResult> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Reviewed Snowflake SQL writes are unavailable for this engine",
        ))
    }
    async fn preflight_snowflake_ddl(
        &self,
        _session_id: &str,
        _request: &SnowflakeDdlPreflightRequest,
    ) -> AppResult<SnowflakeDdlPreflightResponse> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Reviewed Snowflake table DDL is unavailable for this engine",
        ))
    }
    /// Pure validation only; this cannot consume authority or submit SQL.
    fn validate_snowflake_ddl(&self, _request: &SnowflakeDdlRequest) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Reviewed Snowflake table DDL is unavailable for this engine",
        ))
    }
    async fn snowflake_ddl(
        &self,
        _session_id: &str,
        _request: &SnowflakeDdlRequest,
    ) -> AppResult<SnowflakeDdlResult> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Reviewed Snowflake table DDL is unavailable for this engine",
        ))
    }
    /// Pure validation only: preparation must never consume authority or send SQL.
    fn validate_transaction_control(&self, _request: &TransactionControlRequest) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Reviewed explicit transaction controls are unavailable for this engine",
        ))
    }
    async fn transaction_control(
        &self,
        _session_id: &str,
        _request: &TransactionControlRequest,
    ) -> AppResult<TransactionControlResult> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Reviewed explicit transaction controls are unavailable for this engine",
        ))
    }
    async fn set_transaction_mode(&self, _mode: TransactionMode) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Transaction mode controls are unavailable for this engine",
        ))
    }
    async fn commit_transaction(&self) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Transactions are unavailable for this engine",
        ))
    }
    async fn rollback_transaction(&self) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Transactions are unavailable for this engine",
        ))
    }
    async fn catalog(&self) -> AppResult<Vec<TableInfo>>;
    async fn database_objects(&self) -> AppResult<Vec<DatabaseObjectSummary>> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Extended database object discovery is unavailable for this engine",
        ))
    }
    async fn object_details(&self, _object: &DatabaseObjectRef) -> AppResult<ObjectDetails> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Database object details are unavailable for this engine",
        ))
    }
    async fn schema(&self, table: &TableRef) -> AppResult<TableSchema>;
    async fn referencing_keys(&self, _table: &TableRef) -> AppResult<Vec<InboundForeignKey>> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Referencing foreign-key discovery is unavailable for this engine",
        ))
    }
    /// Bounded, read-only relationship graph for one schema.
    async fn schema_diagram(&self, _schema: &str) -> AppResult<SchemaDiagram> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Schema diagrams are unavailable for this engine",
        ))
    }
    /// Bounded, canonical structural snapshot for one schema.
    async fn schema_snapshot(&self, _schema: &str) -> AppResult<SchemaSnapshot> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Schema snapshots are unavailable for this engine",
        ))
    }
    async fn browse(
        &self,
        schema: &TableSchema,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows>;
    /// Browses with typed server-side sort/filter. Engines without
    /// `table_filtering` accept only the empty query.
    async fn browse_query(
        &self,
        schema: &TableSchema,
        query: &BrowseQuery,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        if !query.is_empty() {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Sorting and filtering table data is unavailable for this engine",
            ));
        }
        self.browse(schema, after).await
    }
    async fn apply(&self, schema: &TableSchema, rows: &[RowMutation]) -> AppResult<()>;
    async fn processes(&self) -> AppResult<Vec<ProcessInfo>>;
    /// Sessions currently waiting on locks and the sessions blocking them.
    async fn lock_waits(&self) -> AppResult<LockWaitSnapshot> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Lock inspection is unavailable for this engine",
        ))
    }
    async fn cancel_process(&self, id: &str, identity: &str) -> AppResult<()>;
    async fn redis_scan(&self, _pattern: &str, _cursor: Option<&str>) -> AppResult<RedisScanPage> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "This session is not Redis",
        ))
    }
    async fn redis_inspect(
        &self,
        _argument: &str,
        _cursor: Option<&str>,
    ) -> AppResult<RedisValuePreview> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "This session is not Redis",
        ))
    }
    async fn health(&self) -> AppResult<u32>;
    fn is_closed(&self) -> bool;
    fn close(&self);
}
