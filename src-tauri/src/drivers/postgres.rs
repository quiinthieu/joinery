use super::*;
use crate::{results::BATCH_ROWS, safety::quote_identifier};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use parking_lot::Mutex;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::OnceCell;
use tokio_postgres::{Client, Config, SimpleQueryMessage, types::ToSql};
use tokio_postgres_rustls::MakeRustlsConnect;

pub struct Postgres {
    client: Client,
    metadata: OnceCell<Client>,
    config: Config,
    tls: MakeRustlsConnect,
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
    edit_cancel: Mutex<Option<tokio_postgres::CancelToken>>,
    own_pid: i32,
}

impl Postgres {
    pub async fn open(
        profile: &ConnectionProfile,
        password: Option<&str>,
        session_id: &str,
        forwarded_port: Option<u16>,
    ) -> AppResult<(Arc<Self>, SessionInfo)> {
        let started = Instant::now();
        let mut config = Config::new();
        config.host(&profile.host).port(forwarded_port.unwrap_or(profile.port))
            .user(&profile.username).dbname(&profile.database).application_name(format!("Joinery {session_id}"))
            .connect_timeout(Duration::from_secs(10))
            .options("-c statement_timeout=60000 -c lock_timeout=5000 -c idle_in_transaction_session_timeout=60000");
        if forwarded_port.is_some() {
            config.hostaddr(std::net::Ipv4Addr::LOCALHOST.into());
        }
        if let Some(password) = password {
            config.password(password);
        }
        if profile.safe_mode == SafeMode::ReadOnly {
            config.options("-c statement_timeout=60000 -c lock_timeout=5000 -c idle_in_transaction_session_timeout=60000 -c default_transaction_read_only=on");
        }
        config.ssl_mode(match profile.tls_mode {
            TlsMode::VerifyFull => tokio_postgres::config::SslMode::Require,
            TlsMode::Disable => tokio_postgres::config::SslMode::Disable,
        });
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = rustls::RootCertStore::empty();
        for cert in rustls_native_certs::load_native_certs().certs {
            let _ = roots.add(cert);
        }
        if let Some(path) = &profile.ca_cert_path {
            let bytes = crate::files::read_bounded(path, 1024 * 1024)?;
            let mut certificate_count = 0;
            for cert in rustls_pemfile::certs(&mut std::io::Cursor::new(bytes.as_slice())) {
                roots
                    .add(cert.map_err(|_| AppError::invalid("Invalid CA certificate"))?)
                    .map_err(|_| AppError::invalid("Invalid CA certificate"))?;
                certificate_count += 1;
            }
            if certificate_count == 0 {
                return Err(AppError::invalid(
                    "The selected file contains no PEM certificates",
                ));
            }
        }
        let tls = MakeRustlsConnect::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        let (client, connection) = config.connect(tls.clone()).await.map_err(pg_error)?;
        let task = tokio::spawn(async move {
            let _ = connection.await;
        });
        let row = client
            .query_one("SELECT version(),pg_backend_pid()", &[])
            .await
            .map_err(pg_error)?;
        let info = SessionInfo {
            id: session_id.into(),
            connection_id: profile.id.clone(),
            engine: EngineKind::Postgres,
            server_version: row.get(0),
            latency_ms: started.elapsed().as_millis() as u32,
            encrypted: profile.tls_mode == TlsMode::VerifyFull,
            capabilities: EngineCapabilities {
                sql: true,
                transactions: true,
                cancellation: true,
                table_browsing: true,
                table_editing: true,
                table_filtering: true,
                processes: true,
            },
        };
        let driver = Self {
            own_pid: row.get(1),
            client,
            metadata: OnceCell::new(),
            config,
            tls,
            tasks: Mutex::new(vec![task.abort_handle()]),
            edit_cancel: Mutex::new(None),
        };
        Ok((Arc::new(driver), info))
    }

    async fn metadata(&self) -> AppResult<&Client> {
        self.metadata
            .get_or_try_init(|| async {
                let mut config = self.config.clone();
                config.application_name("Joinery metadata");
                let (client, connection) =
                    config.connect(self.tls.clone()).await.map_err(pg_error)?;
                let task = tokio::spawn(async move {
                    let _ = connection.await;
                });
                self.tasks.lock().push(task.abort_handle());
                Ok(client)
            })
            .await
    }
}

#[async_trait]
impl Driver for Postgres {
    async fn execute(
        &self,
        sql: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        let metadata = if kind == StatementKind::Read {
            Some(
                self.client
                    .prepare(sql)
                    .await
                    .map_err(pg_error)?
                    .columns()
                    .iter()
                    .enumerate()
                    .map(|(i, c)| column(i, c.name(), c.type_().name()))
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        let stream = self.client.simple_query_raw(sql).await.map_err(pg_error)?;
        futures_util::pin_mut!(stream);
        let mut pending = Vec::new();
        let mut pending_bytes = 0usize;
        let mut retaining = true;
        let mut total = 0u64;
        let mut limit_cancel_sent = false;
        loop {
            let message = match stream.try_next().await {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(error) if !retaining && error.code().is_some_and(|c| c.code() == "57014") => {
                    sink.complete(total)?;
                    return Ok(total);
                }
                Err(error) => return Err(pg_error(error)),
            };
            match message {
                SimpleQueryMessage::RowDescription(columns) => {
                    retaining = true;
                    let columns = metadata
                        .clone()
                        .filter(|m| m.len() == columns.len())
                        .unwrap_or_else(|| {
                            columns
                                .iter()
                                .enumerate()
                                .map(|(i, c)| column(i, c.name(), "unknown"))
                                .collect()
                        });
                    sink.describe(columns)?;
                }
                SimpleQueryMessage::Row(row) if retaining => {
                    let bytes = (0..row.len())
                        .map(|i| row.get(i).map_or(0, str::len))
                        .sum::<usize>();
                    if bytes > 2 * 1024 * 1024 {
                        retaining = false;
                        sink.limit("A returned row exceeds the 2 MiB viewer limit. Use a query with a smaller projection.");
                        if kind == StatementKind::Read {
                            if !pending.is_empty() {
                                sink.rows(std::mem::take(&mut pending))?;
                            }
                            sink.complete(total)?;
                            self.close();
                            return Ok(total);
                        }
                        continue;
                    }
                    pending.push(
                        (0..row.len())
                            .map(|i| row.get(i).map(str::to_string))
                            .collect(),
                    );
                    pending_bytes += bytes;
                    if pending.len() >= BATCH_ROWS || pending_bytes >= 256 * 1024 {
                        retaining = sink.rows(std::mem::take(&mut pending))?;
                        pending_bytes = 0;
                        if !retaining && kind == StatementKind::Read {
                            limit_cancel_sent = true;
                            self.cancel().await?;
                        }
                    }
                }
                SimpleQueryMessage::CommandComplete(affected) => {
                    if !pending.is_empty() {
                        let _ = sink.rows(std::mem::take(&mut pending))?;
                        pending_bytes = 0;
                    }
                    total = total.saturating_add(affected);
                    sink.complete(affected)?;
                }
                _ => {}
            }
        }
        if limit_cancel_sent {
            self.close();
        }
        Ok(total)
    }

    async fn cancel(&self) -> AppResult<()> {
        let token = self
            .edit_cancel
            .lock()
            .clone()
            .unwrap_or_else(|| self.client.cancel_token());
        token.cancel_query(self.tls.clone()).await.map_err(pg_error)
    }

    async fn completion_identity(&self) -> AppResult<CompletionIdentity> {
        let row = self
            .client
            .query_one(
                "SELECT pg_catalog.current_database()::pg_catalog.text,current_user::pg_catalog.text,pg_catalog.current_schemas(true)::pg_catalog.text[]",
                &[],
            )
            .await
            .map_err(pg_error)?;
        let identity = CompletionIdentity {
            database: row.get(0),
            role: row.get(1),
            search_path: row.get(2),
            case_sensitive_table_names: None,
        };
        if identity.search_path.len() > 128 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "The schema search path exceeds the completion budget",
            ));
        }
        Ok(identity)
    }
    async fn completion_catalog(
        &self,
        _identity: &CompletionIdentity,
    ) -> AppResult<super::CompletionRelations> {
        let rows = self.client.query("SELECT n.nspname,c.relname,c.relkind::pg_catalog.text,pg_catalog.pg_table_is_visible(c.oid) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p','v','m') AND (n.nspname NOT LIKE 'pg\\_%' OR n.nspname='pg_catalog' OR n.oid=pg_catalog.pg_my_temp_schema()) AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND (pg_catalog.has_table_privilege(c.oid,'SELECT') OR pg_catalog.has_any_column_privilege(c.oid,'SELECT')) ORDER BY n.nspname,c.relname LIMIT 10001", &[]).await.map_err(pg_error)?;
        if rows.len() > 10000 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "The catalog exceeds the live completion budget",
            ));
        }
        let unqualified_tables = rows
            .iter()
            .filter(|r| r.get::<_, bool>(3))
            .map(|r| TableRef {
                schema: r.get(0),
                name: r.get(1),
            })
            .collect();
        let tables = rows
            .iter()
            .map(|r| TableInfo {
                table: TableRef {
                    schema: r.get(0),
                    name: r.get(1),
                },
                kind: r.get(2),
                estimated_rows: None,
                total_bytes: None,
            })
            .collect();
        Ok(super::CompletionRelations {
            tables,
            unqualified_tables,
        })
    }
    async fn completion_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        schema_on(&self.client, table, true).await
    }
    async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
        let rows=self.metadata().await?.query(
            "SELECT n.nspname,c.relname,c.relkind::text,c.reltuples::float8,pg_total_relation_size(c.oid)::float8
             FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
             WHERE c.relkind IN ('r','p','v','m') AND n.nspname NOT LIKE 'pg\\_%' AND n.nspname <> 'information_schema'
             AND has_schema_privilege(n.oid,'USAGE') ORDER BY n.nspname,c.relname LIMIT 10000",&[]).await.map_err(pg_error)?;
        Ok(rows
            .iter()
            .map(|r| {
                let estimate: f64 = r.get(3);
                TableInfo {
                    table: TableRef {
                        schema: r.get(0),
                        name: r.get(1),
                    },
                    kind: r.get(2),
                    estimated_rows: (estimate >= 0.0).then_some(estimate),
                    total_bytes: Some(r.get(4)),
                }
            })
            .collect())
    }

    async fn schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        schema_on(self.metadata().await?, table, false).await
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
        let parameters = built
            .binds
            .iter()
            .map(|v| v as &(dyn ToSql + Sync))
            .collect::<Vec<_>>();
        let stream = self
            .metadata()
            .await?
            .query_raw(&sql, parameters)
            .await
            .map_err(pg_error)?;
        futures_util::pin_mut!(stream);
        // Row counts alone do not bound wide text/JSON tables. Stop before copying
        // the row that would exceed the page budget; the last retained PK resumes it.
        const PAGE_BYTES: usize = 2 * 1024 * 1024;
        let mut bytes = 0;
        let mut rows = Vec::new();
        let mut limited = false;
        while let Some(row) = stream.try_next().await.map_err(pg_error)? {
            let size = (0..row.len())
                .map(|i| row.get::<_, Option<&str>>(i).map_or(5, |v| v.len() + 5))
                .sum::<usize>();
            if rows.len() == 500 || bytes + size > PAGE_BYTES {
                if rows.is_empty() {
                    return Err(AppError::new(
                        ErrorCode::ResultLimitReached,
                        "A table row exceeds the 2 MiB page budget. Use a query with a smaller projection.",
                    ));
                }
                limited = true;
                break;
            }
            bytes += size;
            rows.push(
                (0..row.len())
                    .map(|i| row.get::<_, Option<String>>(i))
                    .collect::<TextRow>(),
            );
        }
        let has_more = limited && built.pageable;
        let next_key = if has_more {
            rows.last().map(|r| built.cursor(r))
        } else {
            None
        };
        Ok(BrowseRows {
            rows,
            next_key,
            has_more,
            truncated: limited && !has_more,
        })
    }

    async fn apply(&self, schema: &TableSchema, rows: &[RowMutation]) -> AppResult<()> {
        if !schema.editable {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This table cannot be edited safely",
            ));
        }
        // A grid commit must never commit an open transaction in the SQL editor.
        let mut config = self.config.clone();
        config.application_name("Joinery grid commit");
        let (client, connection) = config.connect(self.tls.clone()).await.map_err(pg_error)?;
        let task = tokio::spawn(async move {
            let _ = connection.await;
        });
        {
            let mut tasks = self.tasks.lock();
            tasks.retain(|t| !t.is_finished());
            tasks.push(task.abort_handle());
        }
        *self.edit_cancel.lock() = Some(client.cancel_token());
        let outcome = self.apply_on(&client, schema, rows).await;
        *self.edit_cancel.lock() = None;
        task.abort();
        outcome
    }

    async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
        let rows=self.metadata().await?.query("SELECT pid,backend_start::text || '|' || COALESCE(query_start::text,''),COALESCE(usename::text,''),COALESCE(datname,''),COALESCE(state,''),COALESCE(EXTRACT(EPOCH FROM now()-query_start)::float8,0),left(query,4096) FROM pg_catalog.pg_stat_activity WHERE backend_type='client backend' ORDER BY query_start NULLS LAST LIMIT 500",&[]).await.map_err(pg_error)?;
        Ok(rows
            .iter()
            .map(|r| {
                let pid: i32 = r.get(0);
                ProcessInfo {
                    id: pid.to_string(),
                    identity: r.get(1),
                    user: r.get(2),
                    database: r.get(3),
                    state: r.get(4),
                    duration_seconds: r.get(5),
                    query: r.get(6),
                    own_session: pid == self.own_pid,
                }
            })
            .collect())
    }
    async fn cancel_process(&self, id: &str, identity: &str) -> AppResult<()> {
        let pid = id
            .parse::<i32>()
            .map_err(|_| AppError::invalid("Invalid process identifier"))?;
        let row=self.metadata().await?.query_opt("SELECT pg_catalog.pg_cancel_backend(pid) FROM pg_catalog.pg_stat_activity WHERE pid=$1 AND backend_start::text || '|' || COALESCE(query_start::text,'')=$2 AND state='active'",&[&pid,&identity]).await.map_err(pg_error)?;
        if row.is_none_or(|row| !row.get::<_, bool>(0)) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The selected query finished, changed, or could not be cancelled. Refresh the process list.",
            ));
        }
        Ok(())
    }
    async fn health(&self) -> AppResult<u32> {
        let start = Instant::now();
        self.metadata()
            .await?
            .simple_query("SELECT 1")
            .await
            .map_err(pg_error)?;
        Ok(start.elapsed().as_millis() as u32)
    }
    fn is_closed(&self) -> bool {
        self.client.is_closed()
    }
    fn close(&self) {
        for task in self.tasks.lock().drain(..) {
            task.abort();
        }
    }
}

impl Postgres {
    async fn apply_on(
        &self,
        client: &Client,
        schema: &TableSchema,
        rows: &[RowMutation],
    ) -> AppResult<()> {
        client.batch_execute("BEGIN").await.map_err(pg_error)?;
        let result=async {
            for row in rows {
                let (predicate,keys)=key_predicate(schema,&row.original,1)?;
                let params=keys.iter().map(|v|v as &(dyn ToSql+Sync)).collect::<Vec<_>>();
                let sql=format!("SELECT {} FROM {} WHERE {predicate} FOR UPDATE",projection(schema),qualified(&schema.table));
                let locked=client.query(&sql,&params).await.map_err(pg_error)?;
                if locked.len()!=1 { return Err(AppError::new(ErrorCode::Conflict,"A target row was deleted or its key changed. Refresh before applying edits.")); }
                let actual=(0..locked[0].len()).map(|i|locked[0].get::<_,Option<String>>(i)).collect::<TextRow>();
                if actual!=row.original { return Err(AppError::new(ErrorCode::Conflict,"A row changed since it was loaded. No edits were committed.")); }
                let (sql,values)=update_sql(schema,row)?;
                let params=values.iter().map(|v|v as &(dyn ToSql+Sync)).collect::<Vec<_>>();
                if client.execute(&sql,&params).await.map_err(pg_error)?!=1 { return Err(AppError::new(ErrorCode::Conflict,"An update did not match exactly one row. No edits were committed.")); }
            }
            Ok(())
        }.await;
        if let Err(error) = result {
            if client.batch_execute("ROLLBACK").await.is_err() {
                self.close();
            }
            return Err(error);
        }
        match client.batch_execute("COMMIT").await {
            Ok(()) => Ok(()),
            Err(error) if error.is_closed() || error.as_db_error().is_none() => {
                self.close();
                Err(AppError::new(
                    ErrorCode::OutcomeUnknown,
                    "The connection was lost while committing. Verify the database before retrying these edits.",
                ))
            }
            Err(error) => {
                let _ = client.batch_execute("ROLLBACK").await;
                Err(pg_error(error))
            }
        }
    }
}

impl Drop for Postgres {
    fn drop(&mut self) {
        for task in self.tasks.get_mut().drain(..) {
            task.abort();
        }
    }
}

pub fn qualified(table: &TableRef) -> String {
    format!(
        "{}.{}",
        quote_identifier(&table.schema),
        quote_identifier(&table.name)
    )
}

/// Browse SQL aliases `column::text` to `__joinery_cN` and keeps the base
/// table unaliased so user `WHERE` / `ORDER BY` expressions resolve typed
/// columns both unqualified and qualified (`table.col`).
pub(crate) struct PostgresDialect<'a>(pub &'a TableRef);

impl browse_query::Dialect for PostgresDialect<'_> {
    fn engine(&self) -> EngineKind {
        EngineKind::Postgres
    }
    fn column(&self, name: &str) -> String {
        format!("{}.{}", qualified(self.0), quote_identifier(name))
    }
    fn typed_bind(&self, column: &ColumnInfo, index: usize) -> Option<String> {
        cast_type(column).map(|cast| format!("(${}::text)::{cast}", index + 1))
    }
    fn like(&self, column: &ColumnInfo, index: usize) -> String {
        format!(
            "{}::text ILIKE ${}::text{}",
            self.column(&column.name),
            index + 1,
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
    let dialect = PostgresDialect(&schema.table);
    let built = browse_query::build(schema, query, after, &dialect)?;
    let table = qualified(&schema.table);
    let projection = schema
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                "{table}.{}::text AS __joinery_c{i}",
                quote_identifier(&c.name)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT {projection} FROM {table}{}{} LIMIT 501",
        built.where_sql, built.order_sql
    );
    Ok((sql, built))
}

fn projection(schema: &TableSchema) -> String {
    schema
        .columns
        .iter()
        .map(|c| format!("{}::text", quote_identifier(&c.name)))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn cast_type(column: &ColumnInfo) -> Option<&'static str> {
    let t = column.database_type.as_str();
    if ["smallint", "integer", "bigint", "int2", "int4", "int8"].contains(&t) {
        Some("bigint")
    } else if t.starts_with("numeric") || t == "decimal" {
        Some("numeric")
    } else if ["real", "double precision", "float4", "float8"].contains(&t) {
        Some("double precision")
    } else if t == "boolean" || t == "bool" {
        Some("boolean")
    } else if t == "uuid" {
        Some("uuid")
    } else if t == "date" {
        Some("date")
    } else if t.starts_with("timestamp") && t.contains("with time zone") {
        Some("timestamptz")
    } else if t.starts_with("timestamp") {
        Some("timestamp")
    } else if t == "jsonb" {
        Some("jsonb")
    } else if t == "json" {
        Some("json")
    } else if t == "text" || t.starts_with("character") || ["varchar", "bpchar"].contains(&t) {
        Some("text")
    } else {
        None
    }
}

fn key_predicate(
    schema: &TableSchema,
    original: &TextRow,
    offset: usize,
) -> AppResult<(String, Vec<Option<String>>)> {
    let mut values = Vec::new();
    let mut parts = Vec::new();
    for key in &schema.primary_key {
        let index = schema
            .columns
            .iter()
            .position(|c| &c.name == key)
            .ok_or_else(|| AppError::invalid("Missing primary key"))?;
        let cast = cast_type(&schema.columns[index])
            .ok_or_else(|| AppError::new(ErrorCode::Unsupported, "Unsupported primary key type"))?;
        parts.push(format!(
            "{}=(${}::text)::{cast}",
            quote_identifier(key),
            offset + values.len()
        ));
        values.push(original[index].clone());
    }
    if parts.is_empty() {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "A real primary key is required for edits",
        ));
    }
    Ok((parts.join(" AND "), values))
}

pub fn update_sql(
    schema: &TableSchema,
    row: &RowMutation,
) -> AppResult<(String, Vec<Option<String>>)> {
    let mut assignments = Vec::new();
    let mut values = Vec::new();
    for (index, value) in &row.changes {
        let column = schema
            .columns
            .get(*index)
            .ok_or_else(|| AppError::invalid("Invalid edited column"))?;
        if !column.editable {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This column is read-only",
            ));
        }
        let cast = cast_type(column).ok_or_else(|| {
            AppError::new(ErrorCode::Unsupported, "This data type cannot be edited")
        })?;
        assignments.push(format!(
            "{}=(${}::text)::{cast}",
            quote_identifier(&column.name),
            values.len() + 1
        ));
        values.push(value.clone());
    }
    let (predicate, keys) = key_predicate(schema, &row.original, values.len() + 1)?;
    values.extend(keys);
    Ok((
        format!(
            "UPDATE {} SET {} WHERE {predicate}",
            qualified(&schema.table),
            assignments.join(", ")
        ),
        values,
    ))
}

fn column(index: usize, name: &str, type_name: &str) -> ColumnInfo {
    let kind = match type_name {
        "bool" => ColumnKind::Boolean,
        "int2" | "int4" | "int8" | "oid" => ColumnKind::Integer,
        "numeric" | "float4" | "float8" => ColumnKind::Decimal,
        "json" | "jsonb" => ColumnKind::Json,
        "date" | "time" | "timetz" | "timestamp" | "timestamptz" | "interval" => {
            ColumnKind::Temporal
        }
        "bytea" => ColumnKind::Binary,
        "text" | "varchar" | "bpchar" | "name" | "uuid" => ColumnKind::Text,
        _ => ColumnKind::Unknown,
    };
    ColumnInfo {
        id: format!("c{index}"),
        name: name.into(),
        database_type: type_name.into(),
        kind,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: false,
    }
}

fn pg_error(error: tokio_postgres::Error) -> AppError {
    if let Some(db) = error.as_db_error() {
        let code = match db.code().code() {
            "57014" => ErrorCode::Cancelled,
            "42501" => ErrorCode::PermissionDenied,
            _ => ErrorCode::Database,
        };
        AppError::new(code, db.message())
    } else if error.is_closed() {
        AppError::new(
            ErrorCode::ConnectionClosed,
            "The database connection closed",
        )
    } else {
        AppError::new(
            ErrorCode::ConnectionFailed,
            format!("Could not communicate with PostgreSQL: {error}"),
        )
    }
}

async fn schema_on(client: &Client, table: &TableRef, completion: bool) -> AppResult<TableSchema> {
    let qualified = qualified(table);
    let rows=client.query(
            "SELECT a.attname,pg_catalog.format_type(a.atttypid,a.atttypmod),t.typname,a.attnotnull,a.attgenerated::pg_catalog.text,a.attidentity::pg_catalog.text,
              EXISTS(SELECT 1 FROM pg_catalog.pg_constraint k WHERE k.conrelid=a.attrelid AND k.contype='p' AND a.attnum=ANY(k.conkey)),
              EXISTS(SELECT 1 FROM pg_catalog.pg_constraint k WHERE k.conrelid=a.attrelid AND k.contype='f' AND a.attnum=ANY(k.conkey)), c.relkind::pg_catalog.text
             FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_type t ON t.oid=a.atttypid JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
             WHERE a.attrelid=pg_catalog.to_regclass($1) AND a.attnum>0 AND NOT a.attisdropped AND (NOT $2 OR (pg_catalog.has_schema_privilege(c.relnamespace,'USAGE') AND pg_catalog.has_column_privilege(c.oid,a.attnum,'SELECT'))) ORDER BY a.attnum",&[&qualified, &completion]).await.map_err(pg_error)?;
    if rows.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The table is missing or its schema is unavailable",
        ));
    }
    let physical = matches!(rows[0].get::<_, String>(8).as_str(), "r" | "p");
    let columns: Vec<ColumnInfo> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut c = column(i, r.get::<_, &str>(0), r.get::<_, &str>(2));
            c.database_type = r.get(1);
            c.nullable = !r.get::<_, bool>(3);
            c.primary_key = r.get(6);
            c.foreign_key = r.get(7);
            c.editable = !completion
                && physical
                && !c.primary_key
                && r.get::<_, String>(4).is_empty()
                && r.get::<_, String>(5).is_empty()
                && cast_type(&c).is_some();
            c
        })
        .collect();
    let primary_key = columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.clone())
        .collect::<Vec<_>>();
    if completion {
        return Ok(TableSchema {
            table: table.clone(),
            columns,
            primary_key,
            foreign_keys: vec![],
            editable: false,
        });
    }
    let foreign=client.query(
            "SELECT k.conname::pg_catalog.text,a.attname,n.nspname,c.relname,b.attname FROM pg_catalog.pg_constraint k
             CROSS JOIN LATERAL ROWS FROM (pg_catalog.unnest(k.conkey),pg_catalog.unnest(k.confkey)) WITH ORDINALITY AS x(src,dst,ord)
             JOIN pg_catalog.pg_attribute a ON a.attrelid=k.conrelid AND a.attnum=x.src
             JOIN pg_catalog.pg_class c ON c.oid=k.confrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
             JOIN pg_catalog.pg_attribute b ON b.attrelid=k.confrelid AND b.attnum=x.dst
             WHERE k.contype='f' AND k.conrelid=pg_catalog.to_regclass($1) ORDER BY k.conname,k.oid,x.ord",&[&qualified]).await.map_err(pg_error)?;
    let foreign_keys = ForeignKey::group(foreign.iter().map(|r| {
        (
            r.get(0),
            r.get(1),
            TableRef {
                schema: r.get(2),
                name: r.get(3),
            },
            r.get(4),
        )
    }));
    let editable = physical
        && !primary_key.is_empty()
        && columns
            .iter()
            .filter(|c| c.primary_key)
            .all(|c| cast_type(c).is_some());
    Ok(TableSchema {
        table: table.clone(),
        columns,
        primary_key,
        foreign_keys,
        editable,
    })
}
