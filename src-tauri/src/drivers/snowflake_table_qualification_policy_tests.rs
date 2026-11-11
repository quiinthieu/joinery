use super::*;
use crate::drivers::snowflake_process::{
    Certainty, Impact, Phase as WirePhase, Report, StatementIdentity,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    task::{Context, Waker},
};

fn profile() -> Value {
    json!({"account":"synthetic-account", "host":"synthetic-account.snowflakecomputing.com",
        "username":"login@example.test", "role":"QUALIFICATION", "warehouse":"FIXTURE_WH",
        "database":"SANDBOX", "schema":"BILLIE_TESTING"})
}

struct Frame {
    envelope: Value,
    bytes: Vec<u8>,
    ticket: u64,
    operation: String,
    impact: Impact,
}
impl Frame {
    fn new(id: u64, operation: &str, impact: Impact, payload: Value) -> Self {
        let envelope = json!({"v":1,"generation":"fixture-generation","id":id.to_string(),
            "op":operation,"payload":payload});
        let bytes = serde_json::to_vec(&envelope).unwrap();
        Self {
            envelope,
            bytes,
            ticket: id - 1,
            operation: operation.into(),
            impact,
        }
    }
    fn request(&self) -> PhysicalRequest<'_> {
        PhysicalRequest {
            generation: "fixture-generation",
            operation_id: "reusable-logical-handle",
            ticket: self.ticket,
            operation: &self.operation,
            impact: self.impact,
            target: None,
            payload: &self.envelope,
            wire_payload: &self.bytes,
        }
    }
    fn terminal<'a>(&'a self, report: &'a Report) -> PhysicalTerminal<'a> {
        PhysicalTerminal {
            generation: "fixture-generation",
            operation_id: "reusable-logical-handle",
            ticket: self.ticket,
            wire_id: self.envelope["id"].as_str().unwrap(),
            operation: &self.operation,
            impact: self.impact,
            report,
        }
    }
}

fn report(result: Value) -> Report {
    Report {
        ok: true,
        certainty: Certainty::StatementSucceeded,
        phase: WirePhase::Acknowledged,
        result: Some(result),
        error_code: None,
        retirement: None,
        statements: vec![StatementIdentity {
            sdk_request_id: "fixture-sdk-id".into(),
            query_id: Some("fixture-query-id".into()),
        }],
        use_acknowledged: None,
        boundary_acknowledged: None,
        dml_acknowledged: None,
        ddl_acknowledged: None,
    }
}

fn connect_report() -> Report {
    let mut report = report(json!({}));
    report.certainty = Certainty::ServerTerminal;
    report.statements.clear();
    report
}

async fn policy() -> (tempfile::TempDir, Policy) {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let policy = Policy::create(directory.path().to_owned(), profile())
        .await
        .unwrap();
    (directory, policy)
}

fn records(directory: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(directory.join(super::super::journal::FILE_NAME))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn owning_create_with_restored_plan_pins_exact_identity_in_ledger_and_journal_header() {
    let original = Plan::generate().unwrap();
    let bytes = original.canonical_json().unwrap();
    let hash = original.sha256().unwrap();
    let restored = Plan::restore_snapshot(&bytes, &hash).unwrap();
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let policy = Policy::create_with_plan(directory.path().to_owned(), profile(), restored)
        .await
        .unwrap();
    assert_eq!(policy.plan().canonical_json().unwrap(), bytes);
    assert_eq!(policy.plan().run(), original.run());
    assert_eq!(
        policy.plan().qualified_target(),
        original.qualified_target()
    );
    assert_eq!(policy.state.lock().sequence.ledger().plan_sha256(), hash);
    let header = records(directory.path());
    assert_eq!(header.len(), 1);
    assert_eq!(header[0]["event"]["planSha256"], hash);
    let connect = Frame::new(1, "connect", Impact::Control, profile());
    TransportPolicy::admit(&policy, connect.request())
        .await
        .unwrap();
    assert_eq!(records(directory.path()).len(), 2);
    assert_eq!(policy.plan().canonical_json().unwrap(), bytes);
    assert_eq!(policy.state.lock().sequence.ledger().plan_sha256(), hash);
}

#[tokio::test]
async fn owning_create_with_plan_rejects_invalid_profiles_before_journal_creation() {
    for profile in [Value::Null, json!({}), {
        let mut changed = profile();
        changed["database"] = json!("OTHER");
        changed
    }] {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            Policy::create_with_plan(
                directory.path().to_owned(),
                profile,
                Plan::generate().unwrap()
            )
            .await
            .is_err()
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn owning_admission_syncs_intent_and_prior_original_terminal_before_returning() {
    let (directory, policy) = policy().await;
    let connect = Frame::new(1, "connect", Impact::Control, profile());
    TransportPolicy::admit(&policy, connect.request())
        .await
        .unwrap();
    let rows = records(directory.path());
    assert_eq!(rows.len(), 2);
    assert!(rows[1]["event"].to_string().contains("fixture-generation"));
    let original = connect_report();
    TransportPolicy::terminal(&policy, connect.terminal(&original)).unwrap();
    assert_eq!(
        records(directory.path()).len(),
        2,
        "callback performs no IO"
    );
    let context = Frame::new(2, "context", Impact::Read, json!({}));
    TransportPolicy::admit(&policy, context.request())
        .await
        .unwrap();
    let rows = records(directory.path());
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[3]["sequence"], 3);
    assert!(policy.state.lock().pending.is_empty());
    let original = report(
        json!({"user":"FIXTURE", "role":"QUALIFICATION", "database":"SANDBOX",
        "schema":"BILLIE_TESTING", "warehouse":"FIXTURE_WH", "session":"opaque-session",
        "sdkRequestId":"fixture-sdk-id", "queryId":"fixture-query-id"}),
    );
    TransportPolicy::terminal(&policy, context.terminal(&original)).unwrap();
    let entry = policy.state.lock().sequence.next_entry().unwrap().clone();
    let retention = entry.retention().unwrap();
    let version = Frame::new(
        3,
        "query",
        Impact::Read,
        json!({"sql":entry.sql(),"binds":[],
        "classification":"read","credits":1,"maxRows":retention.max_rows,"maxBytes":retention.max_bytes}),
    );
    TransportPolicy::admit(&policy, version.request())
        .await
        .unwrap();
    let rows = records(directory.path());
    assert_eq!(rows.len(), 6);
    assert!(rows[4]["event"].to_string().contains("fixture-sdk-id"));
    assert!(rows[4]["event"].to_string().contains("fixture-query-id"));
}

#[tokio::test]
async fn owning_admission_poisoned_journal_cannot_retry_reserved_connect() {
    let (directory, policy) = policy().await;
    let journal = directory.path().join(super::super::journal::FILE_NAME);
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o400)).unwrap();
    let frame = Frame::new(1, "connect", Impact::Control, profile());
    assert!(
        TransportPolicy::admit(&policy, frame.request())
            .await
            .is_err()
    );
    assert_eq!(records(directory.path()).len(), 1);
    assert_eq!(
        policy.state.lock().pending.len(),
        1,
        "original reserved intent retained"
    );
    assert!(
        TransportPolicy::admit(&policy, frame.request())
            .await
            .is_err()
    );
    assert!(TransportPolicy::check(&policy).is_err());
    assert_eq!(records(directory.path()).len(), 1);
}

#[tokio::test]
async fn owning_cancelled_admission_latches_failure_while_blocking_sync_is_outstanding() {
    let (directory, policy) = policy().await;
    let frame = Frame::new(1, "connect", Impact::Control, profile());
    {
        let _held = policy.journal.lock().unwrap();
        let mut future = TransportPolicy::admit(&policy, frame.request());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        assert_eq!(policy.state.lock().pending.len(), 1);
        drop(future);
        assert!(TransportPolicy::check(&policy).is_err());
    }
    assert!(
        TransportPolicy::admit(&policy, frame.request())
            .await
            .is_err()
    );
    assert!(policy.persist_final_evidence().await.is_err());
    assert!(policy.persist_final_evidence().await.is_err());
    assert_eq!(
        records(directory.path()).len(),
        2,
        "cancelled original intent persisted exactly once"
    );
    assert!(policy.state.lock().pending.is_empty());
}

fn original_ddl_ack() -> crate::domain::SnowflakeDdlResult {
    use crate::domain::*;
    SnowflakeDdlResult {
        kind: SnowflakeDdlKind::CreateTable,
        qualified_target: "\"SANDBOX\".\"BILLIE_TESTING\".\"ORIGINAL_TARGET\"".into(),
        statement_outcome: SnowflakeDdlOutcome::Succeeded,
        summary_status: SnowflakeDdlSummaryStatus::Acknowledged,
        catalog_reconciliation_required: true,
        state: SessionRuntimeState {
            session_id: "original-console".into(),
            transaction_mode: TransactionMode::AutoCommit,
            transaction_state: TransactionState::Idle,
            context_kind: SessionContextKind::Schema,
            active_context: Some(SCHEMA.into()),
            database: "SANDBOX".into(),
            role: Some("QUALIFICATION".into()),
            search_path: vec!["SANDBOX.BILLIE_TESTING".into()],
            context_fingerprint: Some("original-fingerprint".into()),
            transaction_observation: Some(TransactionObservation::Observed {
                transaction_id: None,
                abort_on_error: true,
                usability: TransactionUsability::Unknown,
            }),
            transaction_control_authority: Some(TransactionControlAuthority::Observed {
                revision: "original-authority".into(),
                allowed_actions: vec![
                    ExplicitTransactionAction::Begin,
                    ExplicitTransactionAction::Rollback,
                ],
            }),
        },
        retired: true,
        error: Some(AppError::internal("original post-ACK retirement")),
    }
}

fn completion_records(directory: &std::path::Path) -> Vec<Value> {
    records(directory)
        .into_iter()
        .map(|record| record["event"].clone())
        .filter(|event| event["type"] == "operationCompletion")
        .collect()
}

fn final_guard_records(directory: &std::path::Path) -> Vec<Value> {
    records(directory)
        .into_iter()
        .map(|record| record["event"].clone())
        .filter(|event| event["type"] == "executionEnvelopeFinal")
        .collect()
}

#[tokio::test]
async fn owning_final_guard_syncs_after_failure_without_replacing_original_ack() {
    let (directory, policy) = policy().await;
    let original = original_ddl_ack();
    policy
        .record_operation_completion(OperationCompletion::Returned(PhaseEvidence::Ddl(&original)))
        .await
        .unwrap();
    TransportPolicy::failed(&policy, AppError::internal("earlier observer failure"));
    let failure = AppError::internal("final envelope drift");
    policy
        .record_final_guard(&Err(failure.clone()), true)
        .await
        .unwrap();
    let final_records = final_guard_records(directory.path());
    assert_eq!(final_records.len(), 1);
    assert_eq!(
        final_records[0],
        json!({"type":"executionEnvelopeFinal",
        "revalidation":{"ok":false,"error":failure},"scannerSettled":true})
    );
    assert_eq!(
        completion_records(directory.path())[0]["completion"]["original"]["value"],
        serde_json::to_value(&original).unwrap()
    );
    assert_eq!(
        TransportPolicy::check(&policy).unwrap_err().message,
        "earlier observer failure"
    );
    assert!(policy.record_final_guard(&Ok(()), true).await.is_err());
    assert!(policy.persist_final_evidence().await.is_err());
    assert_eq!(final_guard_records(directory.path()), final_records);
    assert!(policy.state.lock().pending.is_empty());
}

#[tokio::test]
async fn owning_final_guard_records_unsettled_scan_as_failure() {
    let (directory, policy) = policy().await;
    policy.record_final_guard(&Ok(()), false).await.unwrap();
    assert_eq!(
        final_guard_records(directory.path()),
        vec![json!({
        "type":"executionEnvelopeFinal","revalidation":{"ok":true},
        "scannerSettled":false})]
    );
    assert!(
        TransportPolicy::check(&policy)
            .unwrap_err()
            .message
            .contains("not settled")
    );
}

#[tokio::test]
async fn owning_final_guard_poisoned_journal_cannot_claim_storage_or_replace_record() {
    let (directory, policy) = policy().await;
    fs::set_permissions(
        directory.path().join(super::super::journal::FILE_NAME),
        fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    let original = Err(AppError::internal("original final guard failure"));
    assert!(policy.record_final_guard(&original, false).await.is_err());
    assert!(policy.state.lock().final_guard_recorded);
    assert_eq!(policy.state.lock().pending.len(), 1);
    assert!(policy.record_final_guard(&Ok(()), true).await.is_err());
    assert_eq!(policy.state.lock().pending.len(), 1);
    assert!(policy.persist_final_evidence().await.is_err());
    assert!(final_guard_records(directory.path()).is_empty());
    assert_eq!(
        TransportPolicy::check(&policy).unwrap_err().message,
        "original final guard failure"
    );
}

#[tokio::test]
async fn owning_cancelled_final_guard_syncs_original_once_without_replacement() {
    let (directory, policy) = policy().await;
    let failure = AppError::internal("original final guard failure");
    let original = Err(failure.clone());
    {
        let _held = policy.journal.lock().unwrap();
        let mut future = Box::pin(policy.record_final_guard(&original, false));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        assert_eq!(policy.state.lock().pending.len(), 1);
        drop(future);
        assert!(TransportPolicy::check(&policy).is_err());
    }
    assert!(policy.persist_final_evidence().await.is_err());
    assert!(policy.record_final_guard(&Ok(()), true).await.is_err());
    assert!(policy.persist_final_evidence().await.is_err());
    assert_eq!(
        final_guard_records(directory.path()),
        vec![json!({
        "type":"executionEnvelopeFinal",
        "revalidation":{"ok":false,"error":failure},
        "scannerSettled":false})]
    );
    assert!(policy.state.lock().pending.is_empty());
}

#[tokio::test]
async fn owning_completion_syncs_original_ack_and_prior_report_after_observer_failure() {
    let (directory, policy) = policy().await;
    let connect = Frame::new(1, "connect", Impact::Control, profile());
    TransportPolicy::admit(&policy, connect.request())
        .await
        .unwrap();
    TransportPolicy::terminal(&policy, connect.terminal(&connect_report())).unwrap();
    TransportPolicy::failed(&policy, AppError::internal("first observer failure"));
    let original = original_ddl_ack();

    let failure = policy
        .record_operation_completion(OperationCompletion::Returned(PhaseEvidence::Ddl(&original)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failure.message, "first observer failure");
    let rows = records(directory.path());
    assert_eq!(
        rows.len(),
        4,
        "header, admission, original report, completion"
    );
    assert!(rows[2]["event"].to_string().contains("fixture-generation"));
    let completions = completion_records(directory.path());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0]["phase"], 1);
    assert_eq!(completions[0]["completion"]["status"], "returned");
    assert_eq!(completions[0]["completion"]["original"]["kind"], "ddl");
    assert_eq!(
        completions[0]["completion"]["original"]["value"],
        serde_json::to_value(&original).unwrap()
    );
    assert!(policy.state.lock().pending.is_empty());
    assert!(
        policy
            .finish_phase(PhaseEvidence::Ddl(&original))
            .await
            .is_err()
    );
    assert!(
        policy
            .record_operation_completion(OperationCompletion::Returned(PhaseEvidence::Ddl(
                &original
            )))
            .await
            .is_err()
    );
    assert_eq!(records(directory.path()).len(), 4);
    assert_eq!(policy.state.lock().sequence.phase().unwrap().number(), 1);
    assert_eq!(
        TransportPolicy::check(&policy).unwrap_err().message,
        "first observer failure"
    );
}

#[tokio::test]
async fn owning_completion_retains_unvalidated_typed_output_before_finish_rejection() {
    let (directory, policy) = policy().await;
    let original = original_ddl_ack();
    assert!(
        policy
            .record_operation_completion(OperationCompletion::Returned(PhaseEvidence::Ddl(
                &original
            )))
            .await
            .unwrap()
            .is_none()
    );
    // This DDL result cannot complete bootstrap. Retention is deliberately
    // independent of whether finish_phase accepts the operation's evidence.
    assert!(
        policy
            .finish_phase(PhaseEvidence::Ddl(&original))
            .await
            .is_err()
    );
    assert!(policy.persist_final_evidence().await.is_err());
    let completions = completion_records(directory.path());
    assert_eq!(completions.len(), 1);
    assert_eq!(
        completions[0]["completion"]["original"]["value"],
        serde_json::to_value(&original).unwrap()
    );
    assert_eq!(policy.state.lock().sequence.phase().unwrap().number(), 1);
}

#[tokio::test]
async fn owning_completion_preserves_original_error_and_partial_query_events() {
    use crate::domain::{ErrorCode, ErrorLocation, SqlSpan};
    let (directory, policy) = policy().await;
    let original = AppError {
        code: ErrorCode::PermissionDenied,
        message: "original collector failure".into(),
        location: Some(Box::new(ErrorLocation {
            span: SqlSpan {
                start_utf16: 4,
                end_utf16: 9,
                line: 2,
                column: 3,
            },
            statement_index: Some(0),
            sql_state: Some("42501".into()),
            detail: Some("original detail".into()),
            hint: Some("original hint".into()),
        })),
        completion_invalidated: Some(true),
    };
    let events = vec![
        QueryEvent::Started {
            run_id: "original-run".into(),
        },
        QueryEvent::StatementStarted {
            run_id: "original-run".into(),
            index: 0,
        },
        QueryEvent::Progress {
            result_id: "original-result".into(),
            retained_rows: 1,
        },
    ];
    let failure = policy
        .record_operation_completion(OperationCompletion::Failed {
            error: &original,
            events: &events,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(failure).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    let completions = completion_records(directory.path());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0]["completion"]["status"], "failed");
    assert_eq!(
        completions[0]["completion"]["original"]["error"],
        serde_json::to_value(&original).unwrap()
    );
    assert_eq!(
        completions[0]["completion"]["original"]["events"],
        serde_json::to_value(&events).unwrap()
    );
    assert!(policy.state.lock().pending.is_empty());
    assert_eq!(policy.state.lock().sequence.phase().unwrap().number(), 1);
    assert!(TransportPolicy::check(&policy).is_err());
}

#[tokio::test]
async fn owning_completion_does_not_replace_operation_error_with_prior_policy_failure() {
    let (directory, policy) = policy().await;
    TransportPolicy::failed(&policy, AppError::internal("first callback failure"));
    let original = AppError::new(
        crate::domain::ErrorCode::Cancelled,
        "original operation cancelled",
    );
    let events = vec![QueryEvent::Started {
        run_id: "partial-run".into(),
    }];
    let failure = policy
        .record_operation_completion(OperationCompletion::Failed {
            error: &original,
            events: &events,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failure.message, "first callback failure");
    let completions = completion_records(directory.path());
    assert_eq!(completions.len(), 1);
    assert_eq!(
        completions[0]["completion"]["original"]["error"],
        serde_json::to_value(&original).unwrap()
    );
    assert_eq!(
        completions[0]["completion"]["original"]["events"],
        serde_json::to_value(&events).unwrap()
    );
    assert!(policy.state.lock().pending.is_empty());
}

#[tokio::test]
async fn owning_incomplete_read_retains_original_and_cleanup_errors_durably() {
    for prior_failure in [false, true] {
        let (directory, policy) = policy().await;
        if prior_failure {
            TransportPolicy::failed(&policy, AppError::internal("earlier callback failure"));
        }
        let capture_error = AppError::internal("bounded original read capture incomplete");
        let original = AppError::new(
            crate::domain::ErrorCode::Cancelled,
            "original operation cancelled",
        );
        let cleanup_error = AppError::internal("original local disconnect failure");
        let events = vec![QueryEvent::Started {
            run_id: "incomplete-original-run".into(),
        }];
        let failure = policy
            .record_operation_completion(OperationCompletion::IncompleteRead {
                error: &capture_error,
                original_error: Some(&original),
                events: &events,
                cleanup_error: Some(&cleanup_error),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            failure.message,
            if prior_failure {
                "earlier callback failure"
            } else {
                &capture_error.message
            }
        );
        let completions = completion_records(directory.path());
        assert_eq!(completions.len(), 1);
        let completion = &completions[0]["completion"];
        assert_eq!(completion["status"], "incompleteRead");
        assert_eq!(
            completion["original"]["error"],
            serde_json::to_value(&capture_error).unwrap()
        );
        assert_eq!(
            completion["original"]["originalError"],
            serde_json::to_value(&original).unwrap()
        );
        assert_eq!(
            completion["original"]["events"],
            serde_json::to_value(&events).unwrap()
        );
        assert_eq!(
            completion["original"]["cleanupError"],
            serde_json::to_value(&cleanup_error).unwrap()
        );
        assert!(policy.state.lock().pending.is_empty());
        assert_eq!(policy.state.lock().sequence.phase().unwrap().number(), 1);
        assert!(TransportPolicy::check(&policy).is_err());
        let retained = policy.persist_final_evidence().await.unwrap_err();
        assert_eq!(retained.message, failure.message);
        assert_eq!(completion_records(directory.path()).len(), 1);
    }
}

#[tokio::test]
async fn owning_cancelled_completion_syncs_exactly_once_without_phase_advance() {
    let (directory, policy) = policy().await;
    let original = original_ddl_ack();
    {
        let _held = policy.journal.lock().unwrap();
        let mut future = Box::pin(policy.record_operation_completion(
            OperationCompletion::Returned(PhaseEvidence::Ddl(&original)),
        ));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        assert_eq!(policy.state.lock().pending.len(), 1);
        drop(future);
        assert!(TransportPolicy::check(&policy).is_err());
    }
    assert!(policy.persist_final_evidence().await.is_err());
    assert!(policy.persist_final_evidence().await.is_err());
    assert!(
        policy
            .record_operation_completion(OperationCompletion::Returned(PhaseEvidence::Ddl(
                &original
            )))
            .await
            .is_err()
    );
    let completions = completion_records(directory.path());
    assert_eq!(completions.len(), 1);
    assert_eq!(
        completions[0]["completion"]["original"]["value"],
        serde_json::to_value(&original).unwrap()
    );
    assert!(policy.state.lock().pending.is_empty());
    assert_eq!(policy.state.lock().sequence.phase().unwrap().number(), 1);
}

#[tokio::test]
async fn owning_completion_does_not_claim_persistence_when_journal_is_poisoned() {
    let (directory, policy) = policy().await;
    let journal = directory.path().join(super::super::journal::FILE_NAME);
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o400)).unwrap();
    TransportPolicy::failed(&policy, AppError::internal("earlier observer failure"));
    let original = original_ddl_ack();
    assert!(
        policy
            .record_operation_completion(OperationCompletion::Returned(PhaseEvidence::Ddl(
                &original
            )))
            .await
            .is_err()
    );
    assert_eq!(records(directory.path()).len(), 1);
    assert_eq!(policy.state.lock().pending.len(), 1);
    assert!(policy.persist_final_evidence().await.is_err());
    assert_eq!(records(directory.path()).len(), 1);
    assert_eq!(
        TransportPolicy::check(&policy).unwrap_err().message,
        "earlier observer failure"
    );
}

#[tokio::test]
async fn owning_shared_transport_failure_revokes_concrete_phase_owner() {
    use crate::drivers::snowflake_qualification::QualificationAttachment;
    let (_directory, policy) = policy().await;
    let policy = Arc::new(policy);
    let attachment = QualificationAttachment::new(policy.clone(), Default::default()).unwrap();
    let phase = policy.state.lock().sequence.phase().unwrap().number();
    attachment
        .transport
        .fail(AppError::internal("synthetic external transport failure"));
    assert!(attachment.metadata.check().is_err());
    assert!(
        TransportPolicy::check(policy.as_ref()).is_err(),
        "concrete owner must receive external failure"
    );
    attachment
        .transport
        .fail(AppError::internal("later distinct failure"));
    assert_eq!(
        TransportPolicy::check(policy.as_ref()).unwrap_err().message,
        "synthetic external transport failure"
    );
    assert_eq!(
        attachment.metadata.check().unwrap_err().message,
        "synthetic external transport failure"
    );
    assert!(policy.persist_final_evidence().await.is_err());
    assert!(policy.persist_final_evidence().await.is_err());
    assert_eq!(
        policy.state.lock().sequence.phase().unwrap().number(),
        phase
    );
}

#[path = "snowflake_table_qualification_policy_adapter_tests.rs"]
mod adapter_tests;
#[path = "snowflake_table_qualification_policy_route_tests.rs"]
mod route_tests;

#[tokio::test]
async fn owning_phase_cannot_advance_on_request_or_terminal_exhaustion_without_evidence() {
    let (_directory, policy) = policy().await;
    let connect = Frame::new(1, "connect", Impact::Control, profile());
    TransportPolicy::admit(&policy, connect.request())
        .await
        .unwrap();
    TransportPolicy::terminal(&policy, connect.terminal(&connect_report())).unwrap();
    assert!(policy.state.lock().sequence.phase_ready().is_err());
    assert_eq!(policy.state.lock().sequence.phase().unwrap().number(), 1);
    // A real runner must complete bootstrap and version, then pass actual
    // SessionInfo through finish_phase. No caller-controlled success boolean.
}

#[test]
fn publication_schedule_uses_actual_publication_ordinals_for_every_census_size() {
    let plan = super::super::Plan::generate().unwrap();
    for count in 1..=super::super::MAX_SCHEMAS {
        let mut names = vec![SCHEMA.to_owned()];
        names.extend((1..count).map(|i| format!("Z{i:02}")));
        let census = super::super::SchemaCensus::freeze(&names).unwrap();
        let ledger = plan.expand_physical_ledger(&census).unwrap();
        for phase in plan.phases() {
            let entries: Vec<_> = ledger
                .entries()
                .iter()
                .filter(|e| e.phase() == phase.number())
                .collect();
            let callbacks: Vec<_> = entries
                .iter()
                .flat_map(|entry| {
                    observations(entry, Some(census.names()))
                        .unwrap()
                        .into_iter()
                        .map(|observation| (entry.phase_ordinal(), observation))
                })
                .collect();
            let runtimes: Vec<_> = callbacks
                .iter()
                .filter(|(_, o)| *o == Observation::Runtime)
                .map(|(ordinal, _)| *ordinal)
                .collect();
            let expected = match phase.number() {
                1 | 6 | 7 | 11 | 12 | 28 | 32 => vec![],
                2 => vec![12],
                3 => vec![18],
                4 | 10 | 31 => vec![11, count as u16 + 27],
                8 => vec![15],
                9 => vec![21],
                17 => vec![6],
                29 => vec![11],
                30 => vec![17],
                _ => vec![11],
            };
            assert_eq!(runtimes, expected, "phase{} census{count}", phase.number());
            if matches!(phase.number(), 4 | 10 | 31 | 6 | 11 | 28 | 32) {
                let snapshots: Vec<_> = callbacks
                    .iter()
                    .filter(|(_, o)| {
                        *o == Observation::Relations(MetadataSite::RelationCatalogSnapshot, None)
                    })
                    .collect();
                let offset = if matches!(phase.number(), 4 | 10 | 31) {
                    16
                } else {
                    5
                };
                assert_eq!(snapshots.len(), 1);
                assert_eq!(snapshots[0].0, count as u16 + offset);
            }
        }
    }
}
