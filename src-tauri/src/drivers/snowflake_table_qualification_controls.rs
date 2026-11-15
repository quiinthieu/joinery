//! Pure control grammar and target matching. Active ownership, replay limits,
//! generation pinning and phase effects belong to the owning attempt policy.
use super::wire;
use crate::{
    domain::{AppError, AppResult},
    drivers::{snowflake_process::Impact, snowflake_qualification_transport::PhysicalRequest},
};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlKind {
    Credit,
    Discard,
    Cancel,
    Close,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ControlTarget<'a> {
    pub logical_id: &'a str,
    pub ticket: u64,
    pub wire_id: u64,
    pub operation: &'a str,
}

fn denied() -> AppError {
    AppError::invalid("Physical Snowflake control does not match its qualification owner")
}

fn keys(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
    })
}

pub(crate) fn validate_control(
    request: &PhysicalRequest<'_>,
    target: Option<ControlTarget<'_>>,
    allow_close: bool,
) -> AppResult<ControlKind> {
    wire::validate_envelope(request)?;
    if request.impact != Impact::Control {
        return Err(denied());
    }
    let payload = &request.payload["payload"];
    if request.operation == "close" {
        return if allow_close && target.is_none() && request.target.is_none() && keys(payload, &[])
        {
            Ok(ControlKind::Close)
        } else {
            Err(denied())
        };
    }
    let target = target.ok_or_else(denied)?;
    if target.logical_id.is_empty()
        || target.logical_id.len() > 128
        || target.logical_id.contains('\0')
        || target.wire_id == 0
        || request.target != Some((target.logical_id, Some(target.ticket)))
        || payload["target"].as_str() != Some(target.wire_id.to_string().as_str())
    {
        return Err(denied());
    }
    match request.operation {
        "credit"
            if target.operation == "query" && keys(payload, &["target", "batches", "discard"]) =>
        {
            match (payload["batches"].as_u64(), payload["discard"].as_bool()) {
                (Some(1), Some(false)) => Ok(ControlKind::Credit),
                (Some(0), Some(true)) => Ok(ControlKind::Discard),
                _ => Err(denied()),
            }
        }
        "cancel"
            if keys(payload, &["target"])
                && matches!(
                    target.operation,
                    "query" | "connect" | "switchSchema" | "transaction" | "dml" | "ddl"
                ) =>
        {
            Ok(ControlKind::Cancel)
        }
        _ => Err(denied()),
    }
}

#[cfg(test)]
#[path = "snowflake_table_qualification_controls_tests.rs"]
mod tests;
