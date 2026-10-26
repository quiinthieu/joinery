use joinery_core::{
    domain::*,
    execution::Studio,
    secrets::{MemorySecrets, OsSecrets, SecretStore},
};
use std::sync::Arc;

struct UnavailableStore;
impl SecretStore for UnavailableStore {
    fn get(&self, _: &str) -> AppResult<Option<String>> {
        Err(unavailable())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Err(unavailable())
    }
    fn delete(&self, _: &str) -> AppResult<()> {
        Err(unavailable())
    }
}
fn unavailable() -> AppError {
    AppError::new(
        ErrorCode::CredentialStore,
        "Unlock the credential store or use a session password",
    )
}

#[test]
fn deleting_saved_connections_needs_no_passphrase_and_survives_restart() {
    for (group, environment) in [
        ("local", "local"),
        ("staging", "staging"),
        ("production", "production"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let studio = Studio::with_secrets(root.path(), secrets.clone()).unwrap();
        let profile = studio
            .save_profile(
                serde_json::from_value(serde_json::json!({
                    "id":new_id(), "name":"Delete fixture", "groupId":group,
                    "engine":"postgres", "host":"127.0.0.1", "port":1,
                    "database":"fixture", "username":"fixture", "environment":environment,
                    "safeMode":"readOnly", "tlsMode":"verifyFull", "caCertPath":null,
                    "ssh":null, "rememberPassword":true, "storeQueryText":false,
                    "storeDrafts":true, "snowflake":null
                }))
                .unwrap(),
                None,
            )
            .unwrap();
        let key = format!("database:{}", profile.id);
        secrets.set(&key, "disposable-fixture-only").unwrap();
        studio.delete_profile(&profile.id).unwrap();
        assert!(secrets.get(&key).unwrap().is_none());
        assert!(studio.bootstrap().unwrap().profiles.is_empty());
        drop(studio);
        let reopened = Studio::with_secrets(root.path(), secrets).unwrap();
        assert!(reopened.bootstrap().unwrap().profiles.is_empty());
    }
}

#[tokio::test]
async fn unavailable_store_fails_before_dialing_and_preserves_saved_configuration() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(UnavailableStore)).unwrap();
    let profile = studio
        .save_profile(
            ConnectionProfile {
                id: new_id(),
                name: "Unavailable store".into(),
                group_id: "local".into(),
                engine: EngineKind::Postgres,
                host: "127.0.0.1".into(),
                port: 1,
                database: "fixture".into(),
                username: "fixture".into(),
                environment: Environment::Local,
                safe_mode: SafeMode::ReadOnly,
                tls_mode: TlsMode::Disable,
                ca_cert_path: None,
                ssh: None,
                remember_password: true,
                store_query_text: false,
                store_drafts: false,
                snowflake: None,
            },
            None,
        )
        .unwrap();
    let error = studio
        .connect(&profile.id, ConnectionSecrets::default())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::CredentialStore);
    assert_eq!(studio.usage().session_count, 0);
    assert_eq!(studio.usage().active_runs, 0);
    assert_eq!(
        studio.delete_profile(&profile.id).unwrap_err().code,
        ErrorCode::CredentialStore
    );
    let mut changed = profile.clone();
    changed.remember_password = false;
    assert_eq!(
        studio.save_profile(changed, None).unwrap_err().code,
        ErrorCode::CredentialStore
    );
    let profiles = studio.bootstrap().unwrap().profiles;
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, profile.id);
    assert!(profiles[0].remember_password);
}

// Explicit local qualification only. Never runs in CI or reads saved profiles.
#[test]
#[ignore = "writes one disposable synthetic credential to the native OS store"]
fn native_store_roundtrip_and_cleanup() {
    let store = OsSecrets::initialize();
    let name = format!("qualification-{}", uuid::Uuid::new_v4());
    let value = uuid::Uuid::new_v4().to_string();
    assert!(store.get(&name).unwrap().is_none());
    let written = store.set(&name, &value);
    // Delete before asserting the read result so read failures still clean up.
    let retrieved = store.get(&name);
    let deleted = store.delete(&name);
    assert!(
        deleted.is_ok(),
        "Could not remove disposable native credential"
    );
    assert!(
        written.is_ok(),
        "Could not write disposable native credential"
    );
    assert!(retrieved.is_ok_and(|found| found.as_deref() == Some(value.as_str())));
    assert!(store.get(&name).unwrap().is_none());
    store.delete(&name).unwrap();
}
