use super::super::history_owner::AdmissionGate;
use super::*;

fn profile() -> ConnectionProfile {
    serde_json::from_value(serde_json::json!({"id":"00000000-0000-4000-8000-000000000007","name":"Original outcome fixture","groupId":"local","engine":"postgres","host":"127.0.0.1","port":5432,"database":"fixture","username":"fixture","environment":"local","safeMode":"readOnly","tlsMode":"disable","caCertPath":null,"ssh":null,"rememberPassword":false,"storeQueryText":true})).unwrap()
}
fn setup() -> (tempfile::TempDir, Persistence, HistoryOwner) {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    store.save_profile(&profile()).unwrap();
    let owner = store.claim_history_owner().unwrap();
    (root, store, owner)
}
fn seed() -> HistoryEntry {
    HistoryEntry {
        id: new_id(),
        connection_id: profile().id,
        connection_name: profile().name,
        started_at: now_ms(),
        sql: Some("SELECT 'keep only the admission SQL'".into()),
        statement_count: 1,
        provenance: HistoryProvenance::Admitted,
        status: None,
        elapsed_ms: None,
        execution_summary: None,
        snowflake_dml: None,
        snowflake_ddl: None,
    }
}
fn descriptor(s: &HistoryEntry) -> OutcomeDescriptor {
    OutcomeDescriptor {
        operation_id: s.id.clone(),
        connection_id: s.connection_id.clone(),
        session_id: new_id(),
        started_at: s.started_at,
        digest: "a".repeat(64),
        kind: OutcomeKind::Execute { planned: 1 },
    }
}
fn completion() -> HistoryCompletion {
    HistoryCompletion {
        provenance: HistoryProvenance::Recorded,
        status: Some(RunStatus::Completed),
        elapsed_ms: Some(37),
        execution_summary: Some(ExecutionSummary {
            planned: 1,
            attempted: 1,
            completed: 1,
            not_attempted: 0,
            stop_reason: None,
        }),
        snowflake_dml: None,
        snowflake_ddl: None,
    }
}
fn boundary(action: ExplicitTransactionAction) -> OutcomeDescriptor {
    OutcomeDescriptor {
        operation_id: new_id(),
        connection_id: profile().id,
        session_id: new_id(),
        started_at: now_ms(),
        digest: "b".repeat(64),
        kind: OutcomeKind::Boundary { action },
    }
}
fn state(store: &Persistence, id: &str) -> String {
    store
        .connection
        .lock()
        .query_row(
            "SELECT phase FROM outcome_operations_v7 WHERE operation_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn outcomes_capture_survives_history_update_failure_and_dead_owner_recovery() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let seed = seed();
    let d = descriptor(&seed);
    let r = store
        .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
        .unwrap();
    let original = OriginalOutcome::history(&d, &completion(), now_ms()).unwrap();
    store
        .capture_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    store.connection.lock().execute_batch("CREATE TEMP TRIGGER fail_outcome_history BEFORE UPDATE ON history_v8 BEGIN SELECT RAISE(ABORT,'fixture history failure'); END;").unwrap();
    assert!(
        store
            .settle_original_outcome(&owner, &gate, &r, &original)
            .is_err()
    );
    assert_eq!(state(&store, &seed.id), "captured");
    assert_eq!(store.audit_log().unwrap().len(), 1);
    assert_eq!(
        store.history_entry(&seed.id).unwrap().unwrap().provenance,
        HistoryProvenance::Admitted
    );
    store
        .connection
        .lock()
        .execute_batch("DROP TRIGGER fail_outcome_history")
        .unwrap();
    drop(gate);
    drop(owner);
    let _observer = store.claim_history_owner().unwrap();
    let history = store.history_entry(&seed.id).unwrap().unwrap();
    assert_eq!(history.provenance, HistoryProvenance::Recorded);
    assert_eq!(history.status, Some(RunStatus::Completed));
    assert_eq!(history.elapsed_ms, Some(37));
    assert_eq!(state(&store, &seed.id), "settled");
    assert_eq!(store.audit_log().unwrap().len(), 2);
    assert_eq!(history.sql, seed.sql);
}

#[test]
fn outcomes_audit_and_release_faults_rollback_projection_but_not_capture() {
    for fault in [
        "CREATE TEMP TRIGGER fail_projection BEFORE INSERT ON audit_v8 WHEN NEW.outcome!='started' BEGIN SELECT RAISE(ABORT,'audit fault'); END;",
        "CREATE TEMP TRIGGER fail_projection BEFORE DELETE ON history_owners_v6 BEGIN SELECT RAISE(ABORT,'release fault'); END;",
    ] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let seed = seed();
        let d = descriptor(&seed);
        let r = store
            .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
            .unwrap();
        let original = OriginalOutcome::history(&d, &completion(), now_ms()).unwrap();
        store
            .capture_original_outcome(&owner, &gate, &r, &original)
            .unwrap();
        store.connection.lock().execute_batch(fault).unwrap();
        assert!(
            store
                .settle_original_outcome(&owner, &gate, &r, &original)
                .is_err()
        );
        assert_eq!(state(&store, &seed.id), "captured");
        assert_eq!(
            store.history_entry(&seed.id).unwrap().unwrap().provenance,
            HistoryProvenance::Admitted
        );
        assert_eq!(store.audit_log().unwrap().len(), 1);
        store
            .connection
            .lock()
            .execute_batch("DROP TRIGGER fail_projection")
            .unwrap();
        store
            .settle_original_outcome(&owner, &gate, &r, &original)
            .unwrap();
        store
            .settle_original_outcome(&owner, &gate, &r, &original)
            .unwrap();
        store
            .capture_original_outcome(&owner, &gate, &r, &original)
            .unwrap();
        assert_eq!(store.audit_log().unwrap().len(), 2);
        assert_eq!(state(&store, &seed.id), "settled");
        let changed = OriginalOutcome::history(&d, &completion(), now_ms() + 1).unwrap();
        assert!(
            store
                .capture_original_outcome(&owner, &gate, &r, &changed)
                .is_err()
        );
    }
}

#[test]
fn outcomes_started_audit_reservation_and_history_are_one_transaction() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let seed = seed();
    let d = descriptor(&seed);
    store.connection.lock().execute_batch("CREATE TEMP TRIGGER fail_started BEFORE INSERT ON audit_v8 BEGIN SELECT RAISE(ABORT,'started unavailable'); END;").unwrap();
    assert!(
        store
            .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
            .is_err()
    );
    assert!(store.history_entry(&seed.id).unwrap().is_none());
    assert!(
        read_row(&store.connection.lock(), &seed.id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .connection
            .lock()
            .query_row("SELECT count(*) FROM history_owners_v6", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn outcomes_missing_reserved_or_captured_records_never_become_new_admission() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let seed = seed();
    let d = descriptor(&seed);
    let r = store
        .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
        .unwrap();
    let original = OriginalOutcome::history(&d, &completion(), now_ms()).unwrap();
    store
        .connection
        .lock()
        .execute(
            "DELETE FROM outcome_operations_v7 WHERE operation_id=?1",
            [&seed.id],
        )
        .unwrap();
    assert!(
        store
            .capture_original_outcome(&owner, &gate, &r, &original)
            .is_err()
    );
    assert!(
        store
            .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
            .is_err()
    );
    assert!(
        store
            .make_volatile_rollback_token(&owner, &gate, &d)
            .is_err()
    );
    let d = boundary(ExplicitTransactionAction::Rollback);
    let r = store
        .reserve_owned_operation(&owner, &gate, None, &d, OutcomePool::EmergencyRollback)
        .unwrap();
    let original = OriginalOutcome::boundary(
        &d,
        &BoundaryEvidence {
            action: ExplicitTransactionAction::Rollback,
            boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
            retired: false,
        },
        now_ms(),
    )
    .unwrap();
    store
        .capture_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    store
        .connection
        .lock()
        .execute(
            "UPDATE outcome_operations_v7 SET payload_json=NULL WHERE operation_id=?1",
            [&d.operation_id],
        )
        .unwrap();
    assert!(
        store
            .settle_original_outcome(&owner, &gate, &r, &original)
            .is_err()
    );
    assert!(validate_records(&store.connection.lock()).is_err());
}

#[test]
fn outcomes_boundary_ack_remains_readable_after_payload_settlement_and_restart() {
    let (root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = boundary(ExplicitTransactionAction::Commit);
    let r = store
        .reserve_owned_operation(&owner, &gate, None, &d, OutcomePool::Ordinary)
        .unwrap();
    let b = BoundaryEvidence {
        action: ExplicitTransactionAction::Commit,
        boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
        retired: true,
    };
    let original = OriginalOutcome::boundary(&d, &b, d.started_at + 1).unwrap();
    store
        .capture_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    store
        .settle_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    let row = read_row(&store.connection.lock(), &d.operation_id)
        .unwrap()
        .unwrap();
    assert!(row.payload.is_none());
    assert!(row.descriptor.is_none());
    assert_eq!(phase(&row).unwrap(), OutcomePhase::Settled);
    drop(gate);
    drop(owner);
    drop(store);
    let store = Persistence::open(root.path()).unwrap();
    let evidence = store
        .settled_boundary_evidence(&d.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(evidence.action, b.action);
    assert_eq!(evidence.boundary_outcome, b.boundary_outcome);
    assert!(evidence.retired);
    assert_eq!(evidence.connection_id, d.connection_id);
    assert_eq!(evidence.session_id, d.session_id);
    assert_eq!(evidence.finished_at, d.started_at + 1);
    assert!(store.history("").unwrap().is_empty());
    assert_eq!(store.audit_log().unwrap().len(), 2);
}

#[test]
fn outcomes_no_original_abandon_is_negative_witness_without_finished_audit() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let seed = seed();
    let d = descriptor(&seed);
    let r = store
        .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
        .unwrap();
    store.abandon_reserved_operation(&owner, &gate, &r).unwrap();
    store.abandon_reserved_operation(&owner, &gate, &r).unwrap();
    assert_eq!(store.audit_log().unwrap().len(), 1);
    let h = store.history_entry(&seed.id).unwrap().unwrap();
    assert_eq!(h.provenance, HistoryProvenance::Interrupted);
    assert!(h.status.is_none());
    let row = read_row(&store.connection.lock(), &seed.id)
        .unwrap()
        .unwrap();
    assert_eq!(row.terminal.as_deref(), Some("noOriginal"));
    assert!(row.capture_digest.is_none());
    assert!(
        store
            .capture_original_outcome(
                &owner,
                &gate,
                &r,
                &OriginalOutcome::history(&d, &completion(), now_ms()).unwrap()
            )
            .is_err()
    );
}

#[test]
fn outcomes_volatile_rollback_gate_survives_recording_failure_and_only_final_drop_unlocks() {
    let (_root, store, owner) = setup();
    let first = store.operation_gate(&owner).unwrap();
    let second = store.operation_gate(&owner).unwrap();
    let d = boundary(ExplicitTransactionAction::Rollback);
    let token = store
        .make_volatile_rollback_token(&owner, &first, &d)
        .unwrap();
    store
        .connection
        .lock()
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    assert!(
        store
            .promote_volatile_rollback(&owner, &first, &token)
            .is_err()
    );
    drop(first);
    assert!(AdmissionGate::claim(&store.root, true).is_err());
    // The preopened OS facility is independent of SQLite write availability.
    let third = store.operation_gate(&owner).unwrap();
    drop(second);
    assert!(AdmissionGate::claim(&store.root, true).is_err());
    store
        .connection
        .lock()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    let r = store
        .promote_volatile_rollback(&owner, &third, &token)
        .unwrap();
    let original = OriginalOutcome::boundary(
        &d,
        &BoundaryEvidence {
            action: ExplicitTransactionAction::Rollback,
            boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
            retired: false,
        },
        now_ms(),
    )
    .unwrap();
    store
        .capture_original_outcome(&owner, &third, &r, &original)
        .unwrap();
    store
        .settle_original_outcome(&owner, &third, &r, &original)
        .unwrap();
    drop(third);
    assert!(AdmissionGate::claim(&store.root, true).is_ok());
}

#[test]
fn outcomes_volatile_rollback_captures_ack_when_started_audit_is_unwritable() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = boundary(ExplicitTransactionAction::Rollback);
    let token = store
        .make_volatile_rollback_token(&owner, &gate, &d)
        .unwrap();
    store.connection.lock().execute_batch("CREATE TEMP TRIGGER reject_started BEFORE INSERT ON audit_v8 WHEN NEW.outcome='started' BEGIN SELECT RAISE(ABORT,'started audit unavailable'); END;").unwrap();
    assert!(
        store
            .reserve_owned_operation(&owner, &gate, None, &d, OutcomePool::EmergencyRollback)
            .is_err()
    );
    let r = store
        .promote_volatile_rollback(&owner, &gate, &token)
        .unwrap();
    let original = OriginalOutcome::boundary(
        &d,
        &BoundaryEvidence {
            action: ExplicitTransactionAction::Rollback,
            boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
            retired: false,
        },
        now_ms(),
    )
    .unwrap();
    store
        .capture_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    assert!(
        store
            .settle_original_outcome(&owner, &gate, &r, &original)
            .is_err()
    );
    assert_eq!(state(&store, &d.operation_id), "captured");
    assert!(store.audit_log().unwrap().is_empty());
    store
        .connection
        .lock()
        .execute_batch("DROP TRIGGER reject_started")
        .unwrap();
    store
        .settle_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    assert_eq!(store.audit_log().unwrap().len(), 2);
    assert_eq!(
        store
            .settled_boundary_evidence(&d.operation_id)
            .unwrap()
            .unwrap()
            .boundary_outcome,
        TransactionBoundaryOutcome::Acknowledged
    );
}

#[test]
fn outcomes_recursive_whitelist_counts_and_buffer_ownership_are_enforced() {
    let seed = seed();
    let d = descriptor(&seed);
    let original = OriginalOutcome::history_in(
        &d,
        &completion(),
        now_ms(),
        Vec::with_capacity(MAX_OUTCOME_BYTES),
    )
    .unwrap();
    let copied = original.clone();
    let original = original.try_reclaim_buffer().unwrap_err();
    drop(copied);
    let buffer = original.try_reclaim_buffer().unwrap();
    assert!(buffer.is_empty());
    assert_eq!(buffer.capacity(), MAX_OUTCOME_BYTES);
    let mut c = completion();
    c.execution_summary.as_mut().unwrap().completed = 2;
    assert!(OriginalOutcome::history(&d, &c, now_ms()).is_err());
    let original = OriginalOutcome::history(&d, &completion(), now_ms()).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(original.as_bytes()).unwrap();
    value["completion"]["executionSummary"]["sql"] = serde_json::json!("secret SQL");
    assert!(decode_payload(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(
        OriginalOutcome::history_in(&d, &completion(), now_ms(), Vec::with_capacity(1)).is_err()
    );
    let mut d = boundary(ExplicitTransactionAction::Rollback);
    d.kind = OutcomeKind::SnowflakeDdl {
        kind: SnowflakeDdlKind::CreateTable,
        qualified_target: "x".repeat(4097),
    };
    assert!(d.validate().is_err());
}

#[test]
fn outcomes_closed_decoder_rejects_large_arrays_and_duplicate_or_unknown_nested_fields() {
    // These fit the encoded budget but must fail at their first wrong type or
    // field, without building a generic JSON array/tree as replay scratch.
    let wide_array = format!("[{}0]", "0,".repeat(15_000));
    assert!(wide_array.len() < MAX_OUTCOME_BYTES);
    assert!(decode_payload(wide_array.as_bytes()).is_err());

    let seed = seed();
    let d = descriptor(&seed);
    let original = OriginalOutcome::history(&d, &completion(), now_ms()).unwrap();
    let text = std::str::from_utf8(original.as_bytes()).unwrap();
    let kind_array = text.replacen("\"planned\":1", &format!("\"planned\":{wide_array}"), 1);
    assert!(kind_array.len() <= MAX_OUTCOME_BYTES);
    assert!(decode_payload(kind_array.as_bytes()).is_err());
    let nested_array = text.replace(
        "\"executionSummary\":{",
        &format!("\"unknown\":{wide_array},\"executionSummary\":{{"),
    );
    assert!(nested_array.len() <= MAX_OUTCOME_BYTES);
    assert!(decode_payload(nested_array.as_bytes()).is_err());
    let duplicate = text.replace("\"planned\":1", "\"planned\":1,\"planned\":1");
    assert_ne!(duplicate, text);
    assert!(decode_payload(duplicate.as_bytes()).is_err());

    let (_root, store, _owner) = setup();
    let c = store.connection.lock();
    c.execute(
        "INSERT INTO audit_v8 VALUES('bad-timestamp',-1,'c','execute','d','started')",
        [],
    )
    .unwrap();
    assert!(read_audit(&c, "bad-timestamp").is_err());
}

#[test]
fn outcomes_sql_projections_cap_malformed_metadata_and_preserve_phase_nulls() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = boundary(ExplicitTransactionAction::Commit);
    let r = store
        .reserve_owned_operation(&owner, &gate, None, &d, OutcomePool::Ordinary)
        .unwrap();
    let original = OriginalOutcome::boundary(
        &d,
        &BoundaryEvidence {
            action: ExplicitTransactionAction::Commit,
            boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
            retired: false,
        },
        now_ms(),
    )
    .unwrap();
    {
        let c = store.connection.lock();
        let row = read_row(&c, &d.operation_id).unwrap().unwrap();
        assert!(row.payload.is_none());
        assert_eq!(phase(&row).unwrap(), OutcomePhase::Reserved);
        c.execute("UPDATE outcome_operations_v7 SET owner_nonce=replace(hex(zeroblob(65536)),'0','x') WHERE operation_id=?1", [&d.operation_id]).unwrap();
        let capped = read_row(&c, &d.operation_id).unwrap().unwrap();
        assert!(capped.owner.is_empty());
        assert!(phase(&capped).is_err());
        assert!(validate_records(&c).is_err());
        c.execute(
            "UPDATE outcome_operations_v7 SET owner_nonce=?1 WHERE operation_id=?2",
            params![owner.nonce, d.operation_id],
        )
        .unwrap();
    }
    store
        .capture_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    store
        .settle_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    let c = store.connection.lock();
    let row = read_row(&c, &d.operation_id).unwrap().unwrap();
    assert!(row.descriptor.is_none() && row.payload.is_none());
    assert_eq!(phase(&row).unwrap(), OutcomePhase::Settled);
    let start_id = format!("{}:started", d.operation_id);
    c.execute(
        "UPDATE audit_v8 SET connection_id=hex(zeroblob(65536)) WHERE id=?1",
        [&start_id],
    )
    .unwrap();
    assert!(read_audit(&c, &start_id).unwrap().connection_id.is_empty());
    assert!(boundary_evidence(&c, &row).is_err());
    assert!(audit_once(&c, &started(&d)).is_err());
}

#[test]
fn outcomes_typed_evidence_preserves_ack_and_exact_decimal_bounds_without_sdk_state() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    for ddl in [false, true] {
        let mut seed = seed();
        seed.sql = None;
        let mut d = descriptor(&seed);
        let mut c = completion();
        c.execution_summary = None;
        if ddl {
            d.kind = OutcomeKind::SnowflakeDdl {
                kind: SnowflakeDdlKind::CreateTable,
                qualified_target: "\"DB\".\"PUBLIC\".\"T\"".into(),
            };
            c.snowflake_ddl = Some(SnowflakeDdlEvidence {
                kind: SnowflakeDdlKind::CreateTable,
                qualified_target: "\"DB\".\"PUBLIC\".\"T\"".into(),
                statement_outcome: SnowflakeDdlOutcome::Succeeded,
                summary_status: SnowflakeDdlSummaryStatus::Acknowledged,
                catalog_reconciliation_required: true,
                retired: true,
            });
        } else {
            d.kind = OutcomeKind::SnowflakeDml {
                kind: SnowflakeDmlKind::Update,
            };
            c.snowflake_dml = Some(SnowflakeDmlEvidence {
                kind: SnowflakeDmlKind::Update,
                statement_outcome: SnowflakeDmlOutcome::Succeeded,
                summary: SnowflakeDmlSummary {
                    status: SnowflakeDmlSummaryStatus::Validated,
                    affected_rows: Some("9".repeat(38)),
                    multi_joined_rows: Some("0".into()),
                },
                retired: false,
            });
        }
        let r = store
            .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
            .unwrap();
        let original = OriginalOutcome::history(&d, &c, now_ms()).unwrap();
        store
            .capture_original_outcome(&owner, &gate, &r, &original)
            .unwrap();
        store
            .settle_original_outcome(&owner, &gate, &r, &original)
            .unwrap();
        let entry = store.history_entry(&seed.id).unwrap().unwrap();
        assert!(entry.sql.is_none());
        assert!(HistoryCompletion::from_entry(&entry).matches(&c));
        if !ddl {
            c.snowflake_dml.as_mut().unwrap().summary.affected_rows = Some("9".repeat(39));
            assert!(OriginalOutcome::history(&d, &c, now_ms()).is_err());
        }
    }
}

#[test]
fn outcomes_maximum_legal_ddl_identifiers_and_escaped_connection_fit_exact_capture() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let mut profile = profile();
    profile.id = "\u{1}".repeat(256);
    store.save_profile(&profile).unwrap();
    let mixed = (0..255)
        .map(|index| if index % 2 == 0 { '"' } else { '😀' })
        .collect::<String>();
    for identifier in ["\"".repeat(255), "😀".repeat(255), mixed] {
        assert_eq!(identifier.chars().count(), 255);
        let quoted = format!("\"{}\"", identifier.replace('"', "\"\""));
        let target = format!("{quoted}.{quoted}.{quoted}");
        let mut seed = seed();
        seed.connection_id = profile.id.clone();
        seed.sql = None;
        let mut descriptor = descriptor(&seed);
        descriptor.kind = OutcomeKind::SnowflakeDdl {
            kind: SnowflakeDdlKind::CreateTable,
            qualified_target: target.clone(),
        };
        let mut complete = completion();
        complete.execution_summary = None;
        complete.snowflake_ddl = Some(SnowflakeDdlEvidence {
            kind: SnowflakeDdlKind::CreateTable,
            qualified_target: target.clone(),
            statement_outcome: SnowflakeDdlOutcome::Succeeded,
            summary_status: SnowflakeDdlSummaryStatus::Acknowledged,
            catalog_reconciliation_required: true,
            retired: false,
        });
        let reserved = store
            .reserve_owned_operation(
                &owner,
                &gate,
                Some(&seed),
                &descriptor,
                OutcomePool::Ordinary,
            )
            .unwrap();
        let original = OriginalOutcome::history_in(
            &descriptor,
            &complete,
            descriptor.started_at + 1,
            Vec::with_capacity(MAX_OUTCOME_BYTES),
        )
        .unwrap();
        assert!(
            original.as_bytes().len() + encoded(&reserved.data).unwrap().len() <= MAX_OUTCOME_BYTES
        );
        assert!(
            original
                .payload()
                .unwrap()
                .completion
                .unwrap()
                .matches(&complete)
        );
        store
            .capture_original_outcome(&owner, &gate, &reserved, &original)
            .unwrap();
        {
            let connection = store.connection.lock();
            let row = read_row(&connection, &descriptor.operation_id)
                .unwrap()
                .unwrap();
            assert_eq!(row.payload.unwrap().as_bytes(), original.as_bytes());
        }
        store
            .settle_original_outcome(&owner, &gate, &reserved, &original)
            .unwrap();
        let entry = store.history_entry(&seed.id).unwrap().unwrap();
        assert_eq!(entry.connection_id, profile.id);
        assert!(HistoryCompletion::from_entry(&entry).matches(&complete));
        assert_eq!(entry.snowflake_ddl.unwrap().qualified_target, target);
    }
    assert_eq!(store.audit_log().unwrap().len(), 6);
}

#[test]
fn outcomes_settled_marker_ceiling_is_reserved_before_any_new_admission() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    store.connection.lock().execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<65536) INSERT INTO outcome_operations_v7(operation_id,owner_nonce,pool,phase,reservation_digest,terminal_kind) SELECT printf('%08x-0000-4000-8000-%012x',x,x),?1,'ordinary','settled',?2,'noOriginal' FROM n",params![owner.nonce,"0".repeat(64)]).unwrap();
    let d = boundary(ExplicitTransactionAction::Commit);
    assert_eq!(
        store
            .reserve_owned_operation(&owner, &gate, None, &d, OutcomePool::Ordinary)
            .unwrap_err()
            .code,
        ErrorCode::ResultLimitReached
    );
    assert!(store.audit_log().unwrap().is_empty());
    assert_eq!(
        store
            .connection
            .lock()
            .query_row("SELECT count(*) FROM outcome_operations_v7", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        MAX_MARKERS
    );
}

#[test]
fn outcomes_owner_and_root_quotas_keep_rollback_capacity_separate() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let mut owners = Vec::new();
    for _ in 0..32 {
        let d = boundary(ExplicitTransactionAction::Commit);
        store
            .reserve_owned_operation(&owner, &gate, None, &d, OutcomePool::Ordinary)
            .unwrap();
    }
    assert!(
        store
            .reserve_owned_operation(
                &owner,
                &gate,
                None,
                &boundary(ExplicitTransactionAction::Commit),
                OutcomePool::Ordinary
            )
            .is_err()
    );
    let rollback = boundary(ExplicitTransactionAction::Rollback);
    assert!(
        store
            .reserve_owned_operation(
                &owner,
                &gate,
                None,
                &rollback,
                OutcomePool::EmergencyRollback
            )
            .is_ok()
    );
    for _ in 0..3 {
        let other = store.claim_history_owner().unwrap();
        let other_gate = store.operation_gate(&other).unwrap();
        for _ in 0..32 {
            store
                .reserve_owned_operation(
                    &other,
                    &other_gate,
                    None,
                    &boundary(ExplicitTransactionAction::Commit),
                    OutcomePool::Ordinary,
                )
                .unwrap();
        }
        owners.push((other, other_gate));
    }
    let other = store.claim_history_owner().unwrap();
    let other_gate = store.operation_gate(&other).unwrap();
    assert!(
        store
            .reserve_owned_operation(
                &other,
                &other_gate,
                None,
                &boundary(ExplicitTransactionAction::Commit),
                OutcomePool::Ordinary
            )
            .is_err()
    );
    assert!(
        store
            .reserve_owned_operation(
                &other,
                &other_gate,
                None,
                &boundary(ExplicitTransactionAction::Rollback),
                OutcomePool::EmergencyRollback
            )
            .is_ok()
    );
}

#[test]
fn outcomes_dead_capture_honors_clear_optout_delete_and_missing_visible_rows() {
    for privacy in ["clear", "optout", "delete", "missing"] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let seed = seed();
        let d = descriptor(&seed);
        let r = store
            .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
            .unwrap();
        let original = OriginalOutcome::history(&d, &completion(), now_ms()).unwrap();
        store
            .capture_original_outcome(&owner, &gate, &r, &original)
            .unwrap();
        match privacy {
            "clear" => store.clear_history().unwrap(),
            "optout" => {
                let mut p = profile();
                p.store_query_text = false;
                store.save_profile(&p).unwrap();
            }
            "delete" => store.delete_profile(&seed.connection_id).unwrap(),
            _ => {
                store
                    .connection
                    .lock()
                    .execute("DELETE FROM history_v8 WHERE id=?1", [&seed.id])
                    .unwrap();
            }
        }
        drop(gate);
        drop(owner);
        let _next = store.claim_history_owner().unwrap();
        assert_eq!(state(&store, &seed.id), "settled");
        let entry = store.history_entry(&seed.id).unwrap();
        if ["clear", "missing"].contains(&privacy) {
            assert!(entry.is_none());
        } else {
            let entry = entry.unwrap();
            assert!(entry.sql.is_none());
            assert_eq!(entry.status, Some(RunStatus::Completed));
        }
    }
}

#[test]
fn outcomes_restore_reconciles_only_current_originals_before_replacement() {
    let (_root, store, owner) = setup();
    let backup = store.create_metadata_backup().unwrap();
    let gate = store.operation_gate(&owner).unwrap();
    let seed = seed();
    let d = descriptor(&seed);
    let r = store
        .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
        .unwrap();
    let original = OriginalOutcome::history(&d, &completion(), now_ms()).unwrap();
    store
        .capture_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    assert!(
        store
            .restore_metadata_backup(&backup.id, &backup.digest)
            .is_err()
    );
    drop(gate);
    drop(owner);
    store
        .restore_metadata_backup(&backup.id, &backup.digest)
        .unwrap();
    assert_eq!(state(&store, &seed.id), "settled");
    assert_eq!(
        store.history_entry(&seed.id).unwrap().unwrap().status,
        Some(RunStatus::Completed)
    );
    assert_eq!(store.audit_log().unwrap().len(), 2);
}

#[test]
fn outcomes_imported_capture_and_settled_audits_remain_history_without_local_authority() {
    for settled in [false, true] {
        let (_root, store, owner) = setup();
        let mut current = Connection::open_in_memory().unwrap();
        {
            let live = store.connection.lock();
            let copy = rusqlite::backup::Backup::new(&live, &mut current).unwrap();
            assert_eq!(copy.step(-1).unwrap(), rusqlite::backup::StepResult::Done);
        }
        let gate = store.operation_gate(&owner).unwrap();
        let d = boundary(ExplicitTransactionAction::Commit);
        let r = store
            .reserve_owned_operation(&owner, &gate, None, &d, OutcomePool::Ordinary)
            .unwrap();
        let original = OriginalOutcome::boundary(
            &d,
            &BoundaryEvidence {
                action: ExplicitTransactionAction::Commit,
                boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
                retired: false,
            },
            now_ms(),
        )
        .unwrap();
        store
            .capture_original_outcome(&owner, &gate, &r, &original)
            .unwrap();
        if settled {
            store
                .settle_original_outcome(&owner, &gate, &r, &original)
                .unwrap();
        }
        let mut candidate = Connection::open_in_memory().unwrap();
        {
            let live = store.connection.lock();
            let copy = rusqlite::backup::Backup::new(&live, &mut candidate).unwrap();
            assert_eq!(copy.step(-1).unwrap(), rusqlite::backup::StepResult::Done);
        }
        super::super::snowflake_console::merge_for_restore(&current, &mut candidate).unwrap();
        super::super::recovery::validate(&candidate).unwrap();
        assert!(read_row(&candidate, &d.operation_id).unwrap().is_none());
        assert_eq!(
            candidate
                .query_row("SELECT count(*) FROM audit_v8", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            if settled { 2 } else { 1 }
        );
        // Retained historical audit identity still prevents same-ID admission.
        let tx = candidate.transaction().unwrap();
        assert!(reserve(&tx, &owner.nonce, None, &d, OutcomePool::Ordinary, true).is_err());
    }
}

#[test]
fn outcomes_deleted_capture_row_is_detected_before_dead_owner_fallback() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let seed = seed();
    let d = descriptor(&seed);
    let r = store
        .reserve_owned_operation(&owner, &gate, Some(&seed), &d, OutcomePool::Ordinary)
        .unwrap();
    let original = OriginalOutcome::history(&d, &completion(), now_ms()).unwrap();
    store
        .capture_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    store
        .connection
        .lock()
        .execute(
            "DELETE FROM outcome_operations_v7 WHERE operation_id=?1",
            [&seed.id],
        )
        .unwrap();
    drop(gate);
    drop(owner);
    assert!(store.claim_history_owner().is_err());
    assert_eq!(
        store.history_entry(&seed.id).unwrap().unwrap().provenance,
        HistoryProvenance::Admitted
    );
}

#[test]
fn outcomes_dead_replay_batch_commits_at_most_eight_originals() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    for _ in 0..9 {
        let d = boundary(ExplicitTransactionAction::Commit);
        let r = store
            .reserve_owned_operation(&owner, &gate, None, &d, OutcomePool::Ordinary)
            .unwrap();
        let original = OriginalOutcome::boundary(
            &d,
            &BoundaryEvidence {
                action: ExplicitTransactionAction::Commit,
                boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
                retired: false,
            },
            now_ms(),
        )
        .unwrap();
        store
            .capture_original_outcome(&owner, &gate, &r, &original)
            .unwrap();
    }
    let mut c = store.connection.lock();
    let tx = c.transaction().unwrap();
    assert!(recover_batch(&tx, &owner.nonce).unwrap());
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM outcome_operations_v7 WHERE phase='settled'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        8
    );
    tx.commit().unwrap();
    let tx = c.transaction().unwrap();
    assert!(!recover_batch(&tx, &owner.nonce).unwrap());
    tx.commit().unwrap();
}

#[test]
fn outcomes_v7_migration_excludes_idle_v6_nonce_before_any_writer_change() {
    let (root, store, owner) = setup();
    super::super::batch_outcomes::drop_schema(&store.connection.lock());
    store.connection.lock().execute_batch("DROP TABLE outcome_bindings_v7; DROP TABLE outcome_operations_v7; ALTER TABLE profiles_v8 RENAME TO profiles_v6; ALTER TABLE history_v8 RENAME TO history_v6; ALTER TABLE audit_v8 RENAME TO audit_v6; PRAGMA user_version=6;").unwrap();
    assert!(Persistence::open(root.path()).is_err());
    assert_eq!(
        store
            .connection
            .lock()
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        6
    );
    drop(owner);
    drop(store);
    let store = Persistence::open(root.path()).unwrap();
    assert_eq!(
        schema_version(&store.connection.lock()).unwrap(),
        super::super::SCHEMA_VERSION
    );
    assert!(
        store
            .connection
            .lock()
            .prepare("DELETE FROM history_v6")
            .is_err()
    );
}
