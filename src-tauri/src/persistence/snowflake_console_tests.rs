use super::*;
use crate::console_contract::*;

fn metadata(steps: u32) -> ConsoleRunMetadata {
    ConsoleRunMetadata {
        run_id: crate::domain::new_id(),
        client_request_id: crate::domain::new_id(),
        session_id: crate::domain::new_id(),
        connection_id: crate::domain::new_id(),
        plan_digest: "a".repeat(64),
        step_count: steps,
        created_at: 1000,
    }
}

fn acknowledgement() -> ConsoleStepOutcome {
    ConsoleStepOutcome::Acknowledged(ConsoleAcknowledgement {
        statement: ConsoleStatement {
            sdk_request_id: crate::domain::new_id(),
            query_id: Some(crate::domain::new_id()),
        },
        context: ConsoleContext {
            database: Some("quoted database".into()),
            schema: Some("quoted schema".into()),
            role: None,
            warehouse: Some("WAREHOUSE".into()),
        },
    })
}

fn setup() -> (tempfile::TempDir, Persistence, ConsoleJournalOwner) {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let owner = store.claim_console_journal_owner().unwrap();
    (root, store, owner)
}

fn prepared(store: &Persistence, owner: &ConsoleJournalOwner, count: u32) -> ConsoleRunSnapshot {
    let result = store.console_prepare_run(owner, metadata(count)).unwrap();
    assert!(result.created);
    result.run
}

fn intended(
    store: &Persistence,
    owner: &ConsoleJournalOwner,
    run: &ConsoleRunSnapshot,
) -> ConsoleRunSnapshot {
    store
        .console_step_intent(
            owner,
            &run.metadata.run_id,
            run.revision,
            run.steps.len() as u32,
            1001,
        )
        .unwrap()
}

fn acknowledged(
    store: &Persistence,
    owner: &ConsoleJournalOwner,
    run: &ConsoleRunSnapshot,
) -> ConsoleRunSnapshot {
    store
        .console_step_outcome(
            owner,
            &run.metadata.run_id,
            run.revision,
            run.steps.len() as u32 - 1,
            acknowledgement(),
            ConsoleObservations::default(),
        )
        .unwrap()
}

fn pinned() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    connection.execute_batch(SCHEMA_SQL).unwrap();
    connection.execute_batch("ALTER TABLE snowflake_console_runs RENAME TO snowflake_console_runs_v3; ALTER TABLE snowflake_console_steps RENAME TO snowflake_console_steps_v3;").unwrap();
    connection.execute_batch(REJECTION_SCHEMA_SQL).unwrap();
    for (_, sql) in DECISION_TRIGGERS {
        connection.execute_batch(sql).unwrap();
    }
    connection.pragma_update(None, "user_version", 3).unwrap();
    migrate_v4(&connection).unwrap();
    connection
        .pragma_update(None, "user_version", super::super::SCHEMA_VERSION)
        .unwrap();
    connection
}

fn refusal() -> ConsoleAdmissionRejection {
    ConsoleAdmissionRejection {
        client_request_id: crate::domain::new_id(),
        session_id: crate::domain::new_id(),
        plan_digest: "a".repeat(64),
        code: ErrorCode::ApprovalRequired,
        recorded_at: 1000,
    }
}

fn for_refusal(value: &ConsoleAdmissionRejection) -> ConsoleRunMetadata {
    ConsoleRunMetadata {
        client_request_id: value.client_request_id.clone(),
        session_id: value.session_id.clone(),
        plan_digest: value.plan_digest.clone(),
        ..metadata(1)
    }
}

fn downgrade_fixture_to_v2(connection: &Connection) {
    for (kind, name) in schema_objects(4)
        .unwrap()
        .into_iter()
        .filter(|(kind, _)| kind != "table")
    {
        connection
            .execute_batch(&format!("DROP {kind} {name}"))
            .unwrap();
    }
    connection.execute_batch("DROP TABLE snowflake_console_retention_v4; DROP TABLE snowflake_console_rejections_v4;").unwrap();
    for column in ["receipt_sequence", "archived_at", "browsing_revision"] {
        connection
            .execute_batch(&format!(
                "ALTER TABLE snowflake_console_runs_v4 DROP COLUMN {column}"
            ))
            .unwrap();
    }
    connection.execute_batch("ALTER TABLE snowflake_console_runs_v4 RENAME TO snowflake_console_runs; ALTER TABLE snowflake_console_steps_v4 RENAME TO snowflake_console_steps; PRAGMA user_version=2;").unwrap();
}

fn downgrade_fixture_to_v3(connection: &Connection) {
    for (kind, name) in schema_objects(4)
        .unwrap()
        .into_iter()
        .filter(|(kind, _)| kind != "table")
    {
        connection
            .execute_batch(&format!("DROP {kind} {name}"))
            .unwrap();
    }
    connection
        .execute_batch("DROP TABLE snowflake_console_retention_v4")
        .unwrap();
    for table in [
        "snowflake_console_runs_v4",
        "snowflake_console_rejections_v4",
    ] {
        for column in ["receipt_sequence", "archived_at", "browsing_revision"] {
            connection
                .execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column}"))
                .unwrap();
        }
    }
    connection.execute_batch("ALTER TABLE snowflake_console_runs_v4 RENAME TO snowflake_console_runs_v3; ALTER TABLE snowflake_console_steps_v4 RENAME TO snowflake_console_steps_v3; ALTER TABLE snowflake_console_rejections_v4 RENAME TO snowflake_console_rejections_v3; PRAGMA user_version=3;").unwrap();
    for (_, sql) in DECISION_TRIGGERS {
        connection.execute_batch(sql).unwrap();
    }
}

fn receipt_page(limit: u32) -> SnowflakeConsoleReceiptPageRequest {
    SnowflakeConsoleReceiptPageRequest {
        filter: SnowflakeConsoleReceiptFilter {
            visibility: SnowflakeConsoleReceiptVisibility::All,
            kind: None,
            connection_id: None,
        },
        cursor: None,
        limit,
    }
}

fn archive_item(summary: &SnowflakeConsoleReceiptSummary) -> SnowflakeConsoleReceiptArchiveItem {
    SnowflakeConsoleReceiptArchiveItem {
        client_request_id: summary.client_request_id.clone(),
        session_id: summary.session_id.clone(),
        plan_digest: summary.plan_digest.clone(),
        run_id: summary.run_id.clone(),
        kind: summary.kind,
        recorded_at: summary.recorded_at.clone(),
        original_revision: summary.original_revision.clone(),
        browsing_revision: summary.browsing_revision.clone(),
        refusal_code: summary.refusal_code,
    }
}

#[test]
fn console_retention_v4_migration_fences_cached_v3_sql_without_reclassifying_originals() {
    let (root, store, owner) = setup();
    let run = prepared(&store, &owner, 2);
    let run = intended(&store, &owner, &run);
    let run = acknowledged(&store, &owner, &run);
    let run = intended(&store, &owner, &run);
    let negative = refusal();
    store.console_record_rejection(&owner, &negative).unwrap();
    let canonical = store.root.clone();
    drop(owner);
    drop(store);
    let old = Connection::open(root.path().join("joinery.sqlite")).unwrap();
    downgrade_fixture_to_v3(&old);
    let mut insert=old.prepare("INSERT INTO snowflake_console_runs_v3(run_id,client_request_id,session_id,connection_id,plan_digest,step_count,created_at,revision,state,observations_json) VALUES(?1,?2,?3,?4,?5,1,1000,0,'prepared',?6)").unwrap();
    let mut outcome = old
        .prepare(
            "UPDATE snowflake_console_steps_v3 SET outcome_json=?1 WHERE run_id=?2 AND ordinal=1",
        )
        .unwrap();
    let mut recovery = old
        .prepare("SELECT run_id FROM snowflake_console_runs_v3 WHERE state!='terminal'")
        .unwrap();
    let lease = claim_migration_lease(&canonical).unwrap();
    assert_eq!(
        Persistence::open(root.path()).err().unwrap().code,
        ErrorCode::Conflict
    );
    assert_eq!(super::super::schema_version(&old).unwrap(), 3);
    assert_eq!(
        read_run(&old, &run.metadata.run_id).unwrap(),
        Some(run.clone())
    );
    drop(lease);
    let upgraded = Persistence::open(root.path()).unwrap();
    assert_eq!(
        upgraded.console_run(&run.metadata.run_id).unwrap(),
        Some(run.clone())
    );
    assert_eq!(
        upgraded
            .console_rejection_by_client_request(&negative.client_request_id)
            .unwrap(),
        Some(negative.clone())
    );
    let other = metadata(1);
    assert!(
        insert
            .execute(params![
                other.run_id,
                other.client_request_id,
                other.session_id,
                other.connection_id,
                other.plan_digest,
                encode(&ConsoleObservations::default()).unwrap()
            ])
            .unwrap_err()
            .to_string()
            .contains("no such table")
    );
    assert!(
        outcome
            .execute(params![
                encode(&acknowledgement()).unwrap(),
                run.metadata.run_id
            ])
            .unwrap_err()
            .to_string()
            .contains("no such table")
    );
    assert!(
        recovery
            .query([])
            .unwrap()
            .next()
            .unwrap_err()
            .to_string()
            .contains("no such table")
    );
    let page = upgraded.console_receipts(&receipt_page(100)).unwrap();
    assert_eq!(page.receipts.len(), 2);
    let mut expected = vec![run.metadata.client_request_id, negative.client_request_id];
    expected.sort();
    expected.reverse();
    assert_eq!(
        page.receipts
            .iter()
            .map(|r| r.client_request_id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    let db = upgraded.connection.lock();
    validate_schema(&db).unwrap();
    validate_records(&db).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('snowflake_console_runs_v3','snowflake_console_steps_v3','snowflake_console_rejections_v3','snowflake_console_runs','snowflake_console_steps')",[],|r|r.get::<_,u32>(0)).unwrap(),0);
}

#[test]
fn console_retention_pages_are_bounded_filtered_and_snapshot_high_water_is_stable() {
    let (_root, store, owner) = setup();
    let positive = prepared(&store, &owner, 1);
    for _ in 0..105 {
        store.console_record_rejection(&owner, &refusal()).unwrap();
    }
    let mut request = receipt_page(10);
    let first = store.console_receipts(&request).unwrap();
    assert_eq!(first.receipts.len(), 10);
    let newest = refusal();
    store.console_record_rejection(&owner, &newest).unwrap();
    let mut all = first.receipts.clone();
    request.cursor = first.next_cursor;
    while request.cursor.is_some() {
        let page = store.console_receipts(&request).unwrap();
        assert!(page.receipts.len() <= 10);
        all.extend(page.receipts);
        request.cursor = page.next_cursor;
    }
    assert_eq!(all.len(), 106);
    assert_eq!(
        all.iter()
            .map(|s| &s.client_request_id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        106
    );
    assert!(
        !all.iter()
            .any(|s| s.client_request_id == newest.client_request_id)
    );
    assert!(
        all.windows(2)
            .all(|p| p[0].sequence.parse::<u64>().unwrap() > p[1].sequence.parse::<u64>().unwrap())
    );
    request = receipt_page(100);
    request.filter.connection_id = Some(positive.metadata.connection_id.clone());
    let page = store.console_receipts(&request).unwrap();
    assert_eq!(page.receipts.len(), 1);
    assert_eq!(
        page.receipts[0].client_request_id,
        positive.metadata.client_request_id
    );
    request.filter.kind = Some(SnowflakeConsoleReceiptKind::Refusal);
    assert!(
        store
            .console_receipts(&request)
            .unwrap()
            .receipts
            .is_empty()
    );
    for limit in [0, 101, u32::MAX] {
        assert!(store.console_receipts(&receipt_page(limit)).is_err());
    }
    let mut changed = receipt_page(10);
    changed.cursor = store
        .console_receipts(&receipt_page(10))
        .unwrap()
        .next_cursor;
    changed.filter.visibility = SnowflakeConsoleReceiptVisibility::Archived;
    assert_eq!(
        store.console_receipts(&changed).unwrap_err().code,
        ErrorCode::Conflict
    );
    let storage = store.console_storage().unwrap();
    assert_eq!(storage.positive_count, "1");
    assert_eq!(storage.refusal_count, "106");
    assert!(storage.charged_bytes.unwrap().parse::<u64>().unwrap() > 0);
    assert!(storage.reserved_bytes.is_none());
    assert!(!storage.retention_enabled);
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn console_retention_archive_is_atomic_browsing_only_and_rejects_stale_authority() {
    let (_root, store, owner) = setup();
    let initial = prepared(&store, &owner, 1);
    let pending = intended(&store, &owner, &initial);
    let ack = acknowledged(&store, &owner, &pending);
    let original = store
        .console_finish(
            &owner,
            &ack.metadata.run_id,
            ack.revision,
            ConsoleStopReason::Completed,
            2000,
        )
        .unwrap();
    let negative = refusal();
    store.console_record_rejection(&owner, &negative).unwrap();
    let a = store
        .console_receipt_summary(&original.metadata.client_request_id)
        .unwrap()
        .unwrap();
    let b = store
        .console_receipt_summary(&negative.client_request_id)
        .unwrap()
        .unwrap();
    let cursor = store
        .console_receipts(&receipt_page(1))
        .unwrap()
        .next_cursor;
    let mut batch = SnowflakeConsoleReceiptArchiveRequest {
        inventory_epoch: a.inventory_epoch.clone(),
        items: vec![archive_item(&a), archive_item(&b)],
        archived: true,
    };
    batch.items[1].refusal_code = Some(ErrorCode::ApprovalExpired);
    assert_eq!(
        store
            .console_archive_receipts(&owner, &batch, 3000)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        store
            .console_receipt_summary(&a.client_request_id)
            .unwrap()
            .unwrap(),
        a
    );
    batch.items[1] = archive_item(&b);
    let result = store
        .console_archive_receipts(&owner, &batch, 3000)
        .unwrap();
    assert!(
        result
            .receipts
            .iter()
            .all(|s| s.archived && s.browsing_revision == "1")
    );
    assert_eq!(
        store.console_run(&original.metadata.run_id).unwrap(),
        Some(original.clone())
    );
    assert_eq!(
        store
            .console_rejection_by_client_request(&negative.client_request_id)
            .unwrap(),
        Some(negative)
    );
    assert!(
        store
            .console_archive_receipts(&owner, &batch, 3000)
            .is_err()
    );
    let mut old_page = receipt_page(1);
    old_page.cursor = cursor;
    assert_eq!(
        store.console_receipts(&old_page).unwrap_err().code,
        ErrorCode::Conflict
    );
    let mut visible = receipt_page(100);
    visible.filter.visibility = SnowflakeConsoleReceiptVisibility::Visible;
    assert!(
        store
            .console_receipts(&visible)
            .unwrap()
            .receipts
            .is_empty()
    );
    let mut unarchive = SnowflakeConsoleReceiptArchiveRequest {
        inventory_epoch: a.inventory_epoch,
        items: result.receipts.iter().map(archive_item).collect(),
        archived: false,
    };
    unarchive.inventory_epoch = crate::domain::new_id();
    assert!(
        store
            .console_archive_receipts(&owner, &unarchive, 4000)
            .is_err()
    );
    unarchive.inventory_epoch = result.receipts[0].inventory_epoch.clone();
    assert!(
        store
            .console_archive_receipts(&owner, &unarchive, 4000)
            .unwrap()
            .receipts
            .iter()
            .all(|s| !s.archived)
    );
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn console_retention_archive_lease_never_recovers_unrelated_incomplete_originals() {
    let (_root, store, owner) = setup();
    let pending = intended(&store, &owner, &prepared(&store, &owner, 2));
    let ack = acknowledged(
        &store,
        &owner,
        &intended(&store, &owner, &prepared(&store, &owner, 1)),
    );
    let settled = store
        .console_finish(
            &owner,
            &ack.metadata.run_id,
            ack.revision,
            ConsoleStopReason::Completed,
            2000,
        )
        .unwrap();
    let summary = store
        .console_receipt_summary(&settled.metadata.client_request_id)
        .unwrap()
        .unwrap();
    drop(owner);
    let archive_owner = store.claim_console_archive_owner().unwrap();
    assert_eq!(
        store.console_run(&pending.metadata.run_id).unwrap(),
        Some(pending.clone())
    );
    assert!(
        store.claim_console_journal_owner().is_err(),
        "the raw lease remains exclusive"
    );
    let mut request = SnowflakeConsoleReceiptArchiveRequest {
        inventory_epoch: summary.inventory_epoch.clone(),
        items: vec![archive_item(&summary)],
        archived: true,
    };
    request.items[0].original_revision = "999".into();
    assert!(
        store
            .console_archive_receipts(&archive_owner, &request, 3000)
            .is_err()
    );
    assert_eq!(
        store.console_run(&pending.metadata.run_id).unwrap(),
        Some(pending.clone())
    );
    request.items[0] = archive_item(&summary);
    store
        .console_archive_receipts(&archive_owner, &request, 3000)
        .unwrap();
    assert_eq!(
        store.console_run(&pending.metadata.run_id).unwrap(),
        Some(pending)
    );
    assert_eq!(
        store.console_run(&settled.metadata.run_id).unwrap(),
        Some(settled)
    );
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn console_retention_validation_rejects_empty_ids_in_every_journal_schema() {
    for version in [2, 3, 4] {
        for refusal_record in [false, true] {
            if version == 2 && refusal_record {
                continue;
            }
            let (_root, store, owner) = setup();
            if refusal_record {
                store.console_record_rejection(&owner, &refusal()).unwrap();
            } else {
                prepared(&store, &owner, 1);
            }
            let connection = store.connection.lock();
            match version {
                2 => downgrade_fixture_to_v2(&connection),
                3 => downgrade_fixture_to_v3(&connection),
                _ => {}
            }
            validate_schema(&connection).unwrap();
            validate_records(&connection).unwrap();
            let (table, column) = match (version, refusal_record) {
                (2, false) => ("snowflake_console_runs", "run_id"),
                (3, false) => ("snowflake_console_runs_v3", "run_id"),
                (3, true) => ("snowflake_console_rejections_v3", "client_request_id"),
                (4, false) => ("snowflake_console_runs_v4", "run_id"),
                (4, true) => ("snowflake_console_rejections_v4", "client_request_id"),
                _ => unreachable!(),
            };
            connection
                .execute(&format!("UPDATE {table} SET {column}=''"), [])
                .unwrap();
            // The v4 update triggers keep counts/charges consistent, isolating
            // ID validation rather than relying on an unrelated totals error.
            validate_schema(&connection).unwrap();
            assert!(
                validate_records(&connection).is_err(),
                "schema {version}, refusal={refusal_record}"
            );
        }
    }
}

#[test]
fn console_retention_pending_cannot_archive_and_unknown_terminal_keeps_original_uncertainty() {
    let (_root, store, owner) = setup();
    let initial = prepared(&store, &owner, 2);
    let run = intended(&store, &owner, &initial);
    let before = store
        .console_receipt_summary(&run.metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert!(!before.archive_eligible);
    assert!(before.unknown);
    let batch = SnowflakeConsoleReceiptArchiveRequest {
        inventory_epoch: before.inventory_epoch.clone(),
        items: vec![archive_item(&before)],
        archived: true,
    };
    assert!(
        store
            .console_archive_receipts(&owner, &batch, 3000)
            .is_err()
    );
    store.console_interrupt_incomplete_runs(&owner).unwrap();
    let original = store.console_run(&run.metadata.run_id).unwrap().unwrap();
    let summary = store
        .console_receipt_summary(&run.metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert!(summary.archive_eligible && summary.unknown);
    let batch = SnowflakeConsoleReceiptArchiveRequest {
        inventory_epoch: summary.inventory_epoch.clone(),
        items: vec![archive_item(&summary)],
        archived: true,
    };
    assert!(
        store
            .console_archive_receipts(&owner, &batch, crate::domain::now_ms())
            .unwrap()
            .receipts[0]
            .unknown
    );
    assert_eq!(
        store.console_run(&run.metadata.run_id).unwrap(),
        Some(original)
    );
}

#[test]
fn console_retention_restore_union_refreshes_epoch_and_preserves_live_archive_and_originals() {
    let (_root, store, owner) = setup();
    let run = prepared(&store, &owner, 1);
    let run = intended(&store, &owner, &run);
    let run = acknowledged(&store, &owner, &run);
    let original = store
        .console_finish(
            &owner,
            &run.metadata.run_id,
            run.revision,
            ConsoleStopReason::Completed,
            2000,
        )
        .unwrap();
    let summary = store
        .console_receipt_summary(&run.metadata.client_request_id)
        .unwrap()
        .unwrap();
    let request = SnowflakeConsoleReceiptArchiveRequest {
        inventory_epoch: summary.inventory_epoch.clone(),
        items: vec![archive_item(&summary)],
        archived: true,
    };
    store
        .console_archive_receipts(&owner, &request, 3000)
        .unwrap();
    let mut backup = pinned();
    let backup_only = refusal();
    insert_rejection(&backup, &backup_only).unwrap();
    let old_epoch: String = backup
        .query_row(
            "SELECT inventory_epoch FROM snowflake_console_retention_v4",
            [],
            |r| r.get(0),
        )
        .unwrap();
    merge_for_restore(&store.connection.lock(), &mut backup).unwrap();
    assert_eq!(
        read_run(&backup, &run.metadata.run_id).unwrap(),
        Some(original)
    );
    assert_eq!(
        read_rejection(&backup, &backup_only.client_request_id).unwrap(),
        Some(backup_only)
    );
    let new_epoch: String = backup
        .query_row(
            "SELECT inventory_epoch FROM snowflake_console_retention_v4",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(new_epoch, old_epoch);
    assert_ne!(new_epoch, summary.inventory_epoch);
    assert_eq!(
        backup
            .query_row(
                "SELECT archived_at FROM snowflake_console_runs_v4",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        3000
    );
    validate_schema(&backup).unwrap();
    validate_records(&backup).unwrap();
}

#[test]
fn console_retention_corrupt_control_or_browsing_records_are_rejected() {
    for sql in [
        "UPDATE snowflake_console_retention_v4 SET positive_count=positive_count+1",
        "UPDATE snowflake_console_retention_v4 SET charged_bytes=charged_bytes+1",
        "UPDATE snowflake_console_retention_v4 SET inventory_epoch='invalid'",
        "UPDATE snowflake_console_retention_v4 SET next_sequence=1",
        "PRAGMA ignore_check_constraints=ON; UPDATE snowflake_console_runs_v4 SET browsing_revision=-1",
    ] {
        let (_root, store, owner) = setup();
        prepared(&store, &owner, 1);
        store.connection.lock().execute_batch(sql).unwrap();
        assert!(validate_records(&store.connection.lock()).is_err(), "{sql}");
    }
    let (_root, store, owner) = setup();
    let run = prepared(&store, &owner, 1);
    let negative = refusal();
    store.console_record_rejection(&owner, &negative).unwrap();
    assert!(
        store
            .connection
            .lock()
            .execute(
                "UPDATE snowflake_console_runs_v4 SET receipt_sequence=2 WHERE run_id=?1",
                [run.metadata.run_id]
            )
            .is_err()
    );
    store.connection.lock().execute_batch("DROP INDEX console_run_browse_v4; CREATE INDEX console_run_browse_v4 ON snowflake_console_runs_v4(client_request_id)").unwrap();
    assert!(validate_schema(&store.connection.lock()).is_err());
}

#[test]
fn console_retention_decimal_identity_preserves_i64_precision_and_rejects_sequence_exhaustion() {
    let (_root, store, owner) = setup();
    store
        .connection
        .lock()
        .execute(
            "UPDATE snowflake_console_retention_v4 SET next_sequence=?1",
            [9_007_199_254_740_993i64],
        )
        .unwrap();
    let negative = refusal();
    store.console_record_rejection(&owner, &negative).unwrap();
    store.connection.lock().execute("UPDATE snowflake_console_rejections_v4 SET browsing_revision=?1 WHERE client_request_id=?2",params![9_007_199_254_740_993i64,negative.client_request_id]).unwrap();
    let summary = store
        .console_receipt_summary(&negative.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(summary.sequence, "9007199254740993");
    assert_eq!(summary.browsing_revision, "9007199254740993");
    let request = SnowflakeConsoleReceiptArchiveRequest {
        inventory_epoch: summary.inventory_epoch.clone(),
        items: vec![archive_item(&summary)],
        archived: true,
    };
    assert_eq!(
        store
            .console_archive_receipts(&owner, &request, 3000)
            .unwrap()
            .receipts[0]
            .browsing_revision,
        "9007199254740994"
    );
    validate_records(&store.connection.lock()).unwrap();
    store
        .connection
        .lock()
        .execute(
            "UPDATE snowflake_console_retention_v4 SET next_sequence=?1",
            [i64::MAX],
        )
        .unwrap();
    assert!(store.console_record_rejection(&owner, &refusal()).is_err());
    assert_eq!(store.console_storage().unwrap().refusal_count, "1");
    validate_records(&store.connection.lock()).unwrap();
    for raw in ["x".repeat(2049), "{\"unexpected\":true}".into()] {
        let mut request = receipt_page(1);
        request.cursor = Some(raw);
        assert!(store.console_receipts(&request).is_err());
    }
}

#[test]
fn console_retention_failed_physical_migration_rolls_back_without_rewriting_originals() {
    let (root, store, owner) = setup();
    let original = prepared(&store, &owner, 1);
    drop(owner);
    drop(store);
    let old = Connection::open(root.path().join("joinery.sqlite")).unwrap();
    downgrade_fixture_to_v3(&old);
    old.execute_batch("CREATE TABLE snowflake_console_retention_v4(unexpected TEXT)")
        .unwrap();
    assert!(Persistence::open(root.path()).is_err());
    assert_eq!(super::super::schema_version(&old).unwrap(), 3);
    assert_eq!(
        read_run(&old, &original.metadata.run_id).unwrap(),
        Some(original)
    );
    assert_eq!(
        old.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='snowflake_console_runs_v4'",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        old.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='trigger' AND name LIKE 'console_%'",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        4
    );
}

#[test]
fn console_negative_reopen_dedup_is_immutable_and_independent_of_history_and_profile_retention() {
    let (root, store, owner) = setup();
    let value = refusal();
    assert_eq!(
        store.console_record_rejection(&owner, &value).unwrap(),
        value
    );
    assert_eq!(
        store.console_record_rejection(&owner, &value).unwrap(),
        value
    );
    assert!(
        store
            .console_run_by_client_request(&value.client_request_id)
            .unwrap()
            .is_none()
    );
    for field in 0..4 {
        let mut changed = value.clone();
        match field {
            0 => changed.session_id = crate::domain::new_id(),
            1 => changed.plan_digest = "b".repeat(64),
            2 => changed.code = ErrorCode::PolicyBlocked,
            _ => changed.recorded_at += 1,
        }
        assert_eq!(
            store
                .console_record_rejection(&owner, &changed)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
    }
    store.clear_history().unwrap();
    store.delete_profile(&crate::domain::new_id()).unwrap();
    store
        .set_setting(
            "changed-policy",
            "new authorization cannot revive a refused client",
        )
        .unwrap();
    drop(owner);
    drop(store);
    let reopened = Persistence::open(root.path()).unwrap();
    assert_eq!(
        reopened
            .console_rejection_by_client_request(&value.client_request_id)
            .unwrap(),
        Some(value.clone())
    );
    let owner = reopened.claim_console_journal_owner().unwrap();
    assert_eq!(
        reopened
            .console_prepare_run(&owner, for_refusal(&value))
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    assert!(
        reopened
            .console_rejection_by_client_request(&crate::domain::new_id())
            .unwrap()
            .is_none()
    );
    validate_records(&reopened.connection.lock()).unwrap();
}

#[test]
fn console_negative_identity_time_and_closed_payload_reject_without_writes() {
    let (_root, store, owner) = setup();
    for field in 0..7 {
        let mut value = refusal();
        match field {
            0 => value.client_request_id = "not-an-id".into(),
            1 => value.client_request_id = "11111111-AAAA-4111-8111-111111111111".into(),
            2 => value.session_id = "foreign-session".into(),
            3 => value.plan_digest = "A".repeat(64),
            4 => value.plan_digest = "a".repeat(65),
            5 => value.recorded_at = u64::MAX,
            _ => value.session_id = "x".repeat(100_000),
        }
        assert!(store.console_record_rejection(&owner, &value).is_err());
    }
    assert!(visit_ids(&store.connection.lock(), true, false, |_| Ok(())).unwrap() == 0);
    let mut raw = serde_json::to_value(refusal()).unwrap();
    raw["sql"] = serde_json::json!("must not be admitted into a receipt");
    assert!(serde_json::from_value::<ConsoleAdmissionRejection>(raw).is_err());
    let mut raw = serde_json::to_value(refusal()).unwrap();
    raw["code"] = serde_json::json!("futureOrArbitraryErrorText");
    assert!(serde_json::from_value::<ConsoleAdmissionRejection>(raw).is_err());
    let mut limit = refusal();
    limit.recorded_at = i64::MAX as u64;
    store.console_record_rejection(&owner, &limit).unwrap();
    assert_eq!(
        store
            .console_rejection_by_client_request(&limit.client_request_id)
            .unwrap(),
        Some(limit)
    );
}

#[test]
fn console_negative_foreign_owner_and_both_decision_orders_cannot_overwrite_evidence() {
    let (_root, store, owner) = setup();
    let (_other_root, other, other_owner) = setup();
    assert_eq!(
        store
            .console_record_rejection(&other_owner, &refusal())
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        other
            .console_record_rejection(&owner, &refusal())
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let negative = refusal();
    store.console_record_rejection(&owner, &negative).unwrap();
    assert_eq!(
        store
            .console_prepare_run(&owner, for_refusal(&negative))
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    let positive = prepared(&store, &owner, 1);
    let denied = ConsoleAdmissionRejection {
        client_request_id: positive.metadata.client_request_id.clone(),
        session_id: positive.metadata.session_id.clone(),
        plan_digest: positive.metadata.plan_digest.clone(),
        ..refusal()
    };
    assert_eq!(
        store
            .console_record_rejection(&owner, &denied)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        store.console_run(&positive.metadata.run_id).unwrap(),
        Some(positive)
    );
    assert_eq!(
        store
            .console_rejection_by_client_request(&negative.client_request_id)
            .unwrap(),
        Some(negative)
    );
}

#[test]
fn console_negative_sqlite_insert_commit_and_readback_failures_never_return_a_receipt() {
    let (_root, store, owner) = setup();
    let value = refusal();
    store.connection.lock().execute_batch("CREATE TRIGGER refuse_negative_insert BEFORE INSERT ON snowflake_console_rejections_v4 BEGIN SELECT RAISE(FAIL,'fixture insert failure'); END;").unwrap();
    assert!(store.console_record_rejection(&owner, &value).is_err());
    assert!(
        store
            .console_rejection_by_client_request(&value.client_request_id)
            .unwrap()
            .is_none()
    );
    store.connection.lock().execute_batch("DROP TRIGGER refuse_negative_insert;
        CREATE TABLE refusal_commit_fault(parent TEXT REFERENCES settings(key) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER refuse_negative_commit AFTER INSERT ON snowflake_console_rejections_v4 BEGIN INSERT INTO refusal_commit_fault VALUES('missing-commit-parent'); END;").unwrap();
    assert!(
        store.console_record_rejection(&owner, &value).is_err(),
        "deferred FK must fail at COMMIT"
    );
    assert!(
        store
            .console_rejection_by_client_request(&value.client_request_id)
            .unwrap()
            .is_none(),
        "failed commit must roll back the inserted refusal"
    );
    assert_eq!(
        store
            .connection
            .lock()
            .query_row("SELECT count(*) FROM refusal_commit_fault", [], |row| row
                .get::<_, u32>(
                0
            ))
            .unwrap(),
        0
    );
    store
        .connection
        .lock()
        .execute_batch("DROP TRIGGER refuse_negative_commit; DROP TABLE refusal_commit_fault;")
        .unwrap();
    store.console_record_rejection(&owner, &value).unwrap();
    store.connection.lock().execute("UPDATE snowflake_console_rejections_v4 SET code='unrecognized' WHERE client_request_id=?1", [&value.client_request_id]).unwrap();
    assert!(
        store
            .console_rejection_by_client_request(&value.client_request_id)
            .is_err(),
        "unreadable evidence is not an absent or valid refusal"
    );
    assert!(
        store.console_record_rejection(&owner, &value).is_err(),
        "retry cannot overwrite malformed evidence"
    );
}

#[test]
fn console_negative_database_guards_and_all_readers_reject_dual_decisions() {
    let (_root, store, owner) = setup();
    let negative = refusal();
    store.console_record_rejection(&owner, &negative).unwrap();
    let positive = ConsoleRunSnapshot {
        metadata: for_refusal(&negative),
        revision: 0,
        state: ConsoleRunState::Prepared,
        observations: ConsoleObservations::default(),
        steps: vec![],
        terminal: None,
    };
    assert!(
        insert_snapshot(&store.connection.lock(), &positive).is_err(),
        "DB guard must reject raw conflicting INSERT"
    );
    let other = prepared(&store, &owner, 1);
    assert!(
        store
            .connection
            .lock()
            .execute(
                "UPDATE snowflake_console_runs_v4 SET client_request_id=?1 WHERE run_id=?2",
                params![negative.client_request_id, other.metadata.run_id]
            )
            .is_err()
    );
    let denied = ConsoleAdmissionRejection {
        client_request_id: other.metadata.client_request_id.clone(),
        ..refusal()
    };
    assert!(insert_rejection(&store.connection.lock(), &denied).is_err());
    assert!(store.connection.lock().execute("UPDATE snowflake_console_rejections_v4 SET client_request_id=?1 WHERE client_request_id=?2", params![other.metadata.client_request_id,negative.client_request_id]).is_err());
    // Deliberately corrupt a fixture after proving the database guard, to test
    // the independent read-time defense rather than assuming triggers exist.
    store
        .connection
        .lock()
        .execute_batch("DROP TRIGGER console_run_insert_decision;")
        .unwrap();
    insert_snapshot(&store.connection.lock(), &positive).unwrap();
    assert!(store.console_run(&positive.metadata.run_id).is_err());
    assert!(
        store
            .console_run_by_client_request(&negative.client_request_id)
            .is_err()
    );
    assert!(
        store
            .console_rejection_by_client_request(&negative.client_request_id)
            .is_err()
    );
    assert!(validate_records(&store.connection.lock()).is_err());
}

fn fill_refusal_capacity(connection: &Connection, seed: &ConsoleAdmissionRejection) {
    connection.execute("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<10000) INSERT INTO snowflake_console_rejections_v4(client_request_id,session_id,plan_digest,code,recorded_at) SELECT printf('%08x-0000-4000-8000-000000000002',n),?1,?2,'approvalRequired',1000 FROM numbers", params![seed.session_id,seed.plan_digest]).unwrap();
}

#[test]
fn console_negative_capacity_preserves_lookup_dedup_and_rejects_restore_union_atomically() {
    let (_root, store, owner) = setup();
    let seed = refusal();
    fill_refusal_capacity(&store.connection.lock(), &seed);
    assert_eq!(
        store
            .console_record_rejection(&owner, &seed)
            .unwrap_err()
            .code,
        ErrorCode::ResultLimitReached
    );
    let first = store
        .console_rejection_by_client_request("00000001-0000-4000-8000-000000000002")
        .unwrap()
        .unwrap();
    assert_eq!(
        store.console_record_rejection(&owner, &first).unwrap(),
        first
    );
    assert_eq!(
        visit_ids(&store.connection.lock(), true, false, |_| Ok(())).unwrap(),
        MAX_REJECTIONS as usize
    );
    assert!(
        store.console_prepare_run(&owner, metadata(1)).is_ok(),
        "refusal budget is independent of positive-run capacity"
    );
    let mut full = pinned();
    fill_refusal_capacity(&full, &seed);
    let extra = pinned();
    insert_rejection(&extra, &seed).unwrap();
    assert!(merge_for_restore(&extra, &mut full).is_err());
    assert!(
        read_rejection(&full, &seed.client_request_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        visit_ids(&full, true, false, |_| Ok(())).unwrap(),
        MAX_REJECTIONS as usize
    );
}

#[test]
fn console_negative_restore_union_preserves_exact_refusals_and_rejects_both_cross_conflicts() {
    let (_root, store, owner) = setup();
    let value = refusal();
    store.console_record_rejection(&owner, &value).unwrap();
    let mut backup = pinned();
    insert_rejection(&backup, &value).unwrap();
    let backup_only = refusal();
    insert_rejection(&backup, &backup_only).unwrap();
    merge_for_restore(&store.connection.lock(), &mut backup).unwrap();
    assert_eq!(
        read_rejection(&backup, &value.client_request_id).unwrap(),
        Some(value.clone())
    );
    assert_eq!(
        read_rejection(&backup, &backup_only.client_request_id).unwrap(),
        Some(backup_only)
    );
    let mut changed = pinned();
    let mut different = value.clone();
    different.recorded_at += 1;
    insert_rejection(&changed, &different).unwrap();
    assert!(merge_for_restore(&store.connection.lock(), &mut changed).is_err());
    assert_eq!(
        read_rejection(&changed, &value.client_request_id).unwrap(),
        Some(different)
    );
    let mut backup_positive = pinned();
    let run = ConsoleRunSnapshot {
        metadata: for_refusal(&value),
        revision: 0,
        state: ConsoleRunState::Prepared,
        observations: ConsoleObservations::default(),
        steps: vec![],
        terminal: None,
    };
    insert_snapshot(&backup_positive, &run).unwrap();
    assert!(merge_for_restore(&store.connection.lock(), &mut backup_positive).is_err());
    assert_eq!(
        read_run(&backup_positive, &run.metadata.run_id).unwrap(),
        Some(run)
    );
    let positive = prepared(&store, &owner, 1);
    let mut backup_negative = pinned();
    let denied = ConsoleAdmissionRejection {
        client_request_id: positive.metadata.client_request_id.clone(),
        ..refusal()
    };
    insert_rejection(&backup_negative, &denied).unwrap();
    assert!(merge_for_restore(&store.connection.lock(), &mut backup_negative).is_err());
    assert_eq!(
        read_rejection(&backup_negative, &denied.client_request_id).unwrap(),
        Some(denied)
    );
}

#[test]
fn console_negative_v3_migration_excludes_live_owner_and_fences_an_already_open_v2_writer() {
    let (root, store, owner) = setup();
    let start = prepared(&store, &owner, 2);
    let pending = intended(&store, &owner, &start);
    let canonical_root = store.root.clone();
    drop(owner);
    drop(store);
    let old = Connection::open(root.path().join("joinery.sqlite")).unwrap();
    downgrade_fixture_to_v2(&old);
    // Prepare actual version-2 statements before migration, keeping this same
    // SQLite handle alive throughout. Reopening an old app would miss the race.
    let mut old_insert = old.prepare("INSERT INTO snowflake_console_runs(run_id,client_request_id,session_id,connection_id,plan_digest,step_count,created_at,revision,state,observations_json,terminal_json) VALUES(?1,?2,?3,?4,?5,?6,?7,0,'prepared',?8,NULL)").unwrap();
    let mut old_update = old.prepare("UPDATE snowflake_console_runs SET revision=?1,state=?2,observations_json=?3,terminal_json=?4 WHERE run_id=?5 AND revision=?6").unwrap();
    let mut old_step_update = old.prepare("UPDATE snowflake_console_steps SET outcome_json=?1 WHERE run_id=?2 AND ordinal=?3 AND outcome_json IS NULL").unwrap();
    let mut old_claim = old.prepare("SELECT CASE WHEN length(CAST(run_id AS BLOB))=36 THEN run_id ELSE '' END FROM snowflake_console_runs WHERE state!='terminal' ORDER BY run_id LIMIT 10001").unwrap();
    let lease = claim_migration_lease(&canonical_root).unwrap();
    assert_eq!(
        Persistence::open(root.path()).err().unwrap().code,
        ErrorCode::Conflict
    );
    assert_eq!(super::super::schema_version(&old).unwrap(), 2);
    assert_eq!(
        read_run(&old, &pending.metadata.run_id).unwrap(),
        Some(pending.clone())
    );
    assert_eq!(old.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('snowflake_console_runs_v4','snowflake_console_steps_v4','snowflake_console_rejections_v4')", [], |row| row.get::<_,u32>(0)).unwrap(), 0);
    drop(lease);
    let upgraded = Persistence::open(root.path()).unwrap();
    assert_eq!(
        upgraded.console_run(&pending.metadata.run_id).unwrap(),
        Some(pending.clone()),
        "migration must not recover or reclassify an original intent"
    );
    let another = metadata(1);
    let error = old_insert
        .execute(params![
            another.run_id,
            another.client_request_id,
            another.session_id,
            another.connection_id,
            another.plan_digest,
            another.step_count,
            1000,
            encode(&ConsoleObservations::default()).unwrap()
        ])
        .unwrap_err();
    assert!(error.to_string().contains("no such table"));
    let error = old_update
        .execute(params![
            sqlite_integer(pending.revision + 1).unwrap(),
            "terminal",
            encode(&ConsoleObservations::default()).unwrap(),
            Option::<String>::None,
            pending.metadata.run_id,
            sqlite_integer(pending.revision).unwrap()
        ])
        .unwrap_err();
    assert!(error.to_string().contains("no such table"));
    let error = old_step_update
        .execute(params![
            encode(&ConsoleStepOutcome::Unknown(ConsoleUnknown {
                statement: None
            }))
            .unwrap(),
            pending.metadata.run_id,
            0
        ])
        .unwrap_err();
    assert!(error.to_string().contains("no such table"));
    // query() only creates a lazy cursor. Exercise the actual old recovery
    // reader's first sqlite_step, where SQLite invalidates the cached schema.
    let mut old_rows = old_claim.query([]).unwrap();
    let error = old_rows
        .next()
        .expect_err("the stale recovery SELECT must fail when stepped");
    assert!(error.to_string().contains("no such table"));
    drop(old_rows);
    assert_eq!(
        upgraded.console_run(&pending.metadata.run_id).unwrap(),
        Some(pending.clone())
    );
    let owner = upgraded.claim_console_journal_owner().unwrap();
    let recovered = upgraded
        .console_run(&pending.metadata.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered.terminal.unwrap().reason,
        ConsoleStopReason::Interrupted
    );
    assert!(matches!(
        &recovered.steps[0].outcome,
        Some(ConsoleStepOutcome::Unknown(_))
    ));
    let refused = refusal();
    upgraded.console_record_rejection(&owner, &refused).unwrap();
    assert_eq!(
        upgraded
            .console_rejection_by_client_request(&refused.client_request_id)
            .unwrap(),
        Some(refused)
    );
}

#[test]
fn console_negative_all_journal_mutation_paths_recheck_schema_version_without_changing_evidence() {
    let (_root, store, owner) = setup();
    let start = prepared(&store, &owner, 1);
    let pending = intended(&store, &owner, &start);
    for version in [2, super::super::SCHEMA_VERSION + 1] {
        store
            .connection
            .lock()
            .pragma_update(None, "user_version", version)
            .unwrap();
        assert_eq!(
            store
                .console_prepare_run(&owner, metadata(1))
                .err()
                .unwrap()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            store
                .console_record_rejection(&owner, &refusal())
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            store
                .console_step_intent(&owner, &start.metadata.run_id, pending.revision, 1, 2000)
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            store
                .console_step_outcome(
                    &owner,
                    &start.metadata.run_id,
                    pending.revision,
                    0,
                    acknowledgement(),
                    ConsoleObservations::default()
                )
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            store
                .console_observe(
                    &owner,
                    &start.metadata.run_id,
                    pending.revision,
                    None,
                    ConsoleObservations::default()
                )
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            store
                .console_finish(
                    &owner,
                    &start.metadata.run_id,
                    pending.revision,
                    ConsoleStopReason::Cancelled,
                    2000
                )
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            store
                .console_interrupt_incomplete_runs(&owner)
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            interrupt_records(&mut store.connection.lock())
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            store.console_run(&start.metadata.run_id).unwrap_err().code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            store
                .console_rejection_by_client_request(&refusal().client_request_id)
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        store
            .connection
            .lock()
            .pragma_update(None, "user_version", super::super::SCHEMA_VERSION)
            .unwrap();
        assert_eq!(
            store.console_run(&start.metadata.run_id).unwrap(),
            Some(pending.clone())
        );
        assert!(visit_ids(&store.connection.lock(), true, false, |_| Ok(())).unwrap() == 0);
    }
    drop(owner);
    store
        .connection
        .lock()
        .pragma_update(None, "user_version", super::super::SCHEMA_VERSION + 1)
        .unwrap();
    assert_eq!(
        store.claim_console_journal_owner().err().unwrap().code,
        ErrorCode::Unsupported
    );
    store
        .connection
        .lock()
        .pragma_update(None, "user_version", super::super::SCHEMA_VERSION)
        .unwrap();
    assert_eq!(
        store.console_run(&start.metadata.run_id).unwrap(),
        Some(pending)
    );
}

#[test]
fn console_negative_concurrent_current_writers_commit_exactly_one_decision() {
    use std::sync::{Arc, Barrier};
    let (_root, store, owner) = setup();
    let store = Arc::new(store);
    let owner = Arc::new(owner);
    let negative = refusal();
    let positive = for_refusal(&negative);
    let ready = Arc::new(Barrier::new(3));
    let writes: Vec<_> = [false, true]
        .into_iter()
        .map(|reject| {
            let (store, owner, ready, negative, positive) = (
                store.clone(),
                owner.clone(),
                ready.clone(),
                negative.clone(),
                positive.clone(),
            );
            std::thread::spawn(move || {
                ready.wait();
                if reject {
                    store
                        .console_record_rejection(&owner, &negative)
                        .map(|_| ())
                } else {
                    store.console_prepare_run(&owner, positive).map(|_| ())
                }
            })
        })
        .collect();
    ready.wait();
    let outcomes: Vec<_> = writes
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes.into_iter().find_map(Result::err).unwrap().code,
        ErrorCode::Conflict
    );
    let yes = store
        .console_run_by_client_request(&negative.client_request_id)
        .unwrap()
        .is_some();
    let no = store
        .console_rejection_by_client_request(&negative.client_request_id)
        .unwrap()
        .is_some();
    assert_ne!(yes, no);
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn console_negative_two_coordinators_cannot_admit_while_other_lease_is_live() {
    use std::sync::{Arc, Barrier};
    let root = tempfile::tempdir().unwrap();
    let one = Persistence::open(root.path()).unwrap();
    let two = Persistence::open(root.path()).unwrap();
    let negative = refusal();
    let positive = for_refusal(&negative);
    let ready = Arc::new(Barrier::new(3));
    let attempted = Arc::new(Barrier::new(2));
    let writes: Vec<_> = [(one, false), (two, true)]
        .into_iter()
        .map(|(store, reject)| {
            let (ready, attempted, negative, positive) = (
                ready.clone(),
                attempted.clone(),
                negative.clone(),
                positive.clone(),
            );
            std::thread::spawn(move || {
                ready.wait();
                match store.claim_console_journal_owner() {
                    Ok(owner) => {
                        let result = if reject {
                            store
                                .console_record_rejection(&owner, &negative)
                                .map(|_| ())
                        } else {
                            store.console_prepare_run(&owner, positive).map(|_| ())
                        };
                        attempted.wait(); // retain the owner until the other claim finishes
                        result
                    }
                    Err(error) => {
                        attempted.wait();
                        Err(error)
                    }
                }
            })
        })
        .collect();
    ready.wait();
    let outcomes: Vec<_> = writes
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes.into_iter().find_map(Result::err).unwrap().code,
        ErrorCode::Conflict
    );
    let observed = Persistence::open(root.path()).unwrap();
    let yes = observed
        .console_run_by_client_request(&negative.client_request_id)
        .unwrap()
        .is_some();
    let no = observed
        .console_rejection_by_client_request(&negative.client_request_id)
        .unwrap()
        .is_some();
    assert_ne!(yes, no);
}

#[test]
fn console_journal_deduplicates_authority_identity_and_preserves_original_owner_metadata() {
    let (_root, store, owner) = setup();
    let run = prepared(&store, &owner, 2);
    let mut retry = run.metadata.clone();
    retry.run_id = crate::domain::new_id();
    retry.created_at += 900;
    let repeated = store.console_prepare_run(&owner, retry.clone()).unwrap();
    assert!(!repeated.created);
    assert_eq!(repeated.run, run);
    assert_eq!(
        store
            .console_run_by_client_request(&run.metadata.client_request_id)
            .unwrap(),
        Some(run.clone())
    );
    retry.plan_digest = "b".repeat(64);
    assert_eq!(
        store.console_prepare_run(&owner, retry).err().unwrap().code,
        ErrorCode::Conflict
    );
    let mut reused_owner = run.metadata.clone();
    reused_owner.client_request_id = crate::domain::new_id();
    assert_eq!(
        store
            .console_prepare_run(&owner, reused_owner)
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    let (_other_root, other, other_owner) = setup();
    assert_eq!(
        store
            .console_prepare_run(&other_owner, metadata(1))
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    assert!(other.console_prepare_run(&owner, metadata(1)).is_err());
}

#[test]
fn console_journal_rejects_stale_duplicate_and_out_of_order_transitions() {
    let (_root, store, owner) = setup();
    let start = prepared(&store, &owner, 2);
    let id = &start.metadata.run_id;
    assert!(store.console_step_intent(&owner, id, 0, 1, 1001).is_err());
    assert!(
        store
            .console_step_outcome(
                &owner,
                id,
                0,
                0,
                acknowledgement(),
                ConsoleObservations::default()
            )
            .is_err()
    );
    let pending = intended(&store, &owner, &start);
    assert!(
        store
            .console_step_intent(&owner, id, pending.revision, 1, 1001)
            .is_err()
    );
    assert!(
        store
            .console_step_outcome(
                &owner,
                id,
                0,
                0,
                acknowledgement(),
                ConsoleObservations::default()
            )
            .is_err()
    );
    assert!(
        store
            .console_finish(
                &owner,
                id,
                pending.revision,
                ConsoleStopReason::Completed,
                2000
            )
            .is_err()
    );
    let first = acknowledged(&store, &owner, &pending);
    assert!(
        store
            .console_step_outcome(
                &owner,
                id,
                first.revision,
                0,
                ConsoleStepOutcome::Unknown(ConsoleUnknown { statement: None }),
                ConsoleObservations::default()
            )
            .is_err()
    );
    let second = intended(&store, &owner, &first);
    let cancelled = store
        .console_observe(
            &owner,
            id,
            second.revision,
            None,
            ConsoleObservations {
                cancellation_observed: true,
                ..Default::default()
            },
        )
        .unwrap();
    // Cancellation stops admission, not retention of an already owned original.
    let completed = acknowledged(&store, &owner, &cancelled);
    let terminal = store
        .console_finish(
            &owner,
            id,
            completed.revision,
            ConsoleStopReason::Cancelled,
            2000,
        )
        .unwrap();
    assert_eq!(terminal.steps[0].outcome, first.steps[0].outcome);
    assert_eq!(
        terminal.terminal.as_ref().unwrap().accounting,
        ConsoleAccounting {
            planned: 2,
            intended: 2,
            acknowledged: 2,
            rejected: 0,
            unknown: 0,
            not_dispatched: 0,
            not_intended: 0,
            not_attempted: 0
        }
    );
    assert!(
        store
            .console_step_intent(&owner, id, terminal.revision, 2, 2001)
            .is_err()
    );
    assert!(
        store
            .console_finish(
                &owner,
                id,
                terminal.revision,
                ConsoleStopReason::Cancelled,
                2001
            )
            .is_err()
    );
}

#[test]
fn console_journal_limit_and_delivery_failure_do_not_replace_ack_or_touch_suffix() {
    let (_root, store, owner) = setup();
    let start = prepared(&store, &owner, 3);
    let pending = intended(&store, &owner, &start);
    let original = acknowledgement();
    let limited = store
        .console_step_outcome(
            &owner,
            &start.metadata.run_id,
            pending.revision,
            0,
            original.clone(),
            ConsoleObservations {
                truncated: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        store
            .console_step_intent(&owner, &start.metadata.run_id, limited.revision, 1, 1002)
            .is_err()
    );
    let stopped = store
        .console_finish(
            &owner,
            &start.metadata.run_id,
            limited.revision,
            ConsoleStopReason::ResultLimit,
            2000,
        )
        .unwrap();
    let terminal = stopped.terminal.clone();
    assert_eq!(terminal.as_ref().unwrap().accounting.not_attempted, 2);
    let observed = store
        .console_observe(
            &owner,
            &start.metadata.run_id,
            stopped.revision,
            Some(0),
            ConsoleObservations {
                delivered: true,
                delivery_failed: true,
                retired: true,
                first_error: Some(ConsoleErrorCode::Delivery),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(observed.steps[0].outcome, Some(original));
    assert_eq!(observed.terminal, terminal);
    assert_eq!(observed.steps.len(), 1);
    assert!(
        observed.observations.delivered
            && observed.observations.delivery_failed
            && observed.observations.truncated
    );
    assert!(
        store
            .console_observe(
                &owner,
                &start.metadata.run_id,
                observed.revision,
                Some(0),
                ConsoleObservations {
                    first_error: Some(ConsoleErrorCode::Protocol),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert!(
        store
            .console_observe(
                &owner,
                &start.metadata.run_id,
                observed.revision,
                Some(0),
                ConsoleObservations::default()
            )
            .is_err()
    );
}

#[test]
fn console_journal_wal_intent_is_committed_and_reopen_seals_unknown_without_replay() {
    let (root, store, owner) = setup();
    let start = prepared(&store, &owner, 3);
    let first = acknowledged(&store, &owner, &intended(&store, &owner, &start));
    let pending = intended(&store, &owner, &first);
    let reader = Connection::open(root.path().join("joinery.sqlite")).unwrap();
    let mode: String = reader
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    assert_eq!(
        read_run(&reader, &start.metadata.run_id).unwrap(),
        Some(pending.clone())
    );
    let second_store = Persistence::open(root.path()).unwrap();
    assert_eq!(
        second_store.console_run(&start.metadata.run_id).unwrap(),
        Some(pending)
    );
    assert_eq!(
        second_store
            .claim_console_journal_owner()
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    drop(reader);
    drop(second_store);
    drop(owner);
    drop(store);
    let reopened = Persistence::open(root.path()).unwrap();
    let owner = reopened.claim_console_journal_owner().unwrap();
    let recovered = reopened
        .console_run(&start.metadata.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.steps[0].outcome, first.steps[0].outcome);
    assert_eq!(
        recovered.steps[1].outcome,
        Some(ConsoleStepOutcome::Unknown(ConsoleUnknown {
            statement: None
        }))
    );
    let terminal = recovered.terminal.as_ref().unwrap();
    assert_eq!(terminal.reason, ConsoleStopReason::Interrupted);
    assert_eq!(
        terminal.accounting,
        ConsoleAccounting {
            planned: 3,
            intended: 2,
            acknowledged: 1,
            rejected: 0,
            unknown: 1,
            not_dispatched: 0,
            not_intended: 1,
            not_attempted: 1
        }
    );
    assert_eq!(
        reopened.console_interrupt_incomplete_runs(&owner).unwrap(),
        0
    );
    assert!(
        reopened
            .console_step_intent(&owner, &start.metadata.run_id, recovered.revision, 2, 2000)
            .is_err()
    );
    assert!(
        !reopened
            .console_prepare_run(&owner, start.metadata)
            .unwrap()
            .created
    );
}

#[test]
fn console_journal_read_only_write_failure_retains_original_for_explicit_retry() {
    let (root, store, owner) = setup();
    let start = prepared(&store, &owner, 2);
    let pending = intended(&store, &owner, &start);
    let read_only = Persistence {
        connection: parking_lot::Mutex::new(
            Connection::open_with_flags(
                root.path().join("joinery.sqlite"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap(),
        ),
        root: std::fs::canonicalize(root.path()).unwrap(),
    };
    let original = acknowledgement();
    let failure = read_only
        .console_step_outcome(
            &owner,
            &start.metadata.run_id,
            pending.revision,
            0,
            original.clone(),
            ConsoleObservations::default(),
        )
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::Internal);
    assert!(!failure.message.contains(root.path().to_str().unwrap()));
    assert_eq!(
        store.console_run(&start.metadata.run_id).unwrap(),
        Some(pending.clone())
    );
    // This retained original is caller-owned evidence; the failed write did not
    // pretend to persist it. Recovery of storage may record that exact evidence.
    let noted = store
        .console_observe(
            &owner,
            &start.metadata.run_id,
            pending.revision,
            None,
            ConsoleObservations {
                recording_failed: true,
                first_error: Some(ConsoleErrorCode::Recording),
                ..Default::default()
            },
        )
        .unwrap();
    let recorded = store
        .console_step_outcome(
            &owner,
            &start.metadata.run_id,
            noted.revision,
            0,
            original.clone(),
            ConsoleObservations::default(),
        )
        .unwrap();
    assert_eq!(recorded.steps[0].outcome, Some(original));
    assert!(
        store
            .console_step_intent(&owner, &start.metadata.run_id, recorded.revision, 1, 2000)
            .is_err()
    );
    assert!(
        store
            .console_finish(
                &owner,
                &start.metadata.run_id,
                recorded.revision,
                ConsoleStopReason::RecordingFailed,
                2000
            )
            .is_ok()
    );
}

#[test]
fn console_journal_original_rejection_and_unknown_stop_admissions_without_rewriting_evidence() {
    for original in [
        ConsoleStepOutcome::Rejected(ConsoleRejection {
            statement: None,
            code: ConsoleErrorCode::Database,
        }),
        ConsoleStepOutcome::Unknown(ConsoleUnknown {
            statement: Some(ConsoleStatement {
                sdk_request_id: crate::domain::new_id(),
                query_id: None,
            }),
        }),
    ] {
        let (_root, store, owner) = setup();
        let start = prepared(&store, &owner, 2);
        let pending = intended(&store, &owner, &start);
        let recorded = store
            .console_step_outcome(
                &owner,
                &start.metadata.run_id,
                pending.revision,
                0,
                original.clone(),
                ConsoleObservations::default(),
            )
            .unwrap();
        assert!(
            store
                .console_step_intent(&owner, &start.metadata.run_id, recorded.revision, 1, 2000)
                .is_err()
        );
        let reason = if matches!(original, ConsoleStepOutcome::Rejected(_)) {
            ConsoleStopReason::Failed
        } else {
            ConsoleStopReason::OutcomeUnknown
        };
        let terminal = store
            .console_finish(
                &owner,
                &start.metadata.run_id,
                recorded.revision,
                reason,
                2000,
            )
            .unwrap();
        assert_eq!(terminal.steps[0].outcome, Some(original));
        assert_eq!(terminal.terminal.unwrap().accounting.not_attempted, 1);
    }
}

#[test]
fn console_journal_rejects_corrupt_gaps_unknown_fields_and_oversized_records() {
    let (_root, store, owner) = setup();
    let start = prepared(&store, &owner, 2);
    let pending = intended(&store, &owner, &start);
    let id = &start.metadata.run_id;
    store
        .connection
        .lock()
        .execute(
            "UPDATE snowflake_console_steps_v4 SET ordinal=1 WHERE run_id=?1",
            [id],
        )
        .unwrap();
    assert!(store.console_run(id).is_err());
    assert!(
        store
            .console_step_outcome(
                &owner,
                id,
                pending.revision,
                0,
                acknowledgement(),
                ConsoleObservations::default()
            )
            .is_err()
    );
    store
        .connection
        .lock()
        .execute(
            "UPDATE snowflake_console_steps_v4 SET ordinal=0,outcome_json=?1 WHERE run_id=?2",
            params![
                r#"{"kind":"unknown","evidence":{"statement":null,"sql":"must-not-load"}}"#,
                id
            ],
        )
        .unwrap();
    assert!(validate_records(&store.connection.lock()).is_err());
    store.connection.lock().execute("UPDATE snowflake_console_steps_v4 SET outcome_json=NULL,observations_json=?1 WHERE run_id=?2", params![" ".repeat(MAX_JSON_BYTES + 1), id]).unwrap();
    assert!(store.console_run(id).is_err());
}

#[test]
fn console_journal_metadata_and_passive_context_bounds_are_closed() {
    let (_root, store, owner) = setup();
    for change in 0..5 {
        let mut value = metadata(1);
        match change {
            0 => value.step_count = MAX_STEPS + 1,
            1 => value.client_request_id = "not-an-id".into(),
            2 => value.plan_digest = "A".repeat(64),
            3 => value.created_at = u64::MAX,
            _ => value.run_id = "11111111-AAAA-4111-8111-111111111111".into(),
        }
        assert!(store.console_prepare_run(&owner, value).is_err());
    }
    let start = prepared(&store, &owner, 1);
    let pending = intended(&store, &owner, &start);
    for bad in ["x".repeat(256), "bad\0identity".into(), "界".repeat(256)] {
        let mut outcome = acknowledgement();
        if let ConsoleStepOutcome::Acknowledged(ack) = &mut outcome {
            ack.context.database = Some(bad);
        }
        assert!(
            store
                .console_step_outcome(
                    &owner,
                    &start.metadata.run_id,
                    pending.revision,
                    0,
                    outcome,
                    ConsoleObservations::default()
                )
                .is_err()
        );
    }
    let mut accepted = acknowledgement();
    if let ConsoleStepOutcome::Acknowledged(ack) = &mut accepted {
        ack.context.database = Some("😀".repeat(255));
    }
    assert!(
        store
            .console_step_outcome(
                &owner,
                &start.metadata.run_id,
                pending.revision,
                0,
                accepted,
                ConsoleObservations::default()
            )
            .is_ok()
    );
}

#[test]
fn console_journal_restore_preserves_current_ack_tombstones_and_seals_backup_only_intent() {
    let (_root, store, owner) = setup();
    let start = prepared(&store, &owner, 1);
    let pending = intended(&store, &owner, &start);
    let mut backup = pinned();
    insert_snapshot(&backup, &pending).unwrap();
    let ack = acknowledged(&store, &owner, &pending);
    let completed = store
        .console_finish(
            &owner,
            &start.metadata.run_id,
            ack.revision,
            ConsoleStopReason::Completed,
            2000,
        )
        .unwrap();
    let imported = ConsoleRunSnapshot {
        metadata: metadata(2),
        revision: 1,
        state: ConsoleRunState::Running,
        observations: ConsoleObservations::default(),
        steps: vec![ConsoleStep {
            ordinal: 0,
            intended_at: 1001,
            outcome: None,
            observations: ConsoleObservations::default(),
        }],
        terminal: None,
    };
    insert_snapshot(&backup, &imported).unwrap();
    merge_for_restore(&store.connection.lock(), &mut backup).unwrap();
    assert_eq!(
        read_run(&backup, &start.metadata.run_id).unwrap(),
        Some(completed)
    );
    let recovered = read_run(&backup, &imported.metadata.run_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        recovered.steps[0].outcome,
        Some(ConsoleStepOutcome::Unknown(_))
    ));
    assert_eq!(
        recovered.terminal.unwrap().reason,
        ConsoleStopReason::Interrupted
    );
    let clients: u32 = backup
        .query_row(
            "SELECT count(*) FROM snowflake_console_runs_v4 WHERE client_request_id IN (?1,?2)",
            params![
                start.metadata.client_request_id,
                imported.metadata.client_request_id
            ],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(clients, 2);
    validate_schema(&backup).unwrap();
    validate_records(&backup).unwrap();
}

#[test]
fn console_journal_restore_rejects_newer_or_conflicting_backup_evidence_atomically() {
    let (_root, store, owner) = setup();
    let start = prepared(&store, &owner, 1);
    let pending = intended(&store, &owner, &start);
    let mut newer = pending.clone();
    newer.revision += 1;
    newer.steps[0].outcome = Some(acknowledgement());
    let mut backup = pinned();
    insert_snapshot(&backup, &newer).unwrap();
    assert!(merge_for_restore(&store.connection.lock(), &mut backup).is_err());
    assert_eq!(
        read_run(&backup, &newer.metadata.run_id).unwrap(),
        Some(newer)
    );
    assert_eq!(
        store.console_run(&pending.metadata.run_id).unwrap(),
        Some(pending)
    );
    let mut clash = start.clone();
    clash.metadata.run_id = crate::domain::new_id();
    let mut other = pinned();
    insert_snapshot(&other, &clash).unwrap();
    assert!(merge_for_restore(&store.connection.lock(), &mut other).is_err());
    assert_eq!(
        read_run(&other, &clash.metadata.run_id).unwrap(),
        Some(clash)
    );
}

fn fill_capacity(connection: &Connection, seed: &ConsoleRunMetadata) {
    connection.execute("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<10000) INSERT INTO snowflake_console_runs_v4(run_id,client_request_id,session_id,connection_id,plan_digest,step_count,created_at,revision,state,observations_json) SELECT printf('%08x-0000-4000-8000-000000000000',n),printf('%08x-0000-4000-8000-000000000001',n),?1,?2,?3,1,1000,0,'prepared',?4 FROM numbers",
        params![seed.session_id,seed.connection_id,seed.plan_digest,encode(&ConsoleObservations::default()).unwrap()]).unwrap();
}

#[test]
fn console_journal_capacity_rejects_new_admission_and_restore_union_without_eviction() {
    let (_root, store, owner) = setup();
    let seed = metadata(1);
    fill_capacity(&store.connection.lock(), &seed);
    assert_eq!(
        store
            .console_prepare_run(&owner, metadata(1))
            .err()
            .unwrap()
            .code,
        ErrorCode::ResultLimitReached
    );
    assert_eq!(
        visit_ids(&store.connection.lock(), false, false, |_| Ok(())).unwrap(),
        MAX_RUNS as usize
    );
    let mut backup = pinned();
    fill_capacity(&backup, &seed);
    let empty = pinned();
    let extra = ConsoleRunSnapshot {
        metadata: metadata(1),
        revision: 0,
        state: ConsoleRunState::Prepared,
        observations: ConsoleObservations::default(),
        steps: vec![],
        terminal: None,
    };
    insert_snapshot(&empty, &extra).unwrap();
    assert!(merge_for_restore(&empty, &mut backup).is_err());
    assert_eq!(
        visit_ids(&backup, false, false, |_| Ok(())).unwrap(),
        MAX_RUNS as usize
    );
    assert!(read_run(&backup, &extra.metadata.run_id).unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn console_journal_rejects_linked_or_replaced_owner_lock() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let target = root.path().join("unrelated");
    std::fs::write(&target, b"preserve").unwrap();
    let lock = root.path().join("snowflake-console.lock");
    symlink(&target, &lock).unwrap();
    assert!(store.claim_console_journal_owner().is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"preserve");
    std::fs::remove_file(&lock).unwrap();
    let owner = store.claim_console_journal_owner().unwrap();
    std::fs::rename(&lock, root.path().join("original-lock")).unwrap();
    let replacement = store.claim_console_journal_owner().unwrap();
    assert!(store.console_prepare_run(&owner, metadata(1)).is_err());
    assert!(store.console_prepare_run(&replacement, metadata(1)).is_ok());
}

#[cfg(unix)]
#[test]
#[ignore = "internal subprocess fixture; invoked by console_journal_child_lease test"]
fn console_journal_lock_child() {
    let root =
        std::path::PathBuf::from(std::env::var_os("JOINERY_CONSOLE_JOURNAL_TEST_ROOT").unwrap());
    let store = Persistence::open(&root).unwrap();
    let owner = store.claim_console_journal_owner().unwrap();
    let start = prepared(&store, &owner, 3);
    let ack = acknowledged(&store, &owner, &intended(&store, &owner, &start));
    let pending = intended(&store, &owner, &ack);
    std::fs::write(
        root.join("child-ready.tmp"),
        serde_json::to_vec(&pending).unwrap(),
    )
    .unwrap();
    std::fs::rename(root.join("child-ready.tmp"), root.join("child-ready")).unwrap();
    loop {
        std::thread::park();
    }
}

#[cfg(unix)]
#[test]
fn console_journal_child_lease_blocks_live_recovery_and_kill_releases_for_unknown_sealing() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    struct OwnedChild(std::process::Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "persistence::snowflake_console::tests::console_journal_lock_child",
                "--nocapture",
            ])
            .env("JOINERY_CONSOLE_JOURNAL_TEST_ROOT", root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let ready = root.path().join("child-ready");
    while !ready.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "journal child exited before readiness"
        );
        assert!(
            Instant::now() < deadline,
            "journal child readiness deadline exceeded"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let pending: ConsoleRunSnapshot =
        serde_json::from_slice(&std::fs::read(&ready).unwrap()).unwrap();
    let store = Persistence::open(root.path()).unwrap();
    assert_eq!(
        store.console_run(&pending.metadata.run_id).unwrap(),
        Some(pending.clone())
    );
    assert_eq!(
        store.claim_console_journal_owner().err().unwrap().code,
        ErrorCode::Conflict
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let _owner = store.claim_console_journal_owner().unwrap();
    let recovered = store
        .console_run(&pending.metadata.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.steps[0].outcome, pending.steps[0].outcome);
    assert!(matches!(
        recovered.steps[1].outcome,
        Some(ConsoleStepOutcome::Unknown(_))
    ));
    assert_eq!(
        recovered.terminal.as_ref().unwrap().reason,
        ConsoleStopReason::Interrupted
    );
    assert_eq!(
        recovered
            .terminal
            .as_ref()
            .unwrap()
            .accounting
            .not_attempted,
        1
    );
}

#[test]
fn console_journal_terminal_reason_cannot_hide_unknown_or_stop_flags() {
    let (_root, store, owner) = setup();
    for observations in [
        ConsoleObservations {
            delivery_failed: true,
            ..Default::default()
        },
        ConsoleObservations {
            cancellation_observed: true,
            ..Default::default()
        },
        ConsoleObservations {
            truncated: true,
            ..Default::default()
        },
        ConsoleObservations {
            recording_failed: true,
            ..Default::default()
        },
        ConsoleObservations {
            retired: true,
            ..Default::default()
        },
    ] {
        let start = prepared(&store, &owner, 1);
        let pending = intended(&store, &owner, &start);
        let recorded = store
            .console_step_outcome(
                &owner,
                &start.metadata.run_id,
                pending.revision,
                0,
                acknowledgement(),
                observations,
            )
            .unwrap();
        assert!(
            store
                .console_finish(
                    &owner,
                    &start.metadata.run_id,
                    recorded.revision,
                    ConsoleStopReason::Completed,
                    2000
                )
                .is_err()
        );
        assert_eq!(
            store.console_run(&start.metadata.run_id).unwrap(),
            Some(recorded)
        );
    }
    let start = prepared(&store, &owner, 2);
    let pending = intended(&store, &owner, &start);
    let recorded = store
        .console_step_outcome(
            &owner,
            &start.metadata.run_id,
            pending.revision,
            0,
            ConsoleStepOutcome::Unknown(ConsoleUnknown { statement: None }),
            ConsoleObservations {
                cancellation_observed: true,
                delivery_failed: true,
                truncated: true,
                recording_failed: true,
                ..Default::default()
            },
        )
        .unwrap();
    for reason in [
        ConsoleStopReason::Cancelled,
        ConsoleStopReason::Failed,
        ConsoleStopReason::ResultLimit,
        ConsoleStopReason::RecordingFailed,
        ConsoleStopReason::Completed,
        ConsoleStopReason::Interrupted,
    ] {
        assert!(
            store
                .console_finish(
                    &owner,
                    &start.metadata.run_id,
                    recorded.revision,
                    reason,
                    2000
                )
                .is_err()
        );
    }
    let finished = store
        .console_finish(
            &owner,
            &start.metadata.run_id,
            recorded.revision,
            ConsoleStopReason::OutcomeUnknown,
            2000,
        )
        .unwrap();
    assert_eq!(finished.steps, recorded.steps);
    assert_eq!(finished.terminal.unwrap().accounting.unknown, 1);
}

#[test]
fn console_journal_late_observations_preserve_terminal_time_evidence() {
    let (_root, store, owner) = setup();
    let start = prepared(&store, &owner, 1);
    let pending = intended(&store, &owner, &start);
    let recorded = acknowledged(&store, &owner, &pending);
    let finished = store
        .console_finish(
            &owner,
            &start.metadata.run_id,
            recorded.revision,
            ConsoleStopReason::Completed,
            2000,
        )
        .unwrap();
    let late = store
        .console_observe(
            &owner,
            &start.metadata.run_id,
            finished.revision,
            Some(0),
            ConsoleObservations {
                delivery_failed: true,
                cancellation_observed: true,
                truncated: true,
                recording_failed: true,
                retired: true,
                first_error: Some(ConsoleErrorCode::Delivery),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(late.terminal, finished.terminal);
    assert_eq!(late.steps[0].outcome, recorded.steps[0].outcome);
    assert!(late.observations.delivery_failed && late.observations.recording_failed);
    assert!(!late.terminal.as_ref().unwrap().observations.delivery_failed);
    validate_records(&store.connection.lock()).unwrap();
    assert_eq!(
        store.console_run(&start.metadata.run_id).unwrap(),
        Some(late)
    );
}

#[test]
fn console_journal_rejects_malformed_persisted_terminal_snapshot() {
    let (_root, store, owner) = setup();
    let start = prepared(&store, &owner, 1);
    let pending = intended(&store, &owner, &start);
    let recorded = acknowledged(&store, &owner, &pending);
    let finished = store
        .console_finish(
            &owner,
            &start.metadata.run_id,
            recorded.revision,
            ConsoleStopReason::Completed,
            2000,
        )
        .unwrap();
    let mut forged_completed = finished.clone();
    forged_completed.observations.delivery_failed = true;
    forged_completed
        .terminal
        .as_mut()
        .unwrap()
        .observations
        .delivery_failed = true;
    assert!(validate_snapshot(&forged_completed).is_err());
    let original = serde_json::to_value(finished.terminal.unwrap()).unwrap();
    let mut malformed = original.clone();
    malformed["observations"]["deliveryFailed"] = true.into();
    let mut extra = original.clone();
    extra["serverError"] = "must not persist".into();
    let mut timestamp = original.clone();
    timestamp["finishedAt"] = serde_json::Value::from(u64::MAX);
    for value in [malformed, extra, timestamp] {
        store
            .connection
            .lock()
            .execute(
                "UPDATE snowflake_console_runs_v4 SET terminal_json=?1 WHERE run_id=?2",
                params![value.to_string(), start.metadata.run_id],
            )
            .unwrap();
        assert!(store.console_run(&start.metadata.run_id).is_err());
        assert!(validate_records(&store.connection.lock()).is_err());
    }
    store
        .connection
        .lock()
        .execute(
            "UPDATE snowflake_console_runs_v4 SET terminal_json=?1 WHERE run_id=?2",
            params![original.to_string(), start.metadata.run_id],
        )
        .unwrap();
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn console_journal_proven_non_dispatch_survives_restart_without_ack_unknown_or_replay() {
    for finish_before_restart in [false, true] {
        let (root, store, owner) = setup();
        let start = prepared(&store, &owner, 3);
        let pending = intended(&store, &owner, &start);
        // This value is permitted only when the retained adapter returned
        // attempted=false/report=None before supervisor admission. Absence of a
        // report after an accepted request does not establish this evidence.
        let original = ConsoleStepOutcome::NotDispatched(ConsoleNotDispatched {
            reason: ConsoleNotDispatchedReason::Cancelled,
        });
        let recorded = store
            .console_step_outcome(
                &owner,
                &start.metadata.run_id,
                pending.revision,
                0,
                original.clone(),
                ConsoleObservations {
                    cancellation_observed: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(
            store
                .console_step_intent(&owner, &start.metadata.run_id, recorded.revision, 1, 2000)
                .is_err()
        );
        assert!(
            store
                .console_step_outcome(
                    &owner,
                    &start.metadata.run_id,
                    recorded.revision,
                    0,
                    acknowledgement(),
                    ConsoleObservations::default()
                )
                .is_err()
        );
        assert!(
            store
                .console_finish(
                    &owner,
                    &start.metadata.run_id,
                    recorded.revision,
                    ConsoleStopReason::OutcomeUnknown,
                    2000
                )
                .is_err()
        );
        let expected_reason = if finish_before_restart {
            store
                .console_finish(
                    &owner,
                    &start.metadata.run_id,
                    recorded.revision,
                    ConsoleStopReason::Cancelled,
                    2000,
                )
                .unwrap();
            ConsoleStopReason::Cancelled
        } else {
            ConsoleStopReason::Interrupted
        };
        drop(owner);
        drop(store);
        let reopened = Persistence::open(root.path()).unwrap();
        let owner = reopened.claim_console_journal_owner().unwrap();
        let recovered = reopened
            .console_run(&start.metadata.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.steps.len(), 1);
        assert_eq!(recovered.steps[0].outcome, Some(original));
        let terminal = recovered.terminal.as_ref().unwrap();
        assert_eq!(terminal.reason, expected_reason);
        assert_eq!(
            terminal.accounting,
            ConsoleAccounting {
                planned: 3,
                intended: 1,
                acknowledged: 0,
                rejected: 0,
                unknown: 0,
                not_dispatched: 1,
                not_intended: 2,
                not_attempted: 3
            }
        );
        assert_eq!(
            reopened.console_interrupt_incomplete_runs(&owner).unwrap(),
            0
        );
        assert!(
            !reopened
                .console_prepare_run(&owner, start.metadata.clone())
                .unwrap()
                .created
        );
        assert!(
            reopened
                .console_step_intent(&owner, &start.metadata.run_id, recovered.revision, 1, 3000)
                .is_err()
        );
    }
}

#[test]
fn console_journal_non_dispatch_local_reason_is_closed_and_separate_from_rejection() {
    let (_root, store, owner) = setup();
    for reason in [
        ConsoleNotDispatchedReason::Revoked,
        ConsoleNotDispatchedReason::SessionClosed,
        ConsoleNotDispatchedReason::InvalidInput,
        ConsoleNotDispatchedReason::AdmissionRejected,
    ] {
        let start = prepared(&store, &owner, 2);
        let pending = intended(&store, &owner, &start);
        let recorded = store
            .console_step_outcome(
                &owner,
                &start.metadata.run_id,
                pending.revision,
                0,
                ConsoleStepOutcome::NotDispatched(ConsoleNotDispatched { reason }),
                ConsoleObservations::default(),
            )
            .unwrap();
        assert!(
            store
                .console_finish(
                    &owner,
                    &start.metadata.run_id,
                    recorded.revision,
                    ConsoleStopReason::Cancelled,
                    2000
                )
                .is_err()
        );
        let finished = store
            .console_finish(
                &owner,
                &start.metadata.run_id,
                recorded.revision,
                ConsoleStopReason::Failed,
                2000,
            )
            .unwrap();
        let counts = finished.terminal.unwrap().accounting;
        assert_eq!(counts.not_dispatched, 1);
        assert_eq!(counts.rejected, 0);
        assert_eq!(counts.unknown, 0);
    }
    assert!(
        decode::<ConsoleStepOutcome>(
            r#"{"kind":"notDispatched","evidence":{"reason":"serverError"}}"#
        )
        .is_err()
    );
    assert!(
        decode::<ConsoleStepOutcome>(
            r#"{"kind":"notDispatched","evidence":{"reason":"cancelled","statement":{}}}"#
        )
        .is_err()
    );
}

#[test]
fn console_journal_sqlite_integer_boundaries_are_checked_without_wrapping() {
    let (_root, store, owner) = setup();
    let mut maximum = metadata(1);
    maximum.created_at = i64::MAX as u64;
    let start = store.console_prepare_run(&owner, maximum).unwrap().run;
    let pending = store
        .console_step_intent(
            &owner,
            &start.metadata.run_id,
            start.revision,
            0,
            i64::MAX as u64,
        )
        .unwrap();
    assert_eq!(
        store.console_run(&start.metadata.run_id).unwrap(),
        Some(pending.clone())
    );
    let ack = acknowledged(&store, &owner, &pending);
    let finished = store
        .console_finish(
            &owner,
            &start.metadata.run_id,
            ack.revision,
            ConsoleStopReason::Completed,
            i64::MAX as u64,
        )
        .unwrap();
    assert_eq!(
        finished.terminal.as_ref().unwrap().finished_at,
        i64::MAX as u64
    );
    let mut overflow = metadata(1);
    overflow.created_at = i64::MAX as u64 + 1;
    let overflow_id = overflow.run_id.clone();
    assert!(store.console_prepare_run(&owner, overflow).is_err());
    assert_eq!(store.console_run(&overflow_id).unwrap(), None);
    let next = prepared(&store, &owner, 1);
    assert!(
        store
            .console_step_intent(
                &owner,
                &next.metadata.run_id,
                next.revision,
                0,
                i64::MAX as u64 + 1
            )
            .is_err()
    );
    assert_eq!(
        store.console_run(&next.metadata.run_id).unwrap(),
        Some(next.clone())
    );
    assert!(
        store
            .console_finish(
                &owner,
                &next.metadata.run_id,
                next.revision,
                ConsoleStopReason::Completed,
                u64::MAX
            )
            .is_err()
    );
    {
        let connection = store.connection.lock();
        connection
            .execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        for column in ["created_at", "revision"] {
            connection
                .execute(
                    &format!("UPDATE snowflake_console_runs_v4 SET {column}=-1 WHERE run_id=?1"),
                    [&next.metadata.run_id],
                )
                .unwrap();
            assert!(read_run(&connection, &next.metadata.run_id).is_err());
            connection
                .execute(
                    &format!("UPDATE snowflake_console_runs_v4 SET {column}=?1 WHERE run_id=?2"),
                    params![
                        if column == "created_at" {
                            1000_i64
                        } else {
                            0_i64
                        },
                        next.metadata.run_id
                    ],
                )
                .unwrap();
        }
        connection
            .execute(
                "UPDATE snowflake_console_steps_v4 SET intended_at=-1 WHERE run_id=?1",
                [&start.metadata.run_id],
            )
            .unwrap();
        assert!(read_run(&connection, &start.metadata.run_id).is_err());
        connection
            .execute(
                "UPDATE snowflake_console_steps_v4 SET intended_at=?1 WHERE run_id=?2",
                params![i64::MAX, start.metadata.run_id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE snowflake_console_runs_v4 SET revision=?1 WHERE run_id=?2",
                params![i64::MAX, next.metadata.run_id],
            )
            .unwrap();
        assert!(read_run(&connection, &next.metadata.run_id).is_err());
        connection
            .execute(
                "UPDATE snowflake_console_runs_v4 SET revision=?1 WHERE run_id=?2",
                params![
                    sqlite_integer(MAX_REVISION - 1).unwrap(),
                    next.metadata.run_id
                ],
            )
            .unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
    }
    let ceiling = store
        .console_observe(
            &owner,
            &next.metadata.run_id,
            MAX_REVISION - 1,
            None,
            ConsoleObservations {
                cancellation_observed: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(ceiling.revision, MAX_REVISION);
    assert!(
        store
            .console_finish(
                &owner,
                &next.metadata.run_id,
                MAX_REVISION,
                ConsoleStopReason::Cancelled,
                2000
            )
            .is_err()
    );
    assert_eq!(
        store.console_run(&next.metadata.run_id).unwrap(),
        Some(ceiling)
    );
    validate_records(&store.connection.lock()).unwrap();
}

fn expected_logical_charge(
    runs: &[&ConsoleRunSnapshot],
    refusals: &[&ConsoleAdmissionRejection],
) -> u64 {
    fn json_size(value: &impl Serialize) -> usize {
        serde_json::to_vec(value).unwrap().len()
    }
    let mut bytes = 0usize;
    for run in runs {
        let metadata = &run.metadata;
        bytes += 512
            + metadata.run_id.len()
            + metadata.client_request_id.len()
            + metadata.session_id.len()
            + metadata.connection_id.len()
            + metadata.plan_digest.len()
            + state_name(run.state).len()
            + json_size(&run.observations)
            + run.terminal.as_ref().map_or(0, json_size);
        for step in &run.steps {
            bytes +=
                256 + step.outcome.as_ref().map_or(0, json_size) + json_size(&step.observations);
        }
    }
    for refusal in refusals {
        bytes += 256
            + refusal.client_request_id.len()
            + refusal.session_id.len()
            + refusal.plan_digest.len()
            + serde_json::to_value(refusal.code)
                .unwrap()
                .as_str()
                .unwrap()
                .len();
    }
    bytes as u64
}

fn assert_logical_accounting(
    store: &Persistence,
    runs: &[&ConsoleRunSnapshot],
    refusals: &[&ConsoleAdmissionRejection],
) {
    let storage = store.console_storage().unwrap();
    assert_eq!(storage.positive_count, runs.len().to_string());
    assert_eq!(storage.refusal_count, refusals.len().to_string());
    assert_eq!(
        storage.charged_bytes,
        Some(expected_logical_charge(runs, refusals).to_string())
    );
    assert!(storage.reserved_bytes.is_none());
    assert!(storage.admission_budget_bytes.is_none());
    assert!(storage.free_bytes.is_none());
    assert!(!storage.retention_enabled);
    // This independently streams the rows and checks the trigger-maintained
    // totals rather than trusting the storage DTO's cached control row.
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn console_accounting_tracks_utf8_growth_shrink_late_observations_and_migration() {
    let (root, store, owner) = setup();
    assert_logical_accounting(&store, &[], &[]);
    let negative = refusal();
    store.console_record_rejection(&owner, &negative).unwrap();
    let prepared = prepared(&store, &owner, 1);
    assert_logical_accounting(&store, &[&prepared], &[&negative]);
    let intent = intended(&store, &owner, &prepared);
    assert_logical_accounting(&store, &[&intent], &[&negative]);

    let before = expected_logical_charge(&[&intent], &[&negative]);
    let delivered = store
        .console_observe(
            &owner,
            &intent.metadata.run_id,
            intent.revision,
            None,
            ConsoleObservations {
                delivered: true,
                ..Default::default()
            },
        )
        .unwrap();
    // Canonical JSON true is one byte shorter than false. Charge updates must
    // allow shrinkage without an unsigned subtraction wrapping.
    assert_eq!(
        expected_logical_charge(&[&delivered], &[&negative]),
        before - 1
    );
    assert_logical_accounting(&store, &[&delivered], &[&negative]);

    let ConsoleStepOutcome::Acknowledged(mut ack) = acknowledgement() else {
        unreachable!()
    };
    ack.context.database = Some("資料庫 \"quoted\" \\ 😀".into());
    ack.context.schema = Some("schema\nwith\ttabs".into());
    let acknowledged = store
        .console_step_outcome(
            &owner,
            &delivered.metadata.run_id,
            delivered.revision,
            0,
            ConsoleStepOutcome::Acknowledged(ack),
            ConsoleObservations::default(),
        )
        .unwrap();
    assert_logical_accounting(&store, &[&acknowledged], &[&negative]);
    let terminal = store
        .console_finish(
            &owner,
            &acknowledged.metadata.run_id,
            acknowledged.revision,
            ConsoleStopReason::Completed,
            2000,
        )
        .unwrap();
    assert_logical_accounting(&store, &[&terminal], &[&negative]);
    let late = store
        .console_observe(
            &owner,
            &terminal.metadata.run_id,
            terminal.revision,
            None,
            ConsoleObservations {
                delivery_failed: true,
                first_error: Some(ConsoleErrorCode::Delivery),
                ..Default::default()
            },
        )
        .unwrap();
    assert_logical_accounting(&store, &[&late], &[&negative]);

    let summary = store
        .console_receipt_summary(&late.metadata.client_request_id)
        .unwrap()
        .unwrap();
    store
        .console_archive_receipts(
            &owner,
            &SnowflakeConsoleReceiptArchiveRequest {
                inventory_epoch: summary.inventory_epoch.clone(),
                items: vec![archive_item(&summary)],
                archived: true,
            },
            3000,
        )
        .unwrap();
    assert_logical_accounting(&store, &[&late], &[&negative]);
    drop(owner);
    drop(store);
    let reopened = Persistence::open(root.path()).unwrap();
    assert_logical_accounting(&reopened, &[&late], &[&negative]);
    assert_eq!(
        reopened.console_run(&late.metadata.run_id).unwrap(),
        Some(late.clone())
    );
    drop(reopened);

    let legacy = Connection::open(root.path().join("joinery.sqlite")).unwrap();
    downgrade_fixture_to_v3(&legacy);
    drop(legacy);
    let migrated = Persistence::open(root.path()).unwrap();
    assert_logical_accounting(&migrated, &[&late], &[&negative]);
    assert_eq!(
        migrated.console_run(&late.metadata.run_id).unwrap(),
        Some(late)
    );
}

#[test]
fn console_accounting_restore_recomputes_disjoint_decisions_without_losing_originals() {
    let (_root, store, owner) = setup();
    let intent = intended(&store, &owner, &prepared(&store, &owner, 1));
    let ack = acknowledged(&store, &owner, &intent);
    let terminal = store
        .console_finish(
            &owner,
            &ack.metadata.run_id,
            ack.revision,
            ConsoleStopReason::Completed,
            2000,
        )
        .unwrap();
    let live_refusal = refusal();
    store
        .console_record_rejection(&owner, &live_refusal)
        .unwrap();
    let backup_refusal = refusal();
    let mut backup = pinned();
    insert_rejection(&backup, &backup_refusal).unwrap();
    merge_for_restore(&store.connection.lock(), &mut backup).unwrap();
    validate_records(&backup).unwrap();
    validate_schema(&backup).unwrap();
    let (positive, negative, bytes): (i64, i64, i64) = backup
        .query_row(
            "SELECT positive_count,refusal_count,charged_bytes FROM snowflake_console_retention_v4",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!((positive, negative), (1, 2));
    assert_eq!(
        bytes as u64,
        expected_logical_charge(&[&terminal], &[&live_refusal, &backup_refusal])
    );
    assert_eq!(
        read_run(&backup, &terminal.metadata.run_id).unwrap(),
        Some(terminal)
    );
    assert_eq!(
        read_rejection(&backup, &live_refusal.client_request_id).unwrap(),
        Some(live_refusal)
    );
    assert_eq!(
        read_rejection(&backup, &backup_refusal.client_request_id).unwrap(),
        Some(backup_refusal)
    );
}
