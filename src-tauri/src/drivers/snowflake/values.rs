use crate::domain::*;
use crate::drivers::RowMutation;
use crate::drivers::snowflake::catalog::identifier;

/// Returns true if the column is a Snowflake floating point type (FLOAT, REAL, DOUBLE)
pub fn is_snowflake_float(database_type: &str) -> bool {
    let ty = database_type.trim().to_ascii_uppercase();
    ty == "REAL"
        || ty == "FLOAT"
        || ty == "FLOAT4"
        || ty == "FLOAT8"
        || ty == "DOUBLE"
        || ty == "DOUBLE PRECISION"
        || ty.starts_with("FLOAT")
        || ty.starts_with("DOUBLE")
}

/// Generates parameter-bound SQL and values for change plan review in execution.rs
pub fn mutation_sql(
    schema: &TableSchema,
    row: &RowMutation,
) -> AppResult<(String, Vec<Option<String>>)> {
    if !schema.editable {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "This table cannot be edited safely",
        ));
    }
    match row {
        RowMutation::Update { original, changes } => {
            if changes.is_empty() {
                return Err(AppError::new(
                    ErrorCode::PolicyBlocked,
                    "This table cannot be edited safely",
                ));
            }
            let mut assignments = Vec::new();
            let mut values = Vec::new();
            for (index, val) in changes {
                let col = schema
                    .columns
                    .get(*index)
                    .ok_or_else(|| AppError::invalid("Invalid edited column"))?;
                if !col.editable || col.primary_key {
                    return Err(AppError::new(
                        ErrorCode::PolicyBlocked,
                        "This column is read-only",
                    ));
                }
                let cast_expr = match col.kind {
                    ColumnKind::Text => "?".into(),
                    ColumnKind::Integer => "CAST(? AS NUMBER)".into(),
                    ColumnKind::Decimal => {
                        if is_snowflake_float(&col.database_type) {
                            "CAST(? AS FLOAT)".into()
                        } else {
                            "CAST(? AS NUMBER)".into()
                        }
                    }
                    ColumnKind::Boolean => "CAST(? AS BOOLEAN)".into(),
                    ColumnKind::Temporal => {
                        let ty = if col.database_type.is_empty() {
                            "TIMESTAMP_NTZ"
                        } else {
                            &col.database_type
                        };
                        format!("CAST(? AS {ty})")
                    }
                    ColumnKind::Json => "PARSE_JSON(?)".into(),
                    ColumnKind::Binary => "CAST(? AS BINARY)".into(),
                    ColumnKind::Unknown => {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "Unsupported data type",
                        ));
                    }
                };
                assignments.push(format!("{} = {cast_expr}", identifier(&col.name)?));
                values.push(val.clone());
            }
            let (predicate, where_vals) = optimistic_predicate(schema, original)?;
            values.extend(where_vals);
            let qualified_table = format!(
                "{}.{}",
                identifier(&schema.table.schema)?,
                identifier(&schema.table.name)?
            );
            Ok((
                format!(
                    "UPDATE {qualified_table} SET {} WHERE {predicate}",
                    assignments.join(", ")
                ),
                values,
            ))
        }
        RowMutation::Delete { original } => {
            let (predicate, where_vals) = optimistic_predicate(schema, original)?;
            let qualified_table = format!(
                "{}.{}",
                identifier(&schema.table.schema)?,
                identifier(&schema.table.name)?
            );
            Ok((
                format!("DELETE FROM {qualified_table} WHERE {predicate}"),
                where_vals,
            ))
        }
        RowMutation::Insert {
            values: insert_vals,
        } => {
            let qualified_table = format!(
                "{}.{}",
                identifier(&schema.table.schema)?,
                identifier(&schema.table.name)?
            );
            if insert_vals.is_empty() {
                return Ok((
                    format!("INSERT INTO {qualified_table} DEFAULT VALUES"),
                    Vec::new(),
                ));
            }
            let mut col_names = Vec::new();
            let mut exprs = Vec::new();
            let mut values = Vec::new();
            for (index, val) in insert_vals {
                let col = schema
                    .columns
                    .get(*index)
                    .ok_or_else(|| AppError::invalid("Invalid inserted column"))?;
                if !col.insertable {
                    return Err(AppError::new(
                        ErrorCode::PolicyBlocked,
                        "This column cannot be set on insert",
                    ));
                }
                col_names.push(identifier(&col.name)?);
                let cast_expr = match col.kind {
                    ColumnKind::Text => "?".into(),
                    ColumnKind::Integer => "CAST(? AS NUMBER)".into(),
                    ColumnKind::Decimal => {
                        if is_snowflake_float(&col.database_type) {
                            "CAST(? AS FLOAT)".into()
                        } else {
                            "CAST(? AS NUMBER)".into()
                        }
                    }
                    ColumnKind::Boolean => "CAST(? AS BOOLEAN)".into(),
                    ColumnKind::Temporal => {
                        let ty = if col.database_type.is_empty() {
                            "TIMESTAMP_NTZ"
                        } else {
                            &col.database_type
                        };
                        format!("CAST(? AS {ty})")
                    }
                    ColumnKind::Json => "PARSE_JSON(?)".into(),
                    ColumnKind::Binary => "CAST(? AS BINARY)".into(),
                    ColumnKind::Unknown => {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "Unsupported data type",
                        ));
                    }
                };
                exprs.push(cast_expr);
                values.push(val.clone());
            }
            Ok((
                format!(
                    "INSERT INTO {qualified_table} ({}) VALUES ({})",
                    col_names.join(", "),
                    exprs.join(", ")
                ),
                values,
            ))
        }
    }
}

/// Builds optimistic concurrency predicate using EQUAL_NULL for all non-variant columns
pub fn optimistic_predicate(
    schema: &TableSchema,
    original: &TextRow,
) -> AppResult<(String, Vec<Option<String>>)> {
    let mut parts = Vec::new();
    let mut values = Vec::new();

    // 1. Primary keys first
    for pk_name in &schema.primary_key {
        let index = schema
            .columns
            .iter()
            .position(|c| &c.name == pk_name)
            .ok_or_else(|| AppError::invalid("Missing primary key column"))?;
        let col = &schema.columns[index];
        let ty = if col.database_type.is_empty() {
            match col.kind {
                ColumnKind::Integer | ColumnKind::Decimal => "NUMBER",
                ColumnKind::Text => "VARCHAR",
                ColumnKind::Boolean => "BOOLEAN",
                ColumnKind::Temporal => "TIMESTAMP_NTZ",
                _ => "VARCHAR",
            }
        } else {
            &col.database_type
        };
        parts.push(format!("{} = CAST(? AS {ty})", identifier(&col.name)?));
        values.push(original.get(index).cloned().flatten());
    }

    // 2. Concurrency check on all other non-variant, non-binary columns
    for (index, col) in schema.columns.iter().enumerate() {
        if schema.primary_key.contains(&col.name) {
            continue;
        }
        if matches!(
            col.kind,
            ColumnKind::Json | ColumnKind::Binary | ColumnKind::Unknown
        ) {
            continue;
        }
        let cast_type = match col.kind {
            ColumnKind::Text => "VARCHAR".to_string(),
            ColumnKind::Integer => "NUMBER".to_string(),
            ColumnKind::Decimal => {
                if is_snowflake_float(&col.database_type) {
                    "FLOAT".to_string()
                } else {
                    "NUMBER".to_string()
                }
            }
            ColumnKind::Boolean => "BOOLEAN".to_string(),
            ColumnKind::Temporal => {
                if col.database_type.is_empty() {
                    "TIMESTAMP_NTZ".to_string()
                } else {
                    col.database_type.clone()
                }
            }
            _ => "VARCHAR".to_string(),
        };
        parts.push(format!(
            "EQUAL_NULL({}, CAST(? AS {cast_type}))",
            identifier(&col.name)?
        ));
        values.push(original.get(index).cloned().flatten());
    }

    if parts.is_empty() {
        return Err(AppError::new(
            ErrorCode::PolicyBlocked,
            "A primary key or unique columns are required for edits",
        ));
    }
    Ok((parts.join(" AND "), values))
}

/// Formats a literal value safely escaped for inclusion in Snowflake Scripting blocks
pub fn format_snowflake_literal(col: &ColumnInfo, value: Option<&str>) -> String {
    let Some(val) = value else {
        return "NULL".to_string();
    };
    match col.kind {
        ColumnKind::Integer => {
            if val.parse::<i64>().is_ok() {
                val.to_string()
            } else {
                format!("'{}'::NUMBER", val.replace('\'', "''"))
            }
        }
        ColumnKind::Decimal => {
            let trimmed = val.trim();
            if let Ok(f) = trimmed.parse::<f64>() {
                if f.is_nan() {
                    "'NaN'::FLOAT".to_string()
                } else if f.is_infinite() {
                    if f.is_sign_negative() {
                        "'-Infinity'::FLOAT".to_string()
                    } else {
                        "'Infinity'::FLOAT".to_string()
                    }
                } else {
                    trimmed.to_string()
                }
            } else {
                let ty = if is_snowflake_float(&col.database_type) {
                    "FLOAT"
                } else {
                    "NUMBER"
                };
                format!("'{}'::{ty}", val.replace('\'', "''"))
            }
        }
        ColumnKind::Boolean => match val.to_ascii_lowercase().as_str() {
            "true" | "t" | "1" => "TRUE".into(),
            "false" | "f" | "0" => "FALSE".into(),
            _ => format!("'{}'::BOOLEAN", val.replace('\'', "''")),
        },
        ColumnKind::Temporal => {
            let ty = if col.database_type.is_empty() {
                "TIMESTAMP_NTZ"
            } else {
                &col.database_type
            };
            format!("'{}'::{ty}", val.replace('\'', "''"))
        }
        ColumnKind::Json => {
            format!("PARSE_JSON('{}')", val.replace('\'', "''"))
        }
        ColumnKind::Binary => {
            format!("'{}'::BINARY", val.replace('\'', "''"))
        }
        ColumnKind::Text | ColumnKind::Unknown => {
            format!("'{}'", val.replace('\'', "''"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_schema() -> TableSchema {
        TableSchema {
            table: TableRef {
                schema: "PUBLIC".into(),
                name: "CUSTOMERS".into(),
            },
            columns: vec![
                ColumnInfo {
                    id: "c0".into(),
                    name: "id".into(),
                    database_type: "NUMBER(38,0)".into(),
                    kind: ColumnKind::Integer,
                    nullable: false,
                    primary_key: true,
                    foreign_key: false,
                    editable: false,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c1".into(),
                    name: "name".into(),
                    database_type: "VARCHAR(255)".into(),
                    kind: ColumnKind::Text,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c2".into(),
                    name: "balance".into(),
                    database_type: "NUMBER(10,2)".into(),
                    kind: ColumnKind::Decimal,
                    nullable: false,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
            ],
            primary_key: vec!["id".into()],
            foreign_keys: Vec::new(),
            inbound_foreign_keys: Vec::new(),
            editable: true,
        }
    }

    #[test]
    fn test_update_generates_equal_null_optimistic_checks() {
        let schema = test_schema();
        let mutation = RowMutation::Update {
            original: vec![
                Some("101".into()),
                Some("Acme Corp".into()),
                Some("500.00".into()),
            ],
            changes: vec![(2, Some("600.00".into()))],
        };
        let (sql, vals) = mutation_sql(&schema, &mutation).unwrap();
        assert_eq!(
            sql,
            "UPDATE \"PUBLIC\".\"CUSTOMERS\" SET \"balance\" = CAST(? AS NUMBER) WHERE \"id\" = CAST(? AS NUMBER(38,0)) AND EQUAL_NULL(\"name\", CAST(? AS VARCHAR)) AND EQUAL_NULL(\"balance\", CAST(? AS NUMBER))"
        );
        assert_eq!(
            vals,
            vec![
                Some("600.00".into()),
                Some("101".into()),
                Some("Acme Corp".into()),
                Some("500.00".into())
            ]
        );
    }

    #[test]
    fn test_insert_binds_columns() {
        let schema = test_schema();
        let mutation = RowMutation::Insert {
            values: vec![
                (0, Some("102".into())),
                (1, Some("Globex".into())),
                (2, Some("250.00".into())),
            ],
        };
        let (sql, vals) = mutation_sql(&schema, &mutation).unwrap();
        assert_eq!(
            sql,
            "INSERT INTO \"PUBLIC\".\"CUSTOMERS\" (\"id\", \"name\", \"balance\") VALUES (CAST(? AS NUMBER), ?, CAST(? AS NUMBER))"
        );
        assert_eq!(
            vals,
            vec![
                Some("102".into()),
                Some("Globex".into()),
                Some("250.00".into())
            ]
        );
    }

    #[test]
    fn test_delete_includes_optimistic_checks() {
        let schema = test_schema();
        let mutation = RowMutation::Delete {
            original: vec![
                Some("101".into()),
                Some("Acme Corp".into()),
                Some("500.00".into()),
            ],
        };
        let (sql, vals) = mutation_sql(&schema, &mutation).unwrap();
        assert_eq!(
            sql,
            "DELETE FROM \"PUBLIC\".\"CUSTOMERS\" WHERE \"id\" = CAST(? AS NUMBER(38,0)) AND EQUAL_NULL(\"name\", CAST(? AS VARCHAR)) AND EQUAL_NULL(\"balance\", CAST(? AS NUMBER))"
        );
        assert_eq!(
            vals,
            vec![
                Some("101".into()),
                Some("Acme Corp".into()),
                Some("500.00".into())
            ]
        );
    }

    #[test]
    fn test_format_literal_escaping() {
        let col_text = ColumnInfo {
            id: "c".into(),
            name: "t".into(),
            database_type: "VARCHAR".into(),
            kind: ColumnKind::Text,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: true,
            insertable: true,
            has_default: false,
        };
        assert_eq!(
            format_snowflake_literal(&col_text, Some("O'Reilly")),
            "'O''Reilly'"
        );
        assert_eq!(format_snowflake_literal(&col_text, None), "NULL");

        let col_int = ColumnInfo {
            id: "c".into(),
            name: "i".into(),
            database_type: "NUMBER".into(),
            kind: ColumnKind::Integer,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: true,
            insertable: true,
            has_default: false,
        };
        assert_eq!(format_snowflake_literal(&col_int, Some("42")), "42");

        let col_bool = ColumnInfo {
            id: "c".into(),
            name: "b".into(),
            database_type: "BOOLEAN".into(),
            kind: ColumnKind::Boolean,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: true,
            insertable: true,
            has_default: false,
        };
        assert_eq!(format_snowflake_literal(&col_bool, Some("true")), "TRUE");
        assert_eq!(format_snowflake_literal(&col_bool, Some("0")), "FALSE");
    }

    #[test]
    fn test_insert_empty_values_generates_default_values() {
        let schema = test_schema();
        let mutation = RowMutation::Insert { values: vec![] };
        let (sql, vals) = mutation_sql(&schema, &mutation).unwrap();
        assert_eq!(sql, "INSERT INTO \"PUBLIC\".\"CUSTOMERS\" DEFAULT VALUES");
        assert!(vals.is_empty());
    }

    #[test]
    fn test_format_snowflake_literal_decimal_special_floats() {
        let col_dec = ColumnInfo {
            id: "c_dec".into(),
            name: "val".into(),
            database_type: "NUMBER(10,2)".into(),
            kind: ColumnKind::Decimal,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: true,
            insertable: true,
            has_default: false,
        };

        // Special floats
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("NaN")),
            "'NaN'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("nan")),
            "'NaN'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("Infinity")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("+Infinity")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("inf")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("-Infinity")),
            "'-Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("-inf")),
            "'-Infinity'::FLOAT"
        );

        // Trimmed special floats
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("  NaN  ")),
            "'NaN'::FLOAT"
        );

        // Standard finite numbers (preserving exact digits)
        assert_eq!(format_snowflake_literal(&col_dec, Some("42.50")), "42.50");
        assert_eq!(format_snowflake_literal(&col_dec, Some("-12.34")), "-12.34");
        assert_eq!(format_snowflake_literal(&col_dec, Some("0")), "0");
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("12345678901234567890.1234567890")),
            "12345678901234567890.1234567890"
        );

        // Float vs Number fallback on unparseable strings
        let col_float = ColumnInfo {
            id: "c_flt".into(),
            name: "val".into(),
            database_type: "REAL".into(),
            kind: ColumnKind::Decimal,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: true,
            insertable: true,
            has_default: false,
        };
        assert_eq!(
            format_snowflake_literal(&col_dec, Some("invalid")),
            "'invalid'::NUMBER"
        );
        assert_eq!(
            format_snowflake_literal(&col_float, Some("invalid")),
            "'invalid'::FLOAT"
        );
    }
}
