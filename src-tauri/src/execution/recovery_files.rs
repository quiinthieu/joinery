//! Native-selected handles only. No backup path is accepted over IPC.
use super::*;
use std::{fs::File, path::PathBuf};

fn file_error() -> AppError {
    AppError::invalid("Choose a readable regular backup file or a new writable export filename")
}

pub(super) fn open_source(path: &Path) -> AppResult<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| file_error())?;
    if !file.metadata().map_err(|_| file_error())?.is_file() {
        return Err(file_error());
    }
    Ok(file)
}

#[cfg(unix)]
pub(super) struct ExportTarget {
    staging: PrivateDirectory,
    temporary: std::ffi::CString,
    name: std::ffi::CString,
    pub file: File,
}

#[cfg(unix)]
struct PrivateDirectory {
    parent: File,
    directory: File,
    name: std::ffi::CString,
}

#[cfg(unix)]
fn same_entry(directory: &File, name: &std::ffi::CStr, held: &File) -> bool {
    use std::os::fd::AsRawFd;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let rc = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc != 0 {
        return false;
    }
    let mut held_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(held.as_raw_fd(), held_stat.as_mut_ptr()) } != 0 {
        return false;
    }
    let named = unsafe { stat.assume_init() };
    let held = unsafe { held_stat.assume_init() };
    named.st_dev == held.st_dev && named.st_ino == held.st_ino
}

#[cfg(unix)]
impl PrivateDirectory {
    fn new(path: &Path) -> AppResult<Self> {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        };
        let parent = std::fs::canonicalize(path.parent().ok_or_else(file_error)?)
            .map_err(|_| file_error())?;
        let parent =
            std::ffi::CString::new(parent.as_os_str().as_bytes()).map_err(|_| file_error())?;
        let fd = unsafe {
            libc::open(
                parent.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(file_error());
        }
        let parent = unsafe { File::from_raw_fd(fd) };
        let name = std::ffi::CString::new(format!(".joinery-backup-{}.partial", new_id()))
            .map_err(|_| file_error())?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(file_error());
        }
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        // On failure leave the unfamiliar/unpinned name alone rather than
        // removing a directory that might have replaced the one just created.
        if fd < 0 {
            return Err(file_error());
        }
        let directory = unsafe { File::from_raw_fd(fd) };
        let result = Self {
            parent,
            directory,
            name,
        };
        let metadata = result.directory.metadata().map_err(|_| file_error())?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
            || !same_entry(&result.parent, &result.name, &result.directory)
        {
            return Err(file_error());
        }
        Ok(result)
    }
}

#[cfg(unix)]
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // Never recurse: unfamiliar children or a replaced parent entry remain.
        // Unix has no atomic inode-conditional unlink. The private 0700 namespace
        // excludes other UIDs; concurrent malicious same-UID mutation between
        // identity checks and unlink is outside that protection boundary.
        if same_entry(&self.parent, &self.name, &self.directory) {
            let _ = unsafe {
                libc::unlinkat(
                    self.parent.as_raw_fd(),
                    self.name.as_ptr(),
                    libc::AT_REMOVEDIR,
                )
            };
            let _ = self.parent.sync_all();
        }
    }
}

#[cfg(unix)]
impl ExportTarget {
    pub fn new(path: PathBuf) -> AppResult<Self> {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        };
        let name = std::ffi::CString::new(path.file_name().ok_or_else(file_error)?.as_bytes())
            .map_err(|_| file_error())?;
        let staging = PrivateDirectory::new(&path)?;
        let temporary = std::ffi::CString::new("ciphertext").map_err(|_| file_error())?;
        let fd = unsafe {
            libc::openat(
                staging.directory.as_raw_fd(),
                temporary.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(file_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        Ok(Self {
            staging,
            temporary,
            name,
            file,
        })
    }

    pub fn publish(self) -> AppResult<()> {
        if !same_entry(&self.staging.directory, &self.temporary, &self.file)
            || !same_entry(
                &self.staging.parent,
                &self.staging.name,
                &self.staging.directory,
            )
        {
            return Err(file_error());
        }
        self.file.sync_all().map_err(|_| file_error())?;
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsRawFd;
            // Apple's atomic clone takes the SOURCE FD, not a mutable filename.
            // APFS gets an independent copy-on-write snapshot; existing targets
            // (including symlinks) are never overwritten.
            let rc = unsafe {
                libc::fclonefileat(
                    self.file.as_raw_fd(),
                    self.staging.parent.as_raw_fd(),
                    self.name.as_ptr(),
                    0,
                )
            };
            if rc == 0 {
                return self.confirm_clone();
            }
            let error = std::io::Error::last_os_error();
            if !matches!(
                error.raw_os_error(),
                Some(libc::ENOTSUP) | Some(libc::EXDEV) | Some(libc::EINVAL) | Some(libc::ENOSYS)
            ) {
                return Err(publication_error());
            }
        }
        self.publish_link()
    }

    fn publish_link(&self) -> AppResult<()> {
        use std::os::fd::AsRawFd;
        // Portable no-overwrite fallback. Its source entry lives in our pinned
        // private namespace; verify both names around the atomic link.
        if !same_entry(&self.staging.directory, &self.temporary, &self.file)
            || !same_entry(
                &self.staging.parent,
                &self.staging.name,
                &self.staging.directory,
            )
        {
            return Err(file_error());
        }
        self.file.sync_all().map_err(|_| file_error())?;
        if unsafe {
            libc::linkat(
                self.staging.directory.as_raw_fd(),
                self.temporary.as_ptr(),
                self.staging.parent.as_raw_fd(),
                self.name.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(AppError::invalid(
                "The export filename already exists or cannot be published; choose a new filename",
            ));
        }
        self.confirm_link()
    }

    fn confirm_link(&self) -> AppResult<()> {
        if !same_entry(&self.staging.parent, &self.name, &self.file)
            || !same_entry(&self.staging.directory, &self.temporary, &self.file)
        {
            return Err(publication_error());
        }
        self.staging
            .parent
            .sync_all()
            .map_err(|_| publication_error())?;
        if !same_entry(&self.staging.parent, &self.name, &self.file) {
            return Err(publication_error());
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn confirm_clone(&self) -> AppResult<()> {
        use sha2::{Digest, Sha256};
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::FileExt,
        };
        let fd = unsafe {
            libc::openat(
                self.staging.parent.as_raw_fd(),
                self.name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(publication_error());
        }
        let cloned = unsafe { File::from_raw_fd(fd) };
        if !cloned
            .metadata()
            .map_err(|_| publication_error())?
            .is_file()
            || !same_entry(&self.staging.parent, &self.name, &cloned)
        {
            return Err(publication_error());
        }
        // Confirm the captured destination, rather than trusting a pathname
        // opened after clone. Fixed buffers; ciphertext only, never plaintext.
        let digest = |file: &File| -> AppResult<(u64, [u8; 32])> {
            let mut hash = Sha256::new();
            let mut offset = 0;
            let mut buffer = [0; 64 * 1024];
            loop {
                let count = file
                    .read_at(&mut buffer, offset)
                    .map_err(|_| publication_error())?;
                if count == 0 {
                    break;
                }
                offset += count as u64;
                if offset > crate::persistence::recovery_crypto::MAX_CIPHERTEXT_BYTES {
                    return Err(publication_error());
                }
                hash.update(&buffer[..count]);
            }
            Ok((offset, hash.finalize().into()))
        };
        if digest(&self.file)? != digest(&cloned)? {
            return Err(publication_error());
        }
        cloned.sync_all().map_err(|_| publication_error())?;
        self.staging
            .parent
            .sync_all()
            .map_err(|_| publication_error())?;
        if !same_entry(&self.staging.parent, &self.name, &cloned)
            || !same_entry(&self.staging.directory, &self.temporary, &self.file)
        {
            return Err(publication_error());
        }
        Ok(())
    }
}

#[cfg(unix)]
fn publication_error() -> AppError {
    AppError::invalid(
        "Could not confirm export publication; a complete file may remain at the selected filename",
    )
}

#[cfg(unix)]
impl Drop for ExportTarget {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // Never unlink the selected destination, even after uncertain sync or
        // replacement: that name is outside our private cleanup namespace.
        if same_entry(&self.staging.directory, &self.temporary, &self.file) {
            let _ = unsafe {
                libc::unlinkat(
                    self.staging.directory.as_raw_fd(),
                    self.temporary.as_ptr(),
                    0,
                )
            };
        }
        let _ = self.staging.directory.sync_all();
    }
}

#[cfg(not(unix))]
pub(super) struct ExportTarget {
    pub file: File,
}

#[cfg(not(unix))]
impl ExportTarget {
    pub fn new(_path: PathBuf) -> AppResult<Self> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Encrypted backup export requires native Unix file capabilities",
        ))
    }
    pub fn publish(self) -> AppResult<()> {
        Err(file_error())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::{ffi::OsStrExt, fs::MetadataExt},
    };

    fn staging_path(root: &Path, writer: &ExportTarget) -> PathBuf {
        root.join(std::ffi::OsStr::from_bytes(writer.staging.name.as_bytes()))
    }

    fn partial_path(root: &Path, writer: &ExportTarget) -> PathBuf {
        staging_path(root, writer).join("ciphertext")
    }

    #[test]
    fn atomic_export_preserves_existing_targets_and_cleans_only_owned_partials() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing.age");
        std::fs::write(&existing, b"preserve").unwrap();
        let mut writer = ExportTarget::new(existing.clone()).unwrap();
        writer.file.write_all(b"new encrypted bytes").unwrap();
        assert!(writer.publish().is_err());
        assert_eq!(std::fs::read(&existing).unwrap(), b"preserve");
        let dest = dir.path().join("new.age");
        let mut writer = ExportTarget::new(dest.clone()).unwrap();
        writer.file.write_all(b"ciphertext").unwrap();
        writer.publish().unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"ciphertext");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        let link = dir.path().join("link.age");
        std::os::unix::fs::symlink(&existing, &link).unwrap();
        assert!(open_source(&link).is_err());
        assert!(open_source(Path::new("/dev/zero")).is_err());
    }

    #[test]
    fn atomic_export_rejects_replaced_partial_and_preserves_unfamiliar_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("backup.age");
        let mut writer = ExportTarget::new(dest.clone()).unwrap();
        writer.file.write_all(b"original ciphertext").unwrap();
        let partial = partial_path(dir.path(), &writer);
        std::fs::remove_file(&partial).unwrap();
        std::fs::write(&partial, b"unfamiliar replacement").unwrap();
        assert!(writer.publish().is_err());
        assert!(!dest.exists());
        assert_eq!(std::fs::read(&partial).unwrap(), b"unfamiliar replacement");
    }

    #[test]
    fn atomic_export_rejects_replaced_partial_symlink_without_deleting_it() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("unrelated");
        std::fs::write(&outside, b"preserve").unwrap();
        let dest = dir.path().join("backup.age");
        let mut writer = ExportTarget::new(dest.clone()).unwrap();
        writer.file.write_all(b"original ciphertext").unwrap();
        let partial = partial_path(dir.path(), &writer);
        std::fs::remove_file(&partial).unwrap();
        std::os::unix::fs::symlink(&outside, &partial).unwrap();
        assert!(writer.publish().is_err());
        assert!(!dest.exists());
        assert!(
            std::fs::symlink_metadata(&partial)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"preserve");
    }

    #[test]
    fn atomic_export_rejects_replaced_private_directory_and_preserves_foreign_entry() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("backup.age");
        let mut writer = ExportTarget::new(dest.clone()).unwrap();
        writer.file.write_all(b"original ciphertext").unwrap();
        let original = staging_path(dir.path(), &writer);
        let moved = dir.path().join("moved-private-directory");
        std::fs::rename(&original, &moved).unwrap();
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("unfamiliar"), b"preserve").unwrap();
        assert!(writer.publish().is_err());
        assert!(!dest.exists());
        assert_eq!(
            std::fs::read(original.join("unfamiliar")).unwrap(),
            b"preserve"
        );
        // Cleanup uses the captured directory FD; it never guesses its new name.
        assert_eq!(std::fs::read_dir(moved).unwrap().count(), 0);
    }

    #[test]
    fn atomic_export_uses_captured_parent_after_directory_rename() {
        let dir = tempfile::tempdir().unwrap();
        let selected = dir.path().join("selected");
        std::fs::create_dir(&selected).unwrap();
        let mut writer = ExportTarget::new(selected.join("backup.age")).unwrap();
        writer.file.write_all(b"captured ciphertext").unwrap();
        let moved = dir.path().join("moved-selected");
        std::fs::rename(&selected, &moved).unwrap();
        std::fs::create_dir(&selected).unwrap();
        std::fs::write(selected.join("backup.age"), b"unfamiliar destination").unwrap();
        writer.publish().unwrap();
        assert_eq!(
            std::fs::read(moved.join("backup.age")).unwrap(),
            b"captured ciphertext"
        );
        assert_eq!(
            std::fs::read(selected.join("backup.age")).unwrap(),
            b"unfamiliar destination"
        );
        assert_eq!(std::fs::read_dir(moved).unwrap().count(), 1);
    }

    #[test]
    fn atomic_export_portable_link_verifies_destination_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("backup.age");
        let mut writer = ExportTarget::new(dest.clone()).unwrap();
        writer.file.write_all(b"captured ciphertext").unwrap();
        writer.publish_link().unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"captured ciphertext");
        std::fs::remove_file(&dest).unwrap();
        std::fs::write(&dest, b"unfamiliar destination").unwrap();
        assert!(writer.confirm_link().is_err());
        assert!(writer.publish_link().is_err());
        drop(writer);
        assert_eq!(std::fs::read(&dest).unwrap(), b"unfamiliar destination");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_export_abandoned_writer_cleans_private_namespace_only() {
        let dir = tempfile::tempdir().unwrap();
        let unrelated = dir.path().join("unrelated");
        std::fs::write(&unrelated, b"preserve").unwrap();
        let mut writer = ExportTarget::new(dir.path().join("backup.age")).unwrap();
        writer.file.write_all(b"partial ciphertext").unwrap();
        assert_eq!(
            writer.staging.directory.metadata().unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(writer.file.metadata().unwrap().mode() & 0o777, 0o600);
        drop(writer);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_eq!(std::fs::read(unrelated).unwrap(), b"preserve");
    }

    #[test]
    fn atomic_export_source_handle_survives_path_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("selected.age");
        std::fs::write(&source, b"selected ciphertext").unwrap();
        let mut selected = open_source(&source).unwrap();
        std::fs::rename(&source, dir.path().join("moved.age")).unwrap();
        std::fs::write(&source, b"unfamiliar ciphertext").unwrap();
        let mut bytes = Vec::new();
        selected.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"selected ciphertext");
        assert_eq!(std::fs::read(source).unwrap(), b"unfamiliar ciphertext");
    }
}
