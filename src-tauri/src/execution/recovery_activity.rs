//! Short synchronous admission, with a counter retained through async work.
//! Recovery reservation and ordinary admission share the configuration gate;
//! the counter never holds a mutex across an await.
use super::*;

pub(super) struct RecoveryActivity<'a> {
    count: &'a AtomicUsize,
}

pub(super) struct OwnedRecoveryActivity {
    studio: Arc<Studio>,
}

impl Drop for OwnedRecoveryActivity {
    fn drop(&mut self) {
        self.studio.recovery_activity.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for RecoveryActivity<'_> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Studio {
    pub(super) fn configuration_mutation_guard(
        &self,
    ) -> AppResult<parking_lot::MutexGuard<'_, ()>> {
        let guard = self.configuration_gate.lock();
        if self.recovery_active.load(Ordering::Acquire) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Local metadata recovery is in progress; wait for it to finish",
            ));
        }
        Ok(guard)
    }

    pub(super) fn begin_recovery_activity(&self) -> AppResult<RecoveryActivity<'_>> {
        let _configuration = self.configuration_mutation_guard()?;
        self.increment_recovery_activity()?;
        Ok(RecoveryActivity {
            count: &self.recovery_activity,
        })
    }

    /// Transfer activity coverage to a detached owner before returning its ID.
    /// The caller already holds either configuration admission or an activity
    /// guard, so recovery cannot reserve between increment and task ownership.
    pub(super) fn retain_recovery_activity(self: &Arc<Self>) -> AppResult<OwnedRecoveryActivity> {
        self.increment_recovery_activity()?;
        Ok(OwnedRecoveryActivity {
            studio: self.clone(),
        })
    }

    fn increment_recovery_activity(&self) -> AppResult<()> {
        let mut count = self.recovery_activity.load(Ordering::Acquire);
        loop {
            let next = count
                .checked_add(1)
                .ok_or_else(|| AppError::new(ErrorCode::Conflict, "Too many local operations"))?;
            match self.recovery_activity.compare_exchange_weak(
                count,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(current) => count = current,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    #[test]
    fn nested_and_detached_activity_remain_counted_until_the_last_owner_drops() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        let first = studio.begin_recovery_activity().unwrap();
        let nested = studio.begin_recovery_activity().unwrap();
        let detached = studio.retain_recovery_activity().unwrap();
        assert_eq!(studio.recovery_activity.load(Ordering::Acquire), 3);
        drop(first);
        drop(nested);
        assert_eq!(studio.recovery_activity.load(Ordering::Acquire), 1);
        drop(detached);
        assert_eq!(studio.recovery_activity.load(Ordering::Acquire), 0);
    }

    #[test]
    fn recovery_reservation_rejects_writers_and_receipt_recovery_but_allows_reads() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        {
            let _configuration = studio.configuration_gate.lock();
            assert_eq!(studio.recovery_activity.load(Ordering::Acquire), 0);
            studio.recovery_active.store(true, Ordering::Release);
        }
        assert!(
            matches!(studio.begin_recovery_activity(), Err(error) if error.code == ErrorCode::Conflict)
        );
        assert_eq!(
            studio.clear_history().unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio.set_temporary_result_storage(true).unwrap_err().code,
            ErrorCode::Conflict
        );
        let client = new_id();
        assert_eq!(
            studio.snowflake_console_request(&client).unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            studio.snowflake_console_pending().unwrap_err().code,
            ErrorCode::Conflict
        );
        studio.bootstrap().unwrap();
        studio.metadata_backups().unwrap();
        studio.recovery_active.store(false, Ordering::Release);
        studio.clear_history().unwrap();
    }

    #[tokio::test]
    async fn recovery_reservation_rejects_async_admission_before_session_lookup() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
        studio.recovery_active.store(true, Ordering::Release);
        assert_eq!(
            studio.health("missing-session").await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(studio.recovery_activity.load(Ordering::Acquire), 0);
    }
}
