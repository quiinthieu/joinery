use super::*;

fn key_row(name: &str, kind: &str) -> TextRow {
    let mut row = row(&[
        "DB", "PUBLIC", "ITEM", "DB", "PUBLIC", name, kind, "NO", "YES", "NO", "NO",
    ]);
    row.extend(std::iter::repeat_n(None, 10));
    row
}

fn foreign_row() -> TextRow {
    let mut row = key_row("CHILD_FK", "FOREIGN KEY");
    for (index, value) in [
        (12, "DB"),
        (13, "PUBLIC"),
        (14, "CHILD_FK"),
        (15, "PARENT_DB"),
        (16, "PARENT_SCHEMA"),
        (17, "PARENT_PK"),
        (18, "FULL"),
        (19, "NO ACTION"),
        (20, "CASCADE"),
    ] {
        row[index] = Some(value.into());
    }
    row
}

#[test]
fn constraints_sql_quotes_database_and_binds_only_the_retained_target_identity() {
    let mut target = target();
    target.database = "D\"'\\B".into();
    target.table.schema = "S'; DROP SCHEMA X;--".into();
    target.table.name = "T.with.dot".into();
    let read = metadata::constraints_read(&target).unwrap();
    assert!(
        read.sql()
            .contains("FROM \"D\"\"'\\B\".INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS TC")
    );
    assert!(
        read.sql()
            .contains("LEFT JOIN \"D\"\"'\\B\".INFORMATION_SCHEMA.REFERENTIAL_CONSTRAINTS AS RC")
    );
    assert!(read.sql().contains("TC.CONSTRAINT_TYPE = 'FOREIGN KEY' AND TC.CONSTRAINT_CATALOG = RC.CONSTRAINT_CATALOG AND TC.CONSTRAINT_SCHEMA = RC.CONSTRAINT_SCHEMA AND TC.CONSTRAINT_NAME = RC.CONSTRAINT_NAME"));
    assert!(
        read.sql()
            .contains("WHERE TC.TABLE_CATALOG = ? AND TC.TABLE_SCHEMA = ? AND TC.TABLE_NAME = ?")
    );
    assert!(read.sql().ends_with("LIMIT 1001"));
    assert!(!read.sql().contains(&target.table.schema));
    assert!(!read.sql().contains(&target.table.name));
    assert_eq!(
        read.binds(),
        &[
            serde_json::json!(target.database),
            serde_json::json!(target.table.schema),
            serde_json::json!(target.table.name)
        ]
    );
    assert_eq!((read.max_rows(), read.max_bytes()), (1001, 1024 * 1024));
    target.availability = RelationAvailability::Ambiguous;
    assert!(metadata::constraints_read(&target).is_err());
}

#[test]
fn constraints_parser_preserves_declared_flags_reference_visibility_and_empty_inventory() {
    let mut primary = key_row("ITEM_PK", "PRIMARY KEY");
    primary[11] = Some("declared 雪\ncomment".into());
    let rows = vec![primary, key_row("ITEM_UQ", "UNIQUE"), foreign_row()];
    let parsed = parse_constraints(&target(), &result(&CONSTRAINT_COLUMNS, rows)).unwrap();
    assert_eq!(
        parsed.iter().map(|c| c.kind).collect::<Vec<_>>(),
        [
            RelationKeyConstraintKind::PrimaryKey,
            RelationKeyConstraintKind::Unique,
            RelationKeyConstraintKind::ForeignKey
        ]
    );
    assert!(!parsed[0].is_deferrable && parsed[0].initially_deferred);
    assert!(!parsed[0].enforced && !parsed[0].rely);
    assert_eq!(parsed[0].comment.as_deref(), Some("declared 雪\ncomment"));
    assert_eq!(
        parsed[0].reference_status,
        ConstraintReferenceStatus::NotApplicable
    );
    assert!(parsed[0].reference.is_none());
    let reference = parsed[2].reference.as_ref().unwrap();
    assert_eq!(
        parsed[2].reference_status,
        ConstraintReferenceStatus::Available
    );
    assert_eq!(
        (&*reference.catalog, &*reference.schema, &*reference.name),
        ("PARENT_DB", "PARENT_SCHEMA", "PARENT_PK")
    );
    assert_eq!(
        (
            &*reference.match_option,
            &*reference.update_rule,
            &*reference.delete_rule
        ),
        ("FULL", "NO ACTION", "CASCADE")
    );
    let invisible = parse_constraints(
        &target(),
        &result(
            &CONSTRAINT_COLUMNS,
            vec![key_row("HIDDEN_FK", "FOREIGN KEY")],
        ),
    )
    .unwrap();
    assert_eq!(
        invisible[0].reference_status,
        ConstraintReferenceStatus::NotVisible
    );
    assert!(invisible[0].reference.is_none());
    assert!(
        parse_constraints(&target(), &result(&CONSTRAINT_COLUMNS, vec![]))
            .unwrap()
            .is_empty()
    );
    for option in ["FULL", "SIMPLE", "PARTIAL"] {
        for rule in [
            "CASCADE",
            "SET NULL",
            "SET DEFAULT",
            "RESTRICT",
            "NO ACTION",
        ] {
            let mut row = foreign_row();
            row[18] = Some(option.into());
            row[19] = Some(rule.into());
            row[20] = Some(rule.into());
            assert!(parse_constraints(&target(), &result(&CONSTRAINT_COLUMNS, vec![row])).is_ok());
        }
    }
}

#[test]
fn constraints_parser_rejects_wrong_identity_flags_kinds_duplicates_and_partial_joins() {
    let original = foreign_row();
    let mut invalid_rows = Vec::new();
    for index in 0..=10 {
        let mut changed = original.clone();
        changed[index] = None;
        invalid_rows.push(changed);
    }
    for index in 12..=20 {
        let mut changed = original.clone();
        changed[index] = None;
        invalid_rows.push(changed);
    }
    for (index, bad) in [
        (0, "OTHER"),
        (1, "OTHER"),
        (2, "OTHER"),
        (3, "OTHER"),
        (4, "OTHER"),
        (5, ""),
        (5, "bad\0name"),
        (6, "CHECK"),
        (6, "UNIQUE KEY"),
        (7, "N"),
        (8, "Y"),
        (9, "TRUE"),
        (10, "no"),
        (12, "OTHER"),
        (13, "OTHER"),
        (14, "OTHER"),
        (15, ""),
        (18, "UNKNOWN"),
        (19, "EXECUTE"),
        (20, "no action"),
    ] {
        let mut changed = original.clone();
        changed[index] = Some(bad.into());
        invalid_rows.push(changed);
    }
    let mut inappropriate_join = original.clone();
    inappropriate_join[6] = Some("UNIQUE".into());
    invalid_rows.push(inappropriate_join);
    for row in invalid_rows {
        assert!(parse_constraints(&target(), &result(&CONSTRAINT_COLUMNS, vec![row])).is_err());
    }
    assert!(
        parse_constraints(
            &target(),
            &result(&CONSTRAINT_COLUMNS, vec![original.clone(), original])
        )
        .is_err()
    );
    let primary = key_row("COPIED_NAME", "PRIMARY KEY");
    let unique = key_row("COPIED_NAME", "UNIQUE");
    assert!(
        parse_constraints(
            &target(),
            &result(&CONSTRAINT_COLUMNS, vec![primary, unique])
        )
        .is_err()
    );
}

#[test]
fn constraints_parser_rejects_incomplete_shape_sentinels_and_byte_overflow() {
    let valid = result(&CONSTRAINT_COLUMNS, vec![key_row("ITEM_PK", "PRIMARY KEY")]);
    let mut bad = valid.clone();
    bad.truncated = true;
    assert!(parse_constraints(&target(), &bad).is_err());
    let mut bad = valid.clone();
    bad.rows_seen = 2;
    assert!(parse_constraints(&target(), &bad).is_err());
    let mut bad = valid.clone();
    bad.rows[0].pop();
    assert!(parse_constraints(&target(), &bad).is_err());
    let mut bad = valid.clone();
    bad.columns[0].kind = ColumnKind::Integer;
    assert!(parse_constraints(&target(), &bad).is_err());
    let mut bad = valid.clone();
    bad.columns[0].name = "WRONG".into();
    assert!(parse_constraints(&target(), &bad).is_err());
    let many = (0..=CONSTRAINT_LIMIT)
        .map(|i| key_row(&format!("UQ_{i}"), "UNIQUE"))
        .collect();
    assert!(parse_constraints(&target(), &result(&CONSTRAINT_COLUMNS, many)).is_err());
    let mut bad = valid.clone();
    bad.rows[0][11] = Some("x".repeat(64 * 1024 + 1));
    assert!(parse_constraints(&target(), &bad).is_err());
    let many_bytes = (0..20)
        .map(|i| {
            let mut row = key_row(&format!("UQ_{i}"), "UNIQUE");
            row[11] = Some("x".repeat(64 * 1024));
            row
        })
        .collect();
    assert!(parse_constraints(&target(), &result(&CONSTRAINT_COLUMNS, many_bytes)).is_err());
}
