use super::*;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    roots: Roots,
    window: Window,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let roots = Roots {
            repository: root.join("repository"),
            runtime: root.join("runtime"),
            executable: root.join("qualification-binary"),
            compiler: root.join("rustc"),
            profile: root.join("profile.json"),
        };
        for path in SOURCE_TREES {
            let tree = roots.repository.join(path);
            fs::create_dir_all(&tree).unwrap();
            fs::write(tree.join("included-source"), b"source version A").unwrap();
        }
        for path in SOURCE_FILES {
            let file = roots.repository.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"fixed build input").unwrap();
        }
        fs::create_dir_all(roots.runtime.join("node_modules/guard/deep")).unwrap();
        fs::create_dir(roots.runtime.join("empty-directory")).unwrap();
        fs::write(
            roots.runtime.join("node_modules/guard/deep/library.mjs"),
            b"runtime A",
        )
        .unwrap();
        fs::write(&roots.executable, b"synthetic executable bytes").unwrap();
        fs::write(&roots.compiler, b"synthetic compiler bytes").unwrap();
        fs::write(&roots.profile, serde_json::to_vec(&profile()).unwrap()).unwrap();
        fs::set_permissions(&roots.profile, fs::Permissions::from_mode(0o600)).unwrap();
        let now = now_ms().unwrap();
        Self {
            _directory: directory,
            root,
            roots,
            window: Window {
                not_before_unix_ms: now - 1000,
                expires_unix_ms: now + 60_000,
            },
        }
    }

    fn capture(&self) -> Envelope {
        Envelope::capture(&self.roots, Plan::generate().unwrap(), self.window).unwrap()
    }

    fn store(&self) -> ConsumptionStore {
        let path = self.root.join("consumption");
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        ConsumptionStore::open(&path).unwrap()
    }

    fn markers(&self) -> usize {
        fs::read_dir(self.root.join("consumption")).unwrap().count()
    }
}

fn profile() -> Value {
    json!({
        "account":"synthetic-account", "host":"synthetic-account.snowflakecomputing.com",
        "username":"synthetic@example.test", "role":"QUALIFICATION", "warehouse":"FIXTURE_WH",
        "database":"SANDBOX", "schema":"BILLIE_TESTING"
    })
}

fn approval(envelope: &Envelope) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format":APPROVAL_FORMAT, "scope":APPROVAL_SCOPE,
        "execution_sha256":envelope.sha256().unwrap(), "maximum_attempts":1,
    }))
    .unwrap()
}

// Sibling guard/runner tests use the actual consume path and actual current
// executable. Only source/runtime/compiler/profile inputs are synthetic.
pub(super) fn fixture_unconsumed(
    plan: Plan,
    frozen_profile: Value,
    runtime_root: Option<PathBuf>,
) -> (tempfile::TempDir, Envelope, ConsumptionStore, Vec<u8>) {
    let mut fixture = Fixture::new();
    fixture.roots.executable = std::env::current_exe().unwrap();
    fixture.window.expires_unix_ms = fixture.window.not_before_unix_ms + WINDOW_MS;
    if let Some(runtime) = runtime_root {
        fixture.roots.runtime = runtime;
    } else {
        fs::set_permissions(&fixture.roots.runtime, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::write(
        &fixture.roots.profile,
        serde_json::to_vec(&frozen_profile).unwrap(),
    )
    .unwrap();
    let envelope = Envelope::capture(&fixture.roots, plan, fixture.window).unwrap();
    let grant = approval(&envelope);
    let store = fixture.store();
    (fixture._directory, envelope, store, grant)
}

pub(super) fn fixture_consumed(
    plan: Plan,
    frozen_profile: Value,
) -> (tempfile::TempDir, ConsumedEnvelope, PathBuf) {
    let mut fixture = Fixture::new();
    fixture.roots.executable = std::env::current_exe().unwrap();
    fixture.window.expires_unix_ms = fixture.window.not_before_unix_ms + WINDOW_MS;
    fs::write(
        &fixture.roots.profile,
        serde_json::to_vec(&frozen_profile).unwrap(),
    )
    .unwrap();
    let envelope = Envelope::capture(&fixture.roots, plan, fixture.window).unwrap();
    let grant = approval(&envelope);
    let consumed = envelope.consume(&fixture.store(), &grant).unwrap();
    let drift = fixture
        .roots
        .runtime
        .join("node_modules/guard/deep/library.mjs");
    (fixture._directory, consumed, drift)
}

#[test]
fn envelope_exact_restore_retains_closed_plan_and_complete_artifacts_without_launch() {
    let fixture = Fixture::new();
    let envelope = fixture.capture();
    let bytes = envelope.canonical_json().unwrap();
    let restored =
        Envelope::restore_snapshot(&fixture.roots, &bytes, &envelope.sha256().unwrap()).unwrap();
    assert_eq!(bytes, restored.canonical_json().unwrap());
    assert_eq!(envelope.plan.run(), restored.plan.run());
    assert_eq!(
        envelope.plan.canonical_json().unwrap(),
        restored.plan.canonical_json().unwrap()
    );
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["workflow"], WORKFLOW);
    assert_eq!(
        value["sources"].as_array().unwrap().len(),
        SOURCE_TREES.len() + SOURCE_FILES.len()
    );
    assert_eq!(value["limits"]["maximum_sql"], 610);
    assert_eq!(value["limits"]["inbound_body_bytes"], 32 * 1024 * 1024);
    assert!(!fixture.root.join("consumption").exists());
    assert!(!fixture.root.join("authentication").exists());
}

#[test]
fn envelope_capture_rejects_account_host_mismatch_before_consumption() {
    let fixture = Fixture::new();
    let mut changed = profile();
    changed["account"] = json!("another-account");
    fs::write(
        &fixture.roots.profile,
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert!(
        Envelope::capture(&fixture.roots, Plan::generate().unwrap(), fixture.window).is_err(),
        "the reviewed account must be the account the external-browser driver derives from host"
    );
    assert!(!fixture.root.join("consumption").exists());
}

#[test]
fn envelope_fixed_policy_is_recomputed_instead_of_restored_from_reviewed_bytes() {
    let fixture = Fixture::new();
    let envelope = fixture.capture();
    let canonical = String::from_utf8(envelope.canonical_json().unwrap()).unwrap();
    let value: Value = serde_json::from_str(&canonical).unwrap();
    assert_eq!(
        value["policy"],
        serde_json::to_value(profile::declaration()).unwrap()
    );
    let declaration = serde_json::to_string(&profile::declaration()).unwrap();
    assert!(canonical.contains(&format!("\"policy\":{declaration}")));
    // Keep Manifest and Declaration field ordering. Every rehashed mutation
    // must fail because its fixed policy differs, not because Value reordered it.
    for (field, replacement) in [
        ("groupId", json!("local")),
        ("environment", json!("production")),
        ("safeMode", json!("silent")),
        ("authentication", json!("localApplicationOauth")),
        ("reviewedDml", json!(false)),
        ("reviewedDdl", json!(false)),
        ("tlsMode", json!("disable")),
        ("port", json!(80)),
        ("ssh", json!(true)),
        ("caCertificate", json!(true)),
        ("storedSecrets", json!(true)),
        ("retainQueryText", json!(true)),
        ("persistDrafts", json!(true)),
        ("database", json!("ANOTHER_DATABASE")),
        ("schema", json!("ANOTHER_SCHEMA")),
    ] {
        let from = format!("\"{field}\":{}", value["policy"][field]);
        let to = format!("\"{field}\":{replacement}");
        assert!(declaration.contains(&from), "{field}");
        let changed_policy = declaration.replacen(&from, &to, 1);
        let changed = canonical.replacen(&declaration, &changed_policy, 1);
        assert_ne!(changed, canonical);
        assert!(
            Envelope::restore_snapshot(
                &fixture.roots,
                changed.as_bytes(),
                &hash(changed.as_bytes())
            )
            .is_err(),
            "{field}"
        );
    }
    let absent = canonical.replacen(&format!("\"policy\":{declaration},"), "", 1);
    assert_ne!(absent, canonical);
    assert!(
        Envelope::restore_snapshot(&fixture.roots, absent.as_bytes(), &hash(absent.as_bytes()))
            .is_err()
    );
    assert!(!fixture.root.join("consumption").exists());
}

#[test]
fn envelope_file_restore_reads_large_private_snapshot_without_widening_small_inputs() {
    let fixture = Fixture::new();
    for number in 0..600 {
        fs::write(
            fixture
                .roots
                .runtime
                .join(format!("library-{number:04}.mjs")),
            b"source",
        )
        .unwrap();
    }
    let envelope = fixture.capture();
    let bytes = envelope.canonical_json().unwrap();
    assert!(bytes.len() > 64 * 1024);
    let path = fixture.root.join("envelope.json");
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(read_private_file(&path, 64 * 1024).is_err());
    let restored = Envelope::restore_file(&fixture.roots, &path, &hash(&bytes)).unwrap();
    assert_eq!(restored.canonical_json().unwrap(), bytes);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Envelope::restore_file(&fixture.roots, &path, &hash(&bytes)).is_err());
    assert!(!fixture.root.join("consumption").exists());
}

#[test]
fn envelope_rehashed_untrusted_fields_never_replace_recomputed_identity() {
    let fixture = Fixture::new();
    let envelope = fixture.capture();
    let original: Value = serde_json::from_slice(&envelope.canonical_json().unwrap()).unwrap();
    for field in [
        "format",
        "workflow",
        "plan_json",
        "plan_sha256",
        "profile",
        "policy",
        "sources",
        "runtime",
        "executable",
        "compiler",
        "profile_file",
        "build",
        "limits",
        "extra",
    ] {
        let mut changed = original.clone();
        changed[field] = json!("untrusted replacement");
        let bytes = serde_json::to_vec(&changed).unwrap();
        assert!(
            Envelope::restore_snapshot(&fixture.roots, &bytes, &hash(&bytes)).is_err(),
            "{field}"
        );
    }
    // Preserve the canonical field ordering so rejection cannot be explained
    // solely by generic Value serialization reordering Manifest fields.
    let canonical = String::from_utf8(envelope.canonical_json().unwrap()).unwrap();
    for (from, to) in [
        ("\"maximum_sql\":610", "\"maximum_sql\":611"),
        ("\"role\":\"QUALIFICATION\"", "\"role\":\"DIFFERENT\""),
        ("\"command\":[\"cargo\"", "\"command\":[\"forged\""),
    ] {
        assert!(canonical.contains(from));
        let changed = canonical.replacen(from, to, 1);
        assert_ne!(changed, canonical);
        assert!(
            Envelope::restore_snapshot(
                &fixture.roots,
                changed.as_bytes(),
                &hash(changed.as_bytes())
            )
            .is_err()
        );
    }
    let mut padded = envelope.canonical_json().unwrap();
    padded.push(b' ');
    assert!(Envelope::restore_snapshot(&fixture.roots, &padded, &hash(&padded)).is_err());
    assert!(Envelope::restore_snapshot(&fixture.roots, &[], &hash(b"")).is_err());
    assert!(
        Envelope::restore_snapshot(
            &fixture.roots,
            &[b' '; MAX_ENVELOPE_BYTES + 1],
            &hash(b"different")
        )
        .is_err()
    );
}

#[test]
fn envelope_detects_dirty_source_runtime_dependency_binary_compiler_and_profile_drift() {
    for target in [
        "source",
        "dependency",
        "runtimeAdded",
        "binary",
        "compiler",
        "profile",
    ] {
        let fixture = Fixture::new();
        let envelope = fixture.capture();
        let bytes = envelope.canonical_json().unwrap();
        match target {
            "source" => fs::write(
                fixture
                    .roots
                    .repository
                    .join(SOURCE_TREES[0])
                    .join("included-source"),
                b"source version B",
            )
            .unwrap(),
            "dependency" => fs::write(
                fixture
                    .roots
                    .runtime
                    .join("node_modules/guard/deep/library.mjs"),
                b"runtime B",
            )
            .unwrap(),
            "runtimeAdded" => {
                fs::write(fixture.roots.runtime.join("added"), b"extra runtime").unwrap()
            }
            "binary" => fs::write(&fixture.roots.executable, b"changed executable bytes!").unwrap(),
            "compiler" => fs::write(&fixture.roots.compiler, b"changed compiler bytes!!").unwrap(),
            "profile" => {
                let mut changed = profile();
                changed["role"] = json!("DIFFERENT");
                fs::write(
                    &fixture.roots.profile,
                    serde_json::to_vec(&changed).unwrap(),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(envelope.verify_artifacts().is_err(), "{target}");
        assert!(
            Envelope::restore_snapshot(&fixture.roots, &bytes, &hash(&bytes)).is_err(),
            "{target}"
        );
    }
}

#[test]
fn envelope_requires_each_fixed_source_scope_including_local_dependency_patch() {
    let fixture = Fixture::new();
    fs::remove_file(
        fixture
            .roots
            .repository
            .join("src-tauri/vendor/included-source"),
    )
    .unwrap();
    fs::remove_dir(fixture.roots.repository.join("src-tauri/vendor")).unwrap();
    assert!(Envelope::capture(&fixture.roots, Plan::generate().unwrap(), fixture.window).is_err());
}

#[test]
fn envelope_windows_are_bounded_and_half_open() {
    let window = Window {
        not_before_unix_ms: 1000,
        expires_unix_ms: 1000 + WINDOW_MS,
    };
    assert!(window.check(999).is_err());
    assert!(window.check(1000).is_ok());
    assert!(window.check(1000 + WINDOW_MS - 1).is_ok());
    assert!(window.check(1000 + WINDOW_MS).is_err());
    for (start, end) in [
        (0, 1000),
        (1000, 1000),
        (1000, 999),
        (1000, 1001 + WINDOW_MS),
    ] {
        assert!(
            Window {
                not_before_unix_ms: start,
                expires_unix_ms: end
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn envelope_emission_stops_at_its_bound_before_retaining_excess_bytes() {
    let mut output = BoundedBytes(Vec::new());
    output.write_all(&vec![b'x'; MAX_ENVELOPE_BYTES]).unwrap();
    assert!(output.write_all(b"x").is_err());
    assert_eq!(output.0.len(), MAX_ENVELOPE_BYTES);
    let fixture = Fixture::new();
    let mut envelope = fixture.capture();
    // Exercise the actual serializer too: it cannot append a path beyond its
    // output cap, even if an internal future caller builds an oversized value.
    envelope.manifest.executable.path = PathBuf::from("x".repeat(MAX_ENVELOPE_BYTES));
    assert!(envelope.canonical_json().is_err());
}

#[test]
fn envelope_profile_is_private_exact_nonsecret_and_fixed_to_permitted_schema() {
    let fixture = Fixture::new();
    for (field, value) in [
        ("password", json!("synthetic-secret")),
        ("schema", json!("PUBLIC")),
        ("database", json!("OTHER")),
        ("role", json!("")),
    ] {
        let mut changed = profile();
        changed[field] = value;
        fs::write(
            &fixture.roots.profile,
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(
            Envelope::capture(&fixture.roots, Plan::generate().unwrap(), fixture.window).is_err()
        );
    }
    fs::write(
        &fixture.roots.profile,
        serde_json::to_vec(&profile()).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&fixture.roots.profile, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Envelope::capture(&fixture.roots, Plan::generate().unwrap(), fixture.window).is_err());
}

#[test]
fn envelope_same_account_and_run_stay_spent_across_changed_window_and_execution_hash() {
    let fixture = Fixture::new();
    let first = fixture.capture();
    let second = Envelope::capture(
        &fixture.roots,
        first.plan.clone(),
        Window {
            not_before_unix_ms: fixture.window.not_before_unix_ms + 1,
            expires_unix_ms: fixture.window.expires_unix_ms + 1,
        },
    )
    .unwrap();
    assert_ne!(first.sha256().unwrap(), second.sha256().unwrap());
    assert_eq!(first.run_key().unwrap(), second.run_key().unwrap());
    let store = fixture.store();
    let _spent = store
        .consume(&first.sha256().unwrap(), &first.run_key().unwrap())
        .unwrap();
    assert!(
        store
            .consume(&second.sha256().unwrap(), &second.run_key().unwrap())
            .is_err()
    );
    assert_eq!(fixture.markers(), 2);
}

#[test]
fn envelope_host_case_change_cannot_spend_the_same_account_run_twice() {
    let fixture = Fixture::new();
    let first = fixture.capture();
    let plan = first.plan.clone();
    let store = fixture.store();
    let _spent = store
        .consume(&first.sha256().unwrap(), &first.run_key().unwrap())
        .unwrap();

    let mut upper = profile();
    upper["host"] = json!(upper["host"].as_str().unwrap().to_ascii_uppercase());
    fs::write(&fixture.roots.profile, serde_json::to_vec(&upper).unwrap()).unwrap();
    let second = Envelope::capture(&fixture.roots, plan, fixture.window).unwrap();
    // Exact reviewed bytes still distinguish the two envelopes. Only the
    // permanent RUN identity must follow the driver's canonical host spelling.
    assert_ne!(first.sha256().unwrap(), second.sha256().unwrap());
    assert!(
        store
            .consume(&second.sha256().unwrap(), &second.run_key().unwrap())
            .is_err(),
        "hostname capitalization must not authorize a second run against the same account"
    );
    assert_eq!(fixture.markers(), 2);
    assert_eq!(first.run_key().unwrap(), second.run_key().unwrap());
}

#[test]
fn envelope_rejects_wrong_approval_and_alleged_executable_before_any_marker() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for field in [
        "format",
        "scope",
        "execution_sha256",
        "maximum_attempts",
        "extra",
    ] {
        let envelope = fixture.capture();
        let mut value: Value = serde_json::from_slice(&approval(&envelope)).unwrap();
        value[field] = json!("wrong");
        assert!(
            envelope
                .consume(&store, &serde_json::to_vec(&value).unwrap())
                .is_err()
        );
        assert_eq!(fixture.markers(), 0);
    }
    let envelope = fixture.capture();
    let grant = approval(&envelope);
    assert!(
        envelope.consume(&store, &grant).is_err(),
        "alleged executable differs from current process"
    );
    assert_eq!(fixture.markers(), 0);
}

#[test]
fn envelope_consumption_binds_running_binary_and_retains_sticky_drift_failure() {
    let mut fixture = Fixture::new();
    fixture.roots.executable = std::env::current_exe().unwrap();
    // Hash the actual test process. Runtime/compiler/source remain synthetic;
    // this is an offline capability test, never account qualification.
    let envelope = fixture.capture();
    let original_plan = envelope.plan.clone();
    let grant = approval(&envelope);
    let store = fixture.store();
    let consumed = envelope.consume(&store, &grant).unwrap();
    assert!(consumed.check().is_ok());
    assert_eq!(
        consumed.plan().canonical_json().unwrap(),
        original_plan.canonical_json().unwrap()
    );
    assert_eq!(consumed.frozen_profile().unwrap(), profile());
    assert_eq!(fixture.markers(), 2);
    let target = fixture
        .roots
        .runtime
        .join("node_modules/guard/deep/library.mjs");
    fs::write(&target, b"runtime B").unwrap();
    // Publication callbacks must perform no artifact IO under their locks.
    // Only the full revalidation discovers a changed runtime file.
    assert!(consumed.check().is_ok());
    assert!(consumed.revalidate().is_err());
    assert!(consumed.check().is_err());
    fs::write(&target, b"runtime A").unwrap();
    assert!(
        consumed.revalidate().is_err(),
        "restoring bytes cannot revive failed authority"
    );
    drop(consumed); // Simulates local launch failure; never refund either marker.
    let retry = Envelope::capture(&fixture.roots, original_plan, fixture.window).unwrap();
    // The same exact approval and restored runtime still cannot resume the
    // spent RUN after a local launch failure; no marker is refunded.
    assert!(retry.consume(&store, &grant).is_err());
    assert_eq!(fixture.markers(), 2);
}

#[test]
fn envelope_publication_checks_expiry_and_monotonic_deadline_without_artifact_io() {
    for monotonic in [false, true] {
        let (_directory, mut consumed, drift) =
            fixture_consumed(Plan::generate().unwrap(), profile());
        assert!(consumed.check().is_ok());
        fs::remove_file(drift).unwrap();
        assert!(
            consumed.check().is_ok(),
            "memory check cannot scan artifacts"
        );
        let original_window = consumed.envelope.manifest.window;
        let original_started = consumed.started;
        if monotonic {
            consumed.started = Instant::now().checked_sub(DEADLINE).unwrap();
        } else {
            let now = now_ms().unwrap();
            consumed.envelope.manifest.window = Window {
                not_before_unix_ms: now - 2000,
                expires_unix_ms: now - 1000,
            };
        }
        assert!(consumed.check().is_err());
        assert!(consumed.completion_deadline().is_err());
        consumed.envelope.manifest.window = original_window;
        consumed.started = original_started;
        assert!(
            consumed.check().is_err(),
            "restored time cannot revive the attempt"
        );
    }
}

#[test]
fn envelope_read_deadline_retains_consumption_time_and_shorter_wall_window() {
    let (_directory, mut consumed, drift) = fixture_consumed(Plan::generate().unwrap(), profile());
    // Missing artifact contents are deliberately invisible to this memory-only
    // wait bound; full admission still performs the independent inventory check.
    fs::remove_file(drift).unwrap();
    consumed.started = Instant::now()
        .checked_sub(DEADLINE - Duration::from_secs(1))
        .unwrap();
    let lifecycle = consumed.started.checked_add(DEADLINE).unwrap();
    assert_eq!(consumed.completion_deadline().unwrap(), lifecycle);
    assert_eq!(consumed.completion_deadline().unwrap(), lifecycle);

    consumed.started = Instant::now();
    let now = now_ms().unwrap();
    consumed.envelope.manifest.window = Window {
        not_before_unix_ms: now - 1,
        expires_unix_ms: now + 60_000,
    };
    let deadline = consumed.completion_deadline().unwrap();
    assert!(deadline <= Instant::now() + Duration::from_secs(60));
    assert!(deadline < consumed.started.checked_add(DEADLINE).unwrap());
}
