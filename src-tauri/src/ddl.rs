use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::domain::{
    AppError, AppResult, ConstraintInfo, ConstraintKind, DatabaseObjectKind, DatabaseObjectRef,
    EngineKind, ErrorCode, SchemaSnapshot, SnapshotColumn, SnapshotIndex, SnapshotRoutine,
    SnapshotSequence, SnapshotTable, TableRef, TriggerInfo, UnsupportedSchemaObject,
};
use crate::schema_diff::{
    compare_schema_snapshots, format_column_spec, format_constraint_spec, format_index_spec,
    format_routine_spec, format_sequence_spec, format_trigger_spec, normalize_self_schema_sql,
    quote_engine_identifier,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DdlStatement {
    pub sql: String,
    pub description: String,
    pub object_kind: DatabaseObjectKind,
    pub object_name: String,
    #[serde(default)]
    pub destructive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDdlPlan {
    pub engine: EngineKind,
    pub target_schema: String,
    pub statements: Vec<DdlStatement>,
    pub script: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub destructive_count: usize,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub unsupported: Vec<UnsupportedSchemaObject>,
}

/// Retargets references to `source_schema` within a catalog SQL fragment so the
/// generated DDL can be executed against `target_schema`.
pub fn retarget_schema_sql(
    sql: &str,
    engine: EngineKind,
    source_schema: &str,
    target_schema: &str,
) -> String {
    let src = source_schema.trim();
    let dst = target_schema.trim();
    if src.is_empty() || dst.is_empty() || src == dst {
        return sql.trim().to_owned();
    }
    let src_quoted = format!("{}.", quote_engine_identifier(engine, src));
    let dst_quoted = format!("{}.", quote_engine_identifier(engine, dst));
    let src_unquoted = format!("{src}.");
    let src_single_quoted = format!("'{src}.");
    let dst_single_quoted = format!("'{}.", dst.replace('\'', "''"));
    let src_single_quoted_ident = format!(
        "'{}.",
        quote_engine_identifier(engine, src).replace('\'', "''")
    );
    let dst_single_quoted_ident = format!(
        "'{}.",
        quote_engine_identifier(engine, dst).replace('\'', "''")
    );

    sql.trim()
        .replace(&src_single_quoted_ident, &dst_single_quoted_ident)
        .replace(&src_single_quoted, &dst_single_quoted)
        .replace(&src_quoted, &dst_quoted)
        .replace(&src_unquoted, &dst_quoted)
}

fn quote_qualified(engine: EngineKind, schema: &str, name: &str) -> String {
    if schema.trim().is_empty() {
        quote_engine_identifier(engine, name)
    } else {
        format!(
            "{}.{}",
            quote_engine_identifier(engine, schema.trim()),
            quote_engine_identifier(engine, name)
        )
    }
}

fn quote_sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn ensure_semicolon(sql: &str) -> String {
    let trimmed = sql.trim().trim_end_matches(';').trim_end();
    format!("{trimmed};")
}

fn is_auto_identity_sequence(snapshot: &SchemaSnapshot, seq_name: &str) -> bool {
    if snapshot.engine != EngineKind::Postgres {
        return false;
    }
    snapshot.tables.iter().any(|t| {
        t.columns.iter().any(|c| {
            c.identity.as_ref().is_some_and(|id| !id.trim().is_empty())
                && seq_name == format!("{}_{}_seq", t.name, c.name)
        })
    })
}

fn is_constraint_backed_index(table: &SnapshotTable, idx: &SnapshotIndex) -> bool {
    if idx.primary {
        return true;
    }
    table.constraints.iter().any(|c| {
        matches!(
            c.kind,
            ConstraintKind::PrimaryKey | ConstraintKind::Unique | ConstraintKind::Exclusion
        ) && c.name == idx.name
    })
}

fn format_mysql_default_literal(col: &SnapshotColumn, raw_default: &str) -> String {
    let trimmed = raw_default.trim();
    if trimmed.is_empty() {
        return "''".into();
    }
    if (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('(') && trimmed.ends_with(')'))
    {
        return trimmed.to_owned();
    }
    let upper = trimmed.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "NULL"
            | "TRUE"
            | "FALSE"
            | "CURRENT_TIMESTAMP"
            | "CURRENT_TIMESTAMP()"
            | "NOW()"
            | "LOCALTIME"
            | "LOCALTIME()"
            | "LOCALTIMESTAMP"
            | "LOCALTIMESTAMP()"
            | "CURRENT_DATE"
            | "CURRENT_DATE()"
            | "CURRENT_TIME"
            | "CURRENT_TIME()"
    ) || upper.starts_with("CURRENT_TIMESTAMP(")
        || upper.starts_with("NOW(")
    {
        return trimmed.to_owned();
    }
    let dtype_lower = col.data_type.to_ascii_lowercase();
    let is_numeric_type = [
        "int", "decimal", "numeric", "float", "double", "real", "bit", "bool", "serial", "year",
    ]
    .iter()
    .any(|kw| dtype_lower.contains(kw));
    if is_numeric_type && (trimmed.parse::<i64>().is_ok() || trimmed.parse::<f64>().is_ok()) {
        return trimmed.to_owned();
    }
    quote_sql_literal(trimmed)
}

fn format_column_ddl(
    col: &SnapshotColumn,
    engine: EngineKind,
    source_schema: &str,
    target_schema: &str,
) -> String {
    let col_ident = quote_engine_identifier(engine, &col.name);
    match engine {
        EngineKind::Postgres | EngineKind::Redis | EngineKind::Snowflake => {
            let mut parts = vec![col_ident, col.data_type.clone()];
            if let Some(gen_kind) = &col.generated
                && !gen_kind.trim().is_empty()
            {
                let expr = col
                    .default_expression
                    .as_deref()
                    .map(|e| retarget_schema_sql(e, engine, source_schema, target_schema))
                    .unwrap_or_else(|| "NULL".into());
                parts.push(format!("GENERATED ALWAYS AS ({expr}) {}", gen_kind.trim()));
            } else if let Some(id_mode) = &col.identity
                && !id_mode.trim().is_empty()
            {
                parts.push(format!("GENERATED {} AS IDENTITY", id_mode.trim()));
            } else if let Some(def) = &col.default_expression
                && !def.trim().is_empty()
            {
                parts.push(format!(
                    "DEFAULT {}",
                    retarget_schema_sql(def, engine, source_schema, target_schema)
                ));
            }
            if !col.nullable {
                parts.push("NOT NULL".into());
            }
            parts.join(" ")
        }
        EngineKind::Mysql => {
            let mut parts = vec![col_ident, col.data_type.clone()];
            if let Some(gen_kind) = &col.generated
                && !gen_kind.trim().is_empty()
            {
                let expr = col
                    .default_expression
                    .as_deref()
                    .map(|e| retarget_schema_sql(e, engine, source_schema, target_schema))
                    .unwrap_or_else(|| "NULL".into());
                parts.push(format!("GENERATED ALWAYS AS ({expr}) {}", gen_kind.trim()));
                if !col.nullable {
                    parts.push("NOT NULL".into());
                }
            } else {
                parts.push(if col.nullable {
                    "NULL".into()
                } else {
                    "NOT NULL".into()
                });
                if col.identity.as_deref() == Some("AUTO_INCREMENT") {
                    parts.push("AUTO_INCREMENT".into());
                } else if let Some(def) = &col.default_expression
                    && !def.trim().is_empty()
                {
                    let retargeted = retarget_schema_sql(def, engine, source_schema, target_schema);
                    parts.push(format!(
                        "DEFAULT {}",
                        format_mysql_default_literal(col, &retargeted)
                    ));
                }
            }
            parts.join(" ")
        }
    }
}

fn format_inline_constraint_ddl(
    c: &ConstraintInfo,
    engine: EngineKind,
    source_schema: &str,
    target_schema: &str,
) -> Option<String> {
    if c.kind == ConstraintKind::ForeignKey {
        return None;
    }
    match engine {
        EngineKind::Postgres | EngineKind::Redis | EngineKind::Snowflake => {
            let def = if !c.definition.trim().is_empty() {
                retarget_schema_sql(&c.definition, engine, source_schema, target_schema)
            } else {
                let cols = c
                    .columns
                    .iter()
                    .map(|col| quote_engine_identifier(engine, col))
                    .collect::<Vec<_>>()
                    .join(", ");
                match c.kind {
                    ConstraintKind::PrimaryKey => format!("PRIMARY KEY ({cols})"),
                    ConstraintKind::Unique => format!("UNIQUE ({cols})"),
                    ConstraintKind::Check => "CHECK (true)".into(),
                    ConstraintKind::Exclusion => format!("EXCLUDE ({cols})"),
                    ConstraintKind::ForeignKey => return None,
                }
            };
            Some(format!(
                "CONSTRAINT {} {def}",
                quote_engine_identifier(engine, &c.name)
            ))
        }
        EngineKind::Mysql => {
            let cols = c
                .columns
                .iter()
                .map(|col| quote_engine_identifier(engine, col))
                .collect::<Vec<_>>()
                .join(", ");
            match c.kind {
                ConstraintKind::PrimaryKey => Some(format!("PRIMARY KEY ({cols})")),
                ConstraintKind::Unique => Some(format!(
                    "CONSTRAINT {} UNIQUE ({cols})",
                    quote_engine_identifier(engine, &c.name)
                )),
                ConstraintKind::Check => {
                    let def =
                        retarget_schema_sql(&c.definition, engine, source_schema, target_schema);
                    if def.is_empty() || def == "CHECK (...)" {
                        None
                    } else {
                        Some(format!(
                            "CONSTRAINT {} {def}",
                            quote_engine_identifier(engine, &c.name)
                        ))
                    }
                }
                ConstraintKind::Exclusion | ConstraintKind::ForeignKey => None,
            }
        }
    }
}

fn build_create_sequence_stmt(
    seq: &SnapshotSequence,
    engine: EngineKind,
    target_schema: &str,
) -> DdlStatement {
    let qualified = quote_qualified(engine, target_schema, &seq.name);
    let sql = match engine {
        EngineKind::Mysql => {
            let mut parts = vec![format!("CREATE SEQUENCE {qualified}")];
            if !seq.increment_by.is_empty() {
                parts.push(format!("INCREMENT BY {}", seq.increment_by));
            }
            if !seq.min_value.is_empty() {
                parts.push(format!("MINVALUE {}", seq.min_value));
            }
            if !seq.max_value.is_empty() {
                parts.push(format!("MAXVALUE {}", seq.max_value));
            }
            if !seq.start_value.is_empty() {
                parts.push(format!("START WITH {}", seq.start_value));
            }
            if !seq.cache_size.is_empty() {
                parts.push(format!("CACHE {}", seq.cache_size));
            }
            parts.push(if seq.cycle {
                "CYCLE".into()
            } else {
                "NOCYCLE".into()
            });
            format!("{};", parts.join(" "))
        }
        _ => {
            let mut parts = vec![format!("CREATE SEQUENCE {qualified}")];
            if !seq.data_type.is_empty() {
                parts.push(format!("AS {}", seq.data_type));
            }
            if !seq.increment_by.is_empty() {
                parts.push(format!("INCREMENT BY {}", seq.increment_by));
            }
            if !seq.min_value.is_empty() {
                parts.push(format!("MINVALUE {}", seq.min_value));
            }
            if !seq.max_value.is_empty() {
                parts.push(format!("MAXVALUE {}", seq.max_value));
            }
            if !seq.start_value.is_empty() {
                parts.push(format!("START WITH {}", seq.start_value));
            }
            if !seq.cache_size.is_empty() {
                parts.push(format!("CACHE {}", seq.cache_size));
            }
            parts.push(if seq.cycle {
                "CYCLE".into()
            } else {
                "NO CYCLE".into()
            });
            format!("{};", parts.join(" "))
        }
    };
    DdlStatement {
        sql,
        description: format!("Create sequence {}", seq.name),
        object_kind: DatabaseObjectKind::Sequence,
        object_name: seq.name.clone(),
        destructive: false,
    }
}

fn build_alter_sequence_stmt(
    seq: &SnapshotSequence,
    engine: EngineKind,
    target_schema: &str,
) -> DdlStatement {
    let qualified = quote_qualified(engine, target_schema, &seq.name);
    let sql = match engine {
        EngineKind::Mysql => {
            let mut parts = vec![format!("ALTER SEQUENCE {qualified}")];
            if !seq.increment_by.is_empty() {
                parts.push(format!("INCREMENT BY {}", seq.increment_by));
            }
            if !seq.min_value.is_empty() {
                parts.push(format!("MINVALUE {}", seq.min_value));
            }
            if !seq.max_value.is_empty() {
                parts.push(format!("MAXVALUE {}", seq.max_value));
            }
            if !seq.start_value.is_empty() {
                parts.push(format!("RESTART WITH {}", seq.start_value));
            }
            if !seq.cache_size.is_empty() {
                parts.push(format!("CACHE {}", seq.cache_size));
            }
            parts.push(if seq.cycle {
                "CYCLE".into()
            } else {
                "NOCYCLE".into()
            });
            format!("{};", parts.join(" "))
        }
        _ => {
            let mut parts = vec![format!("ALTER SEQUENCE {qualified}")];
            if !seq.data_type.is_empty() {
                parts.push(format!("AS {}", seq.data_type));
            }
            if !seq.increment_by.is_empty() {
                parts.push(format!("INCREMENT BY {}", seq.increment_by));
            }
            if !seq.min_value.is_empty() {
                parts.push(format!("MINVALUE {}", seq.min_value));
            }
            if !seq.max_value.is_empty() {
                parts.push(format!("MAXVALUE {}", seq.max_value));
            }
            if !seq.start_value.is_empty() {
                parts.push(format!("RESTART WITH {}", seq.start_value));
            }
            if !seq.cache_size.is_empty() {
                parts.push(format!("CACHE {}", seq.cache_size));
            }
            parts.push(if seq.cycle {
                "CYCLE".into()
            } else {
                "NO CYCLE".into()
            });
            format!("{};", parts.join(" "))
        }
    };
    DdlStatement {
        sql,
        description: format!("Alter sequence {}", seq.name),
        object_kind: DatabaseObjectKind::Sequence,
        object_name: seq.name.clone(),
        destructive: false,
    }
}

fn build_create_table_stmts(
    table: &SnapshotTable,
    engine: EngineKind,
    source_schema: &str,
    target_schema: &str,
) -> Vec<DdlStatement> {
    let qualified = quote_qualified(engine, target_schema, &table.name);
    let mut columns = table.columns.clone();
    columns.sort_by(|a, b| {
        a.ordinal_position
            .cmp(&b.ordinal_position)
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut body_lines: Vec<String> = columns
        .iter()
        .map(|col| {
            format!(
                "  {}",
                format_column_ddl(col, engine, source_schema, target_schema)
            )
        })
        .collect();

    for c in &table.constraints {
        if let Some(inline_sql) =
            format_inline_constraint_ddl(c, engine, source_schema, target_schema)
        {
            body_lines.push(format!("  {inline_sql}"));
        }
    }

    let mut suffix = String::new();
    if let Some(part_key) = &table.partition_key
        && !part_key.trim().is_empty()
    {
        suffix.push_str(&format!(" PARTITION BY {}", part_key.trim()));
    }
    if engine == EngineKind::Mysql
        && let Some(comment) = &table.comment
        && !comment.is_empty()
    {
        suffix.push_str(&format!(" COMMENT = {}", quote_sql_literal(comment)));
    }

    let create_sql = format!(
        "CREATE TABLE {qualified} (\n{}\n){suffix};",
        body_lines.join(",\n")
    );
    let mut stmts = vec![DdlStatement {
        sql: create_sql,
        description: format!("Create table {}", table.name),
        object_kind: table.kind,
        object_name: table.name.clone(),
        destructive: false,
    }];

    if engine != EngineKind::Mysql
        && let Some(comment) = &table.comment
        && !comment.is_empty()
    {
        stmts.push(DdlStatement {
            sql: format!(
                "COMMENT ON TABLE {qualified} IS {};",
                quote_sql_literal(comment)
            ),
            description: format!("Set comment on table {}", table.name),
            object_kind: table.kind,
            object_name: table.name.clone(),
            destructive: false,
        });
    }
    stmts
}

fn build_create_index_stmt(
    table_name: &str,
    idx: &SnapshotIndex,
    engine: EngineKind,
    source_schema: &str,
    target_schema: &str,
) -> DdlStatement {
    let sql = match engine {
        EngineKind::Postgres | EngineKind::Redis | EngineKind::Snowflake => {
            if let Some(def) = &idx.definition
                && !def.trim().is_empty()
            {
                ensure_semicolon(&retarget_schema_sql(
                    def,
                    engine,
                    source_schema,
                    target_schema,
                ))
            } else {
                let unique = if idx.unique { "UNIQUE " } else { "" };
                let method = if idx.method.trim().is_empty() {
                    "btree"
                } else {
                    idx.method.trim()
                };
                let pred = idx
                    .predicate
                    .as_deref()
                    .filter(|p| !p.trim().is_empty())
                    .map(|p| {
                        format!(
                            " WHERE {}",
                            retarget_schema_sql(p, engine, source_schema, target_schema)
                        )
                    })
                    .unwrap_or_default();
                format!(
                    "CREATE {unique}INDEX {} ON {} USING {method} ({}){pred};",
                    quote_engine_identifier(engine, &idx.name),
                    quote_qualified(engine, target_schema, table_name),
                    idx.expressions.join(", ")
                )
            }
        }
        EngineKind::Mysql => {
            let prefix = if idx.unique {
                "CREATE UNIQUE INDEX"
            } else if idx.method.eq_ignore_ascii_case("fulltext") {
                "CREATE FULLTEXT INDEX"
            } else if idx.method.eq_ignore_ascii_case("spatial") {
                "CREATE SPATIAL INDEX"
            } else {
                "CREATE INDEX"
            };
            let using_clause = if idx.method.eq_ignore_ascii_case("hash") {
                " USING HASH"
            } else {
                ""
            };
            format!(
                "{prefix} {} ON {} ({}){using_clause};",
                quote_engine_identifier(engine, &idx.name),
                quote_qualified(engine, target_schema, table_name),
                idx.expressions.join(", ")
            )
        }
    };

    DdlStatement {
        sql,
        description: format!("Create index {} on {}", idx.name, table_name),
        object_kind: DatabaseObjectKind::Index,
        object_name: format!("{table_name}.{}", idx.name),
        destructive: false,
    }
}

fn build_add_constraint_stmt(
    table_name: &str,
    c: &ConstraintInfo,
    engine: EngineKind,
    source_schema: &str,
    target_schema: &str,
) -> Option<DdlStatement> {
    let qualified_table = quote_qualified(engine, target_schema, table_name);
    let quoted_cname = quote_engine_identifier(engine, &c.name);
    let sql = match engine {
        EngineKind::Postgres | EngineKind::Redis | EngineKind::Snowflake => {
            let def = if !c.definition.trim().is_empty() {
                retarget_schema_sql(&c.definition, engine, source_schema, target_schema)
            } else if c.kind == ConstraintKind::ForeignKey
                && let Some(target) = &c.target
            {
                let cols = c
                    .columns
                    .iter()
                    .map(|col| quote_engine_identifier(engine, col))
                    .collect::<Vec<_>>()
                    .join(", ");
                let ref_cols = c
                    .target_columns
                    .iter()
                    .map(|col| quote_engine_identifier(engine, col))
                    .collect::<Vec<_>>()
                    .join(", ");
                let ref_schema = if target.schema == source_schema {
                    target_schema
                } else {
                    target.schema.as_str()
                };
                format!(
                    "FOREIGN KEY ({cols}) REFERENCES {} ({ref_cols})",
                    quote_qualified(engine, ref_schema, &target.name)
                )
            } else {
                format_inline_constraint_ddl(c, engine, source_schema, target_schema)?
                    .trim_start_matches(&format!("CONSTRAINT {quoted_cname} "))
                    .to_owned()
            };
            format!("ALTER TABLE {qualified_table} ADD CONSTRAINT {quoted_cname} {def};")
        }
        EngineKind::Mysql => {
            let cols = c
                .columns
                .iter()
                .map(|col| quote_engine_identifier(engine, col))
                .collect::<Vec<_>>()
                .join(", ");
            match c.kind {
                ConstraintKind::PrimaryKey => {
                    format!("ALTER TABLE {qualified_table} ADD PRIMARY KEY ({cols});")
                }
                ConstraintKind::Unique => {
                    format!(
                        "ALTER TABLE {qualified_table} ADD CONSTRAINT {quoted_cname} UNIQUE ({cols});"
                    )
                }
                ConstraintKind::Check => {
                    let def =
                        retarget_schema_sql(&c.definition, engine, source_schema, target_schema);
                    if def.is_empty() || def == "CHECK (...)" {
                        return None;
                    }
                    format!("ALTER TABLE {qualified_table} ADD CONSTRAINT {quoted_cname} {def};")
                }
                ConstraintKind::ForeignKey => {
                    let target = c.target.as_ref()?;
                    let ref_cols = c
                        .target_columns
                        .iter()
                        .map(|col| quote_engine_identifier(engine, col))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let ref_schema = if target.schema == source_schema {
                        target_schema
                    } else {
                        target.schema.as_str()
                    };
                    format!(
                        "ALTER TABLE {qualified_table} ADD CONSTRAINT {quoted_cname} FOREIGN KEY ({cols}) REFERENCES {} ({ref_cols});",
                        quote_qualified(engine, ref_schema, &target.name)
                    )
                }
                ConstraintKind::Exclusion => return None,
            }
        }
    };

    Some(DdlStatement {
        sql,
        description: format!("Add constraint {} on {}", c.name, table_name),
        object_kind: DatabaseObjectKind::Table,
        object_name: format!("{table_name}.{}", c.name),
        destructive: false,
    })
}

fn build_drop_constraint_stmt(
    table_name: &str,
    c: &ConstraintInfo,
    engine: EngineKind,
    target_schema: &str,
    destructive: bool,
) -> DdlStatement {
    let qualified_table = quote_qualified(engine, target_schema, table_name);
    let quoted_cname = quote_engine_identifier(engine, &c.name);
    let sql = match engine {
        EngineKind::Mysql => match c.kind {
            ConstraintKind::PrimaryKey => {
                format!("ALTER TABLE {qualified_table} DROP PRIMARY KEY;")
            }
            ConstraintKind::ForeignKey => {
                format!("ALTER TABLE {qualified_table} DROP FOREIGN KEY {quoted_cname};")
            }
            ConstraintKind::Unique => {
                format!("ALTER TABLE {qualified_table} DROP INDEX {quoted_cname};")
            }
            ConstraintKind::Check => {
                format!("ALTER TABLE {qualified_table} DROP CHECK {quoted_cname};")
            }
            ConstraintKind::Exclusion => {
                format!("ALTER TABLE {qualified_table} DROP CONSTRAINT {quoted_cname};")
            }
        },
        _ => format!("ALTER TABLE {qualified_table} DROP CONSTRAINT {quoted_cname};"),
    };
    DdlStatement {
        sql,
        description: format!("Drop constraint {} on {}", c.name, table_name),
        object_kind: DatabaseObjectKind::Table,
        object_name: format!("{table_name}.{}", c.name),
        destructive,
    }
}

fn order_views_by_dependency<'a>(views: &[&'a SnapshotTable]) -> Vec<&'a SnapshotTable> {
    let mut remaining: BTreeMap<&str, &'a SnapshotTable> =
        views.iter().map(|v| (v.name.as_str(), *v)).collect();
    let all_names: BTreeSet<&str> = remaining.keys().copied().collect();
    let mut ordered = Vec::with_capacity(views.len());

    while !remaining.is_empty() {
        let next_name = remaining
            .iter()
            .find(|(name, view)| {
                let def = view.view_definition.as_deref().unwrap_or("");
                !remaining.keys().any(|other| {
                    *other != **name && all_names.contains(other) && def.contains(*other)
                })
            })
            .map(|(name, _)| *name)
            .unwrap_or_else(|| *remaining.keys().next().unwrap());
        if let Some(view) = remaining.remove(next_name) {
            ordered.push(view);
        }
    }
    ordered
}

fn build_create_view_stmts(
    view: &SnapshotTable,
    engine: EngineKind,
    source_schema: &str,
    target_schema: &str,
    warnings: &mut Vec<String>,
) -> Vec<DdlStatement> {
    let Some(raw_def) = view
        .view_definition
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    else {
        warnings.push(format!(
            "View '{}' definition is unavailable due to privilege restrictions and was skipped.",
            view.name
        ));
        return Vec::new();
    };
    let retargeted = retarget_schema_sql(raw_def, engine, source_schema, target_schema);
    let body = retargeted.trim().trim_end_matches(';').trim_end();
    let qualified = quote_qualified(engine, target_schema, &view.name);
    let create_sql = if view.kind == DatabaseObjectKind::MaterializedView {
        format!("CREATE MATERIALIZED VIEW {qualified} AS {body};")
    } else {
        format!("CREATE OR REPLACE VIEW {qualified} AS {body};")
    };

    let mut stmts = vec![DdlStatement {
        sql: create_sql,
        description: format!(
            "Create {} {}",
            if view.kind == DatabaseObjectKind::MaterializedView {
                "materialized view"
            } else {
                "view"
            },
            view.name
        ),
        object_kind: view.kind,
        object_name: view.name.clone(),
        destructive: false,
    }];

    for idx in &view.indexes {
        if !is_constraint_backed_index(view, idx) {
            stmts.push(build_create_index_stmt(
                &view.name,
                idx,
                engine,
                source_schema,
                target_schema,
            ));
        }
    }

    if engine != EngineKind::Mysql
        && let Some(comment) = &view.comment
        && !comment.is_empty()
    {
        let kw = if view.kind == DatabaseObjectKind::MaterializedView {
            "MATERIALIZED VIEW"
        } else {
            "VIEW"
        };
        stmts.push(DdlStatement {
            sql: format!(
                "COMMENT ON {kw} {qualified} IS {};",
                quote_sql_literal(comment)
            ),
            description: format!("Set comment on {} {}", kw.to_ascii_lowercase(), view.name),
            object_kind: view.kind,
            object_name: view.name.clone(),
            destructive: false,
        });
    }

    stmts
}

fn build_create_routine_stmts(
    r: &SnapshotRoutine,
    engine: EngineKind,
    source_schema: &str,
    target_schema: &str,
    warnings: &mut Vec<String>,
) -> Vec<DdlStatement> {
    let Some(raw_def) = r.definition.as_deref().filter(|s| !s.trim().is_empty()) else {
        warnings.push(format!(
            "Routine '{}' definition is unavailable due to privilege restrictions and was skipped.",
            r.name
        ));
        return Vec::new();
    };
    let retargeted = retarget_schema_sql(raw_def, engine, source_schema, target_schema);
    let qualified = quote_qualified(engine, target_schema, &r.name);
    let sql = match engine {
        EngineKind::Mysql => {
            if retargeted
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE ")
            {
                ensure_semicolon(&retargeted)
            } else {
                let kind_kw = if r.kind == DatabaseObjectKind::Procedure {
                    "PROCEDURE"
                } else {
                    "FUNCTION"
                };
                let mut header = format!("CREATE {kind_kw} {qualified}({})", r.arguments);
                if r.kind == DatabaseObjectKind::Function {
                    header.push_str(&format!(
                        " RETURNS {}",
                        r.return_type.as_deref().unwrap_or("INT")
                    ));
                }
                if r.volatility
                    .as_deref()
                    .is_some_and(|v| v.eq_ignore_ascii_case("DETERMINISTIC"))
                {
                    header.push_str(" DETERMINISTIC");
                } else {
                    header.push_str(" NOT DETERMINISTIC");
                }
                if !r.security_definer {
                    header.push_str(" SQL SECURITY INVOKER");
                }
                if let Some(comment) = &r.comment
                    && !comment.is_empty()
                {
                    header.push_str(&format!(" COMMENT {}", quote_sql_literal(comment)));
                }
                ensure_semicolon(&format!("{header} {retargeted}"))
            }
        }
        _ => {
            if retargeted
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE ")
            {
                ensure_semicolon(&retargeted)
            } else {
                let kind_kw = if r.kind == DatabaseObjectKind::Procedure {
                    "PROCEDURE"
                } else {
                    "FUNCTION"
                };
                let ret_clause = if r.kind == DatabaseObjectKind::Function {
                    format!(" RETURNS {}", r.return_type.as_deref().unwrap_or("void"))
                } else {
                    String::new()
                };
                let vol_clause = r
                    .volatility
                    .as_deref()
                    .filter(|v| !v.is_empty() && r.kind == DatabaseObjectKind::Function)
                    .map(|v| format!(" {v}"))
                    .unwrap_or_default();
                let sec_clause = if r.security_definer {
                    " SECURITY DEFINER"
                } else {
                    ""
                };
                format!(
                    "CREATE OR REPLACE {kind_kw} {qualified}({}){ret_clause} LANGUAGE {}{vol_clause}{sec_clause} AS $function$\n{}\n$function$;",
                    r.arguments,
                    if r.language.is_empty() {
                        "sql"
                    } else {
                        &r.language
                    },
                    retargeted.trim()
                )
            }
        }
    };

    let obj_name = if r.identity_arguments.is_empty() {
        format!("{}()", r.name)
    } else {
        format!("{}({})", r.name, r.identity_arguments)
    };
    let mut stmts = vec![DdlStatement {
        sql,
        description: format!(
            "Create {} {}",
            if r.kind == DatabaseObjectKind::Procedure {
                "procedure"
            } else {
                "function"
            },
            obj_name
        ),
        object_kind: r.kind,
        object_name: obj_name.clone(),
        destructive: false,
    }];

    if engine != EngineKind::Mysql
        && let Some(comment) = &r.comment
        && !comment.is_empty()
    {
        let kind_kw = if r.kind == DatabaseObjectKind::Procedure {
            "PROCEDURE"
        } else {
            "FUNCTION"
        };
        stmts.push(DdlStatement {
            sql: format!(
                "COMMENT ON {kind_kw} {qualified}({}) IS {};",
                r.identity_arguments,
                quote_sql_literal(comment)
            ),
            description: format!(
                "Set comment on {} {}",
                kind_kw.to_ascii_lowercase(),
                obj_name
            ),
            object_kind: r.kind,
            object_name: obj_name,
            destructive: false,
        });
    }

    stmts
}

fn build_create_trigger_stmts(
    table_name: &str,
    trig: &TriggerInfo,
    engine: EngineKind,
    source_schema: &str,
    target_schema: &str,
    warnings: &mut Vec<String>,
) -> Vec<DdlStatement> {
    let Some(raw_def) = trig.definition.as_deref().filter(|s| !s.trim().is_empty()) else {
        warnings.push(format!(
            "Trigger '{}.{}' definition is unavailable and was skipped.",
            table_name, trig.name
        ));
        return Vec::new();
    };
    let retargeted = retarget_schema_sql(raw_def, engine, source_schema, target_schema);
    let sql = match engine {
        EngineKind::Mysql => {
            if retargeted
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE ")
            {
                ensure_semicolon(&retargeted)
            } else {
                let events = trig.events.join(" OR ");
                ensure_semicolon(&format!(
                    "CREATE TRIGGER {} {} {events} ON {} FOR EACH {} {retargeted}",
                    quote_qualified(engine, target_schema, &trig.name),
                    trig.timing,
                    quote_qualified(engine, target_schema, table_name),
                    trig.orientation
                ))
            }
        }
        _ => {
            if retargeted
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE ")
            {
                ensure_semicolon(&retargeted)
            } else {
                let events = trig.events.join(" OR ");
                ensure_semicolon(&format!(
                    "CREATE TRIGGER {} {} {events} ON {} FOR EACH {} {retargeted}",
                    quote_engine_identifier(engine, &trig.name),
                    trig.timing,
                    quote_qualified(engine, target_schema, table_name),
                    trig.orientation
                ))
            }
        }
    };

    let mut stmts = vec![DdlStatement {
        sql,
        description: format!("Create trigger {} on {}", trig.name, table_name),
        object_kind: DatabaseObjectKind::Trigger,
        object_name: format!("{table_name}.{}", trig.name),
        destructive: false,
    }];

    if engine == EngineKind::Postgres && !trig.enabled {
        stmts.push(DdlStatement {
            sql: format!(
                "ALTER TABLE {} DISABLE TRIGGER {};",
                quote_qualified(engine, target_schema, table_name),
                quote_engine_identifier(engine, &trig.name)
            ),
            description: format!("Disable trigger {} on {}", trig.name, table_name),
            object_kind: DatabaseObjectKind::Trigger,
            object_name: format!("{table_name}.{}", trig.name),
            destructive: false,
        });
    }

    stmts
}

fn finalize_plan(
    engine: EngineKind,
    target_schema: String,
    statements: Vec<DdlStatement>,
    truncated: bool,
    restricted: bool,
    mut warnings: Vec<String>,
    unsupported: Vec<UnsupportedSchemaObject>,
) -> SchemaDdlPlan {
    let destructive_count = statements.iter().filter(|s| s.destructive).count();
    if truncated {
        warnings.insert(
            0,
            "Catalog inspection limits were reached while capturing the schema snapshot; generated DDL may be incomplete.".into(),
        );
    }
    if restricted {
        warnings.push(
            "One or more objects had restricted metadata due to database privileges; verify omitted definitions before applying.".into(),
        );
    }
    if destructive_count > 0 {
        warnings.push(format!(
            "{destructive_count} destructive statement(s) (DROP) detected; review carefully before executing."
        ));
    }
    if !unsupported.is_empty() {
        warnings.push(format!(
            "{} unsupported schema object(s) were disclosed in the snapshot and are not emitted in DDL.",
            unsupported.len()
        ));
    }
    let script = statements
        .iter()
        .map(|s| s.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    SchemaDdlPlan {
        engine,
        target_schema,
        statements,
        script,
        truncated,
        destructive_count,
        warnings,
        unsupported,
    }
}

/// Generates a deterministic, dependency-ordered full-schema DDL script from a
/// `SchemaSnapshot`. Never executes SQL or mutates database state.
pub fn generate_schema_ddl(
    snapshot: &SchemaSnapshot,
    target_schema: Option<&str>,
) -> AppResult<SchemaDdlPlan> {
    let mut snap = snapshot.clone();
    snap.canonicalize();

    let engine = snap.engine;
    let source_schema = snap.schema.as_str();
    let dst_schema = target_schema
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(source_schema);

    let mut statements = Vec::new();
    let mut warnings = Vec::new();

    // 1. Standalone Sequences
    for seq in &snap.sequences {
        if !is_auto_identity_sequence(&snap, &seq.name) {
            statements.push(build_create_sequence_stmt(seq, engine, dst_schema));
        }
    }

    let base_tables: Vec<&SnapshotTable> = snap
        .tables
        .iter()
        .filter(|t| {
            matches!(
                t.kind,
                DatabaseObjectKind::Table | DatabaseObjectKind::PartitionedTable
            )
        })
        .collect();
    let views: Vec<&SnapshotTable> = snap
        .tables
        .iter()
        .filter(|t| {
            matches!(
                t.kind,
                DatabaseObjectKind::View | DatabaseObjectKind::MaterializedView
            )
        })
        .collect();

    // 2. Base Tables (with inline PK/Unique/Check/Exclude constraints & comments)
    for table in &base_tables {
        statements.extend(build_create_table_stmts(
            table,
            engine,
            source_schema,
            dst_schema,
        ));
    }

    // 3. Secondary Indexes on Base Tables
    for table in &base_tables {
        for idx in &table.indexes {
            if !is_constraint_backed_index(table, idx) {
                statements.push(build_create_index_stmt(
                    &table.name,
                    idx,
                    engine,
                    source_schema,
                    dst_schema,
                ));
            }
        }
    }

    // 4. Outbound Foreign Keys on Base Tables
    for table in &base_tables {
        for c in &table.constraints {
            if c.kind == ConstraintKind::ForeignKey
                && let Some(stmt) =
                    build_add_constraint_stmt(&table.name, c, engine, source_schema, dst_schema)
            {
                statements.push(stmt);
            }
        }
    }

    // 5. Views & Materialized Views in dependency order
    for view in order_views_by_dependency(&views) {
        statements.extend(build_create_view_stmts(
            view,
            engine,
            source_schema,
            dst_schema,
            &mut warnings,
        ));
    }

    // 6. Routines (Functions & Procedures)
    for routine in &snap.routines {
        statements.extend(build_create_routine_stmts(
            routine,
            engine,
            source_schema,
            dst_schema,
            &mut warnings,
        ));
    }

    // 7. Triggers on Base Tables & Views
    for table in &snap.tables {
        for trig in &table.triggers {
            statements.extend(build_create_trigger_stmts(
                &table.name,
                trig,
                engine,
                source_schema,
                dst_schema,
                &mut warnings,
            ));
        }
    }

    Ok(finalize_plan(
        engine,
        dst_schema.to_owned(),
        statements,
        snap.truncated,
        snap.restricted,
        warnings,
        snap.unsupported_objects,
    ))
}

fn order_tables_for_drop<'a>(tables: &[&'a SnapshotTable]) -> Vec<&'a SnapshotTable> {
    // Drop child tables (that have FKs referencing other tables in `tables`) first.
    let mut remaining: BTreeMap<&str, &'a SnapshotTable> =
        tables.iter().map(|t| (t.name.as_str(), *t)).collect();
    let mut ordered = Vec::with_capacity(tables.len());

    while !remaining.is_empty() {
        let next_name = remaining
            .iter()
            .find(|(name, _)| {
                // Find a table that is NOT referenced by any OTHER remaining table.
                !remaining.iter().any(|(other_name, other_tbl)| {
                    *other_name != **name
                        && other_tbl.constraints.iter().any(|c| {
                            c.kind == ConstraintKind::ForeignKey
                                && c.target
                                    .as_ref()
                                    .is_some_and(|target| target.name == **name)
                        })
                })
            })
            .map(|(name, _)| *name)
            .unwrap_or_else(|| *remaining.keys().next().unwrap());
        if let Some(tbl) = remaining.remove(next_name) {
            ordered.push(tbl);
        }
    }
    ordered
}

/// Generates a deterministic, dependency-ordered migration DDL plan that
/// transforms `left` into `right` (targeted at `target_schema`, which defaults
/// to `left.schema`). Never executes SQL or mutates database state.
pub fn generate_migration_ddl(
    left: &SchemaSnapshot,
    right: &SchemaSnapshot,
    target_schema: Option<&str>,
) -> AppResult<SchemaDdlPlan> {
    if left.engine != right.engine {
        return Err(AppError::invalid(format!(
            "Cannot generate migration DDL across different engines ({:?} vs {:?})",
            left.engine, right.engine
        )));
    }

    let mut l_snap = left.clone();
    let mut r_snap = right.clone();
    l_snap.canonicalize();
    r_snap.canonicalize();

    let engine = l_snap.engine;
    let left_schema = l_snap.schema.as_str();
    let right_schema = r_snap.schema.as_str();
    let dst_schema = target_schema
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(left_schema);

    let mut unsupported = l_snap.unsupported_objects.clone();
    for u in &r_snap.unsupported_objects {
        if !unsupported
            .iter()
            .any(|existing| existing.name == u.name && existing.object_type == u.object_type)
        {
            unsupported.push(u.clone());
        }
    }
    unsupported.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.object_type.cmp(&b.object_type))
    });

    let diff = compare_schema_snapshots(l_snap.clone(), r_snap.clone());
    if diff.entries.is_empty() {
        return Ok(finalize_plan(
            engine,
            dst_schema.to_owned(),
            Vec::new(),
            l_snap.truncated || r_snap.truncated,
            l_snap.restricted || r_snap.restricted,
            Vec::new(),
            unsupported,
        ));
    }

    let mut statements = Vec::new();
    let mut warnings = Vec::new();

    let left_tables: BTreeMap<&str, &SnapshotTable> =
        l_snap.tables.iter().map(|t| (t.name.as_str(), t)).collect();
    let right_tables: BTreeMap<&str, &SnapshotTable> =
        r_snap.tables.iter().map(|t| (t.name.as_str(), t)).collect();

    // Determine views to drop & recreate:
    // If any view is removed/modified or if any table/view is modified/dropped, dropping and recreating
    // affected views in topological order avoids PostgreSQL `cannot change name/type of view column` errors.
    let any_relation_changed = diff.entries.iter().any(|e| {
        matches!(
            e.object_kind,
            DatabaseObjectKind::Table
                | DatabaseObjectKind::PartitionedTable
                | DatabaseObjectKind::View
                | DatabaseObjectKind::MaterializedView
        )
    });
    let left_views: Vec<&SnapshotTable> = l_snap
        .tables
        .iter()
        .filter(|t| {
            matches!(
                t.kind,
                DatabaseObjectKind::View | DatabaseObjectKind::MaterializedView
            )
        })
        .collect();
    let right_views: Vec<&SnapshotTable> = r_snap
        .tables
        .iter()
        .filter(|t| {
            matches!(
                t.kind,
                DatabaseObjectKind::View | DatabaseObjectKind::MaterializedView
            )
        })
        .collect();

    let mut views_to_drop: Vec<&SnapshotTable> = Vec::new();
    let mut views_to_create: Vec<&SnapshotTable> = Vec::new();
    if any_relation_changed {
        let ordered_left_views = order_views_by_dependency(&left_views);
        for v in ordered_left_views.into_iter().rev() {
            views_to_drop.push(v);
        }
        views_to_create = order_views_by_dependency(&right_views);
    }

    // Phase 1a: Drop triggers that are removed or modified on surviving tables
    for (tbl_name, l_tbl) in &left_tables {
        let Some(r_tbl) = right_tables.get(tbl_name) else {
            continue;
        };
        if l_tbl.kind != r_tbl.kind {
            continue;
        }
        let r_trigs: BTreeMap<&str, &TriggerInfo> = r_tbl
            .triggers
            .iter()
            .map(|t| (t.name.as_str(), t))
            .collect();
        for l_trig in &l_tbl.triggers {
            let status = match r_trigs.get(l_trig.name.as_str()) {
                None => Some(true),
                Some(r_trig)
                    if format_trigger_spec(l_trig, engine, left_schema)
                        != format_trigger_spec(r_trig, engine, right_schema) =>
                {
                    Some(false)
                }
                _ => None,
            };
            if let Some(destructive) = status {
                let sql = match engine {
                    EngineKind::Mysql => format!(
                        "DROP TRIGGER {};",
                        quote_qualified(engine, dst_schema, &l_trig.name)
                    ),
                    _ => format!(
                        "DROP TRIGGER {} ON {};",
                        quote_engine_identifier(engine, &l_trig.name),
                        quote_qualified(engine, dst_schema, tbl_name)
                    ),
                };
                statements.push(DdlStatement {
                    sql,
                    description: format!("Drop trigger {} on {}", l_trig.name, tbl_name),
                    object_kind: DatabaseObjectKind::Trigger,
                    object_name: format!("{tbl_name}.{}", l_trig.name),
                    destructive,
                });
            }
        }
    }

    // Phase 1b: Drop views & materialized views (reverse dependency order)
    for v in &views_to_drop {
        let is_removed = !right_tables
            .get(v.name.as_str())
            .is_some_and(|rv| rv.kind == v.kind);
        let kw = if v.kind == DatabaseObjectKind::MaterializedView {
            "MATERIALIZED VIEW"
        } else {
            "VIEW"
        };
        statements.push(DdlStatement {
            sql: format!(
                "DROP {kw} {};",
                quote_qualified(engine, dst_schema, &v.name)
            ),
            description: format!("Drop {} {}", kw.to_ascii_lowercase(), v.name),
            object_kind: v.kind,
            object_name: v.name.clone(),
            destructive: is_removed,
        });
    }

    // Phase 1c: Drop removed or modified routines
    type RoutineKey<'a> = (&'a str, DatabaseObjectKind, &'a str);
    let left_routines: BTreeMap<RoutineKey<'_>, &SnapshotRoutine> = l_snap
        .routines
        .iter()
        .map(|r| ((r.name.as_str(), r.kind, r.identity_arguments.as_str()), r))
        .collect();
    let right_routines: BTreeMap<RoutineKey<'_>, &SnapshotRoutine> = r_snap
        .routines
        .iter()
        .map(|r| ((r.name.as_str(), r.kind, r.identity_arguments.as_str()), r))
        .collect();

    for (key, l_rt) in &left_routines {
        let status = match right_routines.get(key) {
            None => Some(true),
            Some(r_rt)
                if format_routine_spec(l_rt, engine, left_schema)
                    != format_routine_spec(r_rt, engine, right_schema)
                    || l_rt.comment != r_rt.comment =>
            {
                Some(false)
            }
            _ => None,
        };
        if let Some(destructive) = status {
            let kind_kw = if l_rt.kind == DatabaseObjectKind::Procedure {
                "PROCEDURE"
            } else {
                "FUNCTION"
            };
            let qualified = quote_qualified(engine, dst_schema, &l_rt.name);
            let sql = match engine {
                EngineKind::Mysql => format!("DROP {kind_kw} {qualified};"),
                _ => format!("DROP {kind_kw} {qualified}({});", l_rt.identity_arguments),
            };
            let obj_name = if l_rt.identity_arguments.is_empty() {
                format!("{}()", l_rt.name)
            } else {
                format!("{}({})", l_rt.name, l_rt.identity_arguments)
            };
            statements.push(DdlStatement {
                sql,
                description: format!("Drop {} {}", kind_kw.to_ascii_lowercase(), obj_name),
                object_kind: l_rt.kind,
                object_name: obj_name,
                destructive,
            });
        }
    }

    // Phase 1d: Drop removed or modified Foreign Keys on surviving tables
    for (tbl_name, l_tbl) in &left_tables {
        let Some(r_tbl) = right_tables.get(tbl_name) else {
            continue;
        };
        if !matches!(
            l_tbl.kind,
            DatabaseObjectKind::Table | DatabaseObjectKind::PartitionedTable
        ) || l_tbl.kind != r_tbl.kind
        {
            continue;
        }
        let r_cons: BTreeMap<&str, &ConstraintInfo> = r_tbl
            .constraints
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();
        for l_c in &l_tbl.constraints {
            if l_c.kind != ConstraintKind::ForeignKey {
                continue;
            }
            let status = match r_cons.get(l_c.name.as_str()) {
                None => Some(true),
                Some(r_c)
                    if format_constraint_spec(l_c, engine, left_schema)
                        != format_constraint_spec(r_c, engine, right_schema) =>
                {
                    Some(false)
                }
                _ => None,
            };
            if let Some(destructive) = status {
                statements.push(build_drop_constraint_stmt(
                    tbl_name,
                    l_c,
                    engine,
                    dst_schema,
                    destructive,
                ));
            }
        }
    }

    // Phase 1e: Drop removed or modified secondary indexes & non-FK constraints on surviving tables
    for (tbl_name, l_tbl) in &left_tables {
        let Some(r_tbl) = right_tables.get(tbl_name) else {
            continue;
        };
        if !matches!(
            l_tbl.kind,
            DatabaseObjectKind::Table | DatabaseObjectKind::PartitionedTable
        ) || l_tbl.kind != r_tbl.kind
        {
            continue;
        }
        let r_idxs: BTreeMap<&str, &SnapshotIndex> =
            r_tbl.indexes.iter().map(|i| (i.name.as_str(), i)).collect();
        for l_idx in &l_tbl.indexes {
            if is_constraint_backed_index(l_tbl, l_idx) {
                continue;
            }
            let status = match r_idxs.get(l_idx.name.as_str()) {
                None => Some(true),
                Some(r_idx)
                    if is_constraint_backed_index(r_tbl, r_idx)
                        || format_index_spec(l_idx, engine, left_schema)
                            != format_index_spec(r_idx, engine, right_schema) =>
                {
                    Some(false)
                }
                _ => None,
            };
            if let Some(destructive) = status {
                let sql = match engine {
                    EngineKind::Mysql => format!(
                        "DROP INDEX {} ON {};",
                        quote_engine_identifier(engine, &l_idx.name),
                        quote_qualified(engine, dst_schema, tbl_name)
                    ),
                    _ => format!(
                        "DROP INDEX {};",
                        quote_qualified(engine, dst_schema, &l_idx.name)
                    ),
                };
                statements.push(DdlStatement {
                    sql,
                    description: format!("Drop index {} on {}", l_idx.name, tbl_name),
                    object_kind: DatabaseObjectKind::Index,
                    object_name: format!("{tbl_name}.{}", l_idx.name),
                    destructive,
                });
            }
        }

        let r_cons: BTreeMap<&str, &ConstraintInfo> = r_tbl
            .constraints
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();
        for l_c in &l_tbl.constraints {
            if l_c.kind == ConstraintKind::ForeignKey {
                continue;
            }
            let status = match r_cons.get(l_c.name.as_str()) {
                None => Some(true),
                Some(r_c)
                    if format_constraint_spec(l_c, engine, left_schema)
                        != format_constraint_spec(r_c, engine, right_schema) =>
                {
                    Some(false)
                }
                _ => None,
            };
            if let Some(destructive) = status {
                statements.push(build_drop_constraint_stmt(
                    tbl_name,
                    l_c,
                    engine,
                    dst_schema,
                    destructive,
                ));
            }
        }
    }

    // Phase 2: Drop removed tables (ordered child-first) & removed sequences
    let tables_to_drop: Vec<&SnapshotTable> = l_snap
        .tables
        .iter()
        .filter(|t| {
            matches!(
                t.kind,
                DatabaseObjectKind::Table | DatabaseObjectKind::PartitionedTable
            ) && !right_tables
                .get(t.name.as_str())
                .is_some_and(|rt| rt.kind == t.kind)
        })
        .collect();
    for tbl in order_tables_for_drop(&tables_to_drop) {
        statements.push(DdlStatement {
            sql: format!(
                "DROP TABLE {};",
                quote_qualified(engine, dst_schema, &tbl.name)
            ),
            description: format!("Drop table {}", tbl.name),
            object_kind: tbl.kind,
            object_name: tbl.name.clone(),
            destructive: true,
        });
    }

    let left_seqs: BTreeMap<&str, &SnapshotSequence> = l_snap
        .sequences
        .iter()
        .filter(|s| !is_auto_identity_sequence(&l_snap, &s.name))
        .map(|s| (s.name.as_str(), s))
        .collect();
    let right_seqs: BTreeMap<&str, &SnapshotSequence> = r_snap
        .sequences
        .iter()
        .filter(|s| !is_auto_identity_sequence(&r_snap, &s.name))
        .map(|s| (s.name.as_str(), s))
        .collect();

    for (seq_name, l_seq) in &left_seqs {
        if !right_seqs.contains_key(seq_name) {
            statements.push(DdlStatement {
                sql: format!(
                    "DROP SEQUENCE {};",
                    quote_qualified(engine, dst_schema, &l_seq.name)
                ),
                description: format!("Drop sequence {}", l_seq.name),
                object_kind: DatabaseObjectKind::Sequence,
                object_name: l_seq.name.clone(),
                destructive: true,
            });
        }
    }

    // Phase 3: Create new sequences & alter modified sequences
    for (seq_name, r_seq) in &right_seqs {
        match left_seqs.get(seq_name) {
            None => {
                statements.push(build_create_sequence_stmt(r_seq, engine, dst_schema));
            }
            Some(l_seq) if format_sequence_spec(l_seq) != format_sequence_spec(r_seq) => {
                statements.push(build_alter_sequence_stmt(r_seq, engine, dst_schema));
            }
            _ => {}
        }
    }

    // Phase 4: Create new tables & alter surviving tables
    let right_base_tables: Vec<&SnapshotTable> = r_snap
        .tables
        .iter()
        .filter(|t| {
            matches!(
                t.kind,
                DatabaseObjectKind::Table | DatabaseObjectKind::PartitionedTable
            )
        })
        .collect();

    for r_tbl in &right_base_tables {
        let Some(l_tbl) = left_tables
            .get(r_tbl.name.as_str())
            .copied()
            .filter(|lt| lt.kind == r_tbl.kind)
        else {
            statements.extend(build_create_table_stmts(
                r_tbl,
                engine,
                right_schema,
                dst_schema,
            ));
            continue;
        };

        let qualified_table = quote_qualified(engine, dst_schema, &r_tbl.name);
        let l_cols: BTreeMap<&str, &SnapshotColumn> =
            l_tbl.columns.iter().map(|c| (c.name.as_str(), c)).collect();
        let r_cols: BTreeMap<&str, &SnapshotColumn> =
            r_tbl.columns.iter().map(|c| (c.name.as_str(), c)).collect();

        // 4a: Drop removed columns
        for l_col in &l_tbl.columns {
            if !r_cols.contains_key(l_col.name.as_str()) {
                statements.push(DdlStatement {
                    sql: format!(
                        "ALTER TABLE {qualified_table} DROP COLUMN {};",
                        quote_engine_identifier(engine, &l_col.name)
                    ),
                    description: format!("Drop column {}.{}", r_tbl.name, l_col.name),
                    object_kind: r_tbl.kind,
                    object_name: format!("{}.{}", r_tbl.name, l_col.name),
                    destructive: true,
                });
            }
        }

        // 4b: Add new columns (in ordinal order)
        let mut sorted_r_cols = r_tbl.columns.clone();
        sorted_r_cols.sort_by(|a, b| {
            a.ordinal_position
                .cmp(&b.ordinal_position)
                .then_with(|| a.name.cmp(&b.name))
        });
        for r_col in &sorted_r_cols {
            if !l_cols.contains_key(r_col.name.as_str()) {
                statements.push(DdlStatement {
                    sql: format!(
                        "ALTER TABLE {qualified_table} ADD COLUMN {};",
                        format_column_ddl(r_col, engine, right_schema, dst_schema)
                    ),
                    description: format!("Add column {}.{}", r_tbl.name, r_col.name),
                    object_kind: r_tbl.kind,
                    object_name: format!("{}.{}", r_tbl.name, r_col.name),
                    destructive: false,
                });
            }
        }

        // 4c: Alter modified columns
        for r_col in &sorted_r_cols {
            let Some(l_col) = l_cols.get(r_col.name.as_str()) else {
                continue;
            };
            if format_column_spec(l_col, engine, left_schema)
                == format_column_spec(r_col, engine, right_schema)
            {
                continue;
            }
            let quoted_col = quote_engine_identifier(engine, &r_col.name);
            match engine {
                EngineKind::Mysql => {
                    statements.push(DdlStatement {
                        sql: format!(
                            "ALTER TABLE {qualified_table} MODIFY COLUMN {};",
                            format_column_ddl(r_col, engine, right_schema, dst_schema)
                        ),
                        description: format!("Modify column {}.{}", r_tbl.name, r_col.name),
                        object_kind: r_tbl.kind,
                        object_name: format!("{}.{}", r_tbl.name, r_col.name),
                        destructive: false,
                    });
                }
                _ => {
                    if l_col.data_type != r_col.data_type {
                        statements.push(DdlStatement {
                            sql: format!(
                                "ALTER TABLE {qualified_table} ALTER COLUMN {quoted_col} TYPE {};",
                                r_col.data_type
                            ),
                            description: format!(
                                "Change type of {}.{} to {}",
                                r_tbl.name, r_col.name, r_col.data_type
                            ),
                            object_kind: r_tbl.kind,
                            object_name: format!("{}.{}", r_tbl.name, r_col.name),
                            destructive: false,
                        });
                    }
                    if l_col.nullable != r_col.nullable {
                        let clause = if r_col.nullable {
                            "DROP NOT NULL"
                        } else {
                            "SET NOT NULL"
                        };
                        statements.push(DdlStatement {
                            sql: format!(
                                "ALTER TABLE {qualified_table} ALTER COLUMN {quoted_col} {clause};"
                            ),
                            description: format!("{} on {}.{}", clause, r_tbl.name, r_col.name),
                            object_kind: r_tbl.kind,
                            object_name: format!("{}.{}", r_tbl.name, r_col.name),
                            destructive: false,
                        });
                    }
                    if l_col.identity != r_col.identity {
                        let id_sql = match (&l_col.identity, &r_col.identity) {
                            (Some(_), None) => format!(
                                "ALTER TABLE {qualified_table} ALTER COLUMN {quoted_col} DROP IDENTITY IF EXISTS;"
                            ),
                            (None, Some(mode)) => format!(
                                "ALTER TABLE {qualified_table} ALTER COLUMN {quoted_col} ADD GENERATED {} AS IDENTITY;",
                                mode.trim()
                            ),
                            (Some(_), Some(mode)) => format!(
                                "ALTER TABLE {qualified_table} ALTER COLUMN {quoted_col} SET GENERATED {};",
                                mode.trim()
                            ),
                            (None, None) => String::new(),
                        };
                        if !id_sql.is_empty() {
                            statements.push(DdlStatement {
                                sql: id_sql,
                                description: format!(
                                    "Update identity on {}.{}",
                                    r_tbl.name, r_col.name
                                ),
                                object_kind: r_tbl.kind,
                                object_name: format!("{}.{}", r_tbl.name, r_col.name),
                                destructive: false,
                            });
                        }
                    }
                    if r_col.generated.is_none() && r_col.identity.is_none() {
                        let l_def = l_col
                            .default_expression
                            .as_deref()
                            .map(|d| normalize_self_schema_sql(d, engine, left_schema));
                        let r_def = r_col
                            .default_expression
                            .as_deref()
                            .map(|d| normalize_self_schema_sql(d, engine, right_schema));
                        if l_def != r_def {
                            let def_sql = match &r_col.default_expression {
                                Some(def) if !def.trim().is_empty() => format!(
                                    "ALTER TABLE {qualified_table} ALTER COLUMN {quoted_col} SET DEFAULT {};",
                                    retarget_schema_sql(def, engine, right_schema, dst_schema)
                                ),
                                _ => format!(
                                    "ALTER TABLE {qualified_table} ALTER COLUMN {quoted_col} DROP DEFAULT;"
                                ),
                            };
                            statements.push(DdlStatement {
                                sql: def_sql,
                                description: format!(
                                    "Update default on {}.{}",
                                    r_tbl.name, r_col.name
                                ),
                                object_kind: r_tbl.kind,
                                object_name: format!("{}.{}", r_tbl.name, r_col.name),
                                destructive: false,
                            });
                        }
                    }
                }
            }
        }

        // 4d: Update table comment if changed
        if l_tbl.comment != r_tbl.comment {
            let sql = match engine {
                EngineKind::Mysql => format!(
                    "ALTER TABLE {qualified_table} COMMENT = {};",
                    quote_sql_literal(r_tbl.comment.as_deref().unwrap_or(""))
                ),
                _ => match &r_tbl.comment {
                    Some(c) if !c.is_empty() => {
                        format!(
                            "COMMENT ON TABLE {qualified_table} IS {};",
                            quote_sql_literal(c)
                        )
                    }
                    _ => format!("COMMENT ON TABLE {qualified_table} IS NULL;"),
                },
            };
            statements.push(DdlStatement {
                sql,
                description: format!("Update comment on table {}", r_tbl.name),
                object_kind: r_tbl.kind,
                object_name: r_tbl.name.clone(),
                destructive: false,
            });
        }
    }

    // Phase 5a: Add new/modified non-FK constraints on surviving tables
    for r_tbl in &right_base_tables {
        let Some(l_tbl) = left_tables
            .get(r_tbl.name.as_str())
            .copied()
            .filter(|lt| lt.kind == r_tbl.kind)
        else {
            continue;
        };
        let l_cons: BTreeMap<&str, &ConstraintInfo> = l_tbl
            .constraints
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();
        for r_c in &r_tbl.constraints {
            if r_c.kind == ConstraintKind::ForeignKey {
                continue;
            }
            let needs_add = match l_cons.get(r_c.name.as_str()) {
                None => true,
                Some(l_c) => {
                    format_constraint_spec(l_c, engine, left_schema)
                        != format_constraint_spec(r_c, engine, right_schema)
                }
            };
            if needs_add
                && let Some(stmt) =
                    build_add_constraint_stmt(&r_tbl.name, r_c, engine, right_schema, dst_schema)
            {
                statements.push(stmt);
            }
        }
    }

    // Phase 5b: Create new/modified secondary indexes on new and surviving base tables
    for r_tbl in &right_base_tables {
        let l_tbl_opt = left_tables
            .get(r_tbl.name.as_str())
            .copied()
            .filter(|lt| lt.kind == r_tbl.kind);
        let l_idxs: BTreeMap<&str, &SnapshotIndex> = l_tbl_opt
            .map(|lt| lt.indexes.iter().map(|i| (i.name.as_str(), i)).collect())
            .unwrap_or_default();

        for r_idx in &r_tbl.indexes {
            if is_constraint_backed_index(r_tbl, r_idx) {
                continue;
            }
            let needs_create = match (l_tbl_opt, l_idxs.get(r_idx.name.as_str())) {
                (None, _) | (_, None) => true,
                (Some(lt), Some(l_idx)) => {
                    is_constraint_backed_index(lt, l_idx)
                        || format_index_spec(l_idx, engine, left_schema)
                            != format_index_spec(r_idx, engine, right_schema)
                }
            };
            if needs_create {
                statements.push(build_create_index_stmt(
                    &r_tbl.name,
                    r_idx,
                    engine,
                    right_schema,
                    dst_schema,
                ));
            }
        }
    }

    // Phase 5c: Add new/modified Foreign Keys on new and surviving base tables
    for r_tbl in &right_base_tables {
        let l_tbl_opt = left_tables
            .get(r_tbl.name.as_str())
            .copied()
            .filter(|lt| lt.kind == r_tbl.kind);
        let l_cons: BTreeMap<&str, &ConstraintInfo> = l_tbl_opt
            .map(|lt| {
                lt.constraints
                    .iter()
                    .map(|c| (c.name.as_str(), c))
                    .collect()
            })
            .unwrap_or_default();

        for r_c in &r_tbl.constraints {
            if r_c.kind != ConstraintKind::ForeignKey {
                continue;
            }
            let needs_add = match l_cons.get(r_c.name.as_str()) {
                None => true,
                Some(l_c) => {
                    format_constraint_spec(l_c, engine, left_schema)
                        != format_constraint_spec(r_c, engine, right_schema)
                }
            };
            if needs_add
                && let Some(stmt) =
                    build_add_constraint_stmt(&r_tbl.name, r_c, engine, right_schema, dst_schema)
            {
                statements.push(stmt);
            }
        }
    }

    // Phase 6: Create/recreate views & materialized views in dependency order
    for view in &views_to_create {
        statements.extend(build_create_view_stmts(
            view,
            engine,
            right_schema,
            dst_schema,
            &mut warnings,
        ));
    }

    // Phase 7a: Create new or modified routines
    for (key, r_rt) in &right_routines {
        let needs_create = match left_routines.get(key) {
            None => true,
            Some(l_rt) => {
                format_routine_spec(l_rt, engine, left_schema)
                    != format_routine_spec(r_rt, engine, right_schema)
                    || l_rt.comment != r_rt.comment
            }
        };
        if needs_create {
            statements.extend(build_create_routine_stmts(
                r_rt,
                engine,
                right_schema,
                dst_schema,
                &mut warnings,
            ));
        }
    }

    // Phase 7b: Create new or modified triggers
    for r_tbl in &r_snap.tables {
        let l_tbl_opt = left_tables
            .get(r_tbl.name.as_str())
            .copied()
            .filter(|lt| lt.kind == r_tbl.kind);
        let l_trigs: BTreeMap<&str, &TriggerInfo> = l_tbl_opt
            .map(|lt| lt.triggers.iter().map(|t| (t.name.as_str(), t)).collect())
            .unwrap_or_default();

        for r_trig in &r_tbl.triggers {
            let needs_create = match l_trigs.get(r_trig.name.as_str()) {
                None => true,
                Some(l_trig) => {
                    format_trigger_spec(l_trig, engine, left_schema)
                        != format_trigger_spec(r_trig, engine, right_schema)
                }
            };
            if needs_create {
                statements.extend(build_create_trigger_stmts(
                    &r_tbl.name,
                    r_trig,
                    engine,
                    right_schema,
                    dst_schema,
                    &mut warnings,
                ));
            }
        }
    }

    Ok(finalize_plan(
        engine,
        dst_schema.to_owned(),
        statements,
        l_snap.truncated || r_snap.truncated,
        l_snap.restricted || r_snap.restricted,
        warnings,
        unsupported,
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum IndexConstraintAction {
    CreateIndex {
        table: TableRef,
        name: String,
        unique: bool,
        method: String,
        #[serde(default)]
        concurrently: bool,
        expressions: Vec<String>,
        #[serde(default)]
        raw_expressions: bool,
        predicate: Option<String>,
    },
    DropIndex {
        table: TableRef,
        name: String,
        #[serde(default)]
        concurrently: bool,
    },
    AddConstraint {
        table: TableRef,
        name: String,
        kind: ConstraintKind,
        columns: Vec<String>,
        check_expression: Option<String>,
        target: Option<TableRef>,
        #[serde(default)]
        target_columns: Vec<String>,
        on_delete: Option<String>,
        on_update: Option<String>,
        #[serde(default)]
        not_valid: bool,
    },
    DropConstraint {
        table: TableRef,
        name: String,
        kind: ConstraintKind,
        #[serde(default)]
        cascade: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndexConstraintPreview {
    pub statement: DdlStatement,
    pub warnings: Vec<String>,
}

const MAX_IDENTIFIER_CHARS: usize = 128;
const MAX_EXPRESSION_CHARS: usize = 2048;
const MAX_KEY_COUNT: usize = 32;

const ALLOWED_REFERENTIAL_ACTIONS: &[&str] = &[
    "NO ACTION",
    "RESTRICT",
    "CASCADE",
    "SET NULL",
    "SET DEFAULT",
];

fn validate_identifier<'a>(raw: &'a str, label: &str) -> AppResult<&'a str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("Enter a valid {label}"),
        ));
    }
    if trimmed.chars().count() > MAX_IDENTIFIER_CHARS || trimmed.contains('\0') {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{label} must be at most {MAX_IDENTIFIER_CHARS} characters without NUL bytes"),
        ));
    }
    Ok(trimmed)
}

fn validate_table_ref(table: &TableRef, label: &str) -> AppResult<()> {
    validate_identifier(&table.schema, &format!("{label} schema"))?;
    validate_identifier(&table.name, &format!("{label} table"))?;
    Ok(())
}

fn validate_sql_fragment<'a>(raw: &'a str, label: &str) -> AppResult<&'a str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("Enter a non-empty {label}"),
        ));
    }
    if trimmed.chars().count() > MAX_EXPRESSION_CHARS || trimmed.contains('\0') {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{label} exceeds the {MAX_EXPRESSION_CHARS}-character limit"),
        ));
    }
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[i];
                i += 1;
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        if bytes.get(i + 1) == Some(&quote) {
                            i += 2;
                        } else {
                            i += 1;
                            closed = true;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                if !closed {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        format!("{label} contains an unterminated quoted string or identifier"),
                    ));
                }
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    format!("{label} cannot contain SQL line comments"),
                ));
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    format!("{label} cannot contain SQL block comments"),
                ));
            }
            b';' => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    format!("{label} cannot contain a semicolon"),
                ));
            }
            _ => i += 1,
        }
    }
    Ok(trimmed)
}

fn normalize_referential_action(raw: Option<&str>, label: &str) -> AppResult<Option<String>> {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let upper = text.to_ascii_uppercase();
    if !ALLOWED_REFERENTIAL_ACTIONS.contains(&upper.as_str()) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("Unsupported {label} action '{text}'"),
        ));
    }
    Ok(Some(upper))
}

/// Builds a single engine-aware index or constraint DDL statement with lock
/// and transaction impact warnings. Never executes SQL.
pub fn build_index_constraint_ddl(
    engine: EngineKind,
    action: &IndexConstraintAction,
) -> AppResult<IndexConstraintPreview> {
    if !matches!(engine, EngineKind::Postgres | EngineKind::Mysql) {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Index and constraint editing is available for PostgreSQL and MySQL/MariaDB",
        ));
    }

    match action {
        IndexConstraintAction::CreateIndex {
            table,
            name,
            unique,
            method,
            concurrently,
            expressions,
            raw_expressions,
            predicate,
        } => {
            validate_table_ref(table, "Target")?;
            let index_name = validate_identifier(name, "index name")?;
            if expressions.is_empty() || expressions.len() > MAX_KEY_COUNT {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    format!("Choose between 1 and {MAX_KEY_COUNT} index columns or expressions"),
                ));
            }

            let formatted_keys: Vec<String> = if *raw_expressions {
                expressions
                    .iter()
                    .map(|expr| {
                        validate_sql_fragment(expr, "index key expression").map(str::to_owned)
                    })
                    .collect::<AppResult<Vec<_>>>()?
            } else {
                expressions
                    .iter()
                    .map(|col| {
                        let c = validate_identifier(col, "index column")?;
                        Ok(quote_engine_identifier(engine, c))
                    })
                    .collect::<AppResult<Vec<_>>>()?
            };

            let qualified_table = quote_qualified(engine, table.schema.trim(), table.name.trim());
            let quoted_index = quote_engine_identifier(engine, index_name);
            let mut warnings = Vec::new();

            let sql = match engine {
                EngineKind::Postgres => {
                    let norm_method = if method.trim().is_empty() {
                        "btree".to_owned()
                    } else {
                        method.trim().to_ascii_lowercase()
                    };
                    if !matches!(
                        norm_method.as_str(),
                        "btree" | "hash" | "gin" | "gist" | "spgist" | "brin"
                    ) {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            format!("Unsupported PostgreSQL index method '{norm_method}'"),
                        ));
                    }
                    if *unique && norm_method != "btree" {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            "PostgreSQL UNIQUE indexes only support the btree access method",
                        ));
                    }
                    let pred_clause = if let Some(pred) = predicate
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        let valid_pred = validate_sql_fragment(pred, "partial index predicate")?;
                        format!(" WHERE {valid_pred}")
                    } else {
                        String::new()
                    };
                    let unique_kw = if *unique { "UNIQUE " } else { "" };
                    let conc_kw = if *concurrently { "CONCURRENTLY " } else { "" };
                    if *concurrently {
                        warnings.push(
                            "CREATE INDEX CONCURRENTLY avoids blocking writes on the table, but cannot run inside a transaction block and may leave an INVALID index if interrupted."
                                .into(),
                        );
                    } else {
                        warnings.push(
                            "CREATE INDEX without CONCURRENTLY acquires a SHARE lock on the table, blocking INSERT, UPDATE, and DELETE until the index build finishes."
                                .into(),
                        );
                    }
                    format!(
                        "CREATE {unique_kw}INDEX {conc_kw}{quoted_index} ON {qualified_table} USING {norm_method} ({}){pred_clause};",
                        formatted_keys.join(", ")
                    )
                }
                EngineKind::Mysql => {
                    if *concurrently {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "CONCURRENTLY index builds are only supported on PostgreSQL",
                        ));
                    }
                    if predicate.as_deref().is_some_and(|p| !p.trim().is_empty()) {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "Partial index WHERE predicates are only supported on PostgreSQL",
                        ));
                    }
                    let norm_method = if method.trim().is_empty() {
                        "btree".to_owned()
                    } else {
                        method.trim().to_ascii_lowercase()
                    };
                    if !matches!(
                        norm_method.as_str(),
                        "btree" | "hash" | "fulltext" | "spatial"
                    ) {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            format!("Unsupported MySQL/MariaDB index method '{norm_method}'"),
                        ));
                    }
                    if *unique && matches!(norm_method.as_str(), "fulltext" | "spatial") {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            "FULLTEXT and SPATIAL indexes cannot be declared UNIQUE",
                        ));
                    }
                    let prefix = if *unique {
                        "CREATE UNIQUE INDEX"
                    } else if norm_method == "fulltext" {
                        "CREATE FULLTEXT INDEX"
                    } else if norm_method == "spatial" {
                        "CREATE SPATIAL INDEX"
                    } else {
                        "CREATE INDEX"
                    };
                    let using_clause = if norm_method == "hash" {
                        " USING HASH"
                    } else {
                        ""
                    };
                    warnings.push(
                        "CREATE INDEX on MySQL/MariaDB causes an implicit commit of any open transaction on this session."
                            .into(),
                    );
                    format!(
                        "{prefix} {quoted_index} ON {qualified_table} ({}){using_clause};",
                        formatted_keys.join(", ")
                    )
                }
                _ => unreachable!(),
            };

            Ok(IndexConstraintPreview {
                statement: DdlStatement {
                    sql,
                    description: format!("Create index {index_name} on {}", table.name.trim()),
                    object_kind: DatabaseObjectKind::Index,
                    object_name: format!("{}.{index_name}", table.name.trim()),
                    destructive: false,
                },
                warnings,
            })
        }
        IndexConstraintAction::DropIndex {
            table,
            name,
            concurrently,
        } => {
            validate_table_ref(table, "Target")?;
            let index_name = validate_identifier(name, "index name")?;
            let mut warnings = vec![format!(
                "Dropping index '{index_name}' permanently removes its access path and may degrade query performance."
            )];
            let sql = match engine {
                EngineKind::Postgres => {
                    let conc_kw = if *concurrently { "CONCURRENTLY " } else { "" };
                    if *concurrently {
                        warnings.push(
                            "DROP INDEX CONCURRENTLY avoids locking out concurrent reads and writes, but cannot run inside a transaction block."
                                .into(),
                        );
                    }
                    format!(
                        "DROP INDEX {conc_kw}{};",
                        quote_qualified(engine, table.schema.trim(), index_name)
                    )
                }
                EngineKind::Mysql => {
                    if *concurrently {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "DROP INDEX CONCURRENTLY is only supported on PostgreSQL",
                        ));
                    }
                    warnings.push(
                        "DROP INDEX on MySQL/MariaDB causes an implicit commit of any open transaction on this session."
                            .into(),
                    );
                    format!(
                        "DROP INDEX {} ON {};",
                        quote_engine_identifier(engine, index_name),
                        quote_qualified(engine, table.schema.trim(), table.name.trim())
                    )
                }
                _ => unreachable!(),
            };

            Ok(IndexConstraintPreview {
                statement: DdlStatement {
                    sql,
                    description: format!("Drop index {index_name} on {}", table.name.trim()),
                    object_kind: DatabaseObjectKind::Index,
                    object_name: format!("{}.{index_name}", table.name.trim()),
                    destructive: true,
                },
                warnings,
            })
        }
        IndexConstraintAction::AddConstraint {
            table,
            name,
            kind,
            columns,
            check_expression,
            target,
            target_columns,
            on_delete,
            on_update,
            not_valid,
        } => {
            validate_table_ref(table, "Target")?;
            let qualified_table = quote_qualified(engine, table.schema.trim(), table.name.trim());
            let cname = if engine == EngineKind::Mysql
                && *kind == ConstraintKind::PrimaryKey
                && name.trim().is_empty()
            {
                "PRIMARY"
            } else {
                validate_identifier(name, "constraint name")?
            };
            let quoted_cname = quote_engine_identifier(engine, cname);
            if *not_valid && engine != EngineKind::Postgres {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "NOT VALID constraints are only supported on PostgreSQL",
                ));
            }
            if *not_valid && !matches!(kind, ConstraintKind::Check | ConstraintKind::ForeignKey) {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "NOT VALID is only supported on CHECK and FOREIGN KEY constraints",
                ));
            }

            let mut warnings = Vec::new();
            if engine == EngineKind::Mysql {
                warnings.push(
                    "ALTER TABLE on MySQL/MariaDB causes an implicit commit of any open transaction on this session."
                        .into(),
                );
            }

            let sql = match kind {
                ConstraintKind::PrimaryKey => {
                    if columns.is_empty() || columns.len() > MAX_KEY_COUNT {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            format!("Choose between 1 and {MAX_KEY_COUNT} columns for PRIMARY KEY"),
                        ));
                    }
                    let quoted_cols = columns
                        .iter()
                        .map(|c| {
                            Ok(quote_engine_identifier(
                                engine,
                                validate_identifier(c, "primary key column")?,
                            ))
                        })
                        .collect::<AppResult<Vec<_>>>()?
                        .join(", ");
                    warnings.push(
                        "Adding a PRIMARY KEY scans the table and requires all key columns to be NOT NULL and unique."
                            .into(),
                    );
                    if engine == EngineKind::Mysql {
                        format!("ALTER TABLE {qualified_table} ADD PRIMARY KEY ({quoted_cols});")
                    } else {
                        format!(
                            "ALTER TABLE {qualified_table} ADD CONSTRAINT {quoted_cname} PRIMARY KEY ({quoted_cols});"
                        )
                    }
                }
                ConstraintKind::Unique => {
                    if columns.is_empty() || columns.len() > MAX_KEY_COUNT {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            format!(
                                "Choose between 1 and {MAX_KEY_COUNT} columns for UNIQUE constraint"
                            ),
                        ));
                    }
                    let quoted_cols = columns
                        .iter()
                        .map(|c| {
                            Ok(quote_engine_identifier(
                                engine,
                                validate_identifier(c, "unique constraint column")?,
                            ))
                        })
                        .collect::<AppResult<Vec<_>>>()?
                        .join(", ");
                    warnings.push(
                        "Adding a UNIQUE constraint builds a backing unique index and rejects duplicate non-NULL key tuples."
                            .into(),
                    );
                    format!(
                        "ALTER TABLE {qualified_table} ADD CONSTRAINT {quoted_cname} UNIQUE ({quoted_cols});"
                    )
                }
                ConstraintKind::Check => {
                    let raw_expr = check_expression.as_deref().unwrap_or("");
                    let valid_expr = validate_sql_fragment(raw_expr, "CHECK expression")?;
                    let stripped = if valid_expr
                        .get(..5)
                        .is_some_and(|p| p.eq_ignore_ascii_case("CHECK"))
                    {
                        valid_expr[5..].trim()
                    } else {
                        valid_expr
                    };
                    let parenthesized = if stripped.starts_with('(') && stripped.ends_with(')') {
                        stripped.to_owned()
                    } else {
                        format!("({stripped})")
                    };
                    let nv = if *not_valid { " NOT VALID" } else { "" };
                    if *not_valid {
                        warnings.push(
                            "NOT VALID skips scanning existing rows and only enforces the CHECK constraint on new or updated rows until VALIDATE CONSTRAINT is run."
                                .into(),
                        );
                    } else {
                        warnings.push(
                            "Adding a CHECK constraint scans the table to verify every existing row satisfies the predicate."
                                .into(),
                        );
                    }
                    format!(
                        "ALTER TABLE {qualified_table} ADD CONSTRAINT {quoted_cname} CHECK {parenthesized}{nv};"
                    )
                }
                ConstraintKind::ForeignKey => {
                    if columns.is_empty() || columns.len() > MAX_KEY_COUNT {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            format!(
                                "Choose between 1 and {MAX_KEY_COUNT} source columns for FOREIGN KEY"
                            ),
                        ));
                    }
                    let Some(target_ref) = target else {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            "Choose a referenced target table for the foreign key",
                        ));
                    };
                    validate_table_ref(target_ref, "Referenced")?;
                    if target_columns.len() != columns.len() {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            "Foreign key source and referenced column counts must match",
                        ));
                    }
                    let quoted_cols = columns
                        .iter()
                        .map(|c| {
                            Ok(quote_engine_identifier(
                                engine,
                                validate_identifier(c, "foreign key column")?,
                            ))
                        })
                        .collect::<AppResult<Vec<_>>>()?
                        .join(", ");
                    let quoted_target_cols = target_columns
                        .iter()
                        .map(|c| {
                            Ok(quote_engine_identifier(
                                engine,
                                validate_identifier(c, "referenced column")?,
                            ))
                        })
                        .collect::<AppResult<Vec<_>>>()?
                        .join(", ");
                    let qualified_target =
                        quote_qualified(engine, target_ref.schema.trim(), target_ref.name.trim());
                    let on_del = normalize_referential_action(on_delete.as_deref(), "ON DELETE")?
                        .map(|a| format!(" ON DELETE {a}"))
                        .unwrap_or_default();
                    let on_upd = normalize_referential_action(on_update.as_deref(), "ON UPDATE")?
                        .map(|a| format!(" ON UPDATE {a}"))
                        .unwrap_or_default();
                    let nv = if *not_valid { " NOT VALID" } else { "" };
                    if *not_valid {
                        warnings.push(
                            "NOT VALID skips scanning existing rows and only enforces the foreign key on new or updated rows until VALIDATE CONSTRAINT is run."
                                .into(),
                        );
                    } else {
                        warnings.push(
                            "Adding a FOREIGN KEY locks and scans the table to verify all existing rows reference valid target keys."
                                .into(),
                        );
                    }
                    format!(
                        "ALTER TABLE {qualified_table} ADD CONSTRAINT {quoted_cname} FOREIGN KEY ({quoted_cols}) REFERENCES {qualified_target} ({quoted_target_cols}){on_del}{on_upd}{nv};"
                    )
                }
                ConstraintKind::Exclusion => {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "PostgreSQL EXCLUDE constraints require custom operator classes; submit them in the SQL editor.",
                    ));
                }
            };

            Ok(IndexConstraintPreview {
                statement: DdlStatement {
                    sql,
                    description: format!("Add constraint {cname} on {}", table.name.trim()),
                    object_kind: DatabaseObjectKind::Table,
                    object_name: format!("{}.{cname}", table.name.trim()),
                    destructive: false,
                },
                warnings,
            })
        }
        IndexConstraintAction::DropConstraint {
            table,
            name,
            kind,
            cascade,
        } => {
            validate_table_ref(table, "Target")?;
            let cname = if engine == EngineKind::Mysql
                && *kind == ConstraintKind::PrimaryKey
                && name.trim().is_empty()
            {
                "PRIMARY"
            } else {
                validate_identifier(name, "constraint name")?
            };
            let qualified_table = quote_qualified(engine, table.schema.trim(), table.name.trim());
            let quoted_cname = quote_engine_identifier(engine, cname);
            let mut warnings = vec![format!(
                "Dropping constraint '{cname}' permanently removes its integrity enforcement from {}.",
                table.name.trim()
            )];

            let sql = match engine {
                EngineKind::Postgres => {
                    let cascade_kw = if *cascade { " CASCADE" } else { "" };
                    if *cascade {
                        warnings.push(
                            "CASCADE will also drop dependent foreign keys, views, or triggers."
                                .into(),
                        );
                    }
                    format!(
                        "ALTER TABLE {qualified_table} DROP CONSTRAINT {quoted_cname}{cascade_kw};"
                    )
                }
                EngineKind::Mysql => {
                    if *cascade {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "MySQL/MariaDB DROP CONSTRAINT does not support CASCADE",
                        ));
                    }
                    warnings.push(
                        "ALTER TABLE on MySQL/MariaDB causes an implicit commit of any open transaction on this session."
                            .into(),
                    );
                    match kind {
                        ConstraintKind::PrimaryKey => {
                            format!("ALTER TABLE {qualified_table} DROP PRIMARY KEY;")
                        }
                        ConstraintKind::ForeignKey => {
                            format!(
                                "ALTER TABLE {qualified_table} DROP FOREIGN KEY {quoted_cname};"
                            )
                        }
                        ConstraintKind::Unique => {
                            format!("ALTER TABLE {qualified_table} DROP INDEX {quoted_cname};")
                        }
                        ConstraintKind::Check => {
                            format!("ALTER TABLE {qualified_table} DROP CHECK {quoted_cname};")
                        }
                        ConstraintKind::Exclusion => {
                            return Err(AppError::new(
                                ErrorCode::Unsupported,
                                "Exclusion constraints are not supported on MySQL/MariaDB",
                            ));
                        }
                    }
                }
                _ => unreachable!(),
            };

            Ok(IndexConstraintPreview {
                statement: DdlStatement {
                    sql,
                    description: format!("Drop constraint {cname} on {}", table.name.trim()),
                    object_kind: DatabaseObjectKind::Table,
                    object_name: format!("{}.{cname}", table.name.trim()),
                    destructive: true,
                },
                warnings,
            })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TableColumnDraft {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_expression: Option<String>,
    #[serde(default)]
    pub auto_increment: bool,
    #[serde(default)]
    pub primary_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TableObjectAction {
    #[serde(rename_all = "camelCase")]
    CreateTable {
        table: TableRef,
        columns: Vec<TableColumnDraft>,
        comment: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    AddColumn {
        table: TableRef,
        column: TableColumnDraft,
    },
    #[serde(rename_all = "camelCase")]
    AlterColumn {
        table: TableRef,
        column_name: String,
        new_name: Option<String>,
        data_type: String,
        nullable: bool,
        default_expression: Option<String>,
        using_expression: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    DropColumn {
        table: TableRef,
        column_name: String,
        #[serde(default)]
        cascade: bool,
    },
    #[serde(rename_all = "camelCase")]
    DropObject {
        object: DatabaseObjectRef,
        #[serde(default)]
        cascade: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TableObjectPreview {
    pub statements: Vec<DdlStatement>,
    pub script: String,
    pub warnings: Vec<String>,
}

const MAX_TABLE_COLUMNS: usize = 256;

fn is_integer_or_serial_type(dtype: &str) -> bool {
    let lower = dtype.trim().to_ascii_lowercase();
    let base = lower.split('(').next().unwrap_or(&lower).trim();
    matches!(
        base,
        "smallint"
            | "int2"
            | "integer"
            | "int"
            | "int4"
            | "bigint"
            | "int8"
            | "tinyint"
            | "mediumint"
            | "serial"
            | "smallserial"
            | "bigserial"
            | "smallint unsigned"
            | "int unsigned"
            | "integer unsigned"
            | "bigint unsigned"
            | "tinyint unsigned"
            | "mediumint unsigned"
    )
}

fn format_draft_column_spec(
    engine: EngineKind,
    col: &TableColumnDraft,
    inline_primary_key: bool,
) -> AppResult<String> {
    let col_name = validate_identifier(&col.name, "column name")?;
    let dtype = validate_sql_fragment(&col.data_type, "column data type")?;
    let default_expr = match col
        .default_expression
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(expr) => Some(validate_sql_fragment(expr, "column default expression")?),
        None => None,
    };

    if col.auto_increment {
        if !is_integer_or_serial_type(dtype) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                format!(
                    "Identity / AUTO_INCREMENT column '{col_name}' must use an integer data type"
                ),
            ));
        }
        if default_expr.is_some() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                format!(
                    "Identity / AUTO_INCREMENT column '{col_name}' cannot also specify a DEFAULT expression"
                ),
            ));
        }
    }

    let quoted_col = quote_engine_identifier(engine, col_name);
    let mut parts = vec![quoted_col, dtype.to_owned()];

    let is_pg_serial = engine == EngineKind::Postgres
        && matches!(
            dtype.to_ascii_lowercase().as_str(),
            "serial" | "smallserial" | "bigserial"
        );

    if col.auto_increment && engine == EngineKind::Postgres && !is_pg_serial {
        parts.push("GENERATED BY DEFAULT AS IDENTITY".into());
    }

    if let Some(def) = default_expr {
        parts.push(format!("DEFAULT {def}"));
    }

    if !col.nullable || col.primary_key || col.auto_increment {
        parts.push("NOT NULL".into());
    }

    if col.auto_increment && engine == EngineKind::Mysql {
        parts.push("AUTO_INCREMENT".into());
    }

    if inline_primary_key && col.primary_key {
        parts.push("PRIMARY KEY".into());
    }

    Ok(parts.join(" "))
}

/// Builds engine-aware DDL statements for creating tables, adding/modifying/dropping
/// columns, and dropping tables or views on PostgreSQL and MySQL/MariaDB.
/// Never executes SQL.
pub fn build_table_object_ddl(
    engine: EngineKind,
    action: &TableObjectAction,
) -> AppResult<TableObjectPreview> {
    if !matches!(engine, EngineKind::Postgres | EngineKind::Mysql) {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Table and object DDL editing is available for PostgreSQL and MySQL/MariaDB",
        ));
    }

    match action {
        TableObjectAction::CreateTable {
            table,
            columns,
            comment,
        } => {
            validate_table_ref(table, "Target")?;
            if columns.is_empty() || columns.len() > MAX_TABLE_COLUMNS {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    format!("Define between 1 and {MAX_TABLE_COLUMNS} columns for the new table"),
                ));
            }

            let mut seen_names = BTreeSet::new();
            let mut body_lines = Vec::with_capacity(columns.len() + 1);
            let mut pk_cols = Vec::new();
            let mut auto_inc_count = 0usize;

            for col in columns {
                let col_name = validate_identifier(&col.name, "column name")?;
                if !seen_names.insert(col_name.to_lowercase()) {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        format!("Duplicate column name '{col_name}'"),
                    ));
                }
                if col.auto_increment {
                    auto_inc_count += 1;
                    if engine == EngineKind::Mysql && !col.primary_key {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            format!(
                                "MySQL/MariaDB requires AUTO_INCREMENT column '{col_name}' to be part of the PRIMARY KEY"
                            ),
                        ));
                    }
                }
                if col.primary_key {
                    pk_cols.push(quote_engine_identifier(engine, col_name));
                }
                body_lines.push(format!(
                    "  {}",
                    format_draft_column_spec(engine, col, false)?
                ));
            }

            if engine == EngineKind::Mysql && auto_inc_count > 1 {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "MySQL/MariaDB allows at most one AUTO_INCREMENT column per table",
                ));
            }

            if !pk_cols.is_empty() {
                if engine == EngineKind::Postgres {
                    let pk_name = format!("{}_pkey", table.name.trim());
                    body_lines.push(format!(
                        "  CONSTRAINT {} PRIMARY KEY ({})",
                        quote_engine_identifier(engine, &pk_name),
                        pk_cols.join(", ")
                    ));
                } else {
                    body_lines.push(format!("  PRIMARY KEY ({})", pk_cols.join(", ")));
                }
            }

            let comment_clean = match comment.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(c) => {
                    if c.chars().count() > MAX_EXPRESSION_CHARS || c.contains('\0') {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            format!(
                                "Table comment must be at most {MAX_EXPRESSION_CHARS} characters without NUL bytes"
                            ),
                        ));
                    }
                    Some(c)
                }
                None => None,
            };

            let qualified_table = quote_qualified(engine, table.schema.trim(), table.name.trim());
            let mut statements = Vec::new();
            let mut warnings = Vec::new();

            if pk_cols.is_empty() {
                warnings.push(
                    "Creating a table without a PRIMARY KEY disables keyset pagination and row-level grid editing in Joinery until a primary key is added."
                        .into(),
                );
            }
            if engine == EngineKind::Mysql {
                warnings.push(
                    "CREATE TABLE on MySQL/MariaDB causes an implicit commit of any open transaction on this session."
                        .into(),
                );
            }

            let create_sql = if engine == EngineKind::Mysql {
                let comment_suffix = comment_clean
                    .map(|c| format!(" COMMENT = {}", quote_sql_literal(c)))
                    .unwrap_or_default();
                format!(
                    "CREATE TABLE {qualified_table} (\n{}\n){comment_suffix};",
                    body_lines.join(",\n")
                )
            } else {
                format!(
                    "CREATE TABLE {qualified_table} (\n{}\n);",
                    body_lines.join(",\n")
                )
            };

            statements.push(DdlStatement {
                sql: create_sql,
                description: format!("Create table {}", table.name.trim()),
                object_kind: DatabaseObjectKind::Table,
                object_name: table.name.trim().to_owned(),
                destructive: false,
            });

            if engine == EngineKind::Postgres
                && let Some(c) = comment_clean
            {
                statements.push(DdlStatement {
                    sql: format!(
                        "COMMENT ON TABLE {qualified_table} IS {};",
                        quote_sql_literal(c)
                    ),
                    description: format!("Set comment on table {}", table.name.trim()),
                    object_kind: DatabaseObjectKind::Table,
                    object_name: table.name.trim().to_owned(),
                    destructive: false,
                });
            }

            let script = statements
                .iter()
                .map(|s| s.sql.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            Ok(TableObjectPreview {
                statements,
                script,
                warnings,
            })
        }
        TableObjectAction::AddColumn { table, column } => {
            validate_table_ref(table, "Target")?;
            let col_name = validate_identifier(&column.name, "column name")?;
            if engine == EngineKind::Mysql && column.auto_increment && !column.primary_key {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    format!(
                        "MySQL/MariaDB requires AUTO_INCREMENT column '{col_name}' to be marked as Primary Key"
                    ),
                ));
            }
            let spec = format_draft_column_spec(engine, column, true)?;
            let qualified_table = quote_qualified(engine, table.schema.trim(), table.name.trim());
            let sql = format!("ALTER TABLE {qualified_table} ADD COLUMN {spec};");

            let mut warnings = Vec::new();
            if !column.nullable
                && !column.auto_increment
                && column
                    .default_expression
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
            {
                warnings.push(
                    "Adding a NOT NULL column without a DEFAULT expression will fail if the table already contains rows."
                        .into(),
                );
            }
            if engine == EngineKind::Mysql {
                warnings.push(
                    "ALTER TABLE on MySQL/MariaDB causes an implicit commit of any open transaction on this session."
                        .into(),
                );
            } else {
                warnings.push(
                    "ALTER TABLE ADD COLUMN acquires an ACCESS EXCLUSIVE lock on the table.".into(),
                );
            }

            let stmt = DdlStatement {
                sql: sql.clone(),
                description: format!("Add column {col_name} to {}", table.name.trim()),
                object_kind: DatabaseObjectKind::Table,
                object_name: format!("{}.{col_name}", table.name.trim()),
                destructive: false,
            };
            Ok(TableObjectPreview {
                statements: vec![stmt],
                script: sql,
                warnings,
            })
        }
        TableObjectAction::AlterColumn {
            table,
            column_name,
            new_name,
            data_type,
            nullable,
            default_expression,
            using_expression,
        } => {
            validate_table_ref(table, "Target")?;
            let old_col = validate_identifier(column_name, "column name")?;
            let renamed_to = match new_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(n) if n != old_col => Some(validate_identifier(n, "new column name")?),
                _ => None,
            };
            let dtype = validate_sql_fragment(data_type, "column data type")?;
            let default_expr = match default_expression
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(d) => Some(validate_sql_fragment(d, "column default expression")?),
                None => None,
            };
            let using_expr = match using_expression
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(u) => Some(validate_sql_fragment(u, "USING expression")?),
                None => None,
            };

            let qualified_table = quote_qualified(engine, table.schema.trim(), table.name.trim());
            let quoted_old = quote_engine_identifier(engine, old_col);
            let mut statements = Vec::new();
            let mut warnings = Vec::new();

            match engine {
                EngineKind::Postgres => {
                    if using_expr.is_some() {
                        statements.push(DdlStatement {
                            sql: format!(
                                "ALTER TABLE {qualified_table} ALTER COLUMN {quoted_old} DROP DEFAULT;"
                            ),
                            description: format!(
                                "Drop old default on {old_col} before USING type conversion"
                            ),
                            object_kind: DatabaseObjectKind::Table,
                            object_name: format!("{}.{old_col}", table.name.trim()),
                            destructive: false,
                        });
                    }
                    let type_clause = if let Some(u) = using_expr {
                        format!("ALTER COLUMN {quoted_old} TYPE {dtype} USING {u}")
                    } else {
                        format!("ALTER COLUMN {quoted_old} TYPE {dtype}")
                    };
                    let null_clause = if *nullable {
                        format!("ALTER COLUMN {quoted_old} DROP NOT NULL")
                    } else {
                        format!("ALTER COLUMN {quoted_old} SET NOT NULL")
                    };
                    let def_clause = if let Some(def) = default_expr {
                        format!(", ALTER COLUMN {quoted_old} SET DEFAULT {def}")
                    } else if using_expr.is_none() {
                        format!(", ALTER COLUMN {quoted_old} DROP DEFAULT")
                    } else {
                        String::new()
                    };
                    statements.push(DdlStatement {
                        sql: format!(
                            "ALTER TABLE {qualified_table} {type_clause}, {null_clause}{def_clause};"
                        ),
                        description: format!("Modify column {old_col} on {}", table.name.trim()),
                        object_kind: DatabaseObjectKind::Table,
                        object_name: format!("{}.{old_col}", table.name.trim()),
                        destructive: false,
                    });
                    if let Some(target_col) = renamed_to {
                        let quoted_new = quote_engine_identifier(engine, target_col);
                        statements.push(DdlStatement {
                            sql: format!(
                                "ALTER TABLE {qualified_table} RENAME COLUMN {quoted_old} TO {quoted_new};"
                            ),
                            description: format!(
                                "Rename column {old_col} to {target_col} on {}",
                                table.name.trim()
                            ),
                            object_kind: DatabaseObjectKind::Table,
                            object_name: format!("{}.{target_col}", table.name.trim()),
                            destructive: false,
                        });
                    }
                    warnings.push(
                        "ALTER COLUMN … TYPE on PostgreSQL acquires an ACCESS EXCLUSIVE lock and may rewrite the table if the storage type changes."
                            .into(),
                    );
                }
                EngineKind::Mysql => {
                    if using_expr.is_some() {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "PostgreSQL USING cast expressions are not supported on MySQL/MariaDB",
                        ));
                    }
                    let null_kw = if *nullable { "NULL" } else { "NOT NULL" };
                    let def_suffix = default_expr
                        .map(|d| format!(" DEFAULT {d}"))
                        .unwrap_or_default();
                    let sql = if let Some(target_col) = renamed_to {
                        let quoted_new = quote_engine_identifier(engine, target_col);
                        format!(
                            "ALTER TABLE {qualified_table} CHANGE COLUMN {quoted_old} {quoted_new} {dtype} {null_kw}{def_suffix};"
                        )
                    } else {
                        format!(
                            "ALTER TABLE {qualified_table} MODIFY COLUMN {quoted_old} {dtype} {null_kw}{def_suffix};"
                        )
                    };
                    statements.push(DdlStatement {
                        sql,
                        description: format!("Modify column {old_col} on {}", table.name.trim()),
                        object_kind: DatabaseObjectKind::Table,
                        object_name: format!("{}.{old_col}", table.name.trim()),
                        destructive: false,
                    });
                    warnings.push(
                        "ALTER TABLE on MySQL/MariaDB causes an implicit commit of any open transaction on this session and may rebuild the table."
                            .into(),
                    );
                }
                _ => unreachable!(),
            }

            let script = statements
                .iter()
                .map(|s| s.sql.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            Ok(TableObjectPreview {
                statements,
                script,
                warnings,
            })
        }
        TableObjectAction::DropColumn {
            table,
            column_name,
            cascade,
        } => {
            validate_table_ref(table, "Target")?;
            let col_name = validate_identifier(column_name, "column name")?;
            if engine == EngineKind::Mysql && *cascade {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "MySQL/MariaDB DROP COLUMN does not support CASCADE",
                ));
            }
            let qualified_table = quote_qualified(engine, table.schema.trim(), table.name.trim());
            let quoted_col = quote_engine_identifier(engine, col_name);
            let cascade_kw = if *cascade { " CASCADE" } else { "" };
            let sql =
                format!("ALTER TABLE {qualified_table} DROP COLUMN {quoted_col}{cascade_kw};");
            let mut warnings = vec![format!(
                "Dropping column '{col_name}' permanently deletes all values in that column from {}.",
                table.name.trim()
            )];
            if *cascade {
                warnings.push(
                    "CASCADE will also drop dependent views, indexes, triggers, or constraints."
                        .into(),
                );
            }
            if engine == EngineKind::Mysql {
                warnings.push(
                    "ALTER TABLE on MySQL/MariaDB causes an implicit commit of any open transaction on this session."
                        .into(),
                );
            }
            let stmt = DdlStatement {
                sql: sql.clone(),
                description: format!("Drop column {col_name} from {}", table.name.trim()),
                object_kind: DatabaseObjectKind::Table,
                object_name: format!("{}.{col_name}", table.name.trim()),
                destructive: true,
            };
            Ok(TableObjectPreview {
                statements: vec![stmt],
                script: sql,
                warnings,
            })
        }
        TableObjectAction::DropObject { object, cascade } => {
            let schema_name = validate_identifier(&object.schema, "object schema")?;
            let obj_name = validate_identifier(&object.name, "object name")?;
            if engine == EngineKind::Mysql && *cascade {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "MySQL/MariaDB DROP TABLE/VIEW does not support CASCADE",
                ));
            }
            let kw = match object.kind {
                DatabaseObjectKind::Table | DatabaseObjectKind::PartitionedTable => "TABLE",
                DatabaseObjectKind::View => "VIEW",
                DatabaseObjectKind::MaterializedView => {
                    if engine != EngineKind::Postgres {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "Materialized views are only supported on PostgreSQL",
                        ));
                    }
                    "MATERIALIZED VIEW"
                }
                other => {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        format!("Drop object editor supports tables and views (got {other:?})"),
                    ));
                }
            };
            let qualified = quote_qualified(engine, schema_name, obj_name);
            let cascade_kw = if *cascade { " CASCADE" } else { "" };
            let sql = format!("DROP {kw} {qualified}{cascade_kw};");
            let mut warnings = vec![if kw == "TABLE" {
                format!(
                    "DROP TABLE permanently deletes '{obj_name}' and all of its rows, indexes, and triggers."
                )
            } else {
                format!("DROP {kw} permanently removes '{obj_name}'.")
            }];
            if *cascade {
                warnings.push(
                    "CASCADE will also drop dependent views, foreign keys, or routines.".into(),
                );
            }
            if engine == EngineKind::Mysql {
                warnings.push(
                    format!(
                        "DROP {kw} on MySQL/MariaDB causes an implicit commit of any open transaction on this session."
                    ),
                );
            }
            let stmt = DdlStatement {
                sql: sql.clone(),
                description: format!("Drop {} {obj_name}", kw.to_ascii_lowercase()),
                object_kind: object.kind,
                object_name: obj_name.to_owned(),
                destructive: true,
            };
            Ok(TableObjectPreview {
                statements: vec![stmt],
                script: sql,
                warnings,
            })
        }
    }
}

/// Engine-specific table maintenance action for `A06` reviewed administration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[serde(rename_all_fields = "camelCase")]
pub enum TableMaintenanceAction {
    /// PostgreSQL `ANALYZE [(VERBOSE)] "schema"."table" [("col1", ...)]`.
    Analyze {
        table: TableRef,
        verbose: bool,
        #[serde(default)]
        columns: Vec<String>,
    },
    /// PostgreSQL `VACUUM [(FULL, ANALYZE, VERBOSE)] "schema"."table"`.
    Vacuum {
        table: TableRef,
        analyze: bool,
        verbose: bool,
        full: bool,
    },
    /// PostgreSQL `REINDEX TABLE "schema"."table"`.
    ReindexTable { table: TableRef },
    /// MySQL / MariaDB `ANALYZE TABLE \`schema\`.\`table\``.
    AnalyzeTable { table: TableRef },
    /// MySQL / MariaDB `OPTIMIZE TABLE \`schema\`.\`table\``.
    OptimizeTable { table: TableRef },
    /// MySQL / MariaDB `CHECK TABLE \`schema\`.\`table\``.
    CheckTable { table: TableRef },
}

/// Preview of a single reviewed table maintenance statement and its operational warnings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TableMaintenancePreview {
    pub statement: DdlStatement,
    pub warnings: Vec<String>,
    /// True when the statement cannot execute inside an open transaction block (e.g. PostgreSQL `VACUUM`).
    pub non_transactional: bool,
}

/// Builds a reviewed, engine-specific table maintenance statement (`VACUUM`, `ANALYZE`,
/// `REINDEX TABLE`, `ANALYZE TABLE`, `OPTIMIZE TABLE`, `CHECK TABLE`) with explicit lock
/// and transaction warnings.
pub fn build_table_maintenance_sql(
    engine: EngineKind,
    action: &TableMaintenanceAction,
) -> AppResult<TableMaintenancePreview> {
    if !matches!(engine, EngineKind::Postgres | EngineKind::Mysql) {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Table maintenance commands are only supported for PostgreSQL and MySQL/MariaDB",
        ));
    }

    match (engine, action) {
        (
            EngineKind::Postgres,
            TableMaintenanceAction::Analyze {
                table,
                verbose,
                columns,
            },
        ) => {
            validate_table_ref(table, "Target")?;
            if columns.len() > MAX_TABLE_COLUMNS {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    format!("ANALYZE column list cannot exceed {MAX_TABLE_COLUMNS} columns"),
                ));
            }
            let mut seen = BTreeSet::new();
            let mut quoted_cols = Vec::with_capacity(columns.len());
            for col in columns {
                let valid_col = validate_identifier(col, "Column name")?;
                if !seen.insert(valid_col) {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        format!("Duplicate column \"{valid_col}\" in ANALYZE column list"),
                    ));
                }
                quoted_cols.push(quote_engine_identifier(EngineKind::Postgres, valid_col));
            }
            let opts = if *verbose { " (VERBOSE)" } else { "" };
            let cols_clause = if quoted_cols.is_empty() {
                String::new()
            } else {
                format!(" ({})", quoted_cols.join(", "))
            };
            let qualified =
                quote_qualified(EngineKind::Postgres, table.schema.trim(), table.name.trim());
            let sql = format!("ANALYZE{opts} {qualified}{cols_clause};");
            let warnings = vec![
                "ANALYZE samples table pages and updates pg_statistic planner estimates while acquiring only a SHARE UPDATE EXCLUSIVE lock (concurrent reads and writes remain allowed)."
                    .to_string(),
            ];
            Ok(TableMaintenancePreview {
                statement: DdlStatement {
                    sql,
                    description: format!(
                        "Analyze table {}.{}",
                        table.schema.trim(),
                        table.name.trim()
                    ),
                    object_kind: DatabaseObjectKind::Table,
                    object_name: table.name.trim().to_owned(),
                    destructive: false,
                },
                warnings,
                non_transactional: false,
            })
        }
        (
            EngineKind::Postgres,
            TableMaintenanceAction::Vacuum {
                table,
                analyze,
                verbose,
                full,
            },
        ) => {
            validate_table_ref(table, "Target")?;
            let mut options = Vec::new();
            if *full {
                options.push("FULL");
            }
            if *analyze {
                options.push("ANALYZE");
            }
            if *verbose {
                options.push("VERBOSE");
            }
            let opts_clause = if options.is_empty() {
                String::new()
            } else {
                format!(" ({})", options.join(", "))
            };
            let qualified =
                quote_qualified(EngineKind::Postgres, table.schema.trim(), table.name.trim());
            let sql = format!("VACUUM{opts_clause} {qualified};");
            let mut warnings = vec![
                "VACUUM cannot run inside a transaction block. Commit or roll back any open manual transaction before running VACUUM."
                    .to_string(),
            ];
            if *full {
                warnings.push(
                    "VACUUM FULL rewrites the entire table onto new disk pages and holds an ACCESS EXCLUSIVE lock that blocks all concurrent reads and writes until completion."
                        .to_string(),
                );
            } else {
                warnings.push(
                    "Standard VACUUM reclaims dead tuple space for reuse without blocking concurrent reads and writes (SHARE UPDATE EXCLUSIVE lock)."
                        .to_string(),
                );
            }
            Ok(TableMaintenancePreview {
                statement: DdlStatement {
                    sql,
                    description: if *full {
                        format!(
                            "Vacuum full table {}.{}",
                            table.schema.trim(),
                            table.name.trim()
                        )
                    } else if *analyze {
                        format!(
                            "Vacuum and analyze table {}.{}",
                            table.schema.trim(),
                            table.name.trim()
                        )
                    } else {
                        format!("Vacuum table {}.{}", table.schema.trim(), table.name.trim())
                    },
                    object_kind: DatabaseObjectKind::Table,
                    object_name: table.name.trim().to_owned(),
                    destructive: *full,
                },
                warnings,
                non_transactional: true,
            })
        }
        (EngineKind::Postgres, TableMaintenanceAction::ReindexTable { table }) => {
            validate_table_ref(table, "Target")?;
            let qualified =
                quote_qualified(EngineKind::Postgres, table.schema.trim(), table.name.trim());
            let sql = format!("REINDEX TABLE {qualified};");
            let warnings = vec![
                "REINDEX TABLE rebuilds all indexes on the table and acquires a SHARE lock that blocks concurrent writes until index rebuilds complete."
                    .to_string(),
            ];
            Ok(TableMaintenancePreview {
                statement: DdlStatement {
                    sql,
                    description: format!(
                        "Reindex table {}.{}",
                        table.schema.trim(),
                        table.name.trim()
                    ),
                    object_kind: DatabaseObjectKind::Table,
                    object_name: table.name.trim().to_owned(),
                    destructive: false,
                },
                warnings,
                non_transactional: false,
            })
        }
        (EngineKind::Mysql, TableMaintenanceAction::AnalyzeTable { table }) => {
            validate_table_ref(table, "Target")?;
            let qualified =
                quote_qualified(EngineKind::Mysql, table.schema.trim(), table.name.trim());
            let sql = format!("ANALYZE TABLE {qualified};");
            let warnings = vec![
                "ANALYZE TABLE recalculates index key distribution statistics and causes an implicit commit of any open transaction on this session."
                    .to_string(),
            ];
            Ok(TableMaintenancePreview {
                statement: DdlStatement {
                    sql,
                    description: format!(
                        "Analyze table {}.{}",
                        table.schema.trim(),
                        table.name.trim()
                    ),
                    object_kind: DatabaseObjectKind::Table,
                    object_name: table.name.trim().to_owned(),
                    destructive: false,
                },
                warnings,
                non_transactional: false,
            })
        }
        (EngineKind::Mysql, TableMaintenanceAction::OptimizeTable { table }) => {
            validate_table_ref(table, "Target")?;
            let qualified =
                quote_qualified(EngineKind::Mysql, table.schema.trim(), table.name.trim());
            let sql = format!("OPTIMIZE TABLE {qualified};");
            let warnings = vec![
                "OPTIMIZE TABLE defragments table data, reclaims unused space, and rebuilds indexes. It causes an implicit commit of any open transaction on this session."
                    .to_string(),
            ];
            Ok(TableMaintenancePreview {
                statement: DdlStatement {
                    sql,
                    description: format!(
                        "Optimize table {}.{}",
                        table.schema.trim(),
                        table.name.trim()
                    ),
                    object_kind: DatabaseObjectKind::Table,
                    object_name: table.name.trim().to_owned(),
                    destructive: false,
                },
                warnings,
                non_transactional: false,
            })
        }
        (EngineKind::Mysql, TableMaintenanceAction::CheckTable { table }) => {
            validate_table_ref(table, "Target")?;
            let qualified =
                quote_qualified(EngineKind::Mysql, table.schema.trim(), table.name.trim());
            let sql = format!("CHECK TABLE {qualified};");
            let warnings = vec![
                "CHECK TABLE scans table pages and indexes for structural corruption and causes an implicit commit of any open transaction on this session."
                    .to_string(),
            ];
            Ok(TableMaintenancePreview {
                statement: DdlStatement {
                    sql,
                    description: format!(
                        "Check table {}.{}",
                        table.schema.trim(),
                        table.name.trim()
                    ),
                    object_kind: DatabaseObjectKind::Table,
                    object_name: table.name.trim().to_owned(),
                    destructive: false,
                },
                warnings,
                non_transactional: false,
            })
        }
        (
            EngineKind::Postgres,
            TableMaintenanceAction::AnalyzeTable { .. }
            | TableMaintenanceAction::OptimizeTable { .. }
            | TableMaintenanceAction::CheckTable { .. },
        ) => Err(AppError::new(
            ErrorCode::Unsupported,
            "This table maintenance command is only supported on MySQL and MariaDB",
        )),
        (
            EngineKind::Mysql,
            TableMaintenanceAction::Analyze { .. }
            | TableMaintenanceAction::Vacuum { .. }
            | TableMaintenanceAction::ReindexTable { .. },
        ) => Err(AppError::new(
            ErrorCode::Unsupported,
            "This table maintenance command is only supported on PostgreSQL",
        )),
        _ => Err(AppError::new(
            ErrorCode::Unsupported,
            "Table maintenance commands are only supported for PostgreSQL and MySQL/MariaDB",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::TableRef;

    fn sample_postgres_snapshot(schema: &str) -> SchemaSnapshot {
        SchemaSnapshot {
            engine: EngineKind::Postgres,
            schema: schema.into(),
            server_version: "PostgreSQL 18.0".into(),
            database: "app".into(),
            role: "postgres".into(),
            truncated: false,
            restricted: false,
            tables: vec![
                SnapshotTable {
                    name: "customers".into(),
                    kind: DatabaseObjectKind::Table,
                    comment: Some("Customer master".into()),
                    partition_key: None,
                    view_definition: None,
                    columns: vec![
                        SnapshotColumn {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                            default_expression: None,
                            identity: Some("BY DEFAULT".into()),
                            generated: None,
                            ordinal_position: 1,
                        },
                        SnapshotColumn {
                            name: "email".into(),
                            data_type: "text".into(),
                            nullable: false,
                            default_expression: None,
                            identity: None,
                            generated: None,
                            ordinal_position: 2,
                        },
                    ],
                    constraints: vec![
                        ConstraintInfo {
                            name: "customers_pkey".into(),
                            kind: ConstraintKind::PrimaryKey,
                            columns: vec!["id".into()],
                            target: None,
                            target_columns: Vec::new(),
                            definition: "PRIMARY KEY (id)".into(),
                        },
                        ConstraintInfo {
                            name: "customers_email_key".into(),
                            kind: ConstraintKind::Unique,
                            columns: vec!["email".into()],
                            target: None,
                            target_columns: Vec::new(),
                            definition: "UNIQUE (email)".into(),
                        },
                    ],
                    indexes: vec![
                        SnapshotIndex {
                            name: "customers_pkey".into(),
                            method: "btree".into(),
                            unique: true,
                            primary: true,
                            expressions: vec!["id".into()],
                            predicate: None,
                            definition: Some(format!(
                                "CREATE UNIQUE INDEX customers_pkey ON {schema}.customers USING btree (id)"
                            )),
                        },
                        SnapshotIndex {
                            name: "customers_email_key".into(),
                            method: "btree".into(),
                            unique: true,
                            primary: false,
                            expressions: vec!["email".into()],
                            predicate: None,
                            definition: Some(format!(
                                "CREATE UNIQUE INDEX customers_email_key ON {schema}.customers USING btree (email)"
                            )),
                        },
                    ],
                    triggers: Vec::new(),
                },
                SnapshotTable {
                    name: "orders".into(),
                    kind: DatabaseObjectKind::Table,
                    comment: None,
                    partition_key: None,
                    view_definition: None,
                    columns: vec![
                        SnapshotColumn {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                            default_expression: Some(format!(
                                "nextval('{schema}.orders_id_seq'::regclass)"
                            )),
                            identity: None,
                            generated: None,
                            ordinal_position: 1,
                        },
                        SnapshotColumn {
                            name: "customer_id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                            default_expression: None,
                            identity: None,
                            generated: None,
                            ordinal_position: 2,
                        },
                        SnapshotColumn {
                            name: "total".into(),
                            data_type: "numeric(10,2)".into(),
                            nullable: false,
                            default_expression: Some("0.00".into()),
                            identity: None,
                            generated: None,
                            ordinal_position: 3,
                        },
                    ],
                    constraints: vec![
                        ConstraintInfo {
                            name: "orders_pkey".into(),
                            kind: ConstraintKind::PrimaryKey,
                            columns: vec!["id".into()],
                            target: None,
                            target_columns: Vec::new(),
                            definition: "PRIMARY KEY (id)".into(),
                        },
                        ConstraintInfo {
                            name: "orders_customer_fk".into(),
                            kind: ConstraintKind::ForeignKey,
                            columns: vec!["customer_id".into()],
                            target: Some(TableRef {
                                schema: schema.into(),
                                name: "customers".into(),
                            }),
                            target_columns: vec!["id".into()],
                            definition: format!(
                                "FOREIGN KEY (customer_id) REFERENCES {schema}.customers(id) ON DELETE CASCADE"
                            ),
                        },
                    ],
                    indexes: vec![SnapshotIndex {
                        name: "orders_customer_idx".into(),
                        method: "btree".into(),
                        unique: false,
                        primary: false,
                        expressions: vec!["customer_id".into()],
                        predicate: None,
                        definition: Some(format!(
                            "CREATE INDEX orders_customer_idx ON {schema}.orders USING btree (customer_id)"
                        )),
                    }],
                    triggers: vec![TriggerInfo {
                        name: "trg_orders_audit".into(),
                        table_name: "orders".into(),
                        timing: "BEFORE".into(),
                        events: vec!["INSERT".into()],
                        orientation: "ROW".into(),
                        enabled: true,
                        definition: Some(format!(
                            "CREATE TRIGGER trg_orders_audit BEFORE INSERT ON {schema}.orders FOR EACH ROW EXECUTE FUNCTION {schema}.touch_order()"
                        )),
                    }],
                },
                SnapshotTable {
                    name: "v_active_orders".into(),
                    kind: DatabaseObjectKind::View,
                    comment: None,
                    partition_key: None,
                    view_definition: Some(format!(
                        "SELECT o.id, o.total FROM {schema}.orders o WHERE o.total > 0"
                    )),
                    columns: Vec::new(),
                    constraints: Vec::new(),
                    indexes: Vec::new(),
                    triggers: Vec::new(),
                },
            ],
            routines: vec![SnapshotRoutine {
                name: "touch_order".into(),
                kind: DatabaseObjectKind::Function,
                identity_arguments: String::new(),
                arguments: String::new(),
                return_type: Some("trigger".into()),
                language: "plpgsql".into(),
                volatility: Some("VOLATILE".into()),
                security_definer: false,
                definition: Some(format!(
                    "CREATE OR REPLACE FUNCTION {schema}.touch_order()\n RETURNS trigger\n LANGUAGE plpgsql\nAS $function$\nBEGIN\n  RETURN NEW;\nEND;\n$function$"
                )),
                comment: None,
            }],
            sequences: vec![
                SnapshotSequence {
                    name: "customers_id_seq".into(),
                    data_type: "bigint".into(),
                    start_value: "1".into(),
                    min_value: "1".into(),
                    max_value: "9223372036854775807".into(),
                    increment_by: "1".into(),
                    cache_size: "1".into(),
                    cycle: false,
                },
                SnapshotSequence {
                    name: "orders_id_seq".into(),
                    data_type: "bigint".into(),
                    start_value: "1".into(),
                    min_value: "1".into(),
                    max_value: "9223372036854775807".into(),
                    increment_by: "1".into(),
                    cache_size: "1".into(),
                    cycle: false,
                },
            ],
            unsupported_objects: Vec::new(),
        }
    }

    #[test]
    fn postgres_full_schema_ddl_orders_dependencies_and_retargets_schema() {
        let snap = sample_postgres_snapshot("r7_src");
        let plan = generate_schema_ddl(&snap, Some("r7_dst")).expect("ddl plan");

        assert_eq!(plan.engine, EngineKind::Postgres);
        assert_eq!(plan.target_schema, "r7_dst");
        assert_eq!(plan.destructive_count, 0);

        // Auto-identity sequence `customers_id_seq` is skipped, standalone `orders_id_seq` is emitted first.
        assert_eq!(plan.statements[0].object_kind, DatabaseObjectKind::Sequence);
        assert_eq!(plan.statements[0].object_name, "orders_id_seq");
        assert!(
            plan.statements[0]
                .sql
                .contains("CREATE SEQUENCE \"r7_dst\".\"orders_id_seq\"")
        );

        // Constraint-backed indexes (`customers_pkey`, `customers_email_key`) are not emitted as separate CREATE INDEX.
        assert!(
            !plan
                .statements
                .iter()
                .any(|s| s.object_name.ends_with("customers_email_key"))
        );

        // Secondary index `orders_customer_idx` and outbound FK `orders_customer_fk` are emitted and retargeted to `"r7_dst"`.
        assert!(
            plan.script
                .contains("nextval('r7_dst.orders_id_seq'::regclass)")
        );
        assert!(plan.script.contains(
            "ALTER TABLE \"r7_dst\".\"orders\" ADD CONSTRAINT \"orders_customer_fk\" FOREIGN KEY (customer_id) REFERENCES \"r7_dst\".customers(id) ON DELETE CASCADE;"
        ));
        assert!(
            plan.script
                .contains("CREATE OR REPLACE VIEW \"r7_dst\".\"v_active_orders\" AS SELECT o.id, o.total FROM \"r7_dst\".orders o WHERE o.total > 0;")
        );
        assert!(
            plan.script
                .contains("CREATE OR REPLACE FUNCTION \"r7_dst\".touch_order()")
        );
        assert!(plan.script.contains(
            "CREATE TRIGGER trg_orders_audit BEFORE INSERT ON \"r7_dst\".orders FOR EACH ROW EXECUTE FUNCTION \"r7_dst\".touch_order();"
        ));
    }

    #[test]
    fn migration_ddl_generates_dependency_ordered_alter_and_drop_statements() {
        let left = sample_postgres_snapshot("r7_left");
        let mut right = sample_postgres_snapshot("r7_right");

        // Modify customers: add a column `status`, drop unique constraint `customers_email_key`
        right.tables[0].columns.push(SnapshotColumn {
            name: "status".into(),
            data_type: "text".into(),
            nullable: false,
            default_expression: Some("'active'::text".into()),
            identity: None,
            generated: None,
            ordinal_position: 3,
        });
        right.tables[0]
            .constraints
            .retain(|c| c.name != "customers_email_key");
        right.tables[0]
            .indexes
            .retain(|i| i.name != "customers_email_key");

        // Drop trigger on orders
        right.tables[1].triggers.clear();

        let plan = generate_migration_ddl(&left, &right, None).expect("migration plan");
        assert_eq!(plan.target_schema, "r7_left");
        assert!(plan.destructive_count >= 2);
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("destructive statement"))
        );
        assert!(
            plan.script
                .contains("DROP TRIGGER \"trg_orders_audit\" ON \"r7_left\".\"orders\";")
        );
        assert!(plan.script.contains(
            "ALTER TABLE \"r7_left\".\"customers\" DROP CONSTRAINT \"customers_email_key\";"
        ));
        assert!(plan.script.contains(
            "ALTER TABLE \"r7_left\".\"customers\" ADD COLUMN \"status\" text DEFAULT 'active'::text NOT NULL;"
        ));
    }

    #[test]
    fn mysql_full_and_migration_ddl_quotes_backticks_and_formats_defaults() {
        let mut left = sample_postgres_snapshot("r7_mysql_left");
        left.engine = EngineKind::Mysql;
        left.tables[0].columns[0].identity = Some("AUTO_INCREMENT".into());
        left.tables[0].columns[1].data_type = "varchar(120)".into();
        left.tables[0].columns[1].default_expression = Some("guest@example.com".into());
        left.tables[1].columns[0].default_expression = None;
        left.tables[1].columns[0].identity = Some("AUTO_INCREMENT".into());
        left.tables[1].triggers[0].definition = Some("SET NEW.total = ABS(NEW.total)".into());
        left.routines[0].kind = DatabaseObjectKind::Function;
        left.routines[0].arguments = "amount decimal(10,2)".into();
        left.routines[0].identity_arguments = "decimal(10,2)".into();
        left.routines[0].return_type = Some("decimal(10,2)".into());
        left.routines[0].volatility = Some("DETERMINISTIC".into());
        left.routines[0].security_definer = true;
        left.routines[0].definition = Some("RETURN ROUND(amount * 0.1, 2)".into());
        left.sequences.clear();

        let full = generate_schema_ddl(&left, Some("r7_mysql_copy")).expect("mysql full ddl");
        assert!(
            full.script
                .contains("CREATE TABLE `r7_mysql_copy`.`customers`")
        );
        assert!(full.script.contains("`id` bigint NOT NULL AUTO_INCREMENT"));
        assert!(
            full.script
                .contains("`email` varchar(120) NOT NULL DEFAULT 'guest@example.com'")
        );
        assert!(full
            .script
            .contains("CREATE FUNCTION `r7_mysql_copy`.`touch_order`(amount decimal(10,2)) RETURNS decimal(10,2) DETERMINISTIC RETURN ROUND(amount * 0.1, 2);"));
        assert!(full.script.contains(
            "CREATE TRIGGER `r7_mysql_copy`.`trg_orders_audit` BEFORE INSERT ON `r7_mysql_copy`.`orders` FOR EACH ROW SET NEW.total = ABS(NEW.total);"
        ));

        let mut right = left.clone();
        right.schema = "r7_mysql_right".into();
        right.tables[0].columns[1].data_type = "varchar(200)".into();
        let mig = generate_migration_ddl(&left, &right, None).expect("mysql migration ddl");
        assert!(mig.script.contains(
            "ALTER TABLE `r7_mysql_left`.`customers` MODIFY COLUMN `email` varchar(200) NOT NULL DEFAULT 'guest@example.com';"
        ));
    }

    #[test]
    fn postgres_index_and_constraint_builder_handles_composite_partial_and_not_valid_ddl() {
        let table = TableRef {
            schema: "public".into(),
            name: "Order Items".into(),
        };

        let create_idx = build_index_constraint_ddl(
            EngineKind::Postgres,
            &IndexConstraintAction::CreateIndex {
                table: table.clone(),
                name: "order_items_customer_date_idx".into(),
                unique: true,
                method: "btree".into(),
                concurrently: true,
                expressions: vec!["customer_id".into(), "Order Date".into()],
                raw_expressions: false,
                predicate: Some("status = 'open'".into()),
            },
        )
        .expect("pg create index");
        assert_eq!(
            create_idx.statement.sql,
            "CREATE UNIQUE INDEX CONCURRENTLY \"order_items_customer_date_idx\" ON \"public\".\"Order Items\" USING btree (\"customer_id\", \"Order Date\") WHERE status = 'open';"
        );
        assert!(!create_idx.statement.destructive);
        assert!(
            create_idx
                .warnings
                .iter()
                .any(|w| w.contains("cannot run inside a transaction block"))
        );

        let drop_idx = build_index_constraint_ddl(
            EngineKind::Postgres,
            &IndexConstraintAction::DropIndex {
                table: table.clone(),
                name: "order_items_customer_date_idx".into(),
                concurrently: true,
            },
        )
        .expect("pg drop index");
        assert_eq!(
            drop_idx.statement.sql,
            "DROP INDEX CONCURRENTLY \"public\".\"order_items_customer_date_idx\";"
        );
        assert!(drop_idx.statement.destructive);

        let add_fk = build_index_constraint_ddl(
            EngineKind::Postgres,
            &IndexConstraintAction::AddConstraint {
                table: table.clone(),
                name: "order_items_customer_fkey".into(),
                kind: ConstraintKind::ForeignKey,
                columns: vec!["tenant_id".into(), "customer_id".into()],
                check_expression: None,
                target: Some(TableRef {
                    schema: "public".into(),
                    name: "customers".into(),
                }),
                target_columns: vec!["tenant_id".into(), "id".into()],
                on_delete: Some("CASCADE".into()),
                on_update: Some("NO ACTION".into()),
                not_valid: true,
            },
        )
        .expect("pg add fk");
        assert_eq!(
            add_fk.statement.sql,
            "ALTER TABLE \"public\".\"Order Items\" ADD CONSTRAINT \"order_items_customer_fkey\" FOREIGN KEY (\"tenant_id\", \"customer_id\") REFERENCES \"public\".\"customers\" (\"tenant_id\", \"id\") ON DELETE CASCADE ON UPDATE NO ACTION NOT VALID;"
        );
        assert!(
            add_fk
                .warnings
                .iter()
                .any(|w| w.contains("NOT VALID skips scanning existing rows"))
        );

        let add_check = build_index_constraint_ddl(
            EngineKind::Postgres,
            &IndexConstraintAction::AddConstraint {
                table: table.clone(),
                name: "order_items_qty_check".into(),
                kind: ConstraintKind::Check,
                columns: vec![],
                check_expression: Some("\"Unit Price\" >= 0".into()),
                target: None,
                target_columns: vec![],
                on_delete: None,
                on_update: None,
                not_valid: false,
            },
        )
        .expect("pg add check");
        assert_eq!(
            add_check.statement.sql,
            "ALTER TABLE \"public\".\"Order Items\" ADD CONSTRAINT \"order_items_qty_check\" CHECK (\"Unit Price\" >= 0);"
        );

        let drop_con = build_index_constraint_ddl(
            EngineKind::Postgres,
            &IndexConstraintAction::DropConstraint {
                table,
                name: "order_items_qty_check".into(),
                kind: ConstraintKind::Check,
                cascade: true,
            },
        )
        .expect("pg drop constraint");
        assert_eq!(
            drop_con.statement.sql,
            "ALTER TABLE \"public\".\"Order Items\" DROP CONSTRAINT \"order_items_qty_check\" CASCADE;"
        );
        assert!(drop_con.statement.destructive);
    }

    #[test]
    fn mysql_index_and_constraint_builder_uses_engine_specific_drop_and_rejects_unsupported_flags()
    {
        let table = TableRef {
            schema: "shop".into(),
            name: "Order `Items`".into(),
        };

        let create_ft = build_index_constraint_ddl(
            EngineKind::Mysql,
            &IndexConstraintAction::CreateIndex {
                table: table.clone(),
                name: "idx_notes_ft".into(),
                unique: false,
                method: "fulltext".into(),
                concurrently: false,
                expressions: vec!["notes".into()],
                raw_expressions: false,
                predicate: None,
            },
        )
        .expect("mysql fulltext index");
        assert_eq!(
            create_ft.statement.sql,
            "CREATE FULLTEXT INDEX `idx_notes_ft` ON `shop`.`Order ``Items``` (`notes`);"
        );
        assert!(
            create_ft
                .warnings
                .iter()
                .any(|w| w.contains("implicit commit"))
        );

        let drop_idx = build_index_constraint_ddl(
            EngineKind::Mysql,
            &IndexConstraintAction::DropIndex {
                table: table.clone(),
                name: "idx_notes_ft".into(),
                concurrently: false,
            },
        )
        .expect("mysql drop index");
        assert_eq!(
            drop_idx.statement.sql,
            "DROP INDEX `idx_notes_ft` ON `shop`.`Order ``Items```;"
        );

        for (kind, name, expected_clause) in [
            (ConstraintKind::PrimaryKey, "PRIMARY", "DROP PRIMARY KEY;"),
            (
                ConstraintKind::ForeignKey,
                "fk_cust",
                "DROP FOREIGN KEY `fk_cust`;",
            ),
            (ConstraintKind::Unique, "uq_code", "DROP INDEX `uq_code`;"),
            (ConstraintKind::Check, "chk_qty", "DROP CHECK `chk_qty`;"),
        ] {
            let dropped = build_index_constraint_ddl(
                EngineKind::Mysql,
                &IndexConstraintAction::DropConstraint {
                    table: table.clone(),
                    name: name.into(),
                    kind,
                    cascade: false,
                },
            )
            .expect("mysql drop constraint");
            assert!(
                dropped.statement.sql.ends_with(expected_clause),
                "unexpected SQL: {}",
                dropped.statement.sql
            );
        }

        // Rejects PostgreSQL-only flags and statement injection attempts
        assert_eq!(
            build_index_constraint_ddl(
                EngineKind::Mysql,
                &IndexConstraintAction::CreateIndex {
                    table: table.clone(),
                    name: "idx_bad".into(),
                    unique: false,
                    method: "btree".into(),
                    concurrently: false,
                    expressions: vec!["id".into()],
                    raw_expressions: false,
                    predicate: Some("id > 0".into()),
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::Unsupported
        );

        assert_eq!(
            build_index_constraint_ddl(
                EngineKind::Postgres,
                &IndexConstraintAction::CreateIndex {
                    table,
                    name: "idx_inj".into(),
                    unique: false,
                    method: "btree".into(),
                    concurrently: false,
                    expressions: vec!["id".into()],
                    raw_expressions: false,
                    predicate: Some("id > 0; DROP TABLE users".into()),
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn postgres_table_and_column_builder_handles_identity_alter_using_rename_and_drop() {
        let table = TableRef {
            schema: "public".into(),
            name: "Invoices".into(),
        };

        let created = build_table_object_ddl(
            EngineKind::Postgres,
            &TableObjectAction::CreateTable {
                table: table.clone(),
                columns: vec![
                    TableColumnDraft {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                        default_expression: None,
                        auto_increment: true,
                        primary_key: true,
                    },
                    TableColumnDraft {
                        name: "Amount Cents".into(),
                        data_type: "integer".into(),
                        nullable: false,
                        default_expression: Some("0".into()),
                        auto_increment: false,
                        primary_key: false,
                    },
                ],
                comment: Some("Customer invoices".into()),
            },
        )
        .expect("pg create table");
        assert_eq!(created.statements.len(), 2);
        assert_eq!(
            created.statements[0].sql,
            "CREATE TABLE \"public\".\"Invoices\" (\n  \"id\" bigint GENERATED BY DEFAULT AS IDENTITY NOT NULL,\n  \"Amount Cents\" integer DEFAULT 0 NOT NULL,\n  CONSTRAINT \"Invoices_pkey\" PRIMARY KEY (\"id\")\n);"
        );
        assert_eq!(
            created.statements[1].sql,
            "COMMENT ON TABLE \"public\".\"Invoices\" IS 'Customer invoices';"
        );

        let altered = build_table_object_ddl(
            EngineKind::Postgres,
            &TableObjectAction::AlterColumn {
                table: table.clone(),
                column_name: "Amount Cents".into(),
                new_name: Some("total_cents".into()),
                data_type: "bigint".into(),
                nullable: false,
                default_expression: Some("100".into()),
                using_expression: Some("\"Amount Cents\"::bigint".into()),
            },
        )
        .expect("pg alter column");
        assert_eq!(altered.statements.len(), 3);
        assert_eq!(
            altered.statements[0].sql,
            "ALTER TABLE \"public\".\"Invoices\" ALTER COLUMN \"Amount Cents\" DROP DEFAULT;"
        );
        assert_eq!(
            altered.statements[1].sql,
            "ALTER TABLE \"public\".\"Invoices\" ALTER COLUMN \"Amount Cents\" TYPE bigint USING \"Amount Cents\"::bigint, ALTER COLUMN \"Amount Cents\" SET NOT NULL, ALTER COLUMN \"Amount Cents\" SET DEFAULT 100;"
        );
        assert_eq!(
            altered.statements[2].sql,
            "ALTER TABLE \"public\".\"Invoices\" RENAME COLUMN \"Amount Cents\" TO \"total_cents\";"
        );

        let dropped_col = build_table_object_ddl(
            EngineKind::Postgres,
            &TableObjectAction::DropColumn {
                table: table.clone(),
                column_name: "total_cents".into(),
                cascade: true,
            },
        )
        .expect("pg drop column");
        assert_eq!(
            dropped_col.script,
            "ALTER TABLE \"public\".\"Invoices\" DROP COLUMN \"total_cents\" CASCADE;"
        );
        assert!(dropped_col.statements[0].destructive);

        let dropped_obj = build_table_object_ddl(
            EngineKind::Postgres,
            &TableObjectAction::DropObject {
                object: DatabaseObjectRef {
                    schema: "public".into(),
                    name: "Invoices".into(),
                    kind: DatabaseObjectKind::Table,
                    identity: None,
                },
                cascade: true,
            },
        )
        .expect("pg drop table");
        assert_eq!(
            dropped_obj.script,
            "DROP TABLE \"public\".\"Invoices\" CASCADE;"
        );
        assert!(dropped_obj.statements[0].destructive);
    }

    #[test]
    fn mysql_table_and_column_builder_uses_auto_increment_change_modify_and_rejects_pg_clauses() {
        let table = TableRef {
            schema: "shop".into(),
            name: "audit_events".into(),
        };

        let created = build_table_object_ddl(
            EngineKind::Mysql,
            &TableObjectAction::CreateTable {
                table: table.clone(),
                columns: vec![
                    TableColumnDraft {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                        default_expression: None,
                        auto_increment: true,
                        primary_key: true,
                    },
                    TableColumnDraft {
                        name: "event_type".into(),
                        data_type: "varchar(64)".into(),
                        nullable: false,
                        default_expression: Some("'created'".into()),
                        auto_increment: false,
                        primary_key: false,
                    },
                ],
                comment: Some("Audit log".into()),
            },
        )
        .expect("mysql create table");
        assert_eq!(created.statements.len(), 1);
        assert_eq!(
            created.script,
            "CREATE TABLE `shop`.`audit_events` (\n  `id` bigint NOT NULL AUTO_INCREMENT,\n  `event_type` varchar(64) DEFAULT 'created' NOT NULL,\n  PRIMARY KEY (`id`)\n) COMMENT = 'Audit log';"
        );

        let renamed = build_table_object_ddl(
            EngineKind::Mysql,
            &TableObjectAction::AlterColumn {
                table: table.clone(),
                column_name: "event_type".into(),
                new_name: Some("kind".into()),
                data_type: "varchar(128)".into(),
                nullable: true,
                default_expression: None,
                using_expression: None,
            },
        )
        .expect("mysql change column");
        assert_eq!(
            renamed.script,
            "ALTER TABLE `shop`.`audit_events` CHANGE COLUMN `event_type` `kind` varchar(128) NULL;"
        );

        // MySQL rejects AUTO_INCREMENT without PRIMARY KEY and rejects USING expressions
        assert_eq!(
            build_table_object_ddl(
                EngineKind::Mysql,
                &TableObjectAction::CreateTable {
                    table: table.clone(),
                    columns: vec![TableColumnDraft {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                        default_expression: None,
                        auto_increment: true,
                        primary_key: false,
                    }],
                    comment: None,
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            build_table_object_ddl(
                EngineKind::Mysql,
                &TableObjectAction::AlterColumn {
                    table,
                    column_name: "kind".into(),
                    new_name: None,
                    data_type: "varchar(64)".into(),
                    nullable: false,
                    default_expression: None,
                    using_expression: Some("CAST(kind AS CHAR(64))".into()),
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::Unsupported
        );
    }

    #[test]
    fn postgres_table_maintenance_builds_analyze_vacuum_and_reindex_with_warnings() {
        let table = TableRef {
            schema: "public".into(),
            name: "Order Items".into(),
        };

        let analyze_all = build_table_maintenance_sql(
            EngineKind::Postgres,
            &TableMaintenanceAction::Analyze {
                table: table.clone(),
                verbose: false,
                columns: vec![],
            },
        )
        .unwrap();
        assert_eq!(
            analyze_all.statement.sql,
            "ANALYZE \"public\".\"Order Items\";"
        );
        assert!(!analyze_all.statement.destructive);
        assert!(!analyze_all.non_transactional);
        assert!(
            analyze_all
                .warnings
                .iter()
                .any(|w| w.contains("SHARE UPDATE EXCLUSIVE"))
        );

        let analyze_cols = build_table_maintenance_sql(
            EngineKind::Postgres,
            &TableMaintenanceAction::Analyze {
                table: table.clone(),
                verbose: true,
                columns: vec!["Unit Price".into(), "status".into()],
            },
        )
        .unwrap();
        assert_eq!(
            analyze_cols.statement.sql,
            "ANALYZE (VERBOSE) \"public\".\"Order Items\" (\"Unit Price\", \"status\");"
        );

        // Duplicate column is rejected
        let dup_err = build_table_maintenance_sql(
            EngineKind::Postgres,
            &TableMaintenanceAction::Analyze {
                table: table.clone(),
                verbose: false,
                columns: vec!["status".into(), "status".into()],
            },
        )
        .unwrap_err();
        assert_eq!(dup_err.code, ErrorCode::InvalidInput);

        let vacuum_std = build_table_maintenance_sql(
            EngineKind::Postgres,
            &TableMaintenanceAction::Vacuum {
                table: table.clone(),
                analyze: true,
                verbose: false,
                full: false,
            },
        )
        .unwrap();
        assert_eq!(
            vacuum_std.statement.sql,
            "VACUUM (ANALYZE) \"public\".\"Order Items\";"
        );
        assert!(!vacuum_std.statement.destructive);
        assert!(vacuum_std.non_transactional);
        assert!(
            vacuum_std
                .warnings
                .iter()
                .any(|w| w.contains("cannot run inside a transaction block"))
        );

        let vacuum_full = build_table_maintenance_sql(
            EngineKind::Postgres,
            &TableMaintenanceAction::Vacuum {
                table: table.clone(),
                analyze: true,
                verbose: true,
                full: true,
            },
        )
        .unwrap();
        assert_eq!(
            vacuum_full.statement.sql,
            "VACUUM (FULL, ANALYZE, VERBOSE) \"public\".\"Order Items\";"
        );
        assert!(vacuum_full.statement.destructive);
        assert!(vacuum_full.non_transactional);
        assert!(
            vacuum_full
                .warnings
                .iter()
                .any(|w| w.contains("ACCESS EXCLUSIVE"))
        );

        let reindex = build_table_maintenance_sql(
            EngineKind::Postgres,
            &TableMaintenanceAction::ReindexTable { table },
        )
        .unwrap();
        assert_eq!(
            reindex.statement.sql,
            "REINDEX TABLE \"public\".\"Order Items\";"
        );
        assert!(!reindex.non_transactional);
    }

    #[test]
    fn mysql_table_maintenance_builds_analyze_optimize_and_check_and_rejects_mismatched_engines() {
        let table = TableRef {
            schema: "joinery".into(),
            name: "order_items".into(),
        };

        let analyze = build_table_maintenance_sql(
            EngineKind::Mysql,
            &TableMaintenanceAction::AnalyzeTable {
                table: table.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            analyze.statement.sql,
            "ANALYZE TABLE `joinery`.`order_items`;"
        );
        assert!(!analyze.non_transactional);
        assert!(
            analyze
                .warnings
                .iter()
                .any(|w| w.contains("implicit commit"))
        );

        let optimize = build_table_maintenance_sql(
            EngineKind::Mysql,
            &TableMaintenanceAction::OptimizeTable {
                table: table.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            optimize.statement.sql,
            "OPTIMIZE TABLE `joinery`.`order_items`;"
        );

        let check = build_table_maintenance_sql(
            EngineKind::Mysql,
            &TableMaintenanceAction::CheckTable {
                table: table.clone(),
            },
        )
        .unwrap();
        assert_eq!(check.statement.sql, "CHECK TABLE `joinery`.`order_items`;");

        // Cross-engine and Snowflake/Redis rejection
        assert_eq!(
            build_table_maintenance_sql(
                EngineKind::Mysql,
                &TableMaintenanceAction::Vacuum {
                    table: table.clone(),
                    analyze: true,
                    verbose: false,
                    full: false,
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            build_table_maintenance_sql(
                EngineKind::Postgres,
                &TableMaintenanceAction::OptimizeTable {
                    table: table.clone(),
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            build_table_maintenance_sql(
                EngineKind::Snowflake,
                &TableMaintenanceAction::Analyze {
                    table,
                    verbose: false,
                    columns: vec![],
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::Unsupported
        );
    }
}
