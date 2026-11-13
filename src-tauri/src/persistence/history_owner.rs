//! SQL-free runtime ownership. Only a retained OS lease proves liveness/death.
use super::history::lifecycle;
use super::{HistoryCompletion, HistoryTicket, Persistence, db_error, schema_version};
use crate::domain::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub(super) const TABLE: &str = "history_owners_v6";
pub(super) const SCHEMA: &str =
    "CREATE TABLE history_owners_v6(ticket_id TEXT PRIMARY KEY NOT NULL,owner_nonce TEXT NOT NULL)";

fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "History ownership is busy or its local proof is unavailable or invalid; close other Joinery instances and retry",
    )
}
fn invalid() -> AppError {
    AppError::invalid("History ownership metadata is invalid")
}
fn canonical_nonce(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

/// Never unlink this file: its stable inode is the durable ownership identity.
pub(crate) struct HistoryOwner {
    nonce: String,
    lease: PrivateFile,
}

struct PrivateFile {
    file: File,
    root: PathBuf,
    root_file: File,
    directory: PathBuf,
    directory_file: File,
    name: String,
}

fn directory_file(path: &Path) -> AppResult<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x02000000 | 0x00200000);
    }
    let file = options.open(path).map_err(|_| conflict())?;
    validate_named(&file, path, true)?;
    Ok(file)
}

fn validate_named(file: &File, path: &Path, directory: bool) -> AppResult<()> {
    let named = fs::symlink_metadata(path).map_err(|_| conflict())?;
    let held = file.metadata().map_err(|_| conflict())?;
    if named.file_type().is_symlink()
        || named.is_dir() != directory
        || held.is_dir() != directory
        || (!directory && (!named.is_file() || !held.is_file() || held.len() != 0))
    {
        return Err(conflict());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if named.dev() != held.dev()
            || named.ino() != held.ino()
            || held.uid() != unsafe { libc::geteuid() }
            || held.mode() & 0o777 != if directory { 0o700 } else { 0o600 }
            || (!directory && held.nlink() != 1)
        {
            return Err(conflict());
        }
    }
    Ok(())
}

fn open_in(directory: &File, _path: &Path, name: &str, create: bool) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let name = std::ffi::CString::new(name)
            .map_err(|_| std::io::Error::other("invalid owner name"))?;
        let flags = libc::O_RDWR
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if create {
                libc::O_CREAT | libc::O_EXCL
            } else {
                0
            };
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(create);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        options.open(_path.join(name))
    }
}

impl PrivateFile {
    fn open(root: &Path, owners: bool, name: &str, create: bool) -> AppResult<Option<Self>> {
        let root_file = directory_file(root)?;
        let directory = if owners {
            root.join("history-owners")
        } else {
            root.to_owned()
        };
        if owners && create {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => {
                    #[cfg(unix)]
                    root_file.sync_all().map_err(|_| conflict())?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(conflict()),
            }
        }
        let directory_file = match directory_file(&directory) {
            Ok(file) => file,
            Err(_)
                if !create
                    && fs::symlink_metadata(&directory)
                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let file = match open_in(&directory_file, &directory, name, create) {
            Ok(file) => file,
            Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(_) => return Err(conflict()),
        };
        let value = Self {
            file,
            root: root.to_owned(),
            root_file,
            directory,
            directory_file,
            name: name.to_owned(),
        };
        value.validate(root)?;
        if create {
            value.file.sync_all().map_err(|_| conflict())?;
            #[cfg(unix)]
            value.directory_file.sync_all().map_err(|_| conflict())?;
        }
        Ok(Some(value))
    }
    fn validate(&self, root: &Path) -> AppResult<()> {
        if self.root != root {
            return Err(conflict());
        }
        validate_named(&self.root_file, root, true)?;
        validate_named(&self.directory_file, &self.directory, true)?;
        validate_named(&self.file, &self.directory.join(&self.name), false)
    }
    fn lock(&self, shared: bool) -> AppResult<bool> {
        let result = if shared {
            self.file.try_lock_shared()
        } else {
            self.file.try_lock()
        };
        match result {
            Ok(()) => {
                self.validate(&self.root)?;
                Ok(true)
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(false),
            Err(std::fs::TryLockError::Error(_)) => Err(conflict()),
        }
    }
}

/// Admission and startup recovery hold SH; restore holds EX through replacement.
pub(super) struct AdmissionGate {
    _lease: PrivateFile,
}
impl AdmissionGate {
    pub(super) fn claim(root: &Path, exclusive: bool) -> AppResult<Self> {
        let lease = match PrivateFile::open(root, false, "history-admission.lock", true) {
            Ok(Some(file)) => file,
            _ => PrivateFile::open(root, false, "history-admission.lock", false)?
                .ok_or_else(conflict)?,
        };
        // Every opener makes the stable gate durable before admitting work.
        lease.file.sync_all().map_err(|_| conflict())?;
        #[cfg(unix)]
        lease.directory_file.sync_all().map_err(|_| conflict())?;
        if !lease.lock(!exclusive)? {
            return Err(conflict());
        }
        Ok(Self { _lease: lease })
    }
}

pub(super) fn validate_records(connection: &Connection) -> AppResult<()> {
    if schema_version(connection)? < 6 {
        return Ok(());
    }
    let sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
            [TABLE],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if sql.as_deref() != Some(SCHEMA) {
        return Err(invalid());
    }
    let mut statement = connection.prepare("SELECT o.ticket_id,o.owner_nonce,t.id FROM history_owners_v6 o LEFT JOIN history_tickets_v5 t ON t.id=o.ticket_id ORDER BY o.ticket_id").map_err(db_error)?;
    let mut rows = statement.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let ticket: String = row.get(0).map_err(db_error)?;
        let nonce: String = row.get(1).map_err(db_error)?;
        let found: Option<String> = row.get(2).map_err(db_error)?;
        if ticket.is_empty()
            || ticket.len() > 256
            || ticket.contains('\0')
            || found.as_deref() != Some(ticket.as_str())
            || !canonical_nonce(&nonce)
        {
            return Err(invalid());
        }
    }
    Ok(())
}

/// A backup's mappings never grant authority, even if a named local file exists.
pub(super) fn merge_for_restore(current: &Connection, candidate: &Connection) -> AppResult<()> {
    candidate
        .execute("DELETE FROM history_owners_v6", [])
        .map_err(db_error)?;
    let mut statement = current
        .prepare("SELECT ticket_id,owner_nonce FROM history_owners_v6 ORDER BY ticket_id")
        .map_err(db_error)?;
    let mut rows = statement.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let nonce: String = row.get(1).map_err(db_error)?;
        if lifecycle::lookup(candidate, &id)?.is_none() {
            return Err(invalid());
        }
        candidate
            .execute(
                "INSERT INTO history_owners_v6 VALUES(?1,?2)",
                params![id, nonce],
            )
            .map_err(db_error)?;
    }
    Ok(())
}

fn recover_nonce(connection: &Connection, nonce: &str) -> AppResult<()> {
    // A keyset cursor remains deterministic while each recovered binding is removed.
    let mut after = String::new();
    loop {
        let id: Option<String> = connection.query_row(
            "SELECT ticket_id FROM history_owners_v6 WHERE owner_nonce=?1 AND ticket_id>?2 ORDER BY ticket_id LIMIT 1",
            params![nonce,after], |row| row.get(0),
        ).optional().map_err(db_error)?;
        let Some(id) = id else {
            break;
        };
        let (ticket, _, completion) = lifecycle::lookup(connection, &id)?.ok_or_else(invalid)?;
        if completion.is_none() {
            lifecycle::finish(
                connection,
                &ticket,
                &HistoryCompletion {
                    provenance: HistoryProvenance::Interrupted,
                    status: None,
                    elapsed_ms: None,
                    execution_summary: None,
                    snowflake_dml: None,
                    snowflake_ddl: None,
                },
            )?;
        }
        connection
            .execute(
                "DELETE FROM history_owners_v6 WHERE ticket_id=?1 AND owner_nonce=?2",
                params![id, nonce],
            )
            .map_err(db_error)?;
        after = id;
    }
    Ok(())
}

/// All returned handles remain locked until live replacement has completed.
pub(super) struct RestoreOwners {
    owners: Vec<(String, PrivateFile)>,
}
impl RestoreOwners {
    pub(super) fn prove(root: &Path, connection: &Connection) -> AppResult<Self> {
        let mut result = Self { owners: Vec::new() };
        visit_owners(connection, |nonce| {
            let lease = existing_owner(root, nonce)?.ok_or_else(conflict)?;
            if !lease.lock(false)? {
                return Err(conflict());
            }
            result.owners.push((nonce.to_owned(), lease));
            Ok(())
        })?;
        Ok(result)
    }
    pub(super) fn recover(&self, connection: &mut Connection) -> AppResult<()> {
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        for (nonce, lease) in &self.owners {
            lease.validate(&lease.root)?;
            recover_nonce(&tx, nonce)?;
        }
        tx.commit().map_err(db_error)
    }
}
fn existing_owner(root: &Path, nonce: &str) -> AppResult<Option<PrivateFile>> {
    if !canonical_nonce(nonce) {
        return Err(invalid());
    }
    PrivateFile::open(root, true, &format!("{nonce}.lock"), false)
}
fn visit_owners(
    connection: &Connection,
    mut visit: impl FnMut(&str) -> AppResult<()>,
) -> AppResult<()> {
    validate_records(connection)?;
    let mut after = String::new();
    loop {
        let nonce: Option<String> = connection.query_row(
            "SELECT owner_nonce FROM history_owners_v6 WHERE owner_nonce>?1 ORDER BY owner_nonce LIMIT 1",
            [&after], |row| row.get(0),
        ).optional().map_err(db_error)?;
        let Some(nonce) = nonce else {
            break;
        };
        visit(&nonce)?;
        after = nonce;
    }
    Ok(())
}

impl Persistence {
    pub(crate) fn claim_history_owner(&self) -> AppResult<HistoryOwner> {
        let nonce = new_id();
        let lease = PrivateFile::open(&self.root, true, &format!("{nonce}.lock"), true)?
            .ok_or_else(conflict)?;
        if !lease.lock(false)? {
            return Err(conflict());
        }
        let owner = HistoryOwner { nonce, lease };
        let _gate = AdmissionGate::claim(&self.root, false)?;
        let mut connection = self.connection.lock();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        visit_owners(&tx, |nonce| {
            let Some(lease) = existing_owner(&self.root, nonce)? else {
                return Ok(());
            };
            if lease.lock(false)? {
                recover_nonce(&tx, nonce)?;
            }
            Ok(())
        })?;
        tx.commit().map_err(db_error)?;
        Ok(owner)
    }
    pub(crate) fn begin_owned_history(
        &self,
        owner: &HistoryOwner,
        seed: &HistoryEntry,
    ) -> AppResult<HistoryTicket> {
        owner.lease.validate(&self.root)?;
        let _gate = AdmissionGate::claim(&self.root, false)?;
        let mut connection = self.connection.lock();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let binding: Option<String> = tx
            .query_row(
                "SELECT owner_nonce FROM history_owners_v6 WHERE ticket_id=?1",
                [&seed.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if binding.as_ref().is_some_and(|nonce| nonce != &owner.nonce)
            || (binding.is_none() && lifecycle::lookup(&tx, &seed.id)?.is_some())
        {
            return Err(conflict());
        }
        let ticket = lifecycle::begin(&tx, seed)?;
        tx.execute(
            "INSERT INTO history_owners_v6 VALUES(?1,?2) ON CONFLICT(ticket_id) DO NOTHING",
            params![ticket.id(), owner.nonce],
        )
        .map_err(db_error)?;
        owner.lease.validate(&self.root)?;
        tx.commit().map_err(db_error)?;
        Ok(ticket)
    }
    pub(crate) fn release_owned_history(
        &self,
        owner: &HistoryOwner,
        ticket: &HistoryTicket,
    ) -> AppResult<()> {
        owner.lease.validate(&self.root)?;
        let mut connection = self.connection.lock();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let (stored, _, completion) = lifecycle::lookup(&tx, ticket.id())?.ok_or_else(conflict)?;
        if stored != *ticket {
            return Err(conflict());
        }
        let binding: Option<String> = tx
            .query_row(
                "SELECT owner_nonce FROM history_owners_v6 WHERE ticket_id=?1",
                [ticket.id()],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(binding) = binding {
            if binding != owner.nonce || completion.is_none() {
                return Err(conflict());
            }
            tx.execute(
                "DELETE FROM history_owners_v6 WHERE ticket_id=?1 AND owner_nonce=?2",
                params![ticket.id(), owner.nonce],
            )
            .map_err(db_error)?;
        }
        tx.commit().map_err(db_error)
    }
}

#[cfg(test)]
#[path = "history_owner_tests.rs"]
mod tests;
