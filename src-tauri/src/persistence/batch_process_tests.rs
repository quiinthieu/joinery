//! Actual owned-child deaths with real SQLite/OS leases and synthetic evidence.
//! No SDK, authentication, engine connection or remote SQL is exercised here.
#![cfg(unix)]

use super::{BatchDescriptor, BatchPlan, BatchTermination, OriginalBatchOutcome};
use crate::drivers::snowflake_batch::{
    BatchEvidence, BatchRequest, Mode, Role, StepPhase, UnitKind,
};
use crate::persistence::{Persistence, outcomes::MAX_OUTCOME_BYTES};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const FIXTURE: &str =
    "persistence::batch_outcomes::batch_process_tests::batch_recovery_process_fixture";

fn frame(extra: Value) -> Value {
    let mut frame = json!({"v":1,"generation":"owned-process","id":"owned-process"});
    frame
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    frame
}

fn dispatch(evidence: &mut BatchEvidence, role: Role, unit: Option<usize>) -> usize {
    let step = evidence.steps.len() + 1;
    assert!(evidence.apply_event(&frame(json!({
        "type":"batchStep","step":step,"role":role,"unit":unit,
        "phase":StepPhase::Dispatching
    }))));
    assert!(evidence.apply_event(&frame(json!({
        "type":"batchStatement","step":step,"sdkRequestId":Uuid::new_v4(),
        "queryId":Uuid::new_v4()
    }))));
    assert!(evidence.apply_event(&frame(json!({
        "type":"batchStep","step":step,"role":role,"unit":unit,
        "phase":StepPhase::Acknowledged
    }))));
    step
}

fn complete(evidence: &mut BatchEvidence, role: Role, unit: Option<usize>) {
    let step = dispatch(evidence, role, unit);
    if unit.is_some() {
        assert!(evidence.apply_event(&frame(json!({
            "type":"batchCount","step":step,"unit":unit,
            "affectedRows":"1","multiJoinedRows":"0"
        }))));
    }
    assert!(evidence.apply_event(&frame(json!({
        "type":"batchStep","step":step,"role":role,"unit":unit,
        "phase":StepPhase::Drained
    }))));
}

fn request(owner: Option<Uuid>) -> BatchRequest {
    BatchRequest::Batch {
        mode: Mode::Manual,
        owner,
        rollback_prior: owner.is_some(),
        units: vec![UnitKind::Update],
    }
}

fn pending_evidence(request: BatchRequest, helper_owner: Uuid) -> BatchEvidence {
    let mut evidence = BatchEvidence::new(request);
    for role in [
        Role::Context,
        Role::Settings,
        Role::Begin,
        Role::Transaction,
    ] {
        complete(&mut evidence, role, None);
    }
    complete(&mut evidence, Role::Unit, Some(0));
    complete(&mut evidence, Role::Verify, None);
    assert!(evidence.accept_terminal(&json!({
        "outcome":"pending","owner":helper_owner,"transactionId":"123",
        "failure":null,"priorBatches":0
    })));
    evidence
}

fn descriptor(session: &str, generation: &str) -> BatchDescriptor {
    BatchDescriptor {
        operation_id: Uuid::new_v4().to_string(),
        connection_id: "owned-batch-process-fixture".into(),
        session_id: session.into(),
        helper_generation: generation.into(),
        started_at: 1,
        digest: "a".repeat(64),
    }
}

#[test]
#[ignore = "owned subprocess fixture; invoked explicitly by the process regressions"]
fn batch_recovery_process_fixture() {
    let root = PathBuf::from(std::env::var_os("JOINERY_BATCH_PROCESS_ROOT").unwrap());
    let scenario = std::env::var("JOINERY_BATCH_PROCESS_SCENARIO").unwrap();
    assert!(matches!(
        scenario.as_str(),
        "captured" | "idle" | "uncaptured"
    ));
    let store = Persistence::open(&root).unwrap();
    let owner = Arc::new(store.claim_history_owner().unwrap());
    let gate = store.operation_gate(&owner).unwrap();
    let session = Uuid::new_v4().to_string();
    let generation = Uuid::new_v4().to_string();
    let first_descriptor = descriptor(&session, &generation);
    let helper_owner = Uuid::new_v4();
    let plan = BatchPlan::new(request(None)).unwrap();
    let reservation = store
        .reserve_owned_batch(&owner, &gate, &first_descriptor, &plan, None)
        .unwrap();
    let group = reservation.manual_transaction().unwrap();
    let evidence = pending_evidence(request(None), helper_owner);
    let original = OriginalBatchOutcome::capture_in(
        &reservation,
        &evidence,
        BatchTermination::Complete,
        2,
        Vec::with_capacity(MAX_OUTCOME_BYTES),
    )
    .unwrap();
    store
        .capture_batch_outcome(&owner, &gate, &reservation, &original)
        .unwrap();
    fs::write(root.join("expected-original.json"), original.as_bytes()).unwrap();

    if scenario == "captured" {
        store
            .connection
            .lock()
            .execute_batch(
                "CREATE TEMP TRIGGER refuse_batch_archive BEFORE INSERT ON batch_captures_v8 \
             BEGIN SELECT RAISE(ABORT,'owned process archive failure'); END;",
            )
            .unwrap();
        let error = store
            .settle_batch_outcome(&owner, &gate, &reservation, &original)
            .unwrap_err();
        assert_eq!(error.code, crate::domain::ErrorCode::Internal);
        // Production errors intentionally omit raw SQLite diagnostics. Verify
        // the complete pre-settlement state rather than matching that text.
        let connection = store.connection.lock();
        let state: (String, Vec<u8>, i64, i64) = connection
            .query_row(
                "SELECT phase,payload,\
                 (SELECT count(*) FROM batch_captures_v8 WHERE operation_id=?1),\
                 (SELECT count(*) FROM audit_v8 WHERE id=?2) \
                 FROM batch_operations_v8 WHERE operation_id=?1",
                rusqlite::params![
                    first_descriptor.operation_id,
                    format!("{}:finished", first_descriptor.operation_id)
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            state,
            ("captured".into(), original.as_bytes().to_vec(), 0, 0)
        );
    } else {
        store
            .settle_batch_outcome(&owner, &gate, &reservation, &original)
            .unwrap();
    }

    let mut ready = json!({
        "operationId":first_descriptor.operation_id,"groupId":group.id(),
        "scenario":scenario,"secondOperationId":null
    });
    let mut unrecorded_attempt = None;
    if scenario == "uncaptured" {
        let second = descriptor(&session, &generation);
        let plan = BatchPlan::new(request(Some(helper_owner))).unwrap();
        let continuation = store
            .reserve_owned_batch(&owner, &gate, &second, &plan, Some(&group))
            .unwrap();
        assert_eq!(continuation.operation_id(), second.operation_id);
        let mut unrecorded = BatchEvidence::new(request(Some(helper_owner)));
        for role in [Role::Context, Role::Settings] {
            complete(&mut unrecorded, role, None);
        }
        dispatch(&mut unrecorded, Role::Unit, Some(0));
        assert!(unrecorded.has_acknowledgement());
        // Deliberately never call capture: recovery cannot learn this ACK.
        ready["secondOperationId"] = Value::String(second.operation_id);
        unrecorded_attempt = Some((continuation, unrecorded));
    }
    // A manually idle group retains ownership without an operation SH gate.
    drop(gate);
    fs::write(root.join("ready.json"), serde_json::to_vec(&ready).unwrap()).unwrap();
    // Keep the genuine owner and group lease alive until the parent SIGKILL.
    let _retained = (
        owner,
        group,
        reservation,
        original,
        unrecorded_attempt,
        store,
    );
    loop {
        thread::park_timeout(Duration::from_secs(60));
    }
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn spawn(root: &Path, scenario: &str) -> OwnedChild {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", FIXTURE, "--ignored", "--nocapture"]);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("JOINERY_") {
            command.env_remove(key);
        }
    }
    command
        .env("JOINERY_BATCH_PROCESS_ROOT", root)
        .env("JOINERY_BATCH_PROCESS_SCENARIO", scenario)
        .stdin(Stdio::null())
        .stdout(File::create(root.join("child-stdout.log")).unwrap())
        .stderr(File::create(root.join("child-stderr.log")).unwrap());
    OwnedChild(command.spawn().unwrap())
}

fn kill_ready_child(child: &mut OwnedChild, root: &Path) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    let ready = loop {
        if let Ok(bytes) = fs::read(root.join("ready.json"))
            && let Ok(ready) = serde_json::from_slice::<Value>(&bytes)
        {
            break ready;
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "owned child exited: {}",
            fs::read_to_string(root.join("child-stderr.log")).unwrap()
        );
        assert!(Instant::now() < deadline, "owned child readiness timed out");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(child.0.try_wait().unwrap().is_none());
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
    ready
}

fn verify_recovery(scenario: &str) {
    let root = tempfile::tempdir().unwrap();
    let mut child = spawn(root.path(), scenario);
    let ready = kill_ready_child(&mut child, root.path());
    let operation = ready["operationId"].as_str().unwrap();
    let group = ready["groupId"].as_str().unwrap();
    let expected = fs::read(root.path().join("expected-original.json")).unwrap();
    for _ in 0..2 {
        let store = Persistence::open(root.path()).unwrap();
        let _new_owner = store.claim_history_owner().unwrap();
        let historical = store.batch_evidence(operation).unwrap().unwrap();
        assert_eq!(historical.as_bytes(), expected);
        let evidence = historical.evidence().unwrap();
        assert_eq!(
            evidence.summary.unwrap().outcome,
            crate::drivers::snowflake_batch::Outcome::Pending
        );
        let page = store.manual_transaction_evidence(group, 0, 8).unwrap();
        assert_eq!(page.transaction_id, group);
        assert_eq!(page.disposition, "unknown");
        assert_eq!(page.pending_count, 1);
        assert_eq!(
            page.members.len(),
            if scenario == "uncaptured" { 2 } else { 1 }
        );
        if let Some(second) = ready["secondOperationId"].as_str() {
            assert!(store.batch_evidence(second).unwrap().is_none());
        }
        let bindings: i64 = store
            .connection
            .lock()
            .query_row(
                "SELECT count(*) FROM manual_transaction_bindings_v8",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bindings, 0, "dead group must confer no execution authority");
        let original_audits: Vec<(String, String)> = store
            .connection
            .lock()
            .prepare("SELECT id,outcome FROM audit_v8 WHERE id IN (?1,?2) ORDER BY id")
            .unwrap()
            .query_map(
                rusqlite::params![
                    format!("{operation}:started"),
                    format!("{operation}:finished")
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            original_audits,
            vec![
                (format!("{operation}:finished"), "pending".into()),
                (format!("{operation}:started"), "started".into())
            ]
        );
        if let Some(second) = ready["secondOperationId"].as_str() {
            let terminal: (String, String, i64) = store
                .connection
                .lock()
                .query_row(
                    "SELECT phase,terminal_kind,(SELECT count(*) FROM audit_v8 WHERE id=?2) \
                 FROM batch_operations_v8 WHERE operation_id=?1",
                    rusqlite::params![second, format!("{second}:finished")],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(terminal, ("settled".into(), "noOriginal".into(), 0));
        }
    }
}

#[test]
fn batch_captured_original_survives_actual_child_death_and_archive_fault() {
    verify_recovery("captured");
}

#[test]
fn batch_manually_idle_original_survives_actual_child_death() {
    verify_recovery("idle");
}

#[test]
fn batch_uncaptured_ack_after_pending_original_stays_unknown_after_child_death() {
    verify_recovery("uncaptured");
}
