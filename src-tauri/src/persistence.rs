use crate::domain::*;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) mod history;
pub use history::{HistoryCompletion, HistoryDisposition, HistoryTicket};
mod history_owner;
pub(crate) use history_owner::{HistoryOwner, OperationGate};
mod outcomes;
pub(crate) use outcomes::{
    BoundaryEvidence, OriginalOutcome, OutcomeDescriptor, OutcomeKind, OutcomePool,
    OutcomeReservation, VolatileRollbackToken,
};
mod library;
mod recovery;
pub(crate) mod recovery_crypto;
pub(crate) mod recovery_stream;
pub(crate) mod snowflake_console;
mod workspace;

const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

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
            .busy_timeout(DATABASE_BUSY_TIMEOUT)
            .map_err(db_error)?;
        let root = std::fs::canonicalize(root)
            .map_err(|_| AppError::internal("Could not resolve application data directory"))?;
        initialize_schema(&mut connection, Some(&root))?;
        enable_wal(&connection, DATABASE_BUSY_TIMEOUT).map_err(db_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")
            .map_err(db_error)?;
        Ok(Self {
            connection: Mutex::new(connection),
            root,
        })
    }

    pub fn profiles(&self) -> AppResult<Vec<ConnectionProfile>> {
        self.json_rows("SELECT json FROM profiles_v7 ORDER BY id")
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
        let _gate = history_owner::AdmissionGate::claim(&self.root, false)?;
        let json = serde_json::to_string(profile)
            .map_err(|_| AppError::internal("Could not encode profile"))?;
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        tx.execute("INSERT INTO profiles_v7(id,json) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", params![profile.id,json]).map_err(db_error)?;
        history::lifecycle::ensure_profile(&tx, &profile.id)?;
        if !profile.store_query_text {
            history::lifecycle::redact_profile(&tx, &profile.id)?;
        }
        if !profile.store_drafts {
            workspace::purge_drafts(&tx, &profile.id)?;
        }
        tx.commit().map_err(db_error)
    }

    pub fn delete_profile(&self, id: &str) -> AppResult<()> {
        let _gate = history_owner::AdmissionGate::claim(&self.root, false)?;
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        let existing_json: Option<String> = tx
            .query_row("SELECT json FROM profiles_v7 WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_error)?;
        let purge_bound_queries = match existing_json {
            Some(raw) => serde_json::from_str::<ConnectionProfile>(&raw)
                .map(|p| p.environment == Environment::Production || !p.store_query_text)
                .unwrap_or(true),
            None => true,
        };
        tx.execute("DELETE FROM profiles_v7 WHERE id=?1", [id])
            .map_err(db_error)?;
        history::lifecycle::redact_profile(&tx, id)?;
        workspace::purge_connection(&tx, id)?;
        library::purge_connection(&tx, id, purge_bound_queries)?;
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
        let mut statement = conn.prepare("SELECT id,occurred_at,connection_id,action,digest,outcome FROM audit_v7 ORDER BY occurred_at DESC LIMIT 1000").map_err(db_error)?;
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

    /// Compatibility import for completed/recovered local metadata. Runtime
    /// execution registers its ticket before dispatch and never uses this path.
    pub fn record_history(&self, entry: &HistoryEntry) -> AppResult<()> {
        self.import_history(entry)
    }
    fn import_history(&self, entry: &HistoryEntry) -> AppResult<()> {
        let _gate = history_owner::AdmissionGate::claim(&self.root, false)?;
        entry.validate_lifecycle()?;
        if entry.provenance == HistoryProvenance::Admitted {
            return Err(AppError::invalid("Pending history requires admission"));
        }
        let mut seed = entry.clone();
        seed.provenance = HistoryProvenance::Admitted;
        seed.status = None;
        seed.elapsed_ms = None;
        seed.execution_summary = None;
        seed.snowflake_dml = None;
        seed.snowflake_ddl = None;
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let ticket = history::lifecycle::begin(&tx, &seed)?;
        history::lifecycle::finish(&tx, &ticket, &HistoryCompletion::from_entry(entry))?;
        tx.commit().map_err(db_error)
    }

    pub fn history(&self, search: &str) -> AppResult<Vec<HistoryEntry>> {
        history::compatibility_history(self, search)
    }

    pub fn clear_history(&self) -> AppResult<()> {
        let _gate = history_owner::AdmissionGate::claim(&self.root, false)?;
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        history::lifecycle::clear(&tx)?;
        tx.commit().map_err(db_error)
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
                "INSERT INTO audit_v7 VALUES(?1,?2,?3,?4,?5,?6)",
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

    #[cfg(test)]
    pub(crate) fn record_audit_once(&self, entry: &AuditEntry) -> AppResult<()> {
        let timestamp = i64::try_from(entry.occurred_at)
            .map_err(|_| AppError::invalid("Invalid audit timestamp"))?;
        let mut conn = self.connection.lock();
        let tx = conn.transaction().map_err(db_error)?;
        tx.execute(
            "INSERT INTO audit_v7(id,occurred_at,connection_id,action,digest,outcome) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO NOTHING",
            params![entry.id, timestamp, entry.connection_id, entry.action, entry.digest, entry.outcome],
        ).map_err(db_error)?;
        let stored: (i64, String, String, String, String) = tx
            .query_row(
                "SELECT occurred_at,connection_id,action,digest,outcome FROM audit_v7 WHERE id=?1",
                [&entry.id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(db_error)?;
        if stored
            != (
                timestamp,
                entry.connection_id.clone(),
                entry.action.clone(),
                entry.digest.clone(),
                entry.outcome.clone(),
            )
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Audit identity already contains different evidence",
            ));
        }
        tx.commit().map_err(db_error)
    }
}

fn enable_wal(connection: &Connection, budget: Duration) -> rusqlite::Result<()> {
    // WAL promotion needs a lock upgrade. SQLite may return BUSY immediately
    // when another opener holds a reserved writer lock, bypassing its busy
    // handler to avoid deadlock. Retry only this idempotent startup pragma,
    // after each failed statement has released its own read lock.
    let deadline = Instant::now() + budget;
    let result = (|| {
        loop {
            connection.busy_timeout(deadline.saturating_duration_since(Instant::now()))?;
            match connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))
            {
                Ok(mode) if mode.eq_ignore_ascii_case("wal") => return Ok(()),
                Ok(_) => return Err(rusqlite::Error::InvalidQuery),
                Err(error)
                    if matches!(&error, rusqlite::Error::SqliteFailure(error, _)
                        if error.code == rusqlite::ErrorCode::DatabaseBusy)
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(
                        Duration::from_millis(10)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                    if Instant::now() >= deadline {
                        return Err(error);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    })();
    // This temporary admission deadline must not shorten later application SQL.
    let restored = connection.busy_timeout(DATABASE_BUSY_TIMEOUT);
    result.and(restored)
}

const SCHEMA_VERSION: i64 = 7;

pub(super) fn metadata_table(connection: &Connection, base: &str) -> AppResult<&'static str> {
    let version = schema_version(connection)?;
    match (base, version) {
        ("profiles", 7..) => Ok("profiles_v7"),
        ("history", 7..) => Ok("history_v7"),
        ("audit", 7..) => Ok("audit_v7"),
        ("profiles", 6) => Ok("profiles_v6"),
        ("history", 6) => Ok("history_v6"),
        ("audit", 6) => Ok("audit_v6"),
        ("profiles", 5) => Ok("profiles_v5"),
        ("history", 5) => Ok("history_v5"),
        ("audit", 5) => Ok("audit_v5"),
        ("profiles", _) => Ok("profiles"),
        ("history", _) => Ok("history"),
        ("audit", _) => Ok("audit"),
        ("groups", _) => Ok("groups"),
        ("settings", _) => Ok("settings"),
        _ => Err(AppError::invalid("Unknown metadata table")),
    }
}

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

fn initialize_schema(connection: &mut Connection, migration_root: Option<&Path>) -> AppResult<()> {
    // Check before persistent pragmas and again under the writer lock. A second
    // app may have migrated between the first check and transaction acquisition.
    let initial_version = schema_version(connection)?;
    // Never acquire a SQLite writer before admission exclusion: a live
    // volatile rollback can hold SH while it waits for SQLite recording.
    let _migration_lease = if (2..SCHEMA_VERSION).contains(&initial_version) {
        migration_root
            .map(snowflake_console::claim_migration_lease)
            .transpose()?
    } else {
        None
    };
    let _migration_gate = if initial_version < SCHEMA_VERSION {
        migration_root
            .map(|root| history_owner::AdmissionGate::claim(root, true))
            .transpose()?
    } else {
        None
    };
    let _migration_owners = if initial_version < SCHEMA_VERSION {
        migration_root
            .map(history_owner::MigrationOwners::prove)
            .transpose()?
    } else {
        None
    };
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let version = schema_version(&transaction)?;
    // A live version-2 coordinator must finish recording before its physical
    // tables are renamed. Private backup copies need no external lease.
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
    if version < 2 {
        transaction
            .execute_batch(snowflake_console::SCHEMA_SQL)
            .map_err(db_error)?;
    }
    if version < 3 {
        transaction
            .execute_batch(
                "ALTER TABLE snowflake_console_runs RENAME TO snowflake_console_runs_v3;
             ALTER TABLE snowflake_console_steps RENAME TO snowflake_console_steps_v3;",
            )
            .map_err(db_error)?;
        transaction
            .execute_batch(snowflake_console::REJECTION_SCHEMA_SQL)
            .map_err(db_error)?;
        for (_, sql) in snowflake_console::DECISION_TRIGGERS {
            transaction.execute_batch(sql).map_err(db_error)?;
        }
    }
    if version < 4 {
        // Validate the complete legacy evidence before physically fencing old SQL.
        transaction
            .pragma_update(None, "user_version", 3)
            .map_err(db_error)?;
        snowflake_console::validate_schema(&transaction)?;
        snowflake_console::validate_records(&transaction)?;
        snowflake_console::migrate_v4(&transaction)?;
    }
    if version < 5 {
        transaction
            .pragma_update(None, "user_version", 4)
            .map_err(db_error)?;
        snowflake_console::migrate_v5(&transaction)?;
        transaction.execute_batch("ALTER TABLE profiles RENAME TO profiles_v5; ALTER TABLE history RENAME TO history_v5; ALTER TABLE audit RENAME TO audit_v5;").map_err(db_error)?;
        history::lifecycle::migrate(&transaction)?;
    }
    if version < 6 {
        transaction.execute_batch("ALTER TABLE profiles_v5 RENAME TO profiles_v6; ALTER TABLE history_v5 RENAME TO history_v6; ALTER TABLE audit_v5 RENAME TO audit_v6;").map_err(db_error)?;
        transaction
            .execute_batch(history_owner::SCHEMA)
            .map_err(db_error)?;
    }
    if version < 7 {
        transaction.execute_batch("ALTER TABLE profiles_v6 RENAME TO profiles_v7; ALTER TABLE history_v6 RENAME TO history_v7; ALTER TABLE audit_v6 RENAME TO audit_v7;").map_err(db_error)?;
        transaction
            .execute_batch(outcomes::SCHEMA)
            .map_err(db_error)?;
        transaction
            .execute_batch(outcomes::BINDING_SCHEMA)
            .map_err(db_error)?;
    }
    if version < SCHEMA_VERSION {
        transaction
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(db_error)?;
    }
    snowflake_console::validate_schema(&transaction)?;
    snowflake_console::validate_records(&transaction)?;
    history::validate_records(&transaction)?;
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
    for (base, expected) in TABLES {
        let table = metadata_table(connection, base)?;
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
    if schema_version(connection)? >= 2 {
        snowflake_console::validate_schema(connection)?;
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
    fn version_one_metadata_migrates_atomically_without_losing_settings_or_groups() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("joinery.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE profiles(id TEXT PRIMARY KEY,json TEXT NOT NULL);
             CREATE TABLE groups(id TEXT PRIMARY KEY,json TEXT NOT NULL);
             CREATE TABLE history(id TEXT PRIMARY KEY,started_at INTEGER NOT NULL,json TEXT NOT NULL);
             CREATE TABLE audit(id TEXT PRIMARY KEY,occurred_at INTEGER NOT NULL,connection_id TEXT NOT NULL,action TEXT NOT NULL,digest TEXT NOT NULL,outcome TEXT NOT NULL);
             CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             INSERT INTO settings VALUES('marker','preserved');
             PRAGMA user_version=1;",
        ).unwrap();
        let group = ConnectionGroup {
            id: "production".into(),
            name: "Existing protected accounts".into(),
            parent_id: None,
            environment: Environment::Production,
        };
        connection
            .execute(
                "INSERT INTO groups VALUES(?1,?2)",
                params![group.id, serde_json::to_string(&group).unwrap()],
            )
            .unwrap();
        drop(connection);

        let db = Persistence::open(root.path()).unwrap();
        assert_eq!(db.setting("marker").unwrap().as_deref(), Some("preserved"));
        assert!(db.groups().unwrap().iter().any(|g| g.name == group.name));
        let connection = db.connection.lock();
        assert_eq!(schema_version(&connection).unwrap(), SCHEMA_VERSION);
        snowflake_console::validate_schema(&connection).unwrap();
        snowflake_console::validate_records(&connection).unwrap();
    }

    #[test]
    fn current_metadata_missing_journal_is_rejected_without_recreating_it() {
        let root = tempfile::tempdir().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        db.set_setting("marker", "preserved").unwrap();
        db.connection
            .lock()
            .execute_batch("DROP TABLE snowflake_console_steps_v5;")
            .unwrap();
        drop(db);
        let path = root.path().join("joinery.sqlite");
        let before = std::fs::read(&path).unwrap();
        assert!(Persistence::open(root.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let connection = Connection::open(path).unwrap();
        assert_eq!(schema_version(&connection).unwrap(), SCHEMA_VERSION);
        assert_eq!(
            connection
                .query_row("SELECT value FROM settings WHERE key='marker'", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "preserved"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name='snowflake_console_steps_v5'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
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
                    Persistence::open(&root).map(|db| {
                        assert_eq!(db.groups().unwrap().len(), 3);
                        assert_eq!(schema_version(&db.connection.lock()).unwrap(), 7);
                    })
                })
            })
            .collect::<Vec<_>>();
        // Reap every opener before checking results or dropping the root.
        let outcomes = threads
            .into_iter()
            .map(|thread| thread.join())
            .collect::<Vec<_>>();
        assert!(outcomes.iter().all(Result::is_ok));
        let outcomes = outcomes.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        assert!(outcomes.iter().any(Result::is_ok));
        for result in outcomes {
            if let Err(error) = result {
                // Migration must refuse a contended EX gate before SQLite write.
                assert_eq!(error.code, ErrorCode::Conflict);
                assert!(error.message.contains("History ownership is busy"));
                let db = Persistence::open(root.path()).unwrap();
                assert_eq!(db.groups().unwrap().len(), 3);
                assert_eq!(schema_version(&db.connection.lock()).unwrap(), 7);
            }
        }
        let db = Persistence::open(root.path()).unwrap();
        assert_eq!(db.groups().unwrap().len(), 3);
        assert_eq!(schema_version(&db.connection.lock()).unwrap(), 7);
    }

    #[test]
    fn concurrent_current_schema_openers_all_succeed() {
        let root = tempfile::tempdir().unwrap();
        drop(Persistence::open(root.path()).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads = (0..4)
            .map(|_| {
                let root = root.path().to_path_buf();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let db = Persistence::open(&root).unwrap();
                    assert_eq!(schema_version(&db.connection.lock()).unwrap(), 7);
                    assert_eq!(db.groups().unwrap().len(), 3);
                })
            })
            .collect::<Vec<_>>();
        let outcomes = threads
            .into_iter()
            .map(|thread| thread.join())
            .collect::<Vec<_>>();
        assert!(outcomes.iter().all(Result::is_ok));
    }

    #[test]
    fn wal_admission_waits_for_a_reserved_writer_then_verifies_wal() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("reserved.sqlite");
        let owner = Connection::open(&path).unwrap();
        owner.execute_batch("CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES('preserve'); BEGIN IMMEDIATE;").unwrap();
        let (started, awaiting) = std::sync::mpsc::channel();
        let (finished, completed) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let connection = Connection::open(path).unwrap();
            started.send(()).unwrap();
            let result = enable_wal(&connection, Duration::from_secs(2));
            let mode =
                connection.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0));
            let timeout =
                connection.pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0));
            finished.send((result, mode, timeout)).unwrap();
        });
        awaiting.recv_timeout(Duration::from_secs(2)).unwrap();
        let early = completed.recv_timeout(Duration::from_millis(50));
        // Release and join even if the pre-fix one-shot PRAGMA has already
        // failed, retaining the fixture until every SQLite owner has finished.
        owner.execute_batch("ROLLBACK").unwrap();
        worker.join().unwrap();
        assert!(
            matches!(early, Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
            "WAL admission must wait for the reserved owner: {early:?}"
        );
        let (result, mode, timeout) = completed.recv_timeout(Duration::from_secs(1)).unwrap();
        result.unwrap();
        assert_eq!(mode.unwrap(), "wal");
        assert_eq!(timeout.unwrap(), 5000);
        assert_eq!(
            owner
                .query_row("SELECT value FROM marker", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "preserve"
        );
    }

    #[test]
    fn wal_admission_exhausts_its_budget_without_rewriting_contents() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("blocked.sqlite");
        let owner = Connection::open(&path).unwrap();
        owner.execute_batch("CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES('preserve'); BEGIN IMMEDIATE;").unwrap();
        let before = std::fs::read(&path).unwrap();
        let connection = Connection::open(&path).unwrap();
        let started = Instant::now();
        let error = enable_wal(&connection, Duration::from_millis(60)).unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(matches!(error, rusqlite::Error::SqliteFailure(error, _)
            if error.code == rusqlite::ErrorCode::DatabaseBusy));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            connection
                .pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0))
                .unwrap(),
            5000
        );
        owner.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                .unwrap(),
            "delete"
        );
        assert_eq!(
            connection
                .query_row("SELECT value FROM marker", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "preserve"
        );
    }

    #[test]
    fn wal_admission_does_not_retry_non_busy_errors_or_accept_non_wal_mode() {
        let connection = Connection::open_in_memory().unwrap();
        // In-memory SQLite reports MEMORY instead of WAL without an SQL error.
        assert!(matches!(
            enable_wal(&connection, Duration::from_secs(2)),
            Err(rusqlite::Error::InvalidQuery)
        ));
        let root = tempfile::tempdir().unwrap();
        let connection = Connection::open(root.path().join("transaction.sqlite")).unwrap();
        connection
            .execute_batch("CREATE TABLE marker(value TEXT); BEGIN; SELECT * FROM marker;")
            .unwrap();
        let started = Instant::now();
        let error = enable_wal(&connection, Duration::from_secs(2)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(matches!(error, rusqlite::Error::SqliteFailure(error, _)
            if error.code != rusqlite::ErrorCode::DatabaseBusy));
        assert!(
            !connection.is_autocommit(),
            "admission must not commit caller transactions"
        );
        connection.execute_batch("ROLLBACK").unwrap();
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
    fn legacy_history_has_unknown_execution_accounting_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = Persistence::open(dir.path()).unwrap();
        let started_at = now_ms();
        let legacy = serde_json::json!({
            "id": "legacy-script", "connectionId": "p",
            "connectionName": "Local fixture", "startedAt": started_at,
            "sql": null, "statementCount": 4, "status": "completed", "elapsedMs": 5
        });
        let imported: HistoryEntry = serde_json::from_value(legacy.clone()).unwrap();
        db.record_history(&imported).unwrap();
        db.connection
            .lock()
            .execute(
                "UPDATE history_v7 SET json=?1 WHERE id='legacy-script'",
                [legacy.to_string()],
            )
            .unwrap();
        drop(db);
        let restarted = Persistence::open(dir.path()).unwrap();
        let entries = restarted.history("").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].statement_count, 4);
        assert!(entries[0].execution_summary.is_none());
        assert!(
            !serde_json::to_value(&entries[0])
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("executionSummary")
        );
    }

    #[test]
    fn console_outbox_retries_preserve_exact_audit_and_history_across_restart() {
        let root = tempfile::tempdir().unwrap();
        let db = Persistence::open(root.path()).unwrap();
        let mut audit = AuditEntry {
            id: format!("{}:started", new_id()),
            occurred_at: now_ms(),
            connection_id: "synthetic".into(),
            action: "snowflake-console".into(),
            digest: "a".repeat(64),
            outcome: "started".into(),
        };
        let mut history = HistoryEntry {
            provenance: HistoryProvenance::Recorded,
            id: new_id(),
            connection_id: "synthetic".into(),
            connection_name: "Synthetic".into(),
            started_at: now_ms(),
            sql: None,
            statement_count: 2,
            status: Some(RunStatus::Failed),
            elapsed_ms: Some(7),
            execution_summary: None,
            snowflake_dml: None,
            snowflake_ddl: None,
        };
        db.record_audit_once(&audit).unwrap();
        db.record_history(&history).unwrap();
        drop(db);
        let db = Persistence::open(root.path()).unwrap();
        // Model a lost commit return: replay the same retained payload after
        // reopening SQLite, without appending or replacing evidence.
        db.record_audit_once(&audit).unwrap();
        db.record_history(&history).unwrap();
        assert_eq!(db.audit_log().unwrap().len(), 1);
        assert_eq!(db.history("").unwrap().len(), 1);
        audit.outcome = "completed".into();
        history.status = Some(RunStatus::Completed);
        assert_eq!(
            db.record_audit_once(&audit).unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            db.record_history(&history).unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(db.audit_log().unwrap()[0].outcome, "started");
        assert_eq!(db.history("").unwrap()[0].status, Some(RunStatus::Failed));
    }

    #[test]
    fn initializes_groups_and_keeps_history_metadata_without_sql() {
        let dir = tempfile::tempdir().unwrap();
        let db = Persistence::open(dir.path()).unwrap();
        assert_eq!(db.groups().unwrap().len(), 3);
        db.record_history(&HistoryEntry {
            provenance: HistoryProvenance::Recorded,
            execution_summary: None,
            snowflake_dml: None,
            snowflake_ddl: None,
            id: new_id(),
            connection_id: "p".into(),
            connection_name: "Production".into(),
            started_at: now_ms(),
            sql: None,
            statement_count: 1,
            status: Some(RunStatus::Completed),
            elapsed_ms: Some(5),
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
