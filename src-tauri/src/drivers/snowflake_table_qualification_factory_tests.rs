//! Isolated factory acceptance. The actual Node/helper runs only a synthetic
//! SDK; none of these cases qualifies an account or opens private app state.
use super::super::{
    Plan, SCHEMA, SchemaCensus, StatementId,
    envelope::fixture_unconsumed,
    factory::{PreparationFailure, PreparedAttempt},
    journal::FILE_NAME,
    runner::{FixtureControls, Pause},
};
use crate::{
    domain::{ConnectionProfile, Environment, SafeMode, SnowflakeAuthentication, TlsMode},
    snowflake_runtime::studio_fixture::Fixture,
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

fn profile() -> Value {
    json!({
        "account":"synthetic-account", "host":"synthetic-account.snowflakecomputing.com",
        "username":"login@example.test", "role":"QUALIFICATION", "warehouse":"FIXTURE_WH",
        "database":"SANDBOX", "schema":"BILLIE_TESTING"
    })
}

fn output_parent() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

fn physical(directory: &tempfile::TempDir) -> PathBuf {
    directory.path().canonicalize().unwrap()
}

fn prepared(result: Result<PreparedAttempt, PreparationFailure>) -> PreparedAttempt {
    match result {
        Ok(prepared) => prepared,
        Err(failure) => panic!("factory preparation failed: {:?}", failure.error),
    }
}

fn rejected(result: Result<PreparedAttempt, PreparationFailure>) -> PreparationFailure {
    match result {
        Ok(_) => panic!("factory accepted rejected preparation"),
        Err(failure) => failure,
    }
}

fn markers(directory: &tempfile::TempDir) -> usize {
    fs::read_dir(directory.path().join("consumption"))
        .unwrap()
        .count()
}

fn records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("fixture record {} unavailable: {error}", path.display()))
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn queries(fixture: &Fixture) -> Vec<String> {
    records(&fixture.controls.join("queries"))
        .into_iter()
        .map(|record| record.as_str().unwrap().to_owned())
        .collect()
}

fn attempt_path(parent: &Path, plan: &Plan) -> PathBuf {
    parent.join(format!("attempt-{}", plan.run()))
}

#[test]
fn qualification_factory_invalid_approval_does_not_consume() {
    let (artifacts, envelope, store, _) =
        fixture_unconsumed(Plan::generate().unwrap(), profile(), None);
    let output = output_parent();
    let failure = rejected(PreparedAttempt::prepare(
        envelope,
        &store,
        b"{}",
        &physical(&output),
    ));
    assert_eq!(markers(&artifacts), 0);
    assert!(matches!(failure.terminal, Some(Ok(_))));
    let path = failure.path.unwrap();
    assert!(!path.join("workspace/joinery.sqlite").exists());
    assert_eq!(fs::read_dir(path.join("helper-state")).unwrap().count(), 0);
    assert_eq!(records(&path.join("terminal.json")).len(), 1);
}

#[test]
fn qualification_factory_output_collision_does_not_consume_or_replace() {
    let plan = Plan::generate().unwrap();
    let (artifacts, envelope, store, approval) = fixture_unconsumed(plan.clone(), profile(), None);
    let output = output_parent();
    let occupied = attempt_path(&physical(&output), &plan);
    fs::create_dir(&occupied).unwrap();
    let sentinel = occupied.join("preserve");
    fs::write(&sentinel, b"previous output must survive").unwrap();
    let failure = rejected(PreparedAttempt::prepare(
        envelope,
        &store,
        &approval,
        &physical(&output),
    ));
    assert_eq!(markers(&artifacts), 0);
    assert!(failure.terminal.is_none());
    assert_eq!(fs::read(sentinel).unwrap(), b"previous output must survive");
    assert_eq!(fs::read_dir(occupied).unwrap().count(), 1);
}

#[test]
fn qualification_factory_rejects_output_overlapping_reviewed_artifacts() {
    let (artifacts, envelope, store, approval) =
        fixture_unconsumed(Plan::generate().unwrap(), profile(), None);
    let parent = physical(&artifacts).join("runtime");
    let plan = envelope.plan();
    let failure = rejected(PreparedAttempt::prepare(
        envelope, &store, &approval, &parent,
    ));
    assert_eq!(markers(&artifacts), 0);
    assert!(failure.terminal.is_none());
    assert!(!attempt_path(&parent, &plan).exists());
}

#[test]
fn qualification_factory_rejects_output_overlapping_permanent_store() {
    let (artifacts, envelope, store, approval) =
        fixture_unconsumed(Plan::generate().unwrap(), profile(), None);
    let parent = physical(&artifacts).join("consumption");
    let plan = envelope.plan();
    let failure = rejected(PreparedAttempt::prepare(
        envelope, &store, &approval, &parent,
    ));
    assert_eq!(markers(&artifacts), 0);
    assert!(failure.path.is_none());
    assert!(failure.terminal.is_none());
    assert!(!attempt_path(&parent, &plan).exists());
    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_factory_runtime_failure_is_durable_and_cannot_be_reused() {
    let plan = Plan::generate().unwrap();
    let (artifacts, envelope, store, approval) = fixture_unconsumed(plan.clone(), profile(), None);
    let output = output_parent();
    let owner = prepared(PreparedAttempt::prepare(
        envelope,
        &store,
        &approval,
        &physical(&output),
    ));
    // Synthetic input is a valid reviewed inventory, but not a runtime. The
    // production exact-root loader must reject it before opening SQLite/Node.
    assert_eq!(markers(&artifacts), 2);
    let receipt = owner.start().await.unwrap();
    assert!(!receipt.successful());
    assert!(receipt.execution.is_none());
    assert!(receipt.construction_error.is_some());
    assert_eq!(receipt.secret_store_calls(), 0);
    assert!(receipt.terminal.is_ok(), "{:?}", receipt.terminal);
    assert!(!receipt.path.join("workspace/joinery.sqlite").exists());
    assert_eq!(
        fs::read_dir(receipt.path.join("helper-state"))
            .unwrap()
            .count(),
        0
    );
    let terminal = records(&receipt.path.join("terminal.json"));
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0]["result"], receipt.terminal_record);
    assert_eq!(terminal[0]["run"], plan.run());
    assert_eq!(markers(&artifacts), 2);

    // Removing only test-owned output cannot release the permanent run marker.
    fs::remove_dir_all(&receipt.path).unwrap();
    let (_fresh_artifacts, envelope, _unused_store, approval) =
        fixture_unconsumed(plan, profile(), None);
    let failure = rejected(PreparedAttempt::prepare(
        envelope,
        &store,
        &approval,
        &physical(&output),
    ));
    assert_eq!(markers(&artifacts), 2);
    assert!(matches!(failure.terminal, Some(Ok(_))));
    let path = failure.path.unwrap();
    assert!(!path.join("workspace/joinery.sqlite").exists());
    assert_eq!(records(&path.join("terminal.json")).len(), 1);
}

/// Minimal composition around the shared lifecycle SDK. Only the nine frozen
/// statement strings vary; production framing, transport, guard and Studio run.
fn lifecycle_fixture(plan: &Plan) -> Fixture {
    let fixture = Fixture::separated();
    let mut statements = serde_json::Map::new();
    for (name, id) in [
        ("Create", StatementId::Create),
        ("Add", StatementId::Add),
        ("Drop", StatementId::Drop),
        ("RollbackInsert", StatementId::RollbackInsert),
        ("CommittedInsert", StatementId::CommittedInsert),
        ("Update", StatementId::Update),
        ("Delete", StatementId::Delete),
        ("OwnershipRead", StatementId::OwnershipRead),
        ("DataRead", StatementId::DataRead),
    ] {
        statements.insert(name.into(), json!(plan.statement(id).source_sql()));
    }
    fs::write(
        fixture.runtime_root().join("helper/lifecycle-sdk.mjs"),
        include_str!("../../tests/support/snowflake_table_qualification_sdk.mjs"),
    )
    .unwrap();
    let lifecycle_url = serde_json::to_string(
        url::Url::from_file_path(fixture.runtime_root().join("helper/lifecycle-sdk.mjs"))
            .unwrap()
            .as_str(),
    )
    .unwrap();
    let controls_url = serde_json::to_string(
        url::Url::from_directory_path(&fixture.controls)
            .unwrap()
            .as_str(),
    )
    .unwrap();
    let options = json!({"schemaCount":1,"tableName":plan.table(),"statements":statements});
    let wrapper = format!(
        r#"import {{makeSdk}} from {lifecycle_url};
import {{appendFileSync}} from 'node:fs';
const controls=new URL({controls_url});
const append=(name,value)=>appendFileSync(new URL(name,controls),JSON.stringify(value)+'\n');
append('sdk-resolution',{{url:import.meta.url,synthetic:true}});
const sdk=makeSdk({{...{options},log:sql=>append('queries',sql)}});
const create=sdk.createConnection.bind(sdk);
sdk.createConnection=profile=>{{
  append('connections',{{profile,state:process.env.JOINERY_SNOWFLAKE_STATE_DIR}});
  const connection=create(profile);
  const destroy=connection.destroy.bind(connection);
  connection.destroy=callback=>destroy(error=>{{append('destroyed',{{ok:!error}});callback(error);}});
  return connection;
}};
export default sdk;
"#
    );
    fs::write(
        fixture
            .runtime_root()
            .join("helper/node_modules/snowflake-sdk/index.mjs"),
        wrapper,
    )
    .unwrap();
    fixture
}

fn assert_private_profile(path: &Path) {
    let connection = rusqlite::Connection::open_with_flags(
        path.join("workspace/joinery.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement = connection.prepare("SELECT json FROM profiles").unwrap();
    let profiles = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str::<ConnectionProfile>(&row.unwrap()).unwrap())
        .collect::<Vec<_>>();
    let [profile] = profiles.as_slice() else {
        panic!("factory did not save exactly one fresh profile")
    };
    assert_eq!(profile.group_id, "staging");
    assert_eq!(profile.environment, Environment::Staging);
    assert_eq!(profile.safe_mode, SafeMode::Confirm);
    assert_eq!(profile.tls_mode, TlsMode::VerifyFull);
    assert_eq!(profile.port, 443);
    assert_eq!(profile.host, "synthetic-account.snowflakecomputing.com");
    assert_eq!(profile.username, "login@example.test");
    assert!(!profile.remember_password && !profile.store_query_text && !profile.store_drafts);
    assert!(profile.ssh.is_none() && profile.ca_cert_path.is_none());
    let snowflake = profile.snowflake.as_ref().unwrap();
    assert_eq!(
        snowflake.authentication,
        SnowflakeAuthentication::ExternalBrowser
    );
    assert!(snowflake.reviewed_dml && snowflake.reviewed_ddl);
    assert_eq!(snowflake.role, "QUALIFICATION");
    assert_eq!(snowflake.warehouse, "FIXTURE_WH");
    assert_eq!(snowflake.schema, SCHEMA);
}

fn assert_factory_cleanup(fixture: &Fixture, path: &Path, require_logout_callback: bool) {
    assert_eq!(fs::read_dir(path.join("helper-state")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&fixture.state).unwrap().count(), 0);
    let destroyed = fixture.controls.join("destroyed");
    // Revoking the journal also revokes transport admission. Shutdown can then
    // reap the local process without reaching the SDK logout callback. Local
    // settlement and empty state are not evidence of server logout.
    if require_logout_callback || destroyed.exists() {
        assert_eq!(records(&destroyed), [json!({"ok":true})]);
    }
    let connections = records(&fixture.controls.join("connections"));
    assert_eq!(connections.len(), 1, "factory must not reconnect");
    let state = PathBuf::from(connections[0]["state"].as_str().unwrap());
    assert!(state.starts_with(path.join("helper-state")));
    assert!(!state.starts_with(&fixture.state));
    let resolution = records(&fixture.controls.join("sdk-resolution"));
    assert_eq!(resolution.len(), 1);
    assert_eq!(resolution[0]["synthetic"], true);
    assert_eq!(
        resolution[0]["url"],
        url::Url::from_file_path(
            fixture
                .runtime_root()
                .join("helper/node_modules/snowflake-sdk/index.mjs")
        )
        .unwrap()
        .as_str()
    );
    assert_private_profile(path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "optimized serial bound-runtime factory acceptance; --release --ignored --test-threads=1"]
async fn qualification_factory_complete_lifecycle_outlives_dropped_waiter() {
    let plan = Plan::generate().unwrap();
    let fixture = lifecycle_fixture(&plan);
    let (artifacts, envelope, store, approval) = fixture_unconsumed(
        plan.clone(),
        profile(),
        Some(fixture.runtime_root().to_owned()),
    );
    let output = output_parent();
    let path = attempt_path(&physical(&output), &plan);
    let (reached, ready) = tokio::sync::oneshot::channel();
    let resume = Arc::new(tokio::sync::Notify::new());
    let (finished, completed) = tokio::sync::oneshot::channel();
    let owner = prepared(PreparedAttempt::prepare(
        envelope,
        &store,
        &approval,
        &physical(&output),
    ))
    .with_fixture_runtime(fixture.runtime.clone())
    .with_controls(FixtureControls {
        pause: Some(Pause {
            phase: 3,
            reached,
            resume: resume.clone(),
        }),
        ..FixtureControls::default()
    })
    .with_finished(finished)
    .start();
    tokio::time::timeout(Duration::from_secs(300), ready)
        .await
        .unwrap()
        .unwrap();
    drop(owner);
    resume.notify_one();
    tokio::time::timeout(Duration::from_secs(600), completed)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(markers(&artifacts), 2);
    let terminal = records(&path.join("terminal.json"));
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0]["run"], plan.run());
    assert_eq!(terminal[0]["result"]["executionSuccessful"], true);
    assert_eq!(terminal[0]["result"]["secretStoreCalls"], 0);
    let journal = records(&path.join(FILE_NAME));
    assert_eq!(
        journal
            .iter()
            .filter(|v| v["event"]["type"] == "validatedPhase")
            .count(),
        32
    );
    for phase in 1..=32 {
        let completion = journal
            .iter()
            .find(|record| {
                record["event"]["type"] == "operationCompletion"
                    && record["event"]["phase"] == phase
            })
            .unwrap();
        let validation = journal
            .iter()
            .find(|record| {
                record["event"]["type"] == "validatedPhase" && record["event"]["phase"] == phase
            })
            .unwrap();
        assert_eq!(completion["event"]["completion"]["status"], "returned");
        assert_eq!(
            completion["event"]["completion"]["original"],
            validation["event"]["evidence"]
        );
    }
    assert_eq!(
        journal
            .iter()
            .filter(|v| v["event"]["type"] == "operationCompletion")
            .count(),
        32
    );
    let create = journal
        .iter()
        .find(|v| v["event"]["type"] == "operationCompletion" && v["event"]["phase"] == 3)
        .unwrap();
    assert_eq!(create["event"]["completion"]["status"], "returned");
    assert_eq!(
        create["event"]["completion"]["original"]["value"]["statementOutcome"],
        "succeeded"
    );
    let ledger = plan
        .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
        .unwrap();
    assert_eq!(
        queries(&fixture),
        ledger
            .entries()
            .iter()
            .map(|entry| entry.sql().to_owned())
            .collect::<Vec<_>>()
    );
    assert_factory_cleanup(&fixture, &path, true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_factory_actual_create_ack_survives_journal_and_terminal_failure() {
    let plan = Plan::generate().unwrap();
    let fixture = lifecycle_fixture(&plan);
    let (artifacts, envelope, store, approval) = fixture_unconsumed(
        plan.clone(),
        profile(),
        Some(fixture.runtime_root().to_owned()),
    );
    let output = output_parent();
    let path = attempt_path(&physical(&output), &plan);
    let receipt = prepared(PreparedAttempt::prepare(
        envelope,
        &store,
        &approval,
        &physical(&output),
    ))
    .with_fixture_runtime(fixture.runtime.clone())
    .with_controls(FixtureControls {
        break_journal: Some((3, path.join(FILE_NAME))),
        ..FixtureControls::default()
    })
    .with_terminal_failure()
    .start()
    .await
    .unwrap();
    assert!(!receipt.successful());
    assert_eq!(receipt.secret_store_calls(), 0);
    assert!(receipt.construction_error.is_none());
    assert!(receipt.terminal.is_err());
    let execution = receipt.execution.as_ref().unwrap();
    assert_eq!(execution.failure.as_ref().map(|failure| failure.0), Some(3));
    assert_eq!(execution.outputs.len(), 3);
    assert!(execution.original_call_failure.is_none());
    assert!(execution.completion_record_error.is_some());
    assert!(execution.persisted.is_err());
    assert!(execution.shutdown.is_ok(), "{:?}", execution.shutdown);
    assert!(execution.settled);
    let original = execution.outputs[2].evidence().record().unwrap();
    assert_eq!(receipt.terminal_record["execution"]["outputs"][2], original);
    assert_eq!(original["value"]["statementOutcome"], "succeeded");
    assert!(original["value"]["error"].is_null());
    assert_eq!(fs::metadata(path.join("terminal.json")).unwrap().len(), 0);
    assert_eq!(markers(&artifacts), 2);
    let queries = queries(&fixture);
    assert_eq!(
        queries
            .iter()
            .filter(|sql| sql.starts_with("CREATE TABLE "))
            .count(),
        1
    );
    assert!(
        !queries
            .iter()
            .any(|sql| sql.starts_with("ALTER TABLE ") || sql.starts_with("DROP TABLE "))
    );
    let ledger = plan
        .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
        .unwrap();
    assert_eq!(
        queries,
        ledger
            .entries()
            .iter()
            .take_while(|entry| entry.phase() <= 3)
            .map(|entry| entry.sql().to_owned())
            .collect::<Vec<_>>()
    );
    let journal = records(&path.join(FILE_NAME));
    assert!(
        journal
            .iter()
            .filter(|record| record["event"]["type"] == "validatedPhase")
            .all(|record| record["event"]["phase"].as_u64().unwrap() < 3)
    );
    assert_factory_cleanup(&fixture, &path, false);
}
