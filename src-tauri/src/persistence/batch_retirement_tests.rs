//! Actual native child lifecycle and isolated SQLite with synthetic receipts.
//! The child is a framed hello followed by cat; no SDK, account or SQL is used.
use super::tests::{descriptor, evidence, next, original, record, request, setup};
use super::*;
use crate::drivers::snowflake_process::{Bounds, Launch, Supervisor};
use std::{os::unix::fs::PermissionsExt, time::Duration};

async fn helper(generation: &str) -> Supervisor {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().canonicalize().unwrap();
    let script = directory.join("owned-retirement-child.sh");
    let hello = serde_json::to_vec(&serde_json::json!({
        "v":1,"type":"hello","protocol":"joinery-snowflake","sdkVersion":"3.4.1",
        "limits":{"frameBytes":1_048_576,"batchRows":128,"batchBytes":262_144,"queued":8,"credits":32}
    }))
    .unwrap();
    let mut frame = (hello.len() as u32).to_be_bytes().to_vec();
    frame.extend(hello);
    let escaped: String = frame.iter().map(|byte| format!("\\{byte:03o}")).collect();
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '{escaped}'\nexec /bin/cat\n"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let launch = Launch::new(script, None, directory, root).unwrap();
    Supervisor::spawn(launch, generation, Bounds::default())
        .await
        .unwrap()
}

async fn reap(helper: &Supervisor) -> ReapedHelperGeneration {
    helper.request_retirement();
    assert!(
        helper
            .cleanup_observer()
            .wait(Duration::from_secs(5))
            .await
            .is_some_and(|receipt| receipt.child_started && receipt.reaped)
    );
    helper.reaped_generation().expect("private reaper proof")
}

fn reserve_pinned(
    store: &Persistence,
    owner: &Arc<HistoryOwner>,
    gate: &OperationGate,
    helper: &Supervisor,
) -> (BatchDescriptor, BatchReservation, ManualTransactionLease) {
    let mut d = descriptor();
    d.helper_generation = helper.helper_generation().generation().into();
    let plan = BatchPlan::new(request(None, 1)).unwrap();
    let r = store
        .reserve_owned_batch(owner, gate, &d, &plan, None)
        .unwrap();
    store
        .pin_manual_transaction_helper(owner, gate, &r, &helper.helper_generation())
        .unwrap();
    let group = r.manual_transaction().unwrap();
    (d, r, group)
}

fn pending(
    store: &Persistence,
    owner: &Arc<HistoryOwner>,
    gate: &OperationGate,
    r: &BatchReservation,
    helper_owner: uuid::Uuid,
) -> OriginalBatchOutcome {
    let o = original(
        r,
        &evidence(request(None, 1), helper_owner, Outcome::Pending, 0),
    );
    record(store, owner, gate, r, &o);
    o
}

fn audits(store: &Persistence) -> serde_json::Value {
    serde_json::to_value(store.audit_log().unwrap()).unwrap()
}

fn assert_conflict<T>(result: AppResult<T>) {
    assert!(matches!(result, Err(error) if error.code == ErrorCode::Conflict));
}

#[tokio::test(flavor = "current_thread")]
async fn batch_helper_retirement_survives_supervisor_drop_and_waits_for_original() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let process = helper(&uuid::Uuid::new_v4().to_string()).await;
    let (_d, reservation, group) = reserve_pinned(&store, &owner, &gate, &process);
    let observer = process.retirement_observer();
    let original = original(
        &reservation,
        &evidence(request(None, 1), uuid::Uuid::new_v4(), Outcome::Pending, 0),
    );
    let exact_original = original.as_bytes().to_vec();
    assert!(observer.reaped_generation().is_none());
    drop(process);
    // Same-thread reaping has not run yet; dropping the admission owner alone
    // cannot release the recorded transaction's native binding.
    assert!(observer.reaped_generation().is_none());
    let proof = observer
        .wait(Duration::from_secs(5))
        .await
        .expect("private proof remains available after supervisor drop");
    assert!(observer.helper_generation().matches_reaped(&proof));
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &proof));
    store
        .capture_batch_outcome(&owner, &gate, &reservation, &original)
        .unwrap();
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &proof));
    store
        .settle_batch_outcome(&owner, &gate, &reservation, &original)
        .unwrap();
    let before_audits = audits(&store);
    assert_eq!(
        store
            .retire_manual_transaction(&owner, &gate, &group, &proof)
            .unwrap(),
        ManualTransactionRetirement::Retired
    );
    assert_eq!(audits(&store), before_audits);
    assert_eq!(
        store
            .batch_evidence(reservation.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        exact_original
    );
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
            .is_none()
    );
    assert!(!owner.batch_credits.lock().contains_key(group.id()));
    validate_records(&store.connection.lock()).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn batch_helper_retirement_capture_serializes_shared_admission_across_handles() {
    let (root, store, owner) = setup();
    let other = Persistence::open(root.path()).unwrap();
    let gate = store.operation_gate(&owner).unwrap();
    let process = helper(&uuid::Uuid::new_v4().to_string()).await;
    let (_d, r, group) = reserve_pinned(&store, &owner, &gate, &process);
    let native = process.helper_generation();
    let o = original(
        &r,
        &evidence(request(None, 1), uuid::Uuid::new_v4(), Outcome::Pending, 0),
    );
    let exact_original = o.as_bytes().to_vec();
    // Reap before the intended admission assertion so even the RED failure has
    // no outstanding owned child. This tests local serialization, not remote ACKs.
    reap(&process).await;
    {
        let _admission = group.memory.1.lock();
        assert_conflict(other.capture_batch_outcome(&owner, &gate, &r, &o));
        assert_eq!(
            read_row(&store.connection.lock(), r.operation_id())
                .unwrap()
                .unwrap()
                .phase,
            "reserved"
        );
    }
    assert_eq!(o.as_bytes(), exact_original);
    other.capture_batch_outcome(&owner, &gate, &r, &o).unwrap();
    assert_conflict(store.pin_manual_transaction_helper(&owner, &gate, &r, &native));
    store.settle_batch_outcome(&owner, &gate, &r, &o).unwrap();
    assert_eq!(
        other
            .batch_evidence(r.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        exact_original
    );
    validate_records(&other.connection.lock()).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn batch_helper_retirement_matches_physical_child_and_preserves_originals() {
    let studio_pid = std::process::id();
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let generation = uuid::Uuid::new_v4().to_string();
    let first = helper(&generation).await;
    let foreign = helper(&generation).await;
    let (d, r, group) = reserve_pinned(&store, &owner, &gate, &first);
    // An exact physical retry is allowed; an identical wire UUID is insufficient.
    store
        .pin_manual_transaction_helper(&owner, &gate, &r, &first.helper_generation())
        .unwrap();
    assert_conflict(store.pin_manual_transaction_helper(
        &owner,
        &gate,
        &r,
        &foreign.helper_generation(),
    ));
    let helper_owner = uuid::Uuid::new_v4();
    let o = pending(&store, &owner, &gate, &r, helper_owner);
    let (other_d, other_r, other_group) = reserve_pinned(&store, &owner, &gate, &foreign);
    let other_o = pending(&store, &owner, &gate, &other_r, uuid::Uuid::new_v4());
    let before_audits = audits(&store);
    let before_page = store
        .manual_transaction_evidence(group.id(), 0, 64)
        .unwrap();
    let before = read_group(&store.connection.lock(), group.id())
        .unwrap()
        .unwrap();
    assert!(first.reaped_generation().is_none());
    first.request_retirement();
    // No await: the same-thread native reaper cannot yet have published death.
    assert!(first.reaped_generation().is_none());
    let proof = reap(&first).await;
    assert_eq!(std::process::id(), studio_pid);
    assert!(!foreign.is_closed());
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &other_group, &proof));
    {
        let _busy = group.memory.1.lock();
        assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &proof));
    }
    assert_eq!(
        store
            .retire_manual_transaction(&owner, &gate, &group, &proof)
            .unwrap(),
        ManualTransactionRetirement::Retired
    );
    assert_eq!(
        store
            .retire_manual_transaction(&owner, &gate, &group, &proof)
            .unwrap(),
        ManualTransactionRetirement::AlreadyRetired
    );
    assert_eq!(audits(&store), before_audits);
    for (id, bytes) in [
        (d.operation_id.as_str(), o.as_bytes()),
        (other_d.operation_id.as_str(), other_o.as_bytes()),
    ] {
        assert_eq!(store.batch_evidence(id).unwrap().unwrap().as_bytes(), bytes);
    }
    let page = store
        .manual_transaction_evidence(group.id(), 0, 64)
        .unwrap();
    assert_eq!(page.disposition, "unknown");
    assert_eq!(page.pending_count, before_page.pending_count);
    assert_eq!(page.members.len(), before_page.members.len());
    assert_eq!(
        page.members[0].operation_id,
        before_page.members[0].operation_id
    );
    let after = read_group(&store.connection.lock(), group.id())
        .unwrap()
        .unwrap();
    assert_eq!(after.identity, before.identity);
    assert_eq!(after.helper, before.helper);
    assert_eq!(after.remote, before.remote);
    assert_eq!(after.closed_by, before.closed_by);
    assert!(
        binding(&store.connection.lock(), group.id())
            .unwrap()
            .is_none()
    );
    assert!(
        binding(&store.connection.lock(), other_group.id())
            .unwrap()
            .unwrap()
            .1
    );
    assert_eq!(
        store
            .manual_transaction_evidence(other_group.id(), 0, 64)
            .unwrap()
            .disposition,
        "open"
    );
    assert!(!owner.batch_credits.lock().contains_key(group.id()));
    let continuation = BatchPlan::new(request(Some(helper_owner), 1)).unwrap();
    assert_conflict(store.reserve_owned_batch(
        &owner,
        &gate,
        &next(&d),
        &continuation,
        Some(&group),
    ));
    assert_conflict(store.make_volatile_batch_rollback_token(
        &owner,
        &gate,
        &next(&d),
        &group,
        helper_owner,
    ));
    let foreign_owner = Arc::new(store.claim_history_owner().unwrap());
    let foreign_gate = store.operation_gate(&foreign_owner).unwrap();
    assert_conflict(store.retire_manual_transaction(&foreign_owner, &foreign_gate, &group, &proof));
    validate_records(&store.connection.lock()).unwrap();
    let foreign_proof = reap(&foreign).await;
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &foreign_proof));
}

#[tokio::test(flavor = "current_thread")]
async fn batch_helper_retirement_waits_for_reserved_and_captured_original_obligations() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let process = helper(&uuid::Uuid::new_v4().to_string()).await;
    let (_d, r, group) = reserve_pinned(&store, &owner, &gate, &process);
    let o = original(
        &r,
        &evidence(request(None, 1), uuid::Uuid::new_v4(), Outcome::Pending, 0),
    );
    let proof = reap(&process).await;
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &proof));
    assert_eq!(
        read_row(&store.connection.lock(), r.operation_id())
            .unwrap()
            .unwrap()
            .phase,
        "reserved"
    );
    store.capture_batch_outcome(&owner, &gate, &r, &o).unwrap();
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &proof));
    assert_eq!(
        read_row(&store.connection.lock(), r.operation_id())
            .unwrap()
            .unwrap()
            .payload
            .as_deref(),
        Some(o.as_bytes())
    );
    assert_eq!(store.audit_log().unwrap().len(), 1);
    // Guard-only snapshot corruption fixture: DB obligations independently veto
    // retirement even if an in-memory task marker were accidentally absent.
    let in_flight = group.memory.0.lock().in_flight.take().unwrap();
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &proof));
    group.memory.0.lock().in_flight = Some(in_flight);
    store.settle_batch_outcome(&owner, &gate, &r, &o).unwrap();
    store
        .retire_manual_transaction(&owner, &gate, &group, &proof)
        .unwrap();
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

#[tokio::test(flavor = "current_thread")]
async fn batch_helper_retirement_preserves_volatile_original_then_releases_settled_unknown_credit()
{
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let process = helper(&uuid::Uuid::new_v4().to_string()).await;
    let (d, r, group) = reserve_pinned(&store, &owner, &gate, &process);
    let helper_owner = uuid::Uuid::new_v4();
    let first = pending(&store, &owner, &gate, &r, helper_owner);
    let rollback_d = next(&d);
    let token = store
        .make_volatile_batch_rollback_token(&owner, &gate, &rollback_d, &group, helper_owner)
        .unwrap();
    let e = evidence(
        token.plan.request.clone(),
        helper_owner,
        Outcome::Rejected,
        1,
    );
    let o = OriginalBatchOutcome::volatile_rollback_in(
        &token,
        &e,
        BatchTermination::Complete,
        3,
        Vec::with_capacity(MAX_ENVELOPE_BYTES),
    )
    .unwrap();
    let proof = reap(&process).await;
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &proof));
    assert!(
        read_row(&store.connection.lock(), &rollback_d.operation_id)
            .unwrap()
            .is_none()
    );
    let rollback = store
        .promote_volatile_batch_rollback(&owner, &gate, &token)
        .unwrap();
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &proof));
    record(&store, &owner, &gate, &rollback, &o);
    assert!(group.memory.0.lock().credit_operation.is_some());
    let before = read_group(&store.connection.lock(), group.id())
        .unwrap()
        .unwrap();
    assert_eq!(before.disposition, "unknown");
    assert_eq!(before.closed_by.as_deref(), Some(rollback.operation_id()));
    let before_audits = audits(&store);
    store
        .retire_manual_transaction(&owner, &gate, &group, &proof)
        .unwrap();
    assert_eq!(audits(&store), before_audits);
    assert_eq!(
        store
            .batch_evidence(r.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        first.as_bytes()
    );
    assert_eq!(
        store
            .batch_evidence(rollback.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        o.as_bytes()
    );
    assert_eq!(
        read_group(&store.connection.lock(), group.id())
            .unwrap()
            .unwrap()
            .closed_by,
        before.closed_by
    );
    assert!(group.memory.0.lock().credit_operation.is_none());
    assert_conflict(store.promote_volatile_batch_rollback(&owner, &gate, &token));
    assert_conflict(store.make_volatile_batch_rollback_token(
        &owner,
        &gate,
        &next(&d),
        &group,
        helper_owner,
    ));
    validate_records(&store.connection.lock()).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn batch_helper_retirement_sqlite_failure_preserves_authority_and_retry() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let process = helper(&uuid::Uuid::new_v4().to_string()).await;
    let (_d, r, group) = reserve_pinned(&store, &owner, &gate, &process);
    let o = pending(&store, &owner, &gate, &r, uuid::Uuid::new_v4());
    let proof = reap(&process).await;
    let before_audits = audits(&store);
    store.connection.lock().execute_batch("CREATE TEMP TRIGGER fail_retirement BEFORE DELETE ON manual_transaction_bindings_v8 BEGIN SELECT RAISE(ABORT,'retirement failure'); END;").unwrap();
    assert!(
        store
            .retire_manual_transaction(&owner, &gate, &group, &proof)
            .is_err()
    );
    assert_eq!(
        read_group(&store.connection.lock(), group.id())
            .unwrap()
            .unwrap()
            .disposition,
        "open"
    );
    assert!(
        binding(&store.connection.lock(), group.id())
            .unwrap()
            .unwrap()
            .1
    );
    {
        let state = group.memory.0.lock();
        assert!(state.open);
        assert!(!state.helper_retired);
        assert!(state.native_helper.as_ref().unwrap().matches_reaped(&proof));
    }
    assert!(owner.batch_credits.lock().contains_key(group.id()));
    store
        .connection
        .lock()
        .execute_batch("DROP TRIGGER fail_retirement")
        .unwrap();
    assert_eq!(
        store
            .retire_manual_transaction(&owner, &gate, &group, &proof)
            .unwrap(),
        ManualTransactionRetirement::Retired
    );
    assert_eq!(
        store
            .retire_manual_transaction(&owner, &gate, &group, &proof)
            .unwrap(),
        ManualTransactionRetirement::AlreadyRetired
    );
    assert_eq!(audits(&store), before_audits);
    assert_eq!(
        store
            .batch_evidence(r.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        o.as_bytes()
    );
    validate_records(&store.connection.lock()).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn batch_helper_retirement_restore_exclusion_ends_only_after_release() {
    let (_root, store, owner) = setup();
    let backup = store.create_metadata_backup().unwrap();
    let process = helper(&uuid::Uuid::new_v4().to_string()).await;
    let gate = store.operation_gate(&owner).unwrap();
    let (_d, r, group) = reserve_pinned(&store, &owner, &gate, &process);
    let o = pending(&store, &owner, &gate, &r, uuid::Uuid::new_v4());
    drop(gate);
    let proof = reap(&process).await;
    assert_conflict(store.restore_metadata_backup(&backup.id, &backup.digest));
    let gate = store.operation_gate(&owner).unwrap();
    store
        .retire_manual_transaction(&owner, &gate, &group, &proof)
        .unwrap();
    drop(gate);
    store
        .restore_metadata_backup(&backup.id, &backup.digest)
        .unwrap();
    assert_eq!(
        store
            .batch_evidence(r.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        o.as_bytes()
    );
    assert!(
        binding(&store.connection.lock(), group.id())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .manual_transaction_evidence(group.id(), 0, 64)
            .unwrap()
            .disposition,
        "unknown"
    );
    validate_records(&store.connection.lock()).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn batch_helper_retirement_refuses_unpinned_groups_and_preserves_verified_closure() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let process = helper(&uuid::Uuid::new_v4().to_string()).await;
    let (d, r, group) = reserve_pinned(&store, &owner, &gate, &process);
    let helper_owner = uuid::Uuid::new_v4();
    pending(&store, &owner, &gate, &r, helper_owner);
    let close = store
        .reserve_owned_batch_finish(&owner, &gate, &next(&d), &group, FinishAction::Rollback)
        .unwrap();
    let o = original(
        &close,
        &evidence(
            BatchRequest::Finish {
                owner: helper_owner,
                action: FinishAction::Rollback,
            },
            helper_owner,
            Outcome::RolledBack,
            1,
        ),
    );
    record(&store, &owner, &gate, &close, &o);
    let mut unpinned_d = descriptor();
    unpinned_d.helper_generation = d.helper_generation.clone();
    let plan = BatchPlan::new(request(None, 1)).unwrap();
    let unpinned = store
        .reserve_owned_batch(&owner, &gate, &unpinned_d, &plan, None)
        .unwrap();
    let unpinned_group = unpinned.manual_transaction().unwrap();
    pending(&store, &owner, &gate, &unpinned, uuid::Uuid::new_v4());
    assert_conflict(store.pin_manual_transaction_helper(
        &owner,
        &gate,
        &unpinned,
        &process.helper_generation(),
    ));
    let before_audits = audits(&store);
    let proof = reap(&process).await;
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &group, &proof));
    assert_conflict(store.retire_manual_transaction(&owner, &gate, &unpinned_group, &proof));
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
    assert!(
        binding(&store.connection.lock(), unpinned_group.id())
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store
            .batch_evidence(close.operation_id())
            .unwrap()
            .unwrap()
            .as_bytes(),
        o.as_bytes()
    );
    assert_eq!(audits(&store), before_audits);
    validate_records(&store.connection.lock()).unwrap();
}
