//! Closed, offline identity for one application-core table qualification.
//!
//! Capture/restore never spawn or authenticate. Consumption records permanent
//! local markers but does not launch anything. The retained runner checks this
//! capability; the fixed account command remains incomplete. Hashes bind bytes,
//! not build provenance or immutable filesystem state; a same-user writer can
//! race between checks. Full checks run outside process locks.

use super::{
    MAX_SCHEMAS, Plan,
    consumption::{ConsumedAttempt, ConsumptionStore},
    inventory::{Inventory, InventoryKind, read_private_envelope, read_private_file},
    profile::{self, Input as Profile},
    sequence::Sequencer,
};
use crate::{
    domain::{AppError, AppResult},
    drivers::snowflake_qualification_transport::TransportLimits,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FORMAT: &str = "joinery-snowflake-table-lifecycle-envelope-v1";
const WORKFLOW: &str = "existing-schema-table-lifecycle-v1";
const APPROVAL_FORMAT: &str = "joinery-snowflake-table-lifecycle-approval-v1";
const APPROVAL_SCOPE: &str =
    "one-externalbrowser-attempt-fixed-table-lifecycle-with-possible-billing";
const MAX_ENVELOPE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PROFILE_BYTES: usize = 16 * 1024;
const WINDOW_MS: u64 = 30 * 60 * 1000;
const DEADLINE: Duration = Duration::from_secs(15 * 60);

// No caller-selected source list or ignore patterns. Include the full Rust
// source and local dependency patch, rather than relying on a clean HEAD.
const SOURCE_TREES: &[&str] = &["src-tauri/src", "src-tauri/vendor", "snowflake-helper/src"];
const SOURCE_FILES: &[&str] = &[
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
    "src-tauri/build.rs",
    "rust-toolchain.toml",
    "snowflake-helper/package.json",
    "snowflake-helper/package-lock.json",
    "scripts/prepare-snowflake-runtime.mjs",
    "scripts/verify-snowflake-runtime.mjs",
];

fn invalid() -> AppError {
    AppError::invalid("Table qualification envelope, approval or identity is invalid")
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn now_ms() -> AppResult<u64> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis(),
    )
    .map_err(|_| invalid())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Window {
    pub(super) not_before_unix_ms: u64,
    pub(super) expires_unix_ms: u64,
}

impl Window {
    pub(super) fn validate(self) -> AppResult<()> {
        if self.not_before_unix_ms == 0
            || self
                .expires_unix_ms
                .checked_sub(self.not_before_unix_ms)
                .is_none_or(|duration| duration == 0 || duration > WINDOW_MS)
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn check(self, now: u64) -> AppResult<()> {
        self.validate()?;
        if now < self.not_before_unix_ms || now >= self.expires_unix_ms {
            return Err(invalid());
        }
        Ok(())
    }
}

/// Physical paths supplied by the future fixed local entry, never restored from
/// unchecked manifest fields. Paths and complete inventories are bound below.
pub(super) struct Roots {
    pub(super) repository: PathBuf,
    pub(super) runtime: PathBuf,
    pub(super) executable: PathBuf,
    pub(super) compiler: PathBuf,
    pub(super) profile: PathBuf,
}

#[derive(Serialize)]
struct Artifact {
    path: PathBuf,
    inventory: Inventory,
}

impl Artifact {
    fn capture(path: &Path, kind: InventoryKind) -> AppResult<Self> {
        Ok(Self {
            path: path.to_owned(),
            inventory: Inventory::capture(path, kind)?,
        })
    }

    fn verify(&self) -> AppResult<()> {
        self.inventory.verify(&self.path)
    }
}

#[derive(Serialize)]
struct Manifest {
    format: &'static str,
    workflow: &'static str,
    plan_json: String,
    plan_sha256: String,
    window: Window,
    profile: Profile,
    policy: profile::Declaration,
    sources: Vec<Artifact>,
    runtime: Artifact,
    executable: Artifact,
    compiler: Artifact,
    profile_file: Artifact,
    build: BuildRecipe,
    limits: Limits,
}

#[derive(Serialize)]
struct BuildRecipe {
    command: &'static [&'static str],
    os: &'static str,
    architecture: &'static str,
    package_version: &'static str,
    attribution: &'static str,
}

#[derive(Serialize)]
struct Limits {
    authentication_seconds: u64,
    sql_admission_seconds: u64,
    workflow_seconds: u64,
    maximum_sql: u64,
    maximum_schemas: u64,
    readback_rows: u64,
    readback_bytes: u64,
    inbound_body_bytes: u64,
    inbound_wire_bytes: u64,
    inbound_frame_bytes: u64,
    inbound_frames: u64,
    stderr_bytes: u64,
}

pub(super) struct Envelope {
    manifest: Manifest,
    plan: Plan,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    format: String,
    execution_sha256: String,
    scope: String,
    maximum_attempts: u8,
}

/// Non-cloneable. The marker capability and checked envelope stay together.
/// The retained runner accepts this capability; fixed account construction is
/// a separate gate and may not adopt caller-assembled components.
pub(super) struct ConsumedEnvelope {
    envelope: Envelope,
    consumed: ConsumedAttempt,
    started: Instant,
    failure: Mutex<Option<AppError>>,
}

struct BoundedBytes(Vec<u8>);

impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_ENVELOPE_BYTES {
            return Err(io::Error::other("Qualification envelope size limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Envelope {
    pub(super) fn plan(&self) -> Plan {
        self.plan.clone()
    }

    /// Output must not mutate any reviewed artifact or contain its tree.
    /// AttemptStorage separately rejects nonphysical original paths.
    pub(super) fn require_separate_output(&self, output: &Path) -> AppResult<()> {
        let output = output.canonicalize().map_err(|_| invalid())?;
        for artifact in self.manifest.sources.iter().chain([
            &self.manifest.runtime,
            &self.manifest.executable,
            &self.manifest.compiler,
            &self.manifest.profile_file,
        ]) {
            if output.starts_with(&artifact.path) || artifact.path.starts_with(&output) {
                return Err(invalid());
            }
        }
        Ok(())
    }

    pub(super) fn capture(roots: &Roots, plan: Plan, window: Window) -> AppResult<Self> {
        window.validate()?;
        let mut sources = Vec::with_capacity(SOURCE_TREES.len() + SOURCE_FILES.len());
        for path in SOURCE_TREES {
            sources.push(Artifact::capture(
                &roots.repository.join(path),
                InventoryKind::Tree,
            )?);
        }
        for path in SOURCE_FILES {
            sources.push(Artifact::capture(
                &roots.repository.join(path),
                InventoryKind::File,
            )?);
        }
        let profile_file = Artifact::capture(&roots.profile, InventoryKind::File)?;
        let profile = read_profile(&roots.profile)?;
        profile_file.verify()?;
        // Reuse the actual closed transport profile contract; no journal or
        // authentication state is allocated by this pure validation.
        Sequencer::with_plan(
            plan.clone(),
            serde_json::to_value(&profile).map_err(|_| invalid())?,
        )?;
        let transport = TransportLimits::default();
        let manifest = Manifest {
            format: FORMAT,
            workflow: WORKFLOW,
            plan_json: String::from_utf8(plan.canonical_json()?).map_err(|_| invalid())?,
            plan_sha256: plan.sha256()?,
            window,
            profile,
            policy: profile::declaration(),
            sources,
            runtime: Artifact::capture(&roots.runtime, InventoryKind::Tree)?,
            executable: Artifact::capture(&roots.executable, InventoryKind::File)?,
            compiler: Artifact::capture(&roots.compiler, InventoryKind::File)?,
            profile_file,
            build: BuildRecipe {
                command: &[
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
                    "snowflake-table-qualification",
                ],
                os: std::env::consts::OS,
                architecture: std::env::consts::ARCH,
                package_version: env!("CARGO_PKG_VERSION"),
                attribution: "byte binding only; build provenance is not attested",
            },
            // Admission placement is a separate integration gate. These fixed
            // envelope values must not be described as account enforcement yet.
            limits: Limits {
                authentication_seconds: 120,
                sql_admission_seconds: 30,
                workflow_seconds: DEADLINE.as_secs(),
                maximum_sql: u64::from(plan.total_budget.fixed)
                    + u64::from(plan.total_budget.per_schema) * MAX_SCHEMAS as u64,
                maximum_schemas: MAX_SCHEMAS as u64,
                readback_rows: 2,
                readback_bytes: super::runner::READ_BYTES as u64,
                inbound_body_bytes: transport.payload_bytes,
                inbound_wire_bytes: transport.stdout_bytes,
                inbound_frame_bytes: transport.frame_bytes as u64,
                inbound_frames: transport.frames,
                stderr_bytes: transport.stderr_bytes,
            },
        };
        let envelope = Self { manifest, plan };
        envelope.verify_artifacts()?;
        envelope.canonical_json()?;
        Ok(envelope)
    }

    pub(super) fn canonical_json(&self) -> AppResult<Vec<u8>> {
        let mut output = BoundedBytes(Vec::new());
        serde_json::to_writer(&mut output, &self.manifest).map_err(|_| invalid())?;
        if output.0.is_empty() {
            return Err(invalid());
        }
        Ok(output.0)
    }

    pub(super) fn sha256(&self) -> AppResult<String> {
        Ok(hash(&self.canonical_json()?))
    }

    /// Extract only the closed Plan and window, then recapture all evidence.
    /// Caller-provided paths, hashes, limits and templates are never trusted.
    pub(super) fn restore_snapshot(
        roots: &Roots,
        bytes: &[u8],
        expected_sha256: &str,
    ) -> AppResult<Self> {
        if bytes.is_empty()
            || bytes.len() > MAX_ENVELOPE_BYTES
            || !is_hash(expected_sha256)
            || hash(bytes) != expected_sha256
        {
            return Err(invalid());
        }
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let plan = Plan::restore_snapshot(
            value["plan_json"].as_str().ok_or_else(invalid)?.as_bytes(),
            value["plan_sha256"].as_str().ok_or_else(invalid)?,
        )?;
        let window: Window =
            serde_json::from_value(value["window"].clone()).map_err(|_| invalid())?;
        let envelope = Self::capture(roots, plan, window)?;
        if envelope.canonical_json()? != bytes {
            return Err(invalid());
        }
        Ok(envelope)
    }

    /// Closed file route for the fixed entry. This alone allows the larger
    /// envelope bound; private connection and approval readers stay smaller.
    pub(super) fn restore_file(
        roots: &Roots,
        path: &Path,
        expected_sha256: &str,
    ) -> AppResult<Self> {
        let bytes = read_private_envelope(path)?;
        Self::restore_snapshot(roots, &bytes, expected_sha256)
    }

    fn verify_artifacts(&self) -> AppResult<()> {
        for artifact in &self.manifest.sources {
            artifact.verify()?;
        }
        for artifact in [
            &self.manifest.runtime,
            &self.manifest.executable,
            &self.manifest.compiler,
            &self.manifest.profile_file,
        ] {
            artifact.verify()?;
        }
        Ok(())
    }

    fn run_key(&self) -> AppResult<String> {
        let profile = &self.manifest.profile;
        // The driver lowercases the host before deriving the account. Bind its
        // canonical identity here so a spelling change cannot refund the RUN;
        // the execution envelope still binds the exact reviewed profile bytes.
        let host = profile.host.to_ascii_lowercase();
        // An array avoids delimiter ambiguity. Do not include the approval
        // window/source hash: changing either must not permit a second RUN.
        Ok(hash(
            &serde_json::to_vec(&[
                WORKFLOW,
                &profile.account,
                &host,
                &profile.database,
                &profile.schema,
                self.plan.run(),
            ])
            .map_err(|_| invalid())?,
        ))
    }

    pub(super) fn consume(
        self,
        store: &ConsumptionStore,
        approval_json: &[u8],
    ) -> AppResult<ConsumedEnvelope> {
        if approval_json.is_empty() || approval_json.len() > 4096 {
            return Err(invalid());
        }
        let approval: Approval = serde_json::from_slice(approval_json).map_err(|_| invalid())?;
        let execution_sha256 = self.sha256()?;
        if approval.format != APPROVAL_FORMAT
            || approval.scope != APPROVAL_SCOPE
            || approval.maximum_attempts != 1
            || approval.execution_sha256 != execution_sha256
        {
            return Err(invalid());
        }
        // It is the actually running executable, not merely a file supplied as
        // an alleged binary. No authentication/launch is performed here.
        let running = std::env::current_exe().map_err(|_| invalid())?;
        if running != self.manifest.executable.path {
            return Err(invalid());
        }
        self.manifest.window.check(now_ms()?)?;
        self.verify_artifacts()?;
        self.manifest.window.check(now_ms()?)?;
        let started = Instant::now();
        let consumed = store.consume(&execution_sha256, &self.run_key()?)?;
        let checked = ConsumedEnvelope {
            envelope: self,
            consumed,
            started,
            failure: Mutex::new(None),
        };
        // If revalidation fails, the permanent markers remain spent. There is
        // no resume path or retry on a freshly changed approval window.
        checked.revalidate()?;
        Ok(checked)
    }
}

impl ConsumedEnvelope {
    /// Only the consumed identity selects construction inputs. The caller must
    /// fully revalidate before constructing and before handing off to Runner.
    pub(super) fn construction_inputs(&self) -> AppResult<(Profile, PathBuf)> {
        self.check()?;
        Ok((
            self.envelope.manifest.profile.clone(),
            self.envelope.manifest.runtime.path.clone(),
        ))
    }

    #[cfg(test)]
    pub(super) fn fixture_elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    #[cfg(test)]
    pub(super) fn fixture_runtime_root(&self) -> &Path {
        &self.envelope.manifest.runtime.path
    }

    pub(super) fn plan(&self) -> Plan {
        self.envelope.plan.clone()
    }

    pub(super) fn frozen_profile(&self) -> AppResult<serde_json::Value> {
        serde_json::to_value(&self.envelope.manifest.profile).map_err(|_| invalid())
    }

    /// Absolute read-completion bound. Repeated reads cannot renew the consumed
    /// lifecycle, and the approval window can only shorten the remaining wait.
    /// Like publication checks, this performs no artifact IO.
    pub(super) fn completion_deadline(&self) -> AppResult<Instant> {
        self.check()?;
        let deadline = (|| {
            let remaining_ms = self
                .envelope
                .manifest
                .window
                .expires_unix_ms
                .checked_sub(now_ms()?)
                .filter(|remaining| *remaining > 0)
                .ok_or_else(invalid)?;
            let wall = Instant::now()
                .checked_add(Duration::from_millis(remaining_ms))
                .ok_or_else(invalid)?;
            let lifecycle = self.started.checked_add(DEADLINE).ok_or_else(invalid)?;
            let deadline = wall.min(lifecycle);
            if deadline <= Instant::now() {
                return Err(invalid());
            }
            Ok(deadline)
        })();
        deadline.map_err(|error| self.fail(error))
    }

    /// Publication-safe time/failure check. Never reads artifact files or markers.
    pub(super) fn check(&self) -> AppResult<()> {
        let mut failure = self.failure.lock().map_err(|_| invalid())?;
        if failure.is_none() {
            let result = if self.started.elapsed() >= DEADLINE {
                Err(invalid())
            } else {
                now_ms().and_then(|now| self.envelope.manifest.window.check(now))
            };
            if let Err(error) = result {
                *failure = Some(error);
            }
        }
        failure.clone().map_or(Ok(()), Err)
    }

    /// Latch-only revocation; no filesystem or caller reentry.
    pub(super) fn fail(&self, error: AppError) -> AppError {
        match self.failure.lock() {
            Ok(mut failure) => failure.get_or_insert(error).clone(),
            Err(_) => invalid(),
        }
    }

    /// Blocking IO, outside Studio/process/policy locks. Failure is sticky;
    /// restoring changed bytes later cannot revive this capability.
    pub(super) fn revalidate(&self) -> AppResult<()> {
        self.check()?;
        let result = (|| {
            if self.started.elapsed() >= DEADLINE
                || self.consumed.execution_sha256() != self.envelope.sha256()?
                || self.consumed.run_key_sha256() != self.envelope.run_key()?
            {
                return Err(invalid());
            }
            self.envelope.manifest.window.check(now_ms()?)?;
            self.consumed.revalidate()?;
            self.envelope.verify_artifacts()?;
            self.envelope.manifest.window.check(now_ms()?)?;
            if self.started.elapsed() >= DEADLINE {
                return Err(invalid());
            }
            Ok(())
        })();
        let mut failure = self.failure.lock().map_err(|_| invalid())?;
        if let Err(error) = result {
            failure.get_or_insert(error);
        }
        failure.clone().map_or(Ok(()), Err)
    }
}

fn read_profile(path: &Path) -> AppResult<Profile> {
    let bytes = read_private_file(path, MAX_PROFILE_BYTES)?;
    if bytes.is_empty() {
        return Err(invalid());
    }
    let profile: Profile = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    profile.validate()?;
    Ok(profile)
}

#[cfg(test)]
pub(super) fn fixture_unconsumed(
    plan: Plan,
    profile: serde_json::Value,
    runtime_root: Option<PathBuf>,
) -> (tempfile::TempDir, Envelope, ConsumptionStore, Vec<u8>) {
    tests::fixture_unconsumed(plan, profile, runtime_root)
}

#[cfg(test)]
pub(super) fn fixture_consumed(
    plan: Plan,
    profile: serde_json::Value,
) -> (tempfile::TempDir, ConsumedEnvelope, PathBuf) {
    tests::fixture_consumed(plan, profile)
}

#[cfg(test)]
pub(super) fn fixture_consumed_real_content(
    plan: Plan,
    profile: serde_json::Value,
    runtime_root: PathBuf,
) -> (tempfile::TempDir, ConsumedEnvelope, serde_json::Value) {
    perf_tests::fixture_consumed_real_content(plan, profile, runtime_root)
}

#[cfg(test)]
#[path = "snowflake_table_qualification_envelope_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "snowflake_table_qualification_envelope_perf_tests.rs"]
mod perf_tests;
