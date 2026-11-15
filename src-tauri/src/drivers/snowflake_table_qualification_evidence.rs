//! Typed completion checks only. These inputs must come from the owning Studio
//! operation; accepting their shape does not prove their route, admission, order
//! in the whole plan, metadata lifecycle, or durable recording. No IO is done.

use super::{
    DATABASE, Operation, OwnershipMarker, Phase, Plan, ReadResult, SCHEMA, SchemaCensus,
    StatementId, invalid,
};
use crate::{
    domain::{
        AppResult, EngineKind, QueryEvent, ResultInfo, RetentionStatus, RunStatus,
        SessionContextOptions, SessionInfo, SessionRuntimeState, SnowflakeDdlKind,
        SnowflakeDdlPreflightResponse, SnowflakeDdlResult, SnowflakeDmlResult, TextRow,
        TransactionControlResult,
    },
    relation_inspection::{BrowseCatalog, RelationInspection},
};
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub(super) enum PhaseEvidence<'a> {
    Startup(&'a SessionInfo),
    Preflight(&'a SnowflakeDdlPreflightResponse),
    Ddl(&'a SnowflakeDdlResult),
    Reconcile {
        options: &'a SessionContextOptions,
        catalog: &'a BrowseCatalog,
    },
    Catalog(&'a BrowseCatalog),
    Inspect(&'a RelationInspection),
    Read {
        events: &'a [QueryEvent],
        result: &'a ResultInfo,
        rows: &'a [TextRow],
    },
    Boundary(&'a TransactionControlResult),
    Dml(&'a SnowflakeDmlResult),
    Options(&'a SessionContextOptions),
}

impl PhaseEvidence<'_> {
    /// Preserve original typed inputs, including the real initial/final Result
    /// events and original decimal completion count. This is not a receipt.
    pub(super) fn record(&self) -> AppResult<Value> {
        serde_json::to_value(self).map_err(|_| invalid())
    }
}

pub(super) struct ValidatedEvidence<'a> {
    evidence: PhaseEvidence<'a>,
    state: Option<&'a SessionRuntimeState>,
    read: Option<ReadResult<'a>>,
    ownership: Option<OwnershipMarker>,
}

impl<'a> ValidatedEvidence<'a> {
    pub(super) fn state(&self) -> Option<&'a SessionRuntimeState> {
        self.state
    }

    pub(super) fn read(&self) -> Option<&ReadResult<'a>> {
        self.read.as_ref()
    }

    pub(super) fn ownership(&self) -> Option<&OwnershipMarker> {
        self.ownership.as_ref()
    }

    pub(super) fn record(&self) -> AppResult<Value> {
        self.evidence.record()
    }
}

pub(super) fn validate<'a>(
    plan: &Plan,
    phase: &Phase,
    evidence: PhaseEvidence<'a>,
    role: &str,
    contextfp: &str,
    sessionid: &str,
    last_runtime: Option<&SessionRuntimeState>,
) -> AppResult<ValidatedEvidence<'a>> {
    // Bootstrap has not yet supplied a full LiveContext/runtime observation.
    // Its SessionInfo still binds the actual session; all later phases require
    // the role and context fingerprint pinned by the owning metadata policy.
    let startup = phase.number() == 1 && phase.operation() == Operation::StartupIdentityAndVersion;
    if sessionid.is_empty() || (!startup && (role.is_empty() || contextfp.is_empty())) {
        return Err(invalid());
    }
    // The phase may be a clone, but every field must still be the closed plan's.
    let canonical = plan
        .phases()
        .get(
            usize::from(phase.number())
                .checked_sub(1)
                .ok_or_else(invalid)?,
        )
        .ok_or_else(invalid)?;
    if serde_json::to_value(canonical).map_err(|_| invalid())?
        != serde_json::to_value(phase).map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    let mut validated = ValidatedEvidence {
        evidence,
        state: None,
        read: None,
        ownership: None,
    };
    match (phase.operation(), evidence) {
        (Operation::StartupIdentityAndVersion, PhaseEvidence::Startup(info)) => {
            if info.engine != EngineKind::Snowflake
                || !info.encrypted
                || info.id != sessionid
                || info.server_version.trim().is_empty()
            {
                return Err(invalid());
            }
        }
        (Operation::DdlPreflight(id), PhaseEvidence::Preflight(result)) => {
            let kind = match id {
                StatementId::Create => SnowflakeDdlKind::CreateTable,
                StatementId::Add => SnowflakeDdlKind::AddColumn,
                StatementId::Drop => SnowflakeDdlKind::DropTable,
                _ => return Err(invalid()),
            };
            if result.revision.is_empty()
                || result.kind != kind
                || result.database != DATABASE
                || result.schema != SCHEMA
                || result.name != plan.table()
                || result.qualified_target != plan.qualified_target()
                || result.sql != plan.statement(id).source_sql()
            {
                return Err(invalid());
            }
        }
        (Operation::ReviewedDdl(_), PhaseEvidence::Ddl(result)) => {
            plan.validate_ddl_effect(phase, result)?;
            validated.state = Some(&result.state);
        }
        (
            Operation::CoherentReconciliation(presence),
            PhaseEvidence::Reconcile { options, catalog },
        ) => {
            validate_options(options)?;
            plan.validate_catalog_identity(presence, catalog, role, contextfp)?;
            validated.state = Some(&options.state);
        }
        (Operation::Catalog(presence), PhaseEvidence::Catalog(catalog)) => {
            plan.validate_catalog_identity(presence, catalog, role, contextfp)?;
        }
        (Operation::Inspect(shape), PhaseEvidence::Inspect(inspection)) => {
            plan.validate_inspection(shape, inspection)?;
        }
        (
            Operation::Read(readback),
            PhaseEvidence::Read {
                events,
                result,
                rows,
            },
        ) => {
            let state = last_runtime.ok_or_else(invalid)?;
            let read = validate_read(events, result, rows, state)?;
            validated.ownership = readback.validate(
                ReadResult {
                    columns: read.columns,
                    rows: read.rows,
                    rows_seen: read.rows_seen,
                    truncated: read.truncated,
                },
                role,
            )?;
            validated.read = Some(read);
            // Keep the state borrowed from the original event, not a cache or
            // manufactured DTO, even though equality to the supplied pin holds.
            validated.state = events.iter().find_map(|event| match event {
                QueryEvent::SessionState { state } => Some(state),
                _ => None,
            });
        }
        (Operation::TypedBoundary { .. }, PhaseEvidence::Boundary(result)) => {
            phase.effect().validate_boundary(result)?;
            validated.state = Some(&result.state);
        }
        (Operation::ReviewedDml(_), PhaseEvidence::Dml(result)) => {
            phase.effect().validate_dml(result)?;
            validated.state = Some(&result.state);
        }
        (Operation::SessionOptionsResume, PhaseEvidence::Options(options)) => {
            validate_options(options)?;
            validated.state = Some(&options.state);
        }
        _ => return Err(invalid()),
    }
    if let Some(state) = validated.state
        && (state.session_id != sessionid
            || state.role.as_deref() != Some(role)
            || state.context_fingerprint.as_deref() != Some(contextfp)
            || last_runtime != Some(state))
    {
        return Err(invalid());
    }
    Ok(validated)
}

fn validate_options(options: &SessionContextOptions) -> AppResult<()> {
    if options.revision.as_deref().is_none_or(str::is_empty) {
        return Err(invalid());
    }
    SchemaCensus::freeze(&options.available)?;
    Ok(())
}

fn validate_read<'a>(
    events: &'a [QueryEvent],
    result: &'a ResultInfo,
    rows: &'a [TextRow],
    last_runtime: &SessionRuntimeState,
) -> AppResult<ReadResult<'a>> {
    // Closed reads yield zero or one row. Production batching therefore needs
    // at most a tiny event stream; this bound also rejects repeated progress.
    if events.len() > 32
        || rows.len() > 1
        || !matches!(result.columns.len(), 4 | 5)
        || result.id.is_empty()
        || result.run_id.is_empty()
        || result.statement_index != 0
        || result.status != RetentionStatus::Complete
        || result.retained_rows != rows.len()
        || rows.iter().any(|row| row.len() != result.columns.len())
    {
        return Err(invalid());
    }
    let run = result.run_id.as_str();
    let mut events = events.iter();
    if !matches!(events.next(), Some(QueryEvent::Started { run_id }) if run_id == run)
        || !matches!(events.next(), Some(QueryEvent::StatementStarted { run_id, index: 0 }) if run_id == run)
    {
        return Err(invalid());
    }
    let Some(QueryEvent::Result { result: initial }) = events.next() else {
        return Err(invalid());
    };
    let mut expected_initial = result.clone();
    expected_initial.status = RetentionStatus::Streaming;
    expected_initial.retained_rows = 0;
    if initial != &expected_initial {
        return Err(invalid());
    }
    let mut retained = 0;
    let terminal = loop {
        match events.next() {
            Some(QueryEvent::Progress {
                result_id,
                retained_rows,
            }) if result_id == &result.id
                && *retained_rows >= retained
                && *retained_rows <= result.retained_rows =>
            {
                retained = *retained_rows;
            }
            Some(QueryEvent::Result { result: terminal }) => break terminal,
            _ => return Err(invalid()),
        }
    };
    if terminal != result || retained != result.retained_rows {
        return Err(invalid());
    }
    let Some(QueryEvent::StatementCompleted {
        run_id,
        index: 0,
        affected_rows,
        ..
    }) = events.next()
    else {
        return Err(invalid());
    };
    let rows_seen = affected_rows.parse::<u64>().map_err(|_| invalid())?;
    if run_id != run || affected_rows != &rows_seen.to_string() || rows_seen != rows.len() as u64 {
        return Err(invalid());
    }
    if !matches!(events.next(), Some(QueryEvent::SessionState { state }) if state == last_runtime)
        || !matches!(events.next(), Some(QueryEvent::RunMetrics { run_id, .. }) if run_id == run)
        || !matches!(events.next(), Some(QueryEvent::Finished { run_id, status: RunStatus::Completed, .. }) if run_id == run)
        || events.next().is_some()
    {
        return Err(invalid());
    }
    Ok(ReadResult {
        columns: &result.columns,
        rows,
        rows_seen,
        truncated: false,
    })
}

#[cfg(test)]
#[path = "snowflake_table_qualification_evidence_tests.rs"]
mod tests;
