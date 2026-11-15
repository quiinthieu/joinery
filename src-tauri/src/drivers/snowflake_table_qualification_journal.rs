//! Private synchronous storage only. The owner calls this from spawn_blocking,
//! outside policy/process locks. A receipt records a successful local fsync; it
//! grants no execution authority. There is no reopen, resume, retry or cleanup.
//!
//! Directory-FD-relative no-follow opens and identity/length checks detect path
//! replacement and truncation. They do not establish immutable filesystem
//! identity or prevent another process running as the same user from rewriting
//! bytes in place between checks. Hashes are a chain, not an authentication MAC.

use crate::domain::{AppError, AppResult};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};

pub(super) const FILE_NAME: &str = "qualification-journal.jsonl";
const FILE_NAME_C: &std::ffi::CStr = c"qualification-journal.jsonl";
const FORMAT: &str = "joinery-snowflake-table-qualification-journal-v1";
const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RECORDS: u64 = 65_536; // Includes the sequence-zero header.
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 65_536;

fn invalid() -> AppError {
    AppError::invalid("Private qualification journal input or limit is invalid")
}

fn unavailable() -> AppError {
    AppError::internal("Private qualification journal persistence failed")
}

fn identity_changed() -> AppError {
    AppError::internal("Private qualification journal identity or permissions changed")
}

#[derive(Debug, Clone)]
pub(super) struct JournalReceipt {
    sequence: u64,
    sha256: String,
}

impl JournalReceipt {
    pub(super) fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(super) fn sha256(&self) -> &str {
        &self.sha256
    }
}

#[derive(Clone, Copy)]
struct Limits {
    record_bytes: usize,
    total_bytes: u64,
    records: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            record_bytes: MAX_RECORD_BYTES,
            total_bytes: MAX_TOTAL_BYTES,
            records: MAX_RECORDS,
        }
    }
}

#[cfg(test)]
enum Fault {
    PartialWrite(usize),
    Sync,
}

pub(super) struct Journal {
    directory_path: PathBuf,
    directory: File,
    file: File,
    directory_identity: (u64, u64),
    file_identity: (u64, u64),
    plan_sha256: String,
    sequence: u64,
    previous: String,
    bytes: u64,
    records: u64,
    limits: Limits,
    failure: Option<AppError>,
    #[cfg(test)]
    fault: Option<Fault>,
}

impl Journal {
    pub(super) fn create(directory: &Path, plan_sha256: &str) -> AppResult<Self> {
        if !directory.is_absolute()
            || directory
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            || plan_sha256.len() != 64
            || !plan_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid());
        }
        let before = fs::symlink_metadata(directory).map_err(|_| unavailable())?;
        private_directory(&before)?;
        let directory_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory)
            .map_err(|_| unavailable())?;
        let opened = directory_file.metadata().map_err(|_| unavailable())?;
        private_directory(&opened)?;
        let after = fs::symlink_metadata(directory).map_err(|_| unavailable())?;
        private_directory(&after)?;
        if identity(&before) != identity(&opened) || identity(&after) != identity(&opened) {
            return Err(identity_changed());
        }
        let file = open_relative(
            &directory_file,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_APPEND,
        )?;
        let metadata = file.metadata().map_err(|_| unavailable())?;
        private_file(&metadata)?;
        if metadata.len() != 0 {
            return Err(identity_changed());
        }
        let mut journal = Self {
            directory_path: directory.to_path_buf(),
            directory: directory_file,
            file,
            directory_identity: identity(&opened),
            file_identity: identity(&metadata),
            plan_sha256: plan_sha256.to_owned(),
            sequence: 0,
            previous: ZERO_HASH.into(),
            bytes: 0,
            records: 0,
            limits: Limits::default(),
            failure: None,
            #[cfg(test)]
            fault: None,
        };
        let header = json!({"format": FORMAT, "planSha256": plan_sha256, "type": "header"});
        journal.write_record(&header, 0)?;
        // The newly created name must also be durable before create succeeds.
        journal.directory.sync_all().map_err(|_| unavailable())?;
        journal.validate_identity(journal.bytes)?;
        Ok(journal)
    }

    /// Only the untouched, exclusively created header may be moved into a new
    /// policy. This checks the held file rather than reopening or resuming it.
    /// Call outside async/process/policy locks, like all journal filesystem IO.
    pub(super) fn validate_reserved(&self, plan_sha256: &str) -> AppResult<()> {
        if self.failure.is_some()
            || self.plan_sha256 != plan_sha256
            || self.sequence != 0
            || self.records != 1
        {
            return Err(invalid());
        }
        self.validate_identity(self.bytes)
    }

    pub(super) fn append(&mut self, event: &Value) -> AppResult<JournalReceipt> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let result = self
            .sequence
            .checked_add(1)
            .ok_or_else(invalid)
            .and_then(|sequence| self.write_record(event, sequence));
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }

    fn write_record(&mut self, event: &Value, sequence: u64) -> AppResult<JournalReceipt> {
        if self.records >= self.limits.records {
            return Err(invalid());
        }
        self.validate_identity(self.bytes)?;
        let (line, sha256) =
            encode_record(event, sequence, &self.previous, self.limits.record_bytes)?;
        let next_bytes = self
            .bytes
            .checked_add(line.len() as u64)
            .ok_or_else(invalid)?;
        if next_bytes > self.limits.total_bytes {
            return Err(invalid());
        }
        self.persist(&line).map_err(|_| unavailable())?;
        self.validate_identity(next_bytes)?;
        // Advance the chain only after the full write and fsync, with the named
        // file still identifying this private descriptor and expected length.
        self.sequence = sequence;
        self.previous = sha256.clone();
        self.bytes = next_bytes;
        self.records += 1;
        Ok(JournalReceipt { sequence, sha256 })
    }

    fn persist(&mut self, line: &[u8]) -> io::Result<()> {
        #[cfg(test)]
        if let Some(fault) = self.fault.take() {
            match fault {
                Fault::PartialWrite(length) => {
                    self.file.write_all(&line[..length.min(line.len())])?
                }
                Fault::Sync => self.file.write_all(line)?,
            }
            return Err(io::Error::other("injected journal persistence failure"));
        }
        self.file.write_all(line)?;
        self.file.sync_all()
    }

    fn validate_identity(&self, expected_bytes: u64) -> AppResult<()> {
        let opened = self.directory.metadata().map_err(|_| unavailable())?;
        let named = fs::symlink_metadata(&self.directory_path).map_err(|_| unavailable())?;
        private_directory(&opened)?;
        private_directory(&named)?;
        if identity(&opened) != self.directory_identity
            || identity(&named) != self.directory_identity
        {
            return Err(identity_changed());
        }
        let held = self.file.metadata().map_err(|_| unavailable())?;
        // No-follow, nonblocking relative open rejects replacement symlinks or
        // special files without following a caller-controlled replacement path.
        let named_file = open_relative(&self.directory, libc::O_RDONLY | libc::O_NONBLOCK)?;
        let named = named_file.metadata().map_err(|_| unavailable())?;
        for metadata in [&held, &named] {
            private_file(metadata)?;
            if identity(metadata) != self.file_identity || metadata.len() != expected_bytes {
                return Err(identity_changed());
            }
        }
        Ok(())
    }
}

fn identity(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

fn owner() -> u32 {
    // SAFETY: geteuid takes no pointers and has no failure/ownership contract.
    unsafe { libc::geteuid() }
}

fn private_directory(metadata: &Metadata) -> AppResult<()> {
    if !metadata.is_dir() || metadata.uid() != owner() || metadata.mode() & 0o7777 != 0o700 {
        return Err(identity_changed());
    }
    Ok(())
}

fn private_file(metadata: &Metadata) -> AppResult<()> {
    if !metadata.is_file()
        || metadata.uid() != owner()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(identity_changed());
    }
    Ok(())
}

fn open_relative(directory: &File, flags: libc::c_int) -> AppResult<File> {
    // SAFETY: the live directory owns its fd; the name is a static NUL-terminated
    // basename. A successful result transfers a fresh fd exactly once to File.
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            FILE_NAME_C.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600 as libc::c_uint,
        )
    };
    if descriptor < 0 {
        return Err(unavailable());
    }
    // SAFETY: only the successful, newly owned descriptor reaches this branch.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

// Canonical here means recursively sorted keys and compact serde_json lossless
// primitive encoding. This format does not claim RFC 8785/JCS normalization.
fn encode_record(
    event: &Value,
    sequence: u64,
    previous: &str,
    limit: usize,
) -> AppResult<(Vec<u8>, String)> {
    if event.as_object().is_none_or(|object| object.is_empty()) {
        return Err(invalid());
    }
    let mut output = BoundedBytes {
        bytes: Vec::new(),
        limit,
    };
    output.write_all(b"{\"event\":").map_err(|_| invalid())?;
    let mut nodes = 0;
    canonical(event, &mut output, 1, &mut nodes)?;
    write!(
        output,
        ",\"previous\":\"{previous}\",\"sequence\":{sequence}}}"
    )
    .map_err(|_| invalid())?;
    let sha256 = format!("{:x}", Sha256::digest(&output.bytes));
    output.bytes.pop();
    writeln!(output, ",\"sha256\":\"{sha256}\"}}").map_err(|_| invalid())?;
    Ok((output.bytes, sha256))
}

fn canonical(
    value: &Value,
    output: &mut BoundedBytes,
    depth: usize,
    nodes: &mut usize,
) -> AppResult<()> {
    *nodes = nodes.checked_add(1).ok_or_else(invalid)?;
    if depth > MAX_DEPTH || *nodes > MAX_NODES {
        return Err(invalid());
    }
    match value {
        Value::Object(object) => {
            if object.len() > MAX_NODES {
                return Err(invalid());
            }
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_unstable();
            output.write_all(b"{").map_err(|_| invalid())?;
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.write_all(b",").map_err(|_| invalid())?;
                }
                serde_json::to_writer(&mut *output, key).map_err(|_| invalid())?;
                output.write_all(b":").map_err(|_| invalid())?;
                canonical(&object[key], output, depth + 1, nodes)?;
            }
            output.write_all(b"}").map_err(|_| invalid())?;
        }
        Value::Array(array) => {
            if array.len() > MAX_NODES {
                return Err(invalid());
            }
            output.write_all(b"[").map_err(|_| invalid())?;
            for (index, value) in array.iter().enumerate() {
                if index != 0 {
                    output.write_all(b",").map_err(|_| invalid())?;
                }
                canonical(value, output, depth + 1, nodes)?;
            }
            output.write_all(b"]").map_err(|_| invalid())?;
        }
        _ => serde_json::to_writer(output, value).map_err(|_| invalid())?,
    }
    Ok(())
}

struct BoundedBytes {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|length| *length <= self.limit)
            .ok_or_else(|| io::Error::other("journal record limit"))?;
        if length > self.bytes.capacity() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(4096)
                .max(length)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| io::Error::other("journal allocation limit"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "snowflake_table_qualification_journal_tests.rs"]
mod tests;
