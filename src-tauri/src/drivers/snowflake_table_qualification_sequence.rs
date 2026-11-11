//! In-memory request ownership only. The owning policy must validate phase
//! evidence and durably sync intent/terminal/census/phase records. No I/O here.
//! A core-safe close after failure may still be blocked by a shared attachment's
//! earlier failure check; this is not a claim that attached cleanup is admitted.
use super::{
    DATABASE, Phase, Plan, SCHEMA, SchemaCensus,
    controls::{self, ControlKind, ControlTarget},
    ledger::{PhysicalLedger, SqlEntry, SqlKind},
    wire::{self, WireIdentity},
};
use crate::{
    domain::{AppError, AppResult},
    drivers::{
        snowflake_process::{Certainty, Impact, Phase as ProcessPhase, Report, StatementIdentity},
        snowflake_qualification_transport::{
            LocalTerminal, LocalTerminalOrigin, PhysicalRequest, PhysicalTerminal,
        },
    },
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const MAX_REQUESTS: usize = 32768;
const INDEPENDENT_PREFIX: usize = 8;

fn denied() -> AppError {
    AppError::invalid("Physical qualification sequence does not match its closed owner")
}

#[derive(Debug, Clone)]
pub(crate) enum AdmissionKind {
    Connect,
    Sql(SqlEntry),
    Control(ControlKind),
}

#[derive(Debug, Clone)]
pub(crate) struct Admission {
    pub generation: String,
    pub logical_id: String,
    pub ticket: u64,
    pub wire_id: u64,
    pub operation: String,
    pub impact: Impact,
    pub target: Option<(String, Option<u64>)>,
    pub payload: Value,
    pub wire_payload: Vec<u8>,
    kind: AdmissionKind,
}

impl Admission {
    pub(crate) fn kind(&self) -> &AdmissionKind {
        &self.kind
    }
    pub(crate) fn record(&self) -> Value {
        let kind = match &self.kind {
            AdmissionKind::Connect => json!({"connect":true}),
            AdmissionKind::Sql(entry) => json!({"sql":entry}),
            AdmissionKind::Control(kind) => json!({"control":format!("{kind:?}")}),
        };
        json!({"generation":self.generation,"logicalId":self.logical_id,"ticket":self.ticket,
            "wireId":self.wire_id,"operation":self.operation,"impact":format!("{:?}",self.impact),
            "target":self.target,"envelope":self.payload,"wireBytes":self.wire_payload,"kind":kind})
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TerminalRecord {
    pub generation: String,
    pub logical_id: String,
    pub ticket: u64,
    pub wire_id: String,
    pub operation: String,
    pub impact: Impact,
    admission: Option<Admission>,
    report: Report,
    error: Option<AppError>,
}

fn statement_record(statement: &StatementIdentity) -> Value {
    json!({"sdkRequestId":statement.sdk_request_id,"queryId":statement.query_id})
}

impl TerminalRecord {
    pub(crate) fn admission(&self) -> Option<&Admission> {
        self.admission.as_ref()
    }
    pub(crate) fn report(&self) -> &Report {
        &self.report
    }
    pub(crate) fn error(&self) -> Option<&AppError> {
        self.error.as_ref()
    }
    pub(crate) fn record(&self) -> Value {
        let r = &self.report;
        json!({"generation":self.generation,"logicalId":self.logical_id,"ticket":self.ticket,
            "wireId":self.wire_id,"operation":self.operation,"impact":format!("{:?}",self.impact),
            "admission":self.admission.as_ref().map(Admission::record),"error":self.error,
            "report":{"ok":r.ok,"certainty":format!("{:?}",r.certainty),"phase":format!("{:?}",r.phase),
                "result":r.result,"errorCode":r.error_code,"retirement":r.retirement.map(|v|format!("{v:?}")),
                "statements":r.statements.iter().map(statement_record).collect::<Vec<_>>(),
                "useAcknowledged":r.use_acknowledged.as_ref().map(statement_record),
                "boundaryAcknowledged":r.boundary_acknowledged.as_ref().map(|a|json!({"action":a.action.as_str(),"statement":statement_record(&a.statement)})),
                "dmlAcknowledged":r.dml_acknowledged.as_ref().map(|a|json!({"kind":a.kind,"statement":statement_record(&a.statement)})),
                "ddlAcknowledged":r.ddl_acknowledged.as_ref().map(|a|json!({"kind":a.kind,"statement":statement_record(&a.statement)}))}})
    }
}

/// A local completion is not a helper terminal and never proves SQL execution.
/// Keep the candidate intent separate from an ID actually assigned for writing.
pub(crate) struct LocalTerminalRecord {
    terminal: TerminalRecord,
    wire_id: Option<String>,
    candidate_wire_id: Option<String>,
    target: Option<(String, Option<u64>)>,
    payload: Value,
    origin: LocalTerminalOrigin,
}

impl LocalTerminalRecord {
    pub(crate) fn admission(&self) -> Option<&Admission> {
        self.terminal.admission()
    }
    pub(crate) fn report(&self) -> &Report {
        self.terminal.report()
    }
    pub(crate) fn error(&self) -> Option<&AppError> {
        self.terminal.error()
    }
    pub(crate) fn record(&self) -> Value {
        let mut value = self.terminal.record();
        value["wireId"] = json!(self.wire_id);
        value["candidateWireId"] = json!(self.candidate_wire_id);
        value["target"] = json!(self.target);
        value["payload"] = self.payload.clone();
        value["origin"] = match self.origin {
            LocalTerminalOrigin::QueuedCancellation => json!({"kind":"queuedCancellation"}),
            LocalTerminalOrigin::QueuedClose => json!({"kind":"queuedClose"}),
            LocalTerminalOrigin::InactiveControl { admitted } => {
                json!({"kind":"inactiveControl","admitted":admitted})
            }
            LocalTerminalOrigin::Retirement => json!({"kind":"retirement"}),
        };
        value
    }
}

struct CompletedQuery {
    generation: String,
    logical_id: String,
    ticket: u64,
    wire_id: u64,
    phase: u8,
    discarded: bool,
}

struct Active {
    wire_id: u64,
    cancelled: bool,
    discarded: bool,
}

pub(crate) struct Sequencer {
    plan: Plan,
    ledger: PhysicalLedger,
    profile: Value,
    generation: Option<String>,
    identity: Option<WireIdentity>,
    census_frozen: bool,
    connected: bool,
    connect_reserved: bool,
    closed: bool,
    last_wire_id: u64,
    tickets: HashSet<u64>,
    pending: HashMap<u64, Admission>,
    completed_queries: HashMap<u64, CompletedQuery>,
    active: Option<Active>,
    index: usize,
    phase_index: usize,
    phase_failed: bool,
    suffix_end: Option<usize>,
    failure: Option<AppError>,
}

impl Sequencer {
    pub(crate) fn new(profile: Value) -> AppResult<Self> {
        let fields = [
            "account",
            "host",
            "username",
            "role",
            "warehouse",
            "database",
            "schema",
        ];
        if profile
            .as_object()
            .is_none_or(|object| object.len() != fields.len())
            || fields.iter().any(|field| {
                profile[*field]
                    .as_str()
                    .is_none_or(|s| s.is_empty() || s.len() > 1020 || s.contains('\0'))
            })
            || profile["database"].as_str() != Some(DATABASE)
            || profile["schema"].as_str() != Some(SCHEMA)
        {
            return Err(denied());
        }
        let plan = Plan::generate()?;
        let ledger = plan.expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()])?)?;
        Ok(Self {
            plan,
            ledger,
            profile,
            generation: None,
            identity: None,
            census_frozen: false,
            connected: false,
            connect_reserved: false,
            closed: false,
            last_wire_id: 0,
            tickets: HashSet::new(),
            pending: HashMap::new(),
            completed_queries: HashMap::new(),
            active: None,
            index: 0,
            phase_index: 0,
            phase_failed: false,
            suffix_end: None,
            failure: None,
        })
    }
    pub(crate) fn ledger(&self) -> &PhysicalLedger {
        &self.ledger
    }
    pub(crate) fn identity(&self) -> Option<&WireIdentity> {
        self.identity.as_ref()
    }
    /// Pure predicate; the owner must latch a denial before committing any
    /// lifecycle/metadata observation made with this context.
    pub(crate) fn validate_context(
        &self,
        context: &crate::drivers::snowflake_metadata::LiveContext,
    ) -> AppResult<()> {
        self.check()?;
        let identity = self.identity.as_ref().ok_or_else(denied)?;
        if context.principal != identity.user()
            || context.session_id != identity.session()
            || context.role.as_deref() != self.profile["role"].as_str()
            || context.warehouse.as_deref() != self.profile["warehouse"].as_str()
            || context.database.as_deref() != self.profile["database"].as_str()
            || context.schema.as_deref() != self.profile["schema"].as_str()
        {
            return Err(denied());
        }
        Ok(())
    }
    pub(crate) fn phase(&self) -> Option<&Phase> {
        self.plan.phases().get(self.phase_index)
    }
    pub(crate) fn next_entry(&self) -> Option<&SqlEntry> {
        self.ledger.entries().get(self.index)
    }
    pub(crate) fn census_frozen(&self) -> bool {
        self.census_frozen
    }
    pub(crate) fn check(&self) -> AppResult<()> {
        self.failure.clone().map_or(Ok(()), Err)
    }
    pub(crate) fn fail(&mut self, error: AppError) {
        self.failure.get_or_insert(error);
    }

    pub(crate) fn reserve(&mut self, request: &PhysicalRequest<'_>) -> AppResult<Admission> {
        let result = self.reserve_inner(request);
        if let Err(error) = &result {
            self.fail(error.clone());
        }
        result
    }

    fn reserve_inner(&mut self, request: &PhysicalRequest<'_>) -> AppResult<Admission> {
        let wire_id = wire::validate_envelope(request)?;
        if self.closed
            || wire_id <= self.last_wire_id
            || self.tickets.contains(&request.ticket)
            || self.tickets.len() >= MAX_REQUESTS
            || self
                .pending
                .values()
                .any(|p| p.logical_id == request.operation_id)
            || self
                .generation
                .as_ref()
                .is_some_and(|g| g != request.generation)
        {
            return Err(denied());
        }
        let kind = match request.operation {
            "connect" => {
                self.check()?;
                if self.connect_reserved || !self.pending.is_empty() {
                    return Err(denied());
                }
                wire::validate_connect(request, &self.profile)?;
                AdmissionKind::Connect
            }
            "credit" | "cancel" | "close" => {
                let target = self
                    .active
                    .as_ref()
                    .and_then(|active| self.pending.get(&active.wire_id))
                    .map(|a| ControlTarget {
                        logical_id: &a.logical_id,
                        ticket: a.ticket,
                        wire_id: a.wire_id,
                        operation: &a.operation,
                    });
                let control = controls::validate_control(
                    request,
                    target,
                    self.connect_reserved && self.pending.is_empty() && self.active.is_none(),
                )?;
                if control != ControlKind::Close {
                    // Fatal failures only retain a no-pending close escape. A
                    // definitive primary rejection has not latched that failure.
                    self.check()?;
                    let active = self.active.as_ref().ok_or_else(denied)?;
                    if (control == ControlKind::Cancel && active.cancelled)
                        || (control == ControlKind::Discard && active.discarded)
                        || (control == ControlKind::Credit && active.discarded)
                    {
                        return Err(denied());
                    }
                }
                AdmissionKind::Control(control)
            }
            _ => {
                self.check()?;
                if !self.connected
                    || self.active.is_some()
                    || !self.pending.is_empty()
                    || (!self.census_frozen && self.index >= INDEPENDENT_PREFIX)
                    || self.suffix_end.is_some_and(|end| self.index >= end)
                {
                    return Err(denied());
                }
                let entry = self.next_entry().ok_or_else(denied)?;
                if self
                    .phase()
                    .is_none_or(|phase| phase.number() != entry.phase())
                {
                    return Err(denied());
                }
                wire::validate_sql(entry, request, self.identity.as_ref())?;
                AdmissionKind::Sql(entry.clone())
            }
        };
        let admission = Admission {
            generation: request.generation.into(),
            logical_id: request.operation_id.into(),
            ticket: request.ticket,
            wire_id,
            operation: request.operation.into(),
            impact: request.impact,
            target: request
                .target
                .map(|(logical, ticket)| (logical.into(), ticket)),
            payload: request.payload.clone(),
            wire_payload: request.wire_payload.to_vec(),
            kind,
        };
        self.generation
            .get_or_insert_with(|| request.generation.into());
        self.last_wire_id = wire_id;
        self.tickets.insert(request.ticket);
        match admission.kind() {
            AdmissionKind::Connect => {
                self.connect_reserved = true;
                self.active = Some(Active {
                    wire_id,
                    cancelled: false,
                    discarded: false,
                });
            }
            AdmissionKind::Sql(_) => {
                self.active = Some(Active {
                    wire_id,
                    cancelled: false,
                    discarded: false,
                })
            }
            AdmissionKind::Control(ControlKind::Close) => self.closed = true,
            AdmissionKind::Control(control) => {
                let active = self.active.as_mut().ok_or_else(denied)?;
                active.cancelled |= *control == ControlKind::Cancel;
                active.discarded |= *control == ControlKind::Discard;
            }
        }
        self.pending.insert(wire_id, admission.clone());
        Ok(admission)
    }

    /// Call at the first validated schema callback, possibly before its terminal.
    /// The owning policy must sync the returned ledger/census before further SQL.
    pub(crate) fn freeze_census(&mut self, names: &[String]) -> AppResult<()> {
        let result = self.freeze_census_inner(names);
        if let Err(error) = &result {
            self.fail(error.clone());
        }
        result
    }
    fn freeze_census_inner(&mut self, names: &[String]) -> AppResult<()> {
        self.check()?;
        let census = SchemaCensus::freeze(names)?;
        if self.census_frozen {
            return self.ledger.census().verify_observation(names);
        }
        let schema_reserved=self.active.as_ref().and_then(|active|self.pending.get(&active.wire_id))
            .is_some_and(|a|matches!(a.kind(),AdmissionKind::Sql(entry) if usize::from(entry.ordinal())==INDEPENDENT_PREFIX));
        if self.phase().is_none_or(|p| p.number() != 2)
            || !(self.index == INDEPENDENT_PREFIX
                || (self.index + 1 == INDEPENDENT_PREFIX && schema_reserved))
        {
            return Err(denied());
        }
        let expanded = self.plan.expand_physical_ledger(&census)?;
        let before = serde_json::to_vec(&self.ledger.entries()[..INDEPENDENT_PREFIX])
            .map_err(|_| denied())?;
        let after =
            serde_json::to_vec(&expanded.entries()[..INDEPENDENT_PREFIX]).map_err(|_| denied())?;
        if before != after {
            return Err(denied());
        }
        self.ledger = expanded;
        self.census_frozen = true;
        Ok(())
    }

    /// Internal readiness only; it does not validate database/phase evidence.
    pub(crate) fn phase_ready(&self) -> AppResult<()> {
        self.check()?;
        let phase = self.phase().ok_or_else(denied)?;
        if !self.connected
            || !self.pending.is_empty()
            || self.active.is_some()
            || self.phase_failed
            || self
                .next_entry()
                .is_some_and(|entry| entry.phase() == phase.number())
            || (phase.number() >= 2 && !self.census_frozen)
        {
            return Err(denied());
        }
        Ok(())
    }
    /// INTERNAL: the owner must validate all evidence and sync its phase receipt
    /// before calling. This method is not a qualification-success assertion.
    pub(crate) fn advance_phase(&mut self) -> AppResult<()> {
        self.phase_ready()?;
        self.phase_index += 1;
        Ok(())
    }

    /// Always returns the original report, even for a bad key or latched denial.
    /// The caller must enqueue this record before propagating its error.
    pub(crate) fn terminal(&mut self, terminal: PhysicalTerminal<'_>) -> TerminalRecord {
        let wire_id = terminal
            .wire_id
            .parse::<u64>()
            .ok()
            .filter(|id| *id != 0 && id.to_string() == terminal.wire_id);
        let admission = wire_id
            .and_then(|id| self.pending.get(&id))
            .filter(|a| {
                a.generation == terminal.generation
                    && a.logical_id == terminal.operation_id
                    && a.ticket == terminal.ticket
                    && a.operation == terminal.operation
                    && a.impact == terminal.impact
            })
            .cloned();
        let result = if let Some(admission) = &admission {
            self.pending.remove(&admission.wire_id);
            let discarded = self
                .active
                .as_ref()
                .is_some_and(|active| active.wire_id == admission.wire_id && active.discarded);
            if self
                .active
                .as_ref()
                .is_some_and(|active| active.wire_id == admission.wire_id)
            {
                self.active = None;
            }
            self.check()
                .and_then(|()| self.complete(admission, terminal.report, discarded))
        } else {
            Err(denied())
        };
        if let Err(error) = result {
            self.fail(error);
        }
        TerminalRecord {
            generation: terminal.generation.into(),
            logical_id: terminal.operation_id.into(),
            ticket: terminal.ticket,
            wire_id: terminal.wire_id.into(),
            operation: terminal.operation.into(),
            impact: terminal.impact,
            admission,
            report: terminal.report.clone(),
            error: self.failure.clone(),
        }
    }

    /// Preserve every original local report, including failed/unmatched ones.
    /// Exact matching reservations are consumed for cleanup even after failure;
    /// the only nonfatal result is an unwritten, already-inactive closed credit.
    pub(crate) fn local_terminal(&mut self, terminal: LocalTerminal<'_>) -> LocalTerminalRecord {
        let admission = self
            .pending
            .values()
            .find(|admission| {
                admission.generation == terminal.generation
                    && admission.logical_id == terminal.operation_id
                    && admission.ticket == terminal.ticket
                    && admission.operation == terminal.operation
                    && admission.impact == terminal.impact
            })
            .cloned();
        if let Some(admission) = &admission {
            self.pending.remove(&admission.wire_id);
            if self
                .active
                .as_ref()
                .is_some_and(|active| active.wire_id == admission.wire_id)
            {
                self.active = None;
            }
        }
        if let Err(error) = self.validate_local_inactive(&terminal, admission.as_ref()) {
            self.fail(error);
        }
        LocalTerminalRecord {
            terminal: TerminalRecord {
                generation: terminal.generation.into(),
                logical_id: terminal.operation_id.into(),
                ticket: terminal.ticket,
                wire_id: terminal.wire_id.unwrap_or_default().into(),
                operation: terminal.operation.into(),
                impact: terminal.impact,
                admission,
                report: terminal.report.clone(),
                error: self.failure.clone(),
            },
            wire_id: terminal.wire_id.map(str::to_owned),
            candidate_wire_id: terminal.candidate_wire_id.map(str::to_owned),
            target: terminal
                .target
                .map(|(logical, ticket)| (logical.into(), ticket)),
            payload: terminal.payload.clone(),
            origin: terminal.origin,
        }
    }

    fn validate_local_inactive(
        &mut self,
        terminal: &LocalTerminal<'_>,
        admission: Option<&Admission>,
    ) -> AppResult<()> {
        self.check()?;
        let LocalTerminalOrigin::InactiveControl { admitted } = terminal.origin else {
            return Err(denied());
        };
        let r = terminal.report;
        if terminal.wire_id.is_some()
            || terminal.operation != "credit"
            || terminal.impact != Impact::Control
            || self.generation.as_deref() != Some(terminal.generation)
            || terminal.operation_id.is_empty()
            || terminal.operation_id.len() > 128
            || terminal.operation_id.contains('\0')
            || !r.ok
            || r.certainty != Certainty::NotSubmitted
            || r.phase != ProcessPhase::Queued
            || r.result.as_ref() != Some(&json!({"inactive":true}))
            || r.error_code.is_some()
            || r.retirement.is_some()
            || !r.statements.is_empty()
            || !no_acks(r)
        {
            return Err(denied());
        }
        let (logical, Some(ticket)) = terminal.target.ok_or_else(denied)? else {
            return Err(denied());
        };
        let owner = self.completed_queries.get(&ticket).ok_or_else(denied)?;
        if owner.generation != terminal.generation
            || owner.logical_id != logical
            || owner.ticket != ticket
            || self
                .phase()
                .is_none_or(|phase| phase.number() != owner.phase)
        {
            return Err(denied());
        }
        let kind = if terminal.payload == &json!({"target":logical,"batches":1,"discard":false}) {
            ControlKind::Credit
        } else if terminal.payload == &json!({"target":logical,"batches":0,"discard":true}) {
            ControlKind::Discard
        } else {
            return Err(denied());
        };
        if admitted {
            let admission = admission.ok_or_else(denied)?;
            if terminal.candidate_wire_id != Some(admission.wire_id.to_string().as_str())
                || admission
                    .target
                    .as_ref()
                    .map(|(logical, ticket)| (logical.as_str(), *ticket))
                    != terminal.target
                || !matches!(admission.kind(), AdmissionKind::Control(control) if *control == kind)
            {
                return Err(denied());
            }
            let mut resolved = terminal.payload.clone();
            resolved["target"] = json!(owner.wire_id.to_string());
            if admission.payload["payload"] != resolved {
                return Err(denied());
            }
        } else {
            if admission.is_some()
                || terminal.candidate_wire_id.is_some()
                || owner.discarded
                || self.tickets.contains(&terminal.ticket)
                || self.tickets.len() >= MAX_REQUESTS
                || self
                    .pending
                    .values()
                    .any(|pending| pending.logical_id == terminal.operation_id)
            {
                return Err(denied());
            }
            self.tickets.insert(terminal.ticket);
            if kind == ControlKind::Discard {
                self.completed_queries
                    .get_mut(&ticket)
                    .ok_or_else(denied)?
                    .discarded = true;
            }
        }
        Ok(())
    }

    fn complete(
        &mut self,
        admission: &Admission,
        report: &Report,
        discarded: bool,
    ) -> AppResult<()> {
        match admission.kind() {
            AdmissionKind::Sql(entry) => {
                if self
                    .next_entry()
                    .is_none_or(|next| next.ordinal() != entry.ordinal())
                {
                    return Err(denied());
                }
                if !report.ok {
                    if self.phase_failed || !definitive_rejection(report) {
                        return Err(denied());
                    }
                    let suffix = self.ledger.rejection_suffix(self.index)?;
                    self.phase_failed = true;
                    self.suffix_end = Some(self.index + 1 + suffix.len());
                    self.index += 1;
                    return Ok(());
                }
                validate_sql_success(entry, report)?;
                if matches!(entry.kind(), SqlKind::BootstrapContext) {
                    self.freeze_identity(report)?;
                }
                if admission.operation == "query" {
                    if self.completed_queries.len() >= MAX_REQUESTS {
                        return Err(denied());
                    }
                    self.completed_queries.insert(
                        admission.ticket,
                        CompletedQuery {
                            generation: admission.generation.clone(),
                            logical_id: admission.logical_id.clone(),
                            ticket: admission.ticket,
                            wire_id: admission.wire_id,
                            phase: entry.phase(),
                            discarded,
                        },
                    );
                }
                self.index += 1;
                Ok(())
            }
            AdmissionKind::Connect => {
                validate_control_success(report)?;
                self.connected = true;
                Ok(())
            }
            AdmissionKind::Control(_) => validate_control_success(report),
        }
    }

    fn freeze_identity(&mut self, report: &Report) -> AppResult<()> {
        let value = report.result.as_ref().ok_or_else(denied)?;
        let fields = ["user", "role", "database", "schema", "warehouse", "session"];
        if value.as_object().is_none_or(|object| {
            object.keys().any(|key| {
                !fields.contains(&key.as_str()) && key != "sdkRequestId" && key != "queryId"
            })
        }) || fields
            .iter()
            .any(|field| value[*field].as_str().is_none_or(str::is_empty))
            || ["role", "database", "schema", "warehouse"]
                .iter()
                .any(|field| value[*field] != self.profile[*field])
            || self.identity.is_some()
        {
            return Err(denied());
        }
        self.identity = Some(WireIdentity::new(
            value["user"].as_str().ok_or_else(denied)?.into(),
            value["session"].as_str().ok_or_else(denied)?.into(),
        )?);
        Ok(())
    }
}

fn no_acks(report: &Report) -> bool {
    report.use_acknowledged.is_none()
        && report.boundary_acknowledged.is_none()
        && report.dml_acknowledged.is_none()
        && report.ddl_acknowledged.is_none()
}
fn definitive_rejection(report: &Report) -> bool {
    !report.ok
        && report.retirement.is_none()
        && report.error_code.as_deref() == Some("database")
        && report.certainty == Certainty::ServerTerminal
        && no_acks(report)
}
fn validate_control_success(report: &Report) -> AppResult<()> {
    if report.ok
        && report.error_code.is_none()
        && report.retirement.is_none()
        && report.certainty == Certainty::ServerTerminal
        && no_acks(report)
        && report.statements.is_empty()
    {
        Ok(())
    } else {
        Err(denied())
    }
}
fn validate_sql_success(entry: &SqlEntry, report: &Report) -> AppResult<()> {
    if !report.ok
        || report.error_code.is_some()
        || report.retirement.is_some()
        || report.phase != ProcessPhase::Acknowledged
        || report.certainty != Certainty::StatementSucceeded
        || report.result.is_none()
        || report.use_acknowledged.is_some()
        || report.statements.len() != 1
    {
        return Err(denied());
    }
    let last = report.statements.last();
    let result = report.result.as_ref().ok_or_else(denied)?;
    let valid = match entry.kind() {
        SqlKind::BootstrapContext | SqlKind::Read(_) | SqlKind::Readback(_) => no_acks(report),
        SqlKind::Ddl(kind) => {
            report.boundary_acknowledged.is_none()
                && report.dml_acknowledged.is_none()
                && report
                    .ddl_acknowledged
                    .as_ref()
                    .is_some_and(|a| a.kind == kind && Some(&a.statement) == last)
                && result["kind"].as_str() == Some(kind.as_str())
        }
        SqlKind::Dml(kind) => {
            report.boundary_acknowledged.is_none()
                && report.ddl_acknowledged.is_none()
                && report
                    .dml_acknowledged
                    .as_ref()
                    .is_some_and(|a| a.kind == kind && Some(&a.statement) == last)
                && serde_json::to_value(kind).is_ok_and(|v| result["kind"] == v)
        }
        SqlKind::Transaction(action) => {
            report.dml_acknowledged.is_none()
                && report.ddl_acknowledged.is_none()
                && report.boundary_acknowledged.as_ref().is_some_and(|a| {
                    Some(&a.statement) == last
                        && a.action.as_str() == result["action"].as_str().unwrap_or("")
                        && serde_json::to_value(action).is_ok_and(|v| result["action"] == v)
                })
        }
    };
    if valid { Ok(()) } else { Err(denied()) }
}

#[cfg(test)]
#[path = "snowflake_table_qualification_sequence_tests.rs"]
mod tests;
