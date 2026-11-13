//! Atomic console history/audit transitions; original evidence stays SQL-free.
use super::*;
use crate::domain::{AuditEntry, HistoryEntry, HistoryProvenance, RunStatus};
use crate::persistence::history::lifecycle as history;
use crate::persistence::{HistoryCompletion, HistoryDisposition};

#[derive(Clone, Debug)]
pub(crate) struct ConsoleLifecycleSnapshot {
    pub run: ConsoleRunSnapshot,
    pub history: HistoryDisposition,
    pub completion: Option<HistoryCompletion>,
    pub publisher_sealed: bool,
    pub legacy: bool,
}

fn audit(
    connection: &Connection,
    run: &ConsoleRunSnapshot,
    finished: bool,
    completion: Option<&HistoryCompletion>,
) -> AppResult<()> {
    let entry = AuditEntry {
        id: format!(
            "{}:{}",
            run.metadata.run_id,
            if finished { "finished" } else { "started" }
        ),
        occurred_at: if finished {
            run.terminal.as_ref().ok_or_else(invalid)?.finished_at
        } else {
            run.metadata.created_at
        },
        connection_id: run.metadata.connection_id.clone(),
        action: "snowflake-console".into(),
        digest: run.metadata.plan_digest.clone(),
        outcome: if finished {
            completion
                .and_then(|c| c.status)
                .map(|s| format!("{s:?}"))
                .unwrap_or_else(|| "CompletionUnknown".into())
        } else {
            "started".into()
        },
    };
    connection.execute("INSERT INTO audit_v5(id,occurred_at,connection_id,action,digest,outcome) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO NOTHING",params![entry.id,sqlite_integer(entry.occurred_at)?,entry.connection_id,entry.action,entry.digest,entry.outcome]).map_err(db_error)?;
    check_audit(connection, &entry)
}

fn check_audit(connection: &Connection, entry: &AuditEntry) -> AppResult<()> {
    let value: Option<(i64, String, String, String, String)> = connection
        .query_row(
            "SELECT occurred_at,connection_id,action,digest,outcome FROM audit_v5 WHERE id=?1",
            [&entry.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(db_error)?;
    if value
        != Some((
            sqlite_integer(entry.occurred_at)?,
            entry.connection_id.clone(),
            entry.action.clone(),
            entry.digest.clone(),
            entry.outcome.clone(),
        ))
    {
        return Err(conflict());
    }
    Ok(())
}

pub(super) fn read(
    connection: &Connection,
    id: &str,
) -> AppResult<Option<ConsoleLifecycleSnapshot>> {
    let Some(run) = read_run(connection, id)? else {
        return Ok(None);
    };
    let (ticket, legacy, sealed): (Option<String>, bool, bool) = connection
        .query_row(
            "SELECT ticket_id,legacy,publisher_sealed FROM console_lifecycle_v5 WHERE run_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(db_error)?;
    let (state, completion) = if let Some(id) = ticket {
        if legacy {
            return Err(invalid());
        }
        if id != run.metadata.run_id {
            return Err(invalid());
        }
        let (_, state, completion) = history::lookup(connection, &id)?.ok_or_else(invalid)?;
        let (_, seed, _, _, _, _, _, _) =
            history::read_ticket(connection, &id)?.ok_or_else(invalid)?;
        if seed.id != run.metadata.run_id
            || seed.connection_id != run.metadata.connection_id
            || seed.started_at != run.metadata.created_at
            || seed.statement_count != run.metadata.step_count as usize
        {
            return Err(invalid());
        }
        // Pure readback verifies both atomic audit obligations. Never repairs.
        let started = AuditEntry {
            id: format!("{}:started", run.metadata.run_id),
            occurred_at: run.metadata.created_at,
            connection_id: run.metadata.connection_id.clone(),
            action: "snowflake-console".into(),
            digest: run.metadata.plan_digest.clone(),
            outcome: "started".into(),
        };
        check_audit(connection, &started)?;
        if let Some(c) = &completion {
            let terminal = run.terminal.as_ref().ok_or_else(invalid)?;
            let expected = match terminal.reason {
                ConsoleStopReason::Completed => RunStatus::Completed,
                ConsoleStopReason::Cancelled => RunStatus::Cancelled,
                ConsoleStopReason::OutcomeUnknown | ConsoleStopReason::Interrupted => {
                    RunStatus::OutcomeUnknown
                }
                ConsoleStopReason::ResultLimit => RunStatus::Stopped,
                _ => RunStatus::Failed,
            };
            if c.provenance == HistoryProvenance::Recorded {
                if c.status != Some(expected) {
                    return Err(invalid());
                }
            } else if !matches!(
                c.provenance,
                HistoryProvenance::Interrupted | HistoryProvenance::RestoreRecovered
            ) || c.status.is_some()
                || c.elapsed_ms.is_some()
                || terminal.reason != ConsoleStopReason::Interrupted
            {
                return Err(invalid());
            }
            let finished = AuditEntry {
                id: format!("{}:finished", run.metadata.run_id),
                occurred_at: run.terminal.as_ref().ok_or_else(invalid)?.finished_at,
                connection_id: run.metadata.connection_id.clone(),
                action: "snowflake-console".into(),
                digest: run.metadata.plan_digest.clone(),
                outcome: c
                    .status
                    .map(|s| format!("{s:?}"))
                    .unwrap_or_else(|| "CompletionUnknown".into()),
            };
            check_audit(connection, &finished)?;
        } else if run.terminal.is_some() {
            return Err(invalid());
        }
        (state, completion)
    } else {
        if !legacy {
            return Err(invalid());
        }
        (HistoryDisposition::Suppressed, None)
    };
    if sealed && (run.terminal.is_none() || (!legacy && completion.is_none())) {
        return Err(invalid());
    }
    Ok(Some(ConsoleLifecycleSnapshot {
        run,
        history: state,
        completion,
        publisher_sealed: sealed,
        legacy,
    }))
}

pub(super) fn sealed(connection: &Connection, id: &str) -> AppResult<bool> {
    connection
        .query_row(
            "SELECT publisher_sealed FROM console_lifecycle_v5 WHERE run_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)
        .map(|v| v.unwrap_or(false))
}

impl Persistence {
    pub(crate) fn console_prepare_lifecycle(
        &self,
        owner: &ConsoleJournalOwner,
        metadata: ConsoleRunMetadata,
        seed: &HistoryEntry,
    ) -> AppResult<ConsoleLifecycleSnapshot> {
        owner.validate(self)?;
        validate_metadata(&metadata)?;
        if seed.id != metadata.run_id
            || seed.connection_id != metadata.connection_id
            || seed.started_at != metadata.created_at
            || seed.statement_count != metadata.step_count as usize
        {
            return Err(invalid());
        }
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        require_current_schema(&tx)?;
        if read_rejection(&tx, &metadata.client_request_id)?.is_some() {
            return Err(conflict());
        }
        let prior: Option<String> = tx
            .query_row(
                "SELECT run_id FROM snowflake_console_runs_v5 WHERE client_request_id=?1",
                [&metadata.client_request_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(id) = prior {
            let result = read(&tx, &id)?.ok_or_else(invalid)?;
            if result.run.metadata != metadata || result.legacy {
                return Err(conflict());
            }
            history::begin(&tx, seed)?;
            tx.commit().map_err(db_error)?;
            return Ok(result);
        }
        let count: i64 = tx
            .query_row("SELECT count(*) FROM snowflake_console_runs_v5", [], |r| {
                r.get(0)
            })
            .map_err(db_error)?;
        if count >= i64::from(MAX_RUNS) {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "The reviewed console journal capacity is exhausted; existing evidence has been preserved",
            ));
        }
        let run = ConsoleRunSnapshot {
            metadata,
            revision: 0,
            state: ConsoleRunState::Prepared,
            observations: ConsoleObservations::default(),
            steps: vec![],
            terminal: None,
        };
        insert_snapshot(&tx, &run)?;
        history::begin(&tx, seed)?;
        tx.execute(
            "INSERT INTO console_lifecycle_v5 VALUES(?1,?1,0,0)",
            [&run.metadata.run_id],
        )
        .map_err(db_error)?;
        audit(&tx, &run, false, None)?;
        let result = read(&tx, &run.metadata.run_id)?.ok_or_else(invalid)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }

    pub(crate) fn console_complete_lifecycle(
        &self,
        owner: &ConsoleJournalOwner,
        id: &str,
        revision: u64,
        reason: ConsoleStopReason,
        finished_at: u64,
        completion: &HistoryCompletion,
    ) -> AppResult<ConsoleLifecycleSnapshot> {
        owner.validate(self)?;
        if !valid_time(finished_at) || reason == ConsoleStopReason::Interrupted {
            return Err(invalid());
        }
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        require_current_schema(&tx)?;
        let old = read(&tx, id)?.ok_or_else(invalid)?;
        if let Some(prior) = &old.completion {
            let t = old.run.terminal.as_ref().ok_or_else(invalid)?;
            if t.reason != reason || t.finished_at != finished_at || !prior.matches(completion) {
                return Err(conflict());
            }
            return Ok(old);
        }
        if old.legacy
            || old.publisher_sealed
            || old.run.revision != revision
            || revision >= MAX_REVISION
        {
            return Err(conflict());
        }
        let expected = match reason {
            ConsoleStopReason::Completed => RunStatus::Completed,
            ConsoleStopReason::Cancelled => RunStatus::Cancelled,
            ConsoleStopReason::OutcomeUnknown => RunStatus::OutcomeUnknown,
            ConsoleStopReason::ResultLimit => RunStatus::Stopped,
            _ => RunStatus::Failed,
        };
        if completion.provenance != HistoryProvenance::Recorded
            || completion.status != Some(expected)
        {
            return Err(invalid());
        }
        let mut run = old.run;
        validate_finish(&run, reason, &run.observations)?;
        run.terminal = Some(ConsoleTerminal {
            reason,
            finished_at,
            accounting: accounting(&run),
            observations: run.observations.clone(),
        });
        run.state = ConsoleRunState::Terminal;
        run.revision += 1;
        validate_snapshot(&run)?;
        write_run(&tx, &run, revision)?;
        let (ticket, _, _) = history::lookup(&tx, id)?.ok_or_else(invalid)?;
        history::finish(&tx, &ticket, completion)?;
        audit(&tx, &run, true, Some(completion))?;
        let result = read(&tx, id)?.ok_or_else(invalid)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }

    pub(crate) fn console_seal_publishers(
        &self,
        owner: &ConsoleJournalOwner,
        id: &str,
        revision: u64,
    ) -> AppResult<ConsoleLifecycleSnapshot> {
        owner.validate(self)?;
        let mut c = self.connection.lock();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        require_current_schema(&tx)?;
        let old = read(&tx, id)?.ok_or_else(invalid)?;
        if old.publisher_sealed {
            return Ok(old);
        }
        if old.run.revision != revision
            || old.run.terminal.is_none()
            || (!old.legacy
                && (old.completion.is_none() || old.history == HistoryDisposition::Pending))
        {
            return Err(conflict());
        }
        tx.execute("UPDATE console_lifecycle_v5 SET publisher_sealed=1 WHERE run_id=?1 AND publisher_sealed=0",[id]).map_err(db_error)?;
        let result = read(&tx, id)?.ok_or_else(invalid)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }

    pub(crate) fn console_lifecycle(
        &self,
        id: &str,
    ) -> AppResult<Option<super::ConsoleLifecycleSnapshot>> {
        if !valid_uuid(id) {
            return Err(invalid());
        }
        let mut c = self.connection.lock();
        let tx = c.transaction().map_err(db_error)?;
        require_current_schema(&tx)?;
        let value = read(&tx, id)?;
        tx.commit().map_err(db_error)?;
        Ok(value)
    }
}

/// Exclusive owner acquisition ends orphan publication authority, not evidence
/// of delivery. Existing originals and measured completions remain unchanged.
pub(super) fn recover(connection: &Connection, provenance: HistoryProvenance) -> AppResult<()> {
    let mut cursor=connection.prepare("SELECT run_id,ticket_id,legacy FROM console_lifecycle_v5 WHERE publisher_sealed=0 ORDER BY run_id").map_err(db_error)?;
    let mut rows = cursor.query([]).map_err(db_error)?;
    while let Some(row) = rows.next().map_err(db_error)? {
        let id: String = row.get(0).map_err(db_error)?;
        let ticket: Option<String> = row.get(1).map_err(db_error)?;
        let legacy: bool = row.get(2).map_err(db_error)?;
        let run = read_run(connection, &id)?.ok_or_else(invalid)?;
        if run.terminal.is_none() {
            return Err(invalid());
        }
        if let Some(ticket) = ticket {
            let (ticket, _, completion) =
                history::lookup(connection, &ticket)?.ok_or_else(invalid)?;
            if completion.is_none() {
                let completion = HistoryCompletion {
                    provenance,
                    status: None,
                    elapsed_ms: None,
                    execution_summary: None,
                    snowflake_dml: None,
                    snowflake_ddl: None,
                };
                history::finish(connection, &ticket, &completion)?;
                audit(connection, &run, true, Some(&completion))?;
            }
        } else if !legacy {
            return Err(invalid());
        }
        connection
            .execute(
                "UPDATE console_lifecycle_v5 SET publisher_sealed=1 WHERE run_id=?1",
                [id],
            )
            .map_err(db_error)?;
    }
    Ok(())
}
