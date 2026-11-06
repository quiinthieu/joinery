use joinery_core::domain::*;

#[test]
fn test_database_object_kind_serde_roundtrips() {
    let test_cases = [
        (DatabaseObjectKind::Table, "\"table\""),
        (DatabaseObjectKind::PartitionedTable, "\"partitionedTable\""),
        (DatabaseObjectKind::View, "\"view\""),
        (DatabaseObjectKind::MaterializedView, "\"materializedView\""),
        (DatabaseObjectKind::Function, "\"function\""),
        (DatabaseObjectKind::Procedure, "\"procedure\""),
        (DatabaseObjectKind::Sequence, "\"sequence\""),
        (DatabaseObjectKind::Trigger, "\"trigger\""),
        (DatabaseObjectKind::Index, "\"index\""),
        (DatabaseObjectKind::Stage, "\"stage\""),
        (DatabaseObjectKind::Task, "\"task\""),
        (DatabaseObjectKind::Stream, "\"stream\""),
        (DatabaseObjectKind::Pipe, "\"pipe\""),
        (DatabaseObjectKind::FileFormat, "\"fileFormat\""),
    ];

    for (kind, expected_json) in test_cases {
        // Serialization test
        let serialized = serde_json::to_string(&kind).expect("must serialize");
        assert_eq!(
            serialized, expected_json,
            "Serialization mismatch for {:?}",
            kind
        );

        // Deserialization test
        let deserialized: DatabaseObjectKind =
            serde_json::from_str(expected_json).expect("must deserialize");
        assert_eq!(
            deserialized, kind,
            "Deserialization mismatch for {}",
            expected_json
        );
    }
}

#[test]
fn test_database_object_kind_rejects_invalid_serde_variants() {
    let invalid_inputs = [
        "\"STAGE\"",
        "\"Task\"",
        "\"STREAM\"",
        "\"pipe_object\"",
        "\"file_format\"",
        "\"FileFormat\"",
        "\"unknown\"",
        "\"\"",
        "123",
        "{}",
    ];

    for invalid in invalid_inputs {
        let result = serde_json::from_str::<DatabaseObjectKind>(invalid);
        assert!(
            result.is_err(),
            "Expected deserialization failure for invalid input: {invalid}"
        );
    }
}

#[test]
fn test_database_object_ref_and_summary_serde_roundtrip() {
    let summary = DatabaseObjectSummary {
        object: DatabaseObjectRef {
            schema: "ANALYTICS".into(),
            name: "DAILY_LOAD_TASK".into(),
            kind: DatabaseObjectKind::Task,
            identity: Some("()".into()),
        },
        owner_table: None,
        signature: Some("USING CRON 0 0 * * * UTC".into()),
        estimated_rows: None,
        total_bytes: None,
    };

    let serialized = serde_json::to_value(&summary).expect("summary must serialize");
    assert_eq!(serialized["object"]["kind"], "task");
    assert_eq!(serialized["object"]["name"], "DAILY_LOAD_TASK");
    assert_eq!(serialized["object"]["schema"], "ANALYTICS");
    assert_eq!(serialized["object"]["identity"], "()");
    assert_eq!(serialized["signature"], "USING CRON 0 0 * * * UTC");

    let deserialized: DatabaseObjectSummary =
        serde_json::from_value(serialized).expect("summary must deserialize");
    assert_eq!(deserialized.object.kind, DatabaseObjectKind::Task);
    assert_eq!(deserialized.object.name, "DAILY_LOAD_TASK");
    assert_eq!(deserialized.object.identity.as_deref(), Some("()"));
    assert_eq!(
        deserialized.signature.as_deref(),
        Some("USING CRON 0 0 * * * UTC")
    );
}

#[test]
fn test_schema_diff_kind_rank_ordering_stability() {
    // Stage, Task, Stream, Pipe, FileFormat must have higher rank than base relational objects
    let table = DatabaseObjectKind::Table;
    let view = DatabaseObjectKind::View;
    let proc = DatabaseObjectKind::Procedure;
    let idx = DatabaseObjectKind::Index;
    let stage = DatabaseObjectKind::Stage;
    let task = DatabaseObjectKind::Task;
    let stream = DatabaseObjectKind::Stream;
    let pipe = DatabaseObjectKind::Pipe;
    let file_format = DatabaseObjectKind::FileFormat;

    // Verify ordering
    assert!(table < view);
    assert!(view < proc);
    assert!(proc < idx);
    assert!(idx < stage);
    assert!(stage < task);
    assert!(task < stream);
    assert!(stream < pipe);
    assert!(pipe < file_format);
}

// =========================================================================
// Empirical Challenge Tests: Multi-Engine Safety, LRU Eviction & Cancellation
// =========================================================================

use async_trait::async_trait;
use joinery_core::drivers::*;
use joinery_core::execution::Studio;
use joinery_core::file_capability::SandboxFileDialog;
use joinery_core::persistence::Persistence;
use joinery_core::secrets::MemorySecrets;
use std::sync::Arc;
use tempfile::tempdir;

struct MockNonSnowflakeDriver;

#[async_trait]
impl Driver for MockNonSnowflakeDriver {
    async fn execute(
        &self,
        _sql: &str,
        _kind: StatementKind,
        _sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        Ok(0)
    }
    async fn cancel(&self) -> AppResult<()> {
        Ok(())
    }
    async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
        Ok(Vec::new())
    }
    async fn schema(&self, _table: &TableRef) -> AppResult<TableSchema> {
        Err(AppError::new(ErrorCode::NotFound, "not found"))
    }
    async fn browse(
        &self,
        _schema: &TableSchema,
        _after: Option<Vec<Option<String>>>,
    ) -> AppResult<BrowseRows> {
        Ok(BrowseRows {
            rows: Vec::new(),
            next_key: None,
            has_more: false,
            truncated: false,
        })
    }
    async fn apply(&self, _schema: &TableSchema, _rows: &[RowMutation]) -> AppResult<()> {
        Ok(())
    }
    async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
        Ok(Vec::new())
    }
    async fn cancel_process(&self, _id: &str, _identity: &str) -> AppResult<()> {
        Ok(())
    }
    async fn health(&self) -> AppResult<u32> {
        Ok(0)
    }
    fn is_closed(&self) -> bool {
        false
    }
    fn close(&self) {}
}

#[tokio::test]
async fn test_multi_engine_driver_safety_rejects_extended_database_object_kinds() {
    let driver = MockNonSnowflakeDriver;
    let extended_kinds = [
        DatabaseObjectKind::Stage,
        DatabaseObjectKind::Task,
        DatabaseObjectKind::Stream,
        DatabaseObjectKind::Pipe,
        DatabaseObjectKind::FileFormat,
    ];

    for kind in extended_kinds {
        let obj = DatabaseObjectRef {
            schema: "PUBLIC".into(),
            name: "EXT_OBJ".into(),
            kind,
            identity: None,
        };
        let err = driver.object_details(&obj).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
        assert!(
            err.message
                .contains("Database object details are unavailable for this engine")
        );
    }

    let cat_err = driver.database_objects().await.unwrap_err();
    assert_eq!(cat_err.code, ErrorCode::Unsupported);
    assert!(
        cat_err
            .message
            .contains("Extended database object discovery is unavailable for this engine")
    );
}

#[test]
fn test_sqlite_parameter_persistence_lru_eviction_and_profile_isolation() {
    let dir = tempdir().unwrap();
    let persistence = Persistence::open(dir.path()).unwrap();

    let profile_a = "profile-uuid-a";
    let profile_b = "profile-uuid-b";

    // 1a. Insert 50 oldest items into Profile A
    let batch_old: Vec<QueryParamInput> = (0..50)
        .map(|i| QueryParamInput {
            key: format!("old_param_{i:04}"),
            kind: QueryParamKind::Text,
            value: Some(format!("old_value_{i}")),
        })
        .collect();
    persistence
        .save_parameter_history(profile_a, &batch_old)
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(15));

    // 1b. Insert 950 intermediate items into Profile A (reaching exactly 1000)
    let batch_mid: Vec<QueryParamInput> = (50..1000)
        .map(|i| QueryParamInput {
            key: format!("param_{i:04}"),
            kind: QueryParamKind::Text,
            value: Some(format!("value_{i}")),
        })
        .collect();
    persistence
        .save_parameter_history(profile_a, &batch_mid)
        .unwrap();

    // Verify raw SQLite count is exactly 1000
    let conn = rusqlite::Connection::open(dir.path().join("joinery.sqlite")).unwrap();
    let count_a: i64 = conn
        .query_row(
            "SELECT count(*) FROM parameters WHERE profile_id = ?1",
            [profile_a],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count_a, 1000);

    // 2. Insert 50 additional items into Profile A with newest timestamp
    std::thread::sleep(std::time::Duration::from_millis(15));
    let batch_new: Vec<QueryParamInput> = (1000..1050)
        .map(|i| QueryParamInput {
            key: format!("param_{i:04}"),
            kind: QueryParamKind::Integer,
            value: Some(format!("{i}")),
        })
        .collect();
    persistence
        .save_parameter_history(profile_a, &batch_new)
        .unwrap();

    // Verify total count is still capped at 1000 (LRU eviction of oldest 50)
    let count_a_after: i64 = conn
        .query_row(
            "SELECT count(*) FROM parameters WHERE profile_id = ?1",
            [profile_a],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count_a_after, 1000,
        "LRU pruning must cap parameters at 1000"
    );

    // The oldest 50 items (old_param_0000..old_param_0049) must be evicted
    for i in 0..50 {
        let exists: i64 = conn
            .query_row(
                "SELECT count(*) FROM parameters WHERE profile_id = ?1 AND param_key = ?2",
                rusqlite::params![profile_a, format!("old_param_{i:04}")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            exists, 0,
            "Oldest item old_param_{i:04} should have been evicted"
        );
    }

    // The newest items (param_1000..param_1049) must be retained
    for i in 1000..1050 {
        let exists: i64 = conn
            .query_row(
                "SELECT count(*) FROM parameters WHERE profile_id = ?1 AND param_key = ?2",
                rusqlite::params![profile_a, format!("param_{i:04}")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "New item param_{i:04} must be retained");
    }

    // 3. Profile B isolation: Insert 100 items into Profile B
    let batch_b: Vec<QueryParamInput> = (0..100)
        .map(|i| QueryParamInput {
            key: format!("b_param_{i}"),
            kind: QueryParamKind::Text,
            value: Some(format!("b_val_{i}")),
        })
        .collect();
    persistence
        .save_parameter_history(profile_b, &batch_b)
        .unwrap();

    let count_b: i64 = conn
        .query_row(
            "SELECT count(*) FROM parameters WHERE profile_id = ?1",
            [profile_b],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count_b, 100);

    // Profile A count must remain unaffected
    let count_a_check: i64 = conn
        .query_row(
            "SELECT count(*) FROM parameters WHERE profile_id = ?1",
            [profile_a],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count_a_check, 1000);

    // 4. Upsert semantics: re-saving existing key updates value & refreshes LRU ranking
    std::thread::sleep(std::time::Duration::from_millis(15));
    persistence
        .save_parameter_history(
            profile_a,
            &[QueryParamInput {
                key: "param_0050".into(),
                kind: QueryParamKind::Text,
                value: Some("updated_value_50".into()),
            }],
        )
        .unwrap();

    let history = persistence.parameter_history(profile_a).unwrap();
    // parameter_history returns up to 500 items ordered by used_at DESC
    assert_eq!(history.len(), 500);
    assert_eq!(history[0].param_key, "param_0050");
    assert_eq!(history[0].value.as_deref(), Some("updated_value_50"));

    // 5. Purge parameters: purging Profile B cleans only Profile B
    persistence.purge_parameters(profile_b).unwrap();
    let count_b_purged: i64 = conn
        .query_row(
            "SELECT count(*) FROM parameters WHERE profile_id = ?1",
            [profile_b],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count_b_purged, 0);

    let count_a_still_intact: i64 = conn
        .query_row(
            "SELECT count(*) FROM parameters WHERE profile_id = ?1",
            [profile_a],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count_a_still_intact, 1000);
}

#[tokio::test]
async fn test_scoped_query_cancellation_by_query_id() {
    let dir = tempdir().unwrap();
    let sandbox = SandboxFileDialog::new(&dir.path().join("sandbox-files")).unwrap();
    let studio =
        Studio::with_secrets_and_dialog(dir.path(), Arc::new(MemorySecrets::default()), sandbox)
            .unwrap();

    // 1. Non-existent query ID: cancellation is a safe no-op returning Ok(())
    let res = studio
        .cancel_by_query_id("non-existent-snowflake-query-id-12345")
        .await;
    assert!(
        res.is_ok(),
        "Cancelling non-existent query ID must succeed safely as a no-op"
    );

    // 2. Empty string query ID: also safe no-op
    let res_empty = studio.cancel_by_query_id("").await;
    assert!(res_empty.is_ok());
}
