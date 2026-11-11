use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use std::sync::Arc;

fn profile() -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Read-only warehouse".into(),
        group_id: "production".into(),
        engine: EngineKind::Snowflake,
        host: "org-account.snowflakecomputing.com".into(),
        port: 443,
        database: "ANALYTICS".into(),
        username: "ANALYST".into(),
        environment: Environment::Production,
        safe_mode: SafeMode::ReadOnly,
        tls_mode: TlsMode::VerifyFull,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: Some(SnowflakeConfig {
            authentication: SnowflakeAuthentication::LocalApplicationOauth,
            reviewed_dml: false,
            role: "ANALYST_READ_ONLY".into(),
            warehouse: "ANALYST_WH".into(),
            schema: "PUBLIC".into(),
        }),
    }
}

#[test]
fn studio_accepts_only_read_only_verified_snowflake_oauth_profiles() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    assert!(
        studio
            .bootstrap()
            .unwrap()
            .supported_engines
            .contains(&EngineKind::Snowflake)
    );
    assert!(studio.save_profile(profile(), None).is_ok());

    let mut unsafe_profile = profile();
    unsafe_profile.safe_mode = SafeMode::Silent;
    assert_eq!(
        studio.save_profile(unsafe_profile, None).unwrap_err().code,
        ErrorCode::InvalidInput
    );

    let mut unsafe_profile = profile();
    unsafe_profile.host = "org-account.snowflakecomputing.com.attacker.test".into();
    assert_eq!(
        studio.save_profile(unsafe_profile, None).unwrap_err().code,
        ErrorCode::InvalidInput
    );

    let mut unsafe_profile = profile();
    unsafe_profile.ssh = Some(SshConfig {
        host: "bastion.example.com".into(),
        port: 22,
        username: "analyst".into(),
        authentication: SshAuthentication::Agent,
        private_key_path: None,
        agent_socket_path: None,
    });
    assert_eq!(
        studio.save_profile(unsafe_profile, None).unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

#[test]
fn production_connection_retry_does_not_require_a_passphrase_for_unchanged_settings() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let saved = studio.save_profile(profile(), None).unwrap();
    assert!(studio.save_profile(saved.clone(), None).is_ok());
    let mut changed = saved.clone();
    changed.database = "OTHER_DATABASE".into();
    assert_eq!(
        studio.save_profile(changed.clone(), None).unwrap_err().code,
        ErrorCode::PassphraseRequired
    );
    assert_eq!(
        studio.bootstrap().unwrap().profiles[0].database,
        saved.database
    );
    studio
        .set_passphrase("fixture-write-passphrase", None)
        .unwrap();
    assert!(studio.save_profile(saved, None).is_ok());
    assert!(
        studio
            .save_profile(changed, Some("fixture-write-passphrase"))
            .is_ok()
    );
}

#[tokio::test]
async fn studio_exports_retained_results_reports_truncation_and_cancels_cleanly() {
    use joinery_core::file_capability::SandboxFileDialog;

    let root = tempfile::tempdir().unwrap();
    let sandbox = SandboxFileDialog::new(&root.path().join("exports")).unwrap();
    let studio = Studio::with_secrets_and_dialog(
        root.path(),
        Arc::new(MemorySecrets::default()),
        sandbox.clone(),
    )
    .unwrap();

    let columns = vec![
        ColumnInfo {
            id: "c0".into(),
            name: "ORDER_ID".into(),
            database_type: "NUMBER(38,0)".into(),
            kind: ColumnKind::Integer,
            nullable: false,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        },
        ColumnInfo {
            id: "c1".into(),
            name: "AMOUNT".into(),
            database_type: "NUMBER(38,10)".into(),
            kind: ColumnKind::Decimal,
            nullable: false,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        },
        ColumnInfo {
            id: "c2".into(),
            name: "NOTE".into(),
            database_type: "VARCHAR".into(),
            kind: ColumnKind::Text,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        },
    ];
    let created = studio.results.create("sf-run", 0, columns, None).unwrap();
    for batch in 0..4 {
        studio
            .results
            .append(
                &created.id,
                (0..500)
                    .map(|idx| {
                        let n = batch * 500 + idx + 1;
                        vec![
                            Some(if n == 1 {
                                "9223372036854775807".into()
                            } else {
                                n.to_string()
                            }),
                            Some("12345678901234567890.1234567890".into()),
                            if n % 2 == 0 { None } else { Some("".into()) },
                        ]
                    })
                    .collect(),
            )
            .unwrap();
    }
    studio.results.finish(&created.id, true).unwrap();

    // 1. Export projected columns in custom order as NDJSON and surface source_truncated
    let grant = studio
        .pick_export_destination("warehouse.ndjson", ExportFormat::Ndjson)
        .await
        .unwrap()
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    studio
        .start_export(
            ExportRequest {
                destination_token: grant.token,
                format: ExportFormat::Ndjson,
                source: ExportSource::Result {
                    result_id: created.id.clone(),
                    column_ids: vec!["c2".into(), "c0".into(), "c1".into()],
                },
                include_headers: true,
                engine: Some(EngineKind::Snowflake),
                target_table: None,
            },
            Arc::new(move |e| tx.send(e).is_ok()),
        )
        .await
        .unwrap();
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        let done = matches!(ev, ExportEvent::Finished { .. });
        events.push(ev);
        if done {
            break;
        }
    }
    assert!(matches!(
        events.first(),
        Some(ExportEvent::Started {
            source_truncated: true,
            ..
        })
    ));
    assert!(matches!(
        events.last(),
        Some(ExportEvent::Finished {
            status: RunStatus::Completed,
            rows_written: 2000,
            ..
        })
    ));
    let ndjson = std::fs::read_to_string(sandbox.root().join("warehouse.ndjson")).unwrap();
    let lines: Vec<&str> = ndjson.lines().collect();
    assert_eq!(lines.len(), 2000);
    assert_eq!(
        lines[0],
        "{\"NOTE\":\"\",\"ORDER_ID\":9223372036854775807,\"AMOUNT\":12345678901234567890.1234567890}"
    );
    assert_eq!(
        lines[1],
        "{\"NOTE\":null,\"ORDER_ID\":2,\"AMOUNT\":12345678901234567890.1234567890}"
    );

    // 2. Explicit cancel_export leaves only marked .partial file
    let cancel_grant = studio
        .pick_export_destination("warehouse-cancel.csv", ExportFormat::Csv)
        .await
        .unwrap()
        .unwrap();
    let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::unbounded_channel();
    let studio_for_cancel = studio.clone();
    studio
        .start_export(
            ExportRequest {
                destination_token: cancel_grant.token,
                format: ExportFormat::Csv,
                source: ExportSource::Result {
                    result_id: created.id.clone(),
                    column_ids: Vec::new(),
                },
                include_headers: true,
                engine: Some(EngineKind::Snowflake),
                target_table: None,
            },
            Arc::new(move |e| {
                if let ExportEvent::Progress { export_id, .. } = &e {
                    let _ = studio_for_cancel.cancel_export(export_id);
                }
                cancel_tx.send(e).is_ok()
            }),
        )
        .await
        .unwrap();
    let mut cancel_events = Vec::new();
    while let Some(ev) = cancel_rx.recv().await {
        let done = matches!(ev, ExportEvent::Finished { .. });
        cancel_events.push(ev);
        if done {
            break;
        }
    }
    assert!(matches!(
        cancel_events.last(),
        Some(ExportEvent::Finished {
            status: RunStatus::Cancelled,
            output_file_name: None,
            partial_file_name: Some(name),
            ..
        }) if name == "warehouse-cancel.csv.partial"
    ));
    assert!(!sandbox.root().join("warehouse-cancel.csv").exists());
    let partial =
        std::fs::read_to_string(sandbox.root().join("warehouse-cancel.csv.partial")).unwrap();
    assert!(partial.contains("# JOINERY EXPORT INCOMPLETE"));
    studio.shutdown();
}

#[test]
fn snowflake_authentication_defaults_legacy_and_rejects_unknown_modes() {
    let mut encoded = serde_json::to_value(profile()).unwrap();
    encoded["snowflake"]
        .as_object_mut()
        .unwrap()
        .remove("authentication");
    let legacy: ConnectionProfile = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(
        legacy.snowflake.unwrap().authentication,
        SnowflakeAuthentication::LocalApplicationOauth
    );
    encoded["snowflake"]["authentication"] = serde_json::json!("externalBrowser");
    let sdk: ConnectionProfile = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(
        sdk.snowflake.unwrap().authentication,
        SnowflakeAuthentication::ExternalBrowser
    );
    for unknown in [
        serde_json::json!("externalbrowser"),
        serde_json::Value::Null,
        serde_json::json!({"executable":"/tmp/node"}),
    ] {
        encoded["snowflake"]["authentication"] = unknown;
        assert!(serde_json::from_value::<ConnectionProfile>(encoded.clone()).is_err());
    }
}

#[test]
fn authentication_survives_restart_and_metadata_restore() {
    let root = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let studio = Studio::with_secrets(root.path(), secrets.clone()).unwrap();
    let mut sdk = profile();
    sdk.group_id = "local".into();
    sdk.environment = Environment::Local;
    sdk.snowflake.as_mut().unwrap().authentication = SnowflakeAuthentication::ExternalBrowser;
    let saved = studio.save_profile(sdk, None).unwrap();
    let backup = studio.create_metadata_backup().unwrap();
    drop(studio);
    let studio = Studio::with_secrets(root.path(), secrets).unwrap();
    assert_eq!(
        studio.bootstrap().unwrap().profiles[0]
            .snowflake
            .as_ref()
            .unwrap()
            .authentication,
        SnowflakeAuthentication::ExternalBrowser
    );
    let mut legacy = saved;
    legacy.snowflake.as_mut().unwrap().authentication =
        SnowflakeAuthentication::LocalApplicationOauth;
    studio.save_profile(legacy, None).unwrap();
    studio
        .restore_metadata_backup(&backup.id, &backup.digest, "RESTORE", None)
        .unwrap();
    assert_eq!(
        studio.bootstrap().unwrap().profiles[0]
            .snowflake
            .as_ref()
            .unwrap()
            .authentication,
        SnowflakeAuthentication::ExternalBrowser
    );
}

#[test]
fn explicit_boundary_policies_survive_restart_and_backup_without_elevating_read_only() {
    for mode in [
        SafeMode::ReadOnly,
        SafeMode::Confirm,
        SafeMode::Silent,
        SafeMode::Guard,
        SafeMode::PasswordGated,
    ] {
        let root = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let studio = Studio::with_secrets(root.path(), secrets.clone()).unwrap();
        let mut sdk = profile();
        sdk.group_id = "local".into();
        sdk.environment = Environment::Local;
        sdk.safe_mode = mode;
        sdk.snowflake.as_mut().unwrap().authentication = SnowflakeAuthentication::ExternalBrowser;
        let saved = studio.save_profile(sdk, None).unwrap();
        let backup = studio.create_metadata_backup().unwrap();
        drop(studio);
        let studio = Studio::with_secrets(root.path(), secrets).unwrap();
        assert_eq!(studio.bootstrap().unwrap().profiles[0].safe_mode, mode);
        let mut read_only = saved;
        read_only.safe_mode = SafeMode::ReadOnly;
        studio.save_profile(read_only.clone(), None).unwrap();
        studio
            .restore_metadata_backup(&backup.id, &backup.digest, "RESTORE", None)
            .unwrap();
        let restored = studio.bootstrap().unwrap().profiles.remove(0);
        assert_eq!(restored.safe_mode, mode);
        assert_eq!(
            restored.snowflake.unwrap().authentication,
            SnowflakeAuthentication::ExternalBrowser
        );
        if mode != SafeMode::ReadOnly {
            let mut production = read_only;
            production.id = new_id();
            production.group_id = "production".into();
            production.safe_mode = mode;
            let saved = studio.save_profile(production, None).unwrap();
            assert_eq!(saved.environment, Environment::Production);
            assert_eq!(saved.safe_mode, SafeMode::PasswordGated);
        }
    }
}

#[tokio::test]
async fn unavailable_sdk_rejects_explicit_mode_without_oauth_fallback() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let mut sdk = profile();
    sdk.snowflake.as_mut().unwrap().authentication = SnowflakeAuthentication::ExternalBrowser;
    let sdk = studio.save_profile(sdk, None).unwrap();
    let error = studio
        .connect(&sdk.id, ConnectionSecrets::default())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert!(error.message.contains("runtime is unavailable"));
    // The failed mode does not close global admission or alter its saved mode.
    let reservation = studio.prepare_connection(&sdk.id).unwrap();
    studio.cancel_connection_attempt(&reservation);
    assert_eq!(
        studio.bootstrap().unwrap().profiles[0]
            .snowflake
            .as_ref()
            .unwrap()
            .authentication,
        SnowflakeAuthentication::ExternalBrowser
    );
    let error = joinery_core::drivers::snowflake::Snowflake::open(&sdk, "test-session")
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}
