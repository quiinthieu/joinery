use joinery_core::{domain::*, persistence::Persistence};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const CHILD_ROOT: &str = "JOINERY_RECOVERY_CHILD_ROOT";

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Invoked only by the parent test, with a unique disposable data directory.
#[test]
#[ignore = "subprocess helper for the recovery test; not a standalone check"]
fn recovery_writer() {
    let root = std::env::var_os(CHILD_ROOT).expect("parent must supply disposable root");
    let root = Path::new(&root);
    let db = Persistence::open(root).unwrap();
    db.save_group(&ConnectionGroup {
        id: "local".into(),
        name: "Renamed local group".into(),
        parent_id: None,
        environment: Environment::Local,
    })
    .unwrap();
    db.set_setting("recovery-fixture", "committed").unwrap();
    db.record_history(&HistoryEntry {
        id: "committed-run".into(),
        connection_id: "fixture".into(),
        connection_name: "Disposable recovery fixture".into(),
        started_at: now_ms(),
        sql: None,
        statement_count: 1,
        status: RunStatus::Completed,
        elapsed_ms: 1,
    })
    .unwrap();
    db.audit("fixture", "execute", "synthetic-digest", "Completed")
        .unwrap();

    // Keep both the committed WAL and an uncommitted transaction open when the
    // parent kills this process. No Rust/SQLite destructors can run on that path.
    let pending = rusqlite::Connection::open(root.join("joinery.sqlite")).unwrap();
    pending
        .execute_batch("BEGIN IMMEDIATE; UPDATE settings SET value='uncommitted' WHERE key='recovery-fixture'; INSERT INTO settings VALUES('pending-only','uncommitted');")
        .unwrap();
    std::fs::write(root.join("writer-ready"), b"ready").unwrap();
    loop {
        std::thread::park();
    }
}

fn assert_recovered(root: &Path) {
    let db = Persistence::open(root).unwrap();
    assert_eq!(
        db.setting("recovery-fixture").unwrap().as_deref(),
        Some("committed")
    );
    assert_eq!(db.setting("pending-only").unwrap(), None);
    assert_eq!(db.groups().unwrap().len(), 3);
    assert_eq!(
        db.groups()
            .unwrap()
            .iter()
            .find(|g| g.id == "local")
            .unwrap()
            .name,
        "Renamed local group"
    );
    let history = db.history("").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, "committed-run");
    assert_eq!(history[0].status, RunStatus::Completed);
    assert!(history[0].sql.is_none());
    assert_eq!(db.audit_log().unwrap().len(), 1);
    let conn = rusqlite::Connection::open(root.join("joinery.sqlite")).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[test]
fn killed_writer_recovers_committed_metadata_and_rolls_back_pending_write() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("crash-fixture");
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "recovery_writer", "--ignored", "--nocapture"])
            .env(CHILD_ROOT, &root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join("writer-ready").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "writer exited before crash point"
        );
        assert!(
            Instant::now() < deadline,
            "writer did not reach crash point"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(root.join("joinery.sqlite-wal").metadata().unwrap().len() > 0);
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    assert_recovered(&root);

    // A closed/checkpointed metadata file can be restored into a fresh root.
    // This exercises store recovery, not an unimplemented backup/restore UI.
    let restored = dir.path().join("restored-fixture");
    std::fs::create_dir(&restored).unwrap();
    std::fs::copy(root.join("joinery.sqlite"), restored.join("joinery.sqlite")).unwrap();
    assert_recovered(&restored);
}
