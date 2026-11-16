use super::{Persistence, db_error, metadata_table};
use crate::domain::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};

pub const MAX_LOCAL_HISTORY_ENTRIES: usize = 200;
pub const MAX_LOCAL_HISTORY_PER_TAB: usize = 30;
pub const MAX_LOCAL_HISTORY_SQL_BYTES: usize = 128 * 1024;
pub const MAX_LOCAL_HISTORY_TOTAL_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LOCAL_HISTORY_TAB_KEY_CHARS: usize = 160;
pub const MAX_LOCAL_HISTORY_TITLE_CHARS: usize = 160;

fn clean_field(raw: &str, max_chars: usize, name: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    let count = trimmed.chars().count();
    if trimmed.is_empty() || count > max_chars || trimmed.chars().any(char::is_control) {
        return Err(AppError::invalid(format!(
            "Provide a {name} between 1 and {max_chars} characters without control characters"
        )));
    }
    Ok(trimmed.to_owned())
}

fn validate_entries(items: &[LocalHistoryEntry]) -> AppResult<()> {
    if items.len() > MAX_LOCAL_HISTORY_ENTRIES {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            "Local history exceeds 200 entries",
        ));
    }
    let mut ids = HashSet::with_capacity(items.len());
    let mut per_tab: HashMap<&str, usize> = HashMap::new();
    let mut total_bytes = 0usize;

    for item in items {
        let key_chars = item.tab_key.trim().chars().count();
        let title_chars = item.title.trim().chars().count();
        if uuid::Uuid::parse_str(&item.id).is_err()
            || !ids.insert(item.id.as_str())
            || item.tab_key.trim().is_empty()
            || key_chars > MAX_LOCAL_HISTORY_TAB_KEY_CHARS
            || item.tab_key.chars().any(char::is_control)
            || item.title.trim().is_empty()
            || title_chars > MAX_LOCAL_HISTORY_TITLE_CHARS
            || item.title.chars().any(char::is_control)
            || item.sql.trim().is_empty()
            || item.sql.contains('\0')
            || item
                .connection_id
                .as_ref()
                .is_some_and(|cid| uuid::Uuid::parse_str(cid).is_err())
        {
            return Err(AppError::invalid("A local history record is invalid"));
        }
        if item.sql.len() > MAX_LOCAL_HISTORY_SQL_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Each local history revision is limited to 128 KiB",
            ));
        }
        let tab_count = per_tab.entry(item.tab_key.as_str()).or_insert(0);
        *tab_count += 1;
        if *tab_count > MAX_LOCAL_HISTORY_PER_TAB {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Each tab is limited to 30 local history revisions",
            ));
        }
        total_bytes = total_bytes
            .saturating_add(item.sql.len())
            .saturating_add(item.title.len())
            .saturating_add(item.tab_key.len());
        if total_bytes > MAX_LOCAL_HISTORY_TOTAL_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Local history is limited to 2 MiB in total",
            ));
        }
    }
    Ok(())
}

pub(super) fn read_entries(connection: &Connection) -> AppResult<Vec<LocalHistoryEntry>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key='local-history'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    let Some(json) = json else {
        return Ok(Vec::new());
    };
    if json.len() > MAX_LOCAL_HISTORY_TOTAL_BYTES * 4 + 65_536 {
        return Err(AppError::invalid("Local history metadata is too large"));
    }
    let items: Vec<LocalHistoryEntry> =
        serde_json::from_str(&json).map_err(|_| AppError::invalid("Local history is invalid"))?;
    validate_entries(&items)?;
    Ok(items)
}

fn write_entries(connection: &Connection, items: &[LocalHistoryEntry]) -> AppResult<()> {
    validate_entries(items)?;
    let value = serde_json::to_string(items)
        .map_err(|_| AppError::internal("Could not encode local history"))?;
    connection
        .execute(
            "INSERT INTO settings(key,value) VALUES('local-history',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![value],
        )
        .map_err(db_error)?;
    Ok(())
}

fn load_profile(connection: &Connection, id: &str) -> AppResult<Option<ConnectionProfile>> {
    let raw: Option<String> = connection
        .query_row(
            &format!(
                "SELECT json FROM {} WHERE id=?1",
                metadata_table(connection, "profiles")?
            ),
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    raw.map(|s| {
        serde_json::from_str::<ConnectionProfile>(&s)
            .map_err(|_| AppError::internal("Application profile data is invalid"))
    })
    .transpose()
}

pub(super) fn enforce_profile_privacy(
    connection: &Connection,
    profile: &ConnectionProfile,
) -> AppResult<()> {
    let reviewed_console = profile
        .snowflake
        .as_ref()
        .is_some_and(|sf| sf.reviewed_console);
    if !reviewed_console && profile.store_query_text && profile.store_drafts {
        return Ok(());
    }
    let mut items = read_entries(connection)?;
    let before = items.len();
    items.retain(|entry| {
        if entry.connection_id.as_deref() != Some(profile.id.as_str()) {
            return true;
        }
        if reviewed_console {
            return false;
        }
        match entry.trigger {
            LocalHistoryTrigger::Executed => profile.store_query_text,
            LocalHistoryTrigger::Snapshot | LocalHistoryTrigger::Saved => profile.store_drafts,
        }
    });
    if items.len() != before {
        write_entries(connection, &items)?;
    }
    Ok(())
}

pub(super) fn purge_connection(connection: &Connection, connection_id: &str) -> AppResult<()> {
    let mut items = read_entries(connection)?;
    let before = items.len();
    items.retain(|e| e.connection_id.as_deref() != Some(connection_id));
    if items.len() != before {
        write_entries(connection, &items)?;
    }
    Ok(())
}

pub(super) fn clear_all(connection: &Connection) -> AppResult<()> {
    connection
        .execute("DELETE FROM settings WHERE key='local-history'", [])
        .map_err(db_error)?;
    Ok(())
}

fn entry_charge(entry: &LocalHistoryEntry) -> usize {
    entry
        .sql
        .len()
        .saturating_add(entry.title.len())
        .saturating_add(entry.tab_key.len())
}

impl Persistence {
    pub fn local_history_entries(
        &self,
        tab_key: Option<&str>,
    ) -> AppResult<Vec<LocalHistoryEntry>> {
        let mut items = read_entries(&self.connection.lock())?;
        if let Some(target) = tab_key.map(str::trim).filter(|s| !s.is_empty()) {
            items.retain(|e| e.tab_key == target);
        }
        items.sort_by(|a, b| {
            b.created_at_ms
                .cmp(&a.created_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(items)
    }

    pub fn record_local_history(
        &self,
        input: LocalHistoryInput,
    ) -> AppResult<Option<LocalHistoryEntry>> {
        let tab_key = clean_field(&input.tab_key, MAX_LOCAL_HISTORY_TAB_KEY_CHARS, "tab key")?;
        let title = clean_field(
            &input.title,
            MAX_LOCAL_HISTORY_TITLE_CHARS,
            "snapshot title",
        )?;
        if input.sql.trim().is_empty() || input.sql.contains('\0') {
            return Err(AppError::invalid(
                "Provide non-empty UTF-8 SQL to record in local history",
            ));
        }
        if input.sql.len() > MAX_LOCAL_HISTORY_SQL_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Each local history revision is limited to 128 KiB",
            ));
        }
        let connection_id = input
            .connection_id
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty());
        if let Some(ref cid) = connection_id
            && uuid::Uuid::parse_str(cid).is_err()
        {
            return Err(AppError::invalid("Invalid connection identifier"));
        }

        let mut conn = self.connection.lock();
        let tx = conn.transaction().map_err(db_error)?;
        if let Some(ref cid) = connection_id {
            let Some(profile) = load_profile(&tx, cid)? else {
                return Err(AppError::new(
                    ErrorCode::NotFound,
                    "Connection profile no longer exists",
                ));
            };
            if profile
                .snowflake
                .as_ref()
                .is_some_and(|sf| sf.reviewed_console)
            {
                return Ok(None);
            }
            let allowed = match input.trigger {
                LocalHistoryTrigger::Executed => profile.store_query_text,
                LocalHistoryTrigger::Snapshot | LocalHistoryTrigger::Saved => profile.store_drafts,
            };
            if !allowed {
                return Ok(None);
            }
        } else if input.trigger != LocalHistoryTrigger::Snapshot {
            return Ok(None);
        }

        let mut items = read_entries(&tx)?;
        items.sort_by(|a, b| {
            b.created_at_ms
                .cmp(&a.created_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });

        let created_at_ms = now_ms();
        if let Some(latest_for_tab) = items.iter_mut().find(|e| e.tab_key == tab_key)
            && latest_for_tab.sql == input.sql
        {
            latest_for_tab.created_at_ms = created_at_ms;
            latest_for_tab.title = title;
            latest_for_tab.trigger = input.trigger;
            latest_for_tab.connection_id = connection_id;
            let updated = latest_for_tab.clone();
            items.sort_by(|a, b| {
                b.created_at_ms
                    .cmp(&a.created_at_ms)
                    .then_with(|| a.id.cmp(&b.id))
            });
            write_entries(&tx, &items)?;
            tx.commit().map_err(db_error)?;
            return Ok(Some(updated));
        }

        let new_entry = LocalHistoryEntry {
            id: new_id(),
            tab_key: tab_key.clone(),
            title,
            connection_id,
            trigger: input.trigger,
            sql: input.sql,
            created_at_ms,
        };
        items.insert(0, new_entry.clone());

        // Enforce per-tab limit (newest first).
        let mut seen_per_tab: HashMap<String, usize> = HashMap::new();
        items.retain(|entry| {
            let c = seen_per_tab.entry(entry.tab_key.clone()).or_insert(0);
            *c += 1;
            *c <= MAX_LOCAL_HISTORY_PER_TAB
        });

        // Enforce global count limit.
        if items.len() > MAX_LOCAL_HISTORY_ENTRIES {
            items.truncate(MAX_LOCAL_HISTORY_ENTRIES);
        }

        // Enforce total byte budget by dropping oldest entries from the tail.
        let mut total_bytes: usize = items.iter().map(entry_charge).sum();
        while total_bytes > MAX_LOCAL_HISTORY_TOTAL_BYTES && items.len() > 1 {
            if let Some(removed) = items.pop() {
                total_bytes = total_bytes.saturating_sub(entry_charge(&removed));
            }
        }

        write_entries(&tx, &items)?;
        tx.commit().map_err(db_error)?;
        Ok(Some(new_entry))
    }

    pub fn delete_local_history_entry(&self, id: &str) -> AppResult<()> {
        let mut conn = self.connection.lock();
        let tx = conn.transaction().map_err(db_error)?;
        let mut items = read_entries(&tx)?;
        let before = items.len();
        items.retain(|e| e.id != id);
        if items.len() == before {
            return Err(AppError::new(
                ErrorCode::NotFound,
                "Local history entry no longer exists",
            ));
        }
        write_entries(&tx, &items)?;
        tx.commit().map_err(db_error)
    }

    pub fn clear_local_history(&self, tab_key: Option<&str>) -> AppResult<()> {
        let mut conn = self.connection.lock();
        let tx = conn.transaction().map_err(db_error)?;
        if let Some(target) = tab_key.map(str::trim).filter(|s| !s.is_empty()) {
            let mut items = read_entries(&tx)?;
            items.retain(|e| e.tab_key != target);
            write_entries(&tx, &items)?;
        } else {
            clear_all(&tx)?;
        }
        tx.commit().map_err(db_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile(store_query_text: bool, store_drafts: bool) -> ConnectionProfile {
        ConnectionProfile {
            id: new_id(),
            name: "Local PG".into(),
            group_id: "local".into(),
            engine: EngineKind::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            database: "joinery".into(),
            username: "joinery".into(),
            environment: Environment::Local,
            safe_mode: SafeMode::Confirm,
            tls_mode: TlsMode::Disable,
            ca_cert_path: None,
            ssh: None,
            remember_password: false,
            store_query_text,
            store_drafts,
            snowflake: None,
        }
    }

    #[test]
    fn enforces_privacy_flags_deduplicates_evicts_and_purges_on_profile_and_history_clear() {
        let dir = tempfile::tempdir().unwrap();
        let store = Persistence::open(dir.path()).unwrap();

        let private_prof = make_profile(false, false);
        let normal_prof = make_profile(true, true);
        store.save_profile(&private_prof).unwrap();
        store.save_profile(&normal_prof).unwrap();

        // Private profile must never persist SQL text to local history.
        let skipped = store
            .record_local_history(LocalHistoryInput {
                tab_key: "tab-private".into(),
                title: "Secret Console".into(),
                connection_id: Some(private_prof.id.clone()),
                trigger: LocalHistoryTrigger::Executed,
                sql: "SELECT 'top-secret-marker';".into(),
            })
            .unwrap();
        assert!(skipped.is_none());
        assert!(store.local_history_entries(None).unwrap().is_empty());

        // Normal profile records, deduplicates consecutive identical SQL, and caps at 30 per tab.
        let first = store
            .record_local_history(LocalHistoryInput {
                tab_key: "tab-1".into(),
                title: "Query 1".into(),
                connection_id: Some(normal_prof.id.clone()),
                trigger: LocalHistoryTrigger::Snapshot,
                sql: "SELECT 1;".into(),
            })
            .unwrap()
            .unwrap();
        let deduped = store
            .record_local_history(LocalHistoryInput {
                tab_key: "tab-1".into(),
                title: "Query 1".into(),
                connection_id: Some(normal_prof.id.clone()),
                trigger: LocalHistoryTrigger::Executed,
                sql: "SELECT 1;".into(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(first.id, deduped.id);
        assert_eq!(deduped.trigger, LocalHistoryTrigger::Executed);
        assert_eq!(store.local_history_entries(Some("tab-1")).unwrap().len(), 1);

        for i in 2..=35 {
            store
                .record_local_history(LocalHistoryInput {
                    tab_key: "tab-1".into(),
                    title: "Query 1".into(),
                    connection_id: Some(normal_prof.id.clone()),
                    trigger: LocalHistoryTrigger::Snapshot,
                    sql: format!("SELECT {i};"),
                })
                .unwrap();
        }
        assert_eq!(
            store.local_history_entries(Some("tab-1")).unwrap().len(),
            MAX_LOCAL_HISTORY_PER_TAB
        );

        // Turning off store_drafts while keeping store_query_text purges Snapshot/Saved revisions
        // while retaining Executed revisions; turning off store_query_text purges the rest.
        store
            .record_local_history(LocalHistoryInput {
                tab_key: "tab-exec".into(),
                title: "Executed Query".into(),
                connection_id: Some(normal_prof.id.clone()),
                trigger: LocalHistoryTrigger::Executed,
                sql: "SELECT 'exec';".into(),
            })
            .unwrap()
            .unwrap();
        let mut no_drafts = normal_prof.clone();
        no_drafts.store_drafts = false;
        store.save_profile(&no_drafts).unwrap();
        let after_no_drafts = store.local_history_entries(None).unwrap();
        assert_eq!(after_no_drafts.len(), 1);
        assert_eq!(after_no_drafts[0].trigger, LocalHistoryTrigger::Executed);
        // Recording a Snapshot when store_drafts=false is rejected safely.
        assert!(
            store
                .record_local_history(LocalHistoryInput {
                    tab_key: "tab-1".into(),
                    title: "Query 1".into(),
                    connection_id: Some(normal_prof.id.clone()),
                    trigger: LocalHistoryTrigger::Snapshot,
                    sql: "SELECT 'blocked-draft';".into(),
                })
                .unwrap()
                .is_none()
        );

        let mut locked_down = no_drafts;
        locked_down.store_query_text = false;
        store.save_profile(&locked_down).unwrap();
        assert!(store.local_history_entries(None).unwrap().is_empty());

        // Unconnected automatic file save is never persisted; explicit manual Snapshot is cleared on clear_history().
        assert!(
            store
                .record_local_history(LocalHistoryInput {
                    tab_key: "file:report.sql".into(),
                    title: "report.sql".into(),
                    connection_id: None,
                    trigger: LocalHistoryTrigger::Saved,
                    sql: "SELECT 'unconnected-file';".into(),
                })
                .unwrap()
                .is_none()
        );
        store
            .record_local_history(LocalHistoryInput {
                tab_key: "tab:scratch".into(),
                title: "Scratch".into(),
                connection_id: None,
                trigger: LocalHistoryTrigger::Snapshot,
                sql: "SELECT 42;".into(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(store.local_history_entries(None).unwrap().len(), 1);
        store.clear_history().unwrap();
        assert!(store.local_history_entries(None).unwrap().is_empty());

        store
            .set_setting("local-history", r#"[{"corrupt":true}]"#)
            .unwrap();
        assert_eq!(
            store.create_metadata_backup().unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }
}
