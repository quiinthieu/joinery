//! Bounded SQL-free original outcomes. Capture commits before projection.
use super::history_owner::OperationGate;
use super::{
    HistoryCompletion, HistoryOwner, HistoryTicket, Persistence, db_error, history::lifecycle,
    schema_version,
};
use crate::domain::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Write},
    sync::Arc,
};

pub(crate) const MAX_OUTCOME_BYTES: usize = 32 * 1024;
const MAX_MARKERS: i64 = 65_536;
const MAX_POOL_ROOT: i64 = 128;
const MAX_POOL_OWNER: i64 = 32;
pub(super) const TABLE: &str = "outcome_operations_v7";
pub(super) const SCHEMA: &str = "CREATE TABLE outcome_operations_v7(operation_id TEXT PRIMARY KEY NOT NULL,owner_nonce TEXT NOT NULL,pool TEXT NOT NULL,phase TEXT NOT NULL,reservation_digest TEXT NOT NULL,capture_digest TEXT,descriptor_json TEXT,payload_json TEXT,ticket_id TEXT UNIQUE,terminal_kind TEXT,boundary_session TEXT,boundary_action TEXT,boundary_outcome TEXT,boundary_retired INTEGER)";
pub(super) const BINDING_TABLE: &str = "outcome_bindings_v7";
pub(super) const BINDING_SCHEMA: &str = "CREATE TABLE outcome_bindings_v7(operation_id TEXT PRIMARY KEY NOT NULL,owner_nonce TEXT NOT NULL,ticket_id TEXT UNIQUE)";

fn invalid() -> AppError {
    AppError::invalid("Original outcome metadata is invalid")
}
fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Original outcome identity or recording phase conflicts; preserve the local store and retry after pending recording settles",
    )
}
fn full() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Local outcome recording capacity is full; allow pending recording to settle before starting another action",
    )
}
fn canonical_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}
fn hex_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn current(c: &Connection) -> AppResult<()> {
    if schema_version(c)? == super::SCHEMA_VERSION {
        Ok(())
    } else {
        Err(conflict())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OutcomePool {
    Ordinary,
    EmergencyRollback,
}
impl OutcomePool {
    fn text(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::EmergencyRollback => "emergencyRollback",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum OutcomeKind {
    Execute {
        planned: usize,
    },
    SnowflakeDml {
        kind: SnowflakeDmlKind,
    },
    SnowflakeDdl {
        kind: SnowflakeDdlKind,
        qualified_target: String,
    },
    Boundary {
        action: ExplicitTransactionAction,
    },
}
// Serde's derived internally tagged decoder buffers arbitrary field content.
// Decode through a fixed struct instead, so a malformed planned-count array
// cannot create an unbounded intermediate tree before its type is rejected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClosedOutcomeKind {
    #[serde(rename = "type")]
    tag: String,
    planned: Option<usize>,
    kind: Option<String>,
    qualified_target: Option<String>,
    action: Option<ExplicitTransactionAction>,
}
impl<'de> Deserialize<'de> for OutcomeKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = ClosedOutcomeKind::deserialize(deserializer)?;
        let bad = || <D::Error as serde::de::Error>::custom("Invalid original outcome kind");
        match (
            value.tag.as_str(),
            value.planned,
            value.kind,
            value.qualified_target,
            value.action,
        ) {
            ("execute", Some(planned), None, None, None) => Ok(Self::Execute { planned }),
            ("snowflakeDml", None, Some(kind), None, None) => {
                let kind = match kind.as_str() {
                    "insert" => SnowflakeDmlKind::Insert,
                    "update" => SnowflakeDmlKind::Update,
                    "delete" => SnowflakeDmlKind::Delete,
                    _ => return Err(bad()),
                };
                Ok(Self::SnowflakeDml { kind })
            }
            ("snowflakeDdl", None, Some(kind), Some(qualified_target), None) => {
                let kind = match kind.as_str() {
                    "createTable" => SnowflakeDdlKind::CreateTable,
                    "addColumn" => SnowflakeDdlKind::AddColumn,
                    "dropTable" => SnowflakeDdlKind::DropTable,
                    _ => return Err(bad()),
                };
                Ok(Self::SnowflakeDdl {
                    kind,
                    qualified_target,
                })
            }
            ("boundary", None, None, None, Some(action)) => Ok(Self::Boundary { action }),
            _ => Err(bad()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OutcomeDescriptor {
    pub operation_id: String,
    pub connection_id: String,
    pub session_id: String,
    pub started_at: u64,
    pub digest: String,
    pub kind: OutcomeKind,
}
impl OutcomeDescriptor {
    fn validate(&self) -> AppResult<()> {
        if !canonical_uuid(&self.operation_id)
            || !canonical_uuid(&self.session_id)
            || self.connection_id.is_empty()
            || self.connection_id.len() > 256
            || self.connection_id.contains('\0')
            || self.started_at > 9_007_199_254_740_991
            || !hex_digest(&self.digest)
        {
            return Err(invalid());
        }
        if let OutcomeKind::SnowflakeDdl {
            qualified_target, ..
        } = &self.kind
        {
            validate_target(qualified_target)?;
        }
        Ok(())
    }
    fn action(&self) -> &'static str {
        match self.kind {
            OutcomeKind::Execute { .. } => "execute",
            OutcomeKind::SnowflakeDml { .. } => "snowflake-dml",
            OutcomeKind::SnowflakeDdl { .. } => "snowflake-ddl",
            OutcomeKind::Boundary { .. } => "transaction-boundary",
        }
    }
    fn rollback(&self) -> bool {
        matches!(
            self.kind,
            OutcomeKind::Boundary {
                action: ExplicitTransactionAction::Rollback
            }
        )
    }
}
fn validate_target(target: &str) -> AppResult<()> {
    if target.len() > 4096 {
        return Err(invalid());
    }
    let mut chars = target.chars().peekable();
    for component in 0..3 {
        if chars.next() != Some('"') {
            return Err(invalid());
        }
        let mut count = 0;
        let mut nonspace = false;
        loop {
            let character = chars.next().ok_or_else(invalid)?;
            if character == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                } else {
                    break;
                }
            }
            if character.is_control()
                || matches!(character,'\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{2028}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
            {
                return Err(invalid());
            }
            count += 1;
            nonspace |= !character.is_whitespace();
            if count > 255 {
                return Err(invalid());
            }
        }
        if count == 0 || !nonspace || (component < 2 && chars.next() != Some('.')) {
            return Err(invalid());
        }
    }
    if chars.next().is_some() {
        return Err(invalid());
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TicketIdentity {
    id: String,
    lineage: String,
    era: String,
    generation: String,
}
impl TicketIdentity {
    fn from_ticket(t: &HistoryTicket) -> Self {
        Self {
            id: t.id.clone(),
            lineage: t.lineage.clone(),
            era: t.era.clone(),
            generation: t.generation.clone(),
        }
    }
    fn ticket(&self) -> HistoryTicket {
        HistoryTicket {
            id: self.id.clone(),
            lineage: self.lineage.clone(),
            era: self.era.clone(),
            generation: self.generation.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservationData {
    descriptor: OutcomeDescriptor,
    owner_nonce: String,
    pool: OutcomePool,
    ticket: Option<TicketIdentity>,
}
#[derive(Clone, Debug)]
pub(crate) struct OutcomeReservation {
    data: ReservationData,
    digest: String,
    ticket: Option<HistoryTicket>,
}
impl OutcomeReservation {
    pub(crate) fn operation_id(&self) -> &str {
        &self.data.descriptor.operation_id
    }
    pub(crate) fn history_ticket(&self) -> Option<&HistoryTicket> {
        self.ticket_ref()
    }
    fn ticket_ref(&self) -> Option<&HistoryTicket> {
        self.ticket.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutcomePhase {
    Reserved,
    Captured,
    Settled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BoundaryEvidence {
    pub action: ExplicitTransactionAction,
    pub boundary_outcome: TransactionBoundaryOutcome,
    pub retired: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Payload {
    descriptor: OutcomeDescriptor,
    #[serde(default, deserialize_with = "deserialize_completion")]
    completion: Option<HistoryCompletion>,
    boundary: Option<BoundaryEvidence>,
    finished_at: u64,
}

// Fixed-field decoding rejects unknown keys without first constructing an
// arbitrary JSON tree. Even malformed input can allocate only bounded strings
// from the already capped encoded payload, rather than one Value per element.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClosedCompletion {
    provenance: HistoryProvenance,
    status: Option<RunStatus>,
    elapsed_ms: Option<u32>,
    execution_summary: Option<ClosedExecutionSummary>,
    snowflake_dml: Option<ClosedDmlEvidence>,
    snowflake_ddl: Option<ClosedDdlEvidence>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClosedExecutionSummary {
    planned: usize,
    attempted: usize,
    completed: usize,
    not_attempted: usize,
    stop_reason: Option<ExecutionStopReason>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClosedDmlEvidence {
    kind: SnowflakeDmlKind,
    statement_outcome: SnowflakeDmlOutcome,
    summary: ClosedDmlSummary,
    retired: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClosedDmlSummary {
    status: SnowflakeDmlSummaryStatus,
    affected_rows: Option<String>,
    multi_joined_rows: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClosedDdlEvidence {
    kind: SnowflakeDdlKind,
    qualified_target: String,
    statement_outcome: SnowflakeDdlOutcome,
    summary_status: SnowflakeDdlSummaryStatus,
    catalog_reconciliation_required: bool,
    retired: bool,
}
fn deserialize_completion<'de, D>(deserializer: D) -> Result<Option<HistoryCompletion>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(
        Option::<ClosedCompletion>::deserialize(deserializer)?.map(|c| HistoryCompletion {
            provenance: c.provenance,
            status: c.status,
            elapsed_ms: c.elapsed_ms,
            execution_summary: c.execution_summary.map(|s| ExecutionSummary {
                planned: s.planned,
                attempted: s.attempted,
                completed: s.completed,
                not_attempted: s.not_attempted,
                stop_reason: s.stop_reason,
            }),
            snowflake_dml: c.snowflake_dml.map(|d| SnowflakeDmlEvidence {
                kind: d.kind,
                statement_outcome: d.statement_outcome,
                summary: SnowflakeDmlSummary {
                    status: d.summary.status,
                    affected_rows: d.summary.affected_rows,
                    multi_joined_rows: d.summary.multi_joined_rows,
                },
                retired: d.retired,
            }),
            snowflake_ddl: c.snowflake_ddl.map(|d| SnowflakeDdlEvidence {
                kind: d.kind,
                qualified_target: d.qualified_target,
                statement_outcome: d.statement_outcome,
                summary_status: d.summary_status,
                catalog_reconciliation_required: d.catalog_reconciliation_required,
                retired: d.retired,
            }),
        }),
    )
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PayloadRef<'a> {
    descriptor: &'a OutcomeDescriptor,
    completion: Option<&'a HistoryCompletion>,
    boundary: Option<&'a BoundaryEvidence>,
    finished_at: u64,
}

/// Moving the Vec into Arc does not copy its allocation. Runtime supplies its
/// preallocated safety buffer, and recovers it only after all clones are gone.
#[derive(Clone, Debug)]
pub(crate) struct OriginalOutcome {
    bytes: Arc<Vec<u8>>,
}
struct BudgetWriter<'a>(&'a mut Vec<u8>);
impl Write for BudgetWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > MAX_OUTCOME_BYTES || n > self.0.capacity())
        {
            return Err(io::Error::other(
                "Original outcome exceeds its reserved buffer",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encoded<T: Serialize>(value: &T) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::with_capacity(MAX_OUTCOME_BYTES);
    serde_json::to_writer(BudgetWriter(&mut bytes), value).map_err(|_| invalid())?;
    Ok(bytes)
}
impl OriginalOutcome {
    #[cfg(test)]
    pub(crate) fn history(
        descriptor: &OutcomeDescriptor,
        completion: &HistoryCompletion,
        finished_at: u64,
    ) -> AppResult<Self> {
        Self::history_in(
            descriptor,
            completion,
            finished_at,
            Vec::with_capacity(MAX_OUTCOME_BYTES),
        )
    }
    pub(crate) fn history_in(
        descriptor: &OutcomeDescriptor,
        completion: &HistoryCompletion,
        finished_at: u64,
        buffer: Vec<u8>,
    ) -> AppResult<Self> {
        Self::from_borrowed(
            PayloadRef {
                descriptor,
                completion: Some(completion),
                boundary: None,
                finished_at,
            },
            buffer,
        )
    }
    #[cfg(test)]
    pub(crate) fn boundary(
        descriptor: &OutcomeDescriptor,
        evidence: &BoundaryEvidence,
        finished_at: u64,
    ) -> AppResult<Self> {
        Self::boundary_in(
            descriptor,
            evidence,
            finished_at,
            Vec::with_capacity(MAX_OUTCOME_BYTES),
        )
    }
    pub(crate) fn boundary_in(
        descriptor: &OutcomeDescriptor,
        evidence: &BoundaryEvidence,
        finished_at: u64,
        buffer: Vec<u8>,
    ) -> AppResult<Self> {
        Self::from_borrowed(
            PayloadRef {
                descriptor,
                completion: None,
                boundary: Some(evidence),
                finished_at,
            },
            buffer,
        )
    }
    fn from_borrowed(payload: PayloadRef<'_>, mut buffer: Vec<u8>) -> AppResult<Self> {
        validate_parts(
            payload.descriptor,
            payload.completion,
            payload.boundary,
            payload.finished_at,
        )?;
        buffer.clear();
        serde_json::to_writer(BudgetWriter(&mut buffer), &payload).map_err(|_| invalid())?;
        Ok(Self {
            bytes: Arc::new(buffer),
        })
    }
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn try_reclaim_buffer(self) -> Result<Vec<u8>, Self> {
        match Arc::try_unwrap(self.bytes) {
            Ok(mut b) => {
                b.clear();
                Ok(b)
            }
            Err(bytes) => Err(Self { bytes }),
        }
    }
    fn payload(&self) -> AppResult<Payload> {
        decode_payload(&self.bytes)
    }
}

fn decode_payload(bytes: &[u8]) -> AppResult<Payload> {
    if bytes.len() > MAX_OUTCOME_BYTES {
        return Err(invalid());
    }
    let payload: Payload = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    validate_payload(&payload)?;
    // Exact canonical encoding makes digests independent of JSON key ordering.
    if encoded(&payload)? != bytes {
        return Err(invalid());
    }
    Ok(payload)
}
fn valid_count(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|s| {
        !s.is_empty()
            && s.len() <= 38
            && s.bytes().all(|b| b.is_ascii_digit())
            && (s == "0" || !s.starts_with('0'))
    })
}
fn validate_payload(p: &Payload) -> AppResult<()> {
    validate_parts(
        &p.descriptor,
        p.completion.as_ref(),
        p.boundary.as_ref(),
        p.finished_at,
    )
}
fn validate_parts(
    descriptor: &OutcomeDescriptor,
    completion: Option<&HistoryCompletion>,
    boundary: Option<&BoundaryEvidence>,
    finished_at: u64,
) -> AppResult<()> {
    descriptor.validate()?;
    if finished_at > 9_007_199_254_740_991 {
        return Err(invalid());
    }
    match (&descriptor.kind, completion, boundary) {
        (OutcomeKind::Boundary { action }, None, Some(b)) if *action == b.action => {}
        (kind, Some(c), None) if !matches!(kind, OutcomeKind::Boundary { .. }) => {
            if c.provenance != HistoryProvenance::Recorded
                || (c.provenance == HistoryProvenance::Recorded && c.status.is_none())
                || (c.status.is_none() && c.elapsed_ms.is_some())
            {
                return Err(invalid());
            }
            if let Some(s) = &c.execution_summary {
                let OutcomeKind::Execute { planned } = kind else {
                    return Err(invalid());
                };
                if s.planned != *planned
                    || s.completed > s.attempted
                    || s.attempted > s.planned
                    || s.not_attempted != s.planned - s.attempted
                {
                    return Err(invalid());
                }
            }
            match kind {
                OutcomeKind::Execute { .. }
                    if c.snowflake_dml.is_none() && c.snowflake_ddl.is_none() => {}
                OutcomeKind::SnowflakeDml { kind } if c.snowflake_ddl.is_none() => {
                    if let Some(d) = &c.snowflake_dml {
                        if d.kind != *kind
                            || !valid_count(&d.summary.affected_rows)
                            || !valid_count(&d.summary.multi_joined_rows)
                            || (d.summary.status == SnowflakeDmlSummaryStatus::Validated
                                && d.summary.affected_rows.is_none())
                            || (d.summary.status == SnowflakeDmlSummaryStatus::Unavailable
                                && (d.summary.affected_rows.is_some()
                                    || d.summary.multi_joined_rows.is_some()))
                            || (d.kind != SnowflakeDmlKind::Update
                                && d.summary.multi_joined_rows.is_some())
                        {
                            return Err(invalid());
                        }
                    } else if c.provenance == HistoryProvenance::Recorded {
                        return Err(invalid());
                    }
                }
                OutcomeKind::SnowflakeDdl {
                    kind,
                    qualified_target,
                } if c.snowflake_dml.is_none() => {
                    if let Some(d) = &c.snowflake_ddl {
                        if d.kind != *kind || d.qualified_target != *qualified_target {
                            return Err(invalid());
                        }
                    } else if c.provenance == HistoryProvenance::Recorded {
                        return Err(invalid());
                    }
                }
                _ => return Err(invalid()),
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

fn started(d: &OutcomeDescriptor) -> AuditEntry {
    AuditEntry {
        id: format!("{}:started", d.operation_id),
        occurred_at: d.started_at,
        connection_id: d.connection_id.clone(),
        action: d.action().into(),
        digest: d.digest.clone(),
        outcome: "started".into(),
    }
}
fn finished(p: &Payload) -> AuditEntry {
    let outcome = if let Some(b) = &p.boundary {
        match b.boundary_outcome {
            TransactionBoundaryOutcome::Acknowledged => "acknowledged".into(),
            TransactionBoundaryOutcome::Rejected => "rejected".into(),
            TransactionBoundaryOutcome::Unknown => "unknown".into(),
        }
    } else {
        let c = p.completion.as_ref().expect("validated completion");
        match &p.descriptor.kind {
            OutcomeKind::SnowflakeDml { .. } => {
                c.snowflake_dml.as_ref().map_or("interrupted".into(), |d| {
                    match d.statement_outcome {
                        SnowflakeDmlOutcome::Succeeded => "succeeded".into(),
                        SnowflakeDmlOutcome::Rejected => "rejected".into(),
                        SnowflakeDmlOutcome::Unknown => "unknown".into(),
                    }
                })
            }
            OutcomeKind::SnowflakeDdl { .. } => {
                c.snowflake_ddl.as_ref().map_or("interrupted".into(), |d| {
                    match d.statement_outcome {
                        SnowflakeDdlOutcome::Succeeded => "succeeded".into(),
                        SnowflakeDdlOutcome::Rejected => "rejected".into(),
                        SnowflakeDdlOutcome::Unknown => "unknown".into(),
                    }
                })
            }
            _ => c.status.map_or("interrupted".into(), |s| format!("{s:?}")),
        }
    };
    AuditEntry {
        id: format!("{}:finished", p.descriptor.operation_id),
        occurred_at: p.finished_at,
        connection_id: p.descriptor.connection_id.clone(),
        action: p.descriptor.action().into(),
        digest: p.descriptor.digest.clone(),
        outcome,
    }
}
pub(super) fn audit_once(c: &Connection, a: &AuditEntry) -> AppResult<()> {
    let timestamp = i64::try_from(a.occurred_at).map_err(|_| invalid())?;
    c.execute("INSERT INTO audit_v7(id,occurred_at,connection_id,action,digest,outcome) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO NOTHING",params![a.id,timestamp,a.connection_id,a.action,a.digest,a.outcome]).map_err(db_error)?;
    let old: (i64, String, String, String, String) = c
        .query_row(
            "SELECT occurred_at,CASE WHEN length(CAST(connection_id AS BLOB))<=256 THEN connection_id ELSE '' END,CASE WHEN length(CAST(action AS BLOB))<=32 THEN action ELSE '' END,CASE WHEN length(CAST(digest AS BLOB))<=64 THEN digest ELSE '' END,CASE WHEN length(CAST(outcome AS BLOB))<=32 THEN outcome ELSE '' END FROM audit_v7 WHERE id=?1",
            [&a.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(db_error)?;
    if old
        != (
            timestamp,
            a.connection_id.clone(),
            a.action.clone(),
            a.digest.clone(),
            a.outcome.clone(),
        )
    {
        return Err(conflict());
    }
    Ok(())
}

#[derive(Debug)]
struct Row {
    id: String,
    owner: String,
    pool: String,
    phase: String,
    reservation_digest: String,
    capture_digest: Option<String>,
    descriptor: Option<String>,
    payload: Option<String>,
    ticket: Option<String>,
    terminal: Option<String>,
    session: Option<String>,
    action: Option<String>,
    outcome: Option<String>,
    retired: Option<i64>,
}
fn read_row(c: &Connection, id: &str) -> AppResult<Option<Row>> {
    c.query_row("SELECT
        CASE WHEN length(CAST(operation_id AS BLOB))<=36 THEN operation_id ELSE '' END,
        CASE WHEN length(CAST(owner_nonce AS BLOB))<=36 THEN owner_nonce ELSE '' END,
        CASE WHEN length(CAST(pool AS BLOB))<=17 THEN pool ELSE '' END,
        CASE WHEN length(CAST(phase AS BLOB))<=8 THEN phase ELSE '' END,
        CASE WHEN length(CAST(reservation_digest AS BLOB))<=64 THEN reservation_digest ELSE '' END,
        CASE WHEN capture_digest IS NULL OR length(CAST(capture_digest AS BLOB))<=64 THEN capture_digest ELSE '' END,
        CASE WHEN descriptor_json IS NULL OR length(CAST(descriptor_json AS BLOB))<=32768 THEN descriptor_json ELSE '' END,
        CASE WHEN payload_json IS NULL OR length(CAST(payload_json AS BLOB))<=32768 THEN payload_json ELSE '' END,
        CASE WHEN ticket_id IS NULL OR length(CAST(ticket_id AS BLOB))<=36 THEN ticket_id ELSE '' END,
        CASE WHEN terminal_kind IS NULL OR length(CAST(terminal_kind AS BLOB))<=10 THEN terminal_kind ELSE '' END,
        CASE WHEN boundary_session IS NULL OR length(CAST(boundary_session AS BLOB))<=36 THEN boundary_session ELSE '' END,
        CASE WHEN boundary_action IS NULL OR length(CAST(boundary_action AS BLOB))<=32 THEN boundary_action ELSE '' END,
        CASE WHEN boundary_outcome IS NULL OR length(CAST(boundary_outcome AS BLOB))<=32 THEN boundary_outcome ELSE '' END,
        boundary_retired FROM outcome_operations_v7 WHERE operation_id=?1",[id],|r|Ok(Row {id:r.get(0)?,owner:r.get(1)?,pool:r.get(2)?,phase:r.get(3)?,reservation_digest:r.get(4)?,capture_digest:r.get(5)?,descriptor:r.get(6)?,payload:r.get(7)?,ticket:r.get(8)?,terminal:r.get(9)?,session:r.get(10)?,action:r.get(11)?,outcome:r.get(12)?,retired:r.get(13)?})).optional().map_err(db_error)
}
fn reservation_data(row: &Row) -> AppResult<ReservationData> {
    let raw = row.descriptor.as_ref().ok_or_else(invalid)?;
    let data: ReservationData = serde_json::from_str(raw).map_err(|_| invalid())?;
    data.descriptor.validate()?;
    if !canonical_uuid(&data.owner_nonce)
        || data.owner_nonce != row.owner
        || data.descriptor.operation_id != row.id
        || data.pool.text() != row.pool
        || hash(raw.as_bytes()) != row.reservation_digest
        || encoded(&data)? != raw.as_bytes()
        || (data.pool == OutcomePool::EmergencyRollback && !data.descriptor.rollback())
    {
        return Err(invalid());
    }
    if let Some(t) = &data.ticket {
        if t.id != row.id
            || ![&t.lineage, &t.era, &t.generation]
                .iter()
                .all(|s| canonical_uuid(s))
            || row.ticket.as_deref() != Some(t.id.as_str())
            || matches!(data.descriptor.kind, OutcomeKind::Boundary { .. })
        {
            return Err(invalid());
        }
    } else if row.ticket.is_some() || !matches!(data.descriptor.kind, OutcomeKind::Boundary { .. })
    {
        return Err(invalid());
    }
    Ok(data)
}
fn phase(row: &Row) -> AppResult<OutcomePhase> {
    if !canonical_uuid(&row.id)
        || !canonical_uuid(&row.owner)
        || !hex_digest(&row.reservation_digest)
        || !["ordinary", "emergencyRollback"].contains(&row.pool.as_str())
    {
        return Err(invalid());
    }
    match row.phase.as_str() {
        "reserved"
            if row.capture_digest.is_none()
                && row.payload.is_none()
                && row.terminal.is_none()
                && row.session.is_none()
                && row.action.is_none()
                && row.outcome.is_none()
                && row.retired.is_none() =>
        {
            reservation_data(row)?;
            Ok(OutcomePhase::Reserved)
        }
        "captured"
            if row.terminal.is_none()
                && row.session.is_none()
                && row.action.is_none()
                && row.outcome.is_none()
                && row.retired.is_none() =>
        {
            let data = reservation_data(row)?;
            let raw = row.payload.as_ref().ok_or_else(invalid)?;
            let payload = decode_payload(raw.as_bytes())?;
            if payload.descriptor != data.descriptor
                || row.capture_digest.as_deref() != Some(hash(raw.as_bytes()).as_str())
            {
                return Err(invalid());
            }
            Ok(OutcomePhase::Captured)
        }
        "settled" if row.descriptor.is_none() && row.payload.is_none() && row.ticket.is_none() => {
            match row.terminal.as_deref() {
                Some("original") if row.capture_digest.as_ref().is_some_and(|s| hex_digest(s)) => {}
                Some("noOriginal") if row.capture_digest.is_none() => {}
                _ => return Err(invalid()),
            }
            let has_boundary = row.session.is_some();
            if has_boundary {
                if !row.session.as_ref().is_some_and(|s| canonical_uuid(s))
                    || row.action.is_none()
                    || row.outcome.is_none()
                    || !matches!(row.retired, Some(0 | 1))
                    || row.terminal.as_deref() != Some("original")
                {
                    return Err(invalid());
                }
                serde_json::from_str::<ExplicitTransactionAction>(
                    row.action.as_ref().ok_or_else(invalid)?,
                )
                .map_err(|_| invalid())?;
                serde_json::from_str::<TransactionBoundaryOutcome>(
                    row.outcome.as_ref().ok_or_else(invalid)?,
                )
                .map_err(|_| invalid())?;
            } else if row.action.is_some() || row.outcome.is_some() || row.retired.is_some() {
                return Err(invalid());
            }
            let logical = row.id.len()
                + row.owner.len()
                + row.pool.len()
                + row.phase.len()
                + row.reservation_digest.len()
                + row.capture_digest.as_ref().map_or(0, String::len)
                + row.terminal.as_ref().map_or(0, String::len)
                + row.session.as_ref().map_or(0, String::len)
                + row.action.as_ref().map_or(0, String::len)
                + row.outcome.as_ref().map_or(0, String::len)
                + 8;
            if logical > 512 {
                return Err(invalid());
            }
            Ok(OutcomePhase::Settled)
        }
        _ => Err(invalid()),
    }
}
fn reservation(row: &Row) -> AppResult<OutcomeReservation> {
    let data = reservation_data(row)?;
    let ticket = data.ticket.as_ref().map(TicketIdentity::ticket);
    Ok(OutcomeReservation {
        data,
        digest: row.reservation_digest.clone(),
        ticket,
    })
}
fn expected(c: &Connection, r: &OutcomeReservation) -> AppResult<Row> {
    let row = read_row(c, r.operation_id())?.ok_or_else(conflict)?;
    phase(&row)?;
    if row.owner != r.data.owner_nonce || row.reservation_digest != r.digest {
        return Err(conflict());
    }
    Ok(row)
}
fn bind_owned(c: &Connection, owner: &str, t: &HistoryTicket) -> AppResult<()> {
    let old: Option<String> = c
        .query_row(
            "SELECT CASE WHEN length(CAST(owner_nonce AS BLOB))<=36 THEN owner_nonce ELSE '' END FROM history_owners_v6 WHERE ticket_id=?1",
            [t.id()],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if old.as_ref().is_some_and(|n| n != owner) {
        return Err(conflict());
    }
    c.execute(
        "INSERT INTO history_owners_v6 VALUES(?1,?2) ON CONFLICT(ticket_id) DO NOTHING",
        params![t.id(), owner],
    )
    .map_err(db_error)?;
    Ok(())
}
fn verify_binding(c: &Connection, data: &ReservationData) -> AppResult<()> {
    let witness: Option<(String, Option<String>)> = c
        .query_row(
            "SELECT CASE WHEN length(CAST(owner_nonce AS BLOB))<=36 THEN owner_nonce ELSE '' END,CASE WHEN ticket_id IS NULL OR length(CAST(ticket_id AS BLOB))<=36 THEN ticket_id ELSE '' END FROM outcome_bindings_v7 WHERE operation_id=?1",
            [&data.descriptor.operation_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db_error)?;
    if witness
        != Some((
            data.owner_nonce.clone(),
            data.ticket.as_ref().map(|t| t.id.clone()),
        ))
    {
        return Err(conflict());
    }
    if let Some(identity) = &data.ticket {
        let t = identity.ticket();
        let (actual, _, _) = lifecycle::lookup(c, t.id())?.ok_or_else(conflict)?;
        let owner: Option<String> = c
            .query_row(
                "SELECT CASE WHEN length(CAST(owner_nonce AS BLOB))<=36 THEN owner_nonce ELSE '' END FROM history_owners_v6 WHERE ticket_id=?1",
                [t.id()],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if actual != t || owner.as_deref() != Some(data.owner_nonce.as_str()) {
            return Err(conflict());
        }
    }
    Ok(())
}
fn quota(c: &Connection, owner: &str, pool: OutcomePool) -> AppResult<()> {
    let (all,root,local):(i64,i64,i64)=c.query_row("SELECT count(*),coalesce(sum(CASE WHEN phase!='settled' AND pool=?1 THEN 1 ELSE 0 END),0),coalesce(sum(CASE WHEN phase!='settled' AND pool=?1 AND owner_nonce=?2 THEN 1 ELSE 0 END),0) FROM outcome_operations_v7",params![pool.text(),owner],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?;
    if all >= MAX_MARKERS || root >= MAX_POOL_ROOT || local >= MAX_POOL_OWNER {
        return Err(full());
    }
    Ok(())
}
fn reserve(
    c: &Connection,
    owner: &str,
    seed: Option<&HistoryEntry>,
    d: &OutcomeDescriptor,
    pool: OutcomePool,
    audit_started: bool,
) -> AppResult<OutcomeReservation> {
    current(c)?;
    d.validate()?;
    if pool == OutcomePool::EmergencyRollback && !d.rollback() {
        return Err(invalid());
    }
    if let Some(row) = read_row(c, &d.operation_id)? {
        let state = phase(&row)?;
        if row.owner != owner || row.pool != pool.text() {
            return Err(conflict());
        }
        if state == OutcomePhase::Settled {
            // Late reserve needs the original ticket identity to recreate the
            // descriptor digest, and never reopens a settled operation.
            let ticket = if let Some(s) = seed {
                Some(lifecycle::lookup(c, &s.id)?.ok_or_else(conflict)?.0)
            } else {
                None
            };
            let data = ReservationData {
                descriptor: d.clone(),
                owner_nonce: owner.into(),
                pool,
                ticket: ticket.as_ref().map(TicketIdentity::from_ticket),
            };
            let digest = hash(&encoded(&data)?);
            if digest != row.reservation_digest {
                return Err(conflict());
            }
            return Ok(OutcomeReservation {
                data,
                digest,
                ticket,
            });
        }
        let r = reservation(&row)?;
        if r.data.descriptor != *d
            || seed.map(|s| s.id.as_str()) != r.history_ticket().map(HistoryTicket::id)
        {
            return Err(conflict());
        }
        if let Some(s) = seed
            && lifecycle::begin(c, s)? != *r.history_ticket().ok_or_else(conflict)?
        {
            return Err(conflict());
        }
        verify_binding(c, &r.data)?;
        return Ok(r);
    }
    quota(c, owner, pool)?;
    let previous_audit: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM audit_v7 WHERE id=?1)",
            [format!("{}:started", d.operation_id)],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    let previous_binding: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM outcome_bindings_v7 WHERE operation_id=?1)",
            [&d.operation_id],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if previous_audit || previous_binding {
        return Err(conflict());
    }
    let ticket = match (&d.kind, seed) {
        (OutcomeKind::Boundary { .. }, None) => None,
        (kind, Some(s))
            if !matches!(kind, OutcomeKind::Boundary { .. })
                && s.id == d.operation_id
                && s.connection_id == d.connection_id
                && s.started_at == d.started_at =>
        {
            if let OutcomeKind::Execute { planned } = kind
                && *planned != s.statement_count
            {
                return Err(invalid());
            }
            if lifecycle::lookup(c, &s.id)?.is_some() {
                return Err(conflict());
            }
            let t = lifecycle::begin(c, s)?;
            bind_owned(c, owner, &t)?;
            Some(t)
        }
        _ => return Err(invalid()),
    };
    let data = ReservationData {
        descriptor: d.clone(),
        owner_nonce: owner.into(),
        pool,
        ticket: ticket.as_ref().map(TicketIdentity::from_ticket),
    };
    let raw = encoded(&data)?;
    let digest = hash(&raw);
    let text = std::str::from_utf8(&raw).map_err(|_| invalid())?;
    c.execute("INSERT INTO outcome_operations_v7(operation_id,owner_nonce,pool,phase,reservation_digest,descriptor_json,ticket_id) VALUES(?1,?2,?3,'reserved',?4,?5,?6)",params![d.operation_id,owner,pool.text(),digest,text,ticket.as_ref().map(HistoryTicket::id)]).map_err(db_error)?;
    c.execute(
        "INSERT INTO outcome_bindings_v7 VALUES(?1,?2,?3)",
        params![
            d.operation_id,
            owner,
            ticket.as_ref().map(HistoryTicket::id)
        ],
    )
    .map_err(db_error)?;
    if audit_started {
        audit_once(c, &started(d))?;
    }
    Ok(OutcomeReservation {
        data,
        digest,
        ticket,
    })
}

fn settle(
    c: &Connection,
    r: &OutcomeReservation,
    original: &OriginalOutcome,
) -> AppResult<OutcomePhase> {
    let row = expected(c, r)?;
    let state = phase(&row)?;
    let digest = hash(original.as_bytes());
    if row.capture_digest.as_deref() != Some(digest.as_str()) {
        return Err(conflict());
    }
    if state == OutcomePhase::Settled {
        return Ok(state);
    }
    if state != OutcomePhase::Captured {
        return Err(conflict());
    }
    let p = original.payload()?;
    if p.descriptor != r.data.descriptor
        || row.payload.as_deref().map(str::as_bytes) != Some(original.as_bytes())
    {
        return Err(conflict());
    }
    verify_binding(c, &r.data)?;
    if let Some(t) = r.history_ticket() {
        lifecycle::finish(c, t, p.completion.as_ref().ok_or_else(invalid)?)?;
    }
    audit_once(c, &started(&p.descriptor))?;
    audit_once(c, &finished(&p))?;
    if let Some(t) = r.history_ticket() {
        c.execute(
            "DELETE FROM history_owners_v6 WHERE ticket_id=?1 AND owner_nonce=?2",
            params![t.id(), r.data.owner_nonce],
        )
        .map_err(db_error)?;
    }
    c.execute(
        "DELETE FROM outcome_bindings_v7 WHERE operation_id=?1",
        [r.operation_id()],
    )
    .map_err(db_error)?;
    let boundary = p.boundary.as_ref();
    let action = boundary
        .map(|b| serde_json::to_string(&b.action))
        .transpose()
        .map_err(|_| invalid())?;
    let outcome = boundary
        .map(|b| serde_json::to_string(&b.boundary_outcome))
        .transpose()
        .map_err(|_| invalid())?;
    c.execute("UPDATE outcome_operations_v7 SET phase='settled',terminal_kind='original',descriptor_json=NULL,payload_json=NULL,ticket_id=NULL,boundary_session=?1,boundary_action=?2,boundary_outcome=?3,boundary_retired=?4 WHERE operation_id=?5",params![boundary.map(|_|p.descriptor.session_id.as_str()),action,outcome,boundary.map(|b|b.retired),r.operation_id()]).map_err(db_error)?;
    Ok(OutcomePhase::Settled)
}

#[derive(Clone, Debug)]
pub(crate) struct VolatileRollbackToken {
    owner: String,
    descriptor: OutcomeDescriptor,
}
#[allow(dead_code)] // Bounded inspection API; runtime recovery uses recover_batch.
pub(crate) struct LoadedOutcome {
    pub reservation: OutcomeReservation,
    pub original: Option<OriginalOutcome>,
    pub phase: OutcomePhase,
}
#[allow(dead_code)] // Returned by the bounded storage inspection API.
pub(crate) struct OutcomePage {
    pub records: Vec<LoadedOutcome>,
    pub next: Option<String>,
}

impl Persistence {
    pub(crate) fn reserve_owned_operation(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        seed: Option<&HistoryEntry>,
        descriptor: &OutcomeDescriptor,
        pool: OutcomePool,
    ) -> AppResult<OutcomeReservation> {
        gate.validate(owner, &self.root)?;
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let r = reserve(&tx, &owner.nonce, seed, descriptor, pool, true)?;
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        Ok(r)
    }
    pub(crate) fn make_volatile_rollback_token(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        descriptor: &OutcomeDescriptor,
    ) -> AppResult<VolatileRollbackToken> {
        gate.validate(owner, &self.root)?;
        descriptor.validate()?;
        if !descriptor.rollback() {
            return Err(invalid());
        }
        Ok(VolatileRollbackToken {
            owner: owner.nonce.clone(),
            descriptor: descriptor.clone(),
        })
    }
    pub(crate) fn promote_volatile_rollback(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        token: &VolatileRollbackToken,
    ) -> AppResult<OutcomeReservation> {
        if token.owner != owner.nonce {
            return Err(conflict());
        }
        gate.validate(owner, &self.root)?;
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        // An emergency ACK must be capturable even when secondary started
        // audit recording is the fault. Settlement owes both exact audit rows.
        let r = reserve(
            &tx,
            &owner.nonce,
            None,
            &token.descriptor,
            OutcomePool::EmergencyRollback,
            false,
        )?;
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        Ok(r)
    }
    pub(crate) fn capture_original_outcome(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        r: &OutcomeReservation,
        original: &OriginalOutcome,
    ) -> AppResult<OutcomePhase> {
        gate.validate(owner, &self.root)?;
        if owner.nonce != r.data.owner_nonce || original.payload()?.descriptor != r.data.descriptor
        {
            return Err(conflict());
        }
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        current(&tx)?;
        let row = expected(&tx, r)?;
        let state = phase(&row)?;
        let digest = hash(original.as_bytes());
        if state != OutcomePhase::Reserved {
            if row.capture_digest.as_deref() != Some(digest.as_str()) {
                return Err(conflict());
            }
            return Ok(state);
        }
        verify_binding(&tx, &r.data)?;
        let raw = std::str::from_utf8(original.as_bytes()).map_err(|_| invalid())?;
        tx.execute("UPDATE outcome_operations_v7 SET phase='captured',capture_digest=?1,payload_json=?2 WHERE operation_id=?3",params![digest,raw,r.operation_id()]).map_err(db_error)?;
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        Ok(OutcomePhase::Captured)
    }
    pub(crate) fn settle_original_outcome(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        r: &OutcomeReservation,
        original: &OriginalOutcome,
    ) -> AppResult<OutcomePhase> {
        gate.validate(owner, &self.root)?;
        if owner.nonce != r.data.owner_nonce {
            return Err(conflict());
        }
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        current(&tx)?;
        let state = settle(&tx, r, original)?;
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        Ok(state)
    }
    #[allow(dead_code)] // Explicit inspection API; no product UI consumes it yet.
    pub(crate) fn load_owned_outcomes(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        after: Option<&str>,
    ) -> AppResult<OutcomePage> {
        gate.validate(owner, &self.root)?;
        let c = self.connection.lock();
        current(&c)?;
        let mut stmt=c.prepare("SELECT CASE WHEN length(CAST(operation_id AS BLOB))<=36 THEN operation_id ELSE '' END FROM outcome_operations_v7 WHERE owner_nonce=?1 AND phase!='settled' AND operation_id>?2 ORDER BY operation_id LIMIT 9").map_err(db_error)?;
        let ids = stmt
            .query_map(params![owner.nonce, after.unwrap_or("")], |r| {
                r.get::<_, String>(0)
            })
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        let more = ids.len() > 8;
        let mut records = Vec::new();
        let mut next = None;
        for id in ids.into_iter().take(8) {
            let row = read_row(&c, &id)?.ok_or_else(invalid)?;
            let state = phase(&row)?;
            let original = row.payload.as_ref().map(|raw| OriginalOutcome {
                bytes: Arc::new(raw.as_bytes().to_vec()),
            });
            records.push(LoadedOutcome {
                reservation: reservation(&row)?,
                original,
                phase: state,
            });
            next = Some(id);
        }
        Ok(OutcomePage {
            records,
            next: if more { next } else { None },
        })
    }
    pub(crate) fn abandon_reserved_operation(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        r: &OutcomeReservation,
    ) -> AppResult<OutcomePhase> {
        gate.validate(owner, &self.root)?;
        if owner.nonce != r.data.owner_nonce {
            return Err(conflict());
        }
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        current(&tx)?;
        let state = abandon(&tx, r)?;
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        Ok(state)
    }
    #[allow(dead_code)] // Closed boundary evidence read API; no product UI consumes it yet.
    pub(crate) fn settled_boundary_evidence(
        &self,
        id: &str,
    ) -> AppResult<Option<SettledBoundaryEvidence>> {
        let c = self.connection.lock();
        current(&c)?;
        let Some(row) = read_row(&c, id)? else {
            return Ok(None);
        };
        phase(&row)?;
        boundary_evidence(&c, &row)
    }
}

fn abandon(c: &Connection, r: &OutcomeReservation) -> AppResult<OutcomePhase> {
    let row = expected(c, r)?;
    let state = phase(&row)?;
    if state == OutcomePhase::Settled && row.terminal.as_deref() == Some("noOriginal") {
        return Ok(state);
    }
    if state != OutcomePhase::Reserved {
        return Err(conflict());
    }
    verify_binding(c, &r.data)?;
    if let Some(t) = r.history_ticket() {
        let (_, _, completion) = lifecycle::lookup(c, t.id())?.ok_or_else(conflict)?;
        if completion.is_none() {
            lifecycle::finish(
                c,
                t,
                &HistoryCompletion {
                    provenance: HistoryProvenance::Interrupted,
                    status: None,
                    elapsed_ms: None,
                    execution_summary: None,
                    snowflake_dml: None,
                    snowflake_ddl: None,
                },
            )?;
        }
        c.execute(
            "DELETE FROM history_owners_v6 WHERE ticket_id=?1 AND owner_nonce=?2",
            params![t.id(), r.data.owner_nonce],
        )
        .map_err(db_error)?;
    }
    c.execute("UPDATE outcome_operations_v7 SET phase='settled',terminal_kind='noOriginal',descriptor_json=NULL,payload_json=NULL,ticket_id=NULL WHERE operation_id=?1",[r.operation_id()]).map_err(db_error)?;
    c.execute(
        "DELETE FROM outcome_bindings_v7 WHERE operation_id=?1",
        [r.operation_id()],
    )
    .map_err(db_error)?;
    Ok(OutcomePhase::Settled)
}

#[allow(dead_code)] // Historical evidence returned by the storage read API.
#[derive(Clone, Debug)]
pub(crate) struct SettledBoundaryEvidence {
    pub operation_id: String,
    pub owner_nonce: String,
    pub connection_id: String,
    pub session_id: String,
    pub action: ExplicitTransactionAction,
    pub boundary_outcome: TransactionBoundaryOutcome,
    pub retired: bool,
    pub started_at: u64,
    pub finished_at: u64,
    pub digest: String,
    pub started_audit_id: String,
    pub finished_audit_id: String,
}
fn read_audit(c: &Connection, id: &str) -> AppResult<AuditEntry> {
    c.query_row(
        "SELECT CASE WHEN length(CAST(id AS BLOB))<=45 THEN id ELSE '' END,occurred_at,CASE WHEN length(CAST(connection_id AS BLOB))<=256 THEN connection_id ELSE '' END,CASE WHEN length(CAST(action AS BLOB))<=32 THEN action ELSE '' END,CASE WHEN length(CAST(digest AS BLOB))<=64 THEN digest ELSE '' END,CASE WHEN length(CAST(outcome AS BLOB))<=32 THEN outcome ELSE '' END FROM audit_v7 WHERE id=?1",
        [id],
        |r| {
            let timestamp: i64 = r.get(1)?;
            Ok(AuditEntry {
                id: r.get(0)?,
                occurred_at: u64::try_from(timestamp)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, timestamp))?,
                connection_id: r.get(2)?,
                action: r.get(3)?,
                digest: r.get(4)?,
                outcome: r.get(5)?,
            })
        },
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(invalid)
}
fn boundary_evidence(c: &Connection, row: &Row) -> AppResult<Option<SettledBoundaryEvidence>> {
    let Some(session) = &row.session else {
        return Ok(None);
    };
    if phase(row)? != OutcomePhase::Settled {
        return Err(invalid());
    }
    let action: ExplicitTransactionAction =
        serde_json::from_str(row.action.as_ref().ok_or_else(invalid)?).map_err(|_| invalid())?;
    let boundary_outcome: TransactionBoundaryOutcome =
        serde_json::from_str(row.outcome.as_ref().ok_or_else(invalid)?).map_err(|_| invalid())?;
    let start = read_audit(c, &format!("{}:started", row.id))?;
    let finish = read_audit(c, &format!("{}:finished", row.id))?;
    let descriptor = OutcomeDescriptor {
        operation_id: row.id.clone(),
        connection_id: start.connection_id.clone(),
        session_id: session.clone(),
        started_at: start.occurred_at,
        digest: start.digest.clone(),
        kind: OutcomeKind::Boundary { action },
    };
    descriptor.validate()?;
    let pool = match row.pool.as_str() {
        "ordinary" => OutcomePool::Ordinary,
        "emergencyRollback" => OutcomePool::EmergencyRollback,
        _ => return Err(invalid()),
    };
    let data = ReservationData {
        descriptor: descriptor.clone(),
        owner_nonce: row.owner.clone(),
        pool,
        ticket: None,
    };
    let payload = Payload {
        descriptor: descriptor.clone(),
        completion: None,
        boundary: Some(BoundaryEvidence {
            action,
            boundary_outcome,
            retired: row.retired == Some(1),
        }),
        finished_at: finish.occurred_at,
    };
    validate_payload(&payload)?;
    let expected_start = started(&descriptor);
    let expected_finish = finished(&payload);
    if hash(&encoded(&data)?) != row.reservation_digest
        || row.capture_digest.as_deref() != Some(hash(&encoded(&payload)?).as_str())
        || serde_json::to_value(&start).map_err(|_| invalid())?
            != serde_json::to_value(expected_start).map_err(|_| invalid())?
        || serde_json::to_value(&finish).map_err(|_| invalid())?
            != serde_json::to_value(expected_finish).map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    Ok(Some(SettledBoundaryEvidence {
        operation_id: row.id.clone(),
        owner_nonce: row.owner.clone(),
        connection_id: descriptor.connection_id,
        session_id: session.clone(),
        action,
        boundary_outcome,
        retired: row.retired == Some(1),
        started_at: start.occurred_at,
        finished_at: finish.occurred_at,
        digest: descriptor.digest,
        started_audit_id: start.id,
        finished_audit_id: finish.id,
    }))
}

pub(super) fn recover_batch(c: &Connection, nonce: &str) -> AppResult<bool> {
    let mut after = String::new();
    for _ in 0..8 {
        let id:Option<String>=c.query_row("SELECT CASE WHEN length(CAST(operation_id AS BLOB))<=36 THEN operation_id ELSE '' END FROM outcome_operations_v7 WHERE owner_nonce=?1 AND phase!='settled' AND operation_id>?2 ORDER BY operation_id LIMIT 1",params![nonce,after],|r|r.get(0)).optional().map_err(db_error)?;
        let Some(id) = id else { return Ok(false) };
        let row = read_row(c, &id)?.ok_or_else(invalid)?;
        let state = phase(&row)?;
        let r = reservation(&row)?;
        match state {
            OutcomePhase::Captured => {
                let raw = row.payload.ok_or_else(invalid)?;
                let original = OriginalOutcome {
                    bytes: Arc::new(raw.into_bytes()),
                };
                settle(c, &r, &original)?;
            }
            OutcomePhase::Reserved => {
                abandon(c, &r)?;
            }
            OutcomePhase::Settled => return Err(invalid()),
        }
        after = id;
    }
    c.query_row("SELECT EXISTS(SELECT 1 FROM outcome_operations_v7 WHERE owner_nonce=?1 AND phase!='settled')",[nonce],|r|r.get(0)).map_err(db_error)
}

pub(super) fn validate_records(c: &Connection) -> AppResult<()> {
    if schema_version(c)? < 7 {
        return Ok(());
    }
    let schema: Option<String> = c
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
            [TABLE],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if schema.as_deref() != Some(SCHEMA) {
        return Err(invalid());
    }
    let binding_schema: Option<String> = c
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
            [BINDING_TABLE],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if binding_schema.as_deref() != Some(BINDING_SCHEMA) {
        return Err(invalid());
    }
    // Check lengths inside SQLite before any ID or binding enumeration can
    // allocate malformed, arbitrarily large metadata strings in Rust.
    let oversized: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM outcome_operations_v7 WHERE length(CAST(operation_id AS BLOB))>36 OR length(CAST(owner_nonce AS BLOB))>36 OR length(CAST(pool AS BLOB))>17 OR length(CAST(phase AS BLOB))>8 OR length(CAST(reservation_digest AS BLOB))>64 OR length(CAST(capture_digest AS BLOB))>64 OR length(CAST(descriptor_json AS BLOB))>32768 OR length(CAST(payload_json AS BLOB))>32768 OR length(CAST(ticket_id AS BLOB))>36 OR length(CAST(terminal_kind AS BLOB))>10 OR length(CAST(boundary_session AS BLOB))>36 OR length(CAST(boundary_action AS BLOB))>32 OR length(CAST(boundary_outcome AS BLOB))>32) OR EXISTS(SELECT 1 FROM outcome_bindings_v7 WHERE length(CAST(operation_id AS BLOB))>36 OR length(CAST(owner_nonce AS BLOB))>36 OR length(CAST(ticket_id AS BLOB))>36)", [], |r|r.get(0)).map_err(db_error)?;
    if oversized {
        return Err(invalid());
    }
    let broken:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM outcome_bindings_v7 b LEFT JOIN outcome_operations_v7 o USING(operation_id) WHERE o.operation_id IS NULL OR o.phase='settled' OR b.owner_nonce!=o.owner_nonce OR b.ticket_id IS NOT o.ticket_id) OR EXISTS(SELECT 1 FROM outcome_operations_v7 o LEFT JOIN outcome_bindings_v7 b USING(operation_id) WHERE o.phase!='settled' AND b.operation_id IS NULL)",[],|r|r.get(0)).map_err(db_error)?;
    if broken {
        return Err(invalid());
    }
    let count: i64 = c
        .query_row("SELECT count(*) FROM outcome_operations_v7", [], |r| {
            r.get(0)
        })
        .map_err(db_error)?;
    if count > MAX_MARKERS {
        return Err(full());
    }
    let bad:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM outcome_operations_v7 WHERE phase!='settled' GROUP BY pool HAVING count(*)>128) OR EXISTS(SELECT 1 FROM outcome_operations_v7 WHERE phase!='settled' GROUP BY owner_nonce,pool HAVING count(*)>32)",[],|r|r.get(0)).map_err(db_error)?;
    if bad {
        return Err(full());
    }
    let mut stmt = c
        .prepare("SELECT operation_id FROM outcome_operations_v7 ORDER BY operation_id")
        .map_err(db_error)?;
    let mut rows = stmt.query([]).map_err(db_error)?;
    while let Some(id) = rows.next().map_err(db_error)? {
        let id: String = id.get(0).map_err(db_error)?;
        let row = read_row(c, &id)?.ok_or_else(invalid)?;
        let state = phase(&row)?;
        if state != OutcomePhase::Settled {
            verify_binding(c, &reservation_data(&row)?)?;
        } else if row.session.is_some() {
            boundary_evidence(c, &row)?;
        }
    }
    Ok(())
}

pub(super) fn merge_for_restore(current: &Connection, candidate: &Connection) -> AppResult<()> {
    validate_records(current)?;
    // Imported original payloads and markers are historical input, never local
    // operation authority. Only exact CURRENT authority survives replacement.
    candidate
        .execute("DELETE FROM outcome_operations_v7", [])
        .map_err(db_error)?;
    candidate
        .execute("DELETE FROM outcome_bindings_v7", [])
        .map_err(db_error)?;
    let mut stmt = current
        .prepare("SELECT operation_id FROM outcome_operations_v7 ORDER BY operation_id")
        .map_err(db_error)?;
    let mut rows = stmt.query([]).map_err(db_error)?;
    while let Some(id) = rows.next().map_err(db_error)? {
        let id: String = id.get(0).map_err(db_error)?;
        let r = read_row(current, &id)?.ok_or_else(invalid)?;
        candidate.execute("INSERT INTO outcome_operations_v7 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![r.id,r.owner,r.pool,r.phase,r.reservation_digest,r.capture_digest,r.descriptor,r.payload,r.ticket,r.terminal,r.session,r.action,r.outcome,r.retired]).map_err(db_error)?;
    }
    let mut stmt=current.prepare("SELECT operation_id,owner_nonce,ticket_id FROM outcome_bindings_v7 ORDER BY operation_id").map_err(db_error)?;
    let mut rows = stmt.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let owner: String = row.get(1).map_err(db_error)?;
        let ticket: Option<String> = row.get(2).map_err(db_error)?;
        candidate
            .execute(
                "INSERT INTO outcome_bindings_v7 VALUES(?1,?2,?3)",
                params![id, owner, ticket],
            )
            .map_err(db_error)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "outcomes_tests.rs"]
mod tests;
