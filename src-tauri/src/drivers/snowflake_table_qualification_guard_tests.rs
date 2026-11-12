use super::*;
use crate::{
    domain::SnowflakeDdlKind,
    drivers::{
        snowflake_process::{
            Certainty, DdlAcknowledgement, Impact, Phase, Report, Retirement, StatementIdentity,
        },
        snowflake_qualification_transport::LocalTerminalOrigin,
        snowflake_table_qualification::{envelope, journal},
    },
};
use parking_lot::Condvar;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use tokio::sync::watch;

fn profile() -> Value {
    json!({"account":"synthetic-account", "host":"synthetic-account.snowflakecomputing.com",
        "username":"login@example.test", "role":"QUALIFICATION", "warehouse":"FIXTURE_WH",
        "database":"SANDBOX", "schema":"BILLIE_TESTING"})
}

struct Fixture {
    _artifacts: tempfile::TempDir,
    journal: tempfile::TempDir,
    drift: PathBuf,
    policy: Arc<Policy>,
    guard: Arc<Guard>,
}

impl Fixture {
    async fn new() -> Self {
        let plan = Plan::generate().unwrap();
        let (artifacts, consumed, drift) = envelope::fixture_consumed(plan.clone(), profile());
        let journal = tempfile::tempdir().unwrap();
        fs::set_permissions(journal.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let policy = Arc::new(
            Policy::create_with_plan(journal.path().to_owned(), profile(), plan)
                .await
                .unwrap(),
        );
        let guard = Guard::new(consumed, policy.clone()).unwrap();
        Self {
            _artifacts: artifacts,
            journal,
            drift,
            policy,
            guard,
        }
    }

    fn records(&self) -> Vec<Value> {
        records(self.journal.path())
    }

    fn pause(&mut self) -> (Arc<ScanPause>, ReleaseOnDrop) {
        let pause = Arc::new(ScanPause::new());
        let guard = Arc::get_mut(&mut self.guard).unwrap();
        guard.limit = Duration::from_secs(1);
        guard.pause = Some(pause.clone());
        (pause.clone(), ReleaseOnDrop(pause))
    }

    fn assert_revoked(&self, original: &AppError) {
        for result in [
            self.guard.check(),
            self.guard.shared.consumed.check(),
            TransportPolicy::check(&*self.policy),
        ] {
            let error = result.unwrap_err();
            assert_eq!(error.code, original.code);
            assert_eq!(error.message, original.message);
        }
    }
}

fn records(directory: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(directory.join(journal::FILE_NAME))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

struct Connect {
    payload: Value,
    bytes: Vec<u8>,
}
impl Connect {
    fn new() -> Self {
        let payload = json!({"v":1,"generation":"guard-generation","id":"1",
            "op":"connect","payload":profile()});
        let bytes = serde_json::to_vec(&payload).unwrap();
        Self { payload, bytes }
    }

    fn request(&self) -> PhysicalRequest<'_> {
        PhysicalRequest {
            generation: "guard-generation",
            operation_id: "guard-connect",
            ticket: 0,
            operation: "connect",
            impact: Impact::Control,
            target: None,
            payload: &self.payload,
            wire_payload: &self.bytes,
        }
    }
}

/// Closed test pause only: no production callback or configurable scanner.
pub(super) struct ScanPause {
    released: Mutex<bool>,
    changed: Condvar,
    starts: watch::Sender<usize>,
}
impl ScanPause {
    fn new() -> Self {
        Self {
            released: Mutex::new(false),
            changed: Condvar::new(),
            starts: watch::channel(0).0,
        }
    }

    pub(super) fn wait(&self) {
        self.starts.send_modify(|starts| *starts += 1);
        let mut released = self.released.lock();
        while !*released {
            self.changed.wait(&mut released);
        }
    }

    async fn entered(&self) {
        let mut starts = self.starts.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            while *starts.borrow_and_update() == 0 {
                starts.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }

    fn release(&self) {
        *self.released.lock() = true;
        self.changed.notify_all();
    }
}

struct ReleaseOnDrop(Arc<ScanPause>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test]
async fn guard_constructor_rejects_different_closed_plan_or_frozen_profile_before_admission() {
    for different_plan in [true, false] {
        let plan = Plan::generate().unwrap();
        let (_artifacts, consumed, _) = envelope::fixture_consumed(plan.clone(), profile());
        let mut policy_profile = profile();
        if !different_plan {
            policy_profile["role"] = json!("OTHER_ROLE");
        }
        let journal = tempfile::tempdir().unwrap();
        fs::set_permissions(journal.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let policy = Arc::new(
            Policy::create_with_plan(
                journal.path().to_owned(),
                policy_profile,
                if different_plan {
                    Plan::generate().unwrap()
                } else {
                    plan
                },
            )
            .await
            .unwrap(),
        );
        assert!(Guard::new(consumed, policy.clone()).is_err());
        assert!(TransportPolicy::check(&*policy).is_err());
        assert_eq!(
            records(journal.path()).len(),
            1,
            "header only; no admission"
        );
    }
}

#[tokio::test]
async fn guard_memory_checks_do_not_scan_and_full_drift_failure_is_sticky() {
    let fixture = Fixture::new().await;
    let original = fs::read(&fixture.drift).unwrap();
    fs::write(&fixture.drift, b"runtime B").unwrap();
    fixture.guard.check().unwrap();
    let failure = fixture.guard.revalidate().await.unwrap_err();
    fs::write(&fixture.drift, original).unwrap();
    fixture.assert_revoked(&failure);
    assert!(fixture.guard.revalidate().await.is_err());
    assert!(fixture.guard.admit(Connect::new().request()).await.is_err());
    assert!(fixture.guard.wait_for_settlement().await);
    assert_eq!(fixture.records().len(), 1);
}

#[tokio::test]
async fn guard_full_admission_uses_same_policy_and_retains_plan() {
    let fixture = Fixture::new().await;
    let expected = fixture.policy.plan().canonical_json().unwrap();
    fixture.guard.revalidate().await.unwrap();
    fixture.guard.admit(Connect::new().request()).await.unwrap();
    assert_eq!(fixture.guard.plan().canonical_json().unwrap(), expected);
    let records = fixture.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1]["event"]["wireId"], 1);
    assert_eq!(records[1]["event"]["ticket"], 0);
    assert!(fixture.guard.wait_for_settlement().await);
    let measurements = fixture.guard.fixture_measurements();
    assert_eq!(measurements.admissions, 1);
    assert_eq!(measurements.successful_admissions, 1);
    assert_eq!(measurements.failed_admissions, 0);
    assert_eq!(measurements.admission_total, measurements.admission_max);
    assert!(measurements.elapsed_since_consumption >= measurements.admission_total);
}

#[tokio::test]
async fn guard_timeout_revokes_and_retains_single_scanner_until_actual_completion() {
    let mut fixture = Fixture::new().await;
    let (pause, _release) = fixture.pause();
    Arc::get_mut(&mut fixture.guard).unwrap().limit = Duration::from_millis(80);
    let checking = {
        let guard = fixture.guard.clone();
        tokio::spawn(async move { guard.revalidate().await })
    };
    pause.entered().await;
    let failure = checking.await.unwrap().unwrap_err();
    assert_eq!(failure.code, ErrorCode::PolicyBlocked);
    fixture.assert_revoked(&failure);
    assert!(!fixture.guard.wait_for_settlement().await);
    assert!(fixture.guard.revalidate().await.is_err());
    assert_eq!(*pause.starts.borrow(), 1);
    pause.release();
    assert!(fixture.guard.wait_for_settlement().await);
    fixture.assert_revoked(&failure);
    assert_eq!(fixture.records().len(), 1);
}

async fn dropped_worker(admission: bool) {
    let mut fixture = Fixture::new().await;
    let (pause, _release) = fixture.pause();
    let checking = {
        let guard = fixture.guard.clone();
        tokio::spawn(async move {
            if admission {
                guard.admit(Connect::new().request()).await
            } else {
                guard.revalidate().await
            }
        })
    };
    pause.entered().await;
    assert_eq!(fixture.guard.fixture_measurements().admissions, 0);
    checking.abort();
    assert!(checking.await.unwrap_err().is_cancelled());
    let measurements = fixture.guard.fixture_measurements();
    assert_eq!(measurements.admissions, usize::from(admission));
    assert_eq!(measurements.successful_admissions, 0);
    assert_eq!(measurements.failed_admissions, usize::from(admission));
    let failure = fixture.guard.check().unwrap_err();
    assert_eq!(failure.code, ErrorCode::Cancelled);
    fixture.assert_revoked(&failure);
    assert!(!fixture.guard.wait_for_settlement().await);
    assert!(fixture.guard.admit(Connect::new().request()).await.is_err());
    assert_eq!(*pause.starts.borrow(), 1);
    pause.release();
    assert!(fixture.guard.wait_for_settlement().await);
    fixture.assert_revoked(&failure);
    assert_eq!(fixture.records().len(), 1, "worker never admits by itself");
}

#[tokio::test]
async fn guard_dropped_scan_waiter_cannot_release_or_revive_worker() {
    dropped_worker(false).await;
}

#[tokio::test]
async fn guard_dropped_admission_cannot_admit_after_scanner_finishes() {
    dropped_worker(true).await;
}

#[tokio::test]
async fn guard_unpolled_dropped_check_is_already_revoked() {
    for admission in [false, true] {
        let fixture = Fixture::new().await;
        if admission {
            let connect = Connect::new();
            let pending = fixture.guard.admit(connect.request());
            assert_eq!(fixture.guard.fixture_measurements().admissions, 0);
            drop(pending);
        } else {
            drop(fixture.guard.revalidate());
        }
        let failure = fixture.guard.check().unwrap_err();
        assert_eq!(failure.code, ErrorCode::Cancelled);
        let measurements = fixture.guard.fixture_measurements();
        assert_eq!(measurements.admissions, usize::from(admission));
        assert_eq!(measurements.successful_admissions, 0);
        assert_eq!(measurements.failed_admissions, usize::from(admission));
        fixture.assert_revoked(&failure);
        assert!(fixture.guard.wait_for_settlement().await);
        assert_eq!(fixture.records().len(), 1);
    }
}

#[tokio::test]
async fn guard_failed_attempt_still_durably_forwards_original_terminal_and_local_ack() {
    let fixture = Fixture::new().await;
    let first = AppError::new(ErrorCode::PolicyBlocked, "synthetic artifact revocation");
    TransportPolicy::failed(&*fixture.guard, first.clone());
    let statement = StatementIdentity {
        sdk_request_id: "original-sdk-request".into(),
        query_id: Some("original-query".into()),
    };
    // Deliberately unmatched after revocation: forwarding must retain even an
    // original that the closed sequencer rejects, rather than hide its ACK.
    let report = Report {
        ok: true,
        certainty: Certainty::StatementSucceeded,
        phase: Phase::Acknowledged,
        result: Some(json!({"status":"original successful mutation"})),
        error_code: None,
        retirement: Some(Retirement::Requested),
        statements: vec![statement.clone()],
        use_acknowledged: None,
        boundary_acknowledged: None,
        dml_acknowledged: None,
        statement_acknowledged: None,
        ddl_acknowledged: Some(DdlAcknowledgement {
            kind: SnowflakeDdlKind::CreateTable,
            statement,
        }),
    };
    assert!(
        fixture
            .guard
            .terminal(PhysicalTerminal {
                generation: "original-generation",
                operation_id: "original-mutation",
                ticket: 17,
                wire_id: "4",
                operation: "ddl",
                impact: Impact::Mutating,
                report: &report,
            })
            .is_err()
    );
    let payload = json!({"kind":"createTable"});
    assert!(
        fixture
            .guard
            .local_terminal(LocalTerminal {
                generation: "original-generation",
                operation_id: "original-local",
                ticket: 18,
                wire_id: Some("5"),
                candidate_wire_id: Some("5"),
                operation: "ddl",
                impact: Impact::Mutating,
                target: None,
                payload: &payload,
                origin: LocalTerminalOrigin::Retirement,
                report: &report,
            })
            .is_err()
    );
    assert_eq!(fixture.records().len(), 1, "callbacks perform no IO");
    assert!(fixture.policy.persist_final_evidence().await.is_err());
    let records = fixture.records();
    assert_eq!(records.len(), 3);
    let expected = json!({"ok":true,"certainty":"StatementSucceeded","phase":"Acknowledged",
        "result":{"status":"original successful mutation"},"errorCode":null,"retirement":"Requested",
        "statements":[{"sdkRequestId":"original-sdk-request","queryId":"original-query"}],
        "useAcknowledged":null,"boundaryAcknowledged":null,"dmlAcknowledged":null,
        "ddlAcknowledged":{"kind":"createTable","statement":{
            "sdkRequestId":"original-sdk-request","queryId":"original-query"}}});
    assert_eq!(records[1]["event"]["report"], expected);
    assert_eq!(records[2]["event"]["report"], expected);
    assert_eq!(records[2]["event"]["wireId"], "5");
    assert_eq!(records[2]["event"]["candidateWireId"], "5");
    assert_eq!(records[2]["event"]["origin"]["kind"], "retirement");
    fixture.assert_revoked(&first);
    assert!(
        !records
            .iter()
            .any(|record| record["event"]["type"] == "validatedPhase")
    );
}
