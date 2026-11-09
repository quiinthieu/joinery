//! Retained metadata-lock leases. Names describe a lease; only the original
//! still-live connection makes it an incarnation identity.
use super::*;
use serde::{Serialize, Serializer};
use std::{
    collections::HashMap,
    fmt,
    sync::{OnceLock, Weak},
};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GuardIdentity {
    pub generation: Uuid,
    pub session_id: String,
    pub table: TableRef,
}

/// Internal qualification policy, never accepted from an IPC request.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct GuardPolicy {
    pub lifetime: Duration,
    pub acquisition_timeout: Duration,
    pub verification_timeout: Duration,
    pub close_timeout: Duration,
    pub writer_lock_timeout: Duration,
    pub max_guards: usize,
}
impl Default for GuardPolicy {
    fn default() -> Self {
        Self {
            lifetime: Duration::from_secs(60),
            acquisition_timeout: Duration::from_secs(10),
            verification_timeout: Duration::from_secs(3),
            close_timeout: Duration::from_secs(2),
            writer_lock_timeout: Duration::from_secs(5),
            max_guards: 32,
        }
    }
}
impl GuardPolicy {
    fn validate(&self) -> AppResult<()> {
        if self.lifetime.is_zero()
            || self.acquisition_timeout.is_zero()
            || self.verification_timeout.is_zero()
            || self.close_timeout.is_zero()
            || self.writer_lock_timeout.is_zero()
            || self.max_guards == 0
            || self.max_guards > 128
            || self.writer_lock_timeout > Duration::from_secs(60)
        {
            return Err(AppError::invalid("Invalid retained target guard policy"));
        }
        Ok(())
    }
}

struct Control {
    revoked: CancellationToken,
    finished: CancellationToken,
    deadline: tokio::time::Instant,
}
impl Control {
    fn live(&self) -> bool {
        !self.revoked.is_cancelled() && tokio::time::Instant::now() < self.deadline
    }
}
struct GuardOwner {
    identity: GuardIdentity,
    control: Arc<Control>,
    schema: OnceLock<TableSchema>,
    requests: mpsc::Sender<Verify>,
}
impl Drop for GuardOwner {
    fn drop(&mut self) {
        self.control.revoked.cancel();
    }
}

#[derive(Clone)]
pub struct MysqlGuardHandle(Arc<GuardOwner>);
impl MysqlGuardHandle {
    pub fn identity(&self) -> &GuardIdentity {
        &self.0.identity
    }
    pub(crate) fn is_live(&self) -> bool {
        self.0.control.live()
    }
    pub(super) fn schema(&self) -> Option<&TableSchema> {
        self.0.schema.get()
    }

    /// Called only after this writer owns its own MDL. Dropping this future
    /// revokes the lease: no abandoned PING response can satisfy another request.
    pub(super) async fn verify(
        &self,
        session_id: &str,
        table: &TableRef,
        cancelled: &CancellationToken,
        closed: &CancellationToken,
    ) -> AppResult<()> {
        if cancelled.is_cancelled() {
            return Err(cancelled_error());
        }
        if closed.is_cancelled() {
            return Err(super::closed());
        }
        if self.identity().session_id != session_id
            || &self.identity().table != table
            || !self.is_live()
        {
            return Err(expired());
        }
        let mut abandon = RevokeOnDrop(Some(self.0.control.clone()));
        let (reply, response) = oneshot::channel();
        let request = Verify {
            reply,
            cancelled: cancelled.clone(),
            closed: closed.clone(),
        };
        let result = tokio::select! {
            biased;
            _ = cancelled.cancelled() => Err(cancelled_error()),
            _ = closed.cancelled() => Err(super::closed()),
            _ = self.0.control.revoked.cancelled() => Err(expired()),
            result = async {
                self.0.requests.send(request).await.map_err(|_| expired())?;
                response.await.map_err(|_| expired())?
            } => result,
        };
        if result.is_ok() {
            abandon.0.take();
        }
        result
    }
}
impl PartialEq for MysqlGuardHandle {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}
impl Eq for MysqlGuardHandle {}
impl fmt::Debug for MysqlGuardHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.identity().fmt(f)
    }
}
impl Serialize for MysqlGuardHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.identity().serialize(serializer)
    }
}
struct RevokeOnDrop(Option<Arc<Control>>);
impl Drop for RevokeOnDrop {
    fn drop(&mut self) {
        if let Some(control) = &self.0 {
            control.revoked.cancel();
        }
    }
}
struct Verify {
    reply: oneshot::Sender<AppResult<()>>,
    cancelled: CancellationToken,
    closed: CancellationToken,
}
struct Entry {
    owner: Weak<GuardOwner>,
    control: Arc<Control>,
}
pub(super) struct GuardRegistry {
    session_id: String,
    policy: GuardPolicy,
    entries: parking_lot::Mutex<HashMap<Uuid, Entry>>,
    acquisition: Mutex<()>,
    closed: CancellationToken,
}
impl GuardRegistry {
    pub(super) fn new(
        session_id: &str,
        policy: GuardPolicy,
        closed: CancellationToken,
    ) -> AppResult<Self> {
        policy.validate()?;
        Ok(Self {
            session_id: session_id.into(),
            policy,
            entries: Default::default(),
            acquisition: Mutex::new(()),
            closed,
        })
    }
    pub(super) fn policy(&self) -> &GuardPolicy {
        &self.policy
    }
    pub(super) async fn acquire(
        &self,
        opts: &Opts,
        maria: bool,
        table: &TableRef,
    ) -> AppResult<MysqlGuardHandle> {
        let _acquisition = tokio::select! {
            biased;
            _ = self.closed.cancelled() => return Err(super::closed()),
            lock = tokio::time::timeout(self.policy.acquisition_timeout, self.acquisition.lock()) => lock.map_err(|_| expired())?,
        };
        if self.closed.is_cancelled() {
            return Err(super::closed());
        }
        let (owner, control, receiver) = {
            let mut entries = self.entries.lock();
            entries.retain(|_, entry| !entry.control.finished.is_cancelled());
            if let Some(owner) = entries
                .values()
                .filter_map(|entry| entry.owner.upgrade())
                .find(|owner| {
                    owner.identity.table == *table
                        && owner.control.live()
                        && owner.schema.get().is_some()
                })
            {
                return Ok(MysqlGuardHandle(owner));
            }
            if entries.len() >= self.policy.max_guards {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Too many retained edit targets. Close old results or refresh after their leases expire.",
                ));
            }
            let identity = GuardIdentity {
                generation: Uuid::new_v4(),
                session_id: self.session_id.clone(),
                table: table.clone(),
            };
            let control = Arc::new(Control {
                revoked: CancellationToken::new(),
                finished: CancellationToken::new(),
                deadline: tokio::time::Instant::now() + self.policy.lifetime,
            });
            let (requests, receiver) = mpsc::channel(1);
            let owner = Arc::new(GuardOwner {
                identity,
                control: control.clone(),
                schema: OnceLock::new(),
                requests,
            });
            entries.insert(
                owner.identity.generation,
                Entry {
                    owner: Arc::downgrade(&owner),
                    control: control.clone(),
                },
            );
            (owner, control, receiver)
        };
        let (ready, result) = oneshot::channel();
        // The actor owns Control, never GuardOwner. Last-handle release therefore
        // closes the socket even while the actor is waiting for a request.
        tokio::spawn(run_actor(
            ActorSetup {
                opts: opts.clone(),
                maria,
                table: table.clone(),
                policy: self.policy.clone(),
                control,
                closed: self.closed.clone(),
            },
            receiver,
            ready,
        ));
        let handle = MysqlGuardHandle(owner);
        let schema = result.await.map_err(|_| expired())??;
        if !handle.is_live() || self.closed.is_cancelled() {
            return Err(expired());
        }
        let _ = handle.0.schema.set(schema);
        Ok(handle)
    }
    fn revoke(&self) -> Vec<Arc<Control>> {
        let entries = self.entries.lock();
        entries
            .values()
            .map(|entry| {
                entry.control.revoked.cancel();
                entry.control.clone()
            })
            .collect()
    }
    pub(super) fn close(&self) {
        self.revoke();
    }
    pub(super) async fn invalidate(&self) -> AppResult<()> {
        let controls = self.revoke();
        // All actors close concurrently; this is one total bound, not N bounds.
        tokio::time::timeout(self.policy.close_timeout + Duration::from_secs(1), async {
            for control in controls { control.finished.cancelled().await; }
        }).await.map_err(|_| AppError::new(ErrorCode::Conflict, "Retained table locks are still closing. Retry the approved operation after they close."))
    }
}
impl Drop for GuardRegistry {
    fn drop(&mut self) {
        self.close();
    }
}

struct ActorSetup {
    opts: Opts,
    maria: bool,
    table: TableRef,
    policy: GuardPolicy,
    control: Arc<Control>,
    closed: CancellationToken,
}
async fn run_actor(
    setup: ActorSetup,
    mut requests: mpsc::Receiver<Verify>,
    ready: oneshot::Sender<AppResult<TableSchema>>,
) {
    let ActorSetup {
        opts,
        maria,
        table,
        policy,
        control,
        closed,
    } = setup;
    // Keep the connection outside the cancelled acquisition future so shutdown
    // can explicitly disconnect it even if initialization or locking is pending.
    let mut conn = None;
    let acquired = tokio::select! {
        biased;
        _ = control.revoked.cancelled() => Err(expired()),
        _ = closed.cancelled() => Err(super::closed()),
        _ = tokio::time::sleep_until(control.deadline) => Err(expired()),
        result = tokio::time::timeout(policy.acquisition_timeout, async {
            conn = Some(connect(&opts).await?);
            let conn = conn.as_mut().unwrap();
            initialize(conn, false, maria).await?;
            conn.query_drop("SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED").await.map_err(mysql_error)?;
            conn.query_drop("SET SESSION lock_wait_timeout=5").await.map_err(mysql_error)?;
            // A row read (including an empty result) retains the base table MDL.
            // No session-local temporary objects exist on this dedicated socket.
            conn.query_drop("START TRANSACTION READ ONLY").await.map_err(mysql_error)?;
            conn.query_drop(format!("SELECT 1 FROM {} LIMIT 1", values::qualified(&table))).await.map_err(mysql_error)?;
            let schema = catalog::schema_on(conn, &table).await?;
            if !schema.editable { return Err(AppError::new(ErrorCode::PolicyBlocked, "This table cannot retain an editable target")); }
            Ok(schema)
        }) => result.unwrap_or_else(|_| Err(expired())),
    };
    let usable = acquired.is_ok();
    if ready.send(acquired).is_err() || !usable {
        control.revoked.cancel();
    }
    if usable {
        while control.live() && !closed.is_cancelled() {
            let request = tokio::select! {
                biased;
                _ = control.revoked.cancelled() => break,
                _ = closed.cancelled() => break,
                _ = tokio::time::sleep_until(control.deadline) => break,
                request = requests.recv() => match request { Some(request) => request, None => break },
            };
            let result = tokio::select! {
                biased;
                _ = control.revoked.cancelled() => Err(expired()),
                _ = closed.cancelled() => Err(super::closed()),
                _ = request.closed.cancelled() => Err(super::closed()),
                _ = request.cancelled.cancelled() => Err(cancelled_error()),
                _ = tokio::time::sleep_until(control.deadline) => Err(expired()),
                result = tokio::time::timeout(policy.verification_timeout, async {
                    let conn = conn.as_mut().ok_or_else(expired)?;
                    // COM_PING performs actual I/O, never auto-reconnects, and
                    // refreshes last_ok_packet. Cached flags cannot prove liveness.
                    conn.ping().await.map_err(|_| expired())?;
                    let in_transaction = conn.last_ok_packet().is_some_and(|ok| ok.status_flags().contains(StatusFlags::SERVER_STATUS_IN_TRANS));
                    if !in_transaction || !control.live() { return Err(expired()); }
                    if request.closed.is_cancelled() || closed.is_cancelled() { return Err(super::closed()); }
                    if request.cancelled.is_cancelled() { return Err(cancelled_error()); }
                    Ok(())
                }) => result.unwrap_or_else(|_| Err(expired())),
            };
            let failed = result.is_err();
            if request.reply.send(result).is_err() || failed {
                break;
            }
        }
    }
    control.revoked.cancel();
    if let Some(mut conn) = conn
        && let Some(conn) = conn.0.take()
    {
        // disconnect marks Conn disconnected before awaiting I/O. On timeout
        // its Drop closes the stream directly instead of spawning a drain.
        let _ = tokio::time::timeout(policy.close_timeout, conn.disconnect()).await;
    }
    control.finished.cancel();
}

pub(super) fn expired() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The original table edit target is no longer protected. Refresh or preview again before applying changes.",
    )
}
fn cancelled_error() -> AppError {
    AppError::new(
        ErrorCode::Cancelled,
        "The edit was cancelled before submission",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> MysqlGuardHandle {
        let (requests, _) = mpsc::channel(1);
        MysqlGuardHandle(Arc::new(GuardOwner {
            identity: GuardIdentity {
                generation: Uuid::new_v4(),
                session_id: "session".into(),
                table: TableRef {
                    schema: "fixture".into(),
                    name: "owned".into(),
                },
            },
            control: Arc::new(Control {
                revoked: CancellationToken::new(),
                finished: CancellationToken::new(),
                deadline: tokio::time::Instant::now() + Duration::from_secs(1),
            }),
            schema: OnceLock::new(),
            requests,
        }))
    }
    #[tokio::test]
    async fn last_owner_release_revokes_actor_control() {
        let handle = owner();
        let control = handle.0.control.clone();
        let retained = handle.clone();
        drop(handle);
        assert!(!control.revoked.is_cancelled());
        drop(retained);
        assert!(control.revoked.is_cancelled());
    }
    #[tokio::test]
    async fn identity_is_stable_and_never_depends_on_liveness() {
        let handle = owner();
        let copy = handle.clone();
        let serialized = serde_json::to_string(&handle).unwrap();
        handle.0.control.revoked.cancel();
        assert_eq!(handle, copy);
        assert_eq!(serialized, serde_json::to_string(&handle).unwrap());
        assert_ne!(handle, owner());
    }
    #[tokio::test]
    async fn wrong_session_and_expired_handle_cannot_verify() {
        let handle = owner();
        let cancel = CancellationToken::new();
        let error = handle
            .verify("other", &handle.identity().table, &cancel, &cancel)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        handle.0.control.revoked.cancel();
        let error = handle
            .verify("session", &handle.identity().table, &cancel, &cancel)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
    }
    #[tokio::test]
    async fn abandoned_verification_revokes_original_generation() {
        let mut handle = owner();
        let (requests, mut received) = mpsc::channel(1);
        Arc::get_mut(&mut handle.0).unwrap().requests = requests;
        let control = handle.0.control.clone();
        let cancel = CancellationToken::new();
        let mut future =
            Box::pin(handle.verify("session", &handle.identity().table, &cancel, &cancel));
        let _request = tokio::select! { _ = &mut future => panic!("unexpected reply"), request = received.recv() => request.unwrap() };
        assert!(!control.revoked.is_cancelled());
        drop(future);
        assert!(control.revoked.is_cancelled());
    }
    #[tokio::test]
    async fn revoke_closes_all_generations_without_retaining_owners() {
        let registry =
            GuardRegistry::new("session", GuardPolicy::default(), CancellationToken::new())
                .unwrap();
        let first = owner();
        let second = owner();
        for handle in [&first, &second] {
            registry.entries.lock().insert(
                handle.identity().generation,
                Entry {
                    owner: Arc::downgrade(&handle.0),
                    control: handle.0.control.clone(),
                },
            );
        }
        registry.close();
        assert!(!first.is_live());
        assert!(!second.is_live());
        let weak = Arc::downgrade(&first.0);
        drop(first);
        assert!(weak.upgrade().is_none());
        assert_eq!(registry.entries.lock().len(), 2);
    }
    #[tokio::test]
    async fn deadline_expires_while_owner_is_still_retained() {
        let mut handle = owner();
        Arc::get_mut(&mut handle.0).unwrap().control = Arc::new(Control {
            revoked: CancellationToken::new(),
            finished: CancellationToken::new(),
            deadline: tokio::time::Instant::now() - Duration::from_millis(1),
        });
        assert!(!handle.is_live());
        let cancel = CancellationToken::new();
        assert_eq!(
            handle
                .verify("session", &handle.identity().table, &cancel, &cancel)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
    }

    #[tokio::test]
    async fn cancellation_remains_truthful_after_releasing_the_original_guard() {
        let handle = owner();
        handle.0.control.revoked.cancel();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let closed = CancellationToken::new();
        let error = handle
            .verify("session", &handle.identity().table, &cancelled, &closed)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled);
        closed.cancel();
        let fresh_cancel = CancellationToken::new();
        let error = handle
            .verify("session", &handle.identity().table, &fresh_cancel, &closed)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ConnectionClosed);
    }
}
