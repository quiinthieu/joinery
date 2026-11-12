//! Offline composition tests: real ledger entries and shaped callback/DTO
//! evidence pass through the owning policy. No adapter, account, or runner proof.
use super::super::super::{
    DATABASE, MAX_SCHEMAS, Operation, Plan, Presence, Readback, SCHEMA, Shape, StatementId,
};
use super::*;
use crate::{
    domain::{
        ColumnInfo, ColumnKind, EngineCapabilities, EngineKind, ExplicitTransactionAction,
        QueryEvent, ResultInfo, RetentionStatus, RunStatus, SessionContextKind,
        SessionContextOptions, SessionInfo, SessionRuntimeState, SnowflakeDdlKind,
        SnowflakeDdlOutcome, SnowflakeDdlPreflightResponse, SnowflakeDdlResult,
        SnowflakeDdlSummaryStatus, SnowflakeDmlKind, SnowflakeDmlOutcome, SnowflakeDmlResult,
        SnowflakeDmlSummary, SnowflakeDmlSummaryStatus, TableRef, TextRow,
        TransactionBoundaryOutcome, TransactionControlAuthority, TransactionControlResult,
        TransactionMode, TransactionObservation, TransactionState, TransactionUsability,
    },
    drivers::{
        snowflake_browsing::BrowseRelation,
        snowflake_metadata::{LiveContext, ReadKind, ResolvedSchema, SecondaryRoles},
        snowflake_process::{
            BoundaryAcknowledgement, DdlAcknowledgement, DmlAcknowledgement, TransactionAction,
        },
    },
    relation_inspection::{
        BrowseCatalog, ColumnIdentity, ColumnInspection, ConstraintInspectionStatus,
        InspectedColumnKind, RelationAvailability, RelationInspection, RelationTraits,
        WriteDefaultObservationStatus,
    },
};
use sha2::{Digest, Sha256};
use std::task::Poll;

fn context() -> LiveContext {
    LiveContext {
        principal: "FIXTURE".into(),
        role: Some("QUALIFICATION".into()),
        database: Some(DATABASE.into()),
        schema: Some(SCHEMA.into()),
        warehouse: Some("FIXTURE_WH".into()),
        session_id: "opaque-session".into(),
        search_path: vec![ResolvedSchema {
            raw: format!("{DATABASE}.{SCHEMA}"),
            database: Some(DATABASE.into()),
            schema: Some(SCHEMA.into()),
        }],
        secondary_roles: SecondaryRoles {
            roles: "QUALIFICATION".into(),
            value: "ALL".into(),
        },
        quoted_identifiers_ignore_case: false,
    }
}

fn startup() -> SessionInfo {
    SessionInfo {
        id: "console-session".into(),
        connection_id: "connection".into(),
        engine: EngineKind::Snowflake,
        server_version: "9.0.0".into(),
        latency_ms: 0,
        encrypted: true,
        capabilities: serde_json::from_value::<EngineCapabilities>(json!({
            "sql":true,"transactions":false,"cancellation":true,"tableBrowsing":false,
            "tableEditing":false,"tableFiltering":false,"processes":false
        }))
        .unwrap(),
    }
}

fn ddl_kind(id: StatementId) -> SnowflakeDdlKind {
    match id {
        StatementId::Create => SnowflakeDdlKind::CreateTable,
        StatementId::Add => SnowflakeDdlKind::AddColumn,
        StatementId::Drop => SnowflakeDdlKind::DropTable,
        _ => panic!("not a DDL phase"),
    }
}

fn dml_kind(id: StatementId) -> SnowflakeDmlKind {
    match id {
        StatementId::RollbackInsert | StatementId::CommittedInsert => SnowflakeDmlKind::Insert,
        StatementId::Update => SnowflakeDmlKind::Update,
        StatementId::Delete => SnowflakeDmlKind::Delete,
        _ => panic!("not a DML phase"),
    }
}

fn wire_action(action: ExplicitTransactionAction) -> TransactionAction {
    match action {
        ExplicitTransactionAction::Begin => TransactionAction::Begin,
        ExplicitTransactionAction::Commit => TransactionAction::Commit,
        ExplicitTransactionAction::Rollback => TransactionAction::Rollback,
    }
}

fn rows(read: Readback) -> Vec<TextRow> {
    let values = match read {
        Readback::Ownership => vec![
            Some("QUALIFICATION"),
            Some("BASE TABLE"),
            Some("NO"),
            Some("NO"),
            Some("created-marker"),
        ],
        Readback::Empty => return vec![],
        Readback::RollbackRow => vec![Some("1"), Some("rollback-only"), Some("true"), None],
        Readback::CommittedRow => vec![Some("2"), Some("committed"), Some("true"), None],
        Readback::UpdatedRow => vec![Some("2"), Some("updated"), Some("false"), None],
    };
    vec![
        values
            .into_iter()
            .map(|value| value.map(str::to_owned))
            .collect(),
    ]
}

enum Completion {
    Startup(SessionInfo),
    Preflight(SnowflakeDdlPreflightResponse),
    Ddl(SnowflakeDdlResult),
    Reconcile(SessionContextOptions, BrowseCatalog),
    Catalog(BrowseCatalog),
    Inspect(RelationInspection),
    Read(Vec<QueryEvent>, ResultInfo, Vec<TextRow>),
    Boundary(TransactionControlResult),
    Dml(SnowflakeDmlResult),
    Options(SessionContextOptions),
}

impl Completion {
    fn evidence(&self) -> PhaseEvidence<'_> {
        match self {
            Self::Startup(info) => PhaseEvidence::Startup(info),
            Self::Preflight(value) => PhaseEvidence::Preflight(value),
            Self::Ddl(value) => PhaseEvidence::Ddl(value),
            Self::Reconcile(options, catalog) => PhaseEvidence::Reconcile { options, catalog },
            Self::Catalog(value) => PhaseEvidence::Catalog(value),
            Self::Inspect(value) => PhaseEvidence::Inspect(value),
            Self::Read(events, result, rows) => PhaseEvidence::Read {
                events,
                result,
                rows,
            },
            Self::Boundary(value) => PhaseEvidence::Boundary(value),
            Self::Dml(value) => PhaseEvidence::Dml(value),
            Self::Options(value) => PhaseEvidence::Options(value),
        }
    }
}

struct Route {
    directory: tempfile::TempDir,
    policy: Policy,
    plan: Plan,
    context: LiveContext,
    names: Vec<String>,
    next_wire: u64,
    runtime_count: usize,
    last_runtime: Option<SessionRuntimeState>,
    read_rows_seen: Option<u64>,
    sql: Vec<SqlEntry>,
    originals: Vec<(u64, Report)>,
}

impl Route {
    async fn new(schema_count: usize) -> Self {
        assert!((1..=MAX_SCHEMAS).contains(&schema_count));
        let (directory, policy) = policy().await;
        let plan = policy.state.lock().sequence.plan().clone();
        let mut names = (0..schema_count - 1)
            .map(|index| format!("AUX_{index:02}"))
            .collect::<Vec<_>>();
        names.push(SCHEMA.into());
        names.sort();
        let connect = Frame::new(1, "connect", Impact::Control, profile());
        TransportPolicy::admit(&policy, connect.request())
            .await
            .unwrap();
        TransportPolicy::terminal(&policy, connect.terminal(&connect_report())).unwrap();
        Self {
            directory,
            policy,
            plan,
            context: context(),
            names,
            next_wire: 2,
            runtime_count: 0,
            last_runtime: None,
            read_rows_seen: None,
            sql: vec![],
            originals: vec![],
        }
    }

    fn phase(&self) -> u8 {
        self.policy.state.lock().sequence.phase().unwrap().number()
    }

    fn frame(&self, entry: &SqlEntry) -> Frame {
        let expected = json!({"user":self.context.principal,"session":self.context.session_id});
        let (impact, payload) = match entry.kind() {
            SqlKind::BootstrapContext => (Impact::Read, json!({})),
            SqlKind::Read(_) | SqlKind::Readback(_) => {
                let bounds = entry.retention().unwrap();
                (
                    Impact::Read,
                    json!({"sql":entry.sql(),"binds":[],
                    "classification":"read","credits":1,
                    "maxRows":bounds.max_rows,"maxBytes":bounds.max_bytes}),
                )
            }
            SqlKind::Ddl(kind) => (
                Impact::Mutating,
                json!({"kind":kind.as_str(),"sql":entry.sql(),"expected":expected}),
            ),
            SqlKind::Dml(kind) => (
                Impact::Mutating,
                json!({"kind":kind,"sql":entry.sql(),"binds":[],"expected":expected}),
            ),
            SqlKind::Transaction(action) => (
                wire_action(action).impact(),
                json!({"action":wire_action(action).as_str(),"expected":expected}),
            ),
        };
        Frame::new(self.next_wire, entry.kind().operation(), impact, payload)
    }

    fn relation(&self, phase: u8) -> BrowseRelation {
        BrowseRelation {
            database: DATABASE.into(),
            table: TableRef {
                schema: SCHEMA.into(),
                name: self.plan.table().into(),
            },
            table_type: "BASE TABLE".into(),
            traits: RelationTraits {
                transient: Some(false),
                temporary: Some(false),
                iceberg: Some(false),
                dynamic: Some(false),
                hybrid: Some(false),
                immutable: Some(false),
            },
            availability: RelationAvailability::Available,
            created: Some("created-marker".into()),
            last_ddl: Some(
                if phase < 10 {
                    "create-ddl-marker"
                } else {
                    "post-add-marker"
                }
                .into(),
            ),
        }
    }

    fn present(phase: u8) -> bool {
        (4..=30).contains(&phase)
    }

    fn observed_rows(&self, entry: &SqlEntry) -> u64 {
        match entry.kind() {
            SqlKind::Readback(read) => rows(read).len() as u64,
            SqlKind::Read(ReadKind::Schemas) => self.names.len() as u64,
            SqlKind::Read(ReadKind::Columns | ReadKind::Describe) => {
                if entry.phase() < 10 {
                    3
                } else {
                    4
                }
            }
            SqlKind::Read(ReadKind::ZeroRowResolution) => 0,
            SqlKind::Read(ReadKind::ExactRelation) => u64::from(Self::present(entry.phase())),
            SqlKind::Read(ReadKind::Relations) if matches!(entry.phase(), 2 | 3) => 0,
            SqlKind::Read(ReadKind::Relations) => {
                let offset = if matches!(entry.phase(), 4 | 10 | 31) {
                    14
                } else {
                    3
                };
                let index = usize::from(entry.phase_ordinal() - offset - 1);
                u64::from(Self::present(entry.phase()) && self.names[index] == SCHEMA)
            }
            _ => 1,
        }
    }

    fn original(&self, entry: &SqlEntry) -> Report {
        let statement = StatementIdentity {
            sdk_request_id: format!("00000000-0000-4000-8000-{:012}", self.next_wire),
            query_id: Some(format!("00000000-0000-4000-9000-{:012}", self.next_wire)),
        };
        let mut original = report(json!({}));
        original.statements = vec![statement.clone()];
        let mut result = match entry.kind() {
            SqlKind::BootstrapContext => json!({"user":self.context.principal,
                "role":self.context.role,"database":DATABASE,"schema":SCHEMA,
                "warehouse":self.context.warehouse,"session":self.context.session_id}),
            SqlKind::Read(_) | SqlKind::Readback(_) => {
                json!({"rowsSeen":self.observed_rows(entry),"rowsRetained":self.observed_rows(entry),"truncated":false})
            }
            SqlKind::Ddl(kind) => {
                original.ddl_acknowledged = Some(DdlAcknowledgement {
                    kind,
                    statement: statement.clone(),
                });
                json!({"kind":kind.as_str(),"rowsSeen":1,"rowsRetained":0,"truncated":false,
                    "columns":[{"name":"status","type":"text"}],
                    "values":[["Statement executed successfully."]]})
            }
            SqlKind::Dml(kind) => {
                original.dml_acknowledged = Some(DmlAcknowledgement {
                    kind,
                    statement: statement.clone(),
                });
                let name = match kind {
                    SnowflakeDmlKind::Insert => "number of rows inserted",
                    SnowflakeDmlKind::Update => "number of rows updated",
                    SnowflakeDmlKind::Delete => "number of rows deleted",
                };
                json!({"kind":kind,"rowsSeen":1,"rowsRetained":0,"truncated":false,
                    "columns":[{"name":name,"type":"FIXED"}],"values":[["1"]]})
            }
            SqlKind::Transaction(action) => {
                let action = wire_action(action);
                original.boundary_acknowledged = Some(BoundaryAcknowledgement {
                    action,
                    statement: statement.clone(),
                });
                json!({"action":action.as_str()})
            }
        };
        result["sdkRequestId"] = json!(statement.sdk_request_id);
        result["queryId"] = json!(statement.query_id);
        original.result = Some(result);
        original
    }

    async fn step(&mut self, deliver_callbacks: bool) -> SqlEntry {
        let entry = self
            .policy
            .state
            .lock()
            .sequence
            .next_entry()
            .unwrap()
            .clone();
        let frame = self.frame(&entry);
        TransportPolicy::admit(&self.policy, frame.request())
            .await
            .unwrap();
        let original = self.original(&entry);
        if matches!(entry.kind(), SqlKind::Readback(_)) {
            self.read_rows_seen = original.result.as_ref().unwrap()["rowsSeen"].as_u64();
        }
        TransportPolicy::terminal(&self.policy, frame.terminal(&original)).unwrap();
        self.originals.push((self.next_wire, original));
        self.next_wire += 1;
        self.sql.push(entry.clone());
        if deliver_callbacks {
            self.callbacks(&entry);
            assert!(
                self.policy.state.lock().debt.is_empty(),
                "unconsumed callback at {entry:?}"
            );
        }
        entry
    }

    fn schemas(&self, site: MetadataSite) {
        MetadataObserver::observe_schemas(
            &self.policy,
            SchemaObservation {
                site,
                context: &self.context,
                wire_names: &self.names,
                parsed_names: &self.names,
            },
        )
        .unwrap();
    }

    fn relations(&self, phase: u8, site: MetadataSite, schema: Option<&str>) {
        let relations = if Self::present(phase) && schema.is_none_or(|schema| schema == SCHEMA) {
            vec![self.relation(phase)]
        } else {
            vec![]
        };
        MetadataObserver::observe_relations(
            &self.policy,
            RelationObservation {
                site,
                context: &self.context,
                schema,
                relations: &relations,
            },
        )
        .unwrap();
    }

    fn runtime(&mut self, phase: u8) {
        self.runtime_count += 1;
        let transaction = match phase {
            14..=16 => Some("transaction-A"),
            19 | 20 => Some("transaction-B"),
            _ => None,
        };
        let runtime = SessionRuntimeState {
            session_id: startup().id,
            transaction_mode: TransactionMode::AutoCommit,
            transaction_state: if transaction.is_some() {
                TransactionState::InTransaction
            } else {
                TransactionState::Idle
            },
            context_kind: SessionContextKind::Schema,
            active_context: Some(SCHEMA.into()),
            database: DATABASE.into(),
            role: self.context.role.clone(),
            search_path: self
                .context
                .search_path
                .iter()
                .map(|path| path.raw.clone())
                .collect(),
            context_fingerprint: Some(self.context.fingerprint().unwrap()),
            transaction_observation: Some(TransactionObservation::Observed {
                transaction_id: transaction.map(str::to_owned),
                abort_on_error: true,
                usability: TransactionUsability::Unknown,
            }),
            transaction_control_authority: Some(TransactionControlAuthority::Observed {
                revision: format!("authority-{}", self.runtime_count),
                allowed_actions: vec![
                    if transaction.is_some() {
                        ExplicitTransactionAction::Commit
                    } else {
                        ExplicitTransactionAction::Begin
                    },
                    ExplicitTransactionAction::Rollback,
                ],
            }),
        };
        assert_ne!(runtime.session_id, self.context.session_id);
        MetadataObserver::observe_runtime(&self.policy, &runtime).unwrap();
        self.last_runtime = Some(runtime);
    }

    // Independently transcribed production publication points, rather than
    // calling policy::observations to manufacture whatever debt it requests.
    fn callbacks(&mut self, entry: &SqlEntry) {
        use MetadataSite::*;
        let (phase, ordinal) = (entry.phase(), usize::from(entry.phase_ordinal()));
        match (phase, ordinal) {
            (2 | 3, 6) => self.schemas(CreateTargetCheck),
            (4 | 10 | 31 | 27, 6) => self.schemas(SessionContextOptions),
            (4 | 10 | 31, 14) | (6 | 11 | 28 | 32, 3) => self.schemas(RelationCatalogBatch),
            _ => {}
        }
        match (phase, ordinal) {
            (2 | 3, 7) => self.relations(phase, CreateTargetCheck, Some(SCHEMA)),
            (8 | 9, 6 | 10) | (29 | 30, 6) | (7 | 12, 3 | 7) => {
                self.relations(phase, ExactRelationCheck, Some(SCHEMA))
            }
            _ => {}
        }
        if matches!(phase, 4 | 10 | 31 | 6 | 11 | 28 | 32) {
            let offset = if matches!(phase, 4 | 10 | 31) { 14 } else { 3 };
            if (offset + 1..=offset + self.names.len()).contains(&ordinal) {
                self.relations(
                    phase,
                    RelationCatalogBatch,
                    Some(&self.names[ordinal - offset - 1]),
                );
            }
            if ordinal == offset + self.names.len() + 2 {
                self.relations(phase, RelationCatalogSnapshot, None);
            }
            if matches!(phase, 4 | 10 | 31) && ordinal == self.names.len() + 22 {
                self.schemas(SessionContextOptions);
            }
        }
        let runtime = match phase {
            2 => ordinal == 12,
            3 => ordinal == 18,
            8 => ordinal == 15,
            9 => ordinal == 21,
            29 => ordinal == 11,
            30 => ordinal == 17,
            4 | 10 | 31 => ordinal == 11 || ordinal == self.names.len() + 27,
            5 | 13..=16 | 18..=27 => ordinal == 11,
            17 => ordinal == 6,
            _ => false,
        };
        if runtime {
            self.runtime(phase);
        }
    }

    fn catalog(&self, phase: u8, presence: Presence) -> BrowseCatalog {
        BrowseCatalog {
            revision: format!("catalog-{phase}"),
            database: DATABASE.into(),
            identity: self.context.completion_identity().unwrap(),
            relations: if presence == Presence::Present {
                vec![self.relation(phase).summary(format!("target-{phase}"))]
            } else {
                vec![]
            },
        }
    }

    fn options(&self, phase: u8) -> SessionContextOptions {
        SessionContextOptions {
            state: self.last_runtime.clone().unwrap(),
            available: self.names.clone(),
            revision: Some(format!("options-{phase}")),
        }
    }

    fn completion(&self) -> Completion {
        let phase = self.policy.state.lock().sequence.phase().unwrap().clone();
        let number = phase.number();
        match phase.operation() {
            Operation::StartupIdentityAndVersion => Completion::Startup(startup()),
            Operation::DdlPreflight(id) => Completion::Preflight(SnowflakeDdlPreflightResponse {
                revision: format!("preflight-{number}"),
                kind: ddl_kind(id),
                database: DATABASE.into(),
                schema: SCHEMA.into(),
                name: self.plan.table().into(),
                qualified_target: self.plan.qualified_target().into(),
                sql: self.plan.statement(id).source_sql().into(),
                warnings: vec![],
            }),
            Operation::ReviewedDdl(id) => Completion::Ddl(SnowflakeDdlResult {
                kind: ddl_kind(id),
                qualified_target: self.plan.qualified_target().into(),
                statement_outcome: SnowflakeDdlOutcome::Succeeded,
                summary_status: SnowflakeDdlSummaryStatus::Acknowledged,
                catalog_reconciliation_required: true,
                state: self.last_runtime.clone().unwrap(),
                retired: false,
                error: None,
            }),
            Operation::CoherentReconciliation(presence) => {
                Completion::Reconcile(self.options(number), self.catalog(number, presence))
            }
            Operation::Catalog(presence) => Completion::Catalog(self.catalog(number, presence)),
            Operation::Inspect(shape) => Completion::Inspect(inspection(&self.plan, number, shape)),
            Operation::Read(read) => {
                let rows = rows(read);
                let (events, result) = read_events(
                    number,
                    read,
                    &rows,
                    self.read_rows_seen.unwrap(),
                    self.last_runtime.as_ref().unwrap(),
                );
                Completion::Read(events, result, rows)
            }
            Operation::TypedBoundary { action, .. } => {
                Completion::Boundary(TransactionControlResult {
                    action,
                    boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
                    state: self.last_runtime.clone().unwrap(),
                    retired: false,
                    error: None,
                })
            }
            Operation::ReviewedDml(id) => Completion::Dml(SnowflakeDmlResult {
                kind: dml_kind(id),
                statement_outcome: SnowflakeDmlOutcome::Succeeded,
                summary: SnowflakeDmlSummary {
                    status: SnowflakeDmlSummaryStatus::Validated,
                    affected_rows: Some("1".into()),
                    multi_joined_rows: None,
                },
                state: self.last_runtime.clone().unwrap(),
                retired: false,
                error: None,
            }),
            Operation::SessionOptionsResume => Completion::Options(self.options(number)),
        }
    }

    async fn run_sql_phase(&mut self) {
        let phase = self.phase();
        loop {
            let next = self
                .policy
                .state
                .lock()
                .sequence
                .next_entry()
                .map(|entry| entry.phase());
            if next != Some(phase) {
                break;
            }
            self.step(true).await;
        }
    }

    async fn finish(&self) {
        let completion = self.completion();
        self.policy
            .finish_phase(completion.evidence())
            .await
            .unwrap();
        assert!(self.policy.state.lock().pending.is_empty());
    }
}

fn inspection(plan: &Plan, phase: u8, shape: Shape) -> RelationInspection {
    let count = if shape == Shape::ThreeNullableColumns {
        3
    } else {
        4
    };
    let columns = [
        ("ID", "NUMBER(38,0)"),
        ("NOTE", "VARCHAR(128)"),
        ("ACTIVE", "BOOLEAN"),
        ("EXTRA", "VARCHAR(32)"),
    ]
    .into_iter()
    .take(count)
    .enumerate()
    .map(|(index, (name, full_type))| ColumnInspection {
        name: name.into(),
        ordinal: index as u32 + 1,
        full_type: full_type.into(),
        data_type: full_type.into(),
        data_type_alias: None,
        kind: InspectedColumnKind::Column,
        result_kind: [
            ColumnKind::Decimal,
            ColumnKind::Text,
            ColumnKind::Boolean,
            ColumnKind::Text,
        ][index],
        nullable: true,
        default_expression: None,
        descriptor_default_expression: None,
        write_default_expression: None,
        write_default_status: WriteDefaultObservationStatus::Available,
        comment: None,
        character_maximum_length: None,
        character_octet_length: None,
        numeric_precision: None,
        numeric_precision_radix: None,
        numeric_scale: None,
        expression: None,
        identity: ColumnIdentity {
            is_identity: Some(false),
            generation: None,
            start: None,
            increment: None,
            ordered: None,
        },
    })
    .collect();
    RelationInspection {
        revision: format!("inspection-{phase}"),
        target_id: format!("target-{phase}"),
        database: DATABASE.into(),
        table: TableRef {
            schema: SCHEMA.into(),
            name: plan.table().into(),
        },
        columns,
        constraints: ConstraintInspectionStatus::NotFetched,
    }
}

fn read_events(
    phase: u8,
    read: Readback,
    rows: &[TextRow],
    original_rows_seen: u64,
    runtime: &SessionRuntimeState,
) -> (Vec<QueryEvent>, ResultInfo) {
    let names = if read == Readback::Ownership {
        vec![
            "TABLE_OWNER",
            "TABLE_TYPE",
            "IS_TRANSIENT",
            "IS_TEMPORARY",
            "CREATED",
        ]
    } else {
        vec!["ID", "NOTE", "ACTIVE", "EXTRA"]
    };
    let result = ResultInfo {
        id: format!("result-{phase}"),
        run_id: format!("run-{phase}"),
        statement_index: 0,
        columns: names
            .into_iter()
            .enumerate()
            .map(|(index, name)| ColumnInfo {
                id: format!("column-{phase}-{index}"),
                name: name.into(),
                database_type: "synthetic".into(),
                kind: if read == Readback::Ownership {
                    ColumnKind::Text
                } else {
                    [
                        ColumnKind::Decimal,
                        ColumnKind::Text,
                        ColumnKind::Boolean,
                        ColumnKind::Text,
                    ][index]
                },
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: false,
            })
            .collect(),
        retained_rows: rows.len(),
        status: RetentionStatus::Complete,
        table: None,
        editability: None,
    };
    let mut initial = result.clone();
    initial.retained_rows = 0;
    initial.status = RetentionStatus::Streaming;
    let mut events = vec![
        QueryEvent::Started {
            run_id: result.run_id.clone(),
        },
        QueryEvent::StatementStarted {
            run_id: result.run_id.clone(),
            index: 0,
        },
        QueryEvent::Result { result: initial },
    ];
    if !rows.is_empty() {
        events.push(QueryEvent::Progress {
            result_id: result.id.clone(),
            retained_rows: rows.len(),
        });
    }
    events.extend([
        QueryEvent::Result {
            result: result.clone(),
        },
        QueryEvent::StatementCompleted {
            run_id: result.run_id.clone(),
            index: 0,
            affected_rows: original_rows_seen.to_string(),
            elapsed_ms: 1,
        },
        QueryEvent::SessionState {
            state: runtime.clone(),
        },
        QueryEvent::RunMetrics {
            run_id: result.run_id.clone(),
            queue_ms: 0,
            active_ms: 1,
            first_row_ms: (!rows.is_empty()).then_some(1),
            total_ms: 1,
        },
        QueryEvent::Finished {
            execution_summary: None,
            run_id: result.run_id.clone(),
            status: RunStatus::Completed,
            elapsed_ms: 1,
        },
    ]);
    (events, result)
}

fn assert_chain(rows: &[Value]) {
    let mut previous = "0".repeat(64);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["sequence"], index as u64);
        assert_eq!(row["previous"], previous);
        let unsigned =
            json!({"event":row["event"],"previous":row["previous"],"sequence":row["sequence"]});
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&unsigned).unwrap())
        );
        assert_eq!(row["sha256"], digest, "record {index}");
        previous = digest;
    }
}

#[tokio::test]
async fn owning_route_retired_create_preserves_original_ack_once_without_advancing() {
    use crate::drivers::{
        snowflake_process::Retirement,
        snowflake_qualification_transport::{LocalTerminal, LocalTerminalOrigin},
    };
    let mut route = Route::new(1).await;
    for _ in 1..3 {
        route.run_sql_phase().await;
        route.finish().await;
    }
    let entry = loop {
        let entry = route
            .policy
            .state
            .lock()
            .sequence
            .next_entry()
            .unwrap()
            .clone();
        if matches!(entry.kind(), SqlKind::Ddl(_)) {
            break entry;
        }
        route.step(true).await;
    };
    let frame = route.frame(&entry);
    TransportPolicy::admit(&route.policy, frame.request())
        .await
        .unwrap();
    let mut original = route.original(&entry);
    original.ok = false;
    original.result = None;
    original.error_code = Some("connection_closed".into());
    original.retirement = Some(Retirement::Eof);
    assert!(original.ddl_acknowledged.is_some());
    let wire = frame.envelope["id"].as_str().unwrap();
    assert!(
        TransportPolicy::local_terminal(
            &route.policy,
            LocalTerminal {
                generation: "fixture-generation",
                operation_id: "reusable-logical-handle",
                ticket: frame.ticket,
                wire_id: Some(wire),
                candidate_wire_id: Some(wire),
                operation: &frame.operation,
                impact: frame.impact,
                target: None,
                payload: &frame.envelope["payload"],
                origin: LocalTerminalOrigin::Retirement,
                report: &original,
            }
        )
        .is_err()
    );
    assert!(route.policy.persist_final_evidence().await.is_err());
    assert!(
        TransportPolicy::admit(&route.policy, frame.request())
            .await
            .is_err()
    );
    assert!(route.policy.persist_final_evidence().await.is_err());
    assert_eq!(route.phase(), 3);
    assert_eq!(
        route
            .policy
            .state
            .lock()
            .sequence
            .next_entry()
            .unwrap()
            .ordinal(),
        entry.ordinal()
    );
    let journal = records(route.directory.path());
    assert_chain(&journal);
    let local = journal
        .iter()
        .filter(|record| record["event"]["origin"]["kind"] == "retirement")
        .collect::<Vec<_>>();
    assert_eq!(local.len(), 1);
    let event = &local[0]["event"];
    assert_eq!(event["wireId"], wire);
    assert_eq!(event["candidateWireId"], wire);
    assert_eq!(event["report"]["retirement"], "Eof");
    assert_eq!(event["report"]["certainty"], "StatementSucceeded");
    let ack = original.ddl_acknowledged.as_ref().unwrap();
    assert_eq!(
        event["report"]["ddlAcknowledged"],
        json!({"kind":ack.kind,
        "statement":{"sdkRequestId":ack.statement.sdk_request_id,"queryId":ack.statement.query_id}})
    );
    assert!(event["report"]["result"].is_null());
    assert!(!event["report"]["ok"].as_bool().unwrap());
}

#[tokio::test]
async fn owning_routes_complete_all_32_phases_at_both_census_extremes() {
    for schemas in [1, MAX_SCHEMAS] {
        let mut route = Route::new(schemas).await;
        let expected_budgets = [
            2,
            12,
            18,
            27 + schemas,
            11,
            5 + schemas,
            9,
            15,
            21,
            27 + schemas,
            5 + schemas,
            9,
            11,
            11,
            11,
            11,
            6,
            11,
            11,
            11,
            11,
            11,
            11,
            11,
            11,
            11,
            11,
            5 + schemas,
            11,
            17,
            27 + schemas,
            5 + schemas,
        ];
        for number in 1..=32 {
            assert_eq!(route.phase(), number as u8);
            let start = route.sql.len();
            route.run_sql_phase().await;
            assert_eq!(
                route.sql.len() - start,
                expected_budgets[number - 1],
                "phase {number}"
            );
            route.finish().await;
            let phase = route
                .policy
                .state
                .lock()
                .sequence
                .phase()
                .map(|phase| phase.number());
            assert_eq!(phase, (number < 32).then_some(number as u8 + 1));
            let journal = records(route.directory.path());
            assert_eq!(
                journal
                    .iter()
                    .filter(|row| row["event"]["type"] == "validatedPhase")
                    .count(),
                number
            );
        }
        let expected_sql = 386 + 7 * schemas;
        assert_eq!(route.sql.len(), expected_sql);
        assert_eq!(expected_sql, if schemas == 1 { 393 } else { 610 });
        assert_eq!(route.runtime_count, 28);
        assert!(route.policy.state.lock().sequence.phase().is_none());
        route.policy.persist_final_evidence().await.unwrap();
        let journal = records(route.directory.path());
        assert_chain(&journal);
        assert_eq!(
            journal
                .iter()
                .filter(|row| row["event"]["type"] == "frozenLedger")
                .count(),
            1
        );
        assert_eq!(
            journal
                .iter()
                .filter(|row| row["event"]["report"].is_object())
                .count(),
            expected_sql + 1
        );
        assert_eq!(
            journal
                .iter()
                .filter(|row| row["event"]["kind"].is_object())
                .count(),
            expected_sql + 1
        );
        for (wire, original) in &route.originals {
            let wire_id = wire.to_string();
            let stored = journal
                .iter()
                .find(|row| {
                    row["event"]["report"].is_object()
                        && row["event"]["wireId"].as_str() == Some(wire_id.as_str())
                })
                .unwrap();
            assert_eq!(
                stored["event"]["report"]["result"],
                original.result.as_ref().unwrap().clone()
            );
            assert_eq!(stored["event"]["report"]["certainty"], "StatementSucceeded");
            assert_eq!(
                stored["event"]["report"]["statements"][0]["sdkRequestId"],
                original.statements[0].sdk_request_id
            );
            assert_eq!(
                stored["event"]["report"]["statements"][0]["queryId"],
                original.statements[0].query_id.as_ref().unwrap().as_str()
            );
            let statement = |identity: &StatementIdentity| {
                json!({
                    "sdkRequestId": identity.sdk_request_id,
                    "queryId": identity.query_id
                })
            };
            assert_eq!(
                stored["event"]["report"]["ddlAcknowledged"],
                original
                    .ddl_acknowledged
                    .as_ref()
                    .map(|ack| json!({"kind": ack.kind, "statement": statement(&ack.statement)}))
                    .unwrap_or(Value::Null)
            );
            assert_eq!(
                stored["event"]["report"]["dmlAcknowledged"],
                original
                    .dml_acknowledged
                    .as_ref()
                    .map(|ack| json!({"kind": ack.kind, "statement": statement(&ack.statement)}))
                    .unwrap_or(Value::Null)
            );
            assert_eq!(stored["event"]["report"]["boundaryAcknowledged"],
                original.boundary_acknowledged.as_ref().map(|ack|
                    json!({"action": ack.action.as_str(), "statement": statement(&ack.statement)}))
                    .unwrap_or(Value::Null));
        }
        for (key, count) in [
            ("ddlAcknowledged", 3),
            ("dmlAcknowledged", 4),
            ("boundaryAcknowledged", 4),
        ] {
            assert_eq!(
                journal
                    .iter()
                    .filter(|row| row["event"]["report"][key].is_object())
                    .count(),
                count
            );
        }
        for entry in &route.sql {
            if matches!(entry.kind(), SqlKind::Ddl(_) | SqlKind::Dml(_)) {
                assert!(entry.sql().contains(route.plan.qualified_target()));
                assert!(
                    route
                        .plan
                        .statements()
                        .iter()
                        .any(|statement| statement.wire_sql() == entry.sql())
                );
            }
            assert!(entry.binds().is_empty());
        }
        let before = journal.len();
        route.policy.persist_final_evidence().await.unwrap();
        assert_eq!(
            records(route.directory.path()).len(),
            before,
            "final persistence never replays originals"
        );
    }
}

#[tokio::test]
async fn owning_route_missing_metadata_callback_blocks_next_sql_and_phase_finish() {
    let mut route = Route::new(1).await;
    route.run_sql_phase().await;
    route.finish().await;
    for _ in 0..5 {
        route.step(true).await;
    }
    let entry = route.step(false).await;
    assert_eq!((entry.phase(), entry.phase_ordinal()), (2, 6));
    let next = route
        .policy
        .state
        .lock()
        .sequence
        .next_entry()
        .unwrap()
        .clone();
    let frame = route.frame(&next);
    assert!(
        TransportPolicy::admit(&route.policy, frame.request())
            .await
            .is_err()
    );
    let completion = route.completion();
    assert!(
        route
            .policy
            .finish_phase(completion.evidence())
            .await
            .is_err()
    );
    assert_eq!(route.phase(), 2);
    assert!(route.policy.persist_final_evidence().await.is_err());
    let journal = records(route.directory.path());
    assert_chain(&journal);
    assert_eq!(
        journal
            .iter()
            .filter(|row| row["event"]["type"] == "validatedPhase")
            .count(),
        1
    );
    assert_eq!(
        journal
            .iter()
            .filter(|row| row["event"]["report"].is_object())
            .count(),
        9
    );
    let before = journal.len();
    assert!(route.policy.persist_final_evidence().await.is_err());
    assert_eq!(records(route.directory.path()).len(), before);
}

#[tokio::test]
async fn owning_route_first_schema_context_cannot_repin_role_or_warehouse() {
    for field in ["role", "warehouse"] {
        let mut route = Route::new(1).await;
        route.run_sql_phase().await;
        route.finish().await;
        for _ in 0..5 {
            route.step(true).await;
        }
        let entry = route.step(false).await;
        assert_eq!((entry.phase(), entry.phase_ordinal()), (2, 6));
        let mut changed = route.context.clone();
        match field {
            "role" => changed.role = Some("OTHER_ROLE".into()),
            "warehouse" => changed.warehouse = Some("OTHER_WAREHOUSE".into()),
            _ => unreachable!(),
        }
        assert_eq!(changed.principal, route.context.principal);
        assert_eq!(changed.session_id, route.context.session_id);
        assert!(
            MetadataObserver::observe_schemas(
                &route.policy,
                SchemaObservation {
                    site: MetadataSite::CreateTargetCheck,
                    context: &changed,
                    wire_names: &route.names,
                    parsed_names: &route.names,
                }
            )
            .is_err(),
            "changed {field}"
        );
        {
            let state = route.policy.state.lock();
            assert!(state.lifecycle.context().is_none());
            assert!(!state.sequence.census_frozen());
        }
        assert_eq!(route.phase(), 2);
        assert!(route.policy.persist_final_evidence().await.is_err());
        let journal = records(route.directory.path());
        assert_chain(&journal);
        assert_eq!(
            journal
                .iter()
                .filter(|row| row["event"]["type"] == "frozenLedger")
                .count(),
            0
        );
        assert_eq!(
            journal
                .iter()
                .filter(|row| row["event"]["report"].is_object())
                .count(),
            9
        );
        let next = route
            .policy
            .state
            .lock()
            .sequence
            .next_entry()
            .unwrap()
            .clone();
        assert!(
            TransportPolicy::admit(&route.policy, route.frame(&next).request())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn owning_route_phase_finish_waits_for_its_own_fsync_receipt() {
    let mut route = Route::new(1).await;
    route.run_sql_phase().await;
    let completion = route.completion();
    let mut finish = Box::pin(route.policy.finish_phase(completion.evidence()));
    {
        let journal = route.policy.journal.lock().unwrap();
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(matches!(finish.as_mut().poll(&mut context), Poll::Pending));
        let state = route.policy.state.lock();
        assert!(state.flushing);
        assert_eq!(state.sequence.phase().unwrap().number(), 1);
        assert!(
            state
                .pending
                .iter()
                .any(|(_, event)| event["type"] == "validatedPhase")
        );
        drop(state);
        assert_eq!(
            records(route.directory.path())
                .iter()
                .filter(|row| row["event"]["type"] == "validatedPhase")
                .count(),
            0
        );
        drop(journal);
    }
    finish.await.unwrap();
    assert_eq!(route.phase(), 2);
    let journal = records(route.directory.path());
    assert_chain(&journal);
    assert_eq!(
        journal
            .iter()
            .filter(|row| row["event"]["type"] == "validatedPhase")
            .count(),
        1
    );
    assert!(route.policy.state.lock().pending.is_empty());
}

#[tokio::test]
async fn owning_route_cancelled_finish_retains_one_phase_record_without_advancing() {
    let mut route = Route::new(1).await;
    route.run_sql_phase().await;
    let completion = route.completion();
    let mut finish = Box::pin(route.policy.finish_phase(completion.evidence()));
    {
        let journal = route.policy.journal.lock().unwrap();
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(matches!(finish.as_mut().poll(&mut context), Poll::Pending));
        assert_eq!(route.phase(), 1);
        drop(finish);
        assert!(TransportPolicy::check(&route.policy).is_err());
        drop(journal);
    }
    assert!(route.policy.persist_final_evidence().await.is_err());
    assert_eq!(route.phase(), 1);
    let journal = records(route.directory.path());
    assert_chain(&journal);
    assert_eq!(
        journal
            .iter()
            .filter(|row| row["event"]["type"] == "validatedPhase")
            .count(),
        1
    );
    assert_eq!(
        journal
            .iter()
            .filter(|row| row["event"]["report"].is_object())
            .count(),
        3
    );
    assert!(route.policy.state.lock().pending.is_empty());
    assert!(route.policy.persist_final_evidence().await.is_err());
    assert_eq!(records(route.directory.path()).len(), journal.len());
    let next = route
        .policy
        .state
        .lock()
        .sequence
        .next_entry()
        .unwrap()
        .clone();
    assert!(
        TransportPolicy::admit(&route.policy, route.frame(&next).request())
            .await
            .is_err()
    );
}
