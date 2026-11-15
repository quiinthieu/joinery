//! Encrypted metadata recovery. Published files are encrypted; private SQLite
//! staging and the live database remain plaintext. No path crosses this API.
use super::{
    Persistence, SCHEMA_VERSION, db_error, initialize_schema,
    recovery_crypto::{self, BackupPassphrase, MAX_CIPHERTEXT_BYTES, MAX_PAYLOAD_BYTES},
    schema_version,
    snowflake_console::{self, ConsoleJournalOwner},
};
use crate::domain::*;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension,
    backup::{Backup, StepResult},
    limits::Limit,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const MAX_BACKUPS: usize = 20;
const AGGREGATE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const HEADROOM_BYTES: u64 = 256 * 1024 * 1024;
const BUFFER_BYTES: usize = 64 * 1024;
const AGE_PREFIX: &[u8] = b"age-encryption.org/v1\n";
const SQLITE_PREFIX: &[u8] = b"SQLite format 3\0";
const PREFIX_BYTES: usize = if AGE_PREFIX.len() > SQLITE_PREFIX.len() {
    AGE_PREFIX.len()
} else {
    SQLITE_PREFIX.len()
};
// This bounds an individual SQLite row, not a complete metadata snapshot. Every
// row that fitted the old 64 MiB complete-file limit still fits this bound.
const CELL_BYTES: i32 = 64 * 1024 * 1024;
const STAGE_FILES: &[&str] = &[
    "snapshot.sqlite",
    "snapshot.sqlite-wal",
    "snapshot.sqlite-shm",
    "snapshot.sqlite-journal",
    "restored.sqlite",
    "restored.sqlite-wal",
    "restored.sqlite-shm",
    "restored.sqlite-journal",
    "encrypted.partial",
    "stage-owner",
];

#[derive(Clone)]
pub(crate) struct RecoveryControl {
    pub(crate) cancel: CancellationToken,
    pub(crate) progress: Arc<dyn Fn(RecoveryProgress) + Send + Sync>,
    pub(crate) published: Arc<dyn Fn(RecoveryBackup) + Send + Sync>,
}

impl RecoveryControl {
    fn checkpoint(&self) -> AppResult<()> {
        if self.cancel.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }
    fn report(&self, phase: RecoveryPhase, completed_bytes: u64, total_bytes: Option<u64>) {
        (self.progress)(RecoveryProgress {
            phase,
            completed_bytes,
            total_bytes,
        });
    }
    fn checked<T>(&self, result: AppResult<T>) -> AppResult<T> {
        self.checkpoint()?;
        result
    }
}

fn cancelled() -> AppError {
    AppError::new(
        ErrorCode::Cancelled,
        "Local metadata recovery was cancelled before replacement",
    )
}
fn invalid() -> AppError {
    AppError::invalid("The metadata backup is invalid, changed or incompatible")
}
fn io_error(_: io::Error) -> AppError {
    AppError::internal("Could not read, write or finalize local metadata recovery")
}
fn conflict(message: &str) -> AppError {
    AppError::new(ErrorCode::Conflict, message)
}

/// All directory traversal and ordinary file I/O is relative to pinned private
/// directory descriptors. SQLite also verifies these owned paths before/after
/// opening its own NOFOLLOW connection inside the private UUID directory.
struct Area {
    directory: File,
    path: PathBuf,
}

#[cfg(unix)]
impl Area {
    fn root(path: &Path) -> AppResult<Self> {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(io_error)?;
        let area = Self {
            directory,
            path: path.to_owned(),
        };
        area.validate()?;
        Ok(area)
    }
    fn child(&self, name: &str) -> AppResult<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};
        self.validate()?;
        let name = component(name)?;
        // SAFETY: names are one closed path component and descriptors are live.
        let result = unsafe { libc::mkdirat(self.directory.as_raw_fd(), name.as_ptr(), 0o700) };
        if result != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
            return Err(io_error(io::Error::last_os_error()));
        }
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        let area = Self {
            directory: unsafe { File::from_raw_fd(fd) },
            path: self.path.join(name.to_str().map_err(|_| invalid())?),
        };
        area.validate()?;
        self.directory.sync_all().map_err(io_error)?;
        Ok(area)
    }
    fn validate(&self) -> AppResult<()> {
        use std::os::unix::fs::MetadataExt;
        let held = self.directory.metadata().map_err(io_error)?;
        let named = fs::symlink_metadata(&self.path).map_err(io_error)?;
        if !held.is_dir()
            || !named.is_dir()
            || held.uid() != unsafe { libc::geteuid() }
            || held.mode() & 0o777 != 0o700
            || held.dev() != named.dev()
            || held.ino() != named.ino()
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn file(&self, name: &str, create: bool) -> AppResult<File> {
        self.file_with_links(name, create, 1)
    }
    fn file_with_links(&self, name: &str, create: bool, links: u64) -> AppResult<File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        self.validate()?;
        let name = component(name)?;
        let flags = if create {
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL
        } else {
            libc::O_RDONLY
        };
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        validate_file_links(&file, links)?;
        self.validate()?;
        Ok(file)
    }
    fn remove(&self, name: &str) -> AppResult<()> {
        use std::os::fd::AsRawFd;
        self.validate()?;
        let name = component(name)?;
        if unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        self.directory.sync_all().map_err(io_error)
    }
    fn publish(
        &self,
        source_area: &Area,
        partial: &str,
        final_name: &str,
        held: &File,
        on_linked: &mut dyn FnMut(),
    ) -> AppResult<()> {
        use std::os::fd::AsRawFd;
        self.validate()?;
        source_area.validate()?;
        same_file(held, &source_area.path.join(partial))?;
        let source = component(partial)?;
        let destination = component(final_name)?;
        // linkat publishes without ever replacing an existing opaque identity.
        if unsafe {
            libc::linkat(
                source_area.directory.as_raw_fd(),
                source.as_ptr(),
                self.directory.as_raw_fd(),
                destination.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(io_error(io::Error::last_os_error()));
        }
        // On uncertainty retain both names: never remove a foreign replacement.
        same_file_links(held, &self.path.join(final_name), 2).map_err(|_| {
            AppError::new(ErrorCode::OutcomeUnknown,
                "A backup name may have been published, but its identity could not be confirmed. Inspect retained backups before continuing")
        })?;
        // Record the observed matching artifact before any further fallible
        // cleanup or durability step. This is not a claim that directory sync
        // succeeded; the caller reports that failure separately.
        on_linked();
        same_file_links(held, &source_area.path.join(partial), 2)?;
        if unsafe { libc::unlinkat(source_area.directory.as_raw_fd(), source.as_ptr(), 0) } != 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        sync_publication(&source_area.directory)?;
        sync_publication(&self.directory)
    }
    fn free_bytes(&self) -> AppResult<u64> {
        use std::os::fd::AsRawFd;
        let mut value = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::fstatvfs(self.directory.as_raw_fd(), value.as_mut_ptr()) } != 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        let value = unsafe { value.assume_init() };
        u64::try_from(u128::from(value.f_bavail) * u128::from(value.f_frsize))
            .map_err(|_| invalid())
    }
}

#[cfg(not(unix))]
impl Area {
    fn root(_: &Path) -> AppResult<Self> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Private metadata recovery is unavailable on this platform",
        ))
    }
    fn child(&self, _: &str) -> AppResult<Self> {
        Err(invalid())
    }
    fn validate(&self) -> AppResult<()> {
        Err(invalid())
    }
    fn file(&self, _: &str, _: bool) -> AppResult<File> {
        Err(invalid())
    }
    fn file_with_links(&self, _: &str, _: bool, _: u64) -> AppResult<File> {
        Err(invalid())
    }
    fn remove(&self, _: &str) -> AppResult<()> {
        Err(invalid())
    }
    fn publish(&self, _: &Area, _: &str, _: &str, _: &File, _: &mut dyn FnMut()) -> AppResult<()> {
        Err(invalid())
    }
    fn free_bytes(&self) -> AppResult<u64> {
        Err(invalid())
    }
}

#[cfg(all(test, unix))]
thread_local! {
    static FAIL_PUBLICATION_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(unix)]
fn sync_publication(directory: &File) -> AppResult<()> {
    #[cfg(test)]
    if FAIL_PUBLICATION_SYNC.with(|fault| fault.replace(false)) {
        return Err(io_error(io::Error::other(
            "isolated publication sync fault",
        )));
    }
    directory.sync_all().map_err(io_error)
}

#[cfg(unix)]
fn component(value: &str) -> AppResult<std::ffi::CString> {
    if value.is_empty() || value.contains('/') || value == "." || value == ".." {
        return Err(invalid());
    }
    std::ffi::CString::new(value).map_err(|_| invalid())
}

fn validate_file_links(file: &File, links: u64) -> AppResult<()> {
    #[cfg(not(unix))]
    let _ = links;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() == 0
            || metadata.nlink() > links
        {
            return Err(invalid());
        }
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StageManifest {
    format: String,
    stage_id: String,
    files: Vec<StageIdentity>,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StageIdentity {
    name: String,
    device: u64,
    inode: u64,
}

fn identity(file: &File) -> AppResult<(u64, u64)> {
    let metadata = file.metadata().map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Err(invalid())
    }
}

struct Stage {
    #[cfg(unix)]
    parent: Area,
    area: Area,
    name: String,
    owned: parking_lot::Mutex<Vec<(String, File)>>,
}
impl Stage {
    fn new(root: &Path) -> AppResult<Self> {
        let parent = Area::root(root)?.child("recovery-staging")?;
        let name = new_id();
        let area = parent.child(&name)?;
        let marker = area.file("stage-owner", true)?;
        let stage = Self {
            #[cfg(unix)]
            parent,
            area,
            name,
            owned: parking_lot::Mutex::new(vec![("stage-owner".into(), marker)]),
        };
        stage.persist_inventory()?;
        Ok(stage)
    }
    fn file(&self, name: &str) -> AppResult<File> {
        if name == "stage-owner" || !STAGE_FILES.contains(&name) {
            return Err(invalid());
        }
        let file = self.area.file(name, true)?;
        self.owned
            .lock()
            .push((name.to_owned(), file.try_clone().map_err(io_error)?));
        self.persist_inventory()?;
        Ok(file)
    }
    fn persist_inventory(&self) -> AppResult<()> {
        let owned = self.owned.lock();
        let files = owned
            .iter()
            .map(|(name, file)| {
                let (device, inode) = identity(file)?;
                Ok(StageIdentity {
                    name: name.clone(),
                    device,
                    inode,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        let manifest = StageManifest {
            format: "joinery-recovery-stage-v2".into(),
            stage_id: self.name.clone(),
            files,
        };
        let bytes = serde_json::to_vec(&manifest).map_err(|_| invalid())?;
        if bytes.len() > 16384 {
            return Err(invalid());
        }
        let (_, marker) = owned
            .iter()
            .find(|(name, _)| name.as_str() == "stage-owner")
            .ok_or_else(invalid)?;
        same_file(marker, &self.area.path.join("stage-owner"))?;
        let mut marker = marker.try_clone().map_err(io_error)?;
        marker.rewind().map_err(io_error)?;
        marker.set_len(0).map_err(io_error)?;
        marker.write_all(&bytes).map_err(io_error)?;
        marker.sync_all().map_err(io_error)?;
        self.area.directory.sync_all().map_err(io_error)
    }
    fn sqlite(&self, name: &str) -> AppResult<(File, Connection)> {
        let file = self.file(name)?;
        self.area.validate()?;
        let connection = Connection::open_with_flags(
            self.area.path.join(name),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(db_error)?;
        self.area.validate()?;
        same_file(&file, &self.area.path.join(name))?;
        configure(&connection)?;
        Ok((file, connection))
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        // Remove only files in this operation's freshly created UUID directory.
        // Never recursively remove unknown files or someone else's staging.
        let owned = self.owned.get_mut();
        for (name, file) in owned
            .iter()
            .filter(|(name, _)| name.as_str() != "stage-owner")
        {
            if same_file_links(file, &self.area.path.join(name.as_str()), 2).is_ok() {
                let _ = self.area.remove(name);
            }
        }
        // Keep the ownership manifest until every captured file is removed.
        // A foreign replacement or unexpected sidecar keeps the marker too.
        let marker_only = fs::read_dir(&self.area.path).is_ok_and(|entries| {
            entries
                .into_iter()
                .all(|entry| entry.is_ok_and(|entry| entry.file_name() == "stage-owner"))
        });
        if marker_only
            && let Some((name, file)) = owned
                .iter()
                .find(|(name, _)| name.as_str() == "stage-owner")
            && same_file(file, &self.area.path.join(name)).is_ok()
        {
            let _ = self.area.remove(name);
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if self.parent.validate().is_ok() && self.area.validate().is_ok() {
                if let Ok(name) = component(&self.name) {
                    unsafe {
                        libc::unlinkat(
                            self.parent.directory.as_raw_fd(),
                            name.as_ptr(),
                            libc::AT_REMOVEDIR,
                        );
                    }
                }
                let _ = self.parent.directory.sync_all();
            }
        }
    }
}

fn cleanup_stages(parent: &Area, backups: &Area) -> AppResult<()> {
    #[cfg(not(unix))]
    let _ = backups;
    // The process-wide recovery lock is held by every caller of Stage::new.
    // Only a closed marker and the fixed private file inventory confer cleanup
    // authority; unfamiliar directories/files are left untouched.
    for entry in fs::read_dir(&parent.path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if uuid::Uuid::parse_str(name)
            .ok()
            .is_none_or(|id| id.to_string() != name)
        {
            continue;
        }
        let area = parent.child(name)?;
        let mut marker = match area.file("stage-owner", false) {
            Ok(file) => file,
            Err(_) => continue,
        };
        if marker.metadata().map_err(io_error)?.len() > 16384 {
            continue;
        }
        let mut text = Vec::new();
        Read::by_ref(&mut marker)
            .take(16385)
            .read_to_end(&mut text)
            .map_err(io_error)?;
        let Ok(manifest) = serde_json::from_slice::<StageManifest>(&text) else {
            continue;
        };
        if manifest.format != "joinery-recovery-stage-v2"
            || manifest.stage_id != name
            || manifest.files.is_empty()
            || manifest.files.len() > STAGE_FILES.len()
        {
            continue;
        }
        let mut names = std::collections::HashSet::new();
        if manifest.files.iter().any(|file| {
            !STAGE_FILES.contains(&file.name.as_str()) || !names.insert(file.name.as_str())
        }) {
            continue;
        }
        let marker_identity = identity(&marker)?;
        if !manifest
            .files
            .iter()
            .any(|file| file.name == "stage-owner" && (file.device, file.inode) == marker_identity)
        {
            continue;
        }
        let mut owned = true;
        let mut files = Vec::new();
        for file in fs::read_dir(&area.path).map_err(io_error)? {
            let file = file.map_err(io_error)?;
            let file = file.file_name();
            if !file
                .to_str()
                .is_some_and(|file| STAGE_FILES.contains(&file))
            {
                owned = false;
                break;
            }
            let file_name = file.to_str().ok_or_else(invalid)?;
            let held = area.file_with_links(file_name, false, 2)?;
            let actual = identity(&held)?;
            if !manifest
                .files
                .iter()
                .any(|record| record.name == file_name && (record.device, record.inode) == actual)
            {
                owned = false;
                break;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let metadata = held.metadata().map_err(io_error)?;
                if metadata.nlink() == 2 {
                    if file_name != "encrypted.partial" {
                        return Err(invalid());
                    }
                    let mut matched = false;
                    for final_file in fs::read_dir(&backups.path).map_err(io_error)? {
                        let final_file = final_file.map_err(io_error)?;
                        let name = final_file.file_name();
                        let Some((name, id)) = name
                            .to_str()
                            .and_then(|name| name.strip_suffix(".age").map(|id| (name, id)))
                        else {
                            continue;
                        };
                        if uuid::Uuid::parse_str(id)
                            .ok()
                            .is_none_or(|id2| id2.to_string() != id)
                        {
                            continue;
                        }
                        if same_file_links(&held, &backups.path.join(name), 2).is_ok() {
                            matched = true;
                            break;
                        }
                    }
                    if !matched {
                        return Err(invalid());
                    }
                }
            }
            files.push((file_name.to_owned(), held));
        }
        if owned {
            drop(Stage {
                #[cfg(unix)]
                parent: Area::root(&parent.path)?,
                area,
                name: name.to_owned(),
                owned: parking_lot::Mutex::new(files),
            });
        }
    }
    parent.validate()
}

fn same_file(file: &File, path: &Path) -> AppResult<()> {
    same_file_links(file, path, 1)
}
fn same_file_links(file: &File, path: &Path, links: u64) -> AppResult<()> {
    validate_file_links(file, links)?;
    let named = fs::symlink_metadata(path).map_err(io_error)?;
    let held = file.metadata().map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !named.is_file() || named.dev() != held.dev() || named.ino() != held.ino() {
            return Err(invalid());
        }
    }
    #[cfg(not(unix))]
    if !named.is_file() || named.len() != held.len() {
        return Err(invalid());
    }
    Ok(())
}

fn configure(connection: &Connection) -> AppResult<()> {
    connection
        .set_limit(Limit::SQLITE_LIMIT_LENGTH, CELL_BYTES)
        .map_err(db_error)?;
    connection
        .set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 1024 * 1024)
        .map_err(db_error)?;
    connection
        .busy_timeout(Duration::from_millis(100))
        .map_err(db_error)?;
    connection.execute_batch("PRAGMA synchronous=FULL; PRAGMA mmap_size=0; PRAGMA cache_size=-8192; PRAGMA temp_store=FILE; PRAGMA foreign_keys=ON;").map_err(db_error)?;
    let page_size = sqlite_page_size(connection)?;
    let max_pages = i64::try_from(MAX_PAYLOAD_BYTES / page_size).map_err(|_| invalid())?;
    connection
        .pragma_update(None, "max_page_count", max_pages)
        .map_err(db_error)
}

fn checked_page_size(raw: i64) -> AppResult<u64> {
    let value = u32::try_from(raw).map_err(|_| invalid())?;
    if !(512..=65536).contains(&value) || !value.is_power_of_two() {
        return Err(invalid());
    }
    Ok(u64::from(value))
}

fn sqlite_page_size(connection: &Connection) -> AppResult<u64> {
    let raw: i64 = connection
        .pragma_query_value(None, "page_size", |r| r.get(0))
        .map_err(db_error)?;
    checked_page_size(raw)
}

fn db_bytes(connection: &Connection) -> AppResult<u64> {
    let size: i64 = connection
        .query_row(
            "SELECT page_count*page_size FROM pragma_page_count(),pragma_page_size()",
            [],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    u64::try_from(size)
        .ok()
        .filter(|n| *n <= MAX_PAYLOAD_BYTES)
        .ok_or_else(invalid)
}

fn stepped_copy(
    source: &Connection,
    destination: &mut Connection,
    control: &RecoveryControl,
    phase: RecoveryPhase,
    cancellable: bool,
) -> AppResult<()> {
    let page_size = sqlite_page_size(source)?;
    control.report(phase, 0, Some(db_bytes(source)?));
    if cancellable {
        control.checkpoint()?;
    }
    let backup = Backup::new(source, destination).map_err(db_error)?;
    copy_steps(&backup, control, phase, cancellable, page_size)
}

fn copy_steps(
    backup: &Backup<'_, '_>,
    control: &RecoveryControl,
    phase: RecoveryPhase,
    cancellable: bool,
    page_size: u64,
) -> AppResult<()> {
    let mut busy_since = None;
    loop {
        if cancellable {
            control.checkpoint()?;
        }
        match backup.step(256).map_err(db_error)? {
            StepResult::Done => break,
            StepResult::More => busy_since = None,
            StepResult::Busy | StepResult::Locked => {
                if busy_since.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(5) {
                    return Err(conflict(
                        "Metadata is busy; close other Joinery instances and retry recovery",
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => return Err(invalid()),
        }
        let p = backup.progress();
        control.report(
            phase,
            (p.pagecount - p.remaining).max(0) as u64 * page_size,
            Some(p.pagecount.max(0) as u64 * page_size),
        );
    }
    // Dropping an unfinished SQLite backup rolls its destination transaction
    // back. Once live replacement starts, late cancellation is deliberately not
    // consulted: return the actual completed or rolled-back local operation.
    Ok(())
}

struct LiveWitness {
    connection: Connection,
    version: i64,
}
impl LiveWitness {
    fn new(path: &Path) -> AppResult<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(db_error)?;
        connection
            .busy_timeout(Duration::from_millis(100))
            .map_err(db_error)?;
        let version = connection
            .pragma_query_value(None, "data_version", |r| r.get(0))
            .map_err(db_error)?;
        Ok(Self {
            connection,
            version,
        })
    }
    fn unchanged(&self) -> AppResult<bool> {
        let version: i64 = self
            .connection
            .pragma_query_value(None, "data_version", |r| r.get(0))
            .map_err(db_error)?;
        Ok(version == self.version)
    }
}

fn replace_guarded(
    source: &Connection,
    destination: &mut Connection,
    witness: &LiveWitness,
    control: &RecoveryControl,
) -> AppResult<()> {
    let page_size = sqlite_page_size(source)?;
    let bytes = db_bytes(source)?;
    let backup = Backup::new(source, destination).map_err(db_error)?;
    let start = Instant::now();
    loop {
        // Bundled sqlite3_backup_step starts and retains the destination write
        // transaction before its page-copy loop, even for zero pages. Never
        // query destination while this handle exists: use the separate witness.
        match backup.step(0).map_err(db_error)? {
            StepResult::More => break,
            StepResult::Busy | StepResult::Locked if start.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                return Err(conflict(
                    "Metadata is busy; close other Joinery instances and retry recovery",
                ));
            }
        }
    }
    if !witness.unchanged()? {
        return Err(conflict(
            "Metadata changed during recovery; the live database was preserved. Review the changes and retry",
        ));
    }
    // Competing SQLite writers are now blocked until this exact backup commits
    // or drops. A late cancel does not interrupt atomic replacement settlement.
    control.report(RecoveryPhase::Replacing, 0, Some(bytes));
    copy_steps(&backup, control, RecoveryPhase::Replacing, false, page_size)
}

struct SqlCancellation<'a>(&'a Connection);
impl Drop for SqlCancellation<'_> {
    fn drop(&mut self) {
        // This guard only holds an owned connection; rusqlite's ownership check
        // cannot fail here. Drop still cannot propagate a reset error.
        let _ = self.0.progress_handler(0, None::<fn() -> bool>);
    }
}
fn sql_cancellation<'a>(
    connection: &'a Connection,
    control: &RecoveryControl,
) -> AppResult<SqlCancellation<'a>> {
    let token = control.cancel.clone();
    connection
        .progress_handler(10_000, Some(move || token.is_cancelled()))
        .map_err(db_error)?;
    Ok(SqlCancellation(connection))
}

fn validate(connection: &Connection, control: &RecoveryControl) -> AppResult<()> {
    control.checkpoint()?;
    control.report(RecoveryPhase::Validating, 0, None);
    let _cancel = sql_cancellation(connection, control)?;
    super::validate_schema(connection)?;
    // Reject oversized known JSON settings in SQLite before get::<String>()
    // allocates them. Other cells remain bounded by SQLITE_LIMIT_LENGTH.
    for (key, limit) in [
        ("workspace", 2 * 1024 * 1024 * 6 + 16384),
        ("saved-queries", 4 * 1024 * 1024 * 6 + 65536),
        ("bookmarks", 2 * 1024 * 1024),
        ("sql-format-settings", 4096),
        ("sql-snippets", 512 * 1024 * 6 + 65536),
    ] {
        let valid:bool=connection.query_row("SELECT NOT EXISTS(SELECT 1 FROM settings WHERE key=?1 AND (typeof(value)!='text' OR length(CAST(value AS BLOB))>?2))",rusqlite::params![key,limit],|r|r.get(0)).map_err(db_error)?;
        if !valid {
            return Err(invalid());
        }
    }
    // Run the existing version-specific, closed schema/record validator. Row
    // limits are installed before any JSON materializes in Rust.
    control.checked(super::recovery::validate(connection))
}

struct Transfer<'a, T> {
    inner: T,
    control: &'a RecoveryControl,
    phase: RecoveryPhase,
    bytes: u64,
    limit: u64,
}
impl<'a, T> Transfer<'a, T> {
    fn new(inner: T, control: &'a RecoveryControl, phase: RecoveryPhase, limit: u64) -> Self {
        Self {
            inner,
            control,
            phase,
            bytes: 0,
            limit,
        }
    }
}
impl<T: Read> Read for Transfer<'_, T> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        // age and io::copy retry Interrupted. Cancellation must leave their
        // loops instead; the outer operation translates the token to Cancelled.
        if self.control.cancel.is_cancelled() {
            return Err(io::Error::other("local recovery cancelled"));
        }
        let count = self.inner.read(buffer)?;
        self.bytes = self
            .bytes
            .checked_add(count as u64)
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "metadata recovery size limit")
            })?;
        self.control.report(self.phase, self.bytes, None);
        Ok(count)
    }
}
impl<T: Write> Write for Transfer<'_, T> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.control.cancel.is_cancelled() {
            return Err(io::Error::other("local recovery cancelled"));
        }
        if buffer.len() as u64 > self.limit.saturating_sub(self.bytes) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "metadata recovery size limit",
            ));
        }
        let count = self.inner.write(buffer)?;
        self.bytes += count as u64;
        self.control.report(self.phase, self.bytes, None);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl<T: Seek> Seek for Transfer<'_, T> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.bytes = 0;
        self.inner.seek(position)
    }
}

fn hash(file: &mut File, control: &RecoveryControl, limit: u64) -> AppResult<(u64, String)> {
    file.rewind().map_err(io_error)?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0; BUFFER_BYTES];
    loop {
        control.checkpoint()?;
        let n = file.read(&mut buffer).map_err(io_error)?;
        if n == 0 {
            break;
        }
        bytes = bytes
            .checked_add(n as u64)
            .filter(|n| *n <= limit)
            .ok_or_else(invalid)?;
        hash.update(&buffer[..n]);
    }
    file.rewind().map_err(io_error)?;
    Ok((bytes, hex::encode(hash.finalize())))
}

fn copy_hashed(
    source: &mut File,
    destination: &mut File,
    control: &RecoveryControl,
    phase: RecoveryPhase,
    limit: u64,
) -> AppResult<(u64, String)> {
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0; BUFFER_BYTES];
    loop {
        control.checkpoint()?;
        let count = source.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .filter(|n| *n <= limit)
            .ok_or_else(invalid)?;
        destination.write_all(&buffer[..count]).map_err(io_error)?;
        hash.update(&buffer[..count]);
        control.report(phase, bytes, None);
    }
    control.checkpoint()?;
    Ok((bytes, hex::encode(hash.finalize())))
}

fn backup_name(id: &str, format: RecoveryBackupFormat) -> AppResult<String> {
    if uuid::Uuid::parse_str(id)
        .ok()
        .is_none_or(|parsed| parsed.to_string() != id)
    {
        return Err(invalid());
    }
    Ok(format!(
        "{id}.{}",
        if matches!(format, RecoveryBackupFormat::EncryptedV1) {
            "age"
        } else {
            "sqlite"
        }
    ))
}

fn info(
    area: &Area,
    id: &str,
    format: RecoveryBackupFormat,
    control: &RecoveryControl,
) -> AppResult<RecoveryBackup> {
    let mut file = area.file(&backup_name(id, format)?, false)?;
    let metadata = file.metadata().map_err(io_error)?;
    let limit = if matches!(format, RecoveryBackupFormat::EncryptedV1) {
        MAX_CIPHERTEXT_BYTES
    } else {
        MAX_PAYLOAD_BYTES
    };
    if metadata.len() > limit {
        return Err(invalid());
    }
    let mut prefix = [0; PREFIX_BYTES];
    let required = if matches!(format, RecoveryBackupFormat::EncryptedV1) {
        AGE_PREFIX
    } else {
        SQLITE_PREFIX
    };
    file.read_exact(&mut prefix[..required.len()])
        .map_err(io_error)?;
    if &prefix[..required.len()] != required {
        return Err(invalid());
    }
    let (bytes, digest) = hash(&mut file, control, limit)?;
    Ok(RecoveryBackup {
        format,
        metadata: MetadataBackup {
            id: id.into(),
            created_at: metadata
                .modified()
                .ok()
                .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |v| v.as_millis() as u64),
            size_bytes: bytes,
            digest,
        },
    })
}

fn entries(area: &Area, control: &RecoveryControl) -> AppResult<Vec<RecoveryBackup>> {
    area.validate()?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&area.path).map_err(io_error)? {
        control.checkpoint()?;
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let (id, format) = if let Some(id) = name.strip_suffix(".age") {
            (id, RecoveryBackupFormat::EncryptedV1)
        } else if let Some(id) = name.strip_suffix(".sqlite") {
            (id, RecoveryBackupFormat::LegacySqlite)
        } else {
            continue;
        };
        if uuid::Uuid::parse_str(id)
            .ok()
            .is_none_or(|p| p.to_string() != id)
        {
            continue;
        }
        if entries.len() >= MAX_BACKUPS {
            return Err(conflict(
                "Too many metadata backups; explicitly delete a selected older copy before continuing",
            ));
        }
        if entries
            .iter()
            .any(|prior: &RecoveryBackup| prior.metadata.id == id)
        {
            return Err(invalid());
        }
        entries.push(info(area, id, format, control)?);
    }
    area.validate()?;
    entries.sort_by(|a, b| {
        b.metadata
            .created_at
            .cmp(&a.metadata.created_at)
            .then_with(|| a.metadata.id.cmp(&b.metadata.id))
    });
    Ok(entries)
}

fn find(
    area: &Area,
    id: &str,
    digest: &str,
    control: &RecoveryControl,
) -> AppResult<RecoveryBackup> {
    // Validate UUID before any path construction, then require exactly one format.
    backup_name(id, RecoveryBackupFormat::EncryptedV1)?;
    area.validate()?;
    let mut format = None;
    for candidate in [
        RecoveryBackupFormat::EncryptedV1,
        RecoveryBackupFormat::LegacySqlite,
    ] {
        match fs::symlink_metadata(area.path.join(backup_name(id, candidate)?)) {
            Ok(_) => {
                if format.replace(candidate).is_some() {
                    return Err(invalid());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    let selected = info(
        area,
        id,
        format.ok_or_else(|| AppError::new(ErrorCode::NotFound, "Metadata backup not found"))?,
        control,
    )?;
    if selected.metadata.digest != digest {
        return Err(conflict(
            "The backup changed; refresh its details before continuing",
        ));
    }
    Ok(selected)
}

fn reserve(
    area: &Area,
    control: &RecoveryControl,
    published_bytes: u64,
    work_bytes: u64,
) -> AppResult<()> {
    // Admission counts pinned file lengths rather than rehashing every existing
    // multi-GiB backup. Selected-source CAS still hashes actual streamed bytes.
    area.validate()?;
    let mut ids: Vec<String> = Vec::new();
    let mut total = 0u64;
    for entry in fs::read_dir(&area.path).map_err(io_error)? {
        control.checkpoint()?;
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some((id, limit)) = name
            .strip_suffix(".age")
            .map(|id| (id, MAX_CIPHERTEXT_BYTES))
            .or_else(|| {
                name.strip_suffix(".sqlite")
                    .map(|id| (id, MAX_PAYLOAD_BYTES))
            })
        else {
            continue;
        };
        if uuid::Uuid::parse_str(id)
            .ok()
            .is_none_or(|value| value.to_string() != id)
        {
            continue;
        }
        if ids.len() >= MAX_BACKUPS || ids.iter().any(|old| old.as_str() == id) {
            return Err(invalid());
        }
        let held = area.file(name, false)?;
        let length = held.metadata().map_err(io_error)?.len();
        if length > limit {
            return Err(invalid());
        }
        total = total.checked_add(length).ok_or_else(invalid)?;
        ids.push(id.to_owned());
    }
    area.validate()?;
    check_budget(
        ids.len(),
        total,
        published_bytes,
        area.free_bytes()?,
        work_bytes,
    )
}

fn check_budget(
    count: usize,
    total: u64,
    published: u64,
    available: u64,
    work: u64,
) -> AppResult<()> {
    if count >= MAX_BACKUPS
        || total
            .checked_add(published)
            .is_none_or(|n| n > AGGREGATE_BYTES)
    {
        return Err(conflict(
            "Metadata backup count or 64 GiB storage budget is exhausted; existing copies were preserved",
        ));
    }
    if available < work.checked_add(HEADROOM_BYTES).ok_or_else(invalid)? {
        return Err(conflict(
            "Not enough free space for private staging, encryption and mandatory undo; existing data was preserved",
        ));
    }
    Ok(())
}

fn recovery_lock(root: &Path) -> AppResult<File> {
    let area = Area::root(root)?;
    let name = "metadata-recovery.lock";
    let file = match area.file(name, true) {
        Ok(file) => file,
        Err(_) => area.file(name, false)?,
    };
    file.try_lock()
        .map_err(|_| conflict("Another local metadata recovery operation is active"))?;
    cleanup_stages(&area.child("recovery-staging")?, &area.child("backups")?)?;
    Ok(file)
}

impl Persistence {
    pub(crate) fn recovery_backups(&self) -> AppResult<Vec<RecoveryBackup>> {
        let _lock = recovery_lock(&self.root)?;
        let control = RecoveryControl {
            published: Arc::new(|_| {}),
            cancel: CancellationToken::new(),
            progress: Arc::new(|_| {}),
        };
        entries(&Area::root(&self.root)?.child("backups")?, &control)
    }
    pub(crate) fn delete_recovery_backup(&self, id: &str, digest: &str) -> AppResult<()> {
        let _lock = recovery_lock(&self.root)?;
        let control = RecoveryControl {
            published: Arc::new(|_| {}),
            cancel: CancellationToken::new(),
            progress: Arc::new(|_| {}),
        };
        let area = Area::root(&self.root)?.child("backups")?;
        let selected = find(&area, id, digest, &control)?;
        area.remove(&backup_name(id, selected.format)?)
    }
    pub(crate) fn create_encrypted_metadata_backup(
        &self,
        secret: &BackupPassphrase,
        control: &RecoveryControl,
    ) -> AppResult<RecoveryBackup> {
        let _lock = recovery_lock(&self.root)?;
        let area = Area::root(&self.root)?.child("backups")?;
        let size = db_bytes(&self.connection.lock())?;
        reserve(
            &area,
            control,
            size.saturating_add(16 * 1024 * 1024),
            size.saturating_mul(4),
        )?;
        let current = self.connection.lock();
        if db_bytes(&current)? > size {
            return Err(conflict("Metadata grew during staging; retry the backup"));
        }
        let stage = Stage::new(&self.root)?;
        let (mut file, mut snapshot) = stage.sqlite("snapshot.sqlite")?;
        stepped_copy(
            &current,
            &mut snapshot,
            control,
            RecoveryPhase::Staging,
            true,
        )?;
        drop(current);
        snapshot
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(db_error)?;
        validate(&snapshot, control)?;
        let version = schema_version(&snapshot)?;
        drop(snapshot);
        file.sync_all().map_err(io_error)?;
        publish_encrypted(
            &area,
            &stage,
            &mut file,
            version as u32,
            secret,
            control,
            RecoveryPhase::Encrypting,
            true,
        )
    }
    pub(crate) fn restore_encrypted_metadata_backup(
        &self,
        id: &str,
        digest: &str,
        decrypt: Option<&BackupPassphrase>,
        undo: &BackupPassphrase,
        owner: &ConsoleJournalOwner,
        control: &RecoveryControl,
    ) -> AppResult<RecoveryBackup> {
        owner.validate(self)?;
        let _history_gate = super::history_owner::AdmissionGate::claim(&self.root, true)?;
        let _lock = recovery_lock(&self.root)?;
        control.checkpoint()?;
        let area = Area::root(&self.root)?.child("backups")?;
        let selected = find(&area, id, digest, control)?;
        let live_size = db_bytes(&self.connection.lock())?;
        reserve(
            &area,
            control,
            live_size.saturating_add(16 * 1024 * 1024),
            live_size
                .saturating_mul(4)
                .saturating_add(selected.metadata.size_bytes.saturating_mul(3)),
        )?;
        let mut input = area.file(&backup_name(id, selected.format)?, false)?;
        let stage = Stage::new(&self.root)?;
        let (mut payload, mut candidate) = stage.sqlite("restored.sqlite")?;
        drop(candidate);
        payload.set_len(0).map_err(io_error)?;
        let manifest = if matches!(selected.format, RecoveryBackupFormat::EncryptedV1) {
            let secret = decrypt
                .ok_or_else(|| AppError::invalid("Provide this encrypted backup's passphrase"))?;
            let mut source = Transfer::new(
                &mut input,
                control,
                RecoveryPhase::Decrypting,
                MAX_CIPHERTEXT_BYTES,
            );
            let mut destination = Transfer::new(
                &mut payload,
                control,
                RecoveryPhase::Decrypting,
                MAX_PAYLOAD_BYTES,
            );
            let result = recovery_crypto::decrypt_snapshot(&mut source, secret, &mut destination);
            let result = control.checked(result)?;
            if result.ciphertext_sha256 != digest
                || result.ciphertext_bytes != selected.metadata.size_bytes
            {
                return Err(invalid());
            }
            Some(result.manifest)
        } else {
            let (bytes, actual) = copy_hashed(
                &mut input,
                &mut payload,
                control,
                RecoveryPhase::Staging,
                MAX_PAYLOAD_BYTES,
            )?;
            if actual != digest || bytes != selected.metadata.size_bytes {
                return Err(invalid());
            }
            None
        };
        payload.sync_all().map_err(io_error)?;
        same_file(&payload, &stage.area.path.join("restored.sqlite"))?;
        candidate = Connection::open_with_flags(
            stage.area.path.join("restored.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(db_error)?;
        configure(&candidate)?;
        if candidate
            .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
            .map_err(db_error)?
            != "delete"
        {
            return Err(invalid());
        }
        validate(&candidate, control)?;
        if let Some(manifest) = manifest
            && manifest.schema_version != schema_version(&candidate)? as u32
        {
            return Err(invalid());
        }
        initialize_schema(&mut candidate, None)?;
        let mut current = self.connection.lock();
        let history_owners = super::history_owner::RestoreOwners::prove(&self.root, &current)?;
        owner.validate(self)?;
        if !current.is_autocommit() {
            return Err(conflict(
                "Metadata is busy; retry recovery after local work settles",
            ));
        }
        let witness = LiveWitness::new(&self.root.join("joinery.sqlite"))?;
        let live = db_bytes(&current)?;
        let source = db_bytes(&candidate)?;
        reserve(
            &area,
            control,
            live.saturating_add(16 * 1024 * 1024),
            live.saturating_mul(4)
                .saturating_add(source.saturating_mul(3)),
        )?;
        control.report(RecoveryPhase::Merging, 0, None);
        {
            let _live_cancel = sql_cancellation(&current, control)?;
            let token = control.cancel.clone();
            candidate
                .progress_handler(10_000, Some(move || token.is_cancelled()))
                .map_err(db_error)?;
            // History privacy and SQL-free console evidence settle together in
            // the private candidate. Conflicts precede undo publication and
            // live replacement; neither original receipts nor backups grant
            // permission to restore previously cleared or opted-out SQL.
            let result = snowflake_console::merge_for_restore(&current, &mut candidate);
            let reset = candidate
                .progress_handler(0, None::<fn() -> bool>)
                .map_err(db_error);
            control.checked(result)?;
            reset?;
        }
        history_owners.recover(&mut candidate)?;
        db_bytes(&candidate)?;
        validate(&candidate, control)?;
        // Preserve current verifier configuration; OS credentials never enter a
        // SQLite snapshot. Decryption is not write authorization.
        let configured=current.query_row("SELECT CASE WHEN value IN ('true','false') THEN value ELSE NULL END FROM settings WHERE key='passphrase-configured'",[],|r|r.get::<_,String>(0)).optional().map_err(db_error)?.unwrap_or_else(||"false".into());
        candidate.execute("INSERT INTO settings(key,value) VALUES('passphrase-configured',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[configured]).map_err(db_error)?;
        let (mut undo_file, mut undo_db) = stage.sqlite("snapshot.sqlite")?;
        stepped_copy(
            &current,
            &mut undo_db,
            control,
            RecoveryPhase::CreatingUndo,
            true,
        )?;
        undo_db
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(db_error)?;
        validate(&undo_db, control)?;
        drop(undo_db);
        undo_file.sync_all().map_err(io_error)?;
        let undo = publish_encrypted(
            &area,
            &stage,
            &mut undo_file,
            SCHEMA_VERSION as u32,
            undo,
            control,
            RecoveryPhase::CreatingUndo,
            false,
        )?;
        control.checkpoint()?;
        owner.validate(self)?;
        control.report(RecoveryPhase::Replacing, 0, None);
        control.checkpoint()?;
        replace_guarded(&candidate, &mut current, &witness, control)?;
        control.report(
            RecoveryPhase::Finalizing,
            selected.metadata.size_bytes,
            Some(selected.metadata.size_bytes),
        );
        Ok(undo)
    }
    pub(crate) fn import_recovery_backup(
        &self,
        source: &mut File,
        control: &RecoveryControl,
    ) -> AppResult<RecoveryBackup> {
        let _lock = recovery_lock(&self.root)?;
        let area = Area::root(&self.root)?.child("backups")?;
        let metadata = source.metadata().map_err(io_error)?;
        if !metadata.is_file() || metadata.len() > MAX_CIPHERTEXT_BYTES {
            return Err(invalid());
        }
        reserve(
            &area,
            control,
            metadata.len(),
            metadata.len().saturating_mul(2),
        )?;
        source.rewind().map_err(io_error)?;
        let mut prefix = [0; AGE_PREFIX.len()];
        source.read_exact(&mut prefix).map_err(io_error)?;
        if prefix != AGE_PREFIX {
            return Err(invalid());
        }
        source.rewind().map_err(io_error)?;
        let id = new_id();
        let stage = Stage::new(&self.root)?;
        let mut output = stage.file("encrypted.partial")?;
        let (bytes, digest) = copy_hashed(
            source,
            &mut output,
            control,
            RecoveryPhase::Staging,
            MAX_CIPHERTEXT_BYTES,
        )?;
        reserve(&area, control, bytes, bytes)?;
        output.rewind().map_err(io_error)?;
        output.read_exact(&mut prefix).map_err(io_error)?;
        if prefix != AGE_PREFIX {
            return Err(invalid());
        }
        output.sync_all().map_err(io_error)?;
        let published = published_info(&output, &id, bytes, digest)?;
        control.report(RecoveryPhase::Finalizing, bytes, Some(bytes));
        control.checkpoint()?;
        area.publish(
            &stage.area,
            "encrypted.partial",
            &backup_name(&id, RecoveryBackupFormat::EncryptedV1)?,
            &output,
            &mut || (control.published)(published.clone()),
        )?;
        Ok(published)
    }
    pub(crate) fn export_recovery_backup(
        &self,
        id: &str,
        digest: &str,
        destination: &mut File,
        control: &RecoveryControl,
    ) -> AppResult<()> {
        let _lock = recovery_lock(&self.root)?;
        let area = Area::root(&self.root)?.child("backups")?;
        let selected = find(&area, id, digest, control)?;
        if !matches!(selected.format, RecoveryBackupFormat::EncryptedV1) {
            return Err(AppError::invalid(
                "Legacy plaintext backups must be restored and saved as a new encrypted copy before export",
            ));
        }
        let mut source = area.file(&backup_name(id, selected.format)?, false)?;
        if !destination.metadata().map_err(io_error)?.is_file() {
            return Err(invalid());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let input = source.metadata().map_err(io_error)?;
            let output = destination.metadata().map_err(io_error)?;
            if input.dev() == output.dev() && input.ino() == output.ino() {
                return Err(invalid());
            }
        }
        // Caller grants a private unpublished output handle, not an arbitrary
        // existing user path. A previous failed attempt must leave no suffix.
        destination.set_len(0).map_err(io_error)?;
        destination.rewind().map_err(io_error)?;
        let (bytes, actual) = copy_hashed(
            &mut source,
            destination,
            control,
            RecoveryPhase::Staging,
            MAX_CIPHERTEXT_BYTES,
        )?;
        if actual != digest || bytes != selected.metadata.size_bytes {
            return Err(invalid());
        }
        control.checked(destination.sync_all().map_err(io_error))
    }
}

fn published_info(
    file: &File,
    id: &str,
    size_bytes: u64,
    digest: String,
) -> AppResult<RecoveryBackup> {
    let metadata = file.metadata().map_err(io_error)?;
    Ok(RecoveryBackup {
        format: RecoveryBackupFormat::EncryptedV1,
        metadata: MetadataBackup {
            id: id.to_owned(),
            size_bytes,
            digest,
            created_at: metadata
                .modified()
                .ok()
                .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |v| v.as_millis() as u64),
        },
    })
}

// This helper's extra publication-mode argument keeps an undo publication
// cancellable until the separate live-replacement fence.
#[allow(
    clippy::too_many_arguments,
    reason = "Publication must distinguish a retained undo from the final artifact fence"
)]
fn publish_encrypted(
    area: &Area,
    stage: &Stage,
    source: &mut File,
    version: u32,
    secret: &BackupPassphrase,
    control: &RecoveryControl,
    phase: RecoveryPhase,
    final_publication: bool,
) -> AppResult<RecoveryBackup> {
    let id = new_id();
    let mut output = stage.file("encrypted.partial")?;
    source.rewind().map_err(io_error)?;
    let mut reader = Transfer::new(source, control, phase, MAX_PAYLOAD_BYTES);
    let mut writer = Transfer::new(&mut output, control, phase, MAX_CIPHERTEXT_BYTES);
    #[cfg(not(test))]
    let result = recovery_crypto::encrypt_snapshot(&mut reader, version, secret, &mut writer);
    #[cfg(test)]
    let result =
        recovery_crypto::encrypt_snapshot_fixture(&mut reader, version, secret, &mut writer);
    let encrypted = control.checked(result)?;
    output.sync_all().map_err(io_error)?;
    control.checkpoint()?;
    let published = published_info(
        &output,
        &id,
        encrypted.ciphertext_bytes,
        encrypted.ciphertext_sha256,
    )?;
    if final_publication {
        control.report(
            RecoveryPhase::Finalizing,
            published.metadata.size_bytes,
            Some(published.metadata.size_bytes),
        );
        control.checkpoint()?;
    }
    area.publish(
        &stage.area,
        "encrypted.partial",
        &backup_name(&id, RecoveryBackupFormat::EncryptedV1)?,
        &output,
        &mut || (control.published)(published.clone()),
    )?;
    Ok(published)
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::snowflake_console::{ConsoleAdmissionRejection, ConsoleRunMetadata};
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use zeroize::Zeroizing;

    fn control() -> RecoveryControl {
        RecoveryControl {
            published: Arc::new(|_| {}),
            cancel: CancellationToken::new(),
            progress: Arc::new(|_| {}),
        }
    }
    fn secret() -> BackupPassphrase {
        BackupPassphrase::new(Zeroizing::new("local fixture recovery passphrase".into())).unwrap()
    }
    fn setting(store: &Persistence) -> String {
        store.setting("recovery-fixture").unwrap().unwrap()
    }
    fn staged_files(root: &Path) -> usize {
        match fs::read_dir(root.join("recovery-staging")) {
            Ok(entries) => entries.count(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => panic!("{error}"),
        }
    }

    include!("recovery_history_tests.rs");

    #[test]
    fn encrypted_recovery_inventory_reads_complete_format_headers() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let area = Area::root(&store.root).unwrap().child("backups").unwrap();
        for (format, header) in [
            (
                RecoveryBackupFormat::EncryptedV1,
                b"age-encryption.org/v1\n".as_slice(),
            ),
            (
                RecoveryBackupFormat::LegacySqlite,
                b"SQLite format 3\0".as_slice(),
            ),
        ] {
            let id = new_id();
            let mut file = area.file(&backup_name(&id, format).unwrap(), true).unwrap();
            file.write_all(header).unwrap();
            file.write_all(b"inventory fixture").unwrap();
            file.sync_all().unwrap();
            let entry = info(&area, &id, format, &control()).unwrap();
            assert_eq!(
                entry.metadata.size_bytes,
                (header.len() + b"inventory fixture".len()) as u64
            );
            file.rewind().unwrap();
            file.write_all(b"X").unwrap();
            assert_eq!(
                info(&area, &id, format, &control()).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
    }

    #[test]
    fn encrypted_recovery_roundtrip_undo_verifier_and_pinned_transfer() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        store.set_setting("recovery-fixture", "before").unwrap();
        let first = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        assert_eq!(first.format, RecoveryBackupFormat::EncryptedV1);
        assert_eq!(
            fs::read(
                root.path()
                    .join("backups")
                    .join(format!("{}.age", first.metadata.id))
            )
            .unwrap()[..AGE_PREFIX.len()],
            *AGE_PREFIX
        );
        store.set_setting("recovery-fixture", "after").unwrap();
        store.set_setting("passphrase-configured", "true").unwrap();
        let owner = store.claim_console_archive_owner().unwrap();
        let undo = store
            .restore_encrypted_metadata_backup(
                &first.metadata.id,
                &first.metadata.digest,
                Some(&secret),
                &secret,
                &owner,
                &control(),
            )
            .unwrap();
        assert_eq!(setting(&store), "before");
        assert_eq!(
            store.setting("passphrase-configured").unwrap().as_deref(),
            Some("true")
        );
        assert_eq!(undo.format, RecoveryBackupFormat::EncryptedV1);
        store
            .restore_encrypted_metadata_backup(
                &undo.metadata.id,
                &undo.metadata.digest,
                Some(&secret),
                &secret,
                &owner,
                &control(),
            )
            .unwrap();
        assert_eq!(setting(&store), "after");
        let mut selected = tempfile::tempfile().unwrap();
        store
            .export_recovery_backup(
                &first.metadata.id,
                &first.metadata.digest,
                &mut selected,
                &control(),
            )
            .unwrap();
        let imported = store
            .import_recovery_backup(&mut selected, &control())
            .unwrap();
        assert_eq!(imported.metadata.digest, first.metadata.digest);
        assert_ne!(imported.metadata.id, first.metadata.id);
        assert_eq!(store.recovery_backups().unwrap().len(), 4);
        assert!(
            store
                .delete_recovery_backup(&imported.metadata.id, &"0".repeat(64))
                .is_err()
        );
        store
            .delete_recovery_backup(&imported.metadata.id, &imported.metadata.digest)
            .unwrap();
        assert_eq!(store.recovery_backups().unwrap().len(), 3);
        assert_eq!(staged_files(root.path()), 0);
    }

    #[test]
    fn encrypted_recovery_accepts_legacy_and_publishes_only_encrypted_undo() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        store.set_setting("recovery-fixture", "legacy").unwrap();
        let legacy = store.create_metadata_backup().unwrap();
        store.set_setting("recovery-fixture", "live").unwrap();
        let owner = store.claim_console_archive_owner().unwrap();
        let undo = store
            .restore_encrypted_metadata_backup(
                &legacy.id,
                &legacy.digest,
                Some(&secret),
                &secret,
                &owner,
                &control(),
            )
            .unwrap();
        assert_eq!(setting(&store), "legacy");
        assert_eq!(undo.format, RecoveryBackupFormat::EncryptedV1);
        let listed = store.recovery_backups().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed
                .iter()
                .filter(|b| b.format == RecoveryBackupFormat::LegacySqlite)
                .count(),
            1
        );
        let mut output = tempfile::tempfile().unwrap();
        assert!(
            store
                .export_recovery_backup(&legacy.id, &legacy.digest, &mut output, &control())
                .is_err()
        );
    }

    #[test]
    fn encrypted_recovery_wrong_key_tamper_truncation_and_cas_leave_live_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        store.set_setting("recovery-fixture", "before").unwrap();
        let backup = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        store.set_setting("recovery-fixture", "after").unwrap();
        let owner = store.claim_console_archive_owner().unwrap();
        let wrong =
            BackupPassphrase::new(Zeroizing::new("different fixture passphrase".into())).unwrap();
        assert!(
            store
                .restore_encrypted_metadata_backup(
                    &backup.metadata.id,
                    &backup.metadata.digest,
                    Some(&wrong),
                    &secret,
                    &owner,
                    &control()
                )
                .is_err()
        );
        assert!(
            store
                .restore_encrypted_metadata_backup(
                    &backup.metadata.id,
                    &"0".repeat(64),
                    Some(&secret),
                    &secret,
                    &owner,
                    &control()
                )
                .is_err()
        );
        let path = root
            .path()
            .join("backups")
            .join(format!("{}.age", backup.metadata.id));
        let original = fs::read(&path).unwrap();
        for bytes in [
            original[..original.len() - 1].to_vec(),
            [original.as_slice(), b"trailing"].concat(),
        ] {
            fs::write(&path, &bytes).unwrap();
            let changed = store.recovery_backups().unwrap().remove(0);
            assert!(
                store
                    .restore_encrypted_metadata_backup(
                        &changed.metadata.id,
                        &changed.metadata.digest,
                        Some(&secret),
                        &secret,
                        &owner,
                        &control()
                    )
                    .is_err()
            );
            assert_eq!(setting(&store), "after");
            assert_eq!(store.recovery_backups().unwrap().len(), 1);
            assert_eq!(staged_files(root.path()), 0);
        }
    }

    #[test]
    fn encrypted_recovery_cancellation_before_replacement_preserves_live_and_late_cancel_returns_actual_success()
     {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        store.set_setting("recovery-fixture", "before").unwrap();
        let backup = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        store.set_setting("recovery-fixture", "after").unwrap();
        let owner = store.claim_console_archive_owner().unwrap();
        for phase in [
            RecoveryPhase::Decrypting,
            RecoveryPhase::Validating,
            RecoveryPhase::Merging,
            RecoveryPhase::CreatingUndo,
            RecoveryPhase::Replacing,
        ] {
            let cancel = CancellationToken::new();
            let marker = cancel.clone();
            let ctl = RecoveryControl {
                published: Arc::new(|_| {}),
                cancel,
                progress: Arc::new(move |p| {
                    if p.phase == phase {
                        marker.cancel();
                    }
                }),
            };
            let error = store
                .restore_encrypted_metadata_backup(
                    &backup.metadata.id,
                    &backup.metadata.digest,
                    Some(&secret),
                    &secret,
                    &owner,
                    &ctl,
                )
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::Cancelled, "{phase:?}");
            assert_eq!(setting(&store), "after");
            assert_eq!(staged_files(root.path()), 0);
        }
        let cancel = CancellationToken::new();
        let marker = cancel.clone();
        let ctl = RecoveryControl {
            published: Arc::new(|_| {}),
            cancel,
            progress: Arc::new(move |p| {
                if p.phase == RecoveryPhase::Finalizing {
                    marker.cancel();
                }
            }),
        };
        store
            .restore_encrypted_metadata_backup(
                &backup.metadata.id,
                &backup.metadata.digest,
                Some(&secret),
                &secret,
                &owner,
                &ctl,
            )
            .unwrap();
        assert!(ctl.cancel.is_cancelled());
        assert_eq!(setting(&store), "before");
    }

    #[test]
    fn encrypted_recovery_create_cancellation_never_publishes_partial_backup() {
        for phase in [
            RecoveryPhase::Staging,
            RecoveryPhase::Validating,
            RecoveryPhase::Encrypting,
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = Persistence::open(root.path()).unwrap();
            let secret = secret();
            let cancel = CancellationToken::new();
            let marker = cancel.clone();
            let ctl = RecoveryControl {
                published: Arc::new(|_| {}),
                cancel,
                progress: Arc::new(move |p| {
                    if p.phase == phase {
                        marker.cancel();
                    }
                }),
            };
            assert_eq!(
                store
                    .create_encrypted_metadata_backup(&secret, &ctl)
                    .unwrap_err()
                    .code,
                ErrorCode::Cancelled
            );
            assert!(store.recovery_backups().unwrap().is_empty());
            assert_eq!(staged_files(root.path()), 0);
        }
    }

    #[test]
    fn encrypted_recovery_export_remains_cancellable_during_streaming() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        let backup = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        let cancel = CancellationToken::new();
        let marker = cancel.clone();
        let ctl = RecoveryControl {
            cancel,
            progress: Arc::new(move |p| {
                if p.phase == RecoveryPhase::Staging && p.completed_bytes > 0 {
                    marker.cancel();
                }
            }),
            published: Arc::new(|_| {}),
        };
        let mut output = tempfile::tempfile().unwrap();
        assert_eq!(
            store
                .export_recovery_backup(
                    &backup.metadata.id,
                    &backup.metadata.digest,
                    &mut output,
                    &ctl
                )
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        assert!(
            output.metadata().unwrap().len() > 0,
            "streaming callback ran after actual bytes were copied"
        );
        assert_eq!(store.recovery_backups().unwrap().len(), 1);
    }

    #[test]
    fn encrypted_recovery_final_publication_fence_honors_prior_cancel_and_reports_committed_artifacts()
     {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        let cancel = CancellationToken::new();
        let marker = cancel.clone();
        let ctl = RecoveryControl {
            cancel,
            progress: Arc::new(move |p| {
                if p.phase == RecoveryPhase::Finalizing {
                    marker.cancel();
                }
            }),
            published: Arc::new(|_| panic!("cancelled before publication")),
        };
        assert_eq!(
            store
                .create_encrypted_metadata_backup(&secret, &ctl)
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        assert!(store.recovery_backups().unwrap().is_empty());
        let cancel = CancellationToken::new();
        let marker = cancel.clone();
        let seen = Arc::new(parking_lot::Mutex::new(None));
        let recorded = seen.clone();
        let ctl = RecoveryControl {
            cancel,
            progress: Arc::new(|_| {}),
            published: Arc::new(move |backup| {
                *recorded.lock() = Some(backup);
                marker.cancel();
            }),
        };
        let backup = store
            .create_encrypted_metadata_backup(&secret, &ctl)
            .unwrap();
        assert!(ctl.cancel.is_cancelled());
        assert_eq!(
            seen.lock().as_ref().unwrap().metadata.id,
            backup.metadata.id
        );
        let mut input = File::open(
            root.path()
                .join("backups")
                .join(format!("{}.age", backup.metadata.id)),
        )
        .unwrap();
        let cancel = CancellationToken::new();
        let marker = cancel.clone();
        let imported = Arc::new(parking_lot::Mutex::new(None));
        let recorded = imported.clone();
        let ctl = RecoveryControl {
            cancel,
            progress: Arc::new(|_| {}),
            published: Arc::new(move |backup| {
                *recorded.lock() = Some(backup);
                marker.cancel();
            }),
        };
        let copy = store.import_recovery_backup(&mut input, &ctl).unwrap();
        assert!(ctl.cancel.is_cancelled());
        assert_eq!(
            imported.lock().as_ref().unwrap().metadata.id,
            copy.metadata.id
        );
        assert_eq!(copy.metadata.digest, backup.metadata.digest);
        assert_eq!(store.recovery_backups().unwrap().len(), 2);
    }

    #[test]
    fn encrypted_recovery_page_sizes_reject_negative_zero_non_page_and_overflow_values() {
        for invalid_size in [i64::MIN, -1, 0, 511, 513, 65537, i64::MAX] {
            assert!(checked_page_size(invalid_size).is_err());
        }
        assert_eq!(checked_page_size(512).unwrap(), 512);
        assert_eq!(checked_page_size(65536).unwrap(), 65536);
    }

    #[test]
    fn encrypted_recovery_postlink_sync_failure_retains_observed_backup_identity() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct ResetFault;
        impl Drop for ResetFault {
            fn drop(&mut self) {
                FAIL_PUBLICATION_SYNC.with(|fault| fault.set(false));
            }
        }
        let _reset = ResetFault;
        for operation in ["create", "import", "undo"] {
            let root = tempfile::tempdir().unwrap();
            let store = Persistence::open(root.path()).unwrap();
            let secret = secret();
            store.set_setting("recovery-fixture", "before").unwrap();
            let original = if operation == "create" {
                None
            } else {
                Some(
                    store
                        .create_encrypted_metadata_backup(&secret, &control())
                        .unwrap(),
                )
            };
            store.set_setting("recovery-fixture", "after").unwrap();
            let observed = Arc::new(parking_lot::Mutex::new(None));
            let recorded = observed.clone();
            let calls = Arc::new(AtomicUsize::new(0));
            let counted = calls.clone();
            let ctl = RecoveryControl {
                cancel: CancellationToken::new(),
                progress: Arc::new(|_| {}),
                published: Arc::new(move |backup| {
                    *recorded.lock() = Some(backup);
                    counted.fetch_add(1, Ordering::SeqCst);
                    // Set only after the target was observed, so the fault must be
                    // reached in the subsequent publication sync, never staging.
                    FAIL_PUBLICATION_SYNC.with(|fault| fault.set(true));
                }),
            };
            let error = match operation {
                "create" => store
                    .create_encrypted_metadata_backup(&secret, &ctl)
                    .unwrap_err(),
                "import" => {
                    let original = original.as_ref().unwrap();
                    let mut source = File::open(
                        root.path()
                            .join("backups")
                            .join(format!("{}.age", original.metadata.id)),
                    )
                    .unwrap();
                    store.import_recovery_backup(&mut source, &ctl).unwrap_err()
                }
                _ => {
                    let original = original.as_ref().unwrap();
                    let owner = store.claim_console_archive_owner().unwrap();
                    store
                        .restore_encrypted_metadata_backup(
                            &original.metadata.id,
                            &original.metadata.digest,
                            Some(&secret),
                            &secret,
                            &owner,
                            &ctl,
                        )
                        .unwrap_err()
                }
            };
            assert_eq!(error.code, ErrorCode::Internal, "{operation}");
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(
                !FAIL_PUBLICATION_SYNC.with(|fault| fault.get()),
                "post-link sync fault was consumed"
            );
            let observed = observed
                .lock()
                .take()
                .expect("matching linked artifact must be retained on error");
            let listed = store.recovery_backups().unwrap();
            assert_eq!(listed.len(), if operation == "create" { 1 } else { 2 });
            assert!(
                listed
                    .iter()
                    .any(|backup| backup.metadata.id == observed.metadata.id
                        && backup.metadata.digest == observed.metadata.digest)
            );
            assert_eq!(
                setting(&store),
                "after",
                "failed undo publication must not replace the live database"
            );
            assert_eq!(staged_files(root.path()), 0);
        }
    }

    #[test]
    fn encrypted_recovery_cancel_after_undo_reports_retained_undo_without_replacing_live() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        store.set_setting("recovery-fixture", "before").unwrap();
        let backup = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        store.set_setting("recovery-fixture", "after").unwrap();
        let owner = store.claim_console_archive_owner().unwrap();
        let cancel = CancellationToken::new();
        let marker = cancel.clone();
        let published = Arc::new(parking_lot::Mutex::new(None));
        let recorded = published.clone();
        let ctl = RecoveryControl {
            cancel,
            progress: Arc::new(|_| {}),
            published: Arc::new(move |backup| {
                *recorded.lock() = Some(backup);
                marker.cancel();
            }),
        };
        assert_eq!(
            store
                .restore_encrypted_metadata_backup(
                    &backup.metadata.id,
                    &backup.metadata.digest,
                    Some(&secret),
                    &secret,
                    &owner,
                    &ctl
                )
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        assert_eq!(setting(&store), "after");
        let listed = store.recovery_backups().unwrap();
        assert_eq!(listed.len(), 2);
        let undo = published.lock().take().unwrap();
        assert!(
            listed
                .iter()
                .any(|entry| entry.metadata.id == undo.metadata.id
                    && entry.metadata.digest == undo.metadata.digest)
        );
    }

    #[test]
    fn encrypted_recovery_private_stage_and_link_publication_cleanup_is_closed() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        // Match production's canonical path. SQLite NOFOLLOW rejects symlinked
        // ancestors too (including macOS's /var temporary-directory alias).
        let root_path = &store.root;
        let lock = recovery_lock(root_path).unwrap();
        let stage = Stage::new(root_path).unwrap();
        let name = stage.name.clone();
        stage.sqlite("snapshot.sqlite").unwrap();
        std::mem::forget(stage);
        drop(lock);
        let lock = recovery_lock(root_path).unwrap();
        let stage = Stage::new(root_path).unwrap();
        assert!(!root.path().join("recovery-staging").join(name).exists());
        drop(stage);
        let area = Area::root(root_path).unwrap().child("backups").unwrap();
        let id = new_id();
        let stage = Stage::new(root_path).unwrap();
        let stage_path = stage.area.path.clone();
        let mut partial = stage.file("encrypted.partial").unwrap();
        partial
            .write_all(b"age-encryption.org/v1\nfixture")
            .unwrap();
        partial.sync_all().unwrap();
        fs::hard_link(
            stage.area.path.join("encrypted.partial"),
            area.path.join(format!("{id}.age")),
        )
        .unwrap();
        drop(partial);
        std::mem::forget(stage);
        drop(lock);
        let unrelated = format!("{}.partial", new_id());
        fs::write(area.path.join(&unrelated), b"unfamiliar file").unwrap();
        let listed = store.recovery_backups().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!stage_path.exists());
        assert_eq!(
            fs::read(area.path.join(unrelated)).unwrap(),
            b"unfamiliar file"
        );
        let unknown = root.path().join("recovery-staging").join(new_id());
        fs::create_dir(&unknown).unwrap();
        fs::set_permissions(&unknown, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(unknown.join("user-file"), b"keep").unwrap();
        let _lock = recovery_lock(root_path).unwrap();
        drop(Stage::new(root_path).unwrap());
        assert_eq!(fs::read(unknown.join("user-file")).unwrap(), b"keep");
    }

    #[test]
    fn encrypted_recovery_rejects_unsafe_nodes_foreign_lease_and_oversized_known_json() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("backups")).unwrap();
        assert!(
            store
                .create_encrypted_metadata_backup(&secret, &control())
                .is_err()
        );
        assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
        fs::remove_file(root.path().join("backups")).unwrap();
        let backup = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        let foreign = Persistence::open(outside.path()).unwrap();
        let foreign_owner = foreign.claim_console_archive_owner().unwrap();
        assert!(
            store
                .restore_encrypted_metadata_backup(
                    &backup.metadata.id,
                    &backup.metadata.digest,
                    Some(&secret),
                    &secret,
                    &foreign_owner,
                    &control()
                )
                .is_err()
        );
        store
            .connection
            .lock()
            .execute(
                "INSERT INTO settings(key,value) VALUES('sql-format-settings',?1)",
                ["x".repeat(4097)],
            )
            .unwrap();
        assert!(
            store
                .create_encrypted_metadata_backup(&secret, &control())
                .is_err()
        );
        assert_eq!(store.recovery_backups().unwrap().len(), 1);
    }

    #[test]
    fn encrypted_recovery_rejects_fifo_and_preserves_replaced_stage_names() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let area = Area::root(root.path()).unwrap().child("backups").unwrap();
        let id = new_id();
        let path = area.path.join(format!("{id}.age"));
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        // O_NONBLOCK must reach regular-file validation rather than hang in open.
        assert!(store.recovery_backups().is_err());
        fs::remove_file(path).unwrap();
        let _lock = recovery_lock(root.path()).unwrap();
        let stage = Stage::new(root.path()).unwrap();
        let mut file = stage.file("encrypted.partial").unwrap();
        file.write_all(b"original").unwrap();
        file.sync_all().unwrap();
        let partial = stage.area.path.join("encrypted.partial");
        fs::remove_file(&partial).unwrap();
        fs::write(&partial, b"foreign replacement").unwrap();
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            area.publish(
                &stage.area,
                "encrypted.partial",
                &format!("{}.age", new_id()),
                &file,
                &mut || panic!("replaced source must not publish"),
            )
            .is_err()
        );
        let marker = stage.area.path.join("stage-owner");
        drop(stage);
        assert_eq!(fs::read(&partial).unwrap(), b"foreign replacement");
        assert!(
            marker.exists(),
            "foreign names keep marker until explicit inspection"
        );
        drop(_lock);
        store.recovery_backups().unwrap();
        assert_eq!(fs::read(partial).unwrap(), b"foreign replacement");
        assert!(
            marker.exists(),
            "persisted inode mismatch does not authorize crash cleanup"
        );
    }

    #[test]
    fn encrypted_recovery_other_connection_change_before_replace_is_not_overwritten() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        store.set_setting("recovery-fixture", "before").unwrap();
        let backup = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        store.set_setting("recovery-fixture", "after").unwrap();
        let owner = store.claim_console_archive_owner().unwrap();
        let path = root.path().join("joinery.sqlite");
        let changed = Arc::new(AtomicBool::new(false));
        let marker = changed.clone();
        let ctl = RecoveryControl {
            published: Arc::new(|_| {}),
            cancel: CancellationToken::new(),
            progress: Arc::new(move |p| {
                if p.phase == RecoveryPhase::CreatingUndo && !marker.swap(true, Ordering::SeqCst) {
                    let connection = Connection::open(&path).unwrap();
                    connection
                        .execute(
                            "UPDATE settings SET value='other writer' WHERE key='recovery-fixture'",
                            [],
                        )
                        .unwrap();
                }
            }),
        };
        let error = store
            .restore_encrypted_metadata_backup(
                &backup.metadata.id,
                &backup.metadata.digest,
                Some(&secret),
                &secret,
                &owner,
                &ctl,
            )
            .unwrap_err();
        assert!(changed.load(Ordering::SeqCst));
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(setting(&store), "other writer");
        assert_eq!(
            store.recovery_backups().unwrap().len(),
            2,
            "mandatory encrypted undo is retained when stale replacement rolls back"
        );
    }

    #[test]
    fn encrypted_recovery_authenticated_wrong_schema_and_invalid_sqlite_are_rejected_before_undo() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        store.set_setting("recovery-fixture", "live").unwrap();
        let plaintext = store.create_metadata_backup().unwrap();
        let path = root
            .path()
            .join("backups")
            .join(format!("{}.sqlite", plaintext.id));
        let owner = store.claim_console_archive_owner().unwrap();
        for malformed in [false, true] {
            if malformed {
                let sqlite = Connection::open(&path).unwrap();
                sqlite.execute("INSERT INTO settings(key,value) VALUES('sql-format-settings','not valid JSON')",[]).unwrap();
            }
            let mut source = File::open(&path).unwrap();
            let mut encrypted = tempfile::tempfile().unwrap();
            recovery_crypto::encrypt_snapshot_fixture(
                &mut source,
                if malformed { SCHEMA_VERSION as u32 } else { 1 },
                &secret,
                &mut encrypted,
            )
            .unwrap();
            let imported = store
                .import_recovery_backup(&mut encrypted, &control())
                .unwrap();
            assert!(
                store
                    .restore_encrypted_metadata_backup(
                        &imported.metadata.id,
                        &imported.metadata.digest,
                        Some(&secret),
                        &secret,
                        &owner,
                        &control()
                    )
                    .is_err()
            );
            assert_eq!(setting(&store), "live");
            assert_eq!(
                store.recovery_backups().unwrap().len(),
                if malformed { 3 } else { 2 }
            );
            assert_eq!(staged_files(root.path()), 0);
        }
    }

    #[test]
    fn encrypted_recovery_budgets_fail_before_publication_and_never_raise_count_caps() {
        assert!(check_budget(19, AGGREGATE_BYTES - 10, 10, HEADROOM_BYTES + 20, 20).is_ok());
        assert!(check_budget(20, 0, 0, u64::MAX, 0).is_err());
        assert!(check_budget(0, AGGREGATE_BYTES, 1, u64::MAX, 0).is_err());
        assert!(check_budget(0, 0, 0, HEADROOM_BYTES + 19, 20).is_err());
        assert!(check_budget(0, u64::MAX, 1, u64::MAX, 0).is_err());
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        let area = Area::root(root.path()).unwrap().child("backups").unwrap();
        for _ in 0..MAX_BACKUPS {
            let mut file = area.file(&format!("{}.age", new_id()), true).unwrap();
            file.write_all(AGE_PREFIX).unwrap();
        }
        assert_eq!(
            store
                .create_encrypted_metadata_backup(&secret, &control())
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(store.recovery_backups().unwrap().len(), MAX_BACKUPS);
        assert_eq!(staged_files(root.path()), 0);
    }

    #[test]
    fn encrypted_recovery_zero_page_destination_lock_blocks_other_sqlite_writer() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        store.set_setting("recovery-fixture", "before").unwrap();
        let backup = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        store.set_setting("recovery-fixture", "after").unwrap();
        let owner = store.claim_console_archive_owner().unwrap();
        let path = root.path().join("joinery.sqlite");
        let observed = Arc::new(AtomicBool::new(false));
        let marker = observed.clone();
        let ctl = RecoveryControl {
            published: Arc::new(|_| {}),
            cancel: CancellationToken::new(),
            progress: Arc::new(move |p| {
                if p.phase == RecoveryPhase::Replacing
                    && p.completed_bytes == 0
                    && p.total_bytes.is_some()
                    && !marker.swap(true, Ordering::SeqCst)
                {
                    let connection = Connection::open(&path).unwrap();
                    connection.busy_timeout(Duration::ZERO).unwrap();
                    let error=connection.execute("UPDATE settings SET value='must not enter' WHERE key='recovery-fixture'",[]).unwrap_err();
                    assert!(matches!(
                        error.sqlite_error_code(),
                        Some(
                            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                        )
                    ));
                }
            }),
        };
        store
            .restore_encrypted_metadata_backup(
                &backup.metadata.id,
                &backup.metadata.digest,
                Some(&secret),
                &secret,
                &owner,
                &ctl,
            )
            .unwrap();
        assert!(observed.load(Ordering::SeqCst));
        assert_eq!(setting(&store), "before");
        let connection = Connection::open(root.path().join("joinery.sqlite")).unwrap();
        connection
            .execute(
                "UPDATE settings SET value='after settlement' WHERE key='recovery-fixture'",
                [],
            )
            .unwrap();
        assert_eq!(setting(&store), "after settlement");
    }

    #[test]
    fn encrypted_recovery_preserves_positive_negative_union_and_refreshes_epoch() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        let owner = store.claim_console_archive_owner().unwrap();
        let refusal = ConsoleAdmissionRejection {
            client_request_id: new_id(),
            session_id: new_id(),
            plan_digest: "a".repeat(64),
            code: ErrorCode::ApprovalRequired,
            recorded_at: 1000,
        };
        store.console_record_rejection(&owner, &refusal).unwrap();
        let backup = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        let metadata = ConsoleRunMetadata {
            run_id: new_id(),
            client_request_id: new_id(),
            session_id: new_id(),
            connection_id: new_id(),
            plan_digest: "b".repeat(64),
            step_count: 1,
            created_at: 1001,
        };
        let original = store
            .console_prepare_run(&owner, metadata.clone())
            .unwrap()
            .run;
        let before = store
            .console_receipt_summary(&refusal.client_request_id)
            .unwrap()
            .unwrap()
            .inventory_epoch;
        // A failed restore must not seal this unrelated incomplete live intent.
        let wrong =
            BackupPassphrase::new(Zeroizing::new("wrong recovery fixture key".into())).unwrap();
        assert!(
            store
                .restore_encrypted_metadata_backup(
                    &backup.metadata.id,
                    &backup.metadata.digest,
                    Some(&wrong),
                    &secret,
                    &owner,
                    &control()
                )
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(store.console_run(&metadata.run_id).unwrap().unwrap()).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        store
            .restore_encrypted_metadata_backup(
                &backup.metadata.id,
                &backup.metadata.digest,
                Some(&secret),
                &secret,
                &owner,
                &control(),
            )
            .unwrap();
        assert_eq!(
            store
                .console_rejection_by_client_request(&refusal.client_request_id)
                .unwrap()
                .unwrap(),
            refusal
        );
        let restored = store.console_run(&metadata.run_id).unwrap().unwrap();
        assert_eq!(restored.metadata, original.metadata);
        assert!(restored.terminal.is_some());
        assert_ne!(
            store
                .console_receipt_summary(&refusal.client_request_id)
                .unwrap()
                .unwrap()
                .inventory_epoch,
            before
        );
    }

    #[test]
    fn encrypted_recovery_snapshot_larger_than_legacy_limit_uses_disk_staging() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let secret = secret();
        {
            let mut connection = store.connection.lock();
            let tx = connection.transaction().unwrap();
            for n in 0..1100 {
                tx.execute(
                    "INSERT INTO settings(key,value) VALUES(?1,?2)",
                    rusqlite::params![format!("large-fixture-{n}"), "x".repeat(64 * 1024)],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            assert!(db_bytes(&connection).unwrap() > 64 * 1024 * 1024);
        }
        assert!(store.create_metadata_backup().is_err());
        let backup = store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        assert!(backup.metadata.size_bytes > 64 * 1024 * 1024);
        store.set_setting("large-fixture-0", "changed").unwrap();
        let owner = store.claim_console_archive_owner().unwrap();
        store
            .restore_encrypted_metadata_backup(
                &backup.metadata.id,
                &backup.metadata.digest,
                Some(&secret),
                &secret,
                &owner,
                &control(),
            )
            .unwrap();
        assert_eq!(
            store.setting("large-fixture-0").unwrap().unwrap().len(),
            64 * 1024
        );
        assert_eq!(staged_files(root.path()), 0);
    }

    #[test]
    #[ignore = "internal isolated child for encrypted_recovery_killed_replacement_rolls_back_and_keeps_encrypted_undo"]
    fn encrypted_recovery_replacement_child() {
        let root =
            PathBuf::from(std::env::var_os("JOINERY_ENCRYPTED_RECOVERY_FIXTURE_ROOT").unwrap());
        let id = std::env::var("JOINERY_ENCRYPTED_RECOVERY_FIXTURE_ID").unwrap();
        let digest = std::env::var("JOINERY_ENCRYPTED_RECOVERY_FIXTURE_DIGEST").unwrap();
        let store = Persistence::open(&root).unwrap();
        let owner = store.claim_console_archive_owner().unwrap();
        let secret = secret();
        let marker = root.join("replacement-in-progress");
        let ctl = RecoveryControl {
            published: Arc::new(|_| {}),
            cancel: CancellationToken::new(),
            progress: Arc::new(move |p| {
                if p.phase == RecoveryPhase::Replacing && p.completed_bytes > 0 {
                    fs::write(&marker, b"partial SQLite backup transaction").unwrap();
                    loop {
                        std::thread::park();
                    }
                }
            }),
        };
        store
            .restore_encrypted_metadata_backup(&id, &digest, Some(&secret), &secret, &owner, &ctl)
            .unwrap();
        panic!("replacement must pause inside its live SQLite transaction");
    }

    #[test]
    fn encrypted_recovery_killed_replacement_rolls_back_and_keeps_encrypted_undo() {
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let root = tempfile::tempdir().unwrap();
        let secret = secret();
        let backup = {
            let store = Persistence::open(root.path()).unwrap();
            for n in 0..64 {
                store
                    .set_setting(&format!("copy-fixture-{n}"), &"a".repeat(64 * 1024))
                    .unwrap();
            }
            store.set_setting("recovery-fixture", "before").unwrap();
            let backup = store
                .create_encrypted_metadata_backup(&secret, &control())
                .unwrap();
            store.set_setting("recovery-fixture", "after").unwrap();
            backup
        };
        let mut child = Child(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "persistence::recovery_stream::tests::encrypted_recovery_replacement_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("JOINERY_ENCRYPTED_RECOVERY_FIXTURE_ROOT", root.path())
                .env("JOINERY_ENCRYPTED_RECOVERY_FIXTURE_ID", &backup.metadata.id)
                .env(
                    "JOINERY_ENCRYPTED_RECOVERY_FIXTURE_DIGEST",
                    &backup.metadata.digest,
                )
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(90);
        while !root.path().join("replacement-in-progress").exists() {
            if let Some(status) = child.0.try_wait().unwrap() {
                let mut diagnostic = String::new();
                child
                    .0
                    .stderr
                    .take()
                    .unwrap()
                    .take(16384)
                    .read_to_string(&mut diagnostic)
                    .unwrap();
                panic!(
                    "isolated replacement exited before entering partial backup: {status}: {diagnostic}"
                );
            }
            assert!(
                Instant::now() < deadline,
                "isolated replacement did not enter partial backup"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(
            staged_files(root.path()) > 0,
            "crash left a recognized private stage before recovery cleanup"
        );
        let store = Persistence::open(root.path()).unwrap();
        assert_eq!(setting(&store), "after");
        // Listing acquires the recovery lock and cleans recognized abandoned
        // private staging, without changing either published encrypted backup.
        let listed = store.recovery_backups().unwrap();
        assert_eq!(listed.len(), 2);
        assert!(
            listed
                .iter()
                .all(|b| b.format == RecoveryBackupFormat::EncryptedV1)
        );
        assert_eq!(staged_files(root.path()), 0);
        // A subsequent create remains usable after crash cleanup.
        store
            .create_encrypted_metadata_backup(&secret, &control())
            .unwrap();
        assert_eq!(staged_files(root.path()), 0);
        assert_eq!(setting(&store), "after");
    }
}
