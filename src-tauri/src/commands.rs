use crate::{domain::*, execution::Studio};
use std::sync::Arc;
use tauri::{
    State,
    ipc::{Channel, InvokeResponseBody},
};
use zeroize::Zeroizing;

mod recovery;
pub use recovery::*;

#[derive(Default)]
pub struct WindowGuard {
    pub requested: std::sync::atomic::AtomicU64,
    pub acknowledged: std::sync::atomic::AtomicU64,
}

#[tauri::command]
pub async fn acknowledge_quit(state: State<'_, WindowGuard>, generation: u64) -> AppResult<()> {
    state
        .acknowledged
        .fetch_max(generation, std::sync::atomic::Ordering::Release);
    Ok(())
}

#[tauri::command]
pub async fn bootstrap(studio: State<'_, Arc<Studio>>) -> AppResult<Bootstrap> {
    studio.bootstrap()
}

#[tauri::command]
pub async fn save_group(
    studio: State<'_, Arc<Studio>>,
    group: ConnectionGroup,
) -> AppResult<ConnectionGroup> {
    studio.save_group(group)
}
#[tauri::command]
pub async fn delete_group(studio: State<'_, Arc<Studio>>, id: String) -> AppResult<()> {
    studio.delete_group(&id)
}
#[tauri::command]
pub async fn audit_log(studio: State<'_, Arc<Studio>>) -> AppResult<Vec<AuditEntry>> {
    studio.audit_log()
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
pub async fn delete_profile(studio: State<'_, Arc<Studio>>, id: String) -> AppResult<()> {
    let studio = studio.inner().clone();
    tokio::task::spawn_blocking(move || studio.delete_profile(&id))
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
pub async fn probe_ssh(
    studio: State<'_, Arc<Studio>>,
    profile_id: String,
) -> AppResult<SshHostKey> {
    studio.probe_ssh(&profile_id).await
}

#[tauri::command]
pub async fn trust_ssh_key(
    studio: State<'_, Arc<Studio>>,
    challenge_id: String,
    fingerprint: String,
) -> AppResult<()> {
    studio.trust_ssh_key(&challenge_id, &fingerprint)
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

#[cfg(feature = "native-smoke")]
pub struct NativeSmokeState {
    pub started: std::time::Instant,
    pub root: std::path::PathBuf,
    pub options: crate::native_smoke_config::SmokeOptions,
}

#[cfg(feature = "native-smoke")]
#[tauri::command]
pub async fn native_smoke_fixture(studio: State<'_, Arc<Studio>>) -> AppResult<ResultInfo> {
    let columns = ["exact_integer", "null_value", "empty_value", "unicode"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| ColumnInfo {
            id: format!("c{i}"),
            name: name.into(),
            database_type: if i == 0 { "bigint" } else { "text" }.into(),
            kind: if i == 0 {
                ColumnKind::Integer
            } else {
                ColumnKind::Text
            },
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
        })
        .collect();
    let result = studio.results.create("native-smoke", 0, columns, None)?;
    for _ in 0..40 {
        studio.results.append(
            &result.id,
            (0..500)
                .map(|_| {
                    vec![
                        Some("9223372036854775807".into()),
                        None,
                        Some("".into()),
                        Some("héllo;世界".into()),
                    ]
                })
                .collect(),
        )?;
    }
    studio.results.finish(&result.id, false)
}

#[cfg(feature = "native-smoke")]
#[tauri::command]
pub async fn native_smoke_checkpoint(
    state: State<'_, NativeSmokeState>,
    studio: State<'_, Arc<Studio>>,
    phase: String,
    cycle: Option<u32>,
) -> AppResult<serde_json::Value> {
    if !matches!(
        phase.as_str(),
        "shell" | "loading" | "loaded" | "cycling" | "released" | "idle"
    ) {
        return Err(AppError::invalid("Unknown native test phase"));
    }
    if cycle.is_some_and(|cycle| phase != "cycling" || cycle > state.options.cleanup_cycles) {
        return Err(AppError::invalid("Invalid native test cycle"));
    }
    let checkpoint = serde_json::json!({
        "phase": phase,
        "cycle": cycle,
        "options": state.options,
        "nativeElapsedMs": state.started.elapsed().as_millis() as u64,
        "usage": studio.usage(),
    });
    std::fs::write(
        state.root.join("checkpoint.json"),
        serde_json::to_vec(&checkpoint)
            .map_err(|_| AppError::internal("Could not encode native test checkpoint"))?,
    )
    .map_err(|_| AppError::internal("Could not write native test checkpoint"))?;
    Ok(checkpoint)
}

#[cfg(feature = "native-smoke")]
#[tauri::command]
pub async fn native_smoke_report(
    app: tauri::AppHandle,
    state: State<'_, NativeSmokeState>,
    mut report: serde_json::Value,
) -> AppResult<()> {
    if report.to_string().len() > 16 * 1024 {
        return Err(AppError::invalid("Native test report is too large"));
    }
    report["nativeElapsedMs"] = serde_json::json!(state.started.elapsed().as_millis() as u64);
    report["pid"] = serde_json::json!(std::process::id());
    let passed = report["passed"] == true;
    std::fs::write(
        state.root.join("report.json"),
        serde_json::to_vec_pretty(&report)
            .map_err(|_| AppError::internal("Could not encode native test report"))?,
    )
    .map_err(|_| AppError::internal("Could not write native test report"))?;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        app.exit(if passed { 0 } else { 1 });
    });
    Ok(())
}

#[tauri::command]
pub async fn redis_scan(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
    pattern: String,
    cursor: Option<String>,
) -> AppResult<RedisScanPage> {
    studio
        .redis_scan(&session_id, &pattern, cursor.as_deref())
        .await
}
#[tauri::command]
pub async fn redis_inspect(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
    argument: String,
    cursor: Option<String>,
) -> AppResult<RedisValuePreview> {
    studio
        .redis_inspect(&session_id, &argument, cursor.as_deref())
        .await
}

#[tauri::command]
pub async fn completion_catalog(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
) -> AppResult<CompletionCatalog> {
    studio.completion_catalog(&session_id).await
}
#[tauri::command]
pub async fn completion_schema(
    studio: State<'_, Arc<Studio>>,
    session_id: String,
    revision: String,
    table: TableRef,
) -> AppResult<TableSchema> {
    studio
        .completion_schema(&session_id, &revision, &table)
        .await
}
