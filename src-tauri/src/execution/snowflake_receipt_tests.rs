// Test-owned fault injection happens after durable admission. Completion updates
// the retained history row rather than inserting a second SQL-bearing payload.
fn reject_receipt_write(database: &rusqlite::Connection, table: &str) {
    let operation = match table {
        "history" => "UPDATE",
        "audit" => "INSERT",
        _ => panic!("unknown fixture table"),
    };
    database.execute_batch(&format!("CREATE TRIGGER reject_{table} BEFORE {operation} ON {table}_v7 BEGIN SELECT RAISE(ABORT, 'synthetic local failure'); END;")).unwrap();
}

// Included beside the Studio policy fixtures; no account or helper connection.
fn receipt_database(root: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap()
}

fn compact_receipt(cell: &outcome_recording::OutcomeCell) -> serde_json::Value {
    let bytes = cell.original_bytes();
    assert!(!bytes.is_empty());
    assert!(bytes.len() <= outcome_memory::PAYLOAD_BYTES);
    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    fn assert_whitelist(value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                for (key, child) in fields {
                    assert!(
                        !["sql", "error", "state", "rows", "password", "parameters"]
                            .contains(&key.as_str()),
                        "retained forbidden field: {key}"
                    );
                    assert_whitelist(child);
                }
            }
            serde_json::Value::Array(values) => values.iter().for_each(assert_whitelist),
            _ => {}
        }
    }
    assert_whitelist(&payload);
    assert!(!String::from_utf8(bytes).unwrap().contains(SDK_DML_SQL));
    payload
}

#[tokio::test]
async fn receipt_typed_history_privacy_survives_late_ack_and_retry_without_sql_reinsertion() {
    for ddl in [false, true] {
        for action in [
            "clear",
            "optout-reenable",
            "delete-recreate",
            "failed-completion-clear",
        ] {
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let (root, studio, driver, session, request) = if ddl {
                let (root, studio, driver, session, request) = sdk_ddl_fixture(
                    SafeMode::Silent,
                    false,
                    Some((entered.clone(), release.clone())),
                );
                (root, studio, driver, session, Some(request))
            } else {
                let (root, studio, driver, session) =
                    sdk_dml_fixture(SafeMode::Silent, Some((entered.clone(), release.clone())));
                (root, studio, driver, session, None)
            };
            let mut profile = session.profile.clone();
            profile.store_query_text = true;
            studio.persistence.save_profile(&profile).unwrap();
            let (plan, approval) = if let Some(request) = &request {
                let plan = ddl_review(&studio, request).await;
                let approval = ddl_approve(&studio, &plan);
                (plan, Some(approval))
            } else {
                (
                    studio
                        .prepare_snowflake_dml(&session.info.id, SDK_DML_SQL, None)
                        .unwrap(),
                    None,
                )
            };
            let owner = studio.clone();
            let waiter = tokio::spawn(async move {
                if ddl {
                    serde_json::to_value(
                        owner
                            .execute_snowflake_ddl(&plan.id, approval.as_deref())
                            .await
                            .unwrap(),
                    )
                    .unwrap()
                } else {
                    serde_json::to_value(owner.execute_snowflake_dml(&plan.id, None).await.unwrap())
                        .unwrap()
                }
            });
            tokio::time::timeout(Duration::from_secs(3), entered.notified())
                .await
                .unwrap();
            let pending = studio.history("").unwrap();
            assert_eq!(pending.len(), 1);
            let id = pending[0].id.clone();
            assert_eq!(pending[0].provenance, HistoryProvenance::Admitted);
            assert!(pending[0].sql.is_some());
            let database = receipt_database(&root);
            match action {
                "clear" => studio.clear_history().unwrap(),
                "optout-reenable" => {
                    profile.store_query_text = false;
                    studio.save_profile(profile.clone(), None).unwrap();
                    assert!(studio.history_entry(&id).unwrap().unwrap().sql.is_none());
                    profile.store_query_text = true;
                    studio.save_profile(profile, None).unwrap();
                }
                "delete-recreate" => {
                    studio.delete_profile(&profile.id).unwrap();
                    studio.save_profile(profile, None).unwrap();
                }
                "failed-completion-clear" => reject_receipt_write(&database, "history"),
                _ => unreachable!(),
            }
            release.notify_one();
            let result = waiter.await.unwrap();
            assert_eq!(result["statementOutcome"], "succeeded");
            if action == "failed-completion-clear" {
                assert!(result["error"].is_object());
                assert_eq!(studio.pending_snowflake_receipts.lock().len(), 1);
                studio.clear_history().unwrap();
                database
                    .execute_batch("DROP TRIGGER reject_history")
                    .unwrap();
            }
            studio.shutdown_and_wait().await.unwrap();
            let row = studio.history_entry(&id).unwrap();
            if action.contains("clear") {
                assert!(row.is_none());
            } else {
                assert!(row.unwrap().sql.is_none());
            }
            assert_eq!(driver.dml_calls.lock().len(), usize::from(!ddl));
            assert_eq!(driver.ddl_calls.lock().len(), usize::from(ddl));
            assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
            drop(studio);
            let reopened = Persistence::open(root.path()).unwrap();
            assert!(
                reopened
                    .history_entry(&id)
                    .unwrap()
                    .is_none_or(|entry| entry.sql.is_none())
            );
        }
    }
}

fn terminal_audits(database: &rusqlite::Connection, action: &str, outcome: &str) -> u32 {
    database
        .query_row(
            "SELECT COUNT(*) FROM audit_v7 WHERE action=?1 AND outcome=?2",
            [action, outcome],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn receipt_dml_original_rejection_unknown_and_observer_error_survive_local_failure() {
    for outcome in [
        SnowflakeDmlOutcome::Rejected,
        SnowflakeDmlOutcome::Unknown,
        SnowflakeDmlOutcome::Succeeded,
    ] {
        let (root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
        let private_error = "unbounded-private-observer-error".repeat(4096);
        let primary = AppError::new(
            ErrorCode::Database,
            format!("original remote or observer failure: {private_error}"),
        )
        .with_completion_invalidated();
        driver.dml_result.lock().as_mut().unwrap().statement_outcome = outcome;
        driver.dml_result.lock().as_mut().unwrap().error = Some(primary.clone());
        let plan = studio
            .prepare_execution(&session.info.id, SDK_DML_SQL)
            .unwrap();
        let database = receipt_database(&root);
        database.execute_batch("CREATE TRIGGER reject_terminal BEFORE INSERT ON audit_v7 WHEN NEW.outcome != 'started' BEGIN SELECT RAISE(ABORT, 'local failure'); END; CREATE TRIGGER reject_history BEFORE UPDATE ON history_v7 BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
        let result = studio.execute_snowflake_dml(&plan.id, None).await.unwrap();
        assert_eq!(result.statement_outcome, outcome);
        let returned = result.error.as_ref().unwrap();
        assert_eq!(returned.code, primary.code);
        assert_eq!(
            returned.completion_invalidated,
            primary.completion_invalidated
        );
        assert!(returned.message.starts_with(&primary.message));
        assert!(returned.message.contains("local history or audit"));
        let (original_id, original_evidence) = {
            let receipts = studio.pending_snowflake_receipts.lock();
            assert_eq!(receipts.len(), 1);
            let receipt = receipts.values().next().unwrap();
            assert!(receipt.is_captured());
            let payload = compact_receipt(receipt);
            assert_eq!(
                payload["completion"]["snowflakeDml"]["statementOutcome"],
                serde_json::to_value(outcome).unwrap()
            );
            assert_eq!(
                payload["completion"]["snowflakeDml"]["summary"],
                serde_json::to_value(&result.summary).unwrap()
            );
            assert!(
                !serde_json::to_string(&payload)
                    .unwrap()
                    .contains("unbounded-private-observer-error")
            );
            (
                receipt.ticket().id().to_owned(),
                payload["completion"]["snowflakeDml"].clone(),
            )
        };
        assert_eq!(
            studio.shutdown_and_wait().await.unwrap_err().code,
            ErrorCode::Conflict
        );
        database
            .execute_batch("DROP TRIGGER reject_terminal; DROP TRIGGER reject_history;")
            .unwrap();
        studio.shutdown_and_wait().await.unwrap();
        studio.shutdown_and_wait().await.unwrap();
        let history = studio.history("").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, original_id);
        assert_eq!(
            serde_json::to_value(&history[0].snowflake_dml).unwrap(),
            original_evidence
        );
        let label = match outcome {
            SnowflakeDmlOutcome::Succeeded => "succeeded",
            SnowflakeDmlOutcome::Rejected => "rejected",
            SnowflakeDmlOutcome::Unknown => "unknown",
        };
        assert_eq!(terminal_audits(&database, "snowflake-dml", label), 1);
        assert_eq!(driver.dml_calls.lock().len(), 1);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn receipt_dml_owner_release_failure_keeps_hold_without_redispatch() {
    let (root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
    let plan = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    let database = receipt_database(&root);
    database.execute_batch("CREATE TRIGGER reject_owner_release BEFORE DELETE ON history_owners_v6 BEGIN SELECT RAISE(ABORT, 'synthetic release failure'); END;").unwrap();

    let result = studio.execute_snowflake_dml(&plan.id, None).await.unwrap();
    assert!(
        result
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("local history or audit")
    );
    let history = studio.history("").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].provenance, HistoryProvenance::Admitted);
    assert_eq!(history[0].status, None);
    assert_eq!(history[0].elapsed_ms, None);
    assert_eq!(terminal_audits(&database, "snowflake-dml", "succeeded"), 0);
    {
        let receipts = studio.pending_snowflake_receipts.lock();
        assert_eq!(receipts.len(), 1);
        let cell = receipts.values().next().unwrap();
        assert!(cell.is_captured());
        assert_eq!(compact_receipt(cell)["completion"]["status"], "completed");
    }
    let owners = || -> u32 {
        database
            .query_row("SELECT COUNT(*) FROM history_owners_v6", [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(owners(), 1);
    assert_eq!(
        studio.shutdown_and_wait().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(driver.dml_calls.lock().len(), 1);

    database
        .execute_batch("DROP TRIGGER reject_owner_release")
        .unwrap();
    studio.shutdown_and_wait().await.unwrap();
    assert_eq!(owners(), 0);
    assert_eq!(
        studio.history("").unwrap()[0].provenance,
        HistoryProvenance::Recorded
    );
    assert_eq!(terminal_audits(&database, "snowflake-dml", "succeeded"), 1);
    assert_eq!(driver.dml_calls.lock().len(), 1);
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn receipt_boundary_dropped_waiter_preserves_each_original_outcome_until_local_retry() {
    for scenario in ["ack", "rejected", "unknown", "observer-failed"] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (root, studio, driver, session, request) = explicit_control_fixture(
            SafeMode::Silent,
            ExplicitTransactionAction::Begin,
            Some((entered.clone(), release.clone())),
        );
        let outcome = match scenario {
            "rejected" => TransactionBoundaryOutcome::Rejected,
            "unknown" => TransactionBoundaryOutcome::Unknown,
            _ => TransactionBoundaryOutcome::Acknowledged,
        };
        let primary = (scenario != "ack")
            .then(|| AppError::new(ErrorCode::Database, format!("original {scenario}")));
        {
            let mut fixture = driver.boundary_result.lock();
            let result = fixture.as_mut().unwrap();
            result.boundary_outcome = outcome;
            result.error = primary.clone();
        }
        let plan = studio
            .prepare_transaction_control(&session.info.id, &request)
            .unwrap();
        let owner = studio.clone();
        let waiter =
            tokio::spawn(async move { owner.execute_transaction_control(&plan.id, None).await });
        tokio::time::timeout(Duration::from_secs(3), entered.notified())
            .await
            .unwrap();
        let database = receipt_database(&root);
        database.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit_v7 BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        release.notify_one();
        assert_eq!(
            studio.shutdown_and_wait().await.unwrap_err().code,
            ErrorCode::Conflict
        );
        let receipt_id = {
            let receipts = studio.pending_snowflake_receipts.lock();
            assert_eq!(receipts.len(), 1);
            let (id, receipt) = receipts.iter().next().unwrap();
            assert!(receipt.is_captured());
            let payload = compact_receipt(receipt);
            assert_eq!(payload["completion"], serde_json::Value::Null);
            assert_eq!(
                payload["boundary"],
                serde_json::json!({
                    "action": request.action, "boundaryOutcome": outcome, "retired": true,
                })
            );
            if let Some(error) = &primary {
                assert!(
                    !serde_json::to_string(&payload)
                        .unwrap()
                        .contains(&error.message)
                );
            }
            assert_eq!(payload["descriptor"]["operationId"], *id);
            id.clone()
        };
        assert!(studio.shutdown_and_wait().await.is_err());
        assert!(
            studio
                .pending_snowflake_receipts
                .lock()
                .contains_key(&receipt_id)
        );
        database
            .execute_batch("DROP TRIGGER reject_audit;")
            .unwrap();
        studio.shutdown_and_wait().await.unwrap();
        studio.shutdown_and_wait().await.unwrap();
        assert!(studio.pending_snowflake_receipts.lock().is_empty());
        assert!(studio.history("").unwrap().is_empty());
        let label = match outcome {
            TransactionBoundaryOutcome::Acknowledged => "acknowledged",
            TransactionBoundaryOutcome::Rejected => "rejected",
            TransactionBoundaryOutcome::Unknown => "unknown",
        };
        assert_eq!(terminal_audits(&database, "transaction-boundary", label), 1);
        assert_eq!(
            terminal_audits(&database, "transaction-boundary", "started"),
            1
        );
        assert_eq!(driver.boundary_calls.lock().len(), 1);
        assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn receipt_pending_dml_blocks_mutations_but_allows_one_retained_recovery_rollback() {
    let (root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
    let database = receipt_database(&root);
    database.execute_batch("CREATE TRIGGER reject_terminal BEFORE INSERT ON audit_v7 WHEN NEW.outcome != 'started' BEGIN SELECT RAISE(ABORT, 'local failure'); END; CREATE TRIGGER reject_history BEFORE UPDATE ON history_v7 BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
    let plan = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    let result = studio.execute_snowflake_dml(&plan.id, None).await.unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert!(result.error.is_some());
    let next = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    assert_eq!(
        studio
            .execute_snowflake_dml(&next.id, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let revision = match driver
        .runtime_override
        .lock()
        .as_ref()
        .unwrap()
        .transaction_control_authority
        .as_ref()
        .unwrap()
    {
        TransactionControlAuthority::Observed { revision, .. } => revision.clone(),
        _ => panic!("expected observed authority"),
    };
    let begin = studio
        .prepare_transaction_control(
            &session.info.id,
            &TransactionControlRequest {
                action: ExplicitTransactionAction::Begin,
                revision: revision.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        studio
            .execute_transaction_control(&begin.id, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(driver.boundary_calls.lock().is_empty());
    // A read-only refresh remains available and cannot erase retained evidence.
    studio
        .session_contexts_with_refresh(&session.info.id, true)
        .await
        .unwrap();
    assert_eq!(studio.pending_snowflake_receipts.lock().len(), 1);
    // Even a failed rollback started audit must not strand an active transaction.
    database.execute_batch("DROP TRIGGER reject_terminal; CREATE TRIGGER reject_audit BEFORE INSERT ON audit_v7 BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
    let mut recovery = driver.runtime_override.lock().as_ref().unwrap().clone();
    recovery.transaction_control_authority = Some(TransactionControlAuthority::RollbackOnly {
        revision: revision.clone(),
    });
    *driver.runtime_override.lock() = Some(recovery.clone());
    *driver.boundary_result.lock() = Some(TransactionControlResult {
        action: ExplicitTransactionAction::Rollback,
        boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
        state: recovery,
        retired: false,
        error: None,
    });
    let rollback_request = TransactionControlRequest {
        action: ExplicitTransactionAction::Rollback,
        revision,
    };
    let rollback = studio
        .prepare_transaction_control(&session.info.id, &rollback_request)
        .unwrap();
    let rolled_back = studio
        .execute_transaction_control(&rollback.id, None)
        .await
        .unwrap();
    assert_eq!(
        rolled_back.boundary_outcome,
        TransactionBoundaryOutcome::Acknowledged
    );
    assert!(rolled_back.error.is_some());
    assert_eq!(studio.pending_snowflake_receipts.lock().len(), 2);
    // An unchanged rollback-only observation cannot grow the outbox by replay.
    let repeated = studio
        .prepare_transaction_control(&session.info.id, &rollback_request)
        .unwrap();
    assert_eq!(
        studio
            .execute_transaction_control(&repeated.id, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(studio.pending_snowflake_receipts.lock().len(), 2);
    assert_eq!(driver.boundary_calls.lock().len(), 1);
    studio.disconnect(&session.info.id);
    assert!(studio.shutdown_and_wait().await.is_err());
    database
        .execute_batch("DROP TRIGGER reject_audit; DROP TRIGGER reject_history;")
        .unwrap();
    studio.shutdown_and_wait().await.unwrap();
    studio.shutdown_and_wait().await.unwrap();
    assert!(studio.pending_snowflake_receipts.lock().is_empty());
    assert_eq!(studio.history("").unwrap().len(), 1);
    assert_eq!(terminal_audits(&database, "snowflake-dml", "succeeded"), 1);
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "started"),
        1
    );
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "acknowledged"),
        1
    );
    assert_eq!(driver.dml_calls.lock().len(), 1);
    assert_eq!(driver.boundary_calls.lock().len(), 1);
}

#[tokio::test]
async fn receipt_rollback_failed_started_audit_keeps_both_stages_atomic() {
    let (root, studio, driver, session, request) =
        explicit_control_fixture(SafeMode::Silent, ExplicitTransactionAction::Rollback, None);
    let database = receipt_database(&root);
    database.execute_batch("CREATE TRIGGER reject_started BEFORE INSERT ON audit_v7 WHEN NEW.outcome='started' BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
    let plan = studio
        .prepare_transaction_control(&session.info.id, &request)
        .unwrap();
    let result = studio
        .execute_transaction_control(&plan.id, None)
        .await
        .unwrap();
    assert_eq!(
        result.boundary_outcome,
        TransactionBoundaryOutcome::Acknowledged
    );
    assert!(result.error.is_some());
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "acknowledged"),
        0
    );
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "started"),
        0
    );
    assert_eq!(studio.pending_snowflake_receipts.lock().len(), 1);
    {
        let receipts = studio.pending_snowflake_receipts.lock();
        let cell = receipts.values().next().unwrap();
        assert!(cell.is_captured());
        assert_eq!(
            compact_receipt(cell)["boundary"],
            serde_json::json!({
                "action": "rollback", "boundaryOutcome": "acknowledged", "retired": false,
            })
        );
    }
    let repeated = studio
        .prepare_transaction_control(&session.info.id, &request)
        .unwrap();
    assert_eq!(
        studio
            .execute_transaction_control(&repeated.id, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(studio.shutdown_and_wait().await.is_err());
    database
        .execute_batch("DROP TRIGGER reject_started;")
        .unwrap();
    studio.shutdown_and_wait().await.unwrap();
    studio.shutdown_and_wait().await.unwrap();
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "acknowledged"),
        1
    );
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "started"),
        1
    );
    assert_eq!(driver.boundary_calls.lock().len(), 1);
    assert!(studio.pending_snowflake_receipts.lock().is_empty());
}

#[tokio::test]
async fn receipt_rollback_predispatch_error_does_not_invent_a_finished_outcome() {
    let (root, studio, driver, session, request) =
        explicit_control_fixture(SafeMode::Silent, ExplicitTransactionAction::Rollback, None);
    *driver.boundary_predispatch_error.lock() = Some(ErrorCode::Conflict);
    let database = receipt_database(&root);
    database.execute_batch("CREATE TRIGGER reject_started BEFORE INSERT ON audit_v7 WHEN NEW.outcome='started' BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
    let plan = studio
        .prepare_transaction_control(&session.info.id, &request)
        .unwrap();
    let error = studio
        .execute_transaction_control(&plan.id, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.message, "Fixture rejected before remote submission");
    assert!(driver.boundary_calls.lock().is_empty());
    for cell in studio.pending_snowflake_receipts.lock().values() {
        assert!(!cell.is_captured());
    }
    let captured: u32 = database.query_row(
        "SELECT COUNT(*) FROM outcome_operations_v7 WHERE capture_digest IS NOT NULL OR payload_json IS NOT NULL",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(captured, 0, "predispatch rejection has no original capture");
    assert!(studio.history("").unwrap().is_empty());
    for outcome in ["started", "acknowledged", "rejected", "unknown"] {
        assert_eq!(
            terminal_audits(&database, "transaction-boundary", outcome),
            0
        );
    }
    studio.shutdown_and_wait().await.unwrap();
    studio.shutdown_and_wait().await.unwrap();
    assert!(studio.pending_snowflake_receipts.lock().is_empty());
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn receipt_full_ordinary_capacity_refuses_dml_but_preserves_recovery_rollback() {
    let (root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
    let database = receipt_database(&root);
    let leases: Vec<_> = (0..32)
        .map(|_| studio.outcome_memory.ordinary().unwrap())
        .collect();
    assert!(studio.outcome_memory.ordinary().is_err());
    let plan = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    let error = studio
        .execute_snowflake_dml(&plan.id, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ResultLimitReached);
    assert!(driver.dml_calls.lock().is_empty());
    assert!(studio.history("").unwrap().is_empty());
    assert!(studio.audit_log().unwrap().is_empty());

    let original_runtime = driver.runtime_override.lock().as_ref().unwrap().clone();
    let mut recovery = original_runtime.clone();
    let revision = new_id();
    recovery.transaction_control_authority = Some(TransactionControlAuthority::RollbackOnly {
        revision: revision.clone(),
    });
    *driver.runtime_override.lock() = Some(recovery.clone());
    *driver.boundary_result.lock() = Some(TransactionControlResult {
        action: ExplicitTransactionAction::Rollback,
        boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
        state: recovery,
        retired: false,
        error: None,
    });
    let rollback = studio
        .prepare_transaction_control(
            &session.info.id,
            &TransactionControlRequest {
                action: ExplicitTransactionAction::Rollback,
                revision,
            },
        )
        .unwrap();
    let result = studio
        .execute_transaction_control(&rollback.id, None)
        .await
        .unwrap();
    assert_eq!(
        result.boundary_outcome,
        TransactionBoundaryOutcome::Acknowledged
    );
    assert!(result.error.is_none());
    assert!(driver.dml_calls.lock().is_empty());
    assert_eq!(driver.boundary_calls.lock().len(), 1);
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "started"),
        1
    );
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "acknowledged"),
        1
    );
    assert!(studio.outcome_memory.ordinary().is_err());

    drop(leases);
    *driver.runtime_override.lock() = Some(original_runtime);
    studio
        .session_contexts_with_refresh(&session.info.id, true)
        .await
        .unwrap();
    let next = studio
        .prepare_execution(&session.info.id, SDK_DML_SQL)
        .unwrap();
    let result = studio.execute_snowflake_dml(&next.id, None).await.unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
    assert!(result.error.is_none());
    assert_eq!(driver.dml_calls.lock().len(), 1);
    studio.shutdown_and_wait().await.unwrap();
    assert!(studio.pending_snowflake_receipts.lock().is_empty());
}

#[tokio::test]
async fn receipt_generic_recording_failure_blocks_begin_and_commit_but_allows_rollback() {
    for action in [
        ExplicitTransactionAction::Begin,
        ExplicitTransactionAction::Commit,
    ] {
        let (root, studio, driver, session, request) =
            explicit_control_fixture(SafeMode::Silent, action, None);
        let seed = HistoryEntry {
            id: new_id(),
            connection_id: session.profile.id.clone(),
            connection_name: session.profile.name.clone(),
            started_at: now_ms(),
            sql: None,
            statement_count: 1,
            provenance: HistoryProvenance::Admitted,
            status: None,
            elapsed_ms: None,
            execution_summary: None,
            snowflake_dml: None,
            snowflake_ddl: None,
        };
        let cell = studio
            .admit_outcome(
                studio.outcome_memory.ordinary().unwrap(),
                Some(&seed),
                crate::persistence::OutcomeDescriptor {
                    operation_id: seed.id.clone(),
                    connection_id: seed.connection_id.clone(),
                    session_id: session.info.id.clone(),
                    started_at: seed.started_at,
                    digest: "b".repeat(64),
                    kind: crate::persistence::OutcomeKind::Execute { planned: 1 },
                },
                crate::persistence::OutcomePool::Ordinary,
            )
            .unwrap();
        let database = receipt_database(&root);
        reject_receipt_write(&database, "history");
        assert!(
            studio
                .finish_outcome_history(
                    &cell,
                    &HistoryCompletion {
                        provenance: HistoryProvenance::Recorded,
                        status: Some(RunStatus::Completed),
                        elapsed_ms: Some(7),
                        execution_summary: None,
                        snowflake_dml: None,
                        snowflake_ddl: None,
                    }
                )
                .is_err()
        );
        assert!(cell.is_captured());
        let plan = studio
            .prepare_transaction_control(&session.info.id, &request)
            .unwrap();
        assert_eq!(
            studio
                .execute_transaction_control(&plan.id, None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert!(driver.boundary_calls.lock().is_empty());
        let mut state = driver.runtime_override.lock().as_ref().unwrap().clone();
        let revision = new_id();
        state.transaction_control_authority = Some(TransactionControlAuthority::RollbackOnly {
            revision: revision.clone(),
        });
        *driver.runtime_override.lock() = Some(state.clone());
        *driver.boundary_result.lock() = Some(TransactionControlResult {
            action: ExplicitTransactionAction::Rollback,
            boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
            state,
            retired: false,
            error: None,
        });
        let rollback = studio
            .prepare_transaction_control(
                &session.info.id,
                &TransactionControlRequest {
                    action: ExplicitTransactionAction::Rollback,
                    revision,
                },
            )
            .unwrap();
        let result = studio
            .execute_transaction_control(&rollback.id, None)
            .await
            .unwrap();
        assert_eq!(
            result.boundary_outcome,
            TransactionBoundaryOutcome::Acknowledged
        );
        assert_eq!(driver.boundary_calls.lock().len(), 1);
        assert!(studio.flush_pending_history().is_err());
        database
            .execute_batch("DROP TRIGGER reject_history;")
            .unwrap();
        studio.shutdown_and_wait().await.unwrap();
        assert_eq!(
            studio.history_entry(&seed.id).unwrap().unwrap().status,
            Some(RunStatus::Completed)
        );
        assert_eq!(driver.boundary_calls.lock().len(), 1);
    }
}

#[tokio::test]
async fn receipt_postadmission_callback_panic_abandons_before_task_drain() {
    for dml in [false, true] {
        let (_root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
        let plan = if dml {
            studio
                .prepare_execution(&session.info.id, SDK_DML_SQL)
                .unwrap()
        } else {
            let state = driver.runtime_override.lock().as_ref().unwrap().clone();
            let TransactionControlAuthority::Observed { revision, .. } =
                state.transaction_control_authority.unwrap()
            else {
                panic!("fixture authority")
            };
            studio
                .prepare_transaction_control(
                    &session.info.id,
                    &TransactionControlRequest {
                        action: ExplicitTransactionAction::Begin,
                        revision,
                    },
                )
                .unwrap()
        };
        driver
            .panic_during_invalidation
            .store(true, Ordering::Release);
        let error = if dml {
            studio
                .execute_snowflake_dml(&plan.id, None)
                .await
                .unwrap_err()
        } else {
            studio
                .execute_transaction_control(&plan.id, None)
                .await
                .unwrap_err()
        };
        assert_eq!(error.code, ErrorCode::OutcomeUnknown);
        assert!(driver.dml_calls.lock().is_empty());
        assert!(driver.boundary_calls.lock().is_empty());
        driver
            .panic_during_invalidation
            .store(false, Ordering::Release);
        studio.shutdown_and_wait().await.unwrap();
        assert!(studio.pending_snowflake_receipts.lock().is_empty());
        assert!(
            studio
                .audit_log()
                .unwrap()
                .iter()
                .all(|a| a.outcome == "started")
        );
        for row in studio.history("").unwrap() {
            assert_eq!(row.provenance, HistoryProvenance::Interrupted);
            assert!(row.status.is_none());
        }
    }
}

#[tokio::test]
async fn receipt_boundary_return_preserves_primary_error_without_retaining_it() {
    let (root, studio, driver, session, request) =
        explicit_control_fixture(SafeMode::Silent, ExplicitTransactionAction::Begin, None);
    let primary = AppError::new(
        ErrorCode::Database,
        "private-boundary-observer-error".repeat(4096),
    )
    .with_completion_invalidated();
    driver.boundary_result.lock().as_mut().unwrap().error = Some(primary.clone());
    let plan = studio
        .prepare_transaction_control(&session.info.id, &request)
        .unwrap();
    let database = receipt_database(&root);
    database.execute_batch("CREATE TRIGGER reject_terminal BEFORE INSERT ON audit_v7 WHEN NEW.outcome!='started' BEGIN SELECT RAISE(ABORT,'local fixture failure'); END;").unwrap();
    let result = studio
        .execute_transaction_control(&plan.id, None)
        .await
        .unwrap();
    assert_eq!(result.action, request.action);
    assert_eq!(
        result.boundary_outcome,
        TransactionBoundaryOutcome::Acknowledged
    );
    let returned = result.error.unwrap();
    assert_eq!(returned.code, primary.code);
    assert_eq!(
        returned.completion_invalidated,
        primary.completion_invalidated
    );
    assert!(returned.message.starts_with(&primary.message));
    assert!(returned.message.contains("captured locally"));
    {
        let pending = studio.pending_snowflake_receipts.lock();
        let cell = pending.values().next().unwrap();
        assert!(cell.is_captured());
        assert!(
            !compact_receipt(cell)
                .to_string()
                .contains("private-boundary-observer-error")
        );
    }
    database
        .execute_batch("DROP TRIGGER reject_terminal;")
        .unwrap();
    studio.shutdown_and_wait().await.unwrap();
    assert_eq!(driver.boundary_calls.lock().len(), 1);
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "acknowledged"),
        1
    );
}

#[tokio::test]
async fn receipt_retirement_callback_panic_preserves_each_returned_original() {
    for kind in ["dml", "ddl", "boundary"] {
        let (root, studio, driver, session, ddl_request, boundary_request) = match kind {
            "ddl" => {
                let (root, studio, driver, session, request) =
                    sdk_ddl_fixture(SafeMode::Silent, false, None);
                driver.ddl_result.lock().as_mut().unwrap().retired = true;
                (root, studio, driver, session, Some(request), None)
            }
            "boundary" => {
                let (root, studio, driver, session, request) = explicit_control_fixture(
                    SafeMode::Silent,
                    ExplicitTransactionAction::Begin,
                    None,
                );
                driver.boundary_result.lock().as_mut().unwrap().retired = true;
                (root, studio, driver, session, None, Some(request))
            }
            _ => {
                let (root, studio, driver, session) = sdk_dml_fixture(SafeMode::Silent, None);
                driver.dml_result.lock().as_mut().unwrap().retired = true;
                (root, studio, driver, session, None, None)
            }
        };
        let plan = if let Some(request) = ddl_request.as_ref() {
            ddl_review(&studio, request).await
        } else if let Some(request) = boundary_request.as_ref() {
            studio
                .prepare_transaction_control(&session.info.id, request)
                .unwrap()
        } else {
            studio
                .prepare_snowflake_dml(&session.info.id, SDK_DML_SQL, None)
                .unwrap()
        };
        let approval = ddl_request.as_ref().map(|_| ddl_approve(&studio, &plan));
        let database = receipt_database(&root);
        database.execute_batch("CREATE TRIGGER reject_terminal BEFORE INSERT ON audit_v7 WHEN NEW.outcome!='started' BEGIN SELECT RAISE(ABORT,'local fixture failure'); END;").unwrap();
        driver.panic_during_close.store(true, Ordering::Release);
        let error = match kind {
            "ddl" => studio
                .execute_snowflake_ddl(&plan.id, approval.as_deref())
                .await
                .unwrap_err(),
            "boundary" => studio
                .execute_transaction_control(&plan.id, None)
                .await
                .unwrap_err(),
            _ => studio
                .execute_snowflake_dml(&plan.id, None)
                .await
                .unwrap_err(),
        };
        assert_eq!(error.code, ErrorCode::OutcomeUnknown);
        driver.panic_during_close.store(false, Ordering::Release);
        let original = {
            let pending = studio.pending_snowflake_receipts.lock();
            assert_eq!(pending.len(), 1);
            let cell = pending.values().next().unwrap();
            assert!(cell.is_captured());
            let payload = compact_receipt(cell);
            if kind == "boundary" {
                assert_eq!(payload["boundary"]["boundaryOutcome"], "acknowledged");
                assert_eq!(payload["boundary"]["retired"], true);
            } else {
                let evidence = if kind == "ddl" {
                    "snowflakeDdl"
                } else {
                    "snowflakeDml"
                };
                assert_eq!(
                    payload["completion"][evidence]["statementOutcome"],
                    "succeeded"
                );
                assert_eq!(payload["completion"][evidence]["retired"], true);
            }
            payload
        };
        database
            .execute_batch("DROP TRIGGER reject_terminal;")
            .unwrap();
        studio.shutdown_and_wait().await.unwrap();
        studio.shutdown_and_wait().await.unwrap();
        assert_eq!(driver.dml_calls.lock().len(), usize::from(kind == "dml"));
        assert_eq!(driver.ddl_calls.lock().len(), usize::from(kind == "ddl"));
        assert_eq!(
            driver.boundary_calls.lock().len(),
            usize::from(kind == "boundary")
        );
        let finished: u32 = database
            .query_row(
                "SELECT COUNT(*) FROM audit_v7 WHERE outcome!='started'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(finished, 1);
        if kind != "boundary" {
            let rows = studio.history("").unwrap();
            assert_eq!(rows.len(), 1);
            let completion = serde_json::to_value(HistoryCompletion {
                provenance: rows[0].provenance,
                execution_summary: rows[0].execution_summary.clone(),
                snowflake_dml: rows[0].snowflake_dml.clone(),
                snowflake_ddl: rows[0].snowflake_ddl.clone(),
                status: rows[0].status,
                elapsed_ms: rows[0].elapsed_ms,
            })
            .unwrap();
            assert_eq!(completion, original["completion"]);
        }
    }
}
