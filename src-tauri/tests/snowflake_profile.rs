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
        snowflake: Some(SnowflakeConfig {
            role: "ANALYST_READ_ONLY".into(),
            warehouse: "ANALYST_WH".into(),
            schema: "PUBLIC".into(),
        }),
    }
}

#[test]
fn studio_accepts_only_read_only_verified_snowflake_profiles() {
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
