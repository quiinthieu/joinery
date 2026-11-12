fn target() -> BrowseRelation {
    BrowseRelation {
        database: "DB".into(),
        table: TableRef {
            schema: "PUBLIC".into(),
            name: "ITEM".into(),
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
        created: Some("created".into()),
        last_ddl: Some("ddl".into()),
    }
}
fn declaration(name: &str, kind: RelationKeyConstraintKind) -> RelationKeyConstraint {
    RelationKeyConstraint {
        catalog: "DB".into(),
        schema: "PUBLIC".into(),
        name: name.into(),
        kind,
        is_deferrable: false,
        initially_deferred: true,
        enforced: false,
        rely: false,
        comment: None,
        reference: None,
        reference_status: if kind == RelationKeyConstraintKind::ForeignKey {
            ConstraintReferenceStatus::NotVisible
        } else {
            ConstraintReferenceStatus::NotApplicable
        },
    }
}
fn row(values: &[&str]) -> crate::domain::TextRow {
    values.iter().map(|v| Some((*v).into())).collect()
}
fn result(foreign: bool, rows: Vec<crate::domain::TextRow>) -> MetadataResult {
    MetadataResult {
        columns: (if foreign {
            FOREIGN_COLUMNS.as_slice()
        } else {
            KEY_COLUMNS.as_slice()
        })
        .iter()
        .map(|name| ColumnInfo {
            id: (*name).into(),
            name: (*name).into(),
            database_type: "TEXT".into(),
            kind: ColumnKind::Text,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        })
        .collect(),
        rows_seen: rows.len() as u64,
        rows,
        truncated: false,
    }
}
fn key(name: &str, column: &str, ordinal: &str) -> crate::domain::TextRow {
    row(&["DB", "PUBLIC", "ITEM", column, ordinal, name])
}
fn foreign(column: &str, ordinal: &str, referenced: &str) -> crate::domain::TextRow {
    row(&[
        "DB",
        "PUBLIC",
        "ITEM",
        column,
        ordinal,
        "FK",
        "PARENT_DB",
        "PARENT_SCHEMA",
        "PARENT_TABLE",
        referenced,
        "PARENT_KEY",
    ])
}

#[test]
fn mappings_factories_quote_exact_target_use_closed_pipes_and_empty_binds() {
    let mut t = target();
    t.database = "D\"B".into();
    t.table.schema = "S';drop table X;--".into();
    t.table.name = "A.B".into();
    for (spec, kind) in metadata::mapping_reads(&t)
        .unwrap()
        .into_iter()
        .zip(["PRIMARY", "UNIQUE", "IMPORTED"])
    {
        assert!(spec.sql().starts_with(&format!("SHOW {kind} KEYS IN TABLE \"D\"\"B\".\"S';drop table X;--\".\"A.B\" ->> SELECT TO_VARCHAR(\"")));
        assert!(
            spec.sql()
                .contains("TO_VARCHAR(\"key_sequence\") AS KEY_SEQUENCE")
        );
        assert!(spec.sql().contains(" FROM $1 ORDER BY "));
        assert!(spec.sql().ends_with("LIMIT 1001"));
        assert!(spec.binds().is_empty());
        assert_eq!((spec.max_rows(), spec.max_bytes()), (1001, 1024 * 1024));
    }
    for dynamic in [Some(true), None] {
        t.traits.dynamic = dynamic;
        assert!(metadata::mapping_reads(&t).is_err());
    }
}

#[test]
fn mappings_pair_composite_ordinals_numerically_and_keep_independent_reference_visibility() {
    let declarations = [
        declaration("PK", RelationKeyConstraintKind::PrimaryKey),
        declaration("UQ", RelationKeyConstraintKind::Unique),
        declaration("FK", RelationKeyConstraintKind::ForeignKey),
    ];
    let primary = result(false, vec![key("PK", "B", "2"), key("PK", "A", "1")]);
    let unique = result(false, vec![key("UQ", "U", "1")]);
    let foreign = result(
        true,
        vec![foreign("CHILD_B", "2", "B"), foreign("CHILD_A", "1", "A")],
    );
    let output = parse_mappings(&target(), &declarations, [&primary, &unique, &foreign]).unwrap();
    assert!(
        output
            .iter()
            .all(|m| m.status == ConstraintMappingStatus::Available)
    );
    assert_eq!(
        output[0]
            .columns
            .iter()
            .map(|c| c.column_name.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    assert!(
        output[0]
            .columns
            .iter()
            .all(|c| c.referenced_column_name.is_none())
    );
    assert!(output[0].referenced_table.is_none());
    assert_eq!(
        output[2].columns[0].referenced_column_name.as_deref(),
        Some("A")
    );
    assert_eq!(
        output[2].referenced_table.as_ref().unwrap().database,
        "PARENT_DB"
    );
    let empty = result(false, vec![]);
    let empty_fk = result(true, vec![]);
    let hidden = parse_mappings(&target(), &declarations, [&empty, &empty, &empty_fk]).unwrap();
    assert!(
        hidden
            .iter()
            .all(|m| m.status == ConstraintMappingStatus::NotVisible
                && m.columns.is_empty()
                && m.referenced_table.is_none())
    );
    assert!(
        unsupported_mappings(&declarations)
            .iter()
            .all(|m| m.status == ConstraintMappingStatus::Unsupported
                && m.columns.is_empty()
                && m.referenced_table.is_none())
    );
    assert!(
        parse_mappings(&target(), &[], [&empty, &empty, &empty_fk])
            .unwrap()
            .is_empty()
    );
    let rows = (1..=12)
        .rev()
        .map(|i| key("PK", &format!("C{i}"), &i.to_string()))
        .collect();
    let numeric = result(false, rows);
    assert_eq!(
        parse_mappings(&target(), &declarations, [&numeric, &empty, &empty_fk]).unwrap()[0].columns
            [9]
        .ordinal,
        10
    );
}

#[test]
fn mappings_reject_identity_orphan_duplicate_gap_bad_ordinal_and_reference_conflicts() {
    let declarations = [
        declaration("PK", RelationKeyConstraintKind::PrimaryKey),
        declaration("FK", RelationKeyConstraintKind::ForeignKey),
    ];
    let empty = result(false, vec![]);
    let empty_fk = result(true, vec![]);
    let mut bad = vec![
        vec![key("ORPHAN", "A", "1")],
        vec![key("PK", "A", "2")],
        vec![key("PK", "A", "1"), key("PK", "B", "1")],
        vec![key("PK", "A", "1"), key("PK", "A", "2")],
    ];
    for ordinal in ["0", "-1", "1.0", "01", "+1", "4294967296", ""] {
        bad.push(vec![key("PK", "A", ordinal)]);
    }
    for i in 0..6 {
        let mut changed = key("PK", "A", "1");
        changed[i] = None;
        bad.push(vec![changed]);
    }
    let mut changed = key("PK", "A", "1");
    changed[2] = Some("OTHER".into());
    bad.push(vec![changed]);
    for rows in bad {
        assert!(
            parse_mappings(
                &target(),
                &declarations,
                [&result(false, rows), &empty, &empty_fk]
            )
            .is_err()
        );
    }
    let primary = result(false, vec![key("PK", "A", "1")]);
    let mut bad_fk = vec![vec![foreign("A", "1", "X"), foreign("B", "2", "X")]];
    for i in 0..11 {
        let mut changed = foreign("A", "1", "X");
        changed[i] = None;
        bad_fk.push(vec![changed]);
    }
    for i in [6, 7, 8, 10] {
        let mut changed = foreign("B", "2", "Y");
        changed[i] = Some("DIFFERENT".into());
        bad_fk.push(vec![foreign("A", "1", "X"), changed]);
    }
    for rows in bad_fk {
        assert!(
            parse_mappings(
                &target(),
                &declarations,
                [&primary, &empty, &result(true, rows)]
            )
            .is_err()
        );
    }
    let mut visible = declarations.clone();
    visible[1].reference = Some(RelationConstraintReference {
        catalog: "PARENT_DB".into(),
        schema: "PARENT_SCHEMA".into(),
        name: "PARENT_KEY".into(),
        match_option: "FULL".into(),
        update_rule: "NO ACTION".into(),
        delete_rule: "NO ACTION".into(),
    });
    let fk = result(true, vec![foreign("A", "1", "X")]);
    assert!(parse_mappings(&target(), &visible, [&primary, &empty, &fk]).is_ok());
    for i in 0..3 {
        let mut declarations = visible.clone();
        let reference = declarations[1].reference.as_mut().unwrap();
        match i {
            0 => reference.catalog = "BAD".into(),
            1 => reference.schema = "BAD".into(),
            _ => reference.name = "BAD".into(),
        };
        assert!(parse_mappings(&target(), &declarations, [&primary, &empty, &fk]).is_err());
    }
    assert!(parse_mappings(&target(), &declarations, [&empty, &primary, &empty_fk]).is_err());
}

#[test]
fn mappings_reject_incomplete_shapes_sentinel_and_aggregate_output_overflow() {
    let declarations = [declaration("PK", RelationKeyConstraintKind::PrimaryKey)];
    let empty = result(false, vec![]);
    let empty_fk = result(true, vec![]);
    let valid = result(false, vec![key("PK", "A", "1")]);
    let mut bad = vec![];
    let mut v = valid.clone();
    v.truncated = true;
    bad.push(v);
    let mut v = valid.clone();
    v.rows_seen = 2;
    bad.push(v);
    let mut v = valid.clone();
    v.columns[0].kind = ColumnKind::Integer;
    bad.push(v);
    let mut v = valid.clone();
    v.columns[0].name = "bad".into();
    bad.push(v);
    let mut v = valid.clone();
    v.rows[0].pop();
    bad.push(v);
    bad.push(result(
        false,
        (0..=CONSTRAINT_LIMIT)
            .map(|_| key("PK", "A", "1"))
            .collect(),
    ));
    for value in bad {
        assert!(parse_mappings(&target(), &declarations, [&value, &empty, &empty_fk]).is_err());
    }
    let columns = (1..=1000)
        .map(|i| format!("{}{:04}", "雪".repeat(250), i))
        .collect::<Vec<_>>();
    let primary = result(
        false,
        columns
            .iter()
            .enumerate()
            .map(|(i, c)| key("PK", c, &(i + 1).to_string()))
            .collect(),
    );
    let unique = result(
        false,
        columns
            .iter()
            .enumerate()
            .map(|(i, c)| key("UQ", c, &(i + 1).to_string()))
            .collect(),
    );
    let declarations = [
        declaration("PK", RelationKeyConstraintKind::PrimaryKey),
        declaration("UQ", RelationKeyConstraintKind::Unique),
    ];
    assert!(parse_mappings(&target(), &declarations, [&primary, &empty, &empty_fk]).is_ok());
    assert!(parse_mappings(&target(), &declarations, [&empty, &unique, &empty_fk]).is_ok());
    assert!(parse_mappings(&target(), &declarations, [&primary, &unique, &empty_fk]).is_err());
}
