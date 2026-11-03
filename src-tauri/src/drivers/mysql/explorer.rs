use super::values::{identifier, quote};
use super::*;
use std::collections::HashMap;

const MAX_DISCOVERED_OBJECTS: usize = 10_000;
const MAX_DEPENDENCY_EDGES: usize = 200;

impl Mysql {
    pub(super) async fn explorer_objects(&self) -> AppResult<Vec<DatabaseObjectSummary>> {
        let maria = self.maria;
        self.metadata(move |conn| Box::pin(async move { database_objects_on(conn, maria).await }))
            .await
    }

    pub(super) async fn explorer_object_details(
        &self,
        object: &DatabaseObjectRef,
    ) -> AppResult<ObjectDetails> {
        identifier(&object.schema)?;
        identifier(&object.name)?;
        if let Some(id) = &object.identity
            && matches!(
                object.kind,
                DatabaseObjectKind::Trigger | DatabaseObjectKind::Index
            )
            && !id.is_empty()
        {
            identifier(id)?;
        }
        let maria = self.maria;
        let requested = object.clone();
        self.metadata(move |conn| {
            Box::pin(async move { object_details_on(conn, &requested, maria).await })
        })
        .await
    }

    pub(super) async fn explorer_schema_snapshot(&self, schema: &str) -> AppResult<SchemaSnapshot> {
        identifier(schema)?;
        let maria = self.maria;
        let schema = schema.to_owned();
        self.metadata(move |conn| {
            Box::pin(async move { schema_snapshot_on(conn, &schema, maria).await })
        })
        .await
    }
}

async fn database_objects_on(
    conn: &mut Conn,
    maria: bool,
) -> AppResult<Vec<DatabaseObjectSummary>> {
    let mut items = Vec::new();

    // 1. Routines & ordered parameter types
    let routines = fetch(
        conn,
        "SELECT ROUTINE_SCHEMA, ROUTINE_NAME, ROUTINE_TYPE, SPECIFIC_NAME, COALESCE(DTD_IDENTIFIER, '')
         FROM INFORMATION_SCHEMA.ROUTINES
         WHERE ROUTINE_SCHEMA = DATABASE()
           AND ROUTINE_TYPE IN ('FUNCTION', 'PROCEDURE')
         ORDER BY ROUTINE_NAME, ROUTINE_TYPE
         LIMIT 5000",
        vec![],
        5000,
        4 * 1024 * 1024,
    )
    .await?;

    if !routines.is_empty() {
        let param_rows = fetch(
            conn,
            "SELECT SPECIFIC_NAME, ROUTINE_TYPE, COALESCE(DTD_IDENTIFIER, '')
             FROM INFORMATION_SCHEMA.PARAMETERS
             WHERE SPECIFIC_SCHEMA = DATABASE()
               AND ORDINAL_POSITION > 0
             ORDER BY SPECIFIC_NAME, ROUTINE_TYPE, ORDINAL_POSITION
             LIMIT 10000",
            vec![],
            10_000,
            4 * 1024 * 1024,
        )
        .await
        .unwrap_or_default();

        let mut params_by_routine: HashMap<(String, String), Vec<String>> = HashMap::new();
        for pr in &param_rows {
            params_by_routine
                .entry((cell(pr, 0).to_owned(), cell(pr, 1).to_owned()))
                .or_default()
                .push(cell(pr, 2).to_owned());
        }

        for r in routines {
            let schema = cell(&r, 0).to_owned();
            let name = cell(&r, 1).to_owned();
            let rtype = cell(&r, 2);
            let specific = cell(&r, 3).to_owned();
            let ret_type = cell(&r, 4);
            let kind = if rtype.eq_ignore_ascii_case("PROCEDURE") {
                DatabaseObjectKind::Procedure
            } else {
                DatabaseObjectKind::Function
            };
            let arg_types = params_by_routine
                .get(&(specific, rtype.to_owned()))
                .map(|v| v.join(", "))
                .unwrap_or_default();
            let signature = if kind == DatabaseObjectKind::Procedure {
                format!("({arg_types})")
            } else {
                format!(
                    "({arg_types}) -> {}",
                    if ret_type.is_empty() {
                        "unknown"
                    } else {
                        ret_type
                    }
                )
            };
            items.push(DatabaseObjectSummary {
                object: DatabaseObjectRef {
                    schema,
                    name,
                    kind,
                    identity: None,
                },
                owner_table: None,
                signature: Some(signature),
                estimated_rows: None,
                total_bytes: None,
            });
        }
    }

    // 2. MariaDB sequences (TABLE_TYPE = 'SEQUENCE')
    if maria && items.len() < MAX_DISCOVERED_OBJECTS {
        let seq_rows = fetch(
            conn,
            "SELECT TABLE_SCHEMA, TABLE_NAME, DATA_LENGTH + INDEX_LENGTH
             FROM INFORMATION_SCHEMA.TABLES
             WHERE TABLE_SCHEMA = DATABASE()
               AND TABLE_TYPE = 'SEQUENCE'
             ORDER BY TABLE_NAME
             LIMIT 2000",
            vec![],
            2000,
            2 * 1024 * 1024,
        )
        .await
        .unwrap_or_default();

        for r in seq_rows {
            if items.len() >= MAX_DISCOVERED_OBJECTS {
                break;
            }
            items.push(DatabaseObjectSummary {
                object: DatabaseObjectRef {
                    schema: cell(&r, 0).into(),
                    name: cell(&r, 1).into(),
                    kind: DatabaseObjectKind::Sequence,
                    identity: None,
                },
                owner_table: None,
                signature: Some("bigint".into()),
                estimated_rows: None,
                total_bytes: r[2]
                    .as_ref()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0.0),
            });
        }
    }

    // 3. Triggers
    if items.len() < MAX_DISCOVERED_OBJECTS {
        let trig_rows = fetch(
            conn,
            "SELECT TRIGGER_SCHEMA, TRIGGER_NAME, EVENT_OBJECT_TABLE, ACTION_TIMING, EVENT_MANIPULATION
             FROM INFORMATION_SCHEMA.TRIGGERS
             WHERE TRIGGER_SCHEMA = DATABASE()
             ORDER BY TRIGGER_NAME
             LIMIT 5000",
            vec![],
            5000,
            2 * 1024 * 1024,
        )
        .await
        .unwrap_or_default();

        for r in trig_rows {
            if items.len() >= MAX_DISCOVERED_OBJECTS {
                break;
            }
            let owner = cell(&r, 2).to_owned();
            items.push(DatabaseObjectSummary {
                object: DatabaseObjectRef {
                    schema: cell(&r, 0).into(),
                    name: cell(&r, 1).into(),
                    kind: DatabaseObjectKind::Trigger,
                    identity: Some(owner.clone()),
                },
                owner_table: Some(owner),
                signature: Some(format!("{} {}", cell(&r, 3), cell(&r, 4))),
                estimated_rows: None,
                total_bytes: None,
            });
        }
    }

    // 4. Indexes
    if items.len() < MAX_DISCOVERED_OBJECTS {
        let idx_rows = fetch(
            conn,
            "SELECT TABLE_SCHEMA, TABLE_NAME, INDEX_NAME, INDEX_TYPE
             FROM INFORMATION_SCHEMA.STATISTICS
             WHERE TABLE_SCHEMA = DATABASE()
             GROUP BY TABLE_SCHEMA, TABLE_NAME, INDEX_NAME, INDEX_TYPE
             ORDER BY TABLE_NAME, INDEX_NAME
             LIMIT 5000",
            vec![],
            5000,
            2 * 1024 * 1024,
        )
        .await
        .unwrap_or_default();

        for r in idx_rows {
            if items.len() >= MAX_DISCOVERED_OBJECTS {
                break;
            }
            let owner = cell(&r, 1).to_owned();
            items.push(DatabaseObjectSummary {
                object: DatabaseObjectRef {
                    schema: cell(&r, 0).into(),
                    name: cell(&r, 2).into(),
                    kind: DatabaseObjectKind::Index,
                    identity: Some(owner.clone()),
                },
                owner_table: Some(owner),
                signature: Some(cell(&r, 3).to_ascii_lowercase()),
                estimated_rows: None,
                total_bytes: None,
            });
        }
    }

    Ok(items)
}

async fn object_details_on(
    conn: &mut Conn,
    requested: &DatabaseObjectRef,
    maria: bool,
) -> AppResult<ObjectDetails> {
    match requested.kind {
        DatabaseObjectKind::Table
        | DatabaseObjectKind::PartitionedTable
        | DatabaseObjectKind::View
        | DatabaseObjectKind::MaterializedView => relation_details_on(conn, requested, maria).await,
        DatabaseObjectKind::Function | DatabaseObjectKind::Procedure => {
            routine_details_on(conn, requested).await
        }
        DatabaseObjectKind::Sequence => sequence_details_on(conn, requested, maria).await,
        DatabaseObjectKind::Trigger => trigger_details_on(conn, requested).await,
        DatabaseObjectKind::Index => index_details_on(conn, requested, maria).await,
    }
}

async fn show_create_ddl(conn: &mut Conn, sql: &str, col_index: usize) -> ObjectDdlStatus {
    match fetch_text(conn, sql, 4, 2 * 1024 * 1024).await {
        Ok(rows) => match rows
            .first()
            .and_then(|r| r.get(col_index))
            .and_then(Clone::clone)
        {
            Some(ddl) if !ddl.trim().is_empty() => ObjectDdlStatus::Available {
                ddl: ensure_semicolon(&ddl),
                truncated: false,
            },
            _ => ObjectDdlStatus::Unavailable {
                reason: "DDL definition is unavailable with current privileges".into(),
            },
        },
        Err(err) if matches!(err.code, ErrorCode::PermissionDenied | ErrorCode::Database) => {
            ObjectDdlStatus::Unavailable {
                reason: err.message,
            }
        }
        Err(err) => ObjectDdlStatus::Unavailable {
            reason: err.message,
        },
    }
}

fn ensure_semicolon(sql: &str) -> String {
    let trimmed = sql.trim();
    if trimmed.ends_with(';') {
        trimmed.to_owned()
    } else {
        format!("{trimmed};")
    }
}

pub(crate) fn format_mysql_index_ddl(schema: &str, table: &str, index: &IndexInfo) -> String {
    let keys = index.expressions.join(", ");
    if index.primary {
        format!(
            "ALTER TABLE {}.{} ADD PRIMARY KEY ({keys});",
            quote(schema),
            quote(table)
        )
    } else {
        let unique = if index.unique { "UNIQUE " } else { "" };
        format!(
            "CREATE {unique}INDEX {} ON {}.{} ({keys}) USING {};",
            quote(&index.name),
            quote(schema),
            quote(table),
            index.method.to_ascii_uppercase()
        )
    }
}

async fn relation_details_on(
    conn: &mut Conn,
    requested: &DatabaseObjectRef,
    maria: bool,
) -> AppResult<ObjectDetails> {
    let meta = fetch(
        conn,
        "SELECT TABLE_TYPE,
                COALESCE(TABLE_COMMENT, ''),
                TABLE_ROWS,
                DATA_LENGTH,
                INDEX_LENGTH,
                DATA_FREE,
                COALESCE(DATE_FORMAT(UPDATE_TIME, '%Y-%m-%dT%H:%i:%sZ'), '')
         FROM INFORMATION_SCHEMA.TABLES
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        vec![
            bind(Some(requested.schema.clone())),
            bind(Some(requested.name.clone())),
        ],
        1,
        16 * 1024,
    )
    .await?;
    let row = meta.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The table or view is missing or its metadata is unavailable",
        )
    })?;
    let table_type = cell(row, 0);
    if table_type == "SEQUENCE" {
        return sequence_details_on(conn, requested, maria).await;
    }
    let comment = Some(cell(row, 1).to_owned()).filter(|c| !c.is_empty());
    let estimated_rows = non_negative_f64(row, 2);
    let table_bytes = non_negative_f64(row, 3);
    let index_bytes = non_negative_f64(row, 4);
    let free_bytes = non_negative_f64(row, 5);
    let update_time = Some(cell(row, 6).to_owned()).filter(|s| !s.is_empty());
    let kind = if table_type.contains("VIEW") {
        DatabaseObjectKind::View
    } else {
        DatabaseObjectKind::Table
    };
    let object = DatabaseObjectRef {
        schema: requested.schema.clone(),
        name: requested.name.clone(),
        kind,
        identity: None,
    };

    let show_sql = if kind == DatabaseObjectKind::View {
        format!(
            "SHOW CREATE VIEW {}.{}",
            quote(&object.schema),
            quote(&object.name)
        )
    } else {
        format!(
            "SHOW CREATE TABLE {}.{}",
            quote(&object.schema),
            quote(&object.name)
        )
    };
    let ddl = show_create_ddl(conn, &show_sql, 1).await;

    let (indexes, idx_meta) =
        fetch_mysql_indexes(conn, &object.schema, &object.name, maria).await?;
    let constraints = fetch_mysql_constraints(conn, &object.schema, &object.name, maria).await?;
    let triggers = fetch_mysql_triggers(conn, &object.schema, Some(&object.name)).await?;

    let storage = (kind == DatabaseObjectKind::Table).then(|| {
        let total_bytes = match (table_bytes, index_bytes) {
            (Some(d), Some(i)) => Some(d + i),
            (Some(d), None) => Some(d),
            (None, Some(i)) => Some(i),
            (None, None) => None,
        };
        TableStorageStats {
            estimated_rows,
            table_bytes,
            index_bytes,
            free_bytes,
            total_bytes,
            live_tuples: None,
            dead_tuples: None,
            sequential_scans: idx_meta.sequential_scans,
            index_scans: idx_meta.index_scans,
            last_analyze: update_time,
            last_vacuum: None,
            stats_stale: estimated_rows.is_none(),
            restricted_reason: idx_meta.restricted_reason,
        }
    });

    let (uses, used_by) =
        relation_dependencies_on(conn, &object, &indexes, &constraints, &triggers, maria).await?;

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
        storage,
        uses,
        used_by,
    })
}

#[derive(Default)]
struct MysqlIndexStatsMeta {
    sequential_scans: Option<f64>,
    index_scans: Option<f64>,
    restricted_reason: Option<String>,
}

fn non_negative_f64(row: &[Option<String>], index: usize) -> Option<f64> {
    row.get(index)
        .and_then(|v| v.as_deref())
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
}

async fn fetch_mysql_indexes(
    conn: &mut Conn,
    schema: &str,
    table: &str,
    maria: bool,
) -> AppResult<(Vec<IndexInfo>, MysqlIndexStatsMeta)> {
    let sql = if maria {
        "SELECT INDEX_NAME, INDEX_TYPE, NON_UNIQUE, COLUMN_NAME, SUB_PART, NULL, CARDINALITY
         FROM INFORMATION_SCHEMA.STATISTICS
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
         ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX
         LIMIT 2000"
    } else {
        "SELECT INDEX_NAME, INDEX_TYPE, NON_UNIQUE, COLUMN_NAME, SUB_PART, EXPRESSION, CARDINALITY
         FROM INFORMATION_SCHEMA.STATISTICS
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
         ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX
         LIMIT 2000"
    };
    let rows = fetch(
        conn,
        sql,
        vec![bind(Some(schema.into())), bind(Some(table.into()))],
        2000,
        2 * 1024 * 1024,
    )
    .await?;

    let mut indexes: Vec<IndexInfo> = Vec::new();
    for r in rows {
        let name = cell(&r, 0).to_owned();
        let method = cell(&r, 1).to_ascii_lowercase();
        let unique = cell(&r, 2) == "0";
        let primary = name == "PRIMARY";
        let col_name = cell(&r, 3);
        let sub_part = cell(&r, 4);
        let expr_text = cell(&r, 5);
        let cardinality = non_negative_f64(&r, 6);
        let key_expr = if !col_name.is_empty() {
            if !sub_part.is_empty() {
                format!("{}({sub_part})", quote(col_name))
            } else {
                quote(col_name)
            }
        } else if !expr_text.is_empty() {
            format!("({expr_text})")
        } else {
            continue;
        };

        match indexes.last_mut() {
            Some(last) if last.name == name => {
                last.expressions.push(key_expr);
                if let Some(card) = cardinality {
                    last.cardinality = Some(last.cardinality.map_or(card, |prev| prev.max(card)));
                }
            }
            _ => {
                indexes.push(IndexInfo {
                    name,
                    method,
                    unique,
                    primary,
                    expressions: vec![key_expr],
                    predicate: None,
                    definition: None,
                    size_bytes: None,
                    scans: None,
                    tuples_read: None,
                    tuples_fetched: None,
                    cardinality,
                });
            }
        }
    }

    let mut meta = MysqlIndexStatsMeta::default();
    let mut restricted_parts = Vec::new();

    if !indexes.is_empty() {
        match fetch(
            conn,
            "SELECT index_name, stat_value * @@innodb_page_size
             FROM mysql.innodb_index_stats
             WHERE database_name = ? AND table_name = ? AND stat_name = 'size'
             LIMIT 500",
            vec![bind(Some(schema.into())), bind(Some(table.into()))],
            500,
            256 * 1024,
        )
        .await
        {
            Ok(size_rows) => {
                let mut sizes: HashMap<String, f64> = HashMap::new();
                for r in size_rows {
                    if let Some(bytes) = non_negative_f64(&r, 1) {
                        sizes.insert(cell(&r, 0).to_owned(), bytes);
                    }
                }
                for idx in &mut indexes {
                    idx.size_bytes = sizes.get(&idx.name).copied();
                }
            }
            Err(err) if err.code == ErrorCode::PermissionDenied => {
                restricted_parts.push("per-index byte sizes (SELECT on mysql.innodb_index_stats)");
            }
            Err(_) => {}
        }

        let perf_on = fetch(conn, "SELECT @@performance_schema", vec![], 1, 1024)
            .await
            .ok()
            .and_then(|rows| rows.first().map(|r| cell(r, 0).to_owned()))
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("ON"));

        if perf_on {
            match fetch(
                conn,
                "SELECT COALESCE(INDEX_NAME, ''), COUNT_READ
                 FROM performance_schema.table_io_waits_summary_by_index_usage
                 WHERE OBJECT_SCHEMA = ? AND OBJECT_NAME = ?
                 LIMIT 500",
                vec![bind(Some(schema.into())), bind(Some(table.into()))],
                500,
                256 * 1024,
            )
            .await
            {
                Ok(usage_rows) => {
                    let mut reads_by_index: HashMap<String, f64> = HashMap::new();
                    let mut seq_reads = 0.0;
                    for r in usage_rows {
                        let idx_name = cell(&r, 0);
                        let reads = non_negative_f64(&r, 1).unwrap_or(0.0);
                        if idx_name.is_empty() {
                            seq_reads += reads;
                        } else {
                            *reads_by_index.entry(idx_name.to_owned()).or_insert(0.0) += reads;
                        }
                    }
                    let mut total_idx_reads = 0.0;
                    for idx in &mut indexes {
                        let reads = reads_by_index.get(&idx.name).copied().unwrap_or(0.0);
                        idx.scans = Some(reads);
                        idx.tuples_read = Some(reads);
                        total_idx_reads += reads;
                    }
                    meta.sequential_scans = Some(seq_reads);
                    meta.index_scans = Some(total_idx_reads);
                }
                Err(err) if err.code == ErrorCode::PermissionDenied => {
                    restricted_parts.push(
                        "index scan counters (SELECT on performance_schema.table_io_waits_summary_by_index_usage)",
                    );
                }
                Err(_) => {}
            }
        } else {
            restricted_parts.push("index scan counters (performance_schema is disabled)");
        }
    }

    if !restricted_parts.is_empty() {
        meta.restricted_reason = Some(format!(
            "Unavailable with current server configuration or privileges: {}.",
            restricted_parts.join("; ")
        ));
    }

    for idx in &mut indexes {
        idx.definition = Some(format_mysql_index_ddl(schema, table, idx));
    }
    Ok((indexes, meta))
}

async fn fetch_mysql_constraints(
    conn: &mut Conn,
    schema: &str,
    table: &str,
    maria: bool,
) -> AppResult<Vec<ConstraintInfo>> {
    let params = || vec![bind(Some(schema.into())), bind(Some(table.into()))];
    let tc_rows = fetch(
        conn,
        "SELECT CONSTRAINT_NAME, CONSTRAINT_TYPE
         FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
         ORDER BY CASE CONSTRAINT_TYPE
                    WHEN 'PRIMARY KEY' THEN 1
                    WHEN 'FOREIGN KEY' THEN 2
                    WHEN 'UNIQUE' THEN 3
                    WHEN 'CHECK' THEN 4
                    ELSE 5
                  END,
                  CONSTRAINT_NAME
         LIMIT 1000",
        params(),
        1000,
        1024 * 1024,
    )
    .await?;

    let kcu_rows = fetch(
        conn,
        "SELECT CONSTRAINT_NAME, COLUMN_NAME, REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
         ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION
         LIMIT 2000",
        params(),
        2000,
        2 * 1024 * 1024,
    )
    .await?;

    let check_sql = if maria {
        "SELECT tc.CONSTRAINT_NAME, cc.CHECK_CLAUSE
         FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
         JOIN INFORMATION_SCHEMA.CHECK_CONSTRAINTS cc
           ON cc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA
          AND cc.TABLE_NAME = tc.TABLE_NAME
          AND cc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME
         WHERE tc.TABLE_SCHEMA = ? AND tc.TABLE_NAME = ? AND tc.CONSTRAINT_TYPE = 'CHECK'
         ORDER BY tc.CONSTRAINT_NAME
         LIMIT 500"
    } else {
        "SELECT tc.CONSTRAINT_NAME, cc.CHECK_CLAUSE
         FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
         JOIN INFORMATION_SCHEMA.CHECK_CONSTRAINTS cc
           ON cc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA
          AND cc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME
         WHERE tc.TABLE_SCHEMA = ? AND tc.TABLE_NAME = ? AND tc.CONSTRAINT_TYPE = 'CHECK'
         ORDER BY tc.CONSTRAINT_NAME
         LIMIT 500"
    };
    let check_rows = fetch(conn, check_sql, params(), 500, 1024 * 1024)
        .await
        .unwrap_or_default();

    let mut check_map: HashMap<String, String> = HashMap::new();
    for cr in &check_rows {
        check_map.insert(cell(cr, 0).into(), cell(cr, 1).into());
    }

    type KeyUsageEntry = (Vec<String>, Option<TableRef>, Vec<String>);
    let mut kcu_map: HashMap<String, KeyUsageEntry> = HashMap::new();
    for kr in &kcu_rows {
        let cname = cell(kr, 0).to_owned();
        let entry = kcu_map
            .entry(cname)
            .or_insert_with(|| (Vec::new(), None, Vec::new()));
        let col = cell(kr, 1);
        if !col.is_empty() {
            entry.0.push(col.to_owned());
        }
        let ref_schema = cell(kr, 2);
        let ref_table = cell(kr, 3);
        let ref_col = cell(kr, 4);
        if !ref_table.is_empty() {
            entry.1 = Some(TableRef {
                schema: if ref_schema.is_empty() {
                    schema.into()
                } else {
                    ref_schema.into()
                },
                name: ref_table.into(),
            });
            if !ref_col.is_empty() {
                entry.2.push(ref_col.to_owned());
            }
        }
    }

    let mut constraints = Vec::new();
    for tr in tc_rows {
        let cname = cell(&tr, 0).to_owned();
        let ctype = cell(&tr, 1);
        let kind = match ctype {
            "PRIMARY KEY" => ConstraintKind::PrimaryKey,
            "FOREIGN KEY" => ConstraintKind::ForeignKey,
            "UNIQUE" => ConstraintKind::Unique,
            "CHECK" => ConstraintKind::Check,
            _ => continue,
        };
        let (columns, target, target_columns) = kcu_map
            .remove(&cname)
            .unwrap_or_else(|| (Vec::new(), None, Vec::new()));
        let definition = match kind {
            ConstraintKind::PrimaryKey => format!("PRIMARY KEY ({})", columns.join(", ")),
            ConstraintKind::Unique => format!("UNIQUE ({})", columns.join(", ")),
            ConstraintKind::ForeignKey => {
                if let Some(t) = &target {
                    format!(
                        "FOREIGN KEY ({}) REFERENCES {}.{} ({})",
                        columns.join(", "),
                        quote(&t.schema),
                        quote(&t.name),
                        target_columns.join(", ")
                    )
                } else {
                    format!("FOREIGN KEY ({})", columns.join(", "))
                }
            }
            ConstraintKind::Check => match check_map.get(&cname) {
                Some(clause) if !clause.is_empty() => format!("CHECK ({clause})"),
                _ => "CHECK (...)".into(),
            },
            ConstraintKind::Exclusion => continue,
        };
        constraints.push(ConstraintInfo {
            name: cname,
            kind,
            columns,
            target,
            target_columns,
            definition,
        });
    }
    Ok(constraints)
}

async fn fetch_mysql_triggers(
    conn: &mut Conn,
    schema: &str,
    table: Option<&str>,
) -> AppResult<Vec<TriggerInfo>> {
    let (sql, params) = if let Some(tbl) = table {
        (
            "SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE, ACTION_TIMING, EVENT_MANIPULATION, ACTION_ORIENTATION, ACTION_STATEMENT
             FROM INFORMATION_SCHEMA.TRIGGERS
             WHERE TRIGGER_SCHEMA = ? AND EVENT_OBJECT_TABLE = ?
             ORDER BY TRIGGER_NAME
             LIMIT 500",
            vec![bind(Some(schema.into())), bind(Some(tbl.into()))],
        )
    } else {
        (
            "SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE, ACTION_TIMING, EVENT_MANIPULATION, ACTION_ORIENTATION, ACTION_STATEMENT
             FROM INFORMATION_SCHEMA.TRIGGERS
             WHERE TRIGGER_SCHEMA = ?
             ORDER BY TRIGGER_NAME
             LIMIT 500",
            vec![bind(Some(schema.into()))],
        )
    };
    let rows = fetch(conn, sql, params, 500, 2 * 1024 * 1024).await?;
    Ok(rows
        .into_iter()
        .map(|r| TriggerInfo {
            name: cell(&r, 0).into(),
            table_name: cell(&r, 1).into(),
            timing: cell(&r, 2).into(),
            events: vec![cell(&r, 3).into()],
            orientation: cell(&r, 4).into(),
            enabled: true,
            definition: Some(cell(&r, 5).to_owned()).filter(|s| !s.is_empty()),
        })
        .collect())
}

async fn relation_dependencies_on(
    conn: &mut Conn,
    self_object: &DatabaseObjectRef,
    indexes: &[IndexInfo],
    constraints: &[ConstraintInfo],
    triggers: &[TriggerInfo],
    maria: bool,
) -> AppResult<(Vec<DependencyEdge>, Vec<DependencyEdge>)> {
    let mut raw_uses = Vec::new();
    let mut raw_used_by = Vec::new();

    for c in constraints {
        if c.kind == ConstraintKind::ForeignKey
            && let Some(target) = &c.target
        {
            raw_uses.push(DependencyEdge {
                object: DatabaseObjectRef {
                    schema: target.schema.clone(),
                    name: target.name.clone(),
                    kind: DatabaseObjectKind::Table,
                    identity: None,
                },
                relation: "foreign key".into(),
            });
        }
    }

    let inbound_fk = fetch(
        conn,
        "SELECT DISTINCT TABLE_SCHEMA, TABLE_NAME
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE
         WHERE REFERENCED_TABLE_SCHEMA = ? AND REFERENCED_TABLE_NAME = ?
         ORDER BY TABLE_SCHEMA, TABLE_NAME
         LIMIT 200",
        vec![
            bind(Some(self_object.schema.clone())),
            bind(Some(self_object.name.clone())),
        ],
        200,
        512 * 1024,
    )
    .await
    .unwrap_or_default();

    for r in inbound_fk {
        raw_used_by.push(DependencyEdge {
            object: DatabaseObjectRef {
                schema: cell(&r, 0).into(),
                name: cell(&r, 1).into(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
            relation: "referenced by foreign key".into(),
        });
    }

    for trig in triggers {
        raw_used_by.push(DependencyEdge {
            object: DatabaseObjectRef {
                schema: self_object.schema.clone(),
                name: trig.name.clone(),
                kind: DatabaseObjectKind::Trigger,
                identity: Some(self_object.name.clone()),
            },
            relation: "trigger".into(),
        });
    }

    for idx in indexes {
        raw_used_by.push(DependencyEdge {
            object: DatabaseObjectRef {
                schema: self_object.schema.clone(),
                name: idx.name.clone(),
                kind: DatabaseObjectKind::Index,
                identity: Some(self_object.name.clone()),
            },
            relation: "index".into(),
        });
    }

    if !maria {
        if self_object.kind == DatabaseObjectKind::View {
            let view_sources = fetch(
                conn,
                "SELECT vtu.TABLE_SCHEMA, vtu.TABLE_NAME, COALESCE(t.TABLE_TYPE, 'BASE TABLE')
                 FROM INFORMATION_SCHEMA.VIEW_TABLE_USAGE vtu
                 LEFT JOIN INFORMATION_SCHEMA.TABLES t
                   ON t.TABLE_SCHEMA = vtu.TABLE_SCHEMA AND t.TABLE_NAME = vtu.TABLE_NAME
                 WHERE vtu.VIEW_SCHEMA = ? AND vtu.VIEW_NAME = ?
                 ORDER BY vtu.TABLE_SCHEMA, vtu.TABLE_NAME
                 LIMIT 200",
                vec![
                    bind(Some(self_object.schema.clone())),
                    bind(Some(self_object.name.clone())),
                ],
                200,
                512 * 1024,
            )
            .await
            .unwrap_or_default();

            for r in view_sources {
                let kind = if cell(&r, 2).contains("VIEW") {
                    DatabaseObjectKind::View
                } else {
                    DatabaseObjectKind::Table
                };
                raw_uses.push(DependencyEdge {
                    object: DatabaseObjectRef {
                        schema: cell(&r, 0).into(),
                        name: cell(&r, 1).into(),
                        kind,
                        identity: None,
                    },
                    relation: if kind == DatabaseObjectKind::View {
                        "view".into()
                    } else {
                        "table".into()
                    },
                });
            }
        }

        let dependent_views = fetch(
            conn,
            "SELECT VIEW_SCHEMA, VIEW_NAME
             FROM INFORMATION_SCHEMA.VIEW_TABLE_USAGE
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
             ORDER BY VIEW_SCHEMA, VIEW_NAME
             LIMIT 200",
            vec![
                bind(Some(self_object.schema.clone())),
                bind(Some(self_object.name.clone())),
            ],
            200,
            512 * 1024,
        )
        .await
        .unwrap_or_default();

        for r in dependent_views {
            raw_used_by.push(DependencyEdge {
                object: DatabaseObjectRef {
                    schema: cell(&r, 0).into(),
                    name: cell(&r, 1).into(),
                    kind: DatabaseObjectKind::View,
                    identity: None,
                },
                relation: "view".into(),
            });
        }
    } else {
        // MariaDB lacks VIEW_TABLE_USAGE; match quoted relation identifiers in INFORMATION_SCHEMA.VIEWS.
        let views = fetch(
            conn,
            "SELECT TABLE_NAME, COALESCE(VIEW_DEFINITION, '')
             FROM INFORMATION_SCHEMA.VIEWS
             WHERE TABLE_SCHEMA = ?
             ORDER BY TABLE_NAME
             LIMIT 500",
            vec![bind(Some(self_object.schema.clone()))],
            500,
            2 * 1024 * 1024,
        )
        .await
        .unwrap_or_default();

        let self_quoted = quote(&self_object.name);
        for vr in &views {
            let vname = cell(vr, 0);
            let vdef = cell(vr, 1);
            if vname != self_object.name && vdef.contains(&self_quoted) {
                raw_used_by.push(DependencyEdge {
                    object: DatabaseObjectRef {
                        schema: self_object.schema.clone(),
                        name: vname.into(),
                        kind: DatabaseObjectKind::View,
                        identity: None,
                    },
                    relation: "view".into(),
                });
            }
        }
        if self_object.kind == DatabaseObjectKind::View
            && let Some(self_vdef) = views
                .iter()
                .find(|vr| cell(vr, 0) == self_object.name)
                .map(|vr| cell(vr, 1))
            && !self_vdef.is_empty()
        {
            let siblings = fetch(
                conn,
                "SELECT TABLE_NAME, TABLE_TYPE
                 FROM INFORMATION_SCHEMA.TABLES
                 WHERE TABLE_SCHEMA = ? AND TABLE_TYPE <> 'SEQUENCE'
                 ORDER BY TABLE_NAME
                 LIMIT 1000",
                vec![bind(Some(self_object.schema.clone()))],
                1000,
                1024 * 1024,
            )
            .await
            .unwrap_or_default();
            for sr in siblings {
                let sname = cell(&sr, 0);
                if sname != self_object.name && self_vdef.contains(&quote(sname)) {
                    let kind = if cell(&sr, 1).contains("VIEW") {
                        DatabaseObjectKind::View
                    } else {
                        DatabaseObjectKind::Table
                    };
                    raw_uses.push(DependencyEdge {
                        object: DatabaseObjectRef {
                            schema: self_object.schema.clone(),
                            name: sname.into(),
                            kind,
                            identity: None,
                        },
                        relation: if kind == DatabaseObjectKind::View {
                            "view".into()
                        } else {
                            "table".into()
                        },
                    });
                }
            }
        }
    }

    Ok((
        finalize_dependency_edges(self_object, raw_uses, MAX_DEPENDENCY_EDGES),
        finalize_dependency_edges(self_object, raw_used_by, MAX_DEPENDENCY_EDGES),
    ))
}

async fn routine_details_on(
    conn: &mut Conn,
    requested: &DatabaseObjectRef,
) -> AppResult<ObjectDetails> {
    let rtype = if requested.kind == DatabaseObjectKind::Procedure {
        "PROCEDURE"
    } else {
        "FUNCTION"
    };
    let rows = fetch(
        conn,
        "SELECT ROUTINE_TYPE,
                COALESCE(DTD_IDENTIFIER, ''),
                COALESCE(EXTERNAL_LANGUAGE, 'SQL'),
                COALESCE(IS_DETERMINISTIC, 'NO'),
                COALESCE(SECURITY_TYPE, 'DEFINER'),
                COALESCE(ROUTINE_COMMENT, ''),
                SPECIFIC_NAME
         FROM INFORMATION_SCHEMA.ROUTINES
         WHERE ROUTINE_SCHEMA = ? AND ROUTINE_NAME = ? AND ROUTINE_TYPE = ?
         LIMIT 1",
        vec![
            bind(Some(requested.schema.clone())),
            bind(Some(requested.name.clone())),
            bind(Some(rtype.into())),
        ],
        1,
        64 * 1024,
    )
    .await?;

    let row = rows.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The routine is missing or its metadata is unavailable",
        )
    })?;
    let specific_name = cell(row, 6).to_owned();
    let params = fetch(
        conn,
        "SELECT COALESCE(PARAMETER_MODE, ''), COALESCE(PARAMETER_NAME, ''), COALESCE(DTD_IDENTIFIER, '')
         FROM INFORMATION_SCHEMA.PARAMETERS
         WHERE SPECIFIC_SCHEMA = ? AND SPECIFIC_NAME = ? AND ROUTINE_TYPE = ? AND ORDINAL_POSITION > 0
         ORDER BY ORDINAL_POSITION
         LIMIT 500",
        vec![
            bind(Some(requested.schema.clone())),
            bind(Some(specific_name)),
            bind(Some(rtype.into())),
        ],
        500,
        256 * 1024,
    )
    .await
    .unwrap_or_default();

    let identity_arguments = params
        .iter()
        .map(|p| cell(p, 2).to_owned())
        .collect::<Vec<_>>()
        .join(", ");
    let arguments = params
        .iter()
        .map(|p| {
            let mode = cell(p, 0);
            let pname = cell(p, 1);
            let ptype = cell(p, 2);
            if requested.kind == DatabaseObjectKind::Procedure && !mode.is_empty() && mode != "IN" {
                format!("{mode} {pname} {ptype}").trim().to_owned()
            } else if !pname.is_empty() {
                format!("{pname} {ptype}").trim().to_owned()
            } else {
                ptype.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let return_type = if requested.kind == DatabaseObjectKind::Procedure {
        None
    } else {
        Some(cell(row, 1).to_owned()).filter(|s| !s.is_empty())
    };
    let volatility = Some(
        if cell(row, 3).eq_ignore_ascii_case("YES") {
            "DETERMINISTIC"
        } else {
            "NOT DETERMINISTIC"
        }
        .into(),
    );
    let security_definer = cell(row, 4).eq_ignore_ascii_case("DEFINER");
    let comment = Some(cell(row, 5).to_owned()).filter(|c| !c.is_empty());

    let show_sql = format!(
        "SHOW CREATE {rtype} {}.{}",
        quote(&requested.schema),
        quote(&requested.name)
    );
    let ddl = show_create_ddl(conn, &show_sql, 2).await;

    let object = DatabaseObjectRef {
        schema: requested.schema.clone(),
        name: requested.name.clone(),
        kind: requested.kind,
        identity: None,
    };

    Ok(ObjectDetails {
        object,
        owner_table: None,
        comment,
        ddl,
        indexes: Vec::new(),
        constraints: Vec::new(),
        triggers: Vec::new(),
        routine: Some(RoutineInfo {
            identity_arguments,
            arguments,
            return_type,
            language: cell(row, 2).into(),
            volatility,
            security_definer,
        }),
        sequence: None,
        storage: None,
        uses: Vec::new(),
        used_by: Vec::new(),
    })
}

async fn sequence_details_on(
    conn: &mut Conn,
    requested: &DatabaseObjectRef,
    maria: bool,
) -> AppResult<ObjectDetails> {
    if !maria {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "MySQL sequences are supported on MariaDB only",
        ));
    }
    let meta = fetch(
        conn,
        "SELECT COALESCE(TABLE_COMMENT, '')
         FROM INFORMATION_SCHEMA.TABLES
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND TABLE_TYPE = 'SEQUENCE'",
        vec![
            bind(Some(requested.schema.clone())),
            bind(Some(requested.name.clone())),
        ],
        1,
        16 * 1024,
    )
    .await?;
    let row = meta.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The sequence is missing or its metadata is unavailable",
        )
    })?;
    let comment = Some(cell(row, 0).to_owned()).filter(|c| !c.is_empty());

    let seq_sql = format!(
        "SELECT START_VALUE, MINIMUM_VALUE, MAXIMUM_VALUE, INCREMENT, CACHE_SIZE, CYCLE_OPTION, NEXT_NOT_CACHED_VALUE
         FROM {}.{} LIMIT 1",
        quote(&requested.schema),
        quote(&requested.name)
    );
    let sequence = match fetch_text(conn, &seq_sql, 1, 16 * 1024).await {
        Ok(rows) if !rows.is_empty() => {
            let sr = &rows[0];
            Some(SequenceInfo {
                data_type: "bigint".into(),
                start_value: cell(sr, 0).into(),
                min_value: cell(sr, 1).into(),
                max_value: cell(sr, 2).into(),
                increment_by: cell(sr, 3).into(),
                cache_size: cell(sr, 4).into(),
                cycle: cell(sr, 5) != "0",
                last_value: sr.get(6).and_then(Clone::clone),
            })
        }
        _ => None,
    };

    let show_sql = format!(
        "SHOW CREATE SEQUENCE {}.{}",
        quote(&requested.schema),
        quote(&requested.name)
    );
    let mut ddl = show_create_ddl(conn, &show_sql, 1).await;
    if matches!(ddl, ObjectDdlStatus::Unavailable { .. }) {
        let fallback_sql = format!(
            "SHOW CREATE TABLE {}.{}",
            quote(&requested.schema),
            quote(&requested.name)
        );
        ddl = show_create_ddl(conn, &fallback_sql, 1).await;
    }

    Ok(ObjectDetails {
        object: DatabaseObjectRef {
            schema: requested.schema.clone(),
            name: requested.name.clone(),
            kind: DatabaseObjectKind::Sequence,
            identity: None,
        },
        owner_table: None,
        comment,
        ddl,
        indexes: Vec::new(),
        constraints: Vec::new(),
        triggers: Vec::new(),
        routine: None,
        sequence,
        storage: None,
        uses: Vec::new(),
        used_by: Vec::new(),
    })
}

async fn trigger_details_on(
    conn: &mut Conn,
    requested: &DatabaseObjectRef,
) -> AppResult<ObjectDetails> {
    let (sql, params) = if let Some(tbl) = requested.identity.as_deref().filter(|s| !s.is_empty()) {
        (
            "SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE, ACTION_TIMING, EVENT_MANIPULATION, ACTION_ORIENTATION, ACTION_STATEMENT
             FROM INFORMATION_SCHEMA.TRIGGERS
             WHERE TRIGGER_SCHEMA = ? AND TRIGGER_NAME = ? AND EVENT_OBJECT_TABLE = ?
             LIMIT 1",
            vec![
                bind(Some(requested.schema.clone())),
                bind(Some(requested.name.clone())),
                bind(Some(tbl.into())),
            ],
        )
    } else {
        (
            "SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE, ACTION_TIMING, EVENT_MANIPULATION, ACTION_ORIENTATION, ACTION_STATEMENT
             FROM INFORMATION_SCHEMA.TRIGGERS
             WHERE TRIGGER_SCHEMA = ? AND TRIGGER_NAME = ?
             ORDER BY EVENT_OBJECT_TABLE
             LIMIT 1",
            vec![
                bind(Some(requested.schema.clone())),
                bind(Some(requested.name.clone())),
            ],
        )
    };
    let rows = fetch(conn, sql, params, 1, 512 * 1024).await?;
    let row = rows.first().ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The trigger is missing or its metadata is unavailable",
        )
    })?;
    let table_name = cell(row, 1).to_owned();
    let show_sql = format!(
        "SHOW CREATE TRIGGER {}.{}",
        quote(&requested.schema),
        quote(&requested.name)
    );
    let ddl = show_create_ddl(conn, &show_sql, 2).await;

    let trigger_info = TriggerInfo {
        name: cell(row, 0).into(),
        table_name: table_name.clone(),
        timing: cell(row, 2).into(),
        events: vec![cell(row, 3).into()],
        orientation: cell(row, 4).into(),
        enabled: true,
        definition: match &ddl {
            ObjectDdlStatus::Available { ddl, .. } => Some(ddl.clone()),
            ObjectDdlStatus::Unavailable { .. } => {
                Some(cell(row, 5).to_owned()).filter(|s| !s.is_empty())
            }
        },
    };

    let object = DatabaseObjectRef {
        schema: requested.schema.clone(),
        name: requested.name.clone(),
        kind: DatabaseObjectKind::Trigger,
        identity: Some(table_name.clone()),
    };
    let uses = finalize_dependency_edges(
        &object,
        vec![DependencyEdge {
            object: DatabaseObjectRef {
                schema: requested.schema.clone(),
                name: table_name.clone(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
            relation: "target table".into(),
        }],
        MAX_DEPENDENCY_EDGES,
    );

    Ok(ObjectDetails {
        object,
        owner_table: Some(TableRef {
            schema: requested.schema.clone(),
            name: table_name,
        }),
        comment: None,
        ddl,
        indexes: Vec::new(),
        constraints: Vec::new(),
        triggers: vec![trigger_info],
        routine: None,
        sequence: None,
        storage: None,
        uses,
        used_by: Vec::new(),
    })
}

async fn index_details_on(
    conn: &mut Conn,
    requested: &DatabaseObjectRef,
    maria: bool,
) -> AppResult<ObjectDetails> {
    let table_name = if let Some(tbl) = requested.identity.as_deref().filter(|s| !s.is_empty()) {
        tbl.to_owned()
    } else {
        let rows = fetch(
            conn,
            "SELECT TABLE_NAME
             FROM INFORMATION_SCHEMA.STATISTICS
             WHERE TABLE_SCHEMA = ? AND INDEX_NAME = ?
             ORDER BY TABLE_NAME
             LIMIT 1",
            vec![
                bind(Some(requested.schema.clone())),
                bind(Some(requested.name.clone())),
            ],
            1,
            16 * 1024,
        )
        .await?;
        let r = rows.first().ok_or_else(|| {
            AppError::new(
                ErrorCode::NotFound,
                "The index is missing or its metadata is unavailable",
            )
        })?;
        cell(r, 0).to_owned()
    };

    let (all_indexes, idx_meta) =
        fetch_mysql_indexes(conn, &requested.schema, &table_name, maria).await?;
    let index_info = all_indexes
        .into_iter()
        .find(|idx| idx.name == requested.name)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::NotFound,
                "The index is missing or its metadata is unavailable",
            )
        })?;

    let ddl = ObjectDdlStatus::Available {
        ddl: index_info
            .definition
            .clone()
            .unwrap_or_else(|| format_mysql_index_ddl(&requested.schema, &table_name, &index_info)),
        truncated: false,
    };

    let object = DatabaseObjectRef {
        schema: requested.schema.clone(),
        name: requested.name.clone(),
        kind: DatabaseObjectKind::Index,
        identity: Some(table_name.clone()),
    };
    let uses = finalize_dependency_edges(
        &object,
        vec![DependencyEdge {
            object: DatabaseObjectRef {
                schema: requested.schema.clone(),
                name: table_name.clone(),
                kind: DatabaseObjectKind::Table,
                identity: None,
            },
            relation: "indexed table".into(),
        }],
        MAX_DEPENDENCY_EDGES,
    );

    let storage = idx_meta.restricted_reason.map(|reason| TableStorageStats {
        restricted_reason: Some(reason),
        ..Default::default()
    });

    Ok(ObjectDetails {
        object,
        owner_table: Some(TableRef {
            schema: requested.schema.clone(),
            name: table_name,
        }),
        comment: None,
        ddl,
        indexes: vec![index_info],
        constraints: Vec::new(),
        triggers: Vec::new(),
        routine: None,
        sequence: None,
        storage,
        uses,
        used_by: Vec::new(),
    })
}

async fn schema_snapshot_on(
    conn: &mut Conn,
    schema: &str,
    maria: bool,
) -> AppResult<SchemaSnapshot> {
    let param = || vec![bind(Some(schema.to_owned()))];
    let visible = fetch(
        conn,
        "SELECT 1 FROM INFORMATION_SCHEMA.SCHEMATA WHERE SCHEMA_NAME = ?",
        param(),
        1,
        1024,
    )
    .await?;
    if visible.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The database is missing or unavailable",
        ));
    }

    let ctx_rows = fetch(
        conn,
        "SELECT VERSION(), CURRENT_USER()",
        vec![],
        1,
        16 * 1024,
    )
    .await?;
    let (server_version, role) = ctx_rows
        .first()
        .map(|r| (cell(r, 0).to_owned(), cell(r, 1).to_owned()))
        .unwrap_or_default();

    let mut truncated = false;
    let mut restricted = false;

    let rel_rows = fetch(
        conn,
        "SELECT t.TABLE_NAME,
                t.TABLE_TYPE,
                COALESCE(t.TABLE_COMMENT, ''),
                COALESCE(v.VIEW_DEFINITION, '')
         FROM INFORMATION_SCHEMA.TABLES t
         LEFT JOIN INFORMATION_SCHEMA.VIEWS v
           ON v.TABLE_SCHEMA = t.TABLE_SCHEMA AND v.TABLE_NAME = t.TABLE_NAME
         WHERE t.TABLE_SCHEMA = ? AND t.TABLE_TYPE <> 'SEQUENCE'
         ORDER BY t.TABLE_NAME
         LIMIT 501",
        param(),
        MAX_SNAPSHOT_RELATIONS + 1,
        4 * 1024 * 1024,
    )
    .await?;

    let mut tables = Vec::with_capacity(rel_rows.len().min(MAX_SNAPSHOT_RELATIONS));
    let mut table_index: HashMap<String, usize> = HashMap::new();
    for (idx, r) in rel_rows.into_iter().enumerate() {
        if idx >= MAX_SNAPSHOT_RELATIONS {
            truncated = true;
            break;
        }
        let name = cell(&r, 0).to_owned();
        let table_type = cell(&r, 1);
        let kind = if table_type.contains("VIEW") {
            DatabaseObjectKind::View
        } else {
            DatabaseObjectKind::Table
        };
        let comment = Some(cell(&r, 2).to_owned()).filter(|s| !s.is_empty());
        let raw_view_def = cell(&r, 3).trim().to_owned();
        let view_definition = if kind == DatabaseObjectKind::View {
            if raw_view_def.is_empty() {
                restricted = true;
                None
            } else {
                Some(raw_view_def)
            }
        } else {
            None
        };
        let pos = tables.len();
        table_index.insert(name.clone(), pos);
        tables.push(SnapshotTable {
            name,
            kind,
            comment,
            partition_key: None,
            view_definition,
            columns: Vec::new(),
            constraints: Vec::new(),
            indexes: Vec::new(),
            triggers: Vec::new(),
        });
    }

    let col_rows = fetch(
        conn,
        "SELECT TABLE_NAME,
                COLUMN_NAME,
                COLUMN_TYPE,
                IS_NULLABLE,
                COLUMN_DEFAULT,
                COALESCE(EXTRA, ''),
                COALESCE(GENERATION_EXPRESSION, ''),
                ORDINAL_POSITION
         FROM INFORMATION_SCHEMA.COLUMNS
         WHERE TABLE_SCHEMA = ?
         ORDER BY TABLE_NAME, ORDINAL_POSITION
         LIMIT 20001",
        param(),
        MAX_SNAPSHOT_COLUMNS + 1,
        8 * 1024 * 1024,
    )
    .await?;

    for (idx, r) in col_rows.into_iter().enumerate() {
        if idx >= MAX_SNAPSHOT_COLUMNS {
            truncated = true;
            break;
        }
        let Some(&t_idx) = table_index.get(cell(&r, 0)) else {
            continue;
        };
        let extra = cell(&r, 5);
        let extra_upper = extra.to_ascii_uppercase();
        let identity = extra_upper
            .contains("AUTO_INCREMENT")
            .then(|| "AUTO_INCREMENT".into());
        let generated = if extra_upper.contains("STORED") {
            Some("STORED".into())
        } else if extra_upper.contains("VIRTUAL") {
            Some("VIRTUAL".into())
        } else {
            None
        };
        let gen_expr = cell(&r, 6).trim().to_owned();
        let default_expression = if generated.is_some() && !gen_expr.is_empty() {
            Some(gen_expr)
        } else {
            r.get(4).and_then(Clone::clone)
        };
        let ordinal_position = cell(&r, 7).parse::<u32>().unwrap_or(1).max(1);
        tables[t_idx].columns.push(SnapshotColumn {
            name: cell(&r, 1).into(),
            data_type: cell(&r, 2).into(),
            nullable: cell(&r, 3).eq_ignore_ascii_case("YES"),
            default_expression,
            identity,
            generated,
            ordinal_position,
        });
    }

    let tc_rows = fetch(
        conn,
        "SELECT TABLE_NAME, CONSTRAINT_NAME, CONSTRAINT_TYPE
         FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS
         WHERE TABLE_SCHEMA = ?
         ORDER BY TABLE_NAME,
                  CASE CONSTRAINT_TYPE
                    WHEN 'PRIMARY KEY' THEN 1
                    WHEN 'FOREIGN KEY' THEN 2
                    WHEN 'UNIQUE' THEN 3
                    WHEN 'CHECK' THEN 4
                    ELSE 5
                  END,
                  CONSTRAINT_NAME
         LIMIT 10001",
        param(),
        MAX_SNAPSHOT_CONSTRAINTS + 1,
        2 * 1024 * 1024,
    )
    .await?;

    let kcu_rows = fetch(
        conn,
        "SELECT TABLE_NAME, CONSTRAINT_NAME, COLUMN_NAME, REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE
         WHERE TABLE_SCHEMA = ?
         ORDER BY TABLE_NAME, CONSTRAINT_NAME, ORDINAL_POSITION
         LIMIT 20000",
        param(),
        20_000,
        4 * 1024 * 1024,
    )
    .await?;

    let check_sql = if maria {
        "SELECT tc.TABLE_NAME, tc.CONSTRAINT_NAME, cc.CHECK_CLAUSE
         FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
         JOIN INFORMATION_SCHEMA.CHECK_CONSTRAINTS cc
           ON cc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA
          AND cc.TABLE_NAME = tc.TABLE_NAME
          AND cc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME
         WHERE tc.TABLE_SCHEMA = ? AND tc.CONSTRAINT_TYPE = 'CHECK'
         ORDER BY tc.TABLE_NAME, tc.CONSTRAINT_NAME
         LIMIT 5000"
    } else {
        "SELECT tc.TABLE_NAME, tc.CONSTRAINT_NAME, cc.CHECK_CLAUSE
         FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
         JOIN INFORMATION_SCHEMA.CHECK_CONSTRAINTS cc
           ON cc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA
          AND cc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME
         WHERE tc.TABLE_SCHEMA = ? AND tc.CONSTRAINT_TYPE = 'CHECK'
         ORDER BY tc.TABLE_NAME, tc.CONSTRAINT_NAME
         LIMIT 5000"
    };
    let check_rows = fetch(conn, check_sql, param(), 5000, 2 * 1024 * 1024)
        .await
        .unwrap_or_default();

    let mut check_map: HashMap<(String, String), String> = HashMap::new();
    for cr in &check_rows {
        check_map.insert((cell(cr, 0).into(), cell(cr, 1).into()), cell(cr, 2).into());
    }

    type SchemaKeyUsageEntry = (Vec<String>, Option<TableRef>, Vec<String>);
    let mut kcu_map: HashMap<(String, String), SchemaKeyUsageEntry> = HashMap::new();
    for kr in &kcu_rows {
        let key = (cell(kr, 0).to_owned(), cell(kr, 1).to_owned());
        let entry = kcu_map
            .entry(key)
            .or_insert_with(|| (Vec::new(), None, Vec::new()));
        let col = cell(kr, 2);
        if !col.is_empty() {
            entry.0.push(col.to_owned());
        }
        let ref_schema = cell(kr, 3);
        let ref_table = cell(kr, 4);
        let ref_col = cell(kr, 5);
        if !ref_table.is_empty() {
            entry.1 = Some(TableRef {
                schema: if ref_schema.is_empty() {
                    schema.into()
                } else {
                    ref_schema.into()
                },
                name: ref_table.into(),
            });
            if !ref_col.is_empty() {
                entry.2.push(ref_col.to_owned());
            }
        }
    }

    for (idx, tr) in tc_rows.into_iter().enumerate() {
        if idx >= MAX_SNAPSHOT_CONSTRAINTS {
            truncated = true;
            break;
        }
        let tbl_name = cell(&tr, 0).to_owned();
        let Some(&t_idx) = table_index.get(&tbl_name) else {
            continue;
        };
        let cname = cell(&tr, 1).to_owned();
        let ctype = cell(&tr, 2);
        let kind = match ctype {
            "PRIMARY KEY" => ConstraintKind::PrimaryKey,
            "FOREIGN KEY" => ConstraintKind::ForeignKey,
            "UNIQUE" => ConstraintKind::Unique,
            "CHECK" => ConstraintKind::Check,
            _ => continue,
        };
        let key = (tbl_name, cname.clone());
        let (columns, target, target_columns) = kcu_map
            .remove(&key)
            .unwrap_or_else(|| (Vec::new(), None, Vec::new()));
        let definition = match kind {
            ConstraintKind::PrimaryKey => format!("PRIMARY KEY ({})", columns.join(", ")),
            ConstraintKind::Unique => format!("UNIQUE ({})", columns.join(", ")),
            ConstraintKind::ForeignKey => {
                if let Some(t) = &target {
                    format!(
                        "FOREIGN KEY ({}) REFERENCES {}.{} ({})",
                        columns.join(", "),
                        quote(&t.schema),
                        quote(&t.name),
                        target_columns.join(", ")
                    )
                } else {
                    format!("FOREIGN KEY ({})", columns.join(", "))
                }
            }
            ConstraintKind::Check => match check_map.get(&key) {
                Some(clause) if !clause.is_empty() => format!("CHECK ({clause})"),
                _ => "CHECK (...)".into(),
            },
            ConstraintKind::Exclusion => continue,
        };
        tables[t_idx].constraints.push(ConstraintInfo {
            name: cname,
            kind,
            columns,
            target,
            target_columns,
            definition,
        });
    }

    let idx_sql = if maria {
        "SELECT TABLE_NAME, INDEX_NAME, INDEX_TYPE, NON_UNIQUE, COLUMN_NAME, SUB_PART, NULL
         FROM INFORMATION_SCHEMA.STATISTICS
         WHERE TABLE_SCHEMA = ?
         ORDER BY TABLE_NAME, INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX
         LIMIT 20000"
    } else {
        "SELECT TABLE_NAME, INDEX_NAME, INDEX_TYPE, NON_UNIQUE, COLUMN_NAME, SUB_PART, EXPRESSION
         FROM INFORMATION_SCHEMA.STATISTICS
         WHERE TABLE_SCHEMA = ?
         ORDER BY TABLE_NAME, INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX
         LIMIT 20000"
    };
    let idx_rows = fetch(conn, idx_sql, param(), 20_000, 4 * 1024 * 1024).await?;
    let mut total_indexes = 0usize;
    for r in idx_rows {
        let tbl_name = cell(&r, 0);
        let Some(&t_idx) = table_index.get(tbl_name) else {
            continue;
        };
        let name = cell(&r, 1).to_owned();
        let method = cell(&r, 2).to_ascii_lowercase();
        let unique = cell(&r, 3) == "0";
        let primary = name == "PRIMARY";
        let col_name = cell(&r, 4);
        let sub_part = cell(&r, 5);
        let expr_text = cell(&r, 6);
        let key_expr = if !col_name.is_empty() {
            if !sub_part.is_empty() {
                format!("{}({sub_part})", quote(col_name))
            } else {
                quote(col_name)
            }
        } else if !expr_text.is_empty() {
            format!("({expr_text})")
        } else {
            continue;
        };

        match tables[t_idx].indexes.last_mut() {
            Some(last) if last.name == name => {
                last.expressions.push(key_expr);
            }
            _ => {
                if total_indexes >= MAX_SNAPSHOT_INDEXES {
                    truncated = true;
                    break;
                }
                total_indexes += 1;
                tables[t_idx].indexes.push(SnapshotIndex {
                    name,
                    method,
                    unique,
                    primary,
                    expressions: vec![key_expr],
                    predicate: None,
                    definition: None,
                });
            }
        }
    }
    for tbl in &mut tables {
        let tbl_name = tbl.name.clone();
        for idx in &mut tbl.indexes {
            let info = IndexInfo {
                name: idx.name.clone(),
                method: idx.method.clone(),
                unique: idx.unique,
                primary: idx.primary,
                expressions: idx.expressions.clone(),
                predicate: idx.predicate.clone(),
                definition: None,
                size_bytes: None,
                scans: None,
                tuples_read: None,
                tuples_fetched: None,
                cardinality: None,
            };
            idx.definition = Some(format_mysql_index_ddl(schema, &tbl_name, &info));
        }
    }

    let trig_rows = fetch(
        conn,
        "SELECT EVENT_OBJECT_TABLE, TRIGGER_NAME, ACTION_TIMING, EVENT_MANIPULATION, ACTION_ORIENTATION, ACTION_STATEMENT
         FROM INFORMATION_SCHEMA.TRIGGERS
         WHERE TRIGGER_SCHEMA = ?
         ORDER BY EVENT_OBJECT_TABLE, TRIGGER_NAME
         LIMIT 5001",
        param(),
        MAX_SNAPSHOT_TRIGGERS + 1,
        4 * 1024 * 1024,
    )
    .await?;

    for (idx, r) in trig_rows.into_iter().enumerate() {
        if idx >= MAX_SNAPSHOT_TRIGGERS {
            truncated = true;
            break;
        }
        let tbl_name = cell(&r, 0).to_owned();
        let Some(&t_idx) = table_index.get(&tbl_name) else {
            continue;
        };
        tables[t_idx].triggers.push(TriggerInfo {
            name: cell(&r, 1).into(),
            table_name: tbl_name,
            timing: cell(&r, 2).into(),
            events: vec![cell(&r, 3).into()],
            orientation: cell(&r, 4).into(),
            enabled: true,
            definition: Some(cell(&r, 5).to_owned()).filter(|s| !s.is_empty()),
        });
    }

    let routine_rows = fetch(
        conn,
        "SELECT ROUTINE_NAME,
                ROUTINE_TYPE,
                SPECIFIC_NAME,
                COALESCE(DTD_IDENTIFIER, ''),
                COALESCE(EXTERNAL_LANGUAGE, 'SQL'),
                COALESCE(IS_DETERMINISTIC, 'NO'),
                COALESCE(SECURITY_TYPE, 'DEFINER'),
                COALESCE(ROUTINE_DEFINITION, ''),
                COALESCE(ROUTINE_COMMENT, '')
         FROM INFORMATION_SCHEMA.ROUTINES
         WHERE ROUTINE_SCHEMA = ?
           AND ROUTINE_TYPE IN ('FUNCTION', 'PROCEDURE')
         ORDER BY ROUTINE_NAME, ROUTINE_TYPE
         LIMIT 5001",
        param(),
        MAX_SNAPSHOT_ROUTINES + 1,
        4 * 1024 * 1024,
    )
    .await?;

    let mut routines = Vec::with_capacity(routine_rows.len().min(MAX_SNAPSHOT_ROUTINES));
    if !routine_rows.is_empty() {
        let param_rows = fetch(
            conn,
            "SELECT SPECIFIC_NAME,
                    ROUTINE_TYPE,
                    COALESCE(PARAMETER_MODE, ''),
                    COALESCE(PARAMETER_NAME, ''),
                    COALESCE(DTD_IDENTIFIER, '')
             FROM INFORMATION_SCHEMA.PARAMETERS
             WHERE SPECIFIC_SCHEMA = ?
               AND ORDINAL_POSITION > 0
             ORDER BY SPECIFIC_NAME, ROUTINE_TYPE, ORDINAL_POSITION
             LIMIT 10000",
            param(),
            10_000,
            4 * 1024 * 1024,
        )
        .await
        .unwrap_or_default();

        type RoutineParamTuple = (String, String, String);
        let mut params_by_routine: HashMap<(String, String), Vec<RoutineParamTuple>> =
            HashMap::new();
        for pr in &param_rows {
            params_by_routine
                .entry((cell(pr, 0).to_owned(), cell(pr, 1).to_owned()))
                .or_default()
                .push((
                    cell(pr, 2).to_owned(),
                    cell(pr, 3).to_owned(),
                    cell(pr, 4).to_owned(),
                ));
        }

        for (idx, r) in routine_rows.into_iter().enumerate() {
            if idx >= MAX_SNAPSHOT_ROUTINES {
                truncated = true;
                break;
            }
            let name = cell(&r, 0).to_owned();
            let rtype = cell(&r, 1);
            let specific = cell(&r, 2).to_owned();
            let ret_type = cell(&r, 3);
            let language = cell(&r, 4).to_owned();
            let det = cell(&r, 5);
            let sec = cell(&r, 6);
            let body = cell(&r, 7).trim().to_owned();
            let comment = Some(cell(&r, 8).to_owned()).filter(|s| !s.is_empty());

            let kind = if rtype.eq_ignore_ascii_case("PROCEDURE") {
                DatabaseObjectKind::Procedure
            } else {
                DatabaseObjectKind::Function
            };
            let plist = params_by_routine
                .remove(&(specific, rtype.to_owned()))
                .unwrap_or_default();
            let identity_arguments = plist
                .iter()
                .map(|(_, _, ptype)| ptype.clone())
                .collect::<Vec<_>>()
                .join(", ");
            let arguments = plist
                .iter()
                .map(|(mode, pname, ptype)| {
                    if kind == DatabaseObjectKind::Procedure && !mode.is_empty() && mode != "IN" {
                        format!("{mode} {pname} {ptype}").trim().to_owned()
                    } else if !pname.is_empty() {
                        format!("{pname} {ptype}").trim().to_owned()
                    } else {
                        ptype.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");

            let definition = if body.is_empty() {
                restricted = true;
                None
            } else {
                Some(body)
            };

            routines.push(SnapshotRoutine {
                name,
                kind,
                identity_arguments,
                arguments,
                return_type: (kind == DatabaseObjectKind::Function && !ret_type.is_empty())
                    .then(|| ret_type.to_owned()),
                language,
                volatility: Some(
                    if det.eq_ignore_ascii_case("YES") {
                        "DETERMINISTIC"
                    } else {
                        "NOT DETERMINISTIC"
                    }
                    .into(),
                ),
                security_definer: sec.eq_ignore_ascii_case("DEFINER"),
                definition,
                comment,
            });
        }
    }

    let mut sequences = Vec::new();
    if maria {
        let seq_rows = fetch(
            conn,
            "SELECT TABLE_NAME
             FROM INFORMATION_SCHEMA.TABLES
             WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'SEQUENCE'
             ORDER BY TABLE_NAME
             LIMIT 5001",
            param(),
            MAX_SNAPSHOT_SEQUENCES + 1,
            1024 * 1024,
        )
        .await
        .unwrap_or_default();

        for (idx, r) in seq_rows.into_iter().enumerate() {
            if idx >= MAX_SNAPSHOT_SEQUENCES {
                truncated = true;
                break;
            }
            let seq_name = cell(&r, 0).to_owned();
            if identifier(&seq_name).is_err() {
                continue;
            }
            let seq_sql = format!(
                "SELECT START_VALUE, MINIMUM_VALUE, MAXIMUM_VALUE, INCREMENT, CACHE_SIZE, CYCLE_OPTION
                 FROM {}.{} LIMIT 1",
                quote(schema),
                quote(&seq_name)
            );
            match fetch_text(conn, &seq_sql, 1, 16 * 1024).await {
                Ok(rows) if !rows.is_empty() => {
                    let sr = &rows[0];
                    sequences.push(SnapshotSequence {
                        name: seq_name,
                        data_type: "bigint".into(),
                        start_value: cell(sr, 0).into(),
                        min_value: cell(sr, 1).into(),
                        max_value: cell(sr, 2).into(),
                        increment_by: cell(sr, 3).into(),
                        cache_size: cell(sr, 4).into(),
                        cycle: cell(sr, 5) != "0",
                    });
                }
                _ => {
                    restricted = true;
                    sequences.push(SnapshotSequence {
                        name: seq_name,
                        data_type: "bigint".into(),
                        start_value: "1".into(),
                        min_value: "1".into(),
                        max_value: String::new(),
                        increment_by: "1".into(),
                        cache_size: "1000".into(),
                        cycle: false,
                    });
                }
            }
        }
    }

    let event_rows = fetch(
        conn,
        "SELECT EVENT_NAME
         FROM INFORMATION_SCHEMA.EVENTS
         WHERE EVENT_SCHEMA = ?
         ORDER BY EVENT_NAME
         LIMIT 1001",
        param(),
        MAX_SNAPSHOT_UNSUPPORTED + 1,
        512 * 1024,
    )
    .await
    .unwrap_or_default();

    let mut unsupported_objects =
        Vec::with_capacity(event_rows.len().min(MAX_SNAPSHOT_UNSUPPORTED));
    for (idx, r) in event_rows.into_iter().enumerate() {
        if idx >= MAX_SNAPSHOT_UNSUPPORTED {
            truncated = true;
            break;
        }
        unsupported_objects.push(UnsupportedSchemaObject {
            name: cell(&r, 0).into(),
            object_type: "event".into(),
            reason: "MySQL/MariaDB scheduled events are disclosed but not structurally diffed"
                .into(),
        });
    }

    let mut snapshot = SchemaSnapshot {
        engine: EngineKind::Mysql,
        server_version,
        database: schema.to_owned(),
        schema: schema.to_owned(),
        role,
        tables,
        routines,
        sequences,
        unsupported_objects,
        truncated,
        restricted,
    };
    snapshot.canonicalize();
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_index_ddl_quotes_backticks_and_handles_primary_and_unique() {
        let primary = IndexInfo {
            name: "PRIMARY".into(),
            method: "btree".into(),
            unique: true,
            primary: true,
            expressions: vec!["`id`".into()],
            predicate: None,
            definition: None,
            size_bytes: None,
            scans: None,
            tuples_read: None,
            tuples_fetched: None,
            cardinality: None,
        };
        assert_eq!(
            format_mysql_index_ddl("joinery", "orders", &primary),
            "ALTER TABLE `joinery`.`orders` ADD PRIMARY KEY (`id`);"
        );

        let functional = IndexInfo {
            name: "idx_lower`email".into(),
            method: "btree".into(),
            unique: true,
            primary: false,
            expressions: vec!["(lower(`email`))".into()],
            predicate: None,
            definition: None,
            size_bytes: None,
            scans: None,
            tuples_read: None,
            tuples_fetched: None,
            cardinality: None,
        };
        assert_eq!(
            format_mysql_index_ddl("joinery", "cust`omers", &functional),
            "CREATE UNIQUE INDEX `idx_lower``email` ON `joinery`.`cust``omers` ((lower(`email`))) USING BTREE;"
        );
    }
}
