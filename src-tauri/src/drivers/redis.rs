use super::*;
use ::redis::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::TcpStream, sync::Mutex};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub mod commands;
mod protocol;
use protocol::Wire;

#[derive(Clone)]
struct Options {
    host: String,
    port: u16,
    dial_host: String,
    username: String,
    password: Option<Zeroizing<String>>,
    database: u32,
    tls: Option<Arc<rustls::ClientConfig>>,
}
struct Cursor {
    scope: String,
    value: u64,
    deadline: Instant,
}
pub struct Redis {
    options: Options,
    editor: Mutex<Option<Wire>>,
    closed: CancellationToken,
    cursors: parking_lot::Mutex<HashMap<String, Cursor>>,
    read_only: bool,
}

impl Redis {
    pub fn validate_profile(profile: &ConnectionProfile) -> AppResult<()> {
        profile
            .database
            .parse::<u32>()
            .ok()
            .filter(|v| *v <= i32::MAX as u32)
            .ok_or_else(|| {
                AppError::invalid("Redis database must be a nonnegative integer index")
            })?;
        Ok(())
    }
    pub async fn open(
        profile: &ConnectionProfile,
        password: Option<&str>,
        session: &str,
        port: Option<u16>,
    ) -> AppResult<(Arc<Self>, SessionInfo)> {
        Self::validate_profile(profile)?;
        let start = Instant::now();
        let tls = if profile.tls_mode == TlsMode::VerifyFull {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let mut roots = rustls::RootCertStore::empty();
            for cert in rustls_native_certs::load_native_certs().certs {
                let _ = roots.add(cert);
            }
            if let Some(path) = &profile.ca_cert_path {
                let bytes = crate::files::read_bounded(path, 1024 * 1024)?;
                let mut count = 0;
                for cert in rustls_pemfile::certs(&mut bytes.as_slice()) {
                    roots
                        .add(cert.map_err(|_| AppError::invalid("Invalid CA certificate"))?)
                        .map_err(|_| AppError::invalid("Invalid CA certificate"))?;
                    count += 1;
                }
                if count == 0 {
                    return Err(AppError::invalid(
                        "The selected file contains no PEM certificates",
                    ));
                }
            }
            Some(Arc::new(
                rustls::ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            ))
        } else {
            None
        };
        let options = Options {
            host: profile.host.clone(),
            port: port.unwrap_or(profile.port),
            dial_host: if port.is_some() {
                "127.0.0.1".into()
            } else {
                profile.host.clone()
            },
            username: profile.username.clone(),
            password: password.map(|v| Zeroizing::new(v.into())),
            database: profile.database.parse().unwrap(),
            tls,
        };
        let wire = options.connect().await?;
        let info = SessionInfo {
            id: session.into(),
            connection_id: profile.id.clone(),
            engine: EngineKind::Redis,
            server_version: "Redis / RESP2".into(),
            latency_ms: start.elapsed().as_millis() as u32,
            encrypted: options.tls.is_some(),
            capabilities: EngineCapabilities {
                sql: false,
                transactions: false,
                cancellation: true,
                table_browsing: false,
                table_editing: false,
                processes: false,
            },
        };
        Ok((
            Arc::new(Self {
                options,
                editor: Mutex::new(Some(wire)),
                closed: CancellationToken::new(),
                cursors: parking_lot::Mutex::new(HashMap::new()),
                read_only: profile.safe_mode == SafeMode::ReadOnly,
            }),
            info,
        ))
    }
    async fn request(&self, wire: &mut Wire, args: &[Vec<u8>]) -> AppResult<Value> {
        let outcome = tokio::select! {
            biased;
            _=self.closed.cancelled()=>Err(AppError::new(ErrorCode::Cancelled,"The Redis operation was interrupted")),
            outcome=tokio::time::timeout(Duration::from_secs(10),wire.request(args))=>outcome.unwrap_or_else(|_|Err(protocol::disconnected())),
        };
        if outcome.as_ref().is_err_and(|e| {
            matches!(
                e.code,
                ErrorCode::ConnectionClosed | ErrorCode::ResultLimitReached
            )
        }) {
            self.close();
        }
        outcome
    }
    async fn auxiliary(&self) -> AppResult<Wire> {
        tokio::select! {biased;_=self.closed.cancelled()=>Err(protocol::disconnected()),wire=self.options.connect()=>wire}
    }
    fn cursor(&self, scope: &str, token: Option<&str>) -> AppResult<u64> {
        let Some(token) = token else { return Ok(0) };
        self.cursors.lock().get(token).filter(|c|c.scope==scope&&c.deadline>Instant::now()).map(|c|c.value).ok_or_else(||AppError::new(ErrorCode::Conflict,"This scan cursor expired or belongs to a different pattern/key/type. Restart browsing."))
    }
    fn remember(&self, scope: String, cursor: u64) -> AppResult<Option<String>> {
        if cursor == 0 {
            return Ok(None);
        }
        let mut cursors = self.cursors.lock();
        cursors.retain(|_, c| c.deadline > Instant::now());
        if cursors.len() >= 128 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Restart the Redis browser after old cursors expire (five minutes)",
            ));
        }
        let token = new_id();
        cursors.insert(
            token.clone(),
            Cursor {
                scope,
                value: cursor,
                deadline: Instant::now() + Duration::from_secs(300),
            },
        );
        Ok(Some(token))
    }
    async fn key_info(&self, wire: &mut Wire, key: &[u8]) -> AppResult<RedisKeyInfo> {
        let kind = scalar(self.request(wire, &args(&[b"TYPE", key])).await?)?.unwrap_or_default();
        let ttl = scalar(self.request(wire, &args(&[b"PTTL", key])).await?)?.unwrap_or_default();
        Ok(RedisKeyInfo {
            key: display(key),
            argument: commands::argument(key),
            kind,
            ttl_ms: ttl,
        })
    }
}
impl Options {
    async fn connect(&self) -> AppResult<Wire> {
        tokio::time::timeout(Duration::from_secs(10),async {
            let stream=TcpStream::connect((self.dial_host.as_str(),self.port)).await.map_err(|_|protocol::disconnected())?;
            stream.set_nodelay(true).map_err(|_|protocol::disconnected())?;
            let mut wire=if let Some(tls)=&self.tls {
                let name=rustls::pki_types::ServerName::try_from(self.host.clone()).map_err(|_|AppError::invalid("Invalid Redis TLS hostname"))?;
                let stream=tokio_rustls::TlsConnector::from(tls.clone()).connect(name,stream).await.map_err(|_|AppError::new(ErrorCode::ConnectionFailed,"Redis verified TLS failed. Check the hostname, CA and server certificate."))?;
                Wire::new(stream)
            } else {Wire::new(stream)};
            if let Some(password)=&self.password {
                wire.request(&args(&[b"AUTH",self.username.as_bytes(),password.as_bytes()])).await.map_err(|_|AppError::new(ErrorCode::PermissionDenied,"Redis authentication failed. Check the ACL username and password."))?;
            }
            if self.database!=0 {wire.request(&args(&[b"SELECT",self.database.to_string().as_bytes()])).await?;}
            let pong=wire.request(&args(&[b"PING"])).await?;
            if scalar(pong)?.as_deref()!=Some("PONG") {return Err(AppError::new(ErrorCode::ConnectionFailed,"Redis did not return PONG"));}
            Ok(wire)
        }).await.unwrap_or_else(|_|Err(protocol::disconnected()))
    }
}

#[async_trait]
impl Driver for Redis {
    async fn execute(
        &self,
        source: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        let mut commands = commands::parse(source)?;
        if commands.len() != 1 || commands[0].kind != kind {
            return Err(AppError::invalid(
                "The Redis command no longer matches its reviewed plan",
            ));
        }
        let command = commands.remove(0);
        if self.read_only && kind != StatementKind::Read {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This Redis session is read-only",
            ));
        }
        let mut guard = self.editor.lock().await;
        if self.closed.is_cancelled() {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "The Redis command was cancelled before submission",
            ));
        }
        let wire = guard.as_mut().ok_or_else(protocol::disconnected)?;
        let response = self.request(wire, &command.args).await;
        let response = match response {
            Ok(value) => value,
            Err(e) => {
                if self.closed.is_cancelled() {
                    guard.take();
                }
                return Err(
                    if kind != StatementKind::Read && e.code != ErrorCode::Database {
                        AppError::new(
                            ErrorCode::OutcomeUnknown,
                            "The Redis write response was not observed. Check the key state before retrying; a disconnect does not undo an accepted command.",
                        )
                    } else {
                        e
                    },
                );
            }
        };
        let mut rows = Vec::new();
        flatten(response, "".into(), &mut rows)?;
        sink.describe(vec![column(0, "path"), column(1, "value")])?;
        for chunk in rows.chunks(200) {
            if !sink.rows(chunk.to_vec())? {
                break;
            }
        }
        sink.complete(0)?;
        Ok(0)
    }
    async fn cancel(&self) -> AppResult<()> {
        self.close();
        Ok(())
    }
    async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
        Ok(vec![])
    }
    async fn schema(&self, _: &TableRef) -> AppResult<TableSchema> {
        Err(unsupported())
    }
    async fn browse(
        &self,
        _: &TableSchema,
        _: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        Err(unsupported())
    }
    async fn apply(&self, _: &TableSchema, _: &[RowMutation]) -> AppResult<()> {
        Err(unsupported())
    }
    async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
        Err(unsupported())
    }
    async fn cancel_process(&self, _: &str, _: &str) -> AppResult<()> {
        Err(unsupported())
    }
    async fn redis_scan(&self, pattern: &str, cursor: Option<&str>) -> AppResult<RedisScanPage> {
        if pattern.len() > 1024 {
            return Err(AppError::invalid(
                "Redis MATCH patterns are limited to 1 KiB",
            ));
        }
        let scope = scope(b"SCAN", pattern.as_bytes());
        let position = self.cursor(&scope, cursor)?;
        let mut wire = self.auxiliary().await?;
        let response = self
            .request(
                &mut wire,
                &args(&[
                    b"SCAN",
                    position.to_string().as_bytes(),
                    b"MATCH",
                    pattern.as_bytes(),
                    b"COUNT",
                    b"100",
                ]),
            )
            .await?;
        let (next, values) = scan_result(response)?;
        if values.len() > 500 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Redis returned more than 500 keys despite the SCAN hint. Narrow the MATCH pattern.",
            ));
        }
        let mut keys = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut bytes = 0;
        for value in values {
            let key = bulk(value)?;
            if key.len() > 65536 {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "A Redis key exceeds the 64 KiB browser limit",
                ));
            }
            if !seen.insert(key.clone()) {
                continue;
            }
            let info = self.key_info(&mut wire, &key).await?;
            bytes += info.key.len() + info.argument.len() + info.kind.len() + info.ttl_ms.len();
            if bytes > 2 * 1024 * 1024 {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "The Redis key page exceeds 2 MiB. Narrow the MATCH pattern.",
                ));
            }
            keys.push(info);
        }
        Ok(RedisScanPage {
            keys,
            next_cursor: self.remember(scope, next)?,
        })
    }
    async fn redis_inspect(
        &self,
        argument: &str,
        cursor: Option<&str>,
    ) -> AppResult<RedisValuePreview> {
        if argument.len() > 256 * 1024 {
            return Err(AppError::invalid("The Redis key argument is too large"));
        }
        let command = commands::parse(&format!("GET {argument}"))?;
        if command.len() != 1 {
            return Err(AppError::invalid("Inspect one quoted Redis key"));
        }
        let key = &command[0].args[1];
        let mut wire = self.auxiliary().await?;
        let info = self.key_info(&mut wire, key).await?;
        let scope = scope(info.kind.as_bytes(), key);
        let position = self.cursor(&scope, cursor)?;
        let mut next = 0;
        let (entries, truncated) = match info.kind.as_str() {
            "none" => (vec![], false),
            "string" => {
                if cursor.is_some() {
                    return Err(AppError::invalid("String previews do not use scan cursors"));
                }
                let length = scalar(self.request(&mut wire, &args(&[b"STRLEN", key])).await?)?
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                let value = scalar(
                    self.request(&mut wire, &args(&[b"GETRANGE", key, b"0", b"65535"]))
                        .await?,
                )?;
                (vec![(None, value)], length > 65536)
            }
            "hash" | "set" | "zset" => {
                let name = match info.kind.as_str() {
                    "hash" => b"HSCAN".as_slice(),
                    "set" => b"SSCAN",
                    _ => b"ZSCAN",
                };
                let (following, values) = scan_result(
                    self.request(
                        &mut wire,
                        &args(&[name, key, position.to_string().as_bytes(), b"COUNT", b"100"]),
                    )
                    .await?,
                )?;
                next = following;
                let entries = if info.kind == "set" {
                    values
                        .into_iter()
                        .map(|v| Ok((None, scalar(v)?)))
                        .collect::<AppResult<Vec<_>>>()?
                } else {
                    if values.len() % 2 != 0 {
                        return Err(protocol::disconnected());
                    }
                    let mut entries = Vec::new();
                    let mut values = values.into_iter();
                    while let Some(field) = values.next() {
                        entries.push((scalar(field)?, scalar(values.next().unwrap())?));
                    }
                    entries
                };
                (entries, next != 0)
            }
            "list" => {
                let length = scalar(self.request(&mut wire, &args(&[b"LLEN", key])).await?)?
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                let start = position.to_string();
                let end = position.saturating_add(99).to_string();
                let values = array(
                    self.request(
                        &mut wire,
                        &args(&[b"LRANGE", key, start.as_bytes(), end.as_bytes()]),
                    )
                    .await?,
                )?;
                if position.saturating_add(values.len() as u64) < length {
                    next = position + values.len() as u64;
                }
                let entries = values
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| Ok((Some((position + i as u64).to_string()), scalar(v)?)))
                    .collect::<AppResult<Vec<_>>>()?;
                (entries, next != 0)
            }
            _ => (
                vec![(
                    None,
                    Some("This key type has no viewer in this preview.".into()),
                )],
                true,
            ),
        };
        let bytes = entries
            .iter()
            .map(|(a, b)| a.as_ref().map_or(0, String::len) + b.as_ref().map_or(0, String::len))
            .sum::<usize>();
        if entries.len() > 500 || bytes > 2 * 1024 * 1024 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "The Redis value preview exceeds 500 entries or 2 MiB. Use console commands with a narrower range.",
            ));
        }
        Ok(RedisValuePreview {
            key: info,
            entries,
            next_cursor: self.remember(scope, next)?,
            truncated,
        })
    }
    async fn health(&self) -> AppResult<u32> {
        let start = Instant::now();
        let mut wire = self.auxiliary().await?;
        self.request(&mut wire, &args(&[b"PING"])).await?;
        Ok(start.elapsed().as_millis() as u32)
    }
    fn is_closed(&self) -> bool {
        self.closed.is_cancelled()
    }
    fn close(&self) {
        self.closed.cancel();
        self.cursors.lock().clear();
        if let Ok(mut guard) = self.editor.try_lock() {
            guard.take();
        }
    }
}
fn args(values: &[&[u8]]) -> Vec<Vec<u8>> {
    values.iter().map(|v| v.to_vec()).collect()
}
fn scope(kind: &[u8], key: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(kind);
    hash.update([0]);
    hash.update(key);
    hex::encode(hash.finalize())
}
fn column(index: usize, name: &str) -> ColumnInfo {
    ColumnInfo {
        id: format!("c{index}"),
        name: name.into(),
        database_type: "Redis RESP2".into(),
        kind: ColumnKind::Text,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: false,
    }
}
fn display(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .unwrap_or_else(|_| commands::argument(bytes))
}
fn scalar(value: Value) -> AppResult<Option<String>> {
    match value {
        Value::Nil => Ok(None),
        Value::BulkString(v) => Ok(Some(display(&v))),
        Value::SimpleString(v) => Ok(Some(v)),
        Value::Int(v) => Ok(Some(v.to_string())),
        Value::Okay => Ok(Some("OK".into())),
        _ => Err(protocol::disconnected()),
    }
}
fn bulk(value: Value) -> AppResult<Vec<u8>> {
    match value {
        Value::BulkString(v) => Ok(v),
        _ => Err(protocol::disconnected()),
    }
}
fn array(value: Value) -> AppResult<Vec<Value>> {
    match value {
        Value::Array(v) => Ok(v),
        _ => Err(protocol::disconnected()),
    }
}
fn scan_result(value: Value) -> AppResult<(u64, Vec<Value>)> {
    let mut v = array(value)?.into_iter();
    let cursor = scalar(v.next().ok_or_else(protocol::disconnected)?)?
        .and_then(|v| v.parse().ok())
        .ok_or_else(protocol::disconnected)?;
    let values = array(v.next().ok_or_else(protocol::disconnected)?)?;
    if v.next().is_some() {
        return Err(protocol::disconnected());
    }
    Ok((cursor, values))
}
fn flatten(value: Value, path: String, rows: &mut Vec<TextRow>) -> AppResult<()> {
    match value {
        Value::Array(v) => {
            if v.is_empty() {
                rows.push(vec![Some(path), Some("[]".into())]);
            } else {
                for (i, v) in v.into_iter().enumerate() {
                    flatten(v, format!("{path}[{i}]"), rows)?;
                }
            }
        }
        value => rows.push(vec![Some(path), scalar(value)?]),
    }
    Ok(())
}
fn unsupported() -> AppError {
    AppError::new(
        ErrorCode::Unsupported,
        "Use the dedicated Redis key browser and native command console",
    )
}
