use joinery_core::{
    domain::{
        ConnectionProfile, EngineKind, Environment, HistoryEntry, HistoryPageRequest,
        HistoryProvenance, HistorySummary, SafeMode, TlsMode, now_ms,
    },
    persistence::Persistence,
};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path};

fn open_store(root: &Path) -> Persistence {
    let store = Persistence::open(root).unwrap();
    store.save_profile(&profile_fixture()).unwrap();
    store
}

fn profile_fixture() -> ConnectionProfile {
    ConnectionProfile {
        id: "9d1cc438-08d1-43cd-9fac-8478beb301bb".into(),
        name: "Disposable history fixture".into(),
        group_id: "local".into(),
        engine: EngineKind::Postgres,
        host: "127.0.0.1".into(),
        port: 55432,
        database: "joinery".into(),
        username: "joinery".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::ReadOnly,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: true,
        store_drafts: false,
        snowflake: None,
    }
}

fn store_fixture(store: &Persistence, entry: &HistoryEntry) {
    if entry.provenance == HistoryProvenance::Admitted {
        store.begin_history(entry).unwrap();
    } else {
        store.record_history(entry).unwrap();
    }
}

#[test]
fn admitted_history_roundtrip_preserves_unknown_status_and_duration() {
    let input = json!({
        "id": "d3db5e9f-48a2-42ec-a23d-9718f46ce306",
        "connectionId": "9d1cc438-08d1-43cd-9fac-8478beb301bb",
        "connectionName": "Disposable history fixture",
        "startedAt": 1_800_000_000_000_u64,
        "sql": "SELECT 'pending history fixture'",
        "statementCount": 1,
        "provenance": "admitted",
        "status": null,
        "elapsedMs": null
    });

    let entry: HistoryEntry = serde_json::from_value(input.clone())
        .expect("admitted history must retain unknown completion status and duration");
    let output = serde_json::to_value(entry).expect("history must serialize");

    assert_eq!(output["provenance"], "admitted");
    assert_eq!(output.get("status"), Some(&serde_json::Value::Null));
    assert_eq!(output.get("elapsedMs"), Some(&serde_json::Value::Null));
    assert_eq!(output["sql"], input["sql"]);
    assert_eq!(output["id"], input["id"]);
}

#[test]
fn history_provenance_requires_a_string_instead_of_a_unit_variant_object() {
    for (provenance, status) in [
        ("recorded", json!("completed")),
        ("admitted", Value::Null),
        ("interrupted", Value::Null),
        ("restoreRecovered", Value::Null),
    ] {
        let mut value = fixture("object-provenance", provenance, status, Value::Null);
        value["provenance"] = json!({ provenance: null });
        assert!(
            serde_json::from_value::<HistoryEntry>(value).is_err(),
            "provenance must be a closed string: {provenance}"
        );
    }
}

fn fixture(id: &str, provenance: &str, status: Value, elapsed: Value) -> Value {
    json!({
        "id": id,
        "connectionId": "9d1cc438-08d1-43cd-9fac-8478beb301bb",
        "connectionName": "Disposable fixture",
        "startedAt": now_ms(),
        "sql": format!("SELECT '{}needle_{id}'", "x".repeat(300)),
        "statementCount": 1,
        "provenance": provenance,
        "status": status,
        "elapsedMs": elapsed
    })
}

fn decode(value: Value) -> HistoryEntry {
    serde_json::from_value(value).expect("well-shaped history fixture")
}

fn encoded(entry: &HistoryEntry) -> Value {
    serde_json::to_value(entry).unwrap()
}

fn page_request(search: &str, limit: u32) -> HistoryPageRequest {
    HistoryPageRequest {
        search: search.into(),
        cursor: None,
        limit,
    }
}

fn assert_summary(summary: &HistorySummary, expected: &Value) {
    let summary = serde_json::to_value(summary).unwrap();
    for key in [
        "id",
        "connectionId",
        "connectionName",
        "startedAt",
        "statementCount",
        "provenance",
        "status",
        "elapsedMs",
    ] {
        assert_eq!(summary.get(key), expected.get(key), "field {key}");
    }
    assert_eq!(summary["sqlStored"], expected["sql"].is_string());
    assert!(summary.get("sql").is_none());
}

fn insert_raw(root: &Path, value: &Value) {
    let connection = rusqlite::Connection::open(root.join("joinery.sqlite")).unwrap();
    connection
        .execute(
            "INSERT INTO history_v8(id,started_at,json) VALUES(?1,?2,?3)",
            rusqlite::params![
                value["id"].as_str().unwrap(),
                i64::try_from(value["startedAt"].as_u64().unwrap()).unwrap(),
                value.to_string()
            ],
        )
        .unwrap();
}

fn valid_matrix() -> Vec<HistoryEntry> {
    let mut entries = vec![decode(fixture(
        "pending",
        "admitted",
        Value::Null,
        Value::Null,
    ))];
    for provenance in ["recorded", "interrupted", "restoreRecovered"] {
        for status in [
            "completed",
            "stopped",
            "failed",
            "cancelled",
            "outcomeUnknown",
        ] {
            for (duration, elapsed) in [("measured", json!(17)), ("unavailable", Value::Null)] {
                let id = format!("{provenance}-{status}-{duration}");
                entries.push(decode(fixture(&id, provenance, json!(status), elapsed)));
            }
        }
        if provenance != "recorded" {
            entries.push(decode(fixture(
                &format!("{provenance}-unknown"),
                provenance,
                Value::Null,
                Value::Null,
            )));
        }
    }
    entries
}

#[test]
fn history_lifecycle_restart_preserves_every_valid_state_in_all_read_modes() {
    let root = tempfile::tempdir().unwrap();
    let expected: HashMap<_, _> = valid_matrix()
        .into_iter()
        .map(|entry| (entry.id.clone(), encoded(&entry)))
        .collect();
    {
        let store = open_store(root.path());
        for original in expected.values() {
            store_fixture(&store, &decode(original.clone()));
        }
    }

    let store = open_store(root.path());
    let mut request = page_request("", 3);
    let mut seen = HashMap::new();
    loop {
        let page = store.history_page(&request).unwrap();
        assert!(page.entries.len() <= 3);
        for summary in page.entries {
            let original = &expected[&summary.id];
            assert_summary(&summary, original);
            assert!(seen.insert(summary.id.clone(), ()).is_none());
            let detail = store.history_entry(&summary.id).unwrap().unwrap();
            assert_eq!(encoded(&detail), *original);

            // The distinctive SQL token is beyond the bounded preview.
            let needle = format!("needle_{}", summary.id);
            assert!(!summary.sql_preview.as_deref().unwrap().contains(&needle));
            let matches = store.history_page(&page_request(&needle, 1)).unwrap();
            assert_eq!(matches.entries.len(), 1);
            assert_summary(&matches.entries[0], original);
            let compatibility = store.history(&needle).unwrap();
            assert_eq!(compatibility.len(), 1);
            assert_eq!(encoded(&compatibility[0]), *original);
        }
        let Some(cursor) = page.next_cursor else {
            break;
        };
        request.cursor = Some(cursor);
    }
    assert_eq!(seen.len(), expected.len());
    let compatibility = store.history("").unwrap();
    assert_eq!(compatibility.len(), expected.len());
    for entry in compatibility {
        assert_eq!(encoded(&entry), expected[&entry.id]);
    }
}

#[test]
fn history_lifecycle_search_matches_provenance_without_inventing_completion() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(root.path());
    for (id, provenance, status) in [
        ("one", "recorded", json!("completed")),
        ("two", "admitted", Value::Null),
        ("three", "interrupted", Value::Null),
        ("four", "restoreRecovered", Value::Null),
    ] {
        let mut value = fixture(id, provenance, status, Value::Null);
        value["sql"] = json!("SELECT 'neutral text'");
        store_fixture(&store, &decode(value));
    }
    for (term, id) in [
        ("admitted", "two"),
        ("completion pending", "two"),
        ("interrupted", "three"),
        ("restored", "four"),
    ] {
        let page = store.history_page(&page_request(term, 10)).unwrap();
        assert_eq!(page.entries.len(), 1, "search {term}");
        assert_eq!(page.entries[0].id, id, "search {term}");
        let compatibility = store.history(term).unwrap();
        assert_eq!(compatibility.len(), 1, "compatibility search {term}");
        assert_eq!(compatibility[0].id, id);
    }
    // Search matches words across metadata; "counts were not recorded" is
    // searchable too, so "recorded" is not an exclusive provenance filter.
    assert_eq!(store.history("recorded").unwrap().len(), 4);
    assert_eq!(
        store
            .history_page(&page_request("recorded", 10))
            .unwrap()
            .entries
            .len(),
        4
    );
    for id in ["two", "three", "four"] {
        let entry = encoded(&store.history_entry(id).unwrap().unwrap());
        assert_eq!(entry.get("status"), Some(&Value::Null));
        assert_eq!(entry.get("elapsedMs"), Some(&Value::Null));
    }
}

#[test]
fn legacy_history_without_provenance_remains_recorded_after_restart_and_backup() {
    let root = tempfile::tempdir().unwrap();
    // Create the actual supported version-one format. Inserting unticketed
    // rows into the current schema would test corruption, not legacy migration.
    let legacy = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
    legacy.execute_batch(
        "CREATE TABLE profiles(id TEXT PRIMARY KEY,json TEXT NOT NULL);
         CREATE TABLE groups(id TEXT PRIMARY KEY,json TEXT NOT NULL);
         CREATE TABLE history(id TEXT PRIMARY KEY,started_at INTEGER NOT NULL,json TEXT NOT NULL);
         CREATE TABLE audit(id TEXT PRIMARY KEY,occurred_at INTEGER NOT NULL,connection_id TEXT NOT NULL,action TEXT NOT NULL,digest TEXT NOT NULL,outcome TEXT NOT NULL);
         CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
         PRAGMA user_version=1;",
    ).unwrap();
    let profile = profile_fixture();
    legacy
        .execute(
            "INSERT INTO profiles VALUES(?1,?2)",
            rusqlite::params![profile.id, serde_json::to_string(&profile).unwrap()],
        )
        .unwrap();
    for status in [
        "completed",
        "stopped",
        "failed",
        "cancelled",
        "outcomeUnknown",
    ] {
        let mut value = fixture(status, "recorded", json!(status), json!(23));
        value.as_object_mut().unwrap().remove("provenance");
        legacy
            .execute(
                "INSERT INTO history(id,started_at,json) VALUES(?1,?2,?3)",
                rusqlite::params![
                    status,
                    i64::try_from(value["startedAt"].as_u64().unwrap()).unwrap(),
                    value.to_string()
                ],
            )
            .unwrap();
    }
    drop(legacy);
    let store = open_store(root.path());
    let page = store.history_page(&page_request("recorded", 10)).unwrap();
    assert_eq!(page.entries.len(), 5);
    assert_eq!(store.history("recorded").unwrap().len(), 5);
    for summary in &page.entries {
        let entry = store.history_entry(&summary.id).unwrap().unwrap();
        let value = encoded(&entry);
        assert_eq!(value["provenance"], "recorded");
        assert_eq!(value["status"], summary.id);
        assert_eq!(value["elapsedMs"], 23);
        assert_summary(summary, &value);
    }
    assert!(store.create_metadata_backup().is_ok());
    drop(store);
    let store = open_store(root.path());
    assert_eq!(store.history("recorded").unwrap().len(), 5);
}

fn invalid_semantic_entries() -> Vec<HistoryEntry> {
    let mut entries = vec![
        decode(fixture(
            "recorded-unknown",
            "recorded",
            Value::Null,
            Value::Null,
        )),
        decode(fixture(
            "pending-completed",
            "admitted",
            json!("completed"),
            Value::Null,
        )),
        decode(fixture("pending-timed", "admitted", Value::Null, json!(1))),
        decode(fixture(
            "interrupted-timed",
            "interrupted",
            Value::Null,
            json!(1),
        )),
        decode(fixture(
            "restored-timed",
            "restoreRecovered",
            Value::Null,
            json!(1),
        )),
    ];
    let evidence = [
        (
            "executionSummary",
            json!({
                "planned": 1, "attempted": 1, "completed": 1,
                "notAttempted": 0, "stopReason": null
            }),
        ),
        (
            "snowflakeDml",
            json!({
                "kind": "insert", "statementOutcome": "succeeded",
                "summary": {"status": "unavailable", "affectedRows": null, "multiJoinedRows": null},
                "retired": false
            }),
        ),
        (
            "snowflakeDdl",
            json!({
                "kind": "createTable", "qualifiedTarget": "\"DB\".\"SCHEMA\".\"T\"",
                "statementOutcome": "succeeded", "summaryStatus": "unavailable",
                "catalogReconciliationRequired": true, "retired": false
            }),
        ),
    ];
    for (field, evidence) in evidence {
        let mut value = fixture(field, "admitted", Value::Null, Value::Null);
        value[field] = evidence;
        entries.push(decode(value));
    }
    entries
}

#[test]
fn history_lifecycle_public_writes_reject_inconsistent_typed_entries() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(root.path());
    for entry in invalid_semantic_entries() {
        assert!(store.record_history(&entry).is_err(), "{}", entry.id);
        assert!(store.history_entry(&entry.id).unwrap().is_none());
    }
    assert!(store.history("").unwrap().is_empty());
    assert!(
        store
            .history_page(&page_request("", 10))
            .unwrap()
            .entries
            .is_empty()
    );
}

#[test]
fn history_lifecycle_invalid_persisted_rows_fail_reads_and_backup_validation() {
    let mut invalid: Vec<Value> = invalid_semantic_entries().iter().map(encoded).collect();
    for provenance in [
        Value::Null,
        json!("futureState"),
        json!({"admitted": null}),
        json!({"interrupted": null}),
        json!({"restoreRecovered": null}),
    ] {
        let mut value = fixture("invalid-provenance", "admitted", Value::Null, Value::Null);
        value["provenance"] = provenance;
        assert!(serde_json::from_value::<HistoryEntry>(value.clone()).is_err());
        invalid.push(value);
    }
    for value in invalid {
        let root = tempfile::tempdir().unwrap();
        let store = open_store(root.path());
        insert_raw(root.path(), &value);
        let id = value["id"].as_str().unwrap();
        assert!(store.history_entry(id).is_err(), "detail {id}");
        assert!(store.history("").is_err(), "compatibility {id}");
        assert!(
            store.history_page(&page_request("", 1)).is_err(),
            "page {id}"
        );
        assert!(store.create_metadata_backup().is_err(), "backup {id}");
        assert!(
            store.metadata_backups().unwrap().is_empty(),
            "published {id}"
        );
    }
}

#[test]
fn history_lifecycle_backup_copy_reopens_with_exact_sql_and_nullable_evidence() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(root.path());
    let originals = valid_matrix();
    for entry in &originals {
        store_fixture(&store, entry);
    }
    let backup = store.create_metadata_backup().unwrap();
    let backup_path = root
        .path()
        .join("backups")
        .join(format!("{}.sqlite", backup.id));
    let snapshot = rusqlite::Connection::open(&backup_path).unwrap();
    for entry in &originals {
        let raw: String = snapshot
            .query_row(
                "SELECT json FROM history_v8 WHERE id=?1",
                [&entry.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), encoded(entry));
    }
    drop(snapshot);
    store.clear_history().unwrap();
    assert!(store.history("").unwrap().is_empty());

    // This opens a closed SQLite snapshot in a fresh disposable root. It does
    // not simulate owner recovery or the product restore/merge workflow.
    let restored_root = tempfile::tempdir().unwrap();
    std::fs::copy(&backup_path, restored_root.path().join("joinery.sqlite")).unwrap();
    let reopened = open_store(restored_root.path());
    assert_eq!(reopened.history("").unwrap().len(), originals.len());
    for entry in originals {
        assert_eq!(
            encoded(&reopened.history_entry(&entry.id).unwrap().unwrap()),
            encoded(&entry)
        );
        let page = reopened
            .history_page(&page_request(&format!("needle_{}", entry.id), 1))
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_summary(&page.entries[0], &encoded(&entry));
    }
}
