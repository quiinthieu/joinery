use joinery_core::domain::*;
use joinery_core::execution::Studio;
use joinery_core::params::extract_query_params;
use joinery_core::safety::split_sql;
use joinery_core::secrets::MemorySecrets;
use sqlparser::dialect::SnowflakeDialect;
use std::sync::Arc;

// =========================================================================
// SECTION 1: Snowflake Procedural Block Semicolon Handling
// =========================================================================

#[test]
fn adversarial_procedural_nested_blocks_with_semicolons() {
    let dialect = SnowflakeDialect {};
    let sql = r#"
BEGIN
    LET x := 10;
    BEGIN
        LET y := 20;
        LET z := x + y;
    END;
    RETURN x;
END;
SELECT 42 AS outcome;
"#;
    let parts = split_sql(&dialect, sql).expect("split nested procedural blocks");
    assert_eq!(
        parts.len(),
        2,
        "Nested procedural block must not split on inner semicolons; expected 2 statements, got {}: {:?}",
        parts.len(),
        parts
    );
    assert!(parts[0].starts_with("BEGIN"));
    assert!(parts[0].ends_with("END"));
    assert_eq!(parts[1].trim(), "SELECT 42 AS outcome");
}

#[test]
fn adversarial_procedural_declare_block_with_multiple_declarations() {
    let dialect = SnowflakeDialect {};
    let sql = r#"
DECLARE
    min_val INT DEFAULT 0;
    max_val INT DEFAULT 100;
    step_val INT DEFAULT 5;
    cur CURSOR FOR SELECT id, val FROM metrics;
BEGIN
    FOR rec IN cur DO
        IF rec.val > max_val THEN
            RETURN rec.val;
        END IF;
    END FOR;
    RETURN min_val;
END;
SELECT 99;
"#;
    let parts = split_sql(&dialect, sql).expect("split DECLARE block");
    assert_eq!(
        parts.len(),
        2,
        "DECLARE block with multiple variable declarations must split into exactly 2 statements, got {}: {:?}",
        parts.len(),
        parts
    );
    assert!(parts[0].starts_with("DECLARE"));
    assert!(parts[0].ends_with("END"));
    assert_eq!(parts[1].trim(), "SELECT 99");
}

#[test]
fn adversarial_procedural_blocks_with_semicolons_in_strings_and_comments() {
    let dialect = SnowflakeDialect {};
    let sql = r#"
BEGIN
    -- This is a comment with ; semicolons ;
    /* Multi-line comment
       with ; semicolons ; inside
    */
    LET msg := 'String ; with ; embedded ; semicolons ;';
    LET quoted := "Identifier ; with ; semicolons ;";
    RETURN msg;
END;
SELECT 'next statement';
"#;
    let parts = split_sql(&dialect, sql).expect("split strings and comments with semicolons");
    assert_eq!(
        parts.len(),
        2,
        "Block with semicolons in comments/strings must split into exactly 2 statements, got {}: {:?}",
        parts.len(),
        parts
    );
    assert!(parts[0].starts_with("BEGIN"));
    assert_eq!(parts[1].trim(), "SELECT 'next statement'");
}

#[test]
fn adversarial_procedural_block_with_dollar_quoted_body() {
    let dialect = SnowflakeDialect {};
    let sql = r#"
EXECUTE IMMEDIATE $$
DECLARE
    radius FLOAT;
    area FLOAT;
BEGIN
    radius := 10.0;
    area := 3.14159 * radius * radius;
    RETURN area;
END;
$$;
SELECT 1;
"#;
    let parts = split_sql(&dialect, sql).expect("split execute immediate $$ block");
    assert_eq!(
        parts.len(),
        2,
        "EXECUTE IMMEDIATE $$ ... $$; followed by SELECT must split into 2 statements, got {}: {:?}",
        parts.len(),
        parts
    );
    assert!(parts[0].starts_with("EXECUTE IMMEDIATE $$"));
    assert_eq!(parts[1].trim(), "SELECT 1");
}

#[test]
fn adversarial_procedural_block_with_label_closure() {
    let dialect = SnowflakeDialect {};
    let sql = r#"
outer_block:
BEGIN
    LET val := 1;
    RETURN val;
END outer_block;
SELECT 'after labelled block';
"#;
    let parts = split_sql(&dialect, sql).expect("split labelled block");
    assert_eq!(
        parts.len(),
        2,
        "Labelled procedural block 'END label;' must split cleanly into 2 statements, got {}: {:?}",
        parts.len(),
        parts
    );
    assert!(parts[0].contains("outer_block"));
    assert_eq!(parts[1].trim(), "SELECT 'after labelled block'");
}

#[test]
fn adversarial_transaction_statements_vs_procedural_begin() {
    let dialect = SnowflakeDialect {};

    // 1. Transaction BEGIN; ... COMMIT; must split per statement
    let tx_sql = "BEGIN;\nSELECT 1;\nCOMMIT;";
    let tx_parts = split_sql(&dialect, tx_sql).expect("split transaction");
    assert_eq!(
        tx_parts.len(),
        3,
        "Transaction statements must split into 3 statements, got {}: {:?}",
        tx_parts.len(),
        tx_parts
    );
    assert_eq!(tx_parts[0], "BEGIN");
    assert_eq!(tx_parts[1], "SELECT 1");
    assert_eq!(tx_parts[2], "COMMIT");

    // 2. BEGIN WORK; ... COMMIT;
    let work_sql = "BEGIN WORK;\nSELECT 2;\nCOMMIT;";
    let work_parts = split_sql(&dialect, work_sql).expect("split begin work");
    assert_eq!(work_parts.len(), 3);
    assert_eq!(work_parts[0], "BEGIN WORK");

    // 3. BEGIN TRANSACTION; ... ROLLBACK;
    let trans_sql = "BEGIN TRANSACTION;\nSELECT 3;\nROLLBACK;";
    let trans_parts = split_sql(&dialect, trans_sql).expect("split begin transaction");
    assert_eq!(trans_parts.len(), 3);
    assert_eq!(trans_parts[0], "BEGIN TRANSACTION");
}

// =========================================================================
// SECTION 2: Dollar Parameter Extraction vs $$ Quotes & Comments
// =========================================================================

#[test]
fn adversarial_named_dollar_parameters_extracted_across_sql_engines() {
    for engine in [
        EngineKind::Postgres,
        EngineKind::Snowflake,
        EngineKind::Mysql,
    ] {
        let sql = "SELECT * FROM orders WHERE tenant_id = $tenant_id AND status = $order_status AND amount > $min_amount";
        let slots = extract_query_params(engine, sql).expect("extract dollar params");
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0].key, "$tenant_id");
        assert_eq!(slots[1].key, "$order_status");
        assert_eq!(slots[2].key, "$min_amount");
        assert!(!slots[0].positional);
    }
}

#[test]
fn adversarial_dollar_params_ignored_in_comments_and_string_literals() {
    for engine in [EngineKind::Postgres, EngineKind::Snowflake] {
        let sql = r#"
            -- Single-line comment with $ignored_line_param
            /* Block comment with $ignored_block_param
               and nested /* $ignored_nested_param */
            */
            SELECT '$ignored_single_quote',
                   "$ignored_double_quote",
                   col$not_param,
                   $valid_param1,
                   $valid_param2
            FROM tbl
        "#;
        let slots =
            extract_query_params(engine, sql).expect("extract params with comments/strings");
        assert_eq!(
            slots.len(),
            2,
            "Only $valid_param1 and $valid_param2 must be extracted, got: {:?}",
            slots
        );
        assert_eq!(slots[0].key, "$valid_param1");
        assert_eq!(slots[1].key, "$valid_param2");
    }
}

#[test]
fn adversarial_dollar_params_alongside_dollar_quotes() {
    for engine in [EngineKind::Postgres, EngineKind::Snowflake] {
        let sql = r#"
            SELECT $p_before,
                   $$ This is a string with $ignored_inside_dollar and ; and ' $$,
                   $body$ Another dollar quote with $ignored_tagged_dollar $body$,
                   $p_after
            FROM dual
        "#;
        let slots =
            extract_query_params(engine, sql).expect("extract params alongside dollar quotes");
        assert_eq!(
            slots.len(),
            2,
            "Parameters inside $$...$$ and $tag$...$tag$ must be ignored, got: {:?}",
            slots
        );
        assert_eq!(slots[0].key, "$p_before");
        assert_eq!(slots[1].key, "$p_after");
    }
}

#[test]
fn adversarial_dollar_quote_tag_resembling_parameter() {
    // A dollar tag like `$param$` must not be recognized as parameter `$param`
    for engine in [EngineKind::Postgres, EngineKind::Snowflake] {
        let sql =
            "SELECT $param$ Content inside dollar quote with $p_inner $param$, $p_real FROM tbl";
        let slots = extract_query_params(engine, sql).expect("extract tagged quote");
        assert_eq!(slots.len(), 1, "Expected only $p_real, got: {:?}", slots);
        assert_eq!(slots[0].key, "$p_real");
    }
}

#[test]
fn adversarial_adjacent_dollar_identifiers_and_syntax() {
    let sql = "SELECT col$1, col$name, $$$$ AS empty_dq, $real FROM tbl";
    let slots = extract_query_params(EngineKind::Postgres, sql).expect("adjacent dollars");
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].key, "$real");
}

#[test]
fn adversarial_snowflake_prepare_procedural_with_inner_syntax_error() {
    let profile = ConnectionProfile {
        id: "test-sf".into(),
        name: "Snowflake".into(),
        group_id: "test".into(),
        engine: EngineKind::Snowflake,
        host: "test.snowflakecomputing.com".into(),
        port: 443,
        database: "TEST".into(),
        username: "ADMIN".into(),
        environment: Environment::Local,
        safe_mode: SafeMode::Silent,
        tls_mode: TlsMode::VerifyFull,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: None,
    };

    let sql = "SELECT 1;\nBEGIN\n  LET x := ;\nEND;\nSELECT 3;";
    let plan = joinery_core::safety::prepare("session-1", &profile, sql)
        .expect("prepare must handle batch with procedural block containing syntax defect");

    assert_eq!(
        plan.statements.len(),
        3,
        "Batch must split into 3 statements: SELECT 1, procedural block, SELECT 3; got: {:?}",
        plan.statements
    );
    assert_eq!(plan.statements[0].kind, StatementKind::Read);
    assert_eq!(plan.statements[1].kind, StatementKind::Write);
    assert_eq!(plan.statements[2].kind, StatementKind::Read);
}

#[test]
fn adversarial_dollar_parameter_length_and_boundary_validation() {
    // 1. Name <= 64 chars is valid
    let valid_name = format!("${}", "a".repeat(64));
    let sql_valid = format!("SELECT {valid_name} FROM dual");
    let slots = extract_query_params(EngineKind::Postgres, &sql_valid).unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].key, valid_name);

    // 2. Name > 64 chars must be rejected
    let invalid_name = format!("${}", "a".repeat(65));
    let sql_invalid = format!("SELECT {invalid_name} FROM dual");
    let err = extract_query_params(EngineKind::Postgres, &sql_invalid).unwrap_err();
    assert!(
        err.message.contains("64 characters or fewer"),
        "Expected error about 64 characters, got: {}",
        err.message
    );

    // 3. Positional parameter $0 must be rejected
    let sql_zero = "SELECT $0 FROM dual";
    let err_zero = extract_query_params(EngineKind::Postgres, sql_zero).unwrap_err();
    assert!(err_zero.message.contains("between $1 and $256"));

    // 4. Positional parameter $257 must be rejected
    let sql_257 = "SELECT $257 FROM dual";
    let err_257 = extract_query_params(EngineKind::Postgres, sql_257).unwrap_err();
    assert!(err_257.message.contains("between $1 and $256"));
}

#[test]
fn adversarial_batch_error_policy_serde_conformance() {
    // Verify JSON representations match frontend camelCase contracts
    let stop = BatchErrorPolicy::StopOnError;
    let continue_err = BatchErrorPolicy::ContinueOnError;

    let stop_json = serde_json::to_string(&stop).unwrap();
    let continue_json = serde_json::to_string(&continue_err).unwrap();

    assert_eq!(stop_json, "\"stopOnError\"");
    assert_eq!(continue_json, "\"continueOnError\"");

    let deserialized_stop: BatchErrorPolicy = serde_json::from_str("\"stopOnError\"").unwrap();
    let deserialized_continue: BatchErrorPolicy =
        serde_json::from_str("\"continueOnError\"").unwrap();

    assert_eq!(deserialized_stop, BatchErrorPolicy::StopOnError);
    assert_eq!(deserialized_continue, BatchErrorPolicy::ContinueOnError);
}

#[test]
fn adversarial_procedural_case_statement_vs_case_expression() {
    let dialect = SnowflakeDialect {};
    // Semicolon after END CASE; inside procedural block vs CASE ... END as expression
    let sql = r#"
BEGIN
    LET v := 1;
    -- Procedural CASE statement ending with END CASE;
    CASE v
        WHEN 1 THEN v := 10;
        ELSE v := 20;
    END CASE;

    -- Inline CASE expression inside SELECT
    SELECT CASE WHEN v = 10 THEN 'ten' ELSE 'other' END INTO :res;

    RETURN v;
END;
SELECT 'done';
"#;
    let parts = split_sql(&dialect, sql).expect("split case statement and expression");
    assert_eq!(
        parts.len(),
        2,
        "Block with both CASE statement and CASE expression must split into exactly 2 statements, got {}: {:?}",
        parts.len(),
        parts
    );
    assert!(parts[0].starts_with("BEGIN"));
    assert_eq!(parts[1].trim(), "SELECT 'done'");
}

#[tokio::test]
async fn adversarial_studio_cancel_by_query_id_public_api_invariants() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();

    // 1. Non-existent query handle returns Ok(()) without panicking or erroring
    assert!(
        studio
            .cancel_by_query_id("non-existent-snowflake-handle-001")
            .await
            .is_ok()
    );

    // 2. Empty string returns Ok(()) safely
    assert!(studio.cancel_by_query_id("").await.is_ok());

    // 3. Random UUID returns Ok(()) safely
    let random_id = uuid::Uuid::new_v4().to_string();
    assert!(studio.cancel_by_query_id(&random_id).await.is_ok());

    // 4. Repeated cancel requests on non-existent handles are idempotent and safe
    for _ in 0..10 {
        assert!(
            studio
                .cancel_by_query_id("non-existent-snowflake-handle-001")
                .await
                .is_ok()
        );
    }
}

#[tokio::test]
async fn adversarial_studio_cancel_by_query_id_hostile_inputs_and_concurrent_storm() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();

    // 1. Hostile inputs: SQL injection, very long string, unicode, emojis, control chars
    let hostile_inputs = [
        "'; DROP TABLE sessions; --",
        "\" OR \"\"=\"",
        "❄️-snowflake-query-🔥",
        "null\0byte\r\ninjection",
        &"X".repeat(100_000),
        "../../etc/passwd",
        "\u{FFFF}\u{0000}",
    ];

    for input in &hostile_inputs {
        let res = studio.cancel_by_query_id(input).await;
        assert!(
            res.is_ok(),
            "Hostile input must safely return Ok(()): {input}"
        );
    }

    // 2. Concurrent storm: 50 concurrent tasks calling cancel_by_query_id with mixed handles
    let mut handles = Vec::new();
    for i in 0..50 {
        let st = Arc::clone(&studio);
        let handle_str = if i % 2 == 0 {
            format!("sf-concurrent-handle-{i}")
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        handles.push(tokio::spawn(async move {
            st.cancel_by_query_id(&handle_str).await
        }));
    }

    for h in handles {
        let res = h.await.expect("task must not panic");
        assert!(res.is_ok(), "Concurrent cancel must succeed safely");
    }

    // 3. Studio remains unpoisoned and operational
    let bootstrap = studio.bootstrap();
    assert!(
        bootstrap.is_ok(),
        "Studio bootstrap must be healthy after cancel storm"
    );
}
