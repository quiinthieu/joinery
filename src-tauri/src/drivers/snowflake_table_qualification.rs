//! Offline, closed existing-schema qualification definition. This is not an
//! executable manifest, approval, SQL admission gate, or account runner.
//!
//! Only `Plan::generate` creates a plan. There is no caller-selected target,
//! SQL, transaction, or deserialization entry point. The census-bound expansion
//! describes exact production-derived SQL order and route-specific rejection
//! suffixes. Harness-only policy/Studio composition supplies physical admission,
//! lifecycle observations and durable receipts. Source/binary/private-connection
//! binding and an authorized account entry remain separate, unfinished gates.

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Offline ledger contract; no account runner entry point"
    )
)]
#[path = "snowflake_table_qualification_ledger.rs"]
mod ledger;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Offline validation; no account runner entry point"
    )
)]
#[path = "snowflake_table_qualification_wire.rs"]
mod wire;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Offline validation; no account runner entry point"
    )
)]
#[path = "snowflake_table_qualification_lifecycle.rs"]
mod lifecycle;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Internal qualification integration; no account runner entry point"
    )
)]
#[path = "snowflake_table_qualification_controls.rs"]
mod controls;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Internal qualification integration; no account runner entry point"
    )
)]
#[path = "snowflake_table_qualification_journal.rs"]
mod journal;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Internal qualification integration; no account runner entry point"
    )
)]
#[path = "snowflake_table_qualification_sequence.rs"]
mod sequence;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Internal qualification integration; no account runner entry point"
    )
)]
#[path = "snowflake_table_qualification_evidence.rs"]
mod evidence;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Internal qualification integration; no account runner entry point"
    )
)]
#[path = "snowflake_table_qualification_policy.rs"]
mod policy;

use super::{snowflake_browsing as browsing, snowflake_metadata as metadata, snowflake_session};
use crate::{
    domain::{
        AppError, AppResult, ColumnInfo, ColumnKind, DatabaseObjectKind, DatabaseObjectRef,
        ExplicitTransactionAction, SessionContextKind, SessionRuntimeState, SnowflakeDdlAction,
        SnowflakeDdlColumnDraft, SnowflakeDdlKind, SnowflakeDdlOutcome, SnowflakeDdlResult,
        SnowflakeDdlSummaryStatus, SnowflakeDmlKind, SnowflakeDmlOutcome, SnowflakeDmlResult,
        SnowflakeDmlSummaryStatus, TableColumnDraft, TableObjectAction, TableRef, TextRow,
        TransactionBoundaryOutcome, TransactionControlAuthority, TransactionControlResult,
        TransactionMode, TransactionObservation, TransactionState,
    },
    params::TypedBind,
    relation_inspection::{
        BrowseCatalog, InspectedColumnKind, RelationAvailability, RelationInspection,
        RelationTraits,
    },
    safety::{snowflake_ddl, snowflake_dml},
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const DATABASE: &str = "SANDBOX";
pub const SCHEMA: &str = "BILLIE_TESTING";
pub const MAX_SCHEMAS: usize = 32;
pub const PHASE_COUNT: usize = 32;

fn invalid() -> AppError {
    AppError::invalid("Existing-schema qualification evidence does not match the closed plan")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StatementId {
    Create,
    Add,
    Drop,
    RollbackInsert,
    CommittedInsert,
    Update,
    Delete,
    OwnershipRead,
    DataRead,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrozenStatement {
    id: StatementId,
    source_sql: String,
    wire_sql: String,
    binds: Vec<TypedBind>,
}

impl FrozenStatement {
    pub fn id(&self) -> StatementId {
        self.id
    }
    pub fn source_sql(&self) -> &str {
        &self.source_sql
    }
    pub fn wire_sql(&self) -> &str {
        &self.wire_sql
    }
    pub fn binds(&self) -> &[TypedBind] {
        &self.binds
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TransactionSlot {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ExpectedState {
    Idle,
    NewTransaction(TransactionSlot),
    SameTransaction(TransactionSlot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Readback {
    Ownership,
    Empty,
    RollbackRow,
    CommittedRow,
    UpdatedRow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Shape {
    ThreeNullableColumns,
    FourNullableColumns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Presence {
    FreshNameAbsent,
    Present,
    DroppedNameAbsent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Operation {
    /// The shared helper identity SQL followed by the adapter's version read.
    StartupIdentityAndVersion,
    DdlPreflight(StatementId),
    ReviewedDdl(StatementId),
    CoherentReconciliation(Presence),
    Catalog(Presence),
    Inspect(Shape),
    Read(Readback),
    TypedBoundary {
        action: ExplicitTransactionAction,
        slot: TransactionSlot,
    },
    ReviewedDml(StatementId),
    SessionOptionsResume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ExpectedEffect {
    None,
    DdlAcknowledged(SnowflakeDdlKind),
    OneAffectedRow(SnowflakeDmlKind),
    BoundaryAcknowledged(ExplicitTransactionAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SqlBudget {
    fixed: u16,
    per_schema: u8,
}

impl SqlBudget {
    pub fn fixed(&self) -> u16 {
        self.fixed
    }
    pub fn per_schema(&self) -> u8 {
        self.per_schema
    }
    pub fn total(&self, census: &SchemaCensus) -> u16 {
        self.fixed + u16::from(self.per_schema) * census.names.len() as u16
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AdvancementRule {
    /// A runner must retain the original ACK/outcome while this call completes,
    /// then sync its receipt and all validation before admitting the next phase.
    AllPriorPhasesValidatedAndOriginalReceiptsSynced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FailureRule {
    /// Stop phase advancement; only this active call's frozen, already-budgeted
    /// read-only post-observer suffix may finish. Never cleanup, retry or replay.
    OnlyActiveFrozenReadOnlyPostObserverSuffix,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase {
    number: u8,
    operation: Operation,
    state: ExpectedState,
    effect: ExpectedEffect,
    presence: Option<Presence>,
    budget: SqlBudget,
    advancement: AdvancementRule,
    failure: FailureRule,
}

impl Phase {
    pub fn number(&self) -> u8 {
        self.number
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }
    pub fn state(&self) -> ExpectedState {
        self.state
    }
    pub fn effect(&self) -> ExpectedEffect {
        self.effect
    }
    pub fn presence(&self) -> Option<Presence> {
        self.presence
    }
    pub fn budget(&self) -> SqlBudget {
        self.budget
    }
    pub fn advancement(&self) -> AdvancementRule {
        self.advancement
    }
    pub fn failure(&self) -> FailureRule {
        self.failure
    }
}

/// These are shared SQL templates, NOT an expanded per-phase physical ledger.
/// The separate census-bound ledger expands these into ordered invocations.
/// Every expansion must still be privately recorded before its physical write;
/// physical dispatch/order/count enforcement remains absent.
#[derive(Debug, Clone, Serialize)]
pub struct MetadataTemplates {
    observation: [String; 5],
    schemas: String,
    exact_relation: String,
    columns: String,
    describe: String,
    zero_row_resolution: String,
    startup_sql_exported: bool,
    physical_ledger_expanded: bool,
    bounds: MetadataBounds,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetadataBounds {
    pub schema_sql_sentinel: usize,
    pub qualification_schema_cap: usize,
    pub catalog_relations: usize,
    pub catalog_bytes: usize,
    pub inspection_columns: usize,
    pub inspection_bytes: usize,
    pub sample_rows: usize,
    pub sample_bytes: usize,
}

impl MetadataTemplates {
    pub fn observation(&self) -> &[String; 5] {
        &self.observation
    }
    pub fn schemas(&self) -> &str {
        &self.schemas
    }
    pub fn exact_relation(&self) -> &str {
        &self.exact_relation
    }
    pub fn columns(&self) -> &str {
        &self.columns
    }
    pub fn describe(&self) -> &str {
        &self.describe
    }
    pub fn zero_row_resolution(&self) -> &str {
        &self.zero_row_resolution
    }
    pub fn bounds(&self) -> &MetadataBounds {
        &self.bounds
    }
}

/// Freeze the exact observed ordered census, not an object selector. The future
/// callback must intercept each raw production schema response before any parser
/// discards order, before CREATE and before downstream catalog dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchemaCensus {
    names: Vec<String>,
}

impl SchemaCensus {
    pub fn freeze(names: &[String]) -> AppResult<Self> {
        if names.is_empty()
            || names.len() > MAX_SCHEMAS
            || names.windows(2).any(|pair| pair[0] >= pair[1])
            || !names.iter().any(|name| name == SCHEMA)
        {
            return Err(invalid());
        }
        let context = template_context();
        for name in names {
            // Reuse the production schema/identifier policy and literal escaping.
            browsing::relations_sql(&context, name)?;
        }
        Ok(Self {
            names: names.to_vec(),
        })
    }
    pub fn names(&self) -> &[String] {
        &self.names
    }
    pub fn verify_observation(&self, names: &[String]) -> AppResult<()> {
        if self.names != names {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn catalog_relation_templates(&self) -> AppResult<Vec<String>> {
        let context = template_context();
        self.names
            .iter()
            .map(|schema| browsing::relations_sql(&context, schema))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationLimits {
    pub startup_sql_not_exported: bool,
    pub physical_sql_admission_and_census_callback_missing: bool,
    pub inbound_byte_frame_counters_missing: bool,
    pub raw_catalog_creation_marker_pinning_missing: bool,
    pub source_binary_runtime_private_connection_envelope_missing: bool,
    pub one_use_approval_runner_and_durable_receipts_missing: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    version: u8,
    run: String,
    database: &'static str,
    schema: &'static str,
    table: String,
    qualified_target: String,
    statements: Vec<FrozenStatement>,
    phases: Vec<Phase>,
    metadata: MetadataTemplates,
    total_budget: SqlBudget,
    max_schemas: usize,
    limits: IntegrationLimits,
}

impl Plan {
    /// UUID v4 uses the crate's OS-backed cryptographic randomness; its reserved
    /// version/variant bits do not change the 32-hex representation or nonzero rule.
    pub fn generate() -> AppResult<Self> {
        Self::compile_run(
            &uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .to_ascii_uppercase(),
        )
    }

    // Deliberately private. There is no deterministic public factory accepting
    // an existing name, persisted token, object, SQL, or arbitrary target.
    fn compile_run(run: &str) -> AppResult<Self> {
        if run.len() != 32
            || !run
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
            || run.bytes().all(|byte| byte == b'0')
        {
            return Err(invalid());
        }
        let table = format!("JOINERY_QUAL_{run}");
        let target = TableRef {
            schema: SCHEMA.into(),
            name: table.clone(),
        };
        let qualified_target = format!(
            "{}.{}.{}",
            metadata::quote_identifier(DATABASE)?,
            metadata::quote_identifier(SCHEMA)?,
            metadata::quote_identifier(&table)?
        );
        let mut statements = Vec::new();
        for id in [StatementId::Create, StatementId::Add, StatementId::Drop] {
            let action = fixed_ddl_action(id, target.clone())?;
            let canonical = snowflake_ddl::compile(&action, DATABASE)?;
            if canonical.qualified_target()
                != &[DATABASE.to_string(), SCHEMA.to_string(), table.clone()]
            {
                return Err(invalid());
            }
            statements.push(frozen(id, canonical.sql().to_owned()));
        }
        let dml = [
            (
                StatementId::RollbackInsert,
                SnowflakeDmlKind::Insert,
                format!(
                    "INSERT INTO {qualified_target} (\"ID\", \"NOTE\", \"ACTIVE\") VALUES (1, 'rollback-only', TRUE)"
                ),
            ),
            (
                StatementId::CommittedInsert,
                SnowflakeDmlKind::Insert,
                format!(
                    "INSERT INTO {qualified_target} (\"ID\", \"NOTE\", \"ACTIVE\") VALUES (2, 'committed', TRUE)"
                ),
            ),
            (
                StatementId::Update,
                SnowflakeDmlKind::Update,
                format!(
                    "UPDATE {qualified_target} SET \"NOTE\" = 'updated', \"ACTIVE\" = FALSE WHERE \"ID\" = 2"
                ),
            ),
            (
                StatementId::Delete,
                SnowflakeDmlKind::Delete,
                format!("DELETE FROM {qualified_target} WHERE \"ID\" = 2"),
            ),
        ];
        for (id, kind, sql) in dml {
            let canonical = snowflake_dml::compile(&sql, None, DATABASE)?;
            if canonical.kind() != kind
                || !canonical.validated_inputs().is_empty()
                || !canonical.bound_statement().binds.is_empty()
                || canonical.qualified_target()
                    != &[DATABASE.to_string(), SCHEMA.to_string(), table.clone()]
            {
                return Err(invalid());
            }
            statements.push(FrozenStatement {
                id,
                source_sql: canonical.original_sql().into(),
                wire_sql: canonical.bound_statement().rewritten_sql.clone(),
                binds: canonical.bound_statement().binds.clone(),
            });
        }
        // Only the internally generated uppercase-hex name is interpolated. No
        // caller string/literal or existing object can reach these two templates.
        statements.push(frozen(StatementId::OwnershipRead, format!("SELECT TABLE_OWNER, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, CREATED::VARCHAR AS CREATED FROM \"SANDBOX\".INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = 'SANDBOX' AND TABLE_SCHEMA = 'BILLIE_TESTING' AND TABLE_NAME = '{table}' ORDER BY TABLE_NAME LIMIT 2")));
        statements.push(frozen(StatementId::DataRead, format!("SELECT \"ID\", \"NOTE\", \"ACTIVE\", \"EXTRA\" FROM {qualified_target} ORDER BY \"ID\" LIMIT 2")));
        let relation = template_relation(target);
        let context = template_context();
        Ok(Self {
            version: 1,
            run: run.into(),
            database: DATABASE,
            schema: SCHEMA,
            table,
            qualified_target,
            statements,
            phases: phases(),
            metadata: MetadataTemplates {
                observation: snowflake_session::sample_reads().map(|spec| spec.sql().into()),
                schemas: browsing::schemas_sql(&context)?,
                exact_relation: browsing::relation_sql(&context, &relation)?,
                columns: browsing::columns_sql(&relation)?,
                describe: browsing::describe_sql(&relation)?,
                zero_row_resolution: browsing::resolved_columns_sql(&relation)?,
                startup_sql_exported: true,
                physical_ledger_expanded: false,
                bounds: MetadataBounds {
                    schema_sql_sentinel: browsing::SCHEMA_LIMIT + 1,
                    qualification_schema_cap: MAX_SCHEMAS,
                    catalog_relations: browsing::RELATION_LIMIT,
                    catalog_bytes: browsing::CATALOG_MAX_BYTES,
                    inspection_columns: browsing::COLUMN_LIMIT,
                    inspection_bytes: browsing::COLUMN_MAX_BYTES,
                    sample_rows: 1,
                    sample_bytes: metadata::CONTEXT_MAX_BYTES,
                },
            },
            total_budget: SqlBudget {
                fixed: 386,
                per_schema: 7,
            },
            max_schemas: MAX_SCHEMAS,
            limits: IntegrationLimits {
                startup_sql_not_exported: false,
                physical_sql_admission_and_census_callback_missing: true,
                inbound_byte_frame_counters_missing: true,
                raw_catalog_creation_marker_pinning_missing: true,
                source_binary_runtime_private_connection_envelope_missing: true,
                one_use_approval_runner_and_durable_receipts_missing: true,
            },
        })
    }
    pub fn run(&self) -> &str {
        &self.run
    }
    pub fn table(&self) -> &str {
        &self.table
    }
    pub fn qualified_target(&self) -> &str {
        &self.qualified_target
    }
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }
    pub fn statements(&self) -> &[FrozenStatement] {
        &self.statements
    }
    pub fn metadata(&self) -> &MetadataTemplates {
        &self.metadata
    }
    pub fn limits(&self) -> &IntegrationLimits {
        &self.limits
    }
    pub fn budget(&self, census: &SchemaCensus) -> u16 {
        self.total_budget.total(census)
    }
    pub fn statement(&self, id: StatementId) -> &FrozenStatement {
        // Every closed ID is populated by the sole private compiler.
        self.statements
            .iter()
            .find(|statement| statement.id == id)
            .expect("closed statement ID")
    }

    /// Build only this plan's three fixed DDL actions. The runner must supply
    /// its retained, Policy-validated phase 6/7 evidence for ADD and phase 28
    /// evidence for DROP. DTOs alone cannot authenticate their provenance or
    /// distinguish a coherent old pair from current observations. The adapter
    /// still checks live revision authority during preflight and execution.
    /// CREATE's fresh-name absence is checked by its actual preflight, not here.
    pub fn ddl_action(
        &self,
        id: StatementId,
        catalog: Option<&BrowseCatalog>,
        inspection: Option<&RelationInspection>,
    ) -> AppResult<SnowflakeDdlAction> {
        let target = TableRef {
            schema: SCHEMA.into(),
            name: self.table.clone(),
        };
        let fixed = fixed_ddl_action(id, target)?;
        let relation = match id {
            StatementId::Create if catalog.is_none() && inspection.is_none() => None,
            StatementId::Add | StatementId::Drop => {
                let catalog = catalog.ok_or_else(invalid)?;
                self.validate_catalog_identity(
                    Presence::Present,
                    catalog,
                    &catalog.identity.role,
                    catalog
                        .identity
                        .context_fingerprint
                        .as_deref()
                        .ok_or_else(invalid)?,
                )?;
                let relation = catalog
                    .relations
                    .iter()
                    .find(|relation| {
                        relation.database == DATABASE
                            && relation.table.schema == SCHEMA
                            && relation.table.name == self.table
                    })
                    .ok_or_else(invalid)?;
                // A target token must identify exactly this catalog member.
                if catalog
                    .relations
                    .iter()
                    .filter(|other| other.target_id == relation.target_id)
                    .count()
                    != 1
                {
                    return Err(invalid());
                }
                match (id, inspection) {
                    (StatementId::Add, Some(inspection)) => {
                        self.validate_inspection(Shape::ThreeNullableColumns, inspection)?;
                        if inspection.target_id != relation.target_id {
                            return Err(invalid());
                        }
                    }
                    (StatementId::Drop, None) => {}
                    _ => return Err(invalid()),
                }
                Some((catalog, relation))
            }
            _ => return Err(invalid()),
        };
        match fixed {
            TableObjectAction::CreateTable { table, columns, .. } => {
                Ok(SnowflakeDdlAction::CreateTable {
                    database: DATABASE.into(),
                    schema: table.schema,
                    name: table.name,
                    columns: columns.into_iter().map(ddl_column).collect(),
                })
            }
            TableObjectAction::AddColumn { column, .. } => {
                let (catalog, relation) = relation.ok_or_else(invalid)?;
                Ok(SnowflakeDdlAction::AddColumn {
                    browse_revision: catalog.revision.clone(),
                    target_id: relation.target_id.clone(),
                    inspection_revision: inspection.ok_or_else(invalid)?.revision.clone(),
                    column: ddl_column(column),
                })
            }
            TableObjectAction::DropObject { .. } => {
                let (catalog, relation) = relation.ok_or_else(invalid)?;
                Ok(SnowflakeDdlAction::DropTable {
                    browse_revision: catalog.revision.clone(),
                    target_id: relation.target_id.clone(),
                })
            }
            _ => Err(invalid()),
        }
    }
    pub fn canonical_json(&self) -> AppResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(|_| invalid())
    }
    pub fn sha256(&self) -> AppResult<String> {
        Ok(format!("{:x}", Sha256::digest(self.canonical_json()?)))
    }
    /// Tamper detection against this in-memory definition, not authentication,
    /// source/runtime binding, execution authority or a loadable manifest.
    pub fn verify_snapshot(&self, bytes: &[u8], sha256: &str) -> AppResult<()> {
        if self.canonical_json()? != bytes || self.sha256()? != sha256 {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn validate_ddl_effect(&self, phase: &Phase, result: &SnowflakeDdlResult) -> AppResult<()> {
        if phase.effect != ExpectedEffect::DdlAcknowledged(result.kind)
            || result.qualified_target != self.qualified_target
            || result.statement_outcome != SnowflakeDdlOutcome::Succeeded
            || result.summary_status != SnowflakeDdlSummaryStatus::Acknowledged
            || !result.catalog_reconciliation_required
            || result.retired
            || result.error.is_some()
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn validate_inspection(
        &self,
        shape: Shape,
        inspection: &RelationInspection,
    ) -> AppResult<()> {
        let expected = match shape {
            Shape::ThreeNullableColumns => 3,
            Shape::FourNullableColumns => 4,
        };
        if inspection.database != DATABASE
            || inspection.table.schema != SCHEMA
            || inspection.table.name != self.table
            || inspection.revision.is_empty()
            || inspection.target_id.is_empty()
            || inspection.columns.len() != expected
        {
            return Err(invalid());
        }
        for (index, actual) in inspection.columns.iter().enumerate() {
            let (name, full_type) = [
                ("ID", "NUMBER(38,0)"),
                ("NOTE", "VARCHAR(128)"),
                ("ACTIVE", "BOOLEAN"),
                ("EXTRA", "VARCHAR(32)"),
            ][index];
            let result_kind_matches = match index {
                0 => matches!(
                    actual.result_kind,
                    ColumnKind::Integer | ColumnKind::Decimal
                ),
                1 | 3 => actual.result_kind == ColumnKind::Text,
                2 => actual.result_kind == ColumnKind::Boolean,
                _ => false,
            };
            if actual.name != name
                || actual.ordinal != index as u32 + 1
                || !actual.nullable
                || actual.full_type.replace(' ', "").to_ascii_uppercase() != full_type
                || actual.kind != InspectedColumnKind::Column
                || !result_kind_matches
                || actual.identity.is_identity != Some(false)
                || actual.default_expression.is_some()
                || actual.descriptor_default_expression.is_some()
                || actual.write_default_expression.is_some()
                || actual.expression.is_some()
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    /// Public catalog evidence cannot pin CREATED/LAST_DDL (they are omitted
    /// from its DTO). The future raw observer must also check those markers.
    pub fn validate_catalog(
        &self,
        presence: Presence,
        catalog: &BrowseCatalog,
        pin: &RuntimePin,
    ) -> AppResult<()> {
        self.validate_catalog_identity(
            presence,
            catalog,
            pin.initial.role.as_deref().ok_or_else(invalid)?,
            pin.initial
                .context_fingerprint
                .as_deref()
                .ok_or_else(invalid)?,
        )
    }

    pub(super) fn validate_catalog_identity(
        &self,
        presence: Presence,
        catalog: &BrowseCatalog,
        role: &str,
        context_fingerprint: &str,
    ) -> AppResult<()> {
        if catalog.revision.is_empty()
            || catalog.database != DATABASE
            || catalog.identity.database != DATABASE
            || role.is_empty()
            || context_fingerprint.is_empty()
            || catalog.identity.role != role
            || catalog.identity.context_fingerprint.as_deref() != Some(context_fingerprint)
        {
            return Err(invalid());
        }
        let matching: Vec<_> = catalog
            .relations
            .iter()
            .filter(|relation| {
                relation.database == DATABASE
                    && relation.table.schema == SCHEMA
                    && relation.table.name == self.table
            })
            .collect();
        match presence {
            Presence::FreshNameAbsent | Presence::DroppedNameAbsent if matching.is_empty() => {
                Ok(())
            }
            Presence::Present if matching.len() == 1 => {
                let relation = matching[0];
                if relation.target_id.is_empty()
                    || relation.table_type != "BASE TABLE"
                    || relation.availability != RelationAvailability::Available
                    || relation.traits.temporary != Some(false)
                    || relation.traits.transient.is_none()
                    || relation.traits.iceberg != Some(false)
                    || relation.traits.dynamic != Some(false)
                    || relation.traits.hybrid != Some(false)
                    || relation.traits.immutable != Some(false)
                {
                    return Err(invalid());
                }
                Ok(())
            }
            _ => Err(invalid()),
        }
    }
}

impl ExpectedEffect {
    pub fn validate_dml(self, result: &SnowflakeDmlResult) -> AppResult<()> {
        if self != Self::OneAffectedRow(result.kind)
            || result.statement_outcome != SnowflakeDmlOutcome::Succeeded
            || result.summary.status != SnowflakeDmlSummaryStatus::Validated
            || result.summary.affected_rows.as_deref() != Some("1")
            || !matches!(
                result.summary.multi_joined_rows.as_deref(),
                None | Some("0")
            )
            || result.retired
            || result.error.is_some()
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn validate_boundary(self, result: &TransactionControlResult) -> AppResult<()> {
        if self != Self::BoundaryAcknowledged(result.action)
            || result.boundary_outcome != TransactionBoundaryOutcome::Acknowledged
            || result.retired
            || result.error.is_some()
        {
            return Err(invalid());
        }
        Ok(())
    }
}

/// Pins only what the public runtime DTO actually exposes. Principal, warehouse,
/// secondary roles and raw catalog creation markers need the future low-level
/// integration; this type must not be treated as the complete connection envelope.
#[derive(Debug, Clone)]
pub struct RuntimePin {
    initial: SessionRuntimeState,
    abort_on_error: bool,
}

impl RuntimePin {
    pub fn capture(initial: &SessionRuntimeState) -> AppResult<Self> {
        let Some(TransactionObservation::Observed {
            transaction_id: None,
            abort_on_error,
            ..
        }) = &initial.transaction_observation
        else {
            return Err(invalid());
        };
        if initial.session_id.is_empty()
            || initial.role.as_deref().is_none_or(str::is_empty)
            || initial
                .context_fingerprint
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(invalid());
        }
        let pin = Self {
            initial: initial.clone(),
            abort_on_error: *abort_on_error,
        };
        ExpectedState::Idle.validate(initial, &pin, None, None)?;
        Ok(pin)
    }
}

impl ExpectedState {
    /// Returns a newly observed transaction ID for BEGIN; same-transaction phases
    /// require the captured slot. New B must differ from the already observed A.
    pub fn validate<'a>(
        self,
        state: &'a SessionRuntimeState,
        pin: &RuntimePin,
        transaction_a: Option<&str>,
        transaction_b: Option<&str>,
    ) -> AppResult<Option<&'a str>> {
        if state.session_id != pin.initial.session_id
            || state.database != DATABASE
            || state.role != pin.initial.role
            || state.context_kind != SessionContextKind::Schema
            || state.active_context.as_deref() != Some(SCHEMA)
            || state.search_path != pin.initial.search_path
            || state.context_fingerprint != pin.initial.context_fingerprint
            || state.transaction_mode != TransactionMode::AutoCommit
        {
            return Err(invalid());
        }
        let Some(TransactionObservation::Observed {
            transaction_id,
            abort_on_error,
            ..
        }) = &state.transaction_observation
        else {
            return Err(invalid());
        };
        if *abort_on_error != pin.abort_on_error {
            return Err(invalid());
        }
        let Some(TransactionControlAuthority::Observed {
            revision,
            allowed_actions,
        }) = &state.transaction_control_authority
        else {
            return Err(invalid());
        };
        if revision.is_empty() {
            return Err(invalid());
        }
        match self {
            Self::Idle
                if state.transaction_state == TransactionState::Idle
                    && transaction_id.is_none()
                    && allowed_actions.len() == 2
                    && allowed_actions.contains(&ExplicitTransactionAction::Begin)
                    && allowed_actions.contains(&ExplicitTransactionAction::Rollback) =>
            {
                Ok(None)
            }
            Self::NewTransaction(slot) | Self::SameTransaction(slot) => {
                let id = transaction_id
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(invalid)?;
                if state.transaction_state != TransactionState::InTransaction
                    || allowed_actions.len() != 2
                    || !allowed_actions.contains(&ExplicitTransactionAction::Commit)
                    || !allowed_actions.contains(&ExplicitTransactionAction::Rollback)
                {
                    return Err(invalid());
                }
                let captured = match slot {
                    TransactionSlot::A => transaction_a,
                    TransactionSlot::B => transaction_b,
                };
                match self {
                    Self::SameTransaction(_) if captured != Some(id) => return Err(invalid()),
                    Self::NewTransaction(TransactionSlot::A) if captured.is_some() => {
                        return Err(invalid());
                    }
                    Self::NewTransaction(TransactionSlot::B)
                        if captured.is_some()
                            || transaction_a.is_none()
                            || transaction_a == Some(id) =>
                    {
                        return Err(invalid());
                    }
                    _ => {}
                }
                Ok(Some(id))
            }
            _ => Err(invalid()),
        }
    }
}

/// Terminal collector evidence, including discarded row count. A future runner
/// must obtain these fields from its real collector rather than assume complete.
pub struct ReadResult<'a> {
    pub columns: &'a [ColumnInfo],
    pub rows: &'a [TextRow],
    pub rows_seen: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipMarker {
    owner: String,
    transient: bool,
    created: String,
}

impl OwnershipMarker {
    pub fn owner(&self) -> &str {
        &self.owner
    }
    pub fn transient(&self) -> bool {
        self.transient
    }
    pub fn created(&self) -> &str {
        &self.created
    }
}

impl Readback {
    pub fn validate(
        self,
        result: ReadResult<'_>,
        pinned_primary_role: &str,
    ) -> AppResult<Option<OwnershipMarker>> {
        if result.truncated || result.rows_seen != result.rows.len() as u64 {
            return Err(invalid());
        }
        let names: Vec<_> = result
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect();
        if self == Self::Ownership {
            if names
                != [
                    "TABLE_OWNER",
                    "TABLE_TYPE",
                    "IS_TRANSIENT",
                    "IS_TEMPORARY",
                    "CREATED",
                ]
                || result.rows.len() != 1
            {
                return Err(invalid());
            }
            let row = &result.rows[0];
            if row.len() != 5
                || pinned_primary_role.is_empty()
                || row[0].as_deref() != Some(pinned_primary_role)
                || row[1].as_deref() != Some("BASE TABLE")
                || row[3].as_deref() != Some("NO")
                || !matches!(row[2].as_deref(), Some("YES" | "NO"))
                || row[4].as_deref().is_none_or(str::is_empty)
            {
                return Err(invalid());
            }
            return Ok(Some(OwnershipMarker {
                owner: pinned_primary_role.into(),
                transient: row[2].as_deref() == Some("YES"),
                created: row[4].clone().ok_or_else(invalid)?,
            }));
        }
        if names != ["ID", "NOTE", "ACTIVE", "EXTRA"]
            || result.columns.iter().any(|column| !column.nullable)
            || !matches!(
                result.columns[0].kind,
                ColumnKind::Integer | ColumnKind::Decimal
            )
            || result.columns[1].kind != ColumnKind::Text
            || result.columns[2].kind != ColumnKind::Boolean
            || result.columns[3].kind != ColumnKind::Text
        {
            return Err(invalid());
        }
        let expected: Vec<TextRow> = match self {
            Self::Empty => Vec::new(),
            Self::RollbackRow => vec![vec![
                Some("1".into()),
                Some("rollback-only".into()),
                Some("true".into()),
                None,
            ]],
            Self::CommittedRow => vec![vec![
                Some("2".into()),
                Some("committed".into()),
                Some("true".into()),
                None,
            ]],
            Self::UpdatedRow => vec![vec![
                Some("2".into()),
                Some("updated".into()),
                Some("false".into()),
                None,
            ]],
            Self::Ownership => return Err(invalid()),
        };
        if result.rows != expected {
            return Err(invalid());
        }
        Ok(None)
    }
}

fn fixed_ddl_action(id: StatementId, table: TableRef) -> AppResult<TableObjectAction> {
    match id {
        StatementId::Create => Ok(TableObjectAction::CreateTable {
            table,
            columns: vec![
                column("ID", "NUMBER(38,0)"),
                column("NOTE", "VARCHAR(128)"),
                column("ACTIVE", "BOOLEAN"),
            ],
            comment: None,
        }),
        StatementId::Add => Ok(TableObjectAction::AddColumn {
            table,
            column: column("EXTRA", "VARCHAR(32)"),
        }),
        StatementId::Drop => Ok(TableObjectAction::DropObject {
            object: DatabaseObjectRef {
                schema: table.schema,
                name: table.name,
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
            cascade: false,
        }),
        _ => Err(invalid()),
    }
}

fn ddl_column(column: TableColumnDraft) -> SnowflakeDdlColumnDraft {
    SnowflakeDdlColumnDraft {
        name: column.name,
        data_type: column.data_type,
        nullable: column.nullable,
    }
}

fn column(name: &str, data_type: &str) -> TableColumnDraft {
    TableColumnDraft {
        name: name.into(),
        data_type: data_type.into(),
        nullable: true,
        default_expression: None,
        auto_increment: false,
        primary_key: false,
    }
}

fn frozen(id: StatementId, sql: String) -> FrozenStatement {
    FrozenStatement {
        id,
        source_sql: sql.clone(),
        wire_sql: sql,
        binds: Vec::new(),
    }
}

// Synthetic template inputs only: these values are not a live context, identity,
// object observation, fingerprint, or evidence of permission/existence.
fn template_context() -> metadata::LiveContext {
    metadata::LiveContext {
        principal: String::new(),
        role: None,
        database: Some(DATABASE.into()),
        schema: Some(SCHEMA.into()),
        warehouse: None,
        session_id: String::new(),
        search_path: Vec::new(),
        secondary_roles: metadata::SecondaryRoles {
            roles: String::new(),
            value: String::new(),
        },
        quoted_identifiers_ignore_case: false,
    }
}

fn template_relation(table: TableRef) -> browsing::BrowseRelation {
    browsing::BrowseRelation {
        database: DATABASE.into(),
        table,
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
        created: None,
        last_ddl: None,
    }
}

fn phases() -> Vec<Phase> {
    use ExpectedState::{Idle, NewTransaction, SameTransaction};
    use ExplicitTransactionAction::{Begin, Commit, Rollback};
    use Operation::*;
    use Presence::*;
    use StatementId::*;
    use TransactionSlot::{A, B};
    // Explicit ordered successful path. Failure never enters a later phase,
    // especially DROP; its only admitted continuation is the active call suffix.
    let definitions = [
        (StartupIdentityAndVersion, Idle, 2, 0),
        (DdlPreflight(Create), Idle, 12, 0),
        (ReviewedDdl(Create), Idle, 18, 0),
        (CoherentReconciliation(Present), Idle, 27, 1),
        (Read(Readback::Ownership), Idle, 11, 0),
        (Catalog(Present), Idle, 5, 1),
        (Inspect(Shape::ThreeNullableColumns), Idle, 9, 0),
        (DdlPreflight(Add), Idle, 15, 0),
        (ReviewedDdl(Add), Idle, 21, 0),
        (CoherentReconciliation(Present), Idle, 27, 1),
        (Catalog(Present), Idle, 5, 1),
        (Inspect(Shape::FourNullableColumns), Idle, 9, 0),
        (Read(Readback::Empty), Idle, 11, 0),
        (
            TypedBoundary {
                action: Begin,
                slot: A,
            },
            NewTransaction(A),
            11,
            0,
        ),
        (ReviewedDml(RollbackInsert), SameTransaction(A), 11, 0),
        (Read(Readback::RollbackRow), SameTransaction(A), 11, 0),
        (
            TypedBoundary {
                action: Rollback,
                slot: A,
            },
            Idle,
            6,
            0,
        ),
        (Read(Readback::Empty), Idle, 11, 0),
        (
            TypedBoundary {
                action: Begin,
                slot: B,
            },
            NewTransaction(B),
            11,
            0,
        ),
        (ReviewedDml(CommittedInsert), SameTransaction(B), 11, 0),
        (
            TypedBoundary {
                action: Commit,
                slot: B,
            },
            Idle,
            11,
            0,
        ),
        (Read(Readback::CommittedRow), Idle, 11, 0),
        (ReviewedDml(Update), Idle, 11, 0),
        (Read(Readback::UpdatedRow), Idle, 11, 0),
        (ReviewedDml(Delete), Idle, 11, 0),
        (Read(Readback::Empty), Idle, 11, 0),
        (SessionOptionsResume, Idle, 11, 0),
        (Catalog(Present), Idle, 5, 1),
        (DdlPreflight(Drop), Idle, 11, 0),
        (ReviewedDdl(Drop), Idle, 17, 0),
        (CoherentReconciliation(DroppedNameAbsent), Idle, 27, 1),
        (Catalog(DroppedNameAbsent), Idle, 5, 1),
    ];
    definitions
        .into_iter()
        .enumerate()
        .map(|(index, (operation, state, fixed, per_schema))| {
            let effect = match operation {
                ReviewedDdl(Create) => {
                    ExpectedEffect::DdlAcknowledged(SnowflakeDdlKind::CreateTable)
                }
                ReviewedDdl(Add) => ExpectedEffect::DdlAcknowledged(SnowflakeDdlKind::AddColumn),
                ReviewedDdl(Drop) => ExpectedEffect::DdlAcknowledged(SnowflakeDdlKind::DropTable),
                ReviewedDml(RollbackInsert | CommittedInsert) => {
                    ExpectedEffect::OneAffectedRow(SnowflakeDmlKind::Insert)
                }
                ReviewedDml(Update) => ExpectedEffect::OneAffectedRow(SnowflakeDmlKind::Update),
                ReviewedDml(Delete) => ExpectedEffect::OneAffectedRow(SnowflakeDmlKind::Delete),
                TypedBoundary { action, .. } => ExpectedEffect::BoundaryAcknowledged(action),
                _ => ExpectedEffect::None,
            };
            let presence = match operation {
                DdlPreflight(Create) => Some(FreshNameAbsent),
                DdlPreflight(Add | Drop) => Some(Present),
                Catalog(presence) | CoherentReconciliation(presence) => Some(presence),
                _ => None,
            };
            Phase {
                number: index as u8 + 1,
                operation,
                state,
                effect,
                presence,
                budget: SqlBudget { fixed, per_schema },
                advancement: AdvancementRule::AllPriorPhasesValidatedAndOriginalReceiptsSynced,
                failure: FailureRule::OnlyActiveFrozenReadOnlyPostObserverSuffix,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "snowflake_table_qualification_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "snowflake_table_qualification_runner_tests.rs"]
mod runner_tests;
