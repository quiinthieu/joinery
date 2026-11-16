use super::*;
use sqlparser::{dialect::SnowflakeDialect, parser::Parser};

fn text(value: &str) -> CellValue {
    CellValue::Text(value.into())
}
fn column(name: &str, ty: ScalarType, nullable: bool) -> MutationColumn {
    MutationColumn::new(name, ty, nullable, WriteMode::Writable, false).unwrap()
}
fn target() -> MutationTarget {
    MutationTarget::new(
        "db",
        "schema",
        "keyless",
        vec![
            column(
                "label",
                ScalarType::Text {
                    max_bytes: MAX_CELL_BYTES,
                },
                true,
            ),
            column(
                "amount",
                ScalarType::Decimal {
                    precision: 38,
                    scale: 2,
                },
                false,
            ),
            column("enabled", ScalarType::Boolean, false),
        ],
    )
    .unwrap()
}
fn original(target: &MutationTarget, label: &str, amount: &str) -> OriginalRow {
    OriginalRow::new(target, vec![text(label), text(amount), text("true")]).unwrap()
}
fn parse_batch(batch: &CompiledBatch) {
    for unit in batch.units() {
        let statements = Parser::parse_sql(&SnowflakeDialect {}, unit.sql()).unwrap();
        assert_eq!(statements.len(), 1);
        assert_eq!(unit.expected_rows(), 1);
        assert!(unit.sql().len() <= MAX_UNIT_SQL_BYTES);
        assert!(unit.bindings().len() <= 2 * MAX_COLUMNS);
        assert!(!unit.sql().contains(" LIMIT "));
        assert!(!unit.sql().contains("COUNT("));
    }
    let output_bytes: usize = batch
        .units()
        .iter()
        .map(|u| {
            u.sql().len()
                + u.bindings()
                    .iter()
                    .map(|b| b.value().map_or(0, str::len))
                    .sum::<usize>()
        })
        .sum();
    assert!(output_bytes <= batch.accounted_bytes());
    assert!(batch.accounted_bytes() <= MAX_BATCH_BYTES);
}

#[test]
fn quoted_names_and_injection_values_remain_one_statement_with_binds() {
    let target = MutationTarget::new(
        "d\"; DROP TABLE x;--",
        "雪",
        "T\"able",
        vec![column("c\"ol", ScalarType::Text { max_bytes: 1000 }, false)],
    )
    .unwrap();
    let attack = "'); DELETE FROM secret; --\n\0";
    let batch = compile(
        &target,
        &[Mutation::Insert {
            values: vec![(0, text(attack))],
        }],
    )
    .unwrap();
    let unit = &batch.units()[0];
    assert!(
        unit.sql()
            .contains("\"d\"\"; DROP TABLE x;--\".\"雪\".\"T\"\"able\"")
    );
    assert!(unit.sql().contains("\"c\"\"ol\""));
    assert!(!unit.sql().contains(attack));
    assert_eq!(unit.bindings()[0].value(), Some(attack));
    parse_batch(&batch);
}

#[test]
fn ordered_units_and_assignments_preserve_whole_keyless_original() {
    let target = target();
    let batch = compile(
        &target,
        &[
            Mutation::Update {
                original: original(&target, "old", "9007199254740993.12"),
                changes: vec![(2, text("false")), (0, text("new"))],
            },
            Mutation::Delete {
                original: original(&target, "other", "2.00"),
            },
            Mutation::Insert {
                values: vec![(1, text("3.01")), (0, text("inserted")), (2, text("true"))],
            },
        ],
    )
    .unwrap();
    assert_eq!(
        batch
            .units()
            .iter()
            .map(CompiledUnit::kind)
            .collect::<Vec<_>>(),
        vec![
            MutationKind::Update,
            MutationKind::Delete,
            MutationKind::Insert
        ]
    );
    let update = &batch.units()[0];
    assert!(
        update
            .sql()
            .contains("SET \"enabled\" = CAST(? AS BOOLEAN), \"label\" = CAST(? AS VARCHAR) WHERE")
    );
    assert!(update.sql().contains(
        "TO_BINARY(\"label\", 'UTF-8') IS NOT DISTINCT FROM TO_BINARY(CAST(? AS VARCHAR), 'UTF-8')"
    ));
    assert!(update.sql().contains("\"amount\" IS NOT DISTINCT FROM CAST(? AS NUMBER(38,2)) AND \"enabled\" IS NOT DISTINCT FROM CAST(? AS BOOLEAN)"));
    assert_eq!(
        update
            .bindings()
            .iter()
            .map(Binding::value)
            .collect::<Vec<_>>(),
        vec![
            Some("false"),
            Some("new"),
            Some("old"),
            Some("9007199254740993.12"),
            Some("true")
        ]
    );
    // A duplicate server row would match this whole predicate too. The plan
    // requires exactly one affected row; only runtime rollback can enforce it.
    assert_eq!(batch.units()[1].expected_rows(), 1);
    parse_batch(&batch);
}

#[test]
fn sql_null_empty_and_default_are_distinct() {
    let target = MutationTarget::new(
        "d",
        "s",
        "t",
        vec![
            column("a", ScalarType::Text { max_bytes: 10 }, true),
            MutationColumn::new(
                "b",
                ScalarType::Text { max_bytes: 10 },
                false,
                WriteMode::Writable,
                true,
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let batch = compile(
        &target,
        &[
            Mutation::Insert {
                values: vec![(0, CellValue::Null), (1, CellValue::Default)],
            },
            Mutation::Insert {
                values: vec![(0, text("")), (1, text("x"))],
            },
        ],
    )
    .unwrap();
    assert!(
        batch.units()[0]
            .sql()
            .ends_with("VALUES (CAST(? AS VARCHAR), DEFAULT)")
    );
    assert_eq!(batch.units()[0].bindings()[0].value(), None);
    assert_eq!(batch.units()[1].bindings()[0].value(), Some(""));
    let row = OriginalRow::new(&target, vec![CellValue::Null, text("x")]).unwrap();
    let delete = compile(&target, &[Mutation::Delete { original: row }]).unwrap();
    assert_eq!(delete.units()[0].bindings()[0].value(), None);
    assert!(delete.units()[0].sql().contains("IS NOT DISTINCT FROM"));
    parse_batch(&batch);
    parse_batch(&delete);
}

#[test]
fn decimal_bindings_preserve_large_exact_spelling_without_rounding() {
    let target = target();
    let exact = "-123456789012345678901234567890123456.78";
    let batch = compile(
        &target,
        &[Mutation::Insert {
            values: vec![(0, text("x")), (1, text(exact)), (2, text("false"))],
        }],
    )
    .unwrap();
    assert_eq!(batch.units()[0].bindings()[1].value(), Some(exact));
    assert_eq!(batch.units()[0].bindings()[1].kind(), BindingKind::Decimal);
    for bad in [
        "1.001",
        "1.000",
        "1234567890123456789012345678901234567",
        "1e2",
        "+1",
        "NaN",
        "1.",
        ".1",
        " 1",
        "1;SELECT 2",
    ] {
        assert!(
            compile(
                &target,
                &[Mutation::Insert {
                    values: vec![(0, text("x")), (1, text(bad)), (2, text("true"))]
                }]
            )
            .is_err(),
            "accepted {bad}"
        );
    }
    parse_batch(&batch);
}

#[test]
fn decimal_equivalent_original_spellings_and_negative_zero_are_duplicates() {
    let target = target();
    for (left, right) in [("01", "1.0"), ("-0.00", "00.0")] {
        assert_eq!(
            compile(
                &target,
                &[
                    Mutation::Delete {
                        original: original(&target, "x", left)
                    },
                    Mutation::Delete {
                        original: original(&target, "x", right)
                    }
                ]
            )
            .unwrap_err()
            .code,
            ErrorCode::Conflict
        );
    }
    let batch = compile(
        &target,
        &[Mutation::Insert {
            values: vec![(0, text("x")), (1, text("001.20")), (2, text("true"))],
        }],
    )
    .unwrap();
    assert_eq!(batch.units()[0].bindings()[1].value(), Some("001.20"));
}

#[test]
fn text_originals_are_byte_exact_even_for_collation_equivalent_spellings() {
    let target = target();
    let batch = compile(
        &target,
        &[
            Mutation::Delete {
                original: original(&target, "é", "1"),
            },
            Mutation::Delete {
                original: original(&target, "e\u{301}", "1"),
            },
            Mutation::Delete {
                original: original(&target, "É ", "1"),
            },
        ],
    )
    .unwrap();
    for unit in batch.units() {
        assert!(unit.sql().contains("TO_BINARY(\"label\", 'UTF-8')"));
    }
    assert_eq!(batch.units()[0].bindings()[0].value(), Some("é"));
    assert_eq!(batch.units()[1].bindings()[0].value(), Some("e\u{301}"));
    parse_batch(&batch);
}

#[test]
fn duplicate_originals_and_cross_unit_postimage_overlap_are_rejected() {
    let target = target();
    let row = original(&target, "x", "1");
    assert_eq!(
        compile(
            &target,
            &[
                Mutation::Delete {
                    original: row.clone()
                },
                Mutation::Update {
                    original: row,
                    changes: vec![(0, text("y"))]
                }
            ]
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        compile(
            &target,
            &[
                Mutation::Update {
                    original: original(&target, "x", "1"),
                    changes: vec![(0, text("y"))]
                },
                Mutation::Delete {
                    original: original(&target, "y", "1")
                }
            ]
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        compile(
            &target,
            &[
                Mutation::Insert {
                    values: vec![(0, text("y")), (1, text("1.00")), (2, text("true"))]
                },
                Mutation::Delete {
                    original: original(&target, "y", "1")
                }
            ]
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
}

#[test]
fn omitted_original_columns_invalid_assignments_and_generated_writes_fail() {
    let target = target();
    assert!(OriginalRow::new(&target, vec![text("x")]).is_err());
    assert!(OriginalRow::new(&target, vec![CellValue::Default, text("1"), text("true")]).is_err());
    assert!(OriginalRow::new(&target, vec![text("x"), CellValue::Null, text("true")]).is_err());
    for changes in [
        vec![],
        vec![(0, text("x")), (0, text("y"))],
        vec![(3, text("x"))],
        vec![(2, text("TRUE"))],
    ] {
        assert!(
            compile(
                &target,
                &[Mutation::Update {
                    original: original(&target, "x", "1"),
                    changes
                }]
            )
            .is_err()
        );
    }
    assert!(
        compile(
            &target,
            &[Mutation::Insert {
                values: vec![(0, text("x"))]
            }]
        )
        .is_err()
    );
    let generated = MutationTarget::new(
        "d",
        "s",
        "t",
        vec![
            MutationColumn::new("g", ScalarType::Boolean, false, WriteMode::Generated, false)
                .unwrap(),
        ],
    )
    .unwrap();
    assert!(
        compile(
            &generated,
            &[Mutation::Insert {
                values: vec![(0, text("true"))]
            }]
        )
        .is_err()
    );
}

#[test]
fn explicit_default_and_unknown_postimage_batch_limits_fail_closed() {
    let target = target();
    assert_eq!(
        compile(
            &target,
            &[Mutation::Update {
                original: original(&target, "x", "1"),
                changes: vec![(0, CellValue::Default)]
            }]
        )
        .unwrap_err()
        .code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        compile(
            &target,
            &[
                Mutation::Insert {
                    values: vec![(1, text("2")), (2, text("true"))]
                },
                Mutation::Delete {
                    original: original(&target, "x", "1")
                }
            ]
        )
        .unwrap_err()
        .code,
        ErrorCode::Unsupported
    );
}

#[test]
fn schema_identifiers_types_and_structural_bounds_are_validated() {
    let ty = ScalarType::Text { max_bytes: 10 };
    for name in ["", " ", "a\0b", "a\nb", "x\u{202e}y"] {
        assert!(MutationColumn::new(name, ty, true, WriteMode::Writable, false).is_err());
    }
    assert!(MutationTarget::new("d", "s", "t", vec![]).is_err());
    assert!(
        MutationTarget::new(
            "d",
            "s",
            "t",
            vec![column("x", ty, true), column("x", ty, true)]
        )
        .is_err()
    );
    assert!(
        MutationTarget::new(
            "d",
            "s",
            "t",
            vec![column("x", ty, true), column("X", ty, true)]
        )
        .is_ok()
    );
    assert!(MutationColumn::new(&"雪".repeat(256), ty, true, WriteMode::Writable, false).is_err());
    for ty in [
        ScalarType::Text { max_bytes: 0 },
        ScalarType::Text {
            max_bytes: MAX_CELL_BYTES + 1,
        },
        ScalarType::Decimal {
            precision: 0,
            scale: 0,
        },
        ScalarType::Decimal {
            precision: 39,
            scale: 0,
        },
        ScalarType::Decimal {
            precision: 2,
            scale: 3,
        },
        ScalarType::Decimal {
            precision: 38,
            scale: 39,
        },
        ScalarType::Binary { max_bytes: 0 },
        ScalarType::Binary {
            max_bytes: MAX_CELL_BYTES / 2 + 1,
        },
        ScalarType::Time { scale: 10 },
        ScalarType::TimestampNtz { scale: 10 },
        ScalarType::TimestampLtz { scale: 10 },
        ScalarType::TimestampTz { scale: 10 },
    ] {
        assert!(MutationColumn::new("x", ty, true, WriteMode::Writable, false).is_err());
    }
    assert!(
        MutationColumn::new(
            "x",
            ScalarType::Decimal {
                precision: 38,
                scale: 38,
            },
            true,
            WriteMode::Writable,
            false
        )
        .is_ok()
    );
    for ty in [UnsupportedType::Vector, UnsupportedType::Unknown] {
        assert_eq!(
            MutationColumn::new(
                "x",
                ScalarType::Unsupported(ty),
                true,
                WriteMode::Writable,
                false
            )
            .unwrap_err()
            .code,
            ErrorCode::Unsupported
        );
    }
    let columns = (0..=MAX_COLUMNS)
        .map(|i| column(&format!("c{i}"), ty, true))
        .collect();
    assert!(MutationTarget::new("d", "s", "t", columns).is_err());
}

#[test]
fn row_cell_and_aggregate_byte_limits_reject_before_output() {
    let target = target();
    assert!(compile(&target, &[]).is_err());
    let insert = Mutation::Insert {
        values: vec![(0, text("x")), (1, text("1")), (2, text("true"))],
    };
    assert_eq!(
        compile(&target, &vec![insert; MAX_ROWS + 1])
            .unwrap_err()
            .code,
        ErrorCode::ResultLimitReached
    );
    assert_eq!(
        compile(
            &target,
            &[Mutation::Insert {
                values: vec![
                    (0, CellValue::Text("x".repeat(MAX_CELL_BYTES + 1))),
                    (1, text("1")),
                    (2, text("true"))
                ]
            }]
        )
        .unwrap_err()
        .code,
        ErrorCode::ResultLimitReached
    );
    let large = Mutation::Insert {
        values: vec![
            (0, CellValue::Text("x".repeat(MAX_CELL_BYTES))),
            (1, text("1")),
            (2, text("true")),
        ],
    };
    let accepted = compile(&target, std::slice::from_ref(&large)).unwrap();
    assert!(accepted.accounted_bytes() >= 2 * MAX_CELL_BYTES);
    assert_eq!(
        compile(&target, &vec![large; 4]).unwrap_err().code,
        ErrorCode::ResultLimitReached
    );
}

#[test]
fn unicode_quote_heavy_wide_schema_and_500_rows_fit_accounted_sql_capacity() {
    let name = format!("{}{}", "🦀".repeat(127), "\"".repeat(128));
    let wide_target = MutationTarget::new(
        &name,
        &name,
        &name,
        (0..MAX_COLUMNS)
            .map(|i| {
                column(
                    &format!("{i}{}", "\"".repeat(250)),
                    ScalarType::Boolean,
                    false,
                )
            })
            .collect(),
    )
    .unwrap();
    let row = OriginalRow::new(&wide_target, vec![text("true"); MAX_COLUMNS]).unwrap();
    // Wide identifier SQL is charged before emission and cannot silently grow
    // through String::with_capacity's conservative per-column allowance.
    let batch = compile(
        &wide_target,
        &[Mutation::Update {
            original: row.clone(),
            changes: vec![(0, text("false"))],
        }],
    )
    .unwrap();
    parse_batch(&batch);
    let wide_rows = (0..MAX_ROWS)
        .map(|i| {
            let mut values = row.values().to_vec();
            for (bit, value) in values.iter_mut().take(9).enumerate() {
                *value = text(if i & (1 << bit) == 0 { "false" } else { "true" });
            }
            Mutation::Delete {
                original: OriginalRow::new(&wide_target, values).unwrap(),
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        compile(&wide_target, &wide_rows).unwrap_err().code,
        ErrorCode::ResultLimitReached
    );
    let target = target();
    let rows: Vec<_> = (0..MAX_ROWS)
        .map(|i| Mutation::Delete {
            original: original(&target, &format!("row{i}"), "1"),
        })
        .collect();
    let batch = compile(&target, &rows).unwrap();
    assert_eq!(batch.units().len(), MAX_ROWS);
    parse_batch(&batch);
}

#[test]
fn scale_38_decimal_float_binary_temporal_and_json_codecs_roundtrip_sqlparser() {
    let target = MutationTarget::new(
        "db",
        "schema",
        "all_types",
        vec![
            MutationColumn::new(
                "id",
                ScalarType::Decimal {
                    precision: 38,
                    scale: 0,
                },
                false,
                WriteMode::Writable,
                true,
            )
            .unwrap(),
            column(
                "frac38",
                ScalarType::Decimal {
                    precision: 38,
                    scale: 38,
                },
                false,
            ),
            column("flt", ScalarType::Float, true),
            column("bin", ScalarType::Binary { max_bytes: 8 }, true),
            column("dt", ScalarType::Date, true),
            column("tm", ScalarType::Time { scale: 3 }, true),
            column("ntz", ScalarType::TimestampNtz { scale: 0 }, true),
            column("ltz", ScalarType::TimestampLtz { scale: 6 }, true),
            column("tz", ScalarType::TimestampTz { scale: 9 }, true),
            column("v", ScalarType::Variant, true),
            column("arr", ScalarType::Array, true),
            column("obj", ScalarType::Object, true),
        ],
    )
    .unwrap();

    let exact_38 = "-0.12345678901234567890123456789012345678";
    let orig = OriginalRow::new(
        &target,
        vec![
            text("1"),
            text(exact_38),
            text("NaN"),
            text("deadbeef"),
            text("2024-02-29"),
            text("23:59:59.123000000"),
            text("2026-11-14 12:34:56.000000000"),
            text("2026-11-14 11:34:56.123456000 +00:00"),
            text("2026-11-14 18:34:56.123456789 +07:00"),
            text("null"),
            text("[9007199254740993, \"x\"]"),
            text("{\"b\":1,\"a\":2.0}"),
        ],
    )
    .unwrap();

    // Semi-structured INSERT omits DEFAULT columns and emits INSERT ... SELECT.
    let ins_select = compile(
        &target,
        &[Mutation::Insert {
            values: vec![
                (0, CellValue::Default),
                (1, text("0.00000000000000000000000000000000000001")),
                (2, text("-inf")),
                (3, text("")),
                (4, text("2026-11-14")),
                (5, text("00:00:00")),
                (6, text("2026-11-14T00:00:00")),
                (7, text("2026-11-14T00:00:00.5-05:00")),
                (8, text("2026-11-14 00:00:00.000000001 +00:00")),
                (9, CellValue::Null),
                (10, text("[]")),
                (11, text("{}")),
            ],
        }],
    )
    .unwrap();
    let select_sql = ins_select.units()[0].sql();
    assert!(select_sql.starts_with("INSERT INTO \"db\".\"schema\".\"all_types\" (\"frac38\""));
    assert!(select_sql.contains(") SELECT "));
    assert!(!select_sql.contains("VALUES"));
    assert!(!select_sql.contains("\"id\""));
    parse_batch(&ins_select);

    // Update changing TIMESTAMP_TZ offset on the same UTC instant succeeds on one row,
    // while canonical JSON SQL NULL vs JSON "null" stay distinct.
    let upd = compile(
        &target,
        &[Mutation::Update {
            original: orig.clone(),
            changes: vec![
                (8, text("2026-11-14 11:34:56.123456789 +00:00")),
                (9, CellValue::Null),
            ],
        }],
    )
    .unwrap();
    parse_batch(&upd);

    // Equivalent canonical representations across two units are caught as overlap conflicts.
    let equivalent = OriginalRow::new(
        &target,
        vec![
            text("01"),
            text(exact_38),
            text("NaN"),
            text("DEADBEEF"),
            text("2024-02-29"),
            text("23:59:59.123"),
            text("2026-11-14T12:34:56"),
            text("2026-11-14 18:34:56.123456 +07:00"),
            text("2026-11-14 11:34:56.123456789 +00:00"),
            text(" null "),
            text("[ 9007199254740993 , \"x\" ]"),
            text("{\"a\": 2, \"b\": 1.00}"),
        ],
    )
    .unwrap();
    assert_eq!(
        compile(
            &target,
            &[
                Mutation::Delete {
                    original: orig.clone()
                },
                Mutation::Delete {
                    original: equivalent
                },
            ],
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );

    // Precision/range/type violations fail closed before SQL emission.
    for (col_idx, bad_val) in [
        (1, "1.0"),
        (1, "0.123456789012345678901234567890123456789"),
        (2, "1e999"),
        (2, "+1.0"),
        (2, "0x1p2"),
        (3, "ABC"),
        (3, "GG"),
        (3, "001122334455667788"),
        (4, "2026-02-29"),
        (4, "2026-13-01"),
        (5, "12:34:56.1234"),
        (5, "24:00:00"),
        (6, "2026-11-14 12:34:56.1"),
        (6, "2026-11-14 12:34:56 +00:00"),
        (7, "2026-11-14 12:34:56"),
        (7, "2026-11-14 12:34:56.1234567 +00:00"),
        (9, "undefined"),
        (10, "{\"not\":\"array\"}"),
        (11, "[1,2]"),
        (11, "{\"dup\":1,\"dup\":2}"),
        (9, &format!("{}{}", "[".repeat(70), "]".repeat(70))),
    ] {
        let mut bad_row = orig.values().to_vec();
        bad_row[col_idx] = text(bad_val);
        assert!(
            OriginalRow::new(&target, bad_row).is_err(),
            "accepted invalid value for col {col_idx}: {bad_val}"
        );
    }
}

#[test]
fn thousand_column_full_width_update_compiles_with_2000_binds() {
    use crate::drivers::snowflake_batch::BatchRequest;
    use serde_json::{Value, json};

    let target = MutationTarget::new(
        "db",
        "schema",
        "wide_1000",
        (0..MAX_COLUMNS)
            .map(|i| {
                column(
                    &format!("col_timestamp_tz_metric_{i:04}"),
                    ScalarType::TimestampTz { scale: 9 },
                    false,
                )
            })
            .collect(),
    )
    .unwrap();
    let original = OriginalRow::new(
        &target,
        vec![text("2026-11-14 00:00:00.000000001 +00:00"); MAX_COLUMNS],
    )
    .unwrap();
    let changes: Vec<_> = (0..MAX_COLUMNS)
        .map(|i| (i, text("2026-11-14 01:02:03.000000004 +00:00")))
        .collect();
    let batch = compile(&target, &[Mutation::Update { original, changes }]).unwrap();
    assert_eq!(batch.units()[0].bindings().len(), 2 * MAX_COLUMNS);
    assert!(batch.units()[0].sql().len() > 262_144);
    parse_batch(&batch);

    let wire_payload = json!({
        "mode": "automatic",
        "owner": Value::Null,
        "rollbackPrior": false,
        "expected": {
            "user": "ANALYST",
            "role": "ANALYST",
            "database": "db",
            "schema": "schema",
            "warehouse": "WH",
            "session": "12345",
        },
        "units": [{
            "kind": "update",
            "sql": batch.units()[0].sql(),
            "binds": batch.units()[0]
                .bindings()
                .iter()
                .map(|b| match b.value() {
                    Some(v) => Value::String(v.to_string()),
                    None => Value::Null,
                })
                .collect::<Vec<_>>(),
        }],
    });
    assert!(
        BatchRequest::parse("mutationBatch", &wire_payload)
            .unwrap()
            .is_some()
    );

    // Quote-heavy 1000-column full-width UPDATE whose SQL exceeds MAX_UNIT_SQL_BYTES fails closed at compile.
    let oversized_target = MutationTarget::new(
        "db",
        "schema",
        "oversized_1000",
        (0..MAX_COLUMNS)
            .map(|i| {
                column(
                    &format!("{i}{}", "\"".repeat(250)),
                    ScalarType::Boolean,
                    false,
                )
            })
            .collect(),
    )
    .unwrap();
    let oversized_orig =
        OriginalRow::new(&oversized_target, vec![text("false"); MAX_COLUMNS]).unwrap();
    let oversized_changes: Vec<_> = (0..MAX_COLUMNS).map(|i| (i, text("true"))).collect();
    assert_eq!(
        compile(
            &oversized_target,
            &[Mutation::Update {
                original: oversized_orig,
                changes: oversized_changes,
            }],
        )
        .unwrap_err()
        .code,
        ErrorCode::ResultLimitReached
    );
}

#[test]
fn spatial_geography_and_geometry_compile_wkt_ewkt_geojson_and_detect_cross_format_overlap() {
    let spatial_target = MutationTarget::new(
        "db",
        "schema",
        "spatial_locations",
        vec![
            column(
                "id",
                ScalarType::Decimal {
                    precision: 38,
                    scale: 0,
                },
                false,
            ),
            column("geo", ScalarType::Geography, true),
            column("geom", ScalarType::Geometry, true),
        ],
    )
    .unwrap();

    // INSERT uses SELECT with TO_GEOGRAPHY / TO_GEOMETRY
    let ins_batch = compile(
        &spatial_target,
        &[Mutation::Insert {
            values: vec![
                (0, text("1")),
                (
                    1,
                    text("{\"type\":\"Point\",\"coordinates\":[-122.4194,37.7749]}"),
                ),
                (2, text("SRID=3857;POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))")),
            ],
        }],
    )
    .unwrap();
    assert_eq!(
        ins_batch.units()[0].sql(),
        "INSERT INTO \"db\".\"schema\".\"spatial_locations\" (\"id\", \"geo\", \"geom\") SELECT CAST(? AS NUMBER(38,0)), TO_GEOGRAPHY(CAST(? AS VARCHAR)), TO_GEOMETRY(CAST(? AS VARCHAR))"
    );
    assert_eq!(
        ins_batch.units()[0].bindings()[1].kind(),
        BindingKind::Geography
    );
    assert_eq!(
        ins_batch.units()[0].bindings()[2].kind(),
        BindingKind::Geometry
    );
    parse_batch(&ins_batch);

    // UPDATE uses ST_ASEWKB(...) IS NOT DISTINCT FROM ST_ASEWKB(...)
    let orig = OriginalRow::new(
        &spatial_target,
        vec![
            text("1"),
            text("{\"type\":\"Point\",\"coordinates\":[-122.4194,37.7749]}"),
            CellValue::Null,
        ],
    )
    .unwrap();
    let upd_batch = compile(
        &spatial_target,
        &[Mutation::Update {
            original: orig.clone(),
            changes: vec![
                (1, text("SRID=4326;LINESTRING(-122.4 37.7, -122.5 37.8)")),
                (
                    2,
                    text("GEOMETRYCOLLECTION(POINT(1 2), LINESTRING(0 0, 2 2))"),
                ),
            ],
        }],
    )
    .unwrap();
    let upd_sql = upd_batch.units()[0].sql();
    assert!(upd_sql.contains("\"geo\" = TO_GEOGRAPHY(CAST(? AS VARCHAR))"));
    assert!(upd_sql.contains("\"geom\" = TO_GEOMETRY(CAST(? AS VARCHAR))"));
    assert!(upd_sql.contains(
        "ST_ASEWKB(\"geo\") IS NOT DISTINCT FROM ST_ASEWKB(TO_GEOGRAPHY(CAST(? AS VARCHAR)))"
    ));
    assert!(
        upd_sql.contains("ST_ASEWKB(\"geom\") IS NOT DISTINCT FROM ST_ASEWKB(CAST(? AS GEOMETRY))")
    );
    parse_batch(&upd_batch);

    // Cross-format canonical overlap: GeoJSON Point(-122.4194, 37.7749) == WKT POINT(-122.4194 37.7749)
    let wkt_equivalent_insert = Mutation::Insert {
        values: vec![
            (0, text("1")),
            (1, text("POINT(-122.4194 37.7749)")),
            (2, CellValue::Null),
        ],
    };
    assert_eq!(
        compile(
            &spatial_target,
            &[Mutation::Delete { original: orig }, wkt_equivalent_insert,]
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );

    // Reject out-of-range GEOGRAPHY lon/lat, non-4326 GEOGRAPHY SRID, and unclosed polygons
    for bad_geo in [
        "POINT(181 0)",
        "POINT(0 -91)",
        "SRID=3857;POINT(-122.4 37.7)",
        "POLYGON((0 0, 1 0, 1 1, 0 1))",
        "LINESTRING(0 0)",
        "{\"type\":\"Point\",\"coordinates\":[200,10]}",
    ] {
        assert_eq!(
            compile(
                &spatial_target,
                &[Mutation::Insert {
                    values: vec![(0, text("2")), (1, text(bad_geo)), (2, CellValue::Null),],
                }]
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput,
            "expected invalid spatial input for {bad_geo}"
        );
    }
}
