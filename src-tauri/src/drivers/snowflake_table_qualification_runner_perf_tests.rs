//! Optimized, serial, offline acceptance with the complete copied runtime and
//! genuine source/executable/compiler inventories. Never imports the account SDK.
use super::*;
use crate::drivers::snowflake_table_qualification::envelope::fixture_consumed_real_content;

fn emit_performance(value: Value) {
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(bytes.len() <= 128 * 1024, "bounded performance diagnostics");
    println!("{}", String::from_utf8(bytes).unwrap());
}

async fn copied_runtime_route(schemas: usize) {
    assert_eq!(
        (
            std::env::consts::OS,
            std::env::consts::ARCH,
            cfg!(debug_assertions)
        ),
        ("macos", "aarch64", false),
        "run this ignored acceptance in --release with --test-threads=1"
    );
    let fixture = tokio::task::spawn_blocking(Fixture::copied_packaged)
        .await
        .unwrap();
    let mut harness = OfflineHarness::with_fixture(schemas, Value::Null, fixture).await;
    // SDK package routing and all immutable fixture source changes are complete
    // before capture. This only resolves the module; it never imports the SDK.
    harness.fixture.assert_synthetic_resolution();
    let runtime_root = harness.fixture.runtime_root().to_owned();
    let expected_root = runtime_root.clone();
    let plan = harness.plan.clone();
    let profile = harness.policy.frozen_profile();
    let (artifacts, consumed, evidence) = tokio::task::spawn_blocking(move || {
        fixture_consumed_real_content(plan, profile, runtime_root)
    })
    .await
    .unwrap();
    assert_eq!(consumed.fixture_runtime_root(), expected_root);
    assert_eq!(harness.fixture.runtime_root(), expected_root);
    for mutable in [
        harness.fixture.controls.clone(),
        harness.fixture.state.clone(),
        harness.fixture.directory.path().join("workspace"),
        harness.journal.path().to_owned(),
    ] {
        assert!(!mutable.starts_with(&expected_root));
    }
    let runner = Runner::consumed(
        harness.studio.clone(),
        harness.policy.clone(),
        consumed,
        harness.attempt.clone(),
    )
    .unwrap();
    let guard = runner.fixture_guard().expect("genuine consumed Guard");
    harness.transport = runner.transport();
    harness.runner = Some(runner);
    harness._guard_artifacts = Some(artifacts);
    let run = harness.run();
    tokio::pin!(run);
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.tick().await;
    let mut progress = 0;
    let receipt = loop {
        tokio::select! {
            receipt = &mut run => break receipt,
            _ = interval.tick(), if progress < 20 => {
                progress += 1;
                let timing = guard.fixture_measurements();
                emit_performance(json!({"type":"consumedRuntimeProgress","schemas":schemas,
                    "admissions":timing.admissions,"successfulAdmissions":timing.successful_admissions,
                    "failedAdmissions":timing.failed_admissions,"admissionMaxMs":timing.admission_max.as_millis(),
                    "consumedElapsedMs":timing.elapsed_since_consumption.as_millis()}));
            }
        }
    };
    let timing = guard.fixture_measurements();
    let report = receipt.harness.transport.report();
    // Always emit bounded diagnostics before assertions, including on failure.
    emit_performance(
        json!({"type":"consumedRuntimeAcceptance","schemas":schemas,"inputCapture":evidence,
        "runtimeExecuted":true,"syntheticSdk":true,"accountAuthenticated":false,"optimized":true,
        "sameRuntimeRoot":receipt.harness.fixture.runtime_root()==expected_root,
        "outputs":receipt.outputs.len(),"sqlCount":receipt.queries.len(),"failure":receipt.failure,
        "originalCallError":receipt.original_call_failure.as_ref().map(|v|&v.error),
        "completionRecordError":receipt.completion_record_error,"shutdown":receipt.shutdown,
        "transportSettled":receipt.settled,"persisted":receipt.persisted,
        "guardRevalidation":receipt.guard_revalidation,"guardSettled":receipt.guard_settled,
        "guardRecorded":receipt.guard_recorded,"admissions":timing.admissions,
        "successfulAdmissions":timing.successful_admissions,"failedAdmissions":timing.failed_admissions,
        "admissionTotalMs":timing.admission_total.as_millis(),"admissionMaxMs":timing.admission_max.as_millis(),
        "consumedElapsedMs":timing.elapsed_since_consumption.as_millis(),
        "transport":{"admittedWrites":report.admitted_writes,"completeWrites":report.complete_writes,
            "stdoutBytes":report.stdout_bytes,"payloadBytes":report.payload_bytes,"failed":report.failed}}),
    );
    assert!(
        receipt.failure.is_none(),
        "original failure retained in diagnostic above"
    );
    assert!(receipt.original_call_failure.is_none());
    assert!(receipt.completion_record_error.is_none());
    assert_local_settlement(&receipt);
    assert!(receipt.persisted.is_ok());
    assert!(matches!(receipt.guard_revalidation, Some(Ok(()))));
    assert_eq!(receipt.guard_settled, Some(true));
    assert!(matches!(receipt.guard_recorded, Some(Ok(()))));
    assert!(!report.failed);
    assert!(timing.admissions > 0);
    assert_eq!(timing.successful_admissions, timing.admissions);
    assert_eq!(timing.failed_admissions, 0);
    assert!(timing.admission_max < Duration::from_secs(3));
    assert!(timing.elapsed_since_consumption < Duration::from_secs(900));
    assert_eq!(receipt.outputs.len(), 32);
    let names = std::iter::once(SCHEMA.to_owned())
        .chain((2..=schemas).map(|n| format!("S{n:02}")))
        .collect::<Vec<_>>();
    let ledger = receipt
        .harness
        .plan
        .expand_physical_ledger(&SchemaCensus::freeze(&names).unwrap())
        .unwrap();
    assert_eq!(
        receipt.queries,
        ledger
            .entries()
            .iter()
            .map(|entry| entry.sql().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(receipt.queries.len(), if schemas == 1 { 393 } else { 610 });
    assert_eq!(
        receipt
            .queries
            .iter()
            .filter(|sql| sql.starts_with("CREATE TABLE "))
            .count(),
        1
    );
    assert_eq!(
        receipt
            .queries
            .iter()
            .filter(|sql| sql.starts_with("ALTER TABLE "))
            .count(),
        1
    );
    assert_eq!(
        receipt
            .queries
            .iter()
            .filter(|sql| sql.starts_with("DROP TABLE "))
            .count(),
        1
    );
    assert_eq!(
        receipt
            .queries
            .iter()
            .filter(|sql| sql.starts_with("INSERT INTO "))
            .count(),
        2
    );
    assert_eq!(
        receipt
            .queries
            .iter()
            .filter(|sql| sql.starts_with("UPDATE "))
            .count(),
        1
    );
    assert_eq!(
        receipt
            .queries
            .iter()
            .filter(|sql| sql.starts_with("DELETE FROM "))
            .count(),
        1
    );
    let validations = receipt
        .journal
        .iter()
        .filter(|row| row["event"]["type"] == "validatedPhase")
        .collect::<Vec<_>>();
    let completions = receipt
        .journal
        .iter()
        .filter(|row| row["event"]["type"] == "operationCompletion")
        .collect::<Vec<_>>();
    assert_eq!(validations.len(), 32);
    assert_eq!(completions.len(), 32);
    for (index, output) in receipt.outputs.iter().enumerate() {
        let phase = json!(index + 1);
        let original = output.evidence().record().unwrap();
        let completion = completions
            .iter()
            .find(|row| row["event"]["phase"] == phase)
            .unwrap();
        let validation = validations
            .iter()
            .find(|row| row["event"]["phase"] == phase)
            .unwrap();
        assert_eq!(completion["event"]["completion"]["status"], "returned");
        assert_eq!(completion["event"]["completion"]["original"], original);
        assert_eq!(validation["event"]["evidence"], original);
        assert!(
            completion["sequence"].as_u64().unwrap() < validation["sequence"].as_u64().unwrap()
        );
    }
    let finals = receipt
        .journal
        .iter()
        .filter(|row| row["event"]["type"] == "executionEnvelopeFinal")
        .collect::<Vec<_>>();
    assert_eq!(finals.len(), 1);
    assert_eq!(
        finals[0]["event"],
        json!({"type":"executionEnvelopeFinal","revalidation":{"ok":true},"scannerSettled":true})
    );
    assert!(
        finals[0]["sequence"].as_u64().unwrap()
            > validations.last().unwrap()["sequence"].as_u64().unwrap()
    );
    let Output::Catalog(catalog) = receipt.outputs.last().unwrap() else {
        panic!("final original catalog missing")
    };
    assert!(
        catalog
            .relations
            .iter()
            .all(|relation| relation.table.name != receipt.harness.plan.table())
    );
    let resolution =
        fs::read_to_string(receipt.harness.fixture.controls.join("sdk-resolution")).unwrap();
    let resolutions = resolution
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let sdk_url =
        url::Url::from_file_path(expected_root.join("helper/node_modules/snowflake-sdk/index.mjs"))
            .unwrap();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(
        resolutions[0],
        json!({"url":sdk_url.as_str(),"synthetic":true})
    );
    assert!(!expected_root.join("helper/queries").exists());
    assert!(!expected_root.join("helper/sdk-resolution").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "optimized serial full-runtime consumed acceptance; --release --ignored --test-threads=1 --nocapture"]
async fn qualification_studio_runner_consumed_full_runtime_s1() {
    copied_runtime_route(1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "optimized serial full-runtime consumed acceptance; --release --ignored --test-threads=1 --nocapture"]
async fn qualification_studio_runner_consumed_full_runtime_s32() {
    copied_runtime_route(32).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_fixture_separates_runtime_and_mutable_outputs() {
    let fixture = Fixture::separated();
    let runtime = fixture.runtime_root().to_owned();
    let mut harness = OfflineHarness::with_fixture(1, Value::Null, fixture).await;
    harness.fixture.assert_synthetic_resolution();
    // Stop locally after the successful real startup without adding SQL.
    harness.break_journal_after = Some(1);
    let sdk_source = fs::read(runtime.join("helper/node_modules/snowflake-sdk/index.mjs")).unwrap();
    let receipt = harness.run().await;
    assert_local_settlement(&receipt);
    assert_eq!(receipt.outputs.len(), 1);
    assert_eq!(receipt.failure.as_ref().map(|failure| failure.0), Some(1));
    assert_eq!(receipt.queries.len(), 2);
    assert!(receipt.harness.fixture.controls.join("queries").is_file());
    assert!(
        receipt
            .harness
            .fixture
            .controls
            .join("sdk-resolution")
            .is_file()
    );
    assert!(!receipt.harness.fixture.controls.starts_with(&runtime));
    assert!(!receipt.harness.fixture.state.starts_with(&runtime));
    assert!(!runtime.join("helper/queries").exists());
    assert!(!runtime.join("helper/sdk-resolution").exists());
    assert_eq!(
        fs::read(runtime.join("helper/node_modules/snowflake-sdk/index.mjs")).unwrap(),
        sdk_source
    );
}
