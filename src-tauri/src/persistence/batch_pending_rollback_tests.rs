//! Real isolated SQLite and actual owned framed native children, with synthetic
//! Pending receipts. No SDK, account, engine SQL, or transaction qualification.
use super::tests::{descriptor, evidence, next, original, record, request, setup};
use super::*;
use crate::drivers::snowflake_batch::StepPhase;
use crate::drivers::snowflake_process::{Bounds, Impact, Launch, Operation, Supervisor};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};

fn frame(value: Value) -> String {
    let body = serde_json::to_vec(&value).unwrap();
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    bytes.iter().map(|b| format!("\\{b:03o}")).collect()
}
fn responses(generation: &str, id: &str, e: &BatchEvidence) -> String {
    let mut frames = frame(json!({"v":1,"generation":generation,"id":id,"type":"accepted"}));
    for step in &e.steps {
        let base = json!({"v":1,"generation":generation,"id":id,"type":"batchStep","step":step.step,"role":step.role,"unit":step.unit,"phase":StepPhase::Dispatching});
        frames.push_str(&frame(base.clone()));
        if let Some(identity) = &step.statement {
            frames.push_str(&frame(json!({"v":1,"generation":generation,"id":id,"type":"batchStatement","step":step.step,"sdkRequestId":identity.sdk_request_id,"queryId":identity.query_id})));
        }
        let mut ack = base.clone();
        ack["phase"] = json!(StepPhase::Acknowledged);
        frames.push_str(&frame(ack));
        if let Some(count) = &step.count {
            frames.push_str(&frame(json!({"v":1,"generation":generation,"id":id,"type":"batchCount","step":step.step,"unit":step.unit,"affectedRows":count.affected_rows,"multiJoinedRows":count.multi_joined_rows})));
        }
        let mut drained = base;
        drained["phase"] = json!(StepPhase::Drained);
        frames.push_str(&frame(drained));
    }
    frames.push_str(&frame(json!({"v":1,"generation":generation,"id":id,"type":"terminal","ok":true,"result":e.summary})));
    frames
}
async fn helper(generation: &str, pending: &[BatchEvidence]) -> Supervisor {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().canonicalize().unwrap();
    let script = directory.join("pending-rollback-child.sh");
    let hello = frame(
        json!({"v":1,"type":"hello","protocol":"joinery-snowflake","sdkVersion":"3.4.1","limits":{"frameBytes":1048576,"batchRows":128,"batchBytes":262144,"queued":8,"credits":32}}),
    );
    // Consume each complete native frame before replying; stdout stays open.
    let read = "read_frame() { set -- $(/bin/dd bs=1 count=4 2>/dev/null | /usr/bin/od -An -t u1); [ \"$#\" = 4 ] || exit 2; n=$(($1*16777216+$2*65536+$3*256+$4)); /bin/dd bs=1 count=\"$n\" of=/dev/null 2>/dev/null; }\n";
    let connected = frame(json!({"v":1,"generation":generation,"id":"1","type":"accepted"}))
        + &frame(
            json!({"v":1,"generation":generation,"id":"1","type":"terminal","ok":true,"result":{"connected":true}}),
        );
    let mut body = format!("#!/bin/sh\n{read}printf '{hello}'\nread_frame\nprintf '{connected}'\n");
    for (index, e) in pending.iter().enumerate() {
        body.push_str(&format!(
            "read_frame\nprintf '{}'\n",
            responses(generation, &(index + 2).to_string(), e)
        ));
    }
    body.push_str("exec /bin/cat\n");
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let child = Supervisor::spawn(
        Launch::new(script, None, directory, root).unwrap(),
        generation,
        Bounds::default(),
    )
    .await
    .unwrap();
    let mut connect = child
        .request("connect-fixture", "connect", json!({}), Impact::Control)
        .unwrap();
    assert!(connect.wait_bounded(Duration::from_secs(5)).await.ok);
    child
}
async fn native_operation(
    child: &Supervisor,
    r: &BatchReservation,
    request: &BatchRequest,
) -> Operation {
    let expected = json!({"user":"USER","session":"123","role":"ROLE","database":"DB","schema":"PUBLIC","warehouse":"WH"});
    let (op, payload) = match request {
        BatchRequest::Batch { owner, .. } => (
            "mutationBatch",
            json!({"mode":"manual","owner":owner,"rollbackPrior":owner.is_some(),"expected":expected,"units":[{"kind":"update","sql":"UPDATE T SET V=? WHERE V=?","binds":["2","1"]}]}),
        ),
        BatchRequest::Finish { owner, action } => (
            "mutationFinish",
            json!({"owner":owner,"action":action,"expected":expected}),
        ),
    };
    let mut operation = child
        .request(r.operation_id(), op, payload, Impact::Mutating)
        .unwrap();
    let report = operation.wait_bounded(Duration::from_secs(5)).await;
    assert!(report.ok, "{report:?}");
    operation
}
async fn pending_operation(
    child: &Supervisor,
    r: &BatchReservation,
    owner: Option<uuid::Uuid>,
) -> Operation {
    let operation = native_operation(child, r, &request(owner, 1)).await;
    assert_eq!(
        operation
            .report()
            .unwrap()
            .batch
            .unwrap()
            .summary
            .unwrap()
            .outcome,
        Outcome::Pending
    );
    operation
}
async fn reap(child: &Supervisor) {
    child.request_retirement();
    assert!(
        child
            .cleanup_observer()
            .wait(Duration::from_secs(5))
            .await
            .is_some_and(|r| r.child_started && r.reaped)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn batch_pending_original_storage_failure_keeps_protected_rollback_available() {
    for continuation in [false, true] {
        for capture_fault in [false, true] {
            let (_root, store, owner) = setup();
            let gate = store.operation_gate(&owner).unwrap();
            let mut d = descriptor();
            let helper_owner = uuid::Uuid::new_v4();
            let first_e = evidence(request(None, 1), helper_owner, Outcome::Pending, 0);
            let second_e = evidence(
                request(Some(helper_owner), 1),
                helper_owner,
                Outcome::Pending,
                1,
            );
            let pending_responses = if continuation {
                vec![first_e.clone(), second_e.clone()]
            } else {
                vec![first_e.clone()]
            };
            let child = helper(&d.helper_generation, &pending_responses).await;
            let r = store
                .reserve_owned_batch(
                    &owner,
                    &gate,
                    &d,
                    &BatchPlan::new(first_e.request.clone()).unwrap(),
                    None,
                )
                .unwrap();
            store
                .pin_manual_transaction_helper(&owner, &gate, &r, &child.helper_generation())
                .unwrap();
            let group = r.manual_transaction().unwrap();
            let operation = pending_operation(&child, &r, None).await;
            let first_o = original(&r, operation.report().unwrap().batch.as_ref().unwrap());
            let (r, operation, o) = if continuation {
                record(&store, &owner, &gate, &r, &first_o);
                d = next(&d);
                let r = store
                    .reserve_owned_batch(
                        &owner,
                        &gate,
                        &d,
                        &BatchPlan::new(second_e.request.clone()).unwrap(),
                        Some(&group),
                    )
                    .unwrap();
                let operation = pending_operation(&child, &r, Some(helper_owner)).await;
                let o = original(&r, operation.report().unwrap().batch.as_ref().unwrap());
                (r, operation, o)
            } else {
                (r, operation, first_o.clone())
            };
            let exact = o.as_bytes().to_vec();
            let trigger = if capture_fault {
                "CREATE TRIGGER storage_fault BEFORE UPDATE OF payload ON batch_operations_v8 BEGIN SELECT RAISE(FAIL,'capture'); END"
            } else {
                "CREATE TRIGGER storage_fault BEFORE INSERT ON batch_captures_v8 BEGIN SELECT RAISE(FAIL,'settle'); END"
            };
            store.connection.lock().execute_batch(trigger).unwrap();
            if capture_fault {
                assert!(store.capture_batch_outcome(&owner, &gate, &r, &o).is_err());
            } else {
                store.capture_batch_outcome(&owner, &gate, &r, &o).unwrap();
                assert!(store.settle_batch_outcome(&owner, &gate, &r, &o).is_err());
            }
            // Retire/reap the owned fixture before the intended RED assertion;
            // its immutable completed Operation remains available as proof.
            reap(&child).await;
            // Saturate the ordinary owner pool with genuine reservations;
            // the already reserved cleanup credit still needs no new slot.
            let automatic = BatchPlan::new(BatchRequest::Batch {
                mode: Mode::Automatic,
                owner: None,
                rollback_prior: false,
                units: vec![crate::drivers::snowflake_batch::UnitKind::Update],
            })
            .unwrap();
            let retained: Vec<_> = (0..31)
                .map(|_| {
                    store
                        .reserve_owned_batch(&owner, &gate, &next(&d), &automatic, None)
                        .unwrap()
                })
                .collect();
            assert_eq!(retained.len(), 31);
            assert_eq!(
                store
                    .reserve_owned_batch(&owner, &gate, &next(&d), &automatic, None)
                    .unwrap_err()
                    .code,
                ErrorCode::ResultLimitReached
            );
            let rollback_d = next(&d);
            let authority = store
                .authorize_pending_batch_rollback(&owner, &gate, &r, &o, &operation)
                .unwrap();
            let token = store.make_pending_batch_rollback_token(&owner,&gate,&rollback_d,&authority,&o).expect("actual Pending original must retain protected rollback despite local storage failure");
            // Neither the earlier failure nor this cleanup token may authorize
            // Commit/continuation or another rollback descriptor.
            assert!(
                store
                    .reserve_owned_batch_finish(
                        &owner,
                        &gate,
                        &next(&d),
                        &group,
                        FinishAction::Commit
                    )
                    .is_err()
            );
            assert!(
                store
                    .reserve_owned_batch(
                        &owner,
                        &gate,
                        &next(&d),
                        &BatchPlan::new(request(Some(helper_owner), 1)).unwrap(),
                        Some(&group)
                    )
                    .is_err()
            );
            assert!(
                store
                    .make_pending_batch_rollback_token(
                        &owner,
                        &gate,
                        &next(&rollback_d),
                        &authority,
                        &o
                    )
                    .is_err()
            );
            let rollback_e = evidence(
                BatchRequest::Finish {
                    owner: helper_owner,
                    action: FinishAction::Rollback,
                },
                helper_owner,
                Outcome::RolledBack,
                if continuation { 2 } else { 1 },
            );
            let rollback_o = OriginalBatchOutcome::volatile_rollback_in(
                &token,
                &rollback_e,
                BatchTermination::Complete,
                3,
                Vec::with_capacity(MAX_ENVELOPE_BYTES),
            )
            .unwrap();
            let exact_rollback = rollback_o.as_bytes().to_vec();
            assert!(
                store
                    .promote_volatile_batch_rollback(&owner, &gate, &token)
                    .is_err()
            );
            assert_eq!(o.as_bytes(), exact);
            assert_eq!(rollback_o.as_bytes(), exact_rollback);
            assert!(
                store
                    .batch_evidence(&rollback_d.operation_id)
                    .unwrap()
                    .is_none()
            );
            store
                .connection
                .lock()
                .execute_batch("DROP TRIGGER storage_fault")
                .unwrap();
            let rollback_r = store
                .promote_volatile_batch_rollback(&owner, &gate, &token)
                .unwrap();
            assert!(
                token.predecessor.as_ref().unwrap().lock().is_none(),
                "predecessor buffer Arc must be released after settlement, even when the token survives"
            );
            assert_eq!(
                store
                    .batch_evidence(r.operation_id())
                    .unwrap()
                    .unwrap()
                    .as_bytes(),
                exact
            );
            assert_eq!(
                read_row(&store.connection.lock(), r.operation_id())
                    .unwrap()
                    .unwrap()
                    .phase,
                "settled"
            );
            record(&store, &owner, &gate, &rollback_r, &rollback_o);
            assert_eq!(
                store
                    .batch_evidence(&rollback_d.operation_id)
                    .unwrap()
                    .unwrap()
                    .as_bytes(),
                exact_rollback
            );
            assert_eq!(
                store
                    .batch_evidence(r.operation_id())
                    .unwrap()
                    .unwrap()
                    .as_bytes(),
                exact
            );
            if continuation {
                assert_eq!(
                    store
                        .batch_evidence(
                            &decode(first_o.as_bytes())
                                .unwrap()
                                .0
                                .descriptor
                                .operation_id
                        )
                        .unwrap()
                        .unwrap()
                        .as_bytes(),
                    first_o.as_bytes()
                );
            }
            let page = store
                .manual_transaction_evidence(group.id(), 0, 64)
                .unwrap();
            assert_eq!(page.disposition, "rolledBack");
            assert_eq!(page.pending_count, if continuation { 2 } else { 1 });
            assert_eq!(page.members.len(), if continuation { 3 } else { 2 });
            assert!(
                binding(&store.connection.lock(), group.id())
                    .unwrap()
                    .is_none()
            );
            validate_records(&store.connection.lock()).unwrap();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn batch_pending_rollback_authority_rejects_foreign_same_uuid_and_changed_original() {
    let (_root, store, owner) = setup();
    let gate = store.operation_gate(&owner).unwrap();
    let d = descriptor();
    let helper_owner = uuid::Uuid::new_v4();
    let e = evidence(request(None, 1), helper_owner, Outcome::Pending, 0);
    let child = helper(&d.helper_generation, std::slice::from_ref(&e)).await;
    let foreign = helper(&d.helper_generation, std::slice::from_ref(&e)).await;
    let r = store
        .reserve_owned_batch(
            &owner,
            &gate,
            &d,
            &BatchPlan::new(e.request.clone()).unwrap(),
            None,
        )
        .unwrap();
    store
        .pin_manual_transaction_helper(&owner, &gate, &r, &child.helper_generation())
        .unwrap();
    let operation = pending_operation(&child, &r, None).await;
    let foreign_operation = pending_operation(&foreign, &r, None).await;
    let o = original(&r, &e);
    reap(&child).await;
    reap(&foreign).await;
    assert_eq!(
        store
            .authorize_pending_batch_rollback(&owner, &gate, &r, &o, &foreign_operation)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let changed = OriginalBatchOutcome::capture_in(
        &r,
        &e,
        BatchTermination::Complete,
        3,
        Vec::with_capacity(MAX_ENVELOPE_BYTES),
    )
    .unwrap();
    let authority = store
        .authorize_pending_batch_rollback(&owner, &gate, &r, &o, &operation)
        .unwrap();
    assert_eq!(
        store
            .make_pending_batch_rollback_token(&owner, &gate, &next(&d), &authority, &changed)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(
        r.group
            .as_ref()
            .unwrap()
            .memory
            .0
            .lock()
            .credit_operation
            .is_none()
    );
    assert_eq!(
        read_row(&store.connection.lock(), r.operation_id())
            .unwrap()
            .unwrap()
            .phase,
        "reserved"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn batch_pending_queued_admission_rejection_original_precedes_protected_rollback() {
    for commit in [false, true] {
        for capture_fault in [false, true] {
            let (_root, store, owner) = setup();
            let gate = store.operation_gate(&owner).unwrap();
            let d = descriptor();
            let helper_owner = uuid::Uuid::new_v4();
            let first_e = evidence(request(None, 1), helper_owner, Outcome::Pending, 0);
            let successor_request = if commit {
                BatchRequest::Finish {
                    owner: helper_owner,
                    action: FinishAction::Commit,
                }
            } else {
                request(Some(helper_owner), 1)
            };
            // Production runMutationRequest retains previous.transactionId on
            // zero-step admission refusal; it is not a fresh unknown/null ID.
            let mut rejected_e = BatchEvidence::new(successor_request.clone());
            assert!(rejected_e.accept_terminal(&json!({"outcome":"rejected","owner":helper_owner,"transactionId":"123","failure":"admission","priorBatches":1})));
            let pending_responses = vec![first_e.clone(), rejected_e.clone()];
            let child = helper(&d.helper_generation, &pending_responses).await;
            let r = store
                .reserve_owned_batch(
                    &owner,
                    &gate,
                    &d,
                    &BatchPlan::new(first_e.request.clone()).unwrap(),
                    None,
                )
                .unwrap();
            store
                .pin_manual_transaction_helper(&owner, &gate, &r, &child.helper_generation())
                .unwrap();
            let group = r.manual_transaction().unwrap();
            let operation = pending_operation(&child, &r, None).await;
            let first_o = original(&r, operation.report().unwrap().batch.as_ref().unwrap());
            let authority = store
                .authorize_pending_batch_rollback(&owner, &gate, &r, &first_o, &operation)
                .unwrap();
            record(&store, &owner, &gate, &r, &first_o);
            let successor_d = next(&d);
            let successor = if commit {
                store
                    .reserve_owned_batch_finish(
                        &owner,
                        &gate,
                        &successor_d,
                        &group,
                        FinishAction::Commit,
                    )
                    .unwrap()
            } else {
                store
                    .reserve_owned_batch(
                        &owner,
                        &gate,
                        &successor_d,
                        &BatchPlan::new(successor_request.clone()).unwrap(),
                        Some(&group),
                    )
                    .unwrap()
            };
            let rejected_operation = native_operation(&child, &successor, &successor_request).await;
            let rejected_o = original(
                &successor,
                rejected_operation.report().unwrap().batch.as_ref().unwrap(),
            );
            let exact_rejection = rejected_o.as_bytes().to_vec();
            let trigger = if capture_fault {
                "CREATE TRIGGER storage_fault BEFORE UPDATE OF payload ON batch_operations_v8 BEGIN SELECT RAISE(FAIL,'capture'); END"
            } else {
                "CREATE TRIGGER storage_fault BEFORE INSERT ON batch_captures_v8 BEGIN SELECT RAISE(FAIL,'settle'); END"
            };
            store.connection.lock().execute_batch(trigger).unwrap();
            if capture_fault {
                assert!(
                    store
                        .capture_batch_outcome(&owner, &gate, &successor, &rejected_o)
                        .is_err()
                );
            } else {
                store
                    .capture_batch_outcome(&owner, &gate, &successor, &rejected_o)
                    .unwrap();
                assert!(
                    store
                        .settle_batch_outcome(&owner, &gate, &successor, &rejected_o)
                        .is_err()
                );
            }
            reap(&child).await;
            let rollback_d = next(&successor_d);
            let token = store
                .make_rejected_batch_rollback_token(
                    &owner,
                    &gate,
                    &rollback_d,
                    &authority,
                    &successor,
                    &rejected_o,
                    &rejected_operation,
                )
                .unwrap();
            let rollback_e = evidence(
                BatchRequest::Finish {
                    owner: helper_owner,
                    action: FinishAction::Rollback,
                },
                helper_owner,
                Outcome::RolledBack,
                1,
            );
            let rollback_o = OriginalBatchOutcome::volatile_rollback_in(
                &token,
                &rollback_e,
                BatchTermination::Complete,
                3,
                Vec::with_capacity(MAX_ENVELOPE_BYTES),
            )
            .unwrap();
            assert!(
                store
                    .promote_volatile_batch_rollback(&owner, &gate, &token)
                    .is_err()
            );
            assert_eq!(rejected_o.as_bytes(), exact_rejection);
            store
                .connection
                .lock()
                .execute_batch("DROP TRIGGER storage_fault")
                .unwrap();
            let rollback = store
                .promote_volatile_batch_rollback(&owner, &gate, &token)
                .unwrap();
            assert_eq!(
                store
                    .batch_evidence(successor.operation_id())
                    .unwrap()
                    .unwrap()
                    .as_bytes(),
                exact_rejection
            );
            record(&store, &owner, &gate, &rollback, &rollback_o);
            assert_eq!(
                store
                    .batch_evidence(r.operation_id())
                    .unwrap()
                    .unwrap()
                    .as_bytes(),
                first_o.as_bytes()
            );
            assert_eq!(
                store
                    .batch_evidence(successor.operation_id())
                    .unwrap()
                    .unwrap()
                    .as_bytes(),
                exact_rejection
            );
            assert_eq!(
                store
                    .batch_evidence(rollback.operation_id())
                    .unwrap()
                    .unwrap()
                    .as_bytes(),
                rollback_o.as_bytes()
            );
            let page = store
                .manual_transaction_evidence(group.id(), 0, 64)
                .unwrap();
            assert_eq!(page.pending_count, 1);
            assert_eq!(page.members.len(), 3);
            assert_eq!(page.disposition, "rolledBack");
            let no_dispatch: i64 = store
                .connection
                .lock()
                .query_row(
                    "SELECT count(*) FROM batch_operations_v8 WHERE terminal_kind='noDispatch'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                no_dispatch, 0,
                "queued refusal is an original, never a no-queue witness"
            );
            validate_records(&store.connection.lock()).unwrap();
        }
    }
}
