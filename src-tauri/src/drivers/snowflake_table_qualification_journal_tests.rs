use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

const PLAN_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

fn contents(directory: &Path) -> Vec<u8> {
    fs::read(directory.join(FILE_NAME)).unwrap()
}

fn assert_poisoned(journal: &mut Journal, first: &AppError) {
    let repeated = journal.append(&json!({"type": "later"})).unwrap_err();
    assert_eq!(repeated.code, first.code);
    assert_eq!(repeated.message, first.message);
}

#[test]
fn journal_header_is_exclusive_private_and_binds_plan_before_first_append() {
    let directory = directory();
    let journal = Journal::create(directory.path(), PLAN_HASH).unwrap();
    assert_eq!(journal.sequence, 0);
    assert_eq!(journal.records, 1);
    let file = directory.path().join(FILE_NAME);
    let metadata = fs::symlink_metadata(&file).unwrap();
    assert_eq!(metadata.mode() & 0o7777, 0o600);
    assert_eq!(metadata.uid(), owner());
    assert_eq!(metadata.nlink(), 1);
    let before = fs::read(&file).unwrap();
    let header: Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(header["sequence"], 0);
    assert_eq!(header["previous"], ZERO_HASH);
    assert_eq!(header["event"]["planSha256"], PLAN_HASH);
    assert_eq!(header["event"]["format"], FORMAT);
    assert_eq!(header["event"]["type"], "header");
    let payload = format!(
        r#"{{"event":{{"format":"{FORMAT}","planSha256":"{PLAN_HASH}","type":"header"}},"previous":"{ZERO_HASH}","sequence":0}}"#
    );
    assert_eq!(header["sha256"], digest(payload));
    assert!(Journal::create(directory.path(), PLAN_HASH).is_err());
    drop(journal);
    assert!(Journal::create(directory.path(), PLAN_HASH).is_err());
    assert_eq!(fs::read(&file).unwrap(), before);
}

#[test]
fn journal_rejects_bad_bindings_and_unsafe_directories_without_chmod_or_side_effects() {
    for hash in [
        "",
        "short",
        &"A".repeat(64),
        &"g".repeat(64),
        &format!("{PLAN_HASH}\n"),
    ] {
        let directory = directory();
        assert!(Journal::create(directory.path(), hash).is_err());
        assert!(!directory.path().join(FILE_NAME).exists());
    }
    assert!(Journal::create(Path::new("."), PLAN_HASH).is_err());
    let root = directory();
    let private = root.path().join("private");
    fs::create_dir(&private).unwrap();
    for mode in [0o755, 0o770, 0o777, 0o1700, 0o500] {
        fs::set_permissions(&private, fs::Permissions::from_mode(mode)).unwrap();
        assert!(Journal::create(&private, PLAN_HASH).is_err());
        assert_eq!(fs::metadata(&private).unwrap().mode() & 0o7777, mode);
        assert!(!private.join(FILE_NAME).exists());
    }
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let link = root.path().join("directory-link");
    symlink(&private, &link).unwrap();
    assert!(Journal::create(&link, PLAN_HASH).is_err());
    assert!(!private.join(FILE_NAME).exists());
    let ordinary_file = root.path().join("ordinary-file");
    fs::write(&ordinary_file, b"sentinel").unwrap();
    assert!(Journal::create(&ordinary_file, PLAN_HASH).is_err());
    assert_eq!(fs::read(ordinary_file).unwrap(), b"sentinel");
}

#[test]
fn journal_never_overwrites_preexisting_regular_symlink_or_directory_entries() {
    for kind in 0..3 {
        let directory = directory();
        let path = directory.path().join(FILE_NAME);
        let sentinel = directory.path().join("sentinel");
        fs::write(&sentinel, b"do not modify").unwrap();
        match kind {
            0 => fs::write(&path, b"existing journal").unwrap(),
            1 => symlink(&sentinel, &path).unwrap(),
            _ => fs::create_dir(&path).unwrap(),
        }
        assert!(Journal::create(directory.path(), PLAN_HASH).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"do not modify");
        if kind == 0 {
            assert_eq!(fs::read(path).unwrap(), b"existing journal");
        }
    }
}

#[test]
fn journal_receipts_follow_independent_canonical_payload_hashes_and_escape_jsonl_strings() {
    let directory = directory();
    let mut journal = Journal::create(directory.path(), PLAN_HASH).unwrap();
    let header_hash = journal.previous.clone();
    let first = journal
        .append(&json!({"z": "line\ntext", "a": {"q": 2, "b": true}}))
        .unwrap();
    let event = r#"{"a":{"b":true,"q":2},"z":"line\ntext"}"#;
    let payload = format!(r#"{{"event":{event},"previous":"{header_hash}","sequence":1}}"#);
    assert_eq!(first.sequence(), 1);
    assert_eq!(first.sha256(), digest(&payload));
    let first_hash = first.sha256().to_owned();
    // Event-owned names cannot replace the journal envelope or mint a receipt.
    let second = journal
        .append(&json!({"sequence": 999, "sha256": "forged", "previous": "forged", "synced": true}))
        .unwrap();
    assert_eq!(second.sequence(), 2);
    assert_ne!(second.sha256(), "forged");
    let bytes = contents(directory.path());
    let lines: Vec<_> = std::str::from_utf8(&bytes).unwrap().lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[1],
        format!(
            r#"{{"event":{event},"previous":"{header_hash}","sequence":1,"sha256":"{first_hash}"}}"#
        )
    );
    let recorded: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(recorded["sequence"], 2);
    assert_eq!(recorded["previous"], first.sha256());
    assert_eq!(recorded["sha256"], second.sha256());
    assert_eq!(recorded["event"]["sequence"], 999);
    assert_eq!(journal.bytes, bytes.len() as u64);
}

#[test]
fn journal_recursively_sorts_objects_without_changing_arrays_or_exact_numbers() {
    let first: Value =
        serde_json::from_str(r#"{"z":[{"y":1,"a":2},3],"a":9007199254740993}"#).unwrap();
    let second: Value =
        serde_json::from_str(r#"{"a":9007199254740993,"z":[{"a":2,"y":1},3]}"#).unwrap();
    let (first_bytes, first_hash) = encode_record(&first, 1, ZERO_HASH, MAX_RECORD_BYTES).unwrap();
    let (second_bytes, second_hash) =
        encode_record(&second, 1, ZERO_HASH, MAX_RECORD_BYTES).unwrap();
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first_hash, second_hash);
    assert!(
        std::str::from_utf8(&first_bytes)
            .unwrap()
            .contains("9007199254740993")
    );
    let changed = json!({"a": 9007199254740993_u64, "z": [3, {"a": 2, "y": 1}]});
    assert_ne!(
        encode_record(&changed, 1, ZERO_HASH, MAX_RECORD_BYTES)
            .unwrap()
            .1,
        first_hash
    );
}

#[test]
fn journal_invalid_events_byte_depth_and_node_limits_poison_before_any_write() {
    let mut deep = Value::Null;
    for _ in 0..MAX_DEPTH {
        deep = json!({"nested": deep});
    }
    let cases = [
        Value::Null,
        json!([]),
        json!({}),
        json!({"large": "X".repeat(MAX_RECORD_BYTES)}),
        deep,
        json!({"nodes": vec![Value::Null; MAX_NODES]}),
    ];
    for event in cases {
        let directory = directory();
        let mut journal = Journal::create(directory.path(), PLAN_HASH).unwrap();
        let before = contents(directory.path());
        let error = journal.append(&event).unwrap_err();
        assert_poisoned(&mut journal, &error);
        assert_eq!(contents(directory.path()), before);
        assert_eq!(journal.sequence, 0);
        assert_eq!(journal.records, 1);
    }
    let mut output = BoundedBytes {
        bytes: Vec::new(),
        limit: 8,
    };
    assert!(output.write_all(&[0; 9]).is_err());
    assert_eq!(output.bytes.capacity(), 0); // Rejected before reserving storage.
    output.write_all(&[0; 8]).unwrap();
    assert_eq!(output.bytes.len(), 8);
    assert!(output.bytes.capacity() <= 8);
}

#[test]
fn journal_total_record_and_record_count_boundaries_are_inclusive_and_never_retry() {
    assert_eq!(MAX_RECORD_BYTES, 4 * 1024 * 1024);
    assert_eq!(MAX_TOTAL_BYTES, 128 * 1024 * 1024);
    assert_eq!(MAX_RECORDS, 65_536);
    let event = json!({"type": "bounded"});
    for limit in ["bytes", "records", "record size"] {
        let directory = directory();
        let mut journal = Journal::create(directory.path(), PLAN_HASH).unwrap();
        let line_size = encode_record(&event, 1, &journal.previous, MAX_RECORD_BYTES)
            .unwrap()
            .0
            .len();
        match limit {
            "bytes" => journal.limits.total_bytes = journal.bytes + line_size as u64,
            "records" => journal.limits.records = 2,
            _ => journal.limits.record_bytes = line_size,
        }
        journal.append(&event).unwrap();
        let before = contents(directory.path());
        let next_event = if limit == "record size" {
            json!({"type": "too large for the exact boundary"})
        } else {
            event.clone()
        };
        let error = journal.append(&next_event).unwrap_err();
        assert_poisoned(&mut journal, &error);
        assert_eq!(contents(directory.path()), before);
        assert_eq!(journal.sequence, 1);
        assert_eq!(journal.records, 2);
    }
}

#[test]
fn journal_partial_write_and_sync_failures_return_no_receipt_and_latch_first_failure() {
    for fault in [Fault::PartialWrite(0), Fault::PartialWrite(7), Fault::Sync] {
        let directory = directory();
        let mut journal = Journal::create(directory.path(), PLAN_HASH).unwrap();
        let before = contents(directory.path());
        let hash_before = journal.previous.clone();
        journal.fault = Some(fault);
        let error = journal.append(&json!({"type": "attempt"})).unwrap_err();
        let failed = contents(directory.path());
        assert!(failed.starts_with(&before));
        assert_poisoned(&mut journal, &error);
        assert_eq!(contents(directory.path()), failed);
        assert_eq!(journal.previous, hash_before);
        assert_eq!(journal.sequence, 0);
        assert_eq!(journal.records, 1);
        assert_eq!(journal.bytes, before.len() as u64);
        drop(journal);
        assert!(Journal::create(directory.path(), PLAN_HASH).is_err());
        assert_eq!(contents(directory.path()), failed);
    }
}

#[test]
fn journal_replaced_file_is_not_followed_and_original_descriptor_is_not_written_again() {
    for replacement_symlink in [false, true] {
        let directory = directory();
        let mut journal = Journal::create(directory.path(), PLAN_HASH).unwrap();
        let path = directory.path().join(FILE_NAME);
        let saved = directory.path().join("saved-journal");
        fs::rename(&path, &saved).unwrap();
        let before = fs::read(&saved).unwrap();
        let replacement = directory.path().join("replacement");
        fs::write(&replacement, b"private sentinel").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
        if replacement_symlink {
            symlink(&replacement, &path).unwrap();
        } else {
            fs::rename(&replacement, &path).unwrap();
        }
        let error = journal.append(&json!({"type": "denied"})).unwrap_err();
        assert_eq!(fs::read(&saved).unwrap(), before);
        let replacement_path = if replacement_symlink {
            &replacement
        } else {
            &path
        };
        assert_eq!(fs::read(replacement_path).unwrap(), b"private sentinel");
        // Repairing the pathname cannot clear the permanent failure latch.
        fs::remove_file(&path).unwrap();
        fs::rename(&saved, &path).unwrap();
        assert_poisoned(&mut journal, &error);
        assert_eq!(contents(directory.path()), before);
    }
}

#[test]
fn journal_directory_replacement_and_permission_changes_fail_without_chmod() {
    let root = directory();
    let path = root.path().join("private");
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let mut journal = Journal::create(&path, PLAN_HASH).unwrap();
    let before = contents(&path);
    let saved = root.path().join("renamed");
    fs::rename(&path, &saved).unwrap();
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let error = journal.append(&json!({"type": "denied"})).unwrap_err();
    assert_poisoned(&mut journal, &error);
    assert_eq!(contents(&saved), before);
    assert!(!path.join(FILE_NAME).exists());
    for directory_mode in [false, true] {
        let directory = directory();
        let mut journal = Journal::create(directory.path(), PLAN_HASH).unwrap();
        let before = contents(directory.path());
        let changed = if directory_mode {
            directory.path().to_owned()
        } else {
            directory.path().join(FILE_NAME)
        };
        let mode = if directory_mode { 0o750 } else { 0o640 };
        fs::set_permissions(&changed, fs::Permissions::from_mode(mode)).unwrap();
        let error = journal.append(&json!({"type": "denied"})).unwrap_err();
        assert_poisoned(&mut journal, &error);
        assert_eq!(fs::metadata(&changed).unwrap().mode() & 0o7777, mode);
        assert_eq!(contents(directory.path()), before);
    }
}

#[test]
fn journal_external_truncation_append_and_hardlink_are_detected_before_more_bytes() {
    for tamper in ["truncate", "append", "hardlink"] {
        let directory = directory();
        let mut journal = Journal::create(directory.path(), PLAN_HASH).unwrap();
        let path = directory.path().join(FILE_NAME);
        match tamper {
            "truncate" => OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(1)
                .unwrap(),
            "append" => OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(b"tamper")
                .unwrap(),
            _ => fs::hard_link(&path, directory.path().join("alias")).unwrap(),
        }
        let tampered = contents(directory.path());
        let error = journal.append(&json!({"type": "denied"})).unwrap_err();
        assert_poisoned(&mut journal, &error);
        assert_eq!(contents(directory.path()), tampered);
    }
}
