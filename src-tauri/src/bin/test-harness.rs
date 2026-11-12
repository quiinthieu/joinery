//! Test-only loopback transport for exercising the real core from headless browsers.
//! This binary is excluded from desktop builds and accepts only disposable fixtures.
#[allow(dead_code)] // Shared fixture relay also supports modes exercised by Rust integration tests.
#[path = "../../tests/common/commit_proxy.rs"]
mod commit_proxy;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use commit_proxy::{CommitProxy, Fault, Protocol};
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
    commit_proxies: tokio::sync::Mutex<HashMap<String, CommitProxy>>,
    #[cfg(unix)]
    synthetic_snowflake: Option<joinery_core::snowflake_runtime::studio_fixture::Fixture>,
    stop: Arc<tokio::sync::Notify>,
}

#[cfg(unix)]
fn synthetic_console_profile() -> ConnectionProfile {
    ConnectionProfile {
        id: "00000000-0000-4000-8000-00000000ce01".into(),
        name: "Synthetic retained Snowflake console".into(),
        group_id: "local".into(),
        engine: EngineKind::Snowflake,
        host: "joinery-synthetic.invalid.snowflakecomputing.com".into(),
        port: 443,
        database: "DB".into(),
        username: "synthetic@example.test".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::Confirm,
        tls_mode: TlsMode::VerifyFull,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: Some(SnowflakeConfig {
            authentication: SnowflakeAuthentication::ExternalBrowser,
            reviewed_dml: false,
            reviewed_ddl: false,
            reviewed_console: true,
            reviewed_console_timeout_seconds: None,
            role: "ROLE".into(),
            warehouse: "WH".into(),
            schema: "PUBLIC".into(),
        }),
    }
}

#[cfg(unix)]
fn matches_synthetic_console_profile(profile: &ConnectionProfile) -> bool {
    // Only this bounded budget may vary. Every authentication, identity and
    // other policy field must match the entirely synthetic runtime's profile.
    let mut normalized = profile.clone();
    let Some(config) = &mut normalized.snowflake else {
        return false;
    };
    if config.effective_console_timeout_seconds().is_err() {
        return false;
    }
    config.reviewed_console_timeout_seconds = None;
    let Ok(expected) = serde_json::to_value(synthetic_console_profile()) else {
        return false;
    };
    serde_json::to_value(normalized).is_ok_and(|actual| actual == expected)
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

// Snowflake profile persistence can be exercised offline. This harness must
// never load a genuine SDK. The opt-in retained-console harness owns an entirely
// synthetic runtime and permits only its immutable fixed fixture profile.
fn allow_fixture_authentication(state: &Harness, id: &str) -> AppResult<()> {
    if let Some(profile) = state
        .studio
        .bootstrap()?
        .profiles
        .iter()
        .find(|profile| profile.id == id && profile.engine == EngineKind::Snowflake)
    {
        #[cfg(not(unix))]
        let _ = profile;
        #[cfg(unix)]
        if state.synthetic_snowflake.is_some() && matches_synthetic_console_profile(profile) {
            return Ok(());
        }
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "The browser test harness does not authenticate Snowflake profiles",
        ));
    }
    Ok(())
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
            #[cfg(unix)]
            "test_synthetic_console_evidence" => {
                let fixture = state.synthetic_snowflake.as_ref().ok_or_else(|| AppError::invalid("Synthetic console mode is disabled"))?;
                json!({
                    "queries": std::fs::read_to_string(fixture.controls.join("queries")).unwrap_or_default(),
                    "sdkEvents": std::fs::read_to_string(fixture.controls.join("sdk-events")).unwrap_or_default()
                })
            }
            #[cfg(unix)]
            "test_synthetic_console_shutdown" => {
                if state.synthetic_snowflake.is_none() {
                    return Err(AppError::invalid("Synthetic console mode is disabled"));
                }
                studio.shutdown_and_wait().await?;
                state.stop.notify_one();
                Value::Null
            }
            "test_commit_proxy_start" => {
                let mut proxies = state.commit_proxies.lock().await;
                if proxies.len() >= 4 {
                    return Err(AppError::new(ErrorCode::ResultLimitReached, "Too many owned commit proxies"));
                }
                let upstream = std::env::var("JOINERY_POSTGRES_PORT")
                    .ok().and_then(|value| value.parse::<u16>().ok()).unwrap_or(55432);
                if upstream < 1024 {
                    return Err(AppError::invalid("Commit proxy requires an unprivileged local fixture port"));
                }
                let proxy = CommitProxy::start(Protocol::Postgres, upstream).await;
                let id = uuid::Uuid::new_v4().to_string();
                let result = json!({ "id": id, "port": proxy.port });
                proxies.insert(id, proxy);
                result
            }
            "test_commit_proxy_arm" | "test_commit_proxy_status" => {
                let id: String = arg(&args, "id")?;
                let proxies = state.commit_proxies.lock().await;
                let proxy = proxies.get(&id).ok_or_else(|| AppError::new(ErrorCode::NotFound, "Unknown owned commit proxy"))?;
                if command == "test_commit_proxy_arm" {
                    let fault: String = arg(&args, "fault")?;
                    let fault = match fault.as_str() {
                        "acknowledgement" => Fault::CommitAcknowledgement,
                        "fatal" => Fault::CommitFatalResponse,
                        _ => return Err(AppError::invalid("Unknown commit proxy fault")),
                    };
                    if !proxy.arm(fault) {
                        return Err(AppError::invalid("Commit proxy can only arm once"));
                    }
                }
                json!({ "port": proxy.port, "commits": proxy.commits(), "acknowledged": proxy.acknowledged(), "triggered": proxy.triggered() })
            }
            "test_commit_proxy_stop" => {
                let id: String = arg(&args, "id")?;
                state.commit_proxies.lock().await.remove(&id);
                Value::Null
            }
            "bootstrap" => json!(studio.bootstrap()?),
            "save_workspace" => json!(studio.save_workspace(arg(&args, "workspace")?)?),
            "recovery_backups" => json!(studio.recovery_backups()?),
            "delete_recovery_backup" => json!(studio.delete_recovery_backup(&arg::<String>(&args, "id")?, &arg::<String>(&args, "digest")?, &arg::<String>(&args, "confirmation")?)?),
            "start_metadata_recovery" => json!(studio.start_metadata_recovery(arg(&args, "id")?, arg(&args, "request")?, zeroize::Zeroizing::new(arg(&args, "backupPassphrase")?), zeroize::Zeroizing::new(arg(&args, "backupConfirmation")?), arg::<Option<String>>(&args, "writePassphrase")?.map(zeroize::Zeroizing::new))?),
            "metadata_recovery_operation" => json!(studio.metadata_recovery_operation(&arg::<String>(&args, "id")?)?),
            "cancel_metadata_recovery" => json!(studio.cancel_metadata_recovery(&arg::<String>(&args, "id")?)?),
            "reconcile_metadata_recovery" => json!(studio.reconcile_metadata_recovery(&arg::<String>(&args, "id")?)?),
            "reconcile_unknown_metadata_recovery" => json!(studio.reconcile_unknown_metadata_recovery()?),
            "metadata_recovery_admission_closed" => json!(studio.metadata_recovery_admission_closed()),
            // Test-only isolation for recovery flows; never enrolled in desktop IPC.
            "disconnect_fixtures" => { studio.reset_fixtures().await?; Value::Null },
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
                    (EngineKind::Postgres, port) => port == 55432 || port == pg_port
                        || state.commit_proxies.lock().await.values().any(|proxy| proxy.port == port),
                    (EngineKind::Mysql, 53306 | 53307) => true,
                    (EngineKind::Redis, 56379) => true,
                    _ => false,
                };
                let offline_snowflake = profile.engine == EngineKind::Snowflake
                    && profile.host == "joinery-offline.invalid.snowflakecomputing.com"
                    && profile.port == 443 && profile.database == "DB"
                    && profile.username == "login@example.test"
                    && profile.ssh.is_none() && profile.environment == Environment::Local
                    && profile.safe_mode == SafeMode::ReadOnly
                    && !profile.remember_password;
                #[cfg(unix)]
                let synthetic_console = state.synthetic_snowflake.is_some()
                    && matches_synthetic_console_profile(&profile);
                #[cfg(not(unix))]
                let synthetic_console = false;
                if !offline_snowflake && !synthetic_console && (profile.host != "127.0.0.1"
                    || !allowed_fixture
                    || (profile.engine!=EngineKind::Redis && (profile.database!="joinery"||profile.username!="joinery"))
                    || (profile.engine==EngineKind::Redis && (profile.database!="0"||profile.username!="default"))
                    || profile.ssh.is_some()
                    || profile.environment == Environment::Production)
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
            "prepare_connection" => {
                let id: String = arg(&args, "id")?;
                allow_fixture_authentication(&state, &id)?;
                json!(studio.prepare_connection(&id)?)
            },
            "connect_prepared" => json!(
                studio.connect_prepared(&arg::<String>(&args, "attemptId")?, arg(&args, "secrets")?).await?
            ),
            "cancel_connection_attempt" => {
                studio.cancel_connection_attempt_and_wait(&arg::<String>(&args, "attemptId")?).await?;
                Value::Null
            }
            "app_shutdown" => {
                studio.shutdown_and_wait().await?;
                Value::Null
            }
            "probe_ssh" => json!(studio.probe_ssh_with_attempt(
                &arg::<String>(&args, "profileId")?,
                arg::<Option<String>>(&args, "attemptId")?.as_deref(),
            ).await?),
            "connect" => {
                let id: String = arg(&args, "id")?;
                allow_fixture_authentication(&state, &id)?;
                json!(studio.connect(&id, arg(&args, "secrets")?).await?)
            },
            "disconnect" => {
                studio.disconnect_and_wait(&arg::<String>(&args, "sessionId")?).await?;
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
            "prepare_execution" => json!(studio.prepare_execution_with_params(
                &arg::<String>(&args, "sessionId")?,
                &arg::<String>(&args, "sql")?,
                arg::<Option<Vec<QueryParamInput>>>(&args, "params")?
            )?),
            "authorize_plan" => json!(studio.authorize(
                &arg::<String>(&args, "planId")?,
                arg::<Option<String>>(&args, "passphrase")?.as_deref(),
                arg::<Option<String>>(&args, "confirmation")?.as_deref()
            )?),
            "prepare_snowflake_console" => json!(studio.prepare_snowflake_console(
                &arg::<String>(&args, "sessionId")?,
                &arg(&args, "request")?
            )?),
            "snowflake_console_request" => json!(studio.snowflake_console_request(
                &arg::<String>(&args, "clientRequestId")?,
            )?),
            "snowflake_console_run" => json!(studio.snowflake_console_run(
                &arg::<String>(&args, "clientRequestId")?
            )?),
            "snowflake_console_receipts" => json!(studio.snowflake_console_receipts(
                &arg(&args, "request")?
            )?),
            "snowflake_console_receipt_summary" => json!(studio.snowflake_console_receipt_summary(
                &arg::<String>(&args, "clientRequestId")?
            )?),
            "set_snowflake_console_receipt_archive" => json!(studio.set_snowflake_console_receipt_archive(
                &arg(&args, "request")?
            )?),
            "snowflake_console_pending" => json!(studio.snowflake_console_pending()?),
            "snowflake_console_storage" => json!(studio.snowflake_console_storage()?),
            "release_snowflake_console_results" => {
                studio.release_snowflake_console_results(&arg::<String>(&args, "clientRequestId")?)?;
                json!(null)
            }
            "start_snowflake_console" | "execute_plan" => {
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
                let events: joinery_core::execution::EventHandler = Arc::new(move |event| {
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
                    });
                if command == "start_snowflake_console" {
                    json!(studio.start_snowflake_console(&arg(&args, "request")?, events)?)
                } else {
                    json!(studio.execute(
                        &arg::<String>(&args, "planId")?,
                        arg::<Option<String>>(&args, "approval")?.as_deref(),
                        events
                    )?)
                }
            }
            "cancel_run" => {
                studio.cancel(&arg::<String>(&args, "runId")?).await?;
                Value::Null
            }
            "redis_scan" => json!(studio.redis_scan(&arg::<String>(&args,"sessionId")?, &arg::<String>(&args,"pattern")?, arg::<Option<String>>(&args,"cursor")?.as_deref()).await?),
            "redis_inspect" => json!(studio.redis_inspect(&arg::<String>(&args,"sessionId")?, &arg::<String>(&args,"argument")?, arg::<Option<String>>(&args,"cursor")?.as_deref()).await?),
            "completion_catalog" => json!(studio.completion_catalog(&arg::<String>(&args, "sessionId")?).await?),
            "completion_schema" => json!(studio.completion_schema(&arg::<String>(&args, "sessionId")?, &arg::<String>(&args,"revision")?, &arg(&args,"table")?).await?),
            "inspection_schema" => json!(studio.inspection_schema(&arg::<String>(&args, "sessionId")?, &arg::<String>(&args,"revision")?, &arg(&args,"table")?).await?),
            "relation_catalog" => json!(studio.relation_catalog(&arg::<String>(&args, "sessionId")?).await?),
            "preflight_snowflake_ddl" => json!(studio.preflight_snowflake_ddl(&arg(&args, "request")?).await?),
            "prepare_snowflake_ddl" => json!(studio.prepare_snowflake_ddl(&arg::<String>(&args, "sessionId")?, &arg(&args, "request")?)?),
            "authorize_snowflake_ddl" => json!(studio.authorize_snowflake_ddl(&arg::<String>(&args, "planId")?, arg::<Option<String>>(&args, "passphrase")?.as_deref(), arg::<Option<String>>(&args, "confirmation")?.as_deref(), arg::<Option<String>>(&args, "targetConfirmation")?.as_deref())?),
            "execute_snowflake_ddl" => json!(studio.execute_snowflake_ddl(&arg::<String>(&args, "planId")?, arg::<Option<String>>(&args, "approval")?.as_deref()).await?),
            "refresh_snowflake_ddl_metadata" => json!(studio.refresh_snowflake_ddl_metadata(&arg::<String>(&args, "sessionId")?).await?),
            "inspect_relation" => json!(studio.inspect_relation(&arg::<String>(&args,"sessionId")?, &arg(&args,"request")?).await?),
            "define_relation" => json!(studio.define_relation(&arg::<String>(&args,"sessionId")?, &arg(&args,"request")?).await?),
            "inspect_relation_constraint_mappings" => json!(studio.inspect_relation_constraint_mappings(&arg::<String>(&args,"sessionId")?, &arg(&args,"request")?).await?),
            "inspect_relation_constraints" => json!(studio.inspect_relation_constraints(&arg::<String>(&args,"sessionId")?, &arg(&args,"request")?).await?),
            "inspect_relation_check_constraints" => json!(studio.inspect_relation_check_constraints(&arg::<String>(&args,"sessionId")?, &arg(&args,"request")?).await?),
            "preview_relation" => json!(studio.preview_relation(&arg::<String>(&args,"sessionId")?, &arg(&args,"request")?).await?),
            "catalog" => json!(studio.catalog(&arg::<String>(&args, "sessionId")?).await?),
            "database_objects" => {
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
                        arg(&args, "batchIndex")?,
                        &arg::<String>(&args, "previewToken")?
                    )
                    .await?
            ),
            "release_csv_import_preview" => {
                studio.release_csv_import_preview(
                    &arg::<String>(&args, "sessionId")?,
                    &arg::<String>(&args, "sourceToken")?,
                    &arg::<String>(&args, "previewToken")?,
                );
                Value::Null
            }
            "compare_data" => json!(studio.compare_data(arg(&args, "request")?).await?),
            "session_contexts" => json!(
                studio
                    .session_contexts_with_refresh(&arg::<String>(&args, "sessionId")?, args.get("resumeMetadata").and_then(Value::as_bool).unwrap_or(false))
                    .await?
            ),
            "switch_session_context" => json!(
                studio
                    .switch_session_context(
                        &arg::<String>(&args, "sessionId")?,
                        &arg::<String>(&args, "target")?
                    )
                    .await?
            ),
            "switch_session_schema" => json!(
                studio
                    .switch_session_schema(
                        &arg::<String>(&args, "sessionId")?,
                        &arg(&args, "request")?
                    )
                    .await?
            ),
            "prepare_transaction_control" => json!(studio.prepare_transaction_control(
                &arg::<String>(&args, "sessionId")?, &arg(&args, "request")?
            )?),
            "execute_transaction_control" => json!(studio.execute_transaction_control(
                &arg::<String>(&args, "planId")?,
                arg::<Option<String>>(&args, "approval")?.as_deref()
            ).await?),
            "execute_snowflake_dml" => json!(studio.execute_snowflake_dml(
                &arg::<String>(&args, "planId")?,
                arg::<Option<String>>(&args, "approval")?.as_deref()
            ).await?),
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
            "set_temporary_result_storage" => {
                json!(studio.set_temporary_result_storage(arg(&args, "enabled")?)?)
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
            "test_sandbox_file_summary" => {
                use sha2::{Digest, Sha256};
                use std::io::Read;
                let file_name: String = arg(&args, "fileName")?;
                let path = state.sandbox.resolve_confined(&file_name)?;
                let mut file = std::fs::File::open(path)
                    .map_err(|_| AppError::internal("Could not open disposable sandbox file"))?;
                let mut hash = Sha256::new();
                let mut buffer = [0_u8; 64 * 1024];
                let mut bytes = 0_u64;
                let mut line_breaks = 0_u64;
                loop {
                    let count = file.read(&mut buffer)
                        .map_err(|_| AppError::internal("Could not inspect disposable sandbox file"))?;
                    if count == 0 { break; }
                    bytes += count as u64;
                    line_breaks += buffer[..count].iter().filter(|&&byte| byte == b'\n').count() as u64;
                    hash.update(&buffer[..count]);
                }
                json!({"bytes": bytes, "lineBreaks": line_breaks, "sha256": hex::encode(hash.finalize())})
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
    #[cfg(unix)]
    let synthetic_snowflake =
        if std::env::var("JOINERY_TEST_SYNTHETIC_SNOWFLAKE").as_deref() == Ok("1") {
            Some(joinery_core::snowflake_runtime::studio_fixture::Fixture::separated())
        } else {
            None
        };
    #[cfg(unix)]
    let studio = Studio::with_snowflake_runtime(
        std::path::Path::new(&root),
        Arc::new(MemorySecrets::default()),
        sandbox.clone(),
        synthetic_snowflake
            .as_ref()
            .map(|fixture| fixture.runtime.clone()),
    )
    .unwrap();
    #[cfg(not(unix))]
    let studio = Studio::with_secrets_and_dialog(
        std::path::Path::new(&root),
        Arc::new(MemorySecrets::default()),
        sandbox.clone(),
    )
    .unwrap();
    #[cfg(unix)]
    if synthetic_snowflake.is_some() {
        let mut fixture_profile = synthetic_console_profile();
        if let Some(saved) = studio
            .bootstrap()
            .unwrap()
            .profiles
            .iter()
            .find(|profile| profile.id == fixture_profile.id)
        {
            fixture_profile
                .snowflake
                .as_mut()
                .unwrap()
                .reviewed_console_timeout_seconds = saved
                .snowflake
                .as_ref()
                .and_then(|config| config.reviewed_console_timeout_seconds);
        }
        studio.save_profile(fixture_profile, None).unwrap();
    }
    let stop = Arc::new(tokio::sync::Notify::new());
    let state = Arc::new(Harness {
        studio,
        sandbox,
        token,
        streams: Mutex::new(HashMap::new()),
        export_streams: Mutex::new(HashMap::new()),
        commit_proxies: tokio::sync::Mutex::new(HashMap::new()),
        #[cfg(unix)]
        synthetic_snowflake,
        stop: stop.clone(),
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
    let port = std::env::var("JOINERY_TEST_PORT")
        .map(|value| {
            value
                .parse::<u16>()
                .expect("JOINERY_TEST_PORT must be a port")
        })
        .unwrap_or(1422);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { stop.notified().await })
        .await
        .unwrap();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn synthetic_console_fixture_accepts_only_legacy_or_bounded_timeout_variation() {
        for seconds in [None, Some(1), Some(65), Some(120), Some(86_400)] {
            let mut profile = synthetic_console_profile();
            profile
                .snowflake
                .as_mut()
                .unwrap()
                .reviewed_console_timeout_seconds = seconds;
            assert!(matches_synthetic_console_profile(&profile), "{seconds:?}");
            let restored: ConnectionProfile =
                serde_json::from_value(serde_json::to_value(&profile).unwrap()).unwrap();
            assert!(matches_synthetic_console_profile(&restored));
        }
    }

    #[test]
    fn synthetic_console_fixture_rejects_invalid_timeout_and_absent_configuration() {
        for seconds in [0, 86_401, u32::MAX] {
            let mut profile = synthetic_console_profile();
            profile
                .snowflake
                .as_mut()
                .unwrap()
                .reviewed_console_timeout_seconds = Some(seconds);
            assert!(!matches_synthetic_console_profile(&profile), "{seconds}");
        }
        for value in [
            json!(null),
            json!(0),
            json!(86_401),
            json!(1.5),
            json!("120"),
        ] {
            let mut encoded = serde_json::to_value(synthetic_console_profile()).unwrap();
            encoded["snowflake"]["reviewedConsoleTimeoutSeconds"] = value;
            assert!(serde_json::from_value::<ConnectionProfile>(encoded).is_err());
        }
        let mut profile = synthetic_console_profile();
        profile.snowflake = None;
        assert!(!matches_synthetic_console_profile(&profile));
    }

    #[test]
    fn synthetic_console_fixture_rejects_identity_authentication_and_policy_tampering() {
        for (field, value) in [
            ("id", json!("00000000-0000-4000-8000-00000000ce02")),
            ("name", json!("Other profile")),
            ("groupId", json!("other")),
            ("engine", json!("postgres")),
            ("host", json!("other.snowflakecomputing.com")),
            ("port", json!(444)),
            ("database", json!("OTHER")),
            ("username", json!("other@example.test")),
            ("environment", json!("production")),
            ("safeMode", json!("silent")),
            ("tlsMode", json!("disable")),
            ("caCertPath", json!("/synthetic/other.pem")),
            ("rememberPassword", json!(true)),
            ("storeQueryText", json!(true)),
            ("storeDrafts", json!(true)),
            (
                "ssh",
                json!({"host":"127.0.0.1","port":22,"username":"other",
                    "authentication":"password","privateKeyPath":null}),
            ),
        ] {
            let mut encoded = serde_json::to_value(synthetic_console_profile()).unwrap();
            encoded[field] = value;
            let profile: ConnectionProfile = serde_json::from_value(encoded).unwrap();
            assert!(!matches_synthetic_console_profile(&profile), "{field}");
        }
        for (field, value) in [
            ("authentication", json!("localApplicationOauth")),
            ("reviewedDml", json!(true)),
            ("reviewedDdl", json!(true)),
            ("reviewedConsole", json!(false)),
            ("role", json!("OTHER_ROLE")),
            ("warehouse", json!("OTHER_WH")),
            ("schema", json!("OTHER_SCHEMA")),
        ] {
            let mut encoded = serde_json::to_value(synthetic_console_profile()).unwrap();
            encoded["snowflake"][field] = value;
            let profile: ConnectionProfile = serde_json::from_value(encoded).unwrap();
            assert!(!matches_synthetic_console_profile(&profile), "{field}");
        }
    }
}
