// Studio/ResultStore regression fixture only. No SDK or database connection is
// created; the existing SDK profile fixture supplies local policy/session state.
use super::*;
use serde_json::{Value, json};

const SCRIPT: &str = "SELECT 'first' AS marker; SELECT 'distinctive suffix' AS marker";

#[test]
fn unpolled_generic_task_releases_job_and_records_no_original_without_dispatch() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = Fixture::new(Behavior::Complete);
    let (run_id, _events) = {
        let _entered = runtime.enter();
        fixture.start()
    };
    assert!(fixture.driver.calls.lock().is_empty());
    assert_eq!(
        fixture
            .studio
            .history_entry(&run_id)
            .unwrap()
            .unwrap()
            .provenance,
        HistoryProvenance::Admitted
    );
    // No executor tick has polled the task. Its pre-spawn guard must still run.
    drop(runtime);
    assert!(fixture.driver.calls.lock().is_empty());
    assert!(fixture.studio.jobs.lock().is_empty());
    assert!(fixture.session.closed.load(Ordering::Acquire));
    let entry = fixture.studio.history_entry(&run_id).unwrap().unwrap();
    assert_eq!(entry.provenance, HistoryProvenance::Interrupted);
    assert!(entry.status.is_none());
    assert!(entry.elapsed_ms.is_none());
    assert!(
        fixture
            .studio
            .persistence
            .audit_log()
            .unwrap()
            .iter()
            .all(|audit| audit.id != format!("{run_id}:finished"))
    );
    fixture.studio.flush_pending_history().unwrap();
    assert!(fixture.studio.pending_history.lock().is_empty());
}

#[derive(Clone, Copy, Debug)]
enum Behavior {
    Complete,
    RowLimit,
    ByteLimit,
    Error(ErrorCode),
    ErrorOnSuffix(ErrorCode),
    PreflightFailure,
    HeldCancellation,
    AcknowledgedCancellation,
    HeldCancellationOnSuffix,
    HeldObservation,
    PanicBeforeAck,
}

struct ScriptDriver {
    inner: Arc<RecordingDriver>,
    behavior: Behavior,
    calls: Mutex<Vec<String>>,
    bound_calls: std::sync::atomic::AtomicUsize,
    entered: tokio::sync::Notify,
    cancelled: CancellationToken,
}

#[async_trait::async_trait]
impl Driver for ScriptDriver {
    async fn execute(
        &self,
        sql: &str,
        kind: StatementKind,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        assert_eq!(kind, StatementKind::Read);
        self.calls.lock().push(sql.into());
        assert!(
            !matches!(self.behavior, Behavior::PanicBeforeAck),
            "synthetic driver panic before ACK"
        );
        if sql.contains("distinctive suffix") {
            if let Behavior::ErrorOnSuffix(code) = self.behavior {
                return Err(AppError::new(code, "Synthetic suffix failure"));
            }
            if matches!(self.behavior, Behavior::HeldCancellationOnSuffix) {
                self.entered.notify_one();
                self.cancelled.cancelled().await;
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "Synthetic suffix cancellation",
                ));
            }
        }
        if sql.contains("'first'") {
            match self.behavior {
                Behavior::Error(code) => {
                    return Err(AppError::new(code, "Synthetic first-read failure"));
                }
                Behavior::HeldCancellation => {
                    self.entered.notify_one();
                    self.cancelled.cancelled().await;
                    return Err(AppError::new(
                        ErrorCode::Cancelled,
                        "Synthetic first-read cancellation",
                    ));
                }
                Behavior::AcknowledgedCancellation => {
                    self.entered.notify_one();
                    self.cancelled.cancelled().await;
                    // The original call can still acknowledge success after a
                    // cancellation request; retain that fact independently.
                }
                _ => {}
            }
        }
        let mut column = schema(EngineKind::Postgres).columns.remove(2);
        column.id = "c0".into();
        column.name = "marker".into();
        sink.describe(vec![column])?;
        assert!(sink.rows(vec![vec![Some("retained prefix".into())]])?);
        let limited = sql.contains("'first'")
            && matches!(self.behavior, Behavior::RowLimit | Behavior::ByteLimit);
        if limited {
            let value = if matches!(self.behavior, Behavior::ByteLimit) {
                "x".repeat(16 * 1024)
            } else {
                "second row".into()
            };
            assert!(
                !sink.rows(vec![vec![Some(value)]])?,
                "the real ResultStore must reject the extra row/bytes"
            );
            sink.limit("Synthetic driver drained after the client retention limit");
        }
        let rows_seen = if limited { 2 } else { 1 };
        sink.complete(rows_seen)?;
        Ok(rows_seen)
    }
    async fn execute_bound(
        &self,
        sql: &str,
        kind: StatementKind,
        bound: &crate::params::BoundStatement,
        sink: &mut dyn QuerySink,
    ) -> AppResult<u64> {
        assert!(!bound.binds.is_empty());
        self.bound_calls.fetch_add(1, Ordering::Relaxed);
        self.execute(sql, kind, sink).await
    }
    async fn invalidate_edit_targets(&self) -> AppResult<()> {
        if matches!(self.behavior, Behavior::PreflightFailure) {
            return Err(AppError::new(
                ErrorCode::OutcomeUnknown,
                "Synthetic lease teardown failure before SQL",
            ));
        }
        Ok(())
    }
    async fn cancel(&self) -> AppResult<()> {
        self.cancelled.cancel();
        Ok(())
    }
    async fn session_state(&self, id: &str) -> AppResult<SessionRuntimeState> {
        if matches!(self.behavior, Behavior::HeldObservation) {
            self.entered.notify_one();
            self.cancelled.cancelled().await;
        }
        self.inner.session_state(id).await
    }
    async fn catalog(&self) -> AppResult<Vec<TableInfo>> {
        self.inner.catalog().await
    }
    async fn schema(&self, table: &TableRef) -> AppResult<TableSchema> {
        self.inner.schema(table).await
    }
    async fn browse(
        &self,
        schema: &TableSchema,
        after: Option<Vec<Option<String>>>,
    ) -> AppResult<crate::drivers::BrowseRows> {
        self.inner.browse(schema, after).await
    }
    async fn apply(&self, schema: &TableSchema, rows: &[RowMutation]) -> AppResult<()> {
        self.inner.apply(schema, rows).await
    }
    async fn processes(&self) -> AppResult<Vec<ProcessInfo>> {
        self.inner.processes().await
    }
    async fn cancel_process(&self, id: &str, identity: &str) -> AppResult<()> {
        self.inner.cancel_process(id, identity).await
    }
    async fn health(&self) -> AppResult<u32> {
        self.inner.health().await
    }
    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    fn close(&self) {
        self.inner.close();
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    studio: Arc<Studio>,
    driver: Arc<ScriptDriver>,
    session: Arc<Session>,
}

impl Fixture {
    fn new(behavior: Behavior) -> Self {
        let (root, mut studio, inner, previous) = sdk_dml_fixture(SafeMode::ReadOnly, None);
        // Keep production settings intact. Only this disposable in-memory store
        // gets tiny deterministic limits, exercising the actual Output path.
        Arc::get_mut(&mut studio).unwrap().results = ResultStore::new(
            if matches!(behavior, Behavior::ByteLimit) {
                4096
            } else {
                1024 * 1024
            },
            if matches!(behavior, Behavior::RowLimit) {
                1
            } else {
                100
            },
        );
        let driver = Arc::new(ScriptDriver {
            inner,
            behavior,
            calls: Mutex::new(Vec::new()),
            bound_calls: std::sync::atomic::AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
            cancelled: CancellationToken::new(),
        });
        let mut info = previous.info.clone();
        let mut profile = previous.profile.clone();
        if matches!(behavior, Behavior::PreflightFailure) {
            info.engine = EngineKind::Mysql;
            profile.engine = EngineKind::Mysql;
            profile.snowflake = None;
            profile.safe_mode = SafeMode::Silent;
            studio.persistence.save_profile(&profile).unwrap();
        }
        let session = Arc::new(Session {
            rollback_memory: Mutex::new(None),
            #[cfg(unix)]
            snowflake_external: None,
            info,
            profile,
            driver: driver.clone(),
            lane: tokio::sync::Mutex::new(()),
            cancellation_gate: tokio::sync::Mutex::new(()),
            active_run: Mutex::new(None),
            closed: AtomicBool::new(false),
            tunnel: None,
            completion: Mutex::new(None),
            metadata_gate: Mutex::new(MetadataGate::default()),
            observed_runtime: Mutex::new(previous.observed_runtime.lock().clone()),
        });
        studio
            .sessions
            .lock()
            .insert(session.info.id.clone(), session.clone());
        Self {
            _root: root,
            studio,
            driver,
            session,
        }
    }

    fn start(&self) -> (String, tokio::sync::mpsc::UnboundedReceiver<QueryEvent>) {
        self.start_sql(SCRIPT, None)
    }

    fn start_sql(
        &self,
        sql: &str,
        params: Option<Vec<QueryParamInput>>,
    ) -> (String, tokio::sync::mpsc::UnboundedReceiver<QueryEvent>) {
        let plan = self
            .studio
            .prepare_execution_with_params(&self.session.info.id, sql, params)
            .unwrap();
        let approval = plan.requires_approval.then(|| {
            self.studio
                .authorize(&plan.id, None, plan.confirmation_text.as_deref())
                .unwrap()
        });
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let id = self
            .studio
            .execute(
                &plan.id,
                approval.as_deref(),
                Arc::new(move |event| tx.send(event).is_ok()),
            )
            .unwrap();
        (id, rx)
    }

    async fn finish(
        &self,
        id: &str,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<QueryEvent>,
    ) -> Evidence {
        let events = tokio::time::timeout(Duration::from_secs(5), async {
            let mut events = Vec::new();
            while let Some(event) = rx.recv().await {
                let done = matches!(event, QueryEvent::Finished { .. });
                events.push(serde_json::to_value(event).unwrap());
                if done {
                    return events;
                }
            }
            panic!("query events ended without Finished");
        })
        .await
        .unwrap();
        let history = serde_json::to_value(
            self.studio
                .history("")
                .unwrap()
                .into_iter()
                .find(|entry| entry.id == id)
                .unwrap(),
        )
        .unwrap();
        Evidence {
            events,
            history,
            calls: self.driver.calls.lock().clone(),
        }
    }
}

struct Evidence {
    events: Vec<Value>,
    history: Value,
    calls: Vec<String>,
}
impl Evidence {
    fn finished(&self) -> &Value {
        self.events
            .iter()
            .find(|event| event["type"] == "finished")
            .unwrap()
    }
    fn indices(&self, kind: &str) -> Vec<Value> {
        self.events
            .iter()
            .filter(|event| event["type"] == kind)
            .map(|event| event["index"].clone())
            .collect()
    }
    fn assert_status(&self, expected: &str) {
        assert_eq!(self.finished()["status"], expected);
        assert_eq!(self.history["status"], expected);
    }
    fn assert_summary(&self, attempted: usize, completed: usize, stop: Value) {
        self.assert_summary_for(2, attempted, completed, stop);
    }
    fn assert_summary_for(&self, planned: usize, attempted: usize, completed: usize, stop: Value) {
        let expected = json!({"planned":planned,"attempted":attempted,"completed":completed,"notAttempted":planned-attempted,"stopReason":stop});
        assert_eq!(self.finished()["executionSummary"], expected);
        assert_eq!(self.history["executionSummary"], expected);
    }
}

async fn assert_limited_script(behavior: Behavior) {
    let fixture = Fixture::new(behavior);
    let (id, rx) = fixture.start();
    let evidence = fixture.finish(&id, rx).await;
    assert_eq!(evidence.calls.len(), 1);
    assert!(evidence.calls[0].contains("'first'"));
    assert!(
        !evidence
            .calls
            .iter()
            .any(|sql| sql.contains("distinctive suffix"))
    );
    assert_eq!(evidence.indices("statementStarted"), vec![json!(0)]);
    assert_eq!(evidence.indices("statementCompleted"), vec![json!(0)]);
    let retained = evidence
        .events
        .iter()
        .rfind(|event| event["type"] == "result")
        .unwrap();
    assert_eq!(retained["result"]["status"], "truncated");
    assert_eq!(retained["result"]["retainedRows"], 1);
    let result_id = retained["result"]["id"].as_str().unwrap();
    assert_eq!(
        fixture.studio.results.row(result_id, 0).unwrap(),
        vec![Some("retained prefix".into())]
    );
    assert_eq!(evidence.history["statementCount"], 2);
    // Fails before the product fix: the first read has an acknowledged terminal,
    // but its unattempted suffix must not be reported as a completed script.
    evidence.assert_status("stopped");
    evidence.assert_summary(1, 1, json!("resultLimit"));
}

#[tokio::test]
async fn script_accounting_row_limit_reports_the_unattempted_suffix() {
    assert_limited_script(Behavior::RowLimit).await;
}

#[tokio::test]
async fn script_accounting_byte_limit_reports_the_unattempted_suffix() {
    assert_limited_script(Behavior::ByteLimit).await;
}

#[tokio::test]
async fn script_accounting_nontruncated_control_executes_both_reads() {
    let fixture = Fixture::new(Behavior::Complete);
    let (id, rx) = fixture.start();
    let evidence = fixture.finish(&id, rx).await;
    assert_eq!(evidence.calls.len(), 2);
    assert!(evidence.calls[1].contains("distinctive suffix"));
    assert_eq!(
        evidence.indices("statementStarted"),
        vec![json!(0), json!(1)]
    );
    assert_eq!(
        evidence.indices("statementCompleted"),
        vec![json!(0), json!(1)]
    );
    evidence.assert_status("completed");
    evidence.assert_summary(2, 2, Value::Null);
    assert_eq!(evidence.history["statementCount"], 2);
}

#[tokio::test]
async fn script_accounting_database_error_and_unknown_outcome_stop_the_suffix() {
    for (code, status) in [
        (ErrorCode::Database, "failed"),
        (ErrorCode::OutcomeUnknown, "outcomeUnknown"),
    ] {
        let fixture = Fixture::new(Behavior::Error(code));
        let (id, rx) = fixture.start();
        let evidence = fixture.finish(&id, rx).await;
        assert_eq!(evidence.calls.len(), 1);
        assert_eq!(evidence.indices("statementStarted"), vec![json!(0)]);
        assert!(evidence.indices("statementCompleted").is_empty());
        evidence.assert_status(status);
        evidence.assert_summary(1, 0, json!(status));
        assert_eq!(evidence.history["statementCount"], 2);
    }
}

#[tokio::test]
async fn script_accounting_queued_cancellation_attempts_no_statement() {
    let fixture = Fixture::new(Behavior::Complete);
    let lane = fixture.session.lane.lock().await;
    let (id, rx) = fixture.start();
    fixture.studio.cancel(&id).await.unwrap();
    drop(lane);
    let evidence = fixture.finish(&id, rx).await;
    assert!(evidence.calls.is_empty());
    assert!(evidence.indices("statementStarted").is_empty());
    evidence.assert_status("cancelled");
    evidence.assert_summary(0, 0, json!("cancelled"));
}

#[tokio::test]
async fn script_accounting_inflight_cancellation_attempts_only_the_original_statement() {
    let fixture = Fixture::new(Behavior::HeldCancellation);
    let (id, rx) = fixture.start();
    tokio::time::timeout(Duration::from_secs(5), fixture.driver.entered.notified())
        .await
        .unwrap();
    fixture.studio.cancel(&id).await.unwrap();
    let evidence = fixture.finish(&id, rx).await;
    assert_eq!(evidence.calls.len(), 1);
    assert_eq!(evidence.indices("statementStarted"), vec![json!(0)]);
    assert!(evidence.indices("statementCompleted").is_empty());
    evidence.assert_status("cancelled");
    evidence.assert_summary(1, 0, json!("cancelled"));
}

#[tokio::test]
async fn script_accounting_preparation_rejection_creates_no_run_or_history() {
    let fixture = Fixture::new(Behavior::Complete);
    for sql in [
        "SELECT 'first'; DELETE FROM DB.PUBLIC.ORDERS",
        "SELECT 'first'; SELECT unknown_function()",
    ] {
        assert!(
            fixture
                .studio
                .prepare_execution(&fixture.session.info.id, sql)
                .is_err()
        );
        assert!(fixture.driver.calls.lock().is_empty());
        assert!(fixture.studio.jobs.lock().is_empty());
        assert!(fixture.studio.history("").unwrap().is_empty());
    }
}

#[tokio::test]
async fn script_accounting_last_or_single_limited_read_is_completed() {
    for sql in [
        "SELECT 'first' AS marker",
        "SELECT 'prefix' AS marker; SELECT 'first' AS marker",
    ] {
        let fixture = Fixture::new(Behavior::RowLimit);
        let (id, rx) = fixture.start_sql(sql, None);
        let evidence = fixture.finish(&id, rx).await;
        let count = evidence.calls.len();
        assert_eq!(count, if sql.contains(';') { 2 } else { 1 });
        evidence.assert_status("completed");
        evidence.assert_summary_for(count, count, count, json!("resultLimit"));
    }
}

#[tokio::test]
async fn script_accounting_error_after_completed_prefix_preserves_both_counts() {
    for (code, status) in [
        (ErrorCode::Database, "failed"),
        (ErrorCode::OutcomeUnknown, "outcomeUnknown"),
    ] {
        let fixture = Fixture::new(Behavior::ErrorOnSuffix(code));
        let (id, rx) = fixture.start_sql(&format!("{SCRIPT}; SELECT 'never attempted'"), None);
        let evidence = fixture.finish(&id, rx).await;
        assert_eq!(evidence.calls.len(), 2);
        assert_eq!(evidence.indices("statementCompleted"), vec![json!(0)]);
        evidence.assert_status(status);
        evidence.assert_summary_for(3, 2, 1, json!(status));
    }
}

#[tokio::test]
async fn script_accounting_preflight_error_has_started_event_but_no_driver_attempt() {
    let fixture = Fixture::new(Behavior::PreflightFailure);
    let (id, rx) = fixture.start_sql("CREATE TABLE fixture (id INT); SELECT 'suffix'", None);
    let evidence = fixture.finish(&id, rx).await;
    assert_eq!(evidence.indices("statementStarted"), vec![json!(0)]);
    assert!(evidence.calls.is_empty());
    // Lease teardown failed before execute; it is not an unknown SQL outcome.
    evidence.assert_status("failed");
    evidence.assert_summary(0, 0, json!("failed"));
}

#[tokio::test]
async fn script_accounting_bound_driver_calls_are_counted_once() {
    let fixture = Fixture::new(Behavior::Complete);
    let (id, rx) = fixture.start_sql(
        "SELECT :first AS marker; SELECT :second AS marker",
        Some(vec![
            QueryParamInput {
                key: "first".into(),
                kind: QueryParamKind::Text,
                value: Some("one".into()),
            },
            QueryParamInput {
                key: "second".into(),
                kind: QueryParamKind::Text,
                value: Some("two".into()),
            },
        ]),
    );
    let evidence = fixture.finish(&id, rx).await;
    assert_eq!(fixture.driver.bound_calls.load(Ordering::Relaxed), 2);
    assert_eq!(evidence.calls.len(), 2);
    evidence.assert_status("completed");
    evidence.assert_summary(2, 2, Value::Null);
}

#[tokio::test]
async fn script_accounting_cancellation_after_a_prefix_keeps_completed_acknowledgements() {
    for (behavior, sql, planned, attempted, completed) in [
        (
            Behavior::AcknowledgedCancellation,
            SCRIPT.to_string(),
            2,
            1,
            1,
        ),
        (
            Behavior::HeldCancellationOnSuffix,
            format!("{SCRIPT}; SELECT 'never attempted'"),
            3,
            2,
            1,
        ),
    ] {
        let fixture = Fixture::new(behavior);
        let (id, rx) = fixture.start_sql(&sql, None);
        tokio::time::timeout(Duration::from_secs(5), fixture.driver.entered.notified())
            .await
            .unwrap();
        fixture.studio.cancel(&id).await.unwrap();
        let evidence = fixture.finish(&id, rx).await;
        assert_eq!(evidence.calls.len(), attempted);
        assert_eq!(evidence.indices("statementCompleted"), vec![json!(0)]);
        evidence.assert_status("cancelled");
        evidence.assert_summary_for(planned, attempted, completed, json!("cancelled"));
    }
}

#[tokio::test]
async fn script_accounting_declined_started_event_does_not_count_a_driver_attempt() {
    let fixture = Fixture::new(Behavior::Complete);
    let plan = fixture
        .studio
        .prepare_execution(&fixture.session.info.id, SCRIPT)
        .unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let id = fixture
        .studio
        .execute(
            &plan.id,
            None,
            Arc::new(move |event| {
                let accepted = !matches!(event, QueryEvent::StatementStarted { .. });
                tx.send(event).unwrap();
                accepted
            }),
        )
        .unwrap();
    let evidence = fixture.finish(&id, rx).await;
    assert_eq!(evidence.indices("statementStarted"), vec![json!(0)]);
    assert!(evidence.calls.is_empty());
    evidence.assert_status("cancelled");
    evidence.assert_summary(0, 0, json!("cancelled"));
}

#[tokio::test]
async fn script_history_observed_completion_is_durable_before_secondary_observation() {
    let fixture = Fixture::new(Behavior::HeldObservation);
    let (id, rx) = fixture.start_sql("SELECT 'first'", None);
    tokio::time::timeout(Duration::from_secs(3), fixture.driver.entered.notified())
        .await
        .unwrap();
    let entry = fixture.studio.history_entry(&id).unwrap().unwrap();
    assert_eq!(entry.status, Some(RunStatus::Completed));
    assert_eq!(entry.provenance, HistoryProvenance::Recorded);
    assert!(entry.elapsed_ms.is_some());
    assert!(fixture.studio.jobs.lock().contains_key(&id));
    fixture.driver.cancelled.cancel();
    let evidence = fixture.finish(&id, rx).await;
    evidence.assert_status("completed");
    let terminal_events: Vec<_> = evidence.events[evidence.events.len() - 3..]
        .iter()
        .map(|event| event["type"].as_str().unwrap())
        .collect();
    assert_eq!(terminal_events, ["sessionState", "runMetrics", "finished"]);
}

#[tokio::test]
async fn script_history_panic_preserves_unknown_timing_without_leaking_job_ownership() {
    let fixture = Fixture::new(Behavior::PanicBeforeAck);
    let (id, rx) = fixture.start_sql("SELECT 'first'", None);
    let evidence = fixture.finish(&id, rx).await;
    assert_eq!(evidence.history["provenance"], "interrupted");
    assert_eq!(evidence.history["status"], Value::Null);
    assert_eq!(evidence.history["elapsedMs"], Value::Null);
    assert!(!fixture.studio.jobs.lock().contains_key(&id));
    assert_eq!(fixture.driver.calls.lock().len(), 1);
    fixture.studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn script_history_callback_panic_after_completion_preserves_original_known_status() {
    let fixture = Fixture::new(Behavior::Complete);
    let plan = fixture
        .studio
        .prepare_execution(&fixture.session.info.id, "SELECT 'first'")
        .unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let id = fixture
        .studio
        .execute(
            &plan.id,
            None,
            Arc::new(move |event| {
                assert!(
                    !matches!(event, QueryEvent::RunMetrics { .. }),
                    "synthetic publication panic after completion"
                );
                tx.send(event).is_ok()
            }),
        )
        .unwrap();
    let evidence = fixture.finish(&id, rx).await;
    evidence.assert_status("completed");
    assert!(evidence.history["elapsedMs"].is_number());
    assert_eq!(evidence.history["provenance"], "recorded");
    assert!(!fixture.studio.jobs.lock().contains_key(&id));
    assert!(fixture.studio.pending_history.lock().is_empty());
    fixture.studio.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn script_history_statement_callback_panic_preserves_ack_and_stops_only_the_suffix() {
    for sql in ["SELECT 'first'", SCRIPT] {
        let fixture = Fixture::new(Behavior::Complete);
        let plan = fixture
            .studio
            .prepare_execution(&fixture.session.info.id, sql)
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let id = fixture
            .studio
            .execute(
                &plan.id,
                None,
                Arc::new(move |event| {
                    assert!(
                        !matches!(event, QueryEvent::StatementCompleted { .. }),
                        "synthetic callback panic after ACK"
                    );
                    tx.send(event).is_ok()
                }),
            )
            .unwrap();
        let evidence = fixture.finish(&id, rx).await;
        assert_eq!(evidence.calls.len(), 1);
        assert_eq!(evidence.history["executionSummary"]["completed"], 1);
        assert_eq!(evidence.history["executionSummary"]["attempted"], 1);
        assert!(evidence.history["elapsedMs"].is_number());
        evidence.assert_status(if sql == SCRIPT {
            "cancelled"
        } else {
            "completed"
        });
        fixture.studio.shutdown_and_wait().await.unwrap();
    }
}

#[tokio::test]
async fn script_history_shutdown_waits_for_the_running_owner_before_final_flush() {
    let fixture = Fixture::new(Behavior::HeldCancellation);
    let (id, rx) = fixture.start_sql("SELECT 'first'", None);
    tokio::time::timeout(Duration::from_secs(3), fixture.driver.entered.notified())
        .await
        .unwrap();
    let owner = fixture.studio.clone();
    let mut shutdown = tokio::spawn(async move { owner.shutdown_and_wait().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .studio
            .history_entry(&id)
            .unwrap()
            .unwrap()
            .provenance,
        HistoryProvenance::Admitted
    );
    fixture.driver.cancelled.cancel();
    shutdown.await.unwrap().unwrap();
    fixture.finish(&id, rx).await.assert_status("cancelled");
    assert!(fixture.studio.pending_history.lock().is_empty());
}
