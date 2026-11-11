//! Offline expansion only. Entries describe application SQL, not SDK-internal
//! authentication traffic, execution permission, or an installed admission policy.
use super::*;
use crate::drivers::snowflake_sql_contract;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum SqlKind {
    BootstrapContext,
    Read(metadata::ReadKind),
    Readback(Readback),
    Ddl(SnowflakeDdlKind),
    Dml(SnowflakeDmlKind),
    Transaction(ExplicitTransactionAction),
}

impl SqlKind {
    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::BootstrapContext => "context",
            Self::Read(_) | Self::Readback(_) => "query",
            Self::Ddl(_) => "ddl",
            Self::Dml(_) => "dml",
            Self::Transaction(_) => "transaction",
        }
    }
    pub(crate) fn is_read(self) -> bool {
        matches!(
            self,
            Self::BootstrapContext | Self::Read(_) | Self::Readback(_)
        )
    }
}

/// Query-frame retention ceilings, distinct from the expected number of rows.
/// Non-streaming typed/helper operations do not have query-frame ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Retention {
    pub max_rows: u64,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SqlEntry {
    ordinal: u16,
    phase: u8,
    phase_ordinal: u16,
    kind: SqlKind,
    sql: String,
    binds: Vec<TypedBind>,
    retention: Option<Retention>,
    /// Zero unless this exact primary operation can start an automatic observer
    /// after a definitive database rejection. This is a permitted prefix, not a
    /// requirement to continue after another error, interruption or cancellation.
    rejection_suffix: u8,
}

impl SqlEntry {
    pub(crate) fn ordinal(&self) -> u16 {
        self.ordinal
    }
    pub(crate) fn phase(&self) -> u8 {
        self.phase
    }
    pub(crate) fn phase_ordinal(&self) -> u16 {
        self.phase_ordinal
    }
    pub(crate) fn kind(&self) -> SqlKind {
        self.kind
    }
    pub(crate) fn sql(&self) -> &str {
        &self.sql
    }
    pub(crate) fn binds(&self) -> &[TypedBind] {
        &self.binds
    }
    pub(crate) fn retention(&self) -> Option<Retention> {
        self.retention
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PhysicalLedger {
    version: u8,
    plan_sha256: String,
    census: SchemaCensus,
    entries: Vec<SqlEntry>,
}

impl PhysicalLedger {
    pub(crate) fn entries(&self) -> &[SqlEntry] {
        &self.entries
    }
    pub(crate) fn census(&self) -> &SchemaCensus {
        &self.census
    }
    pub(crate) fn plan_sha256(&self) -> &str {
        &self.plan_sha256
    }
    pub(crate) fn canonical_json(&self) -> AppResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(|_| invalid())
    }
    pub(crate) fn sha256(&self) -> AppResult<String> {
        Ok(format!("{:x}", Sha256::digest(self.canonical_json()?)))
    }
    /// Index is zero-based, unlike the explicitly one-based serialized ordinals.
    /// Never reaches another phase, a mutation or a non-observer metadata read.
    pub(crate) fn rejection_suffix(&self, index: usize) -> AppResult<&[SqlEntry]> {
        let entry = self.entries.get(index).ok_or_else(invalid)?;
        let end = index + 1 + usize::from(entry.rejection_suffix);
        let suffix = self.entries.get(index + 1..end).ok_or_else(invalid)?;
        if suffix
            .iter()
            .any(|next| next.phase != entry.phase || !next.kind.is_read())
        {
            return Err(invalid());
        }
        Ok(suffix)
    }
}

struct Expansion<'a> {
    plan: &'a Plan,
    census: &'a SchemaCensus,
    context: metadata::LiveContext,
    relation: browsing::BrowseRelation,
    entries: Vec<SqlEntry>,
    phase: u8,
    phase_start: usize,
}

impl Plan {
    /// The sole ledger constructor: all SQL comes from this internally generated
    /// plan or closed production factories. No SQL/target/spec is caller input.
    pub(crate) fn expand_physical_ledger(
        &self,
        census: &SchemaCensus,
    ) -> AppResult<PhysicalLedger> {
        // Revalidate rather than trust future changes to this type's constructors.
        let census = SchemaCensus::freeze(census.names())?;
        let mut expansion = Expansion {
            plan: self,
            census: &census,
            context: template_context(),
            relation: template_relation(TableRef {
                schema: SCHEMA.into(),
                name: self.table.clone(),
            }),
            entries: Vec::new(),
            phase: 0,
            phase_start: 0,
        };
        for phase in &self.phases {
            expansion.phase = phase.number;
            expansion.phase_start = expansion.entries.len();
            expansion.operation(phase.operation)?;
            if expansion.entries.len() - expansion.phase_start
                != usize::from(phase.budget.total(&census))
            {
                return Err(invalid());
            }
        }
        if expansion.entries.len() != usize::from(self.budget(&census)) {
            return Err(invalid());
        }
        Ok(PhysicalLedger {
            version: 1,
            plan_sha256: self.sha256()?,
            census: census.clone(),
            entries: expansion.entries,
        })
    }
}

impl Expansion<'_> {
    fn push(
        &mut self,
        kind: SqlKind,
        sql: String,
        retention: Option<Retention>,
        rejection_suffix: u8,
    ) -> AppResult<()> {
        if self.entries.len() >= 610 || sql.is_empty() {
            return Err(invalid());
        }
        self.entries.push(SqlEntry {
            ordinal: (self.entries.len() + 1) as u16,
            phase: self.phase,
            phase_ordinal: (self.entries.len() - self.phase_start + 1) as u16,
            kind,
            sql,
            binds: Vec::new(),
            retention,
            rejection_suffix,
        });
        Ok(())
    }
    fn read(&mut self, spec: metadata::ReadSpec) -> AppResult<()> {
        self.push(
            SqlKind::Read(spec.kind()),
            spec.sql().into(),
            Some(Retention {
                max_rows: spec.max_rows(),
                max_bytes: spec.max_bytes(),
            }),
            0,
        )
    }
    fn context(&mut self) -> AppResult<()> {
        for spec in metadata::context_reads() {
            self.read(spec)?;
        }
        Ok(())
    }
    fn sample(&mut self) -> AppResult<()> {
        for spec in snowflake_session::sample_reads() {
            self.read(spec)?;
        }
        Ok(())
    }
    fn options(&mut self) -> AppResult<()> {
        self.sample()?;
        self.read(metadata::schemas_read(&self.context)?)?;
        self.sample()
    }
    fn catalog(&mut self) -> AppResult<()> {
        self.context()?;
        self.read(metadata::schemas_read(&self.context)?)?;
        for schema in self.census.names() {
            self.read(metadata::relations_read(&self.context, schema)?)?;
        }
        self.context()
    }
    fn exact_relation(&mut self) -> AppResult<()> {
        self.read(metadata::exact_relation_read(
            &self.context,
            &self.relation,
        )?)
    }
    fn columns(&mut self) -> AppResult<()> {
        for spec in metadata::inspection_reads(&self.relation)? {
            self.read(spec)?;
        }
        Ok(())
    }
    fn ddl_checks(&mut self, id: StatementId) -> AppResult<()> {
        match id {
            StatementId::Create => {
                self.read(metadata::schemas_read(&self.context)?)?;
                self.read(metadata::relations_read(&self.context, SCHEMA)?)
            }
            StatementId::Add => {
                self.exact_relation()?;
                self.columns()?;
                self.exact_relation()
            }
            StatementId::Drop => self.exact_relation(),
            _ => Err(invalid()),
        }
    }
    fn operation(&mut self, operation: Operation) -> AppResult<()> {
        match operation {
            Operation::StartupIdentityAndVersion => {
                self.push(
                    SqlKind::BootstrapContext,
                    snowflake_sql_contract::BOOTSTRAP_CONTEXT_SQL.into(),
                    None,
                    0,
                )?;
                self.read(metadata::version_read())
            }
            Operation::DdlPreflight(id) | Operation::ReviewedDdl(id) => {
                self.sample()?;
                self.ddl_checks(id)?;
                self.sample()?;
                if matches!(operation, Operation::ReviewedDdl(_)) {
                    let kind = match id {
                        StatementId::Create => SnowflakeDdlKind::CreateTable,
                        StatementId::Add => SnowflakeDdlKind::AddColumn,
                        StatementId::Drop => SnowflakeDdlKind::DropTable,
                        _ => return Err(invalid()),
                    };
                    self.push(
                        SqlKind::Ddl(kind),
                        self.plan.statement(id).wire_sql.clone(),
                        None,
                        5,
                    )?;
                    self.sample()?;
                }
                Ok(())
            }
            Operation::CoherentReconciliation(_) => {
                self.options()?;
                self.catalog()?;
                self.options()
            }
            Operation::Catalog(_) => self.catalog(),
            Operation::Inspect(_) => {
                self.context()?;
                self.exact_relation()?;
                self.columns()?;
                self.exact_relation()?;
                self.context()
            }
            Operation::Read(readback) => {
                let id = if readback == Readback::Ownership {
                    StatementId::OwnershipRead
                } else {
                    StatementId::DataRead
                };
                self.push(
                    SqlKind::Readback(readback),
                    self.plan.statement(id).wire_sql.clone(),
                    Some(Retention {
                        max_rows: metadata::STANDARD_QUERY_MAX_ROWS,
                        max_bytes: metadata::STANDARD_QUERY_MAX_BYTES,
                    }),
                    10,
                )?;
                self.sample()?;
                self.sample()
            }
            Operation::TypedBoundary { action, .. } => {
                if action != ExplicitTransactionAction::Rollback {
                    self.sample()?;
                }
                let sql = match action {
                    ExplicitTransactionAction::Begin => snowflake_sql_contract::BEGIN_SQL,
                    ExplicitTransactionAction::Commit => snowflake_sql_contract::COMMIT_SQL,
                    ExplicitTransactionAction::Rollback => snowflake_sql_contract::ROLLBACK_SQL,
                };
                self.push(SqlKind::Transaction(action), sql.into(), None, 0)?;
                self.sample()
            }
            Operation::ReviewedDml(id) => {
                let kind = match id {
                    StatementId::RollbackInsert | StatementId::CommittedInsert => {
                        SnowflakeDmlKind::Insert
                    }
                    StatementId::Update => SnowflakeDmlKind::Update,
                    StatementId::Delete => SnowflakeDmlKind::Delete,
                    _ => return Err(invalid()),
                };
                self.sample()?;
                self.push(
                    SqlKind::Dml(kind),
                    self.plan.statement(id).wire_sql.clone(),
                    None,
                    0,
                )?;
                self.sample()
            }
            Operation::SessionOptionsResume => self.options(),
        }
    }
}
