use super::*;
use crate::drivers::snowflake_batch::{Role, StepPhase, UnitKind};
use serde_json::{Value, json};

fn setup() -> (tempfile::TempDir, Persistence, Arc<HistoryOwner>) {
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let owner = Arc::new(store.claim_history_owner().unwrap());
    (root, store, owner)
}

fn descriptor() -> BatchDescriptor {
    BatchDescriptor {
        operation_id: uuid::Uuid::new_v4().to_string(),
        connection_id: "batch-validation-fixture".into(),
        session_id: uuid::Uuid::new_v4().to_string(),
        helper_generation: uuid::Uuid::new_v4().to_string(),
        started_at: 1,
        digest: "a".repeat(64),
    }
}

fn next(previous: &BatchDescriptor) -> BatchDescriptor {
    BatchDescriptor {
        operation_id: uuid::Uuid::new_v4().to_string(),
        ..previous.clone()
    }
}

fn request(owner: Option<uuid::Uuid>) -> BatchRequest {
    BatchRequest::Batch {
        mode: Mode::Manual,
        owner,
        rollback_prior: owner.is_some(),
        units: vec![UnitKind::Update],
    }
}

fn frame(extra: Value) -> Value {
    let mut frame = json!({"v": 1, "generation": "fixture", "id": "fixture"});
    frame
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    frame
}

// Synthetic protocol evidence exercises local validation, not remote SQL facts.
fn complete(evidence: &mut BatchEvidence, role: Role, unit: Option<usize>) {
    let step = evidence.steps.len() + 1;
    for phase in [
        StepPhase::Dispatching,
        StepPhase::Acknowledged,
        StepPhase::Drained,
    ] {
        if phase == StepPhase::Acknowledged {
            assert!(evidence.apply_event(&frame(json!({
                "type": "batchStatement", "step": step,
                "sdkRequestId": uuid::Uuid::new_v4(), "queryId": uuid::Uuid::new_v4(),
            }))));
        }
        if phase == StepPhase::Drained
            && let Some(unit) = unit
        {
            assert!(evidence.apply_event(&frame(json!({
                "type": "batchCount", "step": step, "unit": unit,
                "affectedRows": "1", "multiJoinedRows": "0",
            }))));
        }
        assert!(evidence.apply_event(&frame(json!({
            "type": "batchStep", "step": step, "role": role, "unit": unit, "phase": phase,
        }))));
    }
}

fn evidence(
    request: BatchRequest,
    helper: uuid::Uuid,
    outcome: Outcome,
    prior: u32,
) -> BatchEvidence {
    let mut evidence = BatchEvidence::new(request);
    if outcome != Outcome::Rejected {
        let roles = match &evidence.request {
            BatchRequest::Batch { owner, .. } => {
                let mut roles = vec![Role::Context, Role::Settings];
                if owner.is_none() {
                    roles.extend([Role::Begin, Role::Transaction]);
                }
                for role in roles {
                    complete(&mut evidence, role, None);
                }
                complete(&mut evidence, Role::Unit, Some(0));
                vec![Role::Verify]
            }
            BatchRequest::Finish {
                action: FinishAction::Commit,
                ..
            } => {
                vec![
                    Role::Context,
                    Role::Settings,
                    Role::Commit,
                    Role::Idle,
                    Role::Disposition,
                ]
            }
            BatchRequest::Finish {
                action: FinishAction::Rollback,
                ..
            } => {
                vec![Role::Rollback, Role::Idle, Role::Disposition]
            }
        };
        for role in roles {
            complete(&mut evidence, role, None);
        }
    }
    assert!(evidence.accept_terminal(&json!({
        "outcome": outcome, "owner": helper,
        "transactionId": if outcome == Outcome::Rejected { None } else { Some("123") },
        "failure": if outcome == Outcome::Rejected { Some("admission") } else { None },
        "priorBatches": prior,
    })));
    evidence
}

fn record(
    store: &Persistence,
    owner: &HistoryOwner,
    gate: &OperationGate,
    reservation: &BatchReservation,
    evidence: &BatchEvidence,
) -> Vec<u8> {
    let original = OriginalBatchOutcome::capture_in(
        reservation,
        evidence,
        BatchTermination::Complete,
        2,
        Vec::with_capacity(MAX_ENVELOPE_BYTES),
    )
    .unwrap();
    store
        .capture_batch_outcome(owner, gate, reservation, &original)
        .unwrap();
    store
        .settle_batch_outcome(owner, gate, reservation, &original)
        .unwrap();
    original.as_bytes().to_vec()
}

fn start(
    store: &Persistence,
    owner: &Arc<HistoryOwner>,
    gate: &OperationGate,
    descriptor: &BatchDescriptor,
    helper: uuid::Uuid,
    outcome: Outcome,
) -> (BatchReservation, ManualTransactionLease, Vec<u8>) {
    let plan = BatchPlan::new(request(None)).unwrap();
    let reservation = store
        .reserve_owned_batch(owner, gate, descriptor, &plan, None)
        .unwrap();
    let group = reservation.manual_transaction().unwrap();
    let bytes = record(
        store,
        owner,
        gate,
        &reservation,
        &evidence(plan.request().clone(), helper, outcome, 0),
    );
    validate_records(&store.connection.lock()).unwrap();
    (reservation, group, bytes)
}

fn finish(
    store: &Persistence,
    owner: &Arc<HistoryOwner>,
    gate: &OperationGate,
    descriptor: &BatchDescriptor,
    group: &ManualTransactionLease,
    helper: uuid::Uuid,
    action: FinishAction,
) -> BatchReservation {
    let reservation = store
        .reserve_owned_batch_finish(owner, gate, descriptor, group, action)
        .unwrap();
    let outcome = match action {
        FinishAction::Commit => Outcome::Committed,
        FinishAction::Rollback => Outcome::RolledBack,
    };
    record(
        store,
        owner,
        gate,
        &reservation,
        &evidence(
            BatchRequest::Finish {
                owner: helper,
                action,
            },
            helper,
            outcome,
            1,
        ),
    );
    validate_records(&store.connection.lock()).unwrap();
    reservation
}

// Strip local authority exactly as a historical import does. Malformed cases
// below change group history, rather than depending on a live owner's rights.
fn historical(c: &Connection) {
    validate_records(c).unwrap();
    c.execute("UPDATE batch_captures_v8 SET provenance='imported'", [])
        .unwrap();
    c.execute("DELETE FROM batch_operations_v8", []).unwrap();
    c.execute("DELETE FROM manual_transaction_bindings_v8", [])
        .unwrap();
    c.execute(
        "UPDATE manual_transactions_v8 SET disposition='unknown' WHERE disposition='open'",
        [],
    )
    .unwrap();
    validate_records(c).unwrap();
}

fn canonical_bytes(metadata: &EnvelopeMetadata, evidence: &BatchEvidence) -> Vec<u8> {
    let binary = BatchCapture::encode(evidence).unwrap();
    let bytes = encode_envelope(metadata, &binary, Vec::with_capacity(MAX_ENVELOPE_BYTES)).unwrap();
    assert_eq!(
        decode(&bytes).unwrap(),
        (metadata.clone(), evidence.clone())
    );
    bytes
}

fn replace_archive(c: &Connection, metadata: &EnvelopeMetadata, evidence: &BatchEvidence) {
    let bytes = canonical_bytes(metadata, evidence);
    let member = metadata.membership.as_ref().unwrap();
    assert_eq!(
        c.execute(
            "UPDATE batch_captures_v8 SET capture_digest=?1,envelope=?2 WHERE operation_id=?3",
            params![hash(&bytes), bytes, metadata.descriptor.operation_id],
        )
        .unwrap(),
        1
    );
    assert_eq!(c.execute(
        "UPDATE manual_transaction_members_v8 SET ordinal=?1,prior_pending_count=?2,request_hash=?3 WHERE operation_id=?4",
        params![member.ordinal, member.prior_pending_count, metadata.request_hash, metadata.descriptor.operation_id],
    ).unwrap(), 1);
    // Each envelope, digest, request and member association remains valid alone.
    validate_archive(c, &metadata.descriptor.operation_id).unwrap();
}

fn append_pending(
    c: &Connection,
    descriptor: BatchDescriptor,
    group: &str,
    helper: uuid::Uuid,
    ordinal: u32,
    prior: u32,
) {
    let evidence = evidence(request(Some(helper)), helper, Outcome::Pending, prior);
    let metadata = EnvelopeMetadata {
        version: 1,
        descriptor,
        intent: Intent::ContinueManual,
        membership: Some(Membership {
            transaction_id: group.into(),
            ordinal,
            prior_pending_count: prior,
        }),
        request_hash: request_hash(&evidence.request).unwrap(),
        finished_at: 2,
        termination: BatchTermination::Complete,
    };
    let bytes = canonical_bytes(&metadata, &evidence);
    c.execute(
        "INSERT INTO manual_transaction_members_v8 VALUES(?1,?2,?3,?4,?5)",
        params![
            metadata.descriptor.operation_id,
            group,
            ordinal,
            prior,
            metadata.request_hash
        ],
    )
    .unwrap();
    archive(c, &metadata, &bytes, "ordinary", "imported", "settled").unwrap();
    outcomes::audit_once(c, &audit(&metadata.descriptor, None, metadata.intent)).unwrap();
    outcomes::audit_once(
        c,
        &audit(
            &metadata.descriptor,
            Some((metadata.finished_at, outcome(&evidence))),
            metadata.intent,
        ),
    )
    .unwrap();
    validate_archive(c, &metadata.descriptor.operation_id).unwrap();
}

#[test]
fn batch_validation_rejects_finish_as_first_manual_member() {
    for action in [FinishAction::Commit, FinishAction::Rollback] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let descriptor = descriptor();
        let helper = uuid::Uuid::new_v4();
        let (first, group, _) = start(&store, &owner, &gate, &descriptor, helper, Outcome::Pending);
        let close = finish(
            &store,
            &owner,
            &gate,
            &next(&descriptor),
            &group,
            helper,
            action,
        );
        let c = store.connection.lock();
        historical(&c);
        let (mut metadata, mut evidence) = validate_archive(&c, close.operation_id()).unwrap();
        c.execute(
            "DELETE FROM batch_captures_v8 WHERE operation_id=?1",
            [first.operation_id()],
        )
        .unwrap();
        c.execute(
            "DELETE FROM manual_transaction_members_v8 WHERE operation_id=?1",
            [first.operation_id()],
        )
        .unwrap();
        let member = metadata.membership.as_mut().unwrap();
        member.ordinal = 1;
        member.prior_pending_count = 0;
        evidence.summary.as_mut().unwrap().prior_batches = 0;
        // Keep the graft's individual audit tuples valid; only its position in
        // the group's history is contradictory.
        assert_eq!(
            c.execute(
                "UPDATE audit_v8 SET action='snowflake-mutation-finish' WHERE id IN(?1,?2)",
                params![
                    format!("{}:started", close.operation_id()),
                    format!("{}:finished", close.operation_id())
                ],
            )
            .unwrap(),
            2
        );
        outcomes::audit_once(&c, &audit(&metadata.descriptor, None, metadata.intent)).unwrap();
        outcomes::audit_once(
            &c,
            &audit(
                &metadata.descriptor,
                Some((metadata.finished_at, outcome(&evidence))),
                metadata.intent,
            ),
        )
        .unwrap();
        replace_archive(&c, &metadata, &evidence);
        let mut identity = read_group(&c, group.id()).unwrap().unwrap().identity;
        identity.first_operation = close.operation_id().into();
        c.execute(
            "UPDATE manual_transactions_v8 SET identity_json=?1,next_ordinal=1,pending_count=0 WHERE transaction_id=?2",
            params![serde_json::to_string(&identity).unwrap(), group.id()],
        ).unwrap();
        assert!(
            validate_records(&c).is_err(),
            "a first {action:?} cannot establish a manual transaction"
        );
    }
}

#[test]
fn batch_validation_rejects_continuation_after_rejected_initial_member() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let descriptor = descriptor();
    let helper = uuid::Uuid::new_v4();
    let (_first, group, _) = start(
        &store,
        &owner,
        &gate,
        &descriptor,
        helper,
        Outcome::Rejected,
    );
    let continuation = BatchPlan::new(request(Some(helper))).unwrap();
    assert!(
        store
            .reserve_owned_batch(
                &owner,
                &gate,
                &next(&descriptor),
                &continuation,
                Some(&group)
            )
            .is_err()
    );
    let c = store.connection.lock();
    historical(&c);
    append_pending(&c, next(&descriptor), group.id(), helper, 2, 0);
    c.execute(
        "UPDATE manual_transactions_v8 SET next_ordinal=2,pending_count=1,remote_transaction='123',disposition='unknown',closed_by=NULL WHERE transaction_id=?1",
        [group.id()],
    ).unwrap();
    assert!(
        validate_records(&c).is_err(),
        "a rejected initial attempt leaves no pending transaction to continue"
    );
}

#[test]
fn batch_validation_rejects_pending_member_after_verified_close() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let descriptor = descriptor();
    let helper = uuid::Uuid::new_v4();
    let (first, group, first_bytes) =
        start(&store, &owner, &gate, &descriptor, helper, Outcome::Pending);
    let close = finish(
        &store,
        &owner,
        &gate,
        &next(&descriptor),
        &group,
        helper,
        FinishAction::Commit,
    );
    let c = store.connection.lock();
    historical(&c);
    append_pending(&c, next(&descriptor), group.id(), helper, 3, 1);
    c.execute(
        "UPDATE manual_transactions_v8 SET next_ordinal=3,pending_count=2 WHERE transaction_id=?1",
        [group.id()],
    )
    .unwrap();
    let group_row = read_group(&c, group.id()).unwrap().unwrap();
    assert_eq!(group_row.disposition, "committed");
    assert_eq!(group_row.closed_by.as_deref(), Some(close.operation_id()));
    assert_eq!(
        read_archive(&c, first.operation_id()).unwrap().unwrap().1,
        first_bytes
    );
    assert!(
        validate_records(&c).is_err(),
        "a verified close cannot coexist with a later pending member"
    );
}

#[test]
fn batch_validation_rejects_missing_or_changed_settled_audit() {
    for mutation in [
        "DELETE FROM audit_v8 WHERE id=?1",
        "UPDATE audit_v8 SET outcome='committed' WHERE id=?1",
        "UPDATE audit_v8 SET connection_id='different-fixture' WHERE id=?1",
    ] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let descriptor = descriptor();
        let (first, _group, bytes) = start(
            &store,
            &owner,
            &gate,
            &descriptor,
            uuid::Uuid::new_v4(),
            Outcome::Pending,
        );
        let c = store.connection.lock();
        assert_eq!(
            c.execute(mutation, [format!("{}:finished", first.operation_id())])
                .unwrap(),
            1
        );
        assert_eq!(
            read_archive(&c, first.operation_id()).unwrap().unwrap().1,
            bytes
        );
        assert_eq!(
            decode(&bytes).unwrap().1.summary.unwrap().outcome,
            Outcome::Pending
        );
        assert!(
            validate_records(&c).is_err(),
            "settled original must retain its exact audit tuple: {mutation}"
        );
    }
}

#[test]
fn batch_validation_preserves_unknown_and_released_owner_snapshots() {
    for no_original in [false, true] {
        let (_root, store, owner) = setup();
        let gate = store.operation_gate(&owner).unwrap();
        let descriptor = descriptor();
        let helper = uuid::Uuid::new_v4();
        let (first, group, first_bytes) =
            start(&store, &owner, &gate, &descriptor, helper, Outcome::Pending);
        let plan = BatchPlan::new(request(Some(helper))).unwrap();
        let interrupted = store
            .reserve_owned_batch(&owner, &gate, &next(&descriptor), &plan, Some(&group))
            .unwrap();
        if no_original {
            store
                .abandon_reserved_batch(&owner, &gate, &interrupted)
                .unwrap();
        } else {
            let original = OriginalBatchOutcome::capture_in(
                &interrupted,
                &BatchEvidence::new(plan.request().clone()),
                BatchTermination::Eof,
                2,
                Vec::with_capacity(MAX_ENVELOPE_BYTES),
            )
            .unwrap();
            store
                .capture_batch_outcome(&owner, &gate, &interrupted, &original)
                .unwrap();
            store
                .settle_batch_outcome(&owner, &gate, &interrupted, &original)
                .unwrap();
        }
        let group_id = group.id().to_owned();
        let first_id = first.operation_id().to_owned();
        let interrupted_id = interrupted.operation_id().to_owned();
        validate_records(&store.connection.lock()).unwrap();
        assert!(
            binding(&store.connection.lock(), &group_id)
                .unwrap()
                .is_some()
        );
        drop(interrupted);
        drop(first);
        drop(group);
        drop(gate);
        drop(owner);
        // Releasing every owned lease makes this a genuinely dead local owner;
        // startup recovery must retain uncertainty without inventing a close.
        let _observer = store.claim_history_owner().unwrap();
        validate_records(&store.connection.lock()).unwrap();
        let page = store.manual_transaction_evidence(&group_id, 0, 64).unwrap();
        assert_eq!(page.disposition, "unknown");
        assert_eq!(page.pending_count, 1);
        assert_eq!(page.members.len(), 2);
        assert!(
            binding(&store.connection.lock(), &group_id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store.batch_evidence(&first_id).unwrap().unwrap().as_bytes(),
            first_bytes
        );
        let c = store.connection.lock();
        historical(&c);
        if !no_original {
            // A foreign captured original may predate both audit projections;
            // importing it must not manufacture either a start or a finish.
            c.execute(
                "UPDATE batch_captures_v8 SET projection='captured' WHERE operation_id=?1",
                [&interrupted_id],
            )
            .unwrap();
            c.execute(
                "DELETE FROM audit_v8 WHERE id IN(?1,?2)",
                params![
                    format!("{interrupted_id}:started"),
                    format!("{interrupted_id}:finished")
                ],
            )
            .unwrap();
        }
        // With no original, historical membership legitimately survives after
        // its source operation/owner rows are stripped instead.
        validate_records(&c).unwrap();
        assert_eq!(read_archive(&c, &first_id).unwrap().unwrap().1, first_bytes);
    }
}
