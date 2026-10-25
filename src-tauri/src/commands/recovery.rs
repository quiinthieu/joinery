use super::*;

#[tauri::command]
pub async fn metadata_backups(studio: State<'_, Arc<Studio>>) -> AppResult<Vec<MetadataBackup>> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || studio.metadata_backups())
        .await
        .map_err(join_error)?
}

#[tauri::command]
pub async fn create_metadata_backup(studio: State<'_, Arc<Studio>>) -> AppResult<MetadataBackup> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || studio.create_metadata_backup())
        .await
        .map_err(join_error)?
}

#[tauri::command]
pub async fn delete_metadata_backup(
    studio: State<'_, Arc<Studio>>,
    id: String,
    digest: String,
) -> AppResult<()> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || studio.delete_metadata_backup(&id, &digest))
        .await
        .map_err(join_error)?
}

#[tauri::command]
pub async fn restore_metadata_backup(
    studio: State<'_, Arc<Studio>>,
    id: String,
    digest: String,
    confirmation: String,
    passphrase: Option<String>,
) -> AppResult<MetadataBackup> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || {
        let passphrase = passphrase.map(Zeroizing::new);
        studio.restore_metadata_backup(
            &id,
            &digest,
            &confirmation,
            passphrase.as_deref().map(String::as_str),
        )
    })
    .await
    .map_err(join_error)?
}
