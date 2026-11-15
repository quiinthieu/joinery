use super::*;
use crate::drivers::snowflake_table_qualification::{
    consumption::{ConsumptionStore, TERMINAL_FILE},
    journal::FILE_NAME as JOURNAL_FILE,
};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::PathBuf,
};

const EXECUTION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const RUN_KEY: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Fixture {
    _directory: tempfile::TempDir,
    base: PathBuf,
    output: PathBuf,
    markers: PathBuf,
    plan: Plan,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().canonicalize().unwrap();
        let output = base.join("output");
        let markers = base.join("permanent-markers");
        for path in [&base, &output, &markers] {
            if path != &base {
                fs::create_dir(path).unwrap();
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self {
            _directory: directory,
            base,
            output,
            markers,
            plan: Plan::generate().unwrap(),
        }
    }

    fn reserve(&self) -> AppResult<AttemptStorage> {
        AttemptStorage::reserve(&self.output, &self.plan, EXECUTION)
    }

    fn attempt(&self) -> PathBuf {
        self.output.join(format!("attempt-{}", self.plan.run()))
    }

    fn consume(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let store = ConsumptionStore::open(&self.markers).unwrap();
        store.require_separate_from(&self.output).unwrap();
        let _consumed = store.consume(EXECUTION, RUN_KEY).unwrap();
        let mut files: Vec<_> = fs::read_dir(&self.markers)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }

    fn assert_consumption_unchanged(&self, files: &[(PathBuf, Vec<u8>)]) {
        assert_eq!(fs::read_dir(&self.markers).unwrap().count(), 2);
        for (path, bytes) in files {
            assert_eq!(fs::read(path).unwrap(), *bytes);
        }
        assert!(
            ConsumptionStore::open(&self.markers)
                .unwrap()
                .consume(EXECUTION, RUN_KEY)
                .is_err()
        );
    }
}

#[test]
fn attempt_reserves_private_persistent_paths_and_transfers_journal_once() {
    let fixture = Fixture::new();
    let mut storage = fixture.reserve().unwrap();
    assert_eq!(storage.path(), fixture.attempt());
    assert_eq!(storage.workspace(), fixture.attempt().join("workspace"));
    assert_eq!(storage.state(), fixture.attempt().join("helper-state"));
    for path in [storage.path(), storage.workspace(), storage.state()] {
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o7777, 0o700);
    }
    for name in [JOURNAL_FILE, TERMINAL_FILE] {
        let metadata = fs::metadata(storage.path().join(name)).unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.nlink(), 1);
    }
    let header = fs::read(storage.path().join(JOURNAL_FILE)).unwrap();
    assert!(!header.is_empty());
    assert!(
        fs::read(storage.path().join(TERMINAL_FILE))
            .unwrap()
            .is_empty()
    );
    let mut journal = storage.take_journal().unwrap();
    journal
        .validate_reserved(&fixture.plan.sha256().unwrap())
        .unwrap();
    assert!(storage.take_journal().is_err());
    journal
        .append(&json!({"type":"synthetic-owner-event"}))
        .unwrap();
    storage.validate().unwrap();
    drop(journal);
    drop(storage);
    assert!(fixture.attempt().join(JOURNAL_FILE).exists());
    assert!(fixture.attempt().join(TERMINAL_FILE).exists());
    assert!(fixture.attempt().join("workspace").exists());
    assert!(fixture.attempt().join("helper-state").exists());
    assert!(fixture.reserve().is_err());
}

#[test]
fn attempt_collision_preserves_existing_terminal_journal_and_sentinel() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.attempt()).unwrap();
    fs::set_permissions(fixture.attempt(), fs::Permissions::from_mode(0o700)).unwrap();
    for name in [JOURNAL_FILE, TERMINAL_FILE, "sentinel"] {
        fs::write(fixture.attempt().join(name), name.as_bytes()).unwrap();
    }
    assert!(fixture.reserve().is_err());
    for name in [JOURNAL_FILE, TERMINAL_FILE, "sentinel"] {
        assert_eq!(
            fs::read(fixture.attempt().join(name)).unwrap(),
            name.as_bytes()
        );
    }
    assert_eq!(fs::read_dir(fixture.attempt()).unwrap().count(), 3);

    let fixture = Fixture::new();
    let storage = fixture.reserve().unwrap();
    let header = fs::read(storage.path().join(JOURNAL_FILE)).unwrap();
    assert!(Journal::create(storage.path(), &fixture.plan.sha256().unwrap()).is_err());
    assert!(storage.directory.reserve_terminal().is_err());
    assert_eq!(fs::read(storage.path().join(JOURNAL_FILE)).unwrap(), header);
    assert!(
        fs::read(storage.path().join(TERMINAL_FILE))
            .unwrap()
            .is_empty()
    );
    storage.validate().unwrap();
}

#[test]
fn attempt_failed_journal_transfer_cannot_be_retried_after_path_repair() {
    let fixture = Fixture::new();
    let mut storage = fixture.reserve().unwrap();
    let header = fs::read(storage.path().join(JOURNAL_FILE)).unwrap();
    fs::set_permissions(storage.workspace(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(storage.take_journal().is_err());
    fs::set_permissions(storage.workspace(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(storage.take_journal().is_err());
    assert_eq!(fs::read(storage.path().join(JOURNAL_FILE)).unwrap(), header);
}

#[test]
fn attempt_rejects_invalid_execution_and_unsafe_physical_paths_before_reservation() {
    let fixture = Fixture::new();
    for bad in [
        "".to_owned(),
        "A".repeat(64),
        "a".repeat(63),
        "g".repeat(64),
    ] {
        assert!(AttemptStorage::reserve(&fixture.output, &fixture.plan, &bad).is_err());
    }
    let link = fixture.base.join("output-link");
    symlink(&fixture.output, &link).unwrap();
    for path in [
        link,
        PathBuf::from("relative"),
        fixture.base.join("missing"),
        fixture.output.join("."),
        fixture.output.join(".."),
        PathBuf::from(format!("{}/", fixture.output.display())),
        PathBuf::from(format!("{}//output", fixture.base.display())),
    ] {
        assert!(
            AttemptStorage::reserve(&path, &fixture.plan, EXECUTION).is_err(),
            "{path:?}"
        );
    }
    for mode in [0o755, 0o777, 0o1700] {
        fs::set_permissions(&fixture.output, fs::Permissions::from_mode(mode)).unwrap();
        assert!(fixture.reserve().is_err(), "{mode:o}");
    }
    fs::set_permissions(&fixture.output, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&fixture.base, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(fixture.reserve().is_err());
    fs::set_permissions(&fixture.base, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(fs::read_dir(&fixture.output).unwrap().count(), 0);
}

#[test]
fn attempt_terminal_binds_all_identity_fields_and_is_one_shot() {
    let fixture = Fixture::new();
    let files = fixture.consume();
    let mut storage = fixture.reserve().unwrap();
    let result = json!({"ack":{"original":"preserved"}, "cleanup":{"settled":false}});
    let receipt = storage.persist_terminal(&result).unwrap();
    let path = storage.path().join(TERMINAL_FILE);
    let bytes = fs::read(&path).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert_eq!(value["format"], FORMAT);
    assert_eq!(value["planSha256"], fixture.plan.sha256().unwrap());
    assert_eq!(value["executionSha256"], EXECUTION);
    assert_eq!(value["run"], fixture.plan.run());
    assert_eq!(value["result"], result);
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert_eq!(receipt.sha256, format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(receipt.bytes, bytes.len() as u64);
    assert_eq!(
        serde_json::to_value(&receipt).unwrap()["bytes"],
        receipt.bytes
    );
    assert!(
        storage
            .persist_terminal(&json!({"replacement":true}))
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    storage.validate().unwrap();
    drop(storage);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    fixture.assert_consumption_unchanged(&files);
}

#[test]
fn attempt_failed_terminal_write_is_latched_and_partial_outputs_are_preserved() {
    for fault in [Fault::PartialWrite, Fault::FileSync, Fault::DirectorySync] {
        let fixture = Fixture::new();
        let files = fixture.consume();
        let mut storage = fixture.reserve().unwrap();
        storage.fault = Some(fault);
        assert!(
            storage
                .persist_terminal(&json!({"result":"original"}))
                .is_err()
        );
        let path = storage.path().join(TERMINAL_FILE);
        let partial = fs::read(&path).unwrap();
        assert!(!partial.is_empty());
        assert!(storage.validate().is_err());
        assert!(storage.persist_terminal(&json!({"retry":true})).is_err());
        assert_eq!(fs::read(&path).unwrap(), partial);
        drop(storage);
        assert_eq!(fs::read(&path).unwrap(), partial);
        assert!(fixture.reserve().is_err());
        fixture.assert_consumption_unchanged(&files);
    }
}

#[test]
fn attempt_terminal_encoding_limits_fail_before_write_and_cannot_retry() {
    for value in [
        Value::String("x".repeat(MAX_TERMINAL_BYTES)),
        Value::Array(vec![Value::Null; MAX_RESULT_NODES + 1]),
    ] {
        let fixture = Fixture::new();
        let mut storage = fixture.reserve().unwrap();
        assert!(storage.persist_terminal(&value).is_err());
        assert!(
            fs::read(storage.path().join(TERMINAL_FILE))
                .unwrap()
                .is_empty()
        );
        assert!(storage.persist_terminal(&Value::Null).is_err());
    }
    let fixture = Fixture::new();
    let mut storage = fixture.reserve().unwrap();
    let mut deep = Value::Null;
    for _ in 0..MAX_RESULT_DEPTH + 1 {
        deep = Value::Array(vec![deep]);
    }
    assert!(storage.persist_terminal(&deep).is_err());
    assert!(
        fs::read(storage.path().join(TERMINAL_FILE))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn attempt_rejects_directory_or_terminal_replacement_modes_links_and_byte_changes() {
    for mutation in [
        "parent",
        "attempt",
        "workspace",
        "state",
        "ancestor-mode",
        "workspace-mode",
        "terminal-replace",
        "terminal-symlink",
        "terminal-mode",
        "terminal-hardlink",
        "terminal-bytes",
        "journal-truncate",
    ] {
        let fixture = Fixture::new();
        let mut storage = fixture.reserve().unwrap();
        let path = storage.path().join(TERMINAL_FILE);
        match mutation {
            "parent" | "attempt" | "workspace" | "state" => {
                let target = match mutation {
                    "parent" => fixture.output.clone(),
                    "attempt" => storage.path().to_owned(),
                    "workspace" => storage.workspace().to_owned(),
                    "state" => storage.state().to_owned(),
                    _ => unreachable!(),
                };
                fs::rename(&target, target.with_extension("original")).unwrap();
                fs::create_dir(&target).unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
            }
            "ancestor-mode" => {
                fs::set_permissions(&fixture.base, fs::Permissions::from_mode(0o777)).unwrap()
            }
            "workspace-mode" => {
                fs::set_permissions(storage.workspace(), fs::Permissions::from_mode(0o755)).unwrap()
            }
            "terminal-replace" | "terminal-symlink" => {
                let original = path.with_extension("original");
                fs::rename(&path, &original).unwrap();
                if mutation == "terminal-symlink" {
                    symlink(original, &path).unwrap();
                } else {
                    fs::write(&path, b"").unwrap();
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                }
            }
            "terminal-mode" => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap()
            }
            "terminal-hardlink" => fs::hard_link(&path, path.with_extension("hardlink")).unwrap(),
            "terminal-bytes" => fs::write(&path, b"unexpected bytes").unwrap(),
            "journal-truncate" => fs::write(storage.path().join(JOURNAL_FILE), b"").unwrap(),
            _ => unreachable!(),
        }
        assert!(storage.validate().is_err(), "{mutation}");
        assert!(
            storage.persist_terminal(&Value::Null).is_err(),
            "{mutation}"
        );
        assert!(storage.take_journal().is_err(), "{mutation}");
        if mutation == "ancestor-mode" {
            fs::set_permissions(&fixture.base, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
}

#[test]
fn attempt_detects_same_length_terminal_rewrite_after_success() {
    let fixture = Fixture::new();
    let mut storage = fixture.reserve().unwrap();
    storage
        .persist_terminal(&json!({"state":"before"}))
        .unwrap();
    let path = storage.path().join(TERMINAL_FILE);
    let original = fs::read(&path).unwrap();
    let changed = String::from_utf8(original.clone())
        .unwrap()
        .replace("before", "forged");
    assert_eq!(changed.len(), original.len());
    fs::write(&path, changed.as_bytes()).unwrap();
    assert!(storage.validate().is_err());
    assert!(storage.persist_terminal(&Value::Null).is_err());
    assert_eq!(fs::read(path).unwrap(), changed.as_bytes());
}
