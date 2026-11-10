//! Actual Node + real helper + synthetic SDK, through Studio's production
//! routing and cleanup paths. No Snowflake login, server or remote writes.
use super::*;
use crate::{secrets::MemorySecrets, snowflake_runtime::studio_fixture::Fixture};

fn profile() -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Synthetic SSO".into(),
        group_id: "local".into(),
        engine: EngineKind::Snowflake,
        host: "org-account.snowflakecomputing.com".into(),
        port: 443,
        database: "DB".into(),
        username: "login@example.test".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::ReadOnly,
        tls_mode: TlsMode::VerifyFull,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: Some(SnowflakeConfig {
            authentication: SnowflakeAuthentication::ExternalBrowser,
            role: "ROLE".into(),
            warehouse: "WH".into(),
            schema: "PUBLIC".into(),
        }),
    }
}

fn studio(fixture: &Fixture) -> Arc<Studio> {
    Studio::with_snowflake_runtime(
        &fixture.directory.path().join("workspace"),
        Arc::new(MemorySecrets::default()),
        Arc::new(DisabledFileDialog),
        Some(fixture.runtime.clone()),
    )
    .unwrap()
}

async fn started(fixture: &Fixture) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(bytes) = std::fs::read(fixture.controls.join("auth-started"))
                && let Ok(value) = serde_json::from_slice(&bytes)
            {
                return value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("helper must enter synthetic authentication")
}

fn no_roots(fixture: &Fixture) {
    assert_eq!(std::fs::read_dir(&fixture.state).unwrap().count(), 0);
}

#[tokio::test]
async fn sdk_session_is_published_and_disconnect_waits_with_retained_driver() {
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let info = studio
        .connect(&profile.id, ConnectionSecrets::default())
        .await
        .unwrap();
    assert!(!info.capabilities.table_browsing);
    assert!(!info.capabilities.transactions);
    let retained = studio.session(&info.id).unwrap();
    let marker = started(&fixture).await;
    assert!(Path::new(marker["root"].as_str().unwrap()).exists());
    studio.disconnect_and_wait(&info.id).await.unwrap();
    assert!(retained.driver.is_closed());
    assert!(studio.sessions.lock().is_empty());
    no_roots(&fixture);
    assert_eq!(
        std::fs::read_to_string(fixture.controls.join("queries"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[tokio::test]
async fn cancellation_during_auth_waits_for_reaping_and_exact_root_cleanup() {
    let fixture = Fixture::new();
    std::fs::write(fixture.controls.join("stall-auth"), b"").unwrap();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    let task = tokio::spawn({
        let studio = studio.clone();
        let attempt = attempt.clone();
        async move {
            studio
                .connect_prepared(&attempt, ConnectionSecrets::default())
                .await
        }
    });
    started(&fixture).await;
    studio
        .cancel_connection_attempt_and_wait(&attempt)
        .await
        .unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert!(studio.sessions.lock().is_empty());
    no_roots(&fixture);
    assert!(!fixture.controls.join("queries").exists());
}

#[tokio::test]
async fn abandoning_auth_future_retains_cleanup_until_shutdown_receipt() {
    let fixture = Fixture::new();
    std::fs::write(fixture.controls.join("stall-auth"), b"").unwrap();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let task = tokio::spawn({
        let studio = studio.clone();
        async move {
            studio
                .connect(&profile.id, ConnectionSecrets::default())
                .await
        }
    });
    started(&fixture).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    studio.shutdown_and_wait().await.unwrap();
    no_roots(&fixture);
    assert!(studio.sessions.lock().is_empty());
}

#[tokio::test]
async fn shutdown_during_auth_cannot_publish_and_prevents_another_launch() {
    let fixture = Fixture::new();
    std::fs::write(fixture.controls.join("stall-auth"), b"").unwrap();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let task = tokio::spawn({
        let studio = studio.clone();
        let id = profile.id.clone();
        async move { studio.connect(&id, ConnectionSecrets::default()).await }
    });
    started(&fixture).await;
    studio.shutdown_and_wait().await.unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(
        studio.prepare_connection(&profile.id).unwrap_err().code,
        ErrorCode::Cancelled
    );
    no_roots(&fixture);
}
