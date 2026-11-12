use super::*;

fn check_row(name: &str, clause: &str) -> TextRow {
    row(&["DB", "PUBLIC", "ITEM", name, clause])
}

#[test]
fn check_constraints_factory_quotes_database_and_binds_exact_retained_identity() {
    let mut target = target();
    target.database = "D\"'\\B".into();
    target.table.schema = "S'; DROP SCHEMA X;--".into();
    target.table.name = "T.with.dot".into();
    let read = metadata::check_constraints_read(&target).unwrap();
    assert_eq!(
        read.sql(),
        "SELECT CONSTRAINT_CATALOG, CONSTRAINT_SCHEMA, CONSTRAINT_TABLE, CONSTRAINT_NAME, CHECK_CLAUSE FROM \"D\"\"'\\B\".INFORMATION_SCHEMA.CHECK_CONSTRAINTS WHERE CONSTRAINT_CATALOG = ? AND CONSTRAINT_SCHEMA = ? AND CONSTRAINT_TABLE = ? ORDER BY CONSTRAINT_CATALOG, CONSTRAINT_SCHEMA, CONSTRAINT_TABLE, CONSTRAINT_NAME LIMIT 1001"
    );
    assert_eq!(
        read.binds(),
        &[
            serde_json::json!(target.database),
            serde_json::json!(target.table.schema),
            serde_json::json!(target.table.name)
        ]
    );
    assert_eq!((read.max_rows(), read.max_bytes()), (1001, 1024 * 1024));
    assert!(!read.sql().contains(&target.table.schema));
    assert!(!read.sql().contains(&target.table.name));
    target.availability = RelationAvailability::UnknownTraits;
    assert!(metadata::check_constraints_read(&target).is_err());
}

#[test]
fn check_constraints_parser_preserves_opaque_clauses_and_visible_empty_inventory() {
    let clause = "  \"quoted'雪\" > 0\nAND note <> 'DROP TABLE X;--'  ";
    let parsed = parse_check_constraints(
        &target(),
        &result(
            &CHECK_COLUMNS,
            vec![check_row("CK_ONE", clause), check_row("CK_TWO", clause)],
        ),
    )
    .unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].check_clause, clause);
    assert_eq!(parsed[1].name, "CK_TWO");
    assert!(
        parse_check_constraints(&target(), &result(&CHECK_COLUMNS, vec![]))
            .unwrap()
            .is_empty()
    );
    let mut quoted = target();
    quoted.table.schema = "Schema.\"'雪".into();
    quoted.table.name = "Table.\"'雪".into();
    let values = row(&[
        &quoted.database,
        &quoted.table.schema,
        &quoted.table.name,
        "Check.\"'雪",
        clause,
    ]);
    let parsed = parse_check_constraints(&quoted, &result(&CHECK_COLUMNS, vec![values])).unwrap();
    assert_eq!(parsed[0].schema, quoted.table.schema);
    assert_eq!(parsed[0].table_name, quoted.table.name);
    // This metadata read does not manufacture support/enforcement/validation
    // claims from object traits. Existing unavailable targets remain gated.
    for (table_type, iceberg, hybrid, dynamic) in [
        ("VIEW", false, false, false),
        ("BASE TABLE", true, false, false),
        ("BASE TABLE", false, true, false),
        ("BASE TABLE", false, false, true),
    ] {
        let mut available = target();
        available.table_type = table_type.into();
        available.traits.iceberg = Some(iceberg);
        available.traits.hybrid = Some(hybrid);
        available.traits.dynamic = Some(dynamic);
        assert_eq!(
            initial_availability(table_type, &available.traits),
            RelationAvailability::Available
        );
        assert!(metadata::check_constraints_read(&available).is_ok());
    }
}

#[test]
fn check_constraints_parser_rejects_null_wrong_identity_blank_clause_and_duplicates() {
    let valid = check_row("CK", "ID > 0");
    for index in 0..CHECK_COLUMNS.len() {
        let mut invalid = valid.clone();
        invalid[index] = None;
        assert!(
            parse_check_constraints(&target(), &result(&CHECK_COLUMNS, vec![invalid])).is_err()
        );
    }
    for (index, value) in [
        (0, "OTHER"),
        (1, "OTHER"),
        (2, "OTHER"),
        (3, ""),
        (3, "bad\0name"),
        (4, ""),
        (4, " \n\t "),
        (4, "x\0y"),
    ] {
        let mut invalid = valid.clone();
        invalid[index] = Some(value.into());
        assert!(
            parse_check_constraints(&target(), &result(&CHECK_COLUMNS, vec![invalid])).is_err()
        );
    }
    let mut invalid = valid.clone();
    invalid[3] = Some("x".repeat(256));
    assert!(parse_check_constraints(&target(), &result(&CHECK_COLUMNS, vec![invalid])).is_err());
    assert!(
        parse_check_constraints(
            &target(),
            &result(&CHECK_COLUMNS, vec![valid.clone(), valid])
        )
        .is_err()
    );
    assert!(
        parse_check_constraints(
            &target(),
            &result(
                &CHECK_COLUMNS,
                vec![check_row("SAME", "ID > 0"), check_row("SAME", "ID < 0")]
            )
        )
        .is_err()
    );
}

#[test]
fn check_constraints_parser_rejects_incomplete_shape_sentinels_and_aggregate_bytes() {
    let valid = result(&CHECK_COLUMNS, vec![check_row("CK", "ID > 0")]);
    let mut cases = Vec::new();
    let mut bad = valid.clone();
    bad.truncated = true;
    cases.push(bad);
    let mut bad = valid.clone();
    bad.rows_seen = 2;
    cases.push(bad);
    let mut bad = valid.clone();
    bad.rows[0].pop();
    cases.push(bad);
    let mut bad = valid.clone();
    bad.columns[0].name = "TABLE_NAME".into();
    cases.push(bad);
    let mut bad = valid.clone();
    bad.columns[4].kind = ColumnKind::Integer;
    cases.push(bad);
    for bad in cases {
        assert!(parse_check_constraints(&target(), &bad).is_err());
    }
    let sentinel = (0..=CHECK_LIMIT)
        .map(|i| check_row(&format!("CK_{i}"), "ID > 0"))
        .collect();
    assert!(parse_check_constraints(&target(), &result(&CHECK_COLUMNS, sentinel)).is_err());
    let huge = result(
        &CHECK_COLUMNS,
        vec![check_row("CK", &"x".repeat(CHECK_MAX_BYTES + 1))],
    );
    assert!(parse_check_constraints(&target(), &huge).is_err());
    let many_bytes = (0..20)
        .map(|i| check_row(&format!("CK_{i}"), &"x".repeat(64 * 1024)))
        .collect();
    assert!(parse_check_constraints(&target(), &result(&CHECK_COLUMNS, many_bytes)).is_err());
    // Raw cells fit, but the serialized DTO's field names also count toward
    // the aggregate response bound.
    for clause in ["x".repeat(980), "\\\"".repeat(245)] {
        let expanded = result(
            &CHECK_COLUMNS,
            (0..CHECK_LIMIT)
                .map(|i| check_row(&format!("CK_{i}"), &clause))
                .collect(),
        );
        assert!(serde_json::to_vec(&expanded).unwrap().len() < CHECK_MAX_BYTES);
        assert!(parse_check_constraints(&target(), &expanded).is_err());
    }
}
