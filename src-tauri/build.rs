fn main() {
    generate_snowflake_sql_contract()
        .unwrap_or_else(|error| panic!("Invalid bundled Snowflake SQL contract: {error}"));
    #[cfg(feature = "desktop")]
    build_desktop();
}

fn generate_snowflake_sql_contract() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../snowflake-helper/src/sql-contract.json");
    println!("cargo:rerun-if-changed={}", path.display());
    let bytes = std::fs::read(&path)?;
    if bytes.len() > 8192 {
        return Err("contract exceeds 8192 bytes".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let keys = [
        "contractVersion",
        "bootstrapContext",
        "version",
        "begin",
        "commit",
        "rollback",
    ];
    let object = value.as_object().ok_or("expected a contract object")?;
    if object.len() != keys.len()
        || !keys.iter().all(|key| object.contains_key(*key))
        || value["contractVersion"].as_u64() != Some(1)
    {
        return Err("expected the closed version 1 contract schema".into());
    }
    let mut generated = String::from("// Generated from snowflake-helper/src/sql-contract.json.\n");
    for (key, name) in [
        ("bootstrapContext", "BOOTSTRAP_CONTEXT_SQL"),
        ("version", "VERSION_SQL"),
        ("begin", "BEGIN_SQL"),
        ("commit", "COMMIT_SQL"),
        ("rollback", "ROLLBACK_SQL"),
    ] {
        let sql = value[key].as_str().ok_or("SQL values must be strings")?;
        if sql.is_empty()
            || sql.len() > 1024
            || !sql.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
            || sql.trim() != sql
            || sql.contains(';')
        {
            return Err(format!("invalid SQL value for {key}").into());
        }
        match name {
            "BOOTSTRAP_CONTEXT_SQL" => {
                generated.push_str("#[cfg(any(test, all(unix, feature = \"test-harness\")))]\n")
            }
            "VERSION_SQL" => generated.push_str("#[cfg(any(unix, test))]\n"),
            _ => {}
        }
        generated.push_str(&format!("pub(crate) const {name}: &str = {sql:?};\n"));
    }
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    std::fs::write(output.join("snowflake_sql_contract.rs"), generated)?;
    Ok(())
}

#[cfg(feature = "desktop")]
fn build_desktop() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_info",
            "bootstrap",
            "save_workspace",
            "recovery_backups",
            "delete_recovery_backup",
            "start_metadata_recovery",
            "metadata_recovery_operation",
            "cancel_metadata_recovery",
            "reconcile_metadata_recovery",
            "reconcile_unknown_metadata_recovery",
            "metadata_recovery_admission_closed",
            "acknowledge_quit",
            "save_group",
            "delete_group",
            "audit_log",
            "save_profile",
            "delete_profile",
            "prepare_connection",
            "connect_prepared",
            "cancel_connection_attempt",
            "app_shutdown",
            "connect",
            "disconnect",
            "set_passphrase",
            "extract_query_params",
            "prepare_execution",
            "prepare_explain",
            "authorize_plan",
            "prepare_snowflake_console",
            "start_snowflake_console",
            "snowflake_console_run",
            "snowflake_console_request",
            "snowflake_console_receipts",
            "snowflake_console_receipt_summary",
            "set_snowflake_console_receipt_archive",
            "snowflake_console_pending",
            "snowflake_console_storage",
            "release_snowflake_console_results",
            "execute_plan",
            "cancel_run",
            "completion_catalog",
            "completion_schema",
            "inspection_schema",
            "relation_catalog",
            "inspect_relation",
            "define_relation",
            "inspect_relation_constraints",
            "inspect_relation_constraint_mappings",
            "inspect_relation_check_constraints",
            "preview_relation",
            "catalog",
            "database_objects",
            "object_details",
            "redis_scan",
            "redis_inspect",
            "table_schema",
            "referencing_foreign_keys",
            "schema_diagram",
            "schema_snapshot",
            "compare_schemas",
            "generate_schema_ddl",
            "generate_migration_ddl",
            "preview_index_constraint_ddl",
            "prepare_index_constraint_ddl",
            "preview_table_object_ddl",
            "prepare_table_object_ddl",
            "preview_table_maintenance",
            "prepare_table_maintenance",
            "prepare_migration_ddl",
            "prepare_browse_table",
            "browse_table",
            "result_range",
            "release_result",
            "prepare_changes",
            "prepare_changes_batch",
            "prepare_table_changes",
            "processes",
            "lock_waits",
            "prepare_process_cancel",
            "health",
            "history",
            "history_page",
            "history_entry",
            "clear_history",
            "resource_usage",
            "probe_ssh",
            "trust_ssh_key",
            "pick_export_destination",
            "start_export",
            "cancel_export",
            "pick_import_source",
            "preview_csv_import",
            "release_csv_import_preview",
            "prepare_csv_import_batch",
            "compare_data",
            "session_contexts",
            "switch_session_context",
            "switch_session_schema",
            "snowflake_session_contexts",
            "switch_snowflake_context",
            "prepare_transaction_control",
            "execute_transaction_control",
            "execute_snowflake_dml",
            "prepare_snowflake_grid_plan",
            "execute_snowflake_grid_plan",
            "cancel_snowflake_grid_plan",
            "preview_snowflake_grid_transaction_relation",
            "finish_snowflake_grid_transaction",
            "preflight_snowflake_ddl",
            "prepare_snowflake_ddl",
            "authorize_snowflake_ddl",
            "execute_snowflake_ddl",
            "refresh_snowflake_ddl_metadata",
            "set_transaction_mode",
            "commit_transaction",
            "rollback_transaction",
            "open_sql_file",
            "save_sql_file",
            "release_sql_file_grant",
            "save_saved_query",
            "delete_saved_query",
            "save_bookmark",
            "delete_bookmark",
            "save_sql_format_settings",
            "set_temporary_result_storage",
            "save_sql_snippet",
            "delete_sql_snippet",
            "save_virtual_foreign_key",
            "delete_virtual_foreign_key",
            "run_workspace_sql",
            "search_database_data",
            "snowflake_query_history",
            "snowflake_query_operator_profile",
            "local_history_entries",
            "record_local_history",
            "delete_local_history_entry",
            "clear_local_history",
            "find_sql_usages",
            "query_result_view",
            "native_smoke_fixture",
            "native_smoke_checkpoint",
            "native_smoke_report",
        ]),
    ))
    .expect("Tauri build configuration must be valid");
}
