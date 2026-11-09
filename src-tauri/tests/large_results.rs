use arrow_array::{Array, StringArray};
use joinery_core::{
    domain::*, execution::Studio, file_capability::SandboxFileDialog, secrets::MemorySecrets,
};
use sha2::{Digest, Sha256};
use std::{io::BufRead, sync::Arc, time::Duration};
use tokio::sync::mpsc;

const ROWS: usize = 1_000_000;

fn expected_row(index: usize) -> TextRow {
    let n = index + 1;
    vec![
        Some(n.to_string()),
        Some("9223372036854775807".into()),
        Some("12345678901234567890.1234567890".into()),
        match n % 3 {
            0 => None,
            1 => Some(String::new()),
            _ => Some("héllo;世界".into()),
        },
        Some("x".repeat(96)),
    ]
}

async fn query(studio: &Arc<Studio>, session: &SessionInfo, sql: &str) -> Vec<QueryEvent> {
    let plan = studio.prepare_execution(&session.id, sql).unwrap();
    let approval = plan.requires_approval.then(|| {
        studio
            .authorize(&plan.id, None, plan.confirmation_text.as_deref())
            .unwrap()
    });
    let (tx, mut rx) = mpsc::unbounded_channel();
    studio
        .execute(
            &plan.id,
            approval.as_deref(),
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(120), async {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            let done = matches!(event, QueryEvent::Finished { .. });
            events.push(event);
            if done {
                return events;
            }
        }
        panic!("query event stream ended without terminal status");
    })
    .await
    .expect("million-row query deadline")
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL fixture; reads only synthetic generate_series data"]
async fn million_rows_retain_page_and_export_exact_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let dialog = SandboxFileDialog::new(&root.path().join("exports")).unwrap();
    let studio = Studio::with_secrets_and_dialog(
        root.path(),
        Arc::new(MemorySecrets::default()),
        dialog.clone(),
    )
    .unwrap();
    assert!(!studio.bootstrap().unwrap().temporary_result_storage);
    studio.set_temporary_result_storage(true).unwrap();
    let profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "Million row fixture".into(),
                group_id: "local".into(),
                engine: EngineKind::Postgres,
                host: "127.0.0.1".into(),
                port: std::env::var("JOINERY_TEST_PORT")
                    .unwrap_or_else(|_| "55432".into())
                    .parse()
                    .unwrap(),
                database: "joinery".into(),
                username: "joinery".into(),
                environment: Environment::Local,
                safe_mode: SafeMode::Silent,
                tls_mode: TlsMode::Disable,
                ca_cert_path: None,
                ssh: None,
                remember_password: false,
                store_query_text: false,
                store_drafts: false,
                snowflake: None,
            },
            None,
        )
        .unwrap();
    let session = studio
        .connect(
            &profile.id,
            ConnectionSecrets {
                password: Some("joinery-local-only".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let events = query(
        &studio,
        &session,
        "SELECT n AS id, 9223372036854775807::bigint AS big, \
         12345678901234567890.1234567890::numeric(38,10) AS decimal, \
         CASE n % 3 WHEN 0 THEN NULL WHEN 1 THEN '' ELSE 'héllo;世界' END AS note, \
         repeat('x',96) AS payload FROM generate_series(1,1000000) n",
    )
    .await;
    assert!(
        matches!(
            events.last(),
            Some(QueryEvent::Finished {
                status: RunStatus::Completed,
                ..
            })
        ),
        "{events:?}"
    );
    let result = events
        .iter()
        .rev()
        .find_map(|event| match event {
            QueryEvent::Result { result } => Some(result.clone()),
            _ => None,
        })
        .expect("retained result");
    assert_eq!(result.retained_rows, ROWS, "large result was truncated");
    assert_eq!(result.status, RetentionStatus::Complete);
    assert!(studio.results.usage().0 <= 64 * 1024 * 1024);
    assert!(studio.results.disk_usage() > 64 * 1024 * 1024);
    for index in [0, 510, 511, 512, 99_999, 500_001, ROWS - 1] {
        assert_eq!(
            studio.results.row(&result.id, index).unwrap(),
            expected_row(index)
        );
        let bytes = studio.results.range(&result.id, index, 3).unwrap();
        let reader =
            arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes), None).unwrap();
        let mut offset = index;
        for batch in reader {
            let batch = batch.unwrap();
            for row_index in 0..batch.num_rows() {
                let row: TextRow = batch
                    .columns()
                    .iter()
                    .map(|column| {
                        let values = column.as_any().downcast_ref::<StringArray>().unwrap();
                        (!values.is_null(row_index)).then(|| values.value(row_index).to_owned())
                    })
                    .collect();
                assert_eq!(row, expected_row(offset));
                offset += 1;
            }
        }
        assert_eq!(offset, (index + 3).min(ROWS));
    }
    let grant = studio
        .pick_export_destination("million.csv", ExportFormat::Csv)
        .await
        .unwrap()
        .unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    studio
        .start_export(
            ExportRequest {
                destination_token: grant.token,
                format: ExportFormat::Csv,
                source: ExportSource::Result {
                    result_id: result.id.clone(),
                    column_ids: result
                        .columns
                        .iter()
                        .map(|column| column.id.clone())
                        .collect(),
                },
                include_headers: true,
                engine: Some(EngineKind::Postgres),
                target_table: None,
            },
            Arc::new(move |event| tx.send(event).is_ok()),
        )
        .await
        .unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(120), async {
        while let Some(event) = rx.recv().await {
            if matches!(event, ExportEvent::Finished { .. }) {
                return event;
            }
        }
        panic!("export ended without terminal status");
    })
    .await
    .expect("million-row export deadline");
    assert!(
        matches!(
            terminal,
            ExportEvent::Finished {
                status: RunStatus::Completed,
                rows_written: 1_000_000,
                ..
            }
        ),
        "{terminal:?}"
    );
    let mut lines =
        std::io::BufReader::new(std::fs::File::open(dialog.root().join("million.csv")).unwrap())
            .lines();
    assert_eq!(
        lines.next().unwrap().unwrap(),
        "id,big,decimal,note,payload"
    );
    let mut actual_hash = Sha256::new();
    let mut expected_hash = Sha256::new();
    for index in 0..ROWS {
        let actual = lines.next().expect("export row missing").unwrap();
        let n = index + 1;
        let note = match n % 3 {
            0 => "",
            1 => "\"\"",
            _ => "héllo;世界",
        };
        let expected = format!(
            "{n},9223372036854775807,12345678901234567890.1234567890,{note},{}",
            "x".repeat(96)
        );
        actual_hash.update(actual.as_bytes());
        actual_hash.update(b"\n");
        expected_hash.update(expected.as_bytes());
        expected_hash.update(b"\n");
    }
    assert!(
        lines.next().is_none(),
        "export contains unexpected extra rows"
    );
    assert_eq!(actual_hash.finalize(), expected_hash.finalize());
    assert!(!dialog.root().join("million.csv.partial").exists());
    studio.release_result(&result.id);
    assert_eq!(studio.results.usage(), (0, 0));
    assert_eq!(studio.results.disk_usage(), 0);
    studio.shutdown();
}

#[test]
fn result_storage_opt_in_persists_without_restoring_results() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    assert!(!studio.bootstrap().unwrap().temporary_result_storage);
    assert!(studio.set_temporary_result_storage(true).unwrap());
    assert!(studio.bootstrap().unwrap().temporary_result_storage);
    drop(studio);
    let reopened = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    assert!(reopened.bootstrap().unwrap().temporary_result_storage);
    assert_eq!(reopened.results.usage(), (0, 0));
    assert_eq!(reopened.results.disk_usage(), 0);
    assert!(!reopened.set_temporary_result_storage(false).unwrap());
    drop(reopened);
    let reopened = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    assert!(!reopened.bootstrap().unwrap().temporary_result_storage);
}

#[test]
fn metadata_restore_applies_restored_result_storage_preference() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let disabled_snapshot = studio.create_metadata_backup().unwrap();
    studio.set_temporary_result_storage(true).unwrap();
    let enabled_snapshot = studio.create_metadata_backup().unwrap();
    studio
        .restore_metadata_backup(
            &disabled_snapshot.id,
            &disabled_snapshot.digest,
            "RESTORE",
            None,
        )
        .unwrap();
    assert!(!studio.bootstrap().unwrap().temporary_result_storage);
    assert!(!studio.results.temporary_disk_enabled());
    studio
        .restore_metadata_backup(
            &enabled_snapshot.id,
            &enabled_snapshot.digest,
            "RESTORE",
            None,
        )
        .unwrap();
    assert!(studio.bootstrap().unwrap().temporary_result_storage);
    assert!(studio.results.temporary_disk_enabled());
}
