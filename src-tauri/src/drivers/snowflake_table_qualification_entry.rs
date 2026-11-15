//! Fixed execute entry. Signals revoke admission while retained preparation,
//! original calls, shutdown and terminal persistence continue to completion.
//! Installing Tokio's Unix handlers changes process signal behavior permanently;
//! this synchronous entry is intended for the dedicated command process.

use super::{
    cli::{self, Command, OfflineCommandFailure},
    consumption::ConsumptionStore,
    envelope::Envelope,
    factory::{PreparationFailure, PreparedAttempt},
    inventory::read_private_file,
};
use serde_json::{Value, json};
use std::{ffi::OsString, future::poll_fn, path::PathBuf, task::Poll};
use tokio::signal::unix::{Signal, SignalKind, signal};

/// A failed retained attempt still returns its closed, reviewable receipt.
/// The binary must use this success bit, rather than treating Ok as success.
#[derive(Debug)]
pub struct CommandOutput {
    pub value: Value,
    pub successful: bool,
}

pub fn run_command(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<CommandOutput, OfflineCommandFailure> {
    let command = cli::parse(arguments)?;
    match command {
        Command::Execute { .. } => run_execute(
            command,
            #[cfg(test)]
            None,
        ),
        Command::Help => {
            let mut value = cli::run_parsed_offline(Command::Help)?;
            value["format"] = json!("joinery-snowflake-table-command-v1");
            value["commands"]["execute"] = json!(
                "--repository ABS --runtime ABS --compiler ABS --profile ABS --envelope ABS --sha256 LOWERCASE_SHA256 --approval ABS --output-parent ABS"
            );
            value["executionAvailable"] = json!(true);
            Ok(CommandOutput {
                value,
                successful: true,
            })
        }
        offline => Ok(CommandOutput {
            value: cli::run_parsed_offline(offline)?,
            successful: true,
        }),
    }
}

fn run_execute(
    command: Command,
    #[cfg(test)] fixture: Option<FixtureExecution>,
) -> Result<CommandOutput, OfflineCommandFailure> {
    // Avoid a nested-runtime panic in library callers. Offline commands never
    // construct a runtime or install a signal handler.
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(OfflineCommandFailure::new("runtime_unavailable"));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|_| OfflineCommandFailure::new("runtime_unavailable"))?;
    // Ordinary runtime drop waits for outstanding blocking work. Do not replace
    // this with background shutdown or a timeout that drops an execution owner.
    runtime.block_on(execute(
        command,
        #[cfg(test)]
        fixture,
    ))
}

struct Signals {
    interrupt: Signal,
    terminate: Signal,
}

impl Signals {
    fn register() -> Result<Self, OfflineCommandFailure> {
        let interrupt = signal(SignalKind::interrupt())
            .map_err(|_| OfflineCommandFailure::new("signal_registration_failed"))?;
        let terminate = signal(SignalKind::terminate())
            .map_err(|_| OfflineCommandFailure::new("signal_registration_failed"))?;
        Ok(Self {
            interrupt,
            terminate,
        })
    }

    async fn next(&mut self) {
        tokio::select! {
            biased;
            _ = self.interrupt.recv() => {}
            _ = self.terminate.recv() => {}
        }
    }

    /// Observe already delivered events without awaiting new work. In
    /// particular, preparation completion cannot win over a queued signal.
    async fn drain(&mut self) -> bool {
        poll_fn(|cx| {
            let interrupt = self.interrupt.poll_recv(cx).is_ready();
            let terminate = self.terminate.poll_recv(cx).is_ready();
            Poll::Ready(interrupt || terminate)
        })
        .await
    }
}

struct Identity {
    run: String,
    execution_sha256: String,
    output_path: PathBuf,
}

enum Preparation {
    Ready(Identity, Box<PreparedAttempt>),
    Failed(Identity, PreparationFailure),
}

async fn execute(
    command: Command,
    #[cfg(test)] fixture: Option<FixtureExecution>,
) -> Result<CommandOutput, OfflineCommandFailure> {
    // Registration precedes every blocking artifact read, permanent marker and
    // runtime construction. Both streams stay alive until the owner settles.
    let mut signals = Signals::register()?;
    #[cfg(test)]
    let cancellation_marker = fixture
        .as_ref()
        .and_then(|fixture| fixture.cancellation_observed.clone());
    let mut preparing = tokio::task::spawn_blocking(move || {
        prepare(
            command,
            #[cfg(test)]
            fixture,
        )
    });
    let mut cancellation_observed = false;
    let preparation = loop {
        tokio::select! {
            biased;
            _ = signals.next() => {
                cancellation_observed = true;
                #[cfg(test)]
                mark_cancelled(&cancellation_marker);
            }
            prepared = &mut preparing => {
                break prepared.map_err(|_| OfflineCommandFailure::new("preparation_task_failed"))??;
            }
        }
    };
    if signals.drain().await {
        cancellation_observed = true;
        #[cfg(test)]
        mark_cancelled(&cancellation_marker);
    }
    let (identity, prepared) = match preparation {
        Preparation::Ready(identity, prepared) => (identity, prepared),
        Preparation::Failed(identity, failure) => {
            return Ok(failed_preparation(identity, failure, cancellation_observed));
        }
    };
    let revocation = prepared.revocation();
    if cancellation_observed {
        revocation.revoke();
    }
    let mut owner = prepared.start();
    let receipt = loop {
        tokio::select! {
            biased;
            _ = signals.next() => {
                cancellation_observed = true;
                revocation.revoke();
                #[cfg(test)]
                mark_cancelled(&cancellation_marker);
            }
            result = &mut owner => {
                match result {
                    Ok(receipt) => break receipt,
                    Err(_) => {
                        let mut value = summary(&identity, cancellation_observed);
                        value["outputPath"] = json!(identity.output_path);
                        value["terminalReceipt"] = Value::Null;
                        value["terminalPersisted"] = json!(false);
                        value["successful"] = json!(false);
                        value["mayHaveAccountActivity"] = json!(true);
                        value["failureCode"] = json!("execution_task_failed");
                        return Ok(CommandOutput { value, successful: false });
                    }
                }
            }
        }
    };
    if signals.drain().await {
        cancellation_observed = true;
        revocation.revoke();
        #[cfg(test)]
        mark_cancelled(&cancellation_marker);
    }
    let successful = receipt.successful() && !cancellation_observed;
    let mut value = summary(&identity, cancellation_observed);
    value["outputPath"] = json!(receipt.path);
    value["terminalReceipt"] = receipt
        .terminal
        .as_ref()
        .ok()
        .map_or(Value::Null, |terminal| json!(terminal));
    value["terminalPersisted"] = json!(receipt.terminal.is_ok());
    value["successful"] = json!(successful);
    // An owner may have authenticated even if construction/cleanup later failed.
    // This is conservative and never claims that a cancelled run reached account SQL.
    value["mayHaveAccountActivity"] = json!(true);
    if !successful {
        value["failureCode"] = json!("qualification_failed");
    }
    Ok(CommandOutput { value, successful })
}

fn prepare(
    command: Command,
    #[cfg(test)] fixture: Option<FixtureExecution>,
) -> Result<Preparation, OfflineCommandFailure> {
    let Command::Execute {
        roots,
        envelope,
        sha256,
        approval,
        output_parent,
    } = command
    else {
        return Err(OfflineCommandFailure::new("invalid_arguments"));
    };
    #[cfg(test)]
    if let Some(pause) = fixture
        .as_ref()
        .and_then(|fixture| fixture.before_prepare.as_ref())
    {
        pause.wait()?;
    }
    let envelope = Envelope::restore_file(&roots.bind()?, &envelope, &sha256)
        .map_err(|_| OfflineCommandFailure::new("envelope_verification_failed"))?;
    let run = envelope.plan().run().to_owned();
    let identity = Identity {
        output_path: output_parent.join(format!("attempt-{run}")),
        run,
        execution_sha256: sha256,
    };
    let approval = read_private_file(&approval, 4096)
        .map_err(|_| OfflineCommandFailure::new("approval_unavailable"))?;
    #[cfg(test)]
    let store = match &fixture {
        Some(fixture) => ConsumptionStore::open_fixture_native_home(&fixture.native_home),
        None => ConsumptionStore::open_native(),
    };
    #[cfg(not(test))]
    let store = ConsumptionStore::open_native();
    let store = store.map_err(|_| OfflineCommandFailure::new("consumption_store_unavailable"))?;
    match PreparedAttempt::prepare(envelope, &store, &approval, &output_parent) {
        Ok(prepared) => {
            #[cfg(test)]
            let prepared = match fixture {
                Some(fixture) => {
                    let prepared = prepared.with_fixture_runtime(fixture.runtime);
                    match fixture.controls {
                        Some(controls) => prepared.with_controls(controls),
                        None => prepared,
                    }
                }
                None => prepared,
            };
            Ok(Preparation::Ready(identity, Box::new(prepared)))
        }
        Err(failure) => Ok(Preparation::Failed(identity, failure)),
    }
}

fn summary(identity: &Identity, cancellation_observed: bool) -> Value {
    json!({
        "format":"joinery-snowflake-table-command-v1", "operation":"execute",
        "run":identity.run, "executionSha256":identity.execution_sha256,
        "cancellationObserved":cancellation_observed,
    })
}

fn failed_preparation(
    identity: Identity,
    failure: PreparationFailure,
    cancelled: bool,
) -> CommandOutput {
    let mut value = summary(&identity, cancelled);
    value["successful"] = json!(false);
    value["failureCode"] = json!("preparation_failed");
    value["outputPath"] = json!(failure.path);
    value["terminalReceipt"] = failure
        .terminal
        .as_ref()
        .and_then(|terminal| terminal.as_ref().ok())
        .map_or(Value::Null, |terminal| json!(terminal));
    value["terminalPersisted"] = json!(failure.terminal.as_ref().is_some_and(Result::is_ok));
    value["mayHaveAccountActivity"] = json!(false);
    CommandOutput {
        value,
        successful: false,
    }
}

#[cfg(test)]
pub(super) struct FixtureExecution {
    pub(super) native_home: PathBuf,
    pub(super) runtime: std::sync::Arc<crate::snowflake_runtime::SnowflakeRuntime>,
    pub(super) controls: Option<super::runner::FixtureControls>,
    pub(super) before_prepare: Option<FixturePause>,
    pub(super) cancellation_observed: Option<PathBuf>,
}

#[cfg(test)]
pub(super) struct FixturePause {
    pub(super) ready: PathBuf,
    pub(super) release: PathBuf,
}

#[cfg(test)]
impl FixturePause {
    fn wait(&self) -> Result<(), OfflineCommandFailure> {
        std::fs::write(&self.ready, b"ready")
            .map_err(|_| OfflineCommandFailure::new("fixture_pause_failed"))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !self.release.exists() {
            if std::time::Instant::now() >= deadline {
                return Err(OfflineCommandFailure::new("fixture_pause_failed"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(())
    }
}

#[cfg(test)]
fn mark_cancelled(path: &Option<PathBuf>) {
    if let Some(path) = path {
        std::fs::write(path, b"cancelled").expect("fixture cancellation marker writable");
    }
}

#[cfg(test)]
pub(super) fn run_fixture_command(
    arguments: impl IntoIterator<Item = OsString>,
    fixture: FixtureExecution,
) -> Result<CommandOutput, OfflineCommandFailure> {
    let command = cli::parse(arguments)?;
    if !matches!(command, Command::Execute { .. }) {
        return Err(OfflineCommandFailure::new("invalid_arguments"));
    }
    run_execute(command, Some(fixture))
}

#[cfg(test)]
#[path = "snowflake_table_qualification_entry_tests.rs"]
mod tests;
