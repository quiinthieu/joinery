use crate::domain::*;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub struct Persistence {
    connection: Mutex<Connection>,
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
        let connection = Connection::open(&path).map_err(db_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| AppError::internal("Could not protect application database"))?;
        }
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS profiles(id TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS groups(id TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS history(id TEXT PRIMARY KEY, started_at INTEGER NOT NULL, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS audit(id TEXT PRIMARY KEY, occurred_at INTEGER NOT NULL, connection_id TEXT NOT NULL, action TEXT NOT NULL, digest TEXT NOT NULL, outcome TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             PRAGMA user_version=1;"
        ).map_err(db_error)?;
        let store = Self {
            connection: Mutex::new(connection),
        };
        if store.groups()?.is_empty() {
            for (id, name, environment) in [
                ("production", "Production", Environment::Production),
                ("staging", "Staging", Environment::Staging),
                ("local", "Local", Environment::Local),
            ] {
                store.save_group(&ConnectionGroup {
                    id: id.into(),
                    name: name.into(),
                    parent_id: None,
                    environment,
                })?;
            }
        }
        Ok(store)
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
        self.connection.lock().execute("INSERT INTO profiles(id,json) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", params![profile.id,json]).map_err(db_error)?;
        Ok(())
    }

    pub fn delete_profile(&self, id: &str) -> AppResult<()> {
        self.connection
            .lock()
            .execute("DELETE FROM profiles WHERE id=?1", [id])
            .map_err(db_error)?;
        Ok(())
    }

    pub fn save_group(&self, group: &ConnectionGroup) -> AppResult<()> {
        let json = serde_json::to_string(group)
            .map_err(|_| AppError::internal("Could not encode group"))?;
        self.connection.lock().execute("INSERT INTO groups(id,json) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", params![group.id,json]).map_err(db_error)?;
        Ok(())
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

fn db_error(_: rusqlite::Error) -> AppError {
    AppError::internal("Could not read or write the local application database")
}

#[cfg(test)]
mod tests {
    use super::*;

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
