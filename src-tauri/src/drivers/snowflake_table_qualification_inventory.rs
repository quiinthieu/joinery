//! Offline, complete content inventory; this grants no execution authority.
//!
//! Every input component is opened relative to a held directory with no-follow
//! semantics. Included nodes must belong to the effective user and must not be
//! writable by group/other or carry special permission bits. Ancestors may belong
//! to root or the effective user; root-owned sticky directories are the sole
//! writable/special ancestor exception, to permit physical system temp paths.
//! Such ancestors do not provide exclusive ownership of their child names.
//!
//! Named/opened identities, metadata and complete directory names are rechecked.
//! These checks detect observed replacement and mutation, but do not make the
//! filesystem immutable against the same user between checks or after return.
//! Verification is a new complete capture, not an authenticated/immutable handle.
use crate::domain::{AppError, AppResult};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString},
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
};

const FORMAT: &str = "joinery-qualification-inventory-v1";
const MAX_DEPTH: usize = 64; // Relative components below the included root.
const MAX_ENTRIES: usize = 32_768; // Includes root and empty directories.
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 65_536;
const MAX_PRIVATE_FILE_BYTES: usize = 64 * 1024;
const MAX_PRIVATE_ENVELOPE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum InventoryKind {
    Tree,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    // Lossless relative Unix path bytes, sorted before encoding; root is empty.
    path_hex: String,
    kind: InventoryKind,
    mode: libc::mode_t,
    // Directory sizes are not portable content; directories have length zero.
    length: u64,
    sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Inventory {
    format: &'static str,
    kind: InventoryKind,
    entries: Vec<Entry>,
}

#[derive(Clone, Copy)]
struct Limits {
    depth: usize,
    entries: usize,
    total_bytes: u64,
    file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            depth: MAX_DEPTH,
            entries: MAX_ENTRIES,
            total_bytes: MAX_TOTAL_BYTES,
            file_bytes: MAX_FILE_BYTES,
        }
    }
}

fn invalid() -> AppError {
    AppError::invalid("Qualification inventory input, ownership, permissions or limit is invalid")
}

fn unavailable() -> AppError {
    AppError::internal("Qualification inventory could not be captured")
}

fn changed() -> AppError {
    AppError::internal("Qualification inventory changed during capture or verification")
}

// Native types avoid narrowing filesystem identities across Unix platforms.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    device: libc::dev_t,
    inode: libc::ino_t,
    mode: libc::mode_t,
    uid: libc::uid_t,
    gid: libc::gid_t,
    links: libc::nlink_t,
    length: libc::off_t,
    modified: libc::time_t,
    modified_nsec: libc::c_long,
    changed: libc::time_t,
    changed_nsec: libc::c_long,
}

impl Stamp {
    fn new(value: &libc::stat) -> Self {
        Self {
            device: value.st_dev,
            inode: value.st_ino,
            mode: value.st_mode,
            uid: value.st_uid,
            gid: value.st_gid,
            links: value.st_nlink,
            length: value.st_size,
            modified: value.st_mtime,
            modified_nsec: value.st_mtime_nsec,
            changed: value.st_ctime,
            changed_nsec: value.st_ctime_nsec,
        }
    }

    fn kind(self) -> AppResult<InventoryKind> {
        match self.mode & libc::S_IFMT {
            libc::S_IFDIR => Ok(InventoryKind::Tree),
            libc::S_IFREG => Ok(InventoryKind::File),
            _ => Err(invalid()),
        }
    }

    fn same_anchor(self, other: Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.mode == other.mode
            && self.uid == other.uid
            && self.gid == other.gid
    }
}

fn held(file: &File) -> AppResult<Stamp> {
    // fstat initializes the full stat value on success, without resolving a path.
    let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(file.as_raw_fd(), value.as_mut_ptr()) } != 0 {
        return Err(unavailable());
    }
    Ok(Stamp::new(&unsafe { value.assume_init() }))
}

fn named(parent: &File, name: &CStr) -> AppResult<Stamp> {
    let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
    // No-follow stat rejects links by their own mode before opening a child.
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            value.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(unavailable());
    }
    Ok(Stamp::new(&unsafe { value.assume_init() }))
}

fn open_child(parent: &File, name: &CStr, kind: InventoryKind) -> AppResult<File> {
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | if kind == InventoryKind::Tree {
            libc::O_DIRECTORY
        } else {
            0
        };
    // O_NONBLOCK prevents a concurrently substituted FIFO from blocking an open.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(unavailable());
    }
    // This successful open transfers its sole descriptor ownership to File.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn included(value: Stamp, euid: libc::uid_t) -> AppResult<InventoryKind> {
    let kind = value.kind()?;
    if value.uid != euid
        || value.mode & 0o7022 != 0
        || kind == InventoryKind::File && value.links != 1
    {
        return Err(invalid());
    }
    Ok(kind)
}

fn ancestor(value: Stamp, euid: libc::uid_t) -> AppResult<()> {
    let root_sticky = value.uid == 0 && value.mode & 0o7000 == 0o1000;
    if value.kind()? != InventoryKind::Tree
        || value.uid != 0 && value.uid != euid
        || value.mode & 0o7022 != 0 && !root_sticky
    {
        return Err(invalid());
    }
    Ok(())
}

struct Anchor {
    parent: File,
    parent_stamp: Stamp,
    name: CString,
    child_stamp: Stamp,
}

struct Root {
    anchors: Vec<Anchor>,
    parent: File,
    parent_stamp: Stamp,
    name: CString,
    file: File,
    stamp: Stamp,
}

impl Root {
    fn open(path: &Path, kind: InventoryKind, euid: libc::uid_t) -> AppResult<Self> {
        let raw = path.as_os_str().as_bytes();
        if !path.is_absolute() || raw.len() > MAX_PATH_BYTES || raw == b"/" {
            return Err(invalid());
        }
        // Path::components normalizes inner dots; inspect original bytes instead.
        let parts = raw[1..].split(|byte| *byte == b'/').collect::<Vec<_>>();
        // Bound held ancestor descriptors independently of subtree recursion.
        if parts.len() > MAX_DEPTH
            || parts
                .iter()
                .any(|part| part.is_empty() || *part == b"." || *part == b"..")
        {
            return Err(invalid());
        }
        let fd = unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(unavailable());
        }
        let mut current = unsafe { File::from_raw_fd(fd) };
        let mut anchors = Vec::new();
        let Some((last, parents)) = parts.split_last() else {
            return Err(invalid());
        };
        for part in parents {
            let parent_stamp = held(&current)?;
            ancestor(parent_stamp, euid)?;
            let name = CString::new(*part).map_err(|_| invalid())?;
            let before = named(&current, &name)?;
            ancestor(before, euid)?;
            let next = open_child(&current, &name, InventoryKind::Tree)?;
            let opened = held(&next)?;
            if !before.same_anchor(opened) {
                return Err(changed());
            }
            ancestor(opened, euid)?;
            anchors.push(Anchor {
                parent: current,
                parent_stamp,
                name,
                child_stamp: opened,
            });
            current = next;
        }
        let parent_stamp = held(&current)?;
        ancestor(parent_stamp, euid)?;
        let name = CString::new(*last).map_err(|_| invalid())?;
        let stamp = named(&current, &name)?;
        if included(stamp, euid)? != kind {
            return Err(invalid());
        }
        let file = open_child(&current, &name, kind)?;
        if held(&file)? != stamp {
            return Err(changed());
        }
        Ok(Self {
            anchors,
            parent: current,
            parent_stamp,
            name,
            file,
            stamp,
        })
    }

    fn recheck(&self, euid: libc::uid_t) -> AppResult<()> {
        if held(&self.file)? != self.stamp || named(&self.parent, &self.name)? != self.stamp {
            return Err(changed());
        }
        let parent = held(&self.parent)?;
        ancestor(parent, euid)?;
        if !self.parent_stamp.same_anchor(parent) {
            return Err(changed());
        }
        for anchor in self.anchors.iter().rev() {
            let parent = held(&anchor.parent)?;
            let child = named(&anchor.parent, &anchor.name)?;
            ancestor(parent, euid)?;
            ancestor(child, euid)?;
            if !anchor.parent_stamp.same_anchor(parent) || !anchor.child_stamp.same_anchor(child) {
                return Err(changed());
            }
        }
        Ok(())
    }
}

struct Directory(*mut libc::DIR);

impl Drop for Directory {
    fn drop(&mut self) {
        // fdopendir owns this descriptor; closedir is its only release.
        unsafe { libc::closedir(self.0) };
    }
}

fn clear_errno() -> AppResult<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe {
        *libc::__errno_location() = 0;
        Ok(())
    }
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    unsafe {
        *libc::__error() = 0;
        Ok(())
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd"
    )))]
    {
        // Unsupported errno ABIs fail closed instead of treating enumeration
        // errors as an apparently complete directory listing.
        Err(unavailable())
    }
}

fn names(file: &File, limit: usize) -> AppResult<Vec<Vec<u8>>> {
    // A new open description has an independent directory cursor; dup would
    // share offsets and make the second complete-name check incorrectly empty.
    let listing = open_child(file, c".", InventoryKind::Tree)?;
    if held(&listing)? != held(file)? {
        return Err(changed());
    }
    let fd = listing.into_raw_fd();
    let directory = unsafe { libc::fdopendir(fd) };
    if directory.is_null() {
        drop(unsafe { File::from_raw_fd(fd) });
        return Err(unavailable());
    }
    let directory = Directory(directory);
    let mut result = Vec::new();
    loop {
        clear_errno()?;
        let entry = unsafe { libc::readdir(directory.0) };
        if entry.is_null() {
            if std::io::Error::last_os_error().raw_os_error() != Some(0) {
                return Err(unavailable());
            }
            break;
        }
        // A successful readdir supplies a NUL-terminated name until next call.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        if name.is_empty() || name.contains(&b'/') || result.len() >= limit {
            return Err(invalid());
        }
        result.push(name.to_vec());
    }
    result.sort();
    if result.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(changed());
    }
    Ok(result)
}

struct Capture {
    limits: Limits,
    euid: libc::uid_t,
    reserved_entries: usize,
    total_bytes: u64,
    directories: HashSet<(libc::dev_t, libc::ino_t)>,
    entries: Vec<(Vec<u8>, Entry)>,
    checks: HashMap<Vec<u8>, NodeCheck>,
}

struct NodeCheck {
    stamp: Stamp,
    children: Option<Vec<Vec<u8>>>,
}

impl Capture {
    fn check_named(&self, path: &[u8], observed: Stamp) -> AppResult<()> {
        if self.checks.get(path).ok_or_else(changed)?.stamp != observed {
            return Err(changed());
        }
        Ok(())
    }

    fn visit(&mut self, file: &File, before: Stamp, path: Vec<u8>, depth: usize) -> AppResult<()> {
        if depth > self.limits.depth || held(file)? != before {
            return Err(changed());
        }
        let kind = included(before, self.euid)?;
        let (length, sha256, children) = match kind {
            InventoryKind::File => {
                let length = u64::try_from(before.length).map_err(|_| invalid())?;
                self.total_bytes = self.total_bytes.checked_add(length).ok_or_else(invalid)?;
                if length > self.limits.file_bytes || self.total_bytes > self.limits.total_bytes {
                    return Err(invalid());
                }
                let mut source = file;
                let mut digest = Sha256::new();
                let mut buffer = [0u8; 64 * 1024];
                let mut read = 0u64;
                loop {
                    // At most one byte past the approved length is needed to
                    // detect growth, even if another writer appends continuously.
                    let remaining = usize::try_from(length - read)
                        .map_err(|_| invalid())?
                        .saturating_add(1)
                        .min(buffer.len());
                    let count = source
                        .read(&mut buffer[..remaining])
                        .map_err(|_| unavailable())?;
                    if count == 0 {
                        break;
                    }
                    read = read.checked_add(count as u64).ok_or_else(invalid)?;
                    if read > length {
                        return Err(changed());
                    }
                    digest.update(&buffer[..count]);
                }
                if read != length {
                    return Err(changed());
                }
                (length, Some(format!("{:x}", digest.finalize())), None)
            }
            InventoryKind::Tree => {
                if !self.directories.insert((before.device, before.inode)) {
                    return Err(invalid());
                }
                let children = names(file, self.limits.entries)?;
                self.reserved_entries = self
                    .reserved_entries
                    .checked_add(children.len())
                    .ok_or_else(invalid)?;
                if self.reserved_entries > self.limits.entries {
                    return Err(invalid());
                }
                for child in &children {
                    let name = CString::new(child.as_slice()).map_err(|_| invalid())?;
                    let named_before = named(file, &name)?;
                    let kind = included(named_before, self.euid)?;
                    let child_file = open_child(file, &name, kind)?;
                    let mut relative = path.clone();
                    if !relative.is_empty() {
                        relative.push(b'/');
                    }
                    relative.extend_from_slice(child);
                    self.visit(&child_file, named_before, relative, depth + 1)?;
                    if named(file, &name)? != named_before {
                        return Err(changed());
                    }
                }
                if names(file, self.limits.entries)? != children {
                    return Err(changed());
                }
                (0, None, Some(children))
            }
        };
        if held(file)? != before {
            return Err(changed());
        }
        self.checks.insert(
            path.clone(),
            NodeCheck {
                stamp: before,
                children,
            },
        );
        self.entries.push((
            path.clone(),
            Entry {
                path_hex: hex(&path),
                kind,
                mode: before.mode & 0o7777,
                length,
                sha256,
            },
        ));
        Ok(())
    }

    fn recheck(&self, file: &File, path: &[u8]) -> AppResult<()> {
        let expected = self.checks.get(path).ok_or_else(changed)?;
        if held(file)? != expected.stamp {
            return Err(changed());
        }
        included(expected.stamp, self.euid)?;
        if let Some(children) = &expected.children {
            if names(file, self.limits.entries)? != *children {
                return Err(changed());
            }
            for child in children {
                let name = CString::new(child.as_slice()).map_err(|_| invalid())?;
                let before = named(file, &name)?;
                let kind = included(before, self.euid)?;
                let mut relative = path.to_vec();
                if !relative.is_empty() {
                    relative.push(b'/');
                }
                relative.extend_from_slice(child);
                // Reject an observed mismatch before open, even if another
                // writer could restore the old node before its descriptor opens.
                self.check_named(&relative, before)?;
                let opened = open_child(file, &name, kind)?;
                self.recheck(&opened, &relative)?;
                if named(file, &name)? != before {
                    return Err(changed());
                }
            }
            if names(file, self.limits.entries)? != *children {
                return Err(changed());
            }
        }
        if held(file)? != expected.stamp {
            return Err(changed());
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    result
}

impl Inventory {
    pub(super) fn capture(path: &Path, kind: InventoryKind) -> AppResult<Self> {
        Self::capture_with_limits(path, kind, Limits::default())
    }

    fn capture_with_limits(path: &Path, kind: InventoryKind, limits: Limits) -> AppResult<Self> {
        if limits.entries == 0 {
            return Err(invalid());
        }
        let euid = unsafe { libc::geteuid() };
        let root = Root::open(path, kind, euid)?;
        let mut capture = Capture {
            limits,
            euid,
            reserved_entries: 1,
            total_bytes: 0,
            directories: HashSet::new(),
            entries: Vec::new(),
            checks: HashMap::new(),
        };
        capture.visit(&root.file, root.stamp, Vec::new(), 0)?;
        capture.recheck(&root.file, &[])?;
        root.recheck(euid)?;
        capture.entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Self {
            format: FORMAT,
            kind,
            entries: capture
                .entries
                .into_iter()
                .map(|(_, entry)| entry)
                .collect(),
        })
    }

    pub(super) fn verify(&self, path: &Path) -> AppResult<()> {
        if Self::capture(path, self.kind)? != *self {
            return Err(changed());
        }
        Ok(())
    }

    pub(super) fn sha256(&self) -> AppResult<String> {
        let bytes = serde_json::to_vec(self).map_err(|_| invalid())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

/// Bounded private bytes through the same physical, no-follow component walk.
/// This parses no profile and grants no authority; errors contain no path/data.
pub(super) fn read_private_file(path: &Path, maximum_bytes: usize) -> AppResult<Vec<u8>> {
    if maximum_bytes == 0 || maximum_bytes > MAX_PRIVATE_FILE_BYTES {
        return Err(invalid());
    }
    let euid = unsafe { libc::geteuid() };
    let root = Root::open(path, InventoryKind::File, euid)?;
    read_private_root(root, maximum_bytes, euid)
}

/// Complete private evidence envelope bytes with a fixed 16 MiB ceiling.
/// This grants no execution authority and cannot widen profile/approval limits.
pub(super) fn read_private_envelope(path: &Path) -> AppResult<Vec<u8>> {
    let euid = unsafe { libc::geteuid() };
    let root = Root::open(path, InventoryKind::File, euid)?;
    read_private_root(root, MAX_PRIVATE_ENVELOPE_BYTES, euid)
}

fn read_private_root(root: Root, maximum_bytes: usize, euid: libc::uid_t) -> AppResult<Vec<u8>> {
    let length = usize::try_from(root.stamp.length).map_err(|_| invalid())?;
    if root.stamp.mode & 0o7777 != 0o600 || length > maximum_bytes {
        return Err(invalid());
    }
    let mut result = Vec::with_capacity(length);
    let mut source = &root.file;
    let mut buffer = [0u8; 4096];
    loop {
        let remaining = (maximum_bytes - result.len() + 1).min(buffer.len());
        let count = source
            .read(&mut buffer[..remaining])
            .map_err(|_| unavailable())?;
        if count == 0 {
            break;
        }
        if result.len().saturating_add(count) > maximum_bytes {
            return Err(invalid());
        }
        result.extend_from_slice(&buffer[..count]);
    }
    if result.len() != length {
        return Err(changed());
    }
    root.recheck(euid)?;
    Ok(result)
}

#[cfg(test)]
#[path = "snowflake_table_qualification_inventory_tests.rs"]
mod tests;
