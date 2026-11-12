use super::*;

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
    connection
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
            "UPDATE snowflake_console_steps SET ordinal=1 WHERE run_id=?1",
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
            "UPDATE snowflake_console_steps SET ordinal=0,outcome_json=?1 WHERE run_id=?2",
            params![
                r#"{"kind":"unknown","evidence":{"statement":null,"sql":"must-not-load"}}"#,
                id
            ],
        )
        .unwrap();
    assert!(validate_records(&store.connection.lock()).is_err());
    store.connection.lock().execute("UPDATE snowflake_console_steps SET outcome_json=NULL,observations_json=?1 WHERE run_id=?2", params![" ".repeat(MAX_JSON_BYTES + 1), id]).unwrap();
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
            "SELECT count(*) FROM snowflake_console_runs WHERE client_request_id IN (?1,?2)",
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
    connection.execute("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<10000) INSERT INTO snowflake_console_runs(run_id,client_request_id,session_id,connection_id,plan_digest,step_count,created_at,revision,state,observations_json) SELECT printf('%08x-0000-4000-8000-000000000000',n),printf('%08x-0000-4000-8000-000000000001',n),?1,?2,?3,1,1000,0,'prepared',?4 FROM numbers",
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
        run_ids(&store.connection.lock(), false).unwrap().len(),
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
    assert_eq!(run_ids(&backup, false).unwrap().len(), MAX_RUNS as usize);
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
                "UPDATE snowflake_console_runs SET terminal_json=?1 WHERE run_id=?2",
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
            "UPDATE snowflake_console_runs SET terminal_json=?1 WHERE run_id=?2",
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
                    &format!("UPDATE snowflake_console_runs SET {column}=-1 WHERE run_id=?1"),
                    [&next.metadata.run_id],
                )
                .unwrap();
            assert!(read_run(&connection, &next.metadata.run_id).is_err());
            connection
                .execute(
                    &format!("UPDATE snowflake_console_runs SET {column}=?1 WHERE run_id=?2"),
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
                "UPDATE snowflake_console_steps SET intended_at=-1 WHERE run_id=?1",
                [&start.metadata.run_id],
            )
            .unwrap();
        assert!(read_run(&connection, &start.metadata.run_id).is_err());
        connection
            .execute(
                "UPDATE snowflake_console_steps SET intended_at=?1 WHERE run_id=?2",
                params![i64::MAX, start.metadata.run_id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE snowflake_console_runs SET revision=?1 WHERE run_id=?2",
                params![i64::MAX, next.metadata.run_id],
            )
            .unwrap();
        assert!(read_run(&connection, &next.metadata.run_id).is_err());
        connection
            .execute(
                "UPDATE snowflake_console_runs SET revision=?1 WHERE run_id=?2",
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
