use serde::Serialize;

pub mod domain;
pub mod drivers;
pub mod execution;
mod files;
#[cfg(any(feature = "native-smoke", test))]
mod native_smoke_config;
pub mod persistence;
pub mod results;
pub mod safety;
pub mod secrets;
pub mod ssh;

#[cfg(feature = "desktop")]
mod commands;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

#[cfg_attr(feature = "desktop", tauri::command)]
#[cfg(any(feature = "desktop", test))]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Joinery",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg(feature = "desktop")]
pub fn run() {
    use tauri::{Emitter, Manager};
    #[cfg(feature = "native-smoke")]
    let started = std::time::Instant::now();
    tauri::Builder::default()
        .setup(move |app| {
            app.manage(commands::WindowGuard::default());
            #[cfg(not(feature = "native-smoke"))]
            {
            let root = app.path().app_data_dir()?;
            app.manage(execution::Studio::open(&root)?);
            }
            #[cfg(feature = "native-smoke")]
            {
                let root = std::path::PathBuf::from(std::env::var("JOINERY_NATIVE_SMOKE_ROOT")?);
                let options = match std::fs::read(root.join("options.json")) {
                    Ok(bytes) => native_smoke_config::SmokeOptions::parse(&bytes)
                        .map_err(std::io::Error::other)?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => native_smoke_config::SmokeOptions::default(),
                    Err(error) => return Err(error.into()),
                };
                std::fs::write(root.join("startup.json"), serde_json::to_vec(&serde_json::json!({"pid": std::process::id()}))?)?;
                app.manage(execution::Studio::with_secrets(&root, std::sync::Arc::new(secrets::MemorySecrets::default()))?);
                app.manage(commands::NativeSmokeState { started, root: root.clone(), options });
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(options.timeout_seconds())).await;
                    let _ = std::fs::write(root.join("report.json"), r#"{"passed":false,"error":"Native frontend exceeded its configured test deadline"}"#);
                    handle.exit(1);
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window
                    .state::<std::sync::Arc<execution::Studio>>()
                    .shutdown();
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            commands::bootstrap,
            commands::save_workspace,
            commands::metadata_backups,
            commands::create_metadata_backup,
            commands::restore_metadata_backup,
            commands::delete_metadata_backup,
            commands::acknowledge_quit,
            commands::save_group,
            commands::delete_group,
            commands::audit_log,
            commands::save_profile,
            commands::delete_profile,
            commands::connect,
            commands::disconnect,
            commands::set_passphrase,
            commands::prepare_execution,
            commands::authorize_plan,
            commands::execute_plan,
            commands::cancel_run,
            commands::completion_catalog,
            commands::completion_schema,
            commands::catalog,
            commands::redis_scan,
            commands::redis_inspect,
            commands::table_schema,
            commands::referencing_foreign_keys,
            commands::prepare_browse_table,
            commands::browse_table,
            commands::result_range,
            commands::release_result,
            commands::prepare_changes,
            commands::prepare_changes_batch,
            commands::processes,
            commands::prepare_process_cancel,
            commands::health,
            commands::history,
            commands::clear_history,
            commands::resource_usage,
            commands::probe_ssh,
            commands::trust_ssh_key,
            #[cfg(feature = "native-smoke")]
            commands::native_smoke_fixture,
            #[cfg(feature = "native-smoke")]
            commands::native_smoke_checkpoint,
            #[cfg(feature = "native-smoke")]
            commands::native_smoke_report
        ])
        .build(tauri::generate_context!())
        .expect("could not start Joinery")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
                if let Some(window) = app.get_webview_window("main") {
                    api.prevent_exit();
                    let state = app.state::<commands::WindowGuard>();
                    let generation = state.requested.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    let _ = window.emit("joinery:quit-requested", generation);
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        if handle.state::<commands::WindowGuard>().acknowledged.load(std::sync::atomic::Ordering::Acquire) < generation {
                            handle.exit(0);
                        }
                    });
                }
            } else if matches!(event, tauri::RunEvent::Exit) {
                app.state::<std::sync::Arc<execution::Studio>>().shutdown();
            }
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_version_matches_package() {
        let info = super::app_info();
        assert_eq!(info.name, "Joinery");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }
}
