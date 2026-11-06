use joinery_core::domain::*;
use joinery_core::drivers::RowMutation;
use joinery_core::drivers::snowflake::values::*;

fn create_composite_pk_schema() -> TableSchema {
    TableSchema {
        table: TableRef {
            schema: "APP_DATA".into(),
            name: "TENANT_METRICS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "tenant_id".into(),
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
                name: "dept_id".into(),
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
                id: "c2".into(),
                name: "emp_id".into(),
                database_type: "VARCHAR(64)".into(),
                kind: ColumnKind::Text,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c3".into(),
                name: "performance_score".into(),
                database_type: "NUMBER(5,2)".into(),
                kind: ColumnKind::Decimal,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
            ColumnInfo {
                id: "c4".into(),
                name: "notes".into(),
                database_type: "VARCHAR(1000)".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["tenant_id".into(), "dept_id".into(), "emp_id".into()],
        foreign_keys: Vec::new(),
        inbound_foreign_keys: Vec::new(),
        editable: true,
    }
}

// =========================================================================
// 1. Optimistic Concurrency & Composite Primary Key Tests
// =========================================================================

#[test]
fn test_composite_pk_with_three_columns_joins_with_and_preserves_binding_order() {
    let schema = create_composite_pk_schema();
    let mutation = RowMutation::Update {
        original: vec![
            Some("1001".into()),
            Some("42".into()),
            Some("EMP-999".into()),
            Some("88.50".into()),
            Some("Senior Engineer".into()),
        ],
        changes: vec![(3, Some("92.00".into()))],
    };

    let (sql, vals) = mutation_sql(&schema, &mutation).unwrap();

    // Must update performance_score
    assert!(sql.contains(
        "UPDATE \"APP_DATA\".\"TENANT_METRICS\" SET \"performance_score\" = CAST(? AS NUMBER) WHERE"
    ));

    // Must combine all 3 PKs in WHERE with AND
    assert!(sql.contains("\"tenant_id\" = CAST(? AS NUMBER(38,0))"));
    assert!(sql.contains("\"dept_id\" = CAST(? AS NUMBER(38,0))"));
    assert!(sql.contains("\"emp_id\" = CAST(? AS VARCHAR(64))"));

    // Must check non-PK columns with EQUAL_NULL
    assert!(sql.contains("EQUAL_NULL(\"performance_score\", CAST(? AS NUMBER))"));
    assert!(sql.contains("EQUAL_NULL(\"notes\", CAST(? AS VARCHAR))"));

    // Value bindings order: SET values first, then PKs, then non-PKs
    assert_eq!(
        vals,
        vec![
            Some("92.00".into()),           // SET performance_score
            Some("1001".into()),            // PK 1 tenant_id
            Some("42".into()),              // PK 2 dept_id
            Some("EMP-999".into()),         // PK 3 emp_id
            Some("88.50".into()),           // EQUAL_NULL performance_score
            Some("Senior Engineer".into())  // EQUAL_NULL notes
        ]
    );
}

#[test]
fn test_composite_pk_delete_predicate() {
    let schema = create_composite_pk_schema();
    let mutation = RowMutation::Delete {
        original: vec![
            Some("1001".into()),
            Some("42".into()),
            Some("EMP-999".into()),
            Some("88.50".into()),
            None, // notes is NULL
        ],
    };

    let (sql, vals) = mutation_sql(&schema, &mutation).unwrap();
    assert!(sql.starts_with("DELETE FROM \"APP_DATA\".\"TENANT_METRICS\" WHERE "));
    assert!(sql.contains("\"tenant_id\" = CAST(? AS NUMBER(38,0))"));
    assert!(sql.contains("\"dept_id\" = CAST(? AS NUMBER(38,0))"));
    assert!(sql.contains("\"emp_id\" = CAST(? AS VARCHAR(64))"));
    assert!(sql.contains("EQUAL_NULL(\"performance_score\", CAST(? AS NUMBER))"));
    assert!(sql.contains("EQUAL_NULL(\"notes\", CAST(? AS VARCHAR))"));

    assert_eq!(
        vals,
        vec![
            Some("1001".into()),
            Some("42".into()),
            Some("EMP-999".into()),
            Some("88.50".into()),
            None // NULL notes
        ]
    );
}

// =========================================================================
// 2. Null Comparison & Concurrency Conditions (EQUAL_NULL semantics)
// =========================================================================

#[test]
fn test_equal_null_semantics_null_vs_empty_vs_non_null() {
    let col_text = ColumnInfo {
        id: "c_txt".into(),
        name: "description".into(),
        database_type: "VARCHAR".into(),
        kind: ColumnKind::Text,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: true,
        insertable: true,
        has_default: false,
    };

    // Case 1: Value is SQL NULL
    let lit_null = format_snowflake_literal(&col_text, None);
    assert_eq!(lit_null, "NULL");

    // Case 2: Value is empty string ""
    let lit_empty = format_snowflake_literal(&col_text, Some(""));
    assert_eq!(lit_empty, "''");
    assert_ne!(
        lit_empty, lit_null,
        "Empty string MUST NOT equal NULL literal"
    );

    // Case 3: Value is string "NULL" (literal string)
    let lit_str_null = format_snowflake_literal(&col_text, Some("NULL"));
    assert_eq!(lit_str_null, "'NULL'");
    assert_ne!(
        lit_str_null, lit_null,
        "String literal 'NULL' MUST be quoted and not bare NULL"
    );

    // Case 4: Value is integer 0
    let col_int = ColumnInfo {
        id: "c_int".into(),
        name: "count".into(),
        database_type: "NUMBER".into(),
        kind: ColumnKind::Integer,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: true,
        insertable: true,
        has_default: false,
    };
    let lit_zero = format_snowflake_literal(&col_int, Some("0"));
    assert_eq!(lit_zero, "0");
    assert_ne!(lit_zero, "NULL", "Zero must not format as NULL");

    // Case 5: Value is boolean false
    let col_bool = ColumnInfo {
        id: "c_bool".into(),
        name: "is_active".into(),
        database_type: "BOOLEAN".into(),
        kind: ColumnKind::Boolean,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: true,
        insertable: true,
        has_default: false,
    };
    let lit_false = format_snowflake_literal(&col_bool, Some("false"));
    assert_eq!(lit_false, "FALSE");
    assert_ne!(lit_false, "NULL", "Boolean false must not format as NULL");
}

// =========================================================================
// 3. Floating Point, NaN, Infinity, and Decimal Precision Loss
// =========================================================================

#[test]
fn test_adversarial_decimal_precision_loss_and_special_floats() {
    let col_dec = ColumnInfo {
        id: "c_dec".into(),
        name: "amount".into(),
        database_type: "NUMBER(38,10)".into(),
        kind: ColumnKind::Decimal,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: true,
        insertable: true,
        has_default: false,
    };

    // Verified IEEE floats (NaN, Infinity, -Infinity)
    let lit_nan = format_snowflake_literal(&col_dec, Some("NaN"));
    assert_eq!(
        lit_nan, "'NaN'::FLOAT",
        "NaN must be quoted and cast to FLOAT"
    );

    let lit_inf = format_snowflake_literal(&col_dec, Some("Infinity"));
    assert_eq!(
        lit_inf, "'Infinity'::FLOAT",
        "Infinity must be quoted and cast to FLOAT"
    );

    let lit_neg_inf = format_snowflake_literal(&col_dec, Some("-Infinity"));
    assert_eq!(
        lit_neg_inf, "'-Infinity'::FLOAT",
        "-Infinity must be quoted and cast to FLOAT"
    );

    let lit_lower_nan = format_snowflake_literal(&col_dec, Some("nan"));
    assert_eq!(
        lit_lower_nan, "'NaN'::FLOAT",
        "lowercase nan must be formatted as 'NaN'::FLOAT"
    );

    // High precision decimal: val.to_string() preserves exact digits because val is string
    let high_precision = "12345678901234567890.1234567890";
    let lit_high_prec = format_snowflake_literal(&col_dec, Some(high_precision));
    assert_eq!(lit_high_prec, "12345678901234567890.1234567890");
    // However, in SQL: unquoted 12345678901234567890.1234567890 is emitted as numeric literal
}

// =========================================================================
// 4. Identifier Quoting & SQL Injection Safety
// =========================================================================

#[test]
fn test_special_characters_and_reserved_words_in_identifiers() {
    let schema = TableSchema {
        table: TableRef {
            schema: "MY SCHEMA".into(),
            name: "ORDER DETAILS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "SELECT".into(), // SQL reserved keyword
                database_type: "NUMBER".into(),
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
                name: "COLUMN WITH SPACES".into(),
                database_type: "VARCHAR".into(),
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
                name: "has\"double\"quotes".into(),
                database_type: "VARCHAR".into(),
                kind: ColumnKind::Text,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: true,
                insertable: true,
                has_default: false,
            },
        ],
        primary_key: vec!["SELECT".into()],
        foreign_keys: Vec::new(),
        inbound_foreign_keys: Vec::new(),
        editable: true,
    };

    let mutation = RowMutation::Update {
        original: vec![Some("1".into()), Some("val1".into()), Some("val2".into())],
        changes: vec![(1, Some("new_val".into()))],
    };

    let (sql, _) = mutation_sql(&schema, &mutation).unwrap();

    // Table identifiers quoted
    assert!(sql.contains("UPDATE \"MY SCHEMA\".\"ORDER DETAILS\""));
    // Reserved keyword quoted
    assert!(sql.contains("\"SELECT\" = CAST(? AS NUMBER)"));
    // Spaced column quoted
    assert!(sql.contains("\"COLUMN WITH SPACES\" = ?"));
    // Double quotes escaped as ""
    assert!(sql.contains("\"has\"\"double\"\"quotes\""));
}

// =========================================================================
// 5. Fallback Primary Key Discovery & Variant-Only Tables
// =========================================================================

#[test]
fn test_variant_only_table_fails_gracefully_without_panic() {
    let variant_schema = TableSchema {
        table: TableRef {
            schema: "RAW_DATA".into(),
            name: "EVENTS".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "payload".into(),
                database_type: "VARIANT".into(),
                kind: ColumnKind::Json,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: false,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "metadata".into(),
                database_type: "OBJECT".into(),
                kind: ColumnKind::Json,
                nullable: true,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: false,
                has_default: false,
            },
        ],
        primary_key: Vec::new(), // No primary keys
        foreign_keys: Vec::new(),
        inbound_foreign_keys: Vec::new(),
        editable: false, // Ineligible for edits
    };

    let mutation = RowMutation::Update {
        original: vec![Some("{\"key\": 1}".into()), Some("{}".into())],
        changes: vec![(0, Some("{\"key\": 2}".into()))],
    };

    let err = mutation_sql(&variant_schema, &mutation).unwrap_err();
    assert_eq!(err.code, ErrorCode::PolicyBlocked);
    assert_eq!(err.message, "This table cannot be edited safely");

    // Also test optimistic_predicate directly
    let pred_err = optimistic_predicate(&variant_schema, &vec![None, None]).unwrap_err();
    assert_eq!(pred_err.code, ErrorCode::PolicyBlocked);
    assert_eq!(
        pred_err.message,
        "A primary key or unique columns are required for edits"
    );
}

// =========================================================================
// 6. Procedural Scripting Block Escaping & Dollar Signs ($$)
// =========================================================================

#[test]
fn test_literal_formatting_and_scripting_dollar_quote_breakout_vulnerability() {
    let col = ColumnInfo {
        id: "c0".into(),
        name: "bio".into(),
        database_type: "VARCHAR".into(),
        kind: ColumnKind::Text,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: true,
        insertable: true,
        has_default: false,
    };

    // Standard single quote escaping
    assert_eq!(
        format_snowflake_literal(&col, Some("O'Reilly")),
        "'O''Reilly'"
    );

    // Semicolons inside string literals
    assert_eq!(
        format_snowflake_literal(&col, Some("first; second; third")),
        "'first; second; third'"
    );

    // Newlines inside string literals
    assert_eq!(
        format_snowflake_literal(&col, Some("line1\nline2\r\nline3")),
        "'line1\nline2\r\nline3'"
    );

    // EMPIRICAL DEFECT DISCOVERY 3: Dollar sign $$ escaping vulnerability
    // In Snowflake Scripting, EXECUTE IMMEDIATE $$ ... $$ uses $$ as delimiter.
    // If a string contains $$, it prematurely closes the dollar-quoted block!
    let malicious_payload = "harmless$$; DROP TABLE PROD_USERS; --";
    let formatted = format_snowflake_literal(&col, Some(malicious_payload));

    // format_snowflake_literal leaves $$ UNESCAPED:
    assert_eq!(formatted, "'harmless$$; DROP TABLE PROD_USERS; --'");
    // This empirically confirms that $$ is not replaced or sanitized in literals!
    assert!(
        formatted.contains("$$"),
        "Raw $$ is emitted into procedural script body"
    );
}

// =========================================================================
// 7. Empirical Stress Tests: Special Floats, Decimals & Empty Inserts
// =========================================================================

#[test]
fn test_adversarial_special_float_casing_and_signs() {
    let col_dec = ColumnInfo {
        id: "c_dec".into(),
        name: "amount".into(),
        database_type: "NUMBER(38,10)".into(),
        kind: ColumnKind::Decimal,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: true,
        insertable: true,
        has_default: false,
    };

    let col_float = ColumnInfo {
        id: "c_flt".into(),
        name: "metric".into(),
        database_type: "FLOAT".into(),
        kind: ColumnKind::Decimal,
        nullable: true,
        primary_key: false,
        foreign_key: false,
        editable: true,
        insertable: true,
        has_default: false,
    };

    for col in [&col_dec, &col_float] {
        // Special NaN cases
        assert_eq!(format_snowflake_literal(col, Some("NaN")), "'NaN'::FLOAT");
        assert_eq!(format_snowflake_literal(col, Some("nan")), "'NaN'::FLOAT");
        assert_eq!(format_snowflake_literal(col, Some("NAN")), "'NaN'::FLOAT");
        assert_eq!(format_snowflake_literal(col, Some("Nan")), "'NaN'::FLOAT");
        assert_eq!(format_snowflake_literal(col, Some("+nan")), "'NaN'::FLOAT");
        assert_eq!(format_snowflake_literal(col, Some("-nan")), "'NaN'::FLOAT");
        assert_eq!(format_snowflake_literal(col, Some("+NaN")), "'NaN'::FLOAT");
        assert_eq!(format_snowflake_literal(col, Some("-NaN")), "'NaN'::FLOAT");
        assert_eq!(
            format_snowflake_literal(col, Some("  NaN  ")),
            "'NaN'::FLOAT"
        );

        // Special Infinity cases
        assert_eq!(
            format_snowflake_literal(col, Some("Infinity")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("+Infinity")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("infinity")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("+infinity")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("inf")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("+inf")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("INF")),
            "'Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("+INF")),
            "'Infinity'::FLOAT"
        );

        // Special -Infinity cases
        assert_eq!(
            format_snowflake_literal(col, Some("-Infinity")),
            "'-Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("-infinity")),
            "'-Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("-inf")),
            "'-Infinity'::FLOAT"
        );
        assert_eq!(
            format_snowflake_literal(col, Some("-INF")),
            "'-Infinity'::FLOAT"
        );

        // Ensure NEVER bare unquoted NaN / Infinity
        let lit_nan = format_snowflake_literal(col, Some("NaN"));
        assert!(!lit_nan.contains(" bare ") && lit_nan != "NaN" && lit_nan != "NAN");
        let lit_inf = format_snowflake_literal(col, Some("Infinity"));
        assert!(lit_inf != "Infinity" && lit_inf != "INF");
    }
}

#[test]
fn test_adversarial_38_digit_decimal_preservation() {
    let col_dec = ColumnInfo {
        id: "c_dec".into(),
        name: "balance".into(),
        database_type: "NUMBER(38,10)".into(),
        kind: ColumnKind::Decimal,
        nullable: false,
        primary_key: false,
        foreign_key: false,
        editable: true,
        insertable: true,
        has_default: false,
    };

    let cases = [
        "12345678901234567890.1234567890",
        "9999999999999999999999999999.9999999999",
        "-9999999999999999999999999999.9999999999",
        "0.0000000001",
        "-0.0000000001",
        "10000000000000000000000000000000000000", // 38 digits integer
    ];

    for case in cases {
        let lit = format_snowflake_literal(&col_dec, Some(case));
        // Verify EXACT string digits without float truncation or scientific notation drift
        assert_eq!(
            lit, case,
            "Decimal literal must preserve exact string representation"
        );
    }
}

#[test]
fn test_adversarial_empty_insert_syntax_snowflake_driver() {
    let schema = TableSchema {
        table: TableRef {
            schema: "APP".into(),
            name: "LOG_ENTRIES".into(),
        },
        columns: vec![
            ColumnInfo {
                id: "c0".into(),
                name: "id".into(),
                database_type: "NUMBER".into(),
                kind: ColumnKind::Integer,
                nullable: false,
                primary_key: true,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: true,
            },
            ColumnInfo {
                id: "c1".into(),
                name: "created_at".into(),
                database_type: "TIMESTAMP_NTZ".into(),
                kind: ColumnKind::Temporal,
                nullable: false,
                primary_key: false,
                foreign_key: false,
                editable: false,
                insertable: true,
                has_default: true,
            },
        ],
        primary_key: vec!["id".into()],
        foreign_keys: vec![],
        inbound_foreign_keys: vec![],
        editable: true,
    };

    // 1. mutation_sql level
    let mutation = RowMutation::Insert { values: vec![] };
    let (sql, vals) = mutation_sql(&schema, &mutation).unwrap();
    assert_eq!(sql, "INSERT INTO \"APP\".\"LOG_ENTRIES\" DEFAULT VALUES");
    assert!(vals.is_empty());
    assert!(
        !sql.contains("() VALUES ()"),
        "Must NEVER emit empty () VALUES ()"
    );
}
