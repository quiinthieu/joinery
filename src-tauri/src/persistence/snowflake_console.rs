//! SQL-free evidence for reviewed console owners. A committed intent is not proof
//! of submission. Only an original acknowledgement records statement success.
//! Storage errors leave the caller responsible for retaining its in-process
//! evidence/outbox and stopping admissions; this module never fabricates a write.

use super::{Persistence, db_error};
use crate::domain::{AppError, AppResult, ErrorCode};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
};
use ts_rs::TS;

mod accounting;
#[path = "history/console_lifecycle.rs"]
mod lifecycle;
mod retention;
pub(crate) use lifecycle::ConsoleLifecycleSnapshot;
pub(crate) use retention::schema_objects;
pub(super) use retention::{migrate_v4, migrate_v5};

pub(crate) const MAX_STEPS: u32 = 100;
pub(crate) const MAX_RUNS: u32 = 10_000;
pub(crate) const MAX_REJECTIONS: u32 = 10_000;
const MAX_JSON_BYTES: usize = 16_384;
const MAX_REVISION: u64 = i64::MAX as u64 - 1;

pub(super) const SCHEMA_SQL: &str = "
CREATE TABLE snowflake_console_runs(
 run_id TEXT PRIMARY KEY NOT NULL,
 client_request_id TEXT NOT NULL UNIQUE,
 session_id TEXT NOT NULL,
 connection_id TEXT NOT NULL,
 plan_digest TEXT NOT NULL,
 step_count INTEGER NOT NULL CHECK(step_count BETWEEN 1 AND 100),
 created_at INTEGER NOT NULL CHECK(created_at >= 0),
 revision INTEGER NOT NULL CHECK(revision >= 0),
 state TEXT NOT NULL CHECK(state IN ('prepared','running','terminal')),
 observations_json TEXT NOT NULL,
 terminal_json TEXT
);
CREATE TABLE snowflake_console_steps(
 run_id TEXT NOT NULL REFERENCES snowflake_console_runs(run_id),
 ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 0 AND 99),
 intended_at INTEGER NOT NULL CHECK(intended_at >= 0),
 outcome_json TEXT,
 observations_json TEXT NOT NULL,
 PRIMARY KEY(run_id,ordinal)
);";

pub(super) const REJECTION_SCHEMA_SQL: &str = "
CREATE TABLE snowflake_console_rejections_v3(
 client_request_id TEXT PRIMARY KEY NOT NULL,
 session_id TEXT NOT NULL,
 plan_digest TEXT NOT NULL,
 code TEXT NOT NULL,
 recorded_at INTEGER NOT NULL CHECK(recorded_at >= 0)
);";

pub(super) const DECISION_TRIGGERS: [(&str, &str); 4] = [
    (
        "console_run_insert_decision",
        "CREATE TRIGGER console_run_insert_decision BEFORE INSERT ON snowflake_console_runs_v3 WHEN EXISTS(SELECT 1 FROM snowflake_console_rejections_v3 WHERE client_request_id=NEW.client_request_id) BEGIN SELECT RAISE(ABORT,'conflicting console decision'); END",
    ),
    (
        "console_run_update_decision",
        "CREATE TRIGGER console_run_update_decision BEFORE UPDATE OF client_request_id ON snowflake_console_runs_v3 WHEN EXISTS(SELECT 1 FROM snowflake_console_rejections_v3 WHERE client_request_id=NEW.client_request_id) BEGIN SELECT RAISE(ABORT,'conflicting console decision'); END",
    ),
    (
        "console_rejection_insert_decision",
        "CREATE TRIGGER console_rejection_insert_decision BEFORE INSERT ON snowflake_console_rejections_v3 WHEN EXISTS(SELECT 1 FROM snowflake_console_runs_v3 WHERE client_request_id=NEW.client_request_id) BEGIN SELECT RAISE(ABORT,'conflicting console decision'); END",
    ),
    (
        "console_rejection_update_decision",
        "CREATE TRIGGER console_rejection_update_decision BEFORE UPDATE OF client_request_id ON snowflake_console_rejections_v3 WHEN EXISTS(SELECT 1 FROM snowflake_console_runs_v3 WHERE client_request_id=NEW.client_request_id) BEGIN SELECT RAISE(ABORT,'conflicting console decision'); END",
    ),
];

/// Immutable local refusal evidence, never a remote SQL outcome or authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConsoleAdmissionRejection {
    pub client_request_id: String,
    pub session_id: String,
    pub plan_digest: String,
    pub code: ErrorCode,
    pub recorded_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleRunMetadata {
    pub run_id: String,
    pub client_request_id: String,
    pub session_id: String,
    pub connection_id: String,
    pub plan_digest: String,
    pub step_count: u32,
    #[ts(type = "number")]
    pub created_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConsoleRunState {
    Prepared,
    Running,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConsoleErrorCode {
    InvalidInput,
    Unsupported,
    Database,
    ConnectionClosed,
    OutcomeUnknown,
    Cancelled,
    ResultLimit,
    Protocol,
    Delivery,
    Recording,
    Interrupted,
}

/// These observations accumulate independently of the original statement result.
/// `delivered` and `delivery_failed` may both be true for partial publication.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleObservations {
    pub delivered: bool,
    pub delivery_failed: bool,
    pub cancellation_observed: bool,
    pub retired: bool,
    pub truncated: bool,
    pub recording_failed: bool,
    pub first_error: Option<ConsoleErrorCode>,
}

impl ConsoleObservations {
    fn blocks_admission(&self) -> bool {
        self.delivery_failed
            || self.cancellation_observed
            || self.retired
            || self.truncated
            || self.recording_failed
            || self.first_error.is_some()
    }

    fn absorb(&mut self, other: &Self) {
        self.delivered |= other.delivered;
        self.delivery_failed |= other.delivery_failed;
        self.cancellation_observed |= other.cancellation_observed;
        self.retired |= other.retired;
        self.truncated |= other.truncated;
        self.recording_failed |= other.recording_failed;
        if self.first_error.is_none() {
            self.first_error = other.first_error;
        }
    }

    fn apply(&mut self, patch: &Self) -> AppResult<()> {
        if self.first_error.is_some()
            && patch.first_error.is_some()
            && self.first_error != patch.first_error
        {
            return Err(conflict());
        }
        let prior = self.clone();
        self.absorb(patch);
        if *self == prior {
            return Err(conflict());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleStatement {
    pub sdk_request_id: String,
    pub query_id: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleContext {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub role: Option<String>,
    pub warehouse: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleAcknowledgement {
    pub statement: ConsoleStatement,
    pub context: ConsoleContext,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleRejection {
    pub statement: Option<ConsoleStatement>,
    pub code: ConsoleErrorCode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleUnknown {
    pub statement: Option<ConsoleStatement>,
}

/// This evidence requires a proven negative supervisor admission, not merely
/// absence of a report or a remote error. No SDK request identity exists here.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConsoleNotDispatchedReason {
    Cancelled,
    Revoked,
    SessionClosed,
    InvalidInput,
    AdmissionRejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleNotDispatched {
    pub reason: ConsoleNotDispatchedReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "evidence",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum ConsoleStepOutcome {
    Acknowledged(ConsoleAcknowledgement),
    Rejected(ConsoleRejection),
    Unknown(ConsoleUnknown),
    NotDispatched(ConsoleNotDispatched),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleStep {
    /// Zero-based index in the immutable prepared execution units.
    pub ordinal: u32,
    #[ts(type = "number")]
    pub intended_at: u64,
    pub outcome: Option<ConsoleStepOutcome>,
    pub observations: ConsoleObservations,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConsoleStopReason {
    Completed,
    ResultLimit,
    Cancelled,
    Failed,
    OutcomeUnknown,
    Interrupted,
    RecordingFailed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleAccounting {
    pub planned: u32,
    /// Durable intents, not proof of helper invocation or remote submission.
    pub intended: u32,
    pub acknowledged: u32,
    pub rejected: u32,
    pub unknown: u32,
    /// Proven not accepted by the local supervisor, despite a durable intent.
    pub not_dispatched: u32,
    /// Untouched suffix, with no durable intent.
    pub not_intended: u32,
    /// Untouched suffix plus proven non-admission; never proof of remote SQL.
    pub not_attempted: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleTerminal {
    pub reason: ConsoleStopReason,
    #[ts(type = "number")]
    pub finished_at: u64,
    pub accounting: ConsoleAccounting,
    /// Immutable terminal-time observations; later delivery evidence is separate.
    pub observations: ConsoleObservations,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleRunSnapshot {
    pub metadata: ConsoleRunMetadata,
    #[ts(type = "number")]
    pub revision: u64,
    pub state: ConsoleRunState,
    pub observations: ConsoleObservations,
    pub steps: Vec<ConsoleStep>,
    pub terminal: Option<ConsoleTerminal>,
}

#[allow(
    dead_code,
    reason = "Durable console journal foundation; the retained execution coordinator is not enrolled yet"
)]
pub(crate) struct ConsolePrepareResult {
    pub created: bool,
    pub run: ConsoleRunSnapshot,
}

/// Held across console runs. Dropping the handle releases the OS lease without
/// unlinking the persistent lock inode or performing any database/SDK operation.
pub(crate) struct ConsoleJournalOwner {
    file: File,
    root: PathBuf,
}

fn owner_error() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The reviewed console journal is owned by another coordinator or its owner lease is invalid",
    )
}

impl ConsoleJournalOwner {
    pub(crate) fn validate(&self, store: &Persistence) -> AppResult<()> {
        self.validate_root(&store.root)
    }

    fn validate_root(&self, root: &std::path::Path) -> AppResult<()> {
        if self.root != root {
            return Err(owner_error());
        }
        let path = self.root.join("snowflake-console.lock");
        let named = std::fs::symlink_metadata(path).map_err(|_| owner_error())?;
        let held = self.file.metadata().map_err(|_| owner_error())?;
        if !named.is_file() || named.file_type().is_symlink() || !held.is_file() {
            return Err(owner_error());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if named.dev() != held.dev()
                || named.ino() != held.ino()
                || held.nlink() != 1
                || held.mode() & 0o777 != 0o600
                || held.uid() != unsafe { libc::geteuid() }
            {
                return Err(owner_error());
            }
        }
        Ok(())
    }
}

fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The reviewed console journal transition is stale or incompatible",
    )
}

fn invalid() -> AppError {
    AppError::invalid("The reviewed console journal metadata is invalid or out of bounds")
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok()
}

fn valid_app_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

fn valid_time(value: u64) -> bool {
    value <= i64::MAX as u64
}

pub(crate) fn validate_console_request_identity(
    client: &str,
    session: &str,
    digest: &str,
) -> AppResult<()> {
    if !valid_app_uuid(client)
        || !valid_app_uuid(session)
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid());
    }
    Ok(())
}

fn require_current_schema(connection: &Connection) -> AppResult<()> {
    if super::schema_version(connection)? != super::SCHEMA_VERSION {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "This reviewed console journal needs the matching Joinery version",
        ));
    }
    Ok(())
}

fn validate_rejection(value: &ConsoleAdmissionRejection) -> AppResult<()> {
    validate_console_request_identity(
        &value.client_request_id,
        &value.session_id,
        &value.plan_digest,
    )?;
    if !valid_time(value.recorded_at) {
        return Err(invalid());
    }
    Ok(())
}

fn validate_metadata(value: &ConsoleRunMetadata) -> AppResult<()> {
    validate_console_request_identity(
        &value.client_request_id,
        &value.session_id,
        &value.plan_digest,
    )?;
    if ![
        &value.run_id,
        &value.client_request_id,
        &value.session_id,
        &value.connection_id,
    ]
    .into_iter()
    .all(|id| valid_app_uuid(id))
        || value.plan_digest.len() != 64
        || !value
            .plan_digest
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || !(1..=MAX_STEPS).contains(&value.step_count)
        || !valid_time(value.created_at)
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_outcome(value: &ConsoleStepOutcome) -> AppResult<()> {
    let statement = match value {
        ConsoleStepOutcome::Acknowledged(ack) => {
            if [
                &ack.context.database,
                &ack.context.schema,
                &ack.context.role,
                &ack.context.warehouse,
            ]
            .into_iter()
            .flatten()
            .any(|s| s.is_empty() || s.len() > 1020 || s.chars().count() > 255 || s.contains('\0'))
            {
                return Err(invalid());
            }
            Some(&ack.statement)
        }
        ConsoleStepOutcome::Rejected(rejection) => {
            if !matches!(
                rejection.code,
                ConsoleErrorCode::Database
                    | ConsoleErrorCode::InvalidInput
                    | ConsoleErrorCode::Unsupported
            ) {
                return Err(invalid());
            }
            rejection.statement.as_ref()
        }
        ConsoleStepOutcome::Unknown(unknown) => unknown.statement.as_ref(),
        ConsoleStepOutcome::NotDispatched(_) => None,
    };
    if statement.is_some_and(|s| {
        !valid_uuid(&s.sdk_request_id) || s.query_id.as_ref().is_some_and(|id| !valid_uuid(id))
    }) {
        return Err(invalid());
    }
    Ok(())
}

fn accounting(run: &ConsoleRunSnapshot) -> ConsoleAccounting {
    let mut result = ConsoleAccounting {
        planned: run.metadata.step_count,
        intended: run.steps.len() as u32,
        acknowledged: 0,
        rejected: 0,
        unknown: 0,
        not_dispatched: 0,
        not_intended: run.metadata.step_count - run.steps.len() as u32,
        not_attempted: run.metadata.step_count - run.steps.len() as u32,
    };
    for step in &run.steps {
        match step.outcome {
            Some(ConsoleStepOutcome::Acknowledged(_)) => result.acknowledged += 1,
            Some(ConsoleStepOutcome::Rejected(_)) => result.rejected += 1,
            Some(ConsoleStepOutcome::Unknown(_)) => result.unknown += 1,
            Some(ConsoleStepOutcome::NotDispatched(_)) => {
                result.not_dispatched += 1;
                result.not_attempted += 1;
            }
            None => {}
        }
    }
    result
}

fn validate_snapshot(run: &ConsoleRunSnapshot) -> AppResult<()> {
    validate_metadata(&run.metadata)?;
    if run.revision > MAX_REVISION || run.steps.len() > run.metadata.step_count as usize {
        return Err(invalid());
    }
    for (i, step) in run.steps.iter().enumerate() {
        if step.ordinal as usize != i || !valid_time(step.intended_at) {
            return Err(invalid());
        }
        if let Some(outcome) = &step.outcome {
            validate_outcome(outcome)?;
        }
        if i + 1 < run.steps.len()
            && !matches!(step.outcome, Some(ConsoleStepOutcome::Acknowledged(_)))
        {
            return Err(invalid());
        }
    }
    let mut aggregate = run.observations.clone();
    for step in &run.steps {
        aggregate.absorb(&step.observations);
    }
    if aggregate != run.observations {
        return Err(invalid());
    }
    match (run.state, &run.terminal) {
        (ConsoleRunState::Prepared, None) if run.steps.is_empty() => {}
        (ConsoleRunState::Running, None) if !run.steps.is_empty() => {}
        (ConsoleRunState::Terminal, Some(terminal)) => {
            if !valid_time(terminal.finished_at)
                || terminal.accounting != accounting(run)
                || run.steps.iter().any(|s| s.outcome.is_none())
            {
                return Err(invalid());
            }
            if !observations_cover(&run.observations, &terminal.observations) {
                return Err(invalid());
            }
            validate_finish(run, terminal.reason, &terminal.observations)?;
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

/// Unknown originals dominate every normal stop reason. Recovery interruption
/// remains explicit. For known originals: recording failure, statement/delivery
/// failure, result limit, cancellation, then clean completion, in that order.
fn validate_finish(
    run: &ConsoleRunSnapshot,
    reason: ConsoleStopReason,
    observed: &ConsoleObservations,
) -> AppResult<()> {
    if run.steps.iter().any(|step| step.outcome.is_none()) {
        return Err(conflict());
    }
    let counts = accounting(run);
    let local_not_dispatched = run.steps.iter().find_map(|step| match &step.outcome {
        Some(ConsoleStepOutcome::NotDispatched(local)) => Some(local.reason),
        _ => None,
    });
    if reason == ConsoleStopReason::Interrupted {
        return if observed.retired {
            Ok(())
        } else {
            Err(conflict())
        };
    }
    let expected = if counts.unknown > 0 {
        Some(ConsoleStopReason::OutcomeUnknown)
    } else if observed.recording_failed {
        Some(ConsoleStopReason::RecordingFailed)
    } else if counts.rejected > 0
        || local_not_dispatched
            .is_some_and(|reason| reason != ConsoleNotDispatchedReason::Cancelled)
        || observed.delivery_failed
        || observed.first_error.is_some_and(|code| {
            !matches!(
                code,
                ConsoleErrorCode::Cancelled | ConsoleErrorCode::ResultLimit
            )
        })
    {
        Some(ConsoleStopReason::Failed)
    } else if observed.truncated {
        Some(ConsoleStopReason::ResultLimit)
    } else if observed.cancellation_observed
        || local_not_dispatched == Some(ConsoleNotDispatchedReason::Cancelled)
    {
        Some(ConsoleStopReason::Cancelled)
    } else if observed.retired || observed.first_error.is_some() {
        Some(ConsoleStopReason::Failed)
    } else if counts.acknowledged == counts.planned {
        Some(ConsoleStopReason::Completed)
    } else {
        None
    };
    if expected != Some(reason) {
        return Err(conflict());
    }
    Ok(())
}

fn encode<T: Serialize>(value: &T) -> AppResult<String> {
    let json = serde_json::to_string(value).map_err(|_| invalid())?;
    if json.len() > MAX_JSON_BYTES {
        return Err(invalid());
    }
    Ok(json)
}

fn decode<T: serde::de::DeserializeOwned>(json: &str) -> AppResult<T> {
    if json.len() > MAX_JSON_BYTES {
        return Err(invalid());
    }
    serde_json::from_str(json).map_err(|_| invalid())
}

fn state_name(state: ConsoleRunState) -> &'static str {
    match state {
        ConsoleRunState::Prepared => "prepared",
        ConsoleRunState::Running => "running",
        ConsoleRunState::Terminal => "terminal",
    }
}

fn sqlite_integer(value: u64) -> AppResult<i64> {
    i64::try_from(value).map_err(|_| invalid())
}

fn sqlite_nonnegative(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn read_sql<'a>(connection: &Connection, sql: &'a str) -> AppResult<std::borrow::Cow<'a, str>> {
    match super::schema_version(connection)? {
        2 => Ok(std::borrow::Cow::Owned(
            sql.replace("snowflake_console_runs_v5", "snowflake_console_runs")
                .replace("snowflake_console_steps_v5", "snowflake_console_steps"),
        )),
        3 => Ok(std::borrow::Cow::Owned(sql.replace("_v5", "_v3"))),
        4 => Ok(std::borrow::Cow::Owned(sql.replace("_v5", "_v4"))),
        _ => Ok(std::borrow::Cow::Borrowed(sql)),
    }
}

fn ensure_single_decision(connection: &Connection, client: &str) -> AppResult<()> {
    if super::schema_version(connection)? >= 3 {
        let dual: bool = connection.query_row(&read_sql(connection, "SELECT EXISTS(SELECT 1 FROM snowflake_console_runs_v5 r JOIN snowflake_console_rejections_v5 n USING(client_request_id) WHERE r.client_request_id=?1)")?, [client], |row| row.get(0)).map_err(db_error)?;
        if dual {
            return Err(conflict());
        }
    }
    Ok(())
}

fn read_rejection(
    connection: &Connection,
    client: &str,
) -> AppResult<Option<ConsoleAdmissionRejection>> {
    let row = connection.query_row(
        &read_sql(connection, "SELECT CASE WHEN length(CAST(client_request_id AS BLOB))=36 THEN client_request_id ELSE '' END,CASE WHEN length(CAST(session_id AS BLOB))=36 THEN session_id ELSE '' END,CASE WHEN length(CAST(plan_digest AS BLOB))=64 THEN plan_digest ELSE '' END,CASE WHEN length(CAST(code AS BLOB))<=64 THEN code ELSE '' END,recorded_at FROM snowflake_console_rejections_v5 WHERE client_request_id=?1")?,
        [client], |row| Ok((row.get::<_, String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,sqlite_nonnegative(row,4)?)),
    ).optional().map_err(db_error)?;
    let Some((client_request_id, session_id, plan_digest, code, recorded_at)) = row else {
        return Ok(None);
    };
    let value = ConsoleAdmissionRejection {
        client_request_id,
        session_id,
        plan_digest,
        code: serde_json::from_value(serde_json::Value::String(code)).map_err(|_| invalid())?,
        recorded_at,
    };
    validate_rejection(&value)?;
    ensure_single_decision(connection, client)?;
    Ok(Some(value))
}

fn insert_rejection(connection: &Connection, value: &ConsoleAdmissionRejection) -> AppResult<()> {
    let encoded = serde_json::to_value(value.code).map_err(|_| invalid())?;
    let code = encoded.as_str().ok_or_else(invalid)?;
    connection.execute("INSERT INTO snowflake_console_rejections_v5(client_request_id,session_id,plan_digest,code,recorded_at) VALUES(?1,?2,?3,?4,?5)", params![value.client_request_id,value.session_id,value.plan_digest,code,sqlite_integer(value.recorded_at)?]).map_err(db_error)?;
    Ok(())
}

fn read_run(connection: &Connection, id: &str) -> AppResult<Option<ConsoleRunSnapshot>> {
    // Bound text before copying it out of SQLite, including corrupt local data.
    let oversized: bool = connection.query_row(&read_sql(connection, "SELECT EXISTS(SELECT 1 FROM snowflake_console_runs_v5 WHERE run_id=?1 AND (length(CAST(run_id AS BLOB))!=36 OR length(CAST(client_request_id AS BLOB))!=36 OR length(CAST(session_id AS BLOB))!=36 OR length(CAST(connection_id AS BLOB))!=36 OR length(CAST(plan_digest AS BLOB))!=64 OR length(CAST(state AS BLOB))>8 OR length(CAST(observations_json AS BLOB))>16384 OR length(CAST(terminal_json AS BLOB))>16384)) OR EXISTS(SELECT 1 FROM snowflake_console_steps_v5 WHERE run_id=?1 AND (length(CAST(outcome_json AS BLOB))>16384 OR length(CAST(observations_json AS BLOB))>16384))")?, [id], |row| row.get(0)).map_err(db_error)?;
    if oversized {
        return Err(invalid());
    }
    let row = connection.query_row(
        &read_sql(connection, "SELECT run_id,client_request_id,session_id,connection_id,plan_digest,step_count,created_at,revision,state,observations_json,terminal_json FROM snowflake_console_runs_v5 WHERE run_id=?1")?,
        [id], |row| Ok((ConsoleRunMetadata { run_id: row.get(0)?, client_request_id: row.get(1)?, session_id: row.get(2)?, connection_id: row.get(3)?, plan_digest: row.get(4)?, step_count: row.get(5)?, created_at: sqlite_nonnegative(row,6)? }, sqlite_nonnegative(row,7)?, row.get::<_,String>(8)?, row.get::<_,String>(9)?, row.get::<_,Option<String>>(10)?)),
    ).optional().map_err(db_error)?;
    let Some((metadata, revision, state, observations, terminal)) = row else {
        return Ok(None);
    };
    let state = match state.as_str() {
        "prepared" => ConsoleRunState::Prepared,
        "running" => ConsoleRunState::Running,
        "terminal" => ConsoleRunState::Terminal,
        _ => return Err(invalid()),
    };
    let mut statement = connection.prepare(&read_sql(connection, "SELECT ordinal,intended_at,outcome_json,observations_json FROM snowflake_console_steps_v5 WHERE run_id=?1 ORDER BY ordinal LIMIT 101")?).map_err(db_error)?;
    let rows = statement
        .query_map([id], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                sqlite_nonnegative(row, 1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(db_error)?;
    let mut steps = Vec::new();
    for row in rows {
        let (ordinal, intended_at, outcome, observations) = row.map_err(db_error)?;
        steps.push(ConsoleStep {
            ordinal,
            intended_at,
            outcome: outcome.as_deref().map(decode).transpose()?,
            observations: decode(&observations)?,
        });
    }
    let run = ConsoleRunSnapshot {
        metadata,
        revision,
        state,
        observations: decode(&observations)?,
        steps,
        terminal: terminal.as_deref().map(decode).transpose()?,
    };
    validate_snapshot(&run)?;
    ensure_single_decision(connection, &run.metadata.client_request_id)?;
    Ok(Some(run))
}

fn write_run(
    connection: &Connection,
    run: &ConsoleRunSnapshot,
    prior_revision: u64,
) -> AppResult<()> {
    require_current_schema(connection)?;
    let updated = connection.execute("UPDATE snowflake_console_runs_v5 SET revision=?1,state=?2,observations_json=?3,terminal_json=?4 WHERE run_id=?5 AND revision=?6",
        params![sqlite_integer(run.revision)?, state_name(run.state), encode(&run.observations)?, run.terminal.as_ref().map(encode).transpose()?, run.metadata.run_id, sqlite_integer(prior_revision)?]).map_err(db_error)?;
    if updated != 1 {
        return Err(conflict());
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "Durable console journal foundation; the retained execution coordinator is not enrolled yet"
)]
impl Persistence {
    pub(crate) fn console_prepare_run(
        &self,
        owner: &ConsoleJournalOwner,
        metadata: ConsoleRunMetadata,
    ) -> AppResult<ConsolePrepareResult> {
        owner.validate(self)?;
        validate_metadata(&metadata)?;
        let mut connection = self.connection.lock();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        require_current_schema(&tx)?;
        if read_rejection(&tx, &metadata.client_request_id)?.is_some() {
            return Err(conflict());
        }
        let prior: Option<String> = tx
            .query_row(
                "SELECT CASE WHEN length(CAST(run_id AS BLOB))=36 THEN run_id ELSE '' END FROM snowflake_console_runs_v5 WHERE client_request_id=?1",
                [&metadata.client_request_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(id) = prior {
            let run = read_run(&tx, &id)?.ok_or_else(invalid)?;
            let old = &run.metadata;
            if old.session_id != metadata.session_id
                || old.connection_id != metadata.connection_id
                || old.plan_digest != metadata.plan_digest
                || old.step_count != metadata.step_count
            {
                return Err(conflict());
            }
            tx.commit().map_err(db_error)?;
            return Ok(ConsolePrepareResult {
                created: false,
                run,
            });
        }
        let count: u32 = tx
            .query_row("SELECT count(*) FROM snowflake_console_runs_v5", [], |r| {
                r.get(0)
            })
            .map_err(db_error)?;
        if count >= MAX_RUNS {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "The reviewed console journal capacity is exhausted; existing evidence has been preserved",
            ));
        }
        if read_run(&tx, &metadata.run_id)?.is_some() {
            return Err(conflict());
        }
        let run = ConsoleRunSnapshot {
            metadata,
            revision: 0,
            state: ConsoleRunState::Prepared,
            observations: ConsoleObservations::default(),
            steps: vec![],
            terminal: None,
        };
        tx.execute("INSERT INTO snowflake_console_runs_v5(run_id,client_request_id,session_id,connection_id,plan_digest,step_count,created_at,revision,state,observations_json,terminal_json) VALUES(?1,?2,?3,?4,?5,?6,?7,0,'prepared',?8,NULL)",
            params![run.metadata.run_id,run.metadata.client_request_id,run.metadata.session_id,run.metadata.connection_id,run.metadata.plan_digest,run.metadata.step_count,sqlite_integer(run.metadata.created_at)?,encode(&run.observations)?]).map_err(db_error)?;
        // Compatibility fixture/foundation path; application admissions use the
        // atomic lifecycle API and always have a registered history ticket.
        tx.execute(
            "INSERT INTO console_lifecycle_v5 VALUES(?1,NULL,1,0)",
            [&run.metadata.run_id],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(ConsolePrepareResult { created: true, run })
    }

    pub(crate) fn console_run(&self, run_id: &str) -> AppResult<Option<ConsoleRunSnapshot>> {
        if !valid_uuid(run_id) {
            return Err(invalid());
        }
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        require_current_schema(&tx)?;
        let run = read_run(&tx, run_id)?;
        tx.commit().map_err(db_error)?;
        Ok(run)
    }

    pub(crate) fn console_run_by_client_request(
        &self,
        client_request_id: &str,
    ) -> AppResult<Option<ConsoleRunSnapshot>> {
        if !valid_uuid(client_request_id) {
            return Err(invalid());
        }
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        require_current_schema(&tx)?;
        read_rejection(&tx, client_request_id)?;
        let id: Option<String> = tx
            .query_row(
                "SELECT CASE WHEN length(CAST(run_id AS BLOB))=36 THEN run_id ELSE '' END FROM snowflake_console_runs_v5 WHERE client_request_id=?1",
                [client_request_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let run = id.map(|id| read_run(&tx, &id)).transpose()?.flatten();
        tx.commit().map_err(db_error)?;
        Ok(run)
    }

    pub(crate) fn console_rejection_by_client_request(
        &self,
        client_request_id: &str,
    ) -> AppResult<Option<ConsoleAdmissionRejection>> {
        if !valid_app_uuid(client_request_id) {
            return Err(invalid());
        }
        let mut connection = self.connection.lock();
        let tx = connection.transaction().map_err(db_error)?;
        require_current_schema(&tx)?;
        let value = read_rejection(&tx, client_request_id)?;
        tx.commit().map_err(db_error)?;
        Ok(value)
    }

    pub(crate) fn console_record_rejection(
        &self,
        owner: &ConsoleJournalOwner,
        value: &ConsoleAdmissionRejection,
    ) -> AppResult<ConsoleAdmissionRejection> {
        owner.validate(self)?;
        validate_rejection(value)?;
        let mut connection = self.connection.lock();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        require_current_schema(&tx)?;
        if let Some(prior) = read_rejection(&tx, &value.client_request_id)? {
            if prior != *value {
                return Err(conflict());
            }
            tx.commit().map_err(db_error)?;
            return Ok(prior);
        }
        let positive: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM snowflake_console_runs_v5 WHERE client_request_id=?1)",
                [&value.client_request_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if positive {
            return Err(conflict());
        }
        let count: u32 = tx
            .query_row(
                "SELECT count(*) FROM snowflake_console_rejections_v5",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if count >= MAX_REJECTIONS {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "The reviewed console refusal capacity is exhausted; existing evidence has been preserved",
            ));
        }
        insert_rejection(&tx, value)?;
        tx.commit().map_err(db_error)?;
        Ok(value.clone())
    }

    fn mutate_console_run(
        &self,
        owner: &ConsoleJournalOwner,
        run_id: &str,
        revision: u64,
        apply: impl FnOnce(&Connection, &mut ConsoleRunSnapshot) -> AppResult<()>,
    ) -> AppResult<ConsoleRunSnapshot> {
        owner.validate(self)?;
        if !valid_uuid(run_id) || revision >= MAX_REVISION {
            return Err(invalid());
        }
        let mut connection = self.connection.lock();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        require_current_schema(&tx)?;
        let mut run = read_run(&tx, run_id)?.ok_or_else(|| {
            AppError::new(
                ErrorCode::NotFound,
                "The reviewed console journal run is unavailable",
            )
        })?;
        if run.revision != revision {
            return Err(conflict());
        }
        if lifecycle::sealed(&tx, run_id)? {
            return Err(conflict());
        }
        apply(&tx, &mut run)?;
        run.revision += 1;
        validate_snapshot(&run)?;
        write_run(&tx, &run, revision)?;
        tx.commit().map_err(db_error)?;
        Ok(run)
    }

    pub(crate) fn console_step_intent(
        &self,
        owner: &ConsoleJournalOwner,
        run_id: &str,
        revision: u64,
        ordinal: u32,
        intended_at: u64,
    ) -> AppResult<ConsoleRunSnapshot> {
        if !valid_time(intended_at) {
            return Err(invalid());
        }
        self.mutate_console_run(owner, run_id, revision, |tx, run| {
            if run.state == ConsoleRunState::Terminal || run.observations.blocks_admission() || ordinal != run.steps.len() as u32 || ordinal >= run.metadata.step_count || run.steps.last().is_some_and(|s| !matches!(s.outcome, Some(ConsoleStepOutcome::Acknowledged(_)))) { return Err(conflict()); }
            let step = ConsoleStep { ordinal, intended_at, outcome: None, observations: ConsoleObservations::default() };
            tx.execute("INSERT INTO snowflake_console_steps_v5(run_id,ordinal,intended_at,outcome_json,observations_json) VALUES(?1,?2,?3,NULL,?4)", params![run_id,ordinal,sqlite_integer(intended_at)?,encode(&step.observations)?]).map_err(db_error)?;
            run.steps.push(step);
            run.state = ConsoleRunState::Running;
            Ok(())
        })
    }

    pub(crate) fn console_step_outcome(
        &self,
        owner: &ConsoleJournalOwner,
        run_id: &str,
        revision: u64,
        ordinal: u32,
        outcome: ConsoleStepOutcome,
        observations: ConsoleObservations,
    ) -> AppResult<ConsoleRunSnapshot> {
        validate_outcome(&outcome)?;
        self.mutate_console_run(owner, run_id, revision, |tx, run| {
            if run.state != ConsoleRunState::Running || ordinal as usize + 1 != run.steps.len() { return Err(conflict()); }
            let step = run.steps.get_mut(ordinal as usize).ok_or_else(conflict)?;
            if step.outcome.is_some() { return Err(conflict()); }
            step.outcome = Some(outcome);
            step.observations.absorb(&observations);
            let changed = tx.execute("UPDATE snowflake_console_steps_v5 SET outcome_json=?1,observations_json=?2 WHERE run_id=?3 AND ordinal=?4 AND outcome_json IS NULL", params![encode(&step.outcome)?,encode(&step.observations)?,run_id,ordinal]).map_err(db_error)?;
            if changed != 1 { return Err(conflict()); }
            run.observations.absorb(&observations);
            Ok(())
        })
    }

    pub(crate) fn console_observe(
        &self,
        owner: &ConsoleJournalOwner,
        run_id: &str,
        revision: u64,
        ordinal: Option<u32>,
        observations: ConsoleObservations,
    ) -> AppResult<ConsoleRunSnapshot> {
        self.mutate_console_run(owner, run_id, revision, |tx, run| {
            if let Some(ordinal) = ordinal {
                let step = run.steps.get_mut(ordinal as usize).ok_or_else(conflict)?;
                step.observations.apply(&observations)?;
                tx.execute("UPDATE snowflake_console_steps_v5 SET observations_json=?1 WHERE run_id=?2 AND ordinal=?3", params![encode(&step.observations)?,run_id,ordinal]).map_err(db_error)?;
                run.observations.absorb(&observations);
            } else {
                run.observations.apply(&observations)?;
            }
            Ok(())
        })
    }

    pub(crate) fn console_finish(
        &self,
        owner: &ConsoleJournalOwner,
        run_id: &str,
        revision: u64,
        reason: ConsoleStopReason,
        finished_at: u64,
    ) -> AppResult<ConsoleRunSnapshot> {
        if !valid_time(finished_at) {
            return Err(invalid());
        }
        self.mutate_console_run(owner, run_id, revision, |tx, run| {
            let legacy: bool = tx.query_row(
                "SELECT legacy=1 AND ticket_id IS NULL FROM console_lifecycle_v5 WHERE run_id=?1",
                [run_id], |row| row.get(0)
            ).map_err(db_error)?;
            if !legacy || run.state == ConsoleRunState::Terminal {
                return Err(conflict());
            }
            // Interruption is reserved for the owner-acquisition recovery path.
            if reason == ConsoleStopReason::Interrupted {
                return Err(conflict());
            }
            validate_finish(run, reason, &run.observations)?;
            run.terminal = Some(ConsoleTerminal {
                reason,
                finished_at,
                accounting: accounting(run),
                observations: run.observations.clone(),
            });
            run.state = ConsoleRunState::Terminal;
            Ok(())
        })
    }

    /// Claim before preparing any run. A second store may read evidence but
    /// cannot seal or mutate the current coordinator's journal.
    pub(crate) fn claim_console_journal_owner(&self) -> AppResult<ConsoleJournalOwner> {
        let owner = claim_migration_lease(&self.root)?;
        interrupt_records(&mut self.connection.lock())?;
        Ok(owner)
    }

    /// Browsing changes need exclusivity, but must not recover unrelated runs.
    pub(crate) fn claim_console_archive_owner(&self) -> AppResult<ConsoleJournalOwner> {
        let owner = claim_migration_lease(&self.root)?;
        owner.validate(self)?;
        let connection = self.connection.lock();
        require_current_schema(&connection)?;
        validate_schema(&connection)?;
        Ok(owner)
    }

    /// Recovery only while the caller retains the same exclusive owner lease.
    pub(crate) fn console_interrupt_incomplete_runs(
        &self,
        owner: &ConsoleJournalOwner,
    ) -> AppResult<usize> {
        owner.validate(self)?;
        interrupt_records(&mut self.connection.lock())
    }
}

/// Migration excludes live coordinators without recovering their original work.
pub(super) fn claim_migration_lease(root: &std::path::Path) -> AppResult<ConsoleJournalOwner> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options
        .open(root.join("snowflake-console.lock"))
        .map_err(|_| {
            AppError::internal("Could not acquire the local reviewed console journal lease")
        })?;
    let owner = ConsoleJournalOwner {
        file,
        root: root.to_owned(),
    };
    owner.validate_root(root)?;
    owner.file.try_lock().map_err(|failure| match failure {
        std::fs::TryLockError::WouldBlock => owner_error(),
        std::fs::TryLockError::Error(_) => {
            AppError::internal("Could not lock the local reviewed console journal")
        }
    })?;
    owner.validate_root(root)?;
    Ok(owner)
}

/// Constant-memory keyset traversal; no lifetime-sized ID vector is retained.
fn visit_ids(
    connection: &Connection,
    refusals: bool,
    incomplete_only: bool,
    mut visit: impl FnMut(&str) -> AppResult<()>,
) -> AppResult<usize> {
    let table = if refusals {
        "snowflake_console_rejections_v5"
    } else {
        "snowflake_console_runs_v5"
    };
    let column = if refusals {
        "client_request_id"
    } else {
        "run_id"
    };
    // The empty keyset sentinel must never hide corrupt empty/NULL IDs. Reject
    // them before traversal, including records outside an incomplete-only scan.
    let malformed: bool = connection.query_row(
        &read_sql(connection, &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE typeof({column})!='text' OR length(CAST({column} AS BLOB))!=36)"))?,
        [],
        |row| row.get(0),
    ).map_err(db_error)?;
    if malformed {
        return Err(invalid());
    }
    let predicate = if incomplete_only {
        " AND state!='terminal'"
    } else {
        ""
    };
    let sql=read_sql(connection,&format!("SELECT CASE WHEN length(CAST({column} AS BLOB))=36 THEN {column} ELSE '' END FROM {table} WHERE {column}>?1{predicate} ORDER BY {column} LIMIT 1"))?.into_owned();
    let mut last = String::new();
    let mut count = 0usize;
    loop {
        let next: Option<String> = connection
            .query_row(&sql, [&last], |row| row.get(0))
            .optional()
            .map_err(db_error)?;
        let Some(id) = next else { break };
        if !valid_uuid(&id)
            || count
                >= if refusals {
                    MAX_REJECTIONS as usize
                } else {
                    MAX_RUNS as usize
                }
        {
            return Err(invalid());
        }
        visit(&id)?;
        count += 1;
        last = id;
    }
    Ok(count)
}

fn interrupt_in_transaction(connection: &Connection) -> AppResult<usize> {
    require_current_schema(connection)?;
    visit_ids(connection, false, true, |id| {
        let mut run = read_run(connection, id)?.ok_or_else(invalid)?;
        let revision = run.revision;
        if revision >= MAX_REVISION {
            return Err(invalid());
        }
        if let Some(step) = run.steps.last_mut().filter(|step| step.outcome.is_none()) {
            step.outcome = Some(ConsoleStepOutcome::Unknown(ConsoleUnknown {
                statement: None,
            }));
            let changed = connection.execute("UPDATE snowflake_console_steps_v5 SET outcome_json=?1 WHERE run_id=?2 AND ordinal=?3 AND outcome_json IS NULL", params![encode(&step.outcome)?,id,step.ordinal]).map_err(db_error)?;
            if changed != 1 {
                return Err(conflict());
            }
        }
        run.observations.retired = true;
        run.observations
            .first_error
            .get_or_insert(ConsoleErrorCode::Interrupted);
        run.terminal = Some(ConsoleTerminal {
            reason: ConsoleStopReason::Interrupted,
            finished_at: crate::domain::now_ms(),
            accounting: accounting(&run),
            observations: run.observations.clone(),
        });
        run.state = ConsoleRunState::Terminal;
        run.revision += 1;
        validate_snapshot(&run)?;
        write_run(connection, &run, revision)?;
        Ok(())
    })
}

/// Parent recovery hook. The caller must hold the live store's owner lease.
pub(super) fn interrupt_records(connection: &mut Connection) -> AppResult<usize> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    require_current_schema(&tx)?;
    let count = interrupt_in_transaction(&tx)?;
    lifecycle::recover(&tx, crate::domain::HistoryProvenance::Interrupted)?;
    tx.commit().map_err(db_error)?;
    Ok(count)
}

fn observations_cover(current: &ConsoleObservations, prior: &ConsoleObservations) -> bool {
    let mut union = current.clone();
    union.absorb(prior);
    union == *current && (prior.first_error.is_none() || prior.first_error == current.first_error)
}

fn evidence_covers(current: &ConsoleRunSnapshot, prior: &ConsoleRunSnapshot) -> bool {
    current.metadata == prior.metadata
        && current.revision >= prior.revision
        && observations_cover(&current.observations, &prior.observations)
        && current.steps.len() >= prior.steps.len()
        && prior.steps.iter().zip(&current.steps).all(|(old, new)| {
            old.ordinal == new.ordinal
                && old.intended_at == new.intended_at
                && (old.outcome.is_none() || old.outcome == new.outcome)
                && observations_cover(&new.observations, &old.observations)
        })
        && (prior.terminal.is_none() || current.terminal == prior.terminal)
}

fn insert_snapshot(connection: &Connection, run: &ConsoleRunSnapshot) -> AppResult<()> {
    require_current_schema(connection)?;
    connection.execute("INSERT INTO snowflake_console_runs_v5(run_id,client_request_id,session_id,connection_id,plan_digest,step_count,created_at,revision,state,observations_json,terminal_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![run.metadata.run_id,run.metadata.client_request_id,run.metadata.session_id,run.metadata.connection_id,run.metadata.plan_digest,run.metadata.step_count,sqlite_integer(run.metadata.created_at)?,sqlite_integer(run.revision)?,state_name(run.state),encode(&run.observations)?,run.terminal.as_ref().map(encode).transpose()?]).map_err(db_error)?;
    for step in &run.steps {
        connection.execute("INSERT INTO snowflake_console_steps_v5(run_id,ordinal,intended_at,outcome_json,observations_json) VALUES(?1,?2,?3,?4,?5)",
            params![run.metadata.run_id,step.ordinal,sqlite_integer(step.intended_at)?,step.outcome.as_ref().map(encode).transpose()?,encode(&step.observations)?]).map_err(db_error)?;
    }
    Ok(())
}

/// Preserve live evidence and deduplication tombstones when restoring settings.
/// Conflicting/newer backup evidence fails before the caller replaces live data.
/// The caller holds the live owner lease and a consistent current connection.
pub(super) fn merge_for_restore(current: &Connection, pinned: &mut Connection) -> AppResult<()> {
    require_current_schema(current)?;
    require_current_schema(pinned)?;
    validate_records(current)?;
    validate_records(pinned)?;
    let tx = pinned
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    visit_ids(current, false, false, |id| {
        let live = read_run(current, id)?.ok_or_else(invalid)?;
        if read_rejection(&tx, &live.metadata.client_request_id)?.is_some() {
            return Err(conflict());
        }
        let prior_client: Option<String> = tx
            .query_row(
                "SELECT CASE WHEN length(CAST(run_id AS BLOB))=36 THEN run_id ELSE '' END FROM snowflake_console_runs_v5 WHERE client_request_id=?1",
                [&live.metadata.client_request_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if prior_client.as_ref().is_some_and(|other| other != id) {
            return Err(conflict());
        }
        if let Some(prior) = read_run(&tx, id)? {
            if !evidence_covers(&live, &prior) {
                return Err(conflict());
            }
            tx.execute(
                "DELETE FROM snowflake_console_steps_v5 WHERE run_id=?1",
                [id],
            )
            .map_err(db_error)?;
            tx.execute(
                "DELETE FROM snowflake_console_runs_v5 WHERE run_id=?1",
                [id],
            )
            .map_err(db_error)?;
        }
        insert_snapshot(&tx, &live)?;
        retention::copy_browsing(current, &tx, &live.metadata.client_request_id)?;
        Ok(())
    })?;
    visit_ids(current, true, false, |client| {
        let live = read_rejection(current, client)?.ok_or_else(invalid)?;
        let positive: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM snowflake_console_runs_v5 WHERE client_request_id=?1)",
                [&client],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if positive {
            return Err(conflict());
        }
        if let Some(prior) = read_rejection(&tx, client)? {
            if prior != live {
                return Err(conflict());
            }
        } else {
            insert_rejection(&tx, &live)?;
        }
        retention::copy_browsing(current, &tx, client)?;
        Ok(())
    })?;
    // This checks the bounded union before recovery writes or live replacement.
    visit_ids(&tx, false, false, |_| Ok(()))?;
    visit_ids(&tx, true, false, |_| Ok(()))?;
    super::history::lifecycle::merge_for_restore(current, &tx)?;
    interrupt_in_transaction(&tx)?;
    lifecycle::recover(&tx, crate::domain::HistoryProvenance::RestoreRecovered)?;
    retention::restore_epoch(&tx)?;
    validate_records(&tx)?;
    tx.commit().map_err(db_error)
}

pub(super) fn validate_records(connection: &Connection) -> AppResult<()> {
    visit_ids(connection, false, false, |id| {
        read_run(connection, id)?.ok_or_else(invalid)?;
        Ok(())
    })?;
    let orphan: bool = connection.query_row(&read_sql(connection, "SELECT EXISTS(SELECT 1 FROM snowflake_console_steps_v5 s LEFT JOIN snowflake_console_runs_v5 r ON r.run_id=s.run_id WHERE r.run_id IS NULL)")?, [], |r| r.get(0)).map_err(db_error)?;
    if orphan {
        return Err(invalid());
    }
    if super::schema_version(connection)? >= 3 {
        visit_ids(connection, true, false, |client| {
            read_rejection(connection, client)?.ok_or_else(invalid)?;
            Ok(())
        })?;
    }
    if super::schema_version(connection)? >= 4 {
        retention::validate_records(connection)?;
    }
    if super::schema_version(connection)? == 5 {
        super::history::validate_records(connection)?;
        visit_ids(connection, false, false, |id| {
            lifecycle::read(connection, id)?.ok_or_else(invalid)?;
            Ok(())
        })?;
    }
    Ok(())
}

pub(super) fn validate_schema(connection: &Connection) -> AppResult<()> {
    for (table, expected) in [
        (
            "snowflake_console_runs_v5",
            vec![
                ("run_id", "TEXT", 1, 1),
                ("client_request_id", "TEXT", 1, 0),
                ("session_id", "TEXT", 1, 0),
                ("connection_id", "TEXT", 1, 0),
                ("plan_digest", "TEXT", 1, 0),
                ("step_count", "INTEGER", 1, 0),
                ("created_at", "INTEGER", 1, 0),
                ("revision", "INTEGER", 1, 0),
                ("state", "TEXT", 1, 0),
                ("observations_json", "TEXT", 1, 0),
                ("terminal_json", "TEXT", 0, 0),
            ],
        ),
        (
            "snowflake_console_steps_v5",
            vec![
                ("run_id", "TEXT", 1, 1),
                ("ordinal", "INTEGER", 1, 2),
                ("intended_at", "INTEGER", 1, 0),
                ("outcome_json", "TEXT", 0, 0),
                ("observations_json", "TEXT", 1, 0),
            ],
        ),
    ] {
        let mut statement = connection
            .prepare(&read_sql(
                connection,
                &format!("PRAGMA table_info({table})"),
            )?)
            .map_err(db_error)?;
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?.to_ascii_uppercase(),
                    row.get::<_, u32>(3)?,
                    row.get::<_, u32>(5)?,
                ))
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let mut expected = expected
            .into_iter()
            .map(|(name, kind, not_null, key)| (name.to_string(), kind.to_string(), not_null, key))
            .collect::<Vec<_>>();
        if super::schema_version(connection)? >= 4 && table == "snowflake_console_runs_v5" {
            expected.extend([
                ("receipt_sequence".into(), "INTEGER".into(), 1, 0),
                ("archived_at".into(), "INTEGER".into(), 0, 0),
                ("browsing_revision".into(), "INTEGER".into(), 1, 0),
            ]);
        }
        if columns != expected {
            return Err(conflict());
        }
    }
    let unique_request: bool = connection.query_row(&read_sql(connection, "SELECT EXISTS(SELECT 1 FROM pragma_index_list('snowflake_console_runs_v5') i JOIN pragma_index_info(i.name) c WHERE i.\"unique\"=1 AND i.partial=0 AND c.name='client_request_id' AND (SELECT count(*) FROM pragma_index_info(i.name))=1)")?, [], |r| r.get(0)).map_err(db_error)?;
    let foreign_key: bool = connection.query_row(&read_sql(connection, "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_list('snowflake_console_steps_v5') WHERE \"table\"='snowflake_console_runs_v5' AND \"from\"='run_id' AND \"to\"='run_id' AND on_delete='NO ACTION' AND on_update='NO ACTION')")?, [], |r| r.get(0)).map_err(db_error)?;
    if !unique_request || !foreign_key {
        return Err(conflict());
    }
    if super::schema_version(connection)? >= 3 {
        let mut statement = connection
            .prepare(&read_sql(
                connection,
                "PRAGMA table_info(snowflake_console_rejections_v5)",
            )?)
            .map_err(db_error)?;
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?.to_ascii_uppercase(),
                    row.get::<_, u32>(3)?,
                    row.get::<_, u32>(5)?,
                ))
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let mut expected = vec![
            ("client_request_id", "TEXT", 1, 1),
            ("session_id", "TEXT", 1, 0),
            ("plan_digest", "TEXT", 1, 0),
            ("code", "TEXT", 1, 0),
            ("recorded_at", "INTEGER", 1, 0),
        ]
        .into_iter()
        .map(|(name, kind, not_null, key)| (name.to_owned(), kind.to_owned(), not_null, key))
        .collect::<Vec<_>>();
        if super::schema_version(connection)? >= 4 {
            expected.extend([
                ("receipt_sequence".into(), "INTEGER".into(), 1, 0),
                ("archived_at".into(), "INTEGER".into(), 0, 0),
                ("browsing_revision".into(), "INTEGER".into(), 1, 0),
            ]);
        }
        if columns != expected {
            return Err(conflict());
        }
        if super::schema_version(connection)? >= 4 {
            return retention::validate_schema(connection);
        }
        for (name, sql) in DECISION_TRIGGERS {
            let stored: Option<String> = connection
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
                    [name],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            if stored.as_deref() != Some(sql) {
                return Err(conflict());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "snowflake_console_tests.rs"]
mod tests;
