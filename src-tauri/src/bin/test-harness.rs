//! Test-only loopback transport for exercising the real core from headless browsers.
//! This binary is excluded from desktop builds and accepts only disposable fixtures.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
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
struct Harness {
    studio: Arc<Studio>,
    token: String,
    streams: Mutex<HashMap<String, Stream>>,
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
                if profile.host != "127.0.0.1"
                    || !matches!((profile.engine,profile.port),(EngineKind::Postgres,55432)|(EngineKind::Mysql,53306|53307)|(EngineKind::Redis,56379))
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
            "prepare_execution" => json!(studio.prepare_execution(
                &arg::<String>(&args, "sessionId")?,
                &arg::<String>(&args, "sql")?
            )?),
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
            "cancel_run" => {
                studio.cancel(&arg::<String>(&args, "runId")?).await?;
                Value::Null
            }
            "redis_scan" => json!(studio.redis_scan(&arg::<String>(&args,"sessionId")?, &arg::<String>(&args,"pattern")?, arg::<Option<String>>(&args,"cursor")?.as_deref()).await?),
            "redis_inspect" => json!(studio.redis_inspect(&arg::<String>(&args,"sessionId")?, &arg::<String>(&args,"argument")?, arg::<Option<String>>(&args,"cursor")?.as_deref()).await?),
            "completion_catalog" => json!(studio.completion_catalog(&arg::<String>(&args, "sessionId")?).await?),
            "completion_schema" => json!(studio.completion_schema(&arg::<String>(&args, "sessionId")?, &arg::<String>(&args,"revision")?, &arg(&args,"table")?).await?),
            "catalog" => json!(studio.catalog(&arg::<String>(&args, "sessionId")?).await?),
            "table_schema" => json!(
                studio
                    .schema(&arg::<String>(&args, "sessionId")?, &arg(&args, "table")?)
                    .await?
            ),
            "browse_table" => json!(
                studio
                    .browse(
                        &arg::<String>(&args, "sessionId")?,
                        &arg(&args, "table")?,
                        arg(&args, "after")?
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
            "processes" => json!(
                studio
                    .processes(&arg::<String>(&args, "sessionId")?)
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
    let studio = Studio::with_secrets(
        std::path::Path::new(&root),
        Arc::new(MemorySecrets::default()),
    )
    .unwrap();
    let state = Arc::new(Harness {
        studio,
        token,
        streams: Mutex::new(HashMap::new()),
    });
    let origin = HeaderValue::from_static("http://127.0.0.1:1421");
    let router = Router::new()
        .route("/rpc/{command}", post(rpc))
        .route("/events/{id}", get(events))
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
