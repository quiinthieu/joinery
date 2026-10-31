use super::{Persistence, db_error};
use crate::domain::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;

pub const MAX_SAVED_QUERIES: usize = 256;
pub const MAX_SAVED_QUERY_TITLE_CHARS: usize = 120;
pub const MAX_SAVED_QUERY_SQL_BYTES: usize = 256 * 1024;
pub const MAX_SAVED_QUERIES_TOTAL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_BOOKMARKS: usize = 256;
pub const MAX_BOOKMARK_LABEL_CHARS: usize = 160;
const MAX_OBJECT_IDENTIFIER_BYTES: usize = 256;
const MAX_OBJECT_IDENTITY_BYTES: usize = 512;

fn clean_single_line(raw: &str, max_chars: usize, field: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    let count = trimmed.chars().count();
    if trimmed.is_empty() || count > max_chars || trimmed.chars().any(char::is_control) {
        return Err(AppError::invalid(format!(
            "Provide a {field} between 1 and {max_chars} characters without control characters"
        )));
    }
    Ok(trimmed.to_owned())
}

fn valid_object_ref(object: &DatabaseObjectRef) -> bool {
    !object.schema.trim().is_empty()
        && object.schema.len() <= MAX_OBJECT_IDENTIFIER_BYTES
        && !object.schema.contains('\0')
        && !object.name.trim().is_empty()
        && object.name.len() <= MAX_OBJECT_IDENTIFIER_BYTES
        && !object.name.contains('\0')
        && object
            .identity
            .as_ref()
            .is_none_or(|ident| ident.len() <= MAX_OBJECT_IDENTITY_BYTES && !ident.contains('\0'))
}

fn validate_saved_queries(items: &[SavedQuery]) -> AppResult<()> {
    if items.len() > MAX_SAVED_QUERIES {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            "At most 256 saved queries may be stored",
        ));
    }
    let mut ids = HashSet::with_capacity(items.len());
    let mut total_bytes = 0usize;
    for item in items {
        let title_chars = item.title.trim().chars().count();
        if uuid::Uuid::parse_str(&item.id).is_err()
            || !ids.insert(item.id.as_str())
            || item.title.trim().is_empty()
            || title_chars > MAX_SAVED_QUERY_TITLE_CHARS
            || item.title.chars().any(char::is_control)
            || item.sql.trim().is_empty()
            || item.sql.contains('\0')
            || item
                .connection_id
                .as_ref()
                .is_some_and(|cid| uuid::Uuid::parse_str(cid).is_err())
        {
            return Err(AppError::invalid("A saved query record is invalid"));
        }
        if item.sql.len() > MAX_SAVED_QUERY_SQL_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Each saved query is limited to 256 KiB",
            ));
        }
        total_bytes = total_bytes
            .saturating_add(item.sql.len())
            .saturating_add(item.title.len());
        if total_bytes > MAX_SAVED_QUERIES_TOTAL_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Saved queries library is limited to 4 MiB in total",
            ));
        }
    }
    Ok(())
}

fn validate_bookmarks(items: &[SavedBookmark]) -> AppResult<()> {
    if items.len() > MAX_BOOKMARKS {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            "At most 256 bookmarks may be stored",
        ));
    }
    let mut ids = HashSet::with_capacity(items.len());
    for item in items {
        let label_chars = item.label.trim().chars().count();
        if uuid::Uuid::parse_str(&item.id).is_err()
            || !ids.insert(item.id.as_str())
            || item.label.trim().is_empty()
            || label_chars > MAX_BOOKMARK_LABEL_CHARS
            || item.label.chars().any(char::is_control)
        {
            return Err(AppError::invalid("A bookmark record is invalid"));
        }
        match &item.target {
            BookmarkTarget::Object {
                connection_id,
                object,
            } => {
                if uuid::Uuid::parse_str(connection_id).is_err() || !valid_object_ref(object) {
                    return Err(AppError::invalid("A database object bookmark is invalid"));
                }
            }
            BookmarkTarget::SavedQuery { saved_query_id } => {
                if uuid::Uuid::parse_str(saved_query_id).is_err() {
                    return Err(AppError::invalid("A saved-query bookmark is invalid"));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn read_saved_queries(connection: &Connection) -> AppResult<Vec<SavedQuery>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key='saved-queries'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    let Some(json) = json else {
        return Ok(Vec::new());
    };
    if json.len() > MAX_SAVED_QUERIES_TOTAL_BYTES * 6 + 65_536 {
        return Err(AppError::invalid("Saved queries library is too large"));
    }
    let items: Vec<SavedQuery> = serde_json::from_str(&json)
        .map_err(|_| AppError::invalid("Saved queries library is invalid"))?;
    validate_saved_queries(&items)?;
    Ok(items)
}

fn write_saved_queries(connection: &Connection, items: &[SavedQuery]) -> AppResult<()> {
    validate_saved_queries(items)?;
    let value = serde_json::to_string(items)
        .map_err(|_| AppError::internal("Could not encode saved queries"))?;
    connection
        .execute(
            "INSERT INTO settings(key,value) VALUES('saved-queries',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![value],
        )
        .map_err(db_error)?;
    Ok(())
}

pub(super) fn read_bookmarks(connection: &Connection) -> AppResult<Vec<SavedBookmark>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key='bookmarks'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    let Some(json) = json else {
        return Ok(Vec::new());
    };
    if json.len() > 2 * 1024 * 1024 {
        return Err(AppError::invalid("Saved bookmarks are too large"));
    }
    let items: Vec<SavedBookmark> = serde_json::from_str(&json)
        .map_err(|_| AppError::invalid("Saved bookmarks are invalid"))?;
    validate_bookmarks(&items)?;
    Ok(items)
}

fn write_bookmarks(connection: &Connection, items: &[SavedBookmark]) -> AppResult<()> {
    validate_bookmarks(items)?;
    let value = serde_json::to_string(items)
        .map_err(|_| AppError::internal("Could not encode bookmarks"))?;
    connection
        .execute(
            "INSERT INTO settings(key,value) VALUES('bookmarks',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![value],
        )
        .map_err(db_error)?;
    Ok(())
}

fn profile_exists(connection: &Connection, id: &str) -> AppResult<bool> {
    let exists: Option<i64> = connection
        .query_row("SELECT 1 FROM profiles WHERE id=?1", [id], |r| r.get(0))
        .optional()
        .map_err(db_error)?;
    Ok(exists.is_some())
}

pub(super) fn purge_connection(
    connection: &Connection,
    connection_id: &str,
    purge_bound_queries: bool,
) -> AppResult<()> {
    let mut queries = read_saved_queries(connection)?;
    let mut purged_query_ids = HashSet::new();
    let mut queries_changed = false;
    if purge_bound_queries {
        let before = queries.len();
        queries.retain(|q| {
            if q.connection_id.as_deref() == Some(connection_id) {
                purged_query_ids.insert(q.id.clone());
                false
            } else {
                true
            }
        });
        queries_changed = queries.len() != before;
    } else {
        for query in &mut queries {
            if query.connection_id.as_deref() == Some(connection_id) {
                query.connection_id = None;
                queries_changed = true;
            }
        }
    }
    if queries_changed {
        write_saved_queries(connection, &queries)?;
    }

    let mut bookmarks = read_bookmarks(connection)?;
    let before_bookmarks = bookmarks.len();
    bookmarks.retain(|b| match &b.target {
        BookmarkTarget::Object {
            connection_id: cid, ..
        } => cid != connection_id,
        BookmarkTarget::SavedQuery { saved_query_id } => !purged_query_ids.contains(saved_query_id),
    });
    if bookmarks.len() != before_bookmarks {
        write_bookmarks(connection, &bookmarks)?;
    }
    Ok(())
}

impl Persistence {
    pub fn saved_queries(&self) -> AppResult<Vec<SavedQuery>> {
        read_saved_queries(&self.connection.lock())
    }

    pub fn save_saved_query(&self, input: SavedQueryInput) -> AppResult<SavedQuery> {
        let title = clean_single_line(&input.title, MAX_SAVED_QUERY_TITLE_CHARS, "query title")?;
        if input.sql.trim().is_empty() || input.sql.contains('\0') {
            return Err(AppError::invalid("Provide non-empty UTF-8 SQL to save"));
        }
        if input.sql.len() > MAX_SAVED_QUERY_SQL_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Each saved query is limited to 256 KiB",
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
        let requested_id = input
            .id
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        if let Some(ref id) = requested_id
            && uuid::Uuid::parse_str(id).is_err()
        {
            return Err(AppError::invalid("Invalid saved query identifier"));
        }

        let mut conn = self.connection.lock();
        let tx = conn.transaction().map_err(db_error)?;
        if let Some(ref cid) = connection_id
            && !profile_exists(&tx, cid)?
        {
            return Err(AppError::new(
                ErrorCode::NotFound,
                "Choose an existing connection profile for this saved query",
            ));
        }

        let mut items = read_saved_queries(&tx)?;
        let updated_at_ms = now_ms();
        let saved = if let Some(ref existing_id) = requested_id
            && let Some(pos) = items.iter().position(|q| &q.id == existing_id)
        {
            let entry = SavedQuery {
                id: existing_id.clone(),
                title,
                sql: input.sql,
                connection_id,
                updated_at_ms,
            };
            items[pos] = entry.clone();
            entry
        } else {
            if items.len() >= MAX_SAVED_QUERIES {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "At most 256 saved queries may be stored",
                ));
            }
            let entry = SavedQuery {
                id: requested_id.unwrap_or_else(new_id),
                title,
                sql: input.sql,
                connection_id,
                updated_at_ms,
            };
            items.push(entry.clone());
            entry
        };
        items.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        write_saved_queries(&tx, &items)?;
        tx.commit().map_err(db_error)?;
        Ok(saved)
    }

    pub fn delete_saved_query(&self, id: &str) -> AppResult<()> {
        let mut conn = self.connection.lock();
        let tx = conn.transaction().map_err(db_error)?;
        let mut items = read_saved_queries(&tx)?;
        let before = items.len();
        items.retain(|q| q.id != id);
        if items.len() == before {
            return Err(AppError::new(
                ErrorCode::NotFound,
                "Saved query no longer exists",
            ));
        }
        write_saved_queries(&tx, &items)?;

        let mut bookmarks = read_bookmarks(&tx)?;
        let before_bm = bookmarks.len();
        bookmarks.retain(|b| {
            !matches!(
                &b.target,
                BookmarkTarget::SavedQuery { saved_query_id } if saved_query_id == id
            )
        });
        if bookmarks.len() != before_bm {
            write_bookmarks(&tx, &bookmarks)?;
        }
        tx.commit().map_err(db_error)
    }

    pub fn bookmarks(&self) -> AppResult<Vec<SavedBookmark>> {
        read_bookmarks(&self.connection.lock())
    }

    pub fn save_bookmark(&self, label: &str, target: BookmarkTarget) -> AppResult<SavedBookmark> {
        let label = clean_single_line(label, MAX_BOOKMARK_LABEL_CHARS, "bookmark label")?;
        let mut conn = self.connection.lock();
        let tx = conn.transaction().map_err(db_error)?;
        match &target {
            BookmarkTarget::Object {
                connection_id,
                object,
            } => {
                if uuid::Uuid::parse_str(connection_id).is_err() || !valid_object_ref(object) {
                    return Err(AppError::invalid(
                        "Provide a valid database object bookmark",
                    ));
                }
                if !profile_exists(&tx, connection_id)? {
                    return Err(AppError::new(
                        ErrorCode::NotFound,
                        "Connection profile no longer exists",
                    ));
                }
            }
            BookmarkTarget::SavedQuery { saved_query_id } => {
                if uuid::Uuid::parse_str(saved_query_id).is_err() {
                    return Err(AppError::invalid("Invalid saved query identifier"));
                }
                let queries = read_saved_queries(&tx)?;
                if !queries.iter().any(|q| &q.id == saved_query_id) {
                    return Err(AppError::new(
                        ErrorCode::NotFound,
                        "Saved query no longer exists",
                    ));
                }
            }
        }

        let mut items = read_bookmarks(&tx)?;
        let saved = if let Some(existing) = items.iter_mut().find(|b| b.target == target) {
            existing.label = label;
            existing.clone()
        } else {
            if items.len() >= MAX_BOOKMARKS {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "At most 256 bookmarks may be stored",
                ));
            }
            let entry = SavedBookmark {
                id: new_id(),
                label,
                target,
                created_at_ms: now_ms(),
            };
            items.insert(0, entry.clone());
            entry
        };
        items.sort_by(|a, b| {
            b.created_at_ms
                .cmp(&a.created_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        write_bookmarks(&tx, &items)?;
        tx.commit().map_err(db_error)?;
        Ok(saved)
    }

    pub fn delete_bookmark(&self, id: &str) -> AppResult<()> {
        let mut conn = self.connection.lock();
        let tx = conn.transaction().map_err(db_error)?;
        let mut items = read_bookmarks(&tx)?;
        let before = items.len();
        items.retain(|b| b.id != id);
        if items.len() == before {
            return Err(AppError::new(
                ErrorCode::NotFound,
                "Bookmark no longer exists",
            ));
        }
        write_bookmarks(&tx, &items)?;
        tx.commit().map_err(db_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile(environment: Environment, store_query_text: bool) -> ConnectionProfile {
        ConnectionProfile {
            id: new_id(),
            name: "Profile".into(),
            group_id: match environment {
                Environment::Production => "production".into(),
                Environment::Staging => "staging".into(),
                Environment::Local => "local".into(),
            },
            engine: EngineKind::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            database: "joinery".into(),
            username: "joinery".into(),
            environment,
            safe_mode: if environment == Environment::Production {
                SafeMode::PasswordGated
            } else {
                SafeMode::Confirm
            },
            tls_mode: TlsMode::VerifyFull,
            ca_cert_path: None,
            ssh: None,
            remember_password: false,
            store_query_text,
            store_drafts: false,
            snowflake: None,
        }
    }

    #[test]
    fn saved_queries_and_bookmarks_enforce_bounds_deduplicate_and_cascade_on_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Persistence::open(dir.path()).unwrap();

        let local_keep = make_profile(Environment::Local, true);
        let local_no_history = make_profile(Environment::Local, false);
        let prod_profile = make_profile(Environment::Production, true);
        store.save_profile(&local_keep).unwrap();
        store.save_profile(&local_no_history).unwrap();
        store.save_profile(&prod_profile).unwrap();

        // Create and update saved queries
        let q_local = store
            .save_saved_query(SavedQueryInput {
                id: None,
                title: "  Active Orders  ".into(),
                sql: "SELECT * FROM public.orders WHERE status = 'open';".into(),
                connection_id: Some(local_keep.id.clone()),
            })
            .unwrap();
        assert_eq!(q_local.title, "Active Orders");
        assert_eq!(
            q_local.connection_id.as_deref(),
            Some(local_keep.id.as_str())
        );

        let q_local_updated = store
            .save_saved_query(SavedQueryInput {
                id: Some(q_local.id.clone()),
                title: "Active Orders v2".into(),
                sql: "SELECT id FROM public.orders;".into(),
                connection_id: Some(local_keep.id.clone()),
            })
            .unwrap();
        assert_eq!(q_local_updated.id, q_local.id);
        assert_eq!(q_local_updated.title, "Active Orders v2");

        let q_no_hist = store
            .save_saved_query(SavedQueryInput {
                id: None,
                title: "Sensitive Local".into(),
                sql: "SELECT 1;".into(),
                connection_id: Some(local_no_history.id.clone()),
            })
            .unwrap();
        let q_prod = store
            .save_saved_query(SavedQueryInput {
                id: None,
                title: "Prod Diagnostic".into(),
                sql: "SELECT count(*) FROM audit;".into(),
                connection_id: Some(prod_profile.id.clone()),
            })
            .unwrap();

        // Reject oversized (>256 KiB), control-char title, and unknown profile
        assert_eq!(
            store
                .save_saved_query(SavedQueryInput {
                    id: None,
                    title: "Too Big".into(),
                    sql: "x".repeat(MAX_SAVED_QUERY_SQL_BYTES + 1),
                    connection_id: None,
                })
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
        assert_eq!(
            store
                .save_saved_query(SavedQueryInput {
                    id: None,
                    title: "Bad\nTitle".into(),
                    sql: "SELECT 1;".into(),
                    connection_id: None,
                })
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            store
                .save_saved_query(SavedQueryInput {
                    id: None,
                    title: "Missing Conn".into(),
                    sql: "SELECT 1;".into(),
                    connection_id: Some(new_id()),
                })
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );

        // Bookmarks deduplicate by canonical target and cascade properly
        let obj_target = BookmarkTarget::Object {
            connection_id: local_keep.id.clone(),
            object: DatabaseObjectRef {
                schema: "public".into(),
                name: "orders".into(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
        };
        let bm_obj = store
            .save_bookmark("public.orders", obj_target.clone())
            .unwrap();
        let bm_obj_dedup = store.save_bookmark("Orders Table", obj_target).unwrap();
        assert_eq!(bm_obj.id, bm_obj_dedup.id);
        assert_eq!(bm_obj_dedup.label, "Orders Table");

        let bm_q_local = store
            .save_bookmark(
                "Active Orders Bookmark",
                BookmarkTarget::SavedQuery {
                    saved_query_id: q_local.id.clone(),
                },
            )
            .unwrap();
        let _bm_q_prod = store
            .save_bookmark(
                "Prod Bookmark",
                BookmarkTarget::SavedQuery {
                    saved_query_id: q_prod.id.clone(),
                },
            )
            .unwrap();

        // Deleting local_keep (Local + store_query_text = true) purges object bookmarks,
        // but DETACHES q_local (connection_id -> None) and preserves q_local's bookmark!
        store.delete_profile(&local_keep.id).unwrap();
        let after_local = store.saved_queries().unwrap();
        let detached = after_local.iter().find(|q| q.id == q_local.id).unwrap();
        assert_eq!(detached.connection_id, None);
        let bms_after_local = store.bookmarks().unwrap();
        assert!(!bms_after_local.iter().any(|b| b.id == bm_obj.id));
        assert!(bms_after_local.iter().any(|b| b.id == bm_q_local.id));

        // Deleting local_no_history (!store_query_text) and prod_profile (Production) PURGES their saved queries & bookmarks!
        store.delete_profile(&local_no_history.id).unwrap();
        store.delete_profile(&prod_profile.id).unwrap();
        let remaining_queries = store.saved_queries().unwrap();
        assert_eq!(remaining_queries.len(), 1);
        assert_eq!(remaining_queries[0].id, q_local.id);
        assert!(!remaining_queries.iter().any(|q| q.id == q_no_hist.id));
        assert!(!remaining_queries.iter().any(|q| q.id == q_prod.id));
        let remaining_bms = store.bookmarks().unwrap();
        assert_eq!(remaining_bms.len(), 1);
        assert_eq!(remaining_bms[0].id, bm_q_local.id);

        // Deleting a saved query cascades to its SavedQuery bookmark
        store.delete_saved_query(&q_local.id).unwrap();
        assert!(store.saved_queries().unwrap().is_empty());
        assert!(store.bookmarks().unwrap().is_empty());
    }

    #[test]
    fn metadata_backup_validation_rejects_corrupted_saved_queries_or_bookmarks() {
        let dir = tempfile::tempdir().unwrap();
        let store = Persistence::open(dir.path()).unwrap();
        store
            .save_saved_query(SavedQueryInput {
                id: None,
                title: "Valid".into(),
                sql: "SELECT 1;".into(),
                connection_id: None,
            })
            .unwrap();
        let good_backup = store.create_metadata_backup().unwrap();
        assert!(
            store
                .restore_metadata_backup(&good_backup.id, &good_backup.digest)
                .is_ok()
        );

        // Tamper with settings('bookmarks') inside the backup file and verify restore rejects it
        let bad_backup_path = dir
            .path()
            .join("backups")
            .join(format!("{}.sqlite", good_backup.id));
        {
            let tampered = Connection::open(&bad_backup_path).unwrap();
            tampered
                .execute(
                    "INSERT OR REPLACE INTO settings(key, value) VALUES('bookmarks', '{not-json')",
                    [],
                )
                .unwrap();
        }
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(&bad_backup_path).unwrap();
        let tampered_digest = hex::encode(Sha256::digest(&bytes));
        assert_eq!(
            store
                .restore_metadata_backup(&good_backup.id, &tampered_digest)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );

        // Corrupt settings('saved-queries') in live DB and verify create_metadata_backup rejects it
        store
            .set_setting("saved-queries", "[{\"unexpected\":true}]")
            .unwrap();
        assert_eq!(
            store.create_metadata_backup().unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }
}
