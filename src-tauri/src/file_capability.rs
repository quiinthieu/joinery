use crate::domain::*;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub const MAX_FILE_GRANTS: usize = 32;
pub const FILE_GRANT_TTL: Duration = Duration::from_secs(600);

#[async_trait]
pub trait FileDialogProvider: Send + Sync {
    async fn pick_save_file(
        &self,
        suggested_name: &str,
        format: ExportFormat,
    ) -> AppResult<Option<PathBuf>>;

    async fn pick_open_csv(&self) -> AppResult<Option<PathBuf>>;
}

pub struct DisabledFileDialog;

#[async_trait]
impl FileDialogProvider for DisabledFileDialog {
    async fn pick_save_file(
        &self,
        _suggested_name: &str,
        _format: ExportFormat,
    ) -> AppResult<Option<PathBuf>> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Native file dialogs are only available in the desktop app",
        ))
    }

    async fn pick_open_csv(&self) -> AppResult<Option<PathBuf>> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Native file dialogs are only available in the desktop app",
        ))
    }
}

#[cfg(feature = "desktop")]
pub struct NativeFileDialog;

#[cfg(feature = "desktop")]
#[async_trait]
impl FileDialogProvider for NativeFileDialog {
    async fn pick_save_file(
        &self,
        suggested_name: &str,
        format: ExportFormat,
    ) -> AppResult<Option<PathBuf>> {
        let picked = rfd::AsyncFileDialog::new()
            .set_file_name(suggested_name)
            .add_filter(format.label(), &[format.extension()])
            .save_file()
            .await;
        Ok(picked.map(|handle| handle.path().to_path_buf()))
    }

    async fn pick_open_csv(&self) -> AppResult<Option<PathBuf>> {
        let picked = rfd::AsyncFileDialog::new()
            .add_filter("Delimited text", &["csv", "tsv", "txt"])
            .pick_file()
            .await;
        Ok(picked.map(|handle| handle.path().to_path_buf()))
    }
}

#[derive(Default)]
struct SandboxQueue {
    next_save: Option<Option<PathBuf>>,
    next_open: Option<Option<PathBuf>>,
}

/// Test-only confined dialog provider restricted to a single disposable directory.
pub struct SandboxFileDialog {
    root: PathBuf,
    queue: Mutex<SandboxQueue>,
}

impl SandboxFileDialog {
    pub fn new(root: &Path) -> AppResult<Arc<Self>> {
        std::fs::create_dir_all(root)
            .map_err(|_| AppError::internal("Could not create disposable file sandbox"))?;
        let canonical = std::fs::canonicalize(root)
            .map_err(|_| AppError::internal("Could not resolve disposable file sandbox"))?;
        Ok(Arc::new(Self {
            root: canonical,
            queue: Mutex::new(SandboxQueue::default()),
        }))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve_confined(&self, file_name: &str) -> AppResult<PathBuf> {
        let trimmed = file_name.trim();
        if trimmed.is_empty()
            || trimmed.len() > 200
            || trimmed.contains('\0')
            || trimmed.contains('/')
            || trimmed.contains('\\')
        {
            return Err(AppError::invalid(
                "Sandbox file names must be single plain file names",
            ));
        }
        let mut components = Path::new(trimmed).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => {}
            _ => {
                return Err(AppError::invalid(
                    "Sandbox file names must not traverse directories",
                ));
            }
        }
        let candidate = self.root.join(trimmed);
        let parent = candidate
            .parent()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .ok_or_else(|| AppError::invalid("Invalid sandbox file parent"))?;
        if parent != self.root {
            return Err(AppError::invalid("Sandbox path escaped its root"));
        }
        if let Ok(meta) = std::fs::symlink_metadata(&candidate)
            && meta.file_type().is_symlink()
        {
            return Err(AppError::invalid("Sandbox files must not be symlinks"));
        }
        Ok(candidate)
    }

    pub fn queue_save(&self, file_name: Option<&str>) -> AppResult<()> {
        let resolved = file_name
            .map(|name| self.resolve_confined(name))
            .transpose()?;
        self.queue.lock().next_save = Some(resolved);
        Ok(())
    }

    pub fn queue_open(&self, file_name: Option<&str>) -> AppResult<()> {
        let resolved = file_name
            .map(|name| self.resolve_confined(name))
            .transpose()?;
        self.queue.lock().next_open = Some(resolved);
        Ok(())
    }
}

#[async_trait]
impl FileDialogProvider for SandboxFileDialog {
    async fn pick_save_file(
        &self,
        suggested_name: &str,
        _format: ExportFormat,
    ) -> AppResult<Option<PathBuf>> {
        if let Some(queued) = self.queue.lock().next_save.take() {
            return Ok(queued);
        }
        Ok(Some(self.resolve_confined(suggested_name)?))
    }

    async fn pick_open_csv(&self) -> AppResult<Option<PathBuf>> {
        self.queue.lock().next_open.take().ok_or_else(|| {
            AppError::invalid("No import file was queued in the disposable file sandbox")
        })
    }
}

enum GrantKind {
    Write {
        path: PathBuf,
    },
    #[allow(dead_code)]
    Read {
        path: PathBuf,
        max_bytes: u64,
    },
}

struct StoredGrant {
    kind: GrantKind,
    deadline: Instant,
}

#[derive(Default)]
pub struct FileCapabilities {
    grants: Mutex<HashMap<String, StoredGrant>>,
}

impl FileCapabilities {
    pub fn validate_suggested_name(suggested_name: &str) -> AppResult<String> {
        let trimmed = suggested_name.trim();
        if trimmed.is_empty() || trimmed.len() > 200 || trimmed.contains('\0') {
            return Err(AppError::invalid("Provide a valid export file name"));
        }
        let sanitized: String = trimmed
            .chars()
            .map(|ch| {
                if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                    || ch.is_control()
                {
                    '-'
                } else {
                    ch
                }
            })
            .collect();
        if sanitized.trim_matches('.').trim().is_empty() {
            return Err(AppError::invalid("Provide a valid export file name"));
        }
        Ok(sanitized)
    }

    pub fn mint_write_grant(&self, path: PathBuf) -> AppResult<FileSelection> {
        self.mint_write_grant_with_ttl(path, FILE_GRANT_TTL)
    }

    pub fn mint_write_grant_with_ttl(
        &self,
        path: PathBuf,
        ttl: Duration,
    ) -> AppResult<FileSelection> {
        let file_name = extract_file_name(&path)?;
        if path.is_dir() {
            return Err(AppError::invalid(
                "Choose a destination file rather than a directory",
            ));
        }
        let parent = path
            .parent()
            .filter(|p| p.as_os_str().is_empty() || p.is_dir())
            .ok_or_else(|| AppError::invalid("Choose an existing destination folder"))?;
        let _ = parent;
        if let Ok(meta) = std::fs::symlink_metadata(&path)
            && meta.file_type().is_symlink()
        {
            return Err(AppError::invalid(
                "Refusing to overwrite a symbolic link during export",
            ));
        }

        let token = self.insert_grant(GrantKind::Write { path }, Instant::now() + ttl)?;
        Ok(FileSelection {
            token,
            display_label: file_name.clone(),
            file_name,
        })
    }

    pub fn verify_write_grant(&self, token: &str) -> AppResult<()> {
        let now = Instant::now();
        let mut grants = self.grants.lock();
        grants.retain(|_, g| g.deadline > now);
        let stored = grants.get(token).ok_or_else(expired_grant)?;
        match stored.kind {
            GrantKind::Write { .. } => Ok(()),
            GrantKind::Read { .. } => Err(AppError::invalid(
                "This file capability is read-only and cannot be used for export",
            )),
        }
    }

    pub fn consume_write_grant(&self, token: &str) -> AppResult<PathBuf> {
        let now = Instant::now();
        let mut grants = self.grants.lock();
        grants.retain(|_, g| g.deadline > now);
        let stored = grants.remove(token).ok_or_else(expired_grant)?;
        match stored.kind {
            GrantKind::Write { path } => Ok(path),
            GrantKind::Read { .. } => Err(AppError::invalid(
                "This file capability is read-only and cannot be used for export",
            )),
        }
    }

    #[allow(dead_code)]
    pub fn mint_read_grant(&self, path: PathBuf, max_bytes: u64) -> AppResult<FileSelection> {
        self.mint_read_grant_with_ttl(path, max_bytes, FILE_GRANT_TTL)
    }

    #[allow(dead_code)]
    pub fn mint_read_grant_with_ttl(
        &self,
        path: PathBuf,
        max_bytes: u64,
        ttl: Duration,
    ) -> AppResult<FileSelection> {
        let file_name = extract_file_name(&path)?;
        verify_readable_regular_file(&path, max_bytes)?;
        let token = self.insert_grant(GrantKind::Read { path, max_bytes }, Instant::now() + ttl)?;
        Ok(FileSelection {
            token,
            display_label: file_name.clone(),
            file_name,
        })
    }

    #[allow(dead_code)]
    pub fn resolve_read_grant(&self, token: &str) -> AppResult<(PathBuf, u64)> {
        let now = Instant::now();
        let (path, max_bytes) = {
            let mut grants = self.grants.lock();
            grants.retain(|_, g| g.deadline > now);
            let stored = grants.get(token).ok_or_else(expired_grant)?;
            match &stored.kind {
                GrantKind::Read { path, max_bytes } => (path.clone(), *max_bytes),
                GrantKind::Write { .. } => {
                    return Err(AppError::invalid(
                        "This file capability is write-only and cannot be read",
                    ));
                }
            }
        };
        verify_readable_regular_file(&path, max_bytes)?;
        Ok((path, max_bytes))
    }

    pub fn clear(&self) {
        self.grants.lock().clear();
    }

    fn insert_grant(&self, kind: GrantKind, deadline: Instant) -> AppResult<String> {
        let now = Instant::now();
        let mut grants = self.grants.lock();
        grants.retain(|_, g| g.deadline > now);
        if grants.len() >= MAX_FILE_GRANTS {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Too many file selections are open. Close an export or import dialog first.",
            ));
        }
        let token = new_id();
        grants.insert(token.clone(), StoredGrant { kind, deadline });
        Ok(token)
    }
}

fn extract_file_name(path: &Path) -> AppResult<String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| AppError::invalid("Choose a file with a valid UTF-8 file name"))?;
    Ok(name.to_owned())
}

fn verify_readable_regular_file(path: &Path, max_bytes: u64) -> AppResult<()> {
    let meta = std::fs::metadata(path).map_err(|_| {
        AppError::invalid("Choose a readable regular file before starting CSV import")
    })?;
    if !meta.is_file() {
        return Err(AppError::invalid(
            "Choose a regular file rather than a directory or device",
        ));
    }
    if meta.len() > max_bytes {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            format!(
                "Import files are limited to {} MiB",
                max_bytes / (1024 * 1024)
            ),
        ));
    }
    Ok(())
}

fn expired_grant() -> AppError {
    AppError::new(
        ErrorCode::ApprovalExpired,
        "The selected file grant expired or was already used. Choose the file again.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_grants_are_one_use_and_expire_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let caps = FileCapabilities::default();
        let dest = dir.path().join("orders.csv");
        let grant = caps.mint_write_grant(dest.clone()).unwrap();
        assert_eq!(grant.file_name, "orders.csv");

        let resolved = caps.consume_write_grant(&grant.token).unwrap();
        assert_eq!(resolved, dest);
        // Replaying the same write grant must fail
        assert_eq!(
            caps.consume_write_grant(&grant.token).unwrap_err().code,
            ErrorCode::ApprovalExpired
        );

        let expired = caps
            .mint_write_grant_with_ttl(dir.path().join("expired.csv"), Duration::ZERO)
            .unwrap();
        assert_eq!(
            caps.consume_write_grant(&expired.token).unwrap_err().code,
            ErrorCode::ApprovalExpired
        );
    }

    #[test]
    fn read_grants_validate_regular_files_size_caps_and_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let caps = FileCapabilities::default();
        let csv_path = dir.path().join("customers.csv");
        std::fs::write(&csv_path, b"id,name\n1,Aria\n").unwrap();

        let grant = caps.mint_read_grant(csv_path.clone(), 1024 * 1024).unwrap();
        assert_eq!(caps.resolve_read_grant(&grant.token).unwrap().0, csv_path);
        // Reusable across preview & batch preparation
        assert_eq!(caps.resolve_read_grant(&grant.token).unwrap().0, csv_path);

        // Oversized file is rejected
        assert_eq!(
            caps.mint_read_grant(csv_path.clone(), 4).unwrap_err().code,
            ErrorCode::ResultLimitReached
        );

        #[cfg(unix)]
        assert!(
            caps.mint_read_grant(PathBuf::from("/dev/zero"), 1024)
                .is_err()
        );

        caps.clear();
        for idx in 0..MAX_FILE_GRANTS {
            caps.mint_write_grant(dir.path().join(format!("f{idx}.csv")))
                .unwrap();
        }
        assert_eq!(
            caps.mint_write_grant(dir.path().join("overflow.csv"))
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
    }

    #[test]
    fn sandbox_dialog_rejects_traversal_and_absolute_paths() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = SandboxFileDialog::new(dir.path()).unwrap();
        assert!(sandbox.resolve_confined("valid-export.json").is_ok());
        assert!(sandbox.resolve_confined("../escape.json").is_err());
        assert!(sandbox.resolve_confined("nested/escape.json").is_err());
        assert!(sandbox.resolve_confined("/etc/passwd").is_err());
        assert!(sandbox.resolve_confined("bad\0name.csv").is_err());
    }
}
