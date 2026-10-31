use super::pg_error;
use crate::{domain::*, safety::quote_identifier};
use tokio_postgres::Client;

const MAX_DISCOVERED_OBJECTS: i64 = 10_000;
const MAX_DEPENDENCY_EDGES: usize = 200;

pub(super) async fn database_objects_on(client: &Client) -> AppResult<Vec<DatabaseObjectSummary>> {
    let rows = client
        .query(
            "SELECT n.nspname::pg_catalog.text AS schema_name,
                    p.proname::pg_catalog.text AS obj_name,
                    CASE WHEN p.prokind = 'p' THEN 'procedure' ELSE 'function' END AS obj_kind,
                    pg_catalog.pg_get_function_identity_arguments(p.oid)::pg_catalog.text AS obj_identity,
                    NULL::pg_catalog.text AS owner_table,
                    CASE
                      WHEN p.prokind = 'p' THEN '(' || pg_catalog.pg_get_function_identity_arguments(p.oid) || ')'
                      ELSE '(' || pg_catalog.pg_get_function_identity_arguments(p.oid) || ') -> ' || COALESCE(pg_catalog.pg_get_function_result(p.oid), 'void')
                    END::pg_catalog.text AS signature,
                    NULL::pg_catalog.float8 AS total_bytes,
                    1::pg_catalog.int4 AS kind_order
             FROM pg_catalog.pg_proc p
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
             WHERE p.prokind IN ('f', 'p')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    c.relname::pg_catalog.text,
                    'sequence',
                    NULL::pg_catalog.text,
                    NULL::pg_catalog.text,
                    pg_catalog.format_type(s.seqtypid, NULL)::pg_catalog.text,
                    CASE
                      WHEN pg_catalog.has_sequence_privilege(c.oid, 'SELECT')
                        OR pg_catalog.has_sequence_privilege(c.oid, 'USAGE')
                      THEN pg_catalog.pg_total_relation_size(c.oid)::pg_catalog.float8
                      ELSE NULL::pg_catalog.float8
                    END,
                    2::pg_catalog.int4
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             LEFT JOIN pg_catalog.pg_sequence s ON s.seqrelid = c.oid
             WHERE c.relkind = 'S'
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    t.tgname::pg_catalog.text,
                    'trigger',
                    tbl.relname::pg_catalog.text,
                    tbl.relname::pg_catalog.text,
                    NULL::pg_catalog.text,
                    NULL::pg_catalog.float8,
                    3::pg_catalog.int4
             FROM pg_catalog.pg_trigger t
             JOIN pg_catalog.pg_class tbl ON tbl.oid = t.tgrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = tbl.relnamespace
             WHERE NOT t.tgisinternal
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    idx.relname::pg_catalog.text,
                    'index',
                    tbl.relname::pg_catalog.text,
                    tbl.relname::pg_catalog.text,
                    am.amname::pg_catalog.text,
                    CASE
                      WHEN pg_catalog.has_table_privilege(tbl.oid, 'SELECT')
                      THEN pg_catalog.pg_total_relation_size(idx.oid)::pg_catalog.float8
                      ELSE NULL::pg_catalog.float8
                    END,
                    4::pg_catalog.int4
             FROM pg_catalog.pg_index i
             JOIN pg_catalog.pg_class idx ON idx.oid = i.indexrelid
             JOIN pg_catalog.pg_class tbl ON tbl.oid = i.indrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = idx.relnamespace
             JOIN pg_catalog.pg_am am ON am.oid = idx.relam
             WHERE idx.relkind IN ('i', 'I')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             ORDER BY 1, 8, 2, 4 NULLS FIRST
             LIMIT $1",
            &[&MAX_DISCOVERED_OBJECTS],
        )
        .await
        .map_err(pg_error)?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let kind = match r.get::<_, &str>(2) {
                "function" => DatabaseObjectKind::Function,
                "procedure" => DatabaseObjectKind::Procedure,
                "sequence" => DatabaseObjectKind::Sequence,
                "trigger" => DatabaseObjectKind::Trigger,
                "index" => DatabaseObjectKind::Index,
                _ => return None,
            };
            Some(DatabaseObjectSummary {
                object: DatabaseObjectRef {
                    schema: r.get(0),
                    name: r.get(1),
                    kind,
                    identity: r.get(3),
                },
                owner_table: r.get(4),
                signature: r.get(5),
                estimated_rows: None,
                total_bytes: r.get(6),
            })
        })
        .collect())
}

pub(super) async fn object_details_on(
    client: &Client,
    requested: &DatabaseObjectRef,
) -> AppResult<ObjectDetails> {
    match requested.kind {
        DatabaseObjectKind::Table
        | DatabaseObjectKind::PartitionedTable
        | DatabaseObjectKind::View
        | DatabaseObjectKind::MaterializedView => relation_details(client, requested).await,
        DatabaseObjectKind::Function | DatabaseObjectKind::Procedure => {
            routine_details(client, requested).await
        }
        DatabaseObjectKind::Sequence => sequence_details(client, requested).await,
        DatabaseObjectKind::Trigger => trigger_details(client, requested).await,
        DatabaseObjectKind::Index => index_details(client, requested).await,
    }
}

fn relkind_to_kind(relkind: &str) -> Option<DatabaseObjectKind> {
    match relkind {
        "r" => Some(DatabaseObjectKind::Table),
        "p" => Some(DatabaseObjectKind::PartitionedTable),
        "v" => Some(DatabaseObjectKind::View),
        "m" => Some(DatabaseObjectKind::MaterializedView),
        "S" => Some(DatabaseObjectKind::Sequence),
        "i" | "I" => Some(DatabaseObjectKind::Index),
        _ => None,
    }
}

pub(crate) fn decode_pg_trigger_type(tgtype: i16) -> (&'static str, Vec<String>, &'static str) {
    let orientation = if (tgtype & 1) != 0 {
        "ROW"
    } else {
        "STATEMENT"
    };
    let timing = if (tgtype & 2) != 0 {
        "BEFORE"
    } else if (tgtype & 64) != 0 {
        "INSTEAD OF"
    } else {
        "AFTER"
    };
    let mut events = Vec::new();
    if (tgtype & 4) != 0 {
        events.push("INSERT".into());
    }
    if (tgtype & 8) != 0 {
        events.push("DELETE".into());
    }
    if (tgtype & 16) != 0 {
        events.push("UPDATE".into());
    }
    if (tgtype & 32) != 0 {
        events.push("TRUNCATE".into());
    }
    (timing, events, orientation)
}

fn ensure_semicolon(sql: &str) -> String {
    let trimmed = sql.trim();
    if trimmed.ends_with(';') {
        trimmed.to_owned()
    } else {
        format!("{trimmed};")
    }
}

pub(crate) struct PgTableColumnSpec {
    pub name: String,
    pub formatted_type: String,
    pub not_null: bool,
    pub identity: String,
    pub generated: String,
    pub default_expr: Option<String>,
}

pub(crate) fn synthesize_pg_table_ddl(
    schema: &str,
    name: &str,
    columns: &[PgTableColumnSpec],
    constraints: &[ConstraintInfo],
    partition_key: Option<&str>,
    standalone_index_defs: &[String],
    trigger_defs: &[String],
) -> String {
    let mut entries = Vec::with_capacity(columns.len() + constraints.len());
    for col in columns {
        let mut line = format!("  {} {}", quote_identifier(&col.name), col.formatted_type);
        if col.identity == "a" {
            line.push_str(" GENERATED ALWAYS AS IDENTITY");
        } else if col.identity == "d" {
            line.push_str(" GENERATED BY DEFAULT AS IDENTITY");
        } else if col.generated == "s" {
            if let Some(expr) = &col.default_expr {
                line.push_str(&format!(" GENERATED ALWAYS AS ({expr}) STORED"));
            }
        } else if let Some(expr) = &col.default_expr {
            line.push_str(&format!(" DEFAULT {expr}"));
        }
        if col.not_null {
            line.push_str(" NOT NULL");
        }
        entries.push(line);
    }
    for constraint in constraints {
        entries.push(format!(
            "  CONSTRAINT {} {}",
            quote_identifier(&constraint.name),
            constraint.definition
        ));
    }
    let mut ddl = format!(
        "CREATE TABLE {}.{} (\n{}\n)",
        quote_identifier(schema),
        quote_identifier(name),
        entries.join(",\n")
    );
    if let Some(part) = partition_key.filter(|s| !s.trim().is_empty()) {
        ddl.push_str(&format!("\nPARTITION BY {part}"));
    }
    ddl.push(';');
    for index_def in standalone_index_defs {
        ddl.push_str("\n\n");
        ddl.push_str(&ensure_semicolon(index_def));
    }
    for trig_def in trigger_defs {
        ddl.push_str("\n\n");
        ddl.push_str(&ensure_semicolon(trig_def));
    }
    ddl
}

pub(crate) fn synthesize_pg_sequence_ddl(schema: &str, name: &str, seq: &SequenceInfo) -> String {
    format!(
        "CREATE SEQUENCE {}.{}\n  AS {}\n  INCREMENT BY {}\n  MINVALUE {}\n  MAXVALUE {}\n  START WITH {}\n  CACHE {}\n  {};",
        quote_identifier(schema),
        quote_identifier(name),
        seq.data_type,
        seq.increment_by,
        seq.min_value,
        seq.max_value,
        seq.start_value,
        seq.cache_size,
        if seq.cycle { "CYCLE" } else { "NO CYCLE" }
    )
}

async fn relation_details(
    client: &Client,
    requested: &DatabaseObjectRef,
) -> AppResult<ObjectDetails> {
    let rows = client
        .query(
            "SELECT c.oid,
                    c.relkind::pg_catalog.text,
                    pg_catalog.obj_description(c.oid, 'pg_class')::pg_catalog.text
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1
               AND c.relname = $2
               AND c.relkind IN ('r', 'p', 'v', 'm')
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')",
            &[&requested.schema, &requested.name],
        )
        .await
        .map_err(pg_error)?;
    let row = rows.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The database object is missing or its schema is unavailable",
        )
    })?;
    let oid: u32 = row.get(0);
    let relkind: String = row.get(1);
    let comment: Option<String> = row.get(2);
    let kind = relkind_to_kind(&relkind).unwrap_or(requested.kind);
    let object = DatabaseObjectRef {
        schema: requested.schema.clone(),
        name: requested.name.clone(),
        kind,
        identity: None,
    };

    let (indexes, standalone_index_defs) = fetch_relation_indexes(client, oid).await?;
    let constraints = fetch_relation_constraints(client, oid).await?;
    let triggers = fetch_relation_triggers(client, oid).await?;
    let trigger_defs = triggers
        .iter()
        .filter_map(|t| t.definition.clone())
        .collect::<Vec<_>>();

    let ddl = match kind {
        DatabaseObjectKind::Table | DatabaseObjectKind::PartitionedTable => {
            let col_rows = client
                .query(
                    "SELECT a.attname::pg_catalog.text,
                            pg_catalog.format_type(a.atttypid, a.atttypmod)::pg_catalog.text,
                            a.attnotnull,
                            a.attidentity::pg_catalog.text,
                            a.attgenerated::pg_catalog.text,
                            CASE
                              WHEN ad.adbin IS NOT NULL
                              THEN pg_catalog.pg_get_expr(ad.adbin, ad.adrelid, true)::pg_catalog.text
                              ELSE NULL::pg_catalog.text
                            END
                     FROM pg_catalog.pg_attribute a
                     LEFT JOIN pg_catalog.pg_attrdef ad
                       ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum
                     WHERE a.attrelid = $1::pg_catalog.oid
                       AND a.attnum > 0
                       AND NOT a.attisdropped
                     ORDER BY a.attnum",
                    &[&oid],
                )
                .await
                .map_err(pg_error)?;
            let cols = col_rows
                .into_iter()
                .map(|r| PgTableColumnSpec {
                    name: r.get(0),
                    formatted_type: r.get(1),
                    not_null: r.get(2),
                    identity: r.get(3),
                    generated: r.get(4),
                    default_expr: r.get(5),
                })
                .collect::<Vec<_>>();
            let part_key = if kind == DatabaseObjectKind::PartitionedTable {
                client
                    .query_opt(
                        "SELECT pg_catalog.pg_get_partkeydef($1::pg_catalog.oid)::pg_catalog.text",
                        &[&oid],
                    )
                    .await
                    .ok()
                    .flatten()
                    .and_then(|r| r.get::<_, Option<String>>(0))
            } else {
                None
            };
            ObjectDdlStatus::Available {
                ddl: synthesize_pg_table_ddl(
                    &object.schema,
                    &object.name,
                    &cols,
                    &constraints,
                    part_key.as_deref(),
                    &standalone_index_defs,
                    &trigger_defs,
                ),
                truncated: false,
            }
        }
        DatabaseObjectKind::View | DatabaseObjectKind::MaterializedView => {
            match client
                .query_one(
                    "SELECT pg_catalog.pg_get_viewdef($1::pg_catalog.oid, true)::pg_catalog.text",
                    &[&oid],
                )
                .await
            {
                Ok(r) => match r.get::<_, Option<String>>(0) {
                    Some(body) if !body.trim().is_empty() => {
                        let header = if kind == DatabaseObjectKind::MaterializedView {
                            format!(
                                "CREATE MATERIALIZED VIEW {}.{} AS\n{}",
                                quote_identifier(&object.schema),
                                quote_identifier(&object.name),
                                ensure_semicolon(body.trim().trim_end_matches(';'))
                            )
                        } else {
                            format!(
                                "CREATE OR REPLACE VIEW {}.{} AS\n{}",
                                quote_identifier(&object.schema),
                                quote_identifier(&object.name),
                                ensure_semicolon(body.trim().trim_end_matches(';'))
                            )
                        };
                        let mut full = header;
                        for idx_def in &standalone_index_defs {
                            full.push_str("\n\n");
                            full.push_str(&ensure_semicolon(idx_def));
                        }
                        for trig_def in &trigger_defs {
                            full.push_str("\n\n");
                            full.push_str(&ensure_semicolon(trig_def));
                        }
                        ObjectDdlStatus::Available {
                            ddl: full,
                            truncated: false,
                        }
                    }
                    _ => ObjectDdlStatus::Unavailable {
                        reason: "View definition is unavailable with current privileges".into(),
                    },
                },
                Err(err) => ObjectDdlStatus::Unavailable {
                    reason: pg_error(err).message,
                },
            }
        }
        _ => unreachable!(),
    };

    let (uses, used_by) = relation_dependencies(client, oid, &object).await?;

    Ok(ObjectDetails {
        object,
        owner_table: None,
        comment,
        ddl,
        indexes,
        constraints,
        triggers,
        routine: None,
        sequence: None,
        uses,
        used_by,
    })
}

async fn fetch_relation_indexes(
    client: &Client,
    rel_oid: u32,
) -> AppResult<(Vec<IndexInfo>, Vec<String>)> {
    let rows = client
        .query(
            "SELECT idx.relname::pg_catalog.text,
                    am.amname::pg_catalog.text,
                    i.indisunique,
                    i.indisprimary,
                    ARRAY(
                      SELECT pg_catalog.pg_get_indexdef(i.indexrelid, k::pg_catalog.int4, true)
                      FROM pg_catalog.generate_series(1, i.indnkeyatts::pg_catalog.int4) AS g(k)
                      ORDER BY g.k
                    )::pg_catalog.text[],
                    CASE
                      WHEN i.indpred IS NOT NULL
                      THEN pg_catalog.pg_get_expr(i.indpred, i.indrelid, true)::pg_catalog.text
                      ELSE NULL::pg_catalog.text
                    END,
                    pg_catalog.pg_get_indexdef(i.indexrelid)::pg_catalog.text,
                    EXISTS (
                      SELECT 1 FROM pg_catalog.pg_constraint kc
                      WHERE kc.conrelid = i.indrelid AND kc.conindid = i.indexrelid
                    )
             FROM pg_catalog.pg_index i
             JOIN pg_catalog.pg_class idx ON idx.oid = i.indexrelid
             JOIN pg_catalog.pg_am am ON am.oid = idx.relam
             WHERE i.indrelid = $1::pg_catalog.oid
             ORDER BY i.indisprimary DESC, idx.relname",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;

    let mut indexes = Vec::with_capacity(rows.len());
    let mut standalone = Vec::new();
    for r in rows {
        let definition: Option<String> = r.get(6);
        let backs_constraint: bool = r.get(7);
        if !backs_constraint && let Some(def) = &definition {
            standalone.push(def.clone());
        }
        indexes.push(IndexInfo {
            name: r.get(0),
            method: r.get(1),
            unique: r.get(2),
            primary: r.get(3),
            expressions: r.get(4),
            predicate: r.get(5),
            definition,
        });
    }
    Ok((indexes, standalone))
}

async fn fetch_relation_constraints(
    client: &Client,
    rel_oid: u32,
) -> AppResult<Vec<ConstraintInfo>> {
    let rows = client
        .query(
            "SELECT k.conname::pg_catalog.text,
                    k.contype::pg_catalog.text,
                    ARRAY(
                      SELECT a.attname::pg_catalog.text
                      FROM pg_catalog.unnest(COALESCE(k.conkey, '{}'::pg_catalog.int2[])) WITH ORDINALITY AS u(attnum, ord)
                      JOIN pg_catalog.pg_attribute a
                        ON a.attrelid = k.conrelid AND a.attnum = u.attnum AND NOT a.attisdropped
                      ORDER BY u.ord
                    )::pg_catalog.text[],
                    tn.nspname::pg_catalog.text,
                    tc.relname::pg_catalog.text,
                    ARRAY(
                      SELECT b.attname::pg_catalog.text
                      FROM pg_catalog.unnest(COALESCE(k.confkey, '{}'::pg_catalog.int2[])) WITH ORDINALITY AS u(attnum, ord)
                      JOIN pg_catalog.pg_attribute b
                        ON b.attrelid = k.confrelid AND b.attnum = u.attnum AND NOT b.attisdropped
                      ORDER BY u.ord
                    )::pg_catalog.text[],
                    pg_catalog.pg_get_constraintdef(k.oid, true)::pg_catalog.text
             FROM pg_catalog.pg_constraint k
             LEFT JOIN pg_catalog.pg_class tc ON tc.oid = NULLIF(k.confrelid, 0)
             LEFT JOIN pg_catalog.pg_namespace tn ON tn.oid = tc.relnamespace
             WHERE k.conrelid = $1::pg_catalog.oid
               AND k.contype IN ('p', 'f', 'u', 'c', 'x')
             ORDER BY CASE k.contype
                        WHEN 'p' THEN 1
                        WHEN 'f' THEN 2
                        WHEN 'u' THEN 3
                        WHEN 'c' THEN 4
                        ELSE 5
                      END,
                      k.conname",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let kind = match r.get::<_, &str>(1) {
                "p" => ConstraintKind::PrimaryKey,
                "f" => ConstraintKind::ForeignKey,
                "u" => ConstraintKind::Unique,
                "c" => ConstraintKind::Check,
                "x" => ConstraintKind::Exclusion,
                _ => return None,
            };
            let target = match (r.get::<_, Option<String>>(3), r.get::<_, Option<String>>(4)) {
                (Some(schema), Some(name)) => Some(TableRef { schema, name }),
                _ => None,
            };
            Some(ConstraintInfo {
                name: r.get(0),
                kind,
                columns: r.get(2),
                target,
                target_columns: r.get(5),
                definition: r.get(6),
            })
        })
        .collect())
}

async fn fetch_relation_triggers(client: &Client, rel_oid: u32) -> AppResult<Vec<TriggerInfo>> {
    let rows = client
        .query(
            "SELECT t.tgname::pg_catalog.text,
                    tbl.relname::pg_catalog.text,
                    t.tgtype::pg_catalog.int2,
                    t.tgenabled::pg_catalog.text,
                    pg_catalog.pg_get_triggerdef(t.oid, true)::pg_catalog.text
             FROM pg_catalog.pg_trigger t
             JOIN pg_catalog.pg_class tbl ON tbl.oid = t.tgrelid
             WHERE t.tgrelid = $1::pg_catalog.oid
               AND NOT t.tgisinternal
             ORDER BY t.tgname",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let tgtype: i16 = r.get(2);
            let (timing, events, orientation) = decode_pg_trigger_type(tgtype);
            let enabled_flag: String = r.get(3);
            TriggerInfo {
                name: r.get(0),
                table_name: r.get(1),
                timing: timing.into(),
                events,
                orientation: orientation.into(),
                enabled: enabled_flag != "D",
                definition: r.get(4),
            }
        })
        .collect())
}

async fn routine_details(
    client: &Client,
    requested: &DatabaseObjectRef,
) -> AppResult<ObjectDetails> {
    let rows = client
        .query(
            "SELECT p.oid,
                    p.prokind::pg_catalog.text,
                    pg_catalog.pg_get_function_identity_arguments(p.oid)::pg_catalog.text,
                    pg_catalog.pg_get_function_arguments(p.oid)::pg_catalog.text,
                    pg_catalog.pg_get_function_result(p.oid)::pg_catalog.text,
                    l.lanname::pg_catalog.text,
                    p.provolatile::pg_catalog.text,
                    p.prosecdef,
                    pg_catalog.obj_description(p.oid, 'pg_proc')::pg_catalog.text
             FROM pg_catalog.pg_proc p
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
             JOIN pg_catalog.pg_language l ON l.oid = p.prolang
             WHERE n.nspname = $1
               AND p.proname = $2
               AND p.prokind IN ('f', 'p')
               AND ($3::pg_catalog.text IS NULL OR pg_catalog.pg_get_function_identity_arguments(p.oid) = $3)
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             ORDER BY pg_catalog.pg_get_function_identity_arguments(p.oid), p.oid
             LIMIT 1",
            &[&requested.schema, &requested.name, &requested.identity],
        )
        .await
        .map_err(pg_error)?;

    let row = rows.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The routine is missing or its schema is unavailable",
        )
    })?;
    let oid: u32 = row.get(0);
    let prokind: String = row.get(1);
    let identity_args: String = row.get(2);
    let arguments: String = row.get(3);
    let return_type: Option<String> = row.get(4);
    let language: String = row.get(5);
    let provolatile: String = row.get(6);
    let security_definer: bool = row.get(7);
    let comment: Option<String> = row.get(8);

    let kind = if prokind == "p" {
        DatabaseObjectKind::Procedure
    } else {
        DatabaseObjectKind::Function
    };
    let object = DatabaseObjectRef {
        schema: requested.schema.clone(),
        name: requested.name.clone(),
        kind,
        identity: Some(identity_args.clone()),
    };

    let volatility = if kind == DatabaseObjectKind::Procedure {
        None
    } else {
        Some(
            match provolatile.as_str() {
                "i" => "IMMUTABLE",
                "s" => "STABLE",
                _ => "VOLATILE",
            }
            .into(),
        )
    };

    let ddl = if matches!(language.as_str(), "internal" | "c") {
        ObjectDdlStatus::Unavailable {
            reason: format!("DDL definition is unavailable for {language} routines"),
        }
    } else {
        match client
            .query_one(
                "SELECT pg_catalog.pg_get_functiondef($1::pg_catalog.oid)::pg_catalog.text",
                &[&oid],
            )
            .await
        {
            Ok(r) => match r.get::<_, Option<String>>(0) {
                Some(def) if !def.trim().is_empty() => ObjectDdlStatus::Available {
                    ddl: ensure_semicolon(&def),
                    truncated: false,
                },
                _ => ObjectDdlStatus::Unavailable {
                    reason: "Routine definition is unavailable with current privileges".into(),
                },
            },
            Err(err) => ObjectDdlStatus::Unavailable {
                reason: pg_error(err).message,
            },
        }
    };

    let (uses, used_by) = routine_dependencies(client, oid, &object).await?;

    Ok(ObjectDetails {
        object,
        owner_table: None,
        comment,
        ddl,
        indexes: Vec::new(),
        constraints: Vec::new(),
        triggers: Vec::new(),
        routine: Some(RoutineInfo {
            identity_arguments: identity_args,
            arguments,
            return_type,
            language,
            volatility,
            security_definer,
        }),
        sequence: None,
        uses,
        used_by,
    })
}

async fn sequence_details(
    client: &Client,
    requested: &DatabaseObjectRef,
) -> AppResult<ObjectDetails> {
    let rows = client
        .query(
            "SELECT c.oid,
                    pg_catalog.format_type(s.seqtypid, NULL)::pg_catalog.text,
                    s.seqstart::pg_catalog.text,
                    s.seqmin::pg_catalog.text,
                    s.seqmax::pg_catalog.text,
                    s.seqincrement::pg_catalog.text,
                    s.seqcache::pg_catalog.text,
                    s.seqcycle,
                    pg_catalog.obj_description(c.oid, 'pg_class')::pg_catalog.text
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_sequence s ON s.seqrelid = c.oid
             WHERE n.nspname = $1
               AND c.relname = $2
               AND c.relkind = 'S'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')",
            &[&requested.schema, &requested.name],
        )
        .await
        .map_err(pg_error)?;

    let row = rows.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The sequence is missing or its schema is unavailable",
        )
    })?;
    let oid: u32 = row.get(0);
    let comment: Option<String> = row.get(8);
    let last_value = client
        .query_opt(
            "SELECT last_value::pg_catalog.text
             FROM pg_catalog.pg_sequences
             WHERE schemaname = $1 AND sequencename = $2",
            &[&requested.schema, &requested.name],
        )
        .await
        .ok()
        .flatten()
        .and_then(|r| r.get::<_, Option<String>>(0));

    let sequence = SequenceInfo {
        data_type: row.get(1),
        start_value: row.get(2),
        min_value: row.get(3),
        max_value: row.get(4),
        increment_by: row.get(5),
        cache_size: row.get(6),
        cycle: row.get(7),
        last_value,
    };

    let object = DatabaseObjectRef {
        schema: requested.schema.clone(),
        name: requested.name.clone(),
        kind: DatabaseObjectKind::Sequence,
        identity: None,
    };
    let ddl = ObjectDdlStatus::Available {
        ddl: synthesize_pg_sequence_ddl(&object.schema, &object.name, &sequence),
        truncated: false,
    };
    let (uses, used_by) = relation_dependencies(client, oid, &object).await?;

    Ok(ObjectDetails {
        object,
        owner_table: None,
        comment,
        ddl,
        indexes: Vec::new(),
        constraints: Vec::new(),
        triggers: Vec::new(),
        routine: None,
        sequence: Some(sequence),
        uses,
        used_by,
    })
}

async fn trigger_details(
    client: &Client,
    requested: &DatabaseObjectRef,
) -> AppResult<ObjectDetails> {
    let rows = client
        .query(
            "SELECT t.oid,
                    tbl.relname::pg_catalog.text,
                    t.tgtype::pg_catalog.int2,
                    t.tgenabled::pg_catalog.text,
                    pg_catalog.pg_get_triggerdef(t.oid, true)::pg_catalog.text,
                    tbl.relkind::pg_catalog.text,
                    fn_ns.nspname::pg_catalog.text,
                    fn.proname::pg_catalog.text,
                    fn.prokind::pg_catalog.text,
                    pg_catalog.pg_get_function_identity_arguments(fn.oid)::pg_catalog.text,
                    pg_catalog.has_schema_privilege(fn_ns.oid, 'USAGE'),
                    pg_catalog.obj_description(t.oid, 'pg_trigger')::pg_catalog.text
             FROM pg_catalog.pg_trigger t
             JOIN pg_catalog.pg_class tbl ON tbl.oid = t.tgrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = tbl.relnamespace
             LEFT JOIN pg_catalog.pg_proc fn ON fn.oid = t.tgfoid
             LEFT JOIN pg_catalog.pg_namespace fn_ns ON fn_ns.oid = fn.pronamespace
             WHERE n.nspname = $1
               AND t.tgname = $2
               AND NOT t.tgisinternal
               AND ($3::pg_catalog.text IS NULL OR tbl.relname = $3)
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             ORDER BY tbl.relname
             LIMIT 1",
            &[&requested.schema, &requested.name, &requested.identity],
        )
        .await
        .map_err(pg_error)?;

    let row = rows.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The trigger is missing or its schema is unavailable",
        )
    })?;
    let table_name: String = row.get(1);
    let tgtype: i16 = row.get(2);
    let enabled_flag: String = row.get(3);
    let definition: Option<String> = row.get(4);
    let tbl_relkind: String = row.get(5);
    let comment: Option<String> = row.get(11);

    let (timing, events, orientation) = decode_pg_trigger_type(tgtype);
    let object = DatabaseObjectRef {
        schema: requested.schema.clone(),
        name: requested.name.clone(),
        kind: DatabaseObjectKind::Trigger,
        identity: Some(table_name.clone()),
    };

    let mut raw_uses = vec![DependencyEdge {
        object: DatabaseObjectRef {
            schema: requested.schema.clone(),
            name: table_name.clone(),
            kind: relkind_to_kind(&tbl_relkind).unwrap_or(DatabaseObjectKind::Table),
            identity: None,
        },
        relation: "target table".into(),
    }];

    if let (Some(fn_schema), Some(fn_name), Some(fn_kind), Some(fn_args), Some(true)) = (
        row.get::<_, Option<String>>(6),
        row.get::<_, Option<String>>(7),
        row.get::<_, Option<String>>(8),
        row.get::<_, Option<String>>(9),
        row.get::<_, Option<bool>>(10),
    ) && !fn_schema.starts_with("pg_")
        && fn_schema != "information_schema"
    {
        raw_uses.push(DependencyEdge {
            object: DatabaseObjectRef {
                schema: fn_schema,
                name: fn_name,
                kind: if fn_kind == "p" {
                    DatabaseObjectKind::Procedure
                } else {
                    DatabaseObjectKind::Function
                },
                identity: Some(fn_args),
            },
            relation: "trigger function".into(),
        });
    }

    let ddl = match &definition {
        Some(def) if !def.trim().is_empty() => ObjectDdlStatus::Available {
            ddl: ensure_semicolon(def),
            truncated: false,
        },
        _ => ObjectDdlStatus::Unavailable {
            reason: "Trigger definition is unavailable with current privileges".into(),
        },
    };

    let trigger_info = TriggerInfo {
        name: object.name.clone(),
        table_name: table_name.clone(),
        timing: timing.into(),
        events,
        orientation: orientation.into(),
        enabled: enabled_flag != "D",
        definition,
    };

    Ok(ObjectDetails {
        uses: finalize_dependency_edges(&object, raw_uses, MAX_DEPENDENCY_EDGES),
        used_by: Vec::new(),
        object,
        owner_table: Some(TableRef {
            schema: requested.schema.clone(),
            name: table_name,
        }),
        comment,
        ddl,
        indexes: Vec::new(),
        constraints: Vec::new(),
        triggers: vec![trigger_info],
        routine: None,
        sequence: None,
    })
}

async fn index_details(client: &Client, requested: &DatabaseObjectRef) -> AppResult<ObjectDetails> {
    let rows = client
        .query(
            "SELECT idx.oid,
                    tbl.relname::pg_catalog.text,
                    tbl.relkind::pg_catalog.text,
                    am.amname::pg_catalog.text,
                    i.indisunique,
                    i.indisprimary,
                    ARRAY(
                      SELECT pg_catalog.pg_get_indexdef(i.indexrelid, k::pg_catalog.int4, true)
                      FROM pg_catalog.generate_series(1, i.indnkeyatts::pg_catalog.int4) AS g(k)
                      ORDER BY g.k
                    )::pg_catalog.text[],
                    CASE
                      WHEN i.indpred IS NOT NULL
                      THEN pg_catalog.pg_get_expr(i.indpred, i.indrelid, true)::pg_catalog.text
                      ELSE NULL::pg_catalog.text
                    END,
                    pg_catalog.pg_get_indexdef(i.indexrelid)::pg_catalog.text,
                    pg_catalog.obj_description(idx.oid, 'pg_class')::pg_catalog.text
             FROM pg_catalog.pg_index i
             JOIN pg_catalog.pg_class idx ON idx.oid = i.indexrelid
             JOIN pg_catalog.pg_class tbl ON tbl.oid = i.indrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = idx.relnamespace
             JOIN pg_catalog.pg_am am ON am.oid = idx.relam
             WHERE n.nspname = $1
               AND idx.relname = $2
               AND ($3::pg_catalog.text IS NULL OR tbl.relname = $3)
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             ORDER BY tbl.relname
             LIMIT 1",
            &[&requested.schema, &requested.name, &requested.identity],
        )
        .await
        .map_err(pg_error)?;

    let row = rows.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The index is missing or its schema is unavailable",
        )
    })?;
    let idx_oid: u32 = row.get(0);
    let table_name: String = row.get(1);
    let tbl_relkind: String = row.get(2);
    let definition: Option<String> = row.get(8);
    let comment: Option<String> = row.get(9);

    let object = DatabaseObjectRef {
        schema: requested.schema.clone(),
        name: requested.name.clone(),
        kind: DatabaseObjectKind::Index,
        identity: Some(table_name.clone()),
    };

    let ddl = match &definition {
        Some(def) if !def.trim().is_empty() => ObjectDdlStatus::Available {
            ddl: ensure_semicolon(def),
            truncated: false,
        },
        _ => ObjectDdlStatus::Unavailable {
            reason: "Index definition is unavailable with current privileges".into(),
        },
    };

    let index_info = IndexInfo {
        name: object.name.clone(),
        method: row.get(3),
        unique: row.get(4),
        primary: row.get(5),
        expressions: row.get(6),
        predicate: row.get(7),
        definition,
    };

    let (mut uses, used_by) = relation_dependencies(client, idx_oid, &object).await?;
    let mut combined_uses = vec![DependencyEdge {
        object: DatabaseObjectRef {
            schema: requested.schema.clone(),
            name: table_name.clone(),
            kind: relkind_to_kind(&tbl_relkind).unwrap_or(DatabaseObjectKind::Table),
            identity: None,
        },
        relation: "indexed table".into(),
    }];
    combined_uses.append(&mut uses);

    Ok(ObjectDetails {
        uses: finalize_dependency_edges(&object, combined_uses, MAX_DEPENDENCY_EDGES),
        used_by,
        object,
        owner_table: Some(TableRef {
            schema: requested.schema.clone(),
            name: table_name,
        }),
        comment,
        ddl,
        indexes: vec![index_info],
        constraints: Vec::new(),
        triggers: Vec::new(),
        routine: None,
        sequence: None,
    })
}

async fn relation_dependencies(
    client: &Client,
    rel_oid: u32,
    self_object: &DatabaseObjectRef,
) -> AppResult<(Vec<DependencyEdge>, Vec<DependencyEdge>)> {
    let uses_rows = client
        .query(
            "SELECT tn.nspname::pg_catalog.text,
                    tc.relname::pg_catalog.text,
                    tc.relkind::pg_catalog.text,
                    NULL::pg_catalog.text,
                    'foreign key'::pg_catalog.text
             FROM pg_catalog.pg_constraint k
             JOIN pg_catalog.pg_class tc ON tc.oid = k.confrelid
             JOIN pg_catalog.pg_namespace tn ON tn.oid = tc.relnamespace
             WHERE k.conrelid = $1::pg_catalog.oid
               AND k.contype = 'f'
               AND pg_catalog.has_schema_privilege(tn.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    c.relname::pg_catalog.text,
                    c.relkind::pg_catalog.text,
                    CASE WHEN c.relkind IN ('i', 'I') THEN tbl.relname::pg_catalog.text ELSE NULL::pg_catalog.text END,
                    CASE
                      WHEN c.relkind = 'S' THEN 'sequence'
                      WHEN c.relkind IN ('v', 'm') THEN 'view'
                      ELSE 'relation'
                    END::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_class c ON c.oid = d.refobjid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             LEFT JOIN pg_catalog.pg_index ix ON ix.indexrelid = c.oid
             LEFT JOIN pg_catalog.pg_class tbl ON tbl.oid = ix.indrelid
             WHERE d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
               AND d.deptype IN ('n', 'a', 'i')
               AND c.relkind IN ('r', 'p', 'v', 'm', 'S')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
               AND (
                 (d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objid = $1::pg_catalog.oid)
                 OR (d.classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass AND d.objid IN (
                   SELECT r.oid FROM pg_catalog.pg_rewrite r WHERE r.ev_class = $1::pg_catalog.oid
                 ))
                 OR (d.classid = 'pg_catalog.pg_attrdef'::pg_catalog.regclass AND d.objid IN (
                   SELECT ad.oid FROM pg_catalog.pg_attrdef ad WHERE ad.adrelid = $1::pg_catalog.oid
                 ))
                 OR (d.classid = 'pg_catalog.pg_constraint'::pg_catalog.regclass AND d.objid IN (
                   SELECT kc.oid FROM pg_catalog.pg_constraint kc WHERE kc.conrelid = $1::pg_catalog.oid
                 ))
               )
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    p.proname::pg_catalog.text,
                    CASE WHEN p.prokind = 'p' THEN 'proc' ELSE 'func' END::pg_catalog.text,
                    pg_catalog.pg_get_function_identity_arguments(p.oid)::pg_catalog.text,
                    'routine'::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_proc p ON p.oid = d.refobjid
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
             WHERE d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.deptype IN ('n', 'a')
               AND p.prokind IN ('f', 'p')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
               AND (
                 (d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objid = $1::pg_catalog.oid)
                 OR (d.classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass AND d.objid IN (
                   SELECT r.oid FROM pg_catalog.pg_rewrite r WHERE r.ev_class = $1::pg_catalog.oid
                 ))
                 OR (d.classid = 'pg_catalog.pg_attrdef'::pg_catalog.regclass AND d.objid IN (
                   SELECT ad.oid FROM pg_catalog.pg_attrdef ad WHERE ad.adrelid = $1::pg_catalog.oid
                 ))
                 OR (d.classid = 'pg_catalog.pg_constraint'::pg_catalog.regclass AND d.objid IN (
                   SELECT kc.oid FROM pg_catalog.pg_constraint kc WHERE kc.conrelid = $1::pg_catalog.oid
                 ))
               )
             LIMIT 300",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;

    let used_by_rows = client
        .query(
            "SELECT sn.nspname::pg_catalog.text,
                    sc.relname::pg_catalog.text,
                    sc.relkind::pg_catalog.text,
                    NULL::pg_catalog.text,
                    'referenced by foreign key'::pg_catalog.text
             FROM pg_catalog.pg_constraint k
             JOIN pg_catalog.pg_class sc ON sc.oid = k.conrelid
             JOIN pg_catalog.pg_namespace sn ON sn.oid = sc.relnamespace
             WHERE k.confrelid = $1::pg_catalog.oid
               AND k.contype = 'f'
               AND pg_catalog.has_schema_privilege(sn.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    t.tgname::pg_catalog.text,
                    'trig'::pg_catalog.text,
                    tbl.relname::pg_catalog.text,
                    'trigger'::pg_catalog.text
             FROM pg_catalog.pg_trigger t
             JOIN pg_catalog.pg_class tbl ON tbl.oid = t.tgrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = tbl.relnamespace
             WHERE t.tgrelid = $1::pg_catalog.oid
               AND NOT t.tgisinternal
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    idx.relname::pg_catalog.text,
                    'i'::pg_catalog.text,
                    tbl.relname::pg_catalog.text,
                    'index'::pg_catalog.text
             FROM pg_catalog.pg_index i
             JOIN pg_catalog.pg_class idx ON idx.oid = i.indexrelid
             JOIN pg_catalog.pg_class tbl ON tbl.oid = i.indrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = idx.relnamespace
             WHERE i.indrelid = $1::pg_catalog.oid
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    vc.relname::pg_catalog.text,
                    vc.relkind::pg_catalog.text,
                    NULL::pg_catalog.text,
                    CASE WHEN vc.relkind = 'm' THEN 'materialized view' ELSE 'view' END::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_rewrite r ON r.oid = d.objid
             JOIN pg_catalog.pg_class vc ON vc.oid = r.ev_class
             JOIN pg_catalog.pg_namespace n ON n.oid = vc.relnamespace
             WHERE d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
               AND d.refobjid = $1::pg_catalog.oid
               AND d.classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass
               AND vc.relkind IN ('v', 'm')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    tc.relname::pg_catalog.text,
                    tc.relkind::pg_catalog.text,
                    NULL::pg_catalog.text,
                    'column default'::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_attrdef ad ON ad.oid = d.objid
             JOIN pg_catalog.pg_class tc ON tc.oid = ad.adrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace
             WHERE d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
               AND d.refobjid = $1::pg_catalog.oid
               AND d.classid = 'pg_catalog.pg_attrdef'::pg_catalog.regclass
               AND tc.relkind IN ('r', 'p')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    dc.relname::pg_catalog.text,
                    dc.relkind::pg_catalog.text,
                    CASE WHEN dc.relkind IN ('i', 'I') THEN tbl.relname::pg_catalog.text ELSE NULL::pg_catalog.text END,
                    'dependent relation'::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_class dc ON dc.oid = d.objid
             JOIN pg_catalog.pg_namespace n ON n.oid = dc.relnamespace
             LEFT JOIN pg_catalog.pg_index ix ON ix.indexrelid = dc.oid
             LEFT JOIN pg_catalog.pg_class tbl ON tbl.oid = ix.indrelid
             WHERE d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
               AND d.refobjid = $1::pg_catalog.oid
               AND d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass
               AND d.deptype IN ('n', 'a')
               AND dc.relkind IN ('r', 'p', 'v', 'm', 'S', 'i', 'I')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    p.proname::pg_catalog.text,
                    CASE WHEN p.prokind = 'p' THEN 'proc' ELSE 'func' END::pg_catalog.text,
                    pg_catalog.pg_get_function_identity_arguments(p.oid)::pg_catalog.text,
                    'routine'::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_proc p ON p.oid = d.objid
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
             WHERE d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
               AND d.refobjid = $1::pg_catalog.oid
               AND d.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.deptype IN ('n', 'a')
               AND p.prokind IN ('f', 'p')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             LIMIT 300",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;

    Ok((
        finalize_dependency_edges(
            self_object,
            uses_rows.into_iter().filter_map(decode_dep_row),
            MAX_DEPENDENCY_EDGES,
        ),
        finalize_dependency_edges(
            self_object,
            used_by_rows.into_iter().filter_map(decode_dep_row),
            MAX_DEPENDENCY_EDGES,
        ),
    ))
}

async fn routine_dependencies(
    client: &Client,
    proc_oid: u32,
    self_object: &DatabaseObjectRef,
) -> AppResult<(Vec<DependencyEdge>, Vec<DependencyEdge>)> {
    let uses_rows = client
        .query(
            "SELECT n.nspname::pg_catalog.text,
                    c.relname::pg_catalog.text,
                    c.relkind::pg_catalog.text,
                    NULL::pg_catalog.text,
                    'relation'::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_class c ON c.oid = d.refobjid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE d.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.objid = $1::pg_catalog.oid
               AND d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
               AND d.deptype IN ('n', 'a')
               AND c.relkind IN ('r', 'p', 'v', 'm', 'S')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    p.proname::pg_catalog.text,
                    CASE WHEN p.prokind = 'p' THEN 'proc' ELSE 'func' END::pg_catalog.text,
                    pg_catalog.pg_get_function_identity_arguments(p.oid)::pg_catalog.text,
                    'routine'::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_proc p ON p.oid = d.refobjid
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
             WHERE d.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.objid = $1::pg_catalog.oid
               AND d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.deptype IN ('n', 'a')
               AND p.prokind IN ('f', 'p')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             LIMIT 300",
            &[&proc_oid],
        )
        .await
        .map_err(pg_error)?;

    let used_by_rows = client
        .query(
            "SELECT n.nspname::pg_catalog.text,
                    t.tgname::pg_catalog.text,
                    'trig'::pg_catalog.text,
                    tbl.relname::pg_catalog.text,
                    'trigger'::pg_catalog.text
             FROM pg_catalog.pg_trigger t
             JOIN pg_catalog.pg_class tbl ON tbl.oid = t.tgrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = tbl.relnamespace
             WHERE t.tgfoid = $1::pg_catalog.oid
               AND NOT t.tgisinternal
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    vc.relname::pg_catalog.text,
                    vc.relkind::pg_catalog.text,
                    NULL::pg_catalog.text,
                    CASE WHEN vc.relkind = 'm' THEN 'materialized view' ELSE 'view' END::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_rewrite r ON r.oid = d.objid
             JOIN pg_catalog.pg_class vc ON vc.oid = r.ev_class
             JOIN pg_catalog.pg_namespace n ON n.oid = vc.relnamespace
             WHERE d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.refobjid = $1::pg_catalog.oid
               AND d.classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass
               AND vc.relkind IN ('v', 'm')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    tc.relname::pg_catalog.text,
                    tc.relkind::pg_catalog.text,
                    NULL::pg_catalog.text,
                    'column default'::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_attrdef ad ON ad.oid = d.objid
             JOIN pg_catalog.pg_class tc ON tc.oid = ad.adrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace
             WHERE d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.refobjid = $1::pg_catalog.oid
               AND d.classid = 'pg_catalog.pg_attrdef'::pg_catalog.regclass
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    tc.relname::pg_catalog.text,
                    tc.relkind::pg_catalog.text,
                    NULL::pg_catalog.text,
                    'constraint'::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_constraint kc ON kc.oid = d.objid
             JOIN pg_catalog.pg_class tc ON tc.oid = kc.conrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace
             WHERE d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.refobjid = $1::pg_catalog.oid
               AND d.classid = 'pg_catalog.pg_constraint'::pg_catalog.regclass
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    dc.relname::pg_catalog.text,
                    dc.relkind::pg_catalog.text,
                    CASE WHEN dc.relkind IN ('i', 'I') THEN tbl.relname::pg_catalog.text ELSE NULL::pg_catalog.text END,
                    CASE WHEN dc.relkind IN ('i', 'I') THEN 'expression index' ELSE 'relation' END::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_class dc ON dc.oid = d.objid
             JOIN pg_catalog.pg_namespace n ON n.oid = dc.relnamespace
             LEFT JOIN pg_catalog.pg_index ix ON ix.indexrelid = dc.oid
             LEFT JOIN pg_catalog.pg_class tbl ON tbl.oid = ix.indrelid
             WHERE d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.refobjid = $1::pg_catalog.oid
               AND d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass
               AND d.deptype IN ('n', 'a')
               AND dc.relkind IN ('r', 'p', 'v', 'm', 'i', 'I')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             UNION ALL
             SELECT n.nspname::pg_catalog.text,
                    p.proname::pg_catalog.text,
                    CASE WHEN p.prokind = 'p' THEN 'proc' ELSE 'func' END::pg_catalog.text,
                    pg_catalog.pg_get_function_identity_arguments(p.oid)::pg_catalog.text,
                    'routine'::pg_catalog.text
             FROM pg_catalog.pg_depend d
             JOIN pg_catalog.pg_proc p ON p.oid = d.objid
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
             WHERE d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.refobjid = $1::pg_catalog.oid
               AND d.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass
               AND d.deptype IN ('n', 'a')
               AND p.prokind IN ('f', 'p')
               AND n.nspname NOT LIKE 'pg\\_%'
               AND n.nspname <> 'information_schema'
               AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
             LIMIT 300",
            &[&proc_oid],
        )
        .await
        .map_err(pg_error)?;

    Ok((
        finalize_dependency_edges(
            self_object,
            uses_rows.into_iter().filter_map(decode_dep_row),
            MAX_DEPENDENCY_EDGES,
        ),
        finalize_dependency_edges(
            self_object,
            used_by_rows.into_iter().filter_map(decode_dep_row),
            MAX_DEPENDENCY_EDGES,
        ),
    ))
}

fn decode_dep_row(row: tokio_postgres::Row) -> Option<DependencyEdge> {
    let kind = match row.get::<_, &str>(2) {
        "func" => DatabaseObjectKind::Function,
        "proc" => DatabaseObjectKind::Procedure,
        "trig" => DatabaseObjectKind::Trigger,
        other => relkind_to_kind(other)?,
    };
    Some(DependencyEdge {
        object: DatabaseObjectRef {
            schema: row.get(0),
            name: row.get(1),
            kind,
            identity: row.get(3),
        },
        relation: row.get(4),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_bitmask_decodes_timing_events_and_orientation() {
        // BEFORE (2) | INSERT (4) | UPDATE (16) | ROW (1) = 23
        let (timing, events, orientation) = decode_pg_trigger_type(23);
        assert_eq!(timing, "BEFORE");
        assert_eq!(events, vec!["INSERT", "UPDATE"]);
        assert_eq!(orientation, "ROW");

        // INSTEAD OF (64) | DELETE (8) = 72 (STATEMENT)
        let (timing, events, orientation) = decode_pg_trigger_type(72);
        assert_eq!(timing, "INSTEAD OF");
        assert_eq!(events, vec!["DELETE"]);
        assert_eq!(orientation, "STATEMENT");

        // AFTER (0) | TRUNCATE (32) = 32 (STATEMENT)
        let (timing, events, orientation) = decode_pg_trigger_type(32);
        assert_eq!(timing, "AFTER");
        assert_eq!(events, vec!["TRUNCATE"]);
        assert_eq!(orientation, "STATEMENT");
    }

    #[test]
    fn table_and_sequence_ddl_synthesis_quotes_identifiers_deterministically() {
        let ddl = synthesize_pg_table_ddl(
            "public",
            "Order \"Items\"",
            &[
                PgTableColumnSpec {
                    name: "id".into(),
                    formatted_type: "bigint".into(),
                    not_null: true,
                    identity: "a".into(),
                    generated: String::new(),
                    default_expr: None,
                },
                PgTableColumnSpec {
                    name: "price\"usd".into(),
                    formatted_type: "numeric(12,2)".into(),
                    not_null: true,
                    identity: String::new(),
                    generated: String::new(),
                    default_expr: Some("0.00".into()),
                },
                PgTableColumnSpec {
                    name: "gross".into(),
                    formatted_type: "numeric(12,2)".into(),
                    not_null: false,
                    identity: String::new(),
                    generated: "s".into(),
                    default_expr: Some("\"price\"\"usd\" * 1.1".into()),
                },
            ],
            &[ConstraintInfo {
                name: "order_items_pkey".into(),
                kind: ConstraintKind::PrimaryKey,
                columns: vec!["id".into()],
                target: None,
                target_columns: Vec::new(),
                definition: "PRIMARY KEY (id)".into(),
            }],
            None,
            &["CREATE INDEX idx_items_price ON public.\"Order \"\"Items\"\"\" USING btree (\"price\"\"usd\")".into()],
            &[],
        );
        assert!(ddl.contains("CREATE TABLE \"public\".\"Order \"\"Items\"\"\" ("));
        assert!(ddl.contains("\"id\" bigint GENERATED ALWAYS AS IDENTITY NOT NULL"));
        assert!(ddl.contains("\"price\"\"usd\" numeric(12,2) DEFAULT 0.00 NOT NULL"));
        assert!(ddl.contains(
            "\"gross\" numeric(12,2) GENERATED ALWAYS AS (\"price\"\"usd\" * 1.1) STORED"
        ));
        assert!(ddl.contains("CONSTRAINT \"order_items_pkey\" PRIMARY KEY (id)"));
        assert!(ddl.contains("CREATE INDEX idx_items_price"));

        let seq_ddl = synthesize_pg_sequence_ddl(
            "public",
            "seq\"1",
            &SequenceInfo {
                data_type: "bigint".into(),
                start_value: "10".into(),
                min_value: "1".into(),
                max_value: "9999".into(),
                increment_by: "5".into(),
                cache_size: "1".into(),
                cycle: true,
                last_value: Some("15".into()),
            },
        );
        assert!(seq_ddl.contains("CREATE SEQUENCE \"public\".\"seq\"\"1\""));
        assert!(seq_ddl.contains("INCREMENT BY 5"));
        assert!(seq_ddl.ends_with("CYCLE;"));
    }
}
