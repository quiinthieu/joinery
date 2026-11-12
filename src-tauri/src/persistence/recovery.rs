//! App-owned metadata snapshots. Paths never cross IPC; result buffers and OS secrets
//! are not part of this SQLite store. An incomplete SQLite backup rolls back on drop.
use super::{
    Persistence, SCHEMA_VERSION, db_error, initialize_schema, schema_version, snowflake_console,
    validate_schema,
};
use crate::domain::*;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension,
    backup::{Backup, StepResult},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

const MAX_BACKUPS: usize = 20;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

fn io_error(_: std::io::Error) -> AppError {
    AppError::internal("Could not access local metadata backups")
}

fn private_file(path: &Path) -> AppResult<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(io_error)
}

fn copy(source: &Connection, destination: &mut Connection) -> AppResult<()> {
    #[cfg(test)]
    let pause_restore = tests::PAUSE_RESTORE.load(std::sync::atomic::Ordering::Relaxed)
        && destination
            .path()
            .is_some_and(|path| path.ends_with("joinery.sqlite"));
    let backup = Backup::new(source, destination).map_err(db_error)?;
    #[cfg(test)]
    if pause_restore {
        assert_eq!(backup.step(1).unwrap(), StepResult::More);
        let root = std::env::var_os("JOINERY_RESTORE_FIXTURE_ROOT").unwrap();
        fs::write(Path::new(&root).join("restore-in-progress"), b"ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    if backup.step(-1).map_err(db_error)? != StepResult::Done {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "Metadata is busy; retry the backup or restore after other Joinery instances close",
        ));
    }
    Ok(())
}

pub(super) fn validate(connection: &Connection) -> AppResult<()> {
    let version = schema_version(connection)?;
    if !(1..=SCHEMA_VERSION).contains(&version) {
        return Err(AppError::invalid(
            "This backup needs a compatible Joinery version",
        ));
    }
    let check: String = connection
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(db_error)?;
    if check != "ok" {
        return Err(AppError::invalid(
            "The metadata backup failed its integrity check",
        ));
    }
    validate_schema(connection)?;
    super::workspace::read_workspace(connection)?;
    super::library::read_saved_queries(connection)?;
    super::library::read_bookmarks(connection)?;
    super::library::read_sql_format_settings(connection)?;
    super::library::read_sql_snippets(connection)?;
    if version >= 2 {
        snowflake_console::validate_records(connection)?;
    }
    let mut allowed_objects = ["profiles", "groups", "history", "audit", "settings"]
        .into_iter()
        .map(|name| ("table".to_owned(), name.to_owned()))
        .collect::<Vec<_>>();
    // Journal validation above verifies exact version-specific definitions.
    // Admit only that same version's known objects, including v4 browsing
    // indexes; a name from another schema version conveys no compatibility.
    if version >= 2 {
        allowed_objects.extend(snowflake_console::schema_objects(version)?);
    }
    let mut objects = connection
        .prepare("SELECT type,name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'")
        .map_err(db_error)?;
    let rows = objects
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_error)?;
    for object in rows {
        if !allowed_objects.contains(&object.map_err(db_error)?) {
            return Err(AppError::invalid(
                "The metadata backup contains unexpected schema objects",
            ));
        }
    }
    for (table, kind) in [("profiles", 0), ("groups", 1), ("history", 2)] {
        let mut statement = connection
            .prepare(&format!("SELECT json FROM {table}"))
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?;
        for row in rows {
            let row = row.map_err(db_error)?;
            let valid = match kind {
                0 => serde_json::from_str::<ConnectionProfile>(&row).is_ok(),
                1 => serde_json::from_str::<ConnectionGroup>(&row).is_ok(),
                _ => serde_json::from_str::<HistoryEntry>(&row).is_ok(),
            };
            if !valid {
                return Err(AppError::invalid(
                    "The metadata backup contains invalid records",
                ));
            }
        }
    }
    Ok(())
}

fn info(path: &Path, id: &str) -> AppResult<MetadataBackup> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(AppError::invalid(
            "Backups must be regular files of at most 64 MiB",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(AppError::invalid("The metadata backup is too large"));
    }
    Ok(MetadataBackup {
        id: id.to_owned(),
        created_at: metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64),
        size_bytes: bytes.len() as u64,
        digest: hex::encode(Sha256::digest(&bytes)),
    })
}

impl Persistence {
    fn backup_directory(&self) -> AppResult<PathBuf> {
        let directory = self.root.join("backups");
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_error(e)),
        }
        if !fs::symlink_metadata(&directory)
            .map_err(io_error)?
            .file_type()
            .is_dir()
        {
            return Err(AppError::invalid(
                "The metadata backup directory must not be a link",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        }
        Ok(directory)
    }

    fn backup_path(&self, id: &str) -> AppResult<PathBuf> {
        let parsed = uuid::Uuid::parse_str(id)
            .map_err(|_| AppError::invalid("Invalid metadata backup identifier"))?;
        if parsed.to_string() != id {
            return Err(AppError::invalid("Invalid metadata backup identifier"));
        }
        Ok(self.backup_directory()?.join(format!("{id}.sqlite")))
    }

    pub fn metadata_backups(&self) -> AppResult<Vec<MetadataBackup>> {
        let mut backups = Vec::new();
        for entry in fs::read_dir(self.backup_directory()?).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".sqlite")) else {
                continue;
            };
            if uuid::Uuid::parse_str(id).is_err() {
                continue;
            }
            backups.push(info(&entry.path(), id)?);
            if backups.len() > MAX_BACKUPS {
                return Err(AppError::invalid(
                    "Too many metadata backups; remove older copies from the application backup directory",
                ));
            }
        }
        backups.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(backups)
    }

    fn snapshot(&self, source: &Connection) -> AppResult<MetadataBackup> {
        if self.metadata_backups()?.len() >= MAX_BACKUPS {
            return Err(AppError::invalid(
                "Keep fewer than 20 metadata backups before creating a snapshot or restoring",
            ));
        }
        let size: i64 = source
            .query_row(
                "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
                [],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if size < 0 || size as u64 > MAX_BYTES {
            return Err(AppError::invalid(
                "Metadata snapshots are limited to 64 MiB",
            ));
        }
        let id = new_id();
        let path = self.backup_path(&id)?;
        let partial = path.with_extension("partial");
        let file = private_file(&partial)?;
        let result = (|| {
            let mut destination = Connection::open_with_flags(
                &partial,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )
            .map_err(db_error)?;
            destination
                .pragma_update(None, "synchronous", "FULL")
                .map_err(db_error)?;
            copy(source, &mut destination)?;
            // Backup copies the WAL-mode header too. Publish a single closed file.
            destination
                .pragma_update(None, "journal_mode", "DELETE")
                .map_err(db_error)?;
            validate(&destination)?;
            drop(destination);
            file.sync_all().map_err(io_error)?;
            fs::rename(&partial, &path).map_err(io_error)?;
            #[cfg(unix)]
            File::open(self.backup_directory()?)
                .map_err(io_error)?
                .sync_all()
                .map_err(io_error)?;
            info(&path, &id)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&partial);
        }
        result
    }

    pub fn create_metadata_backup(&self) -> AppResult<MetadataBackup> {
        self.snapshot(&self.connection.lock())
    }

    pub fn delete_metadata_backup(&self, id: &str, digest: &str) -> AppResult<()> {
        let _connection = self.connection.lock();
        let path = self.backup_path(id)?;
        if info(&path, id)?.digest != digest {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The backup changed; refresh before deleting it",
            ));
        }
        fs::remove_file(path).map_err(io_error)
    }

    /// Called only after Studio serializes connections/configuration and authorizes restore.
    /// Returns the undo snapshot; OS credentials and the current passphrase flag are retained.
    pub(crate) fn restore_metadata_backup(
        &self,
        id: &str,
        digest: &str,
    ) -> AppResult<MetadataBackup> {
        // The process-held lease prevents restore from replacing the journal
        // while another application instance still owns an executing console.
        let _console_owner = self.claim_console_journal_owner()?;
        let mut current = self.connection.lock();
        let path = self.backup_path(id)?;
        let before = info(&path, id)?;
        if before.digest != digest {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The backup changed; refresh before restoring it",
            ));
        }
        let source = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(db_error)?;
        if source
            .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
            .map_err(db_error)?
            != "delete"
        {
            return Err(AppError::invalid(
                "Restore requires a self-contained metadata snapshot",
            ));
        }
        let mut pinned = Connection::open_in_memory().map_err(db_error)?;
        copy(&source, &mut pinned)?;
        validate(&pinned)?;
        if info(&path, id)?.digest != before.digest {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The backup changed during validation",
            ));
        }
        // Upgrade a legacy snapshot privately. Unresolved restored attempts are
        // sealed before replacement, so copying cannot create replayable work.
        initialize_schema(&mut pinned, None)?;
        snowflake_console::merge_for_restore(&current, &mut pinned)?;
        snowflake_console::interrupt_records(&mut pinned)?;
        validate(&pinned)?;
        // The verifier lives in the OS store, which is intentionally never backed up.
        let configured = current
            .query_row(
                "SELECT value FROM settings WHERE key='passphrase-configured'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?
            .unwrap_or_else(|| "false".into());
        pinned.execute("INSERT INTO settings VALUES('passphrase-configured',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&configured]).map_err(db_error)?;
        let undo = self.snapshot(&current)?;
        copy(&pinned, &mut current)?;
        Ok(undo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) static PAUSE_RESTORE: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    // These fixtures start with an empty current journal. Rebuild an actual
    // legacy schema, rather than changing only user_version on newer tables.
    fn legacy_empty_backup(path: &Path, version: i64) {
        assert!((1..=3).contains(&version));
        let old = Connection::open(path).unwrap();
        old.pragma_update(None, "foreign_keys", false).unwrap();
        let objects = snowflake_console::schema_objects(SCHEMA_VERSION).unwrap();
        for (kind, name) in &objects {
            if kind == "table" && name != "snowflake_console_retention_v4" {
                let count: i64 = old
                    .query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(count, 0, "legacy fixture must not discard journal evidence");
            }
        }
        for kind in ["trigger", "index", "table"] {
            for (object_kind, name) in &objects {
                if object_kind == kind {
                    old.execute_batch(&format!("DROP {kind} \"{name}\";"))
                        .unwrap();
                }
            }
        }
        if version >= 2 {
            old.execute_batch(snowflake_console::SCHEMA_SQL).unwrap();
        }
        if version == 3 {
            old.execute_batch(
                "ALTER TABLE snowflake_console_runs RENAME TO snowflake_console_runs_v3;
                 ALTER TABLE snowflake_console_steps RENAME TO snowflake_console_steps_v3;",
            )
            .unwrap();
            old.execute_batch(snowflake_console::REJECTION_SCHEMA_SQL)
                .unwrap();
            for (_, sql) in snowflake_console::DECISION_TRIGGERS {
                old.execute_batch(sql).unwrap();
            }
        }
        old.pragma_update(None, "user_version", version).unwrap();
        validate(&old).unwrap();
    }

    #[test]
    #[ignore = "subprocess helper for killed_restore_keeps_complete_live_metadata"]
    fn restore_writer() {
        let root = std::env::var_os("JOINERY_RESTORE_FIXTURE_ROOT").unwrap();
        let snapshot_id = std::env::var("JOINERY_RESTORE_SNAPSHOT_ID").unwrap();
        let snapshot_digest = std::env::var("JOINERY_RESTORE_SNAPSHOT_DIGEST").unwrap();
        let db = Persistence::open(Path::new(&root)).unwrap();
        PAUSE_RESTORE.store(true, std::sync::atomic::Ordering::Relaxed);
        db.restore_metadata_backup(&snapshot_id, &snapshot_digest)
            .unwrap();
    }

    #[test]
    fn killed_restore_keeps_complete_live_metadata_and_undo_snapshot() {
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let root = tempfile::tempdir().unwrap();
        let snapshot = {
            let db = Persistence::open(root.path()).unwrap();
            db.set_setting("fixture-one", "before").unwrap();
            db.set_setting("fixture-two", "before").unwrap();
            let snapshot = db.create_metadata_backup().unwrap();
            db.set_setting("fixture-one", "after").unwrap();
            db.set_setting("fixture-two", "after").unwrap();
            snapshot
        };
        let mut child = Child(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "persistence::recovery::tests::restore_writer",
                    "--ignored",
                ])
                .env("JOINERY_RESTORE_FIXTURE_ROOT", root.path())
                .env("JOINERY_RESTORE_SNAPSHOT_ID", &snapshot.id)
                .env("JOINERY_RESTORE_SNAPSHOT_DIGEST", &snapshot.digest)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !root.path().join("restore-in-progress").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "restore child exited before its checkpoint"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "restore child did not reach its checkpoint"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        assert_eq!(db.setting("fixture-one").unwrap().as_deref(), Some("after"));
        assert_eq!(db.setting("fixture-two").unwrap().as_deref(), Some("after"));
        assert_eq!(db.metadata_backups().unwrap().len(), 2);
        validate(&db.connection.lock()).unwrap();
    }

    #[test]
    fn snapshot_quota_and_oversized_inputs_do_not_change_live_metadata() {
        let root = tempfile::tempdir().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        db.set_setting("fixture", "retained").unwrap();
        let snapshot = db.create_metadata_backup().unwrap();
        let path = db.backup_path(&snapshot.id).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(
            db.restore_metadata_backup(&snapshot.id, &snapshot.digest)
                .is_err()
        );
        fs::remove_file(path).unwrap();
        for _ in 0..MAX_BACKUPS {
            db.create_metadata_backup().unwrap();
        }
        assert!(db.create_metadata_backup().is_err());
        let selected = db.metadata_backups().unwrap().remove(0);
        assert!(
            db.restore_metadata_backup(&selected.id, &selected.digest)
                .is_err()
        );
        assert_eq!(db.setting("fixture").unwrap().as_deref(), Some("retained"));
        db.delete_metadata_backup(&selected.id, &selected.digest)
            .unwrap();
        db.create_metadata_backup().unwrap();
    }

    #[test]
    fn wal_snapshot_restores_metadata_and_preserves_current_passphrase_state() {
        let dir = tempfile::tempdir().unwrap();
        let db = Persistence::open(dir.path()).unwrap();
        db.set_setting("fixture", "before").unwrap();
        let snapshot = db.create_metadata_backup().unwrap();
        db.set_setting("fixture", "after").unwrap();
        db.set_setting("passphrase-configured", "true").unwrap();
        let undo = db
            .restore_metadata_backup(&snapshot.id, &snapshot.digest)
            .unwrap();
        assert_eq!(db.setting("fixture").unwrap().as_deref(), Some("before"));
        assert_eq!(
            db.setting("passphrase-configured").unwrap().as_deref(),
            Some("true")
        );
        db.restore_metadata_backup(&undo.id, &undo.digest).unwrap();
        assert_eq!(db.setting("fixture").unwrap().as_deref(), Some("after"));
        assert_eq!(db.metadata_backups().unwrap().len(), 3);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(db.backup_path(&snapshot.id).unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn version_one_backup_upgrades_privately_and_restores_existing_metadata() {
        let root = tempfile::tempdir().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        db.set_setting("legacy-marker", "before").unwrap();
        let snapshot = db.create_metadata_backup().unwrap();
        let path = db.backup_path(&snapshot.id).unwrap();
        legacy_empty_backup(&path, 1);
        let legacy = info(&path, &snapshot.id).unwrap();
        let snapshot_bytes = fs::read(&path).unwrap();
        db.set_setting("legacy-marker", "after").unwrap();
        db.restore_metadata_backup(&legacy.id, &legacy.digest)
            .unwrap();
        assert_eq!(
            db.setting("legacy-marker").unwrap().as_deref(),
            Some("before")
        );
        assert_eq!(
            schema_version(&db.connection.lock()).unwrap(),
            SCHEMA_VERSION
        );
        snowflake_console::validate_schema(&db.connection.lock()).unwrap();
        // Upgrading the private copy must not rewrite the user's backup.
        assert_eq!(fs::read(path).unwrap(), snapshot_bytes);
    }

    #[test]
    fn version_four_backup_accepts_exact_indexes_and_rejects_schema_substitutions() {
        let root = tempfile::tempdir().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        assert_eq!(schema_version(&db.connection.lock()).unwrap(), 4);
        db.set_setting("v4-recovery", "snapshot").unwrap();
        let snapshot = db.create_metadata_backup().unwrap();
        let path = db.backup_path(&snapshot.id).unwrap();
        let bytes = fs::read(&path).unwrap();
        let index = snowflake_console::schema_objects(4)
            .unwrap()
            .into_iter()
            .find(|(kind, _)| kind == "index")
            .expect("v4 backup must exercise its explicit browsing indexes")
            .1;
        db.set_setting("v4-recovery", "live").unwrap();
        let undo = db
            .restore_metadata_backup(&snapshot.id, &snapshot.digest)
            .unwrap();
        assert_ne!(undo.id, snapshot.id);
        assert_eq!(
            db.setting("v4-recovery").unwrap().as_deref(),
            Some("snapshot")
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        db.set_setting("v4-recovery", "preserved").unwrap();
        for sql in [
            "CREATE INDEX unexpected_backup_index ON profiles(id);".to_owned(),
            "CREATE VIEW unexpected_backup_view AS SELECT id FROM profiles;".to_owned(),
            "CREATE TABLE snowflake_console_runs_v3(run_id TEXT);".to_owned(),
            format!("DROP INDEX \"{index}\"; CREATE INDEX \"{index}\" ON profiles(id);"),
        ] {
            let changed = Connection::open(&path).unwrap();
            changed.execute_batch(&sql).unwrap();
            drop(changed);
            let changed = info(&path, &snapshot.id).unwrap();
            assert!(
                db.restore_metadata_backup(&changed.id, &changed.digest)
                    .is_err()
            );
            assert_eq!(
                db.setting("v4-recovery").unwrap().as_deref(),
                Some("preserved")
            );
            fs::write(&path, &bytes).unwrap();
        }
        assert_eq!(db.metadata_backups().unwrap().len(), 2);
        validate(&db.connection.lock()).unwrap();
    }

    #[test]
    fn console_journal_old_and_legacy_restore_preserve_live_ack_and_client_identity() {
        use snowflake_console::{
            ConsoleAcknowledgement, ConsoleContext, ConsoleObservations, ConsoleRunMetadata,
            ConsoleStatement, ConsoleStepOutcome, ConsoleStopReason,
        };
        for legacy in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let db = Persistence::open(root.path()).unwrap();
            db.set_setting("restore-marker", "before").unwrap();
            let snapshot = db.create_metadata_backup().unwrap();
            let path = db.backup_path(&snapshot.id).unwrap();
            if legacy {
                legacy_empty_backup(&path, 1);
            }
            let snapshot = info(&path, &snapshot.id).unwrap();
            let backup_bytes = fs::read(&path).unwrap();
            let owner = db.claim_console_journal_owner().unwrap();
            let metadata = ConsoleRunMetadata {
                run_id: new_id(),
                client_request_id: new_id(),
                session_id: new_id(),
                connection_id: new_id(),
                plan_digest: "a".repeat(64),
                step_count: 1,
                created_at: 1000,
            };
            let run = db
                .console_prepare_run(&owner, metadata.clone())
                .unwrap()
                .run;
            let pending = db
                .console_step_intent(&owner, &metadata.run_id, run.revision, 0, 1001)
                .unwrap();
            let acknowledged = db
                .console_step_outcome(
                    &owner,
                    &metadata.run_id,
                    pending.revision,
                    0,
                    ConsoleStepOutcome::Acknowledged(ConsoleAcknowledgement {
                        statement: ConsoleStatement {
                            sdk_request_id: new_id(),
                            query_id: Some(new_id()),
                        },
                        context: ConsoleContext::default(),
                    }),
                    ConsoleObservations::default(),
                )
                .unwrap();
            let terminal = db
                .console_finish(
                    &owner,
                    &metadata.run_id,
                    acknowledged.revision,
                    ConsoleStopReason::Completed,
                    2000,
                )
                .unwrap();
            db.set_setting("restore-marker", "after").unwrap();
            // Even an idle retained console owns this journal across runs.
            assert!(
                db.restore_metadata_backup(&snapshot.id, &snapshot.digest)
                    .is_err()
            );
            assert_eq!(
                db.console_run(&metadata.run_id).unwrap(),
                Some(terminal.clone())
            );
            assert_eq!(
                db.setting("restore-marker").unwrap().as_deref(),
                Some("after")
            );
            drop(owner);

            db.restore_metadata_backup(&snapshot.id, &snapshot.digest)
                .unwrap();
            assert_eq!(
                db.setting("restore-marker").unwrap().as_deref(),
                Some("before")
            );
            assert_eq!(
                db.console_run(&metadata.run_id).unwrap(),
                Some(terminal.clone())
            );
            assert_eq!(
                db.console_run_by_client_request(&metadata.client_request_id)
                    .unwrap(),
                Some(terminal.clone())
            );
            let owner = db.claim_console_journal_owner().unwrap();
            let mut retry = metadata;
            retry.run_id = new_id();
            retry.created_at = 3000;
            let existing = db.console_prepare_run(&owner, retry).unwrap();
            assert!(!existing.created);
            assert_eq!(existing.run, terminal);
            assert_eq!(fs::read(path).unwrap(), backup_bytes);
        }
    }

    #[test]
    fn console_negative_versions_one_through_four_restore_preserve_live_refusals() {
        use snowflake_console::{ConsoleAdmissionRejection, ConsoleRunMetadata};
        for version in [1, 2, 3, 4] {
            let root = tempfile::tempdir().unwrap();
            let db = Persistence::open(root.path()).unwrap();
            db.set_setting("refusal-restore", "before").unwrap();
            let backup = db.create_metadata_backup().unwrap();
            let path = db.backup_path(&backup.id).unwrap();
            if version < SCHEMA_VERSION {
                legacy_empty_backup(&path, version);
            }
            let backup = info(&path, &backup.id).unwrap();
            let bytes = fs::read(&path).unwrap();
            let owner = db.claim_console_journal_owner().unwrap();
            let refused = ConsoleAdmissionRejection {
                client_request_id: new_id(),
                session_id: new_id(),
                plan_digest: "a".repeat(64),
                code: ErrorCode::ApprovalRequired,
                recorded_at: 1000,
            };
            db.console_record_rejection(&owner, &refused).unwrap();
            db.set_setting("refusal-restore", "after").unwrap();
            drop(owner);
            db.restore_metadata_backup(&backup.id, &backup.digest)
                .unwrap();
            assert_eq!(
                db.setting("refusal-restore").unwrap().as_deref(),
                Some("before")
            );
            assert_eq!(
                db.console_rejection_by_client_request(&refused.client_request_id)
                    .unwrap(),
                Some(refused.clone())
            );
            assert_eq!(
                fs::read(&path).unwrap(),
                bytes,
                "private migration must preserve backup bytes"
            );
            let owner = db.claim_console_journal_owner().unwrap();
            assert_eq!(
                db.console_prepare_run(
                    &owner,
                    ConsoleRunMetadata {
                        run_id: new_id(),
                        client_request_id: refused.client_request_id,
                        session_id: refused.session_id,
                        connection_id: new_id(),
                        plan_digest: refused.plan_digest,
                        step_count: 1,
                        created_at: 2000,
                    }
                )
                .err()
                .unwrap()
                .code,
                ErrorCode::Conflict
            );
        }
    }

    #[test]
    fn console_negative_backup_conflicts_and_changed_guards_never_replace_live_store() {
        use snowflake_console::ConsoleAdmissionRejection;
        let root = tempfile::tempdir().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        let owner = db.claim_console_journal_owner().unwrap();
        let refused = ConsoleAdmissionRejection {
            client_request_id: new_id(),
            session_id: new_id(),
            plan_digest: "a".repeat(64),
            code: ErrorCode::PolicyBlocked,
            recorded_at: 1000,
        };
        db.console_record_rejection(&owner, &refused).unwrap();
        drop(owner);
        let backup = db.create_metadata_backup().unwrap();
        let path = db.backup_path(&backup.id).unwrap();
        db.set_setting("live-refusal", "preserved").unwrap();
        let copy = Connection::open(&path).unwrap();
        copy.execute("UPDATE snowflake_console_rejections_v4 SET recorded_at=2000 WHERE client_request_id=?1", [&refused.client_request_id]).unwrap();
        drop(copy);
        let changed = info(&path, &backup.id).unwrap();
        assert_eq!(
            db.restore_metadata_backup(&changed.id, &changed.digest)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            db.console_rejection_by_client_request(&refused.client_request_id)
                .unwrap(),
            Some(refused.clone())
        );
        let copy = Connection::open(&path).unwrap();
        let guard = snowflake_console::schema_objects(SCHEMA_VERSION)
            .unwrap()
            .into_iter()
            .find(|(kind, _)| kind == "trigger")
            .unwrap()
            .1;
        copy.execute_batch(&format!("DROP TRIGGER \"{guard}\"; CREATE TRIGGER \"{guard}\" BEFORE INSERT ON snowflake_console_runs_v4 BEGIN SELECT 1; END;")).unwrap();
        drop(copy);
        let tampered = info(&path, &backup.id).unwrap();
        assert!(
            db.restore_metadata_backup(&tampered.id, &tampered.digest)
                .is_err()
        );
        assert_eq!(
            db.setting("live-refusal").unwrap().as_deref(),
            Some("preserved")
        );
        assert_eq!(
            db.console_rejection_by_client_request(&refused.client_request_id)
                .unwrap(),
            Some(refused)
        );
    }

    #[test]
    fn corrupt_newer_changed_and_traversal_backups_never_replace_live_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let db = Persistence::open(dir.path()).unwrap();
        db.set_setting("fixture", "retained").unwrap();
        let snapshot = db.create_metadata_backup().unwrap();
        assert!(
            db.restore_metadata_backup("../joinery", &snapshot.digest)
                .is_err()
        );
        assert!(db.restore_metadata_backup(&snapshot.id, "wrong").is_err());
        let path = db.backup_path(&snapshot.id).unwrap();
        let newer = Connection::open(&path).unwrap();
        newer
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(newer);
        let changed = info(&path, &snapshot.id).unwrap();
        assert!(
            db.restore_metadata_backup(&snapshot.id, &changed.digest)
                .is_err()
        );
        fs::write(&path, b"not SQLite").unwrap();
        let corrupt = info(&path, &snapshot.id).unwrap();
        assert!(
            db.restore_metadata_backup(&snapshot.id, &corrupt.digest)
                .is_err()
        );
        assert_eq!(db.setting("fixture").unwrap().as_deref(), Some("retained"));
        assert_eq!(db.metadata_backups().unwrap().len(), 1);
    }

    #[test]
    fn invalid_records_and_unexpected_triggers_are_rejected_before_restore() {
        let dir = tempfile::tempdir().unwrap();
        let db = Persistence::open(dir.path()).unwrap();
        let snapshot = db.create_metadata_backup().unwrap();
        let path = db.backup_path(&snapshot.id).unwrap();
        let edited = Connection::open(&path).unwrap();
        edited
            .execute("INSERT INTO profiles VALUES('bad','{}')", [])
            .unwrap();
        let changed = info(&path, &snapshot.id).unwrap();
        assert!(
            db.restore_metadata_backup(&snapshot.id, &changed.digest)
                .is_err()
        );
        edited.execute_batch("DELETE FROM profiles; CREATE TRIGGER surprise AFTER INSERT ON settings BEGIN DELETE FROM profiles; END;").unwrap();
        drop(edited);
        let changed = info(&path, &snapshot.id).unwrap();
        assert!(
            db.restore_metadata_backup(&snapshot.id, &changed.digest)
                .is_err()
        );
        assert_eq!(db.groups().unwrap().len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn links_are_rejected_without_following_or_deleting_their_targets() {
        let dir = tempfile::tempdir().unwrap();
        let db = Persistence::open(dir.path()).unwrap();
        let id = new_id();
        let target = dir.path().join("outside");
        fs::write(&target, b"untouched").unwrap();
        std::os::unix::fs::symlink(&target, db.backup_path(&id).unwrap()).unwrap();
        assert!(db.restore_metadata_backup(&id, "ignored").is_err());
        assert!(db.delete_metadata_backup(&id, "ignored").is_err());
        assert_eq!(fs::read(target).unwrap(), b"untouched");
    }
}
