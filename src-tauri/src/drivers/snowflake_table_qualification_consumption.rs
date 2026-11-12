//! Offline, blocking one-use consumption. The owner calls outside async/policy
//! locks. A token proves these local marker fsyncs, never account authority.
//! Failed writes are not removed or resumed. Same-user hostile rewriting and
//! storage that lies about fsync are outside this filesystem identity check.

use crate::domain::{AppError, AppResult};
use serde_json::json;
use std::{
    ffi::CString,
    fs::{self, File, Metadata},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_PATH_BYTES: usize = 65_536;
const MAX_COMPONENTS: usize = 64;
const FORMAT: &str = "joinery-snowflake-table-lifecycle-consumption-v1";

fn invalid() -> AppError {
    AppError::invalid("Private table lifecycle consumption input is invalid")
}
fn unavailable() -> AppError {
    AppError::internal("Private table lifecycle consumption failed or was already used")
}

/// Linear local capability: only successful two-marker consumption constructs it.
/// Dropping it, including after launch failure, does not release either marker.
#[derive(Debug)]
pub(super) struct ConsumedAttempt {
    execution_sha256: String,
    run_key_sha256: String,
    proof: Arc<DirectoryProof>,
    run: File,
    plan: File,
    expected: Vec<u8>,
}
impl ConsumedAttempt {
    pub(super) fn revalidate(&self) -> AppResult<()> {
        self.proof.validate_marker(
            &Marker::Run.name(&self.run_key_sha256),
            &self.run,
            &self.expected,
        )?;
        self.proof.validate_marker(
            &Marker::Plan.name(&self.execution_sha256),
            &self.plan,
            &self.expected,
        )
    }
    pub(super) fn execution_sha256(&self) -> &str {
        &self.execution_sha256
    }
    pub(super) fn run_key_sha256(&self) -> &str {
        &self.run_key_sha256
    }
}

#[derive(Debug)]
pub(super) struct DirectoryProof {
    path: PathBuf,
    directory: File,
    identity: (u64, u64),
}

/// Fixed attempt storage names only; never a caller-selected relative path.
pub(super) enum AttemptDirectory<'a> {
    Run(&'a str),
    Workspace,
    HelperState,
}

pub(super) const TERMINAL_FILE: &str = "terminal.json";

pub(super) struct ConsumptionStore {
    proof: Arc<DirectoryProof>,
    #[cfg(test)]
    fault: std::sync::Mutex<Option<Fault>>,
    #[cfg(test)]
    hook: std::sync::Mutex<Option<Hook>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Marker {
    Run,
    Plan,
}
impl Marker {
    fn name(self, hash: &str) -> String {
        let kind = match self {
            Self::Run => "run",
            Self::Plan => "plan",
        };
        format!("table-lifecycle-consumed-{kind}-{hash}.json")
    }
}
#[cfg(test)]
#[derive(Clone, Copy)]
enum Stage {
    PartialWrite,
    FileSync,
    DirectorySync,
}
#[cfg(test)]
struct Fault {
    marker: Marker,
    stage: Stage,
}
#[cfg(test)]
type Hook = Box<dyn FnOnce(Marker, &Path) + Send>;

impl ConsumptionStore {
    /// Requires an existing absolute physical path: no component follows symlinks.
    pub(super) fn open(path: &Path) -> AppResult<Self> {
        let store = Self {
            proof: Arc::new(DirectoryProof::open(path)?),
            #[cfg(test)]
            fault: std::sync::Mutex::new(None),
            #[cfg(test)]
            hook: std::sync::Mutex::new(None),
        };
        store.proof.validate_directory()?;
        Ok(store)
    }

    /// Permanent markers cannot share an output ancestor or descendant. Both
    /// original path spellings must pass the strict physical-path checks first;
    /// canonicalization here only compares already validated directory locations.
    pub(super) fn require_separate_from(&self, output_parent: &Path) -> AppResult<()> {
        self.proof.validate_directory()?;
        let output = DirectoryProof::open(output_parent)?;
        let store_path = fs::canonicalize(self.proof.path()).map_err(|_| unavailable())?;
        let output_path = fs::canonicalize(output.path()).map_err(|_| unavailable())?;
        self.proof.validate_directory()?;
        output.validate_directory()?;
        if self.proof.identity == output.identity
            || output_path.starts_with(&store_path)
            || store_path.starts_with(&output_path)
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub(super) fn consume(
        &self,
        execution_sha256: &str,
        run_key_sha256: &str,
    ) -> AppResult<ConsumedAttempt> {
        if !valid_hash(execution_sha256) || !valid_hash(run_key_sha256) {
            return Err(invalid());
        }
        self.proof.validate_directory()?;
        let mut bytes = serde_json::to_vec(&json!({
            "format": FORMAT,
            "executionSha256": execution_sha256,
            "runKeySha256": run_key_sha256,
        }))
        .map_err(|_| invalid())?;
        bytes.push(b'\n');
        let run_name = Marker::Run.name(run_key_sha256);
        let plan_name = Marker::Plan.name(execution_sha256);
        // Ordering is intentional: a plan collision still permanently spends this run.
        let run = self.create_marker(Marker::Run, run_key_sha256, &bytes)?;
        let plan = self.create_marker(Marker::Plan, execution_sha256, &bytes)?;
        self.proof.validate_marker(&run_name, &run, &bytes)?;
        self.proof.validate_marker(&plan_name, &plan, &bytes)?;
        Ok(ConsumedAttempt {
            execution_sha256: execution_sha256.into(),
            run_key_sha256: run_key_sha256.into(),
            proof: self.proof.clone(),
            run,
            plan,
            expected: bytes,
        })
    }

    fn create_marker(&self, marker: Marker, hash: &str, bytes: &[u8]) -> AppResult<File> {
        self.proof.validate_directory()?;
        let name = marker.name(hash);
        let mut file = open_relative(
            &self.proof.directory,
            &name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
        )?;
        let created = file.metadata().map_err(|_| unavailable())?;
        private_file(&created)?;
        if created.len() != 0 {
            return Err(unavailable());
        }
        #[cfg(test)]
        let fault = {
            let mut pending = self.fault.lock().unwrap();
            if pending.as_ref().is_some_and(|fault| fault.marker == marker) {
                pending.take()
            } else {
                None
            }
        };
        #[cfg(test)]
        if let Some(fault) = fault {
            match fault.stage {
                Stage::PartialWrite => file.write_all(&bytes[..8]),
                Stage::FileSync | Stage::DirectorySync => file.write_all(bytes),
            }
            .map_err(|_| unavailable())?;
            if matches!(fault.stage, Stage::DirectorySync) {
                file.sync_all().map_err(|_| unavailable())?;
            }
            return Err(unavailable());
        }
        file.write_all(bytes).map_err(|_| unavailable())?;
        file.sync_all().map_err(|_| unavailable())?;
        self.proof.directory.sync_all().map_err(|_| unavailable())?;
        #[cfg(test)]
        if let Some(hook) = self.hook.lock().unwrap().take() {
            hook(marker, &self.proof.path);
        }
        self.proof.validate_marker(&name, &file, bytes)?;
        Ok(file)
    }
}

impl DirectoryProof {
    pub(super) fn open(path: &Path) -> AppResult<Self> {
        let directory = open_directory(path)?;
        let proof = Self {
            path: path.to_path_buf(),
            identity: identity(&directory.metadata().map_err(|_| unavailable())?),
            directory,
        };
        proof.validate_directory()?;
        Ok(proof)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn reserve_attempt_directory(&self, child: AttemptDirectory<'_>) -> AppResult<Self> {
        let name = match child {
            AttemptDirectory::Run(run) => {
                if run.len() != 32
                    || !run
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
                    || run.bytes().all(|b| b == b'0')
                {
                    return Err(invalid());
                }
                format!("attempt-{run}")
            }
            AttemptDirectory::Workspace => "workspace".into(),
            AttemptDirectory::HelperState => "helper-state".into(),
        };
        self.validate_directory()?;
        let name_c = CString::new(name.as_str()).map_err(|_| invalid())?;
        // SAFETY: the parent fd and fixed/generated basename are live. mkdirat
        // creates exclusively: existing names are never adopted or repaired.
        if unsafe { libc::mkdirat(self.directory.as_raw_fd(), name_c.as_ptr(), 0o700) } != 0 {
            return Err(unavailable());
        }
        let child = Self::open(&self.path.join(name))?;
        self.sync_directory()?;
        child.sync_directory()?;
        Ok(child)
    }

    pub(super) fn reserve_terminal(&self) -> AppResult<File> {
        self.validate_directory()?;
        let terminal = open_relative(
            &self.directory,
            TERMINAL_FILE,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
        )?;
        self.validate_terminal(&terminal, &[])?;
        terminal.sync_all().map_err(|_| unavailable())?;
        self.sync_directory()?;
        self.validate_terminal(&terminal, &[])?;
        Ok(terminal)
    }

    pub(super) fn validate_terminal(&self, held: &File, expected: &[u8]) -> AppResult<()> {
        self.validate_marker(TERMINAL_FILE, held, expected)
    }

    pub(super) fn sync_directory(&self) -> AppResult<()> {
        self.validate_directory()?;
        self.directory.sync_all().map_err(|_| unavailable())?;
        self.validate_directory()
    }

    pub(super) fn validate_directory(&self) -> AppResult<()> {
        let held = self.directory.metadata().map_err(|_| unavailable())?;
        private_directory(&held)?;
        let named = open_directory(&self.path)?;
        let named = named.metadata().map_err(|_| unavailable())?;
        if identity(&held) != self.identity || identity(&named) != self.identity {
            return Err(unavailable());
        }
        Ok(())
    }

    fn validate_marker(&self, name: &str, held: &File, expected: &[u8]) -> AppResult<()> {
        self.validate_directory()?;
        let held = held.metadata().map_err(|_| unavailable())?;
        private_file(&held)?;
        let mut named = open_relative(&self.directory, name, libc::O_RDONLY | libc::O_NONBLOCK)?;
        let metadata = named.metadata().map_err(|_| unavailable())?;
        private_file(&metadata)?;
        if identity(&held) != identity(&metadata)
            || held.len() != expected.len() as u64
            || metadata.len() != expected.len() as u64
        {
            return Err(unavailable());
        }
        let mut actual = Vec::with_capacity(expected.len() + 1);
        Read::by_ref(&mut named)
            .take(expected.len() as u64 + 1)
            .read_to_end(&mut actual)
            .map_err(|_| unavailable())?;
        if actual != expected {
            return Err(unavailable());
        }
        Ok(())
    }
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn identity(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}
fn owner() -> u32 {
    // SAFETY: geteuid has no pointer or ownership contract.
    unsafe { libc::geteuid() }
}
fn private_directory(metadata: &Metadata) -> AppResult<()> {
    if !metadata.is_dir() || metadata.uid() != owner() || metadata.mode() & 0o7777 != 0o700 {
        return Err(unavailable());
    }
    Ok(())
}
fn private_file(metadata: &Metadata) -> AppResult<()> {
    if !metadata.is_file()
        || metadata.uid() != owner()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(unavailable());
    }
    Ok(())
}
fn from_descriptor(descriptor: libc::c_int) -> AppResult<File> {
    if descriptor < 0 {
        return Err(unavailable());
    }
    // SAFETY: successful open/openat transfers a fresh owned descriptor once.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}
fn open_relative(directory: &File, name: &str, flags: libc::c_int) -> AppResult<File> {
    let name = CString::new(name).map_err(|_| invalid())?;
    // SAFETY: directory owns its live fd, name is NUL-terminated, and mode is
    // supplied for O_CREAT. The returned fd is transferred to File exactly once.
    from_descriptor(unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600 as libc::c_uint,
        )
    })
}
fn ancestor(metadata: &Metadata) -> AppResult<()> {
    let mode = metadata.mode();
    let root_sticky = metadata.uid() == 0 && mode & 0o7000 == 0o1000;
    if !metadata.is_dir()
        || (metadata.uid() != 0 && metadata.uid() != owner())
        || (mode & 0o7022 != 0 && !root_sticky)
    {
        return Err(unavailable());
    }
    Ok(())
}

fn open_directory(path: &Path) -> AppResult<File> {
    let raw = path.as_os_str().as_bytes();
    if !path.is_absolute() || raw.len() > MAX_PATH_BYTES || raw == b"/" || raw.contains(&0) {
        return Err(invalid());
    }
    // Validate original bytes: Path::components would normalize inner dots and
    // repeated separators, which are not accepted physical-path spellings.
    let parts: Vec<_> = raw[1..].split(|byte| *byte == b'/').collect();
    if parts.len() > MAX_COMPONENTS
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == b"." || *part == b"..")
    {
        return Err(invalid());
    }
    // SAFETY: root is a static NUL-terminated path; flags require a physical directory.
    let mut directory = from_descriptor(unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    })?;
    let mut edges = Vec::with_capacity(parts.len());
    for part in parts {
        // Each parent, including /, must protect its child names. Root-owned
        // sticky system temporary directories are the sole writable exception.
        ancestor(&directory.metadata().map_err(|_| unavailable())?)?;
        let name = CString::new(part).map_err(|_| invalid())?;
        // SAFETY: the parent fd is live and each basename is NUL-terminated.
        let child = from_descriptor(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        let child_identity = identity(&child.metadata().map_err(|_| unavailable())?);
        edges.push((directory, name, child_identity));
        directory = child;
    }
    private_directory(&directory.metadata().map_err(|_| unavailable())?)?;
    // Recheck held parents and their named children after the complete walk so
    // moving/replacing an earlier component cannot validate a detached old path.
    for (parent, name, expected) in edges {
        ancestor(&parent.metadata().map_err(|_| unavailable())?)?;
        // SAFETY: held parent and NUL-terminated basename remain live.
        let named = from_descriptor(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        if identity(&named.metadata().map_err(|_| unavailable())?) != expected {
            return Err(unavailable());
        }
    }
    Ok(directory)
}

#[cfg(test)]
#[path = "snowflake_table_qualification_consumption_tests.rs"]
mod tests;
