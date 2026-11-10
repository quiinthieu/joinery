use joinery_core::{
    domain::*,
    drivers::{Driver, mysql::Mysql, postgres::Postgres},
};
use std::sync::Arc;

fn profile(engine: EngineKind, port: u16) -> ConnectionProfile {
    ConnectionProfile {
        id: new_id(),
        name: "Disposable session observation".into(),
        group_id: "local".into(),
        engine,
        host: "127.0.0.1".into(),
        port,
        database: "joinery".into(),
        username: "joinery".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::ReadOnly,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: None,
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL, MySQL, and MariaDB services"]
async fn unknown_mode_never_changes_real_driver_state_and_is_rejected_even_after_close() {
    for (engine, port) in [
        (EngineKind::Postgres, 55432),
        (EngineKind::Mysql, 53306),
        (EngineKind::Mysql, 53307),
    ] {
        let profile = profile(engine, port);
        let session_id = new_id();
        let driver: Arc<dyn Driver> = match engine {
            EngineKind::Postgres => {
                Postgres::open(&profile, Some("joinery-local-only"), &session_id, None)
                    .await
                    .unwrap()
                    .0
            }
            EngineKind::Mysql => {
                Mysql::open(&profile, Some("joinery-local-only"), &session_id, None)
                    .await
                    .unwrap()
                    .0
            }
            _ => unreachable!(),
        };
        let before = driver.session_state(&session_id).await.unwrap();
        assert_eq!(before.transaction_mode, TransactionMode::AutoCommit);
        assert!(before.transaction_observation.is_none());
        assert!(before.role.is_some());
        assert_eq!(
            driver
                .set_transaction_mode(TransactionMode::Unknown)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput,
            "{engine:?} {port}"
        );
        let after = driver.session_state(&session_id).await.unwrap();
        assert_eq!(
            before, after,
            "Unknown must not change observed session state"
        );
        driver.close();
        // Input validation must precede even a status query on a closed transport.
        assert_eq!(
            driver
                .set_transaction_mode(TransactionMode::Unknown)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput,
            "{engine:?} {port}"
        );
    }
}
