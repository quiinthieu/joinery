use crate::{domain::*, safety::quote_identifier};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

pub const MAX_SNAPSHOT_RELATIONS: usize = 500;
pub const MAX_SNAPSHOT_COLUMNS: usize = 20_000;
pub const MAX_SNAPSHOT_CONSTRAINTS: usize = 10_000;
pub const MAX_SNAPSHOT_INDEXES: usize = 10_000;
pub const MAX_SNAPSHOT_TRIGGERS: usize = 5_000;
pub const MAX_SNAPSHOT_ROUTINES: usize = 5_000;
pub const MAX_SNAPSHOT_SEQUENCES: usize = 5_000;
pub const MAX_SNAPSHOT_UNSUPPORTED: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_expression: Option<String>,
    pub identity: Option<String>,
    pub generated: Option<String>,
    pub ordinal_position: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotIndex {
    pub name: String,
    pub method: String,
    pub unique: bool,
    pub primary: bool,
    pub expressions: Vec<String>,
    pub predicate: Option<String>,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotTable {
    pub name: String,
    pub kind: DatabaseObjectKind,
    pub comment: Option<String>,
    pub partition_key: Option<String>,
    pub view_definition: Option<String>,
    pub columns: Vec<SnapshotColumn>,
    pub constraints: Vec<ConstraintInfo>,
    pub indexes: Vec<SnapshotIndex>,
    pub triggers: Vec<TriggerInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRoutine {
    pub name: String,
    pub kind: DatabaseObjectKind,
    pub identity_arguments: String,
    pub arguments: String,
    pub return_type: Option<String>,
    pub language: String,
    pub volatility: Option<String>,
    pub security_definer: bool,
    pub definition: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSequence {
    pub name: String,
    pub data_type: String,
    pub start_value: String,
    pub min_value: String,
    pub max_value: String,
    pub increment_by: String,
    pub cache_size: String,
    pub cycle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedSchemaObject {
    pub name: String,
    pub object_type: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSnapshot {
    pub engine: EngineKind,
    pub server_version: String,
    pub database: String,
    pub schema: String,
    pub role: String,
    pub tables: Vec<SnapshotTable>,
    pub routines: Vec<SnapshotRoutine>,
    pub sequences: Vec<SnapshotSequence>,
    pub unsupported_objects: Vec<UnsupportedSchemaObject>,
    pub truncated: bool,
    pub restricted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SchemaDiffKind {
    OnlyLeft,
    OnlyRight,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SchemaMemberCategory {
    Column,
    Constraint,
    Index,
    Trigger,
    Definition,
    Property,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SchemaMemberDiff {
    pub category: SchemaMemberCategory,
    pub name: String,
    pub quoted_name: String,
    pub kind: SchemaDiffKind,
    pub left_value: Option<String>,
    pub right_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SchemaObjectDiff {
    pub object_kind: DatabaseObjectKind,
    pub name: String,
    pub identity: Option<String>,
    pub quoted_name: String,
    pub kind: SchemaDiffKind,
    pub summary: String,
    pub member_diffs: Vec<SchemaMemberDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiffReport {
    pub left_label: String,
    pub right_label: String,
    pub left_engine: EngineKind,
    pub right_engine: EngineKind,
    pub engine_mismatch: bool,
    pub left_server_version: String,
    pub right_server_version: String,
    pub left_truncated: bool,
    pub right_truncated: bool,
    pub left_restricted: bool,
    pub right_restricted: bool,
    pub identical_count: usize,
    pub only_left_count: usize,
    pub only_right_count: usize,
    pub modified_count: usize,
    pub entries: Vec<SchemaObjectDiff>,
    pub left_unsupported: Vec<UnsupportedSchemaObject>,
    pub right_unsupported: Vec<UnsupportedSchemaObject>,
}

pub fn quote_engine_identifier(engine: EngineKind, name: &str) -> String {
    match engine {
        EngineKind::Mysql => format!("`{}`", name.replace('`', "``")),
        EngineKind::Postgres | EngineKind::Snowflake | EngineKind::Redis => quote_identifier(name),
    }
}

fn kind_rank(kind: DatabaseObjectKind) -> u8 {
    match kind {
        DatabaseObjectKind::Table => 1,
        DatabaseObjectKind::PartitionedTable => 2,
        DatabaseObjectKind::View => 3,
        DatabaseObjectKind::MaterializedView => 4,
        DatabaseObjectKind::Function => 5,
        DatabaseObjectKind::Procedure => 6,
        DatabaseObjectKind::Sequence => 7,
        DatabaseObjectKind::Trigger => 8,
        DatabaseObjectKind::Index => 9,
    }
}

fn constraint_kind_rank(kind: ConstraintKind) -> u8 {
    match kind {
        ConstraintKind::PrimaryKey => 1,
        ConstraintKind::Unique => 2,
        ConstraintKind::ForeignKey => 3,
        ConstraintKind::Check => 4,
        ConstraintKind::Exclusion => 5,
    }
}

impl SchemaSnapshot {
    /// Normalizes ordering across all object and member collections so catalog
    /// row-order differences never cause false diffs.
    pub fn canonicalize(&mut self) {
        for table in &mut self.tables {
            table.columns.sort_by(|a, b| {
                a.ordinal_position
                    .cmp(&b.ordinal_position)
                    .then_with(|| a.name.cmp(&b.name))
            });
            table.constraints.sort_by(|a, b| {
                a.name
                    .cmp(&b.name)
                    .then_with(|| constraint_kind_rank(a.kind).cmp(&constraint_kind_rank(b.kind)))
                    .then_with(|| a.columns.cmp(&b.columns))
                    .then_with(|| a.definition.cmp(&b.definition))
            });
            table.indexes.sort_by(|a, b| {
                a.name
                    .cmp(&b.name)
                    .then_with(|| a.method.cmp(&b.method))
                    .then_with(|| a.expressions.cmp(&b.expressions))
            });
            for trigger in &mut table.triggers {
                trigger.events.sort();
            }
            table.triggers.sort_by(|a, b| {
                a.name
                    .cmp(&b.name)
                    .then_with(|| a.timing.cmp(&b.timing))
                    .then_with(|| a.orientation.cmp(&b.orientation))
            });
            if let Some(def) = &mut table.view_definition {
                *def = def.trim().to_owned();
            }
            if let Some(part) = &mut table.partition_key {
                *part = part.trim().to_owned();
            }
        }
        self.tables.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
        });
        self.routines.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.identity_arguments.cmp(&b.identity_arguments))
                .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
        });
        self.sequences.sort_by(|a, b| a.name.cmp(&b.name));
        self.unsupported_objects.sort_by(|a, b| {
            a.object_type
                .cmp(&b.object_type)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.reason.cmp(&b.reason))
        });
        self.unsupported_objects.dedup_by(|a, b| {
            a.object_type == b.object_type && a.name == b.name && a.reason == b.reason
        });
    }

    pub fn display_label(&self) -> String {
        let engine = match self.engine {
            EngineKind::Postgres => "PostgreSQL",
            EngineKind::Mysql => "MySQL",
            EngineKind::Snowflake => "Snowflake",
            EngineKind::Redis => "Redis",
        };
        if self.database == self.schema || self.database.is_empty() {
            format!("{engine} · {} ({})", self.schema, self.role)
        } else {
            format!(
                "{engine} · {}.{} ({})",
                self.database, self.schema, self.role
            )
        }
    }
}

pub fn normalize_self_schema_sql(sql: &str, engine: EngineKind, own_schema: &str) -> String {
    let base = sql.trim().trim_end_matches(';').trim_end();
    let trimmed_schema = own_schema.trim();
    if trimmed_schema.is_empty() {
        return base.to_owned();
    }
    let quoted_prefix = format!("{}.", quote_engine_identifier(engine, trimmed_schema));
    let unquoted_prefix = format!("{trimmed_schema}.");
    let single_quoted_prefix = format!("'{trimmed_schema}.");
    let single_quoted_quoted_prefix =
        format!("'{}.", quote_engine_identifier(engine, trimmed_schema));
    base.replace(&single_quoted_quoted_prefix, "'$self_schema.")
        .replace(&single_quoted_prefix, "'$self_schema.")
        .replace(&quoted_prefix, "$self_schema.")
        .replace(&unquoted_prefix, "$self_schema.")
}

pub(crate) fn format_column_spec(
    col: &SnapshotColumn,
    engine: EngineKind,
    own_schema: &str,
) -> String {
    let mut parts = vec![col.data_type.clone()];
    parts.push(if col.nullable {
        "NULL".into()
    } else {
        "NOT NULL".into()
    });
    if let Some(id) = &col.identity
        && !id.trim().is_empty()
    {
        parts.push(format!("IDENTITY {}", id.trim()));
    }
    if let Some(generated_kind) = &col.generated
        && !generated_kind.trim().is_empty()
    {
        if let Some(expr) = &col.default_expression
            && !expr.trim().is_empty()
        {
            parts.push(format!(
                "GENERATED ({}) {}",
                normalize_self_schema_sql(expr, engine, own_schema),
                generated_kind.trim()
            ));
        } else {
            parts.push(format!("GENERATED {}", generated_kind.trim()));
        }
    } else if let Some(default) = &col.default_expression
        && !default.trim().is_empty()
    {
        parts.push(format!(
            "DEFAULT {}",
            normalize_self_schema_sql(default, engine, own_schema)
        ));
    }
    parts.join(" ")
}

pub(crate) fn format_constraint_spec(
    c: &ConstraintInfo,
    engine: EngineKind,
    own_schema: &str,
) -> String {
    let kind_label = match c.kind {
        ConstraintKind::PrimaryKey => "PRIMARY KEY",
        ConstraintKind::ForeignKey => "FOREIGN KEY",
        ConstraintKind::Unique => "UNIQUE",
        ConstraintKind::Check => "CHECK",
        ConstraintKind::Exclusion => "EXCLUDE",
    };
    if !c.definition.trim().is_empty() {
        format!(
            "{kind_label}: {}",
            normalize_self_schema_sql(&c.definition, engine, own_schema)
        )
    } else if let Some(target) = &c.target {
        let target_schema = if target.schema == own_schema {
            "$self_schema"
        } else {
            target.schema.as_str()
        };
        format!(
            "{kind_label} ({}) REFERENCES {}.{} ({})",
            c.columns.join(", "),
            target_schema,
            target.name,
            c.target_columns.join(", ")
        )
    } else {
        format!("{kind_label} ({})", c.columns.join(", "))
    }
}

pub(crate) fn format_index_spec(
    idx: &SnapshotIndex,
    engine: EngineKind,
    own_schema: &str,
) -> String {
    let mut parts = Vec::new();
    if idx.primary {
        parts.push("PRIMARY".to_owned());
    } else if idx.unique {
        parts.push("UNIQUE".to_owned());
    } else {
        parts.push("NON-UNIQUE".to_owned());
    }
    parts.push(idx.method.to_lowercase());
    parts.push(format!("({})", idx.expressions.join(", ")));
    if let Some(pred) = &idx.predicate
        && !pred.trim().is_empty()
    {
        parts.push(format!(
            "WHERE {}",
            normalize_self_schema_sql(pred, engine, own_schema)
        ));
    }
    parts.join(" ")
}

pub(crate) fn format_trigger_spec(
    trg: &TriggerInfo,
    engine: EngineKind,
    own_schema: &str,
) -> String {
    let mut events = trg.events.clone();
    events.sort();
    let status = if trg.enabled { "ENABLED" } else { "DISABLED" };
    match &trg.definition {
        Some(def) if !def.trim().is_empty() => format!(
            "{} {} FOR EACH {} ({status}) — {}",
            trg.timing,
            events.join(" OR "),
            trg.orientation,
            normalize_self_schema_sql(def, engine, own_schema)
        ),
        _ => format!(
            "{} {} FOR EACH {} ({status})",
            trg.timing,
            events.join(" OR "),
            trg.orientation
        ),
    }
}

pub(crate) fn format_routine_spec(
    r: &SnapshotRoutine,
    engine: EngineKind,
    own_schema: &str,
) -> String {
    let ret = r.return_type.as_deref().unwrap_or("void");
    let vol = r.volatility.as_deref().unwrap_or("VOLATILE");
    let sec = if r.security_definer {
        "SECURITY DEFINER"
    } else {
        "SECURITY INVOKER"
    };
    let header = format!(
        "({}) -> {ret} LANGUAGE {} {vol} {sec}",
        r.arguments, r.language
    );
    match &r.definition {
        Some(def) if !def.trim().is_empty() => format!(
            "{header}\n{}",
            normalize_self_schema_sql(def, engine, own_schema)
        ),
        _ => header,
    }
}

pub(crate) fn format_sequence_spec(seq: &SnapshotSequence) -> String {
    format!(
        "AS {} START {} INCREMENT {} MINVALUE {} MAXVALUE {} CACHE {} {}",
        seq.data_type,
        seq.start_value,
        seq.increment_by,
        seq.min_value,
        seq.max_value,
        seq.cache_size,
        if seq.cycle { "CYCLE" } else { "NO CYCLE" }
    )
}

fn summarize_member_diffs(diffs: &[SchemaMemberDiff]) -> String {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for diff in diffs {
        let key = match diff.category {
            SchemaMemberCategory::Column => "column",
            SchemaMemberCategory::Constraint => "constraint",
            SchemaMemberCategory::Index => "index",
            SchemaMemberCategory::Trigger => "trigger",
            SchemaMemberCategory::Definition => "definition",
            SchemaMemberCategory::Property => "property",
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    let parts: Vec<String> = counts
        .into_iter()
        .map(|(k, n)| {
            let plural = match (k, n) {
                (_, 1) => k.to_owned(),
                ("index", _) => "indexes".to_owned(),
                ("property", _) => "properties".to_owned(),
                _ => format!("{k}s"),
            };
            format!("{n} {plural}")
        })
        .collect();
    if parts.is_empty() {
        "Modified".into()
    } else {
        format!("Changed: {}", parts.join(", "))
    }
}

#[allow(clippy::too_many_arguments)]
fn diff_named_specs<T>(
    left: &[T],
    right: &[T],
    engine: EngineKind,
    category: SchemaMemberCategory,
    name_of: impl Fn(&T) -> &str,
    left_spec_of: impl Fn(&T) -> String,
    right_spec_of: impl Fn(&T) -> String,
    out: &mut Vec<SchemaMemberDiff>,
) {
    let left_map: BTreeMap<&str, String> = left
        .iter()
        .map(|item| (name_of(item), left_spec_of(item)))
        .collect();
    let right_map: BTreeMap<&str, String> = right
        .iter()
        .map(|item| (name_of(item), right_spec_of(item)))
        .collect();

    let mut keys: Vec<&str> = left_map.keys().chain(right_map.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();

    for key in keys {
        match (left_map.get(key), right_map.get(key)) {
            (Some(l), None) => out.push(SchemaMemberDiff {
                category,
                name: key.to_owned(),
                quoted_name: quote_engine_identifier(engine, key),
                kind: SchemaDiffKind::OnlyLeft,
                left_value: Some(l.clone()),
                right_value: None,
            }),
            (None, Some(r)) => out.push(SchemaMemberDiff {
                category,
                name: key.to_owned(),
                quoted_name: quote_engine_identifier(engine, key),
                kind: SchemaDiffKind::OnlyRight,
                left_value: None,
                right_value: Some(r.clone()),
            }),
            (Some(l), Some(r)) if l != r => out.push(SchemaMemberDiff {
                category,
                name: key.to_owned(),
                quoted_name: quote_engine_identifier(engine, key),
                kind: SchemaDiffKind::Modified,
                left_value: Some(l.clone()),
                right_value: Some(r.clone()),
            }),
            _ => {}
        }
    }
}

fn diff_table(
    left: &SnapshotTable,
    right: &SnapshotTable,
    engine: EngineKind,
    left_schema: &str,
    right_schema: &str,
) -> Vec<SchemaMemberDiff> {
    let mut diffs = Vec::new();
    if left.kind != right.kind {
        diffs.push(SchemaMemberDiff {
            category: SchemaMemberCategory::Property,
            name: "relation kind".into(),
            quoted_name: "relation kind".into(),
            kind: SchemaDiffKind::Modified,
            left_value: Some(format!("{:?}", left.kind)),
            right_value: Some(format!("{:?}", right.kind)),
        });
    }
    if left.partition_key != right.partition_key {
        diffs.push(SchemaMemberDiff {
            category: SchemaMemberCategory::Property,
            name: "partition key".into(),
            quoted_name: "partition key".into(),
            kind: match (&left.partition_key, &right.partition_key) {
                (Some(_), None) => SchemaDiffKind::OnlyLeft,
                (None, Some(_)) => SchemaDiffKind::OnlyRight,
                _ => SchemaDiffKind::Modified,
            },
            left_value: left.partition_key.clone(),
            right_value: right.partition_key.clone(),
        });
    }
    diff_named_specs(
        &left.columns,
        &right.columns,
        engine,
        SchemaMemberCategory::Column,
        |c| &c.name,
        |c| format_column_spec(c, engine, left_schema),
        |c| format_column_spec(c, engine, right_schema),
        &mut diffs,
    );
    diff_named_specs(
        &left.constraints,
        &right.constraints,
        engine,
        SchemaMemberCategory::Constraint,
        |c| &c.name,
        |c| format_constraint_spec(c, engine, left_schema),
        |c| format_constraint_spec(c, engine, right_schema),
        &mut diffs,
    );
    diff_named_specs(
        &left.indexes,
        &right.indexes,
        engine,
        SchemaMemberCategory::Index,
        |i| &i.name,
        |i| format_index_spec(i, engine, left_schema),
        |i| format_index_spec(i, engine, right_schema),
        &mut diffs,
    );
    diff_named_specs(
        &left.triggers,
        &right.triggers,
        engine,
        SchemaMemberCategory::Trigger,
        |t| &t.name,
        |t| format_trigger_spec(t, engine, left_schema),
        |t| format_trigger_spec(t, engine, right_schema),
        &mut diffs,
    );
    let left_view = left
        .view_definition
        .as_deref()
        .map(|s| normalize_self_schema_sql(s, engine, left_schema))
        .filter(|s| !s.is_empty());
    let right_view = right
        .view_definition
        .as_deref()
        .map(|s| normalize_self_schema_sql(s, engine, right_schema))
        .filter(|s| !s.is_empty());
    if left_view != right_view {
        diffs.push(SchemaMemberDiff {
            category: SchemaMemberCategory::Definition,
            name: "view definition".into(),
            quoted_name: "view definition".into(),
            kind: match (&left_view, &right_view) {
                (Some(_), None) => SchemaDiffKind::OnlyLeft,
                (None, Some(_)) => SchemaDiffKind::OnlyRight,
                _ => SchemaDiffKind::Modified,
            },
            left_value: left_view,
            right_value: right_view,
        });
    }
    diffs
}

/// Deterministically compares two `SchemaSnapshot` instances after canonicalizing
/// both sides. Never mutates database state or executes SQL.
pub fn compare_schema_snapshots(
    mut left: SchemaSnapshot,
    mut right: SchemaSnapshot,
) -> SchemaDiffReport {
    left.canonicalize();
    right.canonicalize();

    let quote_engine = left.engine;
    let mut entries = Vec::new();
    let mut identical_count = 0usize;
    let mut only_left_count = 0usize;
    let mut only_right_count = 0usize;
    let mut modified_count = 0usize;

    // 1. Relations (tables, partitioned tables, views, materialized views)
    let left_tables: BTreeMap<&str, &SnapshotTable> =
        left.tables.iter().map(|t| (t.name.as_str(), t)).collect();
    let right_tables: BTreeMap<&str, &SnapshotTable> =
        right.tables.iter().map(|t| (t.name.as_str(), t)).collect();
    let mut table_names: Vec<&str> = left_tables
        .keys()
        .chain(right_tables.keys())
        .copied()
        .collect();
    table_names.sort_unstable();
    table_names.dedup();

    for name in table_names {
        match (left_tables.get(name), right_tables.get(name)) {
            (Some(l), None) => {
                only_left_count += 1;
                entries.push(SchemaObjectDiff {
                    object_kind: l.kind,
                    name: l.name.clone(),
                    identity: None,
                    quoted_name: quote_engine_identifier(left.engine, &l.name),
                    kind: SchemaDiffKind::OnlyLeft,
                    summary: format!(
                        "Only in left ({} columns, {} constraints, {} indexes)",
                        l.columns.len(),
                        l.constraints.len(),
                        l.indexes.len()
                    ),
                    member_diffs: Vec::new(),
                });
            }
            (None, Some(r)) => {
                only_right_count += 1;
                entries.push(SchemaObjectDiff {
                    object_kind: r.kind,
                    name: r.name.clone(),
                    identity: None,
                    quoted_name: quote_engine_identifier(right.engine, &r.name),
                    kind: SchemaDiffKind::OnlyRight,
                    summary: format!(
                        "Only in right ({} columns, {} constraints, {} indexes)",
                        r.columns.len(),
                        r.constraints.len(),
                        r.indexes.len()
                    ),
                    member_diffs: Vec::new(),
                });
            }
            (Some(l), Some(r)) => {
                let member_diffs = diff_table(l, r, quote_engine, &left.schema, &right.schema);
                if member_diffs.is_empty() {
                    identical_count += 1;
                } else {
                    modified_count += 1;
                    let summary = summarize_member_diffs(&member_diffs);
                    entries.push(SchemaObjectDiff {
                        object_kind: r.kind,
                        name: l.name.clone(),
                        identity: None,
                        quoted_name: quote_engine_identifier(quote_engine, &l.name),
                        kind: SchemaDiffKind::Modified,
                        summary,
                        member_diffs,
                    });
                }
            }
            (None, None) => {}
        }
    }

    // 2. Routines (functions and procedures keyed by (name, identity_arguments))
    let left_routines: BTreeMap<(&str, &str), &SnapshotRoutine> = left
        .routines
        .iter()
        .map(|r| ((r.name.as_str(), r.identity_arguments.as_str()), r))
        .collect();
    let right_routines: BTreeMap<(&str, &str), &SnapshotRoutine> = right
        .routines
        .iter()
        .map(|r| ((r.name.as_str(), r.identity_arguments.as_str()), r))
        .collect();
    let mut routine_keys: Vec<(&str, &str)> = left_routines
        .keys()
        .chain(right_routines.keys())
        .copied()
        .collect();
    routine_keys.sort_unstable();
    routine_keys.dedup();

    for (name, identity_args) in routine_keys {
        let identity = (!identity_args.is_empty()).then(|| identity_args.to_owned());
        let format_quoted_routine = |eng: EngineKind| {
            let q = quote_engine_identifier(eng, name);
            format!("{q}({identity_args})")
        };
        match (
            left_routines.get(&(name, identity_args)),
            right_routines.get(&(name, identity_args)),
        ) {
            (Some(l), None) => {
                only_left_count += 1;
                entries.push(SchemaObjectDiff {
                    object_kind: l.kind,
                    name: l.name.clone(),
                    identity,
                    quoted_name: format_quoted_routine(left.engine),
                    kind: SchemaDiffKind::OnlyLeft,
                    summary: "Only in left".into(),
                    member_diffs: Vec::new(),
                });
            }
            (None, Some(r)) => {
                only_right_count += 1;
                entries.push(SchemaObjectDiff {
                    object_kind: r.kind,
                    name: r.name.clone(),
                    identity,
                    quoted_name: format_quoted_routine(right.engine),
                    kind: SchemaDiffKind::OnlyRight,
                    summary: "Only in right".into(),
                    member_diffs: Vec::new(),
                });
            }
            (Some(l), Some(r)) => {
                let left_spec = format_routine_spec(l, quote_engine, &left.schema);
                let right_spec = format_routine_spec(r, quote_engine, &right.schema);
                if left_spec == right_spec && l.kind == r.kind {
                    identical_count += 1;
                } else {
                    modified_count += 1;
                    let member_diffs = vec![SchemaMemberDiff {
                        category: SchemaMemberCategory::Definition,
                        name: "routine definition".into(),
                        quoted_name: format_quoted_routine(quote_engine),
                        kind: SchemaDiffKind::Modified,
                        left_value: Some(left_spec),
                        right_value: Some(right_spec),
                    }];
                    entries.push(SchemaObjectDiff {
                        object_kind: r.kind,
                        name: l.name.clone(),
                        identity,
                        quoted_name: format_quoted_routine(quote_engine),
                        kind: SchemaDiffKind::Modified,
                        summary: summarize_member_diffs(&member_diffs),
                        member_diffs,
                    });
                }
            }
            (None, None) => {}
        }
    }

    // 3. Sequences
    let left_seqs: BTreeMap<&str, &SnapshotSequence> = left
        .sequences
        .iter()
        .map(|s| (s.name.as_str(), s))
        .collect();
    let right_seqs: BTreeMap<&str, &SnapshotSequence> = right
        .sequences
        .iter()
        .map(|s| (s.name.as_str(), s))
        .collect();
    let mut seq_names: Vec<&str> = left_seqs.keys().chain(right_seqs.keys()).copied().collect();
    seq_names.sort_unstable();
    seq_names.dedup();

    for name in seq_names {
        match (left_seqs.get(name), right_seqs.get(name)) {
            (Some(l), None) => {
                only_left_count += 1;
                entries.push(SchemaObjectDiff {
                    object_kind: DatabaseObjectKind::Sequence,
                    name: l.name.clone(),
                    identity: None,
                    quoted_name: quote_engine_identifier(left.engine, &l.name),
                    kind: SchemaDiffKind::OnlyLeft,
                    summary: "Only in left".into(),
                    member_diffs: Vec::new(),
                });
            }
            (None, Some(r)) => {
                only_right_count += 1;
                entries.push(SchemaObjectDiff {
                    object_kind: DatabaseObjectKind::Sequence,
                    name: r.name.clone(),
                    identity: None,
                    quoted_name: quote_engine_identifier(right.engine, &r.name),
                    kind: SchemaDiffKind::OnlyRight,
                    summary: "Only in right".into(),
                    member_diffs: Vec::new(),
                });
            }
            (Some(l), Some(r)) => {
                let left_spec = format_sequence_spec(l);
                let right_spec = format_sequence_spec(r);
                if left_spec == right_spec {
                    identical_count += 1;
                } else {
                    modified_count += 1;
                    let member_diffs = vec![SchemaMemberDiff {
                        category: SchemaMemberCategory::Property,
                        name: "sequence parameters".into(),
                        quoted_name: quote_engine_identifier(quote_engine, &l.name),
                        kind: SchemaDiffKind::Modified,
                        left_value: Some(left_spec),
                        right_value: Some(right_spec),
                    }];
                    entries.push(SchemaObjectDiff {
                        object_kind: DatabaseObjectKind::Sequence,
                        name: l.name.clone(),
                        identity: None,
                        quoted_name: quote_engine_identifier(quote_engine, &l.name),
                        kind: SchemaDiffKind::Modified,
                        summary: summarize_member_diffs(&member_diffs),
                        member_diffs,
                    });
                }
            }
            (None, None) => {}
        }
    }

    SchemaDiffReport {
        left_label: left.display_label(),
        right_label: right.display_label(),
        left_engine: left.engine,
        right_engine: right.engine,
        engine_mismatch: left.engine != right.engine,
        left_server_version: left.server_version,
        right_server_version: right.server_version,
        left_truncated: left.truncated,
        right_truncated: right.truncated,
        left_restricted: left.restricted,
        right_restricted: right.restricted,
        identical_count,
        only_left_count,
        only_right_count,
        modified_count,
        entries,
        left_unsupported: left.unsupported_objects,
        right_unsupported: right.unsupported_objects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot(engine: EngineKind, schema: &str) -> SchemaSnapshot {
        SchemaSnapshot {
            engine,
            server_version: "PostgreSQL 18.0".into(),
            database: "joinery".into(),
            schema: schema.into(),
            role: "joinery".into(),
            tables: vec![
                SnapshotTable {
                    name: "orders".into(),
                    kind: DatabaseObjectKind::Table,
                    comment: None,
                    partition_key: None,
                    columns: vec![
                        SnapshotColumn {
                            name: "total_cents".into(),
                            ordinal_position: 2,
                            data_type: "bigint".into(),
                            nullable: false,
                            default_expression: Some("0".into()),
                            identity: None,
                            generated: None,
                        },
                        SnapshotColumn {
                            name: "id".into(),
                            ordinal_position: 1,
                            data_type: "bigint".into(),
                            nullable: false,
                            default_expression: None,
                            identity: Some("ALWAYS".into()),
                            generated: None,
                        },
                    ],
                    constraints: vec![
                        ConstraintInfo {
                            name: "orders_total_positive".into(),
                            kind: ConstraintKind::Check,
                            columns: vec!["total_cents".into()],
                            target: None,
                            target_columns: Vec::new(),
                            definition: "CHECK ((total_cents >= 0))".into(),
                        },
                        ConstraintInfo {
                            name: "orders_pkey".into(),
                            kind: ConstraintKind::PrimaryKey,
                            columns: vec!["id".into()],
                            target: None,
                            target_columns: Vec::new(),
                            definition: "PRIMARY KEY (id)".into(),
                        },
                    ],
                    indexes: vec![
                        SnapshotIndex {
                            name: "orders_total_idx".into(),
                            method: "btree".into(),
                            unique: false,
                            primary: false,
                            expressions: vec!["total_cents".into()],
                            predicate: Some("total_cents > 0".into()),
                            definition: None,
                        },
                        SnapshotIndex {
                            name: "orders_pkey".into(),
                            method: "btree".into(),
                            unique: true,
                            primary: true,
                            expressions: vec!["id".into()],
                            predicate: None,
                            definition: None,
                        },
                    ],
                    triggers: vec![TriggerInfo {
                        name: "orders_audit_trg".into(),
                        table_name: "orders".into(),
                        timing: "AFTER".into(),
                        events: vec!["UPDATE".into(), "INSERT".into()],
                        orientation: "ROW".into(),
                        enabled: true,
                        definition: Some("EXECUTE FUNCTION audit_orders()".into()),
                    }],
                    view_definition: None,
                },
                SnapshotTable {
                    name: "active_orders_v".into(),
                    kind: DatabaseObjectKind::View,
                    comment: None,
                    partition_key: None,
                    columns: vec![SnapshotColumn {
                        name: "id".into(),
                        ordinal_position: 1,
                        data_type: "bigint".into(),
                        nullable: true,
                        default_expression: None,
                        identity: None,
                        generated: None,
                    }],
                    constraints: Vec::new(),
                    indexes: Vec::new(),
                    triggers: Vec::new(),
                    view_definition: Some(
                        "  SELECT id FROM orders WHERE total_cents > 0; \n".into(),
                    ),
                },
            ],
            routines: vec![
                SnapshotRoutine {
                    name: "calc_tax".into(),
                    kind: DatabaseObjectKind::Function,
                    identity_arguments: "numeric, integer".into(),
                    arguments: "amount numeric, rate integer".into(),
                    return_type: Some("numeric".into()),
                    language: "sql".into(),
                    volatility: Some("IMMUTABLE".into()),
                    security_definer: false,
                    definition: Some("SELECT amount * rate / 100".into()),
                    comment: None,
                },
                SnapshotRoutine {
                    name: "calc_tax".into(),
                    kind: DatabaseObjectKind::Function,
                    identity_arguments: "integer".into(),
                    arguments: "amount integer".into(),
                    return_type: Some("integer".into()),
                    language: "sql".into(),
                    volatility: Some("IMMUTABLE".into()),
                    security_definer: false,
                    definition: Some("SELECT amount / 10".into()),
                    comment: None,
                },
            ],
            sequences: vec![SnapshotSequence {
                name: "orders_seq".into(),
                data_type: "bigint".into(),
                start_value: "1".into(),
                min_value: "1".into(),
                max_value: "9223372036854775807".into(),
                increment_by: "1".into(),
                cache_size: "1".into(),
                cycle: false,
            }],
            unsupported_objects: vec![
                UnsupportedSchemaObject {
                    name: "order_status".into(),
                    object_type: "enum type".into(),
                    reason: "Custom enum types are reported as unsupported objects".into(),
                },
                UnsupportedSchemaObject {
                    name: "order_status".into(),
                    object_type: "enum type".into(),
                    reason: "Custom enum types are reported as unsupported objects".into(),
                },
            ],
            truncated: false,
            restricted: false,
        }
    }

    #[test]
    fn reordered_metadata_produces_identical_canonical_snapshots_and_zero_diffs() {
        let left = sample_snapshot(EngineKind::Postgres, "public");
        let mut right = sample_snapshot(EngineKind::Postgres, "staging");
        right.tables.reverse();
        right.tables[1].columns.reverse();
        right.tables[1].constraints.reverse();
        right.tables[1].indexes.reverse();
        right.tables[1].triggers[0].events.reverse();
        right.routines.reverse();

        let report = compare_schema_snapshots(left, right);
        assert!(!report.engine_mismatch);
        assert_eq!(report.identical_count, 5);
        assert_eq!(report.only_left_count, 0);
        assert_eq!(report.only_right_count, 0);
        assert_eq!(report.modified_count, 0);
        assert!(report.entries.is_empty());
        assert_eq!(report.left_unsupported.len(), 1);
        assert_eq!(report.right_unsupported.len(), 1);
    }

    #[test]
    fn quoted_identifiers_member_changes_and_unsupported_objects_are_reported_accurately() {
        let mut left = sample_snapshot(EngineKind::Postgres, "public");
        let mut right = sample_snapshot(EngineKind::Postgres, "staging");

        // Add a table with quotes and spaces only on left
        left.tables.push(SnapshotTable {
            name: " Special \"Items\".v1 ".into(),
            kind: DatabaseObjectKind::Table,
            comment: None,
            partition_key: None,
            columns: vec![SnapshotColumn {
                name: "col\"a".into(),
                ordinal_position: 1,
                data_type: "text".into(),
                nullable: false,
                default_expression: None,
                identity: None,
                generated: None,
            }],
            constraints: Vec::new(),
            indexes: Vec::new(),
            triggers: Vec::new(),
            view_definition: None,
        });

        // Modify `orders` on right: change `total_cents` nullability, add column `note "x"`, drop index `orders_total_idx`
        let right_orders = right
            .tables
            .iter_mut()
            .find(|t| t.name == "orders")
            .unwrap();
        right_orders
            .columns
            .iter_mut()
            .find(|c| c.name == "total_cents")
            .unwrap()
            .nullable = true;
        right_orders.columns.push(SnapshotColumn {
            name: "note \"x\"".into(),
            ordinal_position: 3,
            data_type: "text".into(),
            nullable: true,
            default_expression: None,
            identity: None,
            generated: None,
        });
        right_orders
            .indexes
            .retain(|idx| idx.name != "orders_total_idx");

        // Modify overloaded routine `calc_tax(integer)` only
        right
            .routines
            .iter_mut()
            .find(|r| r.identity_arguments == "integer")
            .unwrap()
            .definition = Some("SELECT amount / 5".into());

        // Add an unsupported domain object on right
        right.unsupported_objects.push(UnsupportedSchemaObject {
            name: "positive_cents".into(),
            object_type: "domain".into(),
            reason: "Domains are reported as unsupported objects".into(),
        });

        let report = compare_schema_snapshots(left, right);
        assert_eq!(report.identical_count, 3); // active_orders_v, calc_tax(numeric, integer), orders_seq
        assert_eq!(report.only_left_count, 1);
        assert_eq!(report.only_right_count, 0);
        assert_eq!(report.modified_count, 2); // orders, calc_tax(integer)

        let special = report
            .entries
            .iter()
            .find(|e| e.name == " Special \"Items\".v1 ")
            .expect("quoted table diff");
        assert_eq!(special.kind, SchemaDiffKind::OnlyLeft);
        assert_eq!(special.quoted_name, "\" Special \"\"Items\"\".v1 \"");

        let orders = report
            .entries
            .iter()
            .find(|e| e.name == "orders")
            .expect("orders diff");
        assert_eq!(orders.kind, SchemaDiffKind::Modified);
        assert_eq!(orders.summary, "Changed: 2 columns, 1 index");
        assert!(orders.member_diffs.iter().any(|m| {
            m.category == SchemaMemberCategory::Column
                && m.name == "note \"x\""
                && m.quoted_name == "\"note \"\"x\"\"\""
                && m.kind == SchemaDiffKind::OnlyRight
        }));
        assert!(orders.member_diffs.iter().any(|m| {
            m.category == SchemaMemberCategory::Column
                && m.name == "total_cents"
                && m.kind == SchemaDiffKind::Modified
        }));
        assert!(orders.member_diffs.iter().any(|m| {
            m.category == SchemaMemberCategory::Index
                && m.name == "orders_total_idx"
                && m.kind == SchemaDiffKind::OnlyLeft
        }));

        let routine = report
            .entries
            .iter()
            .find(|e| e.name == "calc_tax" && e.identity.as_deref() == Some("integer"))
            .expect("overloaded routine diff");
        assert_eq!(routine.kind, SchemaDiffKind::Modified);
        assert_eq!(routine.quoted_name, "\"calc_tax\"(integer)");
        assert_eq!(report.right_unsupported.len(), 2);
    }
}
