use joinery_core::{domain::*, execution::Studio, safety, secrets::MemorySecrets};
use std::sync::Arc;
mod common;

fn profile() -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Redis fixture".into(),
        group_id: "local".into(),
        engine: EngineKind::Redis,
        host: "127.0.0.1".into(),
        port: 56379,
        database: "0".into(),
        username: "default".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::Confirm,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        snowflake: None,
    }
}

#[test]
fn redis_is_available_and_commands_use_backend_policy() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    assert!(
        studio
            .bootstrap()
            .unwrap()
            .supported_engines
            .contains(&EngineKind::Redis)
    );
    let mut p = profile();
    let plan = safety::prepare("test", &p, "GET \"a b\"\nPTTL \"a b\"").unwrap();
    assert_eq!(plan.statements.len(), 2);
    assert!(!plan.requires_approval);
    assert!(
        safety::prepare("test", &p, "SET key \"value; DEL other\"")
            .unwrap()
            .requires_approval
    );
    p.safe_mode = SafeMode::ReadOnly;
    assert_eq!(
        safety::prepare("test", &p, "GET k\nSET k v")
            .unwrap_err()
            .code,
        ErrorCode::PolicyBlocked
    );
    for command in [
        "KEYS *",
        "EVAL 'return 1' 0",
        "BLPOP k 0",
        "MULTI",
        "AUTH secret",
        "CONFIG GET *",
        "FLUSHALL",
        "GET k\nFLUSHDB",
    ] {
        assert!(
            safety::prepare("test", &profile(), command).is_err(),
            "{command}"
        );
    }
    p.environment = Environment::Production;
    p.safe_mode = SafeMode::Silent;
    let protected = safety::prepare("test", &p, "SET key value").unwrap();
    assert!(protected.requires_approval);
    assert!(protected.requires_passphrase);
}

async fn setup(mode: SafeMode) -> (tempfile::TempDir, Arc<Studio>, SessionInfo) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let mut p = profile();
    p.safe_mode = mode;
    let p = studio.save_profile(p, None).unwrap();
    let session = studio
        .connect(
            &p.id,
            ConnectionSecrets {
                password: Some("joinery-redis-local-only".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    (root, studio, session)
}
async fn run(studio: &Arc<Studio>, session: &SessionInfo, source: &str) -> Vec<QueryEvent> {
    let plan = studio.prepare_execution(&session.id, source).unwrap();
    let approval = if plan.requires_approval {
        Some(
            studio
                .authorize(&plan.id, None, plan.confirmation_text.as_deref())
                .unwrap(),
        )
    } else {
        None
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    studio
        .execute(
            &plan.id,
            approval.as_deref(),
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            let done = matches!(event, QueryEvent::Finished { .. });
            events.push(event);
            if done {
                break;
            }
        }
        events
    })
    .await
    .unwrap()
}
fn completed(events: &[QueryEvent]) {
    assert!(
        events.iter().any(|e| matches!(
            e,
            QueryEvent::Finished {
                status: RunStatus::Completed,
                ..
            }
        )),
        "{events:?}"
    );
}
fn rows(studio: &Studio, events: &[QueryEvent]) -> Vec<TextRow> {
    let result = events
        .iter()
        .filter_map(|e| {
            if let QueryEvent::Result { result } = e {
                Some(result)
            } else {
                None
            }
        })
        .next_back()
        .unwrap();
    (0..result.retained_rows)
        .map(|v| studio.results.row(&result.id, v).unwrap())
        .collect()
}

#[tokio::test]
#[ignore = "requires disposable Redis service"]
async fn console_preserves_bytes_null_empty_and_exact_integers_and_preflights_batches() {
    let (_root, studio, session) = setup(SafeMode::Confirm).await;
    let key = format!("redis-integration:{}", new_id());
    let batch = format!(
        "SET {key} \"\"\nHSET {key}:hash n 9223372036854775807 b \"\\x00\\xff\"\nGET {key}\nHGET {key}:hash n\nHGET {key}:hash b\nGET {key}:missing"
    );
    let events = run(&studio, &session, &batch).await;
    completed(&events);
    let result_rows = rows(&studio, &events);
    assert_eq!(result_rows[0][1], None);
    let integer = run(&studio, &session, &format!("HGET {key}:hash n")).await;
    completed(&integer);
    assert_eq!(
        rows(&studio, &integer)[0][1].as_deref(),
        Some("9223372036854775807")
    );
    let empty = run(&studio, &session, &format!("GET {key}")).await;
    assert_eq!(rows(&studio, &empty)[0][1].as_deref(), Some(""));
    let binary = run(&studio, &session, &format!("HGET {key}:hash b")).await;
    assert_eq!(
        rows(&studio, &binary)[0][1].as_deref(),
        Some("\"\\x00\\xff\"")
    );
    assert_eq!(
        rows(&studio, &binary)[0][2].as_deref(),
        Some("binary (escaped)")
    );
    let literal = joinery_core::drivers::redis::commands::argument(b"\"\\x00\\xff\"");
    completed(
        &run(
            &studio,
            &session,
            &format!("HSET {key}:hash literal {literal}"),
        )
        .await,
    );
    let text = run(&studio, &session, &format!("HGET {key}:hash literal")).await;
    assert_eq!(rows(&studio, &text)[0][1], rows(&studio, &binary)[0][1]);
    assert_eq!(rows(&studio, &text)[0][2].as_deref(), Some("UTF-8 string"));
    let preview = studio
        .redis_inspect(&session.id, &format!("{key}:hash"), None)
        .await
        .unwrap();
    assert!(
        preview
            .entries
            .iter()
            .find(|e| e.field.as_deref() == Some("b"))
            .unwrap()
            .value_binary
    );
    assert!(
        !preview
            .entries
            .iter()
            .find(|e| e.field.as_deref() == Some("literal"))
            .unwrap()
            .value_binary
    );
    assert!(
        studio
            .prepare_execution(&session.id, &format!("SET {key}:untouched yes\nFLUSHDB"))
            .is_err()
    );
    let missing = run(&studio, &session, &format!("GET {key}:untouched")).await;
    assert_eq!(rows(&studio, &missing)[0][1], None);
    let plan = studio
        .prepare_execution(&session.id, &format!("UNLINK {key} {key}:hash"))
        .unwrap();
    assert_eq!(
        studio
            .execute(&plan.id, None, Arc::new(|_| true))
            .unwrap_err()
            .code,
        ErrorCode::ApprovalRequired
    );
    let plan = studio
        .prepare_execution(&session.id, &format!("UNLINK {key} {key}:hash"))
        .unwrap();
    let approval = studio.authorize(&plan.id, None, None).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    studio
        .execute(
            &plan.id,
            Some(&approval),
            Arc::new(move |e| tx.send(e).is_ok()),
        )
        .unwrap();
    while let Some(e) = rx.recv().await {
        if matches!(e, QueryEvent::Finished { .. }) {
            break;
        }
    }
    assert!(
        studio
            .execute(&plan.id, Some(&approval), Arc::new(|_| true))
            .is_err()
    );
    assert!(!session.capabilities.sql);
    assert!(!session.capabilities.transactions);
    assert!(studio.health(&session.id).await.is_ok());
    studio.disconnect(&session.id);
}

#[tokio::test]
#[ignore = "requires disposable Redis service"]
async fn key_browser_and_typed_previews_handle_paging_expiry_and_binary_keys() {
    let (_root, studio, session) = setup(SafeMode::Confirm).await;
    let prefix = format!("redis-browser:{}", new_id());
    let seed = format!(
        "SET {prefix}:string café\nPEXPIRE {prefix}:string 60000\nHSET {prefix}:hash a one b two\nRPUSH {prefix}:list zero one two\nSADD {prefix}:set first second\nZADD {prefix}:zset 1.5 first 2.25 second\nSET \"{prefix}:\\x00\\xff\" binary"
    );
    completed(&run(&studio, &session, &seed).await);
    let mut cursor = None;
    let mut keys = std::collections::HashMap::new();
    for _ in 0..100 {
        let page = studio
            .redis_scan(&session.id, &format!("{prefix}:*"), cursor.as_deref())
            .await
            .unwrap();
        for key in page.keys {
            keys.insert(key.argument.clone(), key);
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!(cursor.is_none());
    assert_eq!(keys.len(), 6);
    for key in keys.values() {
        let preview = studio
            .redis_inspect(&session.id, &key.argument, None)
            .await
            .unwrap();
        assert_eq!(preview.key.kind, key.kind);
        assert!(!preview.entries.is_empty());
    }
    let string = studio
        .redis_inspect(&session.id, &format!("{prefix}:string"), None)
        .await
        .unwrap();
    assert_eq!(string.entries[0].value.as_deref(), Some("café"));
    assert!(string.key.ttl_ms.parse::<i64>().unwrap() > 0);
    completed(
        &run(
            &studio,
            &session,
            &format!("SET {prefix}:string changed KEEPTTL"),
        )
        .await,
    );
    let changed = studio
        .redis_inspect(&session.id, &format!("{prefix}:string"), None)
        .await
        .unwrap();
    assert!(changed.key.ttl_ms.parse::<i64>().unwrap() > 0);
    assert_eq!(changed.entries[0].value.as_deref(), Some("changed"));
    completed(&run(&studio, &session, &format!("PEXPIRE {prefix}:string 0")).await);
    let missing = studio
        .redis_inspect(&session.id, &format!("{prefix}:string"), None)
        .await
        .unwrap();
    assert_eq!(missing.key.kind, "none");
    assert!(missing.entries.is_empty());
    assert_eq!(missing.key.ttl_ms, "-2");
    assert!(
        studio
            .redis_inspect(&session.id, "a\nUNLINK b", None)
            .await
            .is_err()
    );
    assert!(
        studio
            .redis_scan(&session.id, "*", Some("invented"))
            .await
            .is_err()
    );
    let cleanup = keys.keys().cloned().collect::<Vec<_>>().join(" ");
    completed(&run(&studio, &session, &format!("UNLINK {cleanup}")).await);
}

#[tokio::test]
#[ignore = "requires disposable Redis service"]
async fn read_only_modes_block_writes_and_result_cleanup_is_bounded() {
    let (_root, studio, session) = setup(SafeMode::ReadOnly).await;
    assert_eq!(
        studio
            .prepare_execution(&session.id, "GET k\nSET k v")
            .unwrap_err()
            .code,
        ErrorCode::PolicyBlocked
    );
    for _ in 0..10 {
        let events = run(&studio, &session, "PING").await;
        completed(&events);
        for result in events.iter().filter_map(|e| {
            if let QueryEvent::Result { result } = e {
                Some(result)
            } else {
                None
            }
        }) {
            studio.release_result(&result.id);
        }
        assert_eq!(studio.usage().result_count, 0);
        assert_eq!(studio.usage().retained_bytes, 0);
    }
    studio.disconnect(&session.id);
    assert_eq!(studio.usage().session_count, 0);
}

#[tokio::test]
#[ignore = "requires disposable Redis service"]
async fn list_pages_use_scoped_cursors_and_large_strings_use_bounded_previews() {
    let (_root, studio, session) = setup(SafeMode::Confirm).await;
    let key = format!("redis-pages:{}", new_id());
    for start in [0, 100, 200] {
        let values = (start..start + 100)
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        completed(&run(&studio, &session, &format!("RPUSH {key} {values}")).await);
    }
    let mut cursor = None;
    let mut values = Vec::new();
    for page in 0..3 {
        let preview = studio
            .redis_inspect(&session.id, &key, cursor.as_deref())
            .await
            .unwrap();
        assert_eq!(preview.entries.len(), 100);
        values.extend(preview.entries.into_iter().map(|v| v.value.unwrap()));
        cursor = preview.next_cursor;
        assert_eq!(cursor.is_some(), page < 2);
        if let Some(cursor) = cursor.as_deref() {
            assert!(
                studio
                    .redis_inspect(&session.id, &format!("{key}:other"), Some(cursor))
                    .await
                    .is_err()
            );
            assert!(
                studio
                    .redis_scan(&session.id, "*", Some(cursor))
                    .await
                    .is_err()
            );
        }
    }
    assert_eq!(values, (0..300).map(|v| v.to_string()).collect::<Vec<_>>());
    completed(
        &run(
            &studio,
            &session,
            &format!("SET {key}:wide {}", "x".repeat(70_000)),
        )
        .await,
    );
    let preview = studio
        .redis_inspect(&session.id, &format!("{key}:wide"), None)
        .await
        .unwrap();
    assert!(preview.truncated);
    assert_eq!(preview.entries[0].value.as_ref().unwrap().len(), 65_536);
    assert!(preview.next_cursor.is_none());
    completed(&run(&studio, &session, &format!("UNLINK {key} {key}:wide")).await);
}

#[tokio::test]
#[ignore = "requires disposable Redis service"]
async fn authenticated_commands_and_metadata_cross_the_shared_ssh_bastion() {
    use russh::keys::{Algorithm, PrivateKey};
    let identity = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let bastion = common::bastion(56379, &identity, None, 0).await;
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let mut p = profile();
    p.host = "database.fixture.test".into();
    p.ssh = Some(SshConfig {
        host: "127.0.0.1".into(),
        port: bastion.port,
        username: "fixture".into(),
        authentication: SshAuthentication::Password,
        private_key_path: None,
        agent_socket_path: None,
    });
    let p = studio.save_profile(p, None).unwrap();
    let key = studio.probe_ssh(&p.id).await.unwrap();
    assert_eq!(
        key.fingerprint,
        bastion
            .key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string()
    );
    studio
        .trust_ssh_key(key.challenge_id.as_deref().unwrap(), &key.fingerprint)
        .unwrap();
    let session = studio
        .connect(
            &p.id,
            ConnectionSecrets {
                password: Some("joinery-redis-local-only".into()),
                ssh_password: Some("ssh-test-only".into()),
                key_passphrase: None,
            },
        )
        .await
        .unwrap();
    let events = run(&studio, &session, "PING").await;
    completed(&events);
    assert_eq!(rows(&studio, &events)[0][1].as_deref(), Some("PONG"));
    studio
        .redis_scan(&session.id, "joinery-no-match:*", None)
        .await
        .unwrap();
    studio.health(&session.id).await.unwrap();
    assert!(
        bastion
            .connections
            .load(std::sync::atomic::Ordering::SeqCst)
            > 0
    );
    studio.disconnect(&session.id);
    assert_eq!(studio.usage().session_count, 0);
}

#[tokio::test]
#[ignore = "requires disposable Redis service"]
async fn full_viewer_budgets_do_not_misreport_acknowledged_writes() {
    let (_root, studio, session) = setup(SafeMode::Confirm).await;
    for _ in 0..100 {
        completed(&run(&studio, &session, "PING").await);
    }
    assert_eq!(studio.usage().result_count, 100);
    let key = format!("redis-budget:{}", new_id());
    let events = run(&studio, &session, &format!("SET {key} committed")).await;
    completed(&events);
    assert!(events.iter().any(
        |e| matches!(e,QueryEvent::Notice{message} if message.contains("Close a result tab"))
    ));
    let preview = studio.redis_inspect(&session.id, &key, None).await.unwrap();
    assert_eq!(preview.entries[0].value.as_deref(), Some("committed"));
    completed(&run(&studio, &session, &format!("UNLINK {key}")).await);
    studio.disconnect(&session.id);
    assert_eq!(studio.usage().result_count, 0);
    assert_eq!(studio.usage().retained_bytes, 0);
}
