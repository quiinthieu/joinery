use crate::{domain::*, execution::Studio};
use std::sync::Arc;
use tauri::{
    State,
    ipc::{Channel, InvokeResponseBody},
};
use zeroize::Zeroizing;

#[tauri::command]
pub async fn bootstrap(studio: State<'_, Arc<Studio>>) -> AppResult<Bootstrap> {
    studio.bootstrap()
}

#[tauri::command]
pub async fn save_profile(
    studio: State<'_, Arc<Studio>>,
    profile: ConnectionProfile,
    passphrase: Option<String>,
) -> AppResult<ConnectionProfile> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || {
        let passphrase = passphrase.map(Zeroizing::new);
        studio.save_profile(profile, passphrase.as_deref().map(String::as_str))
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn delete_profile(
    studio: State<'_, Arc<Studio>>,
    id: String,
    passphrase: Option<String>,
) -> AppResult<()> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || {
        let passphrase = passphrase.map(Zeroizing::new);
        studio.delete_profile(&id, passphrase.as_deref().map(String::as_str))
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn connect(
    studio: State<'_, Arc<Studio>>,
    id: String,
    secrets: ConnectionSecrets,
) -> AppResult<SessionInfo> {
    studio.connect(&id, secrets).await
}

#[tauri::command]
pub async fn disconnect(studio: State<'_, Arc<Studio>>, session_id: String) -> AppResult<()> {
    studio.disconnect(&session_id);
    Ok(())
}

#[tauri::command]
pub async fn set_passphrase(
    studio: State<'_, Arc<Studio>>,
    passphrase: String,
    previous: Option<String>,
) -> AppResult<()> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || {
        let passphrase = Zeroizing::new(passphrase);
        let previous = previous.map(Zeroizing::new);
        studio.set_passphrase(&passphrase, previous.as_deref().map(String::as_str))
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn prepare_execution(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
    sql: String,
) -> AppResult<PreparedPlan> {
    studio.prepare_execution(&session_id, &sql)
}

#[tauri::command]
pub async fn authorize_plan(
    studio: State<'_, Arc<Studio>>,
    plan_id: String,
    passphrase: Option<String>,
    confirmation: Option<String>,
) -> AppResult<String> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || {
        let passphrase = passphrase.map(Zeroizing::new);
        studio.authorize(
            &plan_id,
            passphrase.as_deref().map(String::as_str),
            confirmation.as_deref(),
        )
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn execute_plan(
    studio: State<'_, Arc<Studio>>,
    plan_id: String,
    approval: Option<String>,
    on_event: Channel<QueryEvent>,
) -> AppResult<String> {
    studio.inner().execute(
        &plan_id,
        approval.as_deref(),
        Arc::new(move |event| on_event.send(event).is_ok()),
    )
}

#[tauri::command]
pub async fn cancel_run(studio: State<'_, Arc<Studio>>, run_id: String) -> AppResult<()> {
    studio.cancel(&run_id).await
}

#[tauri::command]
pub async fn catalog(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
) -> AppResult<Vec<TableInfo>> {
    studio.catalog(&session_id).await
}

#[tauri::command]
pub async fn table_schema(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
    table: TableRef,
) -> AppResult<TableSchema> {
    studio.schema(&session_id, &table).await
}

#[tauri::command]
pub async fn browse_table(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
    table: TableRef,
    after: Option<Vec<Option<String>>>,
) -> AppResult<TablePage> {
    studio.browse(&session_id, &table, after).await
}

#[tauri::command]
pub async fn result_range(
    studio: State<'_, Arc<Studio>>,
    result_id: String,
    start: usize,
    count: usize,
    on_data: Channel<InvokeResponseBody>,
) -> AppResult<()> {
    let studio = studio.inner().clone();
    let bytes = tokio::task::spawn_blocking(move || studio.range(&result_id, start, count))
        .await
        .map_err(join_error)??;
    on_data
        .send(InvokeResponseBody::Raw(bytes))
        .map_err(|_| AppError::new(ErrorCode::Cancelled, "The result viewer closed"))
}

#[tauri::command]
pub async fn release_result(studio: State<'_, Arc<Studio>>, result_id: String) -> AppResult<()> {
    studio.release_result(&result_id);
    Ok(())
}

#[tauri::command]
pub async fn prepare_changes(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
    result_id: String,
    changes: Vec<CellChange>,
) -> AppResult<ChangePreview> {
    studio.prepare_changes(&session_id, &result_id, changes)
}

#[tauri::command]
pub async fn prepare_changes_batch(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
    inputs: Vec<ResultChanges>,
) -> AppResult<ChangePreview> {
    studio.prepare_changes_batch(&session_id, inputs)
}

#[tauri::command]
pub async fn processes(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
) -> AppResult<Vec<ProcessInfo>> {
    studio.processes(&session_id).await
}

#[tauri::command]
pub async fn prepare_process_cancel(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
    id: String,
    identity: String,
) -> AppResult<PreparedPlan> {
    studio.prepare_process_cancel(&session_id, &id, &identity)
}

#[tauri::command]
pub async fn health(studio: State<'_, Arc<Studio>>, session_id: String) -> AppResult<u32> {
    studio.health(&session_id).await
}

#[tauri::command]
pub async fn history(
    studio: State<'_, Arc<Studio>>,
    search: String,
) -> AppResult<Vec<HistoryEntry>> {
    studio.history(&search)
}

#[tauri::command]
pub async fn clear_history(studio: State<'_, Arc<Studio>>) -> AppResult<()> {
    studio.clear_history()
}

#[tauri::command]
pub async fn resource_usage(studio: State<'_, Arc<Studio>>) -> AppResult<ResourceUsage> {
    Ok(studio.usage())
}

fn join_error(_: tokio::task::JoinError) -> AppError {
    AppError::internal("The background operation could not finish")
}
