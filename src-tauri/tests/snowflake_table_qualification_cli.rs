#![cfg(all(unix, feature = "test-harness"))]

//! Run the actual dedicated binary in separate processes. All account/source/
//! runtime data below is synthetic; trap executables must never be launched.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    repository: PathBuf,
    runtime: PathBuf,
    compiler: PathBuf,
    profile: PathBuf,
    output: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let repository = root.join("repository");
        for tree in ["src-tauri/src", "src-tauri/vendor", "snowflake-helper/src"] {
            fs::create_dir_all(repository.join(tree)).unwrap();
            fs::write(repository.join(tree).join("synthetic-source"), b"source A").unwrap();
        }
        for file in [
            "src-tauri/Cargo.toml",
            "src-tauri/Cargo.lock",
            "src-tauri/build.rs",
            "rust-toolchain.toml",
            "snowflake-helper/package.json",
            "snowflake-helper/package-lock.json",
            "scripts/prepare-snowflake-runtime.mjs",
            "scripts/verify-snowflake-runtime.mjs",
        ] {
            fs::create_dir_all(repository.join(file).parent().unwrap()).unwrap();
            fs::write(repository.join(file), b"synthetic fixed build input").unwrap();
        }
        let runtime = root.join("runtime");
        fs::create_dir(&runtime).unwrap();
        let compiler = root.join("rustc");
        let trap = format!(
            "#!/bin/sh\nprintf launched > '{}'\nexit 99\n",
            root.join("unexpected-launch").display()
        );
        for path in [runtime.join("node"), compiler.clone()] {
            fs::write(&path, &trap).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let profile = root.join("profile.json");
        fs::write(
            &profile,
            serde_json::to_vec(&json!({
                "account":"synthetic-account",
                "host":"synthetic-account.snowflakecomputing.com",
                "username":"synthetic-person@example.test", "role":"QUALIFICATION",
                "warehouse":"SYNTHETIC_WH", "database":"SANDBOX", "schema":"BILLIE_TESTING",
            }))
            .unwrap(),
        )
        .unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
        let output = root.join("output");
        fs::create_dir(&output).unwrap();
        fs::set_permissions(&output, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _directory: directory,
            root,
            repository,
            runtime,
            compiler,
            profile,
            output,
        }
    }

    fn roots(&self, operation: &str) -> Vec<OsString> {
        let mut arguments = vec![operation.into()];
        for (name, value) in [
            ("--repository", &self.repository),
            ("--runtime", &self.runtime),
            ("--compiler", &self.compiler),
            ("--profile", &self.profile),
        ] {
            arguments.push(name.into());
            arguments.push(value.as_os_str().into());
        }
        arguments
    }

    fn emit_arguments(&self) -> Vec<OsString> {
        let mut arguments = self.roots("emit");
        arguments.extend([
            "--output-parent".into(),
            self.output.as_os_str().into(),
            "--not-before-unix-ms".into(),
            "1000".into(),
            "--expires-unix-ms".into(),
            "2000".into(),
        ]);
        arguments
    }

    fn emit(&self) -> Value {
        let output = run(self.emit_arguments());
        assert!(output.status.success(), "{:?}", output.stderr);
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn verify_arguments(&self, receipt: &Value) -> Vec<OsString> {
        let mut arguments = self.roots("verify");
        arguments.extend([
            "--envelope".into(),
            receipt["envelopeFile"].as_str().unwrap().into(),
            "--sha256".into(),
            receipt["executionSha256"].as_str().unwrap().into(),
        ]);
        arguments
    }

    fn execute_arguments(&self) -> Vec<OsString> {
        let mut arguments = self.roots("execute");
        arguments.extend([
            "--envelope".into(),
            self.root.join("missing-envelope.json").into_os_string(),
            "--sha256".into(),
            "a".repeat(64).into(),
            "--approval".into(),
            self.root.join("missing-approval.json").into_os_string(),
            "--output-parent".into(),
            self.output.as_os_str().into(),
        ]);
        arguments
    }

    fn no_launch(&self) {
        assert!(!self.root.join("unexpected-launch").exists());
    }
}

fn run(arguments: Vec<OsString>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_snowflake-table-qualification"))
        .args(arguments)
        .output()
        .unwrap()
}

fn failure(output: &Output, code: &str) {
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap(),
        json!({"failureCode":code})
    );
}

fn snapshot(path: &Path) -> BTreeMap<PathBuf, (u32, String)> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, (u32, String)>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(!metadata.is_symlink());
        let hash = if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), files);
            }
            String::new()
        } else {
            format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
        };
        files.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            (metadata.permissions().mode(), hash),
        );
    }
    let mut files = BTreeMap::new();
    visit(path, path, &mut files);
    files
}

#[test]
fn separate_process_verify_restores_original_bytes_without_launch_or_writes() {
    let fixture = Fixture::new();
    let emitted = fixture.emit();
    let bytes = fs::read(emitted["envelopeFile"].as_str().unwrap()).unwrap();
    assert_eq!(
        emitted["executionSha256"],
        format!("{:x}", Sha256::digest(&bytes))
    );
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_snowflake-table-qualification"))
        .canonicalize()
        .unwrap();
    assert_eq!(manifest["executable"]["path"], json!(binary));
    assert_eq!(
        manifest["executable"]["inventory"]["entries"][0]["sha256"],
        format!("{:x}", Sha256::digest(fs::read(binary).unwrap()))
    );
    assert_eq!(
        manifest["build"]["command"],
        json!([
            "cargo",
            "build",
            "--manifest-path",
            "src-tauri/Cargo.toml",
            "--release",
            "--locked",
            "--no-default-features",
            "--features",
            "test-harness",
            "--bin",
            "snowflake-table-qualification"
        ])
    );
    assert_eq!(
        fs::read(emitted["planFile"].as_str().unwrap()).unwrap(),
        manifest["plan_json"].as_str().unwrap().as_bytes()
    );
    let before = snapshot(&fixture.root);
    let output = run(fixture.verify_arguments(&emitted));
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let verified: Value = serde_json::from_slice(&output.stdout).unwrap();
    for key in [
        "run",
        "table",
        "planSha256",
        "executionSha256",
        "maximumSql",
    ] {
        assert_eq!(emitted[key], verified[key]);
    }
    assert_eq!(verified["maximumSql"], 610);
    assert_eq!(verified["executionAvailable"], false);
    assert_eq!(verified["accountActivity"], false);
    // Offline verification is intentionally possible after the approval window.
    assert_eq!(verified["operation"], "verify");
    for output in [serde_json::to_vec(&emitted).unwrap(), output.stdout] {
        let text = String::from_utf8(output).unwrap();
        for private in ["synthetic-account", "synthetic-person", "SYNTHETIC_WH"] {
            assert!(!text.contains(private));
        }
    }
    assert_eq!(before, snapshot(&fixture.root));
    fixture.no_launch();
}

#[test]
fn changed_profile_or_source_or_envelope_is_rejected_in_another_process() {
    for target in ["profile", "source", "envelope"] {
        let fixture = Fixture::new();
        let receipt = fixture.emit();
        let path = match target {
            "profile" => fixture.profile.clone(),
            "source" => fixture.repository.join("src-tauri/src/synthetic-source"),
            _ => PathBuf::from(receipt["envelopeFile"].as_str().unwrap()),
        };
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b' ');
        fs::write(path, bytes).unwrap();
        let before = snapshot(&fixture.root);
        failure(
            &run(fixture.verify_arguments(&receipt)),
            "envelope_verification_failed",
        );
        assert_eq!(before, snapshot(&fixture.root));
        fixture.no_launch();
    }
}

#[test]
fn copied_executable_cannot_verify_another_executables_envelope() {
    let fixture = Fixture::new();
    let receipt = fixture.emit();
    let copy = fixture.root.join("copied-qualification-command");
    fs::copy(env!("CARGO_BIN_EXE_snowflake-table-qualification"), &copy).unwrap();
    fs::set_permissions(
        &copy,
        fs::metadata(env!("CARGO_BIN_EXE_snowflake-table-qualification"))
            .unwrap()
            .permissions(),
    )
    .unwrap();
    let before = snapshot(&fixture.root);
    let output = Command::new(copy)
        .args(fixture.verify_arguments(&receipt))
        .output()
        .unwrap();
    failure(&output, "envelope_verification_failed");
    assert_eq!(before, snapshot(&fixture.root));
    fixture.no_launch();
}

#[test]
fn unknown_duplicate_and_missing_flags_fail_without_io() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.root);
    let mut duplicate = fixture.emit_arguments();
    duplicate[9] = "--repository".into();
    let mut unknown = fixture.emit_arguments();
    unknown[9] = "--approval".into();
    let mut missing = fixture.emit_arguments();
    missing.pop();
    for arguments in [vec!["execute".into()], duplicate, unknown, missing, vec![]] {
        failure(&run(arguments), "invalid_arguments");
    }
    assert_eq!(before, snapshot(&fixture.root));
    fixture.no_launch();
}

#[test]
fn execute_parser_rejects_invalid_arguments_without_io_or_native_store() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.root);
    let base = fixture.execute_arguments();
    let mut missing = base.clone();
    missing.truncate(missing.len() - 2);
    let mut duplicate = base.clone();
    duplicate[15] = "--approval".into();
    let mut unknown = base.clone();
    unknown[15] = "--consumption-store".into();
    let mut relative = base.clone();
    relative[16] = "relative-output".into();
    let mut bad_hash = base.clone();
    bad_hash[12] = "A".repeat(64).into();
    let mut unencodable = base;
    unencodable[16] = fixture
        .output
        .join(OsString::from_vec(b"bad-\xff".to_vec()))
        .into_os_string();
    for arguments in [missing, duplicate, unknown, relative, bad_hash, unencodable] {
        failure(&run(arguments), "invalid_arguments");
    }
    assert_eq!(before, snapshot(&fixture.root));
    fixture.no_launch();
}

#[test]
fn execute_missing_original_envelope_fails_before_native_store_or_launch() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.root);
    // The original envelope does not exist. Shared preparation must reject it
    // before reading approval or resolving/touching the OS account home/store.
    failure(
        &run(fixture.execute_arguments()),
        "envelope_verification_failed",
    );
    assert_eq!(before, snapshot(&fixture.root));
    fixture.no_launch();
}

#[test]
fn unencodable_output_path_fails_before_persisting_any_output() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.root);
    let mut arguments = fixture.emit_arguments();
    arguments[10] = fixture
        .output
        .join(OsString::from_vec(b"bad-\xff".to_vec()))
        .into_os_string();
    failure(&run(arguments), "invalid_arguments");
    assert_eq!(before, snapshot(&fixture.root));
    fixture.no_launch();
}

#[test]
fn unsafe_output_and_source_overlap_fail_before_emission() {
    for linked in [false, true] {
        let mut fixture = Fixture::new();
        fixture.output = if linked {
            let alias = fixture.root.join("output-alias");
            symlink(&fixture.output, &alias).unwrap();
            alias
        } else {
            fixture.repository.join("src-tauri/src")
        };
        let before = fs::read_dir(&fixture.output).unwrap().count();
        failure(
            &run(fixture.emit_arguments()),
            if linked {
                "emission_persistence_failed"
            } else {
                "output_not_separate"
            },
        );
        assert_eq!(before, fs::read_dir(&fixture.output).unwrap().count());
        fixture.no_launch();
    }
}

#[test]
fn help_is_offline_and_describes_the_fixed_execute_route() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.root);
    let output = run(vec!["--help".into()]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let help: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(help["format"], "joinery-snowflake-table-command-v1");
    assert_eq!(help["executionAvailable"], true);
    assert_eq!(help["accountActivity"], false);
    assert_eq!(
        help["commands"]["execute"],
        "--repository ABS --runtime ABS --compiler ABS --profile ABS --envelope ABS --sha256 LOWERCASE_SHA256 --approval ABS --output-parent ABS"
    );
    assert_eq!(before, snapshot(&fixture.root));
    fixture.no_launch();
}
