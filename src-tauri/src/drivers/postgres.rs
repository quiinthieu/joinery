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

mod explorer;

pub struct Postgres {
    client: Client,
    metadata: OnceCell<Client>,
    config: Config,
    tls: MakeRustlsConnect,
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
    edit_cancel: Mutex<Option<tokio_postgres::CancelToken>>,
    tx_mode: Mutex<TransactionMode>,
    cached_identity: Mutex<CompletionIdentity>,
    stale_prepared: Mutex<Vec<String>>,
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
            .query_one(
                "SELECT version(),pg_backend_pid(),pg_catalog.current_database()::pg_catalog.text,current_user::pg_catalog.text,pg_catalog.current_schemas(true)::pg_catalog.text[]",
                &[],
            )
            .await
            .map_err(pg_error)?;
        let initial_identity = CompletionIdentity {
            database: row.get(2),
            role: row.get(3),
            search_path: row.get(4),
            case_sensitive_table_names: None,
        };
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
                object_explorer: true,
                schema_diagram: true,
                schema_diff: true,
                ddl_editing: true,
                lock_inspection: true,
                explain_analyze: true,
                oauth_browser: false,
                actual_plan: Some(ActualPlanFormat::PostgresJson),
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
            tx_mode: Mutex::new(TransactionMode::AutoCommit),
            cached_identity: Mutex::new(initial_identity),
            stale_prepared: Mutex::new(Vec::new()),
        };
        Ok((Arc::new(driver), info))
    }

    async fn observe_transaction_state(&self) -> AppResult<TransactionState> {
        let meta = self.metadata().await?;
        let _ = meta
            .batch_execute("SELECT pg_catalog.pg_stat_clear_snapshot()")
            .await;
        let row = meta
            .query_opt(
                "SELECT COALESCE(state, 'idle') FROM pg_catalog.pg_stat_activity WHERE pid = $1",
                &[&self.own_pid],
            )
            .await
            .map_err(pg_error)?;
        Ok(match row.as_ref().map(|r| r.get::<_, &str>(0)) {
            Some("idle in transaction") => TransactionState::InTransaction,
            Some("idle in transaction (aborted)") => TransactionState::Failed,
            _ => TransactionState::Idle,
        })
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
    async fn drain_stale_prepared(&self) {
        let names = std::mem::take(&mut *self.stale_prepared.lock());
        for name in names {
            let _ = self
                .client
                .batch_execute(&format!("DEALLOCATE {name}"))
                .await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_simple_sql(
        &self,
        wire_sql: &str,
        original_sql: &str,
        rewritten_sql: &str,
        rewrite_map: Option<&crate::sql_text::RewriteMap>,
        kind: StatementKind,
        metadata: Option<Vec<ColumnInfo>>,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        let stream = self.client.simple_query_raw(wire_sql).await.map_err(|e| {
            pg_sql_error_mapped(
                e,
                original_sql,
                rewritten_sql,
                rewrite_map,
                if wire_sql == rewritten_sql {
                    0
                } else {
                    isize::MAX
                },
            )
        })?;
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
                Err(error) => {
                    return Err(pg_sql_error_mapped(
                        error,
                        original_sql,
                        rewritten_sql,
                        rewrite_map,
                        if wire_sql == rewritten_sql {
                            0
                        } else {
                            isize::MAX
                        },
                    ));
                }
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
}

#[async_trait]
impl Driver for Postgres {
    async fn execute(
        &self,
        sql: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        if !self.stale_prepared.lock().is_empty()
            && self.observe_transaction_state().await? != TransactionState::Failed
        {
            self.drain_stale_prepared().await;
        }
        if *self.tx_mode.lock() == TransactionMode::Manual
            && should_auto_begin_postgres(sql, kind)
            && self.observe_transaction_state().await? == TransactionState::Idle
        {
            self.client.batch_execute("BEGIN").await.map_err(pg_error)?;
        }
        let metadata = if kind == StatementKind::Read {
            Some(
                self.client
                    .prepare(sql)
                    .await
                    .map_err(|e| pg_sql_error(e, sql))?
                    .columns()
                    .iter()
                    .enumerate()
                    .map(|(i, c)| column(i, c.name(), c.type_().name()))
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        self.stream_simple_sql(sql, sql, sql, None, kind, metadata, sink)
            .await
    }

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
        if !self.stale_prepared.lock().is_empty()
            && self.observe_transaction_state().await? != TransactionState::Failed
        {
            self.drain_stale_prepared().await;
        }
        if *self.tx_mode.lock() == TransactionMode::Manual
            && should_auto_begin_postgres(&bound.rewritten_sql, kind)
            && self.observe_transaction_state().await? == TransactionState::Idle
        {
            self.client.batch_execute("BEGIN").await.map_err(pg_error)?;
        }

        use tokio_postgres::types::Type;
        let mut pg_types = Vec::with_capacity(bound.binds.len());
        let mut sql_types = Vec::with_capacity(bound.binds.len());
        let mut exec_args = Vec::with_capacity(bound.binds.len());
        for bind in &bound.binds {
            let (ty, sql_ty) = match bind.kind {
                QueryParamKind::Text => (Type::TEXT, "pg_catalog.text"),
                QueryParamKind::Integer => (Type::INT8, "pg_catalog.int8"),
                QueryParamKind::Decimal => (Type::NUMERIC, "pg_catalog.numeric"),
                QueryParamKind::Boolean => (Type::BOOL, "pg_catalog.bool"),
                QueryParamKind::Uuid => (Type::UUID, "pg_catalog.uuid"),
                QueryParamKind::Date => (Type::DATE, "pg_catalog.date"),
                QueryParamKind::Timestamp => (Type::TIMESTAMPTZ, "pg_catalog.timestamptz"),
                QueryParamKind::Json => (Type::JSONB, "pg_catalog.jsonb"),
            };
            pg_types.push(ty);
            sql_types.push(sql_ty);
            match &bind.value {
                None => exec_args.push(format!("NULL::{sql_ty}")),
                Some(val) => {
                    let hex_bytes = hex::encode(val.as_bytes());
                    exec_args.push(format!(
                        "pg_catalog.convert_from(pg_catalog.decode('{hex_bytes}', 'hex'), 'UTF8')::{sql_ty}"
                    ));
                }
            }
        }

        let metadata = if kind == StatementKind::Read {
            Some(
                self.client
                    .prepare_typed(&bound.rewritten_sql, &pg_types)
                    .await
                    .map_err(|e| {
                        pg_sql_error_mapped(
                            e,
                            sql,
                            &bound.rewritten_sql,
                            Some(&bound.rewrite_map),
                            0,
                        )
                    })?
                    .columns()
                    .iter()
                    .enumerate()
                    .map(|(i, c)| column(i, c.name(), c.type_().name()))
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };

        let stmt_name = format!("__joinery_p_{}", uuid::Uuid::new_v4().simple());
        let prefix = format!("PREPARE {stmt_name} ({}) AS ", sql_types.join(", "));
        // SQL PREPARE accepts the underlying DML, not EXPLAIN itself.
        // EXPLAIN EXECUTE keeps the exact options and executes these same binds.
        let (explain_prefix, inner_sql) = postgres_explain_parts(&bound.rewritten_sql)?;
        let prepare_sql = format!("{prefix}{inner_sql}");
        self.client.batch_execute(&prepare_sql).await.map_err(|e| {
            pg_sql_error_mapped(
                e,
                sql,
                &bound.rewritten_sql,
                Some(&bound.rewrite_map),
                prefix.chars().count() as isize - explain_prefix.chars().count() as isize,
            )
        })?;

        let execute_sql = format!(
            "{explain_prefix}EXECUTE {stmt_name}({})",
            exec_args.join(", ")
        );
        let result = self
            .stream_simple_sql(
                &execute_sql,
                sql,
                &bound.rewritten_sql,
                Some(&bound.rewrite_map),
                kind,
                metadata,
                sink,
            )
            .await;

        if !self.is_closed() {
            match self.observe_transaction_state().await {
                Ok(state) if state != TransactionState::Failed => {
                    let _ = self
                        .client
                        .batch_execute(&format!("DEALLOCATE {stmt_name}"))
                        .await;
                }
                _ => {
                    self.stale_prepared.lock().push(stmt_name);
                }
            }
        }

        result
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
        *self.cached_identity.lock() = identity.clone();
        Ok(identity)
    }

    async fn session_state(&self, session_id: &str) -> AppResult<SessionRuntimeState> {
        if self.is_closed() {
            return Err(AppError::new(
                ErrorCode::ConnectionClosed,
                "The database connection closed",
            ));
        }
        let transaction_state = self.observe_transaction_state().await?;
        let transaction_mode = *self.tx_mode.lock();
        let identity = if transaction_state == TransactionState::Failed {
            self.cached_identity.lock().clone()
        } else {
            self.completion_identity().await?
        };
        Ok(SessionRuntimeState {
            session_id: session_id.to_owned(),
            transaction_mode,
            transaction_state,
            context_kind: SessionContextKind::Schema,
            active_context: active_postgres_schema(&identity.search_path),
            database: identity.database,
            role: identity.role,
            search_path: identity.search_path,
        })
    }

    async fn list_contexts(&self) -> AppResult<Vec<String>> {
        let rows = self
            .metadata()
            .await?
            .query(
                "SELECT n.nspname
                 FROM pg_catalog.pg_namespace n
                 WHERE n.nspname NOT LIKE 'pg\\_toast%'
                   AND n.nspname NOT LIKE 'pg\\_temp%'
                   AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
                 ORDER BY CASE
                     WHEN n.nspname = 'public' THEN 0
                     WHEN n.nspname = 'pg_catalog' THEN 2
                     WHEN n.nspname = 'information_schema' THEN 3
                     ELSE 1
                 END, n.nspname
                 LIMIT 1000",
                &[],
            )
            .await
            .map_err(pg_error)?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    async fn switch_context(&self, target: &str) -> AppResult<()> {
        if target.is_empty() || target.chars().count() > 255 || target.contains('\0') {
            return Err(AppError::invalid("Provide a valid PostgreSQL schema name"));
        }
        if self.observe_transaction_state().await? == TransactionState::Failed {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Roll back the failed transaction before switching schemas",
            ));
        }
        let exists = self
            .client
            .query(
                "SELECT 1
                 FROM pg_catalog.pg_namespace n
                 WHERE n.nspname = $1
                   AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')",
                &[&target],
            )
            .await
            .map_err(pg_error)?;
        if exists.is_empty() {
            return Err(AppError::new(
                ErrorCode::NotFound,
                format!("Schema \"{target}\" does not exist or is not accessible"),
            ));
        }
        self.client
            .query_one(
                "SELECT pg_catalog.set_config(
                    'search_path',
                    pg_catalog.format('%I', $1::pg_catalog.text),
                    false
                )",
                &[&target],
            )
            .await
            .map_err(pg_error)?;
        let _ = self.completion_identity().await?;
        Ok(())
    }

    async fn set_transaction_mode(&self, mode: TransactionMode) -> AppResult<()> {
        let state = self.observe_transaction_state().await?;
        if mode == TransactionMode::AutoCommit && state != TransactionState::Idle {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Commit or roll back the open transaction before switching to auto-commit",
            ));
        }
        *self.tx_mode.lock() = mode;
        Ok(())
    }

    async fn commit_transaction(&self) -> AppResult<()> {
        match self.observe_transaction_state().await? {
            TransactionState::Idle => Err(AppError::new(
                ErrorCode::Conflict,
                "No transaction is currently open",
            )),
            TransactionState::Failed => Err(AppError::new(
                ErrorCode::Conflict,
                "This transaction has failed and can only be rolled back",
            )),
            TransactionState::InTransaction => {
                self.client.batch_execute("COMMIT").await.map_err(pg_error)
            }
        }
    }

    async fn rollback_transaction(&self) -> AppResult<()> {
        if self.observe_transaction_state().await? == TransactionState::Idle {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "No transaction is currently open",
            ));
        }
        self.client
            .batch_execute("ROLLBACK")
            .await
            .map_err(pg_error)?;
        self.drain_stale_prepared().await;
        Ok(())
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
    async fn inspection_schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        schema_on_mode(&self.client, table, true, true).await
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

    async fn database_objects(&self) -> AppResult<Vec<DatabaseObjectSummary>> {
        explorer::database_objects_on(self.metadata().await?).await
    }

    async fn object_details(&self, object: &DatabaseObjectRef) -> AppResult<ObjectDetails> {
        explorer::object_details_on(self.metadata().await?, object).await
    }

    async fn referencing_keys(&self, table: &TableRef) -> AppResult<Vec<InboundForeignKey>> {
        referencing_keys_on(self.metadata().await?, table).await
    }

    async fn schema_diagram(&self, schema: &str) -> AppResult<SchemaDiagram> {
        schema_diagram_on(self.metadata().await?, schema).await
    }

    async fn schema_snapshot(&self, schema: &str) -> AppResult<SchemaSnapshot> {
        explorer::schema_snapshot_on(self.metadata().await?, schema).await
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
    async fn lock_waits(&self) -> AppResult<LockWaitSnapshot> {
        let rows = self
            .metadata()
            .await?
            .query(
                "SELECT w.pid, COALESCE(w.usename::pg_catalog.text, ''),
                        COALESCE(EXTRACT(EPOCH FROM now() - COALESCE(w.state_change, w.query_start))::float8, 0),
                        COALESCE(l.locktype, ''), COALESCE(l.mode, ''),
                        rn.nspname::pg_catalog.text, rc.relname::pg_catalog.text,
                        NULLIF(pg_catalog.concat_ws(':', w.wait_event_type, w.wait_event), ''),
                        left(w.query, 4096),
                        pg_catalog.pg_blocking_pids(w.pid)
                 FROM pg_catalog.pg_stat_activity w
                 LEFT JOIN LATERAL (
                     SELECT k.locktype, k.mode, k.relation FROM pg_catalog.pg_locks k
                     WHERE k.pid = w.pid AND NOT k.granted LIMIT 1
                 ) l ON true
                 LEFT JOIN pg_catalog.pg_class rc ON rc.oid = l.relation
                 LEFT JOIN pg_catalog.pg_namespace rn ON rn.oid = rc.relnamespace
                 WHERE w.backend_type = 'client backend'
                   AND pg_catalog.cardinality(pg_catalog.pg_blocking_pids(w.pid)) > 0
                 ORDER BY 3 DESC, 1
                 LIMIT 501",
                &[],
            )
            .await
            .map_err(pg_error)?;
        let (waits, truncated) = LockWait::merge(rows.iter().map(|r| {
            let pid: i32 = r.get(0);
            let schema: Option<String> = r.get(5);
            let name: Option<String> = r.get(6);
            let blockers: Vec<i32> = r.get(9);
            LockWait {
                waiting_id: pid.to_string(),
                user: r.get(1),
                wait_seconds: r.get(2),
                lock_type: r.get(3),
                lock_mode: r.get(4),
                object: schema
                    .zip(name)
                    .map(|(schema, name)| TableRef { schema, name }),
                wait_event: r.get(7),
                query: r.get(8),
                blocking_ids: blockers.iter().map(i32::to_string).collect(),
                own_session: pid == self.own_pid,
            }
        }));
        Ok(LockWaitSnapshot { waits, truncated })
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
        let result = async {
            for row in rows {
                if let RowMutation::Update { original, .. } | RowMutation::Delete { original } = row
                {
                    let (predicate, keys) = key_predicate(schema, original, 1)?;
                    let params = keys
                        .iter()
                        .map(|v| v as &(dyn ToSql + Sync))
                        .collect::<Vec<_>>();
                    let sql = format!(
                        "SELECT {} FROM {} WHERE {predicate} FOR UPDATE",
                        projection(schema),
                        qualified(&schema.table)
                    );
                    let locked = client.query(&sql, &params).await.map_err(pg_error)?;
                    if locked.len() != 1 {
                        return Err(AppError::new(
                            ErrorCode::Conflict,
                            "A target row was deleted or its key changed. Refresh before applying edits.",
                        ));
                    }
                    let actual = (0..locked[0].len())
                        .map(|i| locked[0].get::<_, Option<String>>(i))
                        .collect::<TextRow>();
                    if &actual != original {
                        return Err(AppError::new(
                            ErrorCode::Conflict,
                            "A row changed since it was loaded. No edits were committed.",
                        ));
                    }
                }
                let (sql, values) = mutation_sql(schema, row)?;
                let params = values
                    .iter()
                    .map(|v| v as &(dyn ToSql + Sync))
                    .collect::<Vec<_>>();
                if client.execute(&sql, &params).await.map_err(pg_error)? != 1 {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "A table change did not affect exactly one row. No edits were committed.",
                    ));
                }
            }
            Ok(())
        }
        .await;
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

pub fn mutation_sql(
    schema: &TableSchema,
    row: &RowMutation,
) -> AppResult<(String, Vec<Option<String>>)> {
    if !schema.editable {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "This table cannot be edited safely",
        ));
    }
    match row {
        RowMutation::Update { original, changes } => {
            if changes.is_empty() {
                return Err(AppError::new(
                    ErrorCode::PolicyBlocked,
                    "This table cannot be edited safely",
                ));
            }
            let mut assignments = Vec::new();
            let mut values = Vec::new();
            for (index, value) in changes {
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
            let (predicate, keys) = key_predicate(schema, original, values.len() + 1)?;
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
        RowMutation::Delete { original } => {
            let (predicate, keys) = key_predicate(schema, original, 1)?;
            Ok((
                format!("DELETE FROM {} WHERE {predicate}", qualified(&schema.table)),
                keys,
            ))
        }
        RowMutation::Insert { values } => {
            if values.is_empty() {
                return Ok((
                    format!("INSERT INTO {} DEFAULT VALUES", qualified(&schema.table)),
                    Vec::new(),
                ));
            }
            let mut columns = Vec::new();
            let mut expressions = Vec::new();
            let mut binds = Vec::new();
            for (index, value) in values {
                let column = schema
                    .columns
                    .get(*index)
                    .ok_or_else(|| AppError::invalid("Invalid inserted column"))?;
                if !column.insertable {
                    return Err(AppError::new(
                        ErrorCode::PolicyBlocked,
                        "This column cannot be set on insert",
                    ));
                }
                if !column.nullable && value.is_none() {
                    return Err(AppError::invalid("This column does not allow NULL"));
                }
                let cast = cast_type(column).ok_or_else(|| {
                    AppError::new(ErrorCode::Unsupported, "This data type cannot be inserted")
                })?;
                columns.push(quote_identifier(&column.name));
                expressions.push(format!("(${}::text)::{cast}", binds.len() + 1));
                binds.push(value.clone());
            }
            Ok((
                format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    qualified(&schema.table),
                    columns.join(", "),
                    expressions.join(", ")
                ),
                binds,
            ))
        }
    }
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
        insertable: false,
        has_default: false,
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

pub(crate) fn pg_sql_error(error: tokio_postgres::Error, sql: &str) -> AppError {
    pg_sql_error_mapped(error, sql, sql, None, 0)
}

fn postgres_explain_parts(sql: &str) -> AppResult<(&str, &str)> {
    use sqlparser::{
        dialect::PostgreSqlDialect,
        tokenizer::{Token, Tokenizer},
    };
    let tokens = Tokenizer::new(&PostgreSqlDialect {}, sql)
        .tokenize_with_location()
        .map_err(|_| AppError::invalid("Cannot tokenize parameterized SQL"))?;
    let mut words = tokens.iter().filter_map(|token| {
        if let Token::Word(word) = &token.token {
            Some((word, token.span.start))
        } else {
            None
        }
    });
    if !words.next().is_some_and(|(word, _)| {
        word.quote_style.is_none() && word.value.eq_ignore_ascii_case("EXPLAIN")
    }) {
        return Ok(("", sql));
    }
    for (word, location) in words {
        if word.quote_style.is_none()
            && [
                "SELECT", "WITH", "VALUES", "TABLE", "INSERT", "UPDATE", "DELETE", "MERGE",
            ]
            .iter()
            .any(|candidate| word.value.eq_ignore_ascii_case(candidate))
        {
            let byte = crate::safety::byte_offset(sql, location);
            return Ok((&sql[..byte], &sql[byte..]));
        }
    }
    Err(AppError::new(
        ErrorCode::Unsupported,
        "Parameterized EXPLAIN requires a supported query or mutation",
    ))
}

pub(crate) fn pg_sql_error_mapped(
    error: tokio_postgres::Error,
    original_sql: &str,
    rewritten_sql: &str,
    rewrite_map: Option<&crate::sql_text::RewriteMap>,
    prefix_chars: isize,
) -> AppError {
    if let Some(db) = error.as_db_error() {
        let state = db.code().code();
        let code = match state {
            "57014" => ErrorCode::Cancelled,
            "42501" => ErrorCode::PermissionDenied,
            _ => ErrorCode::Database,
        };
        let base = AppError::new(code, db.message());
        if code == ErrorCode::Cancelled {
            return base;
        }
        let span = match db.position() {
            Some(tokio_postgres::error::ErrorPosition::Original(pos)) => {
                let adjusted = (*pos as isize - prefix_chars).max(0) as usize;
                if adjusted == 0 {
                    crate::sql_text::trimmed_span(original_sql)
                } else if let Some(map) = rewrite_map {
                    map.span_from_rewritten_pg_char_pos(
                        original_sql,
                        rewritten_sql,
                        adjusted as u32,
                    )
                } else {
                    crate::sql_text::span_from_pg_char_pos(original_sql, adjusted as u32)
                }
            }
            Some(tokio_postgres::error::ErrorPosition::Internal { .. }) | None => {
                crate::sql_text::trimmed_span(original_sql)
            }
        };
        base.with_location(ErrorLocation {
            span,
            statement_index: None,
            sql_state: Some(state.to_owned()),
            detail: db.detail().filter(|s| !s.is_empty()).map(str::to_owned),
            hint: db.hint().filter(|s| !s.is_empty()).map(str::to_owned),
        })
    } else {
        pg_error(error)
    }
}

async fn schema_on(client: &Client, table: &TableRef, completion: bool) -> AppResult<TableSchema> {
    schema_on_mode(client, table, completion, false).await
}

async fn schema_on_mode(
    client: &Client,
    table: &TableRef,
    completion: bool,
    require_complete: bool,
) -> AppResult<TableSchema> {
    let qualified = qualified(table);
    let rows=client.query(
            "SELECT a.attname,pg_catalog.format_type(a.atttypid,a.atttypmod),t.typname,a.attnotnull,a.attgenerated::pg_catalog.text,a.attidentity::pg_catalog.text,
              EXISTS(SELECT 1 FROM pg_catalog.pg_constraint k WHERE k.conrelid=a.attrelid AND k.contype='p' AND a.attnum=ANY(k.conkey)),
              EXISTS(SELECT 1 FROM pg_catalog.pg_constraint k WHERE k.conrelid=a.attrelid AND k.contype='f' AND a.attnum=ANY(k.conkey)), c.relkind::pg_catalog.text,
              a.atthasdef, pg_catalog.has_schema_privilege(c.relnamespace,'USAGE') AND pg_catalog.has_column_privilege(c.oid,a.attnum,'SELECT')
             FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_type t ON t.oid=a.atttypid JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
             WHERE a.attrelid=pg_catalog.to_regclass($1) AND a.attnum>0 AND NOT a.attisdropped AND (NOT $2 OR $3 OR (pg_catalog.has_schema_privilege(c.relnamespace,'USAGE') AND pg_catalog.has_column_privilege(c.oid,a.attnum,'SELECT'))) ORDER BY a.attnum",&[&qualified, &completion, &require_complete]).await.map_err(pg_error)?;
    if require_complete && rows.iter().any(|row| !row.get::<_, bool>(10)) {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Complete selectable column metadata is unavailable; no column claim was made",
        ));
    }
    if rows.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The table is missing or its schema is unavailable",
        ));
    }
    let physical = matches!(rows[0].get::<_, String>(8).as_str(), "r" | "p");
    if require_complete && !physical {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Column inspection requires a direct base table",
        ));
    }
    let mut columns: Vec<ColumnInfo> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut c = column(i, r.get::<_, &str>(0), r.get::<_, &str>(2));
            let generated = r.get::<_, String>(4);
            let identity = r.get::<_, String>(5);
            let has_def: bool = r.get(9);
            c.database_type = r.get(1);
            c.nullable = !r.get::<_, bool>(3);
            c.primary_key = r.get(6);
            c.foreign_key = r.get(7);
            c.has_default = has_def || !identity.is_empty() || !generated.is_empty();
            c.editable = !completion
                && physical
                && !c.primary_key
                && generated.is_empty()
                && identity.is_empty()
                && cast_type(&c).is_some();
            c.insertable = !completion
                && physical
                && generated.is_empty()
                && identity != "a"
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
        let (foreign_keys, inbound_foreign_keys) = completion_fks_on(client, &qualified).await?;
        return Ok(TableSchema {
            table: table.clone(),
            columns,
            primary_key,
            foreign_keys,
            inbound_foreign_keys,
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

async fn completion_fks_on(
    client: &Client,
    qualified: &str,
) -> AppResult<(Vec<ForeignKey>, Vec<InboundForeignKey>)> {
    let outbound_rows = client
        .query(
            "SELECT k.conname::pg_catalog.text, a.attname, n.nspname, c.relname, b.attname,
                    pg_catalog.cardinality(k.conkey)::pg_catalog.int4
             FROM pg_catalog.pg_constraint k
             CROSS JOIN LATERAL ROWS FROM (pg_catalog.unnest(k.conkey), pg_catalog.unnest(k.confkey)) WITH ORDINALITY AS x(src, dst, ord)
             JOIN pg_catalog.pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = x.src AND NOT a.attisdropped
             JOIN pg_catalog.pg_class c ON c.oid = k.confrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_attribute b ON b.attrelid = k.confrelid AND b.attnum = x.dst AND NOT b.attisdropped
             WHERE k.contype = 'f'
               AND k.conrelid = pg_catalog.to_regclass($1)
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
               AND pg_catalog.has_column_privilege(k.conrelid, a.attnum, 'SELECT')
               AND pg_catalog.has_column_privilege(k.confrelid, b.attnum, 'SELECT')
             ORDER BY k.conname, k.oid, x.ord
             LIMIT 200",
            &[&qualified],
        )
        .await
        .map_err(pg_error)?;
    let mut outbound_widths: std::collections::HashMap<(String, String, String), usize> =
        std::collections::HashMap::new();
    let outbound = ForeignKey::group(outbound_rows.iter().map(|r| {
        let name: String = r.get(0);
        let target = TableRef {
            schema: r.get(2),
            name: r.get(3),
        };
        let expected: i32 = r.get(5);
        outbound_widths.insert(
            (target.schema.clone(), target.name.clone(), name.clone()),
            expected.max(0) as usize,
        );
        (name, r.get(1), target, r.get(4))
    }))
    .into_iter()
    .filter(|key| {
        outbound_widths
            .get(&(
                key.target.schema.clone(),
                key.target.name.clone(),
                key.name.clone(),
            ))
            .is_none_or(|&w| key.columns.len() == w)
    })
    .collect();

    let inbound_rows = client
        .query(
            "SELECT k.conname::pg_catalog.text, n.nspname, c.relname, a.attname, b.attname,
                    pg_catalog.cardinality(k.conkey)::pg_catalog.int4
             FROM pg_catalog.pg_constraint k
             CROSS JOIN LATERAL ROWS FROM (pg_catalog.unnest(k.conkey), pg_catalog.unnest(k.confkey)) WITH ORDINALITY AS x(src, dst, ord)
             JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = x.src AND NOT a.attisdropped
             JOIN pg_catalog.pg_attribute b ON b.attrelid = k.confrelid AND b.attnum = x.dst AND NOT b.attisdropped
             WHERE k.contype = 'f'
               AND k.confrelid = pg_catalog.to_regclass($1)
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
               AND pg_catalog.has_column_privilege(k.conrelid, a.attnum, 'SELECT')
               AND pg_catalog.has_column_privilege(k.confrelid, b.attnum, 'SELECT')
             ORDER BY n.nspname, c.relname, k.conname, k.oid, x.ord
             LIMIT 200",
            &[&qualified],
        )
        .await
        .map_err(pg_error)?;
    let mut inbound_widths: std::collections::HashMap<(String, String, String), usize> =
        std::collections::HashMap::new();
    let inbound = InboundForeignKey::group(inbound_rows.iter().map(|r| {
        let name: String = r.get(0);
        let source = TableRef {
            schema: r.get(1),
            name: r.get(2),
        };
        let expected: i32 = r.get(5);
        inbound_widths.insert(
            (source.schema.clone(), source.name.clone(), name.clone()),
            expected.max(0) as usize,
        );
        (name, source, r.get(3), r.get(4))
    }))
    .into_iter()
    .filter(|key| {
        inbound_widths
            .get(&(
                key.source.schema.clone(),
                key.source.name.clone(),
                key.name.clone(),
            ))
            .is_none_or(|&w| key.columns.len() == w)
    })
    .collect();

    Ok((outbound, inbound))
}

async fn referencing_keys_on(
    client: &Client,
    table: &TableRef,
) -> AppResult<Vec<InboundForeignKey>> {
    let exists = client
        .query(
            "SELECT 1 FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relname = $2
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')",
            &[&table.schema, &table.name],
        )
        .await
        .map_err(pg_error)?;
    if exists.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The table is missing or its schema is unavailable",
        ));
    }
    let rows = client
        .query(
            "SELECT k.conname::pg_catalog.text, n.nspname, c.relname, a.attname, b.attname,
                    pg_catalog.cardinality(k.conkey)::pg_catalog.int4
             FROM pg_catalog.pg_constraint k
             JOIN pg_catalog.pg_class t ON t.oid = k.confrelid
             JOIN pg_catalog.pg_namespace tn ON tn.oid = t.relnamespace
             CROSS JOIN LATERAL ROWS FROM (pg_catalog.unnest(k.conkey), pg_catalog.unnest(k.confkey)) WITH ORDINALITY AS x(src, dst, ord)
             JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = x.src AND NOT a.attisdropped
             JOIN pg_catalog.pg_attribute b ON b.attrelid = k.confrelid AND b.attnum = x.dst AND NOT b.attisdropped
             WHERE k.contype = 'f'
               AND tn.nspname = $1 AND t.relname = $2
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
               AND pg_catalog.has_table_privilege(c.oid, 'SELECT')
             ORDER BY n.nspname, c.relname, k.conname, k.oid, x.ord
             LIMIT 500",
            &[&table.schema, &table.name],
        )
        .await
        .map_err(pg_error)?;
    let mut widths: std::collections::HashMap<(String, String, String), usize> =
        std::collections::HashMap::new();
    let grouped = InboundForeignKey::group(rows.iter().map(|r| {
        let name: String = r.get(0);
        let source = TableRef {
            schema: r.get(1),
            name: r.get(2),
        };
        let expected: i32 = r.get(5);
        widths.insert(
            (source.schema.clone(), source.name.clone(), name.clone()),
            expected.max(0) as usize,
        );
        (name, source, r.get(3), r.get(4))
    }));
    Ok(grouped
        .into_iter()
        .filter(|key| {
            widths
                .get(&(
                    key.source.schema.clone(),
                    key.source.name.clone(),
                    key.name.clone(),
                ))
                .is_none_or(|&w| key.columns.len() == w)
        })
        .collect())
}

/// Reads relations, readable columns and outbound foreign keys for one schema.
/// Partitions and cloned partition constraints are omitted; constraints with
/// unreadable columns are withheld rather than shown partially.
async fn schema_diagram_on(client: &Client, schema: &str) -> AppResult<SchemaDiagram> {
    let visible = client
        .query(
            "SELECT 1 FROM pg_catalog.pg_namespace n
             WHERE n.nspname = $1 AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')",
            &[&schema],
        )
        .await
        .map_err(pg_error)?;
    if visible.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The schema is missing or unavailable",
        ));
    }
    let tables = client
        .query(
            "SELECT c.relname, c.relkind::pg_catalog.text
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relkind IN ('r','p','v','m','f') AND NOT c.relispartition
             ORDER BY c.relname
             LIMIT 501",
            &[&schema],
        )
        .await
        .map_err(pg_error)?;
    let columns = client
        .query(
            "SELECT c.relname, a.attname, pg_catalog.format_type(a.atttypid, a.atttypmod),
                    NOT a.attnotnull,
                    EXISTS (SELECT 1 FROM pg_catalog.pg_index i
                            WHERE i.indrelid = c.oid AND i.indisprimary AND a.attnum = ANY (i.indkey))
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
             WHERE n.nspname = $1 AND c.relkind IN ('r','p','v','m','f') AND NOT c.relispartition
               AND pg_catalog.has_column_privilege(c.oid, a.attnum, 'SELECT')
             ORDER BY c.relname, a.attnum
             LIMIT 20001",
            &[&schema],
        )
        .await
        .map_err(pg_error)?;
    let keys = client
        .query(
            "SELECT k.conname::pg_catalog.text, c.relname, tn.nspname, t.relname, a.attname, b.attname,
                    pg_catalog.cardinality(k.conkey)::pg_catalog.int4
             FROM pg_catalog.pg_constraint k
             JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_class t ON t.oid = k.confrelid
             JOIN pg_catalog.pg_namespace tn ON tn.oid = t.relnamespace
             CROSS JOIN LATERAL ROWS FROM (pg_catalog.unnest(k.conkey), pg_catalog.unnest(k.confkey)) WITH ORDINALITY AS x(src, dst, ord)
             JOIN pg_catalog.pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = x.src AND NOT a.attisdropped
             JOIN pg_catalog.pg_attribute b ON b.attrelid = k.confrelid AND b.attnum = x.dst AND NOT b.attisdropped
             WHERE k.contype = 'f' AND k.conparentid = 0
               AND n.nspname = $1 AND NOT c.relispartition
               AND pg_catalog.has_schema_privilege(tn.oid, 'USAGE')
               AND pg_catalog.has_column_privilege(k.conrelid, a.attnum, 'SELECT')
               AND pg_catalog.has_column_privilege(k.confrelid, b.attnum, 'SELECT')
             ORDER BY c.relname, k.conname, k.oid, x.ord
             LIMIT 20001",
            &[&schema],
        )
        .await
        .map_err(pg_error)?;
    Ok(SchemaDiagram::assemble(
        schema,
        tables.iter().map(|r| (r.get(0), r.get(1))).collect(),
        columns
            .iter()
            .map(|r| {
                (
                    r.get(0),
                    DiagramColumn {
                        name: r.get(1),
                        database_type: r.get(2),
                        nullable: r.get(3),
                        primary_key: r.get(4),
                    },
                )
            })
            .collect(),
        keys.iter()
            .map(|r| {
                let expected: i32 = r.get(6);
                DiagramKeyRow {
                    name: r.get(0),
                    source_table: r.get(1),
                    target: TableRef {
                        schema: r.get(2),
                        name: r.get(3),
                    },
                    column: r.get(4),
                    target_column: r.get(5),
                    expected: Some(expected.max(0) as usize),
                }
            })
            .collect(),
    ))
}

fn active_postgres_schema(search_path: &[String]) -> Option<String> {
    search_path
        .iter()
        .find(|s| s.as_str() != "pg_catalog" && !s.starts_with("pg_temp"))
        .or_else(|| search_path.first())
        .cloned()
}

fn postgres_keyword_tokens(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut i = 0usize;
    let mut tokens = Vec::new();
    while i < bytes.len() && tokens.len() < 16 {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                let mut depth = 1usize;
                while i + 1 < bytes.len() && depth > 0 {
                    if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                    } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'\'' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        i += 1;
                        if bytes.get(i) == Some(&b'\'') {
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'"' {
                        i += 1;
                        if bytes.get(i) == Some(&b'"') {
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'$' => {
                let tag_start = i;
                let mut end = i + 1;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                {
                    end += 1;
                }
                if end < bytes.len() && bytes[end] == b'$' {
                    let tag = &bytes[tag_start..=end];
                    i = end + 1;
                    while i + tag.len() <= bytes.len() && &bytes[i..i + tag.len()] != tag {
                        i += 1;
                    }
                    i = (i + tag.len()).min(bytes.len());
                } else {
                    i += 1;
                }
            }
            b if b.is_ascii_alphabetic() || b == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                tokens.push(sql[start..i].to_ascii_uppercase());
            }
            _ => i += 1,
        }
    }
    tokens
}

fn should_auto_begin_postgres(sql: &str, kind: StatementKind) -> bool {
    if kind == StatementKind::Transaction {
        return false;
    }
    let tokens = postgres_keyword_tokens(sql);
    let Some(first) = tokens.first().map(String::as_str) else {
        return false;
    };
    if matches!(
        first,
        "BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "ABORT" | "VACUUM" | "CLUSTER"
    ) {
        return false;
    }
    let second = tokens.get(1).map(String::as_str);
    if first == "ALTER" && second == Some("SYSTEM") {
        return false;
    }
    if matches!(first, "CREATE" | "DROP") && matches!(second, Some("DATABASE" | "TABLESPACE")) {
        return false;
    }
    if matches!(first, "CREATE" | "DROP" | "REINDEX") && tokens.iter().any(|t| t == "CONCURRENTLY")
    {
        return false;
    }
    true
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn active_schema_skips_implicit_pg_catalog_unless_explicit() {
        assert_eq!(
            active_postgres_schema(&["pg_catalog".into(), "public".into()]),
            Some("public".into())
        );
        assert_eq!(
            active_postgres_schema(&["pg_temp_4".into(), "pg_catalog".into(), "analytics".into()]),
            Some("analytics".into())
        );
        assert_eq!(
            active_postgres_schema(&["pg_catalog".into()]),
            Some("pg_catalog".into())
        );
        assert_eq!(active_postgres_schema(&[]), None);
    }

    #[test]
    fn auto_begin_skips_transaction_control_and_non_transactional_commands() {
        assert!(should_auto_begin_postgres(
            "SELECT * FROM orders",
            StatementKind::Read
        ));
        assert!(should_auto_begin_postgres(
            "/* comment */ INSERT INTO orders VALUES (1)",
            StatementKind::Write
        ));
        assert!(!should_auto_begin_postgres(
            "BEGIN",
            StatementKind::Transaction
        ));
        assert!(!should_auto_begin_postgres(
            "-- header\nVACUUM ANALYZE orders",
            StatementKind::Unknown
        ));
        assert!(!should_auto_begin_postgres(
            "CREATE INDEX CONCURRENTLY idx_orders ON orders (id)",
            StatementKind::Ddl
        ));
        assert!(!should_auto_begin_postgres(
            "CREATE DATABASE scratch",
            StatementKind::Ddl
        ));
        assert!(!should_auto_begin_postgres(
            "ALTER SYSTEM SET work_mem = '16MB'",
            StatementKind::Unknown
        ));
    }
}
