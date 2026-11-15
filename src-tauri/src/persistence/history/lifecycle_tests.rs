use super::*;
use crate::persistence::snowflake_console::{
    ConsoleAcknowledgement, ConsoleContext, ConsoleErrorCode, ConsoleJournalOwner,
    ConsoleObservations, ConsoleRunMetadata, ConsoleRunSnapshot, ConsoleStatement,
    ConsoleStepOutcome, ConsoleStopReason,
};

fn profile() -> ConnectionProfile {
    ConnectionProfile {
        id: "00000000-0000-4000-8000-000000000005".into(),
        name: "Local fixture".into(),
        group_id: "local".into(),
        engine: EngineKind::Postgres,
        host: "127.0.0.1".into(),
        port: 5432,
        database: "fixture".into(),
        username: "fixture".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::Confirm,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: true,
        store_drafts: false,
        snowflake: None,
    }
}

fn seed(id: &str, sql: Option<&str>) -> HistoryEntry {
    HistoryEntry {
        id: id.into(),
        connection_id: profile().id,
        connection_name: profile().name,
        started_at: now_ms(),
        sql: sql.map(str::to_owned),
        statement_count: 1,
        provenance: HistoryProvenance::Admitted,
        status: None,
        elapsed_ms: None,
        execution_summary: None,
        snowflake_dml: None,
        snowflake_ddl: None,
    }
}

fn completion() -> HistoryCompletion {
    HistoryCompletion {
        provenance: HistoryProvenance::Recorded,
        status: Some(RunStatus::Completed),
        elapsed_ms: Some(17),
        execution_summary: None,
        snowflake_dml: None,
        snowflake_ddl: None,
    }
}

fn metadata(seed: &HistoryEntry) -> ConsoleRunMetadata {
    ConsoleRunMetadata {
        run_id: seed.id.clone(),
        client_request_id: new_id(),
        session_id: new_id(),
        connection_id: seed.connection_id.clone(),
        plan_digest: "a".repeat(64),
        step_count: 1,
        created_at: seed.started_at,
    }
}

fn acknowledge(
    store: &Persistence,
    owner: &ConsoleJournalOwner,
    run: &ConsoleRunSnapshot,
) -> ConsoleRunSnapshot {
    let intent = store
        .console_step_intent(
            owner,
            &run.metadata.run_id,
            run.revision,
            0,
            run.metadata.created_at,
        )
        .unwrap();
    store
        .console_step_outcome(
            owner,
            &run.metadata.run_id,
            intent.revision,
            0,
            ConsoleStepOutcome::Acknowledged(ConsoleAcknowledgement {
                statement: ConsoleStatement {
                    sdk_request_id: new_id(),
                    query_id: Some(new_id()),
                },
                context: ConsoleContext {
                    database: Some("quoted database".into()),
                    schema: None,
                    role: None,
                    warehouse: None,
                },
            }),
            ConsoleObservations::default(),
        )
        .unwrap()
}

#[test]
fn history_ticket_finish_and_readback_reject_changed_immutable_rows_without_overwriting_them() {
    for (field, changed) in [
        ("connectionName", serde_json::json!("changed")),
        ("sql", serde_json::json!("different SQL")),
        ("startedAt", serde_json::json!(1)),
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        store.save_profile(&profile()).unwrap();
        let original = seed(&new_id(), Some("SELECT 'original 雪'"));
        let ticket = store.begin_history(&original).unwrap();
        let mut corrupted = serde_json::to_value(&original).unwrap();
        corrupted[field] = changed;
        let raw = corrupted.to_string();
        store
            .connection
            .lock()
            .execute(
                "UPDATE history_v8 SET json=?1 WHERE id=?2",
                params![raw, original.id],
            )
            .unwrap();
        assert!(
            store.finish_history(&ticket, &completion()).is_err(),
            "{field}"
        );
        assert!(store.begin_history(&original).is_err(), "{field}");
        let connection = store.connection.lock();
        assert!(lookup(&connection, &original.id).is_err(), "{field}");
        let unchanged: String = connection
            .query_row(
                "SELECT json FROM history_v8 WHERE id=?1",
                [&original.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unchanged, raw);
        assert!(
            read_ticket(&connection, &original.id)
                .unwrap()
                .unwrap()
                .7
                .is_none()
        );
        connection
            .execute(
                "UPDATE history_v8 SET json=?1 WHERE id=?2",
                params![encode(&original).unwrap(), original.id],
            )
            .unwrap();
        drop(connection);
        assert_eq!(
            store.finish_history(&ticket, &completion()).unwrap(),
            HistoryDisposition::Applied
        );
        assert_eq!(
            store.history_entry(&original.id).unwrap().unwrap().sql,
            original.sql
        );
    }
}

#[test]
fn history_ticket_clear_prune_and_text_revocation_cannot_be_undone_by_retries() {
    for action in ["clear", "prune", "missing", "optout", "delete"] {
        let root = tempfile::tempdir().unwrap();
        let store = Persistence::open(root.path()).unwrap();
        let mut connection_profile = profile();
        store.save_profile(&connection_profile).unwrap();
        let mut original = seed(&new_id(), Some("SELECT 'permitted original'"));
        if action == "prune" {
            original.started_at = now_ms() - 31 * 24 * 60 * 60 * 1000;
        }
        let ticket = store.begin_history(&original).unwrap();
        match action {
            "clear" => store.clear_history().unwrap(),
            "prune" => {
                // Admission itself invokes the actual 30-day pruner.
                assert!(store.history_entry(&original.id).unwrap().is_none());
                assert_eq!(
                    lookup(&store.connection.lock(), &original.id)
                        .unwrap()
                        .unwrap()
                        .1,
                    HistoryDisposition::Suppressed
                );
            }
            "missing" => {
                store
                    .connection
                    .lock()
                    .execute("DELETE FROM history_v8 WHERE id=?1", [&original.id])
                    .unwrap();
            }
            "optout" => {
                connection_profile.store_query_text = false;
                store.save_profile(&connection_profile).unwrap();
                connection_profile.store_query_text = true;
                store.save_profile(&connection_profile).unwrap();
            }
            "delete" => {
                store.delete_profile(&connection_profile.id).unwrap();
                store.save_profile(&connection_profile).unwrap();
            }
            _ => unreachable!(),
        }
        store.begin_history(&original).unwrap();
        let disposition = store.finish_history(&ticket, &completion()).unwrap();
        let row = store.history_entry(&original.id).unwrap();
        if matches!(action, "clear" | "prune" | "missing") {
            assert_eq!(disposition, HistoryDisposition::Suppressed);
            assert!(row.is_none());
        } else {
            assert_eq!(disposition, HistoryDisposition::Applied);
            assert!(row.unwrap().sql.is_none());
        }
        assert_eq!(
            store.finish_history(&ticket, &completion()).unwrap(),
            disposition
        );
        validate_records(&store.connection.lock()).unwrap();
        drop(store);
        let reopened = Persistence::open(root.path()).unwrap();
        assert!(
            reopened
                .history_entry(&original.id)
                .unwrap()
                .is_none_or(|row| row.sql.is_none())
        );
    }
}

#[test]
fn history_console_prepare_and_complete_rollback_all_obligations_on_audit_failure() {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    store.save_profile(&profile()).unwrap();
    let owner = store.claim_console_journal_owner().unwrap();
    let original = seed(&new_id(), Some("SELECT 'exact original'"));
    let meta = metadata(&original);
    store.connection.lock().execute_batch("CREATE TRIGGER fail_start BEFORE INSERT ON audit_v8 WHEN NEW.outcome='started' BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    assert_eq!(
        store
            .console_prepare_lifecycle(&owner, meta.clone(), &original)
            .unwrap_err()
            .code,
        ErrorCode::Internal,
        "the injected audit trigger must cause the failure after valid admission"
    );
    assert!(store.console_run(&original.id).unwrap().is_none());
    assert!(store.history_entry(&original.id).unwrap().is_none());
    assert!(
        read_ticket(&store.connection.lock(), &original.id)
            .unwrap()
            .is_none()
    );
    assert!(store.audit_log().unwrap().is_empty());
    store
        .connection
        .lock()
        .execute_batch("DROP TRIGGER fail_start")
        .unwrap();
    let prepared = store
        .console_prepare_lifecycle(&owner, meta.clone(), &original)
        .unwrap();
    assert_eq!(prepared.history, HistoryDisposition::Pending);
    assert_eq!(
        store.history_entry(&original.id).unwrap().unwrap().sql,
        original.sql
    );
    let acknowledged = acknowledge(&store, &owner, &prepared.run);
    // The compatibility terminal path must never bypass atomic completion.
    assert!(
        store
            .console_finish(
                &owner,
                &original.id,
                acknowledged.revision,
                ConsoleStopReason::Completed,
                now_ms()
            )
            .is_err()
    );
    store.connection.lock().execute_batch("CREATE TRIGGER fail_finish BEFORE INSERT ON audit_v8 WHEN NEW.outcome='Completed' BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    let finished_at = now_ms();
    assert!(
        store
            .console_complete_lifecycle(
                &owner,
                &original.id,
                acknowledged.revision,
                ConsoleStopReason::Completed,
                finished_at,
                &completion()
            )
            .is_err()
    );
    assert_eq!(
        store.console_run(&original.id).unwrap(),
        Some(acknowledged.clone())
    );
    assert_eq!(
        store
            .history_entry(&original.id)
            .unwrap()
            .unwrap()
            .provenance,
        HistoryProvenance::Admitted
    );
    assert_eq!(store.audit_log().unwrap().len(), 1);
    store
        .connection
        .lock()
        .execute_batch("DROP TRIGGER fail_finish")
        .unwrap();
    let completed = store
        .console_complete_lifecycle(
            &owner,
            &original.id,
            acknowledged.revision,
            ConsoleStopReason::Completed,
            finished_at,
            &completion(),
        )
        .unwrap();
    assert_eq!(completed.history, HistoryDisposition::Applied);
    assert_eq!(completed.run.steps, acknowledged.steps);
    assert_eq!(store.audit_log().unwrap().len(), 2);
    assert!(
        store
            .console_prepare_lifecycle(&owner, meta, &original)
            .unwrap()
            .completion
            .unwrap()
            .matches(&completion())
    );
    let repeat = store
        .console_complete_lifecycle(
            &owner,
            &original.id,
            acknowledged.revision,
            ConsoleStopReason::Completed,
            finished_at,
            &completion(),
        )
        .unwrap();
    assert_eq!(repeat.run, completed.run);
}

#[test]
fn history_console_seal_retains_late_observations_and_original_ack_then_closes_publication() {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    store.save_profile(&profile()).unwrap();
    let owner = store.claim_console_journal_owner().unwrap();
    let original = seed(&new_id(), Some("SELECT 'original'"));
    let prepared = store
        .console_prepare_lifecycle(&owner, metadata(&original), &original)
        .unwrap();
    assert!(
        store
            .console_seal_publishers(&owner, &original.id, prepared.run.revision)
            .is_err()
    );
    let ack = acknowledge(&store, &owner, &prepared.run);
    let completed = store
        .console_complete_lifecycle(
            &owner,
            &original.id,
            ack.revision,
            ConsoleStopReason::Completed,
            now_ms(),
            &completion(),
        )
        .unwrap();
    assert!(
        !store
            .console_receipt_summary(&completed.run.metadata.client_request_id)
            .unwrap()
            .unwrap()
            .archive_eligible
    );
    let late = store
        .console_observe(
            &owner,
            &original.id,
            completed.run.revision,
            None,
            ConsoleObservations {
                delivery_failed: true,
                first_error: Some(ConsoleErrorCode::Delivery),
                ..Default::default()
            },
        )
        .unwrap();
    let sealed = store
        .console_seal_publishers(&owner, &original.id, late.revision)
        .unwrap();
    assert!(sealed.publisher_sealed);
    assert_eq!(sealed.run, late);
    assert_eq!(sealed.run.steps, ack.steps);
    assert!(
        store
            .console_receipt_summary(&late.metadata.client_request_id)
            .unwrap()
            .unwrap()
            .archive_eligible
    );
    assert!(
        store
            .console_observe(
                &owner,
                &original.id,
                late.revision,
                None,
                ConsoleObservations {
                    cancellation_observed: true,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(
        store
            .console_seal_publishers(&owner, &original.id, completed.run.revision)
            .unwrap()
            .run,
        late
    );
    assert_eq!(
        store.history_entry(&original.id).unwrap().unwrap().status,
        Some(RunStatus::Completed)
    );
    // Even an already-applied completion retry validates the visible row.
    store
        .connection
        .lock()
        .execute(
            "UPDATE history_v8 SET json=json_set(json,'$.status','cancelled') WHERE id=?1",
            [&original.id],
        )
        .unwrap();
    assert!(store.console_lifecycle(&original.id).is_err());
    assert!(
        store
            .console_complete_lifecycle(
                &owner,
                &original.id,
                ack.revision,
                ConsoleStopReason::Completed,
                completed.run.terminal.unwrap().finished_at,
                &completion()
            )
            .is_err()
    );
}

#[test]
fn history_console_recovery_seals_terminal_without_changing_completion_or_other_live_tickets() {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    store.save_profile(&profile()).unwrap();
    let owner = store.claim_console_journal_owner().unwrap();
    let generic = seed(&new_id(), Some("SELECT 'generic still pending'"));
    store.begin_history(&generic).unwrap();
    let original = seed(&new_id(), Some("SELECT 'console'"));
    let prepared = store
        .console_prepare_lifecycle(&owner, metadata(&original), &original)
        .unwrap();
    let ack = acknowledge(&store, &owner, &prepared.run);
    let completed = store
        .console_complete_lifecycle(
            &owner,
            &original.id,
            ack.revision,
            ConsoleStopReason::Completed,
            now_ms(),
            &completion(),
        )
        .unwrap();
    drop(owner);
    drop(store);
    let reopened = Persistence::open(root.path()).unwrap();
    assert!(
        !reopened
            .console_lifecycle(&original.id)
            .unwrap()
            .unwrap()
            .publisher_sealed
    );
    let _owner = reopened.claim_console_journal_owner().unwrap();
    let recovered = reopened.console_lifecycle(&original.id).unwrap().unwrap();
    assert!(recovered.publisher_sealed);
    assert_eq!(recovered.run, completed.run);
    assert!(recovered.completion.unwrap().matches(&completion()));
    let pending = reopened.history_entry(&generic.id).unwrap().unwrap();
    assert_eq!(pending.provenance, HistoryProvenance::Admitted);
    assert!(pending.status.is_none() && pending.elapsed_ms.is_none());
    assert_eq!(pending.sql, generic.sql);
}

#[test]
fn history_restore_unrelated_sql_free_generations_preserves_receipts_without_granting_text() {
    let roots = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let live = Persistence::open(roots[0].path()).unwrap();
    let backup = Persistence::open(roots[1].path()).unwrap();
    for store in [&live, &backup] {
        store.save_profile(&profile()).unwrap();
    }
    let a = seed(&new_id(), None);
    let b = seed(&new_id(), None);
    live.begin_history(&a).unwrap();
    backup.begin_history(&b).unwrap();
    let current = live.connection.lock();
    let mut candidate = backup.connection.lock();
    let tx = candidate.transaction().unwrap();
    merge_for_restore(&current, &tx).unwrap();
    validate_records(&tx).unwrap();
    assert!(read_ticket(&tx, &a.id).unwrap().is_some());
    assert!(read_ticket(&tx, &b.id).unwrap().is_some());
    assert_eq!(
        read_ticket(&tx, &b.id).unwrap().unwrap().6,
        HistoryDisposition::Suppressed
    );
    assert!(history_row(&tx, &b.id).unwrap().is_none());
    assert!(history_row(&tx, &a.id).unwrap().unwrap().sql.is_none());
    tx.commit().unwrap();
}
