use super::*;
use crate::{
    domain::{ErrorCode, SshAuthentication, SshConfig},
    secrets::{MemorySecrets, SecretStore},
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;

fn input() -> Input {
    Input {
        account: "synthetic-account.eu-west-1.privatelink".into(),
        host: "synthetic-account.eu-west-1.privatelink.snowflakecomputing.com".into(),
        username: "synthetic-login@example.test".into(),
        role: "SYNTHETIC_QUALIFICATION".into(),
        warehouse: "SYNTHETIC_WH".into(),
        database: DATABASE.into(),
        schema: SCHEMA.into(),
    }
}

// Fails on every attempted secret operation, including reads. No OS keychain
// is initialized or consulted by any qualification profile persistence test.
#[derive(Default)]
struct DenySecrets(Mutex<Vec<&'static str>>);

impl DenySecrets {
    fn deny<T>(&self, operation: &'static str) -> AppResult<T> {
        self.0.lock().push(operation);
        Err(AppError::new(
            ErrorCode::CredentialStore,
            "Synthetic secret access is forbidden",
        ))
    }
}

impl SecretStore for DenySecrets {
    fn get(&self, _: &str) -> AppResult<Option<String>> {
        self.deny("get")
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        self.deny("set")
    }
    fn delete(&self, _: &str) -> AppResult<()> {
        self.deny("delete")
    }
}

#[test]
fn qualification_profile_declaration_is_fixed_and_contains_no_private_identity() {
    let expected = json!({
        "groupId": "staging",
        "environment": "staging",
        "safeMode": "confirm",
        "authentication": "externalBrowser",
        "reviewedDml": true,
        "reviewedDdl": true,
        "tlsMode": "verifyFull",
        "port": 443,
        "ssh": false,
        "caCertificate": false,
        "storedSecrets": false,
        "retainQueryText": false,
        "persistDrafts": false,
        "database": "SANDBOX",
        "schema": "BILLIE_TESTING"
    });
    assert_eq!(serde_json::to_value(declaration()).unwrap(), expected);
    assert_eq!(
        serde_json::to_vec(&declaration()).unwrap(),
        serde_json::to_vec(&declaration()).unwrap()
    );
    let encoded = serde_json::to_string(&declaration()).unwrap();
    for private in [input().account, input().host, input().username] {
        assert!(!encoded.contains(&private));
    }
}

#[test]
fn qualification_profile_private_input_has_exactly_seven_required_string_fields() {
    let value = serde_json::to_value(input()).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 7);
    for field in [
        "account",
        "host",
        "username",
        "role",
        "warehouse",
        "database",
        "schema",
    ] {
        let mut absent = value.clone();
        absent.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<Input>(absent).is_err());
        let mut wrong_type = value.clone();
        wrong_type[field] = json!(false);
        assert!(serde_json::from_value::<Input>(wrong_type).is_err());
    }
    for field in [
        "mode",
        "environment",
        "safeMode",
        "tlsMode",
        "port",
        "groupId",
        "ssh",
        "caCertPath",
        "reviewedDml",
        "reviewedDdl",
        "password",
        "passphrase",
        "rememberPassword",
        "storeQueryText",
        "storeDrafts",
        "profileId",
    ] {
        let mut extra = value.clone();
        extra[field] = json!("caller-policy");
        assert!(serde_json::from_value::<Input>(extra).is_err());
    }
}

#[test]
fn qualification_profile_input_matches_real_external_account_derivation_and_exact_target() {
    let valid = input();
    valid.validate().unwrap();
    let mut uppercase_host = input();
    uppercase_host.host = uppercase_host.host.to_ascii_uppercase();
    uppercase_host.validate().unwrap();
    for (field, replacement) in [
        ("account", "different-account"),
        ("account", "SYNTHETIC-ACCOUNT.EU-WEST-1.PRIVATELINK"),
        ("host", "different-account.snowflakecomputing.com"),
        (
            "host",
            "synthetic-account.snowflakecomputing.com.attacker.test",
        ),
        ("host", "https://synthetic-account.snowflakecomputing.com"),
        ("host", "synthetic-account.snowflakecomputing.com:443"),
        ("host", "-synthetic-account.snowflakecomputing.com"),
        ("database", "PRODUCTION"),
        ("database", "sandbox"),
        ("schema", "PUBLIC"),
        ("schema", "billie_testing"),
        ("role", "quoted role"),
        ("warehouse", "lowercase_wh"),
    ] {
        let mut changed = serde_json::to_value(&valid).unwrap();
        changed[field] = json!(replacement);
        let changed: Input = serde_json::from_value(changed).unwrap();
        let error = changed.validate().unwrap_err();
        assert!(!error.message.contains(replacement));
    }
}

#[test]
fn qualification_profile_input_rejects_blank_controls_padding_and_oversized_fields() {
    let baseline = serde_json::to_value(input()).unwrap();
    for (field, limit) in [
        ("account", 253),
        ("host", 253),
        ("username", 1020),
        ("role", 255),
        ("warehouse", 255),
        ("database", 255),
        ("schema", 255),
    ] {
        let original = baseline[field].as_str().unwrap();
        for replacement in [
            String::new(),
            " ".into(),
            format!(" {original}"),
            format!("{original} "),
            format!("{original}\n"),
            format!("{original}\0"),
            "X".repeat(limit + 1),
        ] {
            let mut changed = baseline.clone();
            changed[field] = Value::String(replacement);
            assert!(
                serde_json::from_value::<Input>(changed)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }
    let mut at_limit = input();
    at_limit.username = "U".repeat(1020);
    at_limit.role = "R".repeat(255);
    at_limit.warehouse = "W".repeat(255);
    at_limit.validate().unwrap();
}

#[test]
fn qualification_profile_save_uses_ordinary_persistence_with_no_secret_or_passphrase_state() {
    let root = tempfile::tempdir().unwrap();
    let secrets = Arc::new(DenySecrets::default());
    let studio = Studio::with_secrets(root.path(), secrets.clone()).unwrap();
    let saved = input().save_new(&studio).unwrap();
    assert_eq!(saved.group_id, "staging");
    assert_eq!(saved.environment, Environment::Staging);
    assert_eq!(saved.safe_mode, SafeMode::Confirm);
    assert_eq!(safety::effective_mode(&saved), SafeMode::Confirm);
    assert_eq!(
        uuid::Uuid::parse_str(&saved.id).unwrap().get_version_num(),
        4
    );
    validate_saved(&input(), &saved).unwrap();
    assert!(secrets.0.lock().is_empty());
    drop(studio);

    let reopened = Studio::with_secrets(root.path(), secrets.clone()).unwrap();
    let bootstrap = reopened.bootstrap().unwrap();
    let [persisted] = bootstrap.profiles.as_slice() else {
        panic!("sole fresh profile required")
    };
    assert_eq!(
        serde_json::to_value(persisted).unwrap(),
        serde_json::to_value(&saved).unwrap()
    );
    validate_saved(&input(), persisted).unwrap();
    assert!(!bootstrap.passphrase_configured);
    assert!(!persisted.remember_password);
    assert!(!persisted.store_query_text);
    assert!(!persisted.store_drafts);
    assert!(input().save_new(&reopened).is_err());
    assert_eq!(
        serde_json::to_value(reopened.bootstrap().unwrap().profiles).unwrap(),
        json!([saved])
    );
    assert!(secrets.0.lock().is_empty());
}

#[test]
fn qualification_profile_rejects_existing_profile_instead_of_adopting_or_downgrading_it() {
    let root = tempfile::tempdir().unwrap();
    let secrets = Arc::new(DenySecrets::default());
    let studio = Studio::with_secrets(root.path(), secrets.clone()).unwrap();
    let mut existing = input().profile(uuid::Uuid::new_v4().to_string());
    existing.group_id = "production".into();
    existing.environment = Environment::Production;
    let existing = studio.save_profile(existing, None).unwrap();
    assert_eq!(existing.safe_mode, SafeMode::PasswordGated);
    assert!(input().save_new(&studio).is_err());
    assert_eq!(
        serde_json::to_value(studio.bootstrap().unwrap().profiles).unwrap(),
        json!([existing])
    );
    assert!(secrets.0.lock().is_empty());
}

#[test]
fn qualification_profile_save_rejects_preexisting_passphrase_or_temporary_storage() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    studio
        .set_passphrase("synthetic-local-passphrase", None)
        .unwrap();
    assert!(input().save_new(&studio).is_err());
    assert!(studio.bootstrap().unwrap().profiles.is_empty());

    let other_root = tempfile::tempdir().unwrap();
    let secrets = Arc::new(DenySecrets::default());
    let other = Studio::with_secrets(other_root.path(), secrets.clone()).unwrap();
    other.set_temporary_result_storage(true).unwrap();
    assert!(input().save_new(&other).is_err());
    assert!(other.bootstrap().unwrap().profiles.is_empty());
    assert!(secrets.0.lock().is_empty());
}

#[test]
fn qualification_profile_verification_rejects_policy_identity_and_privacy_drift() {
    let input = input();
    let expected = input.profile(uuid::Uuid::new_v4().to_string());
    validate_saved(&input, &expected).unwrap();
    let mutations: &[fn(&mut ConnectionProfile)] = &[
        |p| p.id = "invalid-uuid".into(),
        |p| p.id = uuid::Uuid::nil().to_string(),
        |p| p.name = "Another connection".into(),
        |p| p.group_id = "production".into(),
        |p| p.group_id = "local".into(),
        |p| p.environment = Environment::Production,
        |p| p.environment = Environment::Local,
        |p| p.safe_mode = SafeMode::Silent,
        |p| p.safe_mode = SafeMode::PasswordGated,
        |p| p.safe_mode = SafeMode::ReadOnly,
        |p| p.engine = EngineKind::Postgres,
        |p| p.port = 80,
        |p| p.tls_mode = TlsMode::Disable,
        |p| p.ca_cert_path = Some("synthetic-ca.pem".into()),
        |p| {
            p.ssh = Some(SshConfig {
                host: "synthetic-ssh.test".into(),
                port: 22,
                username: "synthetic".into(),
                authentication: SshAuthentication::Agent,
                private_key_path: None,
                agent_socket_path: None,
            })
        },
        |p| p.remember_password = true,
        |p| p.store_query_text = true,
        |p| p.store_drafts = true,
        |p| p.database = "OTHER_DATABASE".into(),
        |p| p.host = "another-account.snowflakecomputing.com".into(),
        |p| p.username = "another-user@example.test".into(),
        |p| p.snowflake = None,
        |p| p.snowflake.as_mut().unwrap().reviewed_dml = false,
        |p| p.snowflake.as_mut().unwrap().reviewed_ddl = false,
        |p| {
            p.snowflake.as_mut().unwrap().authentication =
                SnowflakeAuthentication::LocalApplicationOauth
        },
        |p| p.snowflake.as_mut().unwrap().role = "OTHER_ROLE".into(),
        |p| p.snowflake.as_mut().unwrap().warehouse = "OTHER_WH".into(),
        |p| p.snowflake.as_mut().unwrap().schema = "OTHER_SCHEMA".into(),
    ];
    for mutate in mutations {
        let mut changed = expected.clone();
        mutate(&mut changed);
        assert!(validate_saved(&input, &changed).is_err());
    }
}

#[test]
fn qualification_profile_does_not_weaken_ordinary_production_password_gates() {
    let root = tempfile::tempdir().unwrap();
    let secrets = Arc::new(DenySecrets::default());
    let studio = Studio::with_secrets(root.path(), secrets.clone()).unwrap();
    let mut requested = input().profile(uuid::Uuid::new_v4().to_string());
    // The ordinary group remains authoritative even if the supplied DTO says
    // Staging, and the ordinary production gate still normalizes Confirm.
    requested.group_id = "production".into();
    assert_eq!(requested.environment, Environment::Staging);
    assert_eq!(requested.safe_mode, SafeMode::Confirm);
    let protected = studio.save_profile(requested, None).unwrap();
    assert_eq!(protected.environment, Environment::Production);
    assert_eq!(protected.safe_mode, SafeMode::PasswordGated);
    assert_eq!(safety::effective_mode(&protected), SafeMode::PasswordGated);
    assert!(validate_saved(&input(), &protected).is_err());
    let mut changed = protected.clone();
    changed.name = "Changed synthetic production connection".into();
    assert_eq!(
        studio.save_profile(changed, None).unwrap_err().code,
        ErrorCode::PassphraseRequired
    );
    assert_eq!(
        serde_json::to_value(studio.bootstrap().unwrap().profiles).unwrap(),
        json!([protected])
    );
    assert!(secrets.0.lock().is_empty());
}
