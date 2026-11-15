use super::*;
use crate::drivers::snowflake_batch::{Role, StepPhase, UnitKind};
use crate::persistence::{OutcomeDescriptor, OutcomeKind, OutcomePool};
use serde_json::{Value, json};

pub(super) fn setup() -> (tempfile::TempDir, Persistence, Arc<HistoryOwner>) {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let owner = Arc::new(store.claim_history_owner().unwrap());
    (root, store, owner)
}
pub(super) fn descriptor() -> BatchDescriptor {
    BatchDescriptor {
        operation_id: uuid::Uuid::new_v4().to_string(),
        connection_id: "batch-fixture".into(),
        session_id: uuid::Uuid::new_v4().to_string(),
        helper_generation: uuid::Uuid::new_v4().to_string(),
        started_at: 1,
        digest: "a".repeat(64),
    }
}
pub(super) fn next(d: &BatchDescriptor) -> BatchDescriptor {
    let mut d = d.clone();
    d.operation_id = uuid::Uuid::new_v4().to_string();
    d
}
pub(super) fn request(owner: Option<uuid::Uuid>, units: usize) -> BatchRequest {
    BatchRequest::Batch {
        mode: Mode::Manual,
        owner,
        rollback_prior: owner.is_some(),
        units: vec![UnitKind::Update; units],
    }
}
pub(super) fn frame(v: Value) -> Value {
    let mut result = json!({"v":1,"generation":"fixture","id":"fixture"});
    result
        .as_object_mut()
        .unwrap()
        .extend(v.as_object().unwrap().clone());
    result
}
pub(super) fn complete(e: &mut BatchEvidence, role: Role, unit: Option<usize>, affected: &str) {
    let step = e.steps.len() + 1;
    assert!(e.apply_event(&frame(json!({"type":"batchStep","step":step,"role":role,"unit":unit,"phase":StepPhase::Dispatching}))));
    assert!(e.apply_event(&frame(json!({"type":"batchStatement","step":step,"sdkRequestId":uuid::Uuid::new_v4(),"queryId":uuid::Uuid::new_v4()}))));
    assert!(e.apply_event(&frame(json!({"type":"batchStep","step":step,"role":role,"unit":unit,"phase":StepPhase::Acknowledged}))));
    if let Some(u) = unit {
        assert!(e.apply_event(&frame(json!({"type":"batchCount","step":step,"unit":u,"affectedRows":affected,"multiJoinedRows":"0"}))));
    }
    assert!(e.apply_event(&frame(
        json!({"type":"batchStep","step":step,"role":role,"unit":unit,"phase":StepPhase::Drained})
    )));
}
pub(super) fn evidence(
    request: BatchRequest,
    helper: uuid::Uuid,
    result: Outcome,
    prior: u32,
) -> BatchEvidence {
    let mut e = BatchEvidence::new(request);
    if result != Outcome::Rejected {
        match &e.request {
            BatchRequest::Batch { owner, .. } => {
                let fresh = owner.is_none();
                let units = e.request.units().len();
                for role in [Role::Context, Role::Settings] {
                    complete(&mut e, role, None, "1");
                }
                if fresh {
                    for role in [Role::Begin, Role::Transaction] {
                        complete(&mut e, role, None, "1");
                    }
                }
                for unit in 0..units {
                    complete(
                        &mut e,
                        Role::Unit,
                        Some(unit),
                        if result == Outcome::RolledBack && unit + 1 == units {
                            "2"
                        } else {
                            "1"
                        },
                    );
                }
                if result != Outcome::RolledBack {
                    complete(&mut e, Role::Verify, None, "1");
                }
                if result == Outcome::Committed {
                    for role in [Role::Commit, Role::Idle, Role::Disposition] {
                        complete(&mut e, role, None, "1");
                    }
                }
            }
            BatchRequest::Finish {
                action: FinishAction::Commit,
                ..
            } => {
                for role in [
                    Role::Context,
                    Role::Settings,
                    Role::Commit,
                    Role::Idle,
                    Role::Disposition,
                ] {
                    complete(&mut e, role, None, "1");
                }
            }
            BatchRequest::Finish {
                action: FinishAction::Rollback,
                ..
            } => {}
        }
        if result == Outcome::RolledBack {
            for role in [Role::Rollback, Role::Idle, Role::Disposition] {
                complete(&mut e, role, None, "1");
            }
        }
    }
    let owner = if matches!(
        e.request,
        BatchRequest::Batch {
            mode: Mode::Automatic,
            ..
        }
    ) {
        None
    } else {
        Some(helper)
    };
    assert!(e.accept_terminal(&json!({"outcome":result,"owner":owner,"transactionId":if result==Outcome::Rejected{None}else{Some("123")},"failure":match result{Outcome::RolledBack=>Some("cardinality"),Outcome::Rejected=>Some("admission"),Outcome::Unknown=>Some("transport"),_=>None},"priorBatches":prior})));
    e
}
pub(super) fn original(r: &BatchReservation, e: &BatchEvidence) -> OriginalBatchOutcome {
    OriginalBatchOutcome::capture_in(
        r,
        e,
        BatchTermination::Complete,
        2,
        Vec::with_capacity(MAX_ENVELOPE_BYTES),
    )
    .unwrap()
}
pub(super) fn record(
    store: &Persistence,
    owner: &HistoryOwner,
    gate: &OperationGate,
    r: &BatchReservation,
    o: &OriginalBatchOutcome,
) {
    assert_eq!(
        store.capture_batch_outcome(owner, gate, r, o).unwrap(),
        outcomes::OutcomePhase::Captured
    );
    assert_eq!(
        store.settle_batch_outcome(owner, gate, r, o).unwrap(),
        outcomes::OutcomePhase::Settled
    );
}
pub(super) fn start(
    store: &Persistence,
    owner: &Arc<HistoryOwner>,
    gate: &OperationGate,
    d: &BatchDescriptor,
    helper: uuid::Uuid,
) -> (
    BatchReservation,
    ManualTransactionLease,
    OriginalBatchOutcome,
) {
    let plan = BatchPlan::new(request(None, 1)).unwrap();
    let r = store
        .reserve_owned_batch(owner, gate, d, &plan, None)
        .unwrap();
    let group = r.manual_transaction().unwrap();
    let o = original(
        &r,
        &evidence(plan.request.clone(), helper, Outcome::Pending, 0),
    );
    record(store, owner, gate, &r, &o);
    (r, group, o)
}

#[test]
fn batch_archive_audit_and_binding_settlement_is_atomic_and_idempotent() {
    for trigger in [
        "CREATE TEMP TRIGGER fault BEFORE INSERT ON batch_captures_v8 BEGIN SELECT RAISE(ABORT,'archive fault'); END;",
        "CREATE TEMP TRIGGER fault BEFORE INSERT ON audit_v8 WHEN NEW.outcome!='started' BEGIN SELECT RAISE(ABORT,'audit fault'); END;",
        "CREATE TEMP TRIGGER fault BEFORE UPDATE ON batch_operations_v8 WHEN NEW.phase='settled' BEGIN SELECT RAISE(ABORT,'binding fault'); END;",
    ] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let d = descriptor();
        let plan = BatchPlan::new(request(None, 1)).unwrap();
        let r = store
            .reserve_owned_batch(&owner, &gate, &d, &plan, None)
            .unwrap();
        let o = original(
            &r,
            &evidence(
                plan.request.clone(),
                uuid::Uuid::new_v4(),
                Outcome::Pending,
                0,
            ),
        );
        store.capture_batch_outcome(&owner, &gate, &r, &o).unwrap();
        store.connection.lock().execute_batch(trigger).unwrap();
        assert!(store.settle_batch_outcome(&owner, &gate, &r, &o).is_err());
        assert!(store.batch_evidence(r.operation_id()).unwrap().is_none());
        assert_eq!(store.audit_log().unwrap().len(), 1);
        assert_eq!(
            read_row(&store.connection.lock(), r.operation_id())
                .unwrap()
                .unwrap()
                .phase,
            "captured"
        );
        store
            .connection
            .lock()
            .execute_batch("DROP TRIGGER fault")
            .unwrap();
        store.settle_batch_outcome(&owner, &gate, &r, &o).unwrap();
        store.settle_batch_outcome(&owner, &gate, &r, &o).unwrap();
        assert_eq!(
            store
                .batch_evidence(r.operation_id())
                .unwrap()
                .unwrap()
                .as_bytes(),
            o.as_bytes()
        );
        assert_eq!(store.audit_log().unwrap().len(), 2);
        validate_records(&store.connection.lock()).unwrap();
    }
}

#[test]
fn batch_attempt_membership_is_immutable_and_distinct_from_pending_count() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let helper = uuid::Uuid::new_v4();
    let (first, group, first_original) = start(&store, &owner, &gate, &d, helper);
    let first_audits = serde_json::to_vec(&store.audit_log().unwrap()).unwrap();
    let rejected_plan = BatchPlan::new(request(Some(helper), 1)).unwrap();
    let rejected = store
        .reserve_owned_batch(&owner, &gate, &next(&d), &rejected_plan, Some(&group))
        .unwrap();
    let rejected_original = original(
        &rejected,
        &evidence(rejected_plan.request.clone(), helper, Outcome::Rejected, 1),
    );
    record(&store, &owner, &gate, &rejected, &rejected_original);
    let page = store
        .manual_transaction_evidence(group.id(), 0, 64)
        .unwrap();
    assert_eq!(page.pending_count, 1);
    assert_eq!(page.members.len(), 2);
    assert_eq!(page.members[1].prior_pending_count, 1);
    let second_plan = BatchPlan::new(request(Some(helper), 1)).unwrap();
    let second = store
        .reserve_owned_batch(&owner, &gate, &next(&d), &second_plan, Some(&group))
        .unwrap();
    assert!(
        store
            .reserve_owned_batch_finish(&owner, &gate, &next(&d), &group, FinishAction::Commit)
            .is_err()
    );
    let second_original = original(
        &second,
        &evidence(second_plan.request.clone(), helper, Outcome::Pending, 1),
    );
    record(&store, &owner, &gate, &second, &second_original);
    let finish = store
        .reserve_owned_batch_finish(&owner, &gate, &next(&d), &group, FinishAction::Rollback)
        .unwrap();
    let finish_evidence = evidence(
        BatchRequest::Finish {
            owner: helper,
            action: FinishAction::Rollback,
        },
        helper,
        Outcome::RolledBack,
        2,
    );
    let finish_original = original(&finish, &finish_evidence);
    record(&store, &owner, &gate, &finish, &finish_original);
    let page = store.manual_transaction_evidence(group.id(), 0, 2).unwrap();
    assert_eq!(page.transaction_id, group.id());
    assert_eq!(page.pending_count, 2);
    assert_eq!(page.disposition, "rolledBack");
    assert_eq!(page.next, Some(2));
    assert_eq!(
        store
            .manual_transaction_evidence(group.id(), 2, 64)
            .unwrap()
            .members
            .len(),
        2
    );
    assert_eq!(
        store
            .batch_evidence(first.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        first_original.as_bytes()
    );
    assert_eq!(
        store
            .batch_evidence(second.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        second_original.as_bytes()
    );
    let current_audits = store.audit_log().unwrap();
    let prior: Vec<_> = current_audits
        .into_iter()
        .filter(|a| a.id.starts_with(first.operation_id()))
        .collect();
    assert_eq!(serde_json::to_vec(&prior).unwrap(), first_audits);
    // A late idempotent retry cannot reopen the closed in-memory lease.
    store
        .settle_batch_outcome(&owner, &gate, &first, &first_original)
        .unwrap();
    assert!(
        store
            .make_volatile_batch_rollback_token(&owner, &gate, &next(&d), &group, helper)
            .is_err()
    );
    assert!(owner.batch_credits.lock().is_empty());
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn batch_whole_maximal_envelope_roundtrips_before_and_after_settlement() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let mut d = descriptor();
    d.connection_id = "\u{1}".repeat(256);
    d.started_at = MAX_TIME;
    let plan = BatchPlan::new(request(None, 500)).unwrap();
    let r = store
        .reserve_owned_batch(&owner, &gate, &d, &plan, None)
        .unwrap();
    let e = evidence(
        plan.request.clone(),
        uuid::Uuid::new_v4(),
        Outcome::Pending,
        0,
    );
    let o = OriginalBatchOutcome::capture_in(
        &r,
        &e,
        BatchTermination::Complete,
        MAX_TIME,
        Vec::with_capacity(MAX_ENVELOPE_BYTES),
    )
    .unwrap();
    assert!(o.as_bytes().len() <= MAX_ENVELOPE_BYTES);
    assert_eq!(decode(o.as_bytes()).unwrap().1, e);
    record(&store, &owner, &gate, &r, &o);
    assert_eq!(
        store
            .batch_evidence(r.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        o.as_bytes()
    );
    assert!(BatchPlan::new(request(None, 501)).is_err());
    let mut bad = next(&d);
    bad.connection_id.push('x');
    assert!(
        store
            .reserve_owned_batch(&owner, &gate, &bad, &plan, None)
            .is_err()
    );
    let mut altered = o.as_bytes().to_vec();
    altered.extend(std::iter::repeat_n(
        b' ',
        MAX_ENVELOPE_BYTES + 1 - altered.len(),
    ));
    assert!(decode(&altered).is_err());
    assert!(
        OriginalBatchOutcome::capture_in(
            &r,
            &e,
            BatchTermination::Complete,
            2,
            Vec::with_capacity(32)
        )
        .is_err()
    );
    validate_records(&store.connection.lock()).unwrap();
    let retained = o.clone();
    let o = o.try_reclaim_buffer().unwrap_err();
    drop(retained);
    let buffer = o.try_reclaim_buffer().unwrap();
    assert!(buffer.is_empty());
    assert_eq!(buffer.capacity(), MAX_ENVELOPE_BYTES);
}

#[test]
fn batch_capture_rejects_missing_expected_row_and_changed_original() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let plan = BatchPlan::new(BatchRequest::Batch {
        mode: Mode::Automatic,
        owner: None,
        rollback_prior: false,
        units: vec![UnitKind::Update],
    })
    .unwrap();
    let r = store
        .reserve_owned_batch(&owner, &gate, &d, &plan, None)
        .unwrap();
    let e = evidence(
        plan.request.clone(),
        uuid::Uuid::new_v4(),
        Outcome::Committed,
        0,
    );
    let o = original(&r, &e);
    store.capture_batch_outcome(&owner, &gate, &r, &o).unwrap();
    let changed = OriginalBatchOutcome::capture_in(
        &r,
        &e,
        BatchTermination::Complete,
        3,
        Vec::with_capacity(MAX_ENVELOPE_BYTES),
    )
    .unwrap();
    assert_eq!(
        store
            .capture_batch_outcome(&owner, &gate, &r, &changed)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    store
        .connection
        .lock()
        .execute(
            "DELETE FROM batch_operations_v8 WHERE operation_id=?1",
            [r.operation_id()],
        )
        .unwrap();
    assert_eq!(
        store
            .capture_batch_outcome(&owner, &gate, &r, &o)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(
        read_row(&store.connection.lock(), r.operation_id())
            .unwrap()
            .is_none()
    );
}

#[test]
fn batch_unknown_and_no_original_prevent_continuation_and_keep_live_proof() {
    for no_original in [false, true] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let d = descriptor();
        let helper = uuid::Uuid::new_v4();
        let (_r, group, _o) = start(&store, &owner, &gate, &d, helper);
        let plan = BatchPlan::new(request(Some(helper), 1)).unwrap();
        let r = store
            .reserve_owned_batch(&owner, &gate, &next(&d), &plan, Some(&group))
            .unwrap();
        if no_original {
            store.abandon_reserved_batch(&owner, &gate, &r).unwrap();
        } else {
            let e = BatchEvidence::new(plan.request.clone());
            let o = OriginalBatchOutcome::capture_in(
                &r,
                &e,
                BatchTermination::Eof,
                2,
                Vec::with_capacity(MAX_ENVELOPE_BYTES),
            )
            .unwrap();
            record(&store, &owner, &gate, &r, &o);
        }
        assert_eq!(
            store
                .manual_transaction_evidence(group.id(), 0, 64)
                .unwrap()
                .disposition,
            "unknown"
        );
        assert!(
            binding(&store.connection.lock(), group.id())
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .reserve_owned_batch(&owner, &gate, &next(&d), &plan, Some(&group))
                .is_err()
        );
        assert!(
            store
                .make_volatile_batch_rollback_token(&owner, &gate, &next(&d), &group, helper)
                .is_err()
        );
        validate_records(&store.connection.lock()).unwrap();
    }
}

#[test]
fn batch_volatile_rollback_encodes_before_promotion_and_defers_started_audit() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let helper = uuid::Uuid::new_v4();
    let (_r, group, _o) = start(&store, &owner, &gate, &d, helper);
    store.connection.lock().execute_batch("CREATE TEMP TRIGGER no_started BEFORE INSERT ON audit_v8 WHEN NEW.outcome='started' BEGIN SELECT RAISE(ABORT,'secondary audit fault'); END;").unwrap();
    let finish = next(&d);
    assert!(
        store
            .reserve_owned_batch_finish(&owner, &gate, &finish, &group, FinishAction::Rollback)
            .is_err()
    );
    let token = store
        .make_volatile_batch_rollback_token(&owner, &gate, &finish, &group, helper)
        .unwrap();
    let e = evidence(
        BatchRequest::Finish {
            owner: helper,
            action: FinishAction::Rollback,
        },
        helper,
        Outcome::RolledBack,
        1,
    );
    let o = OriginalBatchOutcome::volatile_rollback_in(
        &token,
        &e,
        BatchTermination::Complete,
        2,
        Vec::with_capacity(MAX_ENVELOPE_BYTES),
    )
    .unwrap();
    assert!(
        read_row(&store.connection.lock(), &finish.operation_id)
            .unwrap()
            .is_none()
    );
    let r = store
        .promote_volatile_batch_rollback(&owner, &gate, &token)
        .unwrap();
    store.capture_batch_outcome(&owner, &gate, &r, &o).unwrap();
    assert!(store.settle_batch_outcome(&owner, &gate, &r, &o).is_err());
    store
        .connection
        .lock()
        .execute_batch("DROP TRIGGER no_started")
        .unwrap();
    store.settle_batch_outcome(&owner, &gate, &r, &o).unwrap();
    assert_eq!(
        store
            .batch_evidence(r.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        o.as_bytes()
    );
    assert_eq!(
        store
            .manual_transaction_evidence(group.id(), 0, 64)
            .unwrap()
            .disposition,
        "rolledBack"
    );
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn batch_reserved_rollback_survives_marker_and_logical_archive_exhaustion() {
    // These real-SQLite synthetic charges exercise quota wiring, not capacity
    // acceptance or a realistic archive workload. Remove them before validation.
    for archive_full in [false, true] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let d = descriptor();
        let helper = uuid::Uuid::new_v4();
        let (_r, group, _o) = start(&store, &owner, &gate, &d, helper);
        {
            let c = store.connection.lock();
            if archive_full {
                let used = bytes_used(&c, "ordinary").unwrap();
                let full = (MAX_ORDINARY_BYTES - used) / MAX_ENVELOPE_BYTES as i64;
                let remainder = (MAX_ORDINARY_BYTES - used) % MAX_ENVELOPE_BYTES as i64;
                c.execute("WITH RECURSIVE n(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<?1) INSERT INTO batch_captures_v8 SELECT printf('00000000-0000-4000-8000-%012x',x),?2,zeroblob(32768),'ordinary','imported','captured' FROM n",params![full,"0".repeat(64)]).unwrap();
                if remainder > 0 {
                    c.execute("INSERT INTO batch_captures_v8 VALUES('ffffffff-ffff-4fff-8fff-ffffffffffff',?1,zeroblob(?2),'ordinary','imported','captured')",params!["0".repeat(64),remainder]).unwrap();
                }
                assert_eq!(bytes_used(&c, "ordinary").unwrap(), MAX_ORDINARY_BYTES);
            } else {
                let extra = MAX_MARKERS - counts(&c, &owner.nonce, "ordinary").unwrap().0;
                c.execute("WITH RECURSIVE n(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<?1) INSERT INTO outcome_operations_v7(operation_id,owner_nonce,pool,phase,reservation_digest,terminal_kind) SELECT printf('00000000-0000-4000-8000-%012x',x),?2,'ordinary','settled',?3,'noOriginal' FROM n",params![extra,owner.nonce,"0".repeat(64)]).unwrap();
            }
        }
        let plan = BatchPlan::new(request(Some(helper), 1)).unwrap();
        assert_eq!(
            store
                .reserve_owned_batch(&owner, &gate, &next(&d), &plan, Some(&group))
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
        let r = store
            .reserve_owned_batch_finish(&owner, &gate, &next(&d), &group, FinishAction::Rollback)
            .unwrap();
        let e = evidence(
            BatchRequest::Finish {
                owner: helper,
                action: FinishAction::Rollback,
            },
            helper,
            Outcome::RolledBack,
            1,
        );
        let o = original(&r, &e);
        record(&store, &owner, &gate, &r, &o);
        assert_eq!(
            store
                .manual_transaction_evidence(group.id(), 0, 64)
                .unwrap()
                .disposition,
            "rolledBack"
        );
        {
            let c = store.connection.lock();
            if archive_full {
                c.execute(
                    "DELETE FROM batch_captures_v8 WHERE capture_digest=?1",
                    ["0".repeat(64)],
                )
                .unwrap();
            } else {
                c.execute(
                    "DELETE FROM outcome_operations_v7 WHERE reservation_digest=?1",
                    ["0".repeat(64)],
                )
                .unwrap();
            }
        }
        validate_records(&store.connection.lock()).unwrap();
    }
}

#[test]
fn batch_group_caps_are_real_sqlite_logical_limits_not_capacity_acceptance() {
    let (_root, store, first_owner) = setup();
    let mut owners = vec![first_owner];
    let mut held = Vec::new();
    for owner_index in 0..4 {
        if owner_index > 0 {
            owners.push(Arc::new(store.claim_history_owner().unwrap()));
        }
        let owner = &owners[owner_index];
        let gate = store.operation_gate(owner).unwrap();
        for _ in 0..32 {
            let d = descriptor();
            let helper = uuid::Uuid::new_v4();
            held.push(start(&store, owner, &gate, &d, helper));
        }
        let plan = BatchPlan::new(request(None, 1)).unwrap();
        assert_eq!(
            store
                .reserve_owned_batch(owner, &gate, &descriptor(), &plan, None)
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
        assert_eq!(owner.batch_credits.lock().len(), 32);
    }
    let owner = Arc::new(store.claim_history_owner().unwrap());
    let gate = store.operation_gate(&owner).unwrap();
    let plan = BatchPlan::new(request(None, 1)).unwrap();
    assert_eq!(
        store
            .reserve_owned_batch(&owner, &gate, &descriptor(), &plan, None)
            .unwrap_err()
            .code,
        ErrorCode::ResultLimitReached
    );
    assert_eq!(held.len(), 128);
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn batch_and_ordinary_operation_namespaces_are_symmetric() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let plan = BatchPlan::new(BatchRequest::Batch {
        mode: Mode::Automatic,
        owner: None,
        rollback_prior: false,
        units: vec![UnitKind::Update],
    })
    .unwrap();
    let r = store
        .reserve_owned_batch(&owner, &gate, &d, &plan, None)
        .unwrap();
    let ordinary = OutcomeDescriptor {
        operation_id: d.operation_id.clone(),
        connection_id: d.connection_id.clone(),
        session_id: d.session_id.clone(),
        started_at: 1,
        digest: d.digest.clone(),
        kind: OutcomeKind::Boundary {
            action: crate::domain::ExplicitTransactionAction::Commit,
        },
    };
    assert_eq!(
        store
            .reserve_owned_operation(&owner, &gate, None, &ordinary, OutcomePool::Ordinary)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let mut ordinary = ordinary;
    ordinary.operation_id = uuid::Uuid::new_v4().to_string();
    store
        .reserve_owned_operation(&owner, &gate, None, &ordinary, OutcomePool::Ordinary)
        .unwrap();
    let mut collision = next(&d);
    collision.operation_id = ordinary.operation_id;
    assert_eq!(
        store
            .reserve_owned_batch(&owner, &gate, &collision, &plan, None)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    store.abandon_reserved_batch(&owner, &gate, &r).unwrap();
}

#[test]
fn batch_idle_group_retains_real_owner_proof_and_blocks_restore() {
    let (root, store, owner) = setup();
    let backup = store.create_metadata_backup().unwrap();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let helper = uuid::Uuid::new_v4();
    let (r, group, o) = start(&store, &owner, &gate, &d, helper);
    let id = group.id().to_owned();
    drop(gate);
    drop(owner);
    let _observer = store.claim_history_owner().unwrap();
    assert_eq!(
        store
            .manual_transaction_evidence(&id, 0, 64)
            .unwrap()
            .disposition,
        "open"
    );
    assert_eq!(
        store
            .restore_metadata_backup(&backup.id, &backup.digest)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    drop(r);
    drop(group);
    drop(o);
    let _observer2 = store.claim_history_owner().unwrap();
    assert_eq!(
        store
            .manual_transaction_evidence(&id, 0, 64)
            .unwrap()
            .disposition,
        "unknown"
    );
    assert!(binding(&store.connection.lock(), &id).unwrap().is_none());
    store
        .restore_metadata_backup(&backup.id, &backup.digest)
        .unwrap();
    assert!(store.batch_evidence(&d.operation_id).unwrap().is_some());
    assert!(root.path().join("joinery.sqlite").exists());
}

#[test]
fn batch_foreign_captured_and_settled_backups_are_historical_only() {
    for settled in [false, true] {
        let (source_root, source, source_owner) = setup();
        let baseline = source.create_metadata_backup().unwrap();
        let target_root = tempfile::tempdir().unwrap();
        std::fs::copy(
            source_root
                .path()
                .join("backups")
                .join(format!("{}.sqlite", baseline.id)),
            target_root.path().join("joinery.sqlite"),
        )
        .unwrap();
        let target = Persistence::open(target_root.path()).unwrap();
        let gate = source.operation_gate(&source_owner).unwrap();
        let d = descriptor();
        let plan = BatchPlan::new(request(None, 1)).unwrap();
        let r = source
            .reserve_owned_batch(&source_owner, &gate, &d, &plan, None)
            .unwrap();
        let group = r.manual_transaction().unwrap();
        let o = original(
            &r,
            &evidence(
                plan.request.clone(),
                uuid::Uuid::new_v4(),
                Outcome::Pending,
                0,
            ),
        );
        source
            .capture_batch_outcome(&source_owner, &gate, &r, &o)
            .unwrap();
        if settled {
            source
                .settle_batch_outcome(&source_owner, &gate, &r, &o)
                .unwrap();
        }
        let backup = source.create_metadata_backup().unwrap();
        std::fs::create_dir_all(target_root.path().join("backups")).unwrap();
        std::fs::copy(
            source_root
                .path()
                .join("backups")
                .join(format!("{}.sqlite", backup.id)),
            target_root
                .path()
                .join("backups")
                .join(format!("{}.sqlite", backup.id)),
        )
        .unwrap();
        target
            .restore_metadata_backup(&backup.id, &backup.digest)
            .unwrap();
        let historical = target.batch_evidence(r.operation_id()).unwrap().unwrap();
        assert_eq!(historical.as_bytes(), o.as_bytes());
        assert_eq!(
            historical.evidence().unwrap(),
            decode(o.as_bytes()).unwrap().1
        );
        assert!(historical.imported);
        assert_eq!(historical.settled, settled);
        let page = target
            .manual_transaction_evidence(group.id(), 0, 64)
            .unwrap();
        assert_eq!(page.disposition, "unknown");
        assert_eq!(page.pending_count, 1);
        let _target_owner = target.claim_history_owner().unwrap();
        assert_eq!(
            target
                .batch_evidence(r.operation_id())
                .unwrap()
                .unwrap()
                .as_bytes(),
            o.as_bytes()
        );
        assert!(
            binding(&target.connection.lock(), group.id())
                .unwrap()
                .is_none()
        );
        assert!(
            read_row(&target.connection.lock(), r.operation_id())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            target
                .audit_log()
                .unwrap()
                .iter()
                .filter(|a| a.id == format!("{}:finished", r.operation_id()))
                .count(),
            usize::from(settled)
        );
        validate_records(&target.connection.lock()).unwrap();
    }
}

#[test]
fn batch_malformed_or_oversized_metadata_is_rejected_before_payload_allocation() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let plan = BatchPlan::new(request(None, 1)).unwrap();
    let r = store
        .reserve_owned_batch(&owner, &gate, &d, &plan, None)
        .unwrap();
    store
        .connection
        .lock()
        .execute_batch("PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    store.connection.lock().execute("UPDATE batch_operations_v8 SET reservation_json=hex(zeroblob(65536)) WHERE operation_id=?1",[r.operation_id()]).unwrap();
    assert!(validate_records(&store.connection.lock()).is_err());
    assert!(
        row_data(
            r.operation_id(),
            &read_row(&store.connection.lock(), r.operation_id())
                .unwrap()
                .unwrap()
        )
        .is_err()
    );
    let mut malformed = next(&d);
    malformed.helper_generation = "raw SDK context".into();
    assert!(
        store
            .reserve_owned_batch(&owner, &gate, &malformed, &plan, None)
            .is_err()
    );
}

fn restore_merge_case(different: bool) {
    // Exercise the merge hook directly. Public restore also recovers proven-dead
    // owners before publication, so this does not claim it publishes wrong bytes.
    {
        let (root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let d = descriptor();
        let plan = BatchPlan::new(BatchRequest::Batch {
            mode: Mode::Automatic,
            owner: None,
            rollback_prior: false,
            units: vec![UnitKind::Update],
        })
        .unwrap();
        let r = store
            .reserve_owned_batch(&owner, &gate, &d, &plan, None)
            .unwrap();
        let e = evidence(
            plan.request.clone(),
            uuid::Uuid::new_v4(),
            Outcome::Committed,
            0,
        );
        let original = original(&r, &e);
        store
            .capture_batch_outcome(&owner, &gate, &r, &original)
            .unwrap();
        let backup = store.create_metadata_backup().unwrap();
        let candidate = Connection::open(
            root.path()
                .join("backups")
                .join(format!("{}.sqlite", backup.id)),
        )
        .unwrap();
        if different {
            let changed = OriginalBatchOutcome::capture_in(
                &r,
                &e,
                BatchTermination::Complete,
                3,
                Vec::with_capacity(MAX_ENVELOPE_BYTES),
            )
            .unwrap();
            candidate.execute("UPDATE batch_operations_v8 SET payload=?1,capture_digest=?2 WHERE operation_id=?3",params![changed.as_bytes(),hash(changed.as_bytes()),r.operation_id()]).unwrap();
        }
        let c = store.connection.lock();
        let merged = merge_for_restore(&c, &candidate);
        if different {
            assert_eq!(merged.unwrap_err().code, ErrorCode::Conflict);
        } else {
            merged.unwrap();
            settle(&candidate, &r.data, &r.digest, original.as_bytes()).unwrap();
            let archived = read_archive(&candidate, r.operation_id()).unwrap().unwrap();
            assert_eq!(archived.3, "local");
            assert_eq!(archived.4, "settled");
            validate_records(&candidate).unwrap();
        }
    }
}

#[test]
fn batch_restore_hook_rejects_changed_current_capture() {
    restore_merge_case(true);
}

#[test]
fn batch_restore_hook_promotes_identical_archive_after_local_settlement() {
    restore_merge_case(false);
}

#[test]
fn batch_schema8_migration_fences_cached_seven_and_preserves_original_boundary() {
    let (root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let descriptor = OutcomeDescriptor {
        operation_id: uuid::Uuid::new_v4().to_string(),
        connection_id: "migration-fixture".into(),
        session_id: uuid::Uuid::new_v4().to_string(),
        started_at: 1,
        digest: "a".repeat(64),
        kind: OutcomeKind::Boundary {
            action: crate::domain::ExplicitTransactionAction::Commit,
        },
    };
    let r = store
        .reserve_owned_operation(&owner, &gate, None, &descriptor, OutcomePool::Ordinary)
        .unwrap();
    let original = crate::persistence::OriginalOutcome::boundary(
        &descriptor,
        &crate::persistence::BoundaryEvidence {
            action: crate::domain::ExplicitTransactionAction::Commit,
            boundary_outcome: crate::domain::TransactionBoundaryOutcome::Acknowledged,
            retired: false,
        },
        2,
    )
    .unwrap();
    store
        .capture_original_outcome(&owner, &gate, &r, &original)
        .unwrap();
    {
        let c = store.connection.lock();
        drop_schema(&c);
        c.execute_batch("ALTER TABLE profiles_v8 RENAME TO profiles_v7; ALTER TABLE history_v8 RENAME TO history_v7; ALTER TABLE audit_v8 RENAME TO audit_v7; PRAGMA user_version=7;").unwrap();
        super::super::recovery::validate(&c).unwrap();
    }
    drop(gate);
    assert_eq!(
        Persistence::open(root.path()).err().unwrap().code,
        ErrorCode::Conflict
    );
    drop(owner);
    drop(store);
    let cached = Connection::open(root.path().join("joinery.sqlite")).unwrap();
    let mut clear = cached.prepare("DELETE FROM history_v7").unwrap();
    let mut profile = cached.prepare("DELETE FROM profiles_v7").unwrap();
    let store = Persistence::open(root.path()).unwrap();
    assert_eq!(schema_version(&store.connection.lock()).unwrap(), 8);
    assert!(clear.execute([]).is_err());
    assert!(profile.execute([]).is_err());
    let _owner = store.claim_history_owner().unwrap();
    let boundary = store
        .settled_boundary_evidence(&descriptor.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        boundary.boundary_outcome,
        crate::domain::TransactionBoundaryOutcome::Acknowledged
    );
    super::super::recovery::validate(&store.connection.lock()).unwrap();
}

#[test]
fn batch_volatile_rollback_cannot_overlap_reservation_commit() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let helper = uuid::Uuid::new_v4();
    let (_first, group, _original) = start(&store, &owner, &gate, &d, helper);
    let continuation = next(&d);
    let rollback = next(&d);
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    store
        .connection
        .lock()
        .commit_hook(Some(move || {
            entered_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            false
        }))
        .unwrap();
    let token_result = std::thread::scope(|scope| {
        let reservation = scope.spawn(|| {
            let plan = BatchPlan::new(request(Some(helper), 1)).unwrap();
            store.reserve_owned_batch(&owner, &gate, &continuation, &plan, Some(&group))
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        // The continuation has reached COMMIT but has not published its memory
        // snapshot. No second local cleanup authority may escape this window.
        let token_result =
            store.make_volatile_batch_rollback_token(&owner, &gate, &rollback, &group, helper);
        release_tx.send(()).unwrap();
        reservation.join().unwrap().unwrap();
        token_result
    });
    store
        .connection
        .lock()
        .commit_hook(None::<fn() -> bool>)
        .unwrap();
    assert_eq!(token_result.unwrap_err().code, ErrorCode::Conflict);
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn batch_volatile_rollback_rejects_same_id_ordinary_or_commit_reservation() {
    for commit in [false, true] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let d = descriptor();
        let helper = uuid::Uuid::new_v4();
        let (_first, group, _original) = start(&store, &owner, &gate, &d, helper);
        let next = next(&d);
        let _in_flight = if commit {
            store
                .reserve_owned_batch_finish(&owner, &gate, &next, &group, FinishAction::Commit)
                .unwrap()
        } else {
            let plan = BatchPlan::new(request(Some(helper), 1)).unwrap();
            store
                .reserve_owned_batch(&owner, &gate, &next, &plan, Some(&group))
                .unwrap()
        };
        assert_eq!(
            store
                .make_volatile_batch_rollback_token(&owner, &gate, &next, &group, helper,)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        validate_records(&store.connection.lock()).unwrap();
    }
}

#[test]
fn batch_rejected_protected_rollback_retires_group_without_minting_credit() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let helper = uuid::Uuid::new_v4();
    let (_first, group, _original) = start(&store, &owner, &gate, &d, helper);
    let finish = store
        .reserve_owned_batch_finish(&owner, &gate, &next(&d), &group, FinishAction::Rollback)
        .unwrap();
    let e = evidence(
        BatchRequest::Finish {
            owner: helper,
            action: FinishAction::Rollback,
        },
        helper,
        Outcome::Rejected,
        1,
    );
    let o = original(&finish, &e);
    record(&store, &owner, &gate, &finish, &o);
    let page = store
        .manual_transaction_evidence(group.id(), 0, 64)
        .unwrap();
    assert_eq!(page.disposition, "unknown");
    assert_eq!(page.pending_count, 1);
    assert!(
        !binding(&store.connection.lock(), group.id())
            .unwrap()
            .unwrap()
            .1
    );
    assert_eq!(
        store
            .batch_evidence(finish.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        o.as_bytes()
    );
    assert!(
        store
            .reserve_owned_batch_finish(&owner, &gate, &next(&d), &group, FinishAction::Rollback)
            .is_err()
    );
    assert!(
        store
            .make_volatile_batch_rollback_token(&owner, &gate, &next(&d), &group, helper)
            .is_err()
    );
    validate_records(&store.connection.lock()).unwrap();
}

#[test]
fn batch_restore_identical_manual_capture_preserves_prior_count_until_local_settlement() {
    let (root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let plan = BatchPlan::new(request(None, 1)).unwrap();
    let r = store
        .reserve_owned_batch(&owner, &gate, &d, &plan, None)
        .unwrap();
    let group = r.manual_transaction().unwrap();
    let e = evidence(
        plan.request().clone(),
        uuid::Uuid::new_v4(),
        Outcome::Pending,
        0,
    );
    let original = original(&r, &e);
    store
        .capture_batch_outcome(&owner, &gate, &r, &original)
        .unwrap();
    let backup = store.create_metadata_backup().unwrap();
    let candidate = Connection::open(
        root.path()
            .join("backups")
            .join(format!("{}.sqlite", backup.id)),
    )
    .unwrap();
    merge_for_restore(&store.connection.lock(), &candidate).unwrap();
    let before = read_group(&candidate, group.id()).unwrap().unwrap();
    assert_eq!(before.pending, 0);
    assert_eq!(before.disposition, "open");
    assert_eq!(
        read_row(&candidate, r.operation_id())
            .unwrap()
            .unwrap()
            .phase,
        "captured"
    );
    validate_records(&candidate).unwrap();
    settle(&candidate, &r.data, &r.digest, original.as_bytes()).unwrap();
    let after = read_group(&candidate, group.id()).unwrap().unwrap();
    assert_eq!(after.pending, 1);
    assert_eq!(after.disposition, "open");
    let archived = read_archive(&candidate, r.operation_id()).unwrap().unwrap();
    assert_eq!(archived.1, original.as_bytes());
    assert_eq!(
        (archived.3.as_str(), archived.4.as_str()),
        ("local", "settled")
    );
    validate_records(&candidate).unwrap();
}

#[test]
fn batch_final_ordinal_is_reserved_for_the_protected_rollback() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let helper = uuid::Uuid::new_v4();
    let (_first, group, _original) = start(&store, &owner, &gate, &d, helper);
    // Guard-only boundary fixture: these counters are deliberately synthetic.
    // This does not claim a valid 65k-member archive or capacity qualification.
    store
        .connection
        .lock()
        .execute(
            "UPDATE manual_transactions_v8 SET next_ordinal=65535 WHERE transaction_id=?1",
            [group.id()],
        )
        .unwrap();
    group.memory.0.lock().ordinal = 65_535;
    let plan = BatchPlan::new(request(Some(helper), 1)).unwrap();
    assert_eq!(
        store
            .reserve_owned_batch(&owner, &gate, &next(&d), &plan, Some(&group))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        store
            .reserve_owned_batch_finish(&owner, &gate, &next(&d), &group, FinishAction::Commit)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let rollback = store
        .reserve_owned_batch_finish(&owner, &gate, &next(&d), &group, FinishAction::Rollback)
        .unwrap();
    assert_eq!(rollback.data.membership.as_ref().unwrap().ordinal, 65_536);
    let e = evidence(
        BatchRequest::Finish {
            owner: helper,
            action: FinishAction::Rollback,
        },
        helper,
        Outcome::RolledBack,
        1,
    );
    let original = original(&rollback, &e);
    record(&store, &owner, &gate, &rollback, &original);
    assert_eq!(
        read_group(&store.connection.lock(), group.id())
            .unwrap()
            .unwrap()
            .disposition,
        "rolledBack"
    );
    assert!(
        binding(&store.connection.lock(), group.id())
            .unwrap()
            .is_none()
    );
}

#[test]
fn batch_volatile_rollback_token_first_rejects_same_descriptor_ordinary_or_commit() {
    for commit in [false, true] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let d = descriptor();
        let helper = uuid::Uuid::new_v4();
        let (_first, group, _original) = start(&store, &owner, &gate, &d, helper);
        let descriptor = next(&d);
        let token = store
            .make_volatile_batch_rollback_token(&owner, &gate, &descriptor, &group, helper)
            .unwrap();
        // Exact same ID and all frozen descriptor fields: only intent differs.
        let ordinary = if commit {
            store.reserve_owned_batch_finish(
                &owner,
                &gate,
                &descriptor,
                &group,
                FinishAction::Commit,
            )
        } else {
            let plan = BatchPlan::new(request(Some(helper), 1)).unwrap();
            store.reserve_owned_batch(&owner, &gate, &descriptor, &plan, Some(&group))
        };
        assert_eq!(ordinary.unwrap_err().code, ErrorCode::Conflict);
        let reservation = store
            .promote_volatile_batch_rollback(&owner, &gate, &token)
            .unwrap();
        let e = evidence(
            BatchRequest::Finish {
                owner: helper,
                action: FinishAction::Rollback,
            },
            helper,
            Outcome::RolledBack,
            1,
        );
        let original = OriginalBatchOutcome::volatile_rollback_in(
            &token,
            &e,
            BatchTermination::Complete,
            2,
            Vec::with_capacity(MAX_ENVELOPE_BYTES),
        )
        .unwrap();
        record(&store, &owner, &gate, &reservation, &original);
        validate_records(&store.connection.lock()).unwrap();
    }
}

#[test]
fn batch_and_ordinary_share_the_owner_active_pool_count_guard_in_both_orders() {
    fn snapshot(c: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
        let mut rows = Vec::new();
        for table in [
            "outcome_operations_v7",
            "outcome_bindings_v7",
            "batch_operations_v8",
            "audit_v8",
        ] {
            let mut statement = c
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let columns = statement.column_count();
            rows.extend(
                statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|column| row.get(column))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap(),
            );
        }
        rows
    }
    // Count-guard integration only; this is not throughput, RSS, archive-size,
    // or replacement-capacity qualification. Automatic batches avoid group caps.
    for batch_first in [false, true] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let d = descriptor();
        let plan = BatchPlan::new(BatchRequest::Batch {
            mode: Mode::Automatic,
            owner: None,
            rollback_prior: false,
            units: vec![UnitKind::Update],
        })
        .unwrap();
        let mut batches = Vec::new();
        let mut ordinary = Vec::new();
        for pass in 0..2 {
            for index in 0..16 {
                let descriptor = next(&d);
                if (pass == 0) == batch_first {
                    let reservation = store
                        .reserve_owned_batch(&owner, &gate, &descriptor, &plan, None)
                        .unwrap();
                    if index == 0 {
                        let original = original(
                            &reservation,
                            &evidence(
                                plan.request().clone(),
                                uuid::Uuid::new_v4(),
                                Outcome::Committed,
                                0,
                            ),
                        );
                        store
                            .capture_batch_outcome(&owner, &gate, &reservation, &original)
                            .unwrap();
                    }
                    batches.push(reservation);
                } else {
                    let descriptor = OutcomeDescriptor {
                        operation_id: descriptor.operation_id,
                        connection_id: descriptor.connection_id,
                        session_id: descriptor.session_id,
                        started_at: descriptor.started_at,
                        digest: descriptor.digest,
                        kind: OutcomeKind::Boundary {
                            action: crate::domain::ExplicitTransactionAction::Commit,
                        },
                    };
                    let reservation = store
                        .reserve_owned_operation(
                            &owner,
                            &gate,
                            None,
                            &descriptor,
                            OutcomePool::Ordinary,
                        )
                        .unwrap();
                    if index == 0 {
                        let original = super::super::OriginalOutcome::boundary(
                            &descriptor,
                            &super::super::BoundaryEvidence {
                                action: crate::domain::ExplicitTransactionAction::Commit,
                                boundary_outcome:
                                    crate::domain::TransactionBoundaryOutcome::Acknowledged,
                                retired: false,
                            },
                            2,
                        )
                        .unwrap();
                        store
                            .capture_original_outcome(&owner, &gate, &reservation, &original)
                            .unwrap();
                    }
                    ordinary.push(reservation);
                }
            }
        }
        assert_eq!((batches.len(), ordinary.len()), (16, 16));
        let before = snapshot(&store.connection.lock());
        assert_eq!(
            store
                .reserve_owned_batch(&owner, &gate, &next(&d), &plan, None)
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
        let refused = next(&d);
        let refused = OutcomeDescriptor {
            operation_id: refused.operation_id,
            connection_id: refused.connection_id,
            session_id: refused.session_id,
            started_at: refused.started_at,
            digest: refused.digest,
            kind: OutcomeKind::Boundary {
                action: crate::domain::ExplicitTransactionAction::Commit,
            },
        };
        assert_eq!(
            store
                .reserve_owned_operation(&owner, &gate, None, &refused, OutcomePool::Ordinary)
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
        assert_eq!(snapshot(&store.connection.lock()), before);
        assert_eq!(
            counts(&store.connection.lock(), &owner.nonce, "ordinary")
                .unwrap()
                .2,
            16
        );
        super::super::recovery::validate(&store.connection.lock()).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn batch_encrypted_export_decrypt_restore_preserves_members_and_strips_live_credit() {
    use super::super::{
        recovery_crypto::{BackupPassphrase, decrypt_snapshot},
        recovery_stream::RecoveryControl,
    };
    use std::io::{Read, Seek};

    // Real local SQLite and authenticated encrypted producer/consumer. Protocol
    // receipts remain synthetic; this does not qualify remote Snowflake behavior.
    let control = RecoveryControl {
        cancel: tokio_util::sync::CancellationToken::new(),
        progress: Arc::new(|_| {}),
        published: Arc::new(|_| {}),
    };
    let secret = BackupPassphrase::new(zeroize::Zeroizing::new(
        "local batch encrypted recovery fixture".into(),
    ))
    .unwrap();
    let (source_root, source, owner) = setup();
    // Fork compatible privacy lineage before the source gains any batch records.
    let baseline = source.create_metadata_backup().unwrap();
    let target_root = tempfile::tempdir().unwrap();
    std::fs::copy(
        source_root
            .path()
            .join("backups")
            .join(format!("{}.sqlite", baseline.id)),
        target_root.path().join("joinery.sqlite"),
    )
    .unwrap();
    let target = Persistence::open(target_root.path()).unwrap();
    let gate = source.operation_gate(&owner).unwrap();
    let d = descriptor();
    let helper = uuid::Uuid::new_v4();
    let (first, closed_group, first_original) = start(&source, &owner, &gate, &d, helper);
    let finish = source
        .reserve_owned_batch_finish(
            &owner,
            &gate,
            &next(&d),
            &closed_group,
            FinishAction::Rollback,
        )
        .unwrap();
    let finish_original = original(
        &finish,
        &evidence(
            BatchRequest::Finish {
                owner: helper,
                action: FinishAction::Rollback,
            },
            helper,
            Outcome::RolledBack,
            1,
        ),
    );
    record(&source, &owner, &gate, &finish, &finish_original);
    let (idle, idle_group, idle_original) =
        start(&source, &owner, &gate, &next(&d), uuid::Uuid::new_v4());
    let expected = [
        (
            first.operation_id().to_owned(),
            first_original.as_bytes().to_vec(),
        ),
        (
            finish.operation_id().to_owned(),
            finish_original.as_bytes().to_vec(),
        ),
        (
            idle.operation_id().to_owned(),
            idle_original.as_bytes().to_vec(),
        ),
    ];
    let closed_before = source
        .manual_transaction_evidence(closed_group.id(), 0, 64)
        .unwrap();
    let idle_before = source
        .manual_transaction_evidence(idle_group.id(), 0, 64)
        .unwrap();
    assert_eq!(
        (
            closed_before.disposition.as_str(),
            closed_before.pending_count
        ),
        ("rolledBack", 1)
    );
    assert_eq!(
        (idle_before.disposition.as_str(), idle_before.pending_count),
        ("open", 1)
    );
    let live_binding = binding(&source.connection.lock(), idle_group.id()).unwrap();
    assert_eq!(live_binding, Some((owner.nonce.clone(), true)));
    let authority_before: (i64, i64) = source
        .connection
        .lock()
        .query_row(
            "SELECT count(*),coalesce(sum(rollback_credit),0) FROM manual_transaction_bindings_v8",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(authority_before, (1, 1));
    let mut source_audits = source.audit_log().unwrap();
    source_audits.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(
        source_audits
            .iter()
            .filter(|audit| audit.id.ends_with(":finished"))
            .count(),
        3
    );
    let source_audits = serde_json::to_vec(&source_audits).unwrap();

    // The existing cfg(test) encrypted producer selects its standard fast KDF;
    // all framing, authentication, disk publication and consumption stay real.
    let encrypted = source
        .create_encrypted_metadata_backup(&secret, &control)
        .unwrap();
    assert_eq!(
        encrypted.format,
        crate::domain::RecoveryBackupFormat::EncryptedV1
    );
    let mut exported = tempfile::tempfile().unwrap();
    source
        .export_recovery_backup(
            &encrypted.metadata.id,
            &encrypted.metadata.digest,
            &mut exported,
            &control,
        )
        .unwrap();
    exported.rewind().unwrap();
    let mut header = [0; b"age-encryption.org/v1\n".len()];
    exported.read_exact(&mut header).unwrap();
    assert_eq!(&header, b"age-encryption.org/v1\n");
    exported.rewind().unwrap();
    let mut decrypted = tempfile::NamedTempFile::new().unwrap();
    let authenticated = decrypt_snapshot(&mut exported, &secret, decrypted.as_file_mut()).unwrap();
    decrypted.as_file_mut().sync_all().unwrap();
    assert_eq!(
        authenticated.manifest.schema_version,
        super::super::SCHEMA_VERSION as u32
    );
    assert_eq!(authenticated.ciphertext_sha256, encrypted.metadata.digest);
    {
        // macOS's temporary path may traverse /var -> /private/var. Keep the
        // owned file alive and NOFOLLOW enabled on its canonical fixture path.
        let decrypted_path = decrypted.path().canonicalize().unwrap();
        let snapshot = Connection::open_with_flags(
            decrypted_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .unwrap();
        super::super::recovery::validate(&snapshot).unwrap();
        let counts: (i64, i64, i64, i64) = snapshot.query_row(
            "SELECT (SELECT count(*) FROM batch_captures_v8),(SELECT count(*) FROM manual_transaction_members_v8),(SELECT count(*) FROM manual_transaction_bindings_v8),(SELECT coalesce(sum(rollback_credit),0) FROM manual_transaction_bindings_v8)",
            [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(counts, (3, 3, 1, 1));
        for (id, bytes) in &expected {
            assert_eq!(read_archive(&snapshot, id).unwrap().unwrap().1, *bytes);
        }
    }

    exported.rewind().unwrap();
    let imported = target
        .import_recovery_backup(&mut exported, &control)
        .unwrap();
    assert_eq!(
        imported.format,
        crate::domain::RecoveryBackupFormat::EncryptedV1
    );
    assert_eq!(imported.metadata.digest, encrypted.metadata.digest);
    let console_owner = target.claim_console_archive_owner().unwrap();
    let undo = target
        .restore_encrypted_metadata_backup(
            &imported.metadata.id,
            &imported.metadata.digest,
            Some(&secret),
            &secret,
            &console_owner,
            &control,
        )
        .unwrap();
    assert_eq!(
        undo.format,
        crate::domain::RecoveryBackupFormat::EncryptedV1
    );
    let _target_owner = target.claim_history_owner().unwrap();
    for (id, bytes) in &expected {
        let historical = target.batch_evidence(id).unwrap().unwrap();
        assert_eq!(historical.as_bytes(), bytes);
        assert_eq!(historical.evidence().unwrap(), decode(bytes).unwrap().1);
        assert!(historical.imported && historical.settled);
    }
    let closed_after = target
        .manual_transaction_evidence(closed_group.id(), 0, 64)
        .unwrap();
    assert_eq!(closed_after.transaction_id, closed_before.transaction_id);
    assert_eq!(closed_after.members, closed_before.members);
    assert_eq!(
        (
            closed_after.disposition.as_str(),
            closed_after.pending_count
        ),
        ("rolledBack", 1)
    );
    let idle_after = target
        .manual_transaction_evidence(idle_group.id(), 0, 64)
        .unwrap();
    assert_eq!(idle_after.transaction_id, idle_before.transaction_id);
    assert_eq!(idle_after.members, idle_before.members);
    assert_eq!(
        (idle_after.disposition.as_str(), idle_after.pending_count),
        ("unknown", 1)
    );
    let imported_authority: (i64, i64, i64, i64) = target.connection.lock().query_row(
        "SELECT (SELECT count(*) FROM batch_operations_v8),(SELECT count(*) FROM manual_transaction_bindings_v8),(SELECT coalesce(sum(rollback_credit),0) FROM manual_transaction_bindings_v8),(SELECT count(*) FROM outcome_bindings_v7)",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap();
    assert_eq!(imported_authority, (0, 0, 0, 0));
    let mut target_audits = target.audit_log().unwrap();
    target_audits.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(serde_json::to_vec(&target_audits).unwrap(), source_audits);
    super::super::recovery::validate(&target.connection.lock()).unwrap();

    // Importing elsewhere cannot retire the still-live source or mutate any
    // prior Pending original, membership, disposition, audit or rollback credit.
    for (id, bytes) in &expected {
        assert_eq!(
            source.batch_evidence(id).unwrap().unwrap().as_bytes(),
            bytes
        );
    }
    let source_closed = source
        .manual_transaction_evidence(closed_group.id(), 0, 64)
        .unwrap();
    let source_idle = source
        .manual_transaction_evidence(idle_group.id(), 0, 64)
        .unwrap();
    assert_eq!(source_closed.members, closed_before.members);
    assert_eq!(source_closed.disposition, "rolledBack");
    assert_eq!(source_idle.members, idle_before.members);
    assert_eq!(
        (source_idle.disposition.as_str(), source_idle.pending_count),
        ("open", 1)
    );
    assert_eq!(
        binding(&source.connection.lock(), idle_group.id()).unwrap(),
        live_binding
    );
    let mut unchanged_audits = source.audit_log().unwrap();
    unchanged_audits.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(
        serde_json::to_vec(&unchanged_audits).unwrap(),
        source_audits
    );
    super::super::recovery::validate(&source.connection.lock()).unwrap();
}
