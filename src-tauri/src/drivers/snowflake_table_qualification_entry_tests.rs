//! Actual Unix signals around the shared entry; only temporary synthetic inputs
//! and the real helper with a synthetic SDK are used. No native account resolver
//! or stored credential provider is invoked by this cfg(test)-only seam.
use super::super::{
    Plan, SCHEMA, SchemaCensus, StatementId, envelope::fixture_unconsumed,
    factory::fixture_lifecycle, journal::FILE_NAME, ledger::SqlKind,
};
use super::*;
use crate::domain::SnowflakeDdlKind;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

fn private_directory(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn private_json(path: &Path, value: &Value) {
    let pending = path.with_extension("pending");
    private_bytes(&pending, &serde_json::to_vec(value).unwrap());
    fs::rename(pending, path).unwrap();
}

fn private_bytes(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn path(value: &Value, key: &str) -> PathBuf {
    PathBuf::from(value[key].as_str().unwrap())
}

fn marker_directory(home: &Path) -> PathBuf {
    home.join(
        "Library/Application Support/Joinery/private-qualification/table-lifecycle-consumption-v1",
    )
}

struct Process {
    child: Option<Child>,
    group: libc::pid_t,
    root: PathBuf,
}

impl Process {
    fn spawn(root: &Path, mode: &str) -> Self {
        let scope = module_path!().split_once("::").unwrap().1;
        let exact = format!("{scope}::entry_subprocess_child");
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &exact, "--ignored", "--nocapture"])
            .env("JOINERY_ENTRY_TEST_ROOT", root)
            .env("JOINERY_ENTRY_TEST_MODE", mode)
            // All child TempDirs remain below this parent-owned disposable tree.
            .env("TMPDIR", root)
            .env("HOME", root.join(format!("caller-home-{mode}")))
            .env("XDG_DATA_HOME", root.join(format!("caller-data-{mode}")))
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let group = child.id() as libc::pid_t;
        Self {
            child: Some(child),
            group,
            root: root.to_owned(),
        }
    }

    fn wait_for(&mut self, wanted: &Path) {
        let deadline = Instant::now() + Duration::from_secs(300);
        while !wanted.exists() {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                self.kill_group();
                let output = self.child.take().unwrap().wait_with_output().unwrap();
                panic!(
                    "entry child exited {status} before {}\n{}\n{}\nreceipts: {:?}",
                    wanted.display(),
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                    fs::read_dir(&self.root)
                        .unwrap()
                        .map(|v| v.unwrap().path())
                        .collect::<Vec<_>>()
                );
            }
            assert!(
                Instant::now() < deadline,
                "entry child did not reach {}",
                wanted.display()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn signal(&mut self, signal: libc::c_int) {
        let pid = self.child.as_ref().unwrap().id() as libc::pid_t;
        // SAFETY: this PID belongs to the live child this test just spawned.
        assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
    }

    fn signal_group(&self, signal: libc::c_int) {
        // SAFETY: process_group(0) created a group containing only this child
        // and its test-owned helper. This models terminal foreground Ctrl-C.
        assert_eq!(unsafe { libc::kill(-self.group, signal) }, 0);
    }

    fn finish(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(600);
        while self.child.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "entry child did not settle");
            thread::sleep(Duration::from_millis(10));
        }
        self.kill_group();
        self.child.take().unwrap().wait_with_output().unwrap()
    }

    fn kill_group(&self) {
        // SAFETY: process_group(0) created this test-owned group. An exited
        // child may still have an owned helper in it; never address other groups.
        unsafe { libc::kill(-self.group, libc::SIGKILL) };
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            self.kill_group();
            let _ = child.wait();
        }
    }
}

fn fixture_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn assert_exit(output: &Output, successful: bool) {
    assert_eq!(
        output.status.code(),
        Some(if successful { 0 } else { 1 }),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_terminal(receipt: &Value) -> Value {
    let summary = &receipt["value"];
    let terminal_path = path(summary, "outputPath").join("terminal.json");
    let bytes = fs::read(&terminal_path).unwrap();
    assert_eq!(
        summary["terminalReceipt"]["sha256"],
        format!("{:x}", Sha256::digest(&bytes))
    );
    assert_eq!(summary["terminalReceipt"]["bytes"], bytes.len());
    let terminal = records(&terminal_path);
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0]["result"]["secretStoreCalls"], 0);
    terminal[0]["result"].clone()
}

// Process::exit deliberately preserves child fixture inputs until the parent's
// TempDir cleanup, allowing another fresh process to restore the original bytes.
// The real entry has already awaited execution, shutdown and durable receipts.
#[test]
#[ignore = "internal subprocess entry"]
fn entry_subprocess_child() {
    let Some(root) = std::env::var_os("JOINERY_ENTRY_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var("JOINERY_ENTRY_TEST_MODE").unwrap();
    let input_file = root.join("input.json");
    let replay = mode == "replay";
    let plan = Plan::generate().unwrap();
    let fixture = fixture_lifecycle(&plan, mode == "pending-create");
    assert!(fixture.directory.path().starts_with(&root));
    let home = root.join("synthetic-home");
    let output = root.join(format!("output-{mode}"));
    private_directory(&output);
    let input = if replay {
        read_json(&input_file)
    } else {
        private_directory(&home);
        let (artifacts, envelope, _unused_store, approval) = fixture_unconsumed(
            plan.clone(),
            json!({"account":"synthetic-account","host":"synthetic-account.snowflakecomputing.com",
                "username":"entry@example.test","role":"QUALIFICATION","warehouse":"FIXTURE_WH",
                "database":"SANDBOX","schema":"BILLIE_TESTING"}),
            Some(fixture.runtime_root().to_owned()),
        );
        let artifacts_root = artifacts.path().canonicalize().unwrap();
        assert!(artifacts_root.starts_with(&root));
        let envelope_file = root.join("original-envelope.json");
        let approval_file = root.join("approval.json");
        private_bytes(&envelope_file, &envelope.canonical_json().unwrap());
        private_bytes(
            &approval_file,
            if mode == "invalid-approval" {
                b"{}"
            } else {
                &approval
            },
        );
        let arguments: Vec<OsString> = [
            OsString::from("execute"),
            OsString::from("--repository"),
            artifacts_root.join("repository").into_os_string(),
            OsString::from("--runtime"),
            fixture.runtime_root().as_os_str().to_owned(),
            OsString::from("--compiler"),
            artifacts_root.join("rustc").into_os_string(),
            OsString::from("--profile"),
            artifacts_root.join("profile.json").into_os_string(),
            OsString::from("--envelope"),
            envelope_file.into_os_string(),
            OsString::from("--sha256"),
            envelope.sha256().unwrap().into(),
            OsString::from("--approval"),
            approval_file.into_os_string(),
            OsString::from("--output-parent"),
            output.as_os_str().to_owned(),
        ]
        .into();
        let ledger = plan
            .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
            .unwrap();
        let creates: Vec<_> = ledger
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.kind() == SqlKind::Ddl(SnowflakeDdlKind::CreateTable))
            .collect();
        let [(create_index, create)] = creates.as_slice() else {
            panic!("closed physical ledger must contain exactly one CREATE TABLE");
        };
        assert_eq!(create.phase(), 3);
        assert_eq!(
            create.sql(),
            plan.statement(StatementId::Create).source_sql()
        );
        // Revocation at the held CREATE callback must stop before every later
        // observer, including observers that belong to this same logical phase.
        let create_prefix: Vec<_> = ledger.entries()[..=*create_index]
            .iter()
            .map(|entry| entry.sql())
            .collect();
        let value = json!({"arguments":arguments.iter().map(|v|v.to_str().unwrap()).collect::<Vec<_>>(),
            "run":plan.run(),"executionSha256":envelope.sha256().unwrap(),
            "controls":fixture.controls,"nativeHome":home,"output":output,
            "expectedSql":ledger.entries().iter().map(|v|v.sql()).collect::<Vec<_>>(),
            "createPrefix":create_prefix});
        private_json(&input_file, &value);
        if mode == "tamper" {
            fs::write(
                artifacts_root.join("repository/src-tauri/src/included-source"),
                b"changed after original capture",
            )
            .unwrap();
        }
        // All of this fixture's files are below the parent-owned test root.
        let _ = artifacts.keep();
        value
    };
    let mut arguments: Vec<OsString> = input["arguments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().into())
        .collect();
    if replay {
        *arguments.last_mut().unwrap() = output.into_os_string();
    }
    let result = run_fixture_command(
        arguments,
        FixtureExecution {
            native_home: home,
            runtime: fixture.runtime.clone(),
            controls: None,
            before_prepare: (mode == "early").then(|| FixturePause {
                ready: root.join("prepare-ready"),
                release: root.join("prepare-release"),
            }),
            cancellation_observed: Some(root.join("cancellation-observed")),
        },
    );
    let (record, successful) = match result {
        Ok(output) => (
            json!({"value":output.value,"childControls":fixture.controls}),
            output.successful,
        ),
        Err(failure) => (
            json!({"error":failure,"childControls":fixture.controls}),
            false,
        ),
    };
    private_json(&root.join(format!("result-{mode}.json")), &record);
    std::process::exit(if successful { 0 } else { 1 });
}

#[test]
#[ignore = "optimized serial shared-entry signals; --release --ignored --test-threads=1"]
fn entry_os_signals_wait_for_original_create_ack_and_stop_sql_suffix() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let temporary = fixture_root();
        let root = temporary.path().canonicalize().unwrap();
        let mut child = Process::spawn(&root, "pending-create");
        child.wait_for(&root.join("input.json"));
        let input = read_json(&root.join("input.json"));
        let controls = path(&input, "controls");
        child.wait_for(&controls.join("create-pending"));
        child.signal(signal);
        child.wait_for(&root.join("cancellation-observed"));
        assert!(
            child.child.as_mut().unwrap().try_wait().unwrap().is_none(),
            "entry must retain the admitted CREATE callback"
        );
        private_bytes(&controls.join("resume-create"), b"resume");
        assert_exit(&child.finish(), false);
        let receipt = read_json(&root.join("result-pending-create.json"));
        assert_eq!(receipt["value"]["cancellationObserved"], true);
        assert_eq!(receipt["value"]["successful"], false);
        let terminal = assert_terminal(&receipt);
        assert_eq!(terminal["executionSuccessful"], false);
        let execution = &terminal["execution"];
        assert_eq!(
            execution["outputs"][2]["value"]["statementOutcome"],
            "succeeded"
        );
        assert_eq!(
            execution["outputs"][2]["value"]["error"]["code"],
            "connectionClosed"
        );
        assert_eq!(execution["outputs"][2]["value"]["retired"], true);
        assert!(execution["originalCallFailure"].is_null());
        assert_eq!(execution["settled"], true);
        assert!(execution["shutdown"].get("Ok").is_some());
        let queries = records(&controls.join("queries"));
        assert_eq!(queries, input["createPrefix"].as_array().unwrap().clone());
        let attempt = path(&receipt["value"], "outputPath");
        let journal = records(&attempt.join(FILE_NAME));
        let create = journal
            .iter()
            .find(|v| v["event"]["type"] == "operationCompletion" && v["event"]["phase"] == 3)
            .unwrap();
        assert_eq!(create["event"]["completion"]["status"], "returned");
        assert_eq!(
            create["event"]["completion"]["original"]["value"]["statementOutcome"],
            "succeeded"
        );
        assert_eq!(
            fs::read_dir(attempt.join("helper-state")).unwrap().count(),
            0
        );
        assert_eq!(
            fs::read_dir(marker_directory(&path(&input, "nativeHome")))
                .unwrap()
                .count(),
            2
        );
    }
}

#[test]
#[ignore = "optimized serial shared-entry foreground signal; --release --ignored --test-threads=1"]
fn entry_foreground_group_sigint_preserves_unknown_applied_create_without_ack() {
    let temporary = fixture_root();
    let root = temporary.path().canonicalize().unwrap();
    let mut child = Process::spawn(&root, "pending-create");
    child.wait_for(&root.join("input.json"));
    let input = read_json(&root.join("input.json"));
    let controls = path(&input, "controls");
    child.wait_for(&controls.join("create-pending"));
    assert_eq!(
        records(&controls.join("create-pending")),
        [json!({"applied":true})]
    );
    assert!(!controls.join("resume-create").exists());
    // The helper currently shares its owner's foreground group. Its immediate
    // SIGINT exit loses the held SDK callback even though CREATE already applied.
    // The retained coordinator must preserve unknown outcome, never invent ACK.
    child.signal_group(libc::SIGINT);
    child.wait_for(&root.join("cancellation-observed"));
    assert_exit(&child.finish(), false);
    assert!(!controls.join("resume-create").exists());
    let receipt = read_json(&root.join("result-pending-create.json"));
    assert_eq!(receipt["value"]["cancellationObserved"], true);
    assert_eq!(receipt["value"]["successful"], false);
    let terminal = assert_terminal(&receipt);
    assert_eq!(terminal["executionSuccessful"], false);
    let execution = &terminal["execution"];
    assert_eq!(execution["outputs"].as_array().unwrap().len(), 3);
    let original = &execution["outputs"][2];
    assert_eq!(original["value"]["statementOutcome"], "unknown");
    assert_eq!(original["value"]["error"]["code"], "outcomeUnknown");
    assert_eq!(original["value"]["retired"], true);
    assert_eq!(execution["settled"], true);
    assert_eq!(execution["guardSettled"], true);
    assert!(execution["shutdown"].get("Ok").is_some());
    assert_eq!(
        records(&controls.join("queries")),
        input["createPrefix"].as_array().unwrap().clone()
    );
    let attempt = path(&receipt["value"], "outputPath");
    let journal = records(&attempt.join(FILE_NAME));
    let create = journal
        .iter()
        .find(|v| v["event"]["type"] == "operationCompletion" && v["event"]["phase"] == 3)
        .unwrap();
    assert_eq!(create["event"]["completion"]["status"], "returned");
    assert_eq!(create["event"]["completion"]["original"], *original);
    assert!(
        journal
            .iter()
            .filter(|v| v["event"]["type"] == "validatedPhase")
            .all(|v| v["event"]["phase"].as_u64().unwrap() < 3)
    );
    assert_eq!(
        fs::read_dir(attempt.join("helper-state")).unwrap().count(),
        0
    );
    assert_eq!(
        fs::read_dir(marker_directory(&path(&input, "nativeHome")))
            .unwrap()
            .count(),
        2
    );
}

#[test]
#[ignore = "optimized serial shared-entry preparation signal; --release --ignored --test-threads=1"]
fn entry_signal_before_preparation_launches_no_helper_or_sqlite() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let temporary = fixture_root();
        let root = temporary.path().canonicalize().unwrap();
        let mut child = Process::spawn(&root, "early");
        child.wait_for(&root.join("prepare-ready"));
        child.signal(signal);
        child.wait_for(&root.join("cancellation-observed"));
        private_bytes(&root.join("prepare-release"), b"resume");
        assert_exit(&child.finish(), false);
        let receipt = read_json(&root.join("result-early.json"));
        assert_eq!(receipt["value"]["cancellationObserved"], true);
        let terminal = assert_terminal(&receipt);
        assert_eq!(terminal["execution"], Value::Null);
        let attempt = path(&receipt["value"], "outputPath");
        assert!(!attempt.join("workspace/joinery.sqlite").exists());
        assert_eq!(
            fs::read_dir(attempt.join("helper-state")).unwrap().count(),
            0
        );
        assert!(
            !path(&receipt, "childControls")
                .join("sdk-resolution")
                .exists()
        );
    }
}

#[test]
#[ignore = "optimized serial shared-entry approval/tamper; --release --ignored --test-threads=1"]
fn entry_invalid_approval_and_tampered_original_fail_before_helper_launch() {
    for mode in ["invalid-approval", "tamper"] {
        let temporary = fixture_root();
        let root = temporary.path().canonicalize().unwrap();
        assert_exit(&Process::spawn(&root, mode).finish(), false);
        let input = read_json(&root.join("input.json"));
        let receipt = read_json(&root.join(format!("result-{mode}.json")));
        assert!(
            !path(&receipt, "childControls")
                .join("sdk-resolution")
                .exists()
        );
        assert!(!path(&receipt, "childControls").join("connections").exists());
        if mode == "invalid-approval" {
            let terminal = assert_terminal(&receipt);
            assert_eq!(terminal["execution"], Value::Null);
            let attempt = path(&receipt["value"], "outputPath");
            assert!(!attempt.join("workspace/joinery.sqlite").exists());
            assert_eq!(
                fs::read_dir(attempt.join("helper-state")).unwrap().count(),
                0
            );
            assert_eq!(
                fs::read_dir(marker_directory(&path(&input, "nativeHome")))
                    .unwrap()
                    .count(),
                0
            );
        } else {
            assert_eq!(
                receipt["error"]["failureCode"],
                "envelope_verification_failed"
            );
            assert_eq!(fs::read_dir(path(&input, "output")).unwrap().count(), 0);
            assert!(!marker_directory(&path(&input, "nativeHome")).exists());
        }
    }
}

#[test]
#[ignore = "optimized serial shared-entry full lifecycle and replay; --release --ignored --test-threads=1"]
fn entry_full_32_phase_execution_and_fresh_process_replay_use_same_fixed_markers() {
    let temporary = fixture_root();
    let root = temporary.path().canonicalize().unwrap();
    assert_exit(&Process::spawn(&root, "normal").finish(), true);
    let input = read_json(&root.join("input.json"));
    let receipt = read_json(&root.join("result-normal.json"));
    assert_eq!(receipt["value"]["successful"], true);
    let terminal = assert_terminal(&receipt);
    assert_eq!(terminal["executionSuccessful"], true);
    assert_eq!(
        terminal["execution"]["outputs"].as_array().unwrap().len(),
        32
    );
    let attempt = path(&receipt["value"], "outputPath");
    let journal = records(&attempt.join(FILE_NAME));
    assert_eq!(
        journal
            .iter()
            .filter(|v| v["event"]["type"] == "validatedPhase")
            .count(),
        32
    );
    let controls = path(&input, "controls");
    assert_eq!(
        records(&controls.join("queries")),
        input["expectedSql"].as_array().unwrap().clone()
    );
    let original_queries = fs::read(controls.join("queries")).unwrap();
    let markers = marker_directory(&path(&input, "nativeHome"));
    let before: Vec<_> = fs::read_dir(&markers)
        .unwrap()
        .map(|v| {
            let path = v.unwrap().path();
            (
                path.file_name().unwrap().to_owned(),
                fs::read(path).unwrap(),
            )
        })
        .collect();
    assert_eq!(before.len(), 2);
    assert_exit(&Process::spawn(&root, "replay").finish(), false);
    let replay = read_json(&root.join("result-replay.json"));
    assert_eq!(replay["value"]["failureCode"], "preparation_failed");
    assert_eq!(replay["value"]["run"], input["run"]);
    assert_eq!(replay["value"]["executionSha256"], input["executionSha256"]);
    let terminal = assert_terminal(&replay);
    assert_eq!(terminal["execution"], Value::Null);
    let replay_attempt = path(&replay["value"], "outputPath");
    assert!(!replay_attempt.join("workspace/joinery.sqlite").exists());
    assert!(
        !path(&replay, "childControls")
            .join("sdk-resolution")
            .exists()
    );
    assert_eq!(
        original_queries,
        fs::read(controls.join("queries")).unwrap()
    );
    assert_eq!(fs::read_dir(&markers).unwrap().count(), 2);
    for (name, bytes) in before {
        assert_eq!(fs::read(markers.join(name)).unwrap(), bytes);
    }
}
