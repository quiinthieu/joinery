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
        "SELECT TABLE_TYPE, COALESCE(TABLE_COMMENT, '')
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

    let indexes = fetch_mysql_indexes(conn, &object.schema, &object.name, maria).await?;
    let constraints = fetch_mysql_constraints(conn, &object.schema, &object.name, maria).await?;
    let triggers = fetch_mysql_triggers(conn, &object.schema, Some(&object.name)).await?;

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
        uses,
        used_by,
    })
}

async fn fetch_mysql_indexes(
    conn: &mut Conn,
    schema: &str,
    table: &str,
    maria: bool,
) -> AppResult<Vec<IndexInfo>> {
    let sql = if maria {
        "SELECT INDEX_NAME, INDEX_TYPE, NON_UNIQUE, COLUMN_NAME, SUB_PART, NULL
         FROM INFORMATION_SCHEMA.STATISTICS
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
         ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX
         LIMIT 2000"
    } else {
        "SELECT INDEX_NAME, INDEX_TYPE, NON_UNIQUE, COLUMN_NAME, SUB_PART, EXPRESSION
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
                });
            }
        }
    }

    for idx in &mut indexes {
        idx.definition = Some(format_mysql_index_ddl(schema, table, idx));
    }
    Ok(indexes)
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

    let all_indexes = fetch_mysql_indexes(conn, &requested.schema, &table_name, maria).await?;
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
        uses,
        used_by: Vec::new(),
    })
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
        };
        assert_eq!(
            format_mysql_index_ddl("joinery", "cust`omers", &functional),
            "CREATE UNIQUE INDEX `idx_lower``email` ON `joinery`.`cust``omers` ((lower(`email`))) USING BTREE;"
        );
    }
}
