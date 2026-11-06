use joinery_core::domain::{
    ConnectionProfile, EngineKind, Environment, QueryParamInput, QueryParamKind, SafeMode,
    Scratchpad, TlsMode,
};
use joinery_core::execution::Studio;
use joinery_core::file_capability::SandboxFileDialog;
use joinery_core::persistence::Persistence;
use joinery_core::secrets::MemorySecrets;
use serde_json::json;
use std::sync::Arc;
use tempfile::tempdir;

fn make_test_profile(id: &str, name: &str) -> ConnectionProfile {
    ConnectionProfile {
        id: id.to_owned(),
        name: name.to_owned(),
        group_id: "local".to_owned(),
        engine: EngineKind::Postgres,
        host: "127.0.0.1".to_owned(),
        port: 5432,
        database: "test_db".to_owned(),
        username: "test_user".to_owned(),
        environment: Environment::Local,
        safe_mode: SafeMode::Silent,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: true,
        store_drafts: true,
        snowflake: None,
    }
}

// =========================================================================
// CHALLENGE 1: Scratchpad IPC Contract: Flat vs Wrapped Payloads & Field Preservation
// =========================================================================

#[test]
fn test_scratchpad_flat_and_wrapped_payload_compatibility() {
    let dir = tempdir().unwrap();
    let persistence = Arc::new(Persistence::open(dir.path()).unwrap());

    // 1. Flat payload: save_scratchpad_fields with all fields
    let saved_flat = persistence
        .save_scratchpad_fields(
            Some("sp-flat-1".into()),
            Some("Flat Scratchpad".into()),
            "SELECT 'flat';".into(),
            Some("conn-profile-1".into()),
        )
        .expect("Flat payload with all fields should succeed");

    assert_eq!(saved_flat.id, "sp-flat-1");
    assert_eq!(saved_flat.name, "Flat Scratchpad");
    assert_eq!(saved_flat.content, "SELECT 'flat';");
    assert_eq!(saved_flat.connection_id.as_deref(), Some("conn-profile-1"));
    assert!(saved_flat.updated_at > 0);

    // 2. Wrapped payload: save_scratchpad with Scratchpad struct
    let wrapped_struct = Scratchpad {
        id: "sp-wrapped-2".into(),
        name: "Wrapped Scratchpad".into(),
        content: "SELECT 'wrapped';".into(),
        connection_id: Some("conn-profile-2".into()),
        updated_at: 0,
    };
    let saved_wrapped = persistence
        .save_scratchpad(&wrapped_struct)
        .expect("Wrapped payload should succeed");

    assert_eq!(saved_wrapped.id, "sp-wrapped-2");
    assert_eq!(saved_wrapped.name, "Wrapped Scratchpad");
    assert_eq!(saved_wrapped.content, "SELECT 'wrapped';");
    assert_eq!(
        saved_wrapped.connection_id.as_deref(),
        Some("conn-profile-2")
    );

    // 3. Flat payload with omitted ID: auto-generates a new unique ID
    let saved_new = persistence
        .save_scratchpad_fields(
            None,
            Some("Brand New Scratchpad".into()),
            "SELECT 42;".into(),
            None,
        )
        .expect("Omitted ID should auto-generate new ID");

    assert!(!saved_new.id.is_empty());
    assert_eq!(saved_new.name, "Brand New Scratchpad");
    assert_eq!(saved_new.content, "SELECT 42;");
    assert_eq!(saved_new.connection_id, None);
}

#[test]
fn test_studio_save_scratchpad_json_rpc_dispatch_flat_and_wrapped() {
    let dir = tempdir().unwrap();
    let sandbox = SandboxFileDialog::new(&dir.path().join("sandbox-files")).unwrap();
    let studio =
        Studio::with_secrets_and_dialog(dir.path(), Arc::new(MemorySecrets::default()), sandbox)
            .unwrap();

    // Helper function mimicking test-harness arg extraction
    fn dispatch_save_scratchpad(studio: &Studio, args: serde_json::Value) -> Scratchpad {
        let (id, name, content, connection_id) = if let Some(sp) = args.get("scratchpad") {
            (
                sp.get("id").and_then(|v| v.as_str()).map(String::from),
                sp.get("name").and_then(|v| v.as_str()).map(String::from),
                sp.get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                sp.get("connectionId")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            )
        } else {
            (
                args.get("id").and_then(|v| v.as_str()).map(String::from),
                args.get("name").and_then(|v| v.as_str()).map(String::from),
                args.get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                args.get("connectionId")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            )
        };
        studio
            .save_scratchpad(id, name, content, connection_id)
            .unwrap()
    }

    // 1. Dispatch with flat JSON payload as sent by frontend createScratchpad
    let flat_create = json!({
        "name": "Frontend Scratchpad",
        "content": "-- Scratchpad\n",
        "connectionId": null
    });
    let created = dispatch_save_scratchpad(&studio, flat_create);
    assert!(!created.id.is_empty());
    assert_eq!(created.name, "Frontend Scratchpad");
    assert_eq!(created.content, "-- Scratchpad\n");
    assert_eq!(created.connection_id, None);

    // 2. Dispatch with flat auto-save payload (omitting name and connectionId)
    let flat_autosave = json!({
        "id": created.id.clone(),
        "content": "SELECT 'autosaved content';"
    });
    let updated = dispatch_save_scratchpad(&studio, flat_autosave);
    assert_eq!(updated.id, created.id);
    assert_eq!(
        updated.name, "Frontend Scratchpad",
        "Name must be preserved on flat auto-save!"
    );
    assert_eq!(updated.content, "SELECT 'autosaved content';");

    // 3. Dispatch with wrapped JSON payload: { "scratchpad": { ... } }
    let wrapped = json!({
        "scratchpad": {
            "id": "sp-wrapped-rpc",
            "name": "Wrapped RPC Scratchpad",
            "content": "SELECT 33;",
            "connectionId": "conn-xyz",
            "updatedAt": 1730000000000u64
        }
    });
    let saved_wrapped = dispatch_save_scratchpad(&studio, wrapped);
    assert_eq!(saved_wrapped.id, "sp-wrapped-rpc");
    assert_eq!(saved_wrapped.name, "Wrapped RPC Scratchpad");
    assert_eq!(saved_wrapped.content, "SELECT 33;");
    assert_eq!(saved_wrapped.connection_id.as_deref(), Some("conn-xyz"));
}

#[test]
fn test_scratchpad_autosave_preserves_existing_name_and_connection_id_in_sqlite() {
    let dir = tempdir().unwrap();
    let persistence = Arc::new(Persistence::open(dir.path()).unwrap());

    let initial_id = "sp-autosave-target";
    let initial_name = "My Production Queries";
    let initial_conn = "prod-db-conn";

    // Create original scratchpad
    persistence
        .save_scratchpad_fields(
            Some(initial_id.into()),
            Some(initial_name.into()),
            "-- Initial content\n".into(),
            Some(initial_conn.into()),
        )
        .unwrap();

    // Verify in SQLite directly
    let raw_conn = rusqlite::Connection::open(dir.path().join("joinery.sqlite")).unwrap();
    let (db_name, db_conn): (String, Option<String>) = raw_conn
        .query_row(
            "SELECT name, connection_id FROM scratchpads WHERE id = ?1",
            [initial_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(db_name, initial_name);
    assert_eq!(db_conn.as_deref(), Some(initial_conn));

    // Simulate auto-save: only pass id and new content, with name = None and connection_id = None
    let updated = persistence
        .save_scratchpad_fields(
            Some(initial_id.into()),
            None, // Omitted name!
            "SELECT * FROM large_table; -- updated".into(),
            None, // Omitted connection_id!
        )
        .expect("Auto-save without name or connection_id must succeed");

    assert_eq!(updated.id, initial_id);
    assert_eq!(updated.name, initial_name, "Name must be preserved!");
    assert_eq!(
        updated.connection_id.as_deref(),
        Some(initial_conn),
        "Connection ID must be preserved!"
    );
    assert_eq!(updated.content, "SELECT * FROM large_table; -- updated");

    // Check raw SQLite table: row MUST still have initial name and connection_id!
    let (persisted_name, persisted_conn, persisted_content): (String, Option<String>, String) =
        raw_conn
            .query_row(
                "SELECT name, connection_id, content FROM scratchpads WHERE id = ?1",
                [initial_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();

    assert_eq!(persisted_name, initial_name);
    assert_eq!(persisted_conn.as_deref(), Some(initial_conn));
    assert_eq!(persisted_content, "SELECT * FROM large_table; -- updated");
}

#[test]
fn test_scratchpad_updated_at_serialization_no_bigint_hazard() {
    let now = 1730000000123u64;
    let sp = Scratchpad {
        id: "sp-json-test".into(),
        name: "JSON Serialization Test".into(),
        content: "SELECT 1;".into(),
        connection_id: None,
        updated_at: now,
    };

    let serialized = serde_json::to_string(&sp).unwrap();

    // Verify updatedAt is serialized as a plain JSON number, not a string or object
    assert!(
        serialized.contains(r#""updatedAt":1730000000123"#),
        "updatedAt must be serialized as a plain number, got: {serialized}"
    );

    // Verify it deserializes back identically
    let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert!(parsed["updatedAt"].is_number());
    assert_eq!(parsed["updatedAt"].as_u64(), Some(now));

    let deserialized: Scratchpad = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.updated_at, now);
}

// =========================================================================
// CHALLENGE 2: SQLite Parameter History LRU Bounds & Strict Isolation
// =========================================================================

#[test]
fn test_parameter_history_1200_distinct_inserts_strictly_bounded_to_1000() {
    let dir = tempdir().unwrap();
    let persistence = Persistence::open(dir.path()).unwrap();
    let prof_id = "prof-lru-challenge";

    let profile = make_test_profile(prof_id, "LRU Profile");
    persistence.save_profile(&profile).unwrap();

    // Insert 1,200 distinct parameters
    let inputs: Vec<QueryParamInput> = (0..1200)
        .map(|i| QueryParamInput {
            key: format!("challenge_param_{i:04}"),
            kind: QueryParamKind::Text,
            value: Some(format!("value_{i}")),
        })
        .collect();

    persistence
        .save_parameter_history(prof_id, &inputs)
        .unwrap();

    // Check direct SQLite row count
    let raw_conn = rusqlite::Connection::open(dir.path().join("joinery.sqlite")).unwrap();
    let count: i64 = raw_conn
        .query_row(
            "SELECT COUNT(*) FROM parameters WHERE profile_id = ?1",
            [prof_id],
            |r| r.get(0),
        )
        .unwrap();

    assert_eq!(
        count, 1000,
        "Raw SQLite count for profile must be strictly 1,000, got: {count}"
    );
}

#[test]
fn test_parameter_history_profile_isolation_under_lru_pruning() {
    let dir = tempdir().unwrap();
    let persistence = Persistence::open(dir.path()).unwrap();

    let prof_a = "profile-alpha";
    let prof_b = "profile-beta";

    persistence
        .save_profile(&make_test_profile(prof_a, "Alpha"))
        .unwrap();
    persistence
        .save_profile(&make_test_profile(prof_b, "Beta"))
        .unwrap();

    // 1. Seed Profile B with 500 distinct parameters
    let inputs_b: Vec<QueryParamInput> = (0..500)
        .map(|i| QueryParamInput {
            key: format!("beta_param_{i:04}"),
            kind: QueryParamKind::Integer,
            value: Some(i.to_string()),
        })
        .collect();
    persistence
        .save_parameter_history(prof_b, &inputs_b)
        .unwrap();

    let raw_conn = rusqlite::Connection::open(dir.path().join("joinery.sqlite")).unwrap();
    let count_b_initial: i64 = raw_conn
        .query_row(
            "SELECT COUNT(*) FROM parameters WHERE profile_id = ?1",
            [prof_b],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count_b_initial, 500);

    // 2. Insert 1,200 parameters into Profile A (triggering eviction of 200 parameters in Profile A)
    let inputs_a: Vec<QueryParamInput> = (0..1200)
        .map(|i| QueryParamInput {
            key: format!("alpha_param_{i:04}"),
            kind: QueryParamKind::Text,
            value: Some(format!("alpha_val_{i}")),
        })
        .collect();
    persistence
        .save_parameter_history(prof_a, &inputs_a)
        .unwrap();

    // 3. Verify Profile A is capped at 1,000
    let count_a: i64 = raw_conn
        .query_row(
            "SELECT COUNT(*) FROM parameters WHERE profile_id = ?1",
            [prof_a],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count_a, 1000, "Profile A must have exactly 1,000 rows");

    // 4. Verify Profile B STILL has all 500 parameters (ZERO parameters lost)
    let count_b_after: i64 = raw_conn
        .query_row(
            "SELECT COUNT(*) FROM parameters WHERE profile_id = ?1",
            [prof_b],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count_b_after, 500,
        "Profile B must remain completely unaffected with 500 rows"
    );

    // Total rows across both profiles must be 1,500
    let total_rows: i64 = raw_conn
        .query_row("SELECT COUNT(*) FROM parameters", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        total_rows, 1500,
        "Total parameter rows across both profiles must equal 1,000 + 500 = 1,500"
    );

    // Check specific keys in Profile B
    let beta_history = persistence.parameter_history(prof_b).unwrap();
    assert_eq!(beta_history.len(), 500);
    assert!(beta_history.iter().all(|e| e.profile_id == prof_b));
    assert!(
        beta_history
            .iter()
            .any(|e| e.param_key == "beta_param_0000")
    );
    assert!(
        beta_history
            .iter()
            .any(|e| e.param_key == "beta_param_0499")
    );
}

#[test]
fn test_parameter_history_incremental_inserts_preserve_lru_order() {
    let dir = tempdir().unwrap();
    let persistence = Persistence::open(dir.path()).unwrap();
    let prof_id = "prof-incremental";

    // 1. Insert 5 "oldest" items first
    let oldest: Vec<QueryParamInput> = (0..5)
        .map(|i| QueryParamInput {
            key: format!("oldest_{i}"),
            kind: QueryParamKind::Text,
            value: Some(format!("old_val_{i}")),
        })
        .collect();
    persistence
        .save_parameter_history(prof_id, &oldest)
        .unwrap();

    // Sleep so the next batch has a strictly newer used_at timestamp
    std::thread::sleep(std::time::Duration::from_millis(15));

    // 2. Insert 995 newer items (reaching 1000 items total)
    let batch: Vec<QueryParamInput> = (0..995)
        .map(|i| QueryParamInput {
            key: format!("mid_{i:04}"),
            kind: QueryParamKind::Text,
            value: Some(format!("v_{i}")),
        })
        .collect();
    persistence.save_parameter_history(prof_id, &batch).unwrap();

    let raw_conn = rusqlite::Connection::open(dir.path().join("joinery.sqlite")).unwrap();
    let count1: i64 = raw_conn
        .query_row(
            "SELECT COUNT(*) FROM parameters WHERE profile_id = ?1",
            [prof_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count1, 1000);

    // Sleep again
    std::thread::sleep(std::time::Duration::from_millis(15));

    // 3. Insert 5 newest items incrementally (exceeding 1000 by 5)
    let newest: Vec<QueryParamInput> = (0..5)
        .map(|i| QueryParamInput {
            key: format!("newest_{i}"),
            kind: QueryParamKind::Text,
            value: Some(format!("new_val_{i}")),
        })
        .collect();
    persistence
        .save_parameter_history(prof_id, &newest)
        .unwrap();

    let count2: i64 = raw_conn
        .query_row(
            "SELECT COUNT(*) FROM parameters WHERE profile_id = ?1",
            [prof_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count2, 1000, "Count must stay bounded at 1,000");

    // The newly inserted items (newest_0..newest_4) must definitely be present!
    let history = persistence.parameter_history(prof_id).unwrap();
    for i in 0..5 {
        let key = format!("newest_{i}");
        assert!(
            history.iter().any(|e| e.param_key == key),
            "Recently inserted key {key} must be preserved"
        );
    }

    // The 5 oldest items (which have the lowest used_at) MUST have been evicted!
    let remaining_old: i64 = raw_conn
        .query_row(
            "SELECT COUNT(*) FROM parameters WHERE profile_id = ?1 AND param_key LIKE 'oldest_%'",
            [prof_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(remaining_old, 0, "All 5 oldest keys must have been evicted");
}
