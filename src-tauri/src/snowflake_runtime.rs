//! Fixed resource selection for the Snowflake SDK helper.
//!
//! Validation is blocking (including hashing), so constructors and new_launch
//! must run outside Studio's configuration gate. Reserve cleanup capacity first;
//! new_launch_tracked attaches its observer before fallible initialization.
//!
//! The pins cover Node, helper sources, and the build's package-lock digest.
//! They do not authenticate every dependency file. Dependency provenance and
//! packaged code signing remain separate build/release checks. These checks
//! also do not make a caller-writable development tree immutable.
#![cfg(unix)]

use crate::{drivers::snowflake_process::Launch, snowflake_cleanup::CleanupPermit};
use serde::{Deserialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, Metadata},
    io::{self, Read},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

const FORMAT: &str = "joinery-snowflake-runtime-v1";
const TARGET: &str = "darwin-arm64";
const NODE_VERSION: &str = "24.22.2";
const NODE_SHA256: &str = "b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a";
const LOCK_SHA256: &str = "e23933ecb16badf5e9621cb089796193388493f9230976fa4b8c48efa9f13098";
const SDK_VERSION: &str = "3.4.1";
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_PACKAGE_BYTES: u64 = 64 * 1024;
const MAX_HELPER_BYTES: u64 = 1024 * 1024;
const MAX_NODE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TREE_ENTRIES: usize = 25_000;
const MAX_TREE_DEPTH: usize = 64;

/// Paths are selected only by native startup. There is no executable, script,
/// environment, profile, or IPC override and no alternate-route fallback.
#[derive(Clone, Debug)]
pub struct SnowflakeRuntime {
    root: PathBuf,
    state_parent: PathBuf,
    pins: ExpectedPins,
}

impl SnowflakeRuntime {
    /// Native startup only. Creates an application-owned private namespace;
    /// existing unsafe directories are rejected rather than repaired in place.
    pub fn prepare_state_parent(path: &Path) -> io::Result<()> {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        checked_directory(path)?;
        if fs::symlink_metadata(path)?.permissions().mode() & 0o777 != 0o700 {
            return Err(invalid("Snowflake state parent must be private"));
        }
        Ok(())
    }
    /// `resource_dir` comes from Tauri's native resource resolver, not IPC.
    /// `state_parent` must already be a private/trusted native cache directory.
    pub fn from_packaged(resource_dir: &Path, state_parent: &Path) -> io::Result<Self> {
        require_supported_host()?;
        Self::load_packaged(resource_dir, state_parent, ExpectedPins::compiled()?)
    }

    /// Explicit development route. Release builds cannot select the checkout.
    #[cfg(debug_assertions)]
    pub fn from_development(state_parent: &Path) -> io::Result<Self> {
        require_supported_host()?;
        Self::load(development_root(), state_parent, ExpectedPins::compiled()?)
    }

    fn load_packaged(
        resource_dir: &Path,
        state_parent: &Path,
        pins: ExpectedPins,
    ) -> io::Result<Self> {
        let resources = checked_directory(resource_dir)?;
        Self::load(resources.join("snowflake-runtime"), state_parent, pins)
    }

    fn load(root: PathBuf, state_parent: &Path, pins: ExpectedPins) -> io::Result<Self> {
        let root = checked_directory(&root)?;
        validate_resources(&root, &pins)?;
        let state_parent = checked_directory(state_parent)?;
        // State must never be placed inside the packaged/helper resource tree.
        if state_parent.starts_with(&root) || root.starts_with(&state_parent) {
            return Err(invalid("Snowflake state and resources must be separate"));
        }
        Ok(Self {
            root,
            state_parent,
            pins,
        })
    }

    /// Recheck resources before each launch; do not hold Studio's gate here.
    /// No process starts until the caller passes this Launch to the supervisor.
    pub fn new_launch(&self) -> io::Result<Launch> {
        self.allocate_launch(|_| {})
    }

    pub fn new_launch_tracked(&self, permit: CleanupPermit) -> io::Result<Launch> {
        self.allocate_launch(|observer| permit.attach(observer))
    }

    fn allocate_launch(
        &self,
        attach: impl FnOnce(crate::drivers::snowflake_process::CleanupObserver),
    ) -> io::Result<Launch> {
        validate_resources(&self.root, &self.pins)?;
        checked_directory(&self.state_parent)?;
        if fs::symlink_metadata(&self.state_parent)?
            .permissions()
            .mode()
            & 0o777
            != 0o700
        {
            return Err(invalid("Snowflake state parent must be private"));
        }
        let state = tempfile::Builder::new()
            .prefix("snowflake-")
            .tempdir_in(&self.state_parent)?;
        // Launch::new enforces 0700 and owns the no-child cleanup receipt before
        // any spawn future exists. Dropping it removes this exact directory.
        Launch::new_observed(
            self.root.join("bin/node"),
            Some(self.root.join("helper/main.mjs")),
            self.root.join("helper"),
            state,
            attach,
        )
    }
}

fn require_supported_host() -> io::Result<()> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Snowflake SDK runtime is only supported on macOS ARM64",
        ))
    }
}

#[cfg(debug_assertions)]
fn development_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("native manifest has a repository parent")
        .join(".cache/snowflake-runtime/macos-arm64")
}

#[derive(Clone, Debug)]
struct ExpectedPins {
    node_sha256: String,
    lock_sha256: String,
    sources: [(&'static str, String); 6],
}

impl ExpectedPins {
    fn compiled() -> io::Result<Self> {
        let lock_sha256 = digest_bytes(include_bytes!("../../snowflake-helper/package-lock.json"));
        if lock_sha256 != LOCK_SHA256 {
            return Err(invalid("Compiled Snowflake package-lock pin is stale"));
        }
        Ok(Self {
            node_sha256: NODE_SHA256.into(),
            lock_sha256,
            sources: [
                (
                    "main.mjs",
                    digest_bytes(include_bytes!("../../snowflake-helper/src/main.mjs")),
                ),
                (
                    "helper.mjs",
                    digest_bytes(include_bytes!("../../snowflake-helper/src/helper.mjs")),
                ),
                (
                    "framing.mjs",
                    digest_bytes(include_bytes!("../../snowflake-helper/src/framing.mjs")),
                ),
                (
                    "query-submit-guard.mjs",
                    digest_bytes(include_bytes!(
                        "../../snowflake-helper/src/query-submit-guard.mjs"
                    )),
                ),
                (
                    "transport-source-pins.mjs",
                    digest_bytes(include_bytes!(
                        "../../snowflake-helper/src/transport-source-pins.mjs"
                    )),
                ),
                (
                    "disposable-setup-ledger.mjs",
                    digest_bytes(include_bytes!(
                        "../../snowflake-helper/src/disposable-setup-ledger.mjs"
                    )),
                ),
            ],
        })
    }
}

#[derive(Deserialize)]
struct Manifest {
    format: String,
    target: String,
    runtime: RuntimeManifest,
    helper: HelperManifest,
}

#[derive(Deserialize)]
struct RuntimeManifest {
    version: String,
    platform: String,
    arch: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperManifest {
    package_name: String,
    package_version: String,
    sdk_version: String,
    package_lock_sha256: String,
    source_sha256: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct SdkPackage {
    name: String,
    version: String,
}

fn validate_resources(root: &Path, pins: &ExpectedPins) -> io::Result<()> {
    checked_directory(root)?;
    for directory in ["bin", "helper", "helper/node_modules", "notices"] {
        checked_directory(&root.join(directory))?;
    }
    check_tree(root)?;
    for file in [
        "manifest.json",
        "bin/node",
        "helper/main.mjs",
        "helper/helper.mjs",
        "helper/framing.mjs",
        "helper/query-submit-guard.mjs",
        "helper/transport-source-pins.mjs",
        "helper/disposable-setup-ledger.mjs",
        "helper/node_modules/snowflake-sdk/package.json",
    ] {
        let metadata = fs::symlink_metadata(root.join(file))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid(
                "Required Snowflake resource must be a regular file",
            ));
        }
    }
    let node = root.join("bin/node");
    if fs::metadata(&node)?.permissions().mode() & 0o100 == 0 {
        return Err(invalid("Packaged Node must be executable by its owner"));
    }
    let manifest: Manifest = read_json(&root.join("manifest.json"), MAX_MANIFEST_BYTES)?;
    if manifest.format != FORMAT
        || manifest.target != TARGET
        || manifest.runtime.version != NODE_VERSION
        || manifest.runtime.platform != "darwin"
        || manifest.runtime.arch != "arm64"
        || manifest.runtime.sha256 != pins.node_sha256
        || manifest.helper.package_name != "joinery-snowflake-helper"
        || manifest.helper.package_version != "0.0.0"
        || manifest.helper.sdk_version != SDK_VERSION
        || manifest.helper.package_lock_sha256 != pins.lock_sha256
        || manifest.helper.source_sha256.len() != pins.sources.len()
    {
        return Err(invalid(
            "Snowflake runtime manifest does not match compiled pins",
        ));
    }
    if digest_file(&node, MAX_NODE_BYTES)? != pins.node_sha256 {
        return Err(invalid("Packaged Node digest does not match compiled pin"));
    }
    for (name, expected) in &pins.sources {
        if manifest.helper.source_sha256.get(*name) != Some(expected)
            || digest_file(&root.join("helper").join(name), MAX_HELPER_BYTES)? != *expected
        {
            return Err(invalid(
                "Snowflake helper source does not match compiled pin",
            ));
        }
    }
    let sdk: SdkPackage = read_json(
        &root.join("helper/node_modules/snowflake-sdk/package.json"),
        MAX_PACKAGE_BYTES,
    )?;
    if sdk.name != "snowflake-sdk" || sdk.version != SDK_VERSION {
        return Err(invalid(
            "Installed Snowflake SDK version does not match pin",
        ));
    }
    Ok(())
}

fn checked_directory(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid("Snowflake native directories must be absolute"));
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid("Snowflake native directory must not be a symlink"));
    }
    check_mode(&metadata)?;
    fs::canonicalize(path)
}

fn check_mode(metadata: &Metadata) -> io::Result<()> {
    let mode = metadata.permissions().mode();
    if mode & 0o7022 != 0 || mode & 0o400 == 0 || (metadata.is_dir() && mode & 0o100 == 0) {
        return Err(invalid("Snowflake resource has unsafe permissions"));
    }
    Ok(())
}

fn check_tree(root: &Path) -> io::Result<()> {
    let modules = root.join("helper/node_modules");
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut count = 0;
    while let Some((directory, depth)) = pending.pop() {
        if depth > MAX_TREE_DEPTH {
            return Err(invalid("Snowflake resource tree exceeds depth bound"));
        }
        for entry in fs::read_dir(directory)? {
            count += 1;
            if count > MAX_TREE_ENTRIES {
                return Err(invalid("Snowflake resource tree exceeds entry bound"));
            }
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                // npm's .bin links are relocatable and remain strictly inside
                // node_modules. Never traverse a link as a directory.
                if !path.starts_with(&modules) || fs::read_link(&path)?.is_absolute() {
                    return Err(invalid("Snowflake resource symlink is not permitted"));
                }
                let target = fs::canonicalize(&path)?;
                if !target.starts_with(&modules) || target == modules {
                    return Err(invalid("Snowflake dependency symlink escapes node_modules"));
                }
                let target_metadata = fs::metadata(target)?;
                if !target_metadata.is_file() {
                    return Err(invalid("Snowflake dependency symlink must target a file"));
                }
                check_mode(&target_metadata)?;
            } else {
                check_mode(&metadata)?;
                if metadata.is_dir() {
                    pending.push((path, depth + 1));
                } else if !metadata.is_file() {
                    return Err(invalid("Snowflake resources cannot contain special files"));
                }
            }
        }
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path, bound: u64) -> io::Result<T> {
    let bytes = read_bounded(path, bound)?;
    serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid Snowflake runtime JSON"))
}

fn read_bounded(path: &Path, bound: u64) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > bound {
        return Err(invalid("Snowflake resource exceeds size bound"));
    }
    let mut bytes = Vec::new();
    file.take(bound + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > bound {
        return Err(invalid("Snowflake resource exceeds size bound"));
    }
    Ok(bytes)
}

fn digest_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn digest_file(path: &Path, bound: u64) -> io::Result<String> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > bound {
        return Err(invalid("Snowflake resource exceeds size bound"));
    }
    let mut reader = file.take(bound + 1);
    let mut digest = Sha256::new();
    let mut length = 0;
    let mut buffer = [0; 32 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        length += count as u64;
        if length > bound {
            return Err(invalid("Snowflake resource exceeds size bound"));
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::os::unix::fs::symlink;

    struct Fixture {
        directory: tempfile::TempDir,
        resources: PathBuf,
        root: PathBuf,
        state: PathBuf,
        pins: ExpectedPins,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            // macOS may spell the temporary parent through /var or /private/var.
            let base = fs::canonicalize(directory.path()).unwrap();
            let resources = base.join("Resources");
            let root = resources.join("snowflake-runtime");
            let state = base.join("state");
            for path in [
                root.join("bin"),
                root.join("helper/node_modules/snowflake-sdk"),
                root.join("notices"),
                state.clone(),
            ] {
                fs::create_dir_all(path).unwrap();
            }
            fs::write(root.join("bin/node"), b"synthetic node, never executed").unwrap();
            fs::set_permissions(root.join("bin/node"), fs::Permissions::from_mode(0o755)).unwrap();
            let sources = [
                ("main.mjs", "synthetic main"),
                ("helper.mjs", "synthetic helper"),
                ("framing.mjs", "synthetic framing"),
                ("query-submit-guard.mjs", "synthetic guard"),
                ("transport-source-pins.mjs", "synthetic source verifier"),
                ("disposable-setup-ledger.mjs", "synthetic setup ledger"),
            ];
            for (name, source) in sources {
                fs::write(root.join("helper").join(name), source).unwrap();
            }
            let pins = ExpectedPins {
                node_sha256: digest_bytes(b"synthetic node, never executed"),
                lock_sha256: digest_bytes(b"synthetic build lock"),
                sources: sources.map(|(name, source)| (name, digest_bytes(source.as_bytes()))),
            };
            let manifest = json!({
                "format": FORMAT, "target": TARGET,
                "runtime": {"version": NODE_VERSION, "platform": "darwin", "arch": "arm64", "sha256": pins.node_sha256},
                "helper": {
                    "packageName": "joinery-snowflake-helper", "packageVersion": "0.0.0",
                    "sdkVersion": SDK_VERSION, "packageLockSha256": pins.lock_sha256,
                    "sourceSha256": pins.sources.iter().cloned().collect::<BTreeMap<_, _>>()
                },
                "notices": {"disclaimer": "Not a dependency integrity manifest"}
            });
            fs::write(
                root.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            fs::write(
                root.join("helper/node_modules/snowflake-sdk/package.json"),
                br#"{"name":"snowflake-sdk","version":"3.4.1"}"#,
            )
            .unwrap();
            for path in [
                resources.clone(),
                root.clone(),
                root.join("bin"),
                root.join("helper"),
                root.join("helper/node_modules"),
                root.join("helper/node_modules/snowflake-sdk"),
                root.join("notices"),
                state.clone(),
            ] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            Self {
                directory,
                resources,
                root,
                state,
                pins,
            }
        }

        fn load(&self) -> io::Result<SnowflakeRuntime> {
            SnowflakeRuntime::load_packaged(&self.resources, &self.state, self.pins.clone())
        }

        fn change_manifest(&self, pointer: &str, value: Value) {
            let path = self.root.join("manifest.json");
            let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            *manifest.pointer_mut(pointer).unwrap() = value;
            fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        }
    }

    #[test]
    fn each_launch_owns_a_distinct_private_root_and_no_child_receipt() {
        let fixture = Fixture::new();
        let runtime = fixture.load().unwrap();
        let first = runtime.new_launch().unwrap();
        let second = runtime.new_launch().unwrap();
        let first_root = first.state_directory().to_owned();
        let second_root = second.state_directory().to_owned();
        assert_ne!(first_root, second_root);
        assert!(first_root.starts_with(&fixture.state));
        assert_eq!(
            fs::metadata(&first_root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(first.executable, fixture.root.join("bin/node"));
        assert_eq!(first.script, Some(fixture.root.join("helper/main.mjs")));
        assert_eq!(first.working_directory, fixture.root.join("helper"));
        let receipt = first.cleanup_observer();
        assert!(receipt.report().is_none());
        drop(first);
        let report = receipt.report().unwrap();
        assert!(!report.child_started);
        assert!(!report.reaped);
        assert!(report.temporary_root_removed);
        assert!(!first_root.exists());
        assert!(second_root.exists());
        drop(second);
        assert!(!second_root.exists());
    }

    #[test]
    fn required_resources_cannot_be_missing() {
        for file in [
            "manifest.json",
            "bin/node",
            "helper/main.mjs",
            "helper/helper.mjs",
            "helper/framing.mjs",
            "helper/query-submit-guard.mjs",
            "helper/transport-source-pins.mjs",
            "helper/disposable-setup-ledger.mjs",
            "helper/node_modules/snowflake-sdk/package.json",
        ] {
            let fixture = Fixture::new();
            fs::remove_file(fixture.root.join(file)).unwrap();
            assert!(fixture.load().is_err(), "{file}");
        }
    }

    #[test]
    fn rewritten_manifest_cannot_authorize_modified_node_or_helper_sources() {
        for file in [
            "bin/node",
            "helper/main.mjs",
            "helper/helper.mjs",
            "helper/framing.mjs",
            "helper/query-submit-guard.mjs",
            "helper/transport-source-pins.mjs",
            "helper/disposable-setup-ledger.mjs",
        ] {
            let fixture = Fixture::new();
            fs::write(fixture.root.join(file), b"replacement").unwrap();
            let pointer = if file == "bin/node" {
                "/runtime/sha256".to_owned()
            } else {
                format!(
                    "/helper/sourceSha256/{}",
                    Path::new(file).file_name().unwrap().to_str().unwrap()
                )
            };
            fixture.change_manifest(&pointer, json!(digest_bytes(b"replacement")));
            assert!(fixture.load().is_err(), "{file}");
        }
    }

    #[test]
    fn all_manifest_identity_and_version_pins_are_checked() {
        for pointer in [
            "/format",
            "/target",
            "/runtime/version",
            "/runtime/platform",
            "/runtime/arch",
            "/runtime/sha256",
            "/helper/packageName",
            "/helper/packageVersion",
            "/helper/sdkVersion",
            "/helper/packageLockSha256",
            "/helper/sourceSha256/main.mjs",
        ] {
            let fixture = Fixture::new();
            fixture.change_manifest(pointer, json!("wrong"));
            assert!(fixture.load().is_err(), "{pointer}");
        }
    }

    #[test]
    fn installed_sdk_identity_and_version_must_match() {
        for package in [
            json!({"name": "different-sdk", "version": SDK_VERSION}),
            json!({"name": "snowflake-sdk", "version": "0.0.0"}),
        ] {
            let fixture = Fixture::new();
            fs::write(
                fixture
                    .root
                    .join("helper/node_modules/snowflake-sdk/package.json"),
                serde_json::to_vec(&package).unwrap(),
            )
            .unwrap();
            assert!(fixture.load().is_err());
        }
    }

    #[test]
    fn malformed_oversized_and_wrongly_typed_json_is_rejected() {
        for contents in [b"{".to_vec(), vec![b' '; MAX_MANIFEST_BYTES as usize + 1]] {
            let fixture = Fixture::new();
            fs::write(fixture.root.join("manifest.json"), contents).unwrap();
            assert!(fixture.load().is_err());
        }
        let fixture = Fixture::new();
        fixture.change_manifest("/runtime/version", Value::Null);
        assert!(fixture.load().is_err());
        let fixture = Fixture::new();
        fs::write(
            fixture
                .root
                .join("helper/node_modules/snowflake-sdk/package.json"),
            vec![b' '; MAX_PACKAGE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(fixture.load().is_err());
    }

    #[test]
    fn only_contained_dependency_file_symlinks_are_allowed() {
        let fixture = Fixture::new();
        let modules = fixture.root.join("helper/node_modules");
        fs::create_dir(modules.join(".bin")).unwrap();
        symlink(
            "../snowflake-sdk/package.json",
            modules.join(".bin/contained"),
        )
        .unwrap();
        fixture.load().unwrap();
        fs::write(fixture.resources.join("outside.json"), b"outside runtime").unwrap();
        for target in [
            "../../main.mjs",
            "../../../manifest.json",
            "../../../../outside.json",
            "../snowflake-sdk",
            "missing",
        ] {
            let link = modules.join(".bin/invalid");
            symlink(target, &link).unwrap();
            assert!(fixture.load().is_err(), "{target}");
            fs::remove_file(link).unwrap();
        }
        symlink(fixture.root.join("manifest.json"), modules.join("absolute")).unwrap();
        assert!(fixture.load().is_err());
    }

    #[test]
    fn required_paths_and_resource_root_cannot_be_symlinks() {
        let fixture = Fixture::new();
        fs::rename(
            fixture.root.join("bin/node"),
            fixture.root.join("bin/real-node"),
        )
        .unwrap();
        symlink("real-node", fixture.root.join("bin/node")).unwrap();
        assert!(fixture.load().is_err());
        let fixture = Fixture::new();
        let moved = fixture.resources.join("elsewhere");
        fs::rename(&fixture.root, &moved).unwrap();
        symlink("elsewhere", &fixture.root).unwrap();
        assert!(fixture.load().is_err());
        let fixture = Fixture::new();
        symlink("main.mjs", fixture.root.join("helper/extra-link")).unwrap();
        assert!(fixture.load().is_err());
    }

    #[test]
    fn unsafe_modes_and_non_executable_node_are_rejected() {
        for (path, mode) in [
            ("helper", 0o775),
            ("helper/main.mjs", 0o666),
            ("bin/node", 0o644),
            ("bin/node", 0o4755),
        ] {
            let fixture = Fixture::new();
            fs::set_permissions(fixture.root.join(path), fs::Permissions::from_mode(mode)).unwrap();
            assert!(fixture.load().is_err(), "{path} {mode:o}");
        }
    }

    #[test]
    fn dependency_executables_are_allowed_but_special_files_are_not() {
        let fixture = Fixture::new();
        let file = fixture
            .root
            .join("helper/node_modules/other-architecture-binary");
        fs::write(&file, b"not Node, never executed by these tests").unwrap();
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
        fixture.load().unwrap();
        // Bind at a short path before moving into the scanned tree: macOS Unix
        // socket addresses have a much smaller bound than normal file paths.
        let socket_path = fixture.directory.path().join("socket");
        let _socket = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        fs::rename(socket_path, fixture.root.join("helper/socket")).unwrap();
        assert!(fixture.load().is_err());
    }

    #[test]
    fn factory_rechecks_resources_before_each_launch() {
        let fixture = Fixture::new();
        let runtime = fixture.load().unwrap();
        fs::write(
            fixture.root.join("helper/helper.mjs"),
            b"changed after validation",
        )
        .unwrap();
        assert!(runtime.new_launch().is_err());
        assert_eq!(fs::read_dir(&fixture.state).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn tracked_factory_validation_failure_settles_without_allocating_a_root() {
        let fixture = Fixture::new();
        let runtime = fixture.load().unwrap();
        let cleanup = crate::snowflake_cleanup::SnowflakeCleanup::default();
        let permit = cleanup.reserve("attempt").unwrap();
        fs::write(fixture.root.join("helper/helper.mjs"), b"tampered").unwrap();
        assert!(runtime.new_launch_tracked(permit).is_err());
        cleanup
            .wait_all(std::time::Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(fs::read_dir(&fixture.state).unwrap().count(), 0);
    }

    #[test]
    fn native_state_parent_is_private_and_rejects_existing_unsafe_paths() {
        let fixture = Fixture::new();
        let fresh = fixture.directory.path().join("new-private-state");
        SnowflakeRuntime::prepare_state_parent(&fresh).unwrap();
        assert_eq!(
            fs::metadata(&fresh).unwrap().permissions().mode() & 0o777,
            0o700
        );
        SnowflakeRuntime::prepare_state_parent(&fresh).unwrap();
        fs::set_permissions(&fresh, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(SnowflakeRuntime::prepare_state_parent(&fresh).is_err());
        let link = fixture.directory.path().join("state-link");
        symlink(&fixture.state, &link).unwrap();
        assert!(SnowflakeRuntime::prepare_state_parent(&link).is_err());
    }

    #[test]
    fn selected_packaged_route_never_falls_back_to_another_tree() {
        let fixture = Fixture::new();
        let other_resources = fixture.resources.join("other");
        fs::create_dir(&other_resources).unwrap();
        fs::rename(&fixture.root, other_resources.join("snowflake-runtime")).unwrap();
        SnowflakeRuntime::load_packaged(&other_resources, &fixture.state, fixture.pins.clone())
            .unwrap();
        assert!(fixture.load().is_err());
        assert!(
            SnowflakeRuntime::load_packaged(
                Path::new("relative"),
                &fixture.state,
                fixture.pins.clone()
            )
            .is_err()
        );
    }

    #[test]
    fn state_parent_must_be_separate_real_and_safe() {
        let fixture = Fixture::new();
        assert!(
            SnowflakeRuntime::load_packaged(
                &fixture.resources,
                &fixture.root.join("helper"),
                fixture.pins.clone()
            )
            .is_err()
        );
        fs::set_permissions(&fixture.state, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(fixture.load().is_err());
        let fixture = Fixture::new();
        let real_state = fixture.state.with_extension("real");
        fs::rename(&fixture.state, &real_state).unwrap();
        symlink(&real_state, &fixture.state).unwrap();
        assert!(fixture.load().is_err());
    }

    #[test]
    fn node_size_bound_is_enforced_before_reading_large_resources() {
        let fixture = Fixture::new();
        File::options()
            .write(true)
            .open(fixture.root.join("bin/node"))
            .unwrap()
            .set_len(MAX_NODE_BYTES + 1)
            .unwrap();
        assert!(fixture.load().is_err());
    }

    #[test]
    fn compiled_lock_matches_the_reviewed_pin() {
        let pins = ExpectedPins::compiled().unwrap();
        assert_eq!(pins.lock_sha256, LOCK_SHA256);
        assert_eq!(pins.node_sha256, NODE_SHA256);
        assert!(pins.sources.iter().all(|(_, digest)| digest.len() == 64));
    }

    #[test]
    #[cfg(debug_assertions)]
    fn development_route_is_derived_only_from_the_build_manifest() {
        assert_eq!(
            development_root(),
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join(".cache/snowflake-runtime/macos-arm64")
        );
    }
}

// Test-only resource tree: real core helper/guard and pinned Node, synthetic SDK
// and source verifier. Only private fixture pins change; production main has no
// profile, IPC or environment bypass. Actual transport proof uses the real SDK.
#[cfg(all(test, feature = "test-harness"))]
pub(crate) mod studio_fixture {
    use super::*;
    use serde_json::json;
    pub struct Fixture {
        pub directory: tempfile::TempDir,
        pub runtime: Arc<SnowflakeRuntime>,
        pub state: PathBuf,
        pub controls: PathBuf,
    }
    use std::sync::Arc;
    impl Fixture {
        pub fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let base = fs::canonicalize(directory.path()).unwrap();
            let root = base.join("resources/snowflake-runtime");
            let state = base.join("state");
            for path in [
                root.join("bin"),
                root.join("helper/node_modules/snowflake-sdk"),
                root.join("notices"),
                state.clone(),
            ] {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(path)
                    .unwrap();
            }
            let node = PathBuf::from(
                std::env::var_os("JOINERY_TEST_NODE")
                    .expect("run with explicit pinned JOINERY_TEST_NODE"),
            );
            assert!(node.is_absolute());
            fs::copy(&node, root.join("bin/node")).unwrap();
            fs::set_permissions(root.join("bin/node"), fs::Permissions::from_mode(0o755)).unwrap();
            let mut pins = ExpectedPins::compiled().unwrap();
            pins.node_sha256 = digest_file(&node, MAX_NODE_BYTES).unwrap();
            // This private fixture deliberately loads a synthetic SDK. Only
            // its test-owned verifier source/pin is replaced; production main
            // still calls the verifier and has no environment/IPC bypass.
            let synthetic_verifier = b"export function verifyTransportSources() {}\n";
            pins.sources
                .iter_mut()
                .find(|(name, _)| *name == "transport-source-pins.mjs")
                .unwrap()
                .1 = digest_bytes(synthetic_verifier);
            for (name, bytes) in [
                (
                    "main.mjs",
                    include_bytes!("../../snowflake-helper/src/main.mjs").as_slice(),
                ),
                (
                    "helper.mjs",
                    include_bytes!("../../snowflake-helper/src/helper.mjs").as_slice(),
                ),
                (
                    "framing.mjs",
                    include_bytes!("../../snowflake-helper/src/framing.mjs").as_slice(),
                ),
                (
                    "query-submit-guard.mjs",
                    include_bytes!("../../snowflake-helper/src/query-submit-guard.mjs").as_slice(),
                ),
                ("transport-source-pins.mjs", synthetic_verifier.as_slice()),
                (
                    "disposable-setup-ledger.mjs",
                    include_bytes!("../../snowflake-helper/src/disposable-setup-ledger.mjs")
                        .as_slice(),
                ),
            ] {
                fs::write(root.join("helper").join(name), bytes).unwrap();
            }
            fs::write(root.join("manifest.json"), serde_json::to_vec(&json!({
                "format": FORMAT, "target": TARGET,
                "runtime":{"version":NODE_VERSION,"platform":"darwin","arch":"arm64","sha256":pins.node_sha256},
                "helper":{"packageName":"joinery-snowflake-helper","packageVersion":"0.0.0","sdkVersion":SDK_VERSION,"packageLockSha256":pins.lock_sha256,"sourceSha256":pins.sources.iter().cloned().collect::<BTreeMap<_,_>>()}
            })).unwrap()).unwrap();
            fs::write(
                root.join("helper/node_modules/snowflake-sdk/package.json"),
                br#"{"name":"snowflake-sdk","version":"3.4.1","type":"module","main":"index.mjs"}"#,
            )
            .unwrap();
            fs::write(
                root.join("helper/node_modules/snowflake-sdk/index.mjs"),
                include_str!("execution/synthetic_snowflake.mjs"),
            )
            .unwrap();
            let controls = root.join("helper");
            let runtime = Arc::new(
                SnowflakeRuntime::load_packaged(&base.join("resources"), &state, pins).unwrap(),
            );
            Self {
                directory,
                runtime,
                state,
                controls,
            }
        }
    }
}
