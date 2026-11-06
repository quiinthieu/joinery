//! Test-only loopback transport for exercising the real core from headless browsers.
//! This binary is excluded from desktop builds and accepts only disposable fixtures.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use joinery_core::{
    domain::*, execution::Studio, file_capability::SandboxFileDialog, secrets::MemorySecrets,
};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tower_http::cors::CorsLayer;

struct Stream {
    events: Vec<QueryEvent>,
    touched: Instant,
    finished: bool,
}
struct ExportStream {
    events: Vec<ExportEvent>,
    touched: Instant,
    finished: bool,
}
struct Harness {
    studio: Arc<Studio>,
    sandbox: Arc<SandboxFileDialog>,
    token: String,
    streams: Mutex<HashMap<String, Stream>>,
    export_streams: Mutex<HashMap<String, ExportStream>>,
}

fn authorized(headers: &HeaderMap, state: &Harness) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {}", state.token))
}

fn arg<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> AppResult<T> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|_| AppError::invalid(format!("Invalid request field: {key}")))
}

async fn rpc(
    State(state): State<Arc<Harness>>,
    Path(command): Path<String>,
    headers: HeaderMap,
    Json(args): Json<Value>,
) -> Response {
    if !authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let outcome: AppResult<Value> = async {
        let studio = &state.studio;
        Ok(match command.as_str() {
            "bootstrap" => json!(studio.bootstrap()?),
            "save_workspace" => json!(studio.save_workspace(arg(&args, "workspace")?)?),
            // Test-only isolation for recovery flows; never enrolled in desktop IPC.
            "disconnect_fixtures" => { studio.shutdown(); Value::Null },
            "metadata_backups" => json!(studio.metadata_backups()?),
            "create_metadata_backup" => json!(studio.create_metadata_backup()?),
            "delete_metadata_backup" => json!(studio.delete_metadata_backup(&arg::<String>(&args, "id")?, &arg::<String>(&args, "digest")?)?),
            "restore_metadata_backup" => json!(studio.restore_metadata_backup(&arg::<String>(&args, "id")?, &arg::<String>(&args, "digest")?, &arg::<String>(&args, "confirmation")?, arg::<Option<String>>(&args, "passphrase")?.as_deref())?),
            "save_group" => json!(studio.save_group(arg(&args, "group")?)?),
            "delete_group" => {
                studio.delete_group(&arg::<String>(&args, "id")?)?;
                Value::Null
            }
            "audit_log" => json!(studio.audit_log()?),
            "save_profile" => {
                let profile: ConnectionProfile = arg(&args, "profile")?;
                let pg_port = std::env::var("JOINERY_POSTGRES_PORT")
                    .ok()
                    .and_then(|v| v.parse::<u16>().ok())
                    .unwrap_or(55432);
                let allowed_fixture = match (profile.engine, profile.port) {
                    (EngineKind::Postgres, port) => port == 55432 || port == pg_port,
                    (EngineKind::Mysql, 53306 | 53307) => true,
                    (EngineKind::Redis, 56379) => true,
                    _ => false,
                };
                if profile.host != "127.0.0.1"
                    || !allowed_fixture
                    || (profile.engine!=EngineKind::Redis && (profile.database!="joinery"||profile.username!="joinery"))
                    || (profile.engine==EngineKind::Redis && (profile.database!="0"||profile.username!="default"))
                    || profile.ssh.is_some()
                    || profile.environment == Environment::Production
                {
                    return Err(AppError::invalid(
                        "The browser test harness accepts only the local disposable database fixtures",
                    ));
                }
                json!(studio.save_profile(
                    profile,
                    arg::<Option<String>>(&args, "passphrase")?.as_deref()
                )?)
            }
            "delete_profile" => {
                studio.delete_profile(&arg::<String>(&args, "id")?)?;
                Value::Null
            }
            "connect" => json!(
                studio
                    .connect(&arg::<String>(&args, "id")?, arg(&args, "secrets")?)
                    .await?
            ),
            "disconnect" => {
                studio.disconnect(&arg::<String>(&args, "sessionId")?);
                Value::Null
            }
            "set_passphrase" => {
                studio.set_passphrase(
                    &arg::<String>(&args, "passphrase")?,
                    arg::<Option<String>>(&args, "previous")?.as_deref(),
                )?;
                Value::Null
            }
            "extract_query_params" => json!(joinery_core::params::extract_query_params(
                arg::<EngineKind>(&args, "engine")?,
                &arg::<String>(&args, "sql")?
            )?),
            "prepare_explain" => json!(studio.prepare_explain_with_params(
                &arg::<String>(&args, "sessionId")?,
                &arg::<String>(&args, "sql")?,
                arg::<Option<bool>>(&args, "analyze")?.unwrap_or(false),
                arg::<Option<Vec<QueryParamInput>>>(&args, "params")?
            )?),
            "prepare_execution" | "prepare_plan" => {
                let params = arg::<Option<Vec<QueryParamInput>>>(&args, "params")?
                    .or_else(|| arg::<Option<Vec<QueryParamInput>>>(&args, "boundParameters").ok().flatten());
                let error_policy = arg::<Option<BatchErrorPolicy>>(&args, "errorPolicy")?
                    .unwrap_or_default();
                json!(studio.prepare_execution_with_params_and_policy(
                    &arg::<String>(&args, "sessionId")?,
                    &arg::<String>(&args, "sql")?,
                    params,
                    error_policy,
                )?)
            }
            "authorize_plan" => json!(studio.authorize(
                &arg::<String>(&args, "planId")?,
                arg::<Option<String>>(&args, "passphrase")?.as_deref(),
                arg::<Option<String>>(&args, "confirmation")?.as_deref()
            )?),
            "execute_plan" => {
                let stream_id: String = arg(&args, "streamId")?;
                uuid::Uuid::parse_str(&stream_id)
                    .map_err(|_| AppError::invalid("Invalid stream identifier"))?;
                {
                    let mut streams = state.streams.lock();
                    streams.retain(|_, s| s.touched.elapsed().as_secs() < 120);
                    if streams.len() >= 32 {
                        return Err(AppError::new(
                            ErrorCode::ResultLimitReached,
                            "Too many browser test streams",
                        ));
                    }
                    streams.insert(
                        stream_id.clone(),
                        Stream {
                            events: Vec::new(),
                            touched: Instant::now(),
                            finished: false,
                        },
                    );
                }
                let owner = state.clone();
                json!(studio.execute(
                    &arg::<String>(&args, "planId")?,
                    arg::<Option<String>>(&args, "approval")?.as_deref(),
                    Arc::new(move |event| {
                        let mut streams = owner.streams.lock();
                        let Some(stream) = streams.get_mut(&stream_id) else {
                            return false;
                        };
                        if stream.events.len() >= 4096 {
                            return false;
                        }
                        stream.finished = matches!(event, QueryEvent::Finished { .. });
                        stream.events.push(event);
                        true
                    })
                )?)
            }
            "cancel_run" | "cancel_query" => {
                let run_id = arg::<Option<String>>(&args, "runId")?;
                let query_id = arg::<Option<String>>(&args, "queryId")?;
                if let Some(r) = run_id {
                    studio.cancel(&r).await?;
                } else if let Some(q) = query_id {
                    studio.cancel_by_query_id(&q).await?;
                }
                json!({ "success": true })
            }
            "get_console" | "load_console" => json!(studio.get_console(&arg::<String>(&args, "profileId")?)?),
            "save_console" => {
                studio.save_console(
                    &arg::<String>(&args, "profileId")?,
                    &arg::<String>(&args, "content")?,
                    arg::<Option<String>>(&args, "name")?.as_deref(),
                )?;
                json!({ "success": true })
            }
            "list_scratchpads" => json!(studio.scratchpads()?),
            "save_scratchpad" => {
                let (id, name, content, connection_id) = if let Some(sp) = args.get("scratchpad") {
                    (
                        arg::<Option<String>>(sp, "id")?,
                        arg::<Option<String>>(sp, "name")?,
                        arg::<String>(sp, "content")?,
                        arg::<Option<String>>(sp, "connectionId")?,
                    )
                } else {
                    (
                        arg::<Option<String>>(&args, "id")?,
                        arg::<Option<String>>(&args, "name")?,
                        arg::<String>(&args, "content")?,
                        arg::<Option<String>>(&args, "connectionId")?,
                    )
                };
                json!(studio.save_scratchpad(id, name, content, connection_id)?)
            }
            "delete_scratchpad" => {
                studio.delete_scratchpad(&arg::<String>(&args, "id")?)?;
                Value::Null
            }
            "get_parameter_history" => json!(studio.persistence().parameter_history(
                &arg::<String>(&args, "profileId")?
            )?),
            "save_parameter_history" => {
                studio.persistence().save_parameter_history(
                    &arg::<String>(&args, "profileId")?,
                    &arg::<Vec<QueryParamInput>>(&args, "entries")?,
                )?;
                Value::Null
            }
            "redis_scan" => json!(studio.redis_scan(&arg::<String>(&args,"sessionId")?, &arg::<String>(&args,"pattern")?, arg::<Option<String>>(&args,"cursor")?.as_deref()).await?),
            "redis_inspect" => json!(studio.redis_inspect(&arg::<String>(&args,"sessionId")?, &arg::<String>(&args,"argument")?, arg::<Option<String>>(&args,"cursor")?.as_deref()).await?),
            "completion_catalog" => json!(studio.completion_catalog(&arg::<String>(&args, "sessionId")?).await?),
            "completion_schema" => json!(studio.completion_schema(&arg::<String>(&args, "sessionId")?, &arg::<String>(&args,"revision")?, &arg(&args,"table")?).await?),
            "inspection_schema" => json!(studio.inspection_schema(&arg::<String>(&args, "sessionId")?, &arg::<String>(&args,"revision")?, &arg(&args,"table")?).await?),
            "catalog" => json!(studio.catalog(&arg::<String>(&args, "sessionId")?).await?),
            "database_objects" | "catalog_objects" => {
                json!(studio.database_objects(&arg::<String>(&args, "sessionId")?).await?)
            }
            "object_details" => json!(
                studio
                    .object_details(
                        &arg::<String>(&args, "sessionId")?,
                        &arg(&args, "object")?
                    )
                    .await?
            ),
            "table_schema" => json!(
                studio
                    .schema(&arg::<String>(&args, "sessionId")?, &arg(&args, "table")?)
                    .await?
            ),
            "referencing_foreign_keys" => json!(
                studio
                    .referencing_foreign_keys(
                        &arg::<String>(&args, "sessionId")?,
                        &arg(&args, "table")?
                    )
                    .await?
            ),
            "schema_diagram" => json!(
                studio
                    .schema_diagram(
                        &arg::<String>(&args, "sessionId")?,
                        &arg::<String>(&args, "schema")?
                    )
                    .await?
            ),
            "schema_snapshot" => json!(
                studio
                    .schema_snapshot(
                        &arg::<String>(&args, "sessionId")?,
                        &arg::<String>(&args, "schema")?
                    )
                    .await?
            ),
            "compare_schemas" => json!(studio.compare_schemas(
                arg(&args, "left")?,
                arg(&args, "right")?
            )?),
            "generate_schema_ddl" => json!(studio.generate_schema_ddl(
                arg(&args, "snapshot")?,
                arg::<Option<String>>(&args, "targetSchema")?.as_deref()
            )?),
            "generate_migration_ddl" => json!(studio.generate_migration_ddl(
                arg(&args, "left")?,
                arg(&args, "right")?,
                arg::<Option<String>>(&args, "targetSchema")?.as_deref()
            )?),
            "preview_index_constraint_ddl" => json!(studio.preview_index_constraint_ddl(
                arg(&args, "engine")?,
                arg(&args, "action")?
            )?),
            "prepare_index_constraint_ddl" => json!(studio.prepare_index_constraint_ddl(
                &arg::<String>(&args, "sessionId")?,
                arg(&args, "action")?
            )?),
            "preview_table_object_ddl" => json!(studio.preview_table_object_ddl(
                arg(&args, "engine")?,
                arg(&args, "action")?
            )?),
            "prepare_table_object_ddl" => json!(studio.prepare_table_object_ddl(
                &arg::<String>(&args, "sessionId")?,
                arg(&args, "action")?
            )?),
            "preview_table_maintenance" => json!(studio.preview_table_maintenance(
                arg(&args, "engine")?,
                arg(&args, "action")?
            )?),
            "prepare_table_maintenance" => json!(studio.prepare_table_maintenance(
                &arg::<String>(&args, "sessionId")?,
                arg(&args, "action")?
            )?),
            "prepare_migration_ddl" => json!(
                studio
                    .prepare_migration_ddl(
                        &arg::<String>(&args, "sessionId")?,
                        arg(&args, "expectedSnapshot")?,
                        arg(&args, "statements")?,
                    )
                    .await?
            ),
            "prepare_browse_table" => json!(
                studio
                    .prepare_browse(
                        &arg::<String>(&args, "sessionId")?,
                        &arg(&args, "table")?,
                        &arg(&args, "query")?
                    )
                    .await?
            ),
            "browse_table" => json!(
                studio
                    .browse_with_approval(
                        &arg::<String>(&args, "sessionId")?,
                        &arg(&args, "table")?,
                        arg(&args, "after")?,
                        &arg::<Option<BrowseQuery>>(&args, "query")?.unwrap_or_default(),
                        arg::<Option<BrowseApproval>>(&args, "approval")?.as_ref()
                    )
                    .await?
            ),
            "release_result" => {
                studio.release_result(&arg::<String>(&args, "resultId")?);
                Value::Null
            }
            "prepare_changes" => json!(studio.prepare_changes(
                &arg::<String>(&args, "sessionId")?,
                &arg::<String>(&args, "resultId")?,
                arg(&args, "changes")?
            )?),
            "prepare_changes_batch" => json!(studio.prepare_changes_batch(
                &arg::<String>(&args, "sessionId")?,
                arg(&args, "inputs")?
            )?),
            "prepare_table_changes" => json!(studio.prepare_table_changes(
                &arg::<String>(&args, "sessionId")?,
                arg(&args, "changes")?
            )?),
            "processes" => json!(
                studio
                    .processes(&arg::<String>(&args, "sessionId")?)
                    .await?
            ),
            "lock_waits" => json!(
                studio
                    .lock_waits(&arg::<String>(&args, "sessionId")?)
                    .await?
            ),
            "prepare_process_cancel" => json!(studio.prepare_process_cancel(
                &arg::<String>(&args, "sessionId")?,
                &arg::<String>(&args, "id")?,
                &arg::<String>(&args, "identity")?
            )?),
            "health" => json!(studio.health(&arg::<String>(&args, "sessionId")?).await?),
            "history" => json!(studio.history(&arg::<String>(&args, "search")?)?),
            "clear_history" => {
                studio.clear_history()?;
                Value::Null
            }
            "resource_usage" => json!(studio.usage()),
            "pick_export_destination" => json!(
                studio
                    .pick_export_destination(
                        &arg::<String>(&args, "suggestedName")?,
                        arg(&args, "format")?
                    )
                    .await?
            ),
            "start_export" => {
                let stream_id: String = arg(&args, "streamId")?;
                uuid::Uuid::parse_str(&stream_id)
                    .map_err(|_| AppError::invalid("Invalid stream identifier"))?;
                {
                    let mut streams = state.export_streams.lock();
                    streams.retain(|_, s| s.touched.elapsed().as_secs() < 120);
                    if streams.len() >= 32 {
                        return Err(AppError::new(
                            ErrorCode::ResultLimitReached,
                            "Too many browser test export streams",
                        ));
                    }
                    streams.insert(
                        stream_id.clone(),
                        ExportStream {
                            events: Vec::new(),
                            touched: Instant::now(),
                            finished: false,
                        },
                    );
                }
                let owner = state.clone();
                json!(
                    studio
                        .start_export(
                            arg(&args, "request")?,
                            Arc::new(move |event| {
                                let mut streams = owner.export_streams.lock();
                                let Some(stream) = streams.get_mut(&stream_id) else {
                                    return false;
                                };
                                if stream.events.len() >= 4096 {
                                    return false;
                                };
                                stream.finished = matches!(event, ExportEvent::Finished { .. });
                                stream.events.push(event);
                                true
                            })
                        )
                        .await?
                )
            }
            "cancel_export" => {
                studio.cancel_export(&arg::<String>(&args, "exportId")?)?;
                Value::Null
            }
            "pick_import_source" => json!(studio.pick_import_source().await?),
            "preview_csv_import" => json!(
                studio
                    .preview_csv_import(
                        &arg::<String>(&args, "sessionId")?,
                        &arg::<String>(&args, "sourceToken")?,
                        &arg(&args, "table")?,
                        arg(&args, "options")?,
                        arg(&args, "mappings")?
                    )
                    .await?
            ),
            "prepare_csv_import_batch" => json!(
                studio
                    .prepare_csv_import_batch(
                        &arg::<String>(&args, "sessionId")?,
                        &arg::<String>(&args, "sourceToken")?,
                        &arg(&args, "table")?,
                        arg(&args, "options")?,
                        arg(&args, "mappings")?,
                        arg(&args, "batchIndex")?
                    )
                    .await?
            ),
            "compare_data" => json!(studio.compare_data(arg(&args, "request")?).await?),
            "session_contexts" => json!(
                studio
                    .session_contexts(&arg::<String>(&args, "sessionId")?)
                    .await?
            ),
            "list_contexts" => json!(
                studio
                    .list_contexts(&arg::<String>(&args, "sessionId")?)
                    .await?
            ),
            "switch_session_context" => json!(
                studio
                    .switch_session_context_4d(
                        &arg::<String>(&args, "sessionId")?,
                        arg::<Option<String>>(&args, "target")?.as_deref(),
                        arg::<Option<String>>(&args, "warehouse")?.as_deref(),
                        arg::<Option<String>>(&args, "role")?.as_deref(),
                        arg::<Option<String>>(&args, "database")?.as_deref(),
                        arg::<Option<String>>(&args, "schema")?.as_deref(),
                    )
                    .await?
            ),
            "set_transaction_mode" => json!(
                studio
                    .set_transaction_mode(
                        &arg::<String>(&args, "sessionId")?,
                        arg(&args, "mode")?
                    )
                    .await?
            ),
            "commit_transaction" => json!(
                studio
                    .commit_transaction(&arg::<String>(&args, "sessionId")?)
                    .await?
            ),
            "rollback_transaction" => json!(
                studio
                    .rollback_transaction(&arg::<String>(&args, "sessionId")?)
                    .await?
            ),
            "open_sql_file" => json!(studio.open_sql_file().await?),
            "save_sql_file" => json!(
                studio
                    .save_sql_file(
                        arg::<Option<String>>(&args, "token")?.as_deref(),
                        &arg::<String>(&args, "suggestedName")?,
                        &arg::<String>(&args, "content")?
                    )
                    .await?
            ),
            "release_sql_file_grant" => {
                studio.release_sql_file_grant(&arg::<String>(&args, "token")?);
                Value::Null
            }
            "save_saved_query" => json!(studio.save_saved_query(arg(&args, "input")?)?),
            "delete_saved_query" => {
                studio.delete_saved_query(&arg::<String>(&args, "id")?)?;
                Value::Null
            }
            "save_bookmark" => json!(studio.save_bookmark(
                &arg::<String>(&args, "label")?,
                arg(&args, "target")?
            )?),
            "delete_bookmark" => {
                studio.delete_bookmark(&arg::<String>(&args, "id")?)?;
                Value::Null
            }
            "save_sql_format_settings" => {
                json!(studio.save_sql_format_settings(arg(&args, "settings")?)?)
            }
            "save_sql_snippet" => json!(studio.save_sql_snippet(arg(&args, "input")?)?),
            "delete_sql_snippet" => {
                studio.delete_sql_snippet(&arg::<String>(&args, "id")?)?;
                Value::Null
            }
            // Test-only confined sandbox helpers for browser E2E file verification; never in desktop IPC.
            "test_queue_open_file" => {
                let file_name: Option<String> = arg(&args, "fileName")?;
                state.sandbox.queue_open(file_name.as_deref())?;
                Value::Null
            }
            "test_queue_save_file" => {
                let file_name: Option<String> = arg(&args, "fileName")?;
                state.sandbox.queue_save(file_name.as_deref())?;
                Value::Null
            }
            "test_write_sandbox_file" => {
                let file_name: String = arg(&args, "fileName")?;
                let contents: String = arg(&args, "contents")?;
                if contents.len() > 20 * 1024 * 1024 {
                    return Err(AppError::invalid("Sandbox test file exceeds 20 MiB"));
                }
                let path = state.sandbox.resolve_confined(&file_name)?;
                std::fs::write(&path, contents.as_bytes())
                    .map_err(|_| AppError::internal("Could not write disposable sandbox file"))?;
                state.sandbox.queue_open(Some(&file_name))?;
                Value::Null
            }
            "test_read_sandbox_file" => {
                let file_name: String = arg(&args, "fileName")?;
                let path = state.sandbox.resolve_confined(&file_name)?;
                match std::fs::read_to_string(&path) {
                    Ok(contents) => json!({ "exists": true, "contents": contents }),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        json!({ "exists": false, "contents": Value::Null })
                    }
                    Err(_) => {
                        return Err(AppError::internal(
                            "Could not read disposable sandbox file",
                        ));
                    }
                }
            }
            _ => return Err(AppError::new(ErrorCode::NotFound, "Unknown test command")),
        })
    }
    .await;
    match outcome {
        Ok(result) => Json(json!({"result":result})).into_response(),
        Err(error) => Json(json!({"error":error})).into_response(),
    }
}

async fn events(
    State(state): State<Arc<Harness>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut streams = state.streams.lock();
    let Some(stream) = streams.get_mut(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    stream.touched = Instant::now();
    let events = std::mem::take(&mut stream.events);
    let finished = stream.finished;
    if finished {
        streams.remove(&id);
    }
    Json(events).into_response()
}

async fn export_events(
    State(state): State<Arc<Harness>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut streams = state.export_streams.lock();
    let Some(stream) = streams.get_mut(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    stream.touched = Instant::now();
    let events = std::mem::take(&mut stream.events);
    let finished = stream.finished;
    if finished {
        streams.remove(&id);
    }
    Json(events).into_response()
}

#[derive(Deserialize)]
struct Range {
    start: usize,
    count: usize,
}
async fn range(
    State(state): State<Arc<Harness>>,
    Path(id): Path<String>,
    Query(range): Query<Range>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.studio.range(&id, range.start, range.count) {
        Ok(bytes) => (
            [("content-type", "application/vnd.apache.arrow.stream")],
            bytes,
        )
            .into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(error)).into_response(),
    }
}

#[tokio::main]
async fn main() {
    assert_eq!(
        std::env::var("JOINERY_TEST_BRIDGE").as_deref(),
        Ok("1"),
        "This test binary must be explicitly enabled"
    );
    let token = std::env::var("JOINERY_TEST_TOKEN").expect("Set a test-only transport token");
    assert!(
        token.len() >= 16,
        "Use a test transport token of at least 16 characters"
    );
    let root = std::env::var("JOINERY_TEST_DATA_ROOT")
        .expect("Set a dedicated disposable test data directory");
    let sandbox =
        SandboxFileDialog::new(&std::path::Path::new(&root).join("sandbox-files")).unwrap();
    let studio = Studio::with_secrets_and_dialog(
        std::path::Path::new(&root),
        Arc::new(MemorySecrets::default()),
        sandbox.clone(),
    )
    .unwrap();
    let state = Arc::new(Harness {
        studio,
        sandbox,
        token,
        streams: Mutex::new(HashMap::new()),
        export_streams: Mutex::new(HashMap::new()),
    });
    let origin = HeaderValue::from_static("http://127.0.0.1:1421");
    let router = Router::new()
        .route("/rpc/{command}", post(rpc))
        .route("/events/{id}", get(events))
        .route("/export-events/{id}", get(export_events))
        .route("/result/{id}", get(range))
        .route("/health", get(|| async { "ready" }))
        .layer(
            CorsLayer::new()
                .allow_origin(origin)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([
                    axum::http::header::AUTHORIZATION,
                    axum::http::header::CONTENT_TYPE,
                ]),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:1422")
        .await
        .unwrap();
    axum::serve(listener, router).await.unwrap();
}
