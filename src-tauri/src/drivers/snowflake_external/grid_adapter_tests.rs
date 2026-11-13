use super::super::grid::{
    GridAdmission, GridNoDispatch, GridOriginalRecorder, PreparedGridOperation,
};
use super::*;
use crate::drivers::{
    snowflake_batch::{BatchRequest, FinishAction, Mode, Outcome},
    snowflake_edit_snapshot::{PreparedSnowflakeEdits, SnapshotMutation},
    snowflake_mutations::CellValue,
    snowflake_process::HelperGeneration,
};

// Real framed production Helper and guard, injected SDK, no HTTP/account calls.
// The synthetic recorder below proves callback ordering/retention only; it does
// not stand in for SQLite capture, settlement or recovery qualification.
async fn fixture(wide_text: bool) -> Harness {
    let mut source = super::grid_freshness_tests::grid_fixture_source()
        .replace("const zeroRows =", "const editPreview = sql.startsWith('SELECT \"ID\", \"NOTES\" FROM ');\n      const zeroRows =")
        .replace("} else if (zeroRows) {", r#"} else if (editPreview) {
        names = ['ID','NOTES'];
        rows = [['9007199254740993',null],['9007199254740993',''],['2','雪']];
      } else if (zeroRows) {"#)
        .replace("(zeroRows && name === 'ID')", "((zeroRows || editPreview) && name === 'ID')")
        .replace("value('affectedRows','9007199254740993')", "value('affectedRows','1')")
        .replace("relationReads = 0;", "relationReads = 0, disposition = 'running';")
        .replace("currentTransaction = sql === 'BEGIN TRANSACTION'", "disposition = sql === 'COMMIT' ? 'committed' : sql === 'ROLLBACK' ? 'rolled back' : 'running';\n          currentTransaction = sql === 'BEGIN TRANSACTION'")
        .replace("} else if (sql !== 'SELECT 1') {", r#"} else if (sql === 'DESCRIBE TRANSACTION 123456789') {
        names = ['id','session','state']; rows = [['123456789','SESSION',disposition]];
      } else if (sql !== 'SELECT 1') {"#);
    if wide_text {
        source = source
            .replace("'100','400'", "'262144','1048576'")
            .replace("'VARCHAR(100)'", "'VARCHAR(262144)'");
    }
    Harness::open_fixture(SafeMode::Confirm, true, false, false, None, None, &source).await
}

async fn prepared(h: &Harness, mutations: &[SnapshotMutation]) -> PreparedSnowflakeEdits {
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target_id = catalog
        .relations
        .iter()
        .find(|r| r.table.schema == "PUBLIC")
        .unwrap()
        .target_id
        .clone();
    let inspection = h
        .driver
        .inspect_relation(&InspectRelationRequest {
            revision: catalog.revision.clone(),
            target_id: target_id.clone(),
        })
        .await
        .unwrap();
    let request = PreviewRelationRequest {
        revision: catalog.revision,
        target_id,
        inspection_revision: inspection.revision,
    };
    let preview = h.driver.preview_relation(&request).await.unwrap();
    assert_eq!(
        preview.rows,
        vec![
            vec![Some("9007199254740993".into()), None],
            vec![Some("9007199254740993".into()), Some("".into())],
            vec![Some("2".into()), Some("雪".into())]
        ]
    );
    assert!(
        preview
            .columns
            .iter()
            .all(|c| !c.editable && !c.insertable && !c.primary_key)
    );
    h.driver
        .prepare_relation_edits(
            &request,
            preview.edit_snapshot_id.as_deref().unwrap(),
            mutations,
        )
        .await
        .unwrap()
}

fn update(value: &str) -> SnapshotMutation {
    SnapshotMutation::Update {
        row_index: 0,
        changes: vec![(1, CellValue::Text(value.into()))],
    }
}

#[derive(Default)]
struct Admission {
    calls: usize,
    refuse: bool,
    duplicate: bool,
}
impl GridAdmission for Admission {
    fn admit(
        &mut self,
        dispatch: &mut dyn FnMut() -> AppResult<Operation>,
    ) -> AppResult<Operation> {
        self.calls += 1;
        if self.refuse {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Synthetic durable admission refused before dispatch",
            ));
        }
        let operation = dispatch()?;
        if self.duplicate {
            assert!(dispatch().is_err(), "second audited dispatch must refuse");
        }
        Ok(operation)
    }
}

#[derive(Default)]
struct Recorded {
    originals: Vec<Report>,
    no_dispatch: Vec<GridNoDispatch>,
}
struct Recorder {
    retained: Arc<Mutex<Recorded>>,
    helper: HelperGeneration,
    id: String,
    request: BatchRequest,
    fail: bool,
    panic: bool,
}
impl Recorder {
    fn new(h: &Harness, operation: &PreparedGridOperation) -> Self {
        Self {
            retained: Arc::new(Mutex::new(Recorded::default())),
            helper: h.driver.process.helper_generation(),
            id: operation.operation_id().into(),
            request: operation.request().clone(),
            fail: false,
            panic: false,
        }
    }
}
impl GridOriginalRecorder for Recorder {
    fn record_original(&mut self, report: &Report, operation: &Operation) -> AppResult<()> {
        assert!(operation.matches_batch_binding(&self.helper, &self.id, &self.request));
        assert_eq!(operation.report().as_ref().unwrap().batch, report.batch);
        assert!(
            report
                .batch
                .as_ref()
                .is_some_and(|batch| batch.summary.is_some())
        );
        self.retained.lock().originals.push(report.clone());
        assert!(
            !self.panic,
            "synthetic recorder panic after retaining original"
        );
        if self.fail {
            return Err(AppError::new(
                ErrorCode::Internal,
                "Synthetic local recording unavailable",
            ));
        }
        Ok(())
    }
    fn record_no_dispatch(&mut self, proof: &GridNoDispatch) -> AppResult<()> {
        assert!(proof.matches(&self.helper, &self.id, &self.request));
        assert!(!proof.matches(&self.helper, &new_id(), &self.request));
        self.retained.lock().no_dispatch.push(proof.clone());
        Ok(())
    }
}

fn outcome(report: &Report) -> Outcome {
    report
        .batch
        .as_ref()
        .unwrap()
        .summary
        .as_ref()
        .unwrap()
        .outcome
}
fn dml_count(h: &Harness) -> usize {
    h.queries()
        .iter()
        .filter(|s| {
            s.starts_with("UPDATE ")
                || s.starts_with("INSERT INTO ")
                || s.starts_with("DELETE FROM ")
        })
        .count()
}
fn ledger_frames(h: &Harness) -> Vec<Value> {
    std::fs::read_to_string(h.scripts.path().join("grid-frames"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
async fn finish(h: Harness) {
    h.driver.close();
    let receipt = h.cleanup.wait(Duration::from_secs(10)).await.unwrap();
    assert!(receipt.reaped && receipt.temporary_root_removed);
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("connections")).unwrap(),
        "1"
    );
}

#[tokio::test]
async fn grid_adapter_automatic_commits_exact_native_original_and_scalar_bindings_once() {
    let h = fixture(false).await;
    let value = "雪'; DROP TABLE ORDERS; --";
    let edits = prepared(
        &h,
        &[update(value), SnapshotMutation::Delete { row_index: 1 }],
    )
    .await;
    let compiled: Vec<_> = edits
        .compiled()
        .units()
        .iter()
        .map(|u| {
            (
                u.sql().to_owned(),
                u.bindings()
                    .iter()
                    .map(|b| b.value().map(str::to_owned))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let operation = h
        .driver
        .prepare_grid_operation(edits, Mode::Automatic)
        .await
        .unwrap();
    assert!(operation.manual_owner().is_none());
    assert_eq!(operation.digest().len(), 64);
    assert!(
        operation
            .digest()
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    assert!(!operation.digest().contains(value));
    let mut recorder = Recorder::new(&h, &operation);
    let retained = recorder.retained.clone();
    let mut admission = Admission {
        duplicate: true,
        ..Default::default()
    };
    let result = operation
        .execute(&mut admission, &mut recorder)
        .await
        .unwrap();
    assert_eq!(admission.calls, 1);
    assert!(result.recording_error.is_none() && result.manual.is_none());
    assert_eq!(outcome(&result.report), Outcome::Committed);
    let evidence = result.report.batch.as_ref().unwrap();
    let counts: Vec<_> = evidence
        .steps
        .iter()
        .filter_map(|s| s.count.as_ref())
        .collect();
    assert_eq!(counts.len(), 2);
    assert!(
        counts
            .iter()
            .all(|c| c.affected_rows == "1" && c.multi_joined_rows.is_none())
    );
    assert_eq!(retained.lock().originals.len(), 1);
    assert!(retained.lock().no_dispatch.is_empty());
    assert_eq!(retained.lock().originals[0].batch, result.report.batch);
    let calls = h.queries();
    for (sql, _) in &compiled {
        assert_eq!(calls.iter().filter(|s| *s == sql).count(), 1);
        assert!(!sql.contains(value));
    }
    let binds: Vec<Vec<Option<String>>> =
        std::fs::read_to_string(h.scripts.path().join("dml-binds"))
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
    assert_eq!(
        binds,
        compiled.iter().map(|(_, b)| b.clone()).collect::<Vec<_>>()
    );
    assert!(
        binds[0]
            .iter()
            .any(|b| b.as_deref() == Some("9007199254740993"))
    );
    assert!(binds[0].iter().any(Option::is_none));
    assert!(binds[1].iter().any(|b| b.as_deref() == Some("")));
    assert_eq!(dml_count(&h), 2);
    assert_eq!(calls.iter().filter(|s| s.as_str() == "COMMIT").count(), 1);
    finish(h).await;
}

#[tokio::test]
async fn grid_adapter_exact_38_digit_cardinality_rolls_back_without_suffix_write() {
    let h = fixture(false).await;
    let edits = prepared(
        &h,
        &[update("changed"), SnapshotMutation::Delete { row_index: 1 }],
    )
    .await;
    h.settings(json!({"affectedRows":"99999999999999999999999999999999999999"}));
    let operation = h
        .driver
        .prepare_grid_operation(edits, Mode::Automatic)
        .await
        .unwrap();
    let mut recorder = Recorder::new(&h, &operation);
    let result = operation
        .execute(&mut Admission::default(), &mut recorder)
        .await
        .unwrap();
    assert_eq!(outcome(&result.report), Outcome::RolledBack);
    let evidence = result.report.batch.as_ref().unwrap();
    assert_eq!(
        evidence
            .steps
            .iter()
            .find_map(|s| s.count.as_ref())
            .unwrap()
            .affected_rows,
        "99999999999999999999999999999999999999"
    );
    assert_eq!(
        evidence.summary.as_ref().unwrap().failure,
        Some(crate::drivers::snowflake_batch::Failure::Cardinality)
    );
    assert_eq!(dml_count(&h), 1);
    assert!(h.queries().iter().any(|s| s == "ROLLBACK"));
    assert!(!h.queries().iter().any(|s| s == "COMMIT"));
    finish(h).await;
}

#[tokio::test]
async fn grid_adapter_manual_pending_excludes_ordinary_and_finishes_exact_transaction() {
    for action in [FinishAction::Commit, FinishAction::Rollback] {
        let h = fixture(false).await;
        let edits = prepared(&h, &[update("pending")]).await;
        let operation = h
            .driver
            .prepare_grid_operation(edits, Mode::Manual)
            .await
            .unwrap();
        let owner = operation.manual_owner().unwrap();
        let mut recorder = Recorder::new(&h, &operation);
        let result = operation
            .execute(&mut Admission::default(), &mut recorder)
            .await
            .unwrap();
        assert_eq!(outcome(&result.report), Outcome::Pending);
        assert!(result.recording_error.is_none());
        let before = h.queries().len();
        assert_eq!(
            h.driver.relation_catalog().await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(h.queries().len(), before);
        let finish_operation = owner.prepare_finish(action).await.unwrap();
        if action == FinishAction::Rollback {
            assert_eq!(
                h.queries().len(),
                before,
                "safety rollback must have zero fresh SQL probes"
            );
        }
        let mut recorder = Recorder::new(&h, &finish_operation);
        let result = finish_operation
            .execute(&mut Admission::default(), &mut recorder)
            .await
            .unwrap();
        assert_eq!(
            outcome(&result.report),
            if action == FinishAction::Commit {
                Outcome::Committed
            } else {
                Outcome::RolledBack
            }
        );
        assert!(result.recording_error.is_none());
        h.driver.relation_catalog().await.unwrap();
        drop(result);
        drop(owner);
        assert!(!h.driver.is_closed());
        assert_eq!(dml_count(&h), 1);
        finish(h).await;
    }
}

#[tokio::test]
async fn grid_adapter_recording_failure_allows_only_zero_probe_rollback_and_exact_settlement() {
    let h = fixture(false).await;
    let edits = prepared(&h, &[update("pending")]).await;
    let operation = h
        .driver
        .prepare_grid_operation(edits, Mode::Manual)
        .await
        .unwrap();
    let a = operation.operation_id().to_owned();
    let owner = operation.manual_owner().unwrap();
    let mut recorder = Recorder::new(&h, &operation);
    recorder.fail = true;
    let result = operation
        .execute(&mut Admission::default(), &mut recorder)
        .await
        .unwrap();
    assert_eq!(outcome(&result.report), Outcome::Pending);
    assert!(result.recording_error.is_some());
    let before = h.queries().len();
    assert!(owner.prepare_finish(FinishAction::Commit).await.is_err());
    assert_eq!(h.queries().len(), before);
    let rollback = owner.prepare_finish(FinishAction::Rollback).await.unwrap();
    assert_eq!(h.queries().len(), before);
    let b = rollback.operation_id().to_owned();
    assert_ne!(a, b);
    let mut recorder = Recorder::new(&h, &rollback);
    recorder.fail = true;
    let result = rollback
        .execute(&mut Admission::default(), &mut recorder)
        .await
        .unwrap();
    assert_eq!(outcome(&result.report), Outcome::RolledBack);
    assert!(result.recording_error.is_some());
    assert!(owner.recording_settled(&a).is_err());
    let before = h.queries().len();
    assert!(h.driver.relation_catalog().await.is_err());
    assert_eq!(h.queries().len(), before);
    owner.recording_settled(&b).unwrap();
    assert!(owner.recording_settled(&a).is_err());
    assert!(owner.recording_settled(&b).is_err());
    h.driver.relation_catalog().await.unwrap();
    drop(result);
    drop(owner);
    assert!(!h.driver.is_closed());
    assert_eq!(dml_count(&h), 1);
    finish(h).await;
}

#[tokio::test]
async fn grid_adapter_admission_refusal_proves_no_dispatch_and_preserves_pending_rollback() {
    let h = fixture(false).await;
    let edits = prepared(&h, &[update("pending")]).await;
    let operation = h
        .driver
        .prepare_grid_operation(edits, Mode::Manual)
        .await
        .unwrap();
    let owner = operation.manual_owner().unwrap();
    let mut recorder = Recorder::new(&h, &operation);
    operation
        .execute(&mut Admission::default(), &mut recorder)
        .await
        .unwrap();
    // A refused Commit has not claimed the protected rollback dispatch. It can
    // still preserve Pending ownership for an explicit safety rollback.
    let operation = owner.prepare_finish(FinishAction::Commit).await.unwrap();
    let mut recorder = Recorder::new(&h, &operation);
    let retained = recorder.retained.clone();
    let baseline = ledger_frames(&h).len();
    let before = h.queries().len();
    let mut refusal = Admission {
        refuse: true,
        ..Default::default()
    };
    assert!(
        operation
            .execute(&mut refusal, &mut recorder)
            .await
            .is_err()
    );
    assert_eq!(refusal.calls, 1);
    assert_eq!(h.queries().len(), before);
    assert_eq!(ledger_frames(&h).len(), baseline);
    assert!(retained.lock().originals.is_empty());
    assert_eq!(retained.lock().no_dispatch.len(), 1);
    assert!(owner.prepare_finish(FinishAction::Commit).await.is_err());
    let rollback = owner.prepare_finish(FinishAction::Rollback).await.unwrap();
    assert_eq!(h.queries().len(), before);
    let mut recorder = Recorder::new(&h, &rollback);
    assert_eq!(
        outcome(
            &rollback
                .execute(&mut Admission::default(), &mut recorder)
                .await
                .unwrap()
                .report
        ),
        Outcome::RolledBack
    );
    drop(owner);
    assert!(!h.driver.is_closed());
    finish(h).await;
}

#[tokio::test]
async fn grid_adapter_refused_protected_rollback_retains_proof_then_reaps_without_retry() {
    use futures_util::FutureExt;

    struct RollbackRefusalRecorder {
        inner: Recorder,
        driver: Arc<SnowflakeExternal>,
        panic: bool,
    }
    impl GridOriginalRecorder for RollbackRefusalRecorder {
        fn record_original(&mut self, _: &Report, _: &Operation) -> AppResult<()> {
            panic!("refused rollback must not invent an original terminal");
        }
        fn record_no_dispatch(&mut self, proof: &GridNoDispatch) -> AppResult<()> {
            assert!(
                !self.driver.is_closed(),
                "retain the proof before retirement"
            );
            self.inner.record_no_dispatch(proof)?;
            assert!(
                !self.panic,
                "synthetic refusal recorder panic after proof retention"
            );
            Ok(())
        }
    }

    for panic in [false, true] {
        let h = fixture(false).await;
        let edits = prepared(&h, &[update("pending")]).await;
        let operation = h
            .driver
            .prepare_grid_operation(edits, Mode::Manual)
            .await
            .unwrap();
        let owner = operation.manual_owner().unwrap();
        let mut recorder = Recorder::new(&h, &operation);
        let first_recordings = recorder.retained.clone();
        let pending = operation
            .execute(&mut Admission::default(), &mut recorder)
            .await
            .unwrap();
        assert_eq!(outcome(&pending.report), Outcome::Pending);
        let prior = pending.report.batch.clone();
        let before = h.queries().len();
        let rollback = owner.prepare_finish(FinishAction::Rollback).await.unwrap();
        assert_eq!(
            h.queries().len(),
            before,
            "safety rollback adds no fresh probes"
        );
        let recorder = Recorder::new(&h, &rollback);
        let proof_slot = recorder.retained.clone();
        let mut recorder = RollbackRefusalRecorder {
            inner: recorder,
            driver: h.driver.clone(),
            panic,
        };
        let mut admission = Admission {
            refuse: true,
            ..Default::default()
        };
        let caught = std::panic::AssertUnwindSafe(rollback.execute(&mut admission, &mut recorder))
            .catch_unwind()
            .await;
        if panic {
            assert!(
                caught.is_err(),
                "recorder panic must propagate after retaining proof"
            );
        } else {
            assert!(matches!(caught, Ok(Err(_))));
        }
        assert_eq!(admission.calls, 1);
        assert_eq!(proof_slot.lock().no_dispatch.len(), 1);
        assert!(proof_slot.lock().originals.is_empty());
        assert!(h.driver.is_closed());
        assert!(
            h.cleanup
                .wait(Duration::from_secs(10))
                .await
                .unwrap()
                .reaped
        );
        assert_eq!(h.queries().len(), before);
        assert!(
            !h.queries()
                .iter()
                .any(|sql| sql == "ROLLBACK" || sql == "COMMIT")
        );
        assert_eq!(dml_count(&h), 1);
        assert_eq!(pending.report.batch, prior);
        assert_eq!(first_recordings.lock().originals.len(), 1);
        assert_eq!(first_recordings.lock().originals[0].batch, prior);
        assert!(owner.prepare_finish(FinishAction::Rollback).await.is_err());
        assert!(owner.prepare_finish(FinishAction::Commit).await.is_err());
        assert_eq!(proof_slot.lock().no_dispatch.len(), 1);
        finish(h).await;
    }
}

#[tokio::test]
async fn grid_adapter_revoked_final_binding_refuses_before_admission_or_sql() {
    let h = fixture(false).await;
    let edits = prepared(&h, &[update("revoked")]).await;
    let operation = h
        .driver
        .prepare_grid_operation(edits, Mode::Manual)
        .await
        .unwrap();
    let owner = operation.manual_owner().unwrap();
    let mut recorder = Recorder::new(&h, &operation);
    let retained = recorder.retained.clone();
    h.driver.edit_snapshots.clear();
    let before = h.queries().len();
    let mut admission = Admission::default();
    assert_eq!(
        operation
            .execute(&mut admission, &mut recorder)
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(admission.calls, 0);
    assert_eq!(h.queries().len(), before);
    assert!(retained.lock().originals.is_empty());
    assert_eq!(retained.lock().no_dispatch.len(), 1);
    drop(owner);
    assert!(!h.driver.is_closed());
    h.driver.relation_catalog().await.unwrap();
    assert_eq!(dml_count(&h), 0);
    finish(h).await;
}

#[tokio::test]
async fn grid_adapter_escaped_oversize_frame_refuses_before_begin_and_releases_unsubmitted_owner() {
    let h = fixture(true).await;
    let value = "\u{1}".repeat(200_000);
    assert!(
        serde_json::to_vec(&value).unwrap().len()
            > super::super::super::snowflake_process::MAX_FRAME
    );
    let edits = prepared(&h, &[update(&value)]).await;
    assert_eq!(edits.compiled().units().len(), 1);
    let result = h.driver.prepare_grid_operation(edits, Mode::Manual).await;
    assert_eq!(result.err().unwrap().code, ErrorCode::ResultLimitReached);
    assert_eq!(dml_count(&h), 0);
    assert!(!h.queries().iter().any(|s| s == "BEGIN TRANSACTION"));
    assert!(!h.driver.is_closed());
    h.driver.relation_catalog().await.unwrap();
    finish(h).await;
}

#[tokio::test]
async fn grid_adapter_dropped_execute_and_panicking_recorder_retire_without_replay() {
    for panic in [false, true] {
        let h = fixture(false).await;
        let edits = prepared(&h, &[update("once")]).await;
        let operation = h
            .driver
            .prepare_grid_operation(edits, Mode::Manual)
            .await
            .unwrap();
        let owner = operation.manual_owner().unwrap();
        let mut recorder = Recorder::new(&h, &operation);
        recorder.panic = panic;
        let retained = recorder.retained.clone();
        if !panic {
            h.settings(json!({"holdDmlDrain":true}));
        }
        let task = tokio::spawn(async move {
            operation
                .execute(&mut Admission::default(), &mut recorder)
                .await
        });
        if panic {
            assert!(
                tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .err()
                    .unwrap()
                    .is_panic()
            );
            assert_eq!(retained.lock().originals.len(), 1);
            assert_eq!(outcome(&retained.lock().originals[0]), Outcome::Pending);
        } else {
            h.wait_until(|| h.scripts.path().join("held-dml-drain").exists())
                .await;
            task.abort();
            assert!(matches!(task.await,Err(error) if error.is_cancelled()));
            assert!(retained.lock().originals.is_empty());
        }
        assert!(h.driver.is_closed());
        assert!(
            h.cleanup
                .wait(Duration::from_secs(10))
                .await
                .unwrap()
                .reaped
        );
        assert_eq!(dml_count(&h), 1);
        assert!(!h.queries().iter().any(|s| s == "COMMIT" || s == "ROLLBACK"));
        assert!(owner.prepare_finish(FinishAction::Rollback).await.is_err());
        drop(owner);
        finish(h).await;
    }
}

// Unlike Recorder above, this case uses real isolated SQLite. The SQL execution
// evidence still comes from the production helper with the injected SDK.
#[tokio::test]
async fn grid_adapter_actual_noqueue_proof_authorizes_protected_sqlite_rollback() {
    use crate::persistence::{Persistence, batch_outcomes::*};
    struct PendingRecorder<'a> {
        store: &'a Persistence,
        owner: &'a Arc<crate::persistence::HistoryOwner>,
        gate: &'a crate::persistence::OperationGate,
        reservation: &'a BatchReservation,
        authority: Option<PendingBatchRollbackAuthority>,
        original: Option<OriginalBatchOutcome>,
    }
    impl GridOriginalRecorder for PendingRecorder<'_> {
        fn record_original(&mut self, report: &Report, operation: &Operation) -> AppResult<()> {
            let original = OriginalBatchOutcome::capture_in(
                self.reservation,
                report.batch.as_ref().unwrap(),
                BatchTermination::Complete,
                2,
                Vec::with_capacity(MAX_ENVELOPE_BYTES),
            )?;
            self.original = Some(original);
            let original = self.original.as_ref().unwrap();
            self.authority = Some(self.store.authorize_pending_batch_rollback(
                self.owner,
                self.gate,
                self.reservation,
                original,
                operation,
            )?);
            self.store
                .capture_batch_outcome(self.owner, self.gate, self.reservation, original)?;
            self.store
                .settle_batch_outcome(self.owner, self.gate, self.reservation, original)?;
            Ok(())
        }
        fn record_no_dispatch(&mut self, _: &GridNoDispatch) -> AppResult<()> {
            panic!("initial Pending must dispatch");
        }
    }
    struct RollbackRecorder<'a> {
        store: &'a Persistence,
        owner: &'a Arc<crate::persistence::HistoryOwner>,
        gate: &'a crate::persistence::OperationGate,
        token: &'a VolatileBatchRollbackToken,
        original: Option<OriginalBatchOutcome>,
    }
    impl GridOriginalRecorder for RollbackRecorder<'_> {
        fn record_original(&mut self, report: &Report, _: &Operation) -> AppResult<()> {
            // Retain the actual terminal before any fallible SQLite promotion.
            let original = OriginalBatchOutcome::volatile_rollback_in(
                self.token,
                report.batch.as_ref().unwrap(),
                BatchTermination::Complete,
                4,
                Vec::with_capacity(MAX_ENVELOPE_BYTES),
            )?;
            self.original = Some(original);
            let reservation = self
                .store
                .promote_volatile_batch_rollback(self.owner, self.gate, self.token)?;
            let original = self.original.as_ref().unwrap();
            self.store
                .capture_batch_outcome(self.owner, self.gate, &reservation, original)?;
            self.store
                .settle_batch_outcome(self.owner, self.gate, &reservation, original)?;
            Ok(())
        }
        fn record_no_dispatch(&mut self, _: &GridNoDispatch) -> AppResult<()> {
            panic!("protected rollback must dispatch");
        }
    }
    fn descriptor(h: &Harness, operation: &PreparedGridOperation) -> BatchDescriptor {
        BatchDescriptor {
            operation_id: operation.operation_id().into(),
            connection_id: h.driver.profile.id.clone(),
            session_id: h.driver.session_id.clone(),
            helper_generation: h.driver.process.helper_generation().generation().into(),
            started_at: 1,
            digest: operation.digest().into(),
        }
    }

    let mut h = fixture(false).await;
    // Production app session IDs are canonical UUIDs. Set this test-owned
    // driver's app identity before creating any preview/binding/group.
    Arc::get_mut(&mut h.driver).unwrap().session_id = new_id();
    let root = tempfile::tempdir().unwrap();
    let store = Persistence::open(root.path()).unwrap();
    let history_owner = Arc::new(store.claim_history_owner().unwrap());
    let gate = store.operation_gate(&history_owner).unwrap();
    let edits = prepared(&h, &[update("pending")]).await;
    let operation = h
        .driver
        .prepare_grid_operation(edits, Mode::Manual)
        .await
        .unwrap();
    let manual = operation.manual_owner().unwrap();
    let first = descriptor(&h, &operation);
    let reservation = store
        .reserve_owned_batch(
            &history_owner,
            &gate,
            &first,
            &BatchPlan::new(operation.request().clone()).unwrap(),
            None,
        )
        .unwrap();
    let group = reservation.manual_transaction().unwrap();
    store
        .pin_manual_transaction_helper(
            &history_owner,
            &gate,
            &reservation,
            &h.driver.process.helper_generation(),
        )
        .unwrap();
    let mut recorder = PendingRecorder {
        store: &store,
        owner: &history_owner,
        gate: &gate,
        reservation: &reservation,
        authority: None,
        original: None,
    };
    let result = operation
        .execute(&mut Admission::default(), &mut recorder)
        .await
        .unwrap();
    assert_eq!(outcome(&result.report), Outcome::Pending);
    assert!(result.recording_error.is_none());
    let authority = recorder.authority.take().unwrap();
    let first_bytes = recorder.original.as_ref().unwrap().as_bytes().to_vec();
    drop(recorder);
    assert_eq!(
        store
            .batch_evidence(&first.operation_id)
            .unwrap()
            .unwrap()
            .as_bytes(),
        first_bytes
    );

    let commit = manual.prepare_finish(FinishAction::Commit).await.unwrap();
    let commit_d = descriptor(&h, &commit);
    let commit_request = commit.request().clone();
    let successor = store
        .reserve_owned_batch_finish(
            &history_owner,
            &gate,
            &commit_d,
            &group,
            FinishAction::Commit,
        )
        .unwrap();
    let mut recorder = Recorder::new(&h, &commit);
    let proof_slot = recorder.retained.clone();
    let before = h.queries().len();
    assert!(
        commit
            .execute(
                &mut Admission {
                    refuse: true,
                    ..Default::default()
                },
                &mut recorder
            )
            .await
            .is_err()
    );
    assert_eq!(h.queries().len(), before);
    assert_eq!(proof_slot.lock().no_dispatch.len(), 1);
    let proof = proof_slot.lock().no_dispatch[0].clone();
    let rollback = manual.prepare_finish(FinishAction::Rollback).await.unwrap();
    assert_eq!(
        h.queries().len(),
        before,
        "no new metadata before safety rollback"
    );
    let rollback_d = descriptor(&h, &rollback);
    // The exact proof was emitted by execute's no-queue branch; no constructor
    // or serialized success flag can replace it.
    let token = store
        .make_undispatched_batch_rollback_token(
            &history_owner,
            &gate,
            &rollback_d,
            &authority,
            &successor,
            &commit_request,
            &proof,
        )
        .unwrap();
    let mut recorder = RollbackRecorder {
        store: &store,
        owner: &history_owner,
        gate: &gate,
        token: &token,
        original: None,
    };
    let result = rollback
        .execute(&mut Admission::default(), &mut recorder)
        .await
        .unwrap();
    assert_eq!(outcome(&result.report), Outcome::RolledBack);
    assert!(result.recording_error.is_none());
    let page = store
        .manual_transaction_evidence(group.id(), 0, 64)
        .unwrap();
    assert_eq!(page.disposition, "rolledBack");
    assert_eq!(
        page.members
            .iter()
            .map(|m| m.operation_id.as_str())
            .collect::<Vec<_>>(),
        [
            first.operation_id.as_str(),
            commit_d.operation_id.as_str(),
            rollback_d.operation_id.as_str()
        ]
    );
    assert!(
        store
            .batch_evidence(&commit_d.operation_id)
            .unwrap()
            .is_none(),
        "no-dispatch successor has no invented ACK/original"
    );
    assert_eq!(
        store
            .batch_evidence(&first.operation_id)
            .unwrap()
            .unwrap()
            .as_bytes(),
        first_bytes
    );
    let retained = store
        .batch_evidence(&rollback_d.operation_id)
        .unwrap()
        .unwrap();
    assert!(retained.settled && !retained.imported);
    assert_eq!(
        retained.as_bytes(),
        recorder.original.as_ref().unwrap().as_bytes()
    );
    assert_eq!(dml_count(&h), 1);
    assert_eq!(
        h.queries()
            .iter()
            .filter(|s| s.as_str() == "ROLLBACK")
            .count(),
        1
    );
    assert!(!h.queries().iter().any(|s| s == "COMMIT"));
    h.driver.relation_catalog().await.unwrap();
    drop(result);
    drop(manual);
    assert!(!h.driver.is_closed());
    finish(h).await;
}
