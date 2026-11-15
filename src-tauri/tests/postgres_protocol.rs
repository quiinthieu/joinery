//! Regression coverage for the vendored actual-origin protocol API.
//! Synthetic tests use in-memory duplex streams or isolated loopback sockets.
//! Only the ignored fixture test connects to a separately running database.
mod startup_cleanup {
    use joinery_core::{
        domain::*, drivers::postgres::Postgres, execution::Studio, secrets::MemorySecrets,
    };
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    const BOUND: Duration = Duration::from_secs(3);
    type StartupStages = Arc<Mutex<Vec<(&'static str, u128)>>>;

    fn profile(port: u16) -> ConnectionProfile {
        ConnectionProfile {
            id: new_id(),
            name: "Loopback PostgreSQL startup".into(),
            group_id: "local".into(),
            engine: EngineKind::Postgres,
            host: "127.0.0.1".into(),
            port,
            database: "fixture".into(),
            username: "fixture".into(),
            environment: Environment::Local,
            safe_mode: SafeMode::ReadOnly,
            tls_mode: TlsMode::Disable,
            ca_cert_path: None,
            ssh: None,
            remember_password: false,
            store_query_text: false,
            store_drafts: false,
            snowflake: None,
        }
    }
    fn message(tag: u8, body: &[u8], output: &mut Vec<u8>) {
        output.push(tag);
        output.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        output.extend_from_slice(body);
    }
    struct Peer {
        port: u16,
        started: Instant,
        stages: StartupStages,
        identity: oneshot::Receiver<()>,
        eof: oneshot::Receiver<bool>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Peer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl Peer {
        async fn new() -> Self {
            let started = Instant::now();
            let stages = Arc::new(Mutex::new(Vec::new()));
            let observed_stages = stages.clone();
            let mark = move |stage: &'static str| {
                observed_stages
                    .lock()
                    .unwrap()
                    .push((stage, started.elapsed().as_micros()));
            };
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            mark("listener-ready");
            let port = listener.local_addr().unwrap().port();
            let (identity_tx, identity) = oneshot::channel();
            let (eof_tx, eof) = oneshot::channel();
            let task = tokio::spawn(async move {
                mark("accept-wait");
                let (mut stream, _) = listener.accept().await.unwrap();
                mark("accepted");
                let size = stream.read_u32().await.unwrap();
                assert!((8..4096).contains(&size));
                let mut startup = vec![0; size as usize - 4];
                stream.read_exact(&mut startup).await.unwrap();
                assert_eq!(&startup[..4], &196608_u32.to_be_bytes());
                mark("startup-read");
                let mut ready = Vec::new();
                message(b'R', &0_i32.to_be_bytes(), &mut ready);
                message(b'Z', b"I", &mut ready);
                stream.write_all(&ready).await.unwrap();
                mark("authentication-ready-written");
                let mut identity_query = false;
                let mut first_message = true;
                loop {
                    let tag = stream.read_u8().await.unwrap();
                    if first_message {
                        mark("first-frontend-message");
                        first_message = false;
                    }
                    let size = stream.read_u32().await.unwrap();
                    assert!((4..4096).contains(&size));
                    let mut body = vec![0; size as usize - 4];
                    stream.read_exact(&mut body).await.unwrap();
                    if tag == b'P' {
                        identity_query = body
                            .windows(b"SELECT version()".len())
                            .any(|window| window == b"SELECT version()");
                    }
                    if tag == b'S' {
                        mark("identity-sync-received");
                        break;
                    }
                }
                assert!(
                    identity_query,
                    "the production initial identity query must reach the peer"
                );
                identity_tx.send(()).unwrap();
                // Deliberately withhold the identity statement's first response.
                // Closing the fixture itself after timeout must not count as EOF.
                let closed = tokio::time::timeout(BOUND, async {
                    let mut bytes = [0; 1024];
                    loop {
                        match stream.read(&mut bytes).await {
                            Ok(0) => return true,
                            Ok(_) => {}
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::UnexpectedEof
                                        | std::io::ErrorKind::ConnectionReset
                                        | std::io::ErrorKind::ConnectionAborted
                                        | std::io::ErrorKind::BrokenPipe
                                ) =>
                            {
                                return true;
                            }
                            Err(_) => return false,
                        }
                    }
                })
                .await
                .unwrap_or(false);
                let _ = eof_tx.send(closed);
            });
            Self {
                port,
                started,
                stages,
                identity,
                eof,
                task,
            }
        }
        async fn dispatched(&mut self) {
            match tokio::time::timeout(BOUND, &mut self.identity).await {
                Ok(Ok(())) => {}
                outcome => panic!(
                    "identity dispatch failed after {}us: {outcome:?}; peer stages={:?}",
                    self.started.elapsed().as_micros(),
                    self.stages.lock().unwrap()
                ),
            }
        }
        async fn assert_closed(&mut self) {
            assert!(
                tokio::time::timeout(BOUND + Duration::from_secs(1), &mut self.eof)
                    .await
                    .unwrap()
                    .unwrap(),
                "abandoned PostgreSQL startup left its connection task/socket alive"
            );
            (&mut self.task).await.unwrap();
        }
    }

    #[tokio::test]
    async fn cancelled_studio_setup_aborts_the_postgres_task_before_identity_returns() {
        let mut peer = Peer::new().await;
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        let profile = studio.save_profile(profile(peer.port), None).unwrap();
        let id = studio.prepare_connection(&profile.id).unwrap();
        let connecting = studio.clone();
        let attempt = id.clone();
        let setup = tokio::spawn(async move {
            connecting
                .connect_prepared(&attempt, ConnectionSecrets::default())
                .await
        });
        peer.dispatched().await;
        studio.cancel_connection_attempt(&id);
        assert_eq!(
            tokio::time::timeout(BOUND, setup)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        studio.shutdown_and_wait().await.unwrap();
        assert_eq!(studio.usage().session_count, 0);
        peer.assert_closed().await;
    }

    #[tokio::test]
    async fn abandoned_open_future_aborts_the_postgres_task_before_identity_returns() {
        let mut peer = Peer::new().await;
        let profile = profile(peer.port);
        let setup =
            tokio::spawn(
                async move { Postgres::open(&profile, None, "loopback-session", None).await },
            );
        peer.dispatched().await;
        setup.abort();
        assert!(setup.await.err().unwrap().is_cancelled());
        peer.assert_closed().await;
    }

    #[tokio::test]
    async fn explicit_ca_validation_rejects_empty_and_invalid_pem_in_both_tls_modes() {
        let directory = tempfile::tempdir().unwrap();
        for mode in [TlsMode::Disable, TlsMode::VerifyFull] {
            for (name, bytes, expected) in [
                (
                    "empty.pem",
                    "",
                    "The selected file contains no PEM certificates",
                ),
                (
                    "invalid.pem",
                    "-----BEGIN CERTIFICATE-----\n!\n-----END CERTIFICATE-----\n",
                    "Invalid CA certificate",
                ),
            ] {
                let path = directory.path().join(name);
                std::fs::write(&path, bytes).unwrap();
                let mut profile = profile(0);
                profile.tls_mode = mode;
                profile.ca_cert_path = Some(path.to_str().unwrap().into());
                let error = match Postgres::open(&profile, None, "invalid-ca", None).await {
                    Ok(_) => panic!("{mode:?}/{name} unexpectedly accepted an invalid CA file"),
                    Err(error) => error,
                };
                assert_eq!(error.code, ErrorCode::InvalidInput, "{mode:?}/{name}");
                assert_eq!(error.message, expected, "{mode:?}/{name}");
            }
        }
    }
}

mod synthetic {

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_postgres::{Config, NoTls, SimpleQueryMessage};

    fn message(tag: u8, body: &[u8], out: &mut Vec<u8>) {
        out.push(tag);
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(body);
    }

    fn description(fields: &[(&str, u32, i16, u32)], out: &mut Vec<u8>) {
        let mut body = Vec::new();
        body.extend_from_slice(&(fields.len() as i16).to_be_bytes());
        for &(name, table, column, ty) in fields {
            body.extend_from_slice(name.as_bytes());
            body.push(0);
            body.extend_from_slice(&table.to_be_bytes());
            body.extend_from_slice(&column.to_be_bytes());
            body.extend_from_slice(&ty.to_be_bytes());
            body.extend_from_slice(&(-1_i16).to_be_bytes());
            body.extend_from_slice(&(-1_i32).to_be_bytes());
            body.extend_from_slice(&0_i16.to_be_bytes());
        }
        message(b'T', &body, out);
    }

    async fn run_response(response: Vec<u8>) -> Vec<SimpleQueryMessage> {
        let (client_stream, mut server) = tokio::io::duplex(8192);
        let peer = tokio::spawn(async move {
            let len = server.read_u32().await.unwrap();
            let mut startup = vec![0; len as usize - 4];
            server.read_exact(&mut startup).await.unwrap();
            let mut auth = Vec::new();
            message(b'R', &0_i32.to_be_bytes(), &mut auth);
            message(b'Z', b"I", &mut auth);
            server.write_all(&auth).await.unwrap();
            assert_eq!(server.read_u8().await.unwrap(), b'Q');
            let len = server.read_u32().await.unwrap();
            let mut query = vec![0; len as usize - 4];
            server.read_exact(&mut query).await.unwrap();
            assert_eq!(query, b"synthetic only\0");
            server.write_all(&response).await.unwrap();
            let mut discard = [0; 256];
            while server.read(&mut discard).await.unwrap_or(0) != 0 {}
        });
        let (client, connection) = Config::new()
            .user("prototype")
            .connect_raw(client_stream, NoTls)
            .await
            .unwrap();
        let connection = tokio::spawn(connection);
        let messages = client.simple_query("synthetic only").await.unwrap();
        drop(client);
        connection.await.unwrap().unwrap();
        peer.await.unwrap();
        messages
    }

    async fn actual_description_survives_with_no_data_rows_case() {
        let mut response = Vec::new();
        description(
            &[("id", 41, 1, 23), ("tableoid", 41, -6, 26)],
            &mut response,
        );
        message(b'C', b"SELECT 0\0", &mut response);
        message(b'Z', b"I", &mut response);
        let messages = run_response(response).await;
        assert_eq!(messages.len(), 2);
        let SimpleQueryMessage::RowDescription(columns) = &messages[0] else {
            panic!("missing description")
        };
        assert_eq!(
            (
                columns[0].name(),
                columns[0].table_oid(),
                columns[0].column_id(),
                columns[0].type_oid()
            ),
            ("id", Some(41), Some(1), 23)
        );
        assert_eq!(
            (
                columns[1].table_oid(),
                columns[1].column_id(),
                columns[1].type_oid()
            ),
            (Some(41), Some(-6), 26)
        );
        assert!(matches!(
            messages[1],
            SimpleQueryMessage::CommandComplete(0)
        ));
    }

    async fn expression_origin_absent_and_each_result_has_own_origin_case() {
        let mut response = Vec::new();
        description(&[("expression", 0, 0, 1700)], &mut response);
        message(b'C', b"SELECT 0\0", &mut response);
        description(&[("id", 99, 3, 987654)], &mut response);
        message(b'D', &[0, 1, 0, 0, 0, 1, b'7'], &mut response);
        message(b'C', b"SELECT 1\0", &mut response);
        message(b'Z', b"I", &mut response);
        let messages = run_response(response).await;
        let SimpleQueryMessage::RowDescription(first) = &messages[0] else {
            panic!("missing first description")
        };
        assert_eq!(
            (
                first[0].table_oid(),
                first[0].column_id(),
                first[0].type_oid()
            ),
            (None, None, 1700)
        );
        let SimpleQueryMessage::RowDescription(second) = &messages[2] else {
            panic!("missing second description")
        };
        assert_eq!(
            (
                second[0].table_oid(),
                second[0].column_id(),
                second[0].type_oid()
            ),
            (Some(99), Some(3), 987654)
        );
        let SimpleQueryMessage::Row(row) = &messages[3] else {
            panic!("missing row")
        };
        assert_eq!(row.get(0), Some("7"));
        assert_eq!(row.columns()[0].table_oid(), Some(99));
    }

    async fn portal_case(actual_oid: u32, authorize: bool) {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let allowed = Arc::new(AtomicBool::new(false));
        let executed = Arc::new(AtomicBool::new(false));
        let (client_stream, mut server) = tokio::io::duplex(8192);
        let peer_allowed = allowed.clone();
        let peer_executed = executed.clone();
        let peer = tokio::spawn(async move {
            let len = server.read_u32().await.unwrap();
            let mut startup = vec![0; len as usize - 4];
            server.read_exact(&mut startup).await.unwrap();
            let mut response = Vec::new();
            message(b'R', &0_i32.to_be_bytes(), &mut response);
            message(b'Z', b"I", &mut response);
            server.write_all(&response).await.unwrap();
            let mut portal_name = Vec::new();
            let mut described = false;
            while let Ok(tag) = server.read_u8().await {
                let len = server.read_u32().await.unwrap();
                let mut body = vec![0; len as usize - 4];
                server.read_exact(&mut body).await.unwrap();
                let mut response = Vec::new();
                match tag {
                    b'Q' => {
                        assert!(body == b"START TRANSACTION\0" || body == b"ROLLBACK\0");
                        let ready = if body == b"START TRANSACTION\0" {
                            b'T'
                        } else {
                            b'I'
                        };
                        message(b'C', &body, &mut response);
                        message(b'Z', &[ready], &mut response);
                    }
                    b'P' => message(b'1', &[], &mut response),
                    b'B' => {
                        portal_name = body[..body.iter().position(|b| *b == 0).unwrap()].to_vec();
                        message(b'2', &[], &mut response);
                    }
                    b'D' => {
                        if body[0] == b'S' {
                            message(b't', &[0, 0], &mut response);
                            description(&[("tableoid", 41, -6, 26)], &mut response);
                        } else {
                            assert_eq!(body[0], b'P');
                            assert_eq!(&body[1..body.len() - 1], &portal_name);
                            described = true;
                            if actual_oid == 0 {
                                message(b'n', &[], &mut response);
                            } else {
                                description(&[("tableoid", actual_oid, -6, 26)], &mut response);
                                let format_offset = response.len() - 2;
                                response[format_offset..].copy_from_slice(&1_i16.to_be_bytes());
                            }
                        }
                    }
                    b'S' => message(b'Z', b"T", &mut response),
                    b'E' => {
                        assert!(described, "Execute preceded portal Describe");
                        assert!(
                            peer_allowed.load(Ordering::SeqCst),
                            "Execute preceded application validation"
                        );
                        let end = body.iter().position(|b| *b == 0).unwrap();
                        assert_eq!(&body[..end], &portal_name, "Executed a different portal");
                        peer_executed.store(true, Ordering::SeqCst);
                        let mut row = vec![0, 1, 0, 0, 0, 4];
                        row.extend_from_slice(&actual_oid.to_be_bytes());
                        message(b'D', &row, &mut response);
                        message(b'C', b"UPDATE 1\0", &mut response);
                    }
                    b'C' => message(b'3', &[], &mut response),
                    b'X' => break,
                    _ => panic!("Unexpected frontend message {}", tag as char),
                }
                server.write_all(&response).await.unwrap();
            }
        });
        let (mut client, connection) = Config::new()
            .user("prototype")
            .connect_raw(client_stream, NoTls)
            .await
            .unwrap();
        let connection = tokio::spawn(connection);
        let transaction = client.transaction().await.unwrap();
        let statement = transaction
            .prepare("synthetic mutation RETURNING tableoid")
            .await
            .unwrap();
        assert_eq!(statement.columns()[0].table_oid(), Some(41));
        let portal = transaction.bind(&statement, &[]).await.unwrap();
        if actual_oid == 0 {
            assert!(
                portal.columns().is_empty(),
                "NoData must not reuse statement metadata"
            );
        } else {
            assert_eq!(portal.columns()[0].table_oid(), Some(actual_oid));
            assert_eq!(portal.columns()[0].column_id(), Some(-6));
            assert_eq!(portal.columns()[0].type_().oid(), 26);
        }
        assert!(!executed.load(Ordering::SeqCst));
        if authorize {
            assert_eq!(actual_oid, 41);
            allowed.store(true, Ordering::SeqCst);
            let rows = transaction.query_portal(&portal, 0).await.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].columns()[0].table_oid(), Some(actual_oid));
            assert_eq!(rows[0].get::<_, u32>(0), actual_oid);
        } else {
            assert_ne!(actual_oid, 41);
        }
        drop(portal);
        drop(statement);
        transaction.rollback().await.unwrap();
        drop(client);
        connection.await.unwrap().unwrap();
        peer.await.unwrap();
        assert_eq!(executed.load(Ordering::SeqCst), authorize);
    }

    async fn stale_statement_origin_is_replaced_by_bound_portal_origin_without_execute_case() {
        portal_case(99, false).await;
    }

    async fn validated_bound_portal_is_the_one_executed_and_decoded_case() {
        portal_case(41, true).await;
    }

    async fn no_data_portal_does_not_inherit_stale_statement_columns_case() {
        portal_case(0, false).await;
    }

    async fn cancelled_bind_closes_the_named_portal_without_execute_case() {
        use std::sync::Arc;
        use tokio::sync::Notify;
        let bound = Arc::new(Notify::new());
        let peer_bound = bound.clone();
        let (client_stream, mut server) = tokio::io::duplex(8192);
        let peer = tokio::spawn(async move {
            let len = server.read_u32().await.unwrap();
            let mut startup = vec![0; len as usize - 4];
            server.read_exact(&mut startup).await.unwrap();
            let mut response = Vec::new();
            message(b'R', &0_i32.to_be_bytes(), &mut response);
            message(b'Z', b"I", &mut response);
            server.write_all(&response).await.unwrap();
            let mut name = Vec::new();
            let mut closed = false;
            while let Ok(tag) = server.read_u8().await {
                let len = server.read_u32().await.unwrap();
                let mut body = vec![0; len as usize - 4];
                server.read_exact(&mut body).await.unwrap();
                let mut response = Vec::new();
                match tag {
                    b'Q' => {
                        if body == b"ROLLBACK\0" {
                            assert!(closed);
                        }
                        message(b'C', &body, &mut response);
                        message(
                            b'Z',
                            if body == b"ROLLBACK\0" { b"I" } else { b"T" },
                            &mut response,
                        );
                    }
                    b'P' => message(b'1', &[], &mut response),
                    b'D' if body[0] == b'S' => {
                        message(b't', &[0, 0], &mut response);
                        description(&[("tableoid", 41, -6, 26)], &mut response);
                    }
                    b'B' => {
                        name = body[..body.iter().position(|v| *v == 0).unwrap()].to_vec();
                        message(b'2', &[], &mut response);
                        server.write_all(&response).await.unwrap();
                        peer_bound.notify_one();
                        continue;
                    }
                    b'D' => {
                        // Delay the metadata response until the client cancels bind.
                        assert_eq!(body[0], b'P');
                    }
                    b'S' => {
                        if !name.is_empty() && !closed {
                            // Complete the abandoned request; the client has already
                            // queued Close(P) behind it, without any Execute.
                            description(&[("tableoid", 41, -6, 26)], &mut response);
                        }
                        message(b'Z', b"T", &mut response);
                    }
                    b'C' => {
                        if body[0] == b'P' {
                            assert_eq!(&body[1..body.len() - 1], &name);
                            closed = true;
                        }
                        message(b'3', &[], &mut response);
                    }
                    b'E' => panic!("cancelled bind must never Execute"),
                    b'X' => break,
                    _ => panic!("unexpected frontend message {}", tag as char),
                }
                server.write_all(&response).await.unwrap();
            }
            assert!(closed, "cancelled bind leaked the named portal");
        });
        let (mut client, connection) = Config::new()
            .user("prototype")
            .connect_raw(client_stream, NoTls)
            .await
            .unwrap();
        let connection = tokio::spawn(connection);
        let transaction = client.transaction().await.unwrap();
        let statement = transaction
            .prepare("synthetic mutation RETURNING tableoid")
            .await
            .unwrap();
        let mut binding = Box::pin(transaction.bind(&statement, &[]));
        tokio::select! {
            biased;
            _ = bound.notified() => {},
            _ = &mut binding => panic!("bind should wait for its portal metadata"),
        }
        drop(binding);
        drop(statement);
        transaction.rollback().await.unwrap();
        drop(client);
        connection.await.unwrap().unwrap();
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn actual_description_survives_with_no_data_rows() {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            actual_description_survives_with_no_data_rows_case(),
        )
        .await
        .expect("synthetic protocol scenario exceeded 5 seconds");
    }

    #[tokio::test]
    async fn expression_origin_absent_and_each_result_has_own_origin() {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            expression_origin_absent_and_each_result_has_own_origin_case(),
        )
        .await
        .expect("synthetic protocol scenario exceeded 5 seconds");
    }

    #[tokio::test]
    async fn stale_statement_origin_is_replaced_by_bound_portal_origin_without_execute() {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stale_statement_origin_is_replaced_by_bound_portal_origin_without_execute_case(),
        )
        .await
        .expect("synthetic protocol scenario exceeded 5 seconds");
    }

    #[tokio::test]
    async fn validated_bound_portal_is_the_one_executed_and_decoded() {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            validated_bound_portal_is_the_one_executed_and_decoded_case(),
        )
        .await
        .expect("synthetic protocol scenario exceeded 5 seconds");
    }

    #[tokio::test]
    async fn no_data_portal_does_not_inherit_stale_statement_columns() {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            no_data_portal_does_not_inherit_stale_statement_columns_case(),
        )
        .await
        .expect("synthetic protocol scenario exceeded 5 seconds");
    }

    #[tokio::test]
    async fn cancelled_bind_closes_the_named_portal_without_execute() {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            cancelled_bind_closes_the_named_portal_without_execute_case(),
        )
        .await
        .expect("synthetic protocol scenario exceeded 5 seconds");
    }
}

mod fixture {
    use std::{error::Error, time::Duration};
    use tokio_postgres::{Client, Config, NoTls, SimpleQueryMessage};
    type R<T> = Result<T, Box<dyn Error + Send + Sync>>;
    const CASE_TIMEOUT: Duration = Duration::from_secs(15);
    const CLEANUP_TIMEOUT: Duration = Duration::from_secs(6);

    // Aborting our connection task closes its socket and rolls back any pending
    // transaction even when a scenario is cancelled, errors, or unwinds.
    struct OwnedConnection {
        client: Client,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for OwnedConnection {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn connect() -> R<OwnedConnection> {
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(55432)
            .dbname("joinery")
            .user("joinery")
            .password("joinery-local-only")
            .connect_timeout(Duration::from_secs(3));
        let (client, connection) =
            tokio::time::timeout(CLEANUP_TIMEOUT, config.connect(NoTls)).await??;
        let owned = OwnedConnection {
            client,
            task: tokio::spawn(async move {
                if let Err(error) = connection.await {
                    eprintln!("fixture connection: {error}");
                }
            }),
        };
        tokio::time::timeout(
            CLEANUP_TIMEOUT,
            owned
                .client
                .batch_execute("SET statement_timeout = '5s'; SET lock_timeout = '1s'"),
        )
        .await??;
        Ok(owned)
    }
    fn q(v: &str) -> String {
        format!("\"{}\"", v.replace('"', "\"\""))
    }
    async fn oid(c: &Client, schema: &str, table: &str) -> R<u32> {
        Ok(c.query_one("SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2", &[&schema, &table]).await?.get(0))
    }
    async fn original(c: &Client, table: &str, expected: u32) -> R<()> {
        let messages = c
            .simple_query(&format!(
                "SELECT id, note, tableoid FROM {table} WHERE false"
            ))
            .await?;
        let columns = messages
            .iter()
            .find_map(|m| {
                if let SimpleQueryMessage::RowDescription(cs) = m {
                    Some(cs)
                } else {
                    None
                }
            })
            .ok_or("missing zero-row descriptor")?;
        if columns.len() != 3
            || columns[0].table_oid() != Some(expected)
            || columns[0].column_id() != Some(1)
            || columns[0].type_oid() != 23
            || columns[2].table_oid() != Some(expected)
            || columns[2].column_id() != Some(-6)
            || columns[2].type_oid() != 26
        {
            return Err(format!("unexpected original origin {columns:?}").into());
        }
        Ok(())
    }
    async fn rows(c: &Client, table: &str) -> R<Vec<(i32, String)>> {
        Ok(
            c.query(&format!("SELECT id,note FROM {table} ORDER BY id"), &[])
                .await?
                .into_iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect(),
        )
    }
    async fn replace_namespace(c: &Client, schema: &str, renamed: &str, sequence: &str) -> R<()> {
        c.batch_execute(&format!("ALTER SCHEMA {} RENAME TO {}; CREATE SCHEMA {}; CREATE TABLE {}.target(id integer PRIMARY KEY, note text NOT NULL DEFAULT nextval('{}')::text); INSERT INTO {}.target VALUES(1,'before')", q(schema),q(renamed),q(schema),q(schema),sequence,q(schema))).await?;
        Ok(())
    }
    async fn namespace_case(
        writer: &mut Client,
        observer: &Client,
        schema: &str,
        renamed: &str,
        sequence: &str,
        after_bind: bool,
        mode: &str,
    ) -> R<()> {
        let target = format!("{}.target", q(schema));
        observer.batch_execute(&format!("CREATE SCHEMA {}; CREATE TABLE {target}(id integer PRIMARY KEY, note text NOT NULL DEFAULT nextval('{sequence}')::text); INSERT INTO {target} VALUES(1,'before')",q(schema))).await?;
        let expected = oid(observer, schema, "target").await?;
        original(writer, &target, expected).await?;
        writer
            .batch_execute(&format!("SET plan_cache_mode={mode}"))
            .await?;
        let tx = writer.transaction().await?;
        let statement = tx
            .prepare(&format!(
                "INSERT INTO {target}(id) VALUES($1) RETURNING tableoid"
            ))
            .await?;
        if statement.columns().len() != 1 || statement.columns()[0].table_oid() != Some(expected) {
            return Err("prepared origin wrong".into());
        }
        if !after_bind {
            replace_namespace(observer, schema, renamed, sequence).await?;
        }
        let portal = tx.bind(&statement, &[&2_i32]).await?;
        let columns = portal.columns();
        if columns.len() != 1 {
            return Err("expected one portal origin".into());
        }
        let actual = columns[0].table_oid().ok_or("missing portal origin")?;
        if columns[0].column_id() != Some(-6) || columns[0].type_().oid() != 26 {
            return Err("portal attr/type wrong".into());
        }
        if after_bind {
            if actual != expected {
                return Err("original bound portal did not match".into());
            }
            replace_namespace(observer, schema, renamed, sequence).await?;
            let returned = tx.query_portal(&portal, 0).await?;
            if returned.len() != 1 || returned[0].get::<_, u32>(0) != expected {
                return Err("bound execution target changed".into());
            }
            drop(portal);
            tx.commit().await?;
            let oldrows = rows(observer, &format!("{}.target", q(renamed))).await?;
            if oldrows.len() != 2
                || oldrows[1].0 != 2
                || rows(observer, &target).await? != vec![(1, "before".into())]
            {
                return Err("same-portal execution touched replacement or missed original".into());
            }
        } else {
            let replacement = oid(observer, schema, "target").await?;
            if actual == replacement && actual != expected {
                // Strict rejection: no Execute is sent, no post-mutation origin check.
                drop(portal);
                tx.rollback().await?;
                if rows(observer, &target).await? != vec![(1, "before".into())]
                    || rows(observer, &format!("{}.target", q(renamed))).await?
                        != vec![(1, "before".into())]
                {
                    return Err("rejected portal changed a table".into());
                }
                let called: bool = observer
                    .query_one(&format!("SELECT is_called FROM {sequence}"), &[])
                    .await?
                    .get(0);
                if called {
                    return Err("rejected bind ran nontransactional default sequence".into());
                }
            } else if actual == expected {
                // PG may preserve the original relation under this transaction's locks.
                // Descriptor alone is insufficient proof: execute and independently observe both.
                let returned = tx.query_portal(&portal, 0).await?;
                if returned.len() != 1 || returned[0].get::<_, u32>(0) != expected {
                    return Err("before-bind descriptor did not describe actual target".into());
                }
                drop(portal);
                tx.commit().await?;
                if rows(observer, &target).await? != vec![(1, "before".into())]
                    || rows(observer, &format!("{}.target", q(renamed)))
                        .await?
                        .len()
                        != 2
                {
                    return Err(
                        "before-bind execution touched replacement or missed original".into(),
                    );
                }
            } else {
                return Err(format!("unknown origin {actual}").into());
            }
        }
        eprintln!(
            "PASS namespace after_bind={after_bind} mode={mode} statement_oid={expected} portal_oid={actual}"
        );
        Ok(())
    }
    async fn drop_before_bind(
        writer: &mut Client,
        observer: &Client,
        schema: &str,
        sequence: &str,
        mode: &str,
    ) -> R<()> {
        let target = format!("{}.target", q(schema));
        observer.batch_execute(&format!("CREATE SCHEMA {}; CREATE TABLE {target}(id integer PRIMARY KEY,note text DEFAULT nextval('{sequence}')::text); INSERT INTO {target} VALUES(1,'before')",q(schema))).await?;
        let expected = oid(observer, schema, "target").await?;
        original(writer, &target, expected).await?;
        writer
            .batch_execute(&format!("SET plan_cache_mode={mode}"))
            .await?;
        // Prepare outside explicit transaction: release table locks before replacement.
        let statement = writer
            .prepare(&format!(
                "INSERT INTO {target}(id) VALUES($1) RETURNING tableoid"
            ))
            .await?;
        observer.batch_execute(&format!("DROP TABLE {target}; CREATE TABLE {target}(id integer PRIMARY KEY,note text DEFAULT nextval('{sequence}')::text); INSERT INTO {target} VALUES(1,'before')")).await?;
        let replacement = oid(observer, schema, "target").await?;
        let tx = writer.transaction().await?;
        let portal = tx.bind(&statement, &[&2_i32]).await?;
        if portal.columns().len() != 1 {
            return Err("expected one portal origin".into());
        }
        let c = &portal.columns()[0];
        if c.table_oid() != Some(replacement)
            || c.table_oid() == Some(expected)
            || c.column_id() != Some(-6)
            || c.type_().oid() != 26
        {
            return Err(format!("replacement portal descriptor mismatch {c:?}").into());
        }
        drop(portal);
        tx.rollback().await?;
        let called: bool = observer
            .query_one(&format!("SELECT is_called FROM {sequence}"), &[])
            .await?
            .get(0);
        if called || rows(observer, &target).await? != vec![(1, "before".into())] {
            return Err("rejected replacement executed default/DML".into());
        }
        eprintln!(
            "PASS drop/recreate before bind {mode} statement_oid={expected} portal_oid={replacement}, no DML/default"
        );
        Ok(())
    }
    async fn descriptor_case(
        writer: &mut Client,
        observer: &Client,
        schema: &str,
        kind: &str,
    ) -> R<()> {
        observer
            .batch_execute(&format!("CREATE SCHEMA {}", q(schema)))
            .await?;
        let target = format!("{}.target", q(schema));
        let ddl = match kind {
            "plain" => format!("CREATE TABLE {target}(id integer PRIMARY KEY,note text NOT NULL)"),
            "partition" => format!(
                "CREATE TABLE {target}(id integer,note text NOT NULL) PARTITION BY RANGE(id); CREATE TABLE {}.child PARTITION OF {target} FOR VALUES FROM(0) TO(10)",
                q(schema)
            ),
            "inheritance" => format!(
                "CREATE TABLE {target}(id integer,note text NOT NULL); CREATE TABLE {}.child() INHERITS({target})",
                q(schema)
            ),
            _ => unreachable!(),
        };
        observer.batch_execute(&ddl).await?;
        let seed = if kind == "inheritance" {
            format!("{}.child", q(schema))
        } else {
            target.clone()
        };
        observer
            .batch_execute(&format!("INSERT INTO {seed} VALUES(1,'before')"))
            .await?;
        let expected = oid(observer, schema, "target").await?;
        original(writer, &target, expected).await?;
        let physical = if kind == "plain" {
            expected
        } else {
            oid(observer, schema, "child").await?
        };
        let tx = writer.transaction().await?;
        for sql in [
            format!("UPDATE {target} SET note='approved' WHERE id=1 RETURNING tableoid"),
            format!("DELETE FROM {target} WHERE id=1 RETURNING tableoid"),
            format!("INSERT INTO {target} VALUES(2,'inserted') RETURNING tableoid"),
            format!("UPDATE {target} SET note='none' WHERE id=999 RETURNING tableoid"),
        ] {
            let statement = tx.prepare(&sql).await?;
            let portal = tx.bind(&statement, &[]).await?;
            if portal.columns().len() != 1 {
                return Err("expected one portal origin".into());
            }
            let c = &portal.columns()[0];
            if c.table_oid() != Some(expected) || c.column_id() != Some(-6) || c.type_().oid() != 26
            {
                return Err(format!("{kind} origin mismatch {c:?}").into());
            }
            let returned = tx.query_portal(&portal, 0).await?;
            if sql.contains("id=999") {
                if !returned.is_empty() {
                    return Err("zero-row returned data".into());
                }
            } else {
                if returned.len() != 1 {
                    return Err("expected one row".into());
                }
                let got: u32 = returned[0].get(0);
                let want = if sql.starts_with("INSERT") && kind == "inheritance" {
                    expected
                } else {
                    physical
                };
                if got != want {
                    return Err(format!("physical oid {got}!={want}").into());
                }
            }
        }
        tx.rollback().await?;
        if rows(observer, &target).await? != vec![(1, "before".into())] {
            return Err("descriptor rollback failed".into());
        }
        eprintln!(
            "PASS descriptors {kind} logical={expected} physical={physical}, INSERT/UPDATE/DELETE/zero-row and rollback"
        );
        Ok(())
    }

    struct Resources {
        sequence: String,
        schemas: Vec<String>,
    }

    async fn scenario<T>(
        name: &str,
        work: impl std::future::Future<Output = R<T>>,
        errors: &mut Vec<String>,
    ) {
        match tokio::time::timeout(CASE_TIMEOUT, work).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => errors.push(format!("{name}: {error}")),
            Err(_) => errors.push(format!("{name}: scenario exceeded 15 seconds")),
        }
    }

    // Resource cleanup is outside the fallible exercise block. Every owned name
    // is registered before creation, and failures do not skip later cleanup.
    async fn cleanup(resources: &Resources) -> R<()> {
        let cleanup = connect().await?;
        let mut errors = Vec::new();
        for schema in &resources.schemas {
            let sql = format!("DROP SCHEMA IF EXISTS {} CASCADE", q(schema));
            match tokio::time::timeout(CLEANUP_TIMEOUT, cleanup.client.batch_execute(&sql)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("drop {schema}: {error}")),
                Err(_) => errors.push(format!("drop {schema}: timeout")),
            }
        }
        let sql = format!("DROP SEQUENCE IF EXISTS {}", resources.sequence);
        match tokio::time::timeout(CLEANUP_TIMEOUT, cleanup.client.batch_execute(&sql)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => errors.push(format!("drop sequence: {error}")),
            Err(_) => errors.push("drop sequence: timeout".into()),
        }
        let verification = async {
            let remaining: i64 = cleanup
                .client
                .query_one(
                    "SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname=ANY($1)",
                    &[&resources.schemas],
                )
                .await?
                .get(0);
            let sequence_missing: bool = cleanup
                .client
                .query_one(
                    "SELECT pg_catalog.to_regclass($1) IS NULL",
                    &[&resources.sequence],
                )
                .await?
                .get(0);
            if remaining != 0 || !sequence_missing {
                return Err(format!(
                    "cleanup left {remaining} schemas; sequence_missing={sequence_missing}"
                )
                .into());
            }
            Ok::<(), Box<dyn Error + Send + Sync>>(())
        };
        match tokio::time::timeout(CLEANUP_TIMEOUT, verification).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => errors.push(format!("cleanup verification: {error}")),
            Err(_) => errors.push("cleanup verification: timeout".into()),
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; ").into())
        }
    }

    async fn exercise(resources: &Resources, prefix: &str) -> R<()> {
        let mut writer = connect().await?;
        let observer = connect().await?;
        let mut errors = Vec::new();
        // Keep setup failures inside this function: the caller still cleans all
        // pre-registered resources after both connections have been dropped.
        tokio::time::timeout(
            CLEANUP_TIMEOUT,
            observer
                .client
                .batch_execute(&format!("CREATE SEQUENCE {}", resources.sequence)),
        )
        .await??;
        for (mode, short) in [("force_generic_plan", "g"), ("force_custom_plan", "c")] {
            for after in [false, true] {
                let schema = format!("{prefix}_{short}_{}", u8::from(after));
                let renamed = format!("{schema}_old");
                let work = async {
                    observer
                        .client
                        .batch_execute(&format!(
                            "ALTER SEQUENCE {} RESTART WITH 1",
                            resources.sequence
                        ))
                        .await?;
                    namespace_case(
                        &mut writer.client,
                        &observer.client,
                        &schema,
                        &renamed,
                        &resources.sequence,
                        after,
                        mode,
                    )
                    .await
                };
                scenario(&format!("namespace {mode}/{after}"), work, &mut errors).await;
                // A failed/cancelled transaction queues rollback; synchronize it
                // before another scenario. This synchronization is bounded too.
                match tokio::time::timeout(CLEANUP_TIMEOUT, writer.client.simple_query("SELECT 1"))
                    .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => errors.push(format!("writer synchronization: {error}")),
                    Err(_) => errors.push("writer synchronization: timeout".into()),
                }
            }
        }
        for (mode, short) in [("force_generic_plan", "g"), ("force_custom_plan", "c")] {
            let schema = format!("{prefix}_drop_{short}");
            let work = async {
                observer
                    .client
                    .batch_execute(&format!(
                        "ALTER SEQUENCE {} RESTART WITH 1",
                        resources.sequence
                    ))
                    .await?;
                drop_before_bind(
                    &mut writer.client,
                    &observer.client,
                    &schema,
                    &resources.sequence,
                    mode,
                )
                .await
            };
            scenario(&format!("drop-before-bind {mode}"), work, &mut errors).await;
            match tokio::time::timeout(CLEANUP_TIMEOUT, writer.client.simple_query("SELECT 1"))
                .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => errors.push(format!("writer synchronization: {error}")),
                Err(_) => errors.push("writer synchronization: timeout".into()),
            }
        }
        for kind in ["plain", "partition", "inheritance"] {
            let schema = format!("{prefix}_{kind}");
            scenario(
                &format!("descriptor {kind}"),
                descriptor_case(&mut writer.client, &observer.client, &schema, kind),
                &mut errors,
            )
            .await;
            match tokio::time::timeout(CLEANUP_TIMEOUT, writer.client.simple_query("SELECT 1"))
                .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => errors.push(format!("writer synchronization: {error}")),
                Err(_) => errors.push("writer synchronization: timeout".into()),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; ").into())
        }
    }

    #[tokio::test]
    #[ignore = "explicit opt-in: authorized loopback PostgreSQL fixture 55432 only"]
    async fn actual_fixture_portal_identity_before_dml() -> R<()> {
        let prefix = format!("jp_{}", uuid::Uuid::new_v4().simple());
        let mut schemas = Vec::new();
        for short in ["g", "c"] {
            for after in [false, true] {
                let schema = format!("{prefix}_{short}_{}", u8::from(after));
                schemas.push(schema.clone());
                schemas.push(format!("{schema}_old"));
            }
            schemas.push(format!("{prefix}_drop_{short}"));
        }
        for kind in ["plain", "partition", "inheritance"] {
            schemas.push(format!("{prefix}_{kind}"));
        }
        let resources = Resources {
            sequence: format!("public.{}", q(&format!("{prefix}_seq"))),
            schemas,
        };
        // Convert a panic to a failure while still performing cleanup. Each
        // scenario has its own timeout, so an unexpected protocol stall also
        // reaches the independent cleanup phase.
        let exercise_result = {
            use futures_util::FutureExt;
            std::panic::AssertUnwindSafe(exercise(&resources, &prefix))
                .catch_unwind()
                .await
        };
        let cleanup_result = cleanup(&resources).await;
        match (exercise_result, cleanup_result) {
            (Ok(Ok(())), Ok(())) => {
                eprintln!(
                    "PASS nine actual fixture cases; owned schemas+sequence removed; no application integration"
                );
                Ok(())
            }
            (work, cleanup) => {
                let work = match work {
                    Ok(Ok(())) => "passed".into(),
                    Ok(Err(error)) => error.to_string(),
                    Err(_) => "scenario panicked".into(),
                };
                let cleanup = match cleanup {
                    Ok(()) => "confirmed".into(),
                    Err(error) => error.to_string(),
                };
                Err(format!("fixture: {work}; cleanup: {cleanup}").into())
            }
        }
    }
}
