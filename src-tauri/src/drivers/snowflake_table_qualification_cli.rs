//! Offline-only facade. No construction, consumption, process launch, credential
//! provider or account route is reachable from the closed command dispatch.

use super::{
    DATABASE, MAX_SCHEMAS, PHASE_COUNT, Plan, SCHEMA,
    emission::Emission,
    envelope::{Envelope, Roots, Window},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

/// Closed failure codes only. Never serialize input paths, connection data or
/// underlying filesystem/profile errors to stdout/stderr.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineCommandFailure {
    failure_code: &'static str,
}

impl OfflineCommandFailure {
    fn new(failure_code: &'static str) -> Self {
        Self { failure_code }
    }
    fn arguments() -> Self {
        Self::new("invalid_arguments")
    }
}

enum Command {
    Help,
    Emit {
        roots: InputRoots,
        window: Window,
        output_parent: PathBuf,
    },
    Verify {
        roots: InputRoots,
        envelope: PathBuf,
        sha256: String,
    },
}

struct InputRoots {
    repository: PathBuf,
    runtime: PathBuf,
    compiler: PathBuf,
    profile: PathBuf,
}

impl InputRoots {
    fn bind(self) -> Result<Roots, OfflineCommandFailure> {
        Ok(Roots {
            repository: self.repository,
            runtime: self.runtime,
            compiler: self.compiler,
            profile: self.profile,
            // Caller/manifest cannot select another executable's identity.
            executable: std::env::current_exe()
                .map_err(|_| OfflineCommandFailure::new("identity_unavailable"))?,
        })
    }
}

/// Run a closed offline command. Emission alone generates a fresh Plan; verify
/// restores the original bytes against separately supplied roots and digest.
/// This does not validate SDK pins, authenticate, consume an approval, or qualify
/// a database workflow. In particular, `execute` is rejected before any I/O.
pub fn run_offline_command(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Value, OfflineCommandFailure> {
    match parse(arguments)? {
        Command::Help => Ok(json!({
            "format": "joinery-snowflake-table-offline-command-v1",
            "commands": {
                "emit": "--repository ABS --runtime ABS --compiler ABS --profile ABS --output-parent ABS --not-before-unix-ms U64 --expires-unix-ms U64",
                "verify": "--repository ABS --runtime ABS --compiler ABS --profile ABS --envelope ABS --sha256 LOWERCASE_SHA256",
            },
            "executionAvailable": false,
            "accountActivity": false,
        })),
        Command::Emit {
            roots,
            window,
            output_parent,
        } => {
            let plan = Plan::generate()
                .map_err(|_| OfflineCommandFailure::new("plan_generation_failed"))?;
            let envelope = Envelope::capture(&roots.bind()?, plan, window)
                .map_err(|_| OfflineCommandFailure::new("envelope_capture_failed"))?;
            envelope
                .require_separate_output(&output_parent)
                .map_err(|_| OfflineCommandFailure::new("output_not_separate"))?;
            let bytes = envelope
                .canonical_json()
                .map_err(|_| OfflineCommandFailure::new("envelope_capture_failed"))?;
            let receipt = Emission::persist(&output_parent, &envelope.plan(), &bytes)
                .map_err(|_| OfflineCommandFailure::new("emission_persistence_failed"))?;
            let mut result = summary("emit", &envelope)?;
            result["outputPath"] = json!(receipt.path);
            result["envelopeFile"] = json!(receipt.path.join("execution-envelope.json"));
            result["planFile"] = json!(receipt.path.join("plan.json"));
            result["envelopeBytes"] = json!(receipt.envelope_bytes);
            result["executionSha256"] = json!(receipt.envelope_sha256);
            Ok(result)
        }
        Command::Verify {
            roots,
            envelope,
            sha256,
        } => {
            let envelope = Envelope::restore_file(&roots.bind()?, &envelope, &sha256)
                .map_err(|_| OfflineCommandFailure::new("envelope_verification_failed"))?;
            summary("verify", &envelope)
        }
    }
}

fn summary(operation: &str, envelope: &Envelope) -> Result<Value, OfflineCommandFailure> {
    let plan = envelope.plan();
    Ok(json!({
        "format": "joinery-snowflake-table-offline-command-v1",
        "operation": operation,
        "run": plan.run(),
        "database": DATABASE,
        "schema": SCHEMA,
        "table": plan.table(),
        "phases": PHASE_COUNT,
        "maximumSchemas": MAX_SCHEMAS,
        "maximumSql": u64::from(plan.total_budget.fixed)
            + u64::from(plan.total_budget.per_schema) * MAX_SCHEMAS as u64,
        "planSha256": plan.sha256()
            .map_err(|_| OfflineCommandFailure::new("offline_command_failed"))?,
        "executionSha256": envelope.sha256()
            .map_err(|_| OfflineCommandFailure::new("offline_command_failed"))?,
        "executionAvailable": false,
        "accountActivity": false,
    }))
}

fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, OfflineCommandFailure> {
    // Closed argument count and lengths before inspecting any local artifact.
    let arguments: Vec<_> = arguments.into_iter().take(18).collect();
    if arguments.is_empty()
        || arguments.len() > 15
        || arguments.iter().any(|value| value.len() > 65_536)
    {
        return Err(OfflineCommandFailure::arguments());
    }
    let operation = arguments[0]
        .to_str()
        .ok_or_else(OfflineCommandFailure::arguments)?;
    if matches!(operation, "help" | "--help") && arguments.len() == 1 {
        return Ok(Command::Help);
    }
    if !matches!(operation, "emit" | "verify") || arguments.len() % 2 != 1 {
        return Err(OfflineCommandFailure::arguments());
    }
    let mut values = BTreeMap::new();
    for pair in arguments[1..].as_chunks::<2>().0 {
        let key = pair[0]
            .to_str()
            .ok_or_else(OfflineCommandFailure::arguments)?;
        if !matches!(
            key,
            "--repository" | "--runtime" | "--compiler" | "--profile"
        ) && !match operation {
            "emit" => matches!(
                key,
                "--output-parent" | "--not-before-unix-ms" | "--expires-unix-ms"
            ),
            "verify" => matches!(key, "--envelope" | "--sha256"),
            _ => false,
        } {
            return Err(OfflineCommandFailure::arguments());
        }
        if values.insert(key, pair[1].clone()).is_some() {
            return Err(OfflineCommandFailure::arguments());
        }
    }
    let mut path = |key| {
        let value = PathBuf::from(
            values
                .remove(key)
                .ok_or_else(OfflineCommandFailure::arguments)?,
        );
        // Paths appear in canonical JSON and receipts. Reject an unencodable
        // path before any emission can persist without a printable receipt.
        if !value.is_absolute() || value.to_str().is_none() {
            return Err(OfflineCommandFailure::arguments());
        }
        Ok(value)
    };
    let roots = InputRoots {
        repository: path("--repository")?,
        runtime: path("--runtime")?,
        compiler: path("--compiler")?,
        profile: path("--profile")?,
    };
    match operation {
        "emit" => {
            let output_parent = path("--output-parent")?;
            let mut number = |key| {
                let value = values
                    .remove(key)
                    .ok_or_else(OfflineCommandFailure::arguments)?;
                let text = value
                    .to_str()
                    .ok_or_else(OfflineCommandFailure::arguments)?;
                if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(OfflineCommandFailure::arguments());
                }
                text.parse().map_err(|_| OfflineCommandFailure::arguments())
            };
            let window = Window {
                not_before_unix_ms: number("--not-before-unix-ms")?,
                expires_unix_ms: number("--expires-unix-ms")?,
            };
            window
                .validate()
                .map_err(|_| OfflineCommandFailure::arguments())?;
            Ok(Command::Emit {
                roots,
                window,
                output_parent,
            })
        }
        "verify" => {
            let envelope = path("--envelope")?;
            let sha256 = values
                .remove("--sha256")
                .and_then(|value| value.into_string().ok())
                .filter(|value| {
                    value.len() == 64
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
                .ok_or_else(OfflineCommandFailure::arguments)?;
            Ok(Command::Verify {
                roots,
                envelope,
                sha256,
            })
        }
        _ => Err(OfflineCommandFailure::arguments()),
    }
}
