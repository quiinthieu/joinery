//! Bounded Snowflake helper process transport. No remote capability is qualified here.
use super::snowflake_batch::{BatchCapture, BatchEvidence, BatchRequest, Outcome as BatchOutcome};
#[cfg(all(unix, feature = "test-harness"))]
use super::snowflake_qualification_transport::{
    Disposition, LocalTerminal, LocalTerminalOrigin, PhysicalRequest, PhysicalTerminal,
    PublicationGuard, Stream, TransportAttachment,
};
use crate::domain::{SnowflakeDdlKind, SnowflakeDmlKind};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::Command,
    sync::{mpsc, watch},
    time::timeout,
};

pub const MAX_FRAME: usize = 1_048_576;
pub const MAX_BATCH: usize = 262_144;
const MAX_CREDITS: u32 = 32;
pub(crate) const INACTIVE_OPERATION: &str = "stale or inactive operation token";

pub struct Launch {
    /// Absolute, explicitly selected executable; never resolved using PATH.
    pub executable: PathBuf,
    /// Only an absolute packaged helper script, never credentials or SQL.
    pub script: Option<PathBuf>,
    pub working_directory: PathBuf,
    state_directory: Option<PathBuf>,
    cleanup: CleanupObserver,
    #[cfg(all(unix, feature = "test-harness"))]
    transport: Option<TransportAttachment>,
}
impl Launch {
    pub fn new(
        executable: PathBuf,
        script: Option<PathBuf>,
        working_directory: PathBuf,
        state_directory: tempfile::TempDir,
    ) -> std::io::Result<Self> {
        Self::new_observed(
            executable,
            script,
            working_directory,
            state_directory,
            |_| {},
        )
    }

    /// Attach cleanup ownership immediately after allocation, before fallible
    /// validation. The callback must consume an already reserved cleanup permit;
    /// it must not perform admission checks or await anything.
    pub fn new_observed(
        executable: PathBuf,
        script: Option<PathBuf>,
        working_directory: PathBuf,
        state_directory: tempfile::TempDir,
        observe: impl FnOnce(CleanupObserver),
    ) -> std::io::Result<Self> {
        // Keep automatic TempDir deletion disarmed even on constructor errors.
        // The observer retains removal authority and an explicit no-child receipt.
        let path = state_directory.keep();
        let (cleanup_tx, receiver) = watch::channel(None);
        let authority = Arc::new(CleanupAuthority {
            root: OwnedRoot::capture(path.clone()),
            state: Mutex::new(CleanupState::default()),
            updates: cleanup_tx,
            #[cfg(test)]
            before_remove: Mutex::new(None),
        });
        let launch = Self {
            executable,
            script,
            working_directory,
            state_directory: Some(path),
            #[cfg(all(unix, feature = "test-harness"))]
            transport: None,
            cleanup: CleanupObserver {
                receiver,
                authority: Some(authority),
            },
        };
        observe(launch.cleanup_observer());
        launch
            .cleanup
            .authority
            .as_ref()
            .expect("owned cleanup")
            .root
            .validate()?;
        secure_state_directory(launch.state_directory())?;
        Ok(launch)
    }
    #[cfg(all(unix, feature = "test-harness"))]
    pub(crate) fn attach_transport(
        &mut self,
        transport: TransportAttachment,
    ) -> crate::domain::AppResult<()> {
        if self.transport.is_some() {
            return Err(crate::domain::AppError::invalid(
                "Launch already has a qualification transport",
            ));
        }
        transport.claim()?;
        self.transport = Some(transport);
        Ok(())
    }
    pub fn state_directory(&self) -> &std::path::Path {
        self.state_directory
            .as_ref()
            .expect("owned launch root")
            .as_path()
    }
    /// Capture before spawn to observe cleanup even if the spawn future is cancelled.
    pub fn cleanup_observer(&self) -> CleanupObserver {
        self.cleanup.clone()
    }
}
impl Drop for Launch {
    fn drop(&mut self) {
        #[cfg(all(unix, feature = "test-harness"))]
        if let Some(transport) = &self.transport {
            transport.seal();
        }
        if self.state_directory.take().is_some() {
            self.cleanup.finish(false, false);
        }
    }
}
#[cfg(unix)]
fn secure_state_directory(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}
#[cfg(not(unix))]
fn secure_state_directory(_path: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "private state directory policy is not qualified on this platform",
    ))
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupReport {
    /// False only when no child was ever successfully spawned.
    pub child_started: bool,
    pub reaped: bool,
    pub temporary_root_removed: bool,
}
#[derive(Clone)]
pub struct CleanupObserver {
    receiver: watch::Receiver<Option<CleanupReport>>,
    authority: Option<Arc<CleanupAuthority>>,
}
impl CleanupObserver {
    pub fn report(&self) -> Option<CleanupReport> {
        *self.receiver.borrow()
    }
    /// None means no completed cleanup receipt within the bound.
    pub async fn wait(&self, bound: Duration) -> Option<CleanupReport> {
        let mut receiver = self.receiver.clone();
        timeout(bound, async {
            loop {
                if let Some(report) = *receiver.borrow() {
                    return Some(report);
                }
                if receiver.changed().await.is_err() {
                    return None;
                }
            }
        })
        .await
        .ok()
        .flatten()
    }

    /// Wait for disposition and, when safe, retry this exact root once. A
    /// cancelled/timed-out caller never owns the blocking removal job: its
    /// retained authority publishes the eventual receipt independently.
    pub async fn settle(&self, bound: Duration) -> Option<CleanupReport> {
        timeout(bound, async {
            let mut receiver = self.receiver.clone();
            let report = loop {
                if let Some(report) = *receiver.borrow_and_update() {
                    break report;
                }
                if receiver.changed().await.is_err() {
                    return None;
                }
            };
            if report.temporary_root_removed || (report.child_started && !report.reaped) {
                return Some(report);
            }
            let Some(authority) = &self.authority else {
                return Some(report);
            };
            let ticket = authority.retry();
            loop {
                // Subscribe before checking completion; watch retains a change
                // published between this check and changed().
                drop(receiver.borrow_and_update());
                if authority.finished(ticket) {
                    return self.report();
                }
                if receiver.changed().await.is_err() {
                    return self.report();
                }
            }
        })
        .await
        .ok()
        .flatten()
    }

    fn finish(&self, child_started: bool, reaped: bool) {
        self.authority
            .as_ref()
            .expect("owned cleanup")
            .finish(child_started, reaped);
    }
}

#[derive(Default)]
struct CleanupState {
    report: Option<CleanupReport>,
    removing: bool,
    started: u64,
    finished: u64,
}

struct CleanupAuthority {
    root: OwnedRoot,
    state: Mutex<CleanupState>,
    updates: watch::Sender<Option<CleanupReport>>,
    #[cfg(test)]
    before_remove: Mutex<Option<BeforeRemove>>,
}

#[cfg(test)]
type BeforeRemove = Arc<dyn Fn() -> std::io::Result<()> + Send + Sync>;

impl CleanupAuthority {
    fn finish(self: &Arc<Self>, child_started: bool, reaped: bool) {
        let remove = {
            let mut state = self.state.lock().unwrap();
            debug_assert!(state.report.is_none(), "child disposition is final");
            state.report = Some(CleanupReport {
                child_started,
                reaped,
                temporary_root_removed: false,
            });
            let remove = !child_started || reaped;
            if remove {
                state.removing = true;
                state.started += 1;
            }
            if !remove {
                self.updates.send_replace(state.report);
            }
            remove
        };
        if remove {
            self.remove();
        }
    }

    fn retry(self: &Arc<Self>) -> u64 {
        let mut state = self.state.lock().unwrap();
        let safe = state.report.is_some_and(|report| {
            !report.temporary_root_removed && (!report.child_started || report.reaped)
        });
        if safe && !state.removing {
            state.removing = true;
            state.started += 1;
            let authority = self.clone();
            tokio::task::spawn_blocking(move || authority.remove());
        }
        state.started
    }

    fn finished(&self, ticket: u64) -> bool {
        self.state.lock().unwrap().finished >= ticket
    }

    fn remove(&self) {
        let removed = self.try_remove().is_ok();
        let mut state = self.state.lock().unwrap();
        if let Some(report) = &mut state.report {
            report.temporary_root_removed |= removed;
        }
        state.removing = false;
        state.finished = state.started;
        self.updates.send_replace(state.report);
    }

    fn try_remove(&self) -> std::io::Result<()> {
        #[cfg(test)]
        let before_remove = self.before_remove.lock().unwrap().clone();
        #[cfg(test)]
        if let Some(before_remove) = before_remove {
            before_remove()?;
        }
        self.root.remove()
    }
}

/// The root is selected natively and never accepted through IPC. Identity checks
/// protect ownership across retries; they are not a sandbox against same-user
/// concurrent mutations of the trusted native parent directory.
struct OwnedRoot {
    path: PathBuf,
    #[cfg(unix)]
    identity: Option<RootIdentity>,
}

#[cfg(unix)]
struct RootIdentity {
    directory: std::fs::File,
    parent: std::fs::File,
    parent_path: PathBuf,
    device: u64,
    inode: u64,
}

impl OwnedRoot {
    fn capture(path: PathBuf) -> Self {
        #[cfg(unix)]
        let identity = (|| -> std::io::Result<RootIdentity> {
            use std::os::unix::fs::MetadataExt;
            if !path.is_absolute() {
                return Err(invalid_root());
            }
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(invalid_root());
            }
            let directory = std::fs::File::open(&path)?;
            let opened = directory.metadata()?;
            if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
                return Err(invalid_root());
            }
            let parent_path = path.parent().ok_or_else(invalid_root)?.canonicalize()?;
            let parent = std::fs::File::open(&parent_path)?;
            Ok(RootIdentity {
                directory,
                parent,
                parent_path,
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        })()
        .ok();
        Self {
            path,
            #[cfg(unix)]
            identity,
        }
    }

    fn validate(&self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let identity = self.identity.as_ref().ok_or_else(invalid_root)?;
            let parent = std::fs::symlink_metadata(&identity.parent_path)?;
            let opened_parent = identity.parent.metadata()?;
            if !parent.is_dir()
                || parent.file_type().is_symlink()
                || parent.dev() != opened_parent.dev()
                || parent.ino() != opened_parent.ino()
            {
                return Err(invalid_root());
            }
            let metadata = std::fs::symlink_metadata(&self.path)?;
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.dev() == identity.device
                && metadata.ino() == identity.inode
            {
                return Ok(());
            }
        }
        Err(invalid_root())
    }

    fn remove(&self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let identity = self.identity.as_ref().ok_or_else(invalid_root)?;
            // An open handle distinguishes genuine removal from a renamed root
            // whose old pathname has disappeared or been reused.
            if identity.directory.metadata()?.nlink() == 0 {
                return Ok(());
            }
            self.validate()?;
            let result = std::fs::remove_dir_all(&self.path);
            // A successful removal of the identity-checked root is direct
            // evidence. APFS may retain a nonzero link count on an open handle
            // after rmdir; requiring nlink == 0 would fabricate a permanent
            // failure even though this operation removed the exact root.
            if result.is_ok() || identity.directory.metadata()?.nlink() == 0 {
                return Ok(());
            }
            result?;
        }
        Err(invalid_root())
    }
}

fn invalid_root() -> std::io::Error {
    std::io::Error::other("owned Snowflake state directory identity could not be confirmed")
}

#[derive(Debug)]
pub struct ShutdownReport {
    pub logout_acknowledged: bool,
    pub cleanup: Option<CleanupReport>,
}
struct SpawnCancellation {
    owner: Arc<Inner>,
    armed: bool,
}
impl Drop for SpawnCancellation {
    fn drop(&mut self) {
        if self.armed {
            self.owner.retire(Retirement::Dropped);
        }
    }
}

#[derive(Clone)]
pub struct Bounds {
    pub handshake: Duration,
    pub frame: Duration,
    pub write: Duration,
    pub exit: Duration,
}
impl Default for Bounds {
    fn default() -> Self {
        Self {
            handshake: Duration::from_secs(3),
            frame: Duration::from_secs(3),
            write: Duration::from_secs(3),
            exit: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Impact {
    Read,
    ContextChange,
    Begin,
    Mutating,
    Commit,
    Rollback,
    Control,
}
/// A fixed boundary command, never arbitrary SQL or a transaction-disposition claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionAction {
    Begin,
    Commit,
    Rollback,
}
impl TransactionAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Begin => "begin",
            Self::Commit => "commit",
            Self::Rollback => "rollback",
        }
    }
    pub fn impact(self) -> Impact {
        match self {
            Self::Begin => Impact::Begin,
            Self::Commit => Impact::Commit,
            Self::Rollback => Impact::Rollback,
        }
    }
    fn parse(value: &Value) -> Option<Self> {
        match value.as_str()? {
            "begin" => Some(Self::Begin),
            "commit" => Some(Self::Commit),
            "rollback" => Some(Self::Rollback),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryAcknowledgement {
    pub action: TransactionAction,
    pub statement: StatementIdentity,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DmlAcknowledgement {
    pub kind: SnowflakeDmlKind,
    pub statement: StatementIdentity,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DdlAcknowledgement {
    pub kind: SnowflakeDdlKind,
    pub statement: StatementIdentity,
}
/// Context observed from the original statement response, never a transaction grant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedContext {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub role: Option<String>,
    pub warehouse: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedAcknowledgement {
    pub statement: StatementIdentity,
    pub context: ReviewedContext,
}
fn parse_reviewed_context(value: &Value) -> Option<ReviewedContext> {
    let object = value.as_object()?;
    if object.len() != 4 {
        return None;
    }
    let field = |name| match object.get(name)? {
        Value::Null => Some(None),
        Value::String(value)
            if !value.is_empty()
                && value.len() <= 1020
                && value.chars().count() <= 255
                && !value.contains('\0') =>
        {
            Some(Some(value.clone()))
        }
        _ => None,
    };
    Some(ReviewedContext {
        database: field("database")?,
        schema: field("schema")?,
        role: field("role")?,
        warehouse: field("warehouse")?,
    })
}
fn streams_rows(op: &str) -> bool {
    matches!(op, "query" | "reviewedSql")
}
fn parse_dml_kind(value: &Value) -> Option<SnowflakeDmlKind> {
    serde_json::from_value(value.clone()).ok()
}
fn parse_ddl_kind(value: &Value) -> Option<SnowflakeDdlKind> {
    serde_json::from_value(value.clone()).ok()
}
fn valid_transaction_impact(op: &str, payload: &Value, impact: Impact) -> bool {
    if matches!(op, "mutationBatch" | "mutationFinish") {
        return impact == Impact::Mutating && BatchRequest::parse(op, payload).is_ok();
    }
    if op == "reviewedSql" {
        return impact == Impact::Mutating;
    }
    if op == "dml" {
        return impact == Impact::Mutating
            && payload.get("kind").and_then(parse_dml_kind).is_some();
    }
    if op == "ddl" {
        return impact == Impact::Mutating
            && payload.get("kind").and_then(parse_ddl_kind).is_some();
    }
    if op == "transaction" {
        payload
            .get("action")
            .and_then(TransactionAction::parse)
            .is_some_and(|action| action.impact() == impact)
    } else {
        !matches!(impact, Impact::Begin | Impact::Commit | Impact::Rollback)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Phase {
    Queued,
    Sending,
    Sent,
    Dispatching,
    Acknowledged,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Certainty {
    NotSubmitted,
    ReadInterrupted,
    OutcomeUnknown,
    StatementSucceeded,
    /// Reserved for independently verified disposition; never produced by this transport.
    Committed,
    /// Reserved for independently verified disposition; never produced by this transport.
    RolledBack,
    ServerTerminal,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retirement {
    Eof,
    Protocol,
    Write,
    Requested,
    Deadline,
    Dropped,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub ok: bool,
    pub certainty: Certainty,
    pub phase: Phase,
    pub result: Option<Value>,
    pub error_code: Option<String>,
    pub retirement: Option<Retirement>,
    pub statements: Vec<StatementIdentity>,
    /// The USE succeeded, independently of whether final context was verified.
    pub use_acknowledged: Option<StatementIdentity>,
    /// Fixed boundary command success, independent of drain or transaction disposition.
    pub boundary_acknowledged: Option<BoundaryAcknowledgement>,
    /// DML statement success, independently of count parsing or transaction effects.
    pub dml_acknowledged: Option<DmlAcknowledgement>,
    /// DDL statement success, independently of status-row drain or later catalog observation.
    pub ddl_acknowledged: Option<DdlAcknowledgement>,
    /// Original reviewed SQL success, independent of result retention or later failure.
    pub statement_acknowledged: Option<ReviewedAcknowledgement>,
    /// Dedicated batch identities/counts and disposition; never synthesized
    /// from ordinary statement events or lost when the process is retired.
    pub batch: Option<BatchEvidence>,
}
pub use super::snowflake_batch::StatementIdentity;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
}
pub type Rows = Vec<Vec<Option<String>>>;
pub struct RowBatch {
    pub rows: Rows,
    budget: Arc<AtomicUsize>,
}
impl Drop for RowBatch {
    fn drop(&mut self) {
        self.budget.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
enum TargetError {
    Inactive,
    Control(&'static str),
}
impl TargetError {
    fn message(self) -> &'static str {
        match self {
            Self::Inactive => INACTIVE_OPERATION,
            Self::Control(message) => message,
        }
    }
}

struct Pending {
    ticket: u64,
    impact: Impact,
    phase: Phase,
    op: String,
    accepted: bool,
    statements: Vec<StatementIdentity>,
    use_acknowledged: Option<StatementIdentity>,
    transaction_action: Option<TransactionAction>,
    boundary_acknowledged: Option<BoundaryAcknowledgement>,
    dml_kind: Option<SnowflakeDmlKind>,
    dml_acknowledged: Option<DmlAcknowledgement>,
    ddl_kind: Option<SnowflakeDdlKind>,
    ddl_acknowledged: Option<DdlAcknowledgement>,
    statement_acknowledged: Option<ReviewedAcknowledgement>,
    batch: Option<BatchEvidence>,
    statement_events: u8,
    wire_id: Option<String>,
    terminal: watch::Sender<Option<Report>>,
    columns: watch::Sender<Option<Vec<Column>>>,
    rows: mpsc::Sender<RowBatch>,
    width: Option<usize>,
    credits: u32,
    discard: bool,
    #[cfg(all(unix, feature = "test-harness"))]
    qualification: Option<PendingQualification>,
}
#[cfg(all(unix, feature = "test-harness"))]
struct PendingQualification {
    payload: Value,
    target: Option<(String, Option<u64>)>,
    candidate_wire_id: Option<String>,
}
struct LocalPublication {
    id: String,
    pending: Pending,
    report: Report,
    published: bool,
    #[cfg(all(unix, feature = "test-harness"))]
    origin: LocalTerminalOrigin,
    #[cfg(all(unix, feature = "test-harness"))]
    _guard: Option<PublicationGuard>,
}
#[derive(Default)]
struct State {
    retired: Option<Retirement>,
    wire_ids: HashMap<String, String>,
    next_wire_id: u64,
    pending: HashMap<String, Pending>,
    /// Retain known outcomes while the qualification callback runs outside State.
    /// Retirement drains pending operations, never these original terminals.
    #[cfg(all(unix, feature = "test-harness"))]
    finalizing: HashMap<(String, u64), Report>,
    connect_sent: bool,
    connected: bool,
    closing: bool,
}
struct Inner {
    generation: String,
    state: Mutex<State>,
    normal: mpsc::Sender<Outbound>,
    control: mpsc::Sender<Outbound>,
    stop: watch::Sender<bool>,
    exited: CleanupObserver,
    sequence: AtomicU64,
    buffered: Arc<AtomicUsize>,
    logout_acknowledged: AtomicBool,
    #[cfg(all(unix, feature = "test-harness"))]
    transport: Option<TransportAttachment>,
    #[cfg(all(unix, feature = "test-harness"))]
    finalizations: watch::Sender<u64>,
}
struct Outbound {
    id: String,
    ticket: u64,
    target: Option<(String, Option<u64>)>,
    bytes: Vec<u8>,
}
impl Drop for Outbound {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

pub struct Supervisor {
    inner: Arc<Inner>,
    pid: u32,
    bounds: Bounds,
}
#[path = "snowflake_process_retirement.rs"]
mod retirement;
pub use retirement::HelperRetirementObserver;

/// Opaque identity of one physical helper generation, without a liveness promise.
/// Retaining this handle keeps neither the helper nor its supervisor alive.
#[derive(Clone)]
pub struct HelperGeneration {
    owner: std::sync::Weak<Inner>,
    generation: String,
}
impl HelperGeneration {
    /// Canonical UUID for durable association; equality alone is not authority.
    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Compare physical owners for idempotent pinning without granting liveness.
    pub fn same_helper(&self, other: &Self) -> bool {
        self.generation == other.generation && std::sync::Weak::ptr_eq(&self.owner, &other.owner)
    }

    /// Match the exact physical owner, even when two helpers reuse a wire UUID.
    pub fn matches_reaped(&self, reaped: &ReapedHelperGeneration) -> bool {
        self.generation == reaped.generation && std::sync::Weak::ptr_eq(&self.owner, &reaped.owner)
    }
}

/// Proof of sealed local admission and actual reaping of this exact helper.
/// This says nothing about remote rollback, transaction disposition, or logout.
#[derive(Clone)]
pub struct ReapedHelperGeneration {
    owner: std::sync::Weak<Inner>,
    generation: String,
}
impl ReapedHelperGeneration {
    pub fn generation(&self) -> &str {
        &self.generation
    }
}

/// Opaque identity of one operation in one supervisor instance.
/// Cloning this token does not keep the child or operation alive.
#[derive(Clone)]
pub struct OperationToken {
    owner: std::sync::Weak<Inner>,
    id: String,
    ticket: u64,
}
impl std::fmt::Debug for OperationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationToken").finish_non_exhaustive()
    }
}
pub struct Operation {
    pub id: String,
    token: OperationToken,
    op: String,
    pub columns: watch::Receiver<Option<Vec<Column>>>,
    pub rows: mpsc::Receiver<RowBatch>,
    terminal: watch::Receiver<Option<Report>>,
    owner: Arc<Inner>,
}
impl Operation {
    pub fn token(&self) -> OperationToken {
        self.token.clone()
    }
    pub fn phase(&self) -> Phase {
        let state = self.owner.state.lock().unwrap();
        if let Some(report) = self.completion_evidence(&state) {
            return report.phase;
        }
        state
            .pending
            .get(&self.token.id)
            .filter(|p| p.ticket == self.token.ticket)
            .map(|p| p.phase)
            .unwrap_or(Phase::Queued)
    }
    pub fn report(&self) -> Option<Report> {
        self.terminal.borrow().clone()
    }
    /// Called under State so ACK access cannot fall between pending removal and
    /// terminal publication. This does not publish completion to the caller.
    fn completion_evidence(&self, state: &State) -> Option<Report> {
        let report = self.report();
        #[cfg(all(unix, feature = "test-harness"))]
        let report = report.or_else(|| {
            state
                .finalizing
                .get(&(self.token.id.clone(), self.token.ticket))
                .cloned()
        });
        let _ = state;
        report
    }
    /// Observed USE success, not verification of the final operation context.
    pub fn use_acknowledged(&self) -> Option<StatementIdentity> {
        let state = self.owner.state.lock().unwrap();
        if let Some(report) = self.completion_evidence(&state) {
            return report.use_acknowledged;
        }
        state
            .pending
            .get(&self.token.id)
            .filter(|pending| pending.ticket == self.token.ticket)
            .and_then(|pending| pending.use_acknowledged.clone())
    }
    /// Boundary statement ACK does not imply COMMIT/ROLLBACK disposition was observed.
    pub fn boundary_acknowledged(&self) -> Option<BoundaryAcknowledgement> {
        let state = self.owner.state.lock().unwrap();
        if let Some(report) = self.completion_evidence(&state) {
            return report.boundary_acknowledged;
        }
        state
            .pending
            .get(&self.token.id)
            .filter(|pending| pending.ticket == self.token.ticket)
            .and_then(|pending| pending.boundary_acknowledged.clone())
    }
    pub fn dml_acknowledged(&self) -> Option<DmlAcknowledgement> {
        let state = self.owner.state.lock().unwrap();
        if let Some(report) = self.completion_evidence(&state) {
            return report.dml_acknowledged;
        }
        state
            .pending
            .get(&self.token.id)
            .filter(|pending| pending.ticket == self.token.ticket)
            .and_then(|pending| pending.dml_acknowledged.clone())
    }
    pub fn ddl_acknowledged(&self) -> Option<DdlAcknowledgement> {
        let state = self.owner.state.lock().unwrap();
        if let Some(report) = self.completion_evidence(&state) {
            return report.ddl_acknowledged;
        }
        state
            .pending
            .get(&self.token.id)
            .filter(|pending| pending.ticket == self.token.ticket)
            .and_then(|pending| pending.ddl_acknowledged.clone())
    }
    pub fn statement_acknowledged(&self) -> Option<ReviewedAcknowledgement> {
        let state = self.owner.state.lock().unwrap();
        if let Some(report) = self.completion_evidence(&state) {
            return report.statement_acknowledged;
        }
        state
            .pending
            .get(&self.token.id)
            .filter(|pending| pending.ticket == self.token.ticket)
            .and_then(|pending| pending.statement_acknowledged.clone())
    }
    pub fn batch_evidence(&self) -> Option<BatchEvidence> {
        let state = self.owner.state.lock().unwrap();
        if let Some(report) = self.completion_evidence(&state) {
            return report.batch;
        }
        state
            .pending
            .get(&self.token.id)
            .filter(|pending| pending.ticket == self.token.ticket)
            .and_then(|pending| pending.batch.clone())
    }
    /// Observe completion while independently consuming columns and queued rows.
    /// A terminal report does not imply the row receiver has been drained.
    pub fn terminal_receiver(&self) -> watch::Receiver<Option<Report>> {
        self.terminal.clone()
    }
    pub async fn wait(&mut self) -> Report {
        loop {
            if let Some(report) = self.report() {
                return report;
            }
            if self.terminal.changed().await.is_err() {
                // Retirement always publishes first; fail closed if that invariant breaks.
                return Report {
                    ok: false,
                    certainty: Certainty::OutcomeUnknown,
                    phase: Phase::Sent,
                    result: None,
                    error_code: Some("connectionClosed".into()),
                    retirement: Some(Retirement::Protocol),
                    statements: vec![],
                    use_acknowledged: None,
                    boundary_acknowledged: None,
                    dml_acknowledged: None,
                    ddl_acknowledged: None,
                    statement_acknowledged: None,
                    batch: None,
                };
            }
        }
    }
    /// An inactive control target may already have an original terminal inside
    /// its observer. Wait for this exact operation's public watch; the private
    /// cache proves ownership but must not expose an outcome before callbacks.
    pub(crate) async fn wait_if_finalizing(&mut self) -> Option<Report> {
        let completing = {
            let state = self.owner.state.lock().unwrap();
            self.completion_evidence(&state).is_some()
        };
        if completing {
            Some(self.wait().await)
        } else {
            None
        }
    }
    pub async fn wait_bounded(&mut self, limit: Duration) -> Report {
        match timeout(limit, self.wait()).await {
            Ok(report) => report,
            Err(_) => {
                self.owner.retire(Retirement::Deadline);
                self.wait().await
            }
        }
    }
}
impl Operation {
    fn cleanup_unfinished(&self) {
        if matches!(
            self.op.as_str(),
            "connect"
                | "context"
                | "switchSchema"
                | "transaction"
                | "dml"
                | "ddl"
                | "reviewedSql"
                | "mutationBatch"
                | "mutationFinish"
                | "close"
        ) {
            self.owner.retire_target(&self.token, Retirement::Dropped);
            return;
        }
        if self.op != "query" {
            return;
        }
        // Do not strand an active stream when its consumer disappears.
        match self.owner.discard(&self.token) {
            Ok(_) | Err(TargetError::Inactive) => {}
            Err(TargetError::Control(_)) => self.owner.retire(Retirement::Dropped),
        }
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        if self.report().is_none() {
            self.cleanup_unfinished();
        }
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        self.inner.retire(Retirement::Dropped);
    }
}

impl Supervisor {
    pub async fn spawn(
        mut launch: Launch,
        generation: &str,
        bounds: Bounds,
    ) -> Result<Self, &'static str> {
        if !valid_generation(generation)
            || !launch.executable.is_absolute()
            || !launch.working_directory.is_absolute()
            || launch.script.as_ref().is_some_and(|p| !p.is_absolute())
        {
            return Err("explicit absolute paths and UUID generation are required");
        }
        let mut command = Command::new(&launch.executable);
        if let Some(script) = &launch.script {
            command.arg(script);
        }
        #[cfg(all(unix, feature = "test-harness"))]
        if let Some(transport) = &launch.transport {
            transport
                .check()
                .map_err(|_| "qualification transport rejected launch")?;
        }
        #[cfg(all(unix, feature = "test-harness"))]
        let stderr = if launch.transport.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        };
        #[cfg(not(all(unix, feature = "test-harness")))]
        let stderr = Stdio::null();
        let mut child = command
            .current_dir(&launch.working_directory)
            .env_clear()
            .env("JOINERY_SNOWFLAKE_STATE_DIR", launch.state_directory())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "helper could not start")?;
        let pid = child.id();
        // Disable automatic deletion before transferring ownership to an async task:
        // task/runtime cancellation must leave a remediable root, not delete it
        // before actual reaping has been confirmed.
        let _state_directory = launch.state_directory.take().expect("owned launch root");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        #[cfg(all(unix, feature = "test-harness"))]
        let stderr = child.stderr.take();
        let (normal, normal_rx) = mpsc::channel(8);
        let (control, control_rx) = mpsc::channel(16);
        let (stop, mut stopping) = watch::channel(false);
        let inner = Arc::new(Inner {
            generation: generation.to_owned(),
            state: Mutex::new(State::default()),
            normal,
            control,
            stop,
            exited: launch.cleanup.clone(),
            sequence: AtomicU64::new(0),
            buffered: Arc::new(AtomicUsize::new(0)),
            logout_acknowledged: AtomicBool::new(false),
            #[cfg(all(unix, feature = "test-harness"))]
            transport: launch.transport.clone(),
            #[cfg(all(unix, feature = "test-harness"))]
            finalizations: watch::channel(0).0,
        });
        #[cfg(all(unix, feature = "test-harness"))]
        let (stdout_guard, writer_guard) = if let Some(transport) = &inner.transport {
            let stdout_guard = transport.start(Stream::Stdout);
            let writer_guard = transport.start(Stream::Writer);
            if let Some(mut stderr) = stderr {
                let guard = transport.start(Stream::Stderr);
                let attachment = transport.clone();
                let owner = inner.clone();
                tokio::spawn(async move {
                    let _guard = guard;
                    let mut stopping = owner.stop.subscribe();
                    if *stopping.borrow() {
                        return;
                    }
                    tokio::select! {
                        _ = stopping.changed() => {},
                        result = attachment.drain_stderr(&mut stderr) => {
                            if result.is_err() { owner.retire(Retirement::Protocol); }
                        }
                    }
                });
            } else {
                transport.stream_failure(Stream::Stderr, Disposition::Io);
                inner.retire(Retirement::Protocol);
            }
            (Some(stdout_guard), Some(writer_guard))
        } else {
            (None, None)
        };
        let owner = inner.clone();
        // Child and private root transfer before the first await in spawn. The reaper
        // survives cancelled handshakes and bounded caller timeouts.
        let cleanup = launch.cleanup.clone();
        tokio::spawn(async move {
            let reaped = if *stopping.borrow() {
                let _ = child.start_kill();
                child.wait().await.is_ok()
            } else {
                tokio::select! {
                    result = child.wait() => { owner.retire(Retirement::Eof); result.is_ok() }
                    _ = stopping.changed() => {
                        let _ = child.start_kill();
                        child.wait().await.is_ok()
                    }
                }
            };
            // Never delete a root while its child might still be using it. On a wait
            // error retain the path, publish explicit failure, and leave remediation
            // to the application cleanup registry rather than claim success.
            cleanup.finish(true, reaped);
        });
        let mut cancellation = SpawnCancellation {
            owner: inner.clone(),
            armed: true,
        };
        let Some(pid) = pid else {
            return Err("helper has no process id");
        };
        let (Some(stdin), Some(mut stdout)) = (stdin, stdout) else {
            return Err("helper pipes missing");
        };
        let hello = timeout(
            bounds.handshake,
            observed_frame(&inner, &mut stdout, bounds.frame),
        )
        .await;
        if !matches!(hello, Ok(Ok(Some(ref value))) if valid_hello(value)) {
            #[cfg(all(unix, feature = "test-harness"))]
            if let Some(transport) = &inner.transport {
                transport.stream_failure(
                    Stream::Stdout,
                    if hello.is_err() {
                        Disposition::Deadline
                    } else {
                        Disposition::Malformed
                    },
                );
            }
            inner.retire(Retirement::Protocol);
            inner.exited.wait(bounds.exit).await;
            return Err("helper handshake failed");
        }
        let writer_owner = inner.clone();
        let write_bound = bounds.write;
        tokio::spawn(async move {
            #[cfg(all(unix, feature = "test-harness"))]
            let _guard = writer_guard;
            write_loop(writer_owner, stdin, normal_rx, control_rx, write_bound).await;
        });
        let owner = inner.clone();
        let frame_bound = bounds.frame;
        tokio::spawn(async move {
            #[cfg(all(unix, feature = "test-harness"))]
            let _guard = stdout_guard;
            let mut stopping = owner.stop.subscribe();
            loop {
                if *stopping.borrow() {
                    return;
                }
                let frame = tokio::select! { _ = stopping.changed() => return,
                frame = observed_frame(&owner, &mut stdout, frame_bound) => frame };
                match frame {
                    Ok(Some(event)) => {
                        if !owner.event(event) {
                            #[cfg(all(unix, feature = "test-harness"))]
                            if let Some(transport) = &owner.transport {
                                transport.stream_failure(Stream::Stdout, Disposition::Malformed);
                            }
                            owner.retire(Retirement::Protocol);
                            return;
                        }
                    }
                    Ok(None) => {
                        owner.retire(Retirement::Eof);
                        return;
                    }
                    Err(_) => {
                        owner.retire(Retirement::Protocol);
                        return;
                    }
                }
            }
        });
        cancellation.armed = false;
        Ok(Self { inner, pid, bounds })
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub fn is_closed(&self) -> bool {
        let state = self.inner.state.lock().unwrap();
        state.retired.is_some() || state.closing
    }
    pub fn helper_generation(&self) -> HelperGeneration {
        HelperGeneration {
            owner: Arc::downgrade(&self.inner),
            generation: uuid::Uuid::parse_str(&self.inner.generation)
                .expect("supervisor generation was validated before spawn")
                .to_string(),
        }
    }
    /// A logical close, cancellation request, or public cleanup value cannot
    /// mint this proof. Only this supervisor's private reaper receipt can.
    pub fn reaped_generation(&self) -> Option<ReapedHelperGeneration> {
        let state = self.inner.state.lock().unwrap();
        if !(state.retired.is_some() || state.closing)
            || !self
                .inner
                .exited
                .report()
                .is_some_and(|report| report.child_started && report.reaped)
        {
            return None;
        }
        let identity = self.helper_generation();
        Some(ReapedHelperGeneration {
            owner: identity.owner,
            generation: identity.generation,
        })
    }
    /// Low-level trusted-driver API. Typed transaction boundaries are transport
    /// primitives; their availability grants no application write capability.
    /// Impact is lifecycle evidence, never a capability grant.
    pub fn request(
        &self,
        id: &str,
        op: &str,
        payload: Value,
        impact: Impact,
    ) -> Result<Operation, &'static str> {
        if !valid_transaction_impact(op, &payload, impact) {
            return Err("invalid transaction impact");
        }
        if (op == "context" && impact != Impact::Read)
            || (op == "connect" && impact != Impact::Control)
            || ((op == "switchSchema") != (impact == Impact::ContextChange))
        {
            return Err("invalid non-statement impact");
        }
        if matches!(op, "credit" | "cancel" | "close") {
            return Err("use dedicated control methods");
        }
        self.inner.request(id, op, payload, impact)
    }
    pub fn credit(&self, target: &OperationToken, batches: u32) -> Result<Operation, &'static str> {
        if batches == 0 || batches > MAX_CREDITS {
            return Err("invalid credit bound");
        }
        self.inner.validate_owner(target)?;
        let ticket = {
            let mut state = self.inner.state.lock().unwrap();
            let outstanding: u32 = state.pending.values().map(|p| p.credits).sum();
            if outstanding + batches + self.inner.buffered.load(Ordering::SeqCst) as u32
                > MAX_CREDITS
            {
                return Err("global row credit bound");
            }
            let pending = state
                .pending
                .get_mut(&target.id)
                .filter(|pending| pending.ticket == target.ticket)
                .ok_or(INACTIVE_OPERATION)?;
            if pending.phase == Phase::Queued {
                return Err("query has not been transmitted");
            }
            if !streams_rows(&pending.op)
                || pending.discard
                || pending.credits + batches > MAX_CREDITS
            {
                return Err("invalid credit target");
            }
            pending.credits += batches;
            pending.ticket
        };
        let result = self.inner.targeted_control(
            "credit",
            &target.id,
            ticket,
            json!({"target":target.id,"batches":batches,"discard":false}),
        );
        if result.is_err() {
            self.inner.retire(Retirement::Write);
        }
        result
    }
    pub fn discard(&self, target: &OperationToken) -> Result<Operation, &'static str> {
        self.inner.discard(target).map_err(TargetError::message)
    }
    pub fn cancel(&self, target: &OperationToken) -> Result<Operation, &'static str> {
        self.inner.cancel(target)
    }
    /// Close acknowledgement is not evidence of rollback. Always reap the child.
    pub async fn close(&self) -> Option<Report> {
        let decision = {
            let mut state = self.inner.state.lock().unwrap();
            if state.closing || state.retired.is_some() {
                None
            } else {
                state.closing = true;
                let never_sent = !state.connect_sent
                    || state
                        .pending
                        .values()
                        .any(|p| p.op == "connect" && p.phase == Phase::Queued);
                let mut publications = Vec::new();
                if !never_sent {
                    let queued: Vec<_> = state
                        .pending
                        .iter()
                        .filter(|(_, p)| {
                            p.phase == Phase::Queued
                                && matches!(
                                    p.op.as_str(),
                                    "query"
                                        | "context"
                                        | "switchSchema"
                                        | "transaction"
                                        | "dml"
                                        | "ddl"
                                        | "reviewedSql"
                                        | "mutationBatch"
                                        | "mutationFinish"
                                )
                        })
                        .map(|(id, _)| id.clone())
                        .collect();
                    for id in queued {
                        let pending = state.pending.remove(&id).unwrap();
                        let report = Report {
                            ok: false,
                            certainty: Certainty::NotSubmitted,
                            phase: Phase::Queued,
                            result: None,
                            error_code: Some("connectionClosed".into()),
                            retirement: None,
                            statements: pending.statements.clone(),
                            use_acknowledged: pending.use_acknowledged.clone(),
                            boundary_acknowledged: pending.boundary_acknowledged.clone(),
                            dml_acknowledged: pending.dml_acknowledged.clone(),
                            ddl_acknowledged: pending.ddl_acknowledged.clone(),
                            statement_acknowledged: pending.statement_acknowledged.clone(),
                            batch: pending.batch.clone(),
                        };
                        publications.push(self.inner.stage_local(
                            &mut state,
                            id,
                            pending,
                            report,
                            #[cfg(all(unix, feature = "test-harness"))]
                            LocalTerminalOrigin::QueuedClose,
                        ));
                    }
                }
                Some((never_sent, publications))
            }
        };
        let Some((never_sent, publications)) = decision else {
            self.wait_exited().await;
            return None;
        };
        for publication in publications {
            self.inner.publish_local(publication);
        }
        if never_sent {
            self.inner.retire(Retirement::Requested);
            self.wait_exited().await;
            return Some(local_report(
                json!({"closed":true,"logoutAcknowledged":false}),
            ));
        }
        let report = match self.inner.control("close", json!({})) {
            Ok(mut operation) => timeout(self.bounds.exit, operation.wait()).await.ok(),
            Err(_) => None,
        };
        self.inner.retire(Retirement::Requested);
        self.wait_exited().await;
        report
    }
    pub fn cleanup_observer(&self) -> CleanupObserver {
        self.inner.exited.clone()
    }
    /// Immediately stop admission and request termination without a runtime on
    /// the calling thread. Await the cleanup observer on the owning runtime;
    /// this request itself proves neither process exit nor remote rollback.
    pub fn request_retirement(&self) {
        self.inner.retire(Retirement::Requested);
    }
    pub async fn shutdown(&self) -> ShutdownReport {
        self.close().await;
        let cleanup = *self.inner.exited.receiver.borrow();
        ShutdownReport {
            logout_acknowledged: self.inner.logout_acknowledged.load(Ordering::SeqCst),
            cleanup,
        }
    }
    pub async fn wait_exited(&self) -> bool {
        self.inner
            .exited
            .wait(self.bounds.exit + self.bounds.exit)
            .await
            .is_some_and(|report| report.reaped)
    }
}

impl Inner {
    fn stage_local(
        &self,
        state: &mut State,
        id: String,
        pending: Pending,
        report: Report,
        #[cfg(all(unix, feature = "test-harness"))] origin: LocalTerminalOrigin,
    ) -> LocalPublication {
        #[cfg(not(all(unix, feature = "test-harness")))]
        let _ = state;
        #[cfg(all(unix, feature = "test-harness"))]
        let guard = self.transport.as_ref().map(|transport| {
            state
                .finalizing
                .insert((id.clone(), pending.ticket), report.clone());
            transport.publication()
        });
        #[cfg(all(unix, feature = "test-harness"))]
        let published = self.transport.is_none();
        #[cfg(not(all(unix, feature = "test-harness")))]
        let published = true;
        if published {
            // Without an observer preserve the original atomic ACK visibility:
            // removal and watch publication both happen under process State.
            pending.terminal.send_replace(Some(report.clone()));
        }
        LocalPublication {
            id,
            pending,
            report,
            published,
            #[cfg(all(unix, feature = "test-harness"))]
            origin,
            #[cfg(all(unix, feature = "test-harness"))]
            _guard: guard,
        }
    }
    fn publish_local(&self, publication: LocalPublication) {
        if publication.published {
            return;
        }
        let LocalPublication {
            id,
            pending,
            report,
            published: _,
            #[cfg(all(unix, feature = "test-harness"))]
            origin,
            #[cfg(all(unix, feature = "test-harness"))]
            _guard,
        } = publication;
        #[cfg(all(unix, feature = "test-harness"))]
        if let Some(transport) = &self.transport {
            let qualification = pending
                .qualification
                .as_ref()
                .expect("attached pending operation metadata");
            transport.local_terminal(LocalTerminal {
                generation: &self.generation,
                operation_id: &id,
                ticket: pending.ticket,
                wire_id: pending.wire_id.as_deref(),
                candidate_wire_id: qualification.candidate_wire_id.as_deref(),
                operation: &pending.op,
                impact: pending.impact,
                target: qualification
                    .target
                    .as_ref()
                    .map(|(target, ticket)| (target.as_str(), *ticket)),
                payload: &qualification.payload,
                origin,
                report: &report,
            });
        }
        pending.terminal.send_replace(Some(report));
        #[cfg(all(unix, feature = "test-harness"))]
        if self.transport.is_some() {
            self.state
                .lock()
                .unwrap()
                .finalizing
                .remove(&(id, pending.ticket));
            self.finalizations
                .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
        }
        #[cfg(not(all(unix, feature = "test-harness")))]
        let _ = id;
        // The guard outlives callback, watch publication, and ACK-cache removal.
    }
    async fn publish_inactive(&self, publication: LocalPublication, bound: Duration) {
        #[cfg(all(unix, feature = "test-harness"))]
        if let Some(target) = publication
            .pending
            .qualification
            .as_ref()
            .and_then(|q| q.target.as_ref())
            .and_then(|(id, ticket)| ticket.map(|ticket| (id.clone(), ticket)))
        {
            // Target removal precedes its helper observer. A late local control
            // must wait for that exact observer/cache publication, rather than
            // race the owning policy's successful-target proof. Keep the local
            // report and settlement guard owned even if stop becomes ready.
            let mut updates = self.finalizations.subscribe();
            let _ = timeout(bound, async {
                loop {
                    if !self.state.lock().unwrap().finalizing.contains_key(&target) {
                        break;
                    }
                    if updates.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
        }
        #[cfg(not(all(unix, feature = "test-harness")))]
        let _ = bound;
        self.publish_local(publication);
    }
    fn control(self: &Arc<Self>, op: &str, payload: Value) -> Result<Operation, &'static str> {
        let id = format!("control-{}", self.sequence.fetch_add(1, Ordering::Relaxed));
        self.request(&id, op, payload, Impact::Control)
    }
    fn local_ack(self: &Arc<Self>, result: Value) -> Operation {
        let (terminal, terminal_rx) = watch::channel(Some(local_report(result)));
        drop(terminal);
        let (_, columns) = watch::channel(None);
        let (_, rows) = mpsc::channel(1);
        let ticket = self.sequence.fetch_add(1, Ordering::Relaxed);
        let id = format!("local-{ticket}");
        Operation {
            token: OperationToken {
                owner: Arc::downgrade(self),
                id: id.clone(),
                ticket,
            },
            id,
            op: "cancel".into(),
            columns,
            rows,
            terminal: terminal_rx,
            owner: self.clone(),
        }
    }
    fn validate_owner(self: &Arc<Self>, token: &OperationToken) -> Result<(), &'static str> {
        if !std::sync::Weak::ptr_eq(&token.owner, &Arc::downgrade(self)) {
            return Err("operation token belongs to another supervisor");
        }
        Ok(())
    }
    fn prepare_target(
        self: &Arc<Self>,
        target: &OperationToken,
        allow_connect: bool,
        discard: bool,
    ) -> Result<(Option<u64>, bool), TargetError> {
        self.validate_owner(target).map_err(TargetError::Control)?;
        let mut state = self.state.lock().unwrap();
        let pending = state
            .pending
            .get_mut(&target.id)
            .filter(|pending| pending.ticket == target.ticket)
            .ok_or(TargetError::Inactive)?;
        let is_connect = pending.op == "connect";
        if !streams_rows(&pending.op)
            && !(allow_connect
                && (is_connect
                    || matches!(
                        pending.op.as_str(),
                        "switchSchema"
                            | "transaction"
                            | "dml"
                            | "ddl"
                            | "mutationBatch"
                            | "mutationFinish"
                    )))
        {
            return Err(TargetError::Control("invalid query target"));
        }
        if pending.phase != Phase::Queued {
            pending.discard |= discard;
            return Ok((Some(pending.ticket), is_connect));
        }
        let pending = state.pending.remove(&target.id).unwrap();
        let report = Report {
            ok: false,
            certainty: Certainty::NotSubmitted,
            phase: Phase::Queued,
            result: None,
            error_code: Some("cancelled".into()),
            retirement: None,
            statements: pending.statements.clone(),
            use_acknowledged: pending.use_acknowledged.clone(),
            boundary_acknowledged: pending.boundary_acknowledged.clone(),
            dml_acknowledged: pending.dml_acknowledged.clone(),
            ddl_acknowledged: pending.ddl_acknowledged.clone(),
            statement_acknowledged: pending.statement_acknowledged.clone(),
            batch: pending.batch.clone(),
        };
        let publication = self.stage_local(
            &mut state,
            target.id.clone(),
            pending,
            report,
            #[cfg(all(unix, feature = "test-harness"))]
            LocalTerminalOrigin::QueuedCancellation,
        );
        drop(state);
        self.publish_local(publication);
        Ok((None, is_connect))
    }
    fn targeted_control(
        self: &Arc<Self>,
        op: &str,
        target: &str,
        ticket: u64,
        payload: Value,
    ) -> Result<Operation, &'static str> {
        let id = format!("control-{}", self.sequence.fetch_add(1, Ordering::Relaxed));
        self.request_inner(
            &id,
            op,
            payload,
            Impact::Control,
            Some((target.to_owned(), Some(ticket))),
        )
    }
    fn cancel(self: &Arc<Self>, target: &OperationToken) -> Result<Operation, &'static str> {
        let (ticket, is_connect) = self
            .prepare_target(target, true, false)
            .map_err(TargetError::message)?;
        match ticket {
            None => {
                if is_connect {
                    self.retire(Retirement::Requested);
                }
                Ok(self.local_ack(json!({"cancelledLocally":true})))
            }
            Some(ticket) => {
                self.targeted_control("cancel", &target.id, ticket, json!({"target":target.id}))
            }
        }
    }
    fn discard(self: &Arc<Self>, target: &OperationToken) -> Result<Operation, TargetError> {
        match self.prepare_target(target, false, true)?.0 {
            None => Ok(self.local_ack(json!({"discardedLocally":true}))),
            Some(ticket) => self
                .targeted_control(
                    "credit",
                    &target.id,
                    ticket,
                    json!({"target":target.id,"batches":0,"discard":true}),
                )
                .map_err(TargetError::Control),
        }
    }
    fn request(
        self: &Arc<Self>,
        id: &str,
        op: &str,
        payload: Value,
        impact: Impact,
    ) -> Result<Operation, &'static str> {
        self.request_inner(id, op, payload, impact, None)
    }
    fn request_inner(
        self: &Arc<Self>,
        id: &str,
        op: &str,
        payload: Value,
        impact: Impact,
        target_binding: Option<(String, Option<u64>)>,
    ) -> Result<Operation, &'static str> {
        if id.is_empty()
            || id.len() > 128
            || !matches!(
                op,
                "connect"
                    | "query"
                    | "context"
                    | "switchSchema"
                    | "transaction"
                    | "dml"
                    | "ddl"
                    | "reviewedSql"
                    | "mutationBatch"
                    | "mutationFinish"
                    | "credit"
                    | "cancel"
                    | "close"
            )
            || !payload.is_object()
        {
            return Err("invalid request");
        }
        if !valid_transaction_impact(op, &payload, impact) {
            return Err("invalid transaction impact");
        }
        if (op == "switchSchema") != (impact == Impact::ContextChange) {
            return Err("invalid context-change impact");
        }
        let batch = BatchRequest::parse(op, &payload)
            .map_err(|_| "invalid mutation batch payload")?
            .map(BatchEvidence::new);
        let priority = matches!(op, "credit" | "cancel" | "close");
        if priority && impact != Impact::Control {
            return Err("invalid control impact");
        }
        let credits = if streams_rows(op) {
            payload.get("credits").and_then(Value::as_u64).unwrap_or(0)
        } else {
            0
        };
        if credits > u64::from(MAX_CREDITS) || (op == "reviewedSql" && credits == 0) {
            return Err("invalid credit bound");
        }
        let target = payload
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let transaction_action = if op == "transaction" {
            payload.get("action").and_then(TransactionAction::parse)
        } else {
            None
        };
        let dml_kind = if op == "dml" {
            payload.get("kind").and_then(parse_dml_kind)
        } else {
            None
        };
        let ddl_kind = if op == "ddl" {
            payload.get("kind").and_then(parse_ddl_kind)
        } else {
            None
        };
        #[cfg(all(unix, feature = "test-harness"))]
        let qualification_payload = self.transport.as_ref().map(|_| payload.clone());
        let bytes = serde_json::to_vec(
            &json!({"v":1,"generation":self.generation,"id":id,"op":op,"payload":payload}),
        )
        .map_err(|_| "invalid request")?;
        if bytes.len() > MAX_FRAME {
            return Err("request exceeds frame bound");
        }
        let (terminal, terminal_rx) = watch::channel(None);
        let (columns, column_rx) = watch::channel(None);
        let (rows, rows_rx) = mpsc::channel(MAX_CREDITS as usize);
        let mut state = self.state.lock().unwrap();
        if state.retired.is_some() || (state.closing && !priority) {
            return Err("helper is retired or closing");
        }
        if state.pending.len() >= 32 {
            return Err("pending request bound");
        }
        let outstanding: u32 = state.pending.values().map(|p| p.credits).sum();
        if outstanding + credits as u32 + self.buffered.load(Ordering::SeqCst) as u32 > MAX_CREDITS
        {
            return Err("global row credit bound");
        }
        if state.pending.contains_key(id) {
            return Err("request id is active");
        }
        if op == "connect" {
            if state.connect_sent {
                return Err("connection cannot reconnect");
            }
            state.connect_sent = true;
        } else if !priority && !state.connected {
            return Err("connection is not ready");
        }
        let target = target_binding.or_else(|| {
            target.map(|target| {
                let ticket = state.pending.get(&target).map(|p| p.ticket);
                (target, ticket)
            })
        });
        let ticket = self.sequence.fetch_add(1, Ordering::Relaxed);
        state.pending.insert(
            id.to_owned(),
            Pending {
                ticket,
                impact,
                phase: Phase::Queued,
                op: op.into(),
                accepted: false,
                statements: vec![],
                use_acknowledged: None,
                boundary_acknowledged: None,
                dml_acknowledged: None,
                dml_kind,
                ddl_acknowledged: None,
                statement_acknowledged: None,
                batch,
                ddl_kind,
                transaction_action,
                statement_events: 0,
                wire_id: None,
                terminal,
                columns,
                rows,
                width: None,
                credits: credits as u32,
                discard: false,
                #[cfg(all(unix, feature = "test-harness"))]
                qualification: qualification_payload.map(|payload| PendingQualification {
                    payload,
                    target: target.clone(),
                    candidate_wire_id: None,
                }),
            },
        );
        if (if priority {
            &self.control
        } else {
            &self.normal
        })
        .try_send(Outbound {
            id: id.into(),
            ticket,
            target,
            bytes,
        })
        .is_err()
        {
            state.pending.remove(id);
            return Err("helper queue is full");
        }
        Ok(Operation {
            token: OperationToken {
                owner: Arc::downgrade(self),
                id: id.into(),
                ticket,
            },
            id: id.into(),
            op: op.into(),
            columns: column_rx,
            rows: rows_rx,
            terminal: terminal_rx,
            owner: self.clone(),
        })
    }
    fn retire(&self, reason: Retirement) {
        let publications = {
            let mut state = self.state.lock().unwrap();
            self.retire_locked(&mut state, reason)
        };
        for publication in publications {
            self.publish_local(publication);
        }
    }
    fn retire_target(&self, target: &OperationToken, reason: Retirement) {
        let publications = {
            let mut state = self.state.lock().unwrap();
            if state
                .pending
                .get(&target.id)
                .is_some_and(|p| p.ticket == target.ticket)
            {
                self.retire_locked(&mut state, reason)
            } else {
                Vec::new()
            }
        };
        for publication in publications {
            self.publish_local(publication);
        }
    }
    fn retire_locked(&self, state: &mut State, reason: Retirement) -> Vec<LocalPublication> {
        if state.retired.is_some() {
            return Vec::new();
        }
        state.retired = Some(reason);
        let pending_operations = state.pending.drain().collect::<Vec<_>>();
        let mut publications = Vec::with_capacity(pending_operations.len());
        for (id, pending) in pending_operations {
            let certainty = if pending.boundary_acknowledged.is_some()
                || pending.dml_acknowledged.is_some()
                || pending.ddl_acknowledged.is_some()
                || pending.statement_acknowledged.is_some()
            {
                Certainty::StatementSucceeded
            } else {
                interrupted_certainty(pending.impact, pending.phase)
            };
            let report = Report {
                ok: false,
                certainty,
                phase: pending.phase,
                result: None,
                error_code: Some(
                    if certainty == Certainty::OutcomeUnknown {
                        "outcomeUnknown"
                    } else {
                        "connectionClosed"
                    }
                    .into(),
                ),
                retirement: Some(reason),
                statements: pending.statements.clone(),
                use_acknowledged: pending.use_acknowledged.clone(),
                boundary_acknowledged: pending.boundary_acknowledged.clone(),
                dml_acknowledged: pending.dml_acknowledged.clone(),
                ddl_acknowledged: pending.ddl_acknowledged.clone(),
                statement_acknowledged: pending.statement_acknowledged.clone(),
                batch: pending.batch.clone(),
            };
            publications.push(self.stage_local(
                state,
                id,
                pending,
                report,
                #[cfg(all(unix, feature = "test-harness"))]
                LocalTerminalOrigin::Retirement,
            ));
        }
        state.wire_ids.clear();
        self.stop.send_replace(true);
        publications
    }
    fn event(&self, event: Value) -> bool {
        if event.get("v").and_then(Value::as_u64) != Some(1)
            || event.get("generation").and_then(Value::as_str) != Some(self.generation.as_str())
        {
            return false;
        }
        let Some(id) = event.get("id").and_then(Value::as_str) else {
            return false;
        };
        let mut state = self.state.lock().unwrap();
        if state.retired.is_some() {
            return true;
        }
        let Some(logical_id) = state.wire_ids.get(id).cloned() else {
            return false;
        };
        let Some(pending) = state.pending.get_mut(&logical_id) else {
            return false;
        };
        if pending.phase == Phase::Queued {
            return false;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("accepted") => {
                if pending.accepted {
                    return false;
                }
                pending.accepted = true;
                true
            }
            Some("batchStep" | "batchStatement" | "batchCount" | "batchCountUnavailable") => {
                if !pending.accepted || pending.phase < Phase::Sent {
                    return false;
                }
                let Some(batch) = &mut pending.batch else {
                    return false;
                };
                if !batch.apply_event(&event) {
                    return false;
                }
                pending.phase = Phase::Dispatching;
                true
            }
            Some("phase") if pending.batch.is_some() => false,
            Some("phase") => match event.get("phase").and_then(Value::as_str) {
                Some("dispatching")
                    if pending.accepted
                        && matches!(
                            pending.op.as_str(),
                            "query"
                                | "context"
                                | "switchSchema"
                                | "transaction"
                                | "dml"
                                | "ddl"
                                | "reviewedSql"
                        )
                        && pending.phase < Phase::Dispatching =>
                {
                    pending.phase = Phase::Dispatching;
                    true
                }
                Some("acknowledged") if pending.phase == Phase::Dispatching => {
                    if !validate_latest_statement(pending, &event) {
                        return false;
                    }
                    if pending.op == "switchSchema"
                        && (pending.use_acknowledged.is_none()
                            || pending.statements.len() != 2
                            || event.get("sdkRequestId").and_then(Value::as_str).is_none())
                    {
                        return false;
                    }
                    if pending.op == "transaction"
                        && (pending.boundary_acknowledged.is_none()
                            || pending.statements.len() != 1
                            || event.get("sdkRequestId").and_then(Value::as_str).is_none())
                    {
                        return false;
                    }
                    if pending.op == "dml"
                        && (pending.dml_acknowledged.is_none()
                            || pending.statements.len() != 1
                            || event.get("sdkRequestId").and_then(Value::as_str).is_none())
                    {
                        return false;
                    }
                    if pending.op == "ddl"
                        && (pending.ddl_acknowledged.is_none()
                            || pending.statements.len() != 1
                            || event.get("sdkRequestId").and_then(Value::as_str).is_none())
                    {
                        return false;
                    }
                    if pending.op == "reviewedSql"
                        && (pending.statement_acknowledged.is_none()
                            || pending.statements.len() != 1
                            || event.get("sdkRequestId").and_then(Value::as_str).is_none())
                    {
                        return false;
                    }
                    pending.phase = Phase::Acknowledged;
                    true
                }
                _ => false,
            },
            Some("boundaryAcknowledged") => {
                if pending.op != "transaction"
                    || !pending.accepted
                    || pending.phase != Phase::Dispatching
                    || pending.boundary_acknowledged.is_some()
                    || pending.statements.len() != 1
                    || event.get("sdkRequestId").and_then(Value::as_str).is_none()
                    || event.get("action").and_then(TransactionAction::parse)
                        != pending.transaction_action
                    || !event.as_object().is_some_and(|object| {
                        object.keys().all(|key| {
                            matches!(
                                key.as_str(),
                                "v" | "generation"
                                    | "id"
                                    | "type"
                                    | "action"
                                    | "sdkRequestId"
                                    | "queryId"
                            )
                        })
                    })
                    || !validate_latest_statement(pending, &event)
                {
                    return false;
                }
                pending.boundary_acknowledged = Some(BoundaryAcknowledgement {
                    action: pending.transaction_action.unwrap(),
                    statement: pending.statements[0].clone(),
                });
                true
            }
            Some("dmlAcknowledged") => {
                if pending.op != "dml"
                    || !pending.accepted
                    || pending.phase != Phase::Dispatching
                    || pending.dml_acknowledged.is_some()
                    || pending.statements.len() != 1
                    || event.get("sdkRequestId").and_then(Value::as_str).is_none()
                    || event.get("kind").and_then(parse_dml_kind) != pending.dml_kind
                    || !event.as_object().is_some_and(|object| {
                        object.keys().all(|key| {
                            matches!(
                                key.as_str(),
                                "v" | "generation"
                                    | "id"
                                    | "type"
                                    | "kind"
                                    | "sdkRequestId"
                                    | "queryId"
                            )
                        })
                    })
                    || !validate_latest_statement(pending, &event)
                {
                    return false;
                }
                pending.dml_acknowledged = Some(DmlAcknowledgement {
                    kind: pending.dml_kind.unwrap(),
                    statement: pending.statements[0].clone(),
                });
                true
            }
            Some("ddlAcknowledged") => {
                if pending.op != "ddl"
                    || !pending.accepted
                    || pending.phase != Phase::Dispatching
                    || pending.ddl_acknowledged.is_some()
                    || pending.statements.len() != 1
                    || event.get("sdkRequestId").and_then(Value::as_str).is_none()
                    || event.get("kind").and_then(parse_ddl_kind) != pending.ddl_kind
                    || !event.as_object().is_some_and(|object| {
                        object.keys().all(|key| {
                            matches!(
                                key.as_str(),
                                "v" | "generation"
                                    | "id"
                                    | "type"
                                    | "kind"
                                    | "sdkRequestId"
                                    | "queryId"
                            )
                        })
                    })
                    || !validate_latest_statement(pending, &event)
                {
                    return false;
                }
                pending.ddl_acknowledged = Some(DdlAcknowledgement {
                    kind: pending.ddl_kind.unwrap(),
                    statement: pending.statements[0].clone(),
                });
                true
            }
            Some("statementAcknowledged") => {
                if pending.op != "reviewedSql"
                    || pending.impact != Impact::Mutating
                    || !pending.accepted
                    || pending.phase != Phase::Dispatching
                    || pending.statement_acknowledged.is_some()
                    || pending.statements.len() != 1
                    || event.get("sdkRequestId").and_then(Value::as_str).is_none()
                    || !event.as_object().is_some_and(|object| {
                        object.keys().all(|key| {
                            matches!(
                                key.as_str(),
                                "v" | "generation"
                                    | "id"
                                    | "type"
                                    | "sdkRequestId"
                                    | "queryId"
                                    | "context"
                            )
                        })
                    })
                    || !validate_latest_statement(pending, &event)
                {
                    return false;
                }
                let Some(context) = event.get("context").and_then(parse_reviewed_context) else {
                    return false;
                };
                pending.statement_acknowledged = Some(ReviewedAcknowledgement {
                    statement: pending.statements[0].clone(),
                    context,
                });
                true
            }
            Some("useAcknowledged") => {
                if pending.op != "switchSchema"
                    || !pending.accepted
                    || pending.phase != Phase::Dispatching
                    || pending.use_acknowledged.is_some()
                    || pending.statements.len() != 1
                    || event.get("sdkRequestId").is_none()
                    || !validate_latest_statement(pending, &event)
                {
                    return false;
                }
                pending.use_acknowledged = pending.statements.first().cloned();
                true
            }
            Some("statement") => {
                if !pending.accepted
                    || pending.phase < Phase::Dispatching
                    || !matches!(
                        pending.op.as_str(),
                        "query"
                            | "context"
                            | "switchSchema"
                            | "transaction"
                            | "dml"
                            | "ddl"
                            | "reviewedSql"
                    )
                {
                    return false;
                }
                pending.statement_events += 1;
                let limit = if pending.op == "switchSchema" { 4 } else { 2 };
                pending.statement_events <= limit && update_statement(pending, &event)
            }
            Some("columns") => {
                if !streams_rows(&pending.op)
                    || pending.phase != Phase::Acknowledged
                    || pending.width.is_some()
                {
                    return false;
                }
                let Some(columns) = event.get("columns").and_then(Value::as_array) else {
                    return false;
                };
                if columns.len() > 1000 {
                    return false;
                }
                let columns: Option<Vec<_>> = columns
                    .iter()
                    .map(|c| {
                        Some(Column {
                            name: c.get("name")?.as_str()?.into(),
                            data_type: c.get("type")?.as_str()?.into(),
                        })
                    })
                    .collect();
                let Some(columns) = columns else {
                    return false;
                };
                pending.width = Some(columns.len());
                pending.columns.send_replace(Some(columns));
                true
            }
            Some("rows") => {
                let Some(value) = event.get("rows") else {
                    return false;
                };
                if !streams_rows(&pending.op)
                    || pending.phase != Phase::Acknowledged
                    || pending.credits == 0
                    || serde_json::to_vec(value).map_or(true, |b| b.len() > MAX_BATCH)
                {
                    return false;
                }
                let Ok(rows) = serde_json::from_value::<Rows>(value.clone()) else {
                    return false;
                };
                if rows.is_empty()
                    || rows.len() > 128
                    || rows.iter().any(|r| Some(r.len()) != pending.width)
                {
                    return false;
                }
                pending.credits -= 1;
                if pending.discard {
                    true
                } else {
                    self.buffered.fetch_add(1, Ordering::SeqCst);
                    pending
                        .rows
                        .try_send(RowBatch {
                            rows,
                            budget: self.buffered.clone(),
                        })
                        .is_ok()
                }
            }
            Some("terminal") => {
                if !validate_latest_statement(pending, &event) {
                    return false;
                }
                let Some(ok) = event.get("ok").and_then(Value::as_bool) else {
                    return false;
                };
                let mut batch_terminal = pending.batch.clone();
                if let Some(batch) = &mut batch_terminal {
                    if ok {
                        if !pending.accepted
                            || !event
                                .get("result")
                                .is_some_and(|result| batch.accept_terminal(result))
                            || BatchCapture::encode(batch).is_err()
                        {
                            return false;
                        }
                    } else if batch.has_dispatch() {
                        // Once work starts the helper owes a closed batch
                        // summary, including unknown remote disposition.
                        return false;
                    }
                }
                if ok
                    && pending.op == "switchSchema"
                    && event.get("sdkRequestId").and_then(Value::as_str).is_none()
                {
                    return false;
                }
                if ok && pending.op == "transaction" {
                    let Some(result) = event.get("result").and_then(Value::as_object) else {
                        return false;
                    };
                    if event.get("sdkRequestId").and_then(Value::as_str).is_none()
                        || result.get("action").and_then(TransactionAction::parse)
                            != pending.transaction_action
                        || !result.keys().all(|key| {
                            matches!(key.as_str(), "action" | "sdkRequestId" | "queryId")
                        })
                        || result.get("sdkRequestId").and_then(Value::as_str).is_none()
                        || !validate_latest_statement(pending, event.get("result").unwrap())
                    {
                        return false;
                    }
                }
                if ok && pending.op == "dml" {
                    let Some(result) = event.get("result") else {
                        return false;
                    };
                    if event.get("sdkRequestId").and_then(Value::as_str).is_none()
                        || pending.dml_acknowledged.is_none()
                        || !valid_dml_result(pending, result)
                    {
                        return false;
                    }
                }
                if ok && pending.op == "ddl" {
                    let Some(result) = event.get("result") else {
                        return false;
                    };
                    if event.get("sdkRequestId").and_then(Value::as_str).is_none()
                        || pending.ddl_acknowledged.is_none()
                        || !valid_ddl_result(pending, result)
                    {
                        return false;
                    }
                }
                if ok && pending.op == "reviewedSql" {
                    let Some(result) = event.get("result") else {
                        return false;
                    };
                    if pending.statement_acknowledged.is_none()
                        || event.get("sdkRequestId").and_then(Value::as_str).is_none()
                        || !valid_reviewed_result(pending, result)
                    {
                        return false;
                    }
                }
                let (code, result) = if ok {
                    if event.get("error").is_some() || event.get("result").is_none() {
                        return false;
                    }
                    (None, event.get("result").cloned())
                } else {
                    let Some(code) = event
                        .get("error")
                        .and_then(|e| e.get("code"))
                        .and_then(Value::as_str)
                    else {
                        return false;
                    };
                    if !matches!(
                        code,
                        "invalidInput"
                            | "busy"
                            | "unsupported"
                            | "connectionClosed"
                            | "cancelled"
                            | "database"
                            | "outcomeUnknown"
                            | "resultLimitReached"
                    ) {
                        return false;
                    }
                    (Some(code.to_owned()), None)
                };
                if ok
                    && matches!(
                        pending.op.as_str(),
                        "query"
                            | "context"
                            | "switchSchema"
                            | "transaction"
                            | "dml"
                            | "ddl"
                            | "reviewedSql"
                    )
                    && pending.phase != Phase::Acknowledged
                {
                    return false;
                }
                pending.batch = batch_terminal;
                let unverified_use =
                    pending.use_acknowledged.is_some() && pending.phase != Phase::Acknowledged;
                let cancelled_context = matches!(
                    pending.impact,
                    Impact::ContextChange
                        | Impact::Begin
                        | Impact::Commit
                        | Impact::Rollback
                        | Impact::Mutating
                ) && code.as_deref() == Some("cancelled");
                let certainty = if pending.batch.as_ref().is_some_and(|batch| {
                    batch
                        .summary
                        .as_ref()
                        .is_some_and(|summary| summary.outcome == BatchOutcome::Unknown)
                }) {
                    Certainty::OutcomeUnknown
                } else if pending
                    .batch
                    .as_ref()
                    .is_some_and(BatchEvidence::has_acknowledgement)
                    || pending.boundary_acknowledged.is_some()
                    || pending.dml_acknowledged.is_some()
                    || pending.ddl_acknowledged.is_some()
                    || pending.statement_acknowledged.is_some()
                {
                    Certainty::StatementSucceeded
                } else if pending.phase == Phase::Acknowledged {
                    acknowledged_certainty(pending.impact)
                } else if unverified_use
                    || cancelled_context
                    || code
                        .as_deref()
                        .is_some_and(|c| matches!(c, "connectionClosed" | "outcomeUnknown"))
                {
                    interrupted_certainty(pending.impact, pending.phase)
                } else {
                    Certainty::ServerTerminal
                };
                let pending = state.pending.remove(&logical_id).unwrap();
                state.wire_ids.remove(id);
                if pending.op == "connect" && ok {
                    state.connected = true;
                }
                if pending.op == "close"
                    && ok
                    && result
                        .as_ref()
                        .and_then(|value| value.get("logoutAcknowledged"))
                        == Some(&Value::Bool(true))
                {
                    self.logout_acknowledged.store(true, Ordering::SeqCst);
                }
                let terminal_connection_loss = code
                    .as_deref()
                    .is_some_and(|c| matches!(c, "connectionClosed" | "outcomeUnknown"))
                    || unverified_use
                    || ((pending.boundary_acknowledged.is_some()
                        || pending.dml_acknowledged.is_some()
                        || pending.ddl_acknowledged.is_some()
                        || pending.statement_acknowledged.is_some())
                        && pending.phase != Phase::Acknowledged)
                    || cancelled_context
                    || (pending.statement_acknowledged.is_some() && !ok)
                    || pending.batch.as_ref().is_some_and(|batch| {
                        batch
                            .summary
                            .as_ref()
                            .is_some_and(|summary| summary.outcome == BatchOutcome::Unknown)
                    });
                let report = Report {
                    ok,
                    certainty,
                    phase: pending.phase,
                    result,
                    error_code: code,
                    retirement: None,
                    statements: pending.statements,
                    use_acknowledged: pending.use_acknowledged,
                    boundary_acknowledged: pending.boundary_acknowledged,
                    dml_acknowledged: pending.dml_acknowledged,
                    ddl_acknowledged: pending.ddl_acknowledged,
                    statement_acknowledged: pending.statement_acknowledged,
                    batch: pending.batch,
                };
                let closing = (pending.op == "reviewedSql" && state.closing)
                    || state.pending.values().any(|p| p.op == "close");
                #[cfg(all(unix, feature = "test-harness"))]
                let mut publication_guard = None;
                #[cfg(all(unix, feature = "test-harness"))]
                if let Some(transport) = &self.transport {
                    state
                        .finalizing
                        .insert((logical_id.clone(), pending.ticket), report.clone());
                    publication_guard = Some(transport.publication());
                    drop(state);
                    transport.terminal(PhysicalTerminal {
                        generation: &self.generation,
                        operation_id: &logical_id,
                        ticket: pending.ticket,
                        wire_id: id,
                        operation: &pending.op,
                        impact: pending.impact,
                        report: &report,
                    });
                    state = self.state.lock().unwrap();
                }
                // A failed reviewed drain must close admission before waking
                // the original terminal recipient. Preserve its original report;
                // stage other operations for publication outside State.
                let reviewed_retirement = (pending.op == "reviewedSql" || report.batch.is_some())
                    && terminal_connection_loss
                    && !closing;
                let retirement_publications = if reviewed_retirement {
                    self.retire_locked(&mut state, Retirement::Requested)
                } else {
                    Vec::new()
                };
                // A policy failure prevents future admission; it must never
                // replace the helper's validated original outcome or ACK.
                pending.terminal.send_replace(Some(report));
                #[cfg(all(unix, feature = "test-harness"))]
                {
                    state.finalizing.remove(&(logical_id, pending.ticket));
                    self.finalizations
                        .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
                }
                drop(state);
                #[cfg(test)]
                tests::observe_terminal_publication(self);
                #[cfg(all(unix, feature = "test-harness"))]
                drop(publication_guard);
                for publication in retirement_publications {
                    self.publish_local(publication);
                }
                if terminal_connection_loss && !closing && !reviewed_retirement {
                    self.retire(Retirement::Requested);
                }
                true
            }
            _ => false,
        }
    }
}

fn valid_dml_result(pending: &Pending, result: &Value) -> bool {
    result.get("kind").and_then(parse_dml_kind) == pending.dml_kind
        && valid_private_status_result(pending, result)
}

fn valid_ddl_result(pending: &Pending, result: &Value) -> bool {
    result.get("kind").and_then(parse_ddl_kind) == pending.ddl_kind
        && valid_private_status_result(pending, result)
}

fn valid_private_status_result(pending: &Pending, result: &Value) -> bool {
    let Some(object) = result.as_object() else {
        return false;
    };
    if !object.keys().all(|key| {
        matches!(
            key.as_str(),
            "kind"
                | "rowsSeen"
                | "rowsRetained"
                | "truncated"
                | "columns"
                | "values"
                | "sdkRequestId"
                | "queryId"
        )
    }) || result.get("sdkRequestId").and_then(Value::as_str).is_none()
        || !validate_latest_statement(pending, result)
        || result.get("rowsRetained").and_then(Value::as_u64) != Some(0)
        || result.get("truncated").and_then(Value::as_bool).is_none()
        || serde_json::to_vec(result).map_or(true, |bytes| bytes.len() > 65536 + 512)
    {
        return false;
    }
    let Some(columns) = result.get("columns").and_then(Value::as_array) else {
        return false;
    };
    let Some(rows) = result.get("values").and_then(Value::as_array) else {
        return false;
    };
    columns.len() <= 2
        && rows.len() <= 1
        && result.get("rowsSeen").and_then(Value::as_u64) == Some(rows.len() as u64)
        && columns.iter().all(|column| {
            column.as_object().is_some_and(|c| {
                c.len() == 2
                    && ["name", "type"].into_iter().all(|key| {
                        c.get(key).and_then(Value::as_str).is_some_and(|value| {
                            !value.is_empty()
                                && value.len() <= if key == "name" { 4096 } else { 128 }
                                && !value.contains('\0')
                        })
                    })
            })
        })
        && rows.iter().all(|row| {
            row.as_array().is_some_and(|row| {
                row.len() == columns.len()
                    && row.iter().all(|cell| cell.is_null() || cell.is_string())
            })
        })
}

fn valid_reviewed_result(pending: &Pending, result: &Value) -> bool {
    let Some(object) = result.as_object() else {
        return false;
    };
    object.keys().all(|key| {
        matches!(
            key.as_str(),
            "rowsSeen" | "rowsRetained" | "truncated" | "sdkRequestId" | "queryId"
        )
    }) && result.get("sdkRequestId").and_then(Value::as_str).is_some()
        && validate_latest_statement(pending, result)
        && result.get("truncated").and_then(Value::as_bool).is_some()
        && result
            .get("rowsSeen")
            .and_then(Value::as_u64)
            .zip(result.get("rowsRetained").and_then(Value::as_u64))
            .is_some_and(|(seen, retained)| retained <= seen)
}

fn validate_latest_statement(pending: &Pending, event: &Value) -> bool {
    match event.get("sdkRequestId") {
        None => event.get("queryId").is_none(),
        Some(Value::String(request_id)) => {
            let Some(latest) = pending.statements.last() else {
                return false;
            };
            if request_id != &latest.sdk_request_id {
                return false;
            }
            match event.get("queryId") {
                None => true,
                Some(Value::String(query_id)) => latest.query_id.as_ref() == Some(query_id),
                _ => false,
            }
        }
        _ => false,
    }
}

fn update_statement(pending: &mut Pending, event: &Value) -> bool {
    let Some(request_id) = event.get("sdkRequestId").and_then(Value::as_str) else {
        return false;
    };
    if request_id.len() != 36 || uuid::Uuid::parse_str(request_id).is_err() {
        return false;
    }
    let query_id = match event.get("queryId") {
        None => None,
        Some(Value::String(id)) if id.len() == 36 && uuid::Uuid::parse_str(id).is_ok() => {
            Some(id.clone())
        }
        _ => return false,
    };
    if let Some(existing) = pending
        .statements
        .iter_mut()
        .find(|s| s.sdk_request_id == request_id)
    {
        if let Some(query_id) = query_id {
            if existing.query_id.as_ref().is_some_and(|id| id != &query_id) {
                return false;
            }
            existing.query_id = Some(query_id);
        }
        if pending
            .use_acknowledged
            .as_ref()
            .is_some_and(|ack| ack.sdk_request_id == request_id)
        {
            pending.use_acknowledged = Some(existing.clone());
        }
        if let Some(ack) = pending
            .boundary_acknowledged
            .as_mut()
            .filter(|ack| ack.statement.sdk_request_id == request_id)
        {
            ack.statement = existing.clone();
        }
        if let Some(ack) = pending
            .dml_acknowledged
            .as_mut()
            .filter(|ack| ack.statement.sdk_request_id == request_id)
        {
            ack.statement = existing.clone();
        }
        if let Some(ack) = pending
            .ddl_acknowledged
            .as_mut()
            .filter(|ack| ack.statement.sdk_request_id == request_id)
        {
            ack.statement = existing.clone();
        }
        if let Some(ack) = pending
            .statement_acknowledged
            .as_mut()
            .filter(|ack| ack.statement.sdk_request_id == request_id)
        {
            ack.statement = existing.clone();
        }
    } else {
        if pending.statements.len() >= if pending.op == "switchSchema" { 2 } else { 1 }
            || (pending.op == "switchSchema"
                && pending.statements.len() == 1
                && pending.use_acknowledged.is_none())
        {
            return false;
        }
        pending.statements.push(StatementIdentity {
            sdk_request_id: request_id.into(),
            query_id,
        });
    }
    true
}

fn local_report(result: Value) -> Report {
    Report {
        ok: true,
        certainty: Certainty::NotSubmitted,
        phase: Phase::Queued,
        result: Some(result),
        error_code: None,
        retirement: None,
        statements: vec![],
        use_acknowledged: None,
        boundary_acknowledged: None,
        dml_acknowledged: None,
        ddl_acknowledged: None,
        statement_acknowledged: None,
        batch: None,
    }
}

fn acknowledged_certainty(_impact: Impact) -> Certainty {
    Certainty::StatementSucceeded
}
fn interrupted_certainty(impact: Impact, phase: Phase) -> Certainty {
    if phase == Phase::Queued {
        return Certainty::NotSubmitted;
    }
    if phase == Phase::Acknowledged {
        return acknowledged_certainty(impact);
    }
    match impact {
        Impact::ContextChange
        | Impact::Begin
        | Impact::Mutating
        | Impact::Commit
        | Impact::Rollback => Certainty::OutcomeUnknown,
        _ => Certainty::ReadInterrupted,
    }
}
async fn write_loop(
    inner: Arc<Inner>,
    mut stdin: impl AsyncWrite + Unpin,
    mut normal: mpsc::Receiver<Outbound>,
    mut control: mpsc::Receiver<Outbound>,
    limit: Duration,
) {
    let mut stopping = inner.stop.subscribe();
    'outbound: loop {
        if *stopping.borrow() {
            return;
        }
        let outbound = tokio::select! { biased; _ = stopping.changed() => return,
        message = control.recv() => message, message = normal.recv() => message };
        let Some(outbound) = outbound else {
            return;
        };
        // Prepare an exact candidate under the mutex, then admit outside it.
        // The single writer consumes an attached candidate after admission,
        // even if later ownership checks prevent actual assignment or sending.
        let prepared = 'prepare: {
            let mut state = inner.state.lock().unwrap();
            if state.retired.is_some() {
                return;
            }
            let Some(pending) = state
                .pending
                .get(&outbound.id)
                .filter(|p| p.ticket == outbound.ticket)
            else {
                continue 'outbound;
            };
            #[cfg(all(unix, feature = "test-harness"))]
            let metadata = (pending.op.clone(), pending.impact);
            #[cfg(not(all(unix, feature = "test-harness")))]
            let _ = pending;
            let Some(next) = state.next_wire_id.checked_add(1) else {
                drop(state);
                inner.retire(Retirement::Protocol);
                return;
            };
            let wire_id = next.to_string();
            let mut value: Value = match serde_json::from_slice(&outbound.bytes) {
                Ok(value) => value,
                Err(_) => {
                    drop(state);
                    inner.retire(Retirement::Protocol);
                    return;
                }
            };
            value["id"] = Value::String(wire_id.clone());
            if let Some((target, ticket)) = &outbound.target {
                let Some(target_id) = state
                    .pending
                    .get(target)
                    .filter(|p| Some(p.ticket) == *ticket)
                    .and_then(|p| p.wire_id.clone())
                else {
                    if let Some(pending) = state.pending.remove(&outbound.id) {
                        break 'prepare Err(inner.stage_local(
                            &mut state,
                            outbound.id.clone(),
                            pending,
                            local_report(json!({"inactive":true})),
                            #[cfg(all(unix, feature = "test-harness"))]
                            LocalTerminalOrigin::InactiveControl { admitted: false },
                        ));
                    }
                    continue 'outbound;
                };
                value["payload"]["target"] = Value::String(target_id);
            }
            let bytes = serde_json::to_vec(&value).unwrap();
            #[cfg(all(unix, feature = "test-harness"))]
            let prepared = (next, wire_id, value, bytes, metadata);
            #[cfg(not(all(unix, feature = "test-harness")))]
            let prepared = (next, wire_id, value, bytes);
            Ok(prepared)
        };
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(publication) => {
                inner.publish_inactive(publication, limit).await;
                continue;
            }
        };
        #[cfg(all(unix, feature = "test-harness"))]
        let (next, wire_id, value, bytes, (operation, impact)) = prepared;
        #[cfg(not(all(unix, feature = "test-harness")))]
        let (next, wire_id, value, bytes) = prepared;
        #[cfg(all(unix, feature = "test-harness"))]
        if let Some(transport) = &inner.transport {
            {
                let mut state = inner.state.lock().unwrap();
                if let Some(qualification) = state
                    .pending
                    .get_mut(&outbound.id)
                    .filter(|pending| pending.ticket == outbound.ticket)
                    .and_then(|pending| pending.qualification.as_mut())
                {
                    qualification.candidate_wire_id = Some(wire_id.clone());
                } else {
                    continue;
                }
            }
            let request = PhysicalRequest {
                generation: &inner.generation,
                operation_id: &outbound.id,
                ticket: outbound.ticket,
                operation: &operation,
                impact,
                target: outbound
                    .target
                    .as_ref()
                    .map(|(id, ticket)| (id.as_str(), *ticket)),
                payload: &value,
                wire_payload: &bytes,
            };
            let admission = tokio::select! { _ = stopping.changed() => return, result = timeout(limit, transport.admit(request)) => result };
            if !matches!(admission, Ok(Ok(()))) {
                if admission.is_err() {
                    transport.fail(crate::domain::AppError::invalid(
                        "Qualification transport admission deadline",
                    ));
                }
                inner.retire(Retirement::Protocol);
                return;
            }
        }
        let inactive = 'revalidate: {
            let mut state = inner.state.lock().unwrap();
            if state.retired.is_some() {
                return;
            }
            #[cfg(all(unix, feature = "test-harness"))]
            let candidate_consumed = inner.transport.is_some();
            #[cfg(not(all(unix, feature = "test-harness")))]
            let candidate_consumed = false;
            if candidate_consumed {
                if state.next_wire_id.checked_add(1) != Some(next) {
                    drop(state);
                    inner.retire(Retirement::Protocol);
                    return;
                }
                // Successful admission consumes its candidate even if later
                // ownership checks find cancellation, ID reuse, or a dead target.
                // Actual assignment remains absent until the live checks pass.
                state.next_wire_id = next;
            }
            if !state
                .pending
                .get(&outbound.id)
                .is_some_and(|p| p.ticket == outbound.ticket && p.phase == Phase::Queued)
            {
                continue 'outbound;
            }
            if let Some((target, ticket)) = &outbound.target {
                let target_live = state
                    .pending
                    .get(target)
                    .filter(|p| Some(p.ticket) == *ticket)
                    .and_then(|p| p.wire_id.as_deref());
                if target_live != value["payload"]["target"].as_str() || target_live.is_none() {
                    if let Some(pending) = state.pending.remove(&outbound.id) {
                        break 'revalidate Some(inner.stage_local(
                            &mut state,
                            outbound.id.clone(),
                            pending,
                            local_report(json!({"inactive":true})),
                            #[cfg(all(unix, feature = "test-harness"))]
                            LocalTerminalOrigin::InactiveControl {
                                admitted: inner.transport.is_some(),
                            },
                        ));
                    }
                    continue 'outbound;
                }
            }
            if !candidate_consumed {
                if state.next_wire_id.checked_add(1) != Some(next) {
                    drop(state);
                    inner.retire(Retirement::Protocol);
                    return;
                }
                state.next_wire_id = next;
            }
            state.wire_ids.insert(wire_id.clone(), outbound.id.clone());
            let pending = state.pending.get_mut(&outbound.id).unwrap();
            pending.wire_id = Some(wire_id);
            #[cfg(all(unix, feature = "test-harness"))]
            if inner.transport.is_none() {
                pending.phase = Phase::Sending;
            }
            #[cfg(not(all(unix, feature = "test-harness")))]
            {
                pending.phase = Phase::Sending;
            }
            None
        };
        if let Some(publication) = inactive {
            inner.publish_inactive(publication, limit).await;
            continue;
        }
        let result = tokio::select! { _ = stopping.changed() => return,
        result = timeout(limit, observed_write(&inner, &outbound, &value, &mut stdin, &bytes)) => result };
        if !matches!(result, Ok(Ok(()))) {
            #[cfg(all(unix, feature = "test-harness"))]
            if let Some(transport) = &inner.transport {
                transport.stream_failure(
                    Stream::Writer,
                    if result.is_err() {
                        Disposition::Deadline
                    } else {
                        Disposition::Io
                    },
                );
            }
            inner.retire(Retirement::Write);
            return;
        }
        #[cfg(all(unix, feature = "test-harness"))]
        if let Some(transport) = &inner.transport {
            transport.write_complete();
        }
        if let Some(pending) = inner
            .state
            .lock()
            .unwrap()
            .pending
            .get_mut(&outbound.id)
            .filter(|p| p.ticket == outbound.ticket)
        {
            pending.phase = pending.phase.max(Phase::Sent);
        }
    }
}

async fn observed_frame(
    inner: &Inner,
    reader: &mut (impl AsyncRead + Unpin),
    frame_limit: Duration,
) -> Result<Option<Value>, &'static str> {
    #[cfg(all(unix, feature = "test-harness"))]
    if let Some(transport) = &inner.transport {
        return transport.read_frame(reader, frame_limit).await;
    }
    #[cfg(not(all(unix, feature = "test-harness")))]
    let _ = inner;
    read_frame(reader, frame_limit).await
}
async fn observed_write(
    inner: &Inner,
    outbound: &Outbound,
    value: &Value,
    writer: &mut (impl AsyncWrite + Unpin),
    bytes: &[u8],
) -> std::io::Result<()> {
    #[cfg(all(unix, feature = "test-harness"))]
    if let Some(transport) = &inner.transport {
        let prefix = (bytes.len() as u32).to_be_bytes();
        let mut emitted = false;
        for part in [prefix.as_slice(), bytes] {
            let mut offset = 0;
            while offset < part.len() {
                let count = std::future::poll_fn(|context| {
                    use std::{pin::Pin, sync::TryLockError, task::Poll};
                    // Repeat after every Pending, including pipe backpressure.
                    // No policy callback runs under the process-state mutex.
                    if transport.check().is_err() {
                        return Poll::Ready(Err(std::io::ErrorKind::PermissionDenied.into()));
                    }
                    // Never wait for this mutex after the final policy check.
                    // A contended poll yields and rechecks policy on its next poll;
                    // the outer write deadline bounds this readiness wait.
                    let mut state = match inner.state.try_lock() {
                        Ok(state) => state,
                        Err(TryLockError::WouldBlock) => {
                            context.waker().wake_by_ref();
                            return Poll::Pending;
                        }
                        Err(TryLockError::Poisoned(_)) => {
                            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
                        }
                    };
                    if state.retired.is_some() {
                        return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
                    }
                    if let Some((target, ticket)) = &outbound.target {
                        let live = state
                            .pending
                            .get(target)
                            .filter(|pending| Some(pending.ticket) == *ticket)
                            .and_then(|pending| pending.wire_id.as_deref());
                        if live.is_none() || live != value["payload"]["target"].as_str() {
                            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
                        }
                    }
                    let Some(pending) = state.pending.get_mut(&outbound.id).filter(|pending| {
                        pending.ticket == outbound.ticket
                            && pending.wire_id.as_deref() == value["id"].as_str()
                    }) else {
                        return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
                    };
                    pending.phase = pending.phase.max(Phase::Sending);
                    // A single synchronous poll, never an await, owns this guard.
                    // Retirement cannot classify zero bytes between OS acceptance
                    // and the progress update. Pending guarantees no accepted bytes.
                    let result = Pin::new(&mut *writer).poll_write(context, &part[offset..]);
                    match &result {
                        Poll::Ready(Ok(count)) if *count > 0 => {
                            if !emitted {
                                transport.write_started();
                                emitted = true;
                            }
                            transport.wrote(*count);
                        }
                        _ if !emitted => pending.phase = Phase::Queued,
                        _ => {}
                    }
                    result
                })
                .await?;
                if count == 0 {
                    return Err(std::io::ErrorKind::WriteZero.into());
                }
                offset += count;
            }
        }
        return std::future::poll_fn(|context| {
            if transport.check().is_err() {
                return std::task::Poll::Ready(Err(std::io::ErrorKind::PermissionDenied.into()));
            }
            std::pin::Pin::new(&mut *writer).poll_flush(context)
        })
        .await;
    }
    #[cfg(not(all(unix, feature = "test-harness")))]
    let _ = (inner, outbound, value);
    write_bytes(writer, bytes).await
}

fn valid_generation(value: &str) -> bool {
    value.len() == 36
        && uuid::Uuid::parse_str(value).is_ok_and(|id| {
            matches!(id.get_version_num(), 1..=5) && id.get_variant() == uuid::Variant::RFC4122
        })
}

fn valid_hello(value: &Value) -> bool {
    value.get("v").and_then(Value::as_u64) == Some(1)
        && value.get("type").and_then(Value::as_str) == Some("hello")
        && value.get("protocol").and_then(Value::as_str) == Some("joinery-snowflake")
        && value.get("sdkVersion").and_then(Value::as_str) == Some("3.4.1")
        && value.get("limits")
            == Some(
                &json!({"frameBytes":MAX_FRAME,"batchRows":128,"batchBytes":MAX_BATCH,"queued":8,"credits":32}),
            )
}
pub async fn read_frame(
    reader: &mut (impl AsyncRead + Unpin),
    frame_limit: Duration,
) -> Result<Option<Value>, &'static str> {
    let mut prefix = [0u8; 4];
    if reader
        .read(&mut prefix[..1])
        .await
        .map_err(|_| "frame read failed")?
        == 0
    {
        return Ok(None);
    }
    timeout(frame_limit, async {
        reader
            .read_exact(&mut prefix[1..])
            .await
            .map_err(|_| "incomplete frame")?;
        let length = u32::from_be_bytes(prefix) as usize;
        if length == 0 || length > MAX_FRAME {
            return Err("frame exceeds bound");
        }
        let mut bytes = vec![0; length];
        reader
            .read_exact(&mut bytes)
            .await
            .map_err(|_| "incomplete frame")?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "invalid JSON frame")
    })
    .await
    .map_err(|_| "incomplete frame deadline")?
}
pub async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    value: &Value,
) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec(value).map_err(|_| "invalid request")?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("frame exceeds bound");
    }
    write_bytes(writer, &bytes)
        .await
        .map_err(|_| "frame write failed")
}
async fn write_bytes(writer: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> std::io::Result<()> {
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(bytes).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(unix, feature = "test-harness"))]
    mod generation_tests {
        include!("snowflake_process_generation_tests.rs");
    }
    type TerminalPublicationObserver = Box<dyn FnOnce(&Inner)>;
    std::thread_local! {
        // Observe the exact synchronous boundary after a terminal is visible and
        // State unlocks, before the legacy deferred-retirement path. Production
        // has no callback or admission override.
        static TERMINAL_PUBLICATION_OBSERVER: std::cell::RefCell<Option<TerminalPublicationObserver>> = const { std::cell::RefCell::new(None) };
    }
    pub(super) fn observe_terminal_publication(inner: &Inner) {
        let observer = TERMINAL_PUBLICATION_OBSERVER.with(|slot| slot.borrow_mut().take());
        if let Some(observer) = observer {
            observer(inner);
        }
    }
    fn isolated() -> (
        Arc<Inner>,
        mpsc::Receiver<Outbound>,
        mpsc::Receiver<Outbound>,
    ) {
        let (normal, normal_rx) = mpsc::channel(8);
        let (control, control_rx) = mpsc::channel(16);
        let (stop, _) = watch::channel(false);
        let (_, receiver) = watch::channel(None);
        let exited = CleanupObserver {
            receiver,
            authority: None,
        };
        let inner = Arc::new(Inner {
            generation: "00000000-0000-4000-8000-000000000001".into(),
            state: Mutex::new(State {
                connected: true,
                ..State::default()
            }),
            normal,
            control,
            stop,
            exited,
            sequence: AtomicU64::new(0),
            buffered: Arc::new(AtomicUsize::new(0)),
            logout_acknowledged: AtomicBool::new(false),
            #[cfg(all(unix, feature = "test-harness"))]
            transport: None,
            #[cfg(all(unix, feature = "test-harness"))]
            finalizations: watch::channel(0).0,
        });
        (inner, normal_rx, control_rx)
    }
    const USE_ID: &str = "00000000-0000-4000-8000-000000000011";
    const READBACK_ID: &str = "00000000-0000-4000-8000-000000000012";
    fn switch_event(inner: &Inner, extra: Value) -> bool {
        let mut event = json!({"v":1,"generation":inner.generation,"id":"1"});
        event
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        inner.event(event)
    }
    fn reviewed_ack() -> Value {
        json!({"type":"statementAcknowledged","sdkRequestId":USE_ID,
            "context":{"database":"DB", "schema":"Quoted \"schema", "role":null,"warehouse":"WH"}})
    }
    fn dispatched_reviewed() -> (Arc<Inner>, Operation) {
        let (inner, _normal, _control) = isolated();
        let operation = inner
            .request(
                "reviewed",
                "reviewedSql",
                json!({"credits":1}),
                Impact::Mutating,
            )
            .unwrap();
        {
            let mut state = inner.state.lock().unwrap();
            state.pending.get_mut("reviewed").unwrap().phase = Phase::Sent;
            state.wire_ids.insert("1".into(), "reviewed".into());
        }
        assert!(switch_event(&inner, json!({"type":"accepted"})));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"dispatching"})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":USE_ID})
        ));
        (inner, operation)
    }
    fn batch_payload() -> Value {
        json!({"mode":"automatic","owner":null,"rollbackPrior":false,
            "expected":{"user":"TEST","session":"123","role":"ROLE","database":"DB","schema":"PUBLIC","warehouse":"WH"},
            "units":[{"kind":"update","sql":"UPDATE synthetic SET v = ? WHERE id = ?","binds":["value","1"]}]})
    }
    fn batch_frame(inner: &Inner, step: usize, role: &str, unit: Option<usize>, phase: &str) {
        assert!(switch_event(
            inner,
            json!({"type":"batchStep","step":step,"role":role,"unit":unit,"phase":phase})
        ));
    }
    fn batch_identity(step: usize) -> String {
        uuid::Uuid::from_u128(0x12340000000040008000000000000000 + step as u128).to_string()
    }
    fn batch_query(step: usize) -> String {
        uuid::Uuid::from_u128(0x56780000000040008000000000000000 + step as u128).to_string()
    }
    fn batch_step(inner: &Inner, step: usize, role: &str, unit: Option<usize>, drain: bool) {
        batch_frame(inner, step, role, unit, "dispatching");
        assert!(switch_event(
            inner,
            json!({"type":"batchStatement","step":step,"sdkRequestId":batch_identity(step)})
        ));
        batch_frame(inner, step, role, unit, "acknowledged");
        assert!(switch_event(
            inner,
            json!({"type":"batchStatement","step":step,"sdkRequestId":batch_identity(step),"queryId":batch_query(step)})
        ));
        if unit.is_some() {
            assert!(switch_event(
                inner,
                json!({"type":"batchCount","step":step,"unit":unit,"affectedRows":"1","multiJoinedRows":null})
            ));
        }
        if drain {
            batch_frame(inner, step, role, unit, "drained");
        }
    }
    fn dispatched_batch() -> (
        Arc<Inner>,
        Operation,
        mpsc::Receiver<Outbound>,
        mpsc::Receiver<Outbound>,
    ) {
        let (inner, normal, control) = isolated();
        let operation = inner
            .request("batch", "mutationBatch", batch_payload(), Impact::Mutating)
            .unwrap();
        {
            let mut state = inner.state.lock().unwrap();
            state.pending.get_mut("batch").unwrap().phase = Phase::Sent;
            state.wire_ids.insert("1".into(), "batch".into());
        }
        assert!(switch_event(&inner, json!({"type":"accepted"})));
        for (index, role) in ["context", "settings", "begin", "transaction"]
            .iter()
            .enumerate()
        {
            batch_step(&inner, index + 1, role, None, true);
        }
        batch_step(&inner, 5, "unit", Some(0), false);
        (inner, operation, normal, control)
    }
    #[test]
    fn batch_admission_requires_closed_payload_mutating_impact_and_dedicated_events() {
        let (inner, _normal, _control) = isolated();
        for impact in [
            Impact::Read,
            Impact::Control,
            Impact::ContextChange,
            Impact::Begin,
            Impact::Commit,
            Impact::Rollback,
        ] {
            assert!(
                inner
                    .request("invalid", "mutationBatch", batch_payload(), impact)
                    .is_err()
            );
        }
        let mut malformed = batch_payload();
        malformed["units"][0]["binds"] = json!([1.5]);
        assert!(
            inner
                .request("invalid", "mutationBatch", malformed, Impact::Mutating)
                .is_err()
        );
        assert!(inner.state.lock().unwrap().pending.is_empty());
        let (inner, operation) = dispatched_reviewed();
        assert!(!switch_event(
            &inner,
            json!({"type":"batchStep","step":1,"role":"context","unit":null,"phase":"dispatching"})
        ));
        assert!(operation.batch_evidence().is_none());
        let (inner, operation, _normal, _control) = dispatched_batch();
        assert!(!switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":USE_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged"})
        ));
        assert!(operation.batch_evidence().is_some());
    }
    #[tokio::test]
    async fn batch_evidence_survives_eof_deadline_protocol_and_dropped_waiter_without_replay() {
        for reason in [
            Retirement::Eof,
            Retirement::Protocol,
            Retirement::Write,
            Retirement::Requested,
        ] {
            let (inner, mut operation, mut normal, _control) = dispatched_batch();
            let original = operation.batch_evidence().unwrap();
            assert_eq!(
                original.steps[4].count.as_ref().unwrap().multi_joined_rows,
                None
            );
            assert_eq!(
                original.steps[4]
                    .statement
                    .as_ref()
                    .unwrap()
                    .query_id
                    .as_deref(),
                Some(batch_query(5).as_str())
            );
            inner.retire(reason);
            let report = operation.wait().await;
            assert_eq!(report.retirement, Some(reason));
            assert_eq!(report.batch.as_ref(), Some(&original));
            assert_eq!(operation.batch_evidence().as_ref(), Some(&original));
            assert!(report.batch.unwrap().summary.is_none());
            assert!(
                inner
                    .request("replay", "mutationBatch", batch_payload(), Impact::Mutating)
                    .is_err()
            );
            assert!(normal.try_recv().is_ok());
            assert!(normal.try_recv().is_err());
        }
        let (_inner, mut operation, _normal, _control) = dispatched_batch();
        let original = operation.batch_evidence();
        let report = operation.wait_bounded(Duration::ZERO).await;
        assert_eq!(report.retirement, Some(Retirement::Deadline));
        assert_eq!(report.batch, original);
        let (inner, operation, _normal, _control) = dispatched_batch();
        let original = operation.batch_evidence();
        let terminal = operation.terminal_receiver();
        drop(operation);
        let report = terminal.borrow().clone().unwrap();
        assert_eq!(report.retirement, Some(Retirement::Dropped));
        assert_eq!(report.batch, original);
        assert!(inner.state.lock().unwrap().retired.is_some());
    }
    #[tokio::test]
    async fn batch_cancel_unknown_summary_preserves_ack_and_closes_admission_before_publication() {
        let (inner, mut operation, _normal, _control) = dispatched_batch();
        let cancel = inner.cancel(&operation.token()).unwrap();
        let original = operation.batch_evidence().unwrap();
        let recipient = inner.clone();
        TERMINAL_PUBLICATION_OBSERVER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |_| {
                assert!(
                    recipient
                        .request(
                            "replacement",
                            "mutationBatch",
                            batch_payload(),
                            Impact::Mutating
                        )
                        .is_err()
                );
            }));
        });
        assert!(switch_event(
            &inner,
            json!({"type":"terminal","ok":true,"result":{
            "outcome":"unknown","owner":null,"transactionId":"123","failure":"cancelled","priorBatches":0}})
        ));
        let report = operation.wait().await;
        assert_eq!(report.certainty, Certainty::OutcomeUnknown);
        let retained = report.batch.unwrap();
        assert_eq!(retained.steps, original.steps);
        assert_eq!(
            retained.summary.unwrap().failure,
            Some(super::super::snowflake_batch::Failure::Cancelled)
        );
        assert!(cancel.report().is_some());
        assert_eq!(
            inner.state.lock().unwrap().retired,
            Some(Retirement::Requested)
        );
    }
    // Actual child bridge through production Helper, framing and submit guard;
    // the SDK is strictly local synthetic data, with no external account calls.
    #[cfg(all(unix, feature = "test-harness"))]
    fn actual_batch_launch(scenario: &str) -> Launch {
        let node = PathBuf::from(
            std::env::var_os("JOINERY_TEST_NODE")
                .expect("JOINERY_TEST_NODE must name an explicit absolute Node executable"),
        );
        assert!(node.is_absolute() && node.is_file());
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("batch-fixture.mjs");
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../snowflake-helper/src");
        let body = r#"
import { Readable, PassThrough } from 'node:stream';
import { writeFileSync } from 'node:fs';
const { Helper } = await import(HELPER_PATH);
const { Decoder, writer } = await import(FRAMING_PATH);
const { installQuerySubmitGuard } = await import(GUARD_PATH);
const { MUTATION_CONTEXT_SQL, MUTATION_SETTINGS_SQL } = await import(BATCH_PATH);
const { SQL_CONTRACT } = await import(CONTRACT_PATH);
const scenario=SCENARIO, root=process.env.JOINERY_SNOWFLAKE_STATE_DIR;
let serial=0, transaction=null, units=0, connections=0, stopping=false;
const records=new Map(), calls=[];
const id=(prefix,n)=>`${prefix}-${String(n).padStart(12,'0')}`;
const sdk={createConnection:()=>{
  connections++;
  return {
    connectAsync(cb){queueMicrotask(()=>cb(null));return Promise.resolve();},
    destroy(cb){cb(null);},
    execute(options){
      const n=++serial, sql=options.sqlText;
      calls.push(sql);writeFileSync(root+'/calls',JSON.stringify({connections,calls}));
      let names=[],rows=[],unit=false,acknowledged=false;
      if(sql===SQL_CONTRACT.bootstrapContext||sql===MUTATION_CONTEXT_SQL){
        names=['user','role','database','schema','warehouse','session'];
        rows=[['TEST','ROLE','DB','PUBLIC','WH','123']];
        if(sql===MUTATION_CONTEXT_SQL){names.push('transaction');rows[0].push(transaction);}
      }else if(sql===MUTATION_SETTINGS_SQL){names=['key','value'];rows=[['QUOTED_IDENTIFIERS_IGNORE_CASE','false']];}
      else if(sql==='BEGIN TRANSACTION'){transaction=String(1000+n);records.set(transaction,'running');}
      else if(sql==='COMMIT'||sql==='ROLLBACK'){records.set(transaction,sql==='COMMIT'?'committed':'rolled back');transaction=null;}
      else if(sql.startsWith('DESCRIBE TRANSACTION ')){const tid=sql.slice('DESCRIBE TRANSACTION '.length);names=['id','session','state'];rows=[[tid,'123',records.get(tid)]];}
      else if(sql.startsWith('UPDATE synthetic ')){unit=true;units++;names=['number of rows updated'];rows=[['1']];}
      else throw Error('unexpected synthetic SQL');
      const statement={
        getRequestId:()=>id('11111111-1111-4111-8111',n),
        getQueryId:()=>acknowledged?id('22222222-2222-4222-8222',n):undefined,
        getColumns:()=>names.map(name=>({getName:()=>name,getType:()=>name.startsWith('number of ')||sql.startsWith('DESCRIBE TRANSACTION ')&&['id','session'].includes(name)?'FIXED':'TEXT'})),
        streamRows:()=>unit&&units===2&&scenario!=='manual'?new PassThrough({objectMode:true}):Readable.from(rows),
        cancel(cb){cb(null);}
      };
      queueMicrotask(()=>{acknowledged=true;options.complete(null,statement);});
      return statement;
    }
  };
}};
const send=writer(process.stdout.write.bind(process.stdout),()=>process.exit(2));
const stop=()=>{if(!stopping){stopping=true;send.flush().then(()=>process.exit(0));}};
const guard=installQuerySubmitGuard({onViolation:stop});
const emit=event=>{
  const publication=send(event);
  if(scenario==='invalid'&&event.type==='batchStep'&&event.role==='unit'&&event.unit===1&&event.phase==='acknowledged'){
    send({...event,type:'batchCount',affectedRows:'1',multiJoinedRows:null,unit:499,phase:undefined,role:undefined});
  }
  return publication;
};
const helper=new Helper(sdk,emit,stop,{submitGuard:guard,isFileTransfer:()=>false,closeTimeoutMs:20});
const decoder=new Decoder();
process.stdin.on('data',chunk=>{try{for(const frame of decoder.push(chunk))helper.receive(frame);}catch{stop();}});
process.stdin.on('end',stop);
"#;
        let body = [
            ("HELPER_PATH", "helper.mjs"),
            ("FRAMING_PATH", "framing.mjs"),
            ("GUARD_PATH", "query-submit-guard.mjs"),
            ("BATCH_PATH", "mutation-batch.mjs"),
            ("CONTRACT_PATH", "sql-contract.mjs"),
        ]
        .into_iter()
        .fold(body.to_owned(), |body, (token, file)| {
            body.replace(token, &serde_json::to_string(&source.join(file)).unwrap())
        })
        .replace("SCENARIO", &serde_json::to_string(scenario).unwrap());
        std::fs::write(&script, body).unwrap();
        Launch::new(
            node,
            Some(script),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            root,
        )
        .unwrap()
    }
    #[cfg(all(unix, feature = "test-harness"))]
    async fn actual_batch_supervisor(scenario: &str) -> (Supervisor, PathBuf) {
        let launch = actual_batch_launch(scenario);
        let root = launch.state_directory().to_owned();
        let supervisor = Supervisor::spawn(
            launch,
            "00000000-0000-4000-8000-000000000001",
            Bounds::default(),
        )
        .await
        .unwrap();
        let mut connect = supervisor
            .request(
                "connect",
                "connect",
                json!({"account":"fixture","host":"fixture.snowflakecomputing.com",
            "username":"TEST","role":"ROLE","database":"DB","schema":"PUBLIC","warehouse":"WH"}),
                Impact::Control,
            )
            .unwrap();
        assert!(connect.wait_bounded(Duration::from_secs(5)).await.ok);
        let mut context = supervisor
            .request("context", "context", json!({}), Impact::Read)
            .unwrap();
        assert!(context.wait_bounded(Duration::from_secs(5)).await.ok);
        (supervisor, root)
    }
    #[cfg(all(unix, feature = "test-harness"))]
    #[tokio::test]
    #[ignore = "requires explicit JOINERY_TEST_NODE; local production-helper child bridge only"]
    async fn actual_helper_batch_manual_finish_cancel_and_invalid_frame_preserve_evidence() {
        let limit = Duration::from_secs(5);
        for scenario in ["manual", "cancel", "invalid"] {
            let (supervisor, root) = actual_batch_supervisor(scenario).await;
            let mut payload = batch_payload();
            payload["units"] = json!([payload["units"][0].clone(), payload["units"][0].clone()]);
            if scenario == "manual" {
                payload["mode"] = json!("manual");
            }
            let mut operation = supervisor
                .request("batch", "mutationBatch", payload.clone(), Impact::Mutating)
                .unwrap();
            if scenario == "manual" {
                let report = operation.wait_bounded(limit).await;
                assert!(report.ok, "{report:?}");
                let original = report.batch.unwrap();
                let pending = original.summary.as_ref().unwrap();
                assert_eq!(
                    pending.outcome,
                    super::super::snowflake_batch::Outcome::Pending
                );
                assert_eq!(
                    original
                        .steps
                        .iter()
                        .filter(|s| s.unit.is_some() && s.count.is_some())
                        .count(),
                    2
                );
                let original_bytes = BatchCapture::encode(&original).unwrap();
                let mut finish = supervisor.request("finish", "mutationFinish", json!({"owner":pending.owner,"action":"rollback","expected":payload["expected"]}), Impact::Mutating).unwrap();
                let finished = finish.wait_bounded(limit).await;
                assert!(finished.ok, "{finished:?}");
                let evidence = finished.batch.unwrap();
                assert_eq!(
                    evidence.summary.as_ref().unwrap().outcome,
                    super::super::snowflake_batch::Outcome::RolledBack
                );
                assert_eq!(evidence.summary.as_ref().unwrap().owner, pending.owner);
                assert_eq!(BatchCapture::encode(&original).unwrap(), original_bytes);
                let calls: Value =
                    serde_json::from_str(&std::fs::read_to_string(root.join("calls")).unwrap())
                        .unwrap();
                assert_eq!(calls["connections"], 1);
                assert_eq!(
                    calls["calls"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|s| s.as_str().unwrap().starts_with("UPDATE synthetic "))
                        .count(),
                    2
                );
                assert!(supervisor.close().await.unwrap().ok);
            } else {
                if scenario == "cancel" {
                    timeout(limit, async {
                        loop {
                            if operation.batch_evidence().is_some_and(|batch| batch.steps.iter().any(|step|
                                step.unit == Some(1) && step.phase >= super::super::snowflake_batch::StepPhase::Acknowledged)) { break; }
                            tokio::task::yield_now().await;
                        }
                    }).await.unwrap();
                    let mut cancel = supervisor.cancel(&operation.token()).unwrap();
                    assert!(cancel.wait_bounded(limit).await.ok);
                }
                let report = operation.wait_bounded(limit).await;
                if scenario == "invalid" {
                    assert_eq!(report.retirement, Some(Retirement::Protocol));
                }
                let batch = report.batch.unwrap();
                assert_eq!(
                    batch
                        .steps
                        .iter()
                        .filter(|step| step.unit.is_some())
                        .count(),
                    2
                );
                assert_eq!(
                    batch
                        .steps
                        .iter()
                        .find(|step| step.unit == Some(0))
                        .unwrap()
                        .count
                        .as_ref()
                        .unwrap()
                        .affected_rows,
                    "1"
                );
                let second = batch
                    .steps
                    .iter()
                    .find(|step| step.unit == Some(1))
                    .unwrap();
                assert_eq!(
                    second.phase,
                    super::super::snowflake_batch::StepPhase::Acknowledged
                );
                assert!(second.count.is_none());
                assert!(batch.steps.iter().all(|step| !matches!(
                    step.role,
                    super::super::snowflake_batch::Role::Commit
                        | super::super::snowflake_batch::Role::Rollback
                )));
                assert_eq!(
                    BatchCapture::decode(&BatchCapture::encode(&batch).unwrap()).unwrap(),
                    batch
                );
                assert!(
                    supervisor
                        .request("replay", "mutationBatch", payload, Impact::Mutating)
                        .is_err()
                );
            }
            assert!(supervisor.wait_exited().await);
            assert!(!root.exists());
        }
    }
    #[tokio::test]
    async fn reviewed_sql_requires_mutating_impact_and_bounded_initial_credits() {
        let (inner, _normal, _control) = isolated();
        for impact in [
            Impact::Read,
            Impact::Control,
            Impact::ContextChange,
            Impact::Begin,
            Impact::Commit,
            Impact::Rollback,
        ] {
            assert!(
                inner
                    .request("bad", "reviewedSql", json!({"credits":1}), impact)
                    .is_err()
            );
        }
        for credits in [json!(0), json!(33), json!(-1), json!("1"), Value::Null] {
            assert!(
                inner
                    .request(
                        "bad",
                        "reviewedSql",
                        json!({"credits":credits}),
                        Impact::Mutating
                    )
                    .is_err()
            );
        }
        let mut operation = inner
            .request(
                "valid",
                "reviewedSql",
                json!({"credits":32}),
                Impact::Mutating,
            )
            .unwrap();
        assert!(inner.cancel(&operation.token()).unwrap().wait().await.ok);
        let report = operation.wait().await;
        assert_eq!(report.certainty, Certainty::NotSubmitted);
        assert!(report.statement_acknowledged.is_none());
    }
    #[test]
    fn reviewed_ack_rejects_foreign_identity_wrong_shape_and_out_of_order_frames() {
        let mut invalid = vec![
            json!({"sdkRequestId":READBACK_ID}),
            json!({"sdkRequestId":null}),
            json!({"queryId":READBACK_ID}),
            json!({"extra":true}),
            json!({"context":null}),
            json!({"context":{"database":null,"schema":null,"role":null}}),
            json!({"context":{"database":null,"schema":null,"role":null,"warehouse":null,"extra":null}}),
        ];
        for value in [
            json!(""),
            json!("a".repeat(256)),
            json!("😀".repeat(256)),
            json!("nul\0field"),
            json!(17),
            json!([]),
            json!({}),
        ] {
            invalid.push(
                json!({"context":{"database":value,"schema":null,"role":null,"warehouse":null}}),
            );
        }
        for patch in invalid {
            let (inner, operation) = dispatched_reviewed();
            let mut frame = reviewed_ack();
            frame
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(!switch_event(&inner, frame), "{patch}");
            assert!(operation.statement_acknowledged().is_none());
            inner.retire(Retirement::Protocol);
            assert_eq!(
                operation.report().unwrap().certainty,
                Certainty::OutcomeUnknown
            );
        }
        let (inner, operation) = dispatched_reviewed();
        assert!(!switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID})
        ));
        assert!(operation.statement_acknowledged().is_none());
        inner
            .state
            .lock()
            .unwrap()
            .pending
            .get_mut("reviewed")
            .unwrap()
            .statements
            .clear();
        assert!(!switch_event(&inner, reviewed_ack()));
        inner.retire(Retirement::Protocol);
        for (inner, operation) in [
            dispatched_dml(SnowflakeDmlKind::Insert),
            dispatched_transaction(TransactionAction::Commit),
            dispatched_switch(),
        ] {
            assert!(!switch_event(&inner, reviewed_ack()));
            assert!(operation.statement_acknowledged().is_none());
            inner.retire(Retirement::Protocol);
        }
        for (op, payload, impact) in [
            ("query", json!({}), Impact::Read),
            ("context", json!({}), Impact::Read),
            ("ddl", json!({"kind":"createTable"}), Impact::Mutating),
        ] {
            let (inner, _normal, _control) = isolated();
            let operation = inner.request("other", op, payload, impact).unwrap();
            {
                let mut state = inner.state.lock().unwrap();
                let pending = state.pending.get_mut("other").unwrap();
                pending.accepted = true;
                pending.phase = Phase::Dispatching;
                pending.statements.push(StatementIdentity {
                    sdk_request_id: USE_ID.into(),
                    query_id: None,
                });
                state.wire_ids.insert("1".into(), "other".into());
            }
            assert!(!switch_event(&inner, reviewed_ack()), "{op}");
            assert!(operation.statement_acknowledged().is_none());
            inner.retire(Retirement::Protocol);
        }
    }
    #[test]
    fn reviewed_ack_context_bounds_nulls_and_query_id_upgrade_are_preserved() {
        let (inner, operation) = dispatched_reviewed();
        let context =
            json!({"database":"😀".repeat(255),"schema":null,"role":null,"warehouse":null});
        let mut ack = reviewed_ack();
        ack["context"] = context;
        assert!(switch_event(&inner, ack.clone()));
        assert!(!switch_event(&inner, ack));
        assert!(switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":USE_ID,"queryId":READBACK_ID})
        ));
        let retained = operation.statement_acknowledged().unwrap();
        assert_eq!(retained.statement.query_id.as_deref(), Some(READBACK_ID));
        assert_eq!(
            retained.context.database.as_deref(),
            Some("😀".repeat(255).as_str())
        );
        assert_eq!(retained.context.schema, None);
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID,"queryId":READBACK_ID})
        ));
        inner.retire(Retirement::Eof);
        assert_eq!(
            operation.report().unwrap().statement_acknowledged,
            Some(retained)
        );
    }
    #[test]
    fn reviewed_ack_survives_retirement_before_phase_and_after_drain_failure() {
        for reason in [
            Retirement::Eof,
            Retirement::Protocol,
            Retirement::Write,
            Retirement::Requested,
            Retirement::Deadline,
            Retirement::Dropped,
        ] {
            for acknowledged in [false, true] {
                let (inner, operation) = dispatched_reviewed();
                if acknowledged {
                    assert!(switch_event(&inner, reviewed_ack()));
                }
                inner.retire(reason);
                let report = operation.report().unwrap();
                assert!(!report.ok);
                assert_eq!(
                    report.certainty,
                    if acknowledged {
                        Certainty::StatementSucceeded
                    } else {
                        Certainty::OutcomeUnknown
                    }
                );
                assert_eq!(report.statement_acknowledged.is_some(), acknowledged);
                assert_eq!(report.phase, Phase::Dispatching);
            }
        }
        for code in [
            "database",
            "cancelled",
            "connectionClosed",
            "outcomeUnknown",
            "resultLimitReached",
        ] {
            let (inner, operation) = dispatched_reviewed();
            assert!(switch_event(&inner, reviewed_ack()));
            assert!(switch_event(
                &inner,
                json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID})
            ));
            assert!(switch_event(
                &inner,
                json!({"type":"terminal","ok":false,"sdkRequestId":USE_ID,"error":{"code":code}})
            ));
            assert_eq!(
                operation.report().unwrap().certainty,
                Certainty::StatementSucceeded
            );
            assert!(operation.statement_acknowledged().is_some());
            assert!(inner.state.lock().unwrap().retired.is_some(), "{code}");
        }
    }
    #[test]
    fn reviewed_failed_terminal_recipient_cannot_admit_replacement_before_retirement() {
        let (inner, _normal, _control) = isolated();
        let operation = inner
            .request(
                "reviewed",
                "reviewedSql",
                json!({"credits":1}),
                Impact::Mutating,
            )
            .unwrap();
        // Retain both receiver queues: admission failure must prove retirement,
        // not merely a closed/full test queue. This queued probe proves admission
        // was available and will receive its own NotSubmitted retirement report.
        let queued_probe = inner
            .request("probe", "context", json!({}), Impact::Read)
            .unwrap();
        {
            let mut state = inner.state.lock().unwrap();
            let pending = state.pending.get_mut("reviewed").unwrap();
            pending.phase = Phase::Dispatching;
            pending.accepted = true;
            pending.statements.push(StatementIdentity {
                sdk_request_id: USE_ID.into(),
                query_id: None,
            });
            state.wire_ids.insert("1".into(), "reviewed".into());
        }
        assert!(switch_event(&inner, reviewed_ack()));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID})
        ));
        let terminal = operation.terminal_receiver();
        let recipient = inner.clone();
        TERMINAL_PUBLICATION_OBSERVER.with(|slot| {
            assert!(
                slot.borrow_mut()
                    .replace(Box::new(move |_inner| {
                        let report = terminal
                            .borrow()
                            .clone()
                            .expect("original terminal already published");
                        assert_eq!(report.certainty, Certainty::StatementSucceeded);
                        assert!(report.statement_acknowledged.is_some());
                        match recipient.request(
                            "replacement",
                            "reviewedSql",
                            json!({"credits":1}),
                            Impact::Mutating,
                        ) {
                            Err(error) => assert_eq!(error, "helper is retired or closing"),
                            Ok(_) => {
                                panic!("terminal recipient admitted replacement before retirement")
                            }
                        }
                    }))
                    .is_none()
            );
        });
        assert!(switch_event(
            &inner,
            json!({"type":"terminal","ok":false,"sdkRequestId":USE_ID,"error":{"code":"database"}})
        ));
        assert!(
            TERMINAL_PUBLICATION_OBSERVER.with(|slot| slot.borrow().is_none()),
            "publication observer ran"
        );
        assert_eq!(
            operation.report().unwrap().certainty,
            Certainty::StatementSucceeded
        );
        assert_eq!(
            queued_probe.report().unwrap().certainty,
            Certainty::NotSubmitted
        );
    }
    #[test]
    fn reviewed_failure_during_explicit_close_preserves_the_logout_admission_window() {
        let (inner, _normal, _control) = isolated();
        let operation = inner
            .request(
                "reviewed",
                "reviewedSql",
                json!({"credits":1}),
                Impact::Mutating,
            )
            .unwrap();
        {
            let mut state = inner.state.lock().unwrap();
            let pending = state.pending.get_mut("reviewed").unwrap();
            pending.phase = Phase::Dispatching;
            pending.accepted = true;
            pending.statements.push(StatementIdentity {
                sdk_request_id: USE_ID.into(),
                query_id: None,
            });
            state.wire_ids.insert("1".into(), "reviewed".into());
            // This is Supervisor::close's real interval after sealing admission
            // and before queued publications finish and close control is queued.
            state.closing = true;
        }
        assert!(switch_event(&inner, reviewed_ack()));
        assert!(switch_event(
            &inner,
            json!({"type":"terminal","ok":false,"sdkRequestId":USE_ID,"error":{"code":"database"}})
        ));
        let report = operation.report().unwrap();
        assert_eq!(report.certainty, Certainty::StatementSucceeded);
        assert!(report.statement_acknowledged.is_some());
        assert!(!inner.logout_acknowledged.load(Ordering::SeqCst));
        assert!(
            inner.state.lock().unwrap().retired.is_none(),
            "explicit close still owns logout"
        );
        assert!(matches!(
            inner.request(
                "replacement",
                "reviewedSql",
                json!({"credits":1}),
                Impact::Mutating
            ),
            Err("helper is retired or closing")
        ));
        let close = inner
            .control("close", json!({}))
            .expect("priority logout remains admitted");
        {
            let mut state = inner.state.lock().unwrap();
            state.pending.get_mut(&close.id).unwrap().phase = Phase::Sent;
            state.wire_ids.insert("2".into(), close.id.clone());
        }
        assert!(
            inner.event(json!({"v":1,"generation":inner.generation,"id":"2","type":"accepted"}))
        );
        assert!(inner.event(json!({"v":1,"generation":inner.generation,"id":"2","type":"terminal","ok":true,"result":{"closed":true,"logoutAcknowledged":true}})));
        assert!(close.report().unwrap().ok);
        assert!(inner.logout_acknowledged.load(Ordering::SeqCst));
        assert_eq!(
            operation.report().unwrap().statement_acknowledged,
            report.statement_acknowledged
        );
        inner.retire(Retirement::Requested);
    }
    #[tokio::test]
    async fn reviewed_sql_streams_multiple_rows_and_columns_with_generic_terminal() {
        let (inner, mut operation) = dispatched_reviewed();
        assert!(switch_event(&inner, reviewed_ack()));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"columns","columns":[{"name":"same","type":"TEXT"},{"name":"same","type":"TEXT"},{"name":"third","type":"TEXT"}]})
        ));
        let rows = json!([
            ["9007199254740993", null, "a"],
            ["2", "quoted \" text", "b"]
        ]);
        assert!(switch_event(&inner, json!({"type":"rows","rows":rows})));
        let batch = operation.rows.recv().await.unwrap();
        assert_eq!(batch.rows.len(), 2);
        assert_eq!(batch.rows[0].len(), 3);
        assert_eq!(batch.rows[0][0].as_deref(), Some("9007199254740993"));
        assert!(
            !switch_event(&inner, json!({"type":"rows","rows":[[null,null,null]]})),
            "no second batch credit"
        );
        for bad in [
            json!({"rowsSeen":1,"rowsRetained":2,"truncated":false,"sdkRequestId":USE_ID}),
            json!({"rowsSeen":2,"rowsRetained":2,"truncated":false,"sdkRequestId":USE_ID,"affectedRows":"2"}),
        ] {
            assert!(!switch_event(
                &inner,
                json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":bad})
            ));
            assert!(operation.report().is_none());
        }
        assert!(switch_event(
            &inner,
            json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":{"rowsSeen":2,"rowsRetained":2,"truncated":false,"sdkRequestId":USE_ID}})
        ));
        let report = operation.wait().await;
        assert!(report.ok);
        assert_eq!(report.certainty, Certainty::StatementSucceeded);
        assert_eq!(report.result.unwrap()["rowsRetained"], 2);
        assert!(inner.state.lock().unwrap().retired.is_none());
    }
    #[tokio::test]
    async fn reviewed_queued_close_and_stale_controls_cannot_touch_replacement() {
        let (inner, _normal, _control) = isolated();
        let supervisor = Supervisor {
            inner: inner.clone(),
            pid: 0,
            bounds: Bounds {
                exit: Duration::from_millis(1),
                ..Bounds::default()
            },
        };
        let mut queued = supervisor
            .request(
                "reviewed",
                "reviewedSql",
                json!({"credits":1}),
                Impact::Mutating,
            )
            .unwrap();
        inner.state.lock().unwrap().connect_sent = true;
        supervisor.close().await;
        assert_eq!(queued.wait().await.certainty, Certainty::NotSubmitted);
        assert!(queued.statement_acknowledged().is_none());
        let (inner, _normal, _control) = isolated();
        let mut old = inner
            .request(
                "same",
                "reviewedSql",
                json!({"credits":1}),
                Impact::Mutating,
            )
            .unwrap();
        let stale = old.token();
        assert!(inner.cancel(&stale).unwrap().wait().await.ok);
        assert_eq!(old.wait().await.certainty, Certainty::NotSubmitted);
        let replacement = inner
            .request(
                "same",
                "reviewedSql",
                json!({"credits":1}),
                Impact::Mutating,
            )
            .unwrap();
        assert!(inner.cancel(&stale).is_err());
        assert!(inner.discard(&stale).is_err());
        assert!(replacement.report().is_none());
        assert!(inner.state.lock().unwrap().retired.is_none());
        inner.retire(Retirement::Requested);
    }
    fn dispatched_switch() -> (Arc<Inner>, Operation) {
        let (inner, _normal, _control) = isolated();
        let op = inner
            .request("switch", "switchSchema", json!({}), Impact::ContextChange)
            .unwrap();
        {
            let mut state = inner.state.lock().unwrap();
            state.pending.get_mut("switch").unwrap().phase = Phase::Sent;
            state.wire_ids.insert("1".into(), "switch".into());
        }
        assert!(switch_event(&inner, json!({"type":"accepted"})));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"dispatching"})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":USE_ID})
        ));
        (inner, op)
    }
    fn dispatched_transaction(action: TransactionAction) -> (Arc<Inner>, Operation) {
        let (inner, _normal, _control) = isolated();
        let op = inner
            .request(
                "transaction",
                "transaction",
                json!({"action":action.as_str(),"expected":{"user":"USER","session":"OPAQUE"}}),
                action.impact(),
            )
            .unwrap();
        {
            let mut state = inner.state.lock().unwrap();
            state.pending.get_mut("transaction").unwrap().phase = Phase::Sent;
            state.wire_ids.insert("1".into(), "transaction".into());
        }
        assert!(switch_event(&inner, json!({"type":"accepted"})));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"dispatching"})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":USE_ID})
        ));
        (inner, op)
    }
    fn dispatched_dml(kind: SnowflakeDmlKind) -> (Arc<Inner>, Operation) {
        let (inner, _normal, _control) = isolated();
        let op = inner
            .request("dml", "dml", json!({"kind":kind}), Impact::Mutating)
            .unwrap();
        {
            let mut state = inner.state.lock().unwrap();
            state.pending.get_mut("dml").unwrap().phase = Phase::Sent;
            state.wire_ids.insert("1".into(), "dml".into());
        }
        assert!(switch_event(&inner, json!({"type":"accepted"})));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"dispatching"})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":USE_ID})
        ));
        (inner, op)
    }
    #[tokio::test]
    async fn dml_impact_and_kind_are_paired_and_queued_cancel_has_no_submission() {
        let (inner, _normal, _control) = isolated();
        for kind in [
            SnowflakeDmlKind::Insert,
            SnowflakeDmlKind::Update,
            SnowflakeDmlKind::Delete,
        ] {
            for impact in [
                Impact::Read,
                Impact::Control,
                Impact::Begin,
                Impact::Commit,
                Impact::Rollback,
                Impact::ContextChange,
            ] {
                assert!(
                    inner
                        .request("bad", "dml", json!({"kind":kind}), impact)
                        .is_err()
                );
            }
            let mut op = inner
                .request("dml", "dml", json!({"kind":kind}), Impact::Mutating)
                .unwrap();
            assert!(inner.discard(&op.token()).is_err());
            assert!(inner.cancel(&op.token()).unwrap().wait().await.ok);
            let report = op.wait().await;
            assert_eq!(report.certainty, Certainty::NotSubmitted);
            assert!(report.dml_acknowledged.is_none());
            assert!(inner.state.lock().unwrap().retired.is_none());
        }
        for kind in [
            json!("INSERT"),
            json!("merge"),
            json!(["insert"]),
            Value::Null,
        ] {
            assert!(
                inner
                    .request("bad", "dml", json!({"kind":kind}), Impact::Mutating)
                    .is_err()
            );
        }
    }
    #[tokio::test]
    async fn dml_ack_survives_all_retirements_with_exact_statement_identity() {
        for kind in [
            SnowflakeDmlKind::Insert,
            SnowflakeDmlKind::Update,
            SnowflakeDmlKind::Delete,
        ] {
            for reason in [
                Retirement::Eof,
                Retirement::Protocol,
                Retirement::Deadline,
                Retirement::Dropped,
                Retirement::Requested,
                Retirement::Write,
            ] {
                for acknowledged in [false, true] {
                    let (inner, mut op) = dispatched_dml(kind);
                    if acknowledged {
                        assert!(switch_event(
                            &inner,
                            json!({"type":"dmlAcknowledged","kind":kind,"sdkRequestId":USE_ID})
                        ));
                        assert_eq!(op.dml_acknowledged().unwrap().kind, kind);
                        assert!(switch_event(
                            &inner,
                            json!({"type":"statement","sdkRequestId":USE_ID,"queryId":READBACK_ID})
                        ));
                    }
                    inner.retire(reason);
                    let report = op.wait().await;
                    assert!(!report.ok);
                    assert_eq!(
                        report.certainty,
                        if acknowledged {
                            Certainty::StatementSucceeded
                        } else {
                            Certainty::OutcomeUnknown
                        }
                    );
                    assert_eq!(report.dml_acknowledged.is_some(), acknowledged);
                    if let Some(ack) = report.dml_acknowledged {
                        assert_eq!(ack.kind, kind);
                        assert_eq!(ack.statement, report.statements[0]);
                        assert_eq!(ack.statement.query_id.as_deref(), Some(READBACK_ID));
                    }
                }
            }
        }
    }
    #[tokio::test]
    async fn dml_ack_terminal_and_summary_require_exact_association_and_closed_shape() {
        let (inner, mut op) = dispatched_dml(SnowflakeDmlKind::Update);
        for event in [
            json!({"type":"dmlAcknowledged","kind":"update"}),
            json!({"type":"dmlAcknowledged","kind":"delete","sdkRequestId":USE_ID}),
            json!({"type":"dmlAcknowledged","kind":"update","sdkRequestId":READBACK_ID}),
            json!({"type":"dmlAcknowledged","kind":"update","sdkRequestId":USE_ID,"extra":true}),
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID}),
            json!({"type":"boundaryAcknowledged","action":"commit","sdkRequestId":USE_ID}),
        ] {
            assert!(!switch_event(&inner, event));
        }
        assert!(op.dml_acknowledged().is_none());
        assert!(switch_event(
            &inner,
            json!({"type":"dmlAcknowledged","kind":"update","sdkRequestId":USE_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"dmlAcknowledged","kind":"update","sdkRequestId":USE_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID})
        ));
        let result = json!({"kind":"update","rowsSeen":1,"rowsRetained":0,"truncated":false,
            "columns":[{"name":"number of rows updated","type":"fixed"}],"values":[["9007199254740993"]],"sdkRequestId":USE_ID});
        for (key, bad) in [
            ("kind", json!("delete")),
            ("rowsSeen", json!(0)),
            ("rowsRetained", json!(1)),
            ("truncated", Value::Null),
            ("values", json!([[1]])),
            ("values", json!([["1"], ["2"]])),
            ("sdkRequestId", json!(READBACK_ID)),
            ("extra", json!(true)),
        ] {
            let mut malformed = result.clone();
            malformed[key] = bad;
            assert!(
                !switch_event(
                    &inner,
                    json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":malformed})
                ),
                "{key}"
            );
        }
        assert!(switch_event(
            &inner,
            json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":result})
        ));
        let report = op.wait().await;
        assert!(report.ok);
        assert_eq!(report.certainty, Certainty::StatementSucceeded);
        assert_eq!(report.result.unwrap()["values"][0][0], "9007199254740993");
        assert!(report.dml_acknowledged.is_some());
    }
    #[tokio::test]
    async fn dml_database_rejection_and_post_ack_drain_loss_keep_distinct_evidence() {
        for acknowledged in [false, true] {
            let (inner, mut op) = dispatched_dml(SnowflakeDmlKind::Delete);
            if acknowledged {
                assert!(switch_event(
                    &inner,
                    json!({"type":"dmlAcknowledged","kind":"delete","sdkRequestId":USE_ID})
                ));
            }
            assert!(switch_event(
                &inner,
                json!({"type":"terminal","ok":false,"sdkRequestId":USE_ID,"error":{"code":"database"}})
            ));
            let report = op.wait().await;
            assert_eq!(
                report.certainty,
                if acknowledged {
                    Certainty::StatementSucceeded
                } else {
                    Certainty::ServerTerminal
                }
            );
            assert_eq!(report.dml_acknowledged.is_some(), acknowledged);
            assert_eq!(inner.state.lock().unwrap().retired.is_some(), acknowledged);
        }
    }
    #[test]
    fn dropped_dispatched_dml_retires_but_completed_cleanup_cannot_retire_replacement() {
        let (inner, op) = dispatched_dml(SnowflakeDmlKind::Insert);
        drop(op);
        assert_eq!(
            inner.state.lock().unwrap().retired,
            Some(Retirement::Dropped)
        );
        // Keep the synthetic transport receivers alive for the replacement.
        let (inner, _normal, _control) = isolated();
        let old = inner
            .request("dml", "dml", json!({"kind":"delete"}), Impact::Mutating)
            .unwrap();
        inner
            .state
            .lock()
            .unwrap()
            .pending
            .get_mut("dml")
            .unwrap()
            .phase = Phase::Dispatching;
        let pending = inner.state.lock().unwrap().pending.remove("dml").unwrap();
        inner.state.lock().unwrap().wire_ids.clear();
        pending.terminal.send_replace(Some(local_report(json!({}))));
        let replacement = inner
            .request("dml", "dml", json!({"kind":"insert"}), Impact::Mutating)
            .unwrap();
        old.cleanup_unfinished();
        assert!(replacement.report().is_none());
        assert!(inner.state.lock().unwrap().retired.is_none());
    }
    fn dispatched_ddl(kind: SnowflakeDdlKind) -> (Arc<Inner>, Operation) {
        let (inner, _normal, _control) = isolated();
        let op = inner
            .request("ddl", "ddl", json!({"kind":kind}), Impact::Mutating)
            .unwrap();
        {
            let mut state = inner.state.lock().unwrap();
            state.pending.get_mut("ddl").unwrap().phase = Phase::Sent;
            state.wire_ids.insert("1".into(), "ddl".into());
        }
        assert!(switch_event(&inner, json!({"type":"accepted"})));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"dispatching"})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":USE_ID})
        ));
        (inner, op)
    }
    #[tokio::test]
    async fn ddl_impact_and_kind_are_paired_and_queued_cancel_has_no_submission() {
        let (inner, _normal, _control) = isolated();
        for kind in [
            SnowflakeDdlKind::CreateTable,
            SnowflakeDdlKind::AddColumn,
            SnowflakeDdlKind::DropTable,
        ] {
            for impact in [
                Impact::Read,
                Impact::Control,
                Impact::Begin,
                Impact::Commit,
                Impact::Rollback,
                Impact::ContextChange,
            ] {
                assert!(
                    inner
                        .request("bad", "ddl", json!({"kind":kind}), impact)
                        .is_err()
                );
            }
            let mut op = inner
                .request("ddl", "ddl", json!({"kind":kind}), Impact::Mutating)
                .unwrap();
            assert!(inner.discard(&op.token()).is_err());
            assert!(inner.cancel(&op.token()).unwrap().wait().await.ok);
            let report = op.wait().await;
            assert_eq!(report.certainty, Certainty::NotSubmitted);
            assert!(report.ddl_acknowledged.is_none());
            assert!(inner.state.lock().unwrap().retired.is_none());
        }
        for kind in [
            json!("create_table"),
            json!("alterTable"),
            json!(["createTable"]),
            Value::Null,
        ] {
            assert!(
                inner
                    .request("bad", "ddl", json!({"kind":kind}), Impact::Mutating)
                    .is_err()
            );
        }
    }
    #[tokio::test]
    async fn ddl_ack_terminal_and_summary_require_exact_association_and_closed_shape() {
        let (inner, mut op) = dispatched_ddl(SnowflakeDdlKind::CreateTable);
        for event in [
            json!({"type":"ddlAcknowledged","kind":"createTable"}),
            json!({"type":"ddlAcknowledged","kind":"dropTable","sdkRequestId":USE_ID}),
            json!({"type":"ddlAcknowledged","kind":"createTable","sdkRequestId":READBACK_ID}),
            json!({"type":"ddlAcknowledged","kind":"createTable","sdkRequestId":USE_ID,"extra":true}),
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID}),
            json!({"type":"dmlAcknowledged","kind":"insert","sdkRequestId":USE_ID}),
        ] {
            assert!(!switch_event(&inner, event));
        }
        assert!(op.ddl_acknowledged().is_none());
        assert!(switch_event(
            &inner,
            json!({"type":"ddlAcknowledged","kind":"createTable","sdkRequestId":USE_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"ddlAcknowledged","kind":"createTable","sdkRequestId":USE_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID})
        ));
        let result = json!({"kind":"createTable","rowsSeen":1,"rowsRetained":0,"truncated":false,
            "columns":[{"name":"status","type":"text"}],"values":[["Table T successfully created."]],"sdkRequestId":USE_ID});
        for (key, bad) in [
            ("kind", json!("dropTable")),
            ("rowsSeen", json!(0)),
            ("rowsRetained", json!(1)),
            ("truncated", Value::Null),
            ("values", json!([[1]])),
            ("values", json!([["a"], ["b"]])),
            ("sdkRequestId", json!(READBACK_ID)),
            ("extra", json!(true)),
        ] {
            let mut malformed = result.clone();
            malformed[key] = bad;
            assert!(
                !switch_event(
                    &inner,
                    json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":malformed})
                ),
                "{key}"
            );
        }
        assert!(switch_event(
            &inner,
            json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":result})
        ));
        let report = op.wait().await;
        assert!(report.ok);
        assert_eq!(report.certainty, Certainty::StatementSucceeded);
        assert_eq!(
            report.result.unwrap()["values"][0][0],
            "Table T successfully created."
        );
        assert!(report.ddl_acknowledged.is_some());
    }
    #[tokio::test]
    async fn ddl_database_rejection_and_post_ack_drain_loss_keep_distinct_evidence() {
        for acknowledged in [false, true] {
            let (inner, mut op) = dispatched_ddl(SnowflakeDdlKind::DropTable);
            if acknowledged {
                assert!(switch_event(
                    &inner,
                    json!({"type":"ddlAcknowledged","kind":"dropTable","sdkRequestId":USE_ID})
                ));
            }
            assert!(switch_event(
                &inner,
                json!({"type":"terminal","ok":false,"sdkRequestId":USE_ID,"error":{"code":"database"}})
            ));
            let report = op.wait().await;
            assert_eq!(
                report.certainty,
                if acknowledged {
                    Certainty::StatementSucceeded
                } else {
                    Certainty::ServerTerminal
                }
            );
            assert_eq!(report.ddl_acknowledged.is_some(), acknowledged);
            assert_eq!(inner.state.lock().unwrap().retired.is_some(), acknowledged);
        }
    }
    #[tokio::test]
    async fn ddl_ack_survives_all_retirements_with_exact_statement_identity() {
        for kind in [
            SnowflakeDdlKind::CreateTable,
            SnowflakeDdlKind::AddColumn,
            SnowflakeDdlKind::DropTable,
        ] {
            for reason in [
                Retirement::Eof,
                Retirement::Protocol,
                Retirement::Deadline,
                Retirement::Dropped,
                Retirement::Requested,
                Retirement::Write,
            ] {
                for acknowledged in [false, true] {
                    let (inner, mut op) = dispatched_ddl(kind);
                    if acknowledged {
                        assert!(switch_event(
                            &inner,
                            json!({"type":"ddlAcknowledged","kind":kind,"sdkRequestId":USE_ID})
                        ));
                        assert_eq!(op.ddl_acknowledged().unwrap().kind, kind);
                        assert!(switch_event(
                            &inner,
                            json!({"type":"statement","sdkRequestId":USE_ID,"queryId":READBACK_ID})
                        ));
                    }
                    inner.retire(reason);
                    let report = op.wait().await;
                    assert!(!report.ok);
                    assert_eq!(report.retirement, Some(reason));
                    assert_eq!(
                        report.certainty,
                        if acknowledged {
                            Certainty::StatementSucceeded
                        } else {
                            Certainty::OutcomeUnknown
                        }
                    );
                    assert_eq!(report.ddl_acknowledged.is_some(), acknowledged);
                    if let Some(ack) = report.ddl_acknowledged {
                        assert_eq!(ack.kind, kind);
                        assert_eq!(ack.statement, report.statements[0]);
                        assert_eq!(ack.statement.query_id.as_deref(), Some(READBACK_ID));
                    }
                }
            }
        }
    }
    #[tokio::test]
    async fn transaction_action_and_impact_are_paired_and_queued_cancel_is_local() {
        let (inner, _normal, _control) = isolated();
        for action in [
            TransactionAction::Begin,
            TransactionAction::Commit,
            TransactionAction::Rollback,
        ] {
            for impact in [
                Impact::Read,
                Impact::Mutating,
                Impact::ContextChange,
                Impact::Control,
            ] {
                assert!(
                    inner
                        .request(
                            "bad",
                            "transaction",
                            json!({"action":action.as_str()}),
                            impact
                        )
                        .is_err()
                );
            }
            assert!(
                inner
                    .request("bad", "query", json!({}), action.impact())
                    .is_err()
            );
            let mut op = inner
                .request(
                    action.as_str(),
                    "transaction",
                    json!({"action":action.as_str()}),
                    action.impact(),
                )
                .unwrap();
            assert!(inner.discard(&op.token()).is_err());
            assert!(inner.cancel(&op.token()).unwrap().wait().await.ok);
            let report = op.wait().await;
            assert_eq!(report.certainty, Certainty::NotSubmitted);
            assert!(report.boundary_acknowledged.is_none());
            assert!(inner.state.lock().unwrap().retired.is_none());
        }
        assert!(
            inner
                .request(
                    "bad",
                    "transaction",
                    json!({"action":"COMMIT"}),
                    Impact::Commit
                )
                .is_err()
        );
    }
    #[tokio::test]
    async fn transaction_ack_is_statement_success_for_every_retirement_not_disposition() {
        for action in [
            TransactionAction::Begin,
            TransactionAction::Commit,
            TransactionAction::Rollback,
        ] {
            for reason in [
                Retirement::Eof,
                Retirement::Protocol,
                Retirement::Deadline,
                Retirement::Dropped,
                Retirement::Requested,
                Retirement::Write,
            ] {
                let (inner, mut op) = dispatched_transaction(action);
                assert!(switch_event(
                    &inner,
                    json!({"type":"boundaryAcknowledged","action":action.as_str(),"sdkRequestId":USE_ID})
                ));
                assert_eq!(op.boundary_acknowledged().unwrap().action, action);
                assert!(switch_event(
                    &inner,
                    json!({"type":"statement","sdkRequestId":USE_ID,"queryId":READBACK_ID})
                ));
                inner.retire(reason);
                let report = op.wait().await;
                assert!(!report.ok);
                assert_eq!(report.phase, Phase::Dispatching);
                assert_eq!(report.certainty, Certainty::StatementSucceeded);
                let ack = report.boundary_acknowledged.unwrap();
                assert_eq!(ack.action, action);
                assert_eq!(ack.statement, report.statements[0]);
                assert_eq!(ack.statement.query_id.as_deref(), Some(READBACK_ID));
            }
            let (inner, mut op) = dispatched_transaction(action);
            inner.retire(Retirement::Eof);
            assert_eq!(op.wait().await.certainty, Certainty::OutcomeUnknown);
        }
    }
    #[tokio::test]
    async fn transaction_ack_and_terminal_require_exact_action_and_statement_identity() {
        let (inner, mut op) = dispatched_transaction(TransactionAction::Commit);
        for event in [
            json!({"type":"boundaryAcknowledged","action":"commit"}),
            json!({"type":"boundaryAcknowledged","action":"rollback","sdkRequestId":USE_ID}),
            json!({"type":"boundaryAcknowledged","action":"commit","sdkRequestId":READBACK_ID}),
            json!({"type":"boundaryAcknowledged","action":"commit","sdkRequestId":USE_ID,"extra":true}),
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID}),
        ] {
            assert!(!switch_event(&inner, event));
        }
        assert!(op.boundary_acknowledged().is_none());
        assert!(switch_event(
            &inner,
            json!({"type":"boundaryAcknowledged","action":"commit","sdkRequestId":USE_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"boundaryAcknowledged","action":"commit","sdkRequestId":USE_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged"})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":READBACK_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID})
        ));
        for event in [
            json!({"type":"terminal","ok":true,"result":{"action":"commit","sdkRequestId":USE_ID}}),
            json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":{"action":"commit"}}),
            json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":{"action":"rollback","sdkRequestId":USE_ID}}),
            json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":{"action":"commit","sdkRequestId":READBACK_ID}}),
        ] {
            assert!(!switch_event(&inner, event));
        }
        assert!(switch_event(
            &inner,
            json!({"type":"terminal","ok":true,"sdkRequestId":USE_ID,"result":{"action":"commit","sdkRequestId":USE_ID}})
        ));
        let report = op.wait().await;
        assert!(report.ok);
        assert_eq!(report.certainty, Certainty::StatementSucceeded);
        assert!(report.boundary_acknowledged.is_some());
        assert!(!switch_event(
            &inner,
            json!({"type":"boundaryAcknowledged","action":"rollback","sdkRequestId":USE_ID})
        ));
        assert!(op.report().unwrap().ok);
    }
    #[tokio::test]
    async fn transaction_known_ack_survives_failure_but_database_rejection_has_no_ack() {
        for acknowledged in [false, true] {
            let (inner, mut op) = dispatched_transaction(TransactionAction::Begin);
            if acknowledged {
                assert!(switch_event(
                    &inner,
                    json!({"type":"boundaryAcknowledged","action":"begin","sdkRequestId":USE_ID})
                ));
            }
            assert!(switch_event(
                &inner,
                json!({"type":"terminal","ok":false,"error":{"code":"database"},"sdkRequestId":USE_ID})
            ));
            let report = op.wait().await;
            assert_eq!(report.boundary_acknowledged.is_some(), acknowledged);
            assert_eq!(
                report.certainty,
                if acknowledged {
                    Certainty::StatementSucceeded
                } else {
                    Certainty::ServerTerminal
                }
            );
            assert_eq!(inner.state.lock().unwrap().retired.is_some(), acknowledged);
        }
    }
    #[test]
    fn completed_transaction_cleanup_cannot_retire_replacement_work() {
        let (inner, _normal, _control) = isolated();
        let old = inner
            .request(
                "same",
                "transaction",
                json!({"action":"begin"}),
                Impact::Begin,
            )
            .unwrap();
        let pending = inner.state.lock().unwrap().pending.remove("same").unwrap();
        pending.terminal.send_replace(Some(local_report(json!({}))));
        let replacement = inner
            .request(
                "same",
                "transaction",
                json!({"action":"rollback"}),
                Impact::Rollback,
            )
            .unwrap();
        old.cleanup_unfinished();
        assert!(replacement.report().is_none());
        assert!(inner.state.lock().unwrap().retired.is_none());
        inner.retire(Retirement::Requested);
    }
    #[tokio::test]
    async fn context_change_unsent_cancel_is_not_submitted_and_cannot_discard() {
        let (inner, _normal, _control) = isolated();
        let mut op = inner
            .request("switch", "switchSchema", json!({}), Impact::ContextChange)
            .unwrap();
        assert!(inner.discard(&op.token()).is_err());
        assert!(inner.cancel(&op.token()).unwrap().wait().await.ok);
        let report = op.wait().await;
        assert_eq!(report.certainty, Certainty::NotSubmitted);
        assert!(report.use_acknowledged.is_none());
        assert!(inner.state.lock().unwrap().retired.is_none());
    }
    #[tokio::test]
    async fn use_acknowledgement_survives_all_retirement_causes_without_final_success() {
        for reason in [
            Retirement::Eof,
            Retirement::Protocol,
            Retirement::Deadline,
            Retirement::Dropped,
            Retirement::Requested,
            Retirement::Write,
        ] {
            let (inner, mut op) = dispatched_switch();
            assert!(switch_event(
                &inner,
                json!({"type":"useAcknowledged","sdkRequestId":USE_ID})
            ));
            assert!(switch_event(
                &inner,
                json!({"type":"statement","sdkRequestId":USE_ID,"queryId":"00000000-0000-4000-8000-000000000111"})
            ));
            assert!(switch_event(
                &inner,
                json!({"type":"statement","sdkRequestId":READBACK_ID})
            ));
            inner.retire(reason);
            let report = op.wait().await;
            assert_eq!(report.certainty, Certainty::OutcomeUnknown);
            assert_eq!(report.phase, Phase::Dispatching);
            assert_eq!(
                report.use_acknowledged.as_ref().unwrap().sdk_request_id,
                USE_ID
            );
            assert_eq!(report.statements.len(), 2);
            assert_eq!(report.use_acknowledged, report.statements.first().cloned());
            assert!(!report.ok);
        }
    }
    #[tokio::test]
    async fn readback_database_failure_cannot_reclassify_acknowledged_use_as_rejected() {
        let (inner, mut op) = dispatched_switch();
        assert!(switch_event(
            &inner,
            json!({"type":"useAcknowledged","sdkRequestId":USE_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":READBACK_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"terminal","ok":false,"error":{"code":"database"},"sdkRequestId":READBACK_ID})
        ));
        let report = op.wait().await;
        assert_eq!(report.certainty, Certainty::OutcomeUnknown);
        assert!(report.use_acknowledged.is_some());
        assert_eq!(
            inner.state.lock().unwrap().retired,
            Some(Retirement::Requested)
        );
    }
    #[test]
    fn use_ack_and_readback_order_and_statement_budgets_are_strict() {
        let (inner, op) = dispatched_switch();
        assert!(!switch_event(&inner, json!({"type":"useAcknowledged"})));
        assert!(!switch_event(
            &inner,
            json!({"type":"useAcknowledged","sdkRequestId":READBACK_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":READBACK_ID})
        ));
        // Invalid events retire in the read loop. Recreate rather than accepting
        // follow-up traffic after a deliberately invalid event mutated counters.
        inner.retire(Retirement::Protocol);
        drop(op);
        let (inner, op) = dispatched_switch();
        assert!(switch_event(
            &inner,
            json!({"type":"useAcknowledged","sdkRequestId":USE_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"useAcknowledged","sdkRequestId":USE_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":READBACK_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":"00000000-0000-4000-8000-000000000013"})
        ));
        inner.retire(Retirement::Protocol);
        drop(op);
    }
    #[test]
    fn completed_switch_cleanup_cannot_retire_replacement_work() {
        let (inner, _normal, _control) = isolated();
        let old = inner
            .request("same", "switchSchema", json!({}), Impact::ContextChange)
            .unwrap();
        // Model terminal publication between Drop's report check and cleanup.
        assert!(old.report().is_none());
        let pending = inner.state.lock().unwrap().pending.remove("same").unwrap();
        pending.terminal.send_replace(Some(local_report(json!({}))));
        let replacement = inner
            .request("same", "switchSchema", json!({}), Impact::ContextChange)
            .unwrap();
        old.cleanup_unfinished();
        assert!(replacement.report().is_none());
        assert!(inner.state.lock().unwrap().retired.is_none());
        inner.retire(Retirement::Requested);
    }
    #[tokio::test]
    async fn switch_final_ack_and_success_terminal_require_readback_identity() {
        let (inner, mut op) = dispatched_switch();
        assert!(switch_event(
            &inner,
            json!({"type":"useAcknowledged","sdkRequestId":USE_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"statement","sdkRequestId":READBACK_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged"})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":USE_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"phase","phase":"acknowledged","sdkRequestId":READBACK_ID})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"terminal","ok":true,"result":{}})
        ));
        assert!(!switch_event(
            &inner,
            json!({"type":"terminal","ok":true,"result":{},"sdkRequestId":USE_ID})
        ));
        assert!(switch_event(
            &inner,
            json!({"type":"terminal","ok":true,"result":{},"sdkRequestId":READBACK_ID})
        ));
        let report = op.wait().await;
        assert!(report.ok);
        assert_eq!(report.certainty, Certainty::StatementSucceeded);
        assert_eq!(report.use_acknowledged.unwrap().sdk_request_id, USE_ID);
        assert_eq!(
            report.statements.last().unwrap().sdk_request_id,
            READBACK_ID
        );
    }
    #[tokio::test]
    async fn cancelled_unsent_handle_reuse_cannot_execute_old_outbound_payload() {
        let (inner, normal, control) = isolated();
        let mut old = inner
            .request("same", "query", json!({"sql":"OLD"}), Impact::Mutating)
            .unwrap();
        let mut cancellation = inner.cancel(&old.token()).unwrap();
        assert!(cancellation.wait().await.ok);
        assert_eq!(old.wait().await.certainty, Certainty::NotSubmitted);
        let _new = inner
            .request("same", "query", json!({"sql":"NEW"}), Impact::Mutating)
            .unwrap();
        let (pipe, mut reader) = tokio::io::duplex(4096);
        let task = tokio::spawn(write_loop(
            inner.clone(),
            pipe,
            normal,
            control,
            Duration::from_secs(1),
        ));
        let frame = timeout(
            Duration::from_secs(1),
            read_frame(&mut reader, Duration::from_secs(1)),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert_eq!(frame["payload"]["sql"], "NEW");
        assert_eq!(frame["id"], "1");
        inner.retire(Retirement::Requested);
        task.await.unwrap();
    }
    #[tokio::test]
    async fn late_control_cannot_cancel_new_operation_reusing_a_local_handle() {
        let (inner, normal, control) = isolated();
        let mut old = inner
            .request("same", "query", json!({"sql":"OLD"}), Impact::Read)
            .unwrap();
        // Simulate a fully transmitted old query, leaving its old outbound in the queue.
        {
            let mut state = inner.state.lock().unwrap();
            let pending = state.pending.get_mut("same").unwrap();
            pending.phase = Phase::Sent;
            pending.wire_id = Some("1".into());
        }
        let mut cancel = inner.cancel(&old.token()).unwrap();
        {
            let mut state = inner.state.lock().unwrap();
            let pending = state.pending.remove("same").unwrap();
            pending.terminal.send_replace(Some(local_report(json!({}))));
        }
        assert!(old.wait().await.ok);
        let _new = inner
            .request("same", "query", json!({"sql":"NEW"}), Impact::Read)
            .unwrap();
        let (pipe, mut reader) = tokio::io::duplex(4096);
        let task = tokio::spawn(write_loop(
            inner.clone(),
            pipe,
            normal,
            control,
            Duration::from_secs(1),
        ));
        let report = cancel.wait_bounded(Duration::from_secs(1)).await;
        assert_eq!(report.result.unwrap()["inactive"], true);
        let frame = timeout(
            Duration::from_secs(1),
            read_frame(&mut reader, Duration::from_secs(1)),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert_eq!(frame["op"], "query");
        assert_eq!(frame["payload"]["sql"], "NEW");
        inner.retire(Retirement::Requested);
        task.await.unwrap();
    }
    #[tokio::test]
    async fn stale_completed_operation_cannot_cancel_reused_local_id() {
        let (inner, _normal, _control) = isolated();
        let mut old = inner
            .request("same", "query", json!({}), Impact::Read)
            .unwrap();
        {
            let pending = inner.state.lock().unwrap().pending.remove("same").unwrap();
            pending.terminal.send_replace(Some(local_report(json!({}))));
        }
        assert!(old.wait().await.ok);
        let new = inner
            .request("same", "query", json!({}), Impact::Read)
            .unwrap();
        assert!(
            inner.cancel(&old.token()).is_err(),
            "stale operation cancelled replacement"
        );
        assert!(new.report().is_none());
        inner.retire(Retirement::Requested);
    }
    #[tokio::test]
    async fn tokens_cannot_cross_supervisors_even_with_identical_generation_id_and_ticket() {
        let (first, _normal, _control) = isolated();
        let (second, _normal2, _control2) = isolated();
        let old = first
            .request("same", "query", json!({}), Impact::Read)
            .unwrap();
        let new = second
            .request("same", "query", json!({}), Impact::Read)
            .unwrap();
        assert_eq!(old.token.ticket, new.token.ticket);
        assert!(second.cancel(&old.token()).is_err());
        assert!(second.discard(&old.token()).is_err());
        assert!(new.report().is_none());
        first.retire(Retirement::Requested);
        second.retire(Retirement::Requested);
    }
    #[test]
    fn terminal_and_id_reuse_between_drop_check_and_cleanup_keep_replacement_live() {
        let (inner, _normal, _control) = isolated();
        let old = inner
            .request("same", "query", json!({}), Impact::Read)
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let worker_barrier = barrier.clone();
        let worker = std::thread::spawn(move || {
            // This is the exact report-check/cleanup seam used by Drop.
            assert!(old.report().is_none());
            worker_barrier.wait();
            worker_barrier.wait();
            old.cleanup_unfinished();
        });
        barrier.wait();
        let pending = inner.state.lock().unwrap().pending.remove("same").unwrap();
        pending.terminal.send_replace(Some(local_report(json!({}))));
        let replacement = inner
            .request("same", "query", json!({}), Impact::Read)
            .unwrap();
        barrier.wait();
        worker.join().unwrap();
        assert!(
            replacement.report().is_none(),
            "late Drop retired the replacement"
        );
        assert!(inner.state.lock().unwrap().retired.is_none());
        inner.retire(Retirement::Requested);
    }
    #[test]
    fn unfinished_query_drop_still_retires_when_discard_control_cannot_be_queued() {
        let (inner, _normal, _control) = isolated();
        let old = inner
            .request("same", "query", json!({}), Impact::Read)
            .unwrap();
        inner
            .state
            .lock()
            .unwrap()
            .pending
            .get_mut("same")
            .unwrap()
            .phase = Phase::Sent;
        let mut controls = Vec::new();
        for _ in 0..16 {
            controls.push(inner.control("cancel", json!({"target":"same"})).unwrap());
        }
        drop(old);
        assert_eq!(
            inner.state.lock().unwrap().retired,
            Some(Retirement::Dropped)
        );
        assert!(
            controls
                .iter()
                .all(|operation| operation.report().is_some())
        );
    }
    #[test]
    fn asynchronous_close_future_can_be_owned_by_a_tokio_shutdown_task() {
        let (inner, _, _) = isolated();
        let supervisor = Supervisor {
            inner,
            pid: 0,
            bounds: Bounds::default(),
        };
        fn assert_send<T: Send>(_: T) {}
        assert_send(supervisor.close());
    }
    #[tokio::test]
    async fn priority_control_ids_follow_actual_write_order_with_bounded_live_mapping() {
        let (inner, normal, control) = isolated();
        let _query = inner
            .request("first-created", "query", json!({}), Impact::Read)
            .unwrap();
        let _close = inner.control("close", json!({})).unwrap();
        let _context = inner
            .request("last-created", "context", json!({}), Impact::Read)
            .unwrap();
        let (pipe, mut reader) = tokio::io::duplex(4096);
        let task = tokio::spawn(write_loop(
            inner.clone(),
            pipe,
            normal,
            control,
            Duration::from_secs(1),
        ));
        for (id, op) in [("1", "close"), ("2", "query"), ("3", "context")] {
            let frame = timeout(
                Duration::from_secs(1),
                read_frame(&mut reader, Duration::from_secs(1)),
            )
            .await
            .unwrap()
            .unwrap()
            .unwrap();
            assert_eq!(frame["id"], id);
            assert_eq!(frame["op"], op);
        }
        assert_eq!(inner.state.lock().unwrap().wire_ids.len(), 3);
        inner.retire(Retirement::Requested);
        assert!(inner.state.lock().unwrap().wire_ids.is_empty());
        task.await.unwrap();
    }
}

#[cfg(all(test, unix))]
mod cleanup_tests {
    use super::*;
    use std::sync::{Condvar, atomic::AtomicUsize};

    const LIMIT: Duration = Duration::from_secs(3);

    fn launch(parent: &std::path::Path) -> Launch {
        Launch::new(
            PathBuf::from("/missing-snowflake-node"),
            None,
            parent.to_owned(),
            tempfile::tempdir_in(parent).unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn root_retry_preserves_final_no_child_and_reaped_dispositions() {
        for child_started in [false, true] {
            let parent = tempfile::tempdir().unwrap();
            let mut launch = launch(parent.path());
            let path = launch.state_directory().to_owned();
            let cleanup = launch.cleanup_observer();
            let authority = cleanup.authority.as_ref().unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let calls_for_remove = calls.clone();
            *authority.before_remove.lock().unwrap() = Some(Arc::new(move || {
                if calls_for_remove.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(std::io::Error::other("controlled removal failure"))
                } else {
                    Ok(())
                }
            }));
            if child_started {
                // Unit-test the final disposition supplied by the actual reaper.
                // The process integration test separately proves real child reaping.
                let _ = launch.state_directory.take();
                cleanup.finish(true, true);
            }
            drop(launch);
            assert_eq!(
                cleanup.report(),
                Some(CleanupReport {
                    child_started,
                    reaped: child_started,
                    temporary_root_removed: false,
                })
            );
            assert!(path.is_dir());
            assert_eq!(
                cleanup.settle(LIMIT).await,
                Some(CleanupReport {
                    child_started,
                    reaped: child_started,
                    temporary_root_removed: true,
                })
            );
            assert!(!path.exists());
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            assert_eq!(cleanup.settle(LIMIT).await, cleanup.report());
            assert_eq!(
                calls.load(Ordering::SeqCst),
                2,
                "completed roots must not retry"
            );
        }
    }

    #[tokio::test]
    async fn missing_or_unreaped_disposition_cannot_start_removal() {
        let parent = tempfile::tempdir().unwrap();
        let mut launch = launch(parent.path());
        let path = launch.state_directory().to_owned();
        let cleanup = launch.cleanup_observer();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_remove = calls.clone();
        *cleanup
            .authority
            .as_ref()
            .unwrap()
            .before_remove
            .lock()
            .unwrap() = Some(Arc::new(move || {
            calls_for_remove.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        assert!(cleanup.settle(Duration::ZERO).await.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let _ = launch.state_directory.take();
        cleanup.finish(true, false);
        drop(launch);
        assert_eq!(
            cleanup.settle(LIMIT).await,
            Some(CleanupReport {
                child_started: true,
                reaped: false,
                temporary_root_removed: false,
            })
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(path.is_dir());
    }

    struct ReleaseOnDrop(Arc<(Mutex<bool>, Condvar)>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }

    #[tokio::test]
    async fn cancelled_waiter_keeps_one_shared_removal_job_and_eventual_receipt() {
        let parent = tempfile::tempdir().unwrap();
        let launch = launch(parent.path());
        let path = launch.state_directory().to_owned();
        let cleanup = launch.cleanup_observer();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_remove = calls.clone();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let release_for_remove = release.clone();
        // Release even if a preceding assertion panics, so runtime teardown does
        // not wait forever on this deliberately blocked blocking worker.
        let release_guard = ReleaseOnDrop(release);
        let (started, mut started_rx) = mpsc::unbounded_channel();
        *cleanup
            .authority
            .as_ref()
            .unwrap()
            .before_remove
            .lock()
            .unwrap() = Some(Arc::new(move || {
            if calls_for_remove.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(std::io::Error::other("controlled first removal failure"));
            }
            let _ = started.send(());
            let (ready, changed) = &*release_for_remove;
            let mut ready = ready.lock().unwrap();
            while !*ready {
                ready = changed.wait(ready).unwrap();
            }
            Ok(())
        }));
        drop(launch);
        let first_observer = cleanup.clone();
        let first =
            tokio::spawn(async move { first_observer.settle(Duration::from_secs(60)).await });
        timeout(LIMIT, started_rx.recv()).await.unwrap().unwrap();
        let mut second = Box::pin(cleanup.settle(LIMIT));
        assert!(futures_util::poll!(second.as_mut()).is_pending());
        first.abort();
        assert!(matches!(first.await, Err(error) if error.is_cancelled()));
        assert!(path.is_dir());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "concurrent callers must join the same retry"
        );
        drop(release_guard);
        assert_eq!(
            second.await,
            Some(CleanupReport {
                child_started: false,
                reaped: false,
                temporary_root_removed: true,
            })
        );
        assert!(!path.exists());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn replacement_directory_and_symlink_are_never_removed_or_adopted() {
        for symlink in [false, true] {
            let parent = tempfile::tempdir().unwrap();
            let launch = launch(parent.path());
            let path = launch.state_directory().to_owned();
            let cleanup = launch.cleanup_observer();
            let retained = parent.path().join("retained-original");
            let foreign = parent.path().join("foreign");
            std::fs::create_dir(&foreign).unwrap();
            std::fs::write(foreign.join("sentinel"), b"foreign").unwrap();
            std::fs::rename(&path, &retained).unwrap();
            if symlink {
                std::os::unix::fs::symlink(&foreign, &path).unwrap();
            } else {
                std::fs::create_dir(&path).unwrap();
                std::fs::write(path.join("sentinel"), b"replacement").unwrap();
            }
            drop(launch);
            assert!(!cleanup.settle(LIMIT).await.unwrap().temporary_root_removed);
            assert!(retained.is_dir());
            assert_eq!(std::fs::read(foreign.join("sentinel")).unwrap(), b"foreign");
            if symlink {
                std::fs::remove_file(&path).unwrap();
            } else {
                assert_eq!(
                    std::fs::read(path.join("sentinel")).unwrap(),
                    b"replacement"
                );
                std::fs::remove_dir_all(&path).unwrap();
            }
            std::fs::rename(&retained, &path).unwrap();
            assert!(cleanup.settle(LIMIT).await.unwrap().temporary_root_removed);
            assert!(!path.exists());
            assert!(foreign.join("sentinel").is_file());
        }
    }

    #[tokio::test]
    async fn missing_path_is_not_completion_when_original_directory_was_renamed() {
        let parent = tempfile::tempdir().unwrap();
        let launch = launch(parent.path());
        let path = launch.state_directory().to_owned();
        let cleanup = launch.cleanup_observer();
        let retained = parent.path().join("retained-original");
        std::fs::rename(&path, &retained).unwrap();
        drop(launch);
        assert!(!cleanup.settle(LIMIT).await.unwrap().temporary_root_removed);
        assert!(retained.is_dir());
        std::fs::rename(&retained, &path).unwrap();
        assert!(cleanup.settle(LIMIT).await.unwrap().temporary_root_removed);
    }

    #[tokio::test]
    async fn genuinely_removed_original_does_not_authorize_removing_reused_path() {
        let parent = tempfile::tempdir().unwrap();
        let launch = launch(parent.path());
        let path = launch.state_directory().to_owned();
        let cleanup = launch.cleanup_observer();
        std::fs::remove_dir_all(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("sentinel"), b"replacement").unwrap();
        drop(launch);
        // Some filesystems prove the original handle was unlinked; APFS can
        // retain its link count and must conservatively retain failed evidence.
        // Neither outcome authorizes deleting the replacement at this path.
        let report = cleanup.settle(LIMIT).await.unwrap();
        assert!(!report.child_started);
        assert!(!report.reaped);
        assert_eq!(
            std::fs::read(path.join("sentinel")).unwrap(),
            b"replacement"
        );
    }

    #[tokio::test]
    async fn observed_constructor_error_retains_no_child_removal_authority() {
        let parent = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir_in(parent.path()).unwrap();
        let path = state.path().to_owned();
        let retained = parent.path().join("retained-original");
        let mut cleanup = None;
        let result = Launch::new_observed(
            PathBuf::from("/missing-snowflake-node"),
            None,
            parent.path().to_owned(),
            state,
            |observer| {
                cleanup = Some(observer);
                std::fs::rename(&path, &retained).unwrap();
            },
        );
        assert!(
            result.is_err(),
            "validation must occur after observer attachment"
        );
        let cleanup = cleanup.unwrap();
        assert_eq!(
            cleanup.report(),
            Some(CleanupReport {
                child_started: false,
                reaped: false,
                temporary_root_removed: false,
            })
        );
        std::fs::rename(&retained, &path).unwrap();
        assert!(cleanup.settle(LIMIT).await.unwrap().temporary_root_removed);
        assert!(!path.exists());
    }
}

#[cfg(all(test, unix, feature = "test-harness"))]
#[path = "snowflake_qualification_transport_tests.rs"]
mod qualification_transport_tests;
