// Actual encrypted files and SQLite branches; these fixtures never create an
// engine connection or execute owner SQL.
fn history_restore_profile() -> ConnectionProfile {
    serde_json::from_value(serde_json::json!({
        "id": new_id(), "name":"Recovery history fixture", "groupId":"local",
        "engine":"snowflake", "host":"fixture.invalid", "port":443,
        "database":"fixture", "username":"fixture", "environment":"local",
        "safeMode":"readOnly", "tlsMode":"verifyFull", "caCertPath":null,
        "ssh":null, "rememberPassword":false, "storeQueryText":true
    }))
    .unwrap()
}

fn history_restore_pending(profile: &ConnectionProfile, sql: &str) -> HistoryEntry {
    HistoryEntry {
        provenance: HistoryProvenance::Admitted,
        execution_summary: None,
        snowflake_dml: None,
        snowflake_ddl: None,
        id: new_id(),
        connection_id: profile.id.clone(),
        connection_name: profile.name.clone(),
        started_at: now_ms(),
        sql: Some(sql.to_owned()),
        statement_count: 1,
        status: None,
        elapsed_ms: None,
    }
}

fn history_restore_branch(source: &Persistence) -> (tempfile::TempDir, Persistence) {
    let root = tempfile::tempdir().unwrap();
    let mut destination = Connection::open(root.path().join("joinery.sqlite")).unwrap();
    {
        let current = source.connection.lock();
        let backup = Backup::new(&current, &mut destination).unwrap();
        assert_eq!(backup.step(-1).unwrap(), StepResult::Done);
    }
    destination
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    drop(destination);
    let branch = Persistence::open(root.path()).unwrap();
    (root, branch)
}

fn history_restore_import(
    source: &Persistence,
    target: &Persistence,
    passphrase: &BackupPassphrase,
) -> RecoveryBackup {
    let backup = source
        .create_encrypted_metadata_backup(passphrase, &control())
        .unwrap();
    let mut file = File::open(
        source
            .root
            .join("backups")
            .join(format!("{}.age", backup.metadata.id)),
    )
    .unwrap();
    target
        .import_recovery_backup(&mut file, &control())
        .unwrap()
}

fn history_restore_apply(
    target: &Persistence,
    backup: &RecoveryBackup,
    passphrase: &BackupPassphrase,
) -> AppResult<RecoveryBackup> {
    let owner = target.claim_console_archive_owner()?;
    target.restore_encrypted_metadata_backup(
        &backup.metadata.id,
        &backup.metadata.digest,
        Some(passphrase),
        passphrase,
        &owner,
        &control(),
    )
}

#[test]
fn encrypted_history_restore_preserves_disjoint_pending_exact_sql_and_unknown_completion() {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let profile = history_restore_profile();
    store.save_profile(&profile).unwrap();
    let (_branch_root, branch) = history_restore_branch(&store);
    let live = history_restore_pending(&profile, " SELECT 'live unchanged';\n");
    let recovered = history_restore_pending(
        &profile,
        "\nSELECT '雪 exact original', 'a;b' /* owner text */;  \n",
    );
    store.begin_history(&live).unwrap();
    branch.begin_history(&recovered).unwrap();
    let passphrase = secret();
    let backup = history_restore_import(&branch, &store, &passphrase);
    history_restore_apply(&store, &backup, &passphrase).unwrap();
    for expected in [&live, &recovered] {
        let restored = store.history_entry(&expected.id).unwrap().unwrap();
        assert_eq!(restored.sql, expected.sql);
        assert_eq!(restored.status, None);
        assert_eq!(restored.elapsed_ms, None);
    }
    assert_eq!(
        store
            .history_entry(&recovered.id)
            .unwrap()
            .unwrap()
            .provenance,
        HistoryProvenance::RestoreRecovered
    );
    let connection = store.connection.lock();
    let copies: i64 = connection
        .query_row(
            "SELECT count(*) FROM history_v5 WHERE json_extract(json,'$.sql')=?1",
            [recovered.sql.as_deref()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(copies, 1);
    let copied_sql: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM history_tickets_v5 WHERE json_type(seed_json,'$.sql')='text' OR json_type(completion_json,'$.sql')='text')",
        [], |row| row.get(0),
    ).unwrap();
    assert!(!copied_sql, "lifecycle records must remain SQL-free");
    validate(&connection, &control()).unwrap();
}

#[test]
fn encrypted_history_restore_clear_suppresses_unseen_preclear_branch_sql_and_undo() {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let profile = history_restore_profile();
    store.save_profile(&profile).unwrap();
    let (_branch_root, branch) = history_restore_branch(&store);
    let unseen = history_restore_pending(&profile, "SELECT 'only in old branch'");
    branch.begin_history(&unseen).unwrap();
    let passphrase = secret();
    let backup = history_restore_import(&branch, &store, &passphrase);
    store.clear_history().unwrap();
    let current = history_restore_pending(&profile, "SELECT 'after clear'");
    store.begin_history(&current).unwrap();
    let undo = history_restore_apply(&store, &backup, &passphrase).unwrap();
    assert!(store.history_entry(&unseen.id).unwrap().is_none());
    assert_eq!(
        store.history_entry(&current.id).unwrap().unwrap().sql,
        current.sql
    );
    history_restore_apply(&store, &undo, &passphrase).unwrap();
    assert!(store.history_entry(&unseen.id).unwrap().is_none());
    assert_eq!(
        store.history_entry(&current.id).unwrap().unwrap().sql,
        current.sql
    );
}

#[test]
fn encrypted_history_restore_optout_or_delete_cannot_resurrect_sql_after_reenable() {
    for delete in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let mut profile = history_restore_profile();
        store.save_profile(&profile).unwrap();
        let shared = history_restore_pending(&profile, "SELECT 'shared before privacy change'");
        store.begin_history(&shared).unwrap();
        let (_branch_root, branch) = history_restore_branch(&store);
        let unseen = history_restore_pending(&profile, "SELECT 'unseen before profile change'");
        branch.begin_history(&unseen).unwrap();
        let passphrase = secret();
        let backup = history_restore_import(&branch, &store, &passphrase);
        if delete {
            store.delete_profile(&profile.id).unwrap();
        } else {
            profile.store_query_text = false;
            store.save_profile(&profile).unwrap();
        }
        profile.store_query_text = true;
        store.save_profile(&profile).unwrap();
        let current = history_restore_pending(&profile, "SELECT 'new generation consent'");
        store.begin_history(&current).unwrap();
        history_restore_apply(&store, &backup, &passphrase).unwrap();
        for old in [&shared, &unseen] {
            assert!(
                store
                    .history_entry(&old.id)
                    .unwrap()
                    .is_none_or(|entry| entry.sql.is_none()),
                "old SQL survived delete={delete}"
            );
        }
        assert_eq!(
            store.history_entry(&current.id).unwrap().unwrap().sql,
            current.sql
        );
        history_restore_apply(&store, &backup, &passphrase).unwrap();
        for old in [&shared, &unseen] {
            assert!(
                store
                    .history_entry(&old.id)
                    .unwrap()
                    .is_none_or(|entry| entry.sql.is_none()),
                "repeated restore resurrected SQL"
            );
        }
    }
}

#[test]
fn encrypted_history_restore_pristine_destination_adopts_lineage_and_pending_sql() {
    let source_root = tempfile::tempdir().unwrap();
    let source = Persistence::open(source_root.path()).unwrap();
    let profile = history_restore_profile();
    source.save_profile(&profile).unwrap();
    let pending = history_restore_pending(&profile, "SELECT 'new device owner text'");
    source.begin_history(&pending).unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Persistence::open(target_root.path()).unwrap();
    let passphrase = secret();
    let backup = history_restore_import(&source, &target, &passphrase);
    history_restore_apply(&target, &backup, &passphrase).unwrap();
    let recovered = target.history_entry(&pending.id).unwrap().unwrap();
    assert_eq!(recovered.sql, pending.sql);
    assert_eq!(recovered.provenance, HistoryProvenance::RestoreRecovered);
    assert_eq!(recovered.status, None);
}

#[test]
fn encrypted_history_restore_unrelated_or_divergent_eras_reject_before_undo_and_replacement() {
    for unrelated in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let profile = history_restore_profile();
        store.save_profile(&profile).unwrap();
        let (_branch_root, branch) = if unrelated {
            let root = tempfile::tempdir().unwrap();
            let branch = Persistence::open(root.path()).unwrap();
            branch.save_profile(&profile).unwrap();
            (root, branch)
        } else {
            history_restore_branch(&store)
        };
        if !unrelated {
            store.clear_history().unwrap();
            branch.clear_history().unwrap();
        }
        let local = history_restore_pending(&profile, "SELECT 'must survive rejection'");
        let remote = history_restore_pending(&profile, "SELECT 'unverifiable foreign branch'");
        store.begin_history(&local).unwrap();
        branch.begin_history(&remote).unwrap();
        store.set_setting("recovery-fixture", "live").unwrap();
        branch.set_setting("recovery-fixture", "backup").unwrap();
        let passphrase = secret();
        let backup = history_restore_import(&branch, &store, &passphrase);
        let before = store.recovery_backups().unwrap().len();
        assert_eq!(
            history_restore_apply(&store, &backup, &passphrase)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(setting(&store), "live");
        assert_eq!(
            store.recovery_backups().unwrap().len(),
            before,
            "rejected merge must not publish undo"
        );
        assert_eq!(
            store.history_entry(&local.id).unwrap().unwrap().sql,
            local.sql
        );
        assert!(store.history_entry(&remote.id).unwrap().is_none());
        assert_eq!(staged_files(root.path()), 0);
    }
}

#[test]
fn encrypted_history_restore_conflicting_original_sql_rejects_before_undo() {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let profile = history_restore_profile();
    store.save_profile(&profile).unwrap();
    let original = history_restore_pending(&profile, "SELECT 'immutable original'");
    store.begin_history(&original).unwrap();
    let (_branch_root, branch) = history_restore_branch(&store);
    {
        // Make a valid but contradictory original. Lifecycle metadata carries
        // no duplicate SQL to authenticate or silently overwrite either text.
        let mut conflicting = original.clone();
        conflicting.sql = Some("SELECT 'different original'".into());
        branch
            .connection
            .lock()
            .execute(
                "UPDATE history_v5 SET json=?1 WHERE id=?2",
                rusqlite::params![serde_json::to_string(&conflicting).unwrap(), original.id],
            )
            .unwrap();
        branch
            .connection
            .lock()
            .execute(
                "UPDATE history_tickets_v5 SET sql_digest=?1 WHERE id=?2",
                rusqlite::params![
                    hex::encode(Sha256::digest(
                        conflicting.sql.as_deref().unwrap().as_bytes()
                    )),
                    original.id
                ],
            )
            .unwrap();
    }
    let passphrase = secret();
    let backup = history_restore_import(&branch, &store, &passphrase);
    let before = store.recovery_backups().unwrap().len();
    assert_eq!(
        history_restore_apply(&store, &backup, &passphrase)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        store.history_entry(&original.id).unwrap().unwrap().sql,
        original.sql
    );
    assert_eq!(store.recovery_backups().unwrap().len(), before);
}

#[test]
fn encrypted_history_restore_conflicting_completion_rejects_before_undo() {
    use super::super::history::HistoryCompletion;
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let profile = history_restore_profile();
    store.save_profile(&profile).unwrap();
    let original = history_restore_pending(&profile, "SELECT 'completion conflict'");
    let ticket = store.begin_history(&original).unwrap();
    let (_branch_root, branch) = history_restore_branch(&store);
    let complete = HistoryCompletion {
        provenance: HistoryProvenance::Recorded,
        status: Some(RunStatus::Completed),
        elapsed_ms: Some(11),
        execution_summary: None,
        snowflake_dml: None,
        snowflake_ddl: None,
    };
    store.finish_history(&ticket, &complete).unwrap();
    let mut conflicting = complete.clone();
    conflicting.status = Some(RunStatus::Failed);
    branch.finish_history(&ticket, &conflicting).unwrap();
    let passphrase = secret();
    let backup = history_restore_import(&branch, &store, &passphrase);
    let before = store.recovery_backups().unwrap().len();
    assert_eq!(
        history_restore_apply(&store, &backup, &passphrase)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let preserved = store.history_entry(&original.id).unwrap().unwrap();
    assert_eq!(preserved.status, Some(RunStatus::Completed));
    assert_eq!(preserved.elapsed_ms, Some(11));
    assert_eq!(preserved.sql, original.sql);
    assert_eq!(store.recovery_backups().unwrap().len(), before);
}

#[test]
fn encrypted_history_restore_missing_row_stays_suppressed_after_late_finish() {
    use super::super::history::{HistoryCompletion, HistoryDisposition};
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let profile = history_restore_profile();
    store.save_profile(&profile).unwrap();
    let original = history_restore_pending(&profile, "SELECT 'removed retained row'");
    let ticket = store.begin_history(&original).unwrap();
    let (_branch_root, branch) = history_restore_branch(&store);
    let passphrase = secret();
    let backup = history_restore_import(&branch, &store, &passphrase);
    // A row can disappear through bounded retention independently of a clear
    // era. Its SQL-free admission evidence must not reinsert backup SQL.
    store
        .connection
        .lock()
        .execute("DELETE FROM history_v5 WHERE id=?1", [&original.id])
        .unwrap();
    history_restore_apply(&store, &backup, &passphrase).unwrap();
    assert!(store.history_entry(&original.id).unwrap().is_none());
    let disposition = store
        .finish_history(
            &ticket,
            &HistoryCompletion {
                provenance: HistoryProvenance::Recorded,
                status: Some(RunStatus::Completed),
                elapsed_ms: Some(1),
                execution_summary: None,
                snowflake_dml: None,
                snowflake_ddl: None,
            },
        )
        .unwrap();
    assert_eq!(disposition, HistoryDisposition::Suppressed);
    history_restore_apply(&store, &backup, &passphrase).unwrap();
    assert!(store.history_entry(&original.id).unwrap().is_none());
}

#[test]
fn encrypted_history_restore_migrated_v4_sql_cannot_return_after_live_clear() {
    let root = tempfile::tempdir().unwrap();
    let profile = history_restore_profile();
    let mut original = history_restore_pending(
        &profile,
        "\nSELECT 'legacy original must stay cleared 雪';  \n",
    );
    original.provenance = HistoryProvenance::Recorded;
    original.status = Some(RunStatus::Completed);
    original.elapsed_ms = Some(7);
    {
        // Build the supported physical v4 schema, including its actual journal
        // migration. Changing only a v5 user_version would be an invalid fixture.
        let legacy = Connection::open(root.path().join("joinery.sqlite")).unwrap();
        legacy.execute_batch(
            "CREATE TABLE profiles(id TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE groups(id TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE history(id TEXT PRIMARY KEY, started_at INTEGER NOT NULL, json TEXT NOT NULL);
             CREATE TABLE audit(id TEXT PRIMARY KEY, occurred_at INTEGER NOT NULL, connection_id TEXT NOT NULL, action TEXT NOT NULL, digest TEXT NOT NULL, outcome TEXT NOT NULL);
             CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);"
        ).unwrap();
        legacy.execute_batch(snowflake_console::SCHEMA_SQL).unwrap();
        legacy
            .execute_batch(
                "ALTER TABLE snowflake_console_runs RENAME TO snowflake_console_runs_v3;
             ALTER TABLE snowflake_console_steps RENAME TO snowflake_console_steps_v3;",
            )
            .unwrap();
        legacy
            .execute_batch(snowflake_console::REJECTION_SCHEMA_SQL)
            .unwrap();
        for (_, sql) in snowflake_console::DECISION_TRIGGERS {
            legacy.execute_batch(sql).unwrap();
        }
        legacy.pragma_update(None, "user_version", 3).unwrap();
        snowflake_console::migrate_v4(&legacy).unwrap();
        legacy.pragma_update(None, "user_version", 4).unwrap();
        legacy
            .execute(
                "INSERT INTO profiles VALUES(?1,?2)",
                rusqlite::params![profile.id, serde_json::to_string(&profile).unwrap()],
            )
            .unwrap();
        let mut old_json = serde_json::to_value(&original).unwrap();
        old_json.as_object_mut().unwrap().remove("provenance");
        legacy
            .execute(
                "INSERT INTO history VALUES(?1,?2,?3)",
                rusqlite::params![
                    original.id,
                    original.started_at as i64,
                    serde_json::to_string(&old_json).unwrap()
                ],
            )
            .unwrap();
        super::super::recovery::validate(&legacy).unwrap();
    }
    let store = Persistence::open(root.path()).unwrap();
    assert_eq!(schema_version(&store.connection.lock()).unwrap(), 5);
    let migrated = store.history_entry(&original.id).unwrap().unwrap();
    assert_eq!(migrated.sql, original.sql);
    assert_eq!(migrated.status, Some(RunStatus::Completed));
    assert_eq!(migrated.elapsed_ms, Some(7));
    let passphrase = secret();
    let backup = store
        .create_encrypted_metadata_backup(&passphrase, &control())
        .unwrap();
    store.clear_history().unwrap();
    assert!(store.history_entry(&original.id).unwrap().is_none());
    history_restore_apply(&store, &backup, &passphrase).unwrap();
    assert!(
        store.history_entry(&original.id).unwrap().is_none(),
        "v4 migration must give legacy SQL the same irreversible clear authority"
    );
    history_restore_apply(&store, &backup, &passphrase).unwrap();
    assert!(
        store.history_entry(&original.id).unwrap().is_none(),
        "repeating the old encrypted restore must not resurrect legacy SQL"
    );
    let connection = store.connection.lock();
    let retained: i64 = connection
        .query_row(
            "SELECT count(*) FROM history_v5 WHERE json_extract(json,'$.sql')=?1",
            [original.sql.as_deref()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, 0);
}
