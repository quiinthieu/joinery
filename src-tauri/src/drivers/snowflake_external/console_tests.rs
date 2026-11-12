use super::*;
use crate::drivers::snowflake_external::console::ReviewedConsoleAckRecorder;
use crate::drivers::snowflake_process::ReviewedAcknowledgement;
use std::sync::atomic::AtomicUsize;
#[derive(Default)]
struct TestRecorder {
    recorded: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    fail: bool,
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    release: Option<tokio::sync::oneshot::Receiver<()>>,
}
#[async_trait]
impl ReviewedConsoleAckRecorder for TestRecorder {
    async fn record_ack(&mut self, _: &ReviewedAcknowledgement) -> AppResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        if let Some(release) = self.release.take() {
            release.await.unwrap();
        }
        if self.fail {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Synthetic acknowledgement recording failure",
            ));
        }
        self.recorded.store(true, Ordering::SeqCst);
        Ok(())
    }
}

use crate::safety::snowflake_script::{self, CompiledScript, Mode};

// Reuse the real child/framing harness, with an explicitly synthetic SDK branch
// for reviewed units. No SDK import, socket, credentials or owner files are used.
async fn console_harness(safe_mode: SafeMode, enabled: bool) -> Harness {
    let fixture = FIXTURE
        .replace("if (ddl) {\n        ddlSeen", r#"if (options.parameters?.MULTI_STATEMENT_COUNT === 1) {
        appendFileSync(file('reviewed-specs'), JSON.stringify({sql,binds:options.binds,parameters:options.parameters}) + '\n');
        names = ['first','second','third']; rows = [['1','2','3'],['4','5','6']];
      } else if (ddl) {
        ddlSeen"#)
        .replace("getColumns: () => names.map", r#"getSessionState: () => ({
          getCurrentDatabase: () => value('responseDatabase','OTHER_DB'),
          getCurrentSchema: () => value('responseSchema','Other.Schema'),
          getCurrentRole: () => value('responseRole',null),
          getCurrentWarehouse: () => value('responseWarehouse','OTHER_WH'),
        }),
        getColumns: () => names.map"#)
        .replace("streamRows() {\n          if (checks", r#"streamRows() {
          if (options.parameters?.MULTI_STATEMENT_COUNT === 1 && settings.failReviewedDrain) {
            return Readable.from((async function*() { throw {sqlState:'58000'}; })());
          }
          if (options.parameters?.MULTI_STATEMENT_COUNT === 1 && settings.holdReviewedDrain) {
            writeFileSync(file('held-reviewed-drain'), '');
            return Readable.from((async function*() {
              while (!existsSync(file('release')) && !cancelled) await new Promise(resolve => setTimeout(resolve,5));
              if (cancelled) throw {sqlState:'57014'};
              yield* rows;
            })());
          }
          if (checks"#)
        .replace("const denied = checks", "const denied = options.parameters?.MULTI_STATEMENT_COUNT === 1 && settings.rejectReviewed || checks")
        .replace("{ clientConfigFile:'/nonexistent/synthetic'", "{ isFileTransfer: () => false, clientConfigFile:'/nonexistent/synthetic'");
    assert!(fixture.contains("reviewed-specs") && fixture.contains("isFileTransfer"));
    Harness::open_fixture(safe_mode, true, true, enabled, None, None, &fixture).await
}

fn script(sql: &str) -> CompiledScript {
    snowflake_script::compile(sql, Mode::ParsedStatements, None).unwrap()
}
fn selected(sql: &str) -> CompiledScript {
    snowflake_script::compile(sql, Mode::SelectedUnit, None).unwrap()
}
#[derive(Default)]
struct ConsoleSink {
    columns: Vec<ColumnInfo>,
    rows: Vec<TextRow>,
    limit: bool,
    fail: bool,
    retain: bool,
    recorded: Option<Arc<AtomicBool>>,
    events: Arc<AtomicUsize>,
}
impl ConsoleSink {
    fn check_recorded(&self) {
        self.events.fetch_add(1, Ordering::SeqCst);
        if let Some(recorded) = &self.recorded {
            assert!(
                recorded.load(Ordering::SeqCst),
                "sink publication preceded acknowledgement recording"
            );
        }
    }
}
impl QuerySink for ConsoleSink {
    fn describe(&mut self, columns: Vec<ColumnInfo>) -> AppResult<()> {
        self.check_recorded();
        self.columns = columns;
        Ok(())
    }
    fn rows(&mut self, rows: Vec<TextRow>) -> AppResult<bool> {
        self.check_recorded();
        if self.fail {
            return Err(AppError::new(
                ErrorCode::Database,
                "Synthetic local sink failure",
            ));
        }
        self.rows.extend(rows);
        Ok(!self.retain)
    }
    fn limit(&mut self, _: &str) {
        self.check_recorded();
        self.limit = true;
    }
    fn complete(&mut self, _: u64) -> AppResult<()> {
        self.check_recorded();
        Ok(())
    }
}
async fn finish_console(h: Harness) {
    h.driver.close();
    let cleanup = h.cleanup.wait(Duration::from_secs(10)).await.unwrap();
    assert!(cleanup.reaped && cleanup.temporary_root_removed);
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("connections")).unwrap(),
        "1"
    );
}

#[tokio::test]
async fn console_requires_separate_writable_opt_in_and_rejects_restricted_or_retired() {
    for (enabled, downgrade) in [(false, false), (true, true)] {
        let mut h = console_harness(SafeMode::Confirm, enabled).await;
        if downgrade {
            // Profile validation rejects this persisted combination even before
            // opening; also prove the private adapter boundary defends itself.
            Arc::get_mut(&mut h.driver).unwrap().profile.safe_mode = SafeMode::ReadOnly;
        }
        let before = h.queries();
        assert_eq!(
            h.driver.claim_reviewed_console().await.err().unwrap().code,
            ErrorCode::PolicyBlocked
        );
        assert_eq!(h.queries(), before);
        finish_console(h).await;
    }
    let h = console_harness(SafeMode::Confirm, true).await;
    h.driver.transaction_authority.lock().recovery = Some(TransactionRecovery {
        revision: new_id(),
        restricted: true,
    });
    let before = h.queries();
    assert_eq!(
        h.driver.claim_reviewed_console().await.err().unwrap().code,
        ErrorCode::PolicyBlocked
    );
    assert!(
        h.driver
            .transaction_authority
            .lock()
            .recovery
            .as_ref()
            .unwrap()
            .restricted
    );
    assert_eq!(h.queries(), before);
    h.driver.close();
    assert_eq!(
        h.driver.claim_reviewed_console().await.err().unwrap().code,
        ErrorCode::ConnectionClosed
    );
    finish_console(h).await;
}

#[tokio::test]
async fn console_retains_one_session_across_show_result_scan_and_transaction_runs_without_probes() {
    let h = console_harness(SafeMode::Confirm, true).await;
    let before = h.queries().len();
    let mut console = h.driver.claim_reviewed_console().await.unwrap();
    assert!(console.passive_context().is_none());
    for compiled in [
        selected("SHOW TABLES"),
        selected("SELECT * FROM TABLE(RESULT_SCAN(LAST_QUERY_ID()))"),
        script("BEGIN; INSERT INTO DB.PUBLIC.T VALUES (1); ROLLBACK;"),
    ] {
        let mut run = console
            .begin_run(&compiled, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(run.planned_units(), compiled.units().len());
        for index in 0..compiled.units().len() {
            assert_eq!(run.next_index(), index);
            let mut sink = ConsoleSink::default();
            let outcome = run
                .next(&mut sink, &mut TestRecorder::default())
                .await
                .unwrap()
                .unwrap();
            assert!(outcome.attempted && outcome.report.as_ref().unwrap().ok);
            assert!(outcome.report.unwrap().statement_acknowledged.is_some());
            assert_eq!(sink.columns.len(), 3);
            assert_eq!(sink.rows.len(), 2);
            assert_eq!(outcome.retained_rows, 2);
        }
        assert!(run.is_terminal() && !run.is_stopped());
        assert!(
            run.next(&mut ConsoleSink::default(), &mut TestRecorder::default())
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        &h.queries()[before..],
        &[
            "SHOW TABLES",
            "SELECT * FROM TABLE(RESULT_SCAN(LAST_QUERY_ID()))",
            "BEGIN",
            "INSERT INTO DB.PUBLIC.T VALUES (1)",
            "ROLLBACK"
        ]
    );
    let context = console.passive_context().unwrap();
    assert_eq!(context.database.as_deref(), Some("OTHER_DB"));
    assert_eq!(context.schema.as_deref(), Some("Other.Schema"));
    assert_eq!(context.role, None);
    assert_eq!(context.warehouse.as_deref(), Some("OTHER_WH"));
    let specs: Vec<Value> = std::fs::read_to_string(h.scripts.path().join("reviewed-specs"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(specs.len(), 5);
    assert!(
        specs
            .iter()
            .all(|s| s["parameters"] == json!({"MULTI_STATEMENT_COUNT":1}))
    );
    assert!(!h.driver.is_closed());
    drop(console);
    assert!(h.driver.is_closed());
    finish_console(h).await;
}

#[tokio::test]
async fn console_seals_idle_legacy_routes_and_consumes_preexisting_generated_grants() {
    let h = console_harness(SafeMode::Confirm, true).await;
    let catalog = h.driver.relation_catalog().await.unwrap();
    let identity = h.driver.completion_identity().await.unwrap();
    let ddl = h.prepare_create_ddl().await;
    let (revision, sample) = {
        let state = h.driver.transaction_authority.lock();
        let grant = state.grant.as_ref().unwrap();
        (grant.revision.clone(), grant.sample.clone())
    };
    let transaction = TransactionControlRequest {
        action: ExplicitTransactionAction::Rollback,
        revision: revision.clone(),
    };
    let dml = SnowflakeDmlRequest {
        sql: "INSERT INTO DB.PUBLIC.T (V) VALUES (1)".into(),
        parameters: vec![],
        revision,
    };
    assert!(h.driver.validate_transaction_control(&transaction).is_ok());
    assert!(h.driver.validate_snowflake_dml(&dml).is_ok());
    assert!(h.driver.validate_snowflake_ddl(&ddl).is_ok());
    assert!(h.driver.browsing.lock().snapshot.is_some());
    *h.driver.session_options.lock() = SessionOptionsState {
        revision: new_id(),
        snapshot: Some(SessionOptionsSnapshot {
            sample,
            available: vec!["PUBLIC".into()],
        }),
    };
    let before = h.queries();
    let _console = h.driver.claim_reviewed_console().await.unwrap();
    assert!(h.driver.transaction_authority.lock().grant.is_none());
    assert!(h.driver.ddl_authority.lock().grant.is_none());
    assert!(h.driver.browsing.lock().snapshot.is_none());
    assert!(h.driver.session_options.lock().snapshot.is_none());
    assert!(h.driver.health().await.is_err());
    assert!(
        h.driver
            .execute("SELECT 1", StatementKind::Read, &mut ConsoleSink::default())
            .await
            .is_err()
    );
    assert!(h.driver.session_state("observer-session").await.is_err());
    assert!(
        h.driver
            .session_context_options("observer-session")
            .await
            .is_err()
    );
    assert!(h.driver.relation_catalog().await.is_err());
    assert!(h.driver.completion_identity().await.is_err());
    assert!(h.driver.completion_catalog(&identity).await.is_err());
    assert!(
        h.driver
            .completion_schema(&TableRef {
                schema: "PUBLIC".into(),
                name: "ORDERS".into()
            })
            .await
            .is_err()
    );
    let request = InspectRelationRequest {
        revision: catalog.revision.clone(),
        target_id: catalog.relations[0].target_id.clone(),
    };
    assert!(h.driver.inspect_relation(&request).await.is_err());
    assert!(h.driver.validate_snowflake_ddl(&ddl).is_err());
    assert!(
        h.driver
            .snowflake_ddl("observer-session", &ddl)
            .await
            .is_err()
    );
    assert!(h.driver.validate_snowflake_dml(&dml).is_err());
    assert!(
        h.driver
            .snowflake_dml("observer-session", &dml)
            .await
            .is_err()
    );
    assert!(h.driver.validate_transaction_control(&transaction).is_err());
    assert!(
        h.driver
            .transaction_control("observer-session", &transaction)
            .await
            .is_err()
    );
    assert!(h.driver.claim_reviewed_console().await.is_err());
    assert_eq!(h.queries(), before);
    finish_console(h).await;
}

#[tokio::test]
async fn console_pre_dispatch_cancel_stops_suffix_but_allows_new_explicit_review() {
    let h = console_harness(SafeMode::Confirm, true).await;
    let mut console = h.driver.claim_reviewed_console().await.unwrap();
    let compiled = script("SELECT 1; COMMIT;");
    let token = CancellationToken::new();
    token.cancel();
    let before = h.queries();
    {
        let mut run = console.begin_run(&compiled, token).await.unwrap();
        let outcome = run
            .next(&mut ConsoleSink::default(), &mut TestRecorder::default())
            .await
            .unwrap()
            .unwrap();
        assert!(!outcome.attempted && outcome.report.is_none() && outcome.cancellation_observed);
        assert_eq!(outcome.delivery_error.unwrap().code, ErrorCode::Cancelled);
        assert_eq!(run.next_index(), 0);
        assert!(run.is_terminal() && run.is_stopped());
        assert!(
            run.next(&mut ConsoleSink::default(), &mut TestRecorder::default())
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(h.queries(), before);
    let recovery = selected("ROLLBACK");
    let mut run = console
        .begin_run(&recovery, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        run.next(&mut ConsoleSink::default(), &mut TestRecorder::default())
            .await
            .unwrap()
            .unwrap()
            .report
            .unwrap()
            .ok
    );
    drop(run);
    drop(console);
    finish_console(h).await;
}

#[tokio::test]
async fn console_sink_failure_and_result_limit_preserve_ack_and_stop_commit_suffix() {
    for fail in [true, false] {
        let h = console_harness(SafeMode::Confirm, true).await;
        let mut console = h.driver.claim_reviewed_console().await.unwrap();
        let before = h.queries().len();
        let compiled = script("INSERT INTO DB.PUBLIC.T VALUES (1); COMMIT;");
        let mut run = console
            .begin_run(&compiled, CancellationToken::new())
            .await
            .unwrap();
        let mut sink = ConsoleSink {
            fail,
            retain: !fail,
            ..Default::default()
        };
        let outcome = run
            .next(&mut sink, &mut TestRecorder::default())
            .await
            .unwrap()
            .unwrap();
        assert!(
            outcome
                .report
                .as_ref()
                .unwrap()
                .statement_acknowledged
                .is_some()
        );
        assert!(outcome.report.unwrap().ok);
        assert_eq!(outcome.delivery_error.is_some(), fail);
        assert_eq!(outcome.truncated, !fail);
        assert!(run.is_stopped());
        assert!(
            run.next(&mut sink, &mut TestRecorder::default())
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            &h.queries()[before..],
            &["INSERT INTO DB.PUBLIC.T VALUES (1)"]
        );
        drop(run);
        assert!(!h.driver.is_closed());
        drop(console);
        finish_console(h).await;
    }
}

#[tokio::test]
async fn console_post_ack_drain_failure_retains_original_ack_and_retires_without_suffix() {
    let h = console_harness(SafeMode::Confirm, true).await;
    let mut console = h.driver.claim_reviewed_console().await.unwrap();
    h.settings(json!({"failReviewedDrain":true}));
    let before = h.queries().len();
    let compiled = script("INSERT INTO DB.PUBLIC.T VALUES (1); COMMIT;");
    let mut run = console
        .begin_run(&compiled, CancellationToken::new())
        .await
        .unwrap();
    let outcome = run
        .next(&mut ConsoleSink::default(), &mut TestRecorder::default())
        .await
        .unwrap()
        .unwrap();
    let report = outcome.report.unwrap();
    assert!(!report.ok && report.statement_acknowledged.is_some());
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    assert!(run.is_stopped());
    assert!(
        run.next(&mut ConsoleSink::default(), &mut TestRecorder::default())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        &h.queries()[before..],
        &["INSERT INTO DB.PUBLIC.T VALUES (1)"]
    );
    drop(run);
    assert!(h.driver.is_closed());
    drop(console);
    finish_console(h).await;
}

#[tokio::test]
async fn console_cancel_after_ack_keeps_original_evidence_and_never_dispatches_suffix() {
    let h = console_harness(SafeMode::Confirm, true).await;
    let mut console = h.driver.claim_reviewed_console().await.unwrap();
    h.settings(json!({"holdReviewedDrain":true}));
    let before = h.queries().len();
    let compiled = script("INSERT INTO DB.PUBLIC.T VALUES (1); COMMIT;");
    let cancel = CancellationToken::new();
    let mut run = console.begin_run(&compiled, cancel.clone()).await.unwrap();
    let mut recorder = TestRecorder::default();
    let mut sink = ConsoleSink {
        recorded: Some(recorder.recorded.clone()),
        ..Default::default()
    };
    let (outcome, ()) = tokio::join!(run.next(&mut sink, &mut recorder), async {
        h.wait_until(|| h.scripts.path().join("held-reviewed-drain").exists())
            .await;
        cancel.cancel();
    });
    let outcome = outcome.unwrap().unwrap();
    assert!(outcome.cancellation_observed && outcome.attempted);
    assert!(outcome.acknowledgement_recorded);
    assert_eq!(recorder.calls.load(Ordering::SeqCst), 1);
    let report = outcome.report.unwrap();
    assert!(report.statement_acknowledged.is_some());
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    assert!(run.next(&mut sink, &mut recorder).await.unwrap().is_none());
    assert_eq!(
        &h.queries()[before..],
        &["INSERT INTO DB.PUBLIC.T VALUES (1)"]
    );
    drop(run);
    drop(console);
    finish_console(h).await;
}

#[tokio::test]
async fn console_abandoned_inflight_or_incomplete_run_closes_and_cannot_resume_suffix() {
    for inflight in [false, true] {
        let h = console_harness(SafeMode::Confirm, true).await;
        let mut console = h.driver.claim_reviewed_console().await.unwrap();
        let compiled = script("INSERT INTO DB.PUBLIC.T VALUES (1); COMMIT;");
        let before = h.queries().len();
        let mut run = console
            .begin_run(&compiled, CancellationToken::new())
            .await
            .unwrap();
        if inflight {
            h.settings(json!({"holdReviewedDrain":true}));
            let mut sink = ConsoleSink::default();
            let mut recorder = TestRecorder::default();
            let future = run.next(&mut sink, &mut recorder);
            tokio::pin!(future);
            tokio::select! {
                result = &mut future => panic!("held drain unexpectedly finished: {}",result.is_ok()),
                _ = h.wait_until(||h.scripts.path().join("held-reviewed-drain").exists()) => {},
            }
        }
        drop(run);
        assert!(h.driver.is_closed());
        assert!(
            console
                .begin_run(&compiled, CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(h.queries().len() - before, usize::from(inflight));
        drop(console);
        finish_console(h).await;
    }
}

#[tokio::test]
async fn console_revoked_nonce_and_local_stop_submit_no_unit_or_automatic_settlement() {
    let h = console_harness(SafeMode::Confirm, true).await;
    let mut console = h.driver.claim_reviewed_console().await.unwrap();
    let compiled = script("BEGIN; SELECT 1; COMMIT;");
    let before = h.queries();
    {
        let mut run = console
            .begin_run(&compiled, CancellationToken::new())
            .await
            .unwrap();
        run.stop();
        assert!(run.is_stopped() && run.is_terminal());
        assert!(
            run.next(&mut ConsoleSink::default(), &mut TestRecorder::default())
                .await
                .unwrap()
                .is_none()
        );
    }
    *h.driver.console_mode.lock() = Some(new_id());
    assert!(
        console
            .begin_run(&compiled, CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(h.queries(), before);
    drop(console);
    finish_console(h).await;
}

#[tokio::test]
async fn console_claim_blocks_an_ordinary_health_request_already_queued_on_the_lane() {
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };
    let h = console_harness(SafeMode::Confirm, true).await;
    let before = h.queries();
    {
        let lane = h.driver.lane.lock().await;
        let claim = h.driver.claim_reviewed_console();
        tokio::pin!(claim);
        poll_fn(|cx| {
            assert!(claim.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let health = h.driver.health();
        tokio::pin!(health);
        poll_fn(|cx| {
            assert!(health.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(lane);
        let console = claim.await.unwrap();
        assert_eq!(health.await.unwrap_err().code, ErrorCode::PolicyBlocked);
        assert_eq!(h.queries(), before);
        drop(console);
    }
    finish_console(h).await;
}

#[tokio::test]
async fn console_preserves_canonical_scalar_binds_and_definitive_rejection_has_no_ack_or_suffix() {
    for rejected in [false, true] {
        let h = console_harness(SafeMode::Confirm, true).await;
        let mut console = h.driver.claim_reviewed_console().await.unwrap();
        let compiled = snowflake_script::compile(
            "SELECT :amount, :flag, :missing; COMMIT;",
            Mode::ParsedStatements,
            Some(&[
                QueryParamInput {
                    key: "amount".into(),
                    kind: QueryParamKind::Integer,
                    value: Some("9007199254740993".into()),
                },
                QueryParamInput {
                    key: "flag".into(),
                    kind: QueryParamKind::Boolean,
                    value: Some("true".into()),
                },
                QueryParamInput {
                    key: "missing".into(),
                    kind: QueryParamKind::Text,
                    value: None,
                },
            ]),
        )
        .unwrap();
        h.settings(json!({"rejectReviewed":rejected}));
        let before = h.queries().len();
        let mut run = console
            .begin_run(&compiled, CancellationToken::new())
            .await
            .unwrap();
        let outcome = run
            .next(&mut ConsoleSink::default(), &mut TestRecorder::default())
            .await
            .unwrap()
            .unwrap();
        let report = outcome.report.unwrap();
        assert_eq!(report.ok, !rejected);
        assert_eq!(report.statement_acknowledged.is_some(), !rejected);
        assert_ne!(report.certainty, Certainty::OutcomeUnknown);
        if rejected {
            assert!(run.is_stopped());
            assert!(
                run.next(&mut ConsoleSink::default(), &mut TestRecorder::default())
                    .await
                    .unwrap()
                    .is_none()
            );
        } else {
            run.stop();
        }
        assert_eq!(
            &h.queries()[before..],
            &[compiled.units()[0].bound().rewritten_sql.clone()]
        );
        let specs = std::fs::read_to_string(h.scripts.path().join("reviewed-specs")).unwrap();
        let first: Value = serde_json::from_str(specs.lines().next().unwrap()).unwrap();
        assert_eq!(first["binds"], json!(["9007199254740993", true, null]));
        drop(run);
        drop(console);
        finish_console(h).await;
    }
}

#[tokio::test]
async fn console_awaits_exactly_one_async_ack_record_before_any_sink_publication() {
    let h = console_harness(SafeMode::Confirm, true).await;
    let mut console = h.driver.claim_reviewed_console().await.unwrap();
    let compiled = script("SELECT 1; COMMIT;");
    let before = h.queries().len();
    let mut run = console
        .begin_run(&compiled, CancellationToken::new())
        .await
        .unwrap();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let mut recorder = TestRecorder {
        entered: Some(entered_tx),
        release: Some(release_rx),
        ..Default::default()
    };
    let recorded = recorder.recorded.clone();
    let calls = recorder.calls.clone();
    let mut sink = ConsoleSink {
        recorded: Some(recorded.clone()),
        ..Default::default()
    };
    let events = sink.events.clone();
    let outcome = {
        let future = run.next(&mut sink, &mut recorder);
        tokio::pin!(future);
        tokio::select! {
            result = &mut future => panic!("held recorder unexpectedly finished: {}",result.is_ok()),
            entered = tokio::time::timeout(Duration::from_secs(5),entered_rx) => { entered.unwrap().unwrap(); },
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!recorded.load(Ordering::SeqCst));
        assert_eq!(events.load(Ordering::SeqCst), 0);
        assert_eq!(&h.queries()[before..], &["SELECT 1"]);
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), &mut future)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    };
    assert!(outcome.acknowledgement_recorded && outcome.recording_error.is_none());
    assert!(outcome.report.unwrap().statement_acknowledged.is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(events.load(Ordering::SeqCst) >= 3); // describe, rows, complete
    run.stop();
    assert!(run.next(&mut sink, &mut recorder).await.unwrap().is_none());
    assert_eq!(&h.queries()[before..], &["SELECT 1"]);
    drop(run);
    drop(console);
    finish_console(h).await;
}

#[tokio::test]
async fn console_ack_recording_failure_preserves_original_ack_without_publication_or_suffix() {
    let h = console_harness(SafeMode::Confirm, true).await;
    let mut console = h.driver.claim_reviewed_console().await.unwrap();
    let compiled = script("INSERT INTO DB.PUBLIC.T VALUES (1); COMMIT;");
    let before = h.queries().len();
    let mut run = console
        .begin_run(&compiled, CancellationToken::new())
        .await
        .unwrap();
    let mut recorder = TestRecorder {
        fail: true,
        ..Default::default()
    };
    let mut sink = ConsoleSink {
        recorded: Some(recorder.recorded.clone()),
        ..Default::default()
    };
    let outcome = run.next(&mut sink, &mut recorder).await.unwrap().unwrap();
    assert!(outcome.attempted && !outcome.acknowledgement_recorded);
    assert_eq!(outcome.recording_error.unwrap().code, ErrorCode::Conflict);
    assert!(outcome.delivery_error.is_none());
    let report = outcome.report.unwrap();
    assert!(report.ok && report.statement_acknowledged.is_some());
    assert_eq!(report.certainty, Certainty::StatementSucceeded);
    assert_eq!(recorder.calls.load(Ordering::SeqCst), 1);
    assert_eq!(sink.events.load(Ordering::SeqCst), 0);
    assert!(run.is_stopped());
    assert!(run.next(&mut sink, &mut recorder).await.unwrap().is_none());
    assert_eq!(
        &h.queries()[before..],
        &["INSERT INTO DB.PUBLIC.T VALUES (1)"]
    );
    drop(run);
    drop(console);
    finish_console(h).await;
}
