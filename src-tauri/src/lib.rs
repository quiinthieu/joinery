use serde::Serialize;

pub mod domain;
pub mod drivers;
pub mod execution;
pub mod persistence;
pub mod results;
pub mod safety;
pub mod secrets;

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
    use tauri::Manager;
    tauri::Builder::default()
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            app.manage(execution::Studio::open(&root)?);
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
            commands::save_profile,
            commands::delete_profile,
            commands::connect,
            commands::disconnect,
            commands::set_passphrase,
            commands::prepare_execution,
            commands::authorize_plan,
            commands::execute_plan,
            commands::cancel_run,
            commands::catalog,
            commands::table_schema,
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
            commands::resource_usage
        ])
        .run(tauri::generate_context!())
        .expect("could not start Joinery");
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
