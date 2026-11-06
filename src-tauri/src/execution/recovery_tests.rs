use super::*;
use crate::secrets::MemorySecrets;

#[test]
fn restore_requires_confirmation_current_passphrase_and_an_idle_studio() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let snapshot = studio.create_metadata_backup().unwrap();
    assert!(
        studio
            .restore_metadata_backup(&snapshot.id, &snapshot.digest, "", None)
            .is_err()
    );
    studio
        .set_passphrase("fixture-passphrase-123", None)
        .unwrap();
    assert_eq!(
        studio
            .restore_metadata_backup(&snapshot.id, &snapshot.digest, "RESTORE", None)
            .unwrap_err()
            .code,
        ErrorCode::PassphraseRequired
    );
    assert_eq!(
        studio
            .restore_metadata_backup(&snapshot.id, &snapshot.digest, "RESTORE", Some("incorrect"))
            .unwrap_err()
            .code,
        ErrorCode::InvalidPassphrase
    );
    let connecting = studio.connect_gate.try_lock().unwrap();
    assert!(
        studio
            .restore_metadata_backup(
                &snapshot.id,
                &snapshot.digest,
                "RESTORE",
                Some("fixture-passphrase-123")
            )
            .is_err()
    );
    drop(connecting);
    let result = studio.results.create("fixture", 0, vec![], None).unwrap();
    assert!(
        studio
            .restore_metadata_backup(
                &snapshot.id,
                &snapshot.digest,
                "RESTORE",
                Some("fixture-passphrase-123")
            )
            .is_err()
    );
    studio.results.release(&result.id);
    studio
        .restore_metadata_backup(
            &snapshot.id,
            &snapshot.digest,
            "RESTORE",
            Some("fixture-passphrase-123"),
        )
        .unwrap();
    assert!(studio.bootstrap().unwrap().passphrase_configured);
    assert!(
        studio
            .verify_passphrase(Some("fixture-passphrase-123"))
            .is_ok()
    );
    assert_eq!(studio.usage().session_count, 0);
}

use crate::drivers::BrowseRows;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

struct MockScopedDriver {
    #[allow(dead_code)]
    engine: EngineKind,
    active_handle: Mutex<Option<String>>,
    cancel_count: Arc<AtomicUsize>,
    cancel_token: CancellationToken,
}

#[async_trait]
impl Driver for MockScopedDriver {
    async fn execute(
        &self,
        _sql: &str,
        _kind: StatementKind,
        _sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        Ok(1)
    }

    async fn cancel(&self) -> AppResult<()> {
        self.cancel_count.fetch_add(1, Ordering::SeqCst);
        self.cancel_token.cancel();
        Ok(())
    }

    fn active_query_handle(&self) -> Option<String> {
        self.active_handle.lock().clone()
    }

    async fn health(&self) -> AppResult<u32> {
        Ok(0)
    }

    fn is_closed(&self) -> bool {
        false
    }

    fn close(&self) {}

    async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
        Ok(vec![])
    }

    async fn schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        Ok(TableSchema {
            table: table.clone(),
            columns: vec![],
            primary_key: vec![],
            foreign_keys: vec![],
            inbound_foreign_keys: vec![],
            editable: false,
        })
    }

    async fn browse(
        &self,
        _schema: &TableSchema,
        _after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        Ok(BrowseRows {
            rows: vec![],
            next_key: None,
            has_more: false,
            truncated: false,
        })
    }

    async fn apply(&self, _schema: &TableSchema, _rows: &[RowMutation]) -> AppResult<()> {
        Ok(())
    }

    async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
        Ok(vec![])
    }

    async fn cancel_process(&self, _id: &str, _identity: &str) -> AppResult<()> {
        Ok(())
    }
}

fn create_test_session(
    session_id: &str,
    engine: EngineKind,
    active_handle: Option<String>,
) -> (
    Arc<Session>,
    Arc<AtomicUsize>,
    CancellationToken,
    Arc<MockScopedDriver>,
) {
    let cancel_count = Arc::new(AtomicUsize::new(0));
    let cancel_token = CancellationToken::new();
    let driver = Arc::new(MockScopedDriver {
        engine,
        active_handle: Mutex::new(active_handle),
        cancel_count: Arc::clone(&cancel_count),
        cancel_token: cancel_token.clone(),
    });

    let profile = ConnectionProfile {
        id: format!("prof-{session_id}"),
        name: format!("Profile {session_id}"),
        group_id: "local".into(),
        engine,
        host: "localhost".into(),
        port: 5432,
        database: "db".into(),
        username: "user".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::Silent,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: true,
        store_drafts: true,
        snowflake: None,
    };

    let session = Arc::new(Session {
        info: SessionInfo {
            id: session_id.to_string(),
            connection_id: profile.id.clone(),
            engine,
            server_version: "test".into(),
            latency_ms: 0,
            encrypted: false,
            capabilities: EngineCapabilities {
                sql: true,
                actual_plan: None,
                transactions: true,
                cancellation: true,
                table_browsing: true,
                table_editing: true,
                table_filtering: true,
                processes: true,
                object_explorer: false,
                schema_diagram: false,
                schema_diff: false,
                ddl_editing: false,
                lock_inspection: false,
                explain_analyze: false,
                oauth_browser: false,
            },
        },
        profile,
        driver: driver.clone(),
        lane: tokio::sync::Mutex::new(()),
        cancellation_gate: tokio::sync::Mutex::new(()),
        active_run: Mutex::new(None),
        closed: AtomicBool::new(false),
        tunnel: None,
        completion: Mutex::new(None),
    });

    (session, cancel_count, cancel_token, driver)
}

#[tokio::test]
async fn adversarial_cancel_by_query_id_scoped_cancellation_and_zero_blast_radius() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();

    // 1. Setup multi-session environment across Snowflake, Postgres, and MySQL
    let (s1, s1_cancels, s1_token, s1_driver) = create_test_session(
        "sf-session-1",
        EngineKind::Snowflake,
        Some("sf-handle-001".into()),
    );
    let (s2, s2_cancels, s2_token, _s2_driver) = create_test_session(
        "sf-session-2",
        EngineKind::Snowflake,
        Some("sf-handle-002".into()),
    );
    let (s3, s3_cancels, s3_token, _s3_driver) =
        create_test_session("pg-session-1", EngineKind::Postgres, None);
    let (s4, s4_cancels, s4_token, _s4_driver) =
        create_test_session("my-session-1", EngineKind::Mysql, None);

    // Register active runs and jobs
    *s1.active_run.lock() = Some("run-sf-1".into());
    *s2.active_run.lock() = Some("run-sf-2".into());
    *s3.active_run.lock() = Some("run-pg-1".into());
    *s4.active_run.lock() = Some("run-my-1".into());

    {
        let mut sessions = studio.sessions.lock();
        sessions.insert("sf-session-1".into(), Arc::clone(&s1));
        sessions.insert("sf-session-2".into(), Arc::clone(&s2));
        sessions.insert("pg-session-1".into(), Arc::clone(&s3));
        sessions.insert("my-session-1".into(), Arc::clone(&s4));
    }

    {
        let mut jobs = studio.jobs.lock();
        jobs.insert(
            "run-sf-1".into(),
            Job {
                session_id: "sf-session-1".into(),
                token: s1_token.clone(),
                cancellation_sent: Arc::new(AtomicBool::new(false)),
            },
        );
        jobs.insert(
            "run-sf-2".into(),
            Job {
                session_id: "sf-session-2".into(),
                token: s2_token.clone(),
                cancellation_sent: Arc::new(AtomicBool::new(false)),
            },
        );
        jobs.insert(
            "run-pg-1".into(),
            Job {
                session_id: "pg-session-1".into(),
                token: s3_token.clone(),
                cancellation_sent: Arc::new(AtomicBool::new(false)),
            },
        );
        jobs.insert(
            "run-my-1".into(),
            Job {
                session_id: "my-session-1".into(),
                token: s4_token.clone(),
                cancellation_sent: Arc::new(AtomicBool::new(false)),
            },
        );
    }

    {
        let mut query_jobs = studio.query_jobs.lock();
        query_jobs.insert("sf-handle-001".into(), "run-sf-1".into());
        query_jobs.insert("sf-handle-002".into(), "run-sf-2".into());
    }

    // Step A: Trigger cancel with non-existent handle
    assert!(
        studio
            .cancel_by_query_id("non-existent-handle-404")
            .await
            .is_ok()
    );
    assert_eq!(s1_cancels.load(Ordering::SeqCst), 0);
    assert_eq!(s2_cancels.load(Ordering::SeqCst), 0);
    assert_eq!(s3_cancels.load(Ordering::SeqCst), 0);
    assert_eq!(s4_cancels.load(Ordering::SeqCst), 0);
    assert!(!s1_token.is_cancelled());
    assert!(!s2_token.is_cancelled());
    assert!(!s3_token.is_cancelled());
    assert!(!s4_token.is_cancelled());

    // Step B: Trigger cancel on empty string and random uuid
    assert!(studio.cancel_by_query_id("").await.is_ok());
    assert!(
        studio
            .cancel_by_query_id("00000000-0000-0000-0000-000000000000")
            .await
            .is_ok()
    );
    assert_eq!(s1_cancels.load(Ordering::SeqCst), 0);
    assert_eq!(s2_cancels.load(Ordering::SeqCst), 0);

    // Step C: Trigger cancel on sf-handle-001 -> ONLY session 1 is cancelled
    assert!(studio.cancel_by_query_id("sf-handle-001").await.is_ok());
    assert_eq!(s1_cancels.load(Ordering::SeqCst), 1);
    assert!(s1_token.is_cancelled());
    // Crucial: Sibling Snowflake session s2 is UNTOUCHED (zero token poisoning)
    assert_eq!(s2_cancels.load(Ordering::SeqCst), 0);
    assert!(!s2_token.is_cancelled());
    // Relational sessions s3 and s4 are UNTOUCHED
    assert_eq!(s3_cancels.load(Ordering::SeqCst), 0);
    assert_eq!(s4_cancels.load(Ordering::SeqCst), 0);

    // Step D: Trigger cancel on sf-handle-002 via active_query_handle fallback (removed from query_jobs)
    studio.query_jobs.lock().remove("sf-handle-002");
    assert!(studio.cancel_by_query_id("sf-handle-002").await.is_ok());
    assert_eq!(s2_cancels.load(Ordering::SeqCst), 1);
    assert!(s2_token.is_cancelled());
    assert_eq!(s3_cancels.load(Ordering::SeqCst), 0);
    assert_eq!(s4_cancels.load(Ordering::SeqCst), 0);

    // Step E: Trigger cancel on relational Postgres session via Joinery run_id
    assert!(studio.cancel_by_query_id("run-pg-1").await.is_ok());
    assert_eq!(s3_cancels.load(Ordering::SeqCst), 1);
    assert!(s3_token.is_cancelled());
    assert_eq!(s4_cancels.load(Ordering::SeqCst), 0);
    assert!(!s4_token.is_cancelled());

    // Step F: Once s1 finishes execution and cleans up active_run / query_jobs / active_handle, subsequent cancel calls are safe no-ops
    *s1.active_run.lock() = None;
    *s1_driver.active_handle.lock() = None;
    studio.jobs.lock().remove("run-sf-1");
    studio.query_jobs.lock().remove("sf-handle-001");
    assert!(studio.cancel_by_query_id("sf-handle-001").await.is_ok());
    assert_eq!(s1_cancels.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn adversarial_cancel_by_query_id_concurrent_storm() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();

    let (s_target, target_cancels, target_token, _target_driver) = create_test_session(
        "target-sf",
        EngineKind::Snowflake,
        Some("target-handle".into()),
    );
    let (s_bystander, bystander_cancels, bystander_token, _bystander_driver) =
        create_test_session("bystander-pg", EngineKind::Postgres, None);

    *s_target.active_run.lock() = Some("target-run".into());
    *s_bystander.active_run.lock() = Some("bystander-run".into());

    {
        let mut sessions = studio.sessions.lock();
        sessions.insert("target-sf".into(), Arc::clone(&s_target));
        sessions.insert("bystander-pg".into(), Arc::clone(&s_bystander));
    }
    {
        let mut jobs = studio.jobs.lock();
        jobs.insert(
            "target-run".into(),
            Job {
                session_id: "target-sf".into(),
                token: target_token.clone(),
                cancellation_sent: Arc::new(AtomicBool::new(false)),
            },
        );
        jobs.insert(
            "bystander-run".into(),
            Job {
                session_id: "bystander-pg".into(),
                token: bystander_token.clone(),
                cancellation_sent: Arc::new(AtomicBool::new(false)),
            },
        );
    }
    studio
        .query_jobs
        .lock()
        .insert("target-handle".into(), "target-run".into());

    let mut tasks = Vec::new();
    for i in 0..30 {
        let st = Arc::clone(&studio);
        tasks.push(tokio::spawn(async move {
            let handle = if i == 7 {
                "target-handle"
            } else if i % 2 == 0 {
                "non-existent-handle"
            } else {
                "arbitrary-unknown-id"
            };
            st.cancel_by_query_id(handle).await
        }));
    }

    for task in tasks {
        let res = task.await.expect("task join");
        assert!(res.is_ok());
    }

    // Target session was cancelled exactly once
    assert_eq!(target_cancels.load(Ordering::SeqCst), 1);
    assert!(target_token.is_cancelled());
    // Bystander session was NEVER cancelled (strictly 0 blast radius)
    assert_eq!(bystander_cancels.load(Ordering::SeqCst), 0);
    assert!(!bystander_token.is_cancelled());
}
