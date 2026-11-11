//! Per-attempt physical transport plumbing, never an execution approval or SQL ledger.
use super::snowflake_process::{Impact, MAX_FRAME};
use crate::domain::{AppError, AppResult};
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::watch,
    time::timeout,
};

pub(crate) struct PhysicalRequest<'a> {
    pub generation: &'a str,
    pub operation_id: &'a str,
    pub ticket: u64,
    pub operation: &'a str,
    pub impact: Impact,
    pub target: Option<(&'a str, Option<u64>)>,
    /// Exact final envelope, including the resolved wire ID/target.
    pub payload: &'a Value,
    /// Exact serialized envelope, excluding the four-byte length prefix.
    pub wire_payload: &'a [u8],
}
/// A validated helper terminal, after removal from the process pending map and
/// before publication to its caller. Local cancellation/retirement reports do
/// not pass this seam. The original report is always published unchanged.
pub(crate) struct PhysicalTerminal<'a> {
    pub generation: &'a str,
    pub operation_id: &'a str,
    pub ticket: u64,
    pub wire_id: &'a str,
    pub operation: &'a str,
    pub impact: Impact,
    pub report: &'a super::snowflake_process::Report,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalTerminalOrigin {
    QueuedCancellation,
    QueuedClose,
    InactiveControl { admitted: bool },
    Retirement,
}
/// An original pending-operation report produced locally. This excludes the
/// separate synthetic cancel/discard acknowledgement and aggregate close/fallback
/// reports, which have no pending operation or physical admission to complete.
pub(crate) struct LocalTerminal<'a> {
    pub generation: &'a str,
    pub operation_id: &'a str,
    pub ticket: u64,
    /// Actual process wire assignment, never an admission candidate substituted for it.
    pub wire_id: Option<&'a str>,
    /// Candidate passed to admission; presence alone proves neither fsync nor delivery.
    pub candidate_wire_id: Option<&'a str>,
    pub operation: &'a str,
    pub impact: Impact,
    pub target: Option<(&'a str, Option<u64>)>,
    /// Original unresolved operation payload, including the logical target.
    pub payload: &'a Value,
    pub origin: LocalTerminalOrigin,
    pub report: &'a super::snowflake_process::Report,
}
pub(crate) trait TransportPolicy: Send + Sync {
    /// Synchronous, no IO or adapter/Studio/registry reentry. Publication may
    /// call this while holding connection registry and session-map locks.
    fn check(&self) -> AppResult<()> {
        Ok(())
    }
    /// Latch-only notification for cross-domain revocation. Never perform IO here.
    fn failed(&self, _error: AppError) {}
    /// Synchronous in-memory observation, outside the process state mutex. No
    /// I/O or reentry. Rejection revokes subsequent admission without replacing
    /// the original terminal or its independently retained mutation ACK.
    fn terminal(&self, _terminal: PhysicalTerminal<'_>) -> AppResult<()> {
        Ok(())
    }
    /// Same in-memory/no-reentry contract as terminal. Observe even after failure;
    /// denial cannot replace the locally produced original or its retained ACK.
    fn local_terminal(&self, _terminal: LocalTerminal<'_>) -> AppResult<()> {
        Ok(())
    }
    /// May sync a private intent. No process-state mutex is held across this await.
    /// An admitted intent is consumed even if later revalidation prevents writing.
    fn admit<'a>(
        &'a self,
        request: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>>;
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct TransportLimits {
    pub stdout_bytes: u64,
    /// Aggregate of every inbound frame body, excluding four-byte prefixes.
    pub payload_bytes: u64,
    pub frames: u64,
    pub frame_bytes: usize,
    pub stderr_bytes: u64,
}
impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            stdout_bytes: 64 * 1024 * 1024,
            payload_bytes: 32 * 1024 * 1024,
            frames: 32768,
            frame_bytes: MAX_FRAME,
            stderr_bytes: 64 * 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disposition {
    NotStarted,
    Running,
    Eof,
    Interrupted,
    Malformed,
    Truncated,
    Limit,
    Io,
    Deadline,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct TransportReport {
    pub stdout_bytes: u64,
    /// Successfully read body bytes, including partial and malformed frames.
    /// Headers and declared-but-unread body bytes are excluded.
    pub payload_bytes: u64,
    pub complete_frames: u64,
    pub stderr_bytes: u64,
    pub admitted_writes: u64,
    pub started_writes: u64,
    pub complete_writes: u64,
    pub outbound_bytes: u64,
    pub stdout: Disposition,
    pub stderr: Disposition,
    pub writer: Disposition,
    pub failed: bool,
    pub settled: bool,
}
struct State {
    report: TransportReport,
    claimed: bool,
    sealed: bool,
    active_tasks: usize,
    active_publications: usize,
    first_error: Option<AppError>,
    // Declared body lengths reserve budget before allocation and are never refunded.
    reserved_payload_bytes: u64,
    // Errors or cancellation leave this set: a partial frame cannot be replayed.
    frame_open: bool,
}
struct Attached {
    policy: Arc<dyn TransportPolicy>,
    limits: TransportLimits,
    state: Mutex<State>,
    updates: watch::Sender<u64>,
}
#[derive(Clone)]
pub(crate) struct TransportAttachment {
    inner: Arc<Attached>,
}
#[derive(Clone, Copy)]
pub(super) enum Stream {
    Stdout,
    Stderr,
    Writer,
}
pub(super) struct TaskGuard {
    attachment: TransportAttachment,
    stream: Stream,
}
pub(super) struct PublicationGuard {
    attachment: TransportAttachment,
}
impl Drop for PublicationGuard {
    fn drop(&mut self) {
        self.attachment
            .update(|state| state.active_publications -= 1);
    }
}
impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.attachment.finish(self.stream);
    }
}
impl TransportAttachment {
    pub(crate) fn new(
        policy: Arc<dyn TransportPolicy>,
        limits: TransportLimits,
    ) -> AppResult<Self> {
        let max = TransportLimits::default();
        if limits.stdout_bytes == 0
            || limits.stdout_bytes > max.stdout_bytes
            || limits.payload_bytes == 0
            || limits.payload_bytes > max.payload_bytes
            || limits.frames == 0
            || limits.frames > max.frames
            || limits.frame_bytes == 0
            || limits.frame_bytes > max.frame_bytes
            || limits.stderr_bytes == 0
            || limits.stderr_bytes > max.stderr_bytes
        {
            return Err(resource_error());
        }
        let (updates, _) = watch::channel(0);
        Ok(Self {
            inner: Arc::new(Attached {
                policy,
                limits,
                updates,
                state: Mutex::new(State {
                    claimed: false,
                    sealed: false,
                    active_tasks: 0,
                    active_publications: 0,
                    first_error: None,
                    reserved_payload_bytes: 0,
                    frame_open: false,
                    report: TransportReport {
                        stdout_bytes: 0,
                        payload_bytes: 0,
                        complete_frames: 0,
                        stderr_bytes: 0,
                        admitted_writes: 0,
                        started_writes: 0,
                        complete_writes: 0,
                        outbound_bytes: 0,
                        stdout: Disposition::NotStarted,
                        stderr: Disposition::NotStarted,
                        writer: Disposition::NotStarted,
                        failed: false,
                        settled: false,
                    },
                }),
            }),
        })
    }
    fn update(&self, f: impl FnOnce(&mut State)) {
        let mut state = self.inner.state.lock().unwrap();
        f(&mut state);
        state.report.failed = state.first_error.is_some();
        state.report.settled =
            state.sealed && state.active_tasks == 0 && state.active_publications == 0;
        self.inner
            .updates
            .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
    }
    pub(super) fn claim(&self) -> AppResult<()> {
        let mut state = self.inner.state.lock().unwrap();
        if state.claimed {
            return Err(AppError::invalid(
                "Qualification transport attachment was already claimed",
            ));
        }
        state.claimed = true;
        Ok(())
    }
    /// Dispose an attempt rejected before it could be attached to a Launch.
    /// A retained report clone cannot otherwise infer that no launch will occur.
    pub(crate) fn settle_unstarted(&self) -> AppResult<()> {
        self.claim()?;
        self.seal();
        Ok(())
    }
    pub(super) fn seal(&self) {
        self.update(|state| state.sealed = true);
    }
    pub(super) fn start(&self, stream: Stream) -> TaskGuard {
        self.update(|state| {
            *disposition(&mut state.report, stream) = Disposition::Running;
            state.active_tasks += 1;
        });
        TaskGuard {
            attachment: self.clone(),
            stream,
        }
    }
    fn finish(&self, stream: Stream) {
        self.update(|state| {
            let current = disposition(&mut state.report, stream);
            if *current == Disposition::Running {
                *current = Disposition::Interrupted;
            }
            state.active_tasks -= 1;
        });
    }
    pub(super) fn end(&self, stream: Stream, result: Disposition) {
        self.update(|state| {
            let current = disposition(&mut state.report, stream);
            if *current == Disposition::Running {
                *current = result;
            }
        });
    }
    pub(crate) fn report(&self) -> TransportReport {
        self.inner.state.lock().unwrap().report
    }
    pub(crate) fn first_error(&self) -> Option<AppError> {
        self.inner.state.lock().unwrap().first_error.clone()
    }
    pub(crate) async fn wait(&self, bound: Duration) -> Option<TransportReport> {
        let mut receiver = self.inner.updates.subscribe();
        timeout(bound, async {
            loop {
                let report = self.report();
                if report.settled {
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
    pub(super) fn fail(&self, error: AppError) {
        let mut first = false;
        self.update(|state| {
            if state.first_error.is_none() {
                state.first_error = Some(error.clone());
                first = true;
            }
        });
        // Never call policy code under the counter mutex.
        if first {
            self.inner.policy.failed(error);
        }
    }
    pub(crate) fn check(&self) -> AppResult<()> {
        if let Some(error) = self.first_error() {
            return Err(error);
        }
        if let Err(error) = self.inner.policy.check() {
            self.fail(error.clone());
            return Err(error);
        }
        Ok(())
    }
    pub(super) async fn admit(&self, request: PhysicalRequest<'_>) -> AppResult<()> {
        self.check()?;
        if let Err(error) = self.inner.policy.admit(request).await {
            self.fail(error.clone());
            return Err(error);
        }
        self.update(|state| state.report.admitted_writes += 1);
        self.check()
    }
    pub(super) fn terminal(&self, terminal: PhysicalTerminal<'_>) {
        // Do not precheck: a known original outcome must still be observed after
        // another failure has revoked admission.
        if let Err(error) = self.inner.policy.terminal(terminal) {
            self.fail(error);
        }
    }
    pub(super) fn local_terminal(&self, terminal: LocalTerminal<'_>) {
        if let Err(error) = self.inner.policy.local_terminal(terminal) {
            self.fail(error);
        }
    }
    /// Register under process State before publishing outside that mutex, so a
    /// concurrent owner cannot mistake stream settlement for completed capture.
    pub(super) fn publication(&self) -> PublicationGuard {
        self.update(|state| state.active_publications += 1);
        PublicationGuard {
            attachment: self.clone(),
        }
    }
    pub(super) fn write_started(&self) {
        self.update(|state| state.report.started_writes += 1);
    }
    pub(super) fn wrote(&self, count: usize) {
        self.update(|state| state.report.outbound_bytes += count as u64);
    }
    pub(super) fn write_complete(&self) {
        self.update(|state| state.report.complete_writes += 1);
    }
    pub(super) fn stream_failure(&self, stream: Stream, result: Disposition) {
        self.fail(resource_error());
        self.end(stream, result);
    }
    async fn read(
        &self,
        reader: &mut (impl AsyncRead + Unpin),
        buffer: &mut [u8],
        stream: Stream,
        payload: bool,
    ) -> Result<usize, &'static str> {
        let used = {
            let report = self.report();
            match stream {
                Stream::Stdout => report.stdout_bytes,
                Stream::Stderr => report.stderr_bytes,
                Stream::Writer => unreachable!(),
            }
        };
        let cap = match stream {
            Stream::Stdout => self.inner.limits.stdout_bytes,
            Stream::Stderr => self.inner.limits.stderr_bytes,
            Stream::Writer => unreachable!(),
        };
        // One sentinel byte detects overflow without an unbounded read/allocation.
        let bound = buffer.len().min((cap.saturating_sub(used) + 1) as usize);
        let count = reader.read(&mut buffer[..bound]).await.map_err(|_| {
            self.stream_failure(stream, Disposition::Io);
            "qualification pipe read failed"
        })?;
        self.update(|state| {
            match stream {
                Stream::Stdout => state.report.stdout_bytes += count as u64,
                Stream::Stderr => state.report.stderr_bytes += count as u64,
                Stream::Writer => unreachable!(),
            }
            if payload {
                state.report.payload_bytes += count as u64;
            }
        });
        if used + count as u64 > cap {
            self.stream_failure(stream, Disposition::Limit);
            return Err("qualification inbound limit");
        }
        Ok(count)
    }
    async fn exact(
        &self,
        reader: &mut (impl AsyncRead + Unpin),
        buffer: &mut [u8],
        payload: bool,
    ) -> Result<(), &'static str> {
        let mut offset = 0;
        while offset < buffer.len() {
            let count = self
                .read(reader, &mut buffer[offset..], Stream::Stdout, payload)
                .await?;
            if count == 0 {
                self.stream_failure(Stream::Stdout, Disposition::Truncated);
                return Err("incomplete frame");
            }
            offset += count;
        }
        Ok(())
    }
    pub(super) async fn read_frame(
        &self,
        reader: &mut (impl AsyncRead + Unpin),
        frame_limit: Duration,
    ) -> Result<Option<Value>, &'static str> {
        {
            let mut state = self.inner.state.lock().unwrap();
            if state.frame_open {
                return Err("qualification frame reader cannot resume");
            }
            state.frame_open = true;
        }
        let mut prefix = [0; 4];
        if self
            .read(reader, &mut prefix[..1], Stream::Stdout, false)
            .await?
            == 0
        {
            self.end(Stream::Stdout, Disposition::Eof);
            self.update(|state| state.frame_open = false);
            return Ok(None);
        }
        let result = timeout(frame_limit, async {
            self.exact(reader, &mut prefix[1..], false).await?;
            let length = u32::from_be_bytes(prefix) as usize;
            let admitted = {
                let mut state = self.inner.state.lock().unwrap();
                if length == 0
                    || length > self.inner.limits.frame_bytes
                    || state.report.complete_frames >= self.inner.limits.frames
                    || length as u64
                        > self
                            .inner
                            .limits
                            .stdout_bytes
                            .saturating_sub(state.report.stdout_bytes)
                    || length as u64
                        > self
                            .inner
                            .limits
                            .payload_bytes
                            .saturating_sub(state.reserved_payload_bytes)
                {
                    false
                } else {
                    state.reserved_payload_bytes += length as u64;
                    true
                }
            };
            if !admitted {
                self.stream_failure(Stream::Stdout, Disposition::Limit);
                return Err("qualification frame limit");
            }
            let mut bytes = vec![0; length];
            self.exact(reader, &mut bytes, true).await?;
            self.update(|state| state.report.complete_frames += 1);
            serde_json::from_slice(&bytes).map(Some).map_err(|_| {
                self.stream_failure(Stream::Stdout, Disposition::Malformed);
                "invalid JSON frame"
            })
        })
        .await;
        match result {
            Ok(result) => {
                if result.is_ok() {
                    self.update(|state| state.frame_open = false);
                }
                result
            }
            Err(_) => {
                self.stream_failure(Stream::Stdout, Disposition::Deadline);
                Err("incomplete frame deadline")
            }
        }
    }
    pub(super) async fn drain_stderr(
        &self,
        reader: &mut (impl AsyncRead + Unpin),
    ) -> Result<(), &'static str> {
        let mut buffer = [0; 8192];
        loop {
            if self
                .read(reader, &mut buffer, Stream::Stderr, false)
                .await?
                == 0
            {
                self.end(Stream::Stderr, Disposition::Eof);
                return Ok(());
            }
        }
    }
}
fn disposition(report: &mut TransportReport, stream: Stream) -> &mut Disposition {
    match stream {
        Stream::Stdout => &mut report.stdout,
        Stream::Stderr => &mut report.stderr,
        Stream::Writer => &mut report.writer,
    }
}
fn resource_error() -> AppError {
    AppError::invalid("Qualification transport admission or resource limit failed")
}
