use crate::domain::*;
use async_trait::async_trait;

pub mod browse_query;
pub mod mysql;
pub mod postgres;
pub mod redis;
pub mod snowflake;
pub mod snowflake_auth;

pub trait QuerySink: Send {
    fn limit(&mut self, reason: &str);
    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()>;
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool>;
    fn complete(&mut self, affected: u64) -> AppResult<()>;
    fn query_id(&mut self, _query_id: &str) {}
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
    fn active_query_handle(&self) -> Option<String> {
        None
    }
    async fn completion_identity(&self) -> AppResult<CompletionIdentity> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Live completion context is unavailable for this engine",
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
    async fn list_context_dimensions(&self) -> AppResult<Vec<ContextDimension>> {
        let contexts = self.list_contexts().await?;
        Ok(vec![ContextDimension {
            kind: "schema".into(),
            values: contexts,
        }])
    }
    async fn switch_context(&self, _target: &str) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Database or schema switching is unavailable for this engine",
        ))
    }
    async fn switch_session_context(
        &self,
        target: Option<&str>,
        _warehouse: Option<&str>,
        _role: Option<&str>,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> AppResult<SessionContextOptions> {
        if let Some(target) = target.or(schema).or(database) {
            self.switch_context(target).await?;
        }
        Ok(SessionContextOptions::default())
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
