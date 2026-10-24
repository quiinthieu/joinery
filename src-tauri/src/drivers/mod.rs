use crate::domain::*;
use async_trait::async_trait;

pub mod postgres;
pub mod redis;
pub mod snowflake;
pub mod snowflake_auth;

pub trait QuerySink: Send {
    fn limit(&mut self, reason: &str);
    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()>;
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool>;
    fn complete(&mut self, affected: u64) -> AppResult<()>;
}

#[derive(Clone, serde::Serialize)]
pub struct RowMutation {
    pub original: TextRow,
    pub changes: Vec<(usize, Option<String>)>,
}

pub struct BrowseRows {
    pub rows: Vec<TextRow>,
    pub next_key: Option<Vec<Option<String>>>,
    pub has_more: bool,
    pub truncated: bool,
}

#[async_trait]
pub trait Driver: Send + Sync {
    async fn execute(
        &self,
        sql: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64>;
    async fn cancel(&self) -> AppResult<()>;
    async fn catalog(&self) -> AppResult<Vec<TableInfo>>;
    async fn schema(&self, table: &TableRef) -> AppResult<TableSchema>;
    async fn browse(
        &self,
        schema: &TableSchema,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows>;
    async fn apply(&self, schema: &TableSchema, rows: &[RowMutation]) -> AppResult<()>;
    async fn processes(&self) -> AppResult<Vec<ProcessInfo>>;
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
