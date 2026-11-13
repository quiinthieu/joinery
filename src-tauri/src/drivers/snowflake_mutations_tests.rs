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
            precision: 39,
            scale: 0,
        },
        ScalarType::Decimal {
            precision: 2,
            scale: 3,
        },
        ScalarType::Decimal {
            precision: 38,
            scale: 38,
        },
    ] {
        assert!(MutationColumn::new("x", ty, true, WriteMode::Writable, false).is_err());
    }
    for ty in [
        UnsupportedType::Float,
        UnsupportedType::Binary,
        UnsupportedType::Date,
        UnsupportedType::Time,
        UnsupportedType::Timestamp,
        UnsupportedType::Variant,
        UnsupportedType::Array,
        UnsupportedType::Object,
        UnsupportedType::Geography,
        UnsupportedType::Geometry,
        UnsupportedType::Unknown,
    ] {
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
