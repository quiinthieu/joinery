#[cfg(feature = "desktop")]
fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_info",
            "bootstrap",
            "save_profile",
            "delete_profile",
            "connect",
            "disconnect",
            "set_passphrase",
            "prepare_execution",
            "authorize_plan",
            "execute_plan",
            "cancel_run",
            "catalog",
            "table_schema",
            "browse_table",
            "result_range",
            "release_result",
            "prepare_changes",
            "prepare_changes_batch",
            "processes",
            "prepare_process_cancel",
            "health",
            "history",
            "clear_history",
            "resource_usage",
        ]),
    ))
    .expect("Tauri build configuration must be valid");
}

#[cfg(not(feature = "desktop"))]
fn main() {}
