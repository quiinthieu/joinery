// Test-owned fault injection happens after durable admission. Completion updates
// the retained history row rather than inserting a second SQL-bearing payload.
fn reject_receipt_write(database: &rusqlite::Connection, table: &str) {
    let operation = match table {
        "history" => "UPDATE",
        "audit" => "INSERT",
        _ => panic!("unknown fixture table"),
    };
    database.execute_batch(&format!("CREATE TRIGGER reject_{table} BEFORE {operation} ON {table}_v5 BEGIN SELECT RAISE(ABORT, 'synthetic local failure'); END;")).unwrap();
}

// Included beside the Studio policy fixtures; no account or helper connection.
fn receipt_database(root: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap()
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
            "SELECT COUNT(*) FROM audit_v5 WHERE action=?1 AND outcome=?2",
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
        let primary = AppError::new(ErrorCode::Database, "original remote or observer failure")
            .with_completion_invalidated();
        driver.dml_result.lock().as_mut().unwrap().statement_outcome = outcome;
        driver.dml_result.lock().as_mut().unwrap().error = Some(primary.clone());
        let plan = studio
            .prepare_execution(&session.info.id, SDK_DML_SQL)
            .unwrap();
        let database = receipt_database(&root);
        database.execute_batch("CREATE TRIGGER reject_terminal BEFORE INSERT ON audit_v5 WHEN NEW.outcome != 'started' BEGIN SELECT RAISE(ABORT, 'local failure'); END; CREATE TRIGGER reject_history BEFORE UPDATE ON history_v5 BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
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
        let original_entry = {
            let receipts = studio.pending_snowflake_receipts.lock();
            assert_eq!(receipts.len(), 1);
            let receipt = receipts.values().next().unwrap();
            let snowflake_receipts::OriginalResult::Dml(original) = &receipt.original else {
                panic!("wrong receipt type")
            };
            assert_eq!(original.statement_outcome, outcome);
            assert_eq!(original.summary, result.summary);
            assert_eq!(original.error.as_ref().unwrap().message, primary.message);
            receipt.entry.as_ref().unwrap().clone()
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
        assert_eq!(history[0].id, original_entry.id);
        assert_eq!(
            serde_json::to_value(&history[0].snowflake_dml).unwrap(),
            serde_json::to_value(&original_entry.snowflake_dml).unwrap()
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
        database.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit_v5 BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
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
            let snowflake_receipts::OriginalResult::Boundary(original) = &receipt.original else {
                panic!("wrong receipt type")
            };
            assert_eq!(original.action, request.action);
            assert_eq!(original.boundary_outcome, outcome);
            assert_eq!(
                original.error.as_ref().map(|error| &error.message),
                primary.as_ref().map(|error| &error.message)
            );
            assert!(original.retired);
            assert!(original.state.transaction_control_authority.is_none());
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
    database.execute_batch("CREATE TRIGGER reject_terminal BEFORE INSERT ON audit_v5 WHEN NEW.outcome != 'started' BEGIN SELECT RAISE(ABORT, 'local failure'); END; CREATE TRIGGER reject_history BEFORE UPDATE ON history_v5 BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
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
    database.execute_batch("DROP TRIGGER reject_terminal; CREATE TRIGGER reject_audit BEFORE INSERT ON audit_v5 BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
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
async fn receipt_rollback_keeps_only_failed_started_stage_when_terminal_audit_succeeds() {
    let (root, studio, driver, session, request) =
        explicit_control_fixture(SafeMode::Silent, ExplicitTransactionAction::Rollback, None);
    let database = receipt_database(&root);
    database.execute_batch("CREATE TRIGGER reject_started BEFORE INSERT ON audit_v5 WHEN NEW.outcome='started' BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
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
        1
    );
    assert_eq!(
        terminal_audits(&database, "transaction-boundary", "started"),
        0
    );
    assert_eq!(studio.pending_snowflake_receipts.lock().len(), 1);
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
    database.execute_batch("CREATE TRIGGER reject_started BEFORE INSERT ON audit_v5 WHEN NEW.outcome='started' BEGIN SELECT RAISE(ABORT, 'local failure'); END;").unwrap();
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
    assert!(studio.pending_snowflake_receipts.lock().is_empty());
    assert!(studio.history("").unwrap().is_empty());
    for outcome in ["started", "acknowledged", "rejected", "unknown"] {
        assert_eq!(
            terminal_audits(&database, "transaction-boundary", outcome),
            0
        );
    }
    studio.shutdown_and_wait().await.unwrap();
    studio.shutdown_and_wait().await.unwrap();
    assert_eq!(driver.state_calls.load(Ordering::Relaxed), 0);
}
