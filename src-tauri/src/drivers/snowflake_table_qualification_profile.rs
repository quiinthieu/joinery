//! Fixed application policy for the qualification's disposable workspace.
//! Private input selects only the connection identity; it cannot select policy.

use super::{DATABASE, SCHEMA};
use crate::{
    domain::{
        AppError, AppResult, Bootstrap, ConnectionProfile, EngineKind, Environment, SafeMode,
        SnowflakeAuthentication, SnowflakeConfig, TlsMode,
    },
    drivers::snowflake::Snowflake,
    execution::Studio,
    safety,
};
use serde::{Deserialize, Serialize};

const GROUP: &str = "staging";
const NAME: &str = "Isolated Snowflake table qualification";

fn invalid() -> AppError {
    AppError::invalid("The isolated qualification connection or fixed policy is invalid")
}

// No Debug: these private connection identifiers must not enter diagnostics.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) account: String,
    pub(super) host: String,
    pub(super) username: String,
    pub(super) role: String,
    pub(super) warehouse: String,
    pub(super) database: String,
    pub(super) schema: String,
}

/// Serialize only. Restoration must recompute this declaration, never adopt a
/// caller's environment, mode or transport settings from an envelope.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Declaration {
    group_id: &'static str,
    environment: Environment,
    safe_mode: SafeMode,
    authentication: SnowflakeAuthentication,
    reviewed_dml: bool,
    reviewed_ddl: bool,
    reviewed_console: bool,
    tls_mode: TlsMode,
    port: u16,
    ssh: bool,
    ca_certificate: bool,
    stored_secrets: bool,
    retain_query_text: bool,
    persist_drafts: bool,
    database: &'static str,
    schema: &'static str,
}

pub(super) fn declaration() -> Declaration {
    Declaration {
        group_id: GROUP,
        environment: Environment::Staging,
        safe_mode: SafeMode::Confirm,
        authentication: SnowflakeAuthentication::ExternalBrowser,
        reviewed_dml: true,
        reviewed_ddl: true,
        reviewed_console: false,
        tls_mode: TlsMode::VerifyFull,
        port: 443,
        ssh: false,
        ca_certificate: false,
        stored_secrets: false,
        retain_query_text: false,
        persist_drafts: false,
        database: DATABASE,
        schema: SCHEMA,
    }
}

impl Input {
    pub(super) fn validate(&self) -> AppResult<()> {
        for (value, limit) in [
            (self.account.as_str(), 253),
            (self.host.as_str(), 253),
            (self.username.as_str(), 1020),
            (self.role.as_str(), 255),
            (self.warehouse.as_str(), 255),
            (self.database.as_str(), 255),
            (self.schema.as_str(), 255),
        ] {
            if value.is_empty()
                || value.len() > limit
                || value.trim() != value
                || value.chars().any(char::is_control)
            {
                return Err(invalid());
            }
        }
        // Match the external-browser driver's verified host derivation exactly,
        // preserving regional and PrivateLink account components. Account is
        // already canonical; inconsistent input must never be silently repaired.
        let host = self.host.trim().to_ascii_lowercase();
        let account = host
            .strip_suffix(".snowflakecomputing.com")
            .ok_or_else(invalid)?;
        if self.account != account || self.database != DATABASE || self.schema != SCHEMA {
            return Err(invalid());
        }
        Snowflake::validate_profile(&self.profile(uuid::Uuid::nil().to_string()))
            .map_err(|_| invalid())
    }

    fn profile(&self, id: String) -> ConnectionProfile {
        ConnectionProfile {
            id,
            name: NAME.into(),
            group_id: GROUP.into(),
            engine: EngineKind::Snowflake,
            host: self.host.clone(),
            port: 443,
            database: self.database.clone(),
            username: self.username.clone(),
            environment: Environment::Staging,
            safe_mode: SafeMode::Confirm,
            tls_mode: TlsMode::VerifyFull,
            ca_cert_path: None,
            ssh: None,
            remember_password: false,
            store_query_text: false,
            store_drafts: false,
            snowflake: Some(SnowflakeConfig {
                authentication: SnowflakeAuthentication::ExternalBrowser,
                reviewed_dml: true,
                reviewed_ddl: true,
                reviewed_console: false,
                role: self.role.clone(),
                warehouse: self.warehouse.clone(),
                schema: self.schema.clone(),
            }),
        }
    }

    /// The owner must exclusively allocate a fresh workspace and inject a
    /// keychain-free secret store. This route uses ordinary application saving,
    /// and never adopts or edits a previously stored profile.
    pub(super) fn save_new(&self, studio: &Studio) -> AppResult<ConnectionProfile> {
        self.validate()?;
        let before = studio.bootstrap()?;
        validate_workspace(&before)?;
        if !before.profiles.is_empty() {
            return Err(invalid());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let saved = studio.save_profile(self.profile(id.clone()), None)?;
        if saved.id != id {
            return Err(invalid());
        }
        validate_saved(self, &saved)?;
        let after = studio.bootstrap()?;
        validate_workspace(&after)?;
        let [persisted] = after.profiles.as_slice() else {
            return Err(invalid());
        };
        if persisted.id != id {
            return Err(invalid());
        }
        validate_saved(self, persisted)?;
        Ok(saved)
    }
}

fn validate_workspace(workspace: &Bootstrap) -> AppResult<()> {
    let staging: Vec<_> = workspace
        .groups
        .iter()
        .filter(|group| group.id == GROUP)
        .collect();
    if workspace.passphrase_configured
        || workspace.temporary_result_storage
        || workspace.workspace.is_some()
        || !workspace.saved_queries.is_empty()
        || !workspace.bookmarks.is_empty()
        || !workspace.sql_snippets.is_empty()
        || staging.len() != 1
        || staging[0].environment != Environment::Staging
        || staging[0].parent_id.is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

/// Check both the returned DTO and its persisted reload against the requested
/// policy. This catches ordinary save normalization instead of relying on it.
pub(super) fn validate_saved(input: &Input, profile: &ConnectionProfile) -> AppResult<()> {
    input.validate()?;
    if uuid::Uuid::parse_str(&profile.id)
        .map_err(|_| invalid())?
        .get_version_num()
        != 4
        || safety::effective_mode(profile) != SafeMode::Confirm
    {
        return Err(invalid());
    }
    let expected = serde_json::to_vec(&input.profile(profile.id.clone())).map_err(|_| invalid())?;
    let actual = serde_json::to_vec(profile).map_err(|_| invalid())?;
    if actual != expected {
        return Err(invalid());
    }
    Snowflake::validate_profile(profile).map_err(|_| invalid())
}

#[cfg(test)]
#[path = "snowflake_table_qualification_profile_tests.rs"]
mod tests;
