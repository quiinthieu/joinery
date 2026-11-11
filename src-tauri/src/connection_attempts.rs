//! Reservations for connection setup, separate from published SQL sessions.
//!
//! Callers hold Studio's configuration gate while reading live-session counts,
//! reserving, and publishing. Publication additionally holds this registry's lock
//! so cancellation cannot fall between its final check and session insertion.
use crate::domain::{AppError, AppResult, ErrorCode};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const RESERVATION_TTL: Duration = Duration::from_secs(60);
const MAX_CONNECTIONS: usize = 32;
const MAX_PROFILE_CONNECTIONS: usize = 4;

#[derive(Clone)]
pub struct ConnectionAttempts {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    changed: tokio::sync::Notify,
    ttl: Duration,
    max_connections: usize,
    max_profile_connections: usize,
}

#[derive(Default)]
struct State {
    closing: bool,
    entries: HashMap<String, Entry>,
}

struct Entry {
    profile_id: String,
    expires_at: Instant,
    claimed: bool,
    cancellation: CancellationToken,
}

/// A claimed reservation is deliberately not Clone. Dropping its only owner
/// cancels setup and releases capacity even when the connect future is abandoned.
pub struct ConnectionAttempt {
    registry: ConnectionAttempts,
    id: String,
    profile_id: String,
    cancellation: CancellationToken,
    disarmed: bool,
}

impl Default for ConnectionAttempts {
    fn default() -> Self {
        Self::with_limits(RESERVATION_TTL, MAX_CONNECTIONS, MAX_PROFILE_CONNECTIONS)
    }
}

impl ConnectionAttempts {
    fn with_limits(ttl: Duration, max_connections: usize, max_profile_connections: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                changed: tokio::sync::Notify::new(),
                ttl,
                max_connections,
                max_profile_connections,
            }),
        }
    }

    /// `profile_id` must already identify a validated persisted profile. Live
    /// counts must be read under the same configuration gate as this reservation.
    /// Prepared and claimed reservations both consume capacity; closing helper
    /// processes require a separate cleanup-registry capacity reservation.
    pub fn reserve(
        &self,
        profile_id: &str,
        live_total: usize,
        live_for_profile: usize,
    ) -> AppResult<String> {
        self.reserve_at(profile_id, live_total, live_for_profile, Instant::now())
    }

    fn reserve_at(
        &self,
        profile_id: &str,
        live_total: usize,
        live_for_profile: usize,
        now: Instant,
    ) -> AppResult<String> {
        let mut state = self.inner.state.lock();
        self.prune(&mut state, now);
        if state.closing {
            return Err(cancelled());
        }
        if live_total.saturating_add(state.entries.len()) >= self.inner.max_connections {
            return Err(AppError::invalid(
                "At most 32 connections may be open or connecting",
            ));
        }
        let pending_for_profile = state
            .entries
            .values()
            .filter(|entry| entry.profile_id == profile_id)
            .count();
        if live_for_profile.saturating_add(pending_for_profile)
            >= self.inner.max_profile_connections
        {
            return Err(AppError::invalid(
                "This connection already has four open or pending sessions",
            ));
        }
        let id = loop {
            let id = uuid::Uuid::new_v4().to_string();
            if !state.entries.contains_key(&id) {
                break id;
            }
        };
        state.entries.insert(
            id.clone(),
            Entry {
                profile_id: profile_id.to_owned(),
                expires_at: now + self.inner.ttl,
                claimed: false,
                cancellation: CancellationToken::new(),
            },
        );
        Ok(id)
    }

    pub fn claim(&self, id: &str) -> AppResult<ConnectionAttempt> {
        self.claim_at(id, Instant::now())
    }

    fn claim_at(&self, id: &str, now: Instant) -> AppResult<ConnectionAttempt> {
        let mut state = self.inner.state.lock();
        self.prune(&mut state, now);
        if state.closing {
            return Err(cancelled());
        }
        let entry = state.entries.get_mut(id).ok_or_else(cancelled)?;
        if entry.cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if entry.claimed {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This connection request has already started",
            ));
        }
        entry.claimed = true;
        Ok(ConnectionAttempt {
            registry: self.clone(),
            id: id.to_owned(),
            profile_id: entry.profile_id.clone(),
            cancellation: entry.cancellation.clone(),
            disarmed: false,
        })
    }

    pub fn cancel(&self, id: &str) -> bool {
        let mut state = self.inner.state.lock();
        let Some(entry) = state.entries.get(id) else {
            return false;
        };
        let newly_cancelled = !entry.cancellation.is_cancelled();
        entry.cancellation.cancel();
        if !entry.claimed {
            state.entries.remove(id);
        }
        self.inner.changed.notify_waiters();
        newly_cancelled
    }

    /// Profile save/delete use this while holding the configuration gate, before
    /// changing metadata. Even an unchanged save invalidates an older attempt.
    pub fn cancel_profile(&self, profile_id: &str) {
        self.inner.state.lock().entries.retain(|_, entry| {
            if entry.profile_id == profile_id {
                entry.cancellation.cancel();
                entry.claimed
            } else {
                true
            }
        });
        self.inner.changed.notify_waiters();
    }

    /// Retire the harness's current work without changing the shutdown barrier.
    /// Call under the configuration gate; later reservations own fresh work.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn cancel_fixture_snapshot(&self) -> Vec<String> {
        let mut state = self.inner.state.lock();
        let mut owners = Vec::new();
        state.entries.retain(|id, entry| {
            entry.cancellation.cancel();
            if entry.claimed {
                owners.push(id.clone());
                true
            } else {
                false
            }
        });
        drop(state);
        self.inner.changed.notify_waiters();
        owners
    }

    /// Drain only the cancelled owners captured by cancel_fixture_snapshot.
    /// Unrelated reservations admitted afterward must not extend this wait.
    #[cfg(any(test, feature = "test-harness"))]
    pub async fn wait_fixture_snapshot(&self, owners: &[String]) {
        loop {
            let changed = self.inner.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let pending = {
                let state = self.inner.state.lock();
                owners.iter().any(|id| state.entries.contains_key(id))
            };
            if !pending {
                return;
            }
            changed.await;
        }
    }

    /// Irreversible and idempotent. Call before taking the session snapshot that
    /// will be disconnected, so a publication winner is included in that snapshot.
    pub fn shutdown(&self) {
        let mut state = self.inner.state.lock();
        state.closing = true;
        state.entries.retain(|_, entry| {
            entry.cancellation.cancel();
            entry.claimed
        });
        self.inner.changed.notify_waiters();
    }

    /// Wait for claimed setup futures to unwind after shutdown/cancellation.
    /// This says nothing about remote rollback or a driver's asynchronous logout.
    pub async fn wait_idle(&self) {
        loop {
            let changed = self.inner.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.has_pending() {
                return;
            }
            changed.await;
        }
    }

    pub fn has_pending(&self) -> bool {
        let mut state = self.inner.state.lock();
        self.prune(&mut state, Instant::now());
        !state.entries.is_empty()
    }

    fn prune(&self, state: &mut State, now: Instant) {
        state.entries.retain(|_, entry| {
            let expired = now >= entry.expires_at;
            if !entry.claimed && (expired || entry.cancellation.is_cancelled()) {
                entry.cancellation.cancel();
                false
            } else {
                true
            }
        });
    }
}

impl ConnectionAttempt {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Finish preflight without publishing or releasing its reserved capacity.
    /// Call under the configuration gate so profile mutation and cancellation
    /// cannot race the transition back to a connectable reservation.
    pub fn return_to_reserved(self) -> AppResult<()> {
        self.return_to_reserved_at(Instant::now())
    }

    fn return_to_reserved_at(mut self, now: Instant) -> AppResult<()> {
        let mut state = self.registry.inner.state.lock();
        if state.closing || self.cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let entry = state
            .entries
            .get_mut(&self.id)
            .filter(|entry| entry.claimed)
            .ok_or_else(cancelled)?;
        entry.claimed = false;
        entry.expires_at = now + self.registry.inner.ttl;
        self.disarmed = true;
        drop(state);
        self.registry.inner.changed.notify_waiters();
        Ok(())
    }

    /// Atomically choose publication over cancellation/shutdown. The callback
    /// must be synchronous and infallible; it must not call back into this registry.
    /// Its lock order is configuration gate -> attempt registry -> session map.
    pub fn publish<T>(mut self, publish: impl FnOnce() -> T) -> AppResult<T> {
        self.try_publish(publish)
    }

    /// Retain a rejected claim until its owner has closed any acquired driver.
    /// A shutdown drain therefore cannot finish between rejection and close.
    pub fn try_publish<T>(&mut self, publish: impl FnOnce() -> T) -> AppResult<T> {
        self.try_publish_checked(|| Ok(publish()))
    }

    /// Revalidate after publication locks have been acquired. A rejected callback
    /// retains the claim so its owner can close the driver before releasing it.
    /// The synchronous callback must not reenter this registry.
    pub(crate) fn try_publish_checked<T>(
        &mut self,
        publish: impl FnOnce() -> AppResult<T>,
    ) -> AppResult<T> {
        let mut state = self.registry.inner.state.lock();
        if state.closing
            || self.cancellation.is_cancelled()
            || !state
                .entries
                .get(&self.id)
                .is_some_and(|entry| entry.claimed)
        {
            drop(state);
            return Err(cancelled());
        }
        let published = publish()?;
        state.entries.remove(&self.id);
        self.disarmed = true;
        drop(state);
        self.registry.inner.changed.notify_waiters();
        Ok(published)
    }
}

impl Drop for ConnectionAttempt {
    fn drop(&mut self) {
        if !self.disarmed
            && let Some(entry) = self.registry.inner.state.lock().entries.remove(&self.id)
        {
            entry.cancellation.cancel();
            self.registry.inner.changed.notify_waiters();
        }
    }
}

fn cancelled() -> AppError {
    AppError::new(
        ErrorCode::Cancelled,
        "This connection request was cancelled, expired, or the app is closing",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Barrier,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    #[test]
    fn reservations_are_server_issued_and_claimed_once() {
        let registry = ConnectionAttempts::default();
        let first = registry.reserve("profile", 0, 0).unwrap();
        let second = registry.reserve("profile", 0, 0).unwrap();
        assert_ne!(first, second);
        assert_eq!(uuid::Uuid::parse_str(&first).unwrap().get_version_num(), 4);
        let attempt = registry.claim(&first).unwrap();
        assert_eq!(attempt.id(), first);
        assert_eq!(attempt.profile_id(), "profile");
        assert_eq!(
            registry.claim(&first).err().unwrap().code,
            ErrorCode::Conflict
        );
        assert!(registry.claim("caller-chosen-id").is_err());
        drop(attempt);
        assert!(registry.claim(&first).is_err());
    }

    #[test]
    fn checked_publication_rejection_retains_cleanup_ownership() {
        let registry = ConnectionAttempts::with_limits(Duration::from_secs(60), 1, 1);
        let id = registry.reserve("profile", 0, 0).unwrap();
        let mut attempt = registry.claim(&id).unwrap();
        let error = attempt
            .try_publish_checked::<()>(|| {
                Err(AppError::new(
                    ErrorCode::PolicyBlocked,
                    "Publication revoked",
                ))
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PolicyBlocked);
        assert!(registry.has_pending());
        assert_eq!(registry.claim(&id).err().unwrap().code, ErrorCode::Conflict);
        assert!(registry.reserve("other", 0, 0).is_err());
        registry.shutdown();
        assert!(registry.has_pending());
        drop(attempt);
        assert!(!registry.has_pending());
    }

    #[test]
    fn unclaimed_expiry_releases_capacity_but_claimed_auth_is_not_expired() {
        let registry = ConnectionAttempts::with_limits(Duration::from_secs(60), 1, 1);
        let now = Instant::now();
        let expired = registry.reserve_at("profile", 0, 0, now).unwrap();
        assert!(
            registry
                .claim_at(&expired, now + Duration::from_secs(60))
                .is_err()
        );
        let next = registry.reserve_at("profile", 0, 0, now).unwrap();
        let active = registry.claim_at(&next, now).unwrap();
        registry.prune(
            &mut registry.inner.state.lock(),
            now + Duration::from_secs(600),
        );
        assert!(registry.has_pending());
        assert_eq!(active.publish(|| 42).unwrap(), 42);
        assert!(!registry.has_pending());
    }

    #[test]
    fn reservations_include_live_sessions_and_claimed_attempts_in_limits() {
        let registry = ConnectionAttempts::with_limits(Duration::from_secs(60), 3, 2);
        assert!(registry.reserve("profile", 3, 0).is_err());
        assert!(registry.reserve("profile", 0, 2).is_err());
        let id = registry.reserve("profile", 1, 1).unwrap();
        let attempt = registry.claim(&id).unwrap();
        assert!(registry.reserve("profile", 1, 1).is_err());
        registry.reserve("other", 1, 0).unwrap();
        assert!(registry.reserve("third", 1, 0).is_err());
        drop(attempt);
        assert!(registry.reserve("profile", 1, 1).is_ok());
    }

    #[test]
    fn cancellation_and_drop_prevent_publication_and_release_capacity() {
        let registry = ConnectionAttempts::with_limits(Duration::from_secs(60), 1, 1);
        let id = registry.reserve("profile", 0, 0).unwrap();
        let attempt = registry.claim(&id).unwrap();
        let token = attempt.cancellation();
        assert!(registry.cancel(&id));
        assert!(!registry.cancel(&id));
        assert!(registry.has_pending());
        assert!(registry.reserve("profile", 0, 0).is_err());
        let called = AtomicBool::new(false);
        assert!(
            attempt
                .publish(|| called.store(true, Ordering::SeqCst))
                .is_err()
        );
        assert!(!called.load(Ordering::SeqCst));
        assert!(token.is_cancelled());
        let next = registry.reserve("profile", 0, 0).unwrap();
        let abandoned = registry.claim(&next).unwrap();
        let token = abandoned.cancellation();
        drop(abandoned);
        assert!(token.is_cancelled());
        assert!(!registry.has_pending());
    }

    #[test]
    fn cancelled_claim_stays_accounted_until_its_owner_drops() {
        let registry = ConnectionAttempts::with_limits(Duration::from_secs(60), 1, 1);
        let id = registry.reserve("profile", 0, 0).unwrap();
        let active = registry.claim(&id).unwrap();
        registry.cancel_profile("profile");
        registry.prune(
            &mut registry.inner.state.lock(),
            Instant::now() + Duration::from_secs(600),
        );
        assert!(active.cancellation().is_cancelled());
        assert!(registry.has_pending());
        assert!(registry.claim(&id).is_err());
        assert!(registry.reserve("other", 0, 0).is_err());
        drop(active);
        assert!(!registry.has_pending());
        let next = registry.reserve("other", 0, 0).unwrap();
        assert!(registry.cancel(&next));
        assert!(
            !registry.has_pending(),
            "unclaimed cancellation can release immediately"
        );
    }

    #[test]
    fn profile_invalidation_is_exact_and_also_invalidates_unchanged_saves() {
        let registry = ConnectionAttempts::default();
        let id = registry.reserve("profile", 0, 0).unwrap();
        let prepared = registry.reserve("profile", 0, 0).unwrap();
        let other = registry.reserve("other", 0, 0).unwrap();
        let active = registry.claim(&id).unwrap();
        registry.cancel_profile("profile");
        assert!(active.cancellation().is_cancelled());
        assert!(active.publish(|| ()).is_err());
        assert!(registry.claim(&prepared).is_err());
        assert!(registry.claim(&other).is_ok());
    }

    #[test]
    fn shutdown_is_shared_irreversible_and_rejects_late_auth_success() {
        let registry = ConnectionAttempts::default();
        let id = registry.reserve("profile", 0, 0).unwrap();
        let active = registry.claim(&id).unwrap();
        let token = active.cancellation();
        registry.clone().shutdown();
        registry.shutdown();
        assert!(token.is_cancelled());
        assert!(
            registry.has_pending(),
            "shutdown cannot hide an auth owner still unwinding"
        );
        assert!(active.publish(|| ()).is_err());
        assert!(!registry.has_pending());
        assert!(registry.reserve("profile", 0, 0).is_err());
        assert!(registry.claim(&id).is_err());
    }

    #[test]
    fn publication_keeps_its_check_and_session_insertion_atomic_against_cancel() {
        let registry = ConnectionAttempts::default();
        let id = registry.reserve("profile", 0, 0).unwrap();
        let active = registry.claim(&id).unwrap();
        let token = active.cancellation();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let publication = std::thread::spawn(move || {
            active.publish(|| {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                42
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let cancelling = std::thread::spawn(move || registry.cancel(&id));
        release_tx.send(()).unwrap();
        assert_eq!(publication.join().unwrap().unwrap(), 42);
        assert!(!cancelling.join().unwrap());
        assert!(
            !token.is_cancelled(),
            "successful publication must not cancel its authentication token"
        );
    }

    #[test]
    fn concurrent_reservations_cannot_oversubscribe_pending_capacity() {
        let registry = ConnectionAttempts::with_limits(Duration::from_secs(60), 4, 4);
        let barrier = Arc::new(Barrier::new(16));
        let threads = (0..16)
            .map(|_| {
                let registry = registry.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    registry.reserve("profile", 0, 0).is_ok()
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            threads
                .into_iter()
                .filter_map(|thread| thread.join().unwrap().then_some(()))
                .count(),
            4
        );
    }

    #[tokio::test]
    async fn shutdown_drain_waits_for_the_claim_owner_to_drop() {
        let registry = ConnectionAttempts::default();
        let id = registry.reserve("profile", 0, 0).unwrap();
        let owner = registry.claim(&id).unwrap();
        registry.shutdown();
        let waiting = registry.wait_idle();
        tokio::pin!(waiting);
        tokio::select! {
            biased;
            _ = &mut waiting => panic!("a cancelled claim still owns its setup future"),
            _ = tokio::task::yield_now() => {}
        }
        drop(owner);
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap();
    }

    #[test]
    fn rejected_publication_retains_capacity_until_its_driver_owner_finishes() {
        let registry = ConnectionAttempts::default();
        let id = registry.reserve("profile", 0, 0).unwrap();
        let mut owner = registry.claim(&id).unwrap();
        registry.shutdown();
        assert!(
            owner
                .try_publish(|| panic!("cancelled publication"))
                .is_err()
        );
        assert!(registry.has_pending());
        drop(owner);
        assert!(!registry.has_pending());
    }

    #[test]
    fn successful_preflight_preserves_the_id_and_capacity_and_renews_only_unclaimed_ttl() {
        let registry = ConnectionAttempts::with_limits(Duration::from_secs(60), 1, 1);
        let now = Instant::now();
        let id = registry.reserve_at("profile", 0, 0, now).unwrap();
        let preflight = registry.claim_at(&id, now).unwrap();
        assert_eq!(registry.claim(&id).err().unwrap().code, ErrorCode::Conflict);
        preflight
            .return_to_reserved_at(now + Duration::from_secs(120))
            .unwrap();
        assert!(registry.reserve("profile", 0, 0).is_err());
        let connection = registry
            .claim_at(&id, now + Duration::from_secs(179))
            .unwrap();
        assert_eq!(connection.id(), id);
        assert_eq!(connection.profile_id(), "profile");
        connection
            .return_to_reserved_at(now + Duration::from_secs(180))
            .unwrap();
        assert!(
            registry
                .claim_at(&id, now + Duration::from_secs(240))
                .is_err()
        );
        assert!(!registry.has_pending());
    }

    #[test]
    fn preflight_cannot_revive_a_cancelled_invalidated_or_shutdown_reservation() {
        for invalidate in [0, 1, 2] {
            let registry = ConnectionAttempts::default();
            let id = registry.reserve("profile", 0, 0).unwrap();
            let owner = registry.claim(&id).unwrap();
            match invalidate {
                0 => {
                    registry.cancel(&id);
                }
                1 => registry.cancel_profile("profile"),
                _ => registry.shutdown(),
            }
            assert_eq!(
                owner.return_to_reserved().unwrap_err().code,
                ErrorCode::Cancelled
            );
            assert!(!registry.has_pending());
            assert!(registry.claim(&id).is_err());
        }
    }
}
