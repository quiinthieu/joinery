//! Private owner invariants with disposable SQLite/Arrow and the real helper.
use super::*;
use crate::{
    execution::snowflake_console_tests::{
        FIXTURE_LANE, approved, connected, database, finished, new_studio, profile, queries,
        start_run,
    },
    snowflake_runtime::studio_fixture::Fixture,
};

fn empty_terminal() -> ConsoleRunSnapshot {
    ConsoleRunSnapshot {
        metadata: ConsoleRunMetadata {
            run_id: new_id(),
            client_request_id: new_id(),
            session_id: new_id(),
            connection_id: new_id(),
            plan_digest: "a".repeat(64),
            step_count: 1,
            created_at: 1,
        },
        revision: 1,
        state: ConsoleRunState::Terminal,
        observations: ConsoleObservations::default(),
        steps: Vec::new(),
        terminal: Some(ConsoleTerminal {
            reason: ConsoleStopReason::Interrupted,
            finished_at: 2,
            accounting: ConsoleAccounting {
                planned: 1,
                intended: 0,
                acknowledged: 0,
                rejected: 0,
                unknown: 0,
                not_dispatched: 0,
                not_intended: 1,
                not_attempted: 1,
            },
            observations: ConsoleObservations::default(),
        }),
    }
}

fn retained(run: ConsoleRunSnapshot) -> Retained {
    Retained {
        run,
        writes: VecDeque::new(),
        prepare_confirmed: true,
        completion: None,
        seal_pending: false,
        publisher_sealed: true,
    }
}

fn archive_request(
    summary: &SnowflakeConsoleReceiptSummary,
) -> SnowflakeConsoleReceiptArchiveRequest {
    SnowflakeConsoleReceiptArchiveRequest {
        inventory_epoch: summary.inventory_epoch.clone(),
        archived: true,
        items: vec![SnowflakeConsoleReceiptArchiveItem {
            client_request_id: summary.client_request_id.clone(),
            session_id: summary.session_id.clone(),
            plan_digest: summary.plan_digest.clone(),
            run_id: summary.run_id.clone(),
            kind: summary.kind,
            recorded_at: summary.recorded_at.clone(),
            original_revision: summary.original_revision.clone(),
            browsing_revision: summary.browsing_revision.clone(),
            refusal_code: summary.refusal_code,
        }],
    }
}

fn abandoned_and_terminal(studio: &Studio) -> (ConsoleRunSnapshot, ConsoleRunSnapshot) {
    let store = &studio.persistence;
    let owner = store.claim_console_journal_owner().unwrap();
    let pending = store
        .console_prepare_run(&owner, empty_terminal().metadata)
        .unwrap()
        .run;
    let pending = store
        .console_step_intent(&owner, &pending.metadata.run_id, pending.revision, 0, 2)
        .unwrap();
    let settled = store
        .console_prepare_run(&owner, empty_terminal().metadata)
        .unwrap()
        .run;
    let settled = store
        .console_observe(
            &owner,
            &settled.metadata.run_id,
            settled.revision,
            None,
            ConsoleObservations {
                cancellation_observed: true,
                ..Default::default()
            },
        )
        .unwrap();
    let settled = store
        .console_finish(
            &owner,
            &settled.metadata.run_id,
            settled.revision,
            ConsoleStopReason::Cancelled,
            3,
        )
        .unwrap();
    let sealed = store
        .console_seal_publishers(&owner, &settled.metadata.run_id, settled.revision)
        .unwrap();
    assert!(sealed.publisher_sealed);
    assert_eq!(sealed.run, settled);
    (pending, settled)
}

#[tokio::test]
async fn reviewed_console_archive_temporary_lease_preserves_unrelated_pending_originals() {
    let f = Fixture::new();
    let studio = new_studio(&f);
    let (pending, settled) = abandoned_and_terminal(&studio);
    let summary = studio
        .snowflake_console_receipt_summary(&settled.metadata.client_request_id)
        .unwrap()
        .unwrap();
    let valid = archive_request(&summary);
    let mut stale = valid.clone();
    stale.items[0].browsing_revision = "1".into();
    assert_eq!(
        studio
            .set_snowflake_console_receipt_archive(&stale)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio
            .persistence
            .console_run(&pending.metadata.run_id)
            .unwrap(),
        Some(pending.clone())
    );
    assert!(studio.console.journal.lock().owner.is_none());
    studio
        .set_snowflake_console_receipt_archive(&valid)
        .unwrap();
    assert_eq!(
        studio
            .persistence
            .console_run(&pending.metadata.run_id)
            .unwrap(),
        Some(pending.clone())
    );
    assert!(studio.console.journal.lock().owner.is_none());
    // A subsequent explicit recovery lookup still acquires normal ownership
    // and seals the abandoned intent; archive did not install a raw owner.
    let recovered = studio
        .snowflake_console_run(&pending.metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert!(recovered.run.terminal.is_some());
    assert!(matches!(
        recovered.run.steps[0].outcome,
        Some(ConsoleStepOutcome::Unknown(_))
    ));
    assert!(
        !f.controls.join("queries").exists(),
        "no helper was launched"
    );
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_new_normal_lease_discards_unowned_details_and_recovers_pending() {
    let f = Fixture::new();
    let studio = new_studio(&f);
    let (pending, settled) = abandoned_and_terminal(&studio);
    let cached = studio
        .snowflake_console_run(&settled.metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(cached.run, settled);
    assert!(studio.console.journal.lock().owner.is_none());
    assert_eq!(studio.console.journal.lock().details.entries.len(), 1);
    let other = Persistence::open(&f.directory.path().join("workspace")).unwrap();
    let owner = other.claim_console_archive_owner().unwrap();
    assert_eq!(
        other
            .console_observe(
                &owner,
                &settled.metadata.run_id,
                settled.revision,
                None,
                ConsoleObservations {
                    delivery_failed: true,
                    first_error: Some(ConsoleErrorCode::Delivery),
                    ..Default::default()
                },
            )
            .unwrap_err()
            .code,
        ErrorCode::Conflict,
        "sealed details cannot acquire late observations"
    );
    assert_eq!(
        other.console_run(&settled.metadata.run_id).unwrap(),
        Some(settled.clone())
    );
    let updated_pending = other
        .console_observe(
            &owner,
            &pending.metadata.run_id,
            pending.revision,
            None,
            ConsoleObservations {
                delivery_failed: true,
                first_error: Some(ConsoleErrorCode::Delivery),
                ..Default::default()
            },
        )
        .unwrap();
    let held = studio
        .snowflake_console_run(&pending.metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(held.run, updated_pending);
    assert!(held.pending_recording);
    assert!(studio.console.journal.lock().owner.is_none());
    drop(owner);
    // Lookup of another incomplete original is a real None->Some normal lease
    // acquisition, not an artificial cache reset inside this regression.
    let recovered = studio
        .snowflake_console_run(&pending.metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert!(!recovered.pending_recording);
    assert!(recovered.run.terminal.is_some());
    assert!(recovered.run.observations.delivery_failed);
    assert_eq!(
        recovered.run.observations.first_error,
        updated_pending.observations.first_error
    );
    assert!(matches!(
        recovered.run.steps[0].outcome,
        Some(ConsoleStepOutcome::Unknown(_))
    ));
    {
        let journal = studio.console.journal.lock();
        assert!(journal.owner.is_some());
        assert!(
            journal
                .details
                .entries
                .iter()
                .all(|(client, _, _)| client != &settled.metadata.client_request_id),
            "normal lease acquisition invalidates previously unowned details"
        );
    }
    let view = studio
        .snowflake_console_run(&settled.metadata.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(view.run, cached.run);
    assert!(!view.pending_recording);
    assert!(
        !f.controls.join("queries").exists(),
        "no helper was launched"
    );
    studio.console.release_journal();
    assert!(studio.console.journal.lock().details.entries.is_empty());
    studio.shutdown_and_wait().await.unwrap();
}

#[test]
fn reviewed_console_detail_cache_evicts_old_entries_and_rejects_oversized_detail() {
    let mut cache = DetailCache::default();
    let first = empty_terminal();
    let first_client = first.metadata.client_request_id.clone();
    cache.insert(first);
    for _ in 0..DETAIL_CACHE_ENTRIES {
        cache.insert(empty_terminal());
    }
    assert_eq!(cache.entries.len(), DETAIL_CACHE_ENTRIES);
    assert!(cache.get(&first_client).is_none());
    let recent = cache.entries.back().unwrap().1.clone();
    assert_eq!(cache.get(&recent.metadata.client_request_id), Some(recent));
    let prior_bytes = cache.bytes;
    let mut oversized = empty_terminal();
    oversized.metadata.plan_digest = "x".repeat(DETAIL_CACHE_BYTES + 1);
    let oversized_client = oversized.metadata.client_request_id.clone();
    cache.insert(oversized);
    assert!(cache.get(&oversized_client).is_none());
    assert_eq!(cache.entries.len(), DETAIL_CACHE_ENTRIES);
    assert_eq!(cache.bytes, prior_bytes);
    assert!(cache.bytes <= DETAIL_CACHE_BYTES);
}

#[test]
fn reviewed_console_compaction_requires_owner_publisher_and_dirty_completion() {
    let mut journal = Journal::default();
    let run = empty_terminal();
    let id = run.metadata.run_id.clone();
    let client = run.metadata.client_request_id.clone();
    journal.clients.insert(client.clone(), id.clone());
    journal.runs.insert(id.clone(), retained(run.clone()));
    journal.active.insert(id.clone());
    journal.publishers.insert(id.clone());
    journal.compact();
    assert!(journal.runs.contains_key(&id));
    journal.active.remove(&id);
    journal.compact();
    assert!(
        journal.runs.contains_key(&id),
        "Finished may still append observations"
    );
    journal.publishers.remove(&id);
    journal.dirty.insert(id.clone());
    journal.compact();
    assert!(journal.runs.contains_key(&id));
    journal.dirty.remove(&id);
    journal.runs.get_mut(&id).unwrap().publisher_sealed = false;
    journal.compact();
    assert!(
        journal.runs.contains_key(&id),
        "publisher seal remains an authoritative obligation"
    );
    journal.runs.get_mut(&id).unwrap().publisher_sealed = true;
    journal.compact();
    assert!(journal.runs.is_empty());
    assert!(journal.clients.is_empty());
    assert_eq!(journal.details.get(&client), Some(run));
}

#[test]
fn reviewed_console_memory_reservations_and_refusal_outbox_bound_new_identity_admission() {
    let mut journal = Journal::default();
    for _ in 0..HOT_MEMORY_BUDGET / RUN_MEMORY_RESERVATION {
        assert!(journal.can_admit());
        let run = empty_terminal();
        journal
            .runs
            .insert(run.metadata.run_id.clone(), retained(run));
    }
    assert!(!journal.can_admit());
    journal.compact();
    assert!(
        journal.can_admit(),
        "settled compaction releases working reservations"
    );
    for _ in 0..MAX_PENDING_REFUSALS {
        let receipt = ConsoleAdmissionRejection {
            client_request_id: new_id(),
            session_id: new_id(),
            plan_digest: "b".repeat(64),
            code: ErrorCode::ApprovalRequired,
            recorded_at: 1,
        };
        journal
            .rejections
            .insert(receipt.client_request_id.clone(), receipt);
    }
    assert!(
        !journal.can_admit(),
        "no unbounded undurable refusal accumulation"
    );
}

#[tokio::test]
async fn reviewed_console_settled_compaction_preserves_results_dedup_and_restart_lookup() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let first = approved(&studio, &info.id, "SELECT 1");
    let first_id = start_run(&studio, &first, Arc::new(|_| true)).unwrap();
    let original = finished(&studio, &first).await;
    for _ in 0..DETAIL_CACHE_ENTRIES {
        let next = approved(&studio, &info.id, "SELECT 1");
        start_run(&studio, &next, Arc::new(|_| true)).unwrap();
        finished(&studio, &next).await;
    }
    {
        let journal = studio.console.journal.lock();
        assert!(journal.runs.is_empty());
        assert!(journal.clients.is_empty());
        assert!(journal.active.is_empty());
        assert!(journal.publishers.is_empty());
        assert!(journal.dirty.is_empty());
        assert_eq!(journal.details.entries.len(), DETAIL_CACHE_ENTRIES);
        assert!(journal.details.bytes <= DETAIL_CACHE_BYTES);
        assert!(
            !journal
                .details
                .entries
                .iter()
                .any(|(id, _, _)| id == &first.client_request_id)
        );
        assert_eq!(journal.result_runs.len(), DETAIL_CACHE_ENTRIES + 1);
    }
    let before = queries(&f);
    let reloaded = studio
        .snowflake_console_run(&first.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.run, original.run);
    assert_eq!(reloaded.results[0].id, original.results[0].id);
    assert!(studio.console.owns_result(&original.results[0].id));
    assert_eq!(
        start_run(&studio, &first, Arc::new(|_| panic!("dedup has no owner"))).unwrap(),
        first_id
    );
    assert_eq!(queries(&f), before);
    studio.shutdown_and_wait().await.unwrap();
    let restarted = new_studio(&f);
    let restored = restarted
        .snowflake_console_run(&first.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.run, original.run);
    assert!(
        restored.results.is_empty(),
        "restart does not fabricate retained Arrow rows"
    );
    assert_eq!(queries(&f), before);
    restarted.shutdown_and_wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reviewed_console_late_finished_failure_blocks_archive_release_and_compaction_until_recorded()
 {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let start = approved(&studio, &info.id, "SELECT 1");
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let ready = Arc::new(Mutex::new(Some(ready_tx)));
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let id = start_run(
        &studio,
        &start,
        Arc::new(move |event| {
            if matches!(event, QueryEvent::Finished { .. }) {
                ready.lock().take().unwrap().send(()).unwrap();
                release_rx
                    .lock()
                    .recv_timeout(Duration::from_secs(15))
                    .unwrap();
                return false;
            }
            true
        }),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(15), ready_rx)
        .await
        .unwrap()
        .unwrap();
    let original = studio
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert!(original.pending_recording, "held publisher is not settled");
    assert!(!studio.jobs.lock().contains_key(&id));
    {
        let journal = studio.console.journal.lock();
        assert!(!journal.active.contains(&id));
        assert!(journal.publishers.contains(&id));
        assert!(journal.runs.contains_key(&id));
    }
    let summary = studio
        .snowflake_console_receipt_summary(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert!(!summary.archive_eligible);
    assert!(
        summary
            .archive_block_reason
            .as_ref()
            .unwrap()
            .contains("publication")
    );
    let archive = archive_request(&summary);
    assert_eq!(
        studio
            .set_snowflake_console_receipt_archive(&archive)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio
            .release_snowflake_console_results(&start.client_request_id)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        studio
            .snowflake_console_pending()
            .unwrap()
            .client_request_ids,
        vec![start.client_request_id.clone()]
    );
    let db = database(&f);
    db.execute_batch("CREATE TRIGGER deny_late_console_observation BEFORE UPDATE OF observations_json ON snowflake_console_runs_v5 BEGIN SELECT RAISE(FAIL,'fixture late observation failure'); END;").unwrap();
    release_tx.send(()).unwrap();
    let pending = finished(&studio, &start).await;
    assert!(pending.pending_recording);
    assert!(pending.run.observations.delivery_failed);
    assert_eq!(pending.run.steps[0].outcome, original.run.steps[0].outcome);
    assert_eq!(
        pending.run.terminal, original.run.terminal,
        "late observation cannot rewrite original terminal"
    );
    {
        let journal = studio.console.journal.lock();
        assert!(journal.runs.contains_key(&id));
        assert!(journal.dirty.contains(&id));
        assert!(!journal.publishers.contains(&id));
    }
    assert!(studio.snowflake_console_pending().unwrap().recording_hold);
    assert!(studio.shutdown_and_wait().await.is_err());
    db.execute_batch("DROP TRIGGER deny_late_console_observation")
        .unwrap();
    let durable = studio
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert!(!durable.pending_recording);
    assert_eq!(durable.run.steps[0].outcome, original.run.steps[0].outcome);
    assert!(studio.console.journal.lock().runs.is_empty());
    let before_release = studio.results.usage();
    assert!(before_release.0 > 0 && before_release.1 == 1);
    studio
        .release_snowflake_console_results(&start.client_request_id)
        .unwrap();
    assert_eq!(studio.results.usage(), (0, 0));
    assert!(!studio.console.owns_result(&original.results[0].id));
    let released = studio
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(released.run, durable.run);
    assert!(released.results.is_empty());
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_seal_failure_restart_preserves_terminal_history_and_ack_without_sql() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let studio = new_studio(&f);
    let mut saved = profile();
    saved.store_query_text = true;
    let saved = studio.save_profile(saved, None).unwrap();
    let info = studio
        .connect(&saved.id, ConnectionSecrets::default())
        .await
        .unwrap();
    let db = database(&f);
    db.execute_batch("CREATE TRIGGER deny_console_seal BEFORE UPDATE OF publisher_sealed ON console_lifecycle_v5 WHEN NEW.publisher_sealed=1 BEGIN SELECT RAISE(FAIL,'fixture seal failure'); END;").unwrap();
    let start = approved(&studio, &info.id, "SELECT 1");
    let id = start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    let original = finished(&studio, &start).await;
    assert!(original.pending_recording);
    assert_eq!(
        original.run.terminal.as_ref().unwrap().reason,
        ConsoleStopReason::Completed
    );
    assert_eq!(
        original
            .run
            .terminal
            .as_ref()
            .unwrap()
            .accounting
            .acknowledged,
        1
    );
    let history = studio.persistence.history_entry(&id).unwrap().unwrap();
    assert_eq!(history.sql.as_deref(), Some("SELECT 1"));
    assert_eq!(history.status, Some(RunStatus::Completed));
    assert!(history.elapsed_ms.is_some());
    assert!(
        !studio
            .persistence
            .console_lifecycle(&id)
            .unwrap()
            .unwrap()
            .publisher_sealed
    );
    assert!(studio.snowflake_console_pending().unwrap().recording_hold);
    assert!(
        studio
            .set_snowflake_console_receipt_archive(&archive_request(
                &studio
                    .snowflake_console_receipt_summary(&start.client_request_id)
                    .unwrap()
                    .unwrap()
            ))
            .is_err()
    );
    let before = queries(&f);
    assert!(studio.shutdown_and_wait().await.is_err());
    drop(studio);

    let restarted = new_studio(&f);
    let held = restarted
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert!(
        held.pending_recording,
        "failed recovery seal cannot masquerade as settlement"
    );
    assert_eq!(held.run, original.run);
    assert_eq!(
        restarted
            .persistence
            .history_entry(&id)
            .unwrap()
            .unwrap()
            .elapsed_ms,
        history.elapsed_ms
    );
    db.execute_batch("DROP TRIGGER deny_console_seal;").unwrap();
    let recovered = restarted
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert!(!recovered.pending_recording);
    assert_eq!(recovered.run, original.run);
    let recovered_history = restarted.persistence.history_entry(&id).unwrap().unwrap();
    assert_eq!(recovered_history.sql, history.sql);
    assert_eq!(recovered_history.provenance, history.provenance);
    assert_eq!(recovered_history.status, history.status);
    assert_eq!(recovered_history.elapsed_ms, history.elapsed_ms);
    assert!(
        restarted
            .persistence
            .console_lifecycle(&id)
            .unwrap()
            .unwrap()
            .publisher_sealed
    );
    assert_eq!(
        queries(&f),
        before,
        "recovery must not reopen or submit SQL"
    );
    restarted.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_finished_panic_seals_late_observations_without_replacing_completion() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let start = approved(&studio, &info.id, "SELECT 1");
    let first = Arc::new(AtomicBool::new(true));
    let observed = first.clone();
    let id = start_run(
        &studio,
        &start,
        Arc::new(move |event| {
            if matches!(event, QueryEvent::Finished { .. })
                && observed.swap(false, Ordering::AcqRel)
            {
                panic!("fixture final publication panic after atomic completion");
            }
            true
        }),
    )
    .unwrap();
    let view = finished(&studio, &start).await;
    assert!(!view.pending_recording);
    assert_eq!(
        view.run.terminal.as_ref().unwrap().reason,
        ConsoleStopReason::Completed
    );
    assert_eq!(
        view.run.terminal.as_ref().unwrap().accounting.acknowledged,
        1
    );
    assert!(view.run.observations.delivery_failed);
    assert!(view.run.observations.retired);
    let lifecycle = studio.persistence.console_lifecycle(&id).unwrap().unwrap();
    assert!(lifecycle.publisher_sealed);
    let completion = lifecycle.completion.unwrap();
    assert_eq!(completion.provenance, HistoryProvenance::Recorded);
    assert_eq!(completion.status, Some(RunStatus::Completed));
    let history = studio.persistence.history_entry(&id).unwrap().unwrap();
    assert_eq!(history.status, completion.status);
    assert_eq!(history.elapsed_ms, completion.elapsed_ms);
    assert!(history.elapsed_ms.is_some());
    assert_eq!(&queries(&f)[2..], ["SELECT 1"]);
    studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn reviewed_console_paged_archive_is_sql_free_and_keeps_rows_until_explicit_release() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let f = Fixture::new();
    let (studio, info) = connected(&f).await;
    let start = approved(&studio, &info.id, "SELECT 1");
    start_run(&studio, &start, Arc::new(|_| true)).unwrap();
    let original = finished(&studio, &start).await;
    let before = queries(&f);
    let summary = studio
        .snowflake_console_receipt_summary(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert!(summary.archive_eligible);
    let request = archive_request(&summary);
    let archived = studio
        .set_snowflake_console_receipt_archive(&request)
        .unwrap();
    assert!(archived.receipts[0].archived);
    assert_eq!(studio.results.usage().1, 1);
    let view = studio
        .snowflake_console_run(&start.client_request_id)
        .unwrap()
        .unwrap();
    assert_eq!(view.run, original.run);
    assert_eq!(view.results[0].id, original.results[0].id);
    assert_eq!(
        studio
            .set_snowflake_console_receipt_archive(&request)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let page = studio
        .snowflake_console_receipts(&SnowflakeConsoleReceiptPageRequest {
            filter: SnowflakeConsoleReceiptFilter {
                visibility: SnowflakeConsoleReceiptVisibility::Archived,
                kind: None,
                connection_id: Some(info.connection_id.clone()),
            },
            cursor: None,
            limit: 1,
        })
        .unwrap();
    assert_eq!(page.receipts.len(), 1);
    assert_eq!(page.receipts[0].client_request_id, start.client_request_id);
    assert!(page.next_cursor.is_none());
    assert!(
        studio
            .snowflake_console_pending()
            .unwrap()
            .client_request_ids
            .is_empty()
    );
    let storage = studio.snowflake_console_storage().unwrap();
    assert!(
        !storage.retention_enabled,
        "lifetime count gates remain enabled"
    );
    assert_eq!(storage.positive_count, "1");
    studio.disconnect_and_wait(&info.id).await.unwrap();
    assert!(studio.console.owns_result(&original.results[0].id));
    studio
        .release_snowflake_console_results(&start.client_request_id)
        .unwrap();
    assert_eq!(studio.results.usage(), (0, 0));
    assert_eq!(
        studio
            .snowflake_console_run(&start.client_request_id)
            .unwrap()
            .unwrap()
            .run,
        original.run
    );
    assert_eq!(queries(&f), before);
    studio.shutdown_and_wait().await.unwrap();
}
