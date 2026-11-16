use super::*;
use crate::{
    domain::TableRef,
    relation_inspection::{ColumnIdentity, RelationTraits},
};

fn relation() -> BrowseRelation {
    BrowseRelation {
        database: "D\"B".into(),
        table: TableRef {
            schema: "S".into(),
            name: "T".into(),
        },
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
fn text() -> ColumnInspection {
    ColumnInspection {
        name: "text".into(),
        ordinal: 1,
        full_type: "VARCHAR(3)".into(),
        data_type: "TEXT".into(),
        data_type_alias: None,
        kind: InspectedColumnKind::Column,
        result_kind: ColumnKind::Text,
        nullable: true,
        default_expression: None,
        descriptor_default_expression: None,
        write_default_expression: None,
        write_default_status: WriteDefaultObservationStatus::NotReported,
        comment: None,
        character_maximum_length: Some("3".into()),
        character_octet_length: Some("12".into()),
        numeric_precision: None,
        numeric_precision_radix: None,
        numeric_scale: None,
        identity: ColumnIdentity {
            is_identity: Some(false),
            generation: None,
            start: None,
            increment: None,
            ordered: None,
        },
        expression: None,
    }
}
fn number() -> ColumnInspection {
    let mut c = text();
    c.name = "number".into();
    c.ordinal = 3;
    c.nullable = false;
    c.full_type = "NUMBER(38,2)".into();
    c.data_type = "NUMBER".into();
    c.result_kind = ColumnKind::Decimal;
    c.character_maximum_length = None;
    c.character_octet_length = None;
    c.numeric_precision = Some("38".into());
    c.numeric_precision_radix = Some("10".into());
    c.numeric_scale = Some("2".into());
    c
}
fn boolean() -> ColumnInspection {
    let mut c = text();
    c.name = "boolean".into();
    c.ordinal = 8;
    c.full_type = "BOOLEAN".into();
    c.data_type = "BOOLEAN".into();
    c.result_kind = ColumnKind::Boolean;
    c.character_maximum_length = None;
    c.character_octet_length = None;
    c
}
fn target() -> InspectedMutationTarget {
    InspectedMutationTarget::from_inspection(&relation(), &[text(), number(), boolean()]).unwrap()
}
fn row(text: Option<&str>, number: &str, boolean: Option<&str>) -> TextRow {
    vec![
        text.map(str::to_owned),
        Some(number.into()),
        boolean.map(str::to_owned),
    ]
}

#[test]
fn exact_scalars_null_empty_and_all_original_columns_survive_compilation() {
    let target = target();
    let original = target
        .original(&row(Some(""), "9007199254740993.12", Some("true")))
        .unwrap();
    let compiled = target
        .compile(&[Mutation::Update {
            original,
            changes: vec![(0, CellValue::Null)],
        }])
        .unwrap();
    let unit = &compiled.units()[0];
    assert!(unit.sql().starts_with("UPDATE \"D\"\"B\".\"S\".\"T\" SET"));
    assert_eq!(unit.expected_rows(), 1);
    assert_eq!(unit.sql().matches("IS NOT DISTINCT FROM").count(), 3);
    assert_eq!(
        unit.bindings()
            .iter()
            .map(|b| b.value())
            .collect::<Vec<_>>(),
        vec![None, Some(""), Some("9007199254740993.12"), Some("true")]
    );
    assert!(target.original(&row(None, "0.00", None)).is_ok());
    assert!(target.original(&vec![None, None, None]).is_err());
}

#[test]
fn unicode_character_and_octet_limits_apply_to_originals_and_assignments() {
    let target = target();
    assert!(target.original(&row(Some("ééé"), "1", None)).is_ok());
    assert!(target.original(&row(Some("abcd"), "1", None)).is_err());
    assert!(
        target
            .compile(&[Mutation::Insert {
                values: vec![
                    (0, CellValue::Text("abcd".into())),
                    (1, CellValue::Text("1".into()))
                ]
            }])
            .is_err()
    );
    let original = target.original(&row(Some("a"), "1", None)).unwrap();
    assert!(
        target
            .compile(&[Mutation::Update {
                original,
                changes: vec![(0, CellValue::Text("abcd".into()))]
            }])
            .is_err()
    );
    // Direct OriginalRow construction validates the compiler's octet bound,
    // but cannot bypass the retained inspection's extra character dimension.
    let original = OriginalRow::new(
        target.target(),
        vec![
            CellValue::Text("abcd".into()),
            CellValue::Text("1".into()),
            CellValue::Null,
        ],
    )
    .unwrap();
    assert!(target.compile(&[Mutation::Delete { original }]).is_err());
    let mut narrow = text();
    narrow.character_octet_length = Some("3".into());
    let narrow = InspectedMutationTarget::from_inspection(&relation(), &[narrow]).unwrap();
    assert!(narrow.original(&vec![Some("éé".into())]).is_err());
    assert!(narrow.original(&vec![Some("abc".into())]).is_ok());
}

#[test]
fn wide_schema_text_keeps_the_compiler_cell_budget() {
    let mut c = text();
    c.full_type = "VARCHAR(16777216)".into();
    c.character_maximum_length = Some("16777216".into());
    c.character_octet_length = Some("16777216".into());
    let target = InspectedMutationTarget::from_inspection(&relation(), &[c]).unwrap();
    assert_eq!(
        target.target().columns()[0].data_type(),
        ScalarType::Text {
            max_bytes: mutations::MAX_CELL_BYTES
        }
    );
    assert!(
        target
            .original(&vec![Some("x".repeat(mutations::MAX_CELL_BYTES + 1))])
            .is_err()
    );
}

#[test]
fn exact_decimal_precision_scale_and_boolean_grammar_never_coerce() {
    let target = target();
    let max = format!("{}.99", "9".repeat(36));
    assert!(target.original(&row(None, &max, Some("false"))).is_ok());
    for value in ["1e3", "NaN", "+1", "1.", ".1", "1.001", " 1", "1,2"] {
        assert!(target.original(&row(None, value, None)).is_err(), "{value}");
    }
    assert!(target.original(&row(None, &"9".repeat(37), None)).is_err());
    for value in ["TRUE", "1", "0", " true", "yes", ""] {
        assert!(target.original(&row(None, "1", Some(value))).is_err());
    }
}

#[test]
fn mismatched_hostile_dimensions_and_result_kinds_are_rejected() {
    let corrupt_text: &[fn(&mut ColumnInspection)] = &[
        |c| c.full_type = "VARCHAR(3); DROP TABLE T".into(),
        |c| c.full_type = "VARCHAR(3) COLLATE 'en'".into(),
        |c| c.full_type = "VARCHAR(4)".into(),
        |c| c.full_type = "VARCHAR(3,4)".into(),
        |c| c.character_maximum_length = Some("03".into()),
        |c| c.character_octet_length = Some("2".into()),
        |c| c.character_octet_length = Some("13".into()),
        |c| c.character_octet_length = Some("99999999999999999999999999999999999999".into()),
        |c| c.character_octet_length = None,
        |c| c.numeric_precision = Some("1".into()),
        |c| c.result_kind = ColumnKind::Decimal,
        |c| c.data_type_alias = Some("FLOAT".into()),
        |c| c.name = "bad\0name".into(),
    ];
    for corrupt in corrupt_text {
        let mut c = text();
        corrupt(&mut c);
        assert!(InspectedMutationTarget::from_inspection(&relation(), &[c]).is_err());
    }
    for corrupt in [
        |c: &mut ColumnInspection| c.numeric_precision_radix = Some("2".into()),
        |c: &mut ColumnInspection| c.numeric_scale = Some("3".into()),
        |c: &mut ColumnInspection| c.result_kind = ColumnKind::Integer,
        |c: &mut ColumnInspection| c.character_octet_length = Some("1".into()),
    ] {
        let mut c = number();
        corrupt(&mut c);
        assert!(InspectedMutationTarget::from_inspection(&relation(), &[c]).is_err());
    }
    let mut c = number();
    c.full_type = "NUMBER(38,38)".into();
    c.numeric_scale = Some("38".into());
    let scale38 = InspectedMutationTarget::from_inspection(&relation(), &[c]).unwrap();
    assert!(
        scale38
            .original(&vec![Some(
                "-0.12345678901234567890123456789012345678".into()
            )])
            .is_ok()
    );
    assert!(scale38.original(&vec![Some("1.0".into())]).is_err());
}

#[test]
fn generated_identity_and_expression_values_remain_predicates_but_not_assignments() {
    let mut identity = number();
    identity.full_type = "NUMBER(38,0)".into();
    identity.numeric_scale = Some("0".into());
    identity.result_kind = ColumnKind::Integer;
    identity.identity = ColumnIdentity {
        is_identity: Some(true),
        generation: Some("BY DEFAULT".into()),
        start: Some("1".into()),
        increment: Some("1".into()),
        ordered: Some(true),
    };
    let mut expression = text();
    expression.kind = InspectedColumnKind::VirtualColumn;
    expression.expression = Some("'v'".into());
    expression.ordinal = 9;
    let target =
        InspectedMutationTarget::from_inspection(&relation(), &[identity, expression]).unwrap();
    let original = target
        .original(&vec![Some("9007199254740993".into()), Some("v".into())])
        .unwrap();
    let delete = target
        .compile(&[Mutation::Delete {
            original: original.clone(),
        }])
        .unwrap();
    assert_eq!(
        delete.units()[0]
            .sql()
            .matches("IS NOT DISTINCT FROM")
            .count(),
        2
    );
    for index in [0, 1] {
        assert!(
            target
                .compile(&[Mutation::Update {
                    original: original.clone(),
                    changes: vec![(index, CellValue::Text("1".into()))]
                }])
                .is_err()
        );
        assert!(
            target
                .compile(&[Mutation::Insert {
                    values: vec![(index, CellValue::Text("1".into()))]
                }])
                .is_err()
        );
    }
}

#[test]
fn ambiguous_defaults_are_unsupported_and_default_is_never_original_text() {
    let mut c = text();
    c.nullable = false;
    c.full_type = "VARCHAR(64)".into();
    c.character_maximum_length = Some("64".into());
    c.character_octet_length = Some("256".into());
    c.default_expression = Some("'default; not SQL from the client'".into());
    c.descriptor_default_expression = c.default_expression.clone();
    let target = InspectedMutationTarget::from_inspection(&relation(), &[c.clone()]).unwrap();
    let insert = target
        .compile(&[Mutation::Insert {
            values: vec![(0, CellValue::Default)],
        }])
        .unwrap();
    assert!(insert.units()[0].sql().ends_with("VALUES (DEFAULT)"));
    assert!(!insert.units()[0].sql().contains("default;"));
    assert!(insert.units()[0].bindings().is_empty());
    let literal = target.original(&vec![Some("DEFAULT".into())]).unwrap();
    assert_eq!(literal.values(), &[CellValue::Text("DEFAULT".into())]);
    assert!(OriginalRow::new(target.target(), vec![CellValue::Default]).is_err());
    let original = target.original(&vec![Some("x".into())]).unwrap();
    assert!(
        target
            .compile(&[Mutation::Update {
                original,
                changes: vec![(0, CellValue::Default)]
            }])
            .is_err()
    );
    for corrupt in [
        |c: &mut ColumnInspection| {
            c.default_expression = Some("".into());
            c.descriptor_default_expression = Some("".into());
        },
        |c: &mut ColumnInspection| {
            c.write_default_status = WriteDefaultObservationStatus::Available;
            c.write_default_expression = Some("'other'".into());
        },
    ] {
        let mut c = c.clone();
        corrupt(&mut c);
        assert_eq!(
            InspectedMutationTarget::from_inspection(&relation(), &[c])
                .err()
                .unwrap()
                .code,
            ErrorCode::Unsupported
        );
    }
    c.write_default_status = WriteDefaultObservationStatus::Available;
    assert!(InspectedMutationTarget::from_inspection(&relation(), &[c.clone()]).is_ok());
    c.descriptor_default_expression = None;
    assert_eq!(
        InspectedMutationTarget::from_inspection(&relation(), &[c.clone()])
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidInput
    );
    c.descriptor_default_expression = c.default_expression.clone();
    c.write_default_status = WriteDefaultObservationStatus::NotReported;
    c.write_default_expression = Some("''".into());
    assert_eq!(
        InspectedMutationTarget::from_inspection(&relation(), &[c])
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidInput
    );
}

#[test]
fn unsupported_relation_and_type_shapes_do_not_become_writable() {
    for kind in ["VIEW", "EXTERNAL TABLE", "EVENT TABLE", "MATERIALIZED VIEW"] {
        let mut r = relation();
        r.table_type = kind.into();
        assert!(InspectedMutationTarget::from_inspection(&r, &[text()]).is_err());
    }
    for corrupt in [
        |r: &mut BrowseRelation| r.traits.iceberg = Some(true),
        |r: &mut BrowseRelation| r.traits.dynamic = Some(true),
        |r: &mut BrowseRelation| r.traits.hybrid = Some(true),
        |r: &mut BrowseRelation| r.traits.immutable = Some(true),
        |r: &mut BrowseRelation| r.traits.transient = None,
        |r: &mut BrowseRelation| r.availability = RelationAvailability::Shadowed,
    ] {
        let mut r = relation();
        corrupt(&mut r);
        assert!(InspectedMutationTarget::from_inspection(&r, &[text()]).is_err());
    }
    let mut r = relation();
    r.table_type = "TEMPORARY TABLE".into();
    r.traits.temporary = Some(true);
    assert!(InspectedMutationTarget::from_inspection(&r, &[text()]).is_ok());
    for kind in ["VECTOR", "FUTURE_TYPE"] {
        let mut c = text();
        c.data_type = kind.into();
        c.full_type = kind.into();
        assert_eq!(
            InspectedMutationTarget::from_inspection(&relation(), &[c])
                .err()
                .unwrap()
                .code,
            ErrorCode::Unsupported
        );
    }
    for kind in ["GEOGRAPHY", "GEOMETRY"] {
        let mut ok_spatial = text();
        ok_spatial.data_type = kind.into();
        ok_spatial.full_type = kind.into();
        ok_spatial.character_maximum_length = None;
        ok_spatial.character_octet_length = None;
        assert!(InspectedMutationTarget::from_inspection(&relation(), &[ok_spatial]).is_ok());

        let mut bad_spatial = text();
        bad_spatial.data_type = kind.into();
        bad_spatial.full_type = kind.into();
        bad_spatial.result_kind = ColumnKind::Json;
        assert_eq!(
            InspectedMutationTarget::from_inspection(&relation(), &[bad_spatial])
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidInput
        );
    }
    for kind in [
        "FLOAT",
        "BINARY",
        "DATE",
        "TIME",
        "TIMESTAMP_NTZ",
        "TIMESTAMP_LTZ",
        "TIMESTAMP_TZ",
        "VARIANT",
        "ARRAY",
        "OBJECT",
    ] {
        // Supported families still reject mismatched ColumnKind::Text / char limits.
        let mut c = text();
        c.data_type = kind.into();
        c.full_type = kind.into();
        assert_eq!(
            InspectedMutationTarget::from_inspection(&relation(), &[c])
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidInput
        );
    }
    let mut c = text();
    c.kind = InspectedColumnKind::PartitionColumn;
    assert!(InspectedMutationTarget::from_inspection(&relation(), &[c]).is_err());
    let mut c = text();
    c.identity.is_identity = None;
    assert!(InspectedMutationTarget::from_inspection(&relation(), &[c]).is_err());
    let mut c = text();
    c.identity.start = Some("1".into());
    assert!(InspectedMutationTarget::from_inspection(&relation(), &[c]).is_err());
}

#[test]
fn ordering_width_and_compiler_overlap_guards_are_preserved() {
    assert!(InspectedMutationTarget::from_inspection(&relation(), &[]).is_err());
    assert!(InspectedMutationTarget::from_inspection(&relation(), &[text(), text()]).is_err());
    let mut duplicate = text();
    duplicate.ordinal = 2;
    assert!(InspectedMutationTarget::from_inspection(&relation(), &[text(), duplicate]).is_err());
    let target = target();
    assert!(target.original(&vec![]).is_err());
    assert!(target.original(&vec![None; 4]).is_err());
    let original = target.original(&row(None, "1", None)).unwrap();
    assert!(
        target
            .compile(&[
                Mutation::Delete {
                    original: original.clone()
                },
                Mutation::Delete { original }
            ])
            .is_err()
    );
    assert!(
        target
            .compile(&[Mutation::Insert {
                values: vec![(usize::MAX, CellValue::Null)]
            }])
            .is_err()
    );
}

#[test]
fn inspection_and_row_budgets_reject_before_unbounded_cloning() {
    assert_eq!(
        InspectedMutationTarget::from_inspection(
            &relation(),
            &vec![text(); mutations::MAX_COLUMNS + 1]
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::ResultLimitReached
    );
    let columns: Vec<_> = (0..mutations::MAX_COLUMNS)
        .map(|index| {
            let mut column = text();
            column.name = format!("column{index}");
            column.ordinal = index as u32 + 1;
            column.comment = Some("x".repeat(64 * 1024));
            column
        })
        .collect();
    assert_eq!(
        InspectedMutationTarget::from_inspection(&relation(), &columns)
            .err()
            .unwrap()
            .code,
        ErrorCode::ResultLimitReached
    );
    let mut bad_identity = number();
    bad_identity.full_type = "NUMBER(38,0)".into();
    bad_identity.numeric_scale = Some("0".into());
    bad_identity.identity = ColumnIdentity {
        is_identity: Some(true),
        generation: Some("BY DEFAULT".into()),
        start: Some("1".into()),
        increment: Some("0".into()),
        ordered: Some(false),
    };
    assert_eq!(
        InspectedMutationTarget::from_inspection(&relation(), &[bad_identity])
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidInput
    );
}

#[test]
fn float_binary_temporal_json_and_1000_column_inspections_compile_and_guard_bounds() {
    let base = |name: &str, ordinal: u32, data_type: &str, full_type: &str, kind: ColumnKind| {
        let mut c = boolean();
        c.name = name.into();
        c.ordinal = ordinal;
        c.data_type = data_type.into();
        c.full_type = full_type.into();
        c.result_kind = kind;
        c
    };
    let mut flt_radix2 = base("flt2", 2, "FLOAT", "FLOAT", ColumnKind::Decimal);
    flt_radix2.numeric_precision = Some("53".into());
    flt_radix2.numeric_precision_radix = Some("2".into());
    let mut bin4 = base("bin4", 3, "BINARY", "BINARY(4)", ColumnKind::Binary);
    bin4.character_maximum_length = Some("4".into());
    bin4.character_octet_length = Some("4".into());
    let mut bin_max = base(
        "bin_max",
        4,
        "VARBINARY",
        "BINARY(8388608)",
        ColumnKind::Binary,
    );
    bin_max.character_maximum_length = Some("8388608".into());
    bin_max.character_octet_length = Some("8388608".into());
    let mut var_sized = base("var_sized", 10, "VARIANT", "VARIANT", ColumnKind::Json);
    var_sized.character_maximum_length = Some("16777216".into());
    var_sized.character_octet_length = Some("16777216".into());

    let cols = vec![
        base("flt", 1, "FLOAT", "FLOAT", ColumnKind::Decimal),
        flt_radix2,
        bin4,
        bin_max,
        base("dt", 5, "DATE", "DATE", ColumnKind::Temporal),
        base("tm", 6, "TIME", "TIME(3)", ColumnKind::Temporal),
        base(
            "ntz",
            7,
            "DATETIME",
            "TIMESTAMP_NTZ(0)",
            ColumnKind::Temporal,
        ),
        base(
            "ltz",
            8,
            "TIMESTAMP_LTZ",
            "TIMESTAMP_LTZ(6)",
            ColumnKind::Temporal,
        ),
        base(
            "tz",
            9,
            "TIMESTAMP_TZ",
            "TIMESTAMP_TZ(9)",
            ColumnKind::Temporal,
        ),
        var_sized,
        base("arr", 11, "ARRAY", "ARRAY", ColumnKind::Json),
        base("obj", 12, "OBJECT", "OBJECT", ColumnKind::Json),
    ];
    let inspected = InspectedMutationTarget::from_inspection(&relation(), &cols).unwrap();
    let orig = inspected
        .original(&vec![
            Some("1.5".into()),
            Some("NaN".into()),
            Some("01020304".into()),
            Some("".into()),
            Some("2026-11-14".into()),
            Some("12:00:00.123000000".into()),
            Some("2026-11-14 12:00:00.000000000".into()),
            Some("2026-11-14 12:00:00.123456000 +00:00".into()),
            Some("2026-11-14 19:00:00.123456789 +07:00".into()),
            Some("null".into()),
            Some("[1,2]".into()),
            Some("{\"k\":true}".into()),
        ])
        .unwrap();
    let batch = inspected
        .compile(&[Mutation::Update {
            original: orig,
            changes: vec![
                (2, CellValue::Text("deadbeef".into())),
                (9, CellValue::Null),
            ],
        }])
        .unwrap();
    assert_eq!(batch.units().len(), 1);

    // 5 binary bytes ("0102030405") exceeds BINARY(4) even though text length is 10.
    let mut bad_bin_row = vec![None; cols.len()];
    bad_bin_row[2] = Some("0102030405".into());
    assert!(inspected.original(&bad_bin_row).is_err());

    // 1,000-column table inspection succeeds within metadata budget.
    let wide_cols: Vec<_> = (0..mutations::MAX_COLUMNS)
        .map(|i| {
            let mut c = boolean();
            c.name = format!("c{i}");
            c.ordinal = i as u32 + 1;
            c
        })
        .collect();
    assert!(InspectedMutationTarget::from_inspection(&relation(), &wide_cols).is_ok());
}
