//! Public, SQL-free console receipts and immutable local review requests.
use crate::domain::{ErrorCode, PreparedPlan, QueryParamInput, ResultInfo};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use crate::persistence::snowflake_console::{
    ConsoleAccounting, ConsoleAcknowledgement, ConsoleContext, ConsoleErrorCode,
    ConsoleNotDispatched, ConsoleNotDispatchedReason, ConsoleObservations, ConsoleRejection,
    ConsoleRunMetadata, ConsoleRunSnapshot, ConsoleRunState, ConsoleStatement, ConsoleStep,
    ConsoleStepOutcome, ConsoleStopReason, ConsoleTerminal, ConsoleUnknown,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeConsoleMode {
    ParsedStatements,
    SelectedUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeConsoleValidation {
    LocalSyntax,
    ServerAtExecution,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleRequest {
    pub sql: String,
    pub mode: SnowflakeConsoleMode,
    pub parameters: Option<Vec<QueryParamInput>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsolePlan {
    pub review: PreparedPlan,
    /// Server-held plan fingerprint, echoed only for correlation and checked
    /// against the trusted stored plan before the first admission.
    pub digest: String,
    pub mode: SnowflakeConsoleMode,
    /// Aligned with the immutable ordered review statements.
    pub validations: Vec<SnowflakeConsoleValidation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleStart {
    pub session_id: String,
    pub plan_id: String,
    pub plan_digest: String,
    /// Retained before sending start. SQL-free lookup of this same identity is
    /// the recovery path when a start response is lost; never resubmit SQL.
    pub client_request_id: String,
    pub approval: Option<String>,
}

/// A negative is authoritative only after native admission durably records this
/// exact identity as refused before any positive journal insertion or dispatch.
/// Transport errors and a missing lookup receipt never imply NotAdmitted.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SnowflakeConsoleStartResult {
    Started {
        #[serde(rename = "runId")]
        run_id: String,
    },
    NotAdmitted {
        #[serde(rename = "clientRequestId")]
        client_request_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "planDigest")]
        plan_digest: String,
        code: ErrorCode,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleRunView {
    pub run: ConsoleRunSnapshot,
    /// Results retained by this process only. Durable receipts contain no rows;
    /// an empty list after restart does not mean the original query had no rows.
    pub results: Vec<ResultInfo>,
    pub pending_recording: bool,
}

/// SQL-free recovery for both durable admission decisions. Absence stays unknown.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SnowflakeConsoleRequestLookup {
    Started {
        view: SnowflakeConsoleRunView,
    },
    NotAdmitted {
        #[serde(rename = "clientRequestId")]
        client_request_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "planDigest")]
        plan_digest: String,
        code: ErrorCode,
    },
}
