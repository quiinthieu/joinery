//! Retained local reap authority without ownership of the helper or its root.
use super::{CleanupReport, HelperGeneration, ReapedHelperGeneration, Supervisor};
use std::time::Duration;
use tokio::{sync::watch, time::timeout};

/// Observation of one exact helper's retirement and actual child disposition.
/// This retains neither the process nor cleanup/removal authority, and never
/// proves remote rollback, transaction disposition, or successful logout.
#[derive(Clone)]
pub struct HelperRetirementObserver {
    identity: HelperGeneration,
    retired: watch::Receiver<bool>,
    cleanup: watch::Receiver<Option<CleanupReport>>,
}

impl Supervisor {
    pub fn retirement_observer(&self) -> HelperRetirementObserver {
        HelperRetirementObserver {
            identity: self.helper_generation(),
            retired: self.inner.stop.subscribe(),
            cleanup: self.inner.exited.receiver.clone(),
        }
    }
}

impl HelperRetirementObserver {
    pub fn helper_generation(&self) -> HelperGeneration {
        self.identity.clone()
    }

    /// Only the exact private retirement and reaper publications grant proof.
    /// A closed channel or a dead supervisor identity is insufficient.
    pub fn reaped_generation(&self) -> Option<ReapedHelperGeneration> {
        if !*self.retired.borrow()
            || !self
                .cleanup
                .borrow()
                .is_some_and(|report| report.child_started && report.reaped)
        {
            return None;
        }
        Some(ReapedHelperGeneration {
            owner: self.identity.owner.clone(),
            generation: self.identity.generation.clone(),
        })
    }

    /// Cancellation and timeout leave this retained observer available to retry.
    pub async fn wait(&self, bound: Duration) -> Option<ReapedHelperGeneration> {
        if let Some(proof) = self.reaped_generation() {
            return Some(proof);
        }
        let mut retired = self.retired.clone();
        let mut cleanup = self.cleanup.clone();
        timeout(bound, async {
            let mut retired_open = true;
            let mut cleanup_open = true;
            loop {
                if let Some(proof) = self.reaped_generation() {
                    return Some(proof);
                }
                // A closed watch cannot supply a missing fact. A retained true
                // fact still permits waiting for the other private publication.
                let retirement_confirmed = *retired.borrow();
                let reap_confirmed = cleanup
                    .borrow()
                    .is_some_and(|report| report.child_started && report.reaped);
                if (!retired_open && !retirement_confirmed) || (!cleanup_open && !reap_confirmed) {
                    return None;
                }
                if !retired_open && !cleanup_open {
                    return self.reaped_generation();
                }
                tokio::select! {
                    changed = retired.changed(), if retired_open => {
                        retired_open = changed.is_ok();
                    }
                    changed = cleanup.changed(), if cleanup_open => {
                        cleanup_open = changed.is_ok();
                    }
                }
            }
        })
        .await
        .ok()
        .flatten()
    }
}
