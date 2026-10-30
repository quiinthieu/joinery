use crate::domain::*;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::{Path, PathBuf};

mod recovery;
mod workspace;

pub struct Persistence {
    connection: Mutex<Connection>,
    root: PathBuf,
}

impl Persistence {
    pub fn open(root: &Path) -> AppResult<Self> {
        std::fs::create_dir_all(root)
            .map_err(|_| AppError::internal("Could not create application data directory"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| AppError::internal("Could not protect application data directory"))?;
        }
        let path = root.join("joinery.sqlite");
        let mut connection = Connection::open(&path).map_err(db_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| AppError::internal("Could not protect application database"))?;
        }
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        initialize_schema(&mut connection)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
            )
            .map_err(db_error)?;
        Ok(Self {
            connection: Mutex::new(connection),
            root: std::fs::canonicalize(root)
                .map_err(|_| AppError::internal("Could not resolve application data directory"))?,
        })
    }

    pub fn profiles(&self) -> AppResult<Vec<ConnectionProfile>> {
        self.json_rows("SELECT json FROM profiles ORDER BY id")
    }
    pub fn groups(&self) -> AppResult<Vec<ConnectionGroup>> {
        self.json_rows("SELECT json FROM groups ORDER BY id")
    }

    fn json_rows<T: serde::de::DeserializeOwned>(&self, sql: &str) -> AppResult<Vec<T>> {
        let conn = self.connection.lock();
        let mut statement = conn.prepare(sql).map_err(db_error)?;
        let rows = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?;
        rows.map(|row| {
            serde_json::from_str(&row.map_err(db_error)?)
                .map_err(|_| AppError::internal("Application data is invalid"))
        })
        .collect()
    }

    pub fn profile(&self, id: &str) -> AppResult<ConnectionProfile> {
        self.profiles()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "Connection profile no longer exists")
            })
    }

    pub fn save_profile(&self, profile: &ConnectionProfile) -> AppResult<()> {
        let json = serde_json::to_string(profile)
            .map_err(|_| AppError::internal("Could not encode profile"))?;
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        tx.execute("INSERT INTO profiles(id,json) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", params![profile.id,json]).map_err(db_error)?;
        if !profile.store_drafts {
            workspace::purge_drafts(&tx, &profile.id)?;
        }
        tx.commit().map_err(db_error)
    }

    pub fn delete_profile(&self, id: &str) -> AppResult<()> {
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        tx.execute("DELETE FROM profiles WHERE id=?1", [id])
            .map_err(db_error)?;
        workspace::purge_connection(&tx, id)?;
        tx.commit().map_err(db_error)
    }

    pub fn save_group(&self, group: &ConnectionGroup) -> AppResult<()> {
        let json = serde_json::to_string(group)
            .map_err(|_| AppError::internal("Could not encode group"))?;
        self.connection.lock().execute("INSERT INTO groups(id,json) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", params![group.id,json]).map_err(db_error)?;
        Ok(())
    }

    pub fn delete_group(&self, id: &str) -> AppResult<()> {
        self.connection
            .lock()
            .execute("DELETE FROM groups WHERE id=?1", [id])
            .map_err(db_error)?;
        Ok(())
    }

    pub fn audit_log(&self) -> AppResult<Vec<AuditEntry>> {
        let conn = self.connection.lock();
        let mut statement = conn.prepare("SELECT id,occurred_at,connection_id,action,digest,outcome FROM audit ORDER BY occurred_at DESC LIMIT 1000").map_err(db_error)?;
        statement
            .query_map([], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    occurred_at: row.get::<_, i64>(1)? as u64,
                    connection_id: row.get(2)?,
                    action: row.get(3)?,
                    digest: row.get(4)?,
                    outcome: row.get(5)?,
                })
            })
            .map_err(db_error)?
            .map(|r| r.map_err(db_error))
            .collect()
    }

    pub fn setting(&self, key: &str) -> AppResult<Option<String>> {
        self.connection
            .lock()
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_error)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.connection.lock().execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(db_error)?;
        Ok(())
    }

    pub fn record_history(&self, entry: &HistoryEntry) -> AppResult<()> {
        let mut conn = self.connection.lock();
        let tx = conn.transaction().map_err(db_error)?;
        let json = serde_json::to_string(entry)
            .map_err(|_| AppError::internal("Could not encode history"))?;
        tx.execute(
            "INSERT INTO history(id,started_at,json) VALUES(?1,?2,?3)",
            params![entry.id, entry.started_at as i64, json],
        )
        .map_err(db_error)?;
        let cutoff = now_ms().saturating_sub(30 * 24 * 60 * 60 * 1000);
        tx.execute("DELETE FROM history WHERE started_at < ?1 OR id IN (SELECT id FROM history ORDER BY started_at DESC LIMIT -1 OFFSET 10000)", [cutoff as i64]).map_err(db_error)?;
        tx.commit().map_err(db_error)
    }

    pub fn history(&self, search: &str) -> AppResult<Vec<HistoryEntry>> {
        let entries: Vec<HistoryEntry> =
            self.json_rows("SELECT json FROM history ORDER BY started_at DESC LIMIT 1000")?;
        let search = search.to_lowercase();
        Ok(entries
            .into_iter()
            .filter(|e| {
                search.is_empty()
                    || e.connection_name.to_lowercase().contains(&search)
                    || e.sql
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&search)
            })
            .collect())
    }

    pub fn clear_history(&self) -> AppResult<()> {
        self.connection
            .lock()
            .execute("DELETE FROM history", [])
            .map_err(db_error)?;
        Ok(())
    }

    pub fn audit(
        &self,
        connection_id: &str,
        action: &str,
        digest: &str,
        outcome: &str,
    ) -> AppResult<()> {
        self.connection
            .lock()
            .execute(
                "INSERT INTO audit VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    new_id(),
                    now_ms() as i64,
                    connection_id,
                    action,
                    digest,
                    outcome
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
}

const SCHEMA_VERSION: i64 = 1;

fn schema_version(connection: &Connection) -> AppResult<i64> {
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(db_error)?;
    if !(0..=SCHEMA_VERSION).contains(&version) {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "This local database belongs to a newer or unsupported Joinery version. Open it with the matching app; the existing database has been preserved.",
        ));
    }
    Ok(version)
}

fn initialize_schema(connection: &mut Connection) -> AppResult<()> {
    // Check before persistent pragmas and again under the writer lock. A second
    // app may have migrated between the first check and transaction acquisition.
    schema_version(connection)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let version = schema_version(&transaction)?;
    if version == 0 {
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS profiles(id TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS groups(id TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS history(id TEXT PRIMARY KEY, started_at INTEGER NOT NULL, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS audit(id TEXT PRIMARY KEY, occurred_at INTEGER NOT NULL, connection_id TEXT NOT NULL, action TEXT NOT NULL, digest TEXT NOT NULL, outcome TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);"
        ).map_err(db_error)?;
    }
    validate_schema(&transaction)?;
    for (id, name, environment) in [
        ("production", "Production", Environment::Production),
        ("staging", "Staging", Environment::Staging),
        ("local", "Local", Environment::Local),
    ] {
        let group = ConnectionGroup {
            id: id.into(),
            name: name.into(),
            parent_id: None,
            environment,
        };
        let json = serde_json::to_string(&group)
            .map_err(|_| AppError::internal("Could not initialize environment groups"))?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO groups(id,json) VALUES(?1,?2)",
                params![id, json],
            )
            .map_err(db_error)?;
    }
    if version == 0 {
        transaction
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(db_error)?;
    }
    transaction.commit().map_err(db_error)
}

fn validate_schema(connection: &Connection) -> AppResult<()> {
    // Names alone are insufficient: the unique keys and null constraints are
    // required by INSERT OR IGNORE, profile replacement and metadata retention.
    type Column = (&'static str, &'static str, bool, bool);
    const TABLES: &[(&str, &[Column])] = &[
        (
            "profiles",
            &[("id", "TEXT", false, true), ("json", "TEXT", true, false)],
        ),
        (
            "groups",
            &[("id", "TEXT", false, true), ("json", "TEXT", true, false)],
        ),
        (
            "history",
            &[
                ("id", "TEXT", false, true),
                ("started_at", "INTEGER", true, false),
                ("json", "TEXT", true, false),
            ],
        ),
        (
            "audit",
            &[
                ("id", "TEXT", false, true),
                ("occurred_at", "INTEGER", true, false),
                ("connection_id", "TEXT", true, false),
                ("action", "TEXT", true, false),
                ("digest", "TEXT", true, false),
                ("outcome", "TEXT", true, false),
            ],
        ),
        (
            "settings",
            &[("key", "TEXT", false, true), ("value", "TEXT", true, false)],
        ),
    ];
    for (table, expected) in TABLES {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(db_error)?;
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?.to_ascii_uppercase(),
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(5)?,
                ))
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let expected = expected
            .iter()
            .map(|(name, kind, not_null, key)| {
                (name.to_string(), kind.to_string(), *not_null, *key)
            })
            .collect::<Vec<_>>();
        if columns != expected {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The local metadata schema is incomplete or incompatible. Keep the existing database and restore a compatible copy; Joinery has not reset its contents.",
            ));
        }
    }
    Ok(())
}

fn db_error(_: rusqlite::Error) -> AppError {
    AppError::internal("Could not read or write the local application database")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_metadata_versions_are_rejected_without_rewriting_the_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("joinery.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE future_metadata(value TEXT); INSERT INTO future_metadata VALUES('preserve'); PRAGMA user_version=99;").unwrap();
        drop(connection);
        let before = std::fs::read(&path).unwrap();
        let error = match Persistence::open(root.path()) {
            Err(e) => e,
            Ok(_) => panic!("An older app accepted a newer metadata schema"),
        };
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert!(error.message.contains("newer"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            99
        );
        assert_eq!(
            connection
                .query_row("SELECT value FROM future_metadata", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "preserve"
        );
    }

    #[test]
    fn failed_legacy_initialization_rolls_back_schema_and_keeps_existing_rows() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("joinery.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE profiles(id TEXT PRIMARY KEY, unexpected TEXT); INSERT INTO profiles VALUES('old','preserve');").unwrap();
        drop(connection);
        assert!(Persistence::open(root.path()).is_err());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT unexpected FROM profiles", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "preserve"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type='table'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn versioned_metadata_with_missing_tables_is_preserved_instead_of_reset() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("joinery.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO settings VALUES('marker','preserve'); PRAGMA user_version=1;").unwrap();
        drop(connection);
        let before = std::fs::read(&path).unwrap();
        let error = match Persistence::open(root.path()) {
            Err(e) => e,
            Ok(_) => panic!("The incomplete versioned store was silently reset"),
        };
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn partial_unversioned_metadata_and_renamed_groups_survive_initialization() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("joinery.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO settings VALUES('marker','preserve');").unwrap();
        drop(connection);
        let db = Persistence::open(root.path()).unwrap();
        assert_eq!(db.setting("marker").unwrap().as_deref(), Some("preserve"));
        let mut production = db
            .groups()
            .unwrap()
            .into_iter()
            .find(|g| g.id == "production")
            .unwrap();
        production.name = "Protected accounts".into();
        db.save_group(&production).unwrap();
        // Simulate the partial default-group seed left by an interrupted old build.
        db.connection
            .lock()
            .execute("DELETE FROM groups WHERE id IN ('staging','local')", [])
            .unwrap();
        drop(db);
        let db = Persistence::open(root.path()).unwrap();
        let groups = db.groups().unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(
            groups.iter().find(|g| g.id == "production").unwrap().name,
            "Protected accounts"
        );
        assert_eq!(db.setting("marker").unwrap().as_deref(), Some("preserve"));
        assert_eq!(
            db.connection
                .lock()
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
    }

    #[test]
    fn concurrent_app_starts_create_one_complete_metadata_store() {
        let root = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads = (0..4)
            .map(|_| {
                let root = root.path().to_path_buf();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let db = Persistence::open(&root).unwrap();
                    assert_eq!(db.groups().unwrap().len(), 3);
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(
            Persistence::open(root.path())
                .unwrap()
                .groups()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn legacy_tables_without_required_unique_keys_are_not_adopted() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("joinery.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE groups(id TEXT,json TEXT NOT NULL);")
            .unwrap();
        drop(connection);
        let before = std::fs::read(&path).unwrap();
        assert!(Persistence::open(root.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn initializes_groups_and_keeps_history_metadata_without_sql() {
        let dir = tempfile::tempdir().unwrap();
        let db = Persistence::open(dir.path()).unwrap();
        assert_eq!(db.groups().unwrap().len(), 3);
        db.record_history(&HistoryEntry {
            id: new_id(),
            connection_id: "p".into(),
            connection_name: "Production".into(),
            started_at: now_ms(),
            sql: None,
            statement_count: 1,
            status: RunStatus::Completed,
            elapsed_ms: 5,
        })
        .unwrap();
        assert!(db.history("").unwrap()[0].sql.is_none());
        assert_eq!(db.history("production").unwrap().len(), 1);
        drop(db);
        assert_eq!(
            Persistence::open(dir.path())
                .unwrap()
                .groups()
                .unwrap()
                .len(),
            3
        );
    }
}
