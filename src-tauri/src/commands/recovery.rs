use super::*;

#[tauri::command]
pub fn metadata_recovery_admission_closed(studio: State<'_, Arc<Studio>>) -> bool {
    studio.metadata_recovery_admission_closed()
}

#[tauri::command]
pub async fn reconcile_unknown_metadata_recovery(
    studio: State<'_, Arc<Studio>>,
) -> AppResult<RecoveryOperation> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || studio.reconcile_unknown_metadata_recovery())
        .await
        .map_err(join_error)?
}

#[tauri::command]
pub async fn reconcile_metadata_recovery(
    studio: State<'_, Arc<Studio>>,
    id: String,
) -> AppResult<RecoveryOperation> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || studio.reconcile_metadata_recovery(&id))
        .await
        .map_err(join_error)?
}

#[tauri::command]
pub async fn recovery_backups(studio: State<'_, Arc<Studio>>) -> AppResult<Vec<RecoveryBackup>> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || studio.recovery_backups())
        .await
        .map_err(join_error)?
}

#[tauri::command]
pub async fn delete_recovery_backup(
    studio: State<'_, Arc<Studio>>,
    id: String,
    digest: String,
    confirmation: String,
) -> AppResult<()> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || studio.delete_recovery_backup(&id, &digest, &confirmation))
        .await
        .map_err(join_error)?
}

#[tauri::command]
pub async fn start_metadata_recovery(
    studio: State<'_, Arc<Studio>>,
    id: String,
    request: RecoveryRequest,
    backup_passphrase: String,
    backup_confirmation: String,
    write_passphrase: Option<String>,
) -> AppResult<RecoveryOperation> {
    let studio = studio.inner().clone();
    let backup_passphrase = Zeroizing::new(backup_passphrase);
    let backup_confirmation = Zeroizing::new(backup_confirmation);
    let write_passphrase = write_passphrase.map(Zeroizing::new);
    tokio::task::spawn_blocking(move || {
        studio.start_metadata_recovery(
            id,
            request,
            backup_passphrase,
            backup_confirmation,
            write_passphrase,
        )
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn metadata_recovery_operation(
    studio: State<'_, Arc<Studio>>,
    id: String,
) -> AppResult<RecoveryOperation> {
    studio.metadata_recovery_operation(&id)
}

#[tauri::command]
pub async fn cancel_metadata_recovery(
    studio: State<'_, Arc<Studio>>,
    id: String,
) -> AppResult<RecoveryOperation> {
    studio.cancel_metadata_recovery(&id)
}

#[tauri::command]
pub async fn save_workspace(
    studio: State<'_, Arc<Studio>>,
    workspace: SavedWorkspace,
) -> AppResult<()> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || studio.save_workspace(workspace))
        .await
        .map_err(join_error)?
}
