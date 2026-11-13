//! Tracked local recovery. Status contains no input secrets or filesystem paths.
use super::*;
use crate::persistence::{
    recovery_crypto::BackupPassphrase, recovery_stream::RecoveryControl,
    snowflake_console::ConsoleJournalOwner,
};
use std::collections::{HashSet, VecDeque};

const STATUS_LIMIT: usize = 64;
const ID_LIMIT: usize = 4096;

#[derive(Default)]
pub(super) struct RecoveryJobs {
    seen: HashSet<String>,
    statuses: VecDeque<RecoveryOperation>,
    token: Option<CancellationToken>,
    admission_closed: bool,
}

fn recovery_busy() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "A local metadata recovery operation is in progress",
    )
}

struct Reservation {
    studio: Arc<Studio>,
    id: String,
    owner: Option<ConsoleJournalOwner>,
    finished: bool,
}

impl Reservation {
    fn finish(&mut self, result: AppResult<Option<RecoveryBackup>>) {
        let _configuration = self.studio.configuration_gate.lock();
        self.studio.console.return_recovery_owner(self.owner.take());
        let mut jobs = self.studio.recovery_jobs.lock();
        if let Some(status) = jobs.statuses.iter_mut().find(|s| s.id == self.id) {
            match result {
                Ok(backup) => {
                    status.state = RecoveryOperationState::Succeeded;
                    if backup.is_some() {
                        status.backup = backup;
                    }
                }
                Err(error) => {
                    status.state = if error.code == ErrorCode::Cancelled {
                        RecoveryOperationState::Cancelled
                    } else {
                        RecoveryOperationState::Failed
                    };
                    status.error = Some(error);
                }
            }
        }
        jobs.token = None;
        self.studio.recovery_active.store(false, Ordering::Release);
        self.studio.recovery_finished.notify_waiters();
        self.finished = true;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(Err(AppError::new(ErrorCode::OutcomeUnknown,
                "Local recovery stopped unexpectedly. Reload local metadata and inspect the retained backups before trying another operation.")));
        }
    }
}

impl Studio {
    pub fn metadata_recovery_admission_closed(&self) -> bool {
        self.recovery_jobs.lock().admission_closed
    }

    /// A missing original identifier cannot be fenced by inventing a new one.
    /// Close every future start in this process before the UI resumes saving.
    pub fn reconcile_unknown_metadata_recovery(&self) -> AppResult<RecoveryOperation> {
        let _configuration = self.configuration_mutation_guard()?;
        let mut jobs = self.recovery_jobs.lock();
        jobs.admission_closed = true;
        let status = RecoveryOperation {
            id: new_id(),
            kind: None,
            state: RecoveryOperationState::Failed,
            progress: RecoveryProgress {
                phase: RecoveryPhase::Finalizing,
                completed_bytes: 0,
                total_bytes: None,
            },
            cancellation_requested: false,
            backup: None,
            error: Some(AppError::new(
                ErrorCode::OutcomeUnknown,
                "The original recovery identifier is unavailable, so its outcome remains unknown. Further recovery starts are disabled until Joinery fully restarts. Reload metadata and inspect backups before continuing.",
            )),
        };
        if jobs.statuses.len() == STATUS_LIMIT {
            jobs.statuses.pop_front();
        }
        jobs.statuses.push_back(status.clone());
        Ok(status)
    }

    /// Make an unknown start safe to reconcile. The tombstone is written under
    /// the same admission gate, so an original delayed command cannot run later.
    pub fn reconcile_metadata_recovery(&self, id: &str) -> AppResult<RecoveryOperation> {
        if uuid::Uuid::parse_str(id)
            .ok()
            .is_none_or(|uuid| uuid.to_string() != id)
        {
            return Err(AppError::invalid(
                "Invalid local recovery operation identifier",
            ));
        }
        let _configuration = self.configuration_mutation_guard()?;
        let mut jobs = self.recovery_jobs.lock();
        if let Some(status) = jobs.statuses.iter().find(|s| s.id == id) {
            return Ok(status.clone());
        }
        let seen = jobs.seen.contains(id);
        if !seen && jobs.seen.len() >= ID_LIMIT {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Restart Joinery to reconcile additional local recovery identifiers",
            ));
        }
        let status = RecoveryOperation {
            id: id.to_owned(),
            kind: None,
            state: RecoveryOperationState::Failed,
            progress: RecoveryProgress {
                phase: RecoveryPhase::Finalizing,
                completed_bytes: 0,
                total_bytes: None,
            },
            cancellation_requested: true,
            backup: None,
            error: Some(AppError::new(
                ErrorCode::OutcomeUnknown,
                if seen {
                    "The previous status is no longer retained. The operation will not be repeated; reload current metadata and inspect retained backups."
                } else {
                    "This identifier was fenced before any future admission. Reload current metadata and inspect backups; no operation will be automatically repeated."
                },
            )),
        };
        jobs.seen.insert(id.to_owned());
        if jobs.statuses.len() == STATUS_LIMIT {
            jobs.statuses.pop_front();
        }
        jobs.statuses.push_back(status.clone());
        Ok(status)
    }

    pub(super) fn cancel_recovery_for_shutdown(&self) {
        let jobs = self.recovery_jobs.lock();
        if jobs.statuses.back().is_some_and(|status| {
            status.state == RecoveryOperationState::Running
                && !matches!(
                    status.progress.phase,
                    RecoveryPhase::Replacing | RecoveryPhase::Finalizing
                )
        }) && let Some(token) = &jobs.token
        {
            token.cancel();
        }
    }

    pub(super) async fn wait_metadata_recovery_idle(&self) {
        loop {
            let notified = self.recovery_finished.notified();
            // Enable before checking, so a concurrent completion cannot lose
            // its notification between the check and the await.
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.recovery_active.load(Ordering::Acquire) {
                break;
            }
            notified.await;
        }
    }

    pub fn recovery_backups(&self) -> AppResult<Vec<RecoveryBackup>> {
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.recovery_backups()
    }

    pub fn delete_recovery_backup(
        &self,
        id: &str,
        digest: &str,
        confirmation: &str,
    ) -> AppResult<()> {
        if confirmation != "DELETE" {
            return Err(AppError::invalid("Type DELETE to remove this local backup"));
        }
        let _configuration = self.configuration_mutation_guard()?;
        self.persistence.delete_recovery_backup(id, digest)
    }

    pub fn metadata_recovery_operation(&self, id: &str) -> AppResult<RecoveryOperation> {
        self.recovery_jobs.lock().statuses.iter().find(|s| s.id == id).cloned()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound,
                "No retained status exists for this operation. Its outcome is unknown; reload metadata and inspect backups. Do not automatically repeat it."))
    }

    pub fn cancel_metadata_recovery(&self, id: &str) -> AppResult<RecoveryOperation> {
        let mut jobs = self.recovery_jobs.lock();
        let status = jobs
            .statuses
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::NotFound,
                    "Recovery operation status is unavailable",
                )
            })?;
        if status.state != RecoveryOperationState::Running {
            return Ok(status.clone());
        }
        if matches!(
            status.progress.phase,
            RecoveryPhase::Replacing | RecoveryPhase::Finalizing
        ) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Local replacement or publication is finishing; wait for its actual result",
            ));
        }
        status.cancellation_requested = true;
        let snapshot = status.clone();
        if let Some(token) = &jobs.token {
            token.cancel();
        }
        Ok(snapshot)
    }

    /// Caller chooses an opaque UUID before admission, so a lost start response
    /// can be reconciled by lookup without resending the operation or its secret.
    pub fn start_metadata_recovery(
        self: &Arc<Self>,
        id: String,
        request: RecoveryRequest,
        backup_passphrase: Zeroizing<String>,
        backup_confirmation: Zeroizing<String>,
        write_passphrase: Option<Zeroizing<String>>,
    ) -> AppResult<RecoveryOperation> {
        if uuid::Uuid::parse_str(&id)
            .ok()
            .is_none_or(|uuid| uuid.to_string() != id)
        {
            return Err(AppError::invalid(
                "Invalid local recovery operation identifier",
            ));
        }
        let selected = matches!(
            request.kind,
            RecoveryOperationKind::Restore | RecoveryOperationKind::Export
        );
        if selected != request.backup_id.is_some()
            || selected != request.digest.is_some()
            || request
                .backup_id
                .as_ref()
                .is_some_and(|v| uuid::Uuid::parse_str(v).is_err() || v.len() != 36)
            || request.digest.as_ref().is_some_and(|v| {
                v.len() != 64
                    || !v
                        .bytes()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            })
        {
            return Err(AppError::invalid(
                "Choose a valid local backup before starting recovery",
            ));
        }
        if request.kind == RecoveryOperationKind::Restore && request.confirmation != "RESTORE" {
            return Err(AppError::invalid("Type RESTORE to replace local metadata"));
        }
        let secret = if matches!(
            request.kind,
            RecoveryOperationKind::Create | RecoveryOperationKind::Restore
        ) {
            if backup_passphrase.as_str() != backup_confirmation.as_str() {
                return Err(AppError::invalid("The backup passphrases do not match"));
            }
            Some(BackupPassphrase::new(backup_passphrase)?)
        } else {
            if !backup_passphrase.is_empty() || !backup_confirmation.is_empty() {
                return Err(AppError::invalid(
                    "Import and export do not require a backup passphrase",
                ));
            }
            None
        };
        let token = CancellationToken::new();
        let (snapshot, owner) = {
            let _configuration = self.configuration_mutation_guard()?;
            if self.dml_tasks.is_closed() {
                return Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "The app is closing",
                ));
            }
            #[cfg(unix)]
            let _ = self.flush_snowflake_grid_outcomes();
            #[cfg(unix)]
            let snowflake_grid_busy =
                !self.batch_outcomes.is_empty() || !self.snowflake_grid.is_empty();
            #[cfg(not(unix))]
            let snowflake_grid_busy = false;
            if self.recovery_activity.load(Ordering::Acquire) != 0
                || self.connection_attempts.has_pending()
                || !self.sessions.lock().is_empty()
                || !self.jobs.lock().is_empty()
                || !self.exports.lock().is_empty()
                || self.results.usage().1 != 0
                || !self.dml_tasks.is_empty()
                || !self.pending_history.lock().is_empty()
                || !self.pending_snowflake_receipts.lock().is_empty()
                || snowflake_grid_busy
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Disconnect every connection, close result tabs, and wait for all connection, query, export and receipt work to finish before local recovery",
                ));
            }
            if request.kind == RecoveryOperationKind::Restore
                && self
                    .persistence
                    .setting("passphrase-configured")?
                    .as_deref()
                    == Some("true")
            {
                self.verify_passphrase(write_passphrase.as_deref().map(String::as_str))?;
            }
            let mut jobs = self.recovery_jobs.lock();
            if jobs.admission_closed {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Recovery admission was closed while reconciling an unknown operation. Fully restart Joinery before starting another recovery operation",
                ));
            }
            if jobs.seen.contains(&id) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "This operation identifier was already used; look up its status instead of repeating it",
                ));
            }
            if jobs.seen.len() >= ID_LIMIT {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "Restart Joinery after completing recovery to begin more local recovery operations",
                ));
            }
            let existing_owner = self.console.take_recovery_owner()?;
            let owner = match existing_owner {
                Some(owner) => owner,
                None => self.persistence.claim_console_archive_owner()?,
            };
            let snapshot = RecoveryOperation {
                id: id.clone(),
                kind: Some(request.kind),
                state: RecoveryOperationState::Running,
                progress: RecoveryProgress {
                    phase: RecoveryPhase::Staging,
                    completed_bytes: 0,
                    total_bytes: None,
                },
                cancellation_requested: false,
                backup: None,
                error: None,
            };
            if jobs.statuses.len() == STATUS_LIMIT {
                jobs.statuses.pop_front();
            }
            jobs.seen.insert(id.clone());
            jobs.statuses.push_back(snapshot.clone());
            jobs.token = Some(token.clone());
            self.recovery_active.store(true, Ordering::Release);
            (snapshot, owner)
        };
        let mut reservation = Reservation {
            studio: self.clone(),
            id: id.clone(),
            owner: Some(owner),
            finished: false,
        };
        let progress_studio = self.clone();
        let progress_id = id.clone();
        let publication_studio = self.clone();
        let publication_id = id.clone();
        let control = RecoveryControl {
            cancel: token,
            progress: Arc::new(move |progress| {
                if let Some(status) = progress_studio
                    .recovery_jobs
                    .lock()
                    .statuses
                    .iter_mut()
                    .find(|s| s.id == progress_id)
                {
                    status.progress = progress;
                }
            }),
            published: Arc::new(move |backup| {
                if let Some(status) = publication_studio
                    .recovery_jobs
                    .lock()
                    .statuses
                    .iter_mut()
                    .find(|s| s.id == publication_id)
                {
                    status.backup = Some(backup);
                }
            }),
        };
        let studio = self.clone();
        tokio::spawn(async move {
            // File picker awaits retain the reservation, never a mutex guard.
            let selection = match request.kind {
                RecoveryOperationKind::Import => {
                    studio.file_dialog.pick_open_metadata_backup().await
                }
                RecoveryOperationKind::Export => {
                    studio
                        .file_dialog
                        .pick_save_metadata_backup("joinery-backup.age")
                        .await
                }
                _ => Ok(None),
            };
            let selected = match selection {
                Ok(selected) => selected,
                Err(error) => {
                    reservation.finish(Err(error));
                    return;
                }
            };
            if matches!(
                request.kind,
                RecoveryOperationKind::Import | RecoveryOperationKind::Export
            ) && selected.is_none()
            {
                reservation.finish(Err(AppError::new(
                    ErrorCode::Cancelled,
                    "File selection was cancelled",
                )));
                return;
            }
            // A blocking worker owns both lease and zeroizing secret. Dropping
            // the command future cannot cancel an admitted replacement.
            let _ = tokio::task::spawn_blocking(move || {
                let result = studio.run_metadata_recovery_blocking(
                    request,
                    secret,
                    selected,
                    &control,
                    &reservation,
                );
                reservation.finish(result);
            })
            .await;
        });
        Ok(snapshot)
    }

    fn run_metadata_recovery_blocking(
        self: &Arc<Self>,
        request: RecoveryRequest,
        secret: Option<BackupPassphrase>,
        selected: Option<std::path::PathBuf>,
        control: &RecoveryControl,
        reservation: &Reservation,
    ) -> AppResult<Option<RecoveryBackup>> {
        if control.cancel.is_cancelled() {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "Local recovery was cancelled",
            ));
        }
        let studio = self;
        let owner = reservation.owner.as_ref().ok_or_else(recovery_busy)?;
        owner.validate(&self.persistence)?;
        match request.kind {
            RecoveryOperationKind::Create => {
                Ok(Some(studio.persistence.create_encrypted_metadata_backup(
                    secret.as_ref().ok_or_else(recovery_busy)?,
                    control,
                )?))
            }
            RecoveryOperationKind::Restore => {
                let _configuration = studio.configuration_gate.lock();
                if !studio.recovery_active.load(Ordering::Acquire)
                    || studio.recovery_activity.load(Ordering::Acquire) != 0
                {
                    return Err(recovery_busy());
                }
                drop(_configuration);
                let backup_id = request.backup_id.as_deref().ok_or_else(recovery_busy)?;
                let digest = request.digest.as_deref().ok_or_else(recovery_busy)?;
                // Persistence identifies the selected format from its owned
                // extension; an encrypted secret also encrypts the undo copy.
                let passphrase = secret.as_ref().ok_or_else(recovery_busy)?;
                let backup = studio.persistence.restore_encrypted_metadata_backup(
                    backup_id,
                    digest,
                    Some(passphrase),
                    passphrase,
                    owner,
                    control,
                )?;
                studio.console.revoke();
                studio.plans.lock().clear();
                studio.ddl_preflights.lock().clear();
                studio.import_targets.lock().clear();
                studio.file_capabilities.clear();
                studio.results.set_temporary_disk_enabled(false);
                let temporary_storage = studio.persistence.setting("temporary-result-storage").map_err(|_| AppError::new(
                    ErrorCode::Internal,
                    "Metadata was restored, but local result-storage settings could not be refreshed. Reload metadata before continuing; the encrypted undo backup remains available.",
                ))?;
                studio
                    .results
                    .set_temporary_disk_enabled(temporary_storage.as_deref() == Some("true"));
                Ok(Some(backup))
            }
            RecoveryOperationKind::Import => {
                let mut file = super::recovery_files::open_source(
                    selected.as_deref().ok_or_else(recovery_busy)?,
                )?;
                Ok(Some(
                    studio
                        .persistence
                        .import_recovery_backup(&mut file, control)?,
                ))
            }
            RecoveryOperationKind::Export => {
                let mut target =
                    super::recovery_files::ExportTarget::new(selected.ok_or_else(recovery_busy)?)?;
                studio.persistence.export_recovery_backup(
                    request.backup_id.as_deref().ok_or_else(recovery_busy)?,
                    request.digest.as_deref().ok_or_else(recovery_busy)?,
                    &mut target.file,
                    control,
                )?;
                if control.cancel.is_cancelled() {
                    return Err(AppError::new(
                        ErrorCode::Cancelled,
                        "Local backup export was cancelled",
                    ));
                }
                (control.progress)(RecoveryProgress {
                    phase: RecoveryPhase::Finalizing,
                    completed_bytes: 0,
                    total_bytes: None,
                });
                // Cancellation accepted immediately before the phase change
                // must still prevent destination publication.
                if control.cancel.is_cancelled() {
                    return Err(AppError::new(
                        ErrorCode::Cancelled,
                        "Local backup export was cancelled",
                    ));
                }
                target.publish()?;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn create_request() -> RecoveryRequest {
        RecoveryRequest {
            kind: RecoveryOperationKind::Create,
            backup_id: None,
            digest: None,
            confirmation: String::new(),
        }
    }

    fn start(
        studio: &Arc<Studio>,
        id: &str,
        request: RecoveryRequest,
        secret: &str,
    ) -> AppResult<RecoveryOperation> {
        studio.start_metadata_recovery(
            id.to_owned(),
            request,
            Zeroizing::new(secret.to_owned()),
            Zeroizing::new(secret.to_owned()),
            None,
        )
    }

    async fn settled(studio: &Studio, id: &str) -> RecoveryOperation {
        tokio::time::timeout(
            Duration::from_secs(20),
            studio.wait_metadata_recovery_idle(),
        )
        .await
        .unwrap();
        studio.metadata_recovery_operation(id).unwrap()
    }

    #[tokio::test]
    async fn unknown_start_is_fenced_before_delayed_admission() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        let id = new_id();
        assert_eq!(
            studio.metadata_recovery_operation(&id).unwrap_err().code,
            ErrorCode::NotFound
        );
        let fenced = studio.reconcile_metadata_recovery(&id).unwrap();
        assert_eq!(fenced.state, RecoveryOperationState::Failed);
        assert_eq!(fenced.error.unwrap().code, ErrorCode::OutcomeUnknown);
        assert_eq!(fenced.kind, None);
        assert_eq!(
            start(&studio, &id, create_request(), "separate recovery test key")
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert!(!studio.recovery_active.load(Ordering::Acquire));
        assert!(studio.recovery_backups().unwrap().is_empty());
    }

    #[tokio::test]
    async fn admitted_job_survives_a_lost_response_and_wrong_secret_preserves_live_metadata() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        let secret = "separate recovery test key";
        let id = new_id();
        let admitted = start(&studio, &id, create_request(), secret).unwrap();
        assert_eq!(admitted.state, RecoveryOperationState::Running);
        let created = settled(&studio, &id).await;
        assert_eq!(
            created.state,
            RecoveryOperationState::Succeeded,
            "{:?}",
            created.error
        );
        let backup = created.backup.unwrap();
        assert_eq!(backup.format, RecoveryBackupFormat::EncryptedV1);
        assert_eq!(
            start(&studio, &id, create_request(), secret)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        let group = ConnectionGroup {
            id: new_id(),
            name: "Keep this live group".into(),
            parent_id: None,
            environment: Environment::Local,
        };
        studio.save_group(group.clone()).unwrap();
        let before = serde_json::to_value(studio.bootstrap().unwrap()).unwrap();
        let inventory =
            serde_json::to_value(studio.persistence.console_storage().unwrap()).unwrap();
        let bad = new_id();
        start(
            &studio,
            &bad,
            RecoveryRequest {
                kind: RecoveryOperationKind::Restore,
                backup_id: Some(backup.metadata.id),
                digest: Some(backup.metadata.digest),
                confirmation: "RESTORE".into(),
            },
            "incorrect recovery test key",
        )
        .unwrap();
        let failed = settled(&studio, &bad).await;
        assert_eq!(failed.state, RecoveryOperationState::Failed);
        assert_eq!(
            serde_json::to_value(studio.bootstrap().unwrap()).unwrap(),
            before
        );
        assert_eq!(
            serde_json::to_value(studio.persistence.console_storage().unwrap()).unwrap(),
            inventory
        );
        assert_eq!(
            studio.recovery_backups().unwrap().len(),
            1,
            "no undo should be published for an unauthenticated backup"
        );
    }

    #[tokio::test]
    async fn preexisting_async_activity_blocks_reservation_before_any_backup() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        let activity = studio.begin_recovery_activity().unwrap();
        let id = new_id();
        assert_eq!(
            start(&studio, &id, create_request(), "separate recovery test key")
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        drop(activity);
        assert_eq!(
            studio.metadata_recovery_operation(&id).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert!(studio.recovery_backups().unwrap().is_empty());
    }

    #[tokio::test]
    async fn pending_generic_completion_blocks_recovery_until_local_recording_succeeds() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        let seed = HistoryEntry {
            id: new_id(),
            connection_id: new_id(),
            connection_name: "Recovery completion fixture".into(),
            started_at: now_ms(),
            sql: None,
            statement_count: 1,
            provenance: HistoryProvenance::Admitted,
            status: None,
            elapsed_ms: None,
            execution_summary: None,
            snowflake_dml: None,
            snowflake_ddl: None,
        };
        let cell = studio
            .admit_outcome(
                studio.outcome_memory.ordinary().unwrap(),
                Some(&seed),
                crate::persistence::OutcomeDescriptor {
                    operation_id: seed.id.clone(),
                    connection_id: seed.connection_id.clone(),
                    session_id: new_id(),
                    started_at: seed.started_at,
                    digest: "a".repeat(64),
                    kind: crate::persistence::OutcomeKind::Execute { planned: 1 },
                },
                crate::persistence::OutcomePool::Ordinary,
            )
            .unwrap();
        let database = rusqlite::Connection::open(root.path().join("joinery.sqlite")).unwrap();
        database.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE ON history_v8 BEGIN SELECT RAISE(ABORT,'fixture completion failure'); END").unwrap();
        let completion = HistoryCompletion {
            provenance: HistoryProvenance::Recorded,
            status: Some(RunStatus::Completed),
            elapsed_ms: Some(17),
            execution_summary: None,
            snowflake_dml: None,
            snowflake_ddl: None,
        };
        assert!(studio.finish_outcome_history(&cell, &completion).is_err());
        assert_eq!(studio.pending_history.lock().len(), 1);
        assert_eq!(
            studio.history_entry(&seed.id).unwrap().unwrap().provenance,
            HistoryProvenance::Admitted
        );
        let id = new_id();
        let admitted = start(&studio, &id, create_request(), "separate recovery test key");
        if admitted.is_ok() {
            // A failing reproduction still retires the accidentally admitted job.
            settled(&studio, &id).await;
        }
        assert_eq!(admitted.unwrap_err().code, ErrorCode::Conflict);
        assert!(!studio.recovery_active.load(Ordering::Acquire));
        assert!(studio.recovery_backups().unwrap().is_empty());
        database
            .execute_batch("DROP TRIGGER reject_completion")
            .unwrap();
        studio.flush_pending_history().unwrap();
        assert!(studio.pending_history.lock().is_empty());
        let recorded = studio.history_entry(&seed.id).unwrap().unwrap();
        assert_eq!(recorded.provenance, HistoryProvenance::Recorded);
        assert_eq!(recorded.status, Some(RunStatus::Completed));
        assert_eq!(recorded.elapsed_ms, Some(17));
        start(&studio, &id, create_request(), "separate recovery test key").unwrap();
        assert_eq!(
            settled(&studio, &id).await.state,
            RecoveryOperationState::Succeeded
        );
        assert_eq!(studio.recovery_backups().unwrap().len(), 1);
        studio.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn restart_reconciliation_never_invents_the_previous_operation_outcome() {
        let root = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let studio = Studio::with_secrets(root.path(), secrets.clone()).unwrap();
        let id = new_id();
        start(&studio, &id, create_request(), "separate recovery test key").unwrap();
        assert_eq!(
            settled(&studio, &id).await.state,
            RecoveryOperationState::Succeeded
        );
        studio.shutdown_and_wait().await.unwrap();
        drop(studio);
        let reopened = Studio::with_secrets(root.path(), secrets).unwrap();
        let reconciled = reopened.reconcile_metadata_recovery(&id).unwrap();
        assert_eq!(reconciled.state, RecoveryOperationState::Failed);
        assert_eq!(reconciled.kind, None);
        assert_eq!(reconciled.error.unwrap().code, ErrorCode::OutcomeUnknown);
        assert_eq!(reopened.recovery_backups().unwrap().len(), 1);
        assert_eq!(
            start(
                &reopened,
                &id,
                create_request(),
                "separate recovery test key"
            )
            .unwrap_err()
            .code,
            ErrorCode::Conflict
        );
    }
    #[tokio::test]
    async fn unknown_identifier_fence_rejects_all_delayed_starts_until_native_restart() {
        let root = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let studio = Studio::with_secrets(root.path(), secrets.clone()).unwrap();
        let active = new_id();
        start(
            &studio,
            &active,
            create_request(),
            "separate recovery test key",
        )
        .unwrap();
        // The current-thread runtime cannot settle the admitted task until we
        // yield. The global fence must refuse to pretend this owner is idle.
        assert_eq!(
            studio
                .reconcile_unknown_metadata_recovery()
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            settled(&studio, &active).await.state,
            RecoveryOperationState::Succeeded
        );
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let delayed: Vec<_> = (0..2)
            .map(|_| {
                let studio = studio.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let original_id = new_id();
                    barrier.wait();
                    start(
                        &studio,
                        &original_id,
                        create_request(),
                        "separate recovery test key",
                    )
                    .unwrap_err()
                    .code
                })
            })
            .collect();
        let fenced = studio.reconcile_unknown_metadata_recovery().unwrap();
        assert_eq!(fenced.kind, None);
        assert_eq!(fenced.error.unwrap().code, ErrorCode::OutcomeUnknown);
        barrier.wait();
        for delayed in delayed {
            assert_eq!(delayed.join().unwrap(), ErrorCode::Conflict);
        }
        assert_eq!(studio.recovery_backups().unwrap().len(), 1);
        studio
            .save_group(ConnectionGroup {
                id: new_id(),
                name: "Fresh metadata remains writable".into(),
                parent_id: None,
                environment: Environment::Local,
            })
            .unwrap();
        assert!(
            studio
                .bootstrap()
                .unwrap()
                .groups
                .iter()
                .any(|group| group.name == "Fresh metadata remains writable")
        );
        studio.shutdown_and_wait().await.unwrap();
        drop(studio);
        let restarted = Studio::with_secrets(root.path(), secrets).unwrap();
        let fresh_id = new_id();
        start(
            &restarted,
            &fresh_id,
            create_request(),
            "separate recovery test key",
        )
        .unwrap();
        assert_eq!(
            settled(&restarted, &fresh_id).await.state,
            RecoveryOperationState::Succeeded
        );
    }
}
