//! Pure matching of final production frames against the closed offline ledger.
//! Replay/phase ownership and permission to execute remain the caller's job.
use super::ledger::{SqlEntry, SqlKind};
use crate::{
    domain::{AppError, AppResult, ExplicitTransactionAction},
    drivers::{
        snowflake_process::{Impact, TransactionAction},
        snowflake_qualification_transport::PhysicalRequest,
        snowflake_sql_contract,
    },
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WireIdentity {
    user: String,
    session: String,
}

impl WireIdentity {
    pub(crate) fn new(user: String, session: String) -> AppResult<Self> {
        // Match the production helper's expected-identity string boundaries.
        if !bounded(&user, 1020) || user.chars().count() > 255 || !bounded(&session, 1024) {
            return Err(denied());
        }
        Ok(Self { user, session })
    }
    pub(crate) fn user(&self) -> &str {
        &self.user
    }
    pub(crate) fn session(&self) -> &str {
        &self.session
    }
}

fn denied() -> AppError {
    AppError::invalid("Physical Snowflake request does not match the closed qualification ledger")
}

fn bounded(value: &str, bytes: usize) -> bool {
    !value.is_empty() && value.len() <= bytes && !value.contains('\0')
}

fn keys(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
    })
}

/// Return the resolved, canonical decimal wire ID. The logical operation handle
/// and zero-based ticket are owned by the caller, and are not this wire ID.
pub(crate) fn validate_envelope(request: &PhysicalRequest<'_>) -> AppResult<u64> {
    let value = request.payload;
    if !keys(value, &["v", "generation", "id", "op", "payload"])
        || value["v"].as_u64() != Some(1)
        || !bounded(request.generation, 128)
        || !bounded(request.operation_id, 128)
        || !bounded(request.operation, 128)
        || value["generation"].as_str() != Some(request.generation)
        || value["op"].as_str() != Some(request.operation)
        || !value["payload"].is_object()
        || serde_json::to_vec(value).map_err(|_| denied())? != request.wire_payload
    {
        return Err(denied());
    }
    let text = value["id"].as_str().ok_or_else(denied)?;
    let wire_id = text.parse::<u64>().map_err(|_| denied())?;
    if wire_id == 0 || wire_id.to_string() != text {
        return Err(denied());
    }
    Ok(wire_id)
}

/// The profile must already be frozen by the owning attempt. This function
/// admits neither arbitrary SQL nor an alternate caller-selected account route.
pub(crate) fn validate_connect(
    request: &PhysicalRequest<'_>,
    expected_profile: &Value,
) -> AppResult<()> {
    validate_envelope(request)?;
    let payload = &request.payload["payload"];
    let fields = [
        "account",
        "host",
        "username",
        "role",
        "warehouse",
        "database",
        "schema",
    ];
    if request.operation != "connect"
        || request.impact != Impact::Control
        || request.target.is_some()
        || !keys(expected_profile, &fields)
        || !fields
            .iter()
            .all(|field| expected_profile[*field].is_string())
        || payload != expected_profile
    {
        return Err(denied());
    }
    Ok(())
}

fn expected_identity(value: &Value, identity: Option<&WireIdentity>) -> bool {
    identity.is_some_and(|identity| {
        keys(value, &["user", "session"])
            && value["user"].as_str() == Some(identity.user())
            && value["session"].as_str() == Some(identity.session())
    })
}

fn empty_binds(value: &Value) -> bool {
    value.as_array().is_some_and(Vec::is_empty)
}

pub(crate) fn validate_sql(
    entry: &SqlEntry,
    request: &PhysicalRequest<'_>,
    identity: Option<&WireIdentity>,
) -> AppResult<()> {
    validate_envelope(request)?;
    if request.target.is_some()
        || request.operation != entry.kind().operation()
        || !entry.binds().is_empty()
    {
        return Err(denied());
    }
    let payload = &request.payload["payload"];
    let matches = match entry.kind() {
        SqlKind::BootstrapContext => {
            request.impact == Impact::Read
                && entry.retention().is_none()
                && entry.sql() == snowflake_sql_contract::BOOTSTRAP_CONTEXT_SQL
                && keys(payload, &[])
        }
        SqlKind::Read(_) | SqlKind::Readback(_) => {
            request.impact == Impact::Read
                && keys(
                    payload,
                    &[
                        "sql",
                        "binds",
                        "classification",
                        "credits",
                        "maxRows",
                        "maxBytes",
                    ],
                )
                && payload["sql"].as_str() == Some(entry.sql())
                && empty_binds(&payload["binds"])
                && payload["classification"].as_str() == Some("read")
                && payload["credits"].as_u64() == Some(1)
                && entry.retention().is_some_and(|retention| {
                    payload["maxRows"].as_u64() == Some(retention.max_rows)
                        && payload["maxBytes"].as_u64() == Some(retention.max_bytes)
                })
        }
        SqlKind::Ddl(kind) => {
            request.impact == Impact::Mutating
                && entry.retention().is_none()
                && keys(payload, &["kind", "sql", "expected"])
                && payload["kind"].as_str() == Some(kind.as_str())
                && payload["sql"].as_str() == Some(entry.sql())
                && expected_identity(&payload["expected"], identity)
        }
        SqlKind::Dml(kind) => {
            request.impact == Impact::Mutating
                && entry.retention().is_none()
                && keys(payload, &["kind", "sql", "binds", "expected"])
                && serde_json::to_value(kind).is_ok_and(|kind| payload["kind"] == kind)
                && payload["sql"].as_str() == Some(entry.sql())
                && empty_binds(&payload["binds"])
                && expected_identity(&payload["expected"], identity)
        }
        SqlKind::Transaction(action) => {
            let wire_action = match action {
                ExplicitTransactionAction::Begin => TransactionAction::Begin,
                ExplicitTransactionAction::Commit => TransactionAction::Commit,
                ExplicitTransactionAction::Rollback => TransactionAction::Rollback,
            };
            request.impact == wire_action.impact()
                && entry.retention().is_none()
                && entry.sql() == action.sql()
                && keys(payload, &["action", "expected"])
                && payload["action"].as_str() == Some(wire_action.as_str())
                && expected_identity(&payload["expected"], identity)
        }
    };
    if matches { Ok(()) } else { Err(denied()) }
}

#[cfg(test)]
#[path = "snowflake_table_qualification_wire_tests.rs"]
mod tests;
