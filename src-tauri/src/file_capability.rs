use crate::domain::*;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub const MAX_FILE_GRANTS: usize = 32;
pub const FILE_GRANT_TTL: Duration = Duration::from_secs(600);
pub const MAX_SQL_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SQL_FILE_GRANTS: usize = 64;
pub const SQL_FILE_GRANT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[async_trait]
pub trait FileDialogProvider: Send + Sync {
    async fn pick_save_file(
        &self,
        suggested_name: &str,
        format: ExportFormat,
    ) -> AppResult<Option<PathBuf>>;

    async fn pick_open_csv(&self) -> AppResult<Option<PathBuf>>;

    async fn pick_open_sql(&self) -> AppResult<Option<PathBuf>>;

    async fn pick_save_sql(&self, suggested_name: &str) -> AppResult<Option<PathBuf>>;
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

    async fn pick_open_sql(&self) -> AppResult<Option<PathBuf>> {
        Err(AppError::new(
            ErrorCode::Unsupported,
            "Native file dialogs are only available in the desktop app",
        ))
    }

    async fn pick_save_sql(&self, _suggested_name: &str) -> AppResult<Option<PathBuf>> {
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

    async fn pick_open_sql(&self) -> AppResult<Option<PathBuf>> {
        let picked = rfd::AsyncFileDialog::new()
            .add_filter("SQL", &["sql", "ddl", "pgsql", "mysql"])
            .pick_file()
            .await;
        Ok(picked.map(|handle| handle.path().to_path_buf()))
    }

    async fn pick_save_sql(&self, suggested_name: &str) -> AppResult<Option<PathBuf>> {
        let picked = rfd::AsyncFileDialog::new()
            .set_file_name(suggested_name)
            .add_filter("SQL", &["sql"])
            .save_file()
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

    async fn pick_open_sql(&self) -> AppResult<Option<PathBuf>> {
        self.queue.lock().next_open.take().ok_or_else(|| {
            AppError::invalid("No SQL file was queued in the disposable file sandbox")
        })
    }

    async fn pick_save_sql(&self, suggested_name: &str) -> AppResult<Option<PathBuf>> {
        if let Some(queued) = self.queue.lock().next_save.take() {
            return Ok(queued);
        }
        Ok(Some(self.resolve_confined(suggested_name)?))
    }
}

enum GrantKind {
    Write { path: PathBuf },
    Read { path: PathBuf, max_bytes: u64 },
    SqlFile { path: PathBuf },
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
            GrantKind::SqlFile { .. } => Err(AppError::invalid(
                "This SQL file capability cannot be used for data export",
            )),
        }
    }

    pub fn consume_write_grant(&self, token: &str) -> AppResult<PathBuf> {
        let now = Instant::now();
        let mut grants = self.grants.lock();
        grants.retain(|_, g| g.deadline > now);
        match grants.get(token).map(|g| &g.kind) {
            None => return Err(expired_grant()),
            Some(GrantKind::Read { .. }) => {
                return Err(AppError::invalid(
                    "This file capability is read-only and cannot be used for export",
                ));
            }
            Some(GrantKind::SqlFile { .. }) => {
                return Err(AppError::invalid(
                    "This SQL file capability cannot be used for data export",
                ));
            }
            Some(GrantKind::Write { .. }) => {}
        }
        match grants.remove(token) {
            Some(StoredGrant {
                kind: GrantKind::Write { path },
                ..
            }) => Ok(path),
            _ => Err(expired_grant()),
        }
    }

    pub fn mint_read_grant(&self, path: PathBuf, max_bytes: u64) -> AppResult<FileSelection> {
        self.mint_read_grant_with_ttl(path, max_bytes, FILE_GRANT_TTL)
    }

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
                GrantKind::SqlFile { .. } => {
                    return Err(AppError::invalid(
                        "This SQL file capability cannot be used for CSV import",
                    ));
                }
            }
        };
        verify_readable_regular_file(&path, max_bytes)?;
        Ok((path, max_bytes))
    }

    pub fn validate_sql_suggested_name(suggested_name: &str) -> AppResult<String> {
        let fallback = if suggested_name.trim().is_empty() {
            "query.sql"
        } else {
            suggested_name.trim()
        };
        let sanitized = Self::validate_suggested_name(fallback)?;
        let lower = sanitized.to_ascii_lowercase();
        if lower.ends_with(".sql")
            || lower.ends_with(".ddl")
            || lower.ends_with(".pgsql")
            || lower.ends_with(".mysql")
        {
            return Ok(sanitized);
        }
        let stem: String = sanitized.chars().take(196).collect();
        Ok(format!("{stem}.sql"))
    }

    pub fn validate_sql_content(content: &str) -> AppResult<()> {
        if content.len() > MAX_SQL_FILE_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "SQL files are limited to 4 MiB",
            ));
        }
        if content.contains('\0') {
            return Err(AppError::invalid(
                "SQL files must be plain UTF-8 text without binary NUL bytes",
            ));
        }
        Ok(())
    }

    pub fn verify_sql_write_target(path: &Path) -> AppResult<String> {
        let file_name = extract_file_name(path)?;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if meta.file_type().is_symlink() {
                return Err(AppError::invalid(
                    "Refusing to overwrite a symbolic link when saving a SQL file",
                ));
            }
            if meta.is_dir() {
                return Err(AppError::invalid(
                    "Choose a destination SQL file rather than a directory",
                ));
            }
        }
        let parent = path
            .parent()
            .filter(|p| p.as_os_str().is_empty() || p.is_dir())
            .ok_or_else(|| AppError::invalid("Choose an existing destination folder"))?;
        let _ = parent;
        Ok(file_name)
    }

    pub fn read_sql_file_text(path: &Path) -> AppResult<(String, String)> {
        let file_name = extract_file_name(path)?;
        let sym_meta = std::fs::symlink_metadata(path)
            .map_err(|_| AppError::invalid("Choose a readable regular SQL file"))?;
        if sym_meta.file_type().is_symlink() {
            return Err(AppError::invalid(
                "Refusing to open a symbolic link as a SQL file",
            ));
        }
        if !sym_meta.is_file() {
            return Err(AppError::invalid(
                "Choose a regular SQL file rather than a directory or device",
            ));
        }
        if sym_meta.len() > MAX_SQL_FILE_BYTES as u64 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "SQL files are limited to 4 MiB",
            ));
        }
        let file = std::fs::File::open(path)
            .map_err(|_| AppError::invalid("Could not read the selected SQL file"))?;
        let fd_meta = file
            .metadata()
            .map_err(|_| AppError::invalid("Could not inspect the selected SQL file"))?;
        if !fd_meta.is_file() {
            return Err(AppError::invalid(
                "Choose a regular SQL file rather than a directory or device",
            ));
        }
        let cap = (sym_meta.len() as usize).min(MAX_SQL_FILE_BYTES);
        let mut bytes = Vec::with_capacity(cap);
        file.take((MAX_SQL_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| AppError::invalid("Could not read the selected SQL file"))?;
        if bytes.len() > MAX_SQL_FILE_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "SQL files are limited to 4 MiB",
            ));
        }
        if bytes.contains(&0) {
            return Err(AppError::invalid(
                "SQL files must be plain UTF-8 text without binary NUL bytes",
            ));
        }
        let content = String::from_utf8(bytes)
            .map_err(|_| AppError::invalid("SQL files must be valid UTF-8 text"))?;
        Ok((file_name, content))
    }

    pub fn write_sql_file_atomic(path: &Path, content: &str) -> AppResult<String> {
        Self::validate_sql_content(content)?;
        let file_name = Self::verify_sql_write_target(path)?;
        let existing_perms = std::fs::symlink_metadata(path)
            .ok()
            .filter(|m| m.is_file() && !m.file_type().is_symlink())
            .map(|m| m.permissions());
        let tmp_path = path.with_file_name(format!(".joinery-sql-{}.tmp", new_id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut tmp_file = options
            .open(&tmp_path)
            .map_err(|_| AppError::internal("Could not create temporary SQL file for saving"))?;
        let write_outcome: AppResult<()> = (|| {
            tmp_file
                .write_all(content.as_bytes())
                .map_err(|_| AppError::internal("Could not write SQL file contents"))?;
            tmp_file
                .sync_all()
                .map_err(|_| AppError::internal("Could not flush SQL file contents to disk"))?;
            if let Some(perms) = existing_perms {
                let _ = tmp_file.set_permissions(perms);
            }
            drop(tmp_file);
            if let Ok(meta) = std::fs::symlink_metadata(path)
                && (meta.file_type().is_symlink() || meta.is_dir())
            {
                return Err(AppError::invalid(
                    "Refusing to overwrite a symbolic link or directory when saving a SQL file",
                ));
            }
            std::fs::rename(&tmp_path, path)
                .map_err(|_| AppError::internal("Could not finalize saved SQL file"))?;
            Ok(())
        })();
        if let Err(err) = write_outcome {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(err);
        }
        Ok(file_name)
    }

    pub fn mint_sql_file_grant(&self, path: PathBuf) -> AppResult<String> {
        self.mint_sql_file_grant_with_ttl(path, SQL_FILE_GRANT_TTL)
    }

    pub fn mint_sql_file_grant_with_ttl(&self, path: PathBuf, ttl: Duration) -> AppResult<String> {
        let _ = Self::verify_sql_write_target(&path)?;
        self.insert_grant(GrantKind::SqlFile { path }, Instant::now() + ttl)
    }

    pub fn resolve_sql_file_grant(&self, token: &str) -> AppResult<PathBuf> {
        let now = Instant::now();
        let mut grants = self.grants.lock();
        grants.retain(|_, g| g.deadline > now);
        let stored = grants.get_mut(token).ok_or_else(expired_grant)?;
        match &stored.kind {
            GrantKind::SqlFile { path } => {
                stored.deadline = now + SQL_FILE_GRANT_TTL;
                Ok(path.clone())
            }
            GrantKind::Write { .. } | GrantKind::Read { .. } => Err(AppError::invalid(
                "This file capability is not a SQL console file grant",
            )),
        }
    }

    pub fn open_sql_file(&self, path: PathBuf) -> AppResult<OpenedSqlFile> {
        self.open_sql_file_with_ttl(path, SQL_FILE_GRANT_TTL)
    }

    pub fn open_sql_file_with_ttl(&self, path: PathBuf, ttl: Duration) -> AppResult<OpenedSqlFile> {
        let (file_name, content) = Self::read_sql_file_text(&path)?;
        let token = self.mint_sql_file_grant_with_ttl(path, ttl)?;
        Ok(OpenedSqlFile {
            token,
            file_name,
            content,
        })
    }

    pub fn overwrite_sql_file(&self, token: &str, content: &str) -> AppResult<SavedSqlFile> {
        Self::validate_sql_content(content)?;
        let path = self.resolve_sql_file_grant(token)?;
        let file_name = Self::write_sql_file_atomic(&path, content)?;
        Ok(SavedSqlFile {
            token: token.to_owned(),
            file_name,
        })
    }

    pub fn save_new_sql_file(&self, path: PathBuf, content: &str) -> AppResult<SavedSqlFile> {
        Self::validate_sql_content(content)?;
        let token = self.mint_sql_file_grant(path.clone())?;
        match Self::write_sql_file_atomic(&path, content) {
            Ok(file_name) => Ok(SavedSqlFile { token, file_name }),
            Err(err) => {
                self.release_sql_file_grant(&token);
                Err(err)
            }
        }
    }

    pub fn release_sql_file_grant(&self, token: &str) {
        let mut grants = self.grants.lock();
        if matches!(
            grants.get(token).map(|g| &g.kind),
            Some(GrantKind::SqlFile { .. })
        ) {
            grants.remove(token);
        }
    }

    pub fn clear(&self) {
        self.grants.lock().clear();
    }

    fn insert_grant(&self, kind: GrantKind, deadline: Instant) -> AppResult<String> {
        let now = Instant::now();
        let mut grants = self.grants.lock();
        grants.retain(|_, g| g.deadline > now);
        if matches!(kind, GrantKind::SqlFile { .. }) {
            let sql_count = grants
                .values()
                .filter(|g| matches!(g.kind, GrantKind::SqlFile { .. }))
                .count();
            if sql_count >= MAX_SQL_FILE_GRANTS {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "Too many SQL files are open. Close a SQL file tab first.",
                ));
            }
        } else {
            let io_count = grants
                .values()
                .filter(|g| !matches!(g.kind, GrantKind::SqlFile { .. }))
                .count();
            if io_count >= MAX_FILE_GRANTS {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "Too many file selections are open. Close an export or import dialog first.",
                ));
            }
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

    #[test]
    fn sql_file_capabilities_round_trip_enforce_limits_reject_symlinks_and_hide_paths() {
        let dir = tempfile::tempdir().unwrap();
        let caps = FileCapabilities::default();
        let dir_str = dir.path().to_string_lossy().to_string();

        let script_path = dir.path().join("report.sql");
        std::fs::write(&script_path, "SELECT 1 AS ok;\n").unwrap();

        let opened = caps.open_sql_file(script_path.clone()).unwrap();
        assert_eq!(opened.file_name, "report.sql");
        assert_eq!(opened.content, "SELECT 1 AS ok;\n");
        assert!(!opened.token.contains(&dir_str));
        assert!(!opened.file_name.contains('/'));

        // Cross-capability isolation: SqlFile token cannot be consumed as Write or Read grant
        assert_eq!(
            caps.consume_write_grant(&opened.token).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            caps.resolve_read_grant(&opened.token).unwrap_err().code,
            ErrorCode::InvalidInput
        );

        // Reusable atomic Save overwrites target without leaving .tmp sibling files
        let saved = caps
            .overwrite_sql_file(&opened.token, "SELECT 2 AS updated;\n")
            .unwrap();
        assert_eq!(saved.token, opened.token);
        assert_eq!(saved.file_name, "report.sql");
        assert_eq!(
            std::fs::read_to_string(&script_path).unwrap(),
            "SELECT 2 AS updated;\n"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");

        // Save As mints a distinct token; releasing the original token invalidates only the old one
        let copy_path = dir.path().join("report-copy.sql");
        let saved_as = caps
            .save_new_sql_file(copy_path.clone(), "SELECT 3 AS copy;\n")
            .unwrap();
        assert_ne!(saved_as.token, opened.token);
        assert_eq!(saved_as.file_name, "report-copy.sql");
        assert_eq!(
            std::fs::read_to_string(&copy_path).unwrap(),
            "SELECT 3 AS copy;\n"
        );

        caps.release_sql_file_grant(&opened.token);
        assert_eq!(
            caps.overwrite_sql_file(&opened.token, "SELECT 99;")
                .unwrap_err()
                .code,
            ErrorCode::ApprovalExpired
        );
        assert!(
            caps.overwrite_sql_file(&saved_as.token, "SELECT 4 AS still_valid;\n")
                .is_ok()
        );

        // Expired grant refuses overwrite cleanly
        let expired = caps
            .open_sql_file_with_ttl(copy_path.clone(), Duration::ZERO)
            .unwrap();
        assert_eq!(
            caps.overwrite_sql_file(&expired.token, "SELECT 0;")
                .unwrap_err()
                .code,
            ErrorCode::ApprovalExpired
        );

        // > 4 MiB rejected on both read and write
        let big_path = dir.path().join("huge.sql");
        std::fs::write(&big_path, vec![b'a'; MAX_SQL_FILE_BYTES + 1]).unwrap();
        let big_err = caps.open_sql_file(big_path).unwrap_err();
        assert_eq!(big_err.code, ErrorCode::ResultLimitReached);
        assert!(!big_err.message.contains(&dir_str));
        let big_write = "x".repeat(MAX_SQL_FILE_BYTES + 1);
        assert_eq!(
            caps.overwrite_sql_file(&saved_as.token, &big_write)
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );

        // NUL binary and invalid UTF-8 rejected without leaking paths
        let nul_path = dir.path().join("binary.sql");
        std::fs::write(&nul_path, b"SELECT 1;\0DROP TABLE x;").unwrap();
        let nul_err = caps.open_sql_file(nul_path).unwrap_err();
        assert_eq!(nul_err.code, ErrorCode::InvalidInput);
        assert!(!nul_err.message.contains(&dir_str));

        let bad_utf8_path = dir.path().join("latin1.sql");
        std::fs::write(&bad_utf8_path, [0xFF, 0xFE, b'S', b'E', b'L']).unwrap();
        let utf8_err = caps.open_sql_file(bad_utf8_path).unwrap_err();
        assert_eq!(utf8_err.code, ErrorCode::InvalidInput);
        assert!(!utf8_err.message.contains(&dir_str));

        // Symbolic links rejected for both open and save
        #[cfg(unix)]
        {
            let link_path = dir.path().join("symlink.sql");
            std::os::unix::fs::symlink(&script_path, &link_path).unwrap();
            assert_eq!(
                caps.open_sql_file(link_path.clone()).unwrap_err().code,
                ErrorCode::InvalidInput
            );
            assert_eq!(
                caps.save_new_sql_file(link_path, "SELECT 1;")
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput
            );
        }
    }
}
