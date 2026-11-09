//! Opt-in lifecycle qualification against the ordinary public fixture user.
//! No PROCESS grant, privileged dictionary lookup, or external server is used.
use futures_util::FutureExt;
use joinery_core::{
    domain::*,
    drivers::{
        Driver, EditTarget, RowMutation,
        mysql::{Mysql, guard::GuardPolicy},
    },
    execution::Studio,
    file_capability::DisabledFileDialog,
    secrets::MemorySecrets,
};
use mysql_async::{Conn, OptsBuilder, Row, prelude::Queryable};
use std::{
    error::Error,
    ops::{Deref, DerefMut},
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;
mod common;

type R<T> = Result<T, Box<dyn Error + Send + Sync>>;
const DEADLINE: Duration = Duration::from_secs(8);

struct OwnedConnection(Option<Conn>);
impl Deref for OwnedConnection {
    type Target = Conn;
    fn deref(&self) -> &Conn {
        self.0.as_ref().unwrap()
    }
}
impl DerefMut for OwnedConnection {
    fn deref_mut(&mut self) -> &mut Conn {
        self.0.as_mut().unwrap()
    }
}
impl OwnedConnection {
    async fn disconnect(mut self) -> mysql_async::Result<()> {
        self.0.take().unwrap().disconnect().await
    }
}
impl Drop for OwnedConnection {
    fn drop(&mut self) {
        if let Some(conn) = self.0.take() {
            tokio::spawn(async {
                let _ = timeout(Duration::from_secs(1), conn.disconnect()).await;
            });
        }
    }
}
struct OwnedTask<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for OwnedTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn profile(port: u16) -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Owned guard qualification".into(),
        group_id: "local".into(),
        engine: EngineKind::Mysql,
        host: "127.0.0.1".into(),
        port,
        database: "joinery".into(),
        username: "joinery".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::Silent,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: None,
    }
}
fn policy(lifetime: Duration) -> GuardPolicy {
    GuardPolicy {
        lifetime,
        acquisition_timeout: Duration::from_secs(4),
        verification_timeout: Duration::from_secs(1),
        close_timeout: Duration::from_secs(1),
        writer_lock_timeout: Duration::from_secs(3),
        max_guards: 8,
    }
}
async fn driver(port: u16, lifetime: Duration) -> R<Arc<Mysql>> {
    Ok(timeout(
        DEADLINE,
        Mysql::open_with_guard_policy(
            &profile(port),
            Some("joinery-local-only"),
            &new_id(),
            None,
            policy(lifetime),
        ),
    )
    .await??
    .0)
}
async fn observer(port: u16) -> R<OwnedConnection> {
    let opts = OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(port)
        .user(Some("joinery"))
        .pass(Some("joinery-local-only"))
        .db_name(Some("joinery"))
        .prefer_socket(false);
    let mut conn = OwnedConnection(Some(timeout(DEADLINE, Conn::new(opts)).await??));
    timeout(
        DEADLINE,
        conn.query_drop("SET SESSION lock_wait_timeout=1; SET SESSION innodb_lock_wait_timeout=1"),
    )
    .await??;
    Ok(conn)
}
fn quoted(table: &TableRef) -> String {
    format!("`joinery`.`{}`", table.name)
}
async fn sql(conn: &mut Conn, statement: &str) -> R<()> {
    timeout(DEADLINE, conn.query_drop(statement)).await??;
    Ok(())
}
async fn create(conn: &mut Conn, table: &TableRef, note: &str) -> R<()> {
    sql(conn, &format!("CREATE TABLE {} (id INT NOT NULL PRIMARY KEY, note VARCHAR(32) NOT NULL) ENGINE=InnoDB", quoted(table))).await?;
    sql(
        conn,
        &format!("INSERT INTO {} VALUES (1,'{note}')", quoted(table)),
    )
    .await
}
async fn note(conn: &mut Conn, table: &TableRef) -> R<String> {
    Ok(timeout(
        DEADLINE,
        conn.query_first::<String, _>(format!("SELECT note FROM {} WHERE id=1", quoted(table))),
    )
    .await??
    .ok_or("owned row missing")?)
}
fn update(before: &str, after: &str) -> Vec<RowMutation> {
    vec![RowMutation::Update {
        original: vec![Some("1".into()), Some(before.into())],
        changes: vec![(1, Some(after.into()))],
    }]
}
async fn apply(driver: &Mysql, target: &EditTarget, before: &str, after: &str) -> AppResult<()> {
    timeout(
        DEADLINE,
        driver.apply_target(
            &target.schema,
            target.identity.as_ref(),
            &update(before, after),
        ),
    )
    .await
    .expect("generated writer exceeded deadline")
}
async fn blocked_drop(conn: &mut Conn, table: &TableRef) -> R<()> {
    let started = Instant::now();
    let error = timeout(
        Duration::from_secs(3),
        conn.query_drop(format!("DROP TABLE {}", quoted(table))),
    )
    .await?
    .expect_err("healthy retained guard did not block DROP");
    assert!(
        matches!(&error, mysql_async::Error::Server(server) if server.code == 1205),
        "unexpected DROP failure: {error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}
async fn drop_eventually(conn: &mut Conn, table: &TableRef) -> R<()> {
    timeout(Duration::from_secs(5), async {
        loop {
            match conn
                .query_drop(format!("DROP TABLE {}", quoted(table)))
                .await
            {
                Ok(()) => return Ok(()),
                Err(mysql_async::Error::Server(error)) if error.code == 1205 => {}
                Err(error) => return Err(error),
            }
        }
    })
    .await??;
    Ok(())
}

// Every scenario is bounded and panic-aware; cleanup uses a fresh independent
// connection after scenario-local drivers/handles have dropped. Attempt every
// owned name and independently verify absence even if an earlier DROP failed.
async fn fixture<F, Fut>(port: u16, count: usize, scenario: F)
where
    F: FnOnce(Vec<TableRef>) -> Fut,
    Fut: std::future::Future<Output = R<()>>,
{
    let tables: Vec<_> = (0..count)
        .map(|_| TableRef {
            schema: "joinery".into(),
            name: format!("guard_{}", new_id().replace('-', "")),
        })
        .collect();
    let outcome = AssertUnwindSafe(timeout(Duration::from_secs(35), scenario(tables.clone())))
        .catch_unwind()
        .await;
    let mut cleanup_errors = Vec::new();
    match observer(port).await {
        Ok(mut conn) => {
            for table in &tables {
                let function = format!("`joinery`.`{}_f`", table.name);
                let dropped = timeout(
                    DEADLINE,
                    conn.query_drop(format!("DROP FUNCTION IF EXISTS {function}")),
                )
                .await;
                if !matches!(dropped, Ok(Ok(()))) {
                    cleanup_errors.push(format!("{function} cleanup: {dropped:?}"));
                }
            }
            for table in &tables {
                // Actor/socket close can lag last-owner release by its bounded
                // disconnect deadline; retry only owned cleanup lock timeouts.
                let dropped = timeout(Duration::from_secs(5), async {
                    loop {
                        match conn
                            .query_drop(format!("DROP TABLE IF EXISTS {}", quoted(table)))
                            .await
                        {
                            Ok(()) => return Ok(()),
                            Err(mysql_async::Error::Server(error)) if error.code == 1205 => {}
                            Err(error) => return Err(error),
                        }
                    }
                })
                .await;
                if !matches!(dropped, Ok(Ok(()))) {
                    cleanup_errors.push(format!("{} cleanup: {dropped:?}", table.name));
                }
            }
            for table in &tables {
                let remaining = timeout(DEADLINE, conn.exec_first::<u64, _, _>(
                    "SELECT COUNT(*) FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA='joinery' AND TABLE_NAME=?", (&table.name,),
                )).await;
                if !matches!(remaining, Ok(Ok(Some(0)))) {
                    cleanup_errors.push(format!("{} absence: {remaining:?}", table.name));
                }
                let remaining = timeout(DEADLINE, conn.exec_first::<u64, _, _>(
                    "SELECT COUNT(*) FROM INFORMATION_SCHEMA.ROUTINES WHERE ROUTINE_SCHEMA='joinery' AND ROUTINE_NAME=?", (format!("{}_f", table.name),),
                )).await;
                if !matches!(remaining, Ok(Ok(Some(0)))) {
                    cleanup_errors.push(format!("{} function absence: {remaining:?}", table.name));
                }
            }
            let disconnected = timeout(Duration::from_secs(2), conn.disconnect()).await;
            if !matches!(disconnected, Ok(Ok(()))) {
                cleanup_errors.push(format!("cleanup disconnect: {disconnected:?}"));
            }
        }
        Err(error) => cleanup_errors.push(format!("cleanup connection: {error}")),
    }
    assert!(
        cleanup_errors.is_empty(),
        "port {port}: {cleanup_errors:?}; scenario: {outcome:?}"
    );
    match outcome {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => panic!("port {port}: {error}"),
        Ok(Err(error)) => panic!("port {port} scenario deadline: {error}"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn healthy_guard_blocks_ddl_and_original_write_still_commits() {
    for port in [53306, 53307] {
        fixture(port, 1, |tables| async move {
            let table = &tables[0];
            let mut observe = observer(port).await?;
            create(&mut observe, table, "original").await?;
            let driver = driver(port, Duration::from_secs(12)).await?;
            let target = driver.edit_target(table).await?;
            blocked_drop(&mut observe, table).await?;
            apply(&driver, &target, "original", "approved").await?;
            assert_eq!(note(&mut observe, table).await?, "approved");
            driver.invalidate_edit_targets().await?;
            driver.close();
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn last_retained_owner_release_unblocks_ddl() {
    for port in [53306, 53307] {
        fixture(port, 1, |tables| async move {
            let table = &tables[0];
            let mut observe = observer(port).await?;
            create(&mut observe, table, "original").await?;
            let driver = driver(port, Duration::from_secs(12)).await?;
            let target = driver.edit_target(table).await?;
            let retained = target.identity.clone();
            drop(target);
            blocked_drop(&mut observe, table).await?;
            drop(retained);
            drop_eventually(&mut observe, table).await?;
            driver.close();
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn active_expiry_preserves_replacement_and_fresh_target_is_writable() {
    for port in [53306, 53307] {
        fixture(port, 1, |tables| async move {
            let table = &tables[0];
            let mut observe = observer(port).await?;
            create(&mut observe, table, "original").await?;
            let driver = driver(port, Duration::from_millis(700)).await?;
            let target = driver.edit_target(table).await?;
            // Keep target alive: only active deadline expiry can release MDL.
            drop_eventually(&mut observe, table).await?;
            create(&mut observe, table, "replacement").await?;
            let error = apply(&driver, &target, "replacement", "unsafe")
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict, "{error:?}");
            assert_eq!(note(&mut observe, table).await?, "replacement");
            let fresh = driver.edit_target(table).await?;
            assert_ne!(target.identity, fresh.identity);
            apply(&driver, &fresh, "replacement", "fresh").await?;
            assert_eq!(note(&mut observe, table).await?, "fresh");
            driver.invalidate_edit_targets().await?;
            driver.close();
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn invalidation_releases_clones_without_retiring_a_new_generation() {
    for port in [53306, 53307] {
        fixture(port, 1, |tables| async move {
            let table = &tables[0];
            let mut observe = observer(port).await?;
            create(&mut observe, table, "original").await?;
            let driver = driver(port, Duration::from_secs(12)).await?;
            let old = driver.edit_target(table).await?;
            let clone = old.identity.clone();
            driver.invalidate_edit_targets().await?;
            drop_eventually(&mut observe, table).await?;
            create(&mut observe, table, "replacement").await?;
            assert_eq!(
                apply(&driver, &old, "replacement", "unsafe")
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            let fresh = driver.edit_target(table).await?;
            assert_ne!(old.identity, fresh.identity);
            drop(old);
            drop(clone);
            blocked_drop(&mut observe, table).await?;
            apply(&driver, &fresh, "replacement", "fresh").await?;
            assert_eq!(note(&mut observe, table).await?, "fresh");
            driver.invalidate_edit_targets().await?;
            driver.close();
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn retained_guard_cannot_cross_session_or_table_and_missing_guard_fails_closed() {
    for port in [53306, 53307] {
        fixture(port, 2, |tables| async move {
            let mut observe = observer(port).await?;
            for table in &tables {
                create(&mut observe, table, "original").await?;
            }
            let first = driver(port, Duration::from_secs(12)).await?;
            let second = driver(port, Duration::from_secs(12)).await?;
            let target = first.edit_target(&tables[0]).await?;
            assert_eq!(
                apply(&second, &target, "original", "unsafe")
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            let other_schema = first.schema(&tables[1]).await?;
            assert_eq!(
                first
                    .apply_target(
                        &other_schema,
                        target.identity.as_ref(),
                        &update("original", "unsafe")
                    )
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            assert_eq!(
                first
                    .apply(&target.schema, &update("original", "unsafe"))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            for table in &tables {
                assert_eq!(note(&mut observe, table).await?, "original");
            }
            first.invalidate_edit_targets().await?;
            first.close();
            second.close();
            Ok(())
        })
        .await;
    }
}

async fn waiting_query(observe: &mut Conn, table: &TableRef, prefix: &str) -> R<()> {
    timeout(Duration::from_secs(4), async {
        loop {
            let processes: Vec<Row> = observe.query("SHOW FULL PROCESSLIST").await?;
            if processes.iter().any(|row| {
                let info = row
                    .get::<Option<String>, _>("Info")
                    .flatten()
                    .unwrap_or_default();
                let state = row
                    .get::<Option<String>, _>("State")
                    .flatten()
                    .unwrap_or_default();
                info.starts_with(prefix)
                    && info.contains(&table.name)
                    && state.to_ascii_lowercase().contains("metadata lock")
            }) {
                return Ok::<(), mysql_async::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn cancelling_writer_queued_behind_observed_ddl_submits_no_changes() {
    for port in [53306, 53307] {
        fixture(port, 1, |tables| async move {
            let table = tables[0].clone();
            let mut observe = observer(port).await?;
            create(&mut observe, &table, "original").await?;
            let driver = driver(port, Duration::from_secs(20)).await?;
            let target = driver.edit_target(&table).await?;
            let mut ddl = observer(port).await?;
            sql(&mut ddl, "SET SESSION lock_wait_timeout=6").await?;
            let ddl_sql = format!(
                "ALTER TABLE {} COMMENT='owned queued guard test'",
                quoted(&table)
            );
            let mut ddl = OwnedTask(tokio::spawn(async move {
                let result = timeout(DEADLINE, ddl.query_drop(ddl_sql)).await;
                let _ = timeout(Duration::from_secs(2), ddl.disconnect()).await;
                result
            }));
            waiting_query(&mut observe, &table, "ALTER TABLE").await?;
            let writer_driver = driver.clone();
            let mut writer = OwnedTask(tokio::spawn(async move {
                apply(&writer_driver, &target, "original", "unsafe").await
            }));
            waiting_query(&mut observe, &table, "SELECT ").await?;
            let started = Instant::now();
            timeout(Duration::from_secs(3), driver.cancel()).await??;
            let error = timeout(Duration::from_secs(3), &mut writer.0)
                .await??
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::Cancelled, "{error:?}");
            assert!(started.elapsed() < Duration::from_secs(3));
            driver.invalidate_edit_targets().await?;
            timeout(DEADLINE, &mut ddl.0).await????;
            assert_eq!(note(&mut observe, &table).await?, "original");
            driver.close();
            Ok(())
        })
        .await;
    }
}

// Fixed-fixture plaintext relay. It forwards every byte unchanged and closes
// only the socket whose actual SQL packet contains this scenario's owned read.
// No server response, error code, or liveness observation is synthesized.
struct GuardSocketRelay {
    port: u16,
    observed: CancellationToken,
    terminate: CancellationToken,
    terminated: CancellationToken,
    shutdown: CancellationToken,
    connections: Arc<parking_lot::Mutex<Vec<tokio::task::AbortHandle>>>,
    executions: Arc<AtomicUsize>,
    _listener: OwnedTask<()>,
}
impl Drop for GuardSocketRelay {
    fn drop(&mut self) {
        self.shutdown.cancel();
        for connection in self.connections.lock().iter() {
            connection.abort();
        }
    }
}
impl GuardSocketRelay {
    async fn start(upstream_port: u16, table: &TableRef) -> R<Self> {
        Self::matching(
            upstream_port,
            format!("SELECT 1 FROM {} LIMIT 1", quoted(table)).into_bytes(),
        )
        .await
    }
    async fn matching(upstream_port: u16, pattern: Vec<u8>) -> R<Self> {
        assert!([53306, 53307].contains(&upstream_port));
        assert!(!pattern.is_empty());
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let observed = CancellationToken::new();
        let terminate = CancellationToken::new();
        let terminated = CancellationToken::new();
        let shutdown = CancellationToken::new();
        let connections = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let executions = Arc::new(AtomicUsize::new(0));
        let (seen, kill, killed, stop, tasks, counts) = (
            observed.clone(),
            terminate.clone(),
            terminated.clone(),
            shutdown.clone(),
            connections.clone(),
            executions.clone(),
        );
        let task = OwnedTask(tokio::spawn(async move {
            loop {
                let (client, _) = tokio::select! { _ = stop.cancelled() => break, accepted = listener.accept() => match accepted { Ok(socket) => socket, Err(_) => break } };
                let (seen, kill, killed, stop, pattern, counts) = (
                    seen.clone(),
                    kill.clone(),
                    killed.clone(),
                    stop.clone(),
                    pattern.clone(),
                    counts.clone(),
                );
                let task = tokio::spawn(async move {
                    let Ok(Ok(server)) = timeout(
                        Duration::from_secs(3),
                        TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, upstream_port)),
                    )
                    .await
                    else {
                        return;
                    };
                    let (mut client_read, mut client_write) = client.into_split();
                    let (mut server_read, mut server_write) = server.into_split();
                    let matched = CancellationToken::new();
                    let matched_read = matched.clone();
                    let forward = async {
                        let outgoing = async {
                            let mut buffer = [0u8; 4096];
                            let mut packets = Vec::new();
                            let mut target_prepared = false;
                            loop {
                                let count = client_read.read(&mut buffer).await?;
                                if count == 0 {
                                    return Ok::<(), std::io::Error>(());
                                }
                                packets.extend_from_slice(&buffer[..count]);
                                while packets.len() >= 4 {
                                    let length = usize::from(packets[0])
                                        | (usize::from(packets[1]) << 8)
                                        | (usize::from(packets[2]) << 16);
                                    if length > 1024 * 1024 {
                                        return Err(std::io::Error::other(
                                            "owned fixture request exceeded relay bound",
                                        ));
                                    }
                                    if packets.len() < length + 4 {
                                        break;
                                    }
                                    let payload = &packets[4..length + 4];
                                    if packets[3] == 0 {
                                        let command = payload.first().copied();
                                        let named_query = matches!(command, Some(0x03 | 0x16))
                                            && payload
                                                .windows(pattern.len())
                                                .any(|window| window == pattern);
                                        if named_query {
                                            if command == Some(0x16) {
                                                target_prepared = true;
                                            }
                                            if command == Some(0x03) {
                                                counts.fetch_add(1, Ordering::SeqCst);
                                            }
                                            if !killed.is_cancelled() {
                                                matched_read.cancel();
                                                seen.cancel();
                                            }
                                        }
                                        if command == Some(0x17) && target_prepared {
                                            counts.fetch_add(1, Ordering::SeqCst);
                                        }
                                    }
                                    packets.drain(..length + 4);
                                }
                                server_write.write_all(&buffer[..count]).await?;
                            }
                        };
                        let incoming = async {
                            tokio::io::copy(&mut server_read, &mut client_write)
                                .await
                                .map(|_| ())
                        };
                        tokio::try_join!(outgoing, incoming)
                    };
                    tokio::select! {
                        _ = stop.cancelled() => {},
                        _ = async { matched.cancelled().await; kill.cancelled().await; } => { killed.cancel(); },
                        _ = forward => {},
                    }
                });
                tasks.lock().push(task.abort_handle());
            }
        }));
        Ok(Self {
            port,
            observed,
            terminate,
            terminated,
            shutdown,
            connections,
            executions,
            _listener: task,
        })
    }
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn original_guard_socket_loss_cannot_revive_replaced_target() {
    for port in [53306, 53307] {
        fixture(port, 1, |tables| async move {
            let table = &tables[0];
            let mut observe = observer(port).await?;
            create(&mut observe, table, "original").await?;
            let relay = GuardSocketRelay::start(port, table).await?;
            let driver = driver(relay.port, Duration::from_secs(15)).await?;
            let target = driver.edit_target(table).await?;
            timeout(DEADLINE, relay.observed.cancelled()).await?;
            relay.terminate.cancel();
            timeout(DEADLINE, relay.terminated.cancelled()).await?;
            // The editor remains connected. Only original guard transport died.
            driver.session_state("observer-health").await?;
            drop_eventually(&mut observe, table).await?;
            create(&mut observe, table, "replacement").await?;
            assert_eq!(
                apply(&driver, &target, "replacement", "unsafe")
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            assert_eq!(note(&mut observe, table).await?, "replacement");
            // Fault is one-shot. A new target on the still-usable editor must
            // acquire a new physical guard socket and generation.
            let fresh = driver.edit_target(table).await?;
            assert_ne!(target.identity, fresh.identity);
            apply(&driver, &fresh, "replacement", "fresh").await?;
            assert_eq!(note(&mut observe, table).await?, "fresh");
            driver.invalidate_edit_targets().await?;
            driver.close();
            Ok(())
        })
        .await;
    }
}

struct OwnedStudio(Arc<Studio>);
impl Drop for OwnedStudio {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn successful_approved_raw_browse_remains_truthful_after_guard_expiry() {
    for port in [53306, 53307] {
        fixture(port, 2, |tables| async move {
            let target = &tables[0]; let marker = &tables[1];
            let mut observe = observer(port).await?;
            create(&mut observe, target, "original").await?;
            sql(&mut observe, &format!("CREATE TABLE {} (id INT NOT NULL) ENGINE=InnoDB", quoted(marker))).await?;
            let function = format!("`joinery`.`{}_f`", target.name);
            sql(&mut observe, &format!("CREATE FUNCTION {function}(n INT) RETURNS INT NOT DETERMINISTIC MODIFIES SQL DATA BEGIN INSERT INTO {} VALUES(n); DO SLEEP(0.8); RETURN 1; END", quoted(marker))).await?;
            let root = tempfile::tempdir()?;
            let owned = OwnedStudio(Studio::with_mysql_guard_policy(root.path(), Arc::new(MemorySecrets::default()), Arc::new(DisabledFileDialog), policy(Duration::from_millis(400)))?);
            let studio = &owned.0;
            let mut p = profile(port); p.safe_mode = SafeMode::Confirm;
            let p = studio.save_profile(p, None)?;
            let session = timeout(DEADLINE, studio.connect(&p.id, ConnectionSecrets { password: Some("joinery-local-only".into()), ..Default::default() })).await??;
            let query = BrowseQuery { where_clause: Some(format!("{function}(id)=1")), ..Default::default() };
            let plan = studio.prepare_browse(&session.id, target, &query).await?.ok_or("side-effecting function did not require review")?;
            let approval = BrowseApproval { plan_id: plan.id.clone(), approval: studio.authorize(&plan.id, None, plan.confirmation_text.as_deref())? };
            let page = timeout(DEADLINE, studio.browse_with_approval(&session.id, target, None, &query, Some(&approval))).await?;
            let count: Option<u64> = observe.query_first(format!("SELECT COUNT(*) FROM {}", quoted(marker))).await?;
            assert_eq!(count, Some(1), "approved browse must commit its marker exactly once");
            let page = page.expect("successful raw browse was falsely reported failed after its guard expired");
            assert_eq!(page.result.retained_rows, 1);
            assert_eq!(studio.results.row(&page.result.id, 0)?, vec![Some("1".into()), Some("original".into())]);
            assert!(matches!(page.result.editability, Some(ResultEditability::ReadOnly { .. })));
            assert!(page.result.columns.iter().all(|column| !column.editable && !column.insertable));
            assert!(!page.has_more);
            assert_eq!(studio.browse_with_approval(&session.id, target, None, &query, Some(&approval)).await.unwrap_err().code, ErrorCode::ApprovalExpired);
            let count: Option<u64> = observe.query_first(format!("SELECT COUNT(*) FROM {}", quoted(marker))).await?;
            assert_eq!(count, Some(1), "expired approval replayed the function");
            studio.release_result(&page.result.id);
            studio.disconnect(&session.id);
            Ok(())
        }).await;
    }
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn approved_raw_browse_observes_late_errors_beyond_retained_bytes() {
    for port in [53306, 53307] {
        fixture(port, 2, |tables| async move {
            let target = &tables[0]; let marker = &tables[1];
            let mut observe = observer(port).await?;
            sql(&mut observe, &format!("CREATE TABLE {} (id INT NOT NULL PRIMARY KEY, payload LONGTEXT NOT NULL) ENGINE=InnoDB", quoted(target))).await?;
            for id in 1..=4 {
                sql(&mut observe, &format!("INSERT INTO {} VALUES ({id},REPEAT('x',800000))", quoted(target))).await?;
            }
            sql(&mut observe, &format!("CREATE TABLE {} (id INT NOT NULL) ENGINE=InnoDB", quoted(marker))).await?;
            let function = format!("`joinery`.`{}_f`", target.name);
            sql(&mut observe, &format!("CREATE FUNCTION {function}(n INT) RETURNS INT NOT DETERMINISTIC MODIFIES SQL DATA BEGIN INSERT INTO {} VALUES(n); IF n=4 THEN SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='owned late browse error'; END IF; RETURN 1; END", quoted(marker))).await?;
            let root = tempfile::tempdir()?;
            let owned = OwnedStudio(Studio::with_mysql_guard_policy(root.path(), Arc::new(MemorySecrets::default()), Arc::new(DisabledFileDialog), policy(Duration::from_secs(12)))?);
            let studio = &owned.0;
            let mut p = profile(port); p.safe_mode = SafeMode::Confirm;
            let p = studio.save_profile(p, None)?;
            let session = timeout(DEADLINE, studio.connect(&p.id, ConnectionSecrets { password: Some("joinery-local-only".into()), ..Default::default() })).await??;
            let query = BrowseQuery { where_clause: Some(format!("{function}(id)=1")), ..Default::default() };
            let plan = studio.prepare_browse(&session.id, target, &query).await?.ok_or("late-error function did not require review")?;
            let approval = BrowseApproval { plan_id: plan.id.clone(), approval: studio.authorize(&plan.id, None, plan.confirmation_text.as_deref())? };
            let outcome = timeout(DEADLINE, studio.browse_with_approval(&session.id, target, None, &query, Some(&approval))).await?;
            let count: Option<u64> = observe.query_first(format!("SELECT COUNT(*) FROM {}", quoted(marker))).await?;
            assert_eq!(count, Some(0), "observer did not prove the failed statement's marker effects rolled back");
            let error = outcome.expect_err("browse discarded the terminal error after retaining its bounded prefix");
            assert_eq!(error.code, ErrorCode::Database, "{error:?}");
            assert!(error.message.contains("owned late browse error"), "{error:?}");
            assert_eq!(studio.browse_with_approval(&session.id, target, None, &query, Some(&approval)).await.unwrap_err().code, ErrorCode::ApprovalExpired);
            let count: Option<u64> = observe.query_first(format!("SELECT COUNT(*) FROM {}", quoted(marker))).await?;
            assert_eq!(count, Some(0));
            studio.disconnect(&session.id);
            Ok(())
        }).await;
    }
}

async fn observe_function_query(conn: &mut Conn, function: &str, sleeping: bool) -> R<()> {
    let mut owned_states = Vec::new();
    let result = timeout(Duration::from_secs(5), async {
        loop {
            let processes: Vec<Row> = conn.query("SHOW FULL PROCESSLIST").await?;
            let mut found = false;
            for row in processes {
                let info = row
                    .get::<Option<String>, _>("Info")
                    .flatten()
                    .unwrap_or_default();
                let state = row
                    .get::<Option<String>, _>("State")
                    .flatten()
                    .unwrap_or_default();
                if info.contains(function) {
                    if owned_states.len() < 8 && !owned_states.contains(&state) {
                        owned_states.push(state.clone());
                    }
                    found |= !sleeping || state.to_ascii_lowercase().contains("user sleep");
                }
            }
            if found == sleeping {
                return Ok::<(), mysql_async::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    match result {
        Ok(result) => result?,
        Err(_) => return Err(format!("owned function process observation timed out (sleeping={sleeping}, owned states={owned_states:?})").into()),
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires fixed ordinary-user MySQL 53306 and MariaDB 53307 fixtures"]
async fn approved_raw_browse_transport_loss_is_unknown_without_retry_or_session_reuse() {
    for port in [53306, 53307] {
        fixture(port, 2, |tables| async move {
            let target = &tables[0]; let marker = &tables[1];
            let mut observe = observer(port).await?;
            create(&mut observe, target, "original").await?;
            sql(&mut observe, &format!("CREATE TABLE {} (id INT NOT NULL) ENGINE=InnoDB", quoted(marker))).await?;
            let function_name = format!("{}_f", target.name);
            let function = format!("`joinery`.`{function_name}`");
            sql(&mut observe, &format!("CREATE FUNCTION {function}(n INT) RETURNS INT NOT DETERMINISTIC MODIFIES SQL DATA BEGIN INSERT INTO {} VALUES(n); DO SLEEP(2 + 0 * LENGTH('{function_name}')); RETURN 1; END", quoted(marker))).await?;
            let relay = GuardSocketRelay::matching(port, format!("{function}(id)").into_bytes()).await?;
            let root = tempfile::tempdir()?;
            let owned = OwnedStudio(Studio::with_mysql_guard_policy(root.path(), Arc::new(MemorySecrets::default()), Arc::new(DisabledFileDialog), policy(Duration::from_secs(12)))?);
            let studio = &owned.0;
            let mut p = profile(relay.port); p.safe_mode = SafeMode::Confirm;
            let p = studio.save_profile(p, None)?;
            let session = timeout(DEADLINE, studio.connect(&p.id, ConnectionSecrets { password: Some("joinery-local-only".into()), ..Default::default() })).await??;
            let query = BrowseQuery { where_clause: Some(format!("{function}(id)=1")), ..Default::default() };
            let plan = studio.prepare_browse(&session.id, target, &query).await?.ok_or("transport fixture did not require raw review")?;
            let approval = BrowseApproval { plan_id: plan.id.clone(), approval: studio.authorize(&plan.id, None, plan.confirmation_text.as_deref())? };
            let (run_studio, run_session, run_target, run_query, run_approval) = (studio.clone(), session.id.clone(), target.clone(), query.clone(), approval.clone());
            let mut run = OwnedTask(tokio::spawn(async move {
                run_studio.browse_with_approval(&run_session, &run_target, None, &run_query, Some(&run_approval)).await
            }));
            timeout(DEADLINE, relay.observed.cancelled()).await?;
            // Packet observation alone only proves preparation/submission. The
            // actual ordinary-user server process must be inside our function.
            observe_function_query(&mut observe, &function_name, true).await?;
            assert_eq!(relay.executions.load(Ordering::SeqCst), 1);
            assert_eq!(studio.authorize(&plan.id, None, plan.confirmation_text.as_deref()).unwrap_err().code, ErrorCode::ApprovalExpired, "approval was not consumed before the observed execution");
            relay.terminate.cancel();
            timeout(DEADLINE, relay.terminated.cancelled()).await?;
            let error = timeout(DEADLINE, &mut run.0).await??.unwrap_err();
            assert_eq!(error.code, ErrorCode::OutcomeUnknown, "{error:?}");
            assert_eq!(studio.usage().session_count, 0, "terminal raw transport failure retained a backend session");
            assert!(studio.prepare_execution(&session.id, "SELECT 1").is_err());
            assert!(studio.browse_with_approval(&session.id, target, None, &query, Some(&approval)).await.is_err(), "a retired session reused its consumed approval");
            // Wait for the actual server statement to end before observing its
            // independent committed state. Either result is compatible with a
            // lost response; neither is inferred from the OutcomeUnknown error.
            observe_function_query(&mut observe, &function_name, false).await?;
            let ids: Vec<u32> = timeout(DEADLINE, observe.query(format!("SELECT id FROM {} ORDER BY id", quoted(marker)))).await??;
            assert!(ids.is_empty() || ids == [1], "unexpected marker state or replay: {ids:?}");
            assert_eq!(relay.executions.load(Ordering::SeqCst), 1, "raw operation was automatically resubmitted");
            assert_eq!(note(&mut observe, target).await?, "original", "independent observer became unusable");
            Ok(())
        }).await;
    }
}

#[derive(Default)]
struct TlsEvidence {
    accepted: AtomicUsize,
    ssl_requests: AtomicUsize,
    original_sni: AtomicUsize,
    active: AtomicUsize,
    errors: parking_lot::Mutex<Vec<String>>,
}
impl TlsEvidence {
    fn assert_verified(&self) -> usize {
        let accepted = self.accepted.load(Ordering::SeqCst);
        assert_eq!(
            self.ssl_requests.load(Ordering::SeqCst),
            accepted,
            "an owned channel did not request MySQL TLS"
        );
        assert_eq!(
            self.original_sni.load(Ordering::SeqCst),
            accepted,
            "an owned channel did not present the original database hostname"
        );
        let errors = self.errors.lock().clone();
        assert!(errors.is_empty(), "TLS relay errors: {errors:?}");
        accepted
    }
}
struct ActiveTlsSocket(Arc<TlsEvidence>);
impl Drop for ActiveTlsSocket {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

// Observe the real client handshake without terminating TLS or changing a byte.
// The actual fixture server supplies its certificate and validates credentials.
struct TlsWireRelay {
    port: u16,
    evidence: Arc<TlsEvidence>,
    shutdown: CancellationToken,
    connections: Arc<parking_lot::Mutex<Vec<tokio::task::AbortHandle>>>,
    _listener: OwnedTask<()>,
}
impl Drop for TlsWireRelay {
    fn drop(&mut self) {
        self.shutdown.cancel();
        for connection in self.connections.lock().iter() {
            connection.abort();
        }
    }
}
impl TlsWireRelay {
    async fn start(upstream_port: u16) -> R<Self> {
        assert!([53306, 53307].contains(&upstream_port));
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let evidence = Arc::new(TlsEvidence::default());
        let shutdown = CancellationToken::new();
        let connections = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (stats, stop, tasks) = (evidence.clone(), shutdown.clone(), connections.clone());
        let task = OwnedTask(tokio::spawn(async move {
            loop {
                let (client, _) = tokio::select! {
                    _ = stop.cancelled() => break,
                    accepted = listener.accept() => match accepted { Ok(socket) => socket, Err(_) => break },
                };
                stats.accepted.fetch_add(1, Ordering::SeqCst);
                stats.active.fetch_add(1, Ordering::SeqCst);
                let (stats, stop) = (stats.clone(), stop.clone());
                let task = tokio::spawn(async move {
                    let _active = ActiveTlsSocket(stats.clone());
                    let outcome = tokio::select! {
                        _ = stop.cancelled() => return,
                        result = timeout(Duration::from_secs(35), inspect_tls_channel(client, upstream_port, &stats)) => result,
                    };
                    let error = match outcome {
                        Ok(Ok(())) => None,
                        Ok(Err(error)) => Some(error.to_string()),
                        Err(_) => Some("owned TLS channel exceeded its lifetime bound".into()),
                    };
                    if let Some(error) = error {
                        let mut errors = stats.errors.lock();
                        if errors.len() < 8 {
                            errors.push(error);
                        }
                    }
                });
                tasks.lock().push(task.abort_handle());
            }
        }));
        Ok(Self {
            port,
            evidence,
            shutdown,
            connections,
            _listener: task,
        })
    }
}

async fn inspect_tls_channel(
    client: TcpStream,
    upstream_port: u16,
    stats: &TlsEvidence,
) -> std::io::Result<()> {
    let server = timeout(
        Duration::from_secs(3),
        TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, upstream_port)),
    )
    .await??;
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = server.into_split();
    let outgoing = async {
        let mut buffer = [0u8; 4096];
        let mut ssl_request = Vec::new();
        let mut ssl_checked = false;
        let mut acceptor = rustls::server::Acceptor::default();
        let mut verified = false;
        let mut inspected_bytes = 0usize;
        loop {
            let count = if verified {
                client_read.read(&mut buffer).await?
            } else {
                timeout(Duration::from_secs(3), client_read.read(&mut buffer)).await??
            };
            if count == 0 {
                if !verified {
                    return Err(std::io::Error::other(
                        "owned channel ended before a verified ClientHello",
                    ));
                }
                server_write.shutdown().await?;
                return Ok::<(), std::io::Error>(());
            }
            if !verified {
                inspected_bytes += count;
                if inspected_bytes > 64 * 1024 {
                    return Err(std::io::Error::other(
                        "owned ClientHello exceeded 64 KiB inspection bound",
                    ));
                }
                let tls_bytes = if !ssl_checked {
                    ssl_request.extend_from_slice(&buffer[..count]);
                    if ssl_request.len() >= 4 {
                        let length = usize::from(ssl_request[0])
                            | (usize::from(ssl_request[1]) << 8)
                            | (usize::from(ssl_request[2]) << 16);
                        if length != 32 || ssl_request[3] != 1 {
                            return Err(std::io::Error::other(
                                "owned connection did not send a MySQL SSLRequest",
                            ));
                        }
                    }
                    if ssl_request.len() >= 36 {
                        let flags = u32::from_le_bytes(ssl_request[4..8].try_into().unwrap());
                        if flags & 0x0800 == 0 {
                            return Err(std::io::Error::other(
                                "owned MySQL request omitted CLIENT_SSL",
                            ));
                        }
                        stats.ssl_requests.fetch_add(1, Ordering::SeqCst);
                        ssl_checked = true;
                        ssl_request[36..].to_vec()
                    } else {
                        Vec::new()
                    }
                } else {
                    buffer[..count].to_vec()
                };
                let mut input = std::io::Cursor::new(tls_bytes.as_slice());
                while (input.position() as usize) < tls_bytes.len() {
                    let read = acceptor.read_tls(&mut input)?;
                    if read == 0 {
                        return Err(std::io::Error::other(
                            "ClientHello inspection made no progress",
                        ));
                    }
                    match acceptor.accept() {
                        Ok(Some(accepted)) => {
                            if accepted.client_hello().server_name()
                                != Some("database.fixture.test")
                            {
                                return Err(std::io::Error::other(
                                    "owned TLS ClientHello lost the original database hostname",
                                ));
                            }
                            stats.original_sni.fetch_add(1, Ordering::SeqCst);
                            verified = true;
                            break;
                        }
                        Ok(None) => {}
                        Err((error, _)) => {
                            return Err(std::io::Error::other(format!(
                                "invalid owned ClientHello: {error}"
                            )));
                        }
                    }
                }
            }
            // Inspection never writes an SSL alert, handshake, or SQL response.
            server_write.write_all(&buffer[..count]).await?;
        }
    };
    let incoming = async {
        tokio::io::copy(&mut server_read, &mut client_write).await?;
        client_write.shutdown().await
    };
    tokio::try_join!(outgoing, incoming)?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable MySQL/MariaDB and scripts/mysql-tls-fixture.py"]
async fn verified_tls_over_ssh_protects_retained_browse_and_reviewed_writer() {
    use russh::keys::{Algorithm, PrivateKey};
    let ca = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache/mysql-fixture-tls/ca.pem")
        .canonicalize()
        .expect("prepare the disposable fixture CA first");
    for port in [53306, 53307] {
        fixture(port, 1, |tables| {
            let ca = ca.clone();
            async move {
                let table = &tables[0]; let mut observe = observer(port).await?;
                create(&mut observe, table, "original").await?;
                let relay = TlsWireRelay::start(port).await?;
                let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
                let bastion = common::bastion(relay.port, &identity, None, 0).await;
                let root = tempfile::tempdir()?;
                let owned = OwnedStudio(Studio::with_mysql_guard_policy(root.path(), Arc::new(MemorySecrets::default()), Arc::new(DisabledFileDialog), policy(Duration::from_secs(20)))?);
                let studio = &owned.0;
                let mut p = profile(relay.port); p.safe_mode = SafeMode::Confirm;
                p.host = "database.fixture.test".into(); p.tls_mode = TlsMode::VerifyFull;
                p.ca_cert_path = Some(ca.to_string_lossy().into());
                p.ssh = Some(SshConfig { host: "127.0.0.1".into(), port: bastion.port, username: "fixture".into(), authentication: SshAuthentication::Password, private_key_path: None, agent_socket_path: None });
                let p = studio.save_profile(p, None)?;
                let host = timeout(DEADLINE, studio.probe_ssh(&p.id)).await??;
                assert_eq!(host.fingerprint, bastion.key.public_key().fingerprint(russh::keys::ssh_key::HashAlg::Sha256).to_string());
                studio.trust_ssh_key(host.challenge_id.as_deref().ok_or("owned host-key challenge missing")?, &host.fingerprint)?;
                let session = timeout(DEADLINE, studio.connect(&p.id, ConnectionSecrets { password: Some("joinery-local-only".into()), ssh_password: Some("ssh-test-only".into()), ..Default::default() })).await??;
                assert!(session.encrypted);
                let connected_channels = relay.evidence.assert_verified();
                assert!(connected_channels >= 1);
                let page = timeout(DEADLINE, studio.browse(&session.id, table, None)).await??;
                assert!(matches!(&page.result.editability, Some(ResultEditability::Editable { table: actual, .. }) if actual == table));
                assert_eq!(studio.results.row(&page.result.id, 0)?, vec![Some("1".into()), Some("original".into())]);
                let browse_channels = relay.evidence.assert_verified();
                // Besides the editor, browse creates independent metadata/read
                // connections and the persistent guard. MDL proves that the
                // guard is active, rather than inferring it from channel counts.
                assert!(browse_channels >= connected_channels + 3, "guarded browse did not exercise independent TLS channels");
                blocked_drop(&mut observe, table).await?;
                let preview = studio.prepare_changes(&session.id, &page.result.id, vec![CellChange { row: 0, column: "c1".into(), value: Some("tls-approved".into()) }])?;
                assert!(preview.plan.requires_approval);
                let approval = studio.authorize(&preview.plan.id, None, preview.plan.confirmation_text.as_deref())?;
                let before_writer = relay.evidence.assert_verified();
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let run = studio.execute(&preview.plan.id, Some(&approval), Arc::new(move |event| tx.send(event).is_ok()))?;
                let events = match timeout(DEADLINE, async {
                    let mut events = Vec::new();
                    while let Some(event) = rx.recv().await {
                        let finished = matches!(event, QueryEvent::Finished { .. });
                        events.push(event);
                        if finished { return Ok::<_, Box<dyn Error + Send + Sync>>(events); }
                    }
                    Err("reviewed writer ended without Finished".into())
                }).await {
                    Ok(result) => result?,
                    Err(error) => { let _ = timeout(Duration::from_secs(3), studio.cancel(&run)).await; return Err(error.into()); }
                };
                assert!(!events.iter().any(|event| matches!(event, QueryEvent::Error { .. })), "{events:?}");
                assert!(events.iter().any(|event| matches!(event, QueryEvent::Finished { status: RunStatus::Completed, .. })), "{events:?}");
                // The reviewed writer opens another auxiliary TCP channel;
                // all accepted channels must have requested TLS and original SNI.
                let written_channels = relay.evidence.assert_verified();
                assert!(written_channels > before_writer, "reviewed write did not exercise an auxiliary TLS channel");
                assert_eq!(note(&mut observe, table).await?, "tls-approved");
                assert_eq!(studio.execute(&preview.plan.id, Some(&approval), Arc::new(|_| true)).unwrap_err().code, ErrorCode::ApprovalExpired);
                studio.release_result(&page.result.id); studio.disconnect(&session.id);
                timeout(Duration::from_secs(5), async { while relay.evidence.active.load(Ordering::SeqCst) != 0 { tokio::time::sleep(Duration::from_millis(10)).await; } }).await?;
                relay.evidence.assert_verified();
                assert!(bastion.connections.load(Ordering::SeqCst) >= 1);
                Ok(())
            }
        }).await;
    }
}
