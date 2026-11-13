//! Schema8 storage foundation. No application dispatcher consumes these APIs.
//! Original batch receipts are immutable; transaction disposition is separate.
#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Internal batch storage has no application dispatcher yet"
    )
)]

use super::{HistoryOwner, OperationGate, Persistence, db_error, outcomes, schema_version};
use crate::domain::{AppError, AppResult, AuditEntry, ErrorCode};
use crate::drivers::snowflake_batch::{
    BatchCapture, BatchEvidence, BatchRequest, FinishAction, Mode, Outcome,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Write},
    sync::Arc,
};

pub(crate) const MAX_ENVELOPE_BYTES: usize = 32 * 1024;
const MAX_RESERVATION_BYTES: usize = 4096;
const MAX_MARKERS: i64 = 65_536;
// Provisional internal bounds, not replacement-capacity or lifetime parity.
const MAX_ORDINARY_BYTES: i64 = 64 * 1024 * 1024;
const MAX_ROLLBACK_BYTES: i64 = 128 * MAX_ENVELOPE_BYTES as i64;
const MAX_GROUPS_ROOT: i64 = 128;
const MAX_GROUPS_OWNER: i64 = 32;
const MAX_TIME: u64 = 9_007_199_254_740_991;
pub(super) const SCHEMAS: &[(&str, &str)] = &[
    (
        "batch_operations_v8",
        "CREATE TABLE batch_operations_v8(operation_id TEXT PRIMARY KEY NOT NULL,owner_nonce TEXT NOT NULL,pool TEXT NOT NULL,phase TEXT NOT NULL,reservation_digest TEXT NOT NULL,reservation_json TEXT CHECK(reservation_json IS NULL OR length(CAST(reservation_json AS BLOB))<=4096),capture_digest TEXT,payload BLOB CHECK(payload IS NULL OR length(payload)<=32768),terminal_kind TEXT,group_id TEXT,reserved_bytes INTEGER NOT NULL)",
    ),
    (
        "batch_captures_v8",
        "CREATE TABLE batch_captures_v8(operation_id TEXT PRIMARY KEY NOT NULL,capture_digest TEXT NOT NULL,envelope BLOB NOT NULL CHECK(length(envelope)<=32768),pool TEXT NOT NULL,provenance TEXT NOT NULL,projection TEXT NOT NULL)",
    ),
    (
        "manual_transactions_v8",
        "CREATE TABLE manual_transactions_v8(transaction_id TEXT PRIMARY KEY NOT NULL,identity_json TEXT NOT NULL CHECK(length(CAST(identity_json AS BLOB))<=4096),helper_owner TEXT,remote_transaction TEXT,next_ordinal INTEGER NOT NULL,pending_count INTEGER NOT NULL,disposition TEXT NOT NULL,closed_by TEXT)",
    ),
    (
        "manual_transaction_members_v8",
        "CREATE TABLE manual_transaction_members_v8(operation_id TEXT PRIMARY KEY NOT NULL,transaction_id TEXT NOT NULL,ordinal INTEGER NOT NULL,prior_pending_count INTEGER NOT NULL,request_hash TEXT NOT NULL,UNIQUE(transaction_id,ordinal))",
    ),
    (
        "manual_transaction_bindings_v8",
        "CREATE TABLE manual_transaction_bindings_v8(transaction_id TEXT PRIMARY KEY NOT NULL,owner_nonce TEXT NOT NULL,rollback_credit INTEGER NOT NULL)",
    ),
];

fn invalid() -> AppError {
    AppError::invalid("Batch outcome metadata is invalid")
}
fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Batch recording or transaction ownership conflicts; preserve pending recording and retry",
    )
}
fn full() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Local batch recording capacity is full; preserve pending recording before starting another action",
    )
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}
fn current(c: &Connection) -> AppResult<()> {
    if schema_version(c)? == super::SCHEMA_VERSION {
        Ok(())
    } else {
        Err(conflict())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BatchDescriptor {
    pub operation_id: String,
    pub connection_id: String,
    pub session_id: String,
    pub helper_generation: String,
    pub started_at: u64,
    pub digest: String,
}
impl BatchDescriptor {
    fn validate(&self) -> AppResult<()> {
        if !uuid(&self.operation_id)
            || !uuid(&self.session_id)
            || !uuid(&self.helper_generation)
            || self.connection_id.is_empty()
            || self.connection_id.len() > 256
            || self.connection_id.contains('\0')
            || self.started_at > MAX_TIME
            || !digest(&self.digest)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BatchPlan {
    request: BatchRequest,
    request_hash: String,
    maximum_binary: usize,
    new_group: Option<String>,
}
fn request_hash(request: &BatchRequest) -> AppResult<String> {
    BatchCapture::maximum_encoded_len(request).map_err(|_| invalid())?;
    let empty = BatchEvidence::new(request.clone());
    Ok(hash(&BatchCapture::encode(&empty).map_err(|_| invalid())?))
}
impl BatchPlan {
    pub(crate) fn new(request: BatchRequest) -> AppResult<Self> {
        let maximum_binary = BatchCapture::maximum_encoded_len(&request).map_err(|_| invalid())?;
        // Base64 alone is a lower bound on the whole durable envelope. Exact
        // escaped metadata accounting still runs before SQLite admission.
        if BatchCapture::maximum_base64_len(&request).map_err(|_| invalid())? > MAX_ENVELOPE_BYTES {
            return Err(full());
        }
        let request_hash = request_hash(&request)?;
        let new_group = matches!(
            request,
            BatchRequest::Batch {
                mode: Mode::Manual,
                owner: None,
                ..
            }
        )
        .then(|| uuid::Uuid::new_v4().to_string());
        Ok(Self {
            request,
            request_hash,
            maximum_binary,
            new_group,
        })
    }
    pub(crate) fn request(&self) -> &BatchRequest {
        &self.request
    }
}

#[derive(Clone)]
pub(crate) struct ManualTransactionLease {
    owner: Arc<HistoryOwner>,
    id: String,
    memory: Arc<GroupMemory>,
}
// Admission is acquired without waiting, before SQLite. Holding it through
// commit and snapshot publication prevents SQL-free cleanup from observing an
// obsolete open snapshot. No path takes this lock while holding SQLite/state.
pub(super) struct GroupMemory(parking_lot::Mutex<GroupSnapshot>, parking_lot::Mutex<()>);
struct GroupSnapshot {
    credit_operation: Option<BatchDescriptor>,
    identity: GroupIdentity,
    ordinal: u32,
    pending: u32,
    helper: Option<uuid::Uuid>,
    open: bool,
    in_flight: Option<ReservationData>,
}
impl std::fmt::Debug for ManualTransactionLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManualTransactionLease")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl ManualTransactionLease {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    fn claim_rollback(&self, descriptor: &BatchDescriptor) -> AppResult<()> {
        let mut state = self.memory.0.lock();
        if state
            .credit_operation
            .as_ref()
            .is_some_and(|old| old != descriptor)
        {
            return Err(conflict());
        }
        state.credit_operation = Some(descriptor.clone());
        Ok(())
    }
}
fn lease(c: &Connection, owner: &Arc<HistoryOwner>, id: &str) -> AppResult<ManualTransactionLease> {
    let mut credits = owner.batch_credits.lock();
    credits.retain(|_, value| value.strong_count() != 0);
    let memory = if let Some(memory) = credits.get(id).and_then(std::sync::Weak::upgrade) {
        memory
    } else {
        let group = read_group(c, id)?.ok_or_else(conflict)?;
        let memory = Arc::new(GroupMemory(
            parking_lot::Mutex::new(GroupSnapshot {
                credit_operation: None,
                identity: group.identity,
                ordinal: group.ordinal,
                pending: group.pending,
                helper: group
                    .helper
                    .as_ref()
                    .map(|s| uuid::Uuid::parse_str(s).map_err(|_| invalid()))
                    .transpose()?,
                open: group.disposition == "open",
                in_flight: None,
            }),
            parking_lot::Mutex::new(()),
        ));
        credits.insert(id.to_owned(), Arc::downgrade(&memory));
        memory
    };
    Ok(ManualTransactionLease {
        owner: owner.clone(),
        id: id.to_owned(),
        memory,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Intent {
    Automatic,
    NewManual,
    ContinueManual,
    Commit,
    Rollback,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Membership {
    transaction_id: String,
    ordinal: u32,
    prior_pending_count: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReservationData {
    descriptor: BatchDescriptor,
    owner_nonce: String,
    intent: Intent,
    membership: Option<Membership>,
    request_hash: String,
    maximum_binary: usize,
}
impl ReservationData {
    fn pool(&self) -> &'static str {
        if self.intent == Intent::Rollback {
            "emergencyRollback"
        } else {
            "ordinary"
        }
    }
    fn validate(&self) -> AppResult<()> {
        self.descriptor.validate()?;
        if !uuid(&self.owner_nonce)
            || !digest(&self.request_hash)
            || self.maximum_binary > MAX_ENVELOPE_BYTES
            || self.maximum_binary == 0
        {
            return Err(invalid());
        }
        match (&self.membership, self.intent) {
            (None, Intent::Automatic) => {}
            (Some(m), intent)
                if intent != Intent::Automatic
                    && uuid(&m.transaction_id)
                    && (1..=65_536).contains(&m.ordinal)
                    && m.prior_pending_count <= 65_536 => {}
            _ => return Err(invalid()),
        }
        Ok(())
    }
}
#[derive(Clone)]
pub(crate) struct BatchReservation {
    data: ReservationData,
    digest: String,
    _owner: Arc<HistoryOwner>,
    group: Option<ManualTransactionLease>,
}
impl std::fmt::Debug for BatchReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchReservation")
            .field("operation_id", &self.operation_id())
            .field("group", &self.group)
            .finish_non_exhaustive()
    }
}
impl BatchReservation {
    pub(crate) fn operation_id(&self) -> &str {
        &self.data.descriptor.operation_id
    }
    pub(crate) fn manual_transaction(&self) -> Option<ManualTransactionLease> {
        self.group.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BatchTermination {
    Complete,
    Eof,
    Deadline,
    Cancelled,
    ObserverFailure,
    ProtocolFailure,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnvelopeMetadata {
    version: u8,
    descriptor: BatchDescriptor,
    intent: Intent,
    membership: Option<Membership>,
    request_hash: String,
    finished_at: u64,
    termination: BatchTermination,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    version: u8,
    descriptor: BatchDescriptor,
    intent: Intent,
    membership: Option<Membership>,
    request_hash: String,
    finished_at: u64,
    termination: BatchTermination,
    capture: String,
}
impl Envelope {
    fn metadata(&self) -> EnvelopeMetadata {
        EnvelopeMetadata {
            version: self.version,
            descriptor: self.descriptor.clone(),
            intent: self.intent,
            membership: self.membership.clone(),
            request_hash: self.request_hash.clone(),
            finished_at: self.finished_at,
            termination: self.termination,
        }
    }
}
struct BudgetWriter<'a>(&'a mut Vec<u8>);
impl Write for BudgetWriter<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(b.len())
            .is_none_or(|n| n > MAX_ENVELOPE_BYTES || n > self.0.capacity())
        {
            return Err(io::Error::other(
                "Batch envelope exceeds its reserved buffer",
            ));
        }
        self.0.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn json<T: Serialize>(value: &T, limit: usize) -> AppResult<Vec<u8>> {
    let mut result = Vec::with_capacity(limit);
    serde_json::to_writer(BudgetWriter(&mut result), value).map_err(|_| invalid())?;
    Ok(result)
}
fn encode_envelope(
    metadata: &EnvelopeMetadata,
    capture: &[u8],
    mut b: Vec<u8>,
) -> AppResult<Vec<u8>> {
    b.clear();
    serde_json::to_writer(BudgetWriter(&mut b), metadata).map_err(|_| invalid())?;
    if b.pop() != Some(b'}') {
        return Err(invalid());
    }
    BudgetWriter(&mut b)
        .write_all(b",\"capture\":\"")
        .map_err(|_| invalid())?;
    {
        let mut encoder = base64::write::EncoderWriter::new(BudgetWriter(&mut b), &STANDARD);
        encoder.write_all(capture).map_err(|_| invalid())?;
        encoder.finish().map_err(|_| invalid())?;
    }
    BudgetWriter(&mut b)
        .write_all(b"\"}")
        .map_err(|_| invalid())?;
    Ok(b)
}
fn metadata(
    data: &ReservationData,
    finished_at: u64,
    termination: BatchTermination,
) -> EnvelopeMetadata {
    EnvelopeMetadata {
        version: 1,
        descriptor: data.descriptor.clone(),
        intent: data.intent,
        membership: data.membership.clone(),
        request_hash: data.request_hash.clone(),
        finished_at,
        termination,
    }
}
fn preflight(data: &ReservationData) -> AppResult<()> {
    data.validate()?;
    let prefix = json(
        &metadata(data, MAX_TIME, BatchTermination::ProtocolFailure),
        MAX_ENVELOPE_BYTES,
    )?;
    let base64 = data
        .maximum_binary
        .checked_add(2)
        .and_then(|n| n.checked_div(3))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(invalid)?;
    if prefix
        .len()
        .checked_add(base64)
        .and_then(|n| n.checked_add(13))
        .is_none_or(|n| n > MAX_ENVELOPE_BYTES)
    {
        return Err(full());
    }
    Ok(())
}
fn validate_capture(meta: &EnvelopeMetadata, evidence: &BatchEvidence) -> AppResult<()> {
    meta.descriptor.validate()?;
    if meta.version != 1
        || meta.finished_at > MAX_TIME
        || !digest(&meta.request_hash)
        || request_hash(&evidence.request)? != meta.request_hash
        || (evidence.summary.is_none() && meta.termination == BatchTermination::Complete)
    {
        return Err(invalid());
    }
    let intent = match &evidence.request {
        BatchRequest::Batch {
            mode: Mode::Automatic,
            ..
        } => Intent::Automatic,
        BatchRequest::Batch {
            mode: Mode::Manual,
            owner: None,
            ..
        } => Intent::NewManual,
        BatchRequest::Batch {
            mode: Mode::Manual,
            owner: Some(_),
            ..
        } => Intent::ContinueManual,
        BatchRequest::Finish {
            action: FinishAction::Commit,
            ..
        } => Intent::Commit,
        BatchRequest::Finish {
            action: FinishAction::Rollback,
            ..
        } => Intent::Rollback,
    };
    if intent != meta.intent {
        return Err(invalid());
    }
    match (&meta.membership, intent) {
        (None, Intent::Automatic) => {}
        (Some(m), i)
            if i != Intent::Automatic
                && uuid(&m.transaction_id)
                && (1..=65_536).contains(&m.ordinal)
                && m.prior_pending_count <= 65_536 =>
        {
            if evidence
                .summary
                .as_ref()
                .is_some_and(|s| s.prior_batches != u64::from(m.prior_pending_count))
            {
                return Err(conflict());
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}
fn decode(bytes: &[u8]) -> AppResult<(EnvelopeMetadata, BatchEvidence)> {
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(invalid());
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let binary = STANDARD.decode(&envelope.capture).map_err(|_| invalid())?;
    let evidence = BatchCapture::decode(&binary).map_err(|_| invalid())?;
    let meta = envelope.metadata();
    validate_capture(&meta, &evidence)?;
    if encode_envelope(&meta, &binary, Vec::with_capacity(MAX_ENVELOPE_BYTES))? != bytes {
        return Err(invalid());
    }
    Ok((meta, evidence))
}

#[derive(Clone, Debug)]
pub(crate) struct OriginalBatchOutcome {
    bytes: Arc<Vec<u8>>,
}
impl OriginalBatchOutcome {
    pub(crate) fn capture_in(
        r: &BatchReservation,
        evidence: &BatchEvidence,
        termination: BatchTermination,
        finished_at: u64,
        buffer: Vec<u8>,
    ) -> AppResult<Self> {
        let meta = metadata(&r.data, finished_at, termination);
        validate_capture(&meta, evidence)?;
        let binary = BatchCapture::encode(evidence).map_err(|_| invalid())?;
        if binary.len() > r.data.maximum_binary {
            return Err(invalid());
        }
        Ok(Self {
            bytes: Arc::new(encode_envelope(&meta, &binary, buffer)?),
        })
    }
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn volatile_rollback_in(
        token: &VolatileBatchRollbackToken,
        evidence: &BatchEvidence,
        termination: BatchTermination,
        finished_at: u64,
        buffer: Vec<u8>,
    ) -> AppResult<Self> {
        let meta = metadata(&token.data, finished_at, termination);
        validate_capture(&meta, evidence)?;
        let binary = BatchCapture::encode(evidence).map_err(|_| invalid())?;
        if binary.len() > token.data.maximum_binary {
            return Err(invalid());
        }
        Ok(Self {
            bytes: Arc::new(encode_envelope(&meta, &binary, buffer)?),
        })
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
}

#[derive(Clone, Debug)]
pub(crate) struct HistoricalBatchEvidence {
    bytes: Arc<Vec<u8>>,
    pub imported: bool,
    pub settled: bool,
}
impl HistoricalBatchEvidence {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn evidence(&self) -> AppResult<BatchEvidence> {
        decode(&self.bytes).map(|(_, e)| e)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManualTransactionMember {
    pub operation_id: String,
    pub ordinal: u32,
    pub prior_pending_count: u32,
}
#[derive(Clone, Debug)]
pub(crate) struct ManualTransactionPage {
    pub transaction_id: String,
    pub disposition: String,
    pub pending_count: u32,
    pub members: Vec<ManualTransactionMember>,
    pub next: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GroupIdentity {
    connection_id: String,
    session_id: String,
    helper_generation: String,
    first_operation: String,
    created_at: u64,
}
impl GroupIdentity {
    fn descriptor(&self, d: &BatchDescriptor) -> bool {
        self.connection_id == d.connection_id
            && self.session_id == d.session_id
            && self.helper_generation == d.helper_generation
    }
    fn validate(&self) -> AppResult<()> {
        BatchDescriptor {
            operation_id: self.first_operation.clone(),
            connection_id: self.connection_id.clone(),
            session_id: self.session_id.clone(),
            helper_generation: self.helper_generation.clone(),
            started_at: self.created_at,
            digest: "0".repeat(64),
        }
        .validate()
    }
}
struct Group {
    identity: GroupIdentity,
    helper: Option<String>,
    remote: Option<String>,
    ordinal: u32,
    pending: u32,
    disposition: String,
    closed_by: Option<String>,
}
type GroupRow = (
    String,
    Option<String>,
    Option<String>,
    u32,
    u32,
    String,
    Option<String>,
);
fn read_group(c: &Connection, id: &str) -> AppResult<Option<Group>> {
    let raw: Option<GroupRow> = c.query_row(
        "SELECT CASE WHEN length(CAST(identity_json AS BLOB))<=4096 THEN identity_json ELSE '' END,CASE WHEN helper_owner IS NULL OR length(CAST(helper_owner AS BLOB))<=36 THEN helper_owner ELSE '' END,CASE WHEN remote_transaction IS NULL OR length(CAST(remote_transaction AS BLOB))<=20 THEN remote_transaction ELSE '' END,next_ordinal,pending_count,CASE WHEN length(CAST(disposition AS BLOB))<=11 THEN disposition ELSE '' END,CASE WHEN closed_by IS NULL OR length(CAST(closed_by AS BLOB))<=36 THEN closed_by ELSE '' END FROM manual_transactions_v8 WHERE transaction_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(db_error)?;
    raw.map(
        |(identity, helper, remote, ordinal, pending, disposition, closed_by)| {
            let identity: GroupIdentity = serde_json::from_str(&identity).map_err(|_| invalid())?;
            identity.validate()?;
            if !uuid(id)
                || ordinal > 65_536
                || pending > 65_536
                || helper.as_ref().is_some_and(|s| !uuid(s))
                || remote
                    .as_ref()
                    .is_some_and(|s| s.parse::<i64>().ok().is_none_or(|n| n.to_string() != *s))
                || !matches!(
                    disposition.as_str(),
                    "open" | "committed" | "rolledBack" | "unknown" | "unsubmitted"
                )
                || closed_by.as_ref().is_some_and(|s| !uuid(s))
            {
                return Err(invalid());
            }
            Ok(Group {
                identity,
                helper,
                remote,
                ordinal,
                pending,
                disposition,
                closed_by,
            })
        },
    )
    .transpose()
}
fn binding(c: &Connection, id: &str) -> AppResult<Option<(String, bool)>> {
    c.query_row("SELECT CASE WHEN length(CAST(owner_nonce AS BLOB))<=36 THEN owner_nonce ELSE '' END,rollback_credit FROM manual_transaction_bindings_v8 WHERE transaction_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)
}
struct Row {
    owner: String,
    pool: String,
    phase: String,
    reservation_digest: String,
    data: Option<String>,
    capture_digest: Option<String>,
    payload: Option<Vec<u8>>,
    terminal: Option<String>,
    group: Option<String>,
    reserved: i64,
}
fn read_row(c: &Connection, id: &str) -> AppResult<Option<Row>> {
    c.query_row("SELECT CASE WHEN length(CAST(owner_nonce AS BLOB))<=36 THEN owner_nonce ELSE '' END,CASE WHEN length(CAST(pool AS BLOB))<=17 THEN pool ELSE '' END,CASE WHEN length(CAST(phase AS BLOB))<=8 THEN phase ELSE '' END,CASE WHEN length(CAST(reservation_digest AS BLOB))<=64 THEN reservation_digest ELSE '' END,CASE WHEN reservation_json IS NULL OR length(CAST(reservation_json AS BLOB))<=4096 THEN reservation_json ELSE '' END,CASE WHEN capture_digest IS NULL OR length(CAST(capture_digest AS BLOB))<=64 THEN capture_digest ELSE '' END,CASE WHEN payload IS NULL OR length(payload)<=32768 THEN payload ELSE x'' END,CASE WHEN terminal_kind IS NULL OR length(CAST(terminal_kind AS BLOB))<=10 THEN terminal_kind ELSE '' END,CASE WHEN group_id IS NULL OR length(CAST(group_id AS BLOB))<=36 THEN group_id ELSE '' END,reserved_bytes FROM batch_operations_v8 WHERE operation_id=?1", [id], |r| Ok(Row { owner:r.get(0)?,pool:r.get(1)?,phase:r.get(2)?,reservation_digest:r.get(3)?,data:r.get(4)?,capture_digest:r.get(5)?,payload:r.get(6)?,terminal:r.get(7)?,group:r.get(8)?,reserved:r.get(9)? })).optional().map_err(db_error)
}
fn row_data(id: &str, row: &Row) -> AppResult<ReservationData> {
    let raw = row.data.as_ref().ok_or_else(invalid)?;
    let data: ReservationData = serde_json::from_str(raw).map_err(|_| invalid())?;
    preflight(&data)?;
    if data.descriptor.operation_id != id
        || data.owner_nonce != row.owner
        || data.pool() != row.pool
        || hash(raw.as_bytes()) != row.reservation_digest
        || json(&data, MAX_RESERVATION_BYTES)? != raw.as_bytes()
        || data.membership.as_ref().map(|m| &m.transaction_id) != row.group.as_ref()
    {
        return Err(invalid());
    }
    Ok(data)
}
fn expected(c: &Connection, data: &ReservationData, reservation_digest: &str) -> AppResult<Row> {
    let row = read_row(c, &data.descriptor.operation_id)?.ok_or_else(conflict)?;
    if row.owner != data.owner_nonce
        || row.reservation_digest != reservation_digest
        || row.pool != data.pool()
        || row.group.as_deref() != data.membership.as_ref().map(|m| m.transaction_id.as_str())
    {
        return Err(conflict());
    }
    if row.phase != "settled" && row_data(&data.descriptor.operation_id, &row)? != *data {
        return Err(conflict());
    }
    Ok(row)
}
fn verify_member(c: &Connection, data: &ReservationData) -> AppResult<()> {
    if let Some(m) = &data.membership {
        let b = binding(c, &m.transaction_id)?.ok_or_else(conflict)?;
        if b.0 != data.owner_nonce {
            return Err(conflict());
        }
        let member: Option<(String,u32,u32,String)> = c.query_row("SELECT transaction_id,ordinal,prior_pending_count,request_hash FROM manual_transaction_members_v8 WHERE operation_id=?1",[&data.descriptor.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(db_error)?;
        if member
            != Some((
                m.transaction_id.clone(),
                m.ordinal,
                m.prior_pending_count,
                data.request_hash.clone(),
            ))
        {
            return Err(conflict());
        }
    }
    Ok(())
}
pub(super) fn namespace_exists(c: &Connection, id: &str) -> AppResult<bool> {
    if schema_version(c)? < 8 {
        return Ok(false);
    }
    c.query_row("SELECT EXISTS(SELECT 1 FROM batch_operations_v8 WHERE operation_id=?1) OR EXISTS(SELECT 1 FROM batch_captures_v8 WHERE operation_id=?1) OR EXISTS(SELECT 1 FROM manual_transaction_members_v8 WHERE operation_id=?1)",[id],|r|r.get(0)).map_err(db_error)
}
pub(super) fn counts(c: &Connection, owner: &str, pool: &str) -> AppResult<(i64, i64, i64)> {
    if schema_version(c)? < 8 {
        return Ok((0, 0, 0));
    }
    let (all,root,local):(i64,i64,i64)=c.query_row("SELECT count(*),coalesce(sum(phase!='settled' AND pool=?1),0),coalesce(sum(phase!='settled' AND pool=?1 AND owner_nonce=?2),0) FROM batch_operations_v8",params![pool,owner],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?;
    let (credits,own):(i64,i64)=c.query_row("SELECT coalesce(sum(rollback_credit),0),coalesce(sum(rollback_credit AND owner_nonce=?1),0) FROM manual_transaction_bindings_v8",[owner],|r|Ok((r.get(0)?,r.get(1)?))).map_err(db_error)?;
    let historical:i64=c.query_row("SELECT count(*) FROM (SELECT operation_id FROM batch_captures_v8 UNION SELECT operation_id FROM manual_transaction_members_v8) WHERE operation_id NOT IN (SELECT operation_id FROM batch_operations_v8)",[],|r|r.get(0)).map_err(db_error)?;
    Ok((
        all + historical + credits,
        root + if pool == "emergencyRollback" {
            credits
        } else {
            0
        },
        local + if pool == "emergencyRollback" { own } else { 0 },
    ))
}
fn capacity(c: &Connection, owner: &str, pool: &str, markers: i64, active: i64) -> AppResult<()> {
    let (a, r, l) = counts(c, owner, pool)?;
    let (oa,or,ol):(i64,i64,i64)=c.query_row("SELECT count(*),coalesce(sum(phase!='settled' AND pool=?1),0),coalesce(sum(phase!='settled' AND pool=?1 AND owner_nonce=?2),0) FROM outcome_operations_v7",params![pool,owner],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?;
    if a + oa + markers > MAX_MARKERS || r + or + active > 128 || l + ol + active > 32 {
        return Err(full());
    }
    Ok(())
}
fn bytes_used(c: &Connection, pool: &str) -> AppResult<i64> {
    c.query_row("SELECT (SELECT coalesce(sum(reserved_bytes),0) FROM batch_operations_v8 WHERE pool=?1)+(SELECT coalesce(sum(length(envelope)),0) FROM batch_captures_v8 WHERE pool=?1)+(CASE WHEN ?1='emergencyRollback' THEN (SELECT coalesce(sum(rollback_credit),0)*32768 FROM manual_transaction_bindings_v8) ELSE 0 END)",[pool],|r|r.get(0)).map_err(db_error)
}
fn byte_capacity(c: &Connection, pool: &str, extra: i64) -> AppResult<()> {
    if bytes_used(c, pool)? + extra
        > if pool == "ordinary" {
            MAX_ORDINARY_BYTES
        } else {
            MAX_ROLLBACK_BYTES
        }
    {
        return Err(full());
    }
    Ok(())
}
fn audit(d: &BatchDescriptor, finished: Option<(u64, &str)>, intent: Intent) -> AuditEntry {
    AuditEntry {
        id: format!(
            "{}:{}",
            d.operation_id,
            if finished.is_some() {
                "finished"
            } else {
                "started"
            }
        ),
        occurred_at: finished.map_or(d.started_at, |x| x.0),
        connection_id: d.connection_id.clone(),
        action: if matches!(intent, Intent::Commit | Intent::Rollback) {
            "snowflake-mutation-finish"
        } else {
            "snowflake-mutation-batch"
        }
        .into(),
        digest: d.digest.clone(),
        outcome: finished.map_or("started", |x| x.1).into(),
    }
}
fn outcome(e: &BatchEvidence) -> &'static str {
    match e.summary.as_ref().map(|s| s.outcome) {
        Some(Outcome::Committed) => "committed",
        Some(Outcome::Pending) => "pending",
        Some(Outcome::RolledBack) => "rolledBack",
        Some(Outcome::Rejected) => "rejected",
        _ => "unknown",
    }
}

fn reserve(
    c: &Connection,
    owner: &Arc<HistoryOwner>,
    d: &BatchDescriptor,
    plan: &BatchPlan,
    group: Option<&ManualTransactionLease>,
    audit_started: bool,
) -> AppResult<BatchReservation> {
    current(c)?;
    d.validate()?;
    let intent = match &plan.request {
        BatchRequest::Batch {
            mode: Mode::Automatic,
            ..
        } => Intent::Automatic,
        BatchRequest::Batch {
            mode: Mode::Manual,
            owner: None,
            ..
        } => Intent::NewManual,
        BatchRequest::Batch {
            mode: Mode::Manual,
            owner: Some(_),
            ..
        } => Intent::ContinueManual,
        BatchRequest::Finish {
            action: FinishAction::Commit,
            ..
        } => Intent::Commit,
        BatchRequest::Finish {
            action: FinishAction::Rollback,
            ..
        } => Intent::Rollback,
    };
    if let Some(row) = read_row(c, &d.operation_id)? {
        if row.phase == "settled" {
            return Err(conflict());
        }
        let data = row_data(&d.operation_id, &row)?;
        if data.descriptor != *d
            || data.owner_nonce != owner.nonce
            || data.intent != intent
            || data.request_hash != plan.request_hash
        {
            return Err(conflict());
        }
        if data.membership.as_ref().map(|m| m.transaction_id.as_str())
            != group.map(|g| g.id()).or(plan.new_group.as_deref())
        {
            return Err(conflict());
        }
        verify_member(c, &data)?;
        return Ok(BatchReservation {
            digest: row.reservation_digest,
            data,
            _owner: owner.clone(),
            group: row
                .group
                .as_deref()
                .map(|id| lease(c, owner, id))
                .transpose()?,
        });
    }
    let collision:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM outcome_operations_v7 WHERE operation_id=?1) OR EXISTS(SELECT 1 FROM history_tickets_v5 WHERE id=?1) OR EXISTS(SELECT 1 FROM batch_captures_v8 WHERE operation_id=?1) OR EXISTS(SELECT 1 FROM manual_transaction_members_v8 WHERE operation_id=?1) OR EXISTS(SELECT 1 FROM audit_v8 WHERE id IN (?2,?3))",params![d.operation_id,format!("{}:started",d.operation_id),format!("{}:finished",d.operation_id)],|r|r.get(0)).map_err(db_error)?;
    if collision {
        return Err(conflict());
    }
    let mut group_lease = None;
    let membership = if intent == Intent::Automatic {
        if group.is_some() {
            return Err(invalid());
        }
        None
    } else if intent == Intent::NewManual {
        if group.is_some() {
            return Err(invalid());
        }
        let id = plan.new_group.as_ref().ok_or_else(invalid)?;
        if read_group(c, id)?.is_some() {
            return Err(conflict());
        }
        let (root,local):(i64,i64)=c.query_row("SELECT count(*),coalesce(sum(owner_nonce=?1),0) FROM manual_transaction_bindings_v8",[&owner.nonce],|r|Ok((r.get(0)?,r.get(1)?))).map_err(db_error)?;
        if root >= MAX_GROUPS_ROOT || local >= MAX_GROUPS_OWNER {
            return Err(full());
        }
        capacity(c, &owner.nonce, "ordinary", 2, 1)?;
        capacity(c, &owner.nonce, "emergencyRollback", 2, 1)?;
        byte_capacity(c, "ordinary", MAX_ENVELOPE_BYTES as i64)?;
        byte_capacity(c, "emergencyRollback", MAX_ENVELOPE_BYTES as i64)?;
        let identity = GroupIdentity {
            connection_id: d.connection_id.clone(),
            session_id: d.session_id.clone(),
            helper_generation: d.helper_generation.clone(),
            first_operation: d.operation_id.clone(),
            created_at: d.started_at,
        };
        let raw =
            String::from_utf8(json(&identity, MAX_RESERVATION_BYTES)?).map_err(|_| invalid())?;
        c.execute(
            "INSERT INTO manual_transactions_v8 VALUES(?1,?2,NULL,NULL,1,0,'open',NULL)",
            params![id, raw],
        )
        .map_err(db_error)?;
        c.execute(
            "INSERT INTO manual_transaction_bindings_v8 VALUES(?1,?2,1)",
            params![id, owner.nonce],
        )
        .map_err(db_error)?;
        group_lease = Some(lease(c, owner, id)?);
        Some(Membership {
            transaction_id: id.clone(),
            ordinal: 1,
            prior_pending_count: 0,
        })
    } else {
        let g = group.ok_or_else(invalid)?;
        if !Arc::ptr_eq(owner, &g.owner) {
            return Err(conflict());
        }
        {
            let state = g.memory.0.lock();
            if state
                .credit_operation
                .as_ref()
                .is_some_and(|old| old != d || intent != Intent::Rollback)
                || state
                    .in_flight
                    .as_ref()
                    .is_some_and(|data| data.descriptor != *d || data.intent != intent)
            {
                return Err(conflict());
            }
        }
        let b = binding(c, g.id())?.ok_or_else(conflict)?;
        let state = read_group(c, g.id())?.ok_or_else(conflict)?;
        if b.0 != owner.nonce
            || state.disposition != "open"
            || !state.identity.descriptor(d)
            || state.ordinal
                >= if intent == Intent::Rollback {
                    65_536
                } else {
                    65_535
                }
            || state.pending == 0
        {
            return Err(conflict());
        }
        if state.helper.as_deref() != plan.request.owner().map(|id| id.to_string()).as_deref() {
            return Err(conflict());
        }
        let pending:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM batch_operations_v8 WHERE group_id=?1 AND phase!='settled')",[g.id()],|r|r.get(0)).map_err(db_error)?;
        if pending {
            return Err(conflict());
        }
        if intent == Intent::Rollback {
            g.claim_rollback(d)?;
            if !b.1 {
                return Err(conflict());
            }
            // Convert the protected credit: no additional marker, pool slot or bytes.
            c.execute("UPDATE manual_transaction_bindings_v8 SET rollback_credit=0 WHERE transaction_id=?1",[g.id()]).map_err(db_error)?;
        } else {
            capacity(c, &owner.nonce, "ordinary", 1, 1)?;
            byte_capacity(c, "ordinary", MAX_ENVELOPE_BYTES as i64)?;
        }
        c.execute(
            "UPDATE manual_transactions_v8 SET next_ordinal=next_ordinal+1 WHERE transaction_id=?1",
            [g.id()],
        )
        .map_err(db_error)?;
        group_lease = Some(g.clone());
        Some(Membership {
            transaction_id: g.id.clone(),
            ordinal: state.ordinal + 1,
            prior_pending_count: state.pending,
        })
    };
    if intent == Intent::Automatic {
        capacity(c, &owner.nonce, "ordinary", 1, 1)?;
        byte_capacity(c, "ordinary", MAX_ENVELOPE_BYTES as i64)?;
    }
    let data = ReservationData {
        descriptor: d.clone(),
        owner_nonce: owner.nonce.clone(),
        intent,
        membership,
        request_hash: plan.request_hash.clone(),
        maximum_binary: plan.maximum_binary,
    };
    preflight(&data)?;
    let raw = String::from_utf8(json(&data, MAX_RESERVATION_BYTES)?).map_err(|_| invalid())?;
    let reservation_digest = hash(raw.as_bytes());
    if let Some(m) = &data.membership {
        c.execute(
            "INSERT INTO manual_transaction_members_v8 VALUES(?1,?2,?3,?4,?5)",
            params![
                d.operation_id,
                m.transaction_id,
                m.ordinal,
                m.prior_pending_count,
                plan.request_hash
            ],
        )
        .map_err(db_error)?;
    }
    c.execute(
        "INSERT INTO batch_operations_v8 VALUES(?1,?2,?3,'reserved',?4,?5,NULL,NULL,NULL,?6,32768)",
        params![
            d.operation_id,
            owner.nonce,
            data.pool(),
            reservation_digest,
            raw,
            data.membership.as_ref().map(|m| m.transaction_id.as_str())
        ],
    )
    .map_err(db_error)?;
    if audit_started {
        outcomes::audit_once(c, &audit(d, None, intent))?;
    }
    Ok(BatchReservation {
        data,
        digest: reservation_digest,
        _owner: owner.clone(),
        group: group_lease,
    })
}

fn archive(
    c: &Connection,
    meta: &EnvelopeMetadata,
    bytes: &[u8],
    pool: &str,
    provenance: &str,
    projection: &str,
) -> AppResult<()> {
    let digest = hash(bytes);
    c.execute("INSERT INTO batch_captures_v8 VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(operation_id) DO NOTHING",params![meta.descriptor.operation_id,digest,bytes,pool,provenance,projection]).map_err(db_error)?;
    let old = read_archive(c, &meta.descriptor.operation_id)?.ok_or_else(invalid)?;
    if old.0 != digest || old.1 != bytes || old.2 != pool {
        return Err(conflict());
    }
    // Identical imported evidence is historical until genuine local settlement
    // commits its audit and projection. Never downgrade a settled projection.
    if provenance == "local" && projection == "settled" {
        c.execute(
            "UPDATE batch_captures_v8 SET provenance='local',projection='settled' WHERE operation_id=?1",
            [&meta.descriptor.operation_id],
        ).map_err(db_error)?;
    }
    Ok(())
}
type ArchiveRow = (String, Vec<u8>, String, String, String);
fn read_archive(c: &Connection, id: &str) -> AppResult<Option<ArchiveRow>> {
    c.query_row("SELECT CASE WHEN capture_digest IS NULL OR length(CAST(capture_digest AS BLOB))<=64 THEN capture_digest ELSE '' END,CASE WHEN length(envelope)<=32768 THEN envelope ELSE x'' END,CASE WHEN length(CAST(pool AS BLOB))<=17 THEN pool ELSE '' END,CASE WHEN length(CAST(provenance AS BLOB))<=8 THEN provenance ELSE '' END,CASE WHEN length(CAST(projection AS BLOB))<=8 THEN projection ELSE '' END FROM batch_captures_v8 WHERE operation_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(db_error)
}
fn settle(
    c: &Connection,
    data: &ReservationData,
    reservation_digest: &str,
    bytes: &[u8],
) -> AppResult<outcomes::OutcomePhase> {
    let row = expected(c, data, reservation_digest)?;
    if row.capture_digest.as_deref() != Some(hash(bytes).as_str()) {
        return Err(conflict());
    }
    if row.phase == "settled" {
        let archived = read_archive(c, &data.descriptor.operation_id)?.ok_or_else(conflict)?;
        if archived.1 != bytes {
            return Err(conflict());
        }
        return Ok(outcomes::OutcomePhase::Settled);
    }
    if row.phase != "captured" || row.payload.as_deref() != Some(bytes) {
        return Err(conflict());
    }
    let (meta, evidence) = decode(bytes)?;
    if meta != metadata(data, meta.finished_at, meta.termination) {
        return Err(conflict());
    }
    verify_member(c, data)?;
    archive(c, &meta, bytes, data.pool(), "local", "settled")?;
    outcomes::audit_once(c, &audit(&data.descriptor, None, data.intent))?;
    outcomes::audit_once(
        c,
        &audit(
            &data.descriptor,
            Some((meta.finished_at, outcome(&evidence))),
            data.intent,
        ),
    )?;
    if let Some(m) = &data.membership {
        let state = read_group(c, &m.transaction_id)?.ok_or_else(conflict)?;
        if state.disposition != "open"
            || state.pending != m.prior_pending_count
            || state.ordinal != m.ordinal
        {
            return Err(conflict());
        }
        let helper = evidence
            .summary
            .as_ref()
            .and_then(|s| s.owner)
            .map(|id| id.to_string());
        let remote = evidence
            .summary
            .as_ref()
            .and_then(|s| s.transaction_id.as_ref());
        if state
            .helper
            .as_ref()
            .zip(helper.as_ref())
            .is_some_and(|(a, b)| a != b)
            || state
                .remote
                .as_ref()
                .zip(remote)
                .is_some_and(|(a, b)| a != b)
        {
            return Err(conflict());
        }
        let result = outcome(&evidence);
        let disposition = match result {
            "pending" => "open",
            "committed" => "committed",
            "rolledBack" => "rolledBack",
            "rejected" if data.intent == Intent::NewManual => "unsubmitted",
            "rejected" if data.intent != Intent::Rollback => "open",
            _ => "unknown",
        };
        let pending = state.pending + u32::from(result == "pending");
        c.execute("UPDATE manual_transactions_v8 SET helper_owner=coalesce(helper_owner,?1),remote_transaction=coalesce(remote_transaction,?2),pending_count=?3,disposition=?4,closed_by=?5 WHERE transaction_id=?6",params![helper,remote,pending,disposition,if disposition=="open"{None}else{Some(data.descriptor.operation_id.as_str())},m.transaction_id]).map_err(db_error)?;
        // Unknown retains a conservative live binding until OS-proven owner death.
        if matches!(disposition, "committed" | "rolledBack" | "unsubmitted") {
            c.execute(
                "DELETE FROM manual_transaction_bindings_v8 WHERE transaction_id=?1",
                [&m.transaction_id],
            )
            .map_err(db_error)?;
        }
    }
    c.execute("UPDATE batch_operations_v8 SET phase='settled',reservation_json=NULL,payload=NULL,terminal_kind='original',reserved_bytes=0 WHERE operation_id=?1",[&data.descriptor.operation_id]).map_err(db_error)?;
    Ok(outcomes::OutcomePhase::Settled)
}
fn abandon(
    c: &Connection,
    data: &ReservationData,
    reservation_digest: &str,
) -> AppResult<outcomes::OutcomePhase> {
    let row = expected(c, data, reservation_digest)?;
    if row.phase == "settled" && row.terminal.as_deref() == Some("noOriginal") {
        return Ok(outcomes::OutcomePhase::Settled);
    }
    if row.phase != "reserved" || row.capture_digest.is_some() {
        return Err(conflict());
    }
    verify_member(c, data)?;
    if let Some(m) = &data.membership {
        c.execute("UPDATE manual_transactions_v8 SET disposition='unknown',closed_by=?1 WHERE transaction_id=?2",params![data.descriptor.operation_id,m.transaction_id]).map_err(db_error)?;
    }
    c.execute("UPDATE batch_operations_v8 SET phase='settled',reservation_json=NULL,payload=NULL,terminal_kind='noOriginal',reserved_bytes=0 WHERE operation_id=?1",[&data.descriptor.operation_id]).map_err(db_error)?;
    Ok(outcomes::OutcomePhase::Settled)
}

#[derive(Clone, Debug)]
pub(crate) struct VolatileBatchRollbackToken {
    descriptor: BatchDescriptor,
    plan: BatchPlan,
    group: ManualTransactionLease,
    data: ReservationData,
}
impl Persistence {
    pub(crate) fn reserve_owned_batch(
        &self,
        owner: &Arc<HistoryOwner>,
        gate: &OperationGate,
        d: &BatchDescriptor,
        plan: &BatchPlan,
        group: Option<&ManualTransactionLease>,
    ) -> AppResult<BatchReservation> {
        if matches!(plan.request, BatchRequest::Finish { .. }) {
            return Err(invalid());
        }
        self.reserve_batch_inner(owner, gate, d, plan, group, true)
    }
    fn reserve_batch_inner(
        &self,
        owner: &Arc<HistoryOwner>,
        gate: &OperationGate,
        d: &BatchDescriptor,
        plan: &BatchPlan,
        group: Option<&ManualTransactionLease>,
        started: bool,
    ) -> AppResult<BatchReservation> {
        let _admission = group
            .map(|group| group.memory.1.try_lock().ok_or_else(conflict))
            .transpose()?;
        gate.validate(owner, &self.root)?;
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let r = reserve(&tx, owner, d, plan, group, started)?;
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        if let (Some(group), Some(m)) = (&r.group, &r.data.membership) {
            let mut state = group.memory.0.lock();
            state.ordinal = m.ordinal;
            state.pending = m.prior_pending_count;
            state.in_flight = Some(r.data.clone());
        }
        Ok(r)
    }
    pub(crate) fn reserve_owned_batch_finish(
        &self,
        owner: &Arc<HistoryOwner>,
        gate: &OperationGate,
        d: &BatchDescriptor,
        group: &ManualTransactionLease,
        action: FinishAction,
    ) -> AppResult<BatchReservation> {
        gate.validate(owner, &self.root)?;
        let helper = {
            let c = self.connection.lock();
            current(&c)?;
            read_group(&c, group.id())?
                .ok_or_else(conflict)?
                .helper
                .ok_or_else(conflict)?
        };
        let helper = uuid::Uuid::parse_str(&helper).map_err(|_| invalid())?;
        let plan = BatchPlan::new(BatchRequest::Finish {
            owner: helper,
            action,
        })?;
        self.reserve_batch_inner(owner, gate, d, &plan, Some(group), true)
    }
    pub(crate) fn make_volatile_batch_rollback_token(
        &self,
        owner: &Arc<HistoryOwner>,
        gate: &OperationGate,
        d: &BatchDescriptor,
        group: &ManualTransactionLease,
        helper_owner: uuid::Uuid,
    ) -> AppResult<VolatileBatchRollbackToken> {
        let _admission = group.memory.1.try_lock().ok_or_else(conflict)?;
        gate.validate(owner, &self.root)?;
        d.validate()?;
        if !Arc::ptr_eq(owner, &group.owner) {
            return Err(conflict());
        }
        let plan = BatchPlan::new(BatchRequest::Finish {
            owner: helper_owner,
            action: FinishAction::Rollback,
        })?;
        let mut state = group.memory.0.lock();
        if !state.open
            || state.pending == 0
            || state.helper != Some(helper_owner)
            || !state.identity.descriptor(d)
            || state
                .in_flight
                .as_ref()
                .is_some_and(|data| data.descriptor != *d || data.intent != Intent::Rollback)
        {
            return Err(conflict());
        }
        if state.credit_operation.as_ref().is_some_and(|old| old != d) {
            return Err(conflict());
        }
        let ordinal = if state.in_flight.is_some() {
            state.ordinal
        } else {
            state.ordinal.checked_add(1).ok_or_else(full)?
        };
        let data = ReservationData {
            descriptor: d.clone(),
            owner_nonce: owner.nonce.clone(),
            intent: Intent::Rollback,
            membership: Some(Membership {
                transaction_id: group.id.clone(),
                ordinal,
                prior_pending_count: state.pending,
            }),
            request_hash: plan.request_hash.clone(),
            maximum_binary: plan.maximum_binary,
        };
        preflight(&data)?;
        state.credit_operation = Some(d.clone());
        Ok(VolatileBatchRollbackToken {
            descriptor: d.clone(),
            plan,
            group: group.clone(),
            data,
        })
    }
    pub(crate) fn promote_volatile_batch_rollback(
        &self,
        owner: &Arc<HistoryOwner>,
        gate: &OperationGate,
        token: &VolatileBatchRollbackToken,
    ) -> AppResult<BatchReservation> {
        let _admission = token.group.memory.1.try_lock().ok_or_else(conflict)?;
        gate.validate(owner, &self.root)?;
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let reservation = reserve(
            &tx,
            owner,
            &token.descriptor,
            &token.plan,
            Some(&token.group),
            false,
        )?;
        if reservation.data != token.data {
            return Err(conflict());
        }
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        let m = reservation.data.membership.as_ref().ok_or_else(invalid)?;
        let mut state = token.group.memory.0.lock();
        state.ordinal = m.ordinal;
        state.pending = m.prior_pending_count;
        state.in_flight = Some(reservation.data.clone());
        Ok(reservation)
    }
    pub(crate) fn capture_batch_outcome(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        r: &BatchReservation,
        original: &OriginalBatchOutcome,
    ) -> AppResult<outcomes::OutcomePhase> {
        gate.validate(owner, &self.root)?;
        if owner.nonce != r.data.owner_nonce {
            return Err(conflict());
        }
        let (meta, _) = decode(original.as_bytes())?;
        if meta != metadata(&r.data, meta.finished_at, meta.termination) {
            return Err(conflict());
        }
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        current(&tx)?;
        let row = expected(&tx, &r.data, &r.digest)?;
        let capture_digest = hash(original.as_bytes());
        if row.phase != "reserved" {
            if row.capture_digest.as_deref() != Some(capture_digest.as_str()) {
                return Err(conflict());
            }
            return match row.phase.as_str() {
                "captured" => Ok(outcomes::OutcomePhase::Captured),
                "settled" => Ok(outcomes::OutcomePhase::Settled),
                _ => Err(invalid()),
            };
        }
        verify_member(&tx, &r.data)?;
        tx.execute("UPDATE batch_operations_v8 SET phase='captured',capture_digest=?1,payload=?2 WHERE operation_id=?3",params![capture_digest,original.as_bytes(),r.operation_id()]).map_err(db_error)?;
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        Ok(outcomes::OutcomePhase::Captured)
    }
    pub(crate) fn settle_batch_outcome(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        r: &BatchReservation,
        original: &OriginalBatchOutcome,
    ) -> AppResult<outcomes::OutcomePhase> {
        let _admission = r
            .group
            .as_ref()
            .map(|group| group.memory.1.try_lock().ok_or_else(conflict))
            .transpose()?;
        gate.validate(owner, &self.root)?;
        if owner.nonce != r.data.owner_nonce {
            return Err(conflict());
        }
        let (helper, result) = {
            let (_, e) = decode(original.as_bytes())?;
            (e.summary.as_ref().and_then(|s| s.owner), outcome(&e))
        };
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        current(&tx)?;
        let phase = settle(&tx, &r.data, &r.digest, original.as_bytes())?;
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        if let Some(group) = &r.group {
            let mut state = group.memory.0.lock();
            let mut remove_registry = false;
            if state.in_flight.as_ref().is_some_and(|data| data == &r.data) {
                state.pending = r
                    .data
                    .membership
                    .as_ref()
                    .ok_or_else(invalid)?
                    .prior_pending_count
                    + u32::from(result == "pending");
                if let Some(helper) = helper {
                    state.helper = Some(helper);
                }
                state.open = result == "pending"
                    || (result == "rejected"
                        && !matches!(r.data.intent, Intent::NewManual | Intent::Rollback));
                state.in_flight = None;
                remove_registry = matches!(result, "committed" | "rolledBack")
                    || (result == "rejected" && r.data.intent == Intent::NewManual);
            }
            drop(state);
            if remove_registry {
                group.owner.batch_credits.lock().remove(group.id());
            }
        }
        Ok(phase)
    }
    pub(crate) fn abandon_reserved_batch(
        &self,
        owner: &HistoryOwner,
        gate: &OperationGate,
        r: &BatchReservation,
    ) -> AppResult<outcomes::OutcomePhase> {
        let _admission = r
            .group
            .as_ref()
            .map(|group| group.memory.1.try_lock().ok_or_else(conflict))
            .transpose()?;
        gate.validate(owner, &self.root)?;
        if owner.nonce != r.data.owner_nonce {
            return Err(conflict());
        }
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        current(&tx)?;
        let phase = abandon(&tx, &r.data, &r.digest)?;
        gate.validate(owner, &self.root)?;
        tx.commit().map_err(db_error)?;
        if let Some(group) = &r.group {
            let mut state = group.memory.0.lock();
            state.open = false;
            state.in_flight = None;
        }
        Ok(phase)
    }
    pub(crate) fn batch_evidence(&self, id: &str) -> AppResult<Option<HistoricalBatchEvidence>> {
        if !uuid(id) {
            return Err(invalid());
        }
        let c = self.connection.lock();
        current(&c)?;
        let Some((digest, bytes, _, provenance, projection)) = read_archive(&c, id)? else {
            return Ok(None);
        };
        let (m, _) = decode(&bytes)?;
        if m.descriptor.operation_id != id
            || hash(&bytes) != digest
            || !matches!(provenance.as_str(), "local" | "imported")
            || !matches!(projection.as_str(), "settled" | "captured")
        {
            return Err(invalid());
        }
        Ok(Some(HistoricalBatchEvidence {
            bytes: Arc::new(bytes),
            imported: provenance == "imported",
            settled: projection == "settled",
        }))
    }
    pub(crate) fn manual_transaction_evidence(
        &self,
        id: &str,
        after_ordinal: u32,
        limit: usize,
    ) -> AppResult<ManualTransactionPage> {
        if !uuid(id) || limit == 0 || limit > 64 {
            return Err(invalid());
        }
        let c = self.connection.lock();
        current(&c)?;
        let g = read_group(&c, id)?.ok_or_else(conflict)?;
        let mut stmt=c.prepare("SELECT CASE WHEN length(CAST(operation_id AS BLOB))<=36 THEN operation_id ELSE '' END,ordinal,prior_pending_count FROM manual_transaction_members_v8 WHERE transaction_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT ?3").map_err(db_error)?;
        let mut members = stmt
            .query_map(params![id, after_ordinal, limit as i64 + 1], |r| {
                Ok(ManualTransactionMember {
                    operation_id: r.get(0)?,
                    ordinal: r.get(1)?,
                    prior_pending_count: r.get(2)?,
                })
            })
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        if members.iter().any(|m| {
            !uuid(&m.operation_id)
                || m.ordinal == 0
                || m.ordinal > 65_536
                || m.prior_pending_count > 65_536
        }) {
            return Err(invalid());
        }
        let more = members.len() > limit;
        members.truncate(limit);
        let next = if more {
            members.last().map(|m| m.ordinal)
        } else {
            None
        };
        Ok(ManualTransactionPage {
            transaction_id: id.to_owned(),
            disposition: g.disposition,
            pending_count: g.pending,
            members,
            next,
        })
    }
}

/// Called only while the caller retains a nonblocking OS proof of owner death.
pub(super) fn recover_batch(c: &Connection, nonce: &str) -> AppResult<bool> {
    if schema_version(c)? < 8 {
        return Ok(false);
    }
    for _ in 0..8 {
        let id:Option<String>=c.query_row("SELECT CASE WHEN length(CAST(operation_id AS BLOB))<=36 THEN operation_id ELSE '' END FROM batch_operations_v8 WHERE owner_nonce=?1 AND phase!='settled' ORDER BY operation_id LIMIT 1",[nonce],|r|r.get(0)).optional().map_err(db_error)?;
        let Some(id) = id else {
            // Originals were settled first. Death never proves remote disposition.
            c.execute("UPDATE manual_transactions_v8 SET disposition='unknown' WHERE transaction_id IN (SELECT transaction_id FROM manual_transaction_bindings_v8 WHERE owner_nonce=?1)",[nonce]).map_err(db_error)?;
            c.execute(
                "DELETE FROM manual_transaction_bindings_v8 WHERE owner_nonce=?1",
                [nonce],
            )
            .map_err(db_error)?;
            return Ok(false);
        };
        let row = read_row(c, &id)?.ok_or_else(invalid)?;
        let data = row_data(&id, &row)?;
        match row.phase.as_str() {
            "captured" => {
                settle(
                    c,
                    &data,
                    &row.reservation_digest,
                    row.payload.as_deref().ok_or_else(invalid)?,
                )?;
            }
            "reserved" => {
                abandon(c, &data, &row.reservation_digest)?;
            }
            _ => return Err(invalid()),
        }
    }
    // A follow-up transaction also retires idle groups after the final page.
    Ok(true)
}

fn validate_schema(c: &Connection) -> AppResult<()> {
    for (name, sql) in SCHEMAS {
        let old: Option<String> = c
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if old.as_deref() != Some(*sql) {
            return Err(invalid());
        }
    }
    Ok(())
}
fn validate_bounds(c: &Connection) -> AppResult<()> {
    let oversized:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM batch_operations_v8 WHERE length(CAST(operation_id AS BLOB))>36 OR length(CAST(owner_nonce AS BLOB))>36 OR length(CAST(pool AS BLOB))>17 OR length(CAST(phase AS BLOB))>8 OR length(CAST(reservation_digest AS BLOB))>64 OR length(CAST(reservation_json AS BLOB))>4096 OR length(CAST(capture_digest AS BLOB))>64 OR length(payload)>32768 OR length(CAST(terminal_kind AS BLOB))>10 OR length(CAST(group_id AS BLOB))>36) OR EXISTS(SELECT 1 FROM batch_captures_v8 WHERE length(CAST(operation_id AS BLOB))>36 OR length(CAST(capture_digest AS BLOB))>64 OR length(envelope)>32768 OR length(CAST(pool AS BLOB))>17 OR length(CAST(provenance AS BLOB))>8 OR length(CAST(projection AS BLOB))>8) OR EXISTS(SELECT 1 FROM manual_transactions_v8 WHERE length(CAST(transaction_id AS BLOB))>36 OR length(CAST(identity_json AS BLOB))>4096 OR length(CAST(helper_owner AS BLOB))>36 OR length(CAST(remote_transaction AS BLOB))>20 OR length(CAST(disposition AS BLOB))>11 OR length(CAST(closed_by AS BLOB))>36) OR EXISTS(SELECT 1 FROM manual_transaction_members_v8 WHERE length(CAST(operation_id AS BLOB))>36 OR length(CAST(transaction_id AS BLOB))>36 OR length(CAST(request_hash AS BLOB))>64) OR EXISTS(SELECT 1 FROM manual_transaction_bindings_v8 WHERE length(CAST(transaction_id AS BLOB))>36 OR length(CAST(owner_nonce AS BLOB))>36 OR rollback_credit NOT IN(0,1))",[],|r|r.get(0)).map_err(db_error)?;
    if oversized {
        return Err(invalid());
    }
    for (name, _) in SCHEMAS {
        let count: i64 = c
            .query_row(&format!("SELECT count(*) FROM {name}"), [], |r| r.get(0))
            .map_err(db_error)?;
        if count > MAX_MARKERS {
            return Err(full());
        }
    }
    let collisions:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM outcome_operations_v7 WHERE operation_id IN(SELECT operation_id FROM batch_operations_v8 UNION SELECT operation_id FROM batch_captures_v8 UNION SELECT operation_id FROM manual_transaction_members_v8))",[],|r|r.get(0)).map_err(db_error)?;
    if collisions {
        return Err(conflict());
    }
    let (all, _, _) = counts(c, "", "ordinary")?;
    let other: i64 = c
        .query_row("SELECT count(*) FROM outcome_operations_v7", [], |r| {
            r.get(0)
        })
        .map_err(db_error)?;
    if all + other > MAX_MARKERS {
        return Err(full());
    }
    let active:bool=c.query_row("WITH live(owner_nonce,pool) AS (SELECT owner_nonce,pool FROM outcome_operations_v7 WHERE phase!='settled' UNION ALL SELECT owner_nonce,pool FROM batch_operations_v8 WHERE phase!='settled' UNION ALL SELECT owner_nonce,'emergencyRollback' FROM manual_transaction_bindings_v8 WHERE rollback_credit=1) SELECT EXISTS(SELECT 1 FROM live GROUP BY pool HAVING count(*)>128) OR EXISTS(SELECT 1 FROM live GROUP BY owner_nonce,pool HAVING count(*)>32) OR (SELECT count(*) FROM manual_transaction_bindings_v8)>128 OR EXISTS(SELECT 1 FROM manual_transaction_bindings_v8 GROUP BY owner_nonce HAVING count(*)>32)",[],|r|r.get(0)).map_err(db_error)?;
    if active {
        return Err(full());
    }
    byte_capacity(c, "ordinary", 0)?;
    byte_capacity(c, "emergencyRollback", 0)
}
fn validate_archive(c: &Connection, id: &str) -> AppResult<(EnvelopeMetadata, BatchEvidence)> {
    let (d, b, pool, provenance, projection) = read_archive(c, id)?.ok_or_else(invalid)?;
    let (m, e) = decode(&b)?;
    if !uuid(id)
        || m.descriptor.operation_id != id
        || !digest(&d)
        || hash(&b) != d
        || pool
            != if m.intent == Intent::Rollback {
                "emergencyRollback"
            } else {
                "ordinary"
            }
        || !matches!(provenance.as_str(), "local" | "imported")
        || !matches!(projection.as_str(), "settled" | "captured")
    {
        return Err(invalid());
    }
    if provenance == "local" {
        let row = read_row(c, id)?.ok_or_else(invalid)?;
        if row.phase != "settled"
            || row.terminal.as_deref() != Some("original")
            || row.capture_digest.as_deref() != Some(d.as_str())
            || projection != "settled"
        {
            return Err(invalid());
        }
    }
    if projection == "settled" {
        for expected in [
            audit(&m.descriptor, None, m.intent),
            audit(&m.descriptor, Some((m.finished_at, outcome(&e))), m.intent),
        ] {
            let actual = outcomes::read_audit(c, &expected.id)?;
            if actual.id != expected.id
                || actual.occurred_at != expected.occurred_at
                || actual.connection_id != expected.connection_id
                || actual.action != expected.action
                || actual.digest != expected.digest
                || actual.outcome != expected.outcome
            {
                return Err(invalid());
            }
        }
    }
    if let Some(member) = &m.membership {
        let stored:Option<(String,u32,u32,String)>=c.query_row("SELECT transaction_id,ordinal,prior_pending_count,request_hash FROM manual_transaction_members_v8 WHERE operation_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(db_error)?;
        if stored
            != Some((
                member.transaction_id.clone(),
                member.ordinal,
                member.prior_pending_count,
                m.request_hash.clone(),
            ))
        {
            return Err(invalid());
        }
    }
    Ok((m, e))
}
pub(super) fn validate_records(c: &Connection) -> AppResult<()> {
    if schema_version(c)? < 8 {
        return Ok(());
    }
    validate_schema(c)?;
    validate_bounds(c)?;
    let mut operations = c
        .prepare("SELECT operation_id FROM batch_operations_v8 ORDER BY operation_id")
        .map_err(db_error)?;
    let mut rows = operations.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let r = read_row(c, &id)?.ok_or_else(invalid)?;
        if !uuid(&id)
            || !uuid(&r.owner)
            || !digest(&r.reservation_digest)
            || !matches!(r.pool.as_str(), "ordinary" | "emergencyRollback")
            || r.group.as_ref().is_some_and(|g| !uuid(g))
        {
            return Err(invalid());
        }
        match r.phase.as_str() {
            "reserved" | "captured" => {
                let d = row_data(&id, &r)?;
                verify_member(c, &d)?;
                if r.terminal.is_some() || r.reserved != MAX_ENVELOPE_BYTES as i64 {
                    return Err(invalid());
                }
                if r.phase == "reserved" {
                    if r.payload.is_some() || r.capture_digest.is_some() {
                        return Err(invalid());
                    }
                } else {
                    let bytes = r.payload.as_deref().ok_or_else(invalid)?;
                    let (m, e) = decode(bytes)?;
                    if m != metadata(&d, m.finished_at, m.termination)
                        || r.capture_digest.as_deref() != Some(hash(bytes).as_str())
                        || BatchCapture::encode(&e).map_err(|_| invalid())?.len() > d.maximum_binary
                    {
                        return Err(invalid());
                    }
                    if let Some(archived) = read_archive(c, &id)?
                        && (archived.0 != hash(bytes)
                            || archived.1 != bytes
                            || archived.2 != r.pool)
                    {
                        return Err(conflict());
                    }
                }
                if let Some(m) = &d.membership {
                    let g = read_group(c, &m.transaction_id)?.ok_or_else(invalid)?;
                    if g.disposition != "open"
                        || g.ordinal != m.ordinal
                        || g.pending != m.prior_pending_count
                        || !g.identity.descriptor(&d.descriptor)
                    {
                        return Err(invalid());
                    }
                }
            }
            "settled" => {
                if r.data.is_some() || r.payload.is_some() || r.reserved != 0 {
                    return Err(invalid());
                }
                match r.terminal.as_deref() {
                    Some("original") => {
                        let (m, _) = validate_archive(c, &id)?;
                        if r.group.as_deref()
                            != m.membership.as_ref().map(|m| m.transaction_id.as_str())
                        {
                            return Err(invalid());
                        }
                    }
                    Some("noOriginal") => {
                        if r.capture_digest.is_some() || read_archive(c, &id)?.is_some() {
                            return Err(invalid());
                        }
                    }
                    _ => return Err(invalid()),
                }
            }
            _ => return Err(invalid()),
        }
    }
    let mut archives = c
        .prepare("SELECT operation_id FROM batch_captures_v8 ORDER BY operation_id")
        .map_err(db_error)?;
    let mut rows = archives.query([]).map_err(db_error)?;
    while let Some(r) = rows.next().map_err(db_error)? {
        let id: String = r.get(0).map_err(db_error)?;
        validate_archive(c, &id)?;
    }
    let mut groups = c
        .prepare("SELECT transaction_id FROM manual_transactions_v8 ORDER BY transaction_id")
        .map_err(db_error)?;
    let mut rows = groups.query([]).map_err(db_error)?;
    while let Some(r) = rows.next().map_err(db_error)? {
        let id: String = r.get(0).map_err(db_error)?;
        let g = read_group(c, &id)?.ok_or_else(invalid)?;
        let mut members=c.prepare("SELECT operation_id,ordinal,prior_pending_count,request_hash FROM manual_transaction_members_v8 WHERE transaction_id=?1 ORDER BY ordinal").map_err(db_error)?;
        let mut member_rows = members.query([&id]).map_err(db_error)?;
        let mut ordinal = 0;
        let mut pending = 0;
        let mut blocked_after = false;
        // (operation, disposition, exact closed_by): imported unprojected
        // captures and source NoOriginal gaps prove uncertainty, not settlement.
        let mut terminal: Option<(String, &str, bool)> = None;
        while let Some(r) = member_rows.next().map_err(db_error)? {
            let operation: String = r.get(0).map_err(db_error)?;
            let n: u32 = r.get(1).map_err(db_error)?;
            let prior: u32 = r.get(2).map_err(db_error)?;
            let request: String = r.get(3).map_err(db_error)?;
            ordinal += 1;
            if blocked_after
                || !uuid(&operation)
                || n != ordinal
                || prior != pending
                || !digest(&request)
                || (n == 1 && operation != g.identity.first_operation)
            {
                return Err(invalid());
            }
            let operation_row = read_row(c, &operation)?;
            let active = operation_row.as_ref().is_some_and(|r| r.phase != "settled");
            if let Some(archived) = read_archive(c, &operation)? {
                let (m, e) = validate_archive(c, &operation)?;
                let intent_valid = if n == 1 {
                    m.intent == Intent::NewManual && prior == 0
                } else {
                    prior > 0
                        && matches!(
                            m.intent,
                            Intent::ContinueManual | Intent::Commit | Intent::Rollback
                        )
                };
                if !intent_valid
                    || !g.identity.descriptor(&m.descriptor)
                    || m.membership.as_ref().map(|m| m.transaction_id.as_str()) != Some(id.as_str())
                    || request != m.request_hash
                {
                    return Err(invalid());
                }
                if let Some(summary) = &e.summary
                    && (summary
                        .owner
                        .map(|v| v.to_string())
                        .as_ref()
                        .zip(g.helper.as_ref())
                        .is_some_and(|(a, b)| a != b)
                        || summary
                            .transaction_id
                            .as_ref()
                            .zip(g.remote.as_ref())
                            .is_some_and(|(a, b)| a != b))
                {
                    return Err(invalid());
                }
                if active {
                    // Historical identical bytes may coexist with a current
                    // capture until recovery commits its local projection.
                    blocked_after = true;
                    continue;
                }
                let result = outcome(&e);
                pending += u32::from(result == "pending");
                let disposition = match result {
                    _ if archived.4 == "captured" => Some(("unknown", false)),
                    "pending" => None,
                    "committed" => Some(("committed", true)),
                    "rolledBack" => Some(("rolledBack", true)),
                    "rejected" if m.intent == Intent::NewManual => Some(("unsubmitted", true)),
                    "rejected" if m.intent != Intent::Rollback => None,
                    _ => Some(("unknown", true)),
                };
                if let Some((disposition, exact)) = disposition {
                    terminal = Some((operation.clone(), disposition, exact));
                    blocked_after = true;
                }
            } else if active {
                let data = row_data(&operation, operation_row.as_ref().ok_or_else(invalid)?)?;
                let intent_valid = if n == 1 {
                    data.intent == Intent::NewManual && prior == 0
                } else {
                    prior > 0
                        && matches!(
                            data.intent,
                            Intent::ContinueManual | Intent::Commit | Intent::Rollback
                        )
                };
                if !intent_valid || request != data.request_hash {
                    return Err(invalid());
                }
                blocked_after = true;
            } else {
                // Imported reservation/NoOriginal membership has no original
                // receipt. Preserve the gap and forbid a later continuation.
                terminal = Some((operation.clone(), "unknown", operation_row.is_some()));
                blocked_after = true;
            }
        }
        if ordinal == 0 || ordinal != g.ordinal || pending != g.pending {
            return Err(invalid());
        }
        if let Some((nonce, _)) = binding(c, &id)? {
            if !uuid(&nonce) || !matches!(g.disposition.as_str(), "open" | "unknown") {
                return Err(invalid());
            }
        } else if g.disposition == "open" {
            return Err(invalid());
        }
        if let Some((operation, disposition, exact)) = terminal {
            if g.disposition != disposition
                || (exact && g.closed_by.as_deref() != Some(operation.as_str()))
                || g.closed_by
                    .as_deref()
                    .is_some_and(|closed| closed != operation)
            {
                return Err(invalid());
            }
        } else if !matches!(g.disposition.as_str(), "open" | "unknown") || g.closed_by.is_some() {
            return Err(invalid());
        }
    }
    let orphan:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM manual_transaction_members_v8 m LEFT JOIN manual_transactions_v8 g USING(transaction_id) WHERE g.transaction_id IS NULL) OR EXISTS(SELECT 1 FROM manual_transaction_bindings_v8 b LEFT JOIN manual_transactions_v8 g USING(transaction_id) WHERE g.transaction_id IS NULL) OR EXISTS(SELECT 1 FROM batch_operations_v8 o WHERE o.group_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM manual_transaction_members_v8 m WHERE m.operation_id=o.operation_id AND m.transaction_id=o.group_id)) OR EXISTS(SELECT 1 FROM batch_operations_v8 o WHERE o.phase!='settled' AND o.group_id IS NOT NULL AND (SELECT count(*) FROM batch_operations_v8 b WHERE b.group_id=o.group_id AND b.phase!='settled')!=1)",[],|r|r.get(0)).map_err(db_error)?;
    if orphan {
        return Err(invalid());
    }
    Ok(())
}

fn copy_exact(
    c: &Connection,
    d: &Connection,
    table: &str,
    key: &str,
    columns: &str,
    count: usize,
) -> AppResult<()> {
    let mut stmt = c
        .prepare(&format!("SELECT {columns} FROM {table} ORDER BY {key}"))
        .map_err(db_error)?;
    let mut rows = stmt.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let values = (0..count)
            .map(|i| row.get::<_, rusqlite::types::Value>(i))
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        let placeholders = (1..=count)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        d.execute(&format!("INSERT INTO {table}({columns}) VALUES({placeholders}) ON CONFLICT({key}) DO NOTHING"),rusqlite::params_from_iter(values.iter())).map_err(db_error)?;
        let old = d
            .query_row(
                &format!("SELECT {columns} FROM {table} WHERE {key}=?1"),
                [&values[0]],
                |r| {
                    (0..count)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                },
            )
            .map_err(db_error)?;
        if old != values {
            return Err(conflict());
        }
    }
    Ok(())
}
/// Candidate archives are historical evidence; only CURRENT bindings survive.
pub(super) fn merge_for_restore(current: &Connection, candidate: &Connection) -> AppResult<()> {
    validate_records(current)?;
    validate_records(candidate)?;
    // Preserve imported captured originals before removing their authority.
    let mut stmt=candidate.prepare("SELECT operation_id FROM batch_operations_v8 WHERE phase='captured' ORDER BY operation_id").map_err(db_error)?;
    let mut rows = stmt.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let r = read_row(candidate, &id)?.ok_or_else(invalid)?;
        let bytes = r.payload.as_deref().ok_or_else(invalid)?;
        let (m, _) = decode(bytes)?;
        archive(candidate, &m, bytes, &r.pool, "imported", "captured")?;
    }
    drop(rows);
    drop(stmt);
    // Current captured originals have the same immutable precedence as current
    // archives. Detect a different imported original before copying authority.
    let mut stmt = current.prepare("SELECT operation_id FROM batch_operations_v8 WHERE phase='captured' ORDER BY operation_id").map_err(db_error)?;
    let mut rows = stmt.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let current = read_row(current, &id)?.ok_or_else(invalid)?;
        if let Some(imported) = read_archive(candidate, &id)?
            && (Some(imported.0.as_str()) != current.capture_digest.as_deref()
                || Some(imported.1.as_slice()) != current.payload.as_deref()
                || imported.2 != current.pool)
        {
            return Err(conflict());
        }
    }
    drop(rows);
    drop(stmt);
    candidate
        .execute("UPDATE batch_captures_v8 SET provenance='imported'", [])
        .map_err(db_error)?;
    candidate
        .execute("DELETE FROM batch_operations_v8", [])
        .map_err(db_error)?;
    candidate
        .execute("DELETE FROM manual_transaction_bindings_v8", [])
        .map_err(db_error)?;
    // A foreign live group becomes historical uncertainty, never a resumable lease.
    candidate
        .execute(
            "UPDATE manual_transactions_v8 SET disposition='unknown' WHERE disposition='open'",
            [],
        )
        .map_err(db_error)?;
    let mut stmt = current
        .prepare("SELECT operation_id FROM batch_captures_v8 ORDER BY operation_id")
        .map_err(db_error)?;
    let mut rows = stmt.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let (digest, bytes, pool, provenance, projection) =
            read_archive(current, &id)?.ok_or_else(invalid)?;
        if let Some(old) = read_archive(candidate, &id)?
            && (old.0 != digest || old.1 != bytes || old.2 != pool)
        {
            return Err(conflict());
        }
        candidate.execute("INSERT INTO batch_captures_v8 VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(operation_id) DO UPDATE SET provenance=excluded.provenance,projection=excluded.projection",params![id,digest,bytes,pool,provenance,projection]).map_err(db_error)?;
    }
    // Group mutable bookkeeping may legitimately differ between compatible snapshots.
    // Immutable identity and member rows must agree; current disposition wins.
    let mut stmt=current.prepare("SELECT transaction_id,identity_json,helper_owner,remote_transaction,next_ordinal,pending_count,disposition,closed_by FROM manual_transactions_v8 ORDER BY transaction_id").map_err(db_error)?;
    let mut rows = stmt.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let identity: String = row.get(1).map_err(db_error)?;
        if let Some(g) = read_group(candidate, &id)? {
            let raw: String = candidate
                .query_row(
                    "SELECT identity_json FROM manual_transactions_v8 WHERE transaction_id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            let current_group = read_group(current, &id)?.ok_or_else(invalid)?;
            if raw != identity
                || g.helper
                    .as_ref()
                    .zip(current_group.helper.as_ref())
                    .is_some_and(|(a, b)| a != b)
                || g.remote
                    .as_ref()
                    .zip(current_group.remote.as_ref())
                    .is_some_and(|(a, b)| a != b)
            {
                return Err(conflict());
            }
        }
        let values = (0..8)
            .map(|i| row.get::<_, rusqlite::types::Value>(i))
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        candidate.execute("INSERT INTO manual_transactions_v8 VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(transaction_id) DO UPDATE SET helper_owner=coalesce(excluded.helper_owner,helper_owner),remote_transaction=coalesce(excluded.remote_transaction,remote_transaction),disposition=excluded.disposition,closed_by=excluded.closed_by",rusqlite::params_from_iter(values.iter())).map_err(db_error)?;
    }
    copy_exact(
        current,
        candidate,
        "manual_transaction_members_v8",
        "operation_id",
        "operation_id,transaction_id,ordinal,prior_pending_count,request_hash",
        5,
    )?;
    copy_exact(
        current,
        candidate,
        "batch_operations_v8",
        "operation_id",
        "operation_id,owner_nonce,pool,phase,reservation_digest,reservation_json,capture_digest,payload,terminal_kind,group_id,reserved_bytes",
        11,
    )?;
    copy_exact(
        current,
        candidate,
        "manual_transaction_bindings_v8",
        "transaction_id",
        "transaction_id,owner_nonce,rollback_credit",
        3,
    )?;
    // Recompute historical aggregate counts from immutable member captures, while
    // retaining current open-group counters exactly (they cannot accept new imports).
    let mut stmt = candidate
        .prepare("SELECT transaction_id FROM manual_transactions_v8 ORDER BY transaction_id")
        .map_err(db_error)?;
    let mut rows = stmt.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let group = read_group(candidate, &id)?.ok_or_else(invalid)?;
        let ordinal:u32=candidate.query_row("SELECT coalesce(max(ordinal),0) FROM manual_transaction_members_v8 WHERE transaction_id=?1",[&id],|r|r.get(0)).map_err(db_error)?;
        let mut members=candidate.prepare("SELECT operation_id FROM manual_transaction_members_v8 WHERE transaction_id=?1 ORDER BY ordinal").map_err(db_error)?;
        let mut member_rows = members.query([&id]).map_err(db_error)?;
        let mut pending = 0u32;
        while let Some(r) = member_rows.next().map_err(db_error)? {
            let op: String = r.get(0).map_err(db_error)?;
            if let Some((_, bytes, _, _, _)) = read_archive(candidate, &op)?
                && !read_row(candidate, &op)?.is_some_and(|r| r.phase != "settled")
            {
                pending += u32::from(outcome(&decode(&bytes)?.1) == "pending");
            }
        }
        if binding(candidate, &id)?.is_some()
            && (ordinal != group.ordinal || pending != group.pending)
        {
            return Err(conflict());
        }
        candidate.execute("UPDATE manual_transactions_v8 SET next_ordinal=?1,pending_count=?2 WHERE transaction_id=?3",params![ordinal,pending,id]).map_err(db_error)?;
    }
    validate_records(candidate)
}

#[cfg(test)]
pub(super) fn drop_schema(c: &Connection) {
    for (name, _) in SCHEMAS {
        c.execute_batch(&format!("DROP TABLE {name}")).unwrap();
    }
}

#[cfg(test)]
#[path = "batch_process_tests.rs"]
mod batch_process_tests;
#[cfg(test)]
#[path = "batch_outcomes_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "batch_validation_tests.rs"]
mod batch_validation_tests;
