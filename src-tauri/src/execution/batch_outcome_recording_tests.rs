//! Isolated real SQLite and actual owned native children, with synthetic batch
//! receipts. This is not Snowflake SDK/account, SQL, or transaction qualification.
use super::*;
use crate::{
    drivers::{
        snowflake_batch::{Role, StepPhase, UnitKind},
        snowflake_process::{Bounds, Impact, Launch, Supervisor},
    },
    secrets::MemorySecrets,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;

fn setup() -> (tempfile::TempDir, Arc<Studio>, Connection) {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let c = Connection::open(root.path().join("joinery.sqlite")).unwrap();
    (root, studio, c)
}
async fn helper() -> Supervisor {
    launch_helper(&new_id(), false).await
}
fn encoded_frame(value: Value) -> String {
    let body = serde_json::to_vec(&value).unwrap();
    let mut frame = (body.len() as u32).to_be_bytes().to_vec();
    frame.extend(body);
    frame.iter().map(|b| format!("\\{b:03o}")).collect()
}
async fn connected_helper(generation: &str) -> Supervisor {
    let child = launch_helper(generation, true).await;
    let mut connect = child
        .request("fixture-connect", "connect", json!({}), Impact::Control)
        .unwrap();
    assert!(connect.wait_bounded(Duration::from_secs(5)).await.ok);
    child
}
async fn launch_helper(generation: &str, connectable: bool) -> Supervisor {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().canonicalize().unwrap();
    let script = directory.join("batch-recording-owned-child.sh");
    let escaped = encoded_frame(
        json!({"v":1,"type":"hello","protocol":"joinery-snowflake","sdkVersion":"3.4.1","limits":{"frameBytes":1048576,"batchRows":128,"batchBytes":262144,"queued":8,"credits":32}}),
    );
    let suffix = if connectable {
        // The synthetic child acknowledges only the first native connect
        // request, then idles. It never interprets or executes generated SQL.
        let accepted =
            encoded_frame(json!({"v":1,"generation":generation,"id":"1","type":"accepted"}));
        let terminal = encoded_frame(
            json!({"v":1,"generation":generation,"id":"1","type":"terminal","ok":true,"result":{"connected":true}}),
        );
        format!(
            "/bin/dd bs=1 count=1 of=/dev/null 2>/dev/null\nprintf '{accepted}{terminal}'\nexec /bin/sleep 30\n"
        )
    } else {
        "exec /bin/cat\n".into()
    };
    std::fs::write(&script, format!("#!/bin/sh\nprintf '{escaped}'\n{suffix}")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    Supervisor::spawn(
        Launch::new(script, None, directory, root).unwrap(),
        generation,
        Bounds::default(),
    )
    .await
    .unwrap()
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
fn descriptor(child: &Supervisor) -> BatchDescriptor {
    BatchDescriptor {
        operation_id: new_id(),
        connection_id: "recording-fixture".into(),
        session_id: new_id(),
        helper_generation: child.helper_generation().generation().into(),
        started_at: now_ms(),
        digest: "a".repeat(64),
    }
}
fn request(mode: Mode, owner: Option<uuid::Uuid>) -> BatchRequest {
    BatchRequest::Batch {
        mode,
        owner,
        rollback_prior: owner.is_some(),
        units: vec![UnitKind::Update],
    }
}
fn batch_payload(kind: &str) -> Value {
    json!({"mode":"automatic","owner":null,"rollbackPrior":false,"expected":{"user":"USER","session":"123","role":"ROLE","database":"DB","schema":"PUBLIC","warehouse":"WH"},"units":[{"kind":kind,"sql":if kind=="delete"{"DELETE T WHERE V=?"}else{"UPDATE T SET V=? WHERE V=?"},"binds":["2","1"]}]})
}
fn frame(value: Value) -> Value {
    let mut f = json!({"v":1,"generation":"fixture","id":"fixture"});
    f.as_object_mut()
        .unwrap()
        .extend(value.as_object().unwrap().clone());
    f
}
fn step(e: &mut BatchEvidence, role: Role, unit: Option<usize>) {
    let index = e.steps.len() + 1;
    assert!(e.apply_event(&frame(json!({"type":"batchStep","step":index,"role":role,"unit":unit,"phase":StepPhase::Dispatching}))));
    assert!(e.apply_event(&frame(
        json!({"type":"batchStatement","step":index,"sdkRequestId":new_id(),"queryId":new_id()})
    )));
    assert!(e.apply_event(&frame(json!({"type":"batchStep","step":index,"role":role,"unit":unit,"phase":StepPhase::Acknowledged}))));
    if let Some(unit) = unit {
        assert!(e.apply_event(&frame(json!({"type":"batchCount","step":index,"unit":unit,"affectedRows":"1","multiJoinedRows":"0"}))));
    }
    assert!(e.apply_event(&frame(
        json!({"type":"batchStep","step":index,"role":role,"unit":unit,"phase":StepPhase::Drained})
    )));
}
fn evidence(
    request: BatchRequest,
    owner: uuid::Uuid,
    outcome: Outcome,
    prior: u32,
) -> BatchEvidence {
    let mut e = BatchEvidence::new(request);
    match &e.request {
        BatchRequest::Batch { owner, .. } => {
            let fresh = owner.is_none();
            for role in [Role::Context, Role::Settings] {
                step(&mut e, role, None);
            }
            if fresh {
                for role in [Role::Begin, Role::Transaction] {
                    step(&mut e, role, None);
                }
            }
            step(&mut e, Role::Unit, Some(0));
            step(&mut e, Role::Verify, None);
            if outcome == Outcome::Committed {
                for role in [Role::Commit, Role::Idle, Role::Disposition] {
                    step(&mut e, role, None);
                }
            }
        }
        BatchRequest::Finish {
            action: FinishAction::Rollback,
            ..
        } => {
            for role in [Role::Rollback, Role::Idle, Role::Disposition] {
                step(&mut e, role, None);
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
                step(&mut e, role, None);
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
        Some(owner)
    };
    assert!(e.accept_terminal(&json!({"outcome":outcome,"owner":owner,"transactionId":"123","failure":null,"priorBatches":prior})));
    e
}
fn pending(studio: &Studio, child: &Supervisor) -> (Arc<BatchOutcomeCell>, BatchEvidence) {
    let r = request(Mode::Manual, None);
    let e = evidence(r.clone(), uuid::Uuid::new_v4(), Outcome::Pending, 0);
    let cell = studio
        .admit_batch_outcome(
            studio.outcome_memory.ordinary().unwrap(),
            descriptor(child),
            BatchPlan::new(r).unwrap(),
            None,
            Some(studio.outcome_memory.safety().unwrap()),
            child.retirement_observer(),
        )
        .unwrap();
    (cell, e)
}
fn scalar(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn original(cell: &BatchOutcomeCell) -> Vec<u8> {
    cell.state
        .lock()
        .original
        .as_ref()
        .expect("compact original")
        .as_bytes()
        .to_vec()
}
fn archive(studio: &Studio, id: &str) -> Vec<u8> {
    studio
        .persistence
        .batch_evidence(id)
        .unwrap()
        .unwrap()
        .as_bytes()
        .to_vec()
}
fn assert_gate_held(root: &std::path::Path) {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("history-admission.lock"))
        .unwrap();
    assert!(matches!(
        file.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_pending_is_not_commit_and_reaped_idle_group_preserves_original() {
    let (_root, studio, c) = setup();
    let child = helper().await;
    let (cell, e) = pending(&studio, &child);
    studio
        .record_batch_outcome(&cell, &e, BatchTermination::Complete)
        .unwrap();
    assert!(cell.is_settled());
    assert!(!studio.batch_outcomes.is_empty());
    let group = cell.manual_transaction().unwrap();
    let exact = archive(&studio, cell.operation_id());
    let page = studio
        .persistence
        .manual_transaction_evidence(group.id(), 0, 10)
        .unwrap();
    assert_eq!(page.disposition, "open");
    assert_eq!(page.pending_count, 1);
    assert_eq!(
        scalar(
            &c,
            "SELECT count(*) FROM manual_transaction_bindings_v8 WHERE rollback_credit=1"
        ),
        1
    );
    let audits = serde_json::to_value(studio.persistence.audit_log().unwrap()).unwrap();
    reap(&child).await;
    drop(child);
    studio.flush_batch_outcomes().unwrap();
    assert!(studio.batch_outcomes.is_empty());
    assert_eq!(archive(&studio, cell.operation_id()), exact);
    assert_eq!(
        serde_json::to_value(studio.persistence.audit_log().unwrap()).unwrap(),
        audits
    );
    assert_eq!(
        studio
            .persistence
            .manual_transaction_evidence(group.id(), 0, 10)
            .unwrap()
            .disposition,
        "unknown"
    );
    assert_eq!(
        scalar(&c, "SELECT count(*) FROM manual_transaction_bindings_v8"),
        0
    );
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_capture_and_settle_failures_keep_exact_original_gate_and_memory() {
    for capture in [true, false] {
        let (root, studio, c) = setup();
        let child = helper().await;
        let (cell, e) = pending(&studio, &child);
        c.execute_batch(if capture {"CREATE TRIGGER reject_batch BEFORE UPDATE ON batch_operations_v8 WHEN NEW.phase='captured' BEGIN SELECT RAISE(FAIL,'capture'); END"}else{"CREATE TRIGGER reject_batch BEFORE INSERT ON batch_captures_v8 BEGIN SELECT RAISE(FAIL,'archive'); END"}).unwrap();
        assert!(
            studio
                .record_batch_outcome(&cell, &e, BatchTermination::Complete)
                .is_err()
        );
        let exact = original(&cell);
        assert_eq!(cell.recording_state().captured, !capture);
        assert!(!cell.is_settled());
        assert_gate_held(root.path());
        assert!(studio.abandon_batch_before_dispatch(&cell).is_err());
        reap(&child).await;
        assert!(studio.flush_batch_outcomes().is_err());
        assert_eq!(original(&cell), exact);
        assert_eq!(
            scalar(&c, "SELECT count(*) FROM manual_transaction_bindings_v8"),
            1
        );
        c.execute_batch("DROP TRIGGER reject_batch").unwrap();
        studio.flush_batch_outcomes().unwrap();
        assert!(cell.is_settled());
        assert!(studio.batch_outcomes.is_empty());
        assert_eq!(archive(&studio, cell.operation_id()), exact);
        assert_eq!(
            scalar(&c, "SELECT count(*) FROM audit_v8 WHERE outcome='pending'"),
            1
        );
        assert_eq!(
            scalar(
                &c,
                "SELECT count(*) FROM batch_operations_v8 WHERE terminal_kind='original'"
            ),
            1
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_failed_admission_retries_same_candidate_without_dispatch_or_finished_audit()
 {
    let (root, studio, c) = setup();
    let child = helper().await;
    let d = descriptor(&child);
    c.execute_batch("CREATE TRIGGER reject_reserve BEFORE INSERT ON batch_operations_v8 BEGIN SELECT RAISE(FAIL,'reserve'); END").unwrap();
    assert!(
        studio
            .admit_batch_outcome(
                studio.outcome_memory.ordinary().unwrap(),
                d.clone(),
                BatchPlan::new(request(Mode::Manual, None)).unwrap(),
                None,
                Some(studio.outcome_memory.safety().unwrap()),
                child.retirement_observer()
            )
            .is_err()
    );
    assert_eq!(studio.batch_outcomes.pending.lock().len(), 1);
    assert_gate_held(root.path());
    assert_eq!(scalar(&c, "SELECT count(*) FROM batch_operations_v8"), 0);
    c.execute_batch("DROP TRIGGER reject_reserve").unwrap();
    studio.flush_batch_outcomes().unwrap();
    assert_eq!(
        scalar(
            &c,
            "SELECT count(*) FROM batch_operations_v8 WHERE terminal_kind='noOriginal'"
        ),
        1
    );
    assert!(
        studio
            .persistence
            .batch_evidence(&d.operation_id)
            .unwrap()
            .is_none()
    );
    assert!(
        studio
            .persistence
            .audit_log()
            .unwrap()
            .iter()
            .all(|a| a.outcome == "started")
    );
    reap(&child).await;
    studio.flush_batch_outcomes().unwrap();
    assert!(studio.batch_outcomes.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_volatile_rollback_preserves_original_when_ordinary_slots_are_full() {
    let (_root, studio, c) = setup();
    let child = helper().await;
    let (first, e) = pending(&studio, &child);
    studio
        .record_batch_outcome(&first, &e, BatchTermination::Complete)
        .unwrap();
    let first_bytes = archive(&studio, first.operation_id());
    let group = first.manual_transaction().unwrap();
    let owner = e.summary.as_ref().unwrap().owner.unwrap();
    let ordinary: Vec<_> = (0..32)
        .map(|_| studio.outcome_memory.ordinary().unwrap())
        .collect();
    assert!(studio.outcome_memory.ordinary().is_err());
    c.execute_batch("CREATE TRIGGER reject_rollback BEFORE INSERT ON batch_operations_v8 WHEN NEW.pool='emergencyRollback' BEGIN SELECT RAISE(FAIL,'reserve rollback'); END").unwrap();
    let mut d = first.descriptor.clone();
    d.operation_id = new_id();
    d.started_at = now_ms();
    let safety = studio.batch_outcomes.groups.lock()[group.id()]
        .rollback_memory
        .clone();
    let memory = studio.claim_batch_rollback_memory(&group).unwrap();
    let cell = studio
        .admit_batch_finish(
            memory,
            d,
            &group,
            owner,
            FinishAction::Rollback,
            child.retirement_observer(),
        )
        .unwrap();
    assert!(cell.state.lock().volatile.is_some());
    let e = evidence(
        BatchRequest::Finish {
            owner,
            action: FinishAction::Rollback,
        },
        owner,
        Outcome::RolledBack,
        1,
    );
    assert!(
        studio
            .record_batch_outcome(&cell, &e, BatchTermination::Complete)
            .is_err()
    );
    let exact = original(&cell);
    assert_eq!(archive(&studio, first.operation_id()), first_bytes);
    assert!(safety.claim().is_err());
    reap(&child).await;
    assert!(studio.flush_batch_outcomes().is_err());
    c.execute_batch("DROP TRIGGER reject_rollback").unwrap();
    studio.flush_batch_outcomes().unwrap();
    assert_eq!(archive(&studio, cell.operation_id()), exact);
    assert!(safety.claim().is_ok());
    assert!(studio.batch_outcomes.is_empty());
    assert_eq!(
        scalar(&c, "SELECT count(*) FROM manual_transaction_bindings_v8"),
        0
    );
    drop(ordinary);
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_encoder_failure_cannot_be_abandoned_or_release_native_pin() {
    let (root, studio, c) = setup();
    let child = helper().await;
    let (cell, _) = pending(&studio, &child);
    let wrong = evidence(
        request(Mode::Automatic, None),
        uuid::Uuid::new_v4(),
        Outcome::Committed,
        0,
    );
    assert!(
        studio
            .record_batch_outcome(&cell, &wrong, BatchTermination::Complete)
            .is_err()
    );
    assert!(cell.recording_state().encoding_failed);
    assert!(cell.state.lock().original.is_none());
    assert!(studio.abandon_batch_before_dispatch(&cell).is_err());
    reap(&child).await;
    assert!(studio.flush_batch_outcomes().is_err());
    assert_gate_held(root.path());
    assert_eq!(
        scalar(&c, "SELECT count(*) FROM manual_transaction_bindings_v8"),
        1
    );
    assert_eq!(scalar(&c, "SELECT count(*) FROM batch_captures_v8"), 0);
    assert!(
        studio
            .persistence
            .audit_log()
            .unwrap()
            .iter()
            .all(|a| a.outcome == "started")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_dropped_operation_keeps_actual_native_terminal_independent_of_caller() {
    let (_root, studio, c) = setup();
    let child = connected_helper(&new_id()).await;
    let observer = child.retirement_observer();
    let r = request(Mode::Automatic, None);
    let cell = studio
        .admit_batch_outcome(
            studio.outcome_memory.ordinary().unwrap(),
            descriptor(&child),
            BatchPlan::new(r.clone()).unwrap(),
            None,
            None,
            observer.clone(),
        )
        .unwrap();
    let operation = studio
        .dispatch_batch_operation(&cell, || {
            child
                .request(
                    cell.operation_id(),
                    "mutationBatch",
                    batch_payload("update"),
                    Impact::Mutating,
                )
                .map_err(|_| AppError::internal("Synthetic native dispatch failed"))
        })
        .unwrap();
    assert!(cell.recording_state().awaiting_terminal);
    assert!(studio.abandon_batch_before_dispatch(&cell).is_err());
    // The owned child idles after a synthetic connect. Retirement yields a genuine
    // native incomplete prefix, not a synthetic ACK or remote execution proof.
    drop(operation);
    drop(child);
    assert!(observer.wait(Duration::from_secs(5)).await.is_some());
    studio.flush_batch_outcomes().unwrap();
    assert!(cell.is_settled());
    assert!(studio.batch_outcomes.is_empty());
    let stored = studio
        .persistence
        .batch_evidence(cell.operation_id())
        .unwrap()
        .unwrap();
    let evidence = stored.evidence().unwrap();
    assert_eq!(evidence.request, r);
    assert!(evidence.summary.is_none());
    assert_eq!(
        scalar(
            &c,
            "SELECT count(*) FROM batch_operations_v8 WHERE terminal_kind='original'"
        ),
        1
    );
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_explicit_predispatch_abandon_never_fabricates_original() {
    let (_root, studio, c) = setup();
    let child = helper().await;
    let (cell, _) = pending(&studio, &child);
    studio.abandon_batch_before_dispatch(&cell).unwrap();
    assert!(cell.is_settled());
    assert!(
        studio
            .persistence
            .batch_evidence(cell.operation_id())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        scalar(
            &c,
            "SELECT count(*) FROM batch_operations_v8 WHERE terminal_kind='noOriginal'"
        ),
        1
    );
    assert!(
        studio
            .persistence
            .audit_log()
            .unwrap()
            .iter()
            .all(|a| a.outcome == "started")
    );
    reap(&child).await;
    studio.flush_batch_outcomes().unwrap();
    assert!(studio.batch_outcomes.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_idle_group_keeps_same_safety_slot_through_session_owner_loss() {
    let (_root, studio, _c) = setup();
    let child = helper().await;
    let session_slot = studio.outcome_memory.safety().unwrap();
    let session_pointer = Arc::as_ptr(&session_slot);
    let r = request(Mode::Manual, None);
    let e = evidence(r.clone(), uuid::Uuid::new_v4(), Outcome::Pending, 0);
    let cell = studio
        .admit_batch_outcome(
            studio.outcome_memory.ordinary().unwrap(),
            descriptor(&child),
            BatchPlan::new(r).unwrap(),
            None,
            Some(session_slot.clone()),
            child.retirement_observer(),
        )
        .unwrap();
    studio
        .record_batch_outcome(&cell, &e, BatchTermination::Complete)
        .unwrap();
    let group = cell.manual_transaction().unwrap();
    assert_eq!(
        Arc::as_ptr(&studio.batch_outcomes.groups.lock()[group.id()].rollback_memory),
        session_pointer
    );
    drop(session_slot);
    drop(cell);
    let other_sessions: Vec<_> = (0..31)
        .map(|_| studio.outcome_memory.safety().unwrap())
        .collect();
    assert!(studio.outcome_memory.safety().is_err());
    let pending = studio.claim_batch_rollback_memory(&group).unwrap();
    assert!(studio.claim_batch_rollback_memory(&group).is_err());
    drop(pending);
    reap(&child).await;
    studio.flush_batch_outcomes().unwrap();
    assert!(studio.batch_outcomes.is_empty());
    assert!(studio.outcome_memory.safety().is_ok());
    drop(other_sessions);
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_dispatch_rejects_wrong_physical_helper_id_request_and_ordinary_operation()
{
    for kind in ["helper", "id", "request", "ordinary"] {
        let (root, studio, c) = setup();
        let generation = new_id();
        let child = connected_helper(&generation).await;
        let foreign = if kind == "helper" {
            Some(connected_helper(&generation).await)
        } else {
            None
        };
        let cell = studio
            .admit_batch_outcome(
                studio.outcome_memory.ordinary().unwrap(),
                descriptor(&child),
                BatchPlan::new(request(Mode::Automatic, None)).unwrap(),
                None,
                None,
                child.retirement_observer(),
            )
            .unwrap();
        let result = studio.dispatch_batch_operation(&cell, || {
            let actual = foreign.as_ref().unwrap_or(&child);
            let id = if kind == "id" {
                "foreign-logical-id"
            } else {
                cell.operation_id()
            };
            if kind == "ordinary" {
                actual.request(id, "query", json!({"sql":"SELECT 1"}), Impact::Read)
            } else {
                actual.request(
                    id,
                    "mutationBatch",
                    batch_payload(if kind == "request" {
                        "delete"
                    } else {
                        "update"
                    }),
                    Impact::Mutating,
                )
            }
            .map_err(|_| AppError::internal("Synthetic native dispatch failed"))
        });
        reap(&child).await;
        if let Some(foreign) = foreign {
            reap(&foreign).await;
        }
        assert!(
            matches!(result,Err(e) if e.code==ErrorCode::Conflict),
            "{kind}"
        );
        assert!(cell.recording_state().encoding_failed);
        assert!(cell.state.lock().terminal.is_some());
        assert!(!cell.state.lock().no_original);
        assert!(studio.abandon_batch_before_dispatch(&cell).is_err());
        assert!(studio.flush_batch_outcomes().is_err());
        assert_gate_held(root.path());
        assert_eq!(scalar(&c, "SELECT count(*) FROM batch_captures_v8"), 0);
        assert_eq!(
            scalar(
                &c,
                "SELECT count(*) FROM batch_operations_v8 WHERE terminal_kind='noOriginal'"
            ),
            0
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_dispatch_error_and_already_abandoned_cell_never_call_new_closure() {
    let (_root, studio, c) = setup();
    let child = helper().await;
    let cell = studio
        .admit_batch_outcome(
            studio.outcome_memory.ordinary().unwrap(),
            descriptor(&child),
            BatchPlan::new(request(Mode::Automatic, None)).unwrap(),
            None,
            None,
            child.retirement_observer(),
        )
        .unwrap();
    assert!(
        studio
            .dispatch_batch_operation(&cell, || Err(AppError::new(
                ErrorCode::Cancelled,
                "Synthetic prequeue refusal"
            )))
            .is_err()
    );
    assert!(cell.is_settled());
    let mut invoked = false;
    assert!(
        studio
            .dispatch_batch_operation(&cell, || {
                invoked = true;
                panic!("abandoned cell must refuse before closure")
            })
            .is_err()
    );
    assert!(!invoked);
    assert_eq!(
        scalar(
            &c,
            "SELECT count(*) FROM batch_operations_v8 WHERE terminal_kind='noOriginal'"
        ),
        1
    );
    assert_eq!(scalar(&c, "SELECT count(*) FROM batch_captures_v8"), 0);
    assert!(studio.batch_outcomes.is_empty());
    reap(&child).await;
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_dispatch_unwind_after_native_admission_never_allows_abandon() {
    let (root, studio, c) = setup();
    let child = connected_helper(&new_id()).await;
    let cell = studio
        .admit_batch_outcome(
            studio.outcome_memory.ordinary().unwrap(),
            descriptor(&child),
            BatchPlan::new(request(Mode::Automatic, None)).unwrap(),
            None,
            None,
            child.retirement_observer(),
        )
        .unwrap();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = studio.dispatch_batch_operation(&cell, || {
            let _operation = child
                .request(
                    cell.operation_id(),
                    "mutationBatch",
                    batch_payload("update"),
                    Impact::Mutating,
                )
                .unwrap();
            panic!("synthetic producer panic after actual queue admission")
        });
    }));
    reap(&child).await;
    assert!(panicked.is_err());
    assert!(cell.state.lock().dispatched);
    assert!(cell.recording_state().encoding_failed);
    assert!(studio.abandon_batch_before_dispatch(&cell).is_err());
    assert!(studio.flush_batch_outcomes().is_err());
    assert_gate_held(root.path());
    assert_eq!(
        scalar(
            &c,
            "SELECT count(*) FROM batch_operations_v8 WHERE terminal_kind='noOriginal'"
        ),
        0
    );
}

#[tokio::test(flavor = "current_thread")]
async fn batch_recording_atomic_dispatch_excludes_concurrent_predispatch_abandon() {
    let (_root, studio, _c) = setup();
    let child = connected_helper(&new_id()).await;
    let cell = studio
        .admit_batch_outcome(
            studio.outcome_memory.ordinary().unwrap(),
            descriptor(&child),
            BatchPlan::new(request(Mode::Automatic, None)).unwrap(),
            None,
            None,
            child.retirement_observer(),
        )
        .unwrap();
    let mut abandon = None;
    let operation = studio
        .dispatch_batch_operation(&cell, || {
            assert!(cell.state.try_lock().is_none());
            let other = studio.clone();
            let retained = cell.clone();
            let (began, started) = std::sync::mpsc::channel();
            abandon = Some(std::thread::spawn(move || {
                began.send(()).unwrap();
                other.abandon_batch_before_dispatch(&retained)
            }));
            started.recv().unwrap();
            child
                .request(
                    cell.operation_id(),
                    "mutationBatch",
                    batch_payload("update"),
                    Impact::Mutating,
                )
                .map_err(|_| AppError::internal("Synthetic native dispatch failed"))
        })
        .unwrap();
    let result = abandon.unwrap().join().unwrap();
    drop(operation);
    reap(&child).await;
    assert!(matches!(result,Err(e) if e.code==ErrorCode::Conflict));
    studio.flush_batch_outcomes().unwrap();
    assert!(cell.is_settled());
    assert!(studio.batch_outcomes.is_empty());
}
