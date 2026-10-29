#[cfg(feature = "desktop")]
fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_info",
            "bootstrap",
            "save_workspace",
            "metadata_backups",
            "create_metadata_backup",
            "restore_metadata_backup",
            "delete_metadata_backup",
            "acknowledge_quit",
            "save_group",
            "delete_group",
            "audit_log",
            "save_profile",
            "delete_profile",
            "connect",
            "disconnect",
            "set_passphrase",
            "prepare_execution",
            "authorize_plan",
            "execute_plan",
            "cancel_run",
            "completion_catalog",
            "completion_schema",
            "catalog",
            "redis_scan",
            "redis_inspect",
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
            "probe_ssh",
            "trust_ssh_key",
            "native_smoke_fixture",
            "native_smoke_checkpoint",
            "native_smoke_report",
        ]),
    ))
    .expect("Tauri build configuration must be valid");
}

#[cfg(not(feature = "desktop"))]
fn main() {}
