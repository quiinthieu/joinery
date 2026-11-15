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
    /// Immutable per-unit execution/result-delivery budget; settlement is separate.
    pub timeout_seconds: u32,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeConsoleReceiptVisibility {
    Visible,
    Archived,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SnowflakeConsoleReceiptKind {
    Positive,
    Refusal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleReceiptFilter {
    pub visibility: SnowflakeConsoleReceiptVisibility,
    pub kind: Option<SnowflakeConsoleReceiptKind>,
    pub connection_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleReceiptPageRequest {
    pub filter: SnowflakeConsoleReceiptFilter,
    pub cursor: Option<String>,
    pub limit: u32,
}

/// All SQLite ordering/revision integers are decimal strings, preserving i64
/// identity across JavaScript without rounding. No SQL, binds or rows appear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleReceiptSummary {
    pub inventory_epoch: String,
    pub sequence: String,
    pub client_request_id: String,
    pub session_id: String,
    pub connection_id: Option<String>,
    pub run_id: Option<String>,
    pub plan_digest: String,
    pub recorded_at: String,
    pub original_revision: String,
    pub browsing_revision: String,
    pub kind: SnowflakeConsoleReceiptKind,
    pub state: Option<ConsoleRunState>,
    pub stop_reason: Option<ConsoleStopReason>,
    pub unknown: bool,
    pub refusal_code: Option<ErrorCode>,
    pub archived: bool,
    /// Studio adds process-local owner/publisher/outbox eligibility to the
    /// durable check; this flag is presentation, never mutation authority.
    pub archive_eligible: bool,
    pub archive_block_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleReceiptPage {
    pub inventory_epoch: String,
    pub receipts: Vec<SnowflakeConsoleReceiptSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleReceiptArchiveItem {
    pub client_request_id: String,
    pub session_id: String,
    pub plan_digest: String,
    pub run_id: Option<String>,
    pub kind: SnowflakeConsoleReceiptKind,
    pub recorded_at: String,
    pub original_revision: String,
    pub browsing_revision: String,
    pub refusal_code: Option<ErrorCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleReceiptArchiveRequest {
    pub inventory_epoch: String,
    pub items: Vec<SnowflakeConsoleReceiptArchiveItem>,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleReceiptArchiveResult {
    pub receipts: Vec<SnowflakeConsoleReceiptSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsolePending {
    pub client_request_ids: Vec<String>,
    pub recording_hold: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnowflakeConsoleStorage {
    pub inventory_epoch: String,
    pub positive_count: String,
    pub refusal_count: String,
    /// None means not implemented/available; it is never a measured zero.
    pub charged_bytes: Option<String>,
    pub reserved_bytes: Option<String>,
    pub admission_budget_bytes: Option<String>,
    pub free_bytes: Option<String>,
    /// False until the entire recovery/retention workflow is qualified.
    pub retention_enabled: bool,
}
