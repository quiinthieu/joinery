//! Bounded list projections and explicit full-text detail. Searching may scan
//! every retained row; it keeps one SQL string at a time, never a lifetime list.
use super::{Persistence, db_error};
use crate::domain::*;
use rusqlite::{OptionalExtension, params};

pub(in crate::persistence) mod lifecycle;
pub use lifecycle::{HistoryCompletion, HistoryDisposition, HistoryTicket};
pub(super) use lifecycle::{schema_objects, validate_records};

const MAX_SEARCH_BYTES: usize = 512;
const MAX_ID_BYTES: usize = 256;
const PREVIEW_CHARS: usize = 256;
const MAX_SAFE_TIMESTAMP: u64 = 9_007_199_254_740_991;
const COMPATIBILITY_LIMIT: usize = 1_000;

// SQLite extracts only metadata and at most 1028 SQL UTF-8 bytes for an empty
// search. A byte prefix handles embedded NUL without SQLite text truncation;
// it contains at least 257 characters when truncation is needed. CASE does not
// extract full SQL into Rust unless search needs it.
const PAGE_SQL: &str = "SELECT json_object(
 'provenance',CASE WHEN json_type(json,'$.provenance') IS NULL THEN 'recorded' ELSE json_extract(json,'$.provenance') END,
 'id',json_extract(json,'$.id'),
 'connectionId',json_extract(json,'$.connectionId'),
 'connectionName',json_extract(json,'$.connectionName'),
 'startedAt',json_extract(json,'$.startedAt'),
 'executionSummary',json_extract(json,'$.executionSummary'),
 'snowflakeDml',json_extract(json,'$.snowflakeDml'),
 'snowflakeDdl',json_extract(json,'$.snowflakeDdl'),
 'statementCount',json_extract(json,'$.statementCount'),
 'status',json_extract(json,'$.status'),
 'elapsedMs',json_extract(json,'$.elapsedMs'),
 'sqlPreview',NULL,
 'sqlStored',json(CASE WHEN json_type(json,'$.sql')='text' THEN 'true' ELSE 'false' END)),
 CASE WHEN json_type(json,'$.sql')='text'
 THEN coalesce(substr(CAST(json_extract(json,'$.sql') AS BLOB),1,1028),X'')
 ELSE NULL END,
 CASE WHEN ?2 THEN json_extract(json,'$.sql') ELSE NULL END,
 json_type(json,'$.sql')
 FROM history_v8 WHERE id=?1";

// Sort only keys. Including full SQL in this ORDER BY projection could cause
// SQLite's sorter to collect every candidate's SQL before Rust receives a row.
const KEYS_SQL: &str = "SELECT id,started_at FROM history_v8
 WHERE (?1 IS NULL OR started_at<?1 OR (started_at=?1 AND id<?2))
 ORDER BY started_at DESC,id DESC LIMIT ?3";

struct Projection {
    json: String,
    snippet: Option<Vec<u8>>,
    sql: Option<String>,
    sql_type: Option<String>,
}

fn invalid_data() -> AppError {
    AppError::internal("Application history is invalid")
}

fn validate_id(id: &str) -> AppResult<()> {
    if id.is_empty() || id.len() > MAX_ID_BYTES || id.contains('\0') {
        return Err(AppError::invalid("History identity is invalid"));
    }
    Ok(())
}

fn tokens(search: &str) -> AppResult<Vec<String>> {
    if search.len() > MAX_SEARCH_BYTES {
        return Err(AppError::invalid("History search exceeds 512 UTF-8 bytes"));
    }
    Ok(search.split_whitespace().map(str::to_lowercase).collect())
}

fn validate_row(id: &str, started_at: i64, stored_id: &str, stored_at: u64) -> AppResult<()> {
    validate_id(id).map_err(|_| invalid_data())?;
    if stored_id != id
        || stored_at > MAX_SAFE_TIMESTAMP
        || u64::try_from(started_at).ok() != Some(stored_at)
    {
        return Err(invalid_data());
    }
    Ok(())
}

fn preview(source: Vec<u8>) -> AppResult<String> {
    let source = match std::str::from_utf8(&source) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&source[..error.valid_up_to()]).map_err(|_| invalid_data())?
        }
        Err(_) => return Err(invalid_data()),
    };
    let mut result: String = source.chars().take(PREVIEW_CHARS).collect();
    if source.chars().nth(PREVIEW_CHARS).is_some() {
        result.pop();
        result.push('…');
    }
    Ok(result)
}

fn metadata(entry: &HistoryEntry) -> HistorySummary {
    HistorySummary {
        provenance: entry.provenance,
        execution_summary: entry.execution_summary.clone(),
        snowflake_dml: entry.snowflake_dml.clone(),
        snowflake_ddl: entry.snowflake_ddl.clone(),
        id: entry.id.clone(),
        connection_id: entry.connection_id.clone(),
        connection_name: entry.connection_name.clone(),
        started_at: entry.started_at,
        sql_preview: None,
        sql_stored: entry.sql.is_some(),
        statement_count: entry.statement_count,
        status: entry.status,
        elapsed_ms: entry.elapsed_ms,
    }
}

fn matches(summary: &HistorySummary, sql: Option<&str>, tokens: &[String]) -> bool {
    if tokens.is_empty() {
        return true;
    }
    let status = match summary.status {
        Some(RunStatus::Completed) => "completed",
        Some(RunStatus::Stopped) => "stopped",
        Some(RunStatus::Failed) => "failed",
        Some(RunStatus::Cancelled) => "cancelled",
        Some(RunStatus::OutcomeUnknown) => "outcome unknown",
        None => "completion unknown",
    };
    let mut text = format!("{} {status}", summary.connection_name);
    text.push_str(match summary.provenance {
        HistoryProvenance::Recorded => " recorded",
        HistoryProvenance::Admitted => " admitted completion pending",
        HistoryProvenance::Interrupted => " recovered after interruption interrupted recovery",
        HistoryProvenance::RestoreRecovered => {
            " recovered from metadata restore restored restore recovered recovery"
        }
    });
    if summary.elapsed_ms.is_none() {
        text.push_str(" duration unavailable");
    }
    if let Some(execution) = &summary.execution_summary {
        text.push_str(&format!(
            " planned: {} attempted: {} completed: {} not attempted: {}",
            execution.planned, execution.attempted, execution.completed, execution.not_attempted
        ));
        text.push_str(match execution.stop_reason {
            Some(ExecutionStopReason::ResultLimit) => " result retention limit reached",
            Some(ExecutionStopReason::Cancelled) => " execution cancelled",
            Some(ExecutionStopReason::Failed) => " stopped after an error",
            Some(ExecutionStopReason::OutcomeUnknown) => " statement outcome unknown",
            None => "",
        });
        if execution.not_attempted == 1 {
            text.push_str(&format!(
                " statement {} was not attempted",
                execution.planned
            ));
        } else if execution.not_attempted > 1 {
            text.push_str(&format!(
                " statements {}–{} were not attempted",
                execution.attempted.saturating_add(1),
                execution.planned
            ));
        }
    } else {
        text.push_str(" statement counts unknown planned attempted completed and not attempted counts were not recorded");
    }
    if let Some(dml) = &summary.snowflake_dml {
        let kind = match dml.kind {
            SnowflakeDmlKind::Insert => "insert",
            SnowflakeDmlKind::Update => "update",
            SnowflakeDmlKind::Delete => "delete",
        };
        let outcome = match dml.statement_outcome {
            SnowflakeDmlOutcome::Succeeded => "statement succeeded acknowledged",
            SnowflakeDmlOutcome::Rejected => "statement was rejected",
            SnowflakeDmlOutcome::Unknown => "statement outcome is unknown",
        };
        let affected_rows = if dml.summary.status == SnowflakeDmlSummaryStatus::Validated {
            dml.summary.affected_rows.as_deref()
        } else {
            None
        };
        text.push_str(&format!(
            " snowflake {kind} {outcome} affected rows: {}",
            affected_rows.unwrap_or("unavailable")
        ));
        if affected_rows.is_none() {
            text.push_str(" affected-row summary is unavailable");
        }
        if dml.kind == SnowflakeDmlKind::Update
            && dml.summary.status == SnowflakeDmlSummaryStatus::Validated
            && let Some(rows) = &dml.summary.multi_joined_rows
        {
            text.push_str(&format!(" multi-joined rows updated: {rows}"));
        }
        if dml.retired {
            text.push_str(" session retired");
        }
    }
    if let Some(ddl) = &summary.snowflake_ddl {
        let kind = match ddl.kind {
            SnowflakeDdlKind::CreateTable => "create table",
            SnowflakeDdlKind::AddColumn => "add column",
            SnowflakeDdlKind::DropTable => "drop table",
        };
        let outcome = match ddl.statement_outcome {
            SnowflakeDdlOutcome::Succeeded => "statement acknowledged",
            SnowflakeDdlOutcome::Rejected => "statement rejected",
            SnowflakeDdlOutcome::Unknown => "statement outcome unknown",
        };
        text.push_str(&format!(
            " snowflake {kind} {} {outcome}",
            ddl.qualified_target
        ));
        if ddl.summary_status == SnowflakeDdlSummaryStatus::Unavailable {
            text.push_str(" status summary unavailable");
        }
        if ddl.catalog_reconciliation_required {
            text.push_str(" explicit catalog reconciliation required");
        }
        if ddl.retired {
            text.push_str(" session retired");
        }
    }
    text.push(' ');
    text.push_str(sql.unwrap_or(""));
    let lower = text.to_lowercase();
    tokens.iter().all(|token| lower.contains(token))
}

impl Persistence {
    pub fn history_page(&self, request: &HistoryPageRequest) -> AppResult<HistoryPage> {
        if !(1..=100).contains(&request.limit) {
            return Err(AppError::invalid(
                "History page limit must be between 1 and 100",
            ));
        }
        let search = tokens(&request.search)?;
        if let Some(cursor) = &request.cursor {
            validate_id(&cursor.id)?;
            if cursor.started_at > MAX_SAFE_TIMESTAMP {
                return Err(AppError::invalid("History cursor timestamp is invalid"));
            }
        }
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        let page = read_page(&tx, request, &search)?;
        tx.commit().map_err(db_error)?;
        Ok(page)
    }

    pub fn history_entry(&self, id: &str) -> AppResult<Option<HistoryEntry>> {
        validate_id(id)?;
        let connection = self.connection.lock();
        let stored: Option<(i64, String)> = connection
            .query_row(
                "SELECT started_at,json FROM history_v8 WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        stored
            .map(|(started_at, json)| {
                let entry: HistoryEntry =
                    serde_json::from_str(&json).map_err(|_| invalid_data())?;
                entry.validate_lifecycle().map_err(|_| invalid_data())?;
                validate_row(id, started_at, &entry.id, entry.started_at)?;
                Ok(entry)
            })
            .transpose()
    }
}

fn read_page(
    connection: &rusqlite::Connection,
    request: &HistoryPageRequest,
    search: &[String],
) -> AppResult<HistoryPage> {
    let mut statement = connection.prepare(KEYS_SQL).map_err(db_error)?;
    let mut projection = connection.prepare(PAGE_SQL).map_err(db_error)?;
    let cursor_at = request.cursor.as_ref().map(|c| c.started_at as i64);
    let cursor_id = request.cursor.as_ref().map(|c| c.id.as_str());
    let sql_limit = if search.is_empty() {
        i64::from(request.limit) + 1
    } else {
        -1
    };
    let mut rows = statement
        .query(params![cursor_at, cursor_id, sql_limit])
        .map_err(db_error)?;
    let mut entries = Vec::with_capacity(request.limit as usize);
    let mut last = None;
    let mut has_more = false;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let started_at: i64 = row.get(1).map_err(db_error)?;
        let projected = projection
            .query_row(params![id, !search.is_empty()], |row| {
                Ok(Projection {
                    json: row.get(0)?,
                    snippet: row.get(1)?,
                    sql: row.get(2)?,
                    sql_type: row.get(3)?,
                })
            })
            .map_err(db_error)?;
        let mut summary: HistorySummary =
            serde_json::from_str(&projected.json).map_err(|_| invalid_data())?;
        summary.validate_lifecycle().map_err(|_| invalid_data())?;
        validate_row(&id, started_at, &summary.id, summary.started_at)?;
        if !matches!(projected.sql_type.as_deref(), None | Some("null" | "text")) {
            return Err(invalid_data());
        }
        if !matches(&summary, projected.sql.as_deref(), search) {
            continue;
        }
        if entries.len() == request.limit as usize {
            has_more = true;
            break;
        }
        summary.sql_preview = projected.snippet.map(preview).transpose()?;
        last = Some(HistoryCursor {
            started_at: summary.started_at,
            id: summary.id.clone(),
        });
        entries.push(summary);
    }
    Ok(HistoryPage {
        entries,
        next_cursor: if has_more { last } else { None },
    })
}

/// Compatibility remains capped at 1000 matching full entries. Filtering occurs
/// before the cap; only a single scanned row exists beyond the bounded output.
pub(super) fn compatibility_history(
    store: &Persistence,
    search: &str,
) -> AppResult<Vec<HistoryEntry>> {
    let search = tokens(search)?;
    let connection = store.connection.lock();
    let mut statement = connection
        .prepare("SELECT id,started_at FROM history_v8 ORDER BY started_at DESC,id DESC")
        .map_err(db_error)?;
    let mut detail = connection
        .prepare("SELECT json FROM history_v8 WHERE id=?1")
        .map_err(db_error)?;
    let mut rows = statement.query([]).map_err(db_error)?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let started_at: i64 = row.get(1).map_err(db_error)?;
        let json: String = detail.query_row([&id], |r| r.get(0)).map_err(db_error)?;
        let entry: HistoryEntry = serde_json::from_str(&json).map_err(|_| invalid_data())?;
        entry.validate_lifecycle().map_err(|_| invalid_data())?;
        validate_row(&id, started_at, &entry.id, entry.started_at)?;
        if matches(&metadata(&entry), entry.sql.as_deref(), &search) {
            entries.push(entry);
            if entries.len() == COMPATIBILITY_LIMIT {
                break;
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::{HistoryCompletion, MAX_SAFE_TIMESTAMP, PREVIEW_CHARS, lifecycle, metadata};
    use crate::{domain::*, persistence::Persistence};
    use rusqlite::params;

    #[test]
    fn history_search_finds_a_match_older_than_the_newest_thousand_records() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        {
            let mut connection = store.connection.lock();
            let tx = connection.transaction().unwrap();
            for index in 0..=1_001 {
                let entry = HistoryEntry {
                    provenance: HistoryProvenance::Recorded,
                    execution_summary: None,
                    snowflake_dml: None,
                    snowflake_ddl: None,
                    id: format!("history-{index:04}"),
                    connection_id: "fixture".into(),
                    connection_name: "Local fixture".into(),
                    started_at: index + 1,
                    sql: Some(
                        if index == 0 {
                            "SELECT 'olderneedle'"
                        } else {
                            "SELECT 1"
                        }
                        .into(),
                    ),
                    statement_count: 1,
                    status: Some(RunStatus::Completed),
                    elapsed_ms: Some(1),
                };
                tx.execute(
                    "INSERT INTO history_v8(id,started_at,json) VALUES(?1,?2,?3)",
                    params![
                        entry.id,
                        i64::try_from(entry.started_at).unwrap(),
                        serde_json::to_string(&entry).unwrap()
                    ],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        let matches = store.history("olderneedle").unwrap();
        assert_eq!(
            matches.len(),
            1,
            "search must filter before the compatibility result cap"
        );
        assert_eq!(matches[0].id, "history-0000");
        let page = store.history_page(&request("olderneedle", 10)).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].id, "history-0000");
        assert!(page.next_cursor.is_none());
        assert_eq!(store.history("").unwrap().len(), 1_000);
    }

    fn entry(id: &str, started_at: u64, sql: Option<&str>) -> HistoryEntry {
        HistoryEntry {
            provenance: HistoryProvenance::Recorded,
            execution_summary: None,
            snowflake_dml: None,
            snowflake_ddl: None,
            id: id.into(),
            connection_id: "fixture".into(),
            connection_name: "École fixture".into(),
            started_at,
            sql: sql.map(str::to_owned),
            statement_count: 1,
            status: Some(RunStatus::Completed),
            elapsed_ms: Some(7),
        }
    }

    fn seed(store: &Persistence, entries: &[HistoryEntry]) {
        let mut connection = store.connection.lock();
        let tx = connection.transaction().unwrap();
        for entry in entries {
            tx.execute(
                "INSERT INTO history_v8(id,started_at,json) VALUES(?1,?2,?3)",
                params![
                    entry.id,
                    i64::try_from(entry.started_at).unwrap(),
                    serde_json::to_string(entry).unwrap()
                ],
            )
            .unwrap();
            lifecycle::ensure_profile(&tx, &entry.connection_id).unwrap();
            let (lineage, era): (String, String) = tx
                .query_row(
                    "SELECT lineage,current_era FROM history_privacy_v5",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            let generation: String = tx
                .query_row(
                    "SELECT generation FROM history_profile_privacy_v5 WHERE connection_id=?1",
                    [&entry.connection_id],
                    |r| r.get(0),
                )
                .unwrap();
            let mut plain = entry.clone();
            plain.sql = None;
            plain.provenance = HistoryProvenance::Admitted;
            plain.status = None;
            plain.elapsed_ms = None;
            plain.execution_summary = None;
            plain.snowflake_dml = None;
            plain.snowflake_ddl = None;
            let pending = entry.provenance == HistoryProvenance::Admitted;
            use sha2::Digest;
            tx.execute(
                "INSERT INTO history_tickets_v5 VALUES(?1,?2,?3,?4,?5,?6,0,?7,?8)",
                params![
                    entry.id,
                    serde_json::to_string(&plain).unwrap(),
                    lineage,
                    era,
                    generation,
                    entry
                        .sql
                        .as_ref()
                        .map(|sql| hex::encode(sha2::Sha256::digest(sql.as_bytes()))),
                    if pending { "pending" } else { "applied" },
                    if pending {
                        None
                    } else {
                        Some(serde_json::to_string(&HistoryCompletion::from_entry(entry)).unwrap())
                    }
                ],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    fn request(search: &str, limit: u32) -> HistoryPageRequest {
        HistoryPageRequest {
            search: search.into(),
            cursor: None,
            limit,
        }
    }

    #[test]
    fn history_lifecycle_projection_preserves_legacy_defaults_and_nullable_recovery() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let legacy = entry("legacy", 1, None);
        let mut admitted = entry("pending", 2, None);
        admitted.provenance = HistoryProvenance::Admitted;
        admitted.status = None;
        admitted.elapsed_ms = None;
        let mut interrupted = admitted.clone();
        interrupted.id = "interrupted".into();
        interrupted.started_at = 3;
        interrupted.provenance = HistoryProvenance::Interrupted;
        let mut restored = admitted.clone();
        restored.id = "restored".into();
        restored.started_at = 4;
        restored.provenance = HistoryProvenance::RestoreRecovered;
        restored.status = Some(RunStatus::Completed);
        let mut unmeasured = legacy.clone();
        unmeasured.id = "unmeasured".into();
        unmeasured.started_at = 5;
        unmeasured.elapsed_ms = None;
        seed(
            &store,
            &[legacy, admitted, interrupted, restored, unmeasured],
        );
        store
            .connection
            .lock()
            .execute(
                "UPDATE history_v8 SET json=json_remove(json,'$.provenance') WHERE id='legacy'",
                [],
            )
            .unwrap();
        drop(store);
        let store = Persistence::open(root.path()).unwrap();
        let page = store.history_page(&request("", 100)).unwrap();
        assert_eq!(page.entries.len(), 5);
        for summary in &page.entries {
            summary.validate_lifecycle().unwrap();
            let detail = store.history_entry(&summary.id).unwrap().unwrap();
            assert_eq!(summary.provenance, detail.provenance);
            assert_eq!(summary.status, detail.status);
            assert_eq!(summary.elapsed_ms, detail.elapsed_ms);
            let encoded = serde_json::to_value(summary).unwrap();
            assert!(encoded["provenance"].is_string());
            if summary.id == "legacy" {
                assert_eq!(summary.provenance, HistoryProvenance::Recorded);
                assert_eq!(summary.elapsed_ms, Some(7));
            } else {
                assert!(encoded["elapsedMs"].is_null());
            }
        }
        for (search, ids) in [
            ("admitted completion pending", vec!["pending"]),
            ("interrupted", vec!["interrupted"]),
            (
                "recovered after interruption completion unknown",
                vec!["interrupted"],
            ),
            ("recovered from metadata restore", vec!["restored"]),
            ("restored completed", vec!["restored"]),
            (
                "duration unavailable",
                vec!["unmeasured", "restored", "interrupted", "pending"],
            ),
        ] {
            let page = store.history_page(&request(search, 100)).unwrap();
            assert_eq!(
                page.entries
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>(),
                ids
            );
            assert_eq!(
                store
                    .history(search)
                    .unwrap()
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>(),
                ids
            );
        }
    }

    #[test]
    fn history_lifecycle_readers_reject_explicit_null_unknown_and_inconsistent_states() {
        use serde_json::json;
        let patches = [
            json!({"provenance": null}),
            json!({"provenance": "future"}),
            json!({"provenance": 1}),
            json!({"provenance": {"kind": "recorded"}}),
            json!({"status": null, "elapsedMs": null}),
            json!({"provenance": "admitted"}),
            json!({"provenance": "admitted", "status": null, "elapsedMs": 0}),
            json!({"provenance": "interrupted", "status": null, "elapsedMs": 0}),
            json!({"provenance": "restoreRecovered", "status": null, "elapsedMs": 1}),
            json!({"provenance": "admitted", "status": null, "elapsedMs": null,
                "executionSummary": {"planned": 1, "attempted": 0, "completed": 0, "notAttempted": 1, "stopReason": null}}),
        ];
        for patch in patches {
            let root = tempfile::tempdir().unwrap();
            let store = Persistence::open(root.path()).unwrap();
            let original = entry("original", 1, None);
            let mut raw = serde_json::to_value(&original).unwrap();
            for (key, value) in patch.as_object().unwrap() {
                raw[key] = value.clone();
            }
            store
                .connection
                .lock()
                .execute(
                    "INSERT INTO history_v8(id,started_at,json) VALUES('original',1,?1)",
                    [serde_json::to_string(&raw).unwrap()],
                )
                .unwrap();
            for search in ["", "no matching text"] {
                assert!(
                    store.history_page(&request(search, 100)).is_err(),
                    "{patch}"
                );
                assert!(store.history(search).is_err(), "{patch}");
            }
            assert!(store.history_entry("original").is_err(), "{patch}");
        }
    }

    #[test]
    fn history_lifecycle_shared_validator_rejects_admitted_outcome_evidence() {
        let mut value = entry("pending", 1, None);
        value.provenance = HistoryProvenance::Admitted;
        value.status = None;
        value.elapsed_ms = None;
        value.validate_lifecycle().unwrap();
        metadata(&value).validate_lifecycle().unwrap();
        value.execution_summary = Some(ExecutionSummary {
            planned: 1,
            attempted: 0,
            completed: 0,
            not_attempted: 1,
            stop_reason: None,
        });
        assert!(value.validate_lifecycle().is_err());
        assert!(metadata(&value).validate_lifecycle().is_err());
        value.execution_summary = None;
        value.snowflake_dml = Some(
            serde_json::from_value(serde_json::json!({
                "kind": "update", "statementOutcome": "unknown", "summary": {
                    "status": "unavailable", "affectedRows": null, "multiJoinedRows": null
                }, "retired": true
            }))
            .unwrap(),
        );
        assert!(value.validate_lifecycle().is_err());
        assert!(metadata(&value).validate_lifecycle().is_err());
        value.snowflake_dml = None;
        value.snowflake_ddl = Some(
            serde_json::from_value(serde_json::json!({
                "kind": "addColumn", "statementOutcome": "unknown", "summaryStatus": "unavailable",
                "qualifiedTarget": "\"DB\".\"PUBLIC\".\"T\"", "catalogReconciliationRequired": true,
                "retired": true
            }))
            .unwrap(),
        );
        assert!(value.validate_lifecycle().is_err());
        assert!(metadata(&value).validate_lifecycle().is_err());
    }

    #[test]
    fn history_pages_tied_timestamps_have_no_duplicate_or_missing_rows() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let mut expected: Vec<_> = (0..17)
            .map(|i| entry(&format!("row-{i:02}"), 100 + i / 5, Some("SELECT 1")))
            .collect();
        seed(&store, &expected);
        expected.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        let mut input = request("", 3);
        let mut ids = Vec::new();
        loop {
            let page = store.history_page(&input).unwrap();
            assert!(page.entries.len() <= 3);
            ids.extend(page.entries.iter().map(|e| e.id.clone()));
            if page.next_cursor.is_none() {
                break;
            }
            input.cursor = page.next_cursor;
            assert!(ids.len() <= expected.len());
        }
        assert_eq!(ids, expected.into_iter().map(|e| e.id).collect::<Vec<_>>());
    }

    #[test]
    fn history_search_uses_full_sql_unicode_and_literal_tokens_beyond_preview() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let sql = format!(
            "-- {}\nSELECT 'deepNeedle 100%_[] \\\"literal\\\"'",
            "雪".repeat(500)
        );
        seed(
            &store,
            &[
                entry("deep", 10, Some(&sql)),
                entry("other", 11, Some("SELECT 'deepNeedle'")),
            ],
        );
        let page = store
            .history_page(&request("éCOLE 100%_[] DEEPNEEDLE", 1))
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        let summary = &page.entries[0];
        assert_eq!(summary.id, "deep");
        assert!(summary.sql_stored);
        let preview = summary.sql_preview.as_deref().unwrap();
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        assert!(preview.ends_with('…'));
        assert!(!preview.contains("deepNeedle"));
        let serialized = serde_json::to_value(summary).unwrap();
        assert!(serialized.get("sql").is_none());
        assert!(page.next_cursor.is_none());
        assert_eq!(
            store.history_entry("deep").unwrap().unwrap().sql.as_deref(),
            Some(sql.as_str())
        );
        assert!(
            store
                .history_page(&request("100%_[] missingtoken", 100))
                .unwrap()
                .entries
                .is_empty()
        );
    }

    #[test]
    fn history_search_lookahead_counts_matches_instead_of_scanned_rows() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let rows: Vec<_> = (0..12)
            .map(|i| {
                entry(
                    &format!("row-{i:02}"),
                    i + 1,
                    Some(if i % 4 == 0 {
                        "SELECT 'matchneedle'"
                    } else {
                        "SELECT 1"
                    }),
                )
            })
            .collect();
        seed(&store, &rows);
        let mut input = request("matchneedle", 2);
        let first = store.history_page(&input).unwrap();
        assert_eq!(
            first
                .entries
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["row-08", "row-04"]
        );
        input.cursor = first.next_cursor;
        assert!(input.cursor.is_some());
        let last = store.history_page(&input).unwrap();
        assert_eq!(last.entries.len(), 1);
        assert_eq!(last.entries[0].id, "row-00");
        assert!(last.next_cursor.is_none());
    }

    #[test]
    fn history_summaries_distinguish_unstored_empty_and_exact_length_sql() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let exact = "é".repeat(PREVIEW_CHARS);
        seed(
            &store,
            &[
                entry("unstored", 1, None),
                entry("empty", 2, Some("")),
                entry("exact", 3, Some(&exact)),
            ],
        );
        for search in ["", "fixture"] {
            let page = store.history_page(&request(search, 100)).unwrap();
            assert_eq!(page.entries[0].sql_preview.as_deref(), Some(exact.as_str()));
            assert!(page.entries[0].sql_stored);
            assert_eq!(page.entries[1].sql_preview.as_deref(), Some(""));
            assert!(page.entries[1].sql_stored);
            assert!(page.entries[2].sql_preview.is_none());
            assert!(!page.entries[2].sql_stored);
            for summary in page.entries {
                assert!(summary.execution_summary.is_none());
                assert!(
                    serde_json::to_value(summary)
                        .unwrap()
                        .get("executionSummary")
                        .is_none()
                );
            }
        }
        assert_eq!(
            store
                .history_entry("empty")
                .unwrap()
                .unwrap()
                .sql
                .as_deref(),
            Some("")
        );
        assert!(
            store
                .history_entry("unstored")
                .unwrap()
                .unwrap()
                .sql
                .is_none()
        );
    }

    #[test]
    fn history_sql_preview_handles_nul_and_partial_utf8_prefix_without_losing_truncation() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let sql = format!("a\0{}", "雪🦀".repeat(500));
        seed(&store, &[entry("nul", 1, Some(&sql))]);
        let page = store.history_page(&request("", 1)).unwrap();
        let preview = page.entries[0].sql_preview.as_deref().unwrap();
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        assert!(preview.starts_with("a\0雪🦀"));
        assert!(preview.ends_with('…'));
        assert!(!preview.contains('\u{fffd}'));
        assert_eq!(
            store.history_entry("nul").unwrap().unwrap().sql.as_deref(),
            Some(sql.as_str())
        );
        let searched = store.history_page(&request("雪", 1)).unwrap();
        assert_eq!(searched.entries[0].sql_preview, page.entries[0].sql_preview);
    }

    #[test]
    fn history_search_uses_unicode_lowercase_not_full_case_folding() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        seed(
            &store,
            &[
                entry("street", 1, Some("SELECT 'Straße'")),
                entry("sigma", 2, Some("SELECT 'Σ'")),
            ],
        );
        let street = store.history_page(&request("STRAßE", 100)).unwrap();
        assert_eq!(street.entries.len(), 1);
        assert_eq!(street.entries[0].id, "street");
        assert!(
            store
                .history_page(&request("STRASSE", 100))
                .unwrap()
                .entries
                .is_empty()
        );
        let sigma = store.history_page(&request("σ", 100)).unwrap();
        assert_eq!(sigma.entries.len(), 1);
        assert_eq!(sigma.entries[0].id, "sigma");
        assert!(
            store
                .history_page(&request("ς", 100))
                .unwrap()
                .entries
                .is_empty()
        );
    }

    #[test]
    fn history_search_preserves_human_execution_and_snowflake_outcome_terms() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let mut script = entry("script", 1, None);
        script.status = Some(RunStatus::Stopped);
        script.execution_summary = Some(ExecutionSummary {
            planned: 3,
            attempted: 1,
            completed: 1,
            not_attempted: 2,
            stop_reason: Some(ExecutionStopReason::ResultLimit),
        });
        let mut dml = entry("dml", 2, None);
        dml.status = Some(RunStatus::OutcomeUnknown);
        dml.snowflake_dml = Some(SnowflakeDmlEvidence {
            kind: SnowflakeDmlKind::Update,
            statement_outcome: SnowflakeDmlOutcome::Unknown,
            summary: SnowflakeDmlSummary::unavailable(),
            retired: true,
        });
        let mut ddl = entry("ddl", 3, None);
        ddl.snowflake_ddl = Some(SnowflakeDdlEvidence {
            kind: SnowflakeDdlKind::CreateTable,
            qualified_target: "\"DB\".\"S\".\"T\"".into(),
            statement_outcome: SnowflakeDdlOutcome::Succeeded,
            summary_status: SnowflakeDdlSummaryStatus::Acknowledged,
            catalog_reconciliation_required: true,
            retired: false,
        });
        seed(&store, &[script, dml, ddl]);
        for (query, id) in [
            ("stopped retention not attempted: 2", "script"),
            ("snowflake update outcome unknown retired", "dml"),
            ("create table reconciliation acknowledged", "ddl"),
        ] {
            let page = store.history_page(&request(query, 100)).unwrap();
            assert_eq!(page.entries.len(), 1);
            assert_eq!(page.entries[0].id, id);
        }
    }

    #[test]
    fn history_metadata_search_matches_visible_dml_counts_and_ddl_summary_status() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let mut update = entry("update", 1, None);
        update.snowflake_dml = Some(SnowflakeDmlEvidence {
            kind: SnowflakeDmlKind::Update,
            statement_outcome: SnowflakeDmlOutcome::Succeeded,
            summary: SnowflakeDmlSummary {
                status: SnowflakeDmlSummaryStatus::Validated,
                affected_rows: Some("10".into()),
                multi_joined_rows: Some("7".into()),
            },
            retired: false,
        });
        let mut unvalidated = update.clone();
        unvalidated.id = "unvalidated".into();
        unvalidated.connection_name = "UnvalidatedFixture".into();
        unvalidated.snowflake_dml.as_mut().unwrap().summary.status =
            SnowflakeDmlSummaryStatus::Unavailable;
        let mut insert = update.clone();
        insert.id = "insert".into();
        insert.snowflake_dml.as_mut().unwrap().kind = SnowflakeDmlKind::Insert;
        let mut ddl = entry("ddl-unavailable", 2, None);
        ddl.snowflake_ddl = Some(SnowflakeDdlEvidence {
            kind: SnowflakeDdlKind::AddColumn,
            qualified_target: "\"DB\".\"S\".\"T\"".into(),
            statement_outcome: SnowflakeDdlOutcome::Succeeded,
            summary_status: SnowflakeDdlSummaryStatus::Unavailable,
            catalog_reconciliation_required: false,
            retired: false,
        });
        let mut acknowledged = ddl.clone();
        acknowledged.id = "ddl-acknowledged".into();
        acknowledged.snowflake_ddl.as_mut().unwrap().summary_status =
            SnowflakeDdlSummaryStatus::Acknowledged;
        seed(&store, &[update, unvalidated, insert, ddl, acknowledged]);
        for (query, id) in [
            ("multi-joined 7", "update"),
            ("affected-row summary unavailable", "unvalidated"),
            ("status summary unavailable", "ddl-unavailable"),
        ] {
            let page = store.history_page(&request(query, 100)).unwrap();
            assert_eq!(page.entries.len(), 1, "query: {query}");
            assert_eq!(page.entries[0].id, id);
            let compatibility = store.history(query).unwrap();
            assert_eq!(compatibility.len(), 1, "compatibility query: {query}");
            assert_eq!(compatibility[0].id, id);
            assert!(page.entries[0].sql_preview.is_none());
            assert!(!page.entries[0].sql_stored);
        }
        assert!(
            store
                .history_page(&request("UnvalidatedFixture 10", 100))
                .unwrap()
                .entries
                .is_empty()
        );
    }

    #[test]
    fn history_detail_reopen_and_clear_do_not_replay_or_return_stale_rows() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let sql = "  SELECT '雪';\n-- preserve exact whitespace\n";
        seed(&store, &[entry("saved", 50, Some(sql))]);
        drop(store);
        let store = Persistence::open(root.path()).unwrap();
        assert_eq!(
            store
                .history_entry("saved")
                .unwrap()
                .unwrap()
                .sql
                .as_deref(),
            Some(sql)
        );
        let page = store.history_page(&request("", 1)).unwrap();
        assert_eq!(page.entries.len(), 1);
        store.clear_history().unwrap();
        assert!(store.history_entry("saved").unwrap().is_none());
        assert!(store.history_entry("missing").unwrap().is_none());
        assert!(
            store
                .history_page(&request("", 100))
                .unwrap()
                .entries
                .is_empty()
        );
        assert_eq!(
            store
                .connection
                .lock()
                .query_row("SELECT count(*) FROM history_v8", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn history_page_and_detail_reject_invalid_requests_and_unsafe_cursor_numbers() {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        for limit in [0, 101, u32::MAX] {
            assert!(store.history_page(&request("", limit)).is_err());
        }
        for search in ["x".repeat(513), "é".repeat(257)] {
            assert!(store.history_page(&request(&search, 1)).is_err());
        }
        assert!(store.history_page(&request(&"é".repeat(256), 1)).is_ok());
        for id in ["".to_owned(), "x".repeat(257), "nul\0id".to_owned()] {
            assert!(store.history_entry(&id).is_err());
            let mut input = request("", 1);
            input.cursor = Some(HistoryCursor { started_at: 1, id });
            assert!(store.history_page(&input).is_err());
        }
        for started_at in [MAX_SAFE_TIMESTAMP + 1, i64::MAX as u64, u64::MAX] {
            let mut input = request("", 1);
            input.cursor = Some(HistoryCursor {
                started_at,
                id: "valid".into(),
            });
            assert!(store.history_page(&input).is_err());
        }
        let mut input = request("", 1);
        input.cursor = Some(HistoryCursor {
            started_at: MAX_SAFE_TIMESTAMP,
            id: "valid".into(),
        });
        assert!(store.history_page(&input).is_ok());
    }

    #[test]
    fn history_readers_reject_mismatched_physical_identity_time_and_invalid_sql_type() {
        for update in [
            "json_set(json,'$.id','foreign')",
            "json_set(json,'$.startedAt',2)",
            "json_set(json,'$.sql',42)",
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = Persistence::open(root.path()).unwrap();
            seed(&store, &[entry("original", 1, Some("SELECT 1"))]);
            store
                .connection
                .lock()
                .execute(&format!("UPDATE history_v8 SET json={update}"), [])
                .unwrap();
            assert!(store.history_page(&request("", 100)).is_err());
            assert!(store.history_entry("original").is_err());
            assert!(store.history("").is_err());
        }
    }
}
