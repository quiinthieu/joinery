//! Explicit offline measurements of complete, real local artifact inventories.
//! This never launches the packaged runtime or authenticates to an account.

use super::*;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    process::{Command, Output},
};

const REPORT_BYTES: usize = 64 * 1024;

fn emit(value: Value) {
    let bytes = serde_json::to_vec(&value).expect("serialize bounded performance report");
    assert!(
        bytes.len() <= REPORT_BYTES,
        "performance report exceeded bound"
    );
    eprintln!("{}", String::from_utf8(bytes).expect("JSON is UTF-8"));
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).expect("measurement duration fits u64")
}

fn measured<T>(stage: &str, operation: impl FnOnce() -> AppResult<T>) -> (T, u64) {
    let started = Instant::now();
    let result = operation();
    let elapsed = micros(started.elapsed());
    emit(json!({
        "measurement":"completeEnvelope", "stage":stage,
        "elapsedMicros":elapsed, "ok":result.is_ok(),
    }));
    (
        result.unwrap_or_else(|error| panic!("complete envelope {stage} failed: {error:?}")),
        elapsed,
    )
}

fn command_stdout(output: Output, description: &str) -> String {
    assert!(output.status.success(), "{description} failed");
    assert!(
        output.stdout.len() <= 8192 && output.stderr.len() <= 8192,
        "{description} output exceeded its diagnostic bound"
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|_| panic!("{description} did not return UTF-8"))
}

fn pinned_compiler() -> PathBuf {
    let output = Command::new("rustup")
        .args(["which", "--toolchain", "1.99.1", "rustc"])
        .output()
        .expect("resolve pinned Rust 1.99.1 with rustup; no compiler fallback is permitted");
    let path = PathBuf::from(command_stdout(output, "rustup which 1.99.1 rustc").trim());
    assert!(
        path.is_absolute(),
        "rustup must resolve an absolute compiler path"
    );
    let path = path
        .canonicalize()
        .expect("resolved pinned compiler must exist");
    let output = Command::new(&path)
        .arg("--version")
        .output()
        .expect("read the resolved compiler's version");
    let version = command_stdout(output, "resolved rustc --version");
    assert_eq!(
        version.split_whitespace().take(2).collect::<Vec<_>>(),
        ["rustc", "1.99.1"],
        "the inventory must bind the resolved pinned compiler, not a shim or another version"
    );
    path
}

fn artifact_summary(scope: &str, artifact: &Artifact) -> Value {
    // Summaries never print physical paths, profile values or individual entries.
    // The underlying production capture still includes every bounded entry.
    let value = serde_json::to_value(&artifact.inventory).expect("serialize actual inventory");
    let entries = value["entries"].as_array().expect("inventory entries");
    let mut files = 0_u64;
    let mut directories = 0_u64;
    let mut file_bytes = 0_u64;
    for entry in entries {
        match entry["kind"].as_str().expect("inventory entry kind") {
            "file" => {
                files += 1;
                file_bytes = file_bytes
                    .checked_add(entry["length"].as_u64().expect("inventory file length"))
                    .expect("bounded inventory byte sum");
            }
            "tree" => directories += 1,
            _ => panic!("unexpected inventory entry kind"),
        }
    }
    json!({
        "scope":scope, "entries":entries.len(), "files":files,
        "directories":directories, "fileBytes":file_bytes,
        "inventorySha256":artifact.inventory.sha256().expect("hash actual inventory"),
    })
}

fn require_optimized_platform() {
    assert_eq!(
        (
            std::env::consts::OS,
            std::env::consts::ARCH,
            cfg!(debug_assertions),
        ),
        ("macos", "aarch64", false),
        "run this ignored acceptance measurement with --release on macOS ARM64"
    );
}

fn actual_repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Rust manifest belongs to repository")
        .canonicalize()
        .expect("actual repository must exist")
}

struct RealContent {
    _directory: tempfile::TempDir,
    roots: Roots,
    store: ConsumptionStore,
    window: Window,
}

fn real_content_setup(runtime: PathBuf, frozen_profile: Value) -> RealContent {
    require_optimized_platform();
    assert!(
        runtime.is_absolute() && runtime.is_dir(),
        "an existing absolute complete runtime root is required; no synthetic fallback is permitted"
    );
    let private = tempfile::tempdir().expect("create synthetic private fixture directory");
    let private_root = private
        .path()
        .canonicalize()
        .expect("physical fixture path");
    fs::set_permissions(&private_root, fs::Permissions::from_mode(0o700))
        .expect("make fixture directory private");
    let profile = private_root.join("synthetic-profile.json");
    let mut profile_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&profile)
        .expect("exclusively create synthetic 0600 profile");
    let profile_bytes =
        serde_json::to_vec(&frozen_profile).expect("serialize synthetic seven-field profile");
    assert!(
        profile_bytes.len() <= MAX_PROFILE_BYTES,
        "bounded synthetic profile"
    );
    profile_file
        .write_all(&profile_bytes)
        .expect("write synthetic profile");
    profile_file.sync_all().expect("sync synthetic profile");
    let store_path = private_root.join("consumption");
    fs::create_dir(&store_path).expect("create fresh synthetic consumption directory");
    fs::set_permissions(&store_path, fs::Permissions::from_mode(0o700))
        .expect("make consumption directory private");
    let store = ConsumptionStore::open(&store_path).expect("open strict private consumption store");
    let roots = Roots {
        repository: actual_repository(),
        runtime,
        executable: std::env::current_exe().expect("bind actually running optimized executable"),
        compiler: pinned_compiler(),
        profile,
    };
    let now = now_ms().expect("read wall clock");
    let window = Window {
        not_before_unix_ms: now,
        expires_unix_ms: now.checked_add(WINDOW_MS).expect("bounded approval window"),
    };
    RealContent {
        _directory: private,
        roots,
        store,
        window,
    }
}

fn artifact_summaries(envelope: &Envelope, runtime_scope: &str) -> Vec<Value> {
    let mut artifacts = Vec::new();
    for (scope, artifact) in SOURCE_TREES
        .iter()
        .chain(SOURCE_FILES.iter())
        .zip(&envelope.manifest.sources)
    {
        artifacts.push(artifact_summary(scope, artifact));
    }
    assert_eq!(artifacts.len(), SOURCE_TREES.len() + SOURCE_FILES.len());
    for (scope, artifact) in [
        (runtime_scope, &envelope.manifest.runtime),
        ("currentExecutable", &envelope.manifest.executable),
        ("resolvedPinnedCompiler", &envelope.manifest.compiler),
        ("syntheticProfile", &envelope.manifest.profile_file),
    ] {
        artifacts.push(artifact_summary(scope, artifact));
    }
    artifacts
}

fn synthetic_approval(execution_sha256: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format":APPROVAL_FORMAT, "scope":APPROVAL_SCOPE,
        "execution_sha256":execution_sha256, "maximum_attempts":1,
    }))
    .expect("serialize local synthetic one-use approval")
}

/// Shared real-input setup for ignored optimized Studio route measurements.
/// The caller keeps the supplied runtime unchanged and retains its lifetime.
pub(super) fn fixture_consumed_real_content(
    plan: Plan,
    profile: Value,
    runtime_root: PathBuf,
) -> (tempfile::TempDir, ConsumedEnvelope, Value) {
    let setup = real_content_setup(runtime_root, profile);
    let plan_sha256 = plan.sha256().expect("hash supplied closed plan");
    let (envelope, capture_us) = measured("routeCapture", || {
        Envelope::capture(&setup.roots, plan, setup.window)
    });
    let (canonical, canonical_us) = measured("routeCanonical", || envelope.canonical_json());
    let execution_sha256 = hash(&canonical);
    let artifacts = artifact_summaries(&envelope, "callerProvidedRouteRuntime");
    let approval = synthetic_approval(&execution_sha256);
    let (consumed, consume_us) =
        measured("routeConsume", || envelope.consume(&setup.store, &approval));
    assert_eq!(
        consumed.fixture_runtime_root(),
        setup.roots.runtime.as_path()
    );
    let evidence = json!({
        "scope":"offlineRealContentRouteInputs", "runtimeExecuted":false,
        "planSha256":plan_sha256, "executionSha256":execution_sha256,
        "canonicalBytes":canonical.len(), "artifacts":artifacts,
        "durationsMicros":{
            "capture":capture_us, "canonical":canonical_us, "consume":consume_us,
            "consumedElapsed":micros(consumed.fixture_elapsed()),
        },
    });
    assert!(
        serde_json::to_vec(&evidence)
            .expect("bounded artifact evidence")
            .len()
            <= REPORT_BYTES
    );
    (setup._directory, consumed, evidence)
}

#[test]
#[ignore = "Requires --release on macOS ARM64, pinned Rust 1.99.1, and the complete normal packaged zero-link Snowflake runtime"]
fn envelope_real_content_optimized_capture_restore_consume_and_revalidate() {
    require_optimized_platform();
    let total_started = Instant::now();
    // Do not canonicalize or copy the runtime: production inventory must reject
    // links and unsafe permissions in the unchanged normal package itself.
    let runtime = actual_repository().join(
        "src-tauri/target/release/bundle/macos/Joinery.app/Contents/Resources/snowflake-runtime",
    );
    assert!(
        runtime.is_dir(),
        "complete normal packaged runtime is required; build the normal macOS bundle first"
    );
    let setup = real_content_setup(
        runtime,
        json!({
            "account":"synthetic-account",
            "host":"synthetic-account.snowflakecomputing.com",
            "username":"synthetic@example.test", "role":"QUALIFICATION",
            "warehouse":"FIXTURE_WH", "database":"SANDBOX", "schema":"BILLIE_TESTING",
        }),
    );
    let plan = Plan::generate().expect("generate the existing closed plan");
    let plan_sha256 = plan.sha256().expect("hash closed plan");
    let (envelope, capture_us) = measured("capture", || {
        Envelope::capture(&setup.roots, plan, setup.window)
    });
    let (canonical, canonical_us) = measured("canonical", || envelope.canonical_json());
    let execution_sha256 = hash(&canonical);
    let (restored, restore_us) = measured("restore", || {
        Envelope::restore_snapshot(&setup.roots, &canonical, &execution_sha256)
    });
    assert_eq!(
        canonical,
        restored.canonical_json().expect("restored canonical bytes")
    );
    let artifacts = artifact_summaries(&restored, "normalPackagedRuntime");
    let approval = synthetic_approval(&execution_sha256);
    let (consumed, consume_us) = measured("consume", || restored.consume(&setup.store, &approval));
    let mut revalidate_us = Vec::with_capacity(3);
    for stage in ["revalidate1", "revalidate2", "revalidate3"] {
        let ((), elapsed) = measured(stage, || consumed.revalidate());
        revalidate_us.push(elapsed);
    }
    consumed
        .check()
        .expect("unchanged production window and lifecycle deadline");
    assert_eq!(
        fs::read_dir(setup._directory.path().join("consumption"))
            .expect("read private markers")
            .count(),
        2
    );
    emit(json!({
        "measurement":"completeEnvelope", "scope":"offlineRealContentOnly",
        "optimized":true, "authentication":false, "runtimeExecuted":false,
        "planSha256":plan_sha256, "executionSha256":execution_sha256,
        "canonicalBytes":canonical.len(), "artifacts":artifacts,
        "durationsMicros":{
            "capture":capture_us, "canonical":canonical_us, "restore":restore_us,
            "consume":consume_us, "revalidate":revalidate_us,
            "consumedElapsed":micros(consumed.fixture_elapsed()),
            "total":micros(total_started.elapsed()),
        },
        "workflowDeadlineSeconds":DEADLINE.as_secs(), "windowMilliseconds":WINDOW_MS,
    }));
}
