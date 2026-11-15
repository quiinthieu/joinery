use super::super::{Presence, Readback, Shape, template_relation};
use super::*;
use crate::domain::{
    ColumnInfo, ColumnKind, CompletionIdentity, EngineCapabilities, ExplicitTransactionAction,
    SessionContextKind, SnowflakeDdlOutcome, SnowflakeDdlSummaryStatus, SnowflakeDmlKind,
    SnowflakeDmlOutcome, SnowflakeDmlSummary, SnowflakeDmlSummaryStatus, TableRef,
    TransactionBoundaryOutcome, TransactionControlAuthority, TransactionMode,
    TransactionObservation, TransactionState, TransactionUsability,
};
use crate::relation_inspection::{
    ColumnIdentity, ColumnInspection, ConstraintInspectionStatus, InspectedColumnKind,
    WriteDefaultObservationStatus,
};
use serde_json::json;

fn plan() -> Plan {
    Plan::compile_run("0123456789ABCDEF0123456789ABCDEF").unwrap()
}

fn phase(plan: &Plan, operation: Operation) -> &Phase {
    plan.phases()
        .iter()
        .find(|phase| phase.operation() == operation)
        .unwrap()
}

fn runtime() -> SessionRuntimeState {
    SessionRuntimeState {
        session_id: "session".into(),
        transaction_mode: TransactionMode::AutoCommit,
        transaction_state: TransactionState::Idle,
        context_kind: SessionContextKind::Schema,
        active_context: Some(SCHEMA.into()),
        database: DATABASE.into(),
        role: Some("ROLE".into()),
        warehouse: Some("WH".into()),
        search_path: vec!["SANDBOX.BILLIE_TESTING".into()],
        context_fingerprint: Some("fingerprint".into()),
        transaction_observation: Some(TransactionObservation::Observed {
            transaction_id: None,
            abort_on_error: true,
            usability: TransactionUsability::Unknown,
        }),
        transaction_control_authority: Some(TransactionControlAuthority::Observed {
            revision: "authority".into(),
            allowed_actions: vec![
                ExplicitTransactionAction::Begin,
                ExplicitTransactionAction::Rollback,
            ],
        }),
    }
}

fn check<'a>(
    plan: &Plan,
    phase: &Phase,
    evidence: PhaseEvidence<'a>,
    state: Option<&SessionRuntimeState>,
) -> AppResult<ValidatedEvidence<'a>> {
    validate(
        plan,
        phase,
        evidence,
        "ROLE",
        "fingerprint",
        "session",
        state,
    )
}

fn info() -> SessionInfo {
    SessionInfo {
        console_mode: false,
        id: "session".into(),
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

fn columns(readback: Readback) -> Vec<ColumnInfo> {
    let names = if readback == Readback::Ownership {
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
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| ColumnInfo {
            id: format!("column-{index}"),
            name: name.into(),
            database_type: "synthetic".into(),
            kind: if readback == Readback::Ownership {
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
        .collect()
}

fn read_fixture(
    readback: Readback,
) -> (
    ResultInfo,
    Vec<TextRow>,
    Vec<QueryEvent>,
    SessionRuntimeState,
) {
    let rows = match readback {
        Readback::Ownership => vec![vec![
            Some("ROLE".into()),
            Some("BASE TABLE".into()),
            Some("YES".into()),
            Some("NO".into()),
            Some("created".into()),
        ]],
        Readback::Empty => vec![],
        Readback::RollbackRow => vec![vec![
            Some("1".into()),
            Some("rollback-only".into()),
            Some("true".into()),
            None,
        ]],
        Readback::CommittedRow => vec![vec![
            Some("2".into()),
            Some("committed".into()),
            Some("true".into()),
            None,
        ]],
        Readback::UpdatedRow => vec![vec![
            Some("2".into()),
            Some("updated".into()),
            Some("false".into()),
            None,
        ]],
    };
    let result = ResultInfo {
        id: "result".into(),
        run_id: "run".into(),
        statement_index: 0,
        columns: columns(readback),
        retained_rows: rows.len(),
        status: RetentionStatus::Complete,
        table: None,
        editability: None,
    };
    let mut initial = result.clone();
    initial.status = RetentionStatus::Streaming;
    initial.retained_rows = 0;
    let state = runtime();
    let mut events = vec![
        QueryEvent::Started {
            run_id: "run".into(),
        },
        QueryEvent::StatementStarted {
            run_id: "run".into(),
            index: 0,
        },
        QueryEvent::Result { result: initial },
    ];
    if !rows.is_empty() {
        events.push(QueryEvent::Progress {
            result_id: "result".into(),
            retained_rows: rows.len(),
        });
    }
    events.extend([
        QueryEvent::Result {
            result: result.clone(),
        },
        QueryEvent::StatementCompleted {
            run_id: "run".into(),
            index: 0,
            affected_rows: rows.len().to_string(),
            elapsed_ms: 1,
        },
        QueryEvent::SessionState {
            state: state.clone(),
        },
        QueryEvent::RunMetrics {
            run_id: "run".into(),
            queue_ms: 0,
            active_ms: 1,
            first_row_ms: (!rows.is_empty()).then_some(1),
            total_ms: 1,
        },
        QueryEvent::Finished {
            execution_summary: None,
            run_id: "run".into(),
            status: RunStatus::Completed,
            elapsed_ms: 1,
        },
    ]);
    (result, rows, events, state)
}

#[test]
fn evidence_all_closed_readbacks_accept_complete_actual_shape_and_record_original_events() {
    let plan = plan();
    for readback in [
        Readback::Ownership,
        Readback::Empty,
        Readback::RollbackRow,
        Readback::CommittedRow,
        Readback::UpdatedRow,
    ] {
        let (result, rows, events, state) = read_fixture(readback);
        let evidence = PhaseEvidence::Read {
            events: &events,
            result: &result,
            rows: &rows,
        };
        let validated = check(
            &plan,
            phase(&plan, Operation::Read(readback)),
            evidence,
            Some(&state),
        )
        .unwrap();
        assert_eq!(validated.read().unwrap().rows_seen, rows.len() as u64);
        assert!(!validated.read().unwrap().truncated);
        assert_eq!(validated.state(), Some(&state));
        assert_eq!(validated.record().unwrap(), evidence.record().unwrap());
        assert_eq!(
            validated.record().unwrap()["value"]["events"],
            serde_json::to_value(&events).unwrap()
        );
        if readback == Readback::Ownership {
            let marker = validated.ownership().unwrap();
            assert_eq!(marker.owner(), "ROLE");
            assert!(marker.transient());
            assert_eq!(marker.created(), "created");
        } else {
            assert!(validated.ownership().is_none());
        }
    }
}

#[test]
fn evidence_read_requires_every_lifecycle_event_in_order_and_rejects_duplicates() {
    let plan = plan();
    let readback = Readback::CommittedRow;
    let phase = phase(&plan, Operation::Read(readback));
    let (result, rows, events, state) = read_fixture(readback);
    for index in 0..events.len() {
        let mut missing = events.clone();
        missing.remove(index);
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &missing,
                    result: &result,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err(),
            "missing {index}"
        );
        let mut duplicate = events.clone();
        duplicate.insert(index, events[index].clone());
        // Progress is a repeatable production batching event. Every other stage
        // is unique; repeated progress is bounded separately.
        if !matches!(events[index], QueryEvent::Progress { .. }) {
            assert!(
                check(
                    &plan,
                    phase,
                    PhaseEvidence::Read {
                        events: &duplicate,
                        result: &result,
                        rows: &rows
                    },
                    Some(&state)
                )
                .is_err(),
                "duplicate {index}"
            );
        }
    }
    for index in 0..events.len() - 1 {
        let mut reordered = events.clone();
        reordered.swap(index, index + 1);
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &reordered,
                    result: &result,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err(),
            "swap {index}"
        );
    }
}

#[test]
fn evidence_read_rejects_count_spellings_and_discarded_rows_without_inventing_original_count() {
    let plan = plan();
    let phase = phase(&plan, Operation::Read(Readback::CommittedRow));
    let (result, rows, events, state) = read_fixture(Readback::CommittedRow);
    for count in [
        "",
        "01",
        "+1",
        " 1",
        "1.0",
        "-1",
        "0",
        "2",
        "18446744073709551616",
    ] {
        let mut events = events.clone();
        for event in &mut events {
            if let QueryEvent::StatementCompleted { affected_rows, .. } = event {
                *affected_rows = count.into();
            }
        }
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &events,
                    result: &result,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err(),
            "{count:?}"
        );
    }
    // A complete retained prefix cannot hide an original count of two.
    let mut events = events.clone();
    for event in &mut events {
        if let QueryEvent::StatementCompleted { affected_rows, .. } = event {
            *affected_rows = "2".into();
        }
    }
    assert!(
        check(
            &plan,
            phase,
            PhaseEvidence::Read {
                events: &events,
                result: &result,
                rows: &rows
            },
            Some(&state)
        )
        .is_err()
    );
}

#[test]
fn evidence_read_rejects_mixed_run_result_statement_column_and_runtime_identity() {
    let plan = plan();
    let phase = phase(&plan, Operation::Read(Readback::CommittedRow));
    let (result, rows, events, state) = read_fixture(Readback::CommittedRow);
    for index in 0..events.len() {
        let mut changed = events.clone();
        match &mut changed[index] {
            QueryEvent::Started { run_id }
            | QueryEvent::StatementStarted { run_id, .. }
            | QueryEvent::StatementCompleted { run_id, .. }
            | QueryEvent::RunMetrics { run_id, .. }
            | QueryEvent::Finished { run_id, .. } => *run_id = "other-run".into(),
            QueryEvent::Result { result } => result.columns[0].id = "other-column".into(),
            QueryEvent::Progress { result_id, .. } => *result_id = "other-result".into(),
            QueryEvent::SessionState { state } => state.transaction_control_authority = None,
            _ => unreachable!(),
        }
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &changed,
                    result: &result,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err(),
            "event {index}"
        );
    }
    for index in [1, 5] {
        let mut changed = events.clone();
        match &mut changed[index] {
            QueryEvent::StatementStarted { index, .. }
            | QueryEvent::StatementCompleted { index, .. } => *index = 1,
            _ => unreachable!(),
        }
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &changed,
                    result: &result,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err()
        );
    }
    assert!(
        check(
            &plan,
            phase,
            PhaseEvidence::Read {
                events: &events,
                result: &result,
                rows: &rows
            },
            None
        )
        .is_err()
    );
    for field in ["id", "runId"] {
        let mut changed = serde_json::to_value(&result).unwrap();
        changed[field] = "".into();
        let changed = serde_json::from_value(changed).unwrap();
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &events,
                    result: &changed,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err()
        );
    }
}

#[test]
fn evidence_read_rejects_failure_notice_closure_truncation_and_progress_overflow() {
    let plan = plan();
    let phase = phase(&plan, Operation::Read(Readback::CommittedRow));
    let (result, rows, events, state) = read_fixture(Readback::CommittedRow);
    for event in [
        QueryEvent::Notice {
            message: "storage failure".into(),
        },
        QueryEvent::SessionClosed {
            session_id: "session".into(),
            reason: "closed".into(),
        },
        QueryEvent::Error {
            error: invalid(),
            statement_index: 0,
        },
        QueryEvent::PlanContext {
            run_id: "run".into(),
            state: Some(state.clone()),
        },
    ] {
        let mut changed = events.clone();
        changed.insert(6, event);
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &changed,
                    result: &result,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err()
        );
    }
    for status in [
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::OutcomeUnknown,
    ] {
        let mut changed = events.clone();
        if let Some(QueryEvent::Finished { status: actual, .. }) = changed.last_mut() {
            *actual = status;
        }
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &changed,
                    result: &result,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err()
        );
    }
    for status in [RetentionStatus::Streaming, RetentionStatus::Truncated] {
        let mut changed = result.clone();
        changed.status = status;
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &events,
                    result: &changed,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err()
        );
    }
    for count in [0, 2] {
        let mut changed = events.clone();
        if let QueryEvent::Progress { retained_rows, .. } = &mut changed[3] {
            *retained_rows = count;
        }
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Read {
                    events: &changed,
                    result: &result,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err()
        );
    }
    let mut changed = events.clone();
    for _ in 0..32 {
        changed.insert(3, events[3].clone());
    }
    assert!(
        check(
            &plan,
            phase,
            PhaseEvidence::Read {
                events: &changed,
                result: &result,
                rows: &rows
            },
            Some(&state)
        )
        .is_err()
    );
}

#[test]
fn evidence_read_requires_fixed_data_and_ownership_not_just_a_successful_query() {
    let plan = plan();
    for readback in [Readback::Ownership, Readback::Empty, Readback::CommittedRow] {
        let (result, mut rows, events, state) = read_fixture(readback);
        if rows.is_empty() {
            rows.push(vec![None; result.columns.len()]);
        } else {
            rows[0][0] = Some("wrong".into());
        }
        assert!(
            check(
                &plan,
                phase(&plan, Operation::Read(readback)),
                PhaseEvidence::Read {
                    events: &events,
                    result: &result,
                    rows: &rows
                },
                Some(&state)
            )
            .is_err()
        );
    }
    let (result, mut rows, events, state) = read_fixture(Readback::Ownership);
    rows[0].pop();
    assert!(
        check(
            &plan,
            phase(&plan, Operation::Read(Readback::Ownership)),
            PhaseEvidence::Read {
                events: &events,
                result: &result,
                rows: &rows
            },
            Some(&state)
        )
        .is_err()
    );
}

#[test]
fn evidence_startup_and_preflight_bind_the_closed_session_kind_target_and_exact_sql() {
    let plan = plan();
    let startup = phase(&plan, Operation::StartupIdentityAndVersion);
    let info = info();
    assert!(check(&plan, startup, PhaseEvidence::Startup(&info), None).is_ok());
    assert!(
        validate(
            &plan,
            startup,
            PhaseEvidence::Startup(&info),
            "",
            "",
            "session",
            None
        )
        .is_ok()
    );
    assert!(
        validate(
            &plan,
            startup,
            PhaseEvidence::Startup(&info),
            "",
            "",
            "",
            None
        )
        .is_err()
    );
    for field in ["id", "engine", "encrypted", "serverVersion"] {
        let mut value = serde_json::to_value(&info).unwrap();
        value[field] = match field {
            "engine" => json!("postgres"),
            "encrypted" => json!(false),
            _ => json!(""),
        };
        let info = serde_json::from_value(value).unwrap();
        assert!(check(&plan, startup, PhaseEvidence::Startup(&info), None).is_err());
    }
    for (id, kind) in [
        (StatementId::Create, SnowflakeDdlKind::CreateTable),
        (StatementId::Add, SnowflakeDdlKind::AddColumn),
        (StatementId::Drop, SnowflakeDdlKind::DropTable),
    ] {
        let phase = phase(&plan, Operation::DdlPreflight(id));
        let preflight = SnowflakeDdlPreflightResponse {
            revision: "revision".into(),
            kind,
            database: DATABASE.into(),
            schema: SCHEMA.into(),
            name: plan.table().into(),
            qualified_target: plan.qualified_target().into(),
            sql: plan.statement(id).source_sql().into(),
            warnings: vec!["original warning".into()],
        };
        let evidence = check(&plan, phase, PhaseEvidence::Preflight(&preflight), None).unwrap();
        assert_eq!(
            evidence.record().unwrap()["value"]["warnings"],
            json!(["original warning"])
        );
        for field in [
            "revision",
            "kind",
            "database",
            "schema",
            "name",
            "qualifiedTarget",
            "sql",
        ] {
            let mut value = serde_json::to_value(&preflight).unwrap();
            value[field] = if field == "kind" {
                json!(if kind == SnowflakeDdlKind::CreateTable {
                    "dropTable"
                } else {
                    "createTable"
                })
            } else {
                json!("")
            };
            let changed = serde_json::from_value(value).unwrap();
            assert!(
                check(&plan, phase, PhaseEvidence::Preflight(&changed), None).is_err(),
                "{id:?} {field}"
            );
        }
    }
}

fn catalog(plan: &Plan, present: bool) -> BrowseCatalog {
    BrowseCatalog {
        revision: "catalog-revision".into(),
        database: DATABASE.into(),
        identity: CompletionIdentity {
            database: DATABASE.into(),
            role: "ROLE".into(),
            search_path: runtime().search_path,
            case_sensitive_table_names: Some(true),
            context_fingerprint: Some("fingerprint".into()),
        },
        relations: if present {
            vec![
                template_relation(TableRef {
                    schema: SCHEMA.into(),
                    name: plan.table().into(),
                })
                .summary("target".into()),
            ]
        } else {
            vec![]
        },
    }
}
fn options() -> SessionContextOptions {
    SessionContextOptions {
        state: runtime(),
        available: vec![SCHEMA.into()],
        revision: Some("options-revision".into()),
    }
}

#[test]
fn evidence_reconciliation_requires_both_original_options_and_catalog_and_exact_last_state() {
    let plan = plan();
    let phase = phase(&plan, Operation::CoherentReconciliation(Presence::Present));
    let catalog = catalog(&plan, true);
    let options = options();
    let state = runtime();
    let evidence = check(
        &plan,
        phase,
        PhaseEvidence::Reconcile {
            options: &options,
            catalog: &catalog,
        },
        Some(&state),
    )
    .unwrap();
    assert_eq!(
        evidence.record().unwrap()["value"]["catalog"],
        serde_json::to_value(&catalog).unwrap()
    );
    assert!(check(&plan, phase, PhaseEvidence::Options(&options), Some(&state)).is_err());
    assert!(check(&plan, phase, PhaseEvidence::Catalog(&catalog), Some(&state)).is_err());
    assert!(
        check(
            &plan,
            phase,
            PhaseEvidence::Reconcile {
                options: &options,
                catalog: &catalog
            },
            None
        )
        .is_err()
    );
    let mut changed = options.clone();
    changed.state.transaction_control_authority = None;
    assert!(
        check(
            &plan,
            phase,
            PhaseEvidence::Reconcile {
                options: &changed,
                catalog: &catalog
            },
            Some(&state)
        )
        .is_err()
    );
    for available in [
        vec![],
        vec!["OTHER".into()],
        vec![SCHEMA.into(), SCHEMA.into()],
    ] {
        let mut changed = options.clone();
        changed.available = available;
        assert!(
            check(
                &plan,
                phase,
                PhaseEvidence::Reconcile {
                    options: &changed,
                    catalog: &catalog
                },
                Some(&state)
            )
            .is_err()
        );
    }
    let mut changed = options.clone();
    changed.revision = None;
    assert!(
        check(
            &plan,
            phase,
            PhaseEvidence::Reconcile {
                options: &changed,
                catalog: &catalog
            },
            Some(&state)
        )
        .is_err()
    );
    let mut changed = catalog.clone();
    changed.identity.role = "OTHER".into();
    assert!(
        check(
            &plan,
            phase,
            PhaseEvidence::Reconcile {
                options: &options,
                catalog: &changed
            },
            Some(&state)
        )
        .is_err()
    );
}

fn inspection(plan: &Plan, shape: Shape) -> RelationInspection {
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
        revision: "inspection".into(),
        target_id: "target".into(),
        database: DATABASE.into(),
        table: TableRef {
            schema: SCHEMA.into(),
            name: plan.table().into(),
        },
        columns,
        constraints: ConstraintInspectionStatus::NotFetched,
    }
}

#[test]
fn evidence_catalog_and_inspection_reuse_target_presence_and_exact_shape_without_fabricated_state()
{
    let plan = plan();
    for presence in [Presence::Present, Presence::DroppedNameAbsent] {
        let phase = phase(&plan, Operation::Catalog(presence));
        let catalog = catalog(&plan, presence == Presence::Present);
        let evidence = check(&plan, phase, PhaseEvidence::Catalog(&catalog), None).unwrap();
        assert!(evidence.state().is_none());
        let mut changed = catalog.clone();
        changed.revision.clear();
        assert!(check(&plan, phase, PhaseEvidence::Catalog(&changed), None).is_err());
    }
    for shape in [Shape::ThreeNullableColumns, Shape::FourNullableColumns] {
        let phase = phase(&plan, Operation::Inspect(shape));
        let inspection = inspection(&plan, shape);
        assert!(
            check(&plan, phase, PhaseEvidence::Inspect(&inspection), None)
                .unwrap()
                .state()
                .is_none()
        );
        let mut changed = inspection.clone();
        changed.columns[0].result_kind = ColumnKind::Unknown;
        assert!(check(&plan, phase, PhaseEvidence::Inspect(&changed), None).is_err());
    }
}

#[test]
fn evidence_ddl_dml_and_boundary_require_original_success_and_exact_observer_state() {
    let plan = plan();
    let state = runtime();
    for phase in plan.phases() {
        match phase.operation() {
            Operation::ReviewedDdl(id) => {
                let kind = match id {
                    StatementId::Create => SnowflakeDdlKind::CreateTable,
                    StatementId::Add => SnowflakeDdlKind::AddColumn,
                    _ => SnowflakeDdlKind::DropTable,
                };
                let result = SnowflakeDdlResult {
                    kind,
                    qualified_target: plan.qualified_target().into(),
                    statement_outcome: SnowflakeDdlOutcome::Succeeded,
                    summary_status: SnowflakeDdlSummaryStatus::Acknowledged,
                    catalog_reconciliation_required: true,
                    state: state.clone(),
                    retired: false,
                    error: None,
                };
                assert!(check(&plan, phase, PhaseEvidence::Ddl(&result), Some(&state)).is_ok());
                let mut changed = result.clone();
                changed.state.transaction_control_authority = None;
                assert!(check(&plan, phase, PhaseEvidence::Ddl(&changed), Some(&state)).is_err());
                let mut changed = result.clone();
                changed.statement_outcome = SnowflakeDdlOutcome::Unknown;
                assert!(check(&plan, phase, PhaseEvidence::Ddl(&changed), Some(&state)).is_err());
            }
            Operation::ReviewedDml(id) => {
                let kind = match id {
                    StatementId::Update => SnowflakeDmlKind::Update,
                    StatementId::Delete => SnowflakeDmlKind::Delete,
                    _ => SnowflakeDmlKind::Insert,
                };
                let result = SnowflakeDmlResult {
                    kind,
                    statement_outcome: SnowflakeDmlOutcome::Succeeded,
                    summary: SnowflakeDmlSummary {
                        status: SnowflakeDmlSummaryStatus::Validated,
                        affected_rows: Some("1".into()),
                        multi_joined_rows: None,
                    },
                    state: state.clone(),
                    retired: false,
                    error: None,
                };
                assert!(check(&plan, phase, PhaseEvidence::Dml(&result), Some(&state)).is_ok());
                let mut changed = result.clone();
                changed.error = Some(invalid());
                assert!(check(&plan, phase, PhaseEvidence::Dml(&changed), Some(&state)).is_err());
                assert!(check(&plan, phase, PhaseEvidence::Dml(&result), None).is_err());
            }
            Operation::TypedBoundary { action, .. } => {
                let result = TransactionControlResult {
                    action,
                    boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
                    state: state.clone(),
                    retired: false,
                    error: None,
                };
                assert!(
                    check(&plan, phase, PhaseEvidence::Boundary(&result), Some(&state)).is_ok()
                );
                let mut changed = result.clone();
                changed.retired = true;
                assert!(
                    check(
                        &plan,
                        phase,
                        PhaseEvidence::Boundary(&changed),
                        Some(&state)
                    )
                    .is_err()
                );
            }
            _ => {}
        }
    }
}

#[test]
fn evidence_rejects_wrong_variant_noncanonical_phase_and_unbound_runtime_and_checks_resume_options()
{
    let plan = plan();
    let state = runtime();
    let options = options();
    let resume = phase(&plan, Operation::SessionOptionsResume);
    assert!(
        check(
            &plan,
            resume,
            PhaseEvidence::Options(&options),
            Some(&state)
        )
        .is_ok()
    );
    let info = info();
    assert!(check(&plan, resume, PhaseEvidence::Startup(&info), Some(&state)).is_err());
    let mut forged = resume.clone();
    forged.number = 1;
    assert!(
        check(
            &plan,
            &forged,
            PhaseEvidence::Options(&options),
            Some(&state)
        )
        .is_err()
    );
    for (role, fp, session) in [
        ("", "fingerprint", "session"),
        ("OTHER", "fingerprint", "session"),
        ("ROLE", "", "session"),
        ("ROLE", "other", "session"),
        ("ROLE", "fingerprint", ""),
        ("ROLE", "fingerprint", "other"),
    ] {
        assert!(
            validate(
                &plan,
                resume,
                PhaseEvidence::Options(&options),
                role,
                fp,
                session,
                Some(&state)
            )
            .is_err()
        );
    }
}
