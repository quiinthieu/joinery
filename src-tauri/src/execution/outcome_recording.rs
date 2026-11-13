//! One bounded Studio-owned cell survives caller, task and session lifetime.
use super::outcome_memory::MemoryLease;
use super::*;
use crate::persistence::{
    BoundaryEvidence, OperationGate, OriginalOutcome, OutcomeDescriptor, OutcomeKind, OutcomePool,
    OutcomeReservation, VolatileRollbackToken,
};

pub(super) struct OutcomeCell {
    pub(super) descriptor: OutcomeDescriptor,
    state: Mutex<CellState>,
}

struct CellState {
    memory: Option<MemoryLease>,
    gate: Option<OperationGate>,
    reservation: Option<OutcomeReservation>,
    volatile: Option<VolatileRollbackToken>,
    original: Option<OriginalOutcome>,
    encoding_failed: bool,
    inflight: bool,
    captured: bool,
    settled: bool,
    no_original: bool,
}

impl OutcomeCell {
    #[cfg(test)]
    pub(super) fn ticket(&self) -> crate::persistence::HistoryTicket {
        self.state
            .lock()
            .reservation
            .as_ref()
            .and_then(|r| r.history_ticket())
            .expect("history operation ticket")
            .clone()
    }

    #[cfg(test)]
    pub(super) fn original_bytes(&self) -> Vec<u8> {
        self.state
            .lock()
            .original
            .as_ref()
            .expect("retained original")
            .as_bytes()
            .to_vec()
    }

    #[cfg(test)]
    pub(super) fn is_captured(&self) -> bool {
        self.state.lock().captured
    }

    pub(super) fn is_settled(&self) -> bool {
        let state = self.state.lock();
        state.settled && state.memory.is_none()
    }
}

impl Studio {
    pub(super) fn rollback_memory(&self, session: &Session) -> AppResult<MemoryLease> {
        let mut safety = session.rollback_memory.lock();
        // Test fixtures can create a Session without authenticating. Production
        // sessions reserve this before their authentication future is polled.
        if safety.is_none() {
            *safety = Some(self.outcome_memory.safety()?);
        }
        safety
            .as_ref()
            .expect("reserved session safety slot")
            .claim()
    }

    pub(super) fn admit_outcome(
        &self,
        memory: MemoryLease,
        seed: Option<&HistoryEntry>,
        descriptor: OutcomeDescriptor,
        pool: OutcomePool,
    ) -> AppResult<Arc<OutcomeCell>> {
        let gate = self.persistence.operation_gate(&self.history_owner)?;
        let (reservation, volatile) = match self.persistence.reserve_owned_operation(
            &self.history_owner,
            &gate,
            seed,
            &descriptor,
            pool,
        ) {
            Ok(reservation) => (Some(reservation), None),
            Err(error)
                if matches!(
                    descriptor.kind,
                    OutcomeKind::Boundary {
                        action: ExplicitTransactionAction::Rollback
                    }
                ) =>
            {
                let token = self.persistence.make_volatile_rollback_token(
                    &self.history_owner,
                    &gate,
                    &descriptor,
                )?;
                let _ = error;
                (None, Some(token))
            }
            Err(error) => return Err(error),
        };
        let generic = matches!(descriptor.kind, OutcomeKind::Execute { .. });
        let id = descriptor.operation_id.clone();
        let cell = Arc::new(OutcomeCell {
            descriptor,
            state: Mutex::new(CellState {
                memory: Some(memory),
                gate: Some(gate),
                reservation,
                volatile,
                original: None,
                encoding_failed: false,
                inflight: true,
                captured: false,
                settled: false,
                no_original: false,
            }),
        });
        if generic {
            self.pending_history.lock().insert(id, cell.clone());
        } else {
            self.pending_snowflake_receipts
                .lock()
                .insert(id, cell.clone());
        }
        Ok(cell)
    }

    pub(super) fn finish_outcome_history(
        &self,
        cell: &Arc<OutcomeCell>,
        completion: &HistoryCompletion,
    ) -> AppResult<()> {
        self.encode_outcome(cell, |descriptor, buffer| {
            OriginalOutcome::history_in(descriptor, completion, now_ms(), buffer)
        })?;
        let result = self.flush_outcome(cell);
        self.forget_settled_outcome(cell);
        result
    }

    pub(super) fn finish_outcome_boundary(
        &self,
        cell: &Arc<OutcomeCell>,
        evidence: &BoundaryEvidence,
    ) -> AppResult<()> {
        self.encode_outcome(cell, |descriptor, buffer| {
            OriginalOutcome::boundary_in(descriptor, evidence, now_ms(), buffer)
        })?;
        let result = self.flush_outcome(cell);
        self.forget_settled_outcome(cell);
        result
    }

    fn encode_outcome(
        &self,
        cell: &OutcomeCell,
        encode: impl FnOnce(&OutcomeDescriptor, Vec<u8>) -> AppResult<OriginalOutcome>,
    ) -> AppResult<()> {
        let _encoding = self.outcome_memory.encoding.lock();
        let mut state = cell.state.lock();
        if state.settled || state.original.is_some() {
            return Ok(());
        }
        if state.encoding_failed {
            return Err(recording_error(false));
        }
        state.inflight = false;
        // A panic while the encoder owns the buffer must never cause the
        // operation guard to encode a second invented result during unwinding.
        state.encoding_failed = true;
        let buffer = state
            .memory
            .as_mut()
            .expect("unsettled memory slot")
            .take_buffer();
        match encode(&cell.descriptor, buffer) {
            Ok(original) => {
                state.original = Some(original);
                state.encoding_failed = false;
            }
            Err(_) => {
                // A malformed producer response cannot become an invented
                // interruption. Keep ownership and refuse ordinary admission.
                return Err(recording_error(false));
            }
        }
        Ok(())
    }

    pub(super) fn interrupt_outcome(&self, cell: &Arc<OutcomeCell>) {
        {
            let mut state = cell.state.lock();
            if state.original.is_some() || state.encoding_failed || state.settled {
                return;
            }
            // The absence of a result is not an acknowledged remote outcome.
            // A negative witness also covers pre-submission adapter rejection.
            state.inflight = false;
            state.no_original = true;
        }
        let _ = self.flush_outcome(cell);
        self.forget_settled_outcome(cell);
    }

    fn forget_settled_outcome(&self, cell: &OutcomeCell) {
        if cell.is_settled() {
            let map = if matches!(cell.descriptor.kind, OutcomeKind::Execute { .. }) {
                &self.pending_history
            } else {
                &self.pending_snowflake_receipts
            };
            map.lock().remove(&cell.descriptor.operation_id);
        }
    }

    pub(super) fn flush_outcome(&self, cell: &OutcomeCell) -> AppResult<()> {
        let mut state = cell.state.lock();
        if state.memory.is_none() {
            return Ok(());
        }
        if state.original.is_none() && !state.no_original {
            return Err(recording_error(false));
        }
        let gate = state.gate.as_ref().expect("unsettled OS ownership");
        if state.reservation.is_none() {
            let reservation = self
                .persistence
                .promote_volatile_rollback(
                    &self.history_owner,
                    gate,
                    state.volatile.as_ref().expect("volatile rollback token"),
                )
                .map_err(|_| recording_error(false))?;
            state.reservation = Some(reservation);
            state.volatile.take();
        }
        if state.no_original {
            self.persistence
                .abandon_reserved_operation(
                    &self.history_owner,
                    state.gate.as_ref().unwrap(),
                    state.reservation.as_ref().unwrap(),
                )
                .map_err(|_| recording_error(false))?;
            state.settled = true;
            state.memory.take();
            state.gate.take();
            state.volatile.take();
            return Ok(());
        }
        if !state.settled {
            self.persistence
                .capture_original_outcome(
                    &self.history_owner,
                    state.gate.as_ref().unwrap(),
                    state.reservation.as_ref().unwrap(),
                    state.original.as_ref().unwrap(),
                )
                .map_err(|_| recording_error(false))?;
            state.captured = true;
            self.persistence
                .settle_original_outcome(
                    &self.history_owner,
                    state.gate.as_ref().unwrap(),
                    state.reservation.as_ref().unwrap(),
                    state.original.as_ref().unwrap(),
                )
                .map_err(|_| recording_error(true))?;
            state.settled = true;
        }
        let original = state.original.take().unwrap();
        match original.try_reclaim_buffer() {
            Ok(buffer) => {
                state.memory.as_mut().unwrap().restore_buffer(buffer);
                state.memory.take();
                state.gate.take();
                state.volatile.take();
                Ok(())
            }
            Err(original) => {
                state.original = Some(original);
                Err(recording_error(true))
            }
        }
    }

    pub(super) fn flush_outcome_map(
        &self,
        map: &Mutex<HashMap<String, Arc<OutcomeCell>>>,
    ) -> AppResult<()> {
        let mut pending = map.lock();
        let mut all_durable = true;
        let mut recording_pending = false;
        for cell in pending.values() {
            if cell.state.lock().inflight {
                continue;
            }
            let _ = self.flush_outcome(cell);
            let state = cell.state.lock();
            if state.memory.is_some() {
                recording_pending = true;
                all_durable &= state.captured;
            }
        }
        pending.retain(|_, cell| !cell.is_settled());
        if !recording_pending {
            Ok(())
        } else {
            Err(recording_error(all_durable))
        }
    }
}

fn recording_error(durable: bool) -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        if durable {
            "The original outcome was captured locally, but its local history or audit is still pending; keep Joinery open until recording settles"
        } else {
            "The original outcome is not durably recorded; it may be lost if Joinery exits. Keep Joinery open until local history or audit recording settles"
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn final_task_drain_refuses_an_unexpected_inflight_cell() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let cell = studio
            .admit_outcome(
                studio.outcome_memory.ordinary().unwrap(),
                None,
                OutcomeDescriptor {
                    operation_id: new_id(),
                    connection_id: new_id(),
                    session_id: new_id(),
                    started_at: now_ms(),
                    digest: "c".repeat(64),
                    kind: OutcomeKind::Boundary {
                        action: ExplicitTransactionAction::Begin,
                    },
                },
                OutcomePool::Ordinary,
            )
            .unwrap();
        let error = studio.shutdown_and_wait().await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(cell.state.lock().inflight);
        assert!(cell.state.lock().original.is_none());
        assert!(!cell.is_captured());
        studio.interrupt_outcome(&cell);
        studio.shutdown_and_wait().await.unwrap();
        assert!(
            studio
                .audit_log()
                .unwrap()
                .iter()
                .all(|a| a.outcome == "started")
        );
    }

    #[test]
    fn encoding_panic_retains_ownership_without_double_panic_or_invented_outcome() {
        let root = tempfile::tempdir().unwrap();
        let studio = Studio::with_secrets(
            root.path(),
            Arc::new(crate::secrets::MemorySecrets::default()),
        )
        .unwrap();
        let seed = HistoryEntry {
            id: new_id(),
            connection_id: new_id(),
            connection_name: "Encoder panic fixture".into(),
            started_at: now_ms(),
            sql: Some("SELECT 'never redispatch'".into()),
            statement_count: 1,
            provenance: HistoryProvenance::Admitted,
            status: None,
            elapsed_ms: None,
            execution_summary: None,
            snowflake_dml: None,
            snowflake_ddl: None,
        };
        let cell = studio
            .admit_outcome(
                studio.outcome_memory.ordinary().unwrap(),
                Some(&seed),
                OutcomeDescriptor {
                    operation_id: seed.id.clone(),
                    connection_id: seed.connection_id.clone(),
                    session_id: new_id(),
                    started_at: seed.started_at,
                    digest: "a".repeat(64),
                    kind: OutcomeKind::Execute { planned: 1 },
                },
                OutcomePool::Ordinary,
            )
            .unwrap();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _owner = history_receipts::AdmissionOwner::new(&studio, cell.clone());
            let _ = studio.encode_outcome(&cell, |_, _buffer| panic!("synthetic encoder panic"));
        }));
        assert!(panicked.is_err());
        assert!(cell.state.lock().encoding_failed);
        assert!(!cell.is_captured());
        assert!(cell.state.lock().original.is_none());
        assert!(cell.state.lock().memory.is_some());
        assert!(studio.flush_pending_history().is_err());
        assert_eq!(
            studio.history_entry(&seed.id).unwrap().unwrap().provenance,
            HistoryProvenance::Admitted
        );
        assert!(
            studio
                .persistence
                .audit_log()
                .unwrap()
                .iter()
                .all(|a| a.id != format!("{}:finished", seed.id))
        );
        let remaining: Vec<_> = (0..31)
            .map(|_| studio.outcome_memory.ordinary().unwrap())
            .collect();
        assert!(studio.outcome_memory.ordinary().is_err());
        let gate = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.path().join("history-admission.lock"))
            .unwrap();
        assert!(matches!(
            gate.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(remaining);
    }
}
