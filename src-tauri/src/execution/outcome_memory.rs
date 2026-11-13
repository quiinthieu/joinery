//! Payload capacity is owned before dispatch, including rollback after disk failure.
use crate::domain::{AppError, AppResult, ErrorCode};
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(super) const PAYLOAD_BYTES: usize = 32 * 1024;
const SLOTS: usize = 32;

#[derive(Default)]
struct Pool(AtomicUsize);

#[derive(Default)]
pub(super) struct OutcomeMemory {
    ordinary: Arc<Pool>,
    safety: Arc<Pool>,
    // Only one producer serializes its compact original at a time. Storage
    // replay has its own bounded batches; no full result is retained here.
    pub(super) encoding: Mutex<()>,
}

struct BufferLease {
    pool: Arc<Pool>,
    buffer: Option<Vec<u8>>,
}

impl BufferLease {
    fn claim(pool: &Arc<Pool>) -> AppResult<Self> {
        let mut used = pool.0.load(Ordering::Acquire);
        loop {
            if used >= SLOTS {
                return Err(AppError::new(
                    ErrorCode::ResultLimitReached,
                    "Local outcome capacity is full; let pending recording settle before opening another connection or action",
                ));
            }
            match pool
                .0
                .compare_exchange_weak(used, used + 1, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(current) => used = current,
            }
        }
        Ok(Self {
            pool: pool.clone(),
            buffer: Some(Vec::with_capacity(PAYLOAD_BYTES)),
        })
    }
}

impl Drop for BufferLease {
    fn drop(&mut self) {
        self.pool.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A session retains its reserved safety buffer while idle, without an OS lock.
/// An unfinished rollback keeps this Arc even after its session disconnects.
pub(super) struct SafetySlot(Mutex<Option<BufferLease>>);

pub(super) struct MemoryLease {
    lease: Option<BufferLease>,
    return_to: Option<Arc<SafetySlot>>,
}

impl OutcomeMemory {
    pub(super) fn ordinary(&self) -> AppResult<MemoryLease> {
        Ok(MemoryLease {
            lease: Some(BufferLease::claim(&self.ordinary)?),
            return_to: None,
        })
    }

    pub(super) fn safety(&self) -> AppResult<Arc<SafetySlot>> {
        Ok(Arc::new(SafetySlot(Mutex::new(Some(BufferLease::claim(
            &self.safety,
        )?)))))
    }
}

impl SafetySlot {
    #[cfg_attr(
        not(all(test, feature = "test-harness")),
        allow(
            dead_code,
            reason = "Studio grid run owner precedes public UI staging and IPC wiring"
        )
    )]
    pub(super) fn is_available(&self) -> bool {
        self.0.lock().is_some()
    }

    pub(super) fn claim(self: &Arc<Self>) -> AppResult<MemoryLease> {
        let lease = self.0.lock().take().ok_or_else(|| AppError::new(
            ErrorCode::Conflict,
            "The earlier rollback is still running or awaiting local recording; restore local storage before another rollback",
        ))?;
        Ok(MemoryLease {
            lease: Some(lease),
            return_to: Some(self.clone()),
        })
    }
}

impl MemoryLease {
    pub(super) fn take_buffer(&mut self) -> Vec<u8> {
        self.lease
            .as_mut()
            .expect("owned outcome slot")
            .buffer
            .take()
            .expect("outcome buffer has one owner")
    }

    pub(super) fn restore_buffer(&mut self, mut buffer: Vec<u8>) {
        buffer.clear();
        // OriginalOutcome's bounded writer must not grow past its reservation.
        assert!(buffer.capacity() <= PAYLOAD_BYTES);
        self.lease.as_mut().expect("owned outcome slot").buffer = Some(buffer);
    }
}

impl Drop for MemoryLease {
    fn drop(&mut self) {
        if let Some(slot) = &self.return_to {
            // The caller returns the original's sole backing buffer before
            // settlement releases this lease. Never manufacture another slot.
            if self
                .lease
                .as_ref()
                .is_some_and(|lease| lease.buffer.is_some())
            {
                *slot.0.lock() = self.lease.take();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_capacity_is_atomic_and_independent_of_safety() {
        let memory = Arc::new(OutcomeMemory::default());
        let barrier = Arc::new(std::sync::Barrier::new(64));
        let threads: Vec<_> = (0..64)
            .map(|_| {
                let memory = memory.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    memory.ordinary().ok()
                })
            })
            .collect();
        let leases: Vec<_> = threads
            .into_iter()
            .filter_map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(leases.len(), SLOTS);
        assert!(memory.ordinary().is_err());
        let safety: Vec<_> = (0..SLOTS).map(|_| memory.safety().unwrap()).collect();
        assert!(memory.safety().is_err());
        drop(leases);
        assert!(memory.ordinary().is_ok());
        drop(safety);
        assert!(memory.safety().is_ok());
    }

    #[test]
    fn disconnected_rollback_keeps_capacity_and_rejects_session_churn() {
        let memory = OutcomeMemory::default();
        let mut sessions: Vec<_> = (0..SLOTS).map(|_| memory.safety().unwrap()).collect();
        let session = sessions.pop().unwrap();
        let pending = session.claim().unwrap();
        assert!(session.claim().is_err());
        drop(session);
        assert!(
            memory.safety().is_err(),
            "disconnect cannot free a pending original's safety slot"
        );
        drop(pending);
        assert!(memory.safety().is_ok());
    }

    #[test]
    fn settlement_returns_the_same_buffer_without_allocating_a_new_safety_slot() {
        let memory = OutcomeMemory::default();
        let session = memory.safety().unwrap();
        let mut pending = session.claim().unwrap();
        let mut buffer = pending.take_buffer();
        let pointer = buffer.as_ptr();
        buffer.extend_from_slice(b"synthetic captured bytes");
        pending.restore_buffer(buffer);
        drop(pending);
        let mut next = session.claim().unwrap();
        let buffer = next.take_buffer();
        assert_eq!(buffer.as_ptr(), pointer);
        assert!(buffer.is_empty());
        next.restore_buffer(buffer);
    }
}
