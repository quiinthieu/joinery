use super::*;

fn profile() -> ConnectionProfile {
    serde_json::from_value(serde_json::json!({
        "id":"00000000-0000-4000-8000-000000000006", "name":"History owner fixture", "groupId":"local",
        "engine":"postgres", "host":"127.0.0.1", "port":5432, "database":"fixture", "username":"fixture",
        "environment":"local", "safeMode":"readOnly", "tlsMode":"disable", "caCertPath":null,
        "ssh":null, "rememberPassword":false, "storeQueryText":true
    })).unwrap()
}
fn seed() -> HistoryEntry {
    HistoryEntry {
        id: new_id(),
        connection_id: profile().id,
        connection_name: profile().name,
        started_at: now_ms(),
        sql: Some("SELECT 'original owner text'".into()),
        statement_count: 1,
        provenance: HistoryProvenance::Admitted,
        status: None,
        elapsed_ms: None,
        execution_summary: None,
        snowflake_dml: None,
        snowflake_ddl: None,
    }
}
fn completion() -> HistoryCompletion {
    HistoryCompletion {
        provenance: HistoryProvenance::Recorded,
        status: Some(RunStatus::Completed),
        elapsed_ms: Some(19),
        execution_summary: None,
        snowflake_dml: None,
        snowflake_ddl: None,
    }
}
fn setup() -> (tempfile::TempDir, Persistence) {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    store.save_profile(&profile()).unwrap();
    (root, store)
}
fn bindings(store: &Persistence) -> i64 {
    store
        .connection
        .lock()
        .query_row("SELECT count(*) FROM history_owners_v6", [], |r| r.get(0))
        .unwrap()
}
fn owner_path(owner: &HistoryOwner) -> PathBuf {
    owner.lease.directory.join(&owner.lease.name)
}

#[test]
fn history_owner_recovers_only_dead_bound_tickets_without_audit_or_replay() {
    let (_root, store) = setup();
    let owner = store.claim_history_owner().unwrap();
    let entry = seed();
    let ticket = store.begin_owned_history(&owner, &entry).unwrap();
    let legacy = seed();
    store.begin_history(&legacy).unwrap();
    let observer = store.claim_history_owner().unwrap();
    assert_eq!(
        store.history_entry(&entry.id).unwrap().unwrap().provenance,
        HistoryProvenance::Admitted
    );
    drop(owner);
    let recovery = store.claim_history_owner().unwrap();
    let recovered = store.history_entry(&entry.id).unwrap().unwrap();
    assert_eq!(recovered.provenance, HistoryProvenance::Interrupted);
    assert_eq!(recovered.sql, entry.sql);
    assert_eq!(recovered.status, None);
    assert_eq!(recovered.elapsed_ms, None);
    assert!(recovered.execution_summary.is_none());
    assert!(recovered.snowflake_dml.is_none() && recovered.snowflake_ddl.is_none());
    assert_eq!(
        store.history_entry(&legacy.id).unwrap().unwrap().provenance,
        HistoryProvenance::Admitted
    );
    assert_eq!(bindings(&store), 0);
    assert!(store.audit_log().unwrap().is_empty());
    assert_eq!(
        store
            .begin_owned_history(&recovery, &legacy)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        store
            .begin_owned_history(&observer, &entry)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        store
            .finish_history(&ticket, &completion())
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    drop(recovery);
    let _again = store.claim_history_owner().unwrap();
    assert_eq!(
        store.history_entry(&entry.id).unwrap().unwrap().provenance,
        HistoryProvenance::Interrupted
    );
}

#[test]
fn history_owner_completed_receipt_survives_death_and_release_is_identity_bound() {
    let (_root, store) = setup();
    let owner = store.claim_history_owner().unwrap();
    let other = store.claim_history_owner().unwrap();
    let entry = seed();
    let ticket = store.begin_owned_history(&owner, &entry).unwrap();
    assert_eq!(store.begin_owned_history(&owner, &entry).unwrap(), ticket);
    assert_eq!(
        store.begin_owned_history(&other, &entry).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        store
            .release_owned_history(&owner, &ticket)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    store.finish_history(&ticket, &completion()).unwrap();
    assert_eq!(
        bindings(&store),
        1,
        "completion alone cannot release an unsettled publisher"
    );
    assert_eq!(
        store
            .release_owned_history(&other, &ticket)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let completed = store.history_entry(&entry.id).unwrap().unwrap();
    drop(owner);
    let _recovery = store.claim_history_owner().unwrap();
    assert_eq!(
        serde_json::to_value(store.history_entry(&entry.id).unwrap().unwrap()).unwrap(),
        serde_json::to_value(completed).unwrap()
    );
    assert_eq!(bindings(&store), 0);
    assert!(store.audit_log().unwrap().is_empty());
    let new = seed();
    let next = store.begin_owned_history(&other, &new).unwrap();
    store.finish_history(&next, &completion()).unwrap();
    store.release_owned_history(&other, &next).unwrap();
    store.release_owned_history(&other, &next).unwrap();
    let legacy = store.begin_history(&seed()).unwrap();
    store.release_owned_history(&other, &legacy).unwrap();
}

#[test]
fn history_owner_recovery_respects_clear_optout_delete_and_missing_visible_row() {
    for operation in ["clear", "optout", "delete", "missing"] {
        let (_root, store) = setup();
        let owner = store.claim_history_owner().unwrap();
        let entry = seed();
        let ticket = store.begin_owned_history(&owner, &entry).unwrap();
        match operation {
            "clear" => store.clear_history().unwrap(),
            "optout" => {
                let mut p = profile();
                p.store_query_text = false;
                store.save_profile(&p).unwrap();
                p.store_query_text = true;
                store.save_profile(&p).unwrap();
            }
            "delete" => store.delete_profile(&profile().id).unwrap(),
            _ => {
                store
                    .connection
                    .lock()
                    .execute("DELETE FROM history_v6 WHERE id=?1", [&entry.id])
                    .unwrap();
            }
        }
        drop(owner);
        let _recovery = store.claim_history_owner().unwrap();
        let visible = store.history_entry(&entry.id).unwrap();
        if matches!(operation, "clear" | "missing") {
            assert!(visible.is_none());
        } else {
            let visible = visible.unwrap();
            assert!(visible.sql.is_none());
            assert_eq!(visible.provenance, HistoryProvenance::Interrupted);
        }
        let (_, state, original) = lifecycle::lookup(&store.connection.lock(), ticket.id())
            .unwrap()
            .unwrap();
        assert_eq!(original.unwrap().provenance, HistoryProvenance::Interrupted);
        if matches!(operation, "clear" | "missing") {
            assert_eq!(state, super::super::HistoryDisposition::Suppressed);
        }
        assert_eq!(bindings(&store), 0);
    }
}

#[test]
fn history_owner_missing_proof_never_creates_a_recovery_file_or_terminal_claim() {
    let (_root, store) = setup();
    let owner = store.claim_history_owner().unwrap();
    let entry = seed();
    store.begin_owned_history(&owner, &entry).unwrap();
    let path = owner_path(&owner);
    drop(owner);
    fs::remove_file(&path).unwrap();
    let _recovery = store.claim_history_owner().unwrap();
    assert!(!path.exists());
    assert_eq!(bindings(&store), 1);
    assert_eq!(
        store.history_entry(&entry.id).unwrap().unwrap().provenance,
        HistoryProvenance::Admitted
    );
}

#[test]
#[cfg(unix)]
fn history_owner_rejects_symlink_permissions_hardlink_and_stale_owner_handles() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for fault in ["symlink", "permissions", "hardlink", "replacement"] {
        let (root, store) = setup();
        let owner = store.claim_history_owner().unwrap();
        let entry = seed();
        store.begin_owned_history(&owner, &entry).unwrap();
        let path = owner_path(&owner);
        match fault {
            "symlink" => {
                fs::rename(&path, root.path().join("original.lock")).unwrap();
                symlink(root.path().join("original.lock"), &path).unwrap();
            }
            "permissions" => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            "hardlink" => fs::hard_link(&path, root.path().join("other.lock")).unwrap(),
            _ => {
                fs::rename(&path, root.path().join("original.lock")).unwrap();
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .unwrap();
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
        }
        assert_eq!(
            store.begin_owned_history(&owner, &seed()).unwrap_err().code,
            ErrorCode::Conflict
        );
        if fault != "replacement" {
            drop(owner);
            assert!(store.claim_history_owner().is_err());
        }
        assert_eq!(
            store.history_entry(&entry.id).unwrap().unwrap().provenance,
            HistoryProvenance::Admitted
        );
    }
}

#[test]
fn history_owner_restore_rejects_live_completed_missing_and_invalid_proof_before_undo() {
    for fault in ["pending", "completed", "missing", "invalid"] {
        let (_root, store) = setup();
        let backup = store.create_metadata_backup().unwrap();
        let owner = store.claim_history_owner().unwrap();
        let entry = seed();
        let ticket = store.begin_owned_history(&owner, &entry).unwrap();
        if fault == "completed" {
            store.finish_history(&ticket, &completion()).unwrap();
        }
        let path = owner_path(&owner);
        if matches!(fault, "missing" | "invalid") {
            drop(owner);
            if fault == "missing" {
                fs::remove_file(path).unwrap();
            } else {
                fs::write(path, b"invalid").unwrap();
            }
        }
        let before = serde_json::to_value(store.history_entry(&entry.id).unwrap()).unwrap();
        assert_eq!(
            store
                .restore_metadata_backup(&backup.id, &backup.digest)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            store.metadata_backups().unwrap().len(),
            1,
            "refusal precedes undo publication"
        );
        assert_eq!(
            serde_json::to_value(store.history_entry(&entry.id).unwrap()).unwrap(),
            before
        );
        assert_eq!(bindings(&store), 1);
    }
}

#[test]
fn history_owner_restore_recovers_local_dead_maps_and_strips_backup_only_authority() {
    let (_root, store) = setup();
    let backup = store.create_metadata_backup().unwrap();
    let owner = store.claim_history_owner().unwrap();
    let local = seed();
    store.begin_owned_history(&owner, &local).unwrap();
    drop(owner);
    store
        .restore_metadata_backup(&backup.id, &backup.digest)
        .unwrap();
    assert_eq!(
        store.history_entry(&local.id).unwrap().unwrap().provenance,
        HistoryProvenance::Interrupted
    );
    assert_eq!(bindings(&store), 0);
    let (_source_root, source) = setup();
    // Fork the exact pre-admission privacy ancestry and profile generation.
    // The later imported execution and owner binding exist only in the backup.
    let target_root = tempfile::tempdir().unwrap();
    let mut target_connection =
        Connection::open(target_root.path().join("joinery.sqlite")).unwrap();
    {
        let source_connection = source.connection.lock();
        let backup =
            rusqlite::backup::Backup::new(&source_connection, &mut target_connection).unwrap();
        assert_eq!(backup.step(-1).unwrap(), rusqlite::backup::StepResult::Done);
    }
    target_connection
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    drop(target_connection);
    let target = Persistence::open(target_root.path()).unwrap();
    let source_owner = source.claim_history_owner().unwrap();
    let imported = seed();
    source
        .begin_owned_history(&source_owner, &imported)
        .unwrap();
    let snapshot = source.create_metadata_backup().unwrap();
    target.metadata_backups().unwrap();
    fs::copy(
        source
            .root
            .join("backups")
            .join(format!("{}.sqlite", snapshot.id)),
        target
            .root
            .join("backups")
            .join(format!("{}.sqlite", snapshot.id)),
    )
    .unwrap();
    // Even a matching local UUID inode from a copied tree cannot confer authority.
    let owner_directory = target.root.join("history-owners");
    fs::create_dir(&owner_directory).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&owner_directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::copy(
        owner_path(&source_owner),
        owner_directory.join(&source_owner.lease.name),
    )
    .unwrap();
    target
        .restore_metadata_backup(&snapshot.id, &snapshot.digest)
        .unwrap();
    assert_eq!(
        target
            .history_entry(&imported.id)
            .unwrap()
            .unwrap()
            .provenance,
        HistoryProvenance::RestoreRecovered
    );
    assert_eq!(bindings(&target), 0);
    drop(source_owner);
    let _next = target.claim_history_owner().unwrap();
    assert_eq!(
        target
            .history_entry(&imported.id)
            .unwrap()
            .unwrap()
            .provenance,
        HistoryProvenance::RestoreRecovered
    );
}

#[test]
fn history_owner_gate_excludes_admission_recovery_and_privacy_mutations() {
    let (_root, store) = setup();
    let owner = store.claim_history_owner().unwrap();
    let gate = AdmissionGate::claim(&store.root, true).unwrap();
    assert_eq!(
        store.begin_owned_history(&owner, &seed()).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(store.claim_history_owner().is_err());
    assert_eq!(
        store.begin_history(&seed()).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(store.clear_history().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(
        store.save_profile(&profile()).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        store.delete_profile(&profile().id).unwrap_err().code,
        ErrorCode::Conflict
    );
    drop(gate);
    let shared = AdmissionGate::claim(&store.root, false).unwrap();
    assert!(AdmissionGate::claim(&store.root, true).is_err());
    let _also_shared = AdmissionGate::claim(&store.root, false).unwrap();
    drop(shared);
    assert_eq!(bindings(&store), 0);
}

#[test]
fn history_owner_backup_validator_rejects_orphan_and_noncanonical_owner_records() {
    let (_root, store) = setup();
    let owner = store.claim_history_owner().unwrap();
    let entry = seed();
    store.begin_owned_history(&owner, &entry).unwrap();
    let connection = store.connection.lock();
    super::super::recovery::validate(&connection).unwrap();
    connection
        .execute(
            "UPDATE history_owners_v6 SET owner_nonce=?1",
            ["00000000-0000-4000-8000-00000000000A"],
        )
        .unwrap();
    assert!(super::super::recovery::validate(&connection).is_err());
    connection
        .execute(
            "UPDATE history_owners_v6 SET owner_nonce=?1,ticket_id='unknown'",
            [&owner.nonce],
        )
        .unwrap();
    assert!(super::super::recovery::validate(&connection).is_err());
}

#[test]
fn history_owner_v6_migration_fences_cached_v5_privacy_and_preserves_unbound_history() {
    let (root, store) = setup();
    let legacy = seed();
    store.begin_history(&legacy).unwrap();
    drop(store);
    let old = Connection::open(root.path().join("joinery.sqlite")).unwrap();
    old.execute_batch("DROP TABLE history_owners_v6; ALTER TABLE profiles_v6 RENAME TO profiles_v5; ALTER TABLE history_v6 RENAME TO history_v5; ALTER TABLE audit_v6 RENAME TO audit_v5; PRAGMA user_version=5;").unwrap();
    super::super::recovery::validate(&old).unwrap();
    let mut clear = old.prepare("DELETE FROM history_v5").unwrap();
    let mut save = old.prepare("UPDATE profiles_v5 SET json=json").unwrap();
    let mut delete = old.prepare("DELETE FROM profiles_v5").unwrap();
    let mut audit = old
        .prepare("INSERT INTO audit_v5 VALUES('stale',1,'profile','execute','digest','finished')")
        .unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let era: String = old
        .query_row("SELECT current_era FROM history_privacy_v5", [], |r| {
            r.get(0)
        })
        .unwrap();
    old.execute_batch("BEGIN IMMEDIATE; UPDATE history_tickets_v5 SET disposition='suppressed'; UPDATE history_privacy_v5 SET current_era='invalid';").unwrap();
    assert!(clear.execute([]).is_err());
    old.execute_batch("ROLLBACK").unwrap();
    assert!(save.execute([]).is_err());
    assert!(delete.execute([]).is_err());
    assert!(audit.execute([]).is_err());
    assert_eq!(
        old.query_row("SELECT current_era FROM history_privacy_v5", [], |r| r
            .get::<_, String>(
            0
        ))
        .unwrap(),
        era
    );
    let owner = store.claim_history_owner().unwrap();
    assert_eq!(
        store.history_entry(&legacy.id).unwrap().unwrap().provenance,
        HistoryProvenance::Admitted
    );
    assert_eq!(
        store.begin_owned_history(&owner, &legacy).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(schema_version(&store.connection.lock()).unwrap(), 6);
    assert!(store.audit_log().unwrap().is_empty());
}
