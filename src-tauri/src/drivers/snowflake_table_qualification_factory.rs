//! Fresh, private construction for one consumed closed qualification.
//! No desktop state, OS credential store, caller-built Studio or runtime is
//! adopted. Dropping a waiter does not cancel the retained execution or delete
//! its evidence. This internal factory is not an account command or approval.

use super::{
    attempt::{AttemptStorage, TerminalReceipt},
    consumption::ConsumptionStore,
    envelope::{ConsumedEnvelope, ConsumedRevocation, Envelope},
    guard::Guard,
    policy::Policy,
    profile,
    runner::{Output, Runner, RunnerReceipt},
};
use crate::{
    domain::{AppError, AppResult},
    execution::Studio,
    file_capability::DisabledFileDialog,
    secrets::SecretStore,
    snowflake_runtime::SnowflakeRuntime,
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn failed() -> AppError {
    AppError::internal("Private qualification construction or retained task failed")
}

/// Even attempted access fails closed; no operation delegates to a keychain.
#[derive(Default)]
struct DeniedSecrets(AtomicUsize);
impl DeniedSecrets {
    fn reject<T>(&self) -> AppResult<T> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(AppError::invalid(
            "Qualification cannot access stored credentials",
        ))
    }
    fn calls(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}
impl SecretStore for DeniedSecrets {
    fn get(&self, _: &str) -> AppResult<Option<String>> {
        self.reject()
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        self.reject()
    }
    fn delete(&self, _: &str) -> AppResult<()> {
        self.reject()
    }
}

#[derive(Debug)]
pub(super) struct PreparationFailure {
    pub(super) error: AppError,
    pub(super) path: Option<PathBuf>,
    pub(super) terminal: Option<AppResult<TerminalReceipt>>,
}

pub(super) struct PreparedAttempt {
    owner: AttemptOwner,
    #[cfg(test)]
    controls: Option<super::runner::FixtureControls>,
    #[cfg(test)]
    finished: Option<tokio::sync::oneshot::Sender<()>>,
    #[cfg(test)]
    terminal_failure: bool,
}

struct AttemptOwner {
    storage: AttemptStorage,
    consumed: Option<ConsumedEnvelope>,
    studio: Option<Arc<Studio>>,
    policy: Option<Arc<Policy>>,
    guard: Option<Arc<Guard>>,
    secrets: Arc<DeniedSecrets>,
    #[cfg(test)]
    fixture_runtime: Option<Arc<SnowflakeRuntime>>,
}

pub(super) struct AttemptReceipt {
    pub(super) path: PathBuf,
    pub(super) execution: Option<RunnerReceipt>,
    pub(super) construction_error: Option<AppError>,
    pub(super) construction_shutdown: Option<AppResult<()>>,
    pub(super) runner_task_error: Option<AppError>,
    pub(super) terminal: AppResult<TerminalReceipt>,
    pub(super) terminal_record: Value,
    // Keep unsettled Studio/guard owners and the held filesystem identities.
    owner: AttemptOwner,
}

impl PreparedAttempt {
    /// Obtain revocation before transferring ownership to the retained task.
    /// This grants no construction inputs and does not cancel on drop.
    pub(super) fn revocation(&self) -> ConsumedRevocation {
        self.owner
            .consumed
            .as_ref()
            .expect("prepared attempt retains its consumed envelope until start")
            .revocation()
    }

    /// Blocking local work only. Reserve all evidence outputs before spending
    /// either permanent marker; no runtime, SQLite, authentication or SQL here.
    pub(super) fn prepare(
        envelope: Envelope,
        store: &ConsumptionStore,
        approval: &[u8],
        output_parent: &Path,
    ) -> Result<Self, PreparationFailure> {
        let before_reservation = |error| PreparationFailure {
            error,
            path: None,
            terminal: None,
        };
        store
            .require_separate_from(output_parent)
            .map_err(before_reservation)?;
        envelope
            .require_separate_output(output_parent)
            .map_err(before_reservation)?;
        let plan = envelope.plan();
        let execution_hash = envelope.sha256().map_err(before_reservation)?;
        let mut storage = AttemptStorage::reserve(output_parent, &plan, &execution_hash)
            .map_err(before_reservation)?;
        let consumed = match envelope.consume(store, approval) {
            Ok(consumed) => consumed,
            Err(error) => {
                let terminal = storage.persist_terminal(&json!({
                    "executionSuccessful":false, "preparationError":error,
                    "execution":null, "secretStoreCalls":0,
                }));
                return Err(PreparationFailure {
                    error,
                    path: Some(storage.path().to_owned()),
                    terminal: Some(terminal),
                });
            }
        };
        Ok(Self {
            owner: AttemptOwner {
                storage,
                consumed: Some(consumed),
                studio: None,
                policy: None,
                guard: None,
                secrets: Arc::new(DeniedSecrets::default()),
                #[cfg(test)]
                fixture_runtime: None,
            },
            #[cfg(test)]
            controls: None,
            #[cfg(test)]
            finished: None,
            #[cfg(test)]
            terminal_failure: false,
        })
    }

    #[cfg(test)]
    pub(super) fn with_fixture_runtime(mut self, runtime: Arc<SnowflakeRuntime>) -> Self {
        self.owner.fixture_runtime = Some(runtime);
        self
    }
    #[cfg(test)]
    pub(super) fn with_controls(mut self, controls: super::runner::FixtureControls) -> Self {
        self.controls = Some(controls);
        self
    }
    #[cfg(test)]
    pub(super) fn with_finished(mut self, finished: tokio::sync::oneshot::Sender<()>) -> Self {
        self.finished = Some(finished);
        self
    }
    #[cfg(test)]
    pub(super) fn with_terminal_failure(mut self) -> Self {
        self.terminal_failure = true;
        self
    }

    /// The outer task owns construction, ordinary Studio execution, shutdown,
    /// and terminal persistence. Dropping its JoinHandle only detaches a waiter.
    pub(super) fn start(self) -> tokio::task::JoinHandle<AttemptReceipt> {
        tokio::spawn(async move {
            let (mut owner, constructed) = tokio::task::spawn_blocking(move || {
                let mut owner = self.owner;
                let result = catch_unwind(AssertUnwindSafe(|| owner.construct()))
                    .unwrap_or_else(|_| Err(failed()));
                (owner, result)
            })
            .await
            .expect("retained construction owner unwound outside its boundary");
            let mut construction_error = None;
            let mut runner_task_error = None;
            let execution = match constructed {
                Ok(runner) => {
                    #[cfg(test)]
                    let mut runner = runner;
                    #[cfg(test)]
                    if let Some(controls) = self.controls {
                        runner.set_controls(controls);
                    }
                    match tokio::spawn(runner.run()).await {
                        Ok(receipt) => Some(receipt),
                        Err(_) => {
                            runner_task_error = Some(failed());
                            None
                        }
                    }
                }
                Err(error) => {
                    construction_error = Some(error);
                    None
                }
            };
            let construction_shutdown = if execution.is_none() {
                match &owner.studio {
                    Some(studio) => Some(studio.shutdown_and_wait().await),
                    None => None,
                }
            } else {
                None
            };
            #[cfg(test)]
            if self.terminal_failure {
                let _ = fs::set_permissions(
                    owner.storage.path().join("terminal.json"),
                    fs::Permissions::from_mode(0o400),
                );
            }
            let receipt = tokio::task::spawn_blocking(move || {
                let record = terminal_record(
                    &owner,
                    execution.as_ref(),
                    &construction_error,
                    &construction_shutdown,
                    &runner_task_error,
                );
                let terminal = owner.storage.persist_terminal(&record);
                AttemptReceipt {
                    path: owner.storage.path().to_owned(),
                    execution,
                    construction_error,
                    construction_shutdown,
                    runner_task_error,
                    terminal,
                    terminal_record: record,
                    owner,
                }
            })
            .await
            .expect("retained terminal owner unwound outside its boundary");
            #[cfg(test)]
            if let Some(finished) = self.finished {
                let _ = finished.send(());
            }
            receipt
        })
    }
}

impl AttemptOwner {
    /// All blocking inventories and SQLite work occur outside process/policy
    /// locks. The exact consumed inputs select the only runtime and profile.
    fn construct(&mut self) -> AppResult<Runner> {
        self.storage.validate()?;
        let consumed = self.consumed.as_ref().ok_or_else(failed)?;
        consumed.revalidate()?;
        let (input, root) = consumed.construction_inputs()?;
        let plan = consumed.plan();
        let frozen = consumed.frozen_profile()?;
        // Runtime validation precedes all application persistence or launch.
        #[cfg(test)]
        let runtime = match &self.fixture_runtime {
            Some(template) => {
                SnowflakeRuntime::from_qualification_fixture(template, &root, self.storage.state())
            }
            None => SnowflakeRuntime::from_qualification_root(&root, self.storage.state()),
        };
        #[cfg(not(test))]
        let runtime = SnowflakeRuntime::from_qualification_root(&root, self.storage.state());
        let runtime = Arc::new(runtime.map_err(|_| failed())?);
        self.storage.validate()?;
        if fs::read_dir(self.storage.workspace())
            .map_err(|_| failed())?
            .next()
            .is_some()
            || fs::read_dir(self.storage.state())
                .map_err(|_| failed())?
                .next()
                .is_some()
        {
            return Err(failed());
        }
        let studio = Studio::with_snowflake_runtime(
            self.storage.workspace(),
            self.secrets.clone(),
            Arc::new(DisabledFileDialog),
            Some(runtime),
        )?;
        self.studio = Some(studio.clone());
        self.validate_workspace()?;
        let saved = input.save_new(&studio)?;
        profile::validate_saved(&input, &saved)?;
        let attempt = studio.prepare_connection(&saved.id)?;
        if self.secrets.calls() != 0 {
            return Err(failed());
        }
        self.validate_workspace()?;
        consumed.revalidate()?;
        let journal = self.storage.take_journal()?;
        let policy = Arc::new(Policy::from_reserved_journal(journal, frozen, plan)?);
        self.policy = Some(policy.clone());
        let runner = Runner::consumed(
            studio,
            policy,
            self.consumed.take().ok_or_else(failed)?,
            attempt,
        )?;
        self.guard = runner.retained_guard();
        Ok(runner)
    }

    fn validate_workspace(&self) -> AppResult<()> {
        self.storage.validate()?;
        for entry in fs::read_dir(self.storage.workspace()).map_err(|_| failed())? {
            let entry = entry.map_err(|_| failed())?;
            if !["joinery.sqlite", "joinery.sqlite-wal", "joinery.sqlite-shm"]
                .iter()
                .any(|name| entry.file_name() == *name)
            {
                return Err(failed());
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|_| failed())?;
            if !metadata.file_type().is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.nlink() != 1
                || metadata.permissions().mode() & 0o7777 != 0o600
            {
                return Err(failed());
            }
        }
        Ok(())
    }
}

fn execution_successful(receipt: &RunnerReceipt) -> bool {
    receipt.failure.is_none()
        && receipt.original_call_failure.is_none()
        && receipt.completion_record_error.is_none()
        && receipt.shutdown.is_ok()
        && receipt.settled
        && receipt.persisted.is_ok()
        && receipt
            .guard_revalidation
            .as_ref()
            .is_some_and(Result::is_ok)
        && receipt.guard_settled == Some(true)
        && receipt.guard_recorded.as_ref().is_some_and(Result::is_ok)
}

impl AttemptReceipt {
    pub(super) fn secret_store_calls(&self) -> usize {
        self.owner.secrets.calls()
    }
    pub(super) fn successful(&self) -> bool {
        self.construction_error.is_none()
            && self.runner_task_error.is_none()
            && self
                .construction_shutdown
                .as_ref()
                .is_none_or(Result::is_ok)
            && self.execution.as_ref().is_some_and(execution_successful)
            && self.terminal.is_ok()
            && self.secret_store_calls() == 0
    }
}

fn terminal_record(
    owner: &AttemptOwner,
    execution: Option<&RunnerReceipt>,
    construction_error: &Option<AppError>,
    construction_shutdown: &Option<AppResult<()>>,
    runner_task_error: &Option<AppError>,
) -> Value {
    let execution_value = execution.map(|receipt| {
        // Full original evidence stays in the Journal. Preserve mutation and
        // transaction acknowledgements independently of all later IO failures;
        // avoid copying several thousand metadata names into the terminal file.
        let outputs: Vec<Value> = receipt
            .outputs
            .iter()
            .map(|output| match output {
                Output::Reconcile(_, _) => json!({"kind":"reconcile","evidence":"journal"}),
                Output::Catalog(_) => json!({"kind":"catalog","evidence":"journal"}),
                Output::Options(_) => json!({"kind":"options","evidence":"journal"}),
                Output::Read(_) => json!({"kind":"read","evidence":"journal"}),
                _ => serde_json::to_value(output.evidence())
                    .expect("closed qualification evidence serializes"),
            })
            .collect();
        json!({
            "outputs":outputs, "failure":receipt.failure,
            "originalCallFailure":receipt.original_call_failure.as_ref().map(|v|v.completion()),
            "completionRecordError":receipt.completion_record_error,
            "shutdown":receipt.shutdown, "settled":receipt.settled,
            // This can include a retained operation failure, not just IO errors.
            "persisted":receipt.persisted, "guardRevalidation":receipt.guard_revalidation,
            "guardSettled":receipt.guard_settled, "guardRecorded":receipt.guard_recorded,
        })
    });
    json!({
        // Only execution succeeded; terminal durability is established by the
        // separate receipt/digest. Never treat this field alone as acceptance.
        "executionSuccessful":construction_error.is_none() && runner_task_error.is_none()
            && execution.is_some_and(execution_successful) && owner.secrets.calls() == 0,
        "constructionError":construction_error, "constructionShutdown":construction_shutdown,
        "runnerTaskError":runner_task_error, "execution":execution_value,
        "secretStoreCalls":owner.secrets.calls(),
    })
}

#[cfg(test)]
#[path = "snowflake_table_qualification_factory_tests.rs"]
mod tests;
