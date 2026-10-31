//! App-owned metadata snapshots. Paths never cross IPC; result buffers and OS secrets
//! are not part of this SQLite store. An incomplete SQLite backup rolls back on drop.
use super::{Persistence, SCHEMA_VERSION, db_error, schema_version, validate_schema};
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

fn validate(connection: &Connection) -> AppResult<()> {
    if schema_version(connection)? != SCHEMA_VERSION {
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
    let unexpected: i64 = connection.query_row("SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND (type != 'table' OR name NOT IN ('profiles','groups','history','audit','settings'))", [], |r| r.get(0)).map_err(db_error)?;
    if unexpected != 0 {
        return Err(AppError::invalid(
            "The metadata backup contains unexpected schema objects",
        ));
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
