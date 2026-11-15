//! Retain child cleanup authority independently of auth futures and live sessions.
use crate::{domain::*, drivers::snowflake_process::CleanupObserver};
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::watch;

pub struct SnowflakeCleanup {
    entries: Mutex<HashMap<String, Arc<CleanupSlot>>>,
    limit: usize,
}

struct CleanupSlot {
    state: watch::Sender<SlotState>,
}

#[derive(Clone)]
enum SlotState {
    Preparing,
    Attached(CleanupObserver),
    NoLaunch,
}

/// One non-cloneable right to attach cleanup authority to an existing slot.
/// Cancellation/closing never invalidates attachment or releases its capacity.
pub struct CleanupPermit {
    slot: Option<Arc<CleanupSlot>>,
}

impl CleanupPermit {
    /// Consume only from Launch::new_observed's synchronous callback, immediately
    /// after allocation. No admission/capacity check is permitted at attachment.
    pub fn attach(mut self, observer: CleanupObserver) {
        let slot = self.slot.take().expect("owned cleanup permit");
        slot.state.send_replace(SlotState::Attached(observer));
    }
}

impl Drop for CleanupPermit {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            slot.state.send_replace(SlotState::NoLaunch);
        }
    }
}

/// A retirement snapshot includes pending preparation, not just existing child
/// observers. Later reservations are independent; later attachment is included.
#[derive(Clone)]
pub struct CleanupSnapshot {
    entries: Vec<(String, Arc<CleanupSlot>)>,
}

impl Default for SnowflakeCleanup {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            limit: 32,
        }
    }
}

fn complete(observer: &CleanupObserver) -> bool {
    observer.report().is_some_and(|report| {
        report.temporary_root_removed && (!report.child_started || report.reaped)
    })
}

fn slot_complete(slot: &CleanupSlot) -> bool {
    match &*slot.state.borrow() {
        SlotState::Preparing => false,
        SlotState::Attached(observer) => complete(observer),
        SlotState::NoLaunch => true,
    }
}

fn cleanup_error() -> AppError {
    AppError::new(
        ErrorCode::ConnectionFailed,
        "Snowflake helper cleanup could not be confirmed. Remote query or transaction outcomes remain unverified.",
    )
}

impl SnowflakeCleanup {
    /// Call under the configuration gate, before allocating a state root. A
    /// pending permit remains visible until its owner attaches or drops it.
    pub fn reserve(&self, id: &str) -> AppResult<CleanupPermit> {
        let mut entries = self.entries.lock();
        entries.retain(|_, entry| !slot_complete(entry));
        if entries.contains_key(id) || entries.len() >= self.limit {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Snowflake helper cleanup is still pending. Wait before connecting again.",
            ));
        }
        let (state, _) = watch::channel(SlotState::Preparing);
        let slot = Arc::new(CleanupSlot { state });
        entries.insert(id.to_owned(), slot.clone());
        Ok(CleanupPermit { slot: Some(slot) })
    }

    /// Compatibility for existing observers. New launches must reserve before
    /// allocation and attach through Launch::new_observed instead.
    pub fn register(&self, id: &str, observer: CleanupObserver) -> AppResult<()> {
        self.reserve(id)?.attach(observer);
        Ok(())
    }

    pub async fn wait_for(&self, id: &str, bound: Duration) -> AppResult<()> {
        let slot = self.entries.lock().get(id).cloned();
        if let Some(slot) = slot {
            self.wait_entry(id, slot, bound).await?;
        }
        Ok(())
    }

    async fn wait_entry(&self, id: &str, slot: Arc<CleanupSlot>, bound: Duration) -> AppResult<()> {
        let mut receiver = slot.state.subscribe();
        // One bound includes preparation, child disposition and a removal retry.
        tokio::time::timeout(bound, async {
            loop {
                let state = receiver.borrow_and_update().clone();
                match state {
                    SlotState::Preparing => {
                        receiver.changed().await.map_err(|_| cleanup_error())?
                    }
                    SlotState::NoLaunch => return Ok(()),
                    SlotState::Attached(observer) => {
                        let _ = observer.settle(bound).await;
                        return if complete(&observer) {
                            Ok(())
                        } else {
                            Err(cleanup_error())
                        };
                    }
                }
            }
        })
        .await
        .map_err(|_| cleanup_error())??;
        let mut entries = self.entries.lock();
        if entries
            .get(id)
            .is_some_and(|entry| Arc::ptr_eq(entry, &slot))
        {
            entries.remove(id);
        }
        Ok(())
    }

    /// The caller must first prevent new connections and retire existing work.
    pub async fn wait_all(&self, bound: Duration) -> AppResult<()> {
        self.wait_snapshot(&self.snapshot(), bound).await
    }

    /// Capture under the same gate that decides which work is being retired.
    pub fn snapshot(&self) -> CleanupSnapshot {
        CleanupSnapshot {
            entries: self
                .entries
                .lock()
                .iter()
                .map(|(id, slot)| (id.clone(), slot.clone()))
                .collect(),
        }
    }

    pub async fn wait_snapshot(
        &self,
        snapshot: &CleanupSnapshot,
        bound: Duration,
    ) -> AppResult<()> {
        let results = futures_util::future::join_all(
            snapshot
                .entries
                .iter()
                .map(|(id, slot)| async move { self.wait_entry(id, slot.clone(), bound).await }),
        )
        .await;
        for result in results {
            result?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::snowflake_process::Launch;

    fn launch() -> Launch {
        Launch::new(
            std::path::PathBuf::from("/missing-snowflake-node"),
            None,
            std::env::temp_dir(),
            tempfile::tempdir().unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn pending_cleanup_retains_capacity_until_explicit_completion() {
        let registry = SnowflakeCleanup {
            limit: 1,
            ..Default::default()
        };
        let first = launch();
        registry
            .register("first", first.cleanup_observer())
            .unwrap();
        let second = launch();
        assert!(
            registry
                .register("second", second.cleanup_observer())
                .is_err()
        );
        assert!(registry.wait_for("first", Duration::ZERO).await.is_err());
        assert_eq!(registry.entries.lock().len(), 1);
        drop(first);
        registry
            .register("second", second.cleanup_observer())
            .unwrap();
        assert!(!registry.entries.lock().contains_key("first"));
        drop(second);
        registry.wait_all(Duration::from_millis(10)).await.unwrap();
        assert!(registry.entries.lock().is_empty());
    }

    #[tokio::test]
    async fn cancelled_wait_preserves_cleanup_tracking() {
        let registry = SnowflakeCleanup::default();
        let launch = launch();
        registry
            .register("owned", launch.cleanup_observer())
            .unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(1),
                registry.wait_all(Duration::from_secs(60))
            )
            .await
            .is_err()
        );
        assert_eq!(registry.entries.lock().len(), 1);
        drop(launch);
        registry
            .wait_for("owned", Duration::from_millis(10))
            .await
            .unwrap();
        assert!(registry.entries.lock().is_empty());
    }

    #[tokio::test]
    async fn preparing_permit_retains_capacity_until_no_launch_or_attachment() {
        let registry = SnowflakeCleanup {
            limit: 1,
            ..Default::default()
        };
        let permit = registry.reserve("preparing").unwrap();
        assert!(registry.reserve("second").is_err());
        assert!(registry.wait_all(Duration::ZERO).await.is_err());
        assert!(matches!(
            *registry.entries.lock()["preparing"].state.borrow(),
            SlotState::Preparing
        ));
        drop(permit);
        registry
            .wait_for("preparing", Duration::from_secs(1))
            .await
            .unwrap();
        assert!(registry.entries.lock().is_empty());
        let second = registry.reserve("second").unwrap();
        drop(second);
        registry.wait_all(Duration::from_secs(1)).await.unwrap();
    }

    #[tokio::test]
    async fn snapshot_includes_later_attachment_but_not_later_reservations() {
        let registry = SnowflakeCleanup::default();
        let permit = registry.reserve("old").unwrap();
        let snapshot = registry.snapshot();
        // Poll the pending snapshot wait before attachment, exercising its wakeup.
        let mut waiting = Box::pin(registry.wait_snapshot(&snapshot, Duration::from_secs(3)));
        assert!(futures_util::poll!(waiting.as_mut()).is_pending());
        let later = registry.reserve("later").unwrap();
        let launch = launch();
        permit.attach(launch.cleanup_observer());
        assert!(futures_util::poll!(waiting.as_mut()).is_pending());
        drop(launch);
        waiting.await.unwrap();
        assert!(!registry.entries.lock().contains_key("old"));
        assert!(registry.entries.lock().contains_key("later"));
        assert!(registry.wait_all(Duration::ZERO).await.is_err());
        drop(later);
        registry.wait_all(Duration::from_secs(1)).await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_snapshot_wait_preserves_pending_permit_and_late_attachment() {
        let registry = SnowflakeCleanup {
            limit: 1,
            ..Default::default()
        };
        let permit = registry.reserve("owned").unwrap();
        let snapshot = registry.snapshot();
        {
            let mut waiting = Box::pin(registry.wait_snapshot(&snapshot, Duration::from_secs(60)));
            assert!(futures_util::poll!(waiting.as_mut()).is_pending());
        }
        assert!(registry.reserve("second").is_err());
        let launch = launch();
        permit.attach(launch.cleanup_observer());
        drop(launch);
        registry
            .wait_snapshot(&snapshot, Duration::from_secs(1))
            .await
            .unwrap();
        assert!(registry.entries.lock().is_empty());
    }

    #[tokio::test]
    async fn old_snapshot_completion_cannot_remove_a_reused_identifier() {
        let registry = SnowflakeCleanup {
            limit: 1,
            ..Default::default()
        };
        let old = registry.reserve("same").unwrap();
        let snapshot = registry.snapshot();
        drop(old);
        let new = registry.reserve("same").unwrap();
        registry
            .wait_snapshot(&snapshot, Duration::from_secs(1))
            .await
            .unwrap();
        assert!(registry.entries.lock().contains_key("same"));
        assert!(registry.reserve("other").is_err());
        assert!(registry.wait_for("same", Duration::ZERO).await.is_err());
        drop(new);
        registry.wait_all(Duration::from_secs(1)).await.unwrap();
    }

    #[tokio::test]
    async fn observed_constructor_failure_keeps_capacity_until_original_root_is_removed() {
        let registry = SnowflakeCleanup {
            limit: 1,
            ..Default::default()
        };
        let permit = registry.reserve("owned").unwrap();
        let parent = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir_in(parent.path()).unwrap();
        let root = state.path().to_owned();
        let retained = parent.path().join("retained-original");
        let result = Launch::new_observed(
            std::path::PathBuf::from("/missing-snowflake-node"),
            None,
            parent.path().to_owned(),
            state,
            |observer| {
                permit.attach(observer);
                std::fs::rename(&root, &retained).unwrap();
            },
        );
        assert!(result.is_err());
        assert!(
            registry
                .wait_for("owned", Duration::from_secs(1))
                .await
                .is_err()
        );
        assert!(registry.reserve("second").is_err());
        assert!(retained.is_dir());
        std::fs::rename(&retained, &root).unwrap();
        registry
            .wait_for("owned", Duration::from_secs(1))
            .await
            .unwrap();
        assert!(!root.exists());
        let second = registry.reserve("second").unwrap();
        drop(second);
        registry.wait_all(Duration::from_secs(1)).await.unwrap();
    }
}
