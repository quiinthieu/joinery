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
