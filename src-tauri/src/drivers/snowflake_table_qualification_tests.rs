use super::*;
use crate::domain::{CompletionIdentity, SnowflakeDmlSummary, TransactionUsability};
use crate::relation_inspection::{
    ColumnIdentity, ColumnInspection, ConstraintInspectionStatus, WriteDefaultObservationStatus,
};
use std::collections::HashSet;

const RUN: &str = "0123456789ABCDEF0123456789ABCDEF";

fn plan() -> Plan {
    Plan::compile_run(RUN).unwrap()
}
fn census(count: usize) -> SchemaCensus {
    let mut names: Vec<_> = (0..count - 1).map(|index| format!("A{index:02}")).collect();
    names.push(SCHEMA.into());
    SchemaCensus::freeze(&names).unwrap()
}

#[test]
fn canonical_three_ddl_four_dml_and_fixed_readback_wire_sql_are_frozen() {
    let plan = plan();
    let target = "\"SANDBOX\".\"BILLIE_TESTING\".\"JOINERY_QUAL_0123456789ABCDEF0123456789ABCDEF\"";
    let expected = [
        (StatementId::Create, format!("CREATE TABLE {target} (\"ID\" NUMBER(38, 0), \"NOTE\" VARCHAR(128), \"ACTIVE\" BOOLEAN)")),
        (StatementId::Add, format!("ALTER TABLE {target} ADD COLUMN \"EXTRA\" VARCHAR(32)")),
        (StatementId::Drop, format!("DROP TABLE {target} RESTRICT")),
        (StatementId::RollbackInsert, format!("INSERT INTO {target} (\"ID\", \"NOTE\", \"ACTIVE\") VALUES (1, 'rollback-only', TRUE)")),
        (StatementId::CommittedInsert, format!("INSERT INTO {target} (\"ID\", \"NOTE\", \"ACTIVE\") VALUES (2, 'committed', TRUE)")),
        (StatementId::Update, format!("UPDATE {target} SET \"NOTE\" = 'updated', \"ACTIVE\" = FALSE WHERE \"ID\" = 2")),
        (StatementId::Delete, format!("DELETE FROM {target} WHERE \"ID\" = 2")),
        (StatementId::OwnershipRead, "SELECT TABLE_OWNER, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, CREATED::VARCHAR AS CREATED FROM \"SANDBOX\".INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = 'SANDBOX' AND TABLE_SCHEMA = 'BILLIE_TESTING' AND TABLE_NAME = 'JOINERY_QUAL_0123456789ABCDEF0123456789ABCDEF' ORDER BY TABLE_NAME LIMIT 2".into()),
        (StatementId::DataRead, format!("SELECT \"ID\", \"NOTE\", \"ACTIVE\", \"EXTRA\" FROM {target} ORDER BY \"ID\" LIMIT 2")),
    ];
    assert_eq!(plan.statements().len(), expected.len());
    for (id, sql) in expected {
        let statement = plan.statement(id);
        assert_eq!(statement.source_sql(), sql);
        assert_eq!(statement.wire_sql(), sql);
        assert!(statement.binds().is_empty());
        assert_eq!(
            serde_json::to_value(statement).unwrap()["binds"],
            serde_json::json!([])
        );
    }
    assert_eq!(ExplicitTransactionAction::Begin.sql(), "BEGIN TRANSACTION");
    assert_eq!(ExplicitTransactionAction::Rollback.sql(), "ROLLBACK");
    assert_eq!(ExplicitTransactionAction::Commit.sql(), "COMMIT");
}

#[test]
fn sole_factory_generates_unique_nonzero_uppercase_tokens_without_target_input() {
    let mut tokens = HashSet::new();
    for _ in 0..64 {
        let plan = Plan::generate().unwrap();
        assert_eq!(plan.run().len(), 32);
        assert!(
            plan.run()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
        );
        assert!(plan.run().bytes().any(|byte| byte != b'0'));
        assert_eq!(plan.table(), format!("JOINERY_QUAL_{}", plan.run()));
        assert!(tokens.insert(plan.run().to_owned()));
    }
    for invalid_run in [
        "",
        "00000000000000000000000000000000",
        "0123456789abcdef0123456789abcdef",
        "0123456789ABCDEF0123456789ABCDEG",
        "0123456789ABCDEF0123456789ABCDE",
        "0123456789ABCDEF0123456789ABCDEF0",
        "existing_table; DROP DATABASE X",
    ] {
        assert!(Plan::compile_run(invalid_run).is_err());
    }
}

#[test]
fn every_phase_budget_and_order_match_the_closed_success_path() {
    let plan = plan();
    assert_eq!(plan.phases().len(), PHASE_COUNT);
    let expected_fixed = [
        2, 12, 18, 27, 11, 5, 9, 15, 21, 27, 5, 9, 11, 11, 11, 11, 6, 11, 11, 11, 11, 11, 11, 11,
        11, 11, 11, 5, 11, 17, 27, 5,
    ];
    for (index, phase) in plan.phases().iter().enumerate() {
        assert_eq!(phase.number(), index as u8 + 1);
        assert_eq!(phase.budget().fixed(), expected_fixed[index]);
        assert_eq!(
            phase.budget().per_schema(),
            u8::from([4, 6, 10, 11, 28, 31, 32].contains(&phase.number()))
        );
        assert_eq!(
            phase.advancement(),
            AdvancementRule::AllPriorPhasesValidatedAndOriginalReceiptsSynced
        );
        assert_eq!(
            phase.failure(),
            FailureRule::OnlyActiveFrozenReadOnlyPostObserverSuffix
        );
    }
    for count in 1..=MAX_SCHEMAS {
        let census = census(count);
        let sum: u16 = plan
            .phases()
            .iter()
            .map(|phase| phase.budget().total(&census))
            .sum();
        assert_eq!(sum, 386 + 7 * count as u16);
        assert_eq!(plan.budget(&census), sum);
        assert!(sum <= 610);
    }
    assert_eq!(plan.budget(&census(1)), 393);
    assert_eq!(plan.budget(&census(32)), 610);
    let mutations: Vec<_> = plan
        .phases()
        .iter()
        .filter_map(|phase| match phase.operation() {
            Operation::ReviewedDdl(id) | Operation::ReviewedDml(id) => Some(format!("{id:?}")),
            Operation::TypedBoundary { action, .. } => Some(format!("{action:?}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        mutations,
        [
            "Create",
            "Add",
            "Begin",
            "RollbackInsert",
            "Rollback",
            "Begin",
            "CommittedInsert",
            "Commit",
            "Update",
            "Delete",
            "Drop"
        ]
    );
    let reads: Vec<_> = plan
        .phases()
        .iter()
        .filter_map(|phase| {
            if let Operation::Read(read) = phase.operation() {
                Some(read)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        reads,
        [
            Readback::Ownership,
            Readback::Empty,
            Readback::RollbackRow,
            Readback::Empty,
            Readback::CommittedRow,
            Readback::UpdatedRow,
            Readback::Empty
        ]
    );
    assert_eq!(plan.phases()[1].presence(), Some(Presence::FreshNameAbsent));
    assert_eq!(
        plan.phases()[29].operation(),
        Operation::ReviewedDdl(StatementId::Drop)
    );
    assert_eq!(
        plan.phases()[30].presence(),
        Some(Presence::DroppedNameAbsent)
    );
    assert_eq!(
        plan.phases()[31].presence(),
        Some(Presence::DroppedNameAbsent)
    );
    assert_eq!(plan.phases()[30].budget().total(&census(32)), 59);
    // There is one DROP in the success path and no failure/recovery/cleanup phase.
    assert_eq!(
        plan.phases()
            .iter()
            .filter(|phase| phase.operation() == Operation::ReviewedDdl(StatementId::Drop))
            .count(),
        1
    );
}

#[test]
fn census_is_bounded_exact_ordered_and_required_before_catalog_expansion() {
    let frozen = census(3);
    frozen.verify_observation(frozen.names()).unwrap();
    let mut drift = frozen.names().to_vec();
    drift.swap(0, 1);
    assert!(frozen.verify_observation(&drift).is_err());
    assert!(SchemaCensus::freeze(&drift).is_err());
    drift = frozen.names().to_vec();
    drift.push("Z".into());
    assert!(frozen.verify_observation(&drift).is_err());
    assert!(SchemaCensus::freeze(&[]).is_err());
    assert!(SchemaCensus::freeze(&["A".into()]).is_err());
    assert!(SchemaCensus::freeze(&[SCHEMA.into(), SCHEMA.into()]).is_err());
    assert!(SchemaCensus::freeze(&[SCHEMA.into(), "INFORMATION_SCHEMA".into()]).is_err());
    let mut overflow: Vec<_> = (0..32).map(|index| format!("A{index:02}")).collect();
    overflow.push(SCHEMA.into());
    assert!(SchemaCensus::freeze(&overflow).is_err());
    let escaped = SchemaCensus::freeze(&["A'quote".into(), SCHEMA.into()]).unwrap();
    let sql = escaped.catalog_relation_templates().unwrap();
    assert_eq!(sql.len(), 2);
    assert!(sql[0].contains("TABLE_SCHEMA = 'A''quote'"));
    assert!(sql[0].ends_with("LIMIT 10001"));
    assert_eq!(
        sql[1],
        browsing::relations_sql(&template_context(), SCHEMA).unwrap()
    );
}

#[test]
fn metadata_templates_use_production_sources_and_disclose_unexpanded_limits() {
    let plan = plan();
    let metadata = plan.metadata();
    assert_eq!(
        metadata.observation(),
        &[
            metadata::CONTEXT_SQL,
            metadata::QUOTED_IDENTIFIER_POLICY_SQL,
            snowflake_session::AUTOCOMMIT_SQL,
            snowflake_session::ABORT_ON_ERROR_SQL,
            snowflake_session::CURRENT_TRANSACTION_SQL
        ]
        .map(str::to_owned)
    );
    assert_eq!(
        metadata.schemas(),
        browsing::schemas_sql(&template_context()).unwrap()
    );
    let target = template_relation(TableRef {
        schema: SCHEMA.into(),
        name: plan.table().into(),
    });
    assert_eq!(
        metadata.exact_relation(),
        browsing::relation_sql(&template_context(), &target).unwrap()
    );
    assert_eq!(metadata.columns(), browsing::columns_sql(&target).unwrap());
    assert_eq!(
        metadata.describe(),
        format!("DESCRIBE TABLE {} TYPE = COLUMNS", plan.qualified_target())
    );
    assert_eq!(
        metadata.zero_row_resolution(),
        format!("SELECT * FROM {} LIMIT 0", plan.qualified_target())
    );
    assert_eq!(metadata.bounds().schema_sql_sentinel, 1001);
    assert_eq!(metadata.bounds().qualification_schema_cap, 32);
    assert!(!metadata.startup_sql_exported);
    assert!(!metadata.physical_ledger_expanded);
    assert!(plan.limits().startup_sql_not_exported);
    assert!(
        plan.limits()
            .physical_sql_admission_and_census_callback_missing
    );
    assert!(
        plan.limits()
            .one_use_approval_runner_and_durable_receipts_missing
    );
}

#[test]
fn deterministic_snapshot_hash_detects_byte_digest_and_plan_substitution() {
    let plan = plan();
    let bytes = plan.canonical_json().unwrap();
    let digest = plan.sha256().unwrap();
    assert_eq!(digest.len(), 64);
    assert_eq!(bytes, self::plan().canonical_json().unwrap());
    plan.verify_snapshot(&bytes, &digest).unwrap();
    let mut changed = bytes.clone();
    changed.push(b' ');
    assert!(plan.verify_snapshot(&changed, &digest).is_err());
    assert!(plan.verify_snapshot(&bytes, &"0".repeat(64)).is_err());
    let other = Plan::compile_run("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").unwrap();
    assert!(other.verify_snapshot(&bytes, &digest).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["phases"][29]["budget"]["fixed"] = 0.into();
    let tampered = serde_json::to_vec(&value).unwrap();
    let recomputed = format!("{:x}", Sha256::digest(&tampered));
    assert!(plan.verify_snapshot(&tampered, &recomputed).is_err());
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
        search_path: vec!["SANDBOX.BILLIE_TESTING".into()],
        context_fingerprint: Some("fingerprint".into()),
        transaction_observation: Some(TransactionObservation::Observed {
            transaction_id: None,
            abort_on_error: true,
            usability: TransactionUsability::Unknown,
        }),
        transaction_control_authority: Some(TransactionControlAuthority::Observed {
            revision: "revision".into(),
            allowed_actions: vec![
                ExplicitTransactionAction::Begin,
                ExplicitTransactionAction::Rollback,
            ],
        }),
    }
}

fn active(id: &str) -> SessionRuntimeState {
    let mut state = runtime();
    state.transaction_state = TransactionState::InTransaction;
    state.transaction_observation = Some(TransactionObservation::Observed {
        transaction_id: Some(id.into()),
        abort_on_error: true,
        usability: TransactionUsability::Unknown,
    });
    state.transaction_control_authority = Some(TransactionControlAuthority::Observed {
        revision: "active-revision".into(),
        allowed_actions: vec![
            ExplicitTransactionAction::Commit,
            ExplicitTransactionAction::Rollback,
        ],
    });
    state
}

#[test]
fn runtime_validators_require_pinned_idle_and_distinct_captured_transactions() {
    let initial = runtime();
    let pin = RuntimePin::capture(&initial).unwrap();
    ExpectedState::Idle
        .validate(&initial, &pin, None, None)
        .unwrap();
    let a = active("transaction-a");
    let b = active("transaction-b");
    assert_eq!(
        ExpectedState::NewTransaction(TransactionSlot::A)
            .validate(&a, &pin, None, None)
            .unwrap(),
        Some("transaction-a")
    );
    ExpectedState::SameTransaction(TransactionSlot::A)
        .validate(&a, &pin, Some("transaction-a"), None)
        .unwrap();
    assert!(
        ExpectedState::SameTransaction(TransactionSlot::A)
            .validate(&b, &pin, Some("transaction-a"), None)
            .is_err()
    );
    assert!(
        ExpectedState::NewTransaction(TransactionSlot::B)
            .validate(&a, &pin, Some("transaction-a"), None)
            .is_err()
    );
    ExpectedState::NewTransaction(TransactionSlot::B)
        .validate(&b, &pin, Some("transaction-a"), None)
        .unwrap();
    assert!(
        ExpectedState::NewTransaction(TransactionSlot::B)
            .validate(&b, &pin, None, None)
            .is_err()
    );
    assert!(ExpectedState::Idle.validate(&a, &pin, None, None).is_err());
    for mutate in [
        |state: &mut SessionRuntimeState| state.database = "OTHER".into(),
        |state: &mut SessionRuntimeState| state.role = Some("OTHER".into()),
        |state: &mut SessionRuntimeState| state.session_id = "other".into(),
        |state: &mut SessionRuntimeState| state.context_fingerprint = None,
        |state: &mut SessionRuntimeState| state.transaction_mode = TransactionMode::Manual,
        |state: &mut SessionRuntimeState| state.transaction_state = TransactionState::Failed,
        |state: &mut SessionRuntimeState| {
            state.transaction_observation = Some(TransactionObservation::Unavailable)
        },
        |state: &mut SessionRuntimeState| {
            state.transaction_control_authority = Some(TransactionControlAuthority::RollbackOnly {
                revision: "x".into(),
            })
        },
    ] {
        let mut state = initial.clone();
        mutate(&mut state);
        assert!(
            ExpectedState::Idle
                .validate(&state, &pin, None, None)
                .is_err()
        );
    }
}

#[test]
fn typed_effects_reject_unknown_rejected_retired_and_unavailable_summaries() {
    let plan = plan();
    let phase = &plan.phases()[2];
    let mut ddl = SnowflakeDdlResult {
        kind: SnowflakeDdlKind::CreateTable,
        qualified_target: plan.qualified_target().into(),
        statement_outcome: SnowflakeDdlOutcome::Succeeded,
        summary_status: SnowflakeDdlSummaryStatus::Acknowledged,
        catalog_reconciliation_required: true,
        state: runtime(),
        retired: false,
        error: None,
    };
    plan.validate_ddl_effect(phase, &ddl).unwrap();
    ddl.statement_outcome = SnowflakeDdlOutcome::Unknown;
    assert!(plan.validate_ddl_effect(phase, &ddl).is_err());
    ddl.statement_outcome = SnowflakeDdlOutcome::Rejected;
    assert!(plan.validate_ddl_effect(phase, &ddl).is_err());
    ddl.statement_outcome = SnowflakeDdlOutcome::Succeeded;
    ddl.retired = true;
    assert!(plan.validate_ddl_effect(phase, &ddl).is_err());
    let effect = ExpectedEffect::OneAffectedRow(SnowflakeDmlKind::Insert);
    let mut dml = SnowflakeDmlResult {
        kind: SnowflakeDmlKind::Insert,
        statement_outcome: SnowflakeDmlOutcome::Succeeded,
        summary: SnowflakeDmlSummary {
            status: SnowflakeDmlSummaryStatus::Validated,
            affected_rows: Some("1".into()),
            multi_joined_rows: None,
        },
        state: active("a"),
        retired: false,
        error: None,
    };
    effect.validate_dml(&dml).unwrap();
    for count in [None, Some("0"), Some("2"), Some("01")] {
        dml.summary.affected_rows = count.map(str::to_owned);
        assert!(effect.validate_dml(&dml).is_err());
    }
    dml.summary.affected_rows = Some("1".into());
    dml.summary.status = SnowflakeDmlSummaryStatus::Unavailable;
    assert!(effect.validate_dml(&dml).is_err());
    let mut boundary = TransactionControlResult {
        action: ExplicitTransactionAction::Rollback,
        boundary_outcome: TransactionBoundaryOutcome::Acknowledged,
        state: runtime(),
        retired: false,
        error: None,
    };
    let effect = ExpectedEffect::BoundaryAcknowledged(ExplicitTransactionAction::Rollback);
    effect.validate_boundary(&boundary).unwrap();
    boundary.boundary_outcome = TransactionBoundaryOutcome::Unknown;
    assert!(effect.validate_boundary(&boundary).is_err());
    boundary.boundary_outcome = TransactionBoundaryOutcome::Rejected;
    assert!(effect.validate_boundary(&boundary).is_err());
}

fn wire_columns(names: &[&str], kinds: &[ColumnKind]) -> Vec<ColumnInfo> {
    names
        .iter()
        .zip(kinds)
        .enumerate()
        .map(|(id, (name, kind))| ColumnInfo {
            id: id.to_string(),
            name: (*name).into(),
            database_type: "test".into(),
            kind: *kind,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        })
        .collect()
}

#[test]
fn fixed_readbacks_validate_complete_exact_rows_owner_traits_and_nulls() {
    let columns = wire_columns(
        &["ID", "NOTE", "ACTIVE", "EXTRA"],
        &[
            ColumnKind::Decimal,
            ColumnKind::Text,
            ColumnKind::Boolean,
            ColumnKind::Text,
        ],
    );
    let rows = vec![vec![
        Some("2".into()),
        Some("updated".into()),
        Some("false".into()),
        None,
    ]];
    Readback::UpdatedRow
        .validate(
            ReadResult {
                columns: &columns,
                rows: &rows,
                rows_seen: 1,
                truncated: false,
            },
            "ROLE",
        )
        .unwrap();
    assert!(
        Readback::CommittedRow
            .validate(
                ReadResult {
                    columns: &columns,
                    rows: &rows,
                    rows_seen: 1,
                    truncated: false
                },
                "ROLE"
            )
            .is_err()
    );
    assert!(
        Readback::UpdatedRow
            .validate(
                ReadResult {
                    columns: &columns,
                    rows: &rows,
                    rows_seen: 2,
                    truncated: false
                },
                "ROLE"
            )
            .is_err()
    );
    assert!(
        Readback::UpdatedRow
            .validate(
                ReadResult {
                    columns: &columns,
                    rows: &rows,
                    rows_seen: 1,
                    truncated: true
                },
                "ROLE"
            )
            .is_err()
    );
    Readback::Empty
        .validate(
            ReadResult {
                columns: &columns,
                rows: &[],
                rows_seen: 0,
                truncated: false,
            },
            "ROLE",
        )
        .unwrap();
    let owner_columns = wire_columns(
        &[
            "TABLE_OWNER",
            "TABLE_TYPE",
            "IS_TRANSIENT",
            "IS_TEMPORARY",
            "CREATED",
        ],
        &[ColumnKind::Text; 5],
    );
    for transient in ["YES", "NO"] {
        let rows = vec![vec![
            Some("ROLE".into()),
            Some("BASE TABLE".into()),
            Some(transient.into()),
            Some("NO".into()),
            Some("created-marker".into()),
        ]];
        let marker = Readback::Ownership
            .validate(
                ReadResult {
                    columns: &owner_columns,
                    rows: &rows,
                    rows_seen: 1,
                    truncated: false,
                },
                "ROLE",
            )
            .unwrap()
            .unwrap();
        assert_eq!(marker.owner(), "ROLE");
        assert_eq!(marker.transient(), transient == "YES");
        assert_eq!(marker.created(), "created-marker");
        assert!(
            Readback::Ownership
                .validate(
                    ReadResult {
                        columns: &owner_columns,
                        rows: &rows,
                        rows_seen: 1,
                        truncated: false
                    },
                    "OTHER"
                )
                .is_err()
        );
    }
}

fn inspection() -> RelationInspection {
    let columns = [
        ("ID", "NUMBER(38,0)"),
        ("NOTE", "VARCHAR(128)"),
        ("ACTIVE", "BOOLEAN"),
        ("EXTRA", "VARCHAR(32)"),
    ]
    .into_iter()
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
        revision: "inspection-revision".into(),
        target_id: "new-target".into(),
        database: DATABASE.into(),
        table: TableRef {
            schema: SCHEMA.into(),
            name: plan().table().into(),
        },
        columns,
        constraints: ConstraintInspectionStatus::NotFetched,
    }
}

#[test]
fn inspection_and_catalog_validators_only_accept_new_plain_exact_shape_target() {
    let plan = plan();
    let mut inspection = inspection();
    plan.validate_inspection(Shape::FourNullableColumns, &inspection)
        .unwrap();
    assert!(
        plan.validate_inspection(Shape::ThreeNullableColumns, &inspection)
            .is_err()
    );
    inspection.columns.pop();
    plan.validate_inspection(Shape::ThreeNullableColumns, &inspection)
        .unwrap();
    inspection.columns[1].nullable = false;
    assert!(
        plan.validate_inspection(Shape::ThreeNullableColumns, &inspection)
            .is_err()
    );
    inspection.columns[1].nullable = true;
    inspection.table.name = "EXISTING_TABLE".into();
    assert!(
        plan.validate_inspection(Shape::ThreeNullableColumns, &inspection)
            .is_err()
    );
    let initial = runtime();
    let pin = RuntimePin::capture(&initial).unwrap();
    let mut catalog = BrowseCatalog {
        revision: "catalog-revision".into(),
        database: DATABASE.into(),
        identity: CompletionIdentity {
            database: DATABASE.into(),
            role: "ROLE".into(),
            search_path: initial.search_path.clone(),
            case_sensitive_table_names: Some(true),
            context_fingerprint: initial.context_fingerprint.clone(),
        },
        relations: vec![],
    };
    plan.validate_catalog(Presence::FreshNameAbsent, &catalog, &pin)
        .unwrap();
    assert!(
        plan.validate_catalog(Presence::Present, &catalog, &pin)
            .is_err()
    );
    catalog.relations.push(
        template_relation(TableRef {
            schema: SCHEMA.into(),
            name: plan.table().into(),
        })
        .summary("target".into()),
    );
    for transient in [false, true] {
        catalog.relations[0].traits.transient = Some(transient);
        plan.validate_catalog(Presence::Present, &catalog, &pin)
            .unwrap();
    }
    assert!(
        plan.validate_catalog(Presence::DroppedNameAbsent, &catalog, &pin)
            .is_err()
    );
    catalog.relations[0].traits.transient = None;
    assert!(
        plan.validate_catalog(Presence::Present, &catalog, &pin)
            .is_err()
    );
}
