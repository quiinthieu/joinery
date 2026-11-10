//! Bounded Snowflake helper process transport. No remote capability is qualified here.
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

pub struct Launch {
    /// Absolute, explicitly selected executable; never resolved using PATH.
    pub executable: PathBuf,
    /// Only an absolute packaged helper script, never credentials or SQL.
    pub script: Option<PathBuf>,
    pub working_directory: PathBuf,
    state_directory: Option<tempfile::TempDir>,
    cleanup: CleanupObserver,
    cleanup_tx: watch::Sender<Option<CleanupReport>>,
}
impl Launch {
    pub fn new(
        executable: PathBuf,
        script: Option<PathBuf>,
        working_directory: PathBuf,
        state_directory: tempfile::TempDir,
    ) -> std::io::Result<Self> {
        let metadata = std::fs::symlink_metadata(state_directory.path())?;
        if !state_directory.path().is_absolute()
            || !metadata.is_dir()
            || metadata.file_type().is_symlink()
        {
            return Err(std::io::Error::other("invalid owned state directory"));
        }
        secure_state_directory(state_directory.path())?;
        let (cleanup_tx, receiver) = watch::channel(None);
        Ok(Self {
            executable,
            script,
            working_directory,
            state_directory: Some(state_directory),
            cleanup: CleanupObserver { receiver },
            cleanup_tx,
        })
    }
    pub fn state_directory(&self) -> &std::path::Path {
        self.state_directory
            .as_ref()
            .expect("owned launch root")
            .path()
    }
    /// Capture before spawn to observe cleanup even if the spawn future is cancelled.
    pub fn cleanup_observer(&self) -> CleanupObserver {
        self.cleanup.clone()
    }
}
impl Drop for Launch {
    fn drop(&mut self) {
        if let Some(root) = self.state_directory.take() {
            let temporary_root_removed = root.close().is_ok();
            self.cleanup_tx.send_replace(Some(CleanupReport {
                child_started: false,
                reaped: false,
                temporary_root_removed,
            }));
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
    Mutating,
    Commit,
    Rollback,
    Control,
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
    Committed,
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
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatementIdentity {
    pub sdk_request_id: String,
    pub query_id: Option<String>,
}
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
            Self::Inactive => "stale or inactive operation token",
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
    statement_events: u8,
    wire_id: Option<String>,
    terminal: watch::Sender<Option<Report>>,
    columns: watch::Sender<Option<Vec<Column>>>,
    rows: mpsc::Sender<RowBatch>,
    width: Option<usize>,
    credits: u32,
    discard: bool,
}
#[derive(Default)]
struct State {
    retired: Option<Retirement>,
    wire_ids: HashMap<String, String>,
    next_wire_id: u64,
    pending: HashMap<String, Pending>,
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
        if let Some(report) = self.report() {
            return report.phase;
        }
        self.owner
            .state
            .lock()
            .unwrap()
            .pending
            .get(&self.token.id)
            .filter(|p| p.ticket == self.token.ticket)
            .map(|p| p.phase)
            .unwrap_or(Phase::Queued)
    }
    pub fn report(&self) -> Option<Report> {
        self.terminal.borrow().clone()
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
                };
            }
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
        if matches!(self.op.as_str(), "connect" | "context" | "close") {
            self.owner.retire(Retirement::Dropped);
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
        let mut child = command
            .current_dir(&launch.working_directory)
            .env_clear()
            .env("JOINERY_SNOWFLAKE_STATE_DIR", launch.state_directory())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "helper could not start")?;
        let pid = child.id();
        // Disable automatic deletion before transferring ownership to an async task:
        // task/runtime cancellation must leave a remediable root, not delete it
        // before actual reaping has been confirmed.
        let state_directory = launch
            .state_directory
            .take()
            .expect("owned launch root")
            .keep();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
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
        });
        let owner = inner.clone();
        // Child and private root transfer before the first await in spawn. The reaper
        // survives cancelled handshakes and bounded caller timeouts.
        let cleanup_tx = launch.cleanup_tx.clone();
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
            let temporary_root_removed = if reaped {
                std::fs::remove_dir_all(&state_directory).is_ok()
            } else {
                false
            };
            cleanup_tx.send_replace(Some(CleanupReport {
                child_started: true,
                reaped,
                temporary_root_removed,
            }));
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
        let hello = timeout(bounds.handshake, read_frame(&mut stdout, bounds.frame)).await;
        if !matches!(hello, Ok(Ok(Some(ref value))) if valid_hello(value)) {
            inner.retire(Retirement::Protocol);
            inner.exited.wait(bounds.exit).await;
            return Err("helper handshake failed");
        }
        tokio::spawn(write_loop(
            inner.clone(),
            stdin,
            normal_rx,
            control_rx,
            bounds.write,
        ));
        let owner = inner.clone();
        let frame_bound = bounds.frame;
        tokio::spawn(async move {
            let mut stopping = owner.stop.subscribe();
            loop {
                if *stopping.borrow() {
                    return;
                }
                let frame = tokio::select! { _ = stopping.changed() => return,
                frame = read_frame(&mut stdout, frame_bound) => frame };
                match frame {
                    Ok(Some(event)) => {
                        if !owner.event(event) {
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
    /// Low-level trusted-driver API. The official helper STILL rejects mutations.
    /// Impact is lifecycle evidence for future integration, never a capability grant.
    pub fn request(
        &self,
        id: &str,
        op: &str,
        payload: Value,
        impact: Impact,
    ) -> Result<Operation, &'static str> {
        if (op == "context" && impact != Impact::Read)
            || (op == "connect" && impact != Impact::Control)
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
                .ok_or("stale or inactive operation token")?;
            if pending.phase == Phase::Queued {
                return Err("query has not been transmitted");
            }
            if pending.op != "query" || pending.discard || pending.credits + batches > MAX_CREDITS {
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
                if !never_sent {
                    let queued: Vec<_> = state
                        .pending
                        .iter()
                        .filter(|(_, p)| {
                            p.phase == Phase::Queued && matches!(p.op.as_str(), "query" | "context")
                        })
                        .map(|(id, _)| id.clone())
                        .collect();
                    for id in queued {
                        let pending = state.pending.remove(&id).unwrap();
                        pending.terminal.send_replace(Some(Report {
                            ok: false,
                            certainty: Certainty::NotSubmitted,
                            phase: Phase::Queued,
                            result: None,
                            error_code: Some("connectionClosed".into()),
                            retirement: None,
                            statements: pending.statements,
                        }));
                    }
                }
                Some(never_sent)
            }
        };
        let Some(never_sent) = decision else {
            self.wait_exited().await;
            return None;
        };
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
        if pending.op != "query" && !(allow_connect && is_connect) {
            return Err(TargetError::Control("invalid query target"));
        }
        if pending.phase != Phase::Queued {
            pending.discard |= discard;
            return Ok((Some(pending.ticket), is_connect));
        }
        let pending = state.pending.remove(&target.id).unwrap();
        pending.terminal.send_replace(Some(Report {
            ok: false,
            certainty: Certainty::NotSubmitted,
            phase: Phase::Queued,
            result: None,
            error_code: Some("cancelled".into()),
            retirement: None,
            statements: pending.statements,
        }));
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
                "connect" | "query" | "context" | "credit" | "cancel" | "close"
            )
            || !payload.is_object()
        {
            return Err("invalid request");
        }
        let priority = matches!(op, "credit" | "cancel" | "close");
        if priority && impact != Impact::Control {
            return Err("invalid control impact");
        }
        let credits = if op == "query" {
            payload.get("credits").and_then(Value::as_u64).unwrap_or(0)
        } else {
            0
        };
        if credits > u64::from(MAX_CREDITS) {
            return Err("invalid credit bound");
        }
        let target = payload
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_owned);
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
                statement_events: 0,
                wire_id: None,
                terminal,
                columns,
                rows,
                width: None,
                credits: credits as u32,
                discard: false,
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
        let mut state = self.state.lock().unwrap();
        if state.retired.is_some() {
            return;
        }
        state.retired = Some(reason);
        for (_, pending) in state.pending.drain() {
            let certainty = interrupted_certainty(pending.impact, pending.phase);
            pending.terminal.send_replace(Some(Report {
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
                statements: pending.statements,
            }));
        }
        state.wire_ids.clear();
        self.stop.send_replace(true);
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
            Some("phase") => match event.get("phase").and_then(Value::as_str) {
                Some("dispatching")
                    if pending.accepted
                        && matches!(pending.op.as_str(), "query" | "context")
                        && pending.phase < Phase::Dispatching =>
                {
                    pending.phase = Phase::Dispatching;
                    true
                }
                Some("acknowledged") if pending.phase == Phase::Dispatching => {
                    if !validate_latest_statement(pending, &event) {
                        return false;
                    }
                    pending.phase = Phase::Acknowledged;
                    true
                }
                _ => false,
            },
            Some("statement") => {
                if !pending.accepted
                    || pending.phase < Phase::Dispatching
                    || !matches!(pending.op.as_str(), "query" | "context")
                {
                    return false;
                }
                pending.statement_events += 1;
                let limit = if pending.op == "context" { 4 } else { 2 };
                pending.statement_events <= limit && update_statement(pending, &event)
            }
            Some("columns") => {
                if pending.op != "query"
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
                if pending.op != "query"
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
                    && matches!(pending.op.as_str(), "query" | "context")
                    && pending.phase != Phase::Acknowledged
                {
                    return false;
                }
                let certainty = if pending.phase == Phase::Acknowledged {
                    acknowledged_certainty(pending.impact)
                } else if code
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
                    .is_some_and(|c| matches!(c, "connectionClosed" | "outcomeUnknown"));
                pending.terminal.send_replace(Some(Report {
                    ok,
                    certainty,
                    phase: pending.phase,
                    result,
                    error_code: code,
                    retirement: None,
                    statements: pending.statements,
                }));
                let closing = state.pending.values().any(|p| p.op == "close");
                drop(state);
                if terminal_connection_loss && !closing {
                    self.retire(Retirement::Requested);
                }
                true
            }
            _ => false,
        }
    }
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
    } else {
        if pending.statements.len() >= if pending.op == "context" { 2 } else { 1 } {
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
    }
}

fn acknowledged_certainty(impact: Impact) -> Certainty {
    match impact {
        Impact::Commit => Certainty::Committed,
        Impact::Rollback => Certainty::RolledBack,
        _ => Certainty::StatementSucceeded,
    }
}
fn interrupted_certainty(impact: Impact, phase: Phase) -> Certainty {
    if phase == Phase::Queued {
        return Certainty::NotSubmitted;
    }
    if phase == Phase::Acknowledged {
        return acknowledged_certainty(impact);
    }
    match impact {
        Impact::Mutating | Impact::Commit | Impact::Rollback => Certainty::OutcomeUnknown,
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
    loop {
        if *stopping.borrow() {
            return;
        }
        let outbound = tokio::select! { biased; _ = stopping.changed() => return,
        message = control.recv() => message, message = normal.recv() => message };
        let Some(outbound) = outbound else {
            return;
        };
        let bytes = {
            let mut state = inner.state.lock().unwrap();
            if state.retired.is_some() {
                return;
            }
            if !state
                .pending
                .get(&outbound.id)
                .is_some_and(|p| p.ticket == outbound.ticket)
            {
                continue;
            }
            let Some(next) = state.next_wire_id.checked_add(1) else {
                drop(state);
                inner.retire(Retirement::Protocol);
                return;
            };
            state.next_wire_id = next;
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
                    // A terminal may beat its queued cancellation/credit; acknowledge locally.
                    if let Some(pending) = state.pending.remove(&outbound.id) {
                        pending
                            .terminal
                            .send_replace(Some(local_report(json!({"inactive":true}))));
                    }
                    continue;
                };
                value["payload"]["target"] = Value::String(target_id);
            }
            state.wire_ids.insert(wire_id.clone(), outbound.id.clone());
            let pending = state.pending.get_mut(&outbound.id).unwrap();
            pending.wire_id = Some(wire_id);
            pending.phase = Phase::Sending;
            serde_json::to_vec(&value).unwrap()
        };
        let result = tokio::select! { _ = stopping.changed() => return,
        result = timeout(limit, write_bytes(&mut stdin, &bytes)) => result };
        if !matches!(result, Ok(Ok(()))) {
            inner.retire(Retirement::Write);
            return;
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
    fn isolated() -> (
        Arc<Inner>,
        mpsc::Receiver<Outbound>,
        mpsc::Receiver<Outbound>,
    ) {
        let (normal, normal_rx) = mpsc::channel(8);
        let (control, control_rx) = mpsc::channel(16);
        let (stop, _) = watch::channel(false);
        let (_, receiver) = watch::channel(None);
        let exited = CleanupObserver { receiver };
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
        });
        (inner, normal_rx, control_rx)
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
