use joinery_core::domain::{
    ConnectionProfile, EngineKind, Environment, ErrorCode, SafeMode, TlsMode,
};
use joinery_core::drivers::snowflake_auth::{
    extract_account_identifier, generate_jwt, parse_pkcs8_pem,
};
use joinery_core::safety::{prepare, split_sql};
use sqlparser::dialect::SnowflakeDialect;

fn make_snowflake_profile(mode: SafeMode) -> ConnectionProfile {
    ConnectionProfile {
        id: "adv-test-profile".into(),
        name: "AdvSnowflake".into(),
        group_id: "test".into(),
        engine: EngineKind::Snowflake,
        host: "test.snowflakecomputing.com".into(),
        port: 443,
        database: "TEST_DB".into(),
        username: "TEST_USER".into(),
        environment: Environment::Production,
        safe_mode: mode,
        tls_mode: TlsMode::VerifyFull,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: false,
        store_drafts: false,
        snowflake: None,
    }
}

// =========================================================================
// AREA 1: SafeMode Evaluation Evasions
// =========================================================================

#[test]
fn adversarial_safemode_disguised_dml_ddl_blocked_in_readonly() {
    let profile = make_snowflake_profile(SafeMode::ReadOnly);

    // Adversarial queries attempting to disguise DML/DDL as comments, whitespace, or reads
    let attack_vectors = [
        // Comments preceding DDL/DML
        "/* harmless */ DROP TABLE prod_users",
        "-- harmless\nDROP TABLE prod_users",
        "/* SELECT */ DROP TABLE prod_users",
        "/* SELECT 1 */ DELETE FROM accounts",
        "/* SELECT * FROM accounts; */ DROP TABLE prod_users",
        "-- SELECT * FROM accounts\nDELETE FROM accounts",
        "/*!50000 DROP TABLE users */",
        // Disguised DML/DDL with strange casing
        "dRoP tAbLe prod_users",
        "dElEtE fRoM accounts WHERE 1=1",
        "tRuNcAtE tAbLe accounts",
        "aLtEr tAbLe accounts dRoP cOlUmN secret",
        // Whitespace and non-printable characters
        "\n\t   DROP TABLE prod_users",
        "\r\n\tDELETE FROM accounts WHERE 1=1",
        // Multi-statement injection attempts in a single string
        "SELECT 1; DROP TABLE prod_users",
        "SELECT 1; -- comment\nDROP TABLE prod_users",
        "SELECT 1; DELETE FROM accounts",
        "SELECT 1; TRUNCATE accounts",
        "SELECT 1; ALTER TABLE accounts ADD COLUMN x INT",
        "SELECT 1; EXECUTE IMMEDIATE 'DROP TABLE prod_users'",
        // Snowflake procedural blocks containing DML/DDL
        "BEGIN DROP TABLE prod_users; END;",
        "BEGIN DELETE FROM accounts; END;",
        "BEGIN UPDATE accounts SET balance = 0; END;",
        "DECLARE x INT; BEGIN DROP TABLE prod_users; END;",
        "BEGIN TRANSACTION; DROP TABLE prod_users; COMMIT;",
        "EXECUTE IMMEDIATE 'DROP TABLE prod_users'",
        // Labeled procedural blocks
        "<<my_block>> BEGIN DROP TABLE prod_users; END;",
        // Mutating function / expression attempts
        "SELECT SYSTEM$ABORT_SESSION(12345)",
        "SELECT SYSTEM$CANCEL_QUERY('xyz123')",
        "SELECT 1 INTO new_accounts FROM accounts",
    ];

    for &sql in &attack_vectors {
        let result = prepare("adv-session", &profile, sql);
        match result {
            Err(err) => {
                // If it returned an error, verify it is policyBlocked or an invalid error, NOT a silent success
                assert!(
                    err.code == ErrorCode::PolicyBlocked
                        || err.code == ErrorCode::InvalidInput
                        || err.code == ErrorCode::Unsupported,
                    "Expected policy blockage for '{sql}', got error code: {:?}",
                    err.code
                );
            }
            Ok(plan) => {
                // If it did not return Err, every single statement in the plan MUST require approval
                // or must NOT be classified as Read!
                for stmt in &plan.statements {
                    assert_ne!(
                        stmt.kind,
                        joinery_core::domain::StatementKind::Read,
                        "CRITICAL BUG: SQL attack vector '{sql}' was falsely classified as Read! Statement: {:?}",
                        stmt
                    );
                }
                assert!(
                    plan.requires_approval,
                    "CRITICAL BUG: Attack vector '{sql}' produced plan not requiring approval: {:?}",
                    plan
                );
            }
        }
    }
}

// =========================================================================
// AREA 2: Procedural Scripting Block Fragmentation
// =========================================================================

#[test]
fn procedural_blocks_nested_begin_end() {
    let dialect = SnowflakeDialect {};

    // 1. Single BEGIN ... END block with multiple internal statements
    let sql1 = "BEGIN\n  x := 1;\n  y := 2;\nEND;";
    let parts1 = split_sql(&dialect, sql1).expect("split sql1");
    assert_eq!(
        parts1.len(),
        1,
        "Single BEGIN..END block was fragmented into {} parts: {:?}",
        parts1.len(),
        parts1
    );

    // 2. Nested BEGIN ... END blocks
    let sql2 = "BEGIN\n  x := 1;\n  BEGIN\n    y := 2;\n  END;\n  z := 3;\nEND;";
    let parts2 = split_sql(&dialect, sql2).expect("split sql2");
    assert_eq!(
        parts2.len(),
        1,
        "Nested BEGIN..END block was fragmented into {} parts: {:?}",
        parts2.len(),
        parts2
    );

    // 3. Deeply nested BEGIN ... END (4 levels)
    let sql3 = "BEGIN\n BEGIN\n  BEGIN\n   BEGIN\n    a := 1;\n   END;\n  END;\n END;\nEND;";
    let parts3 = split_sql(&dialect, sql3).expect("split sql3");
    assert_eq!(
        parts3.len(),
        1,
        "Deeply nested BEGIN..END was fragmented into {} parts: {:?}",
        parts3.len(),
        parts3
    );
}

#[test]
fn procedural_blocks_case_expressions_and_statements() {
    let dialect = SnowflakeDialect {};

    // 1. Deeply nested CASE ... END inside SELECT
    let sql1 = "SELECT CASE WHEN a THEN CASE WHEN b THEN 1 ELSE 2 END ELSE 3 END FROM t;";
    let parts1 = split_sql(&dialect, sql1).expect("split sql1");
    assert_eq!(
        parts1.len(),
        1,
        "Nested CASE in SELECT fragmented: {:?}",
        parts1
    );

    // 2. Multiple CASE ... END expressions inside a BEGIN ... END block
    let sql2 = r#"BEGIN
  x := CASE WHEN a = 1 THEN 'one' ELSE 'other' END;
  y := CASE WHEN b = 2 THEN 'two' ELSE 'other' END;
END;"#;
    let parts2 = split_sql(&dialect, sql2).expect("split sql2");
    assert_eq!(
        parts2.len(),
        1,
        "CASE expressions inside BEGIN..END fragmented block: {:?}",
        parts2
    );
}

#[test]
fn procedural_blocks_with_quotes_and_comments_containing_semicolons() {
    let dialect = SnowflakeDialect {};

    let sql = r#"BEGIN
  -- internal comment with ; semicolon
  /* multi-line comment
     with ; internal ; semicolons */
  query_str := 'SELECT * FROM users; WHERE active = 1;';
  EXECUTE IMMEDIATE query_str;
END;"#;
    let parts = split_sql(&dialect, sql).expect("split block with semicolons in strings/comments");
    assert_eq!(
        parts.len(),
        1,
        "Block with internal semicolons in comments/strings was fragmented: {:?}",
        parts
    );
}

#[test]
fn procedural_blocks_batch_of_multiple_blocks() {
    let dialect = SnowflakeDialect {};

    // Two independent procedural blocks executed in one batch
    let sql = "BEGIN x := 1; END;\nBEGIN y := 2; END;";
    let parts = split_sql(&dialect, sql).expect("split multi-block batch");
    assert_eq!(
        parts.len(),
        2,
        "Expected exactly 2 blocks, got {}: {:?}",
        parts.len(),
        parts
    );
    assert!(parts[0].contains("x := 1"));
    assert!(parts[1].contains("y := 2"));

    // Block followed by a regular SELECT
    let sql2 = "BEGIN x := 1; END;\nSELECT 1;";
    let parts2 = split_sql(&dialect, sql2).expect("split block and select");
    assert_eq!(
        parts2.len(),
        2,
        "Expected 2 parts for block + select, got {}: {:?}",
        parts2.len(),
        parts2
    );
}

#[test]
fn procedural_blocks_snowflake_scripting_loops_and_conditionals() {
    let dialect = SnowflakeDialect {};

    // Diagnostic printing for each procedural construct
    let sql_if =
        "BEGIN\n  IF x = 1 THEN\n    y := 2;\n  ELSE\n    y := 3;\n  END IF;\n  z := 4;\nEND;";
    let p_if = split_sql(&dialect, sql_if).unwrap();
    eprintln!("DIAGNOSTIC IF: {:?}", p_if);

    let sql_for = "BEGIN\n  FOR i IN 1 TO 10 DO\n    x := x + i;\n  END FOR;\n  y := 1;\nEND;";
    let p_for = split_sql(&dialect, sql_for).unwrap();
    eprintln!("DIAGNOSTIC FOR: {:?}", p_for);

    let sql_while = "BEGIN\n  WHILE x > 0 DO\n    x := x - 1;\n  END WHILE;\n  y := 1;\nEND;";
    let p_while = split_sql(&dialect, sql_while).unwrap();
    eprintln!("DIAGNOSTIC WHILE: {:?}", p_while);

    let sql_loop = "BEGIN\n  LOOP\n    x := x + 1;\n    IF x > 5 THEN\n      BREAK;\n    END IF;\n  END LOOP;\n  y := 1;\nEND;";
    let p_loop = split_sql(&dialect, sql_loop).unwrap();
    eprintln!("DIAGNOSTIC LOOP: {:?}", p_loop);

    let sql_case = "BEGIN\n  CASE (x)\n    WHEN 1 THEN y := 'one';\n    ELSE y := 'other';\n  END CASE;\nEND;\nSELECT 1;";
    let p_case = split_sql(&dialect, sql_case).unwrap();
    eprintln!("DIAGNOSTIC CASE: {:?}", p_case);

    assert_eq!(
        p_if.len(),
        1,
        "Compound IF..END IF block must remain a single unified statement"
    );
    assert_eq!(
        p_if[0],
        "BEGIN\n  IF x = 1 THEN\n    y := 2;\n  ELSE\n    y := 3;\n  END IF;\n  z := 4;\nEND"
    );
    assert_eq!(
        p_for.len(),
        1,
        "Compound FOR..END FOR block must remain a single unified statement"
    );
    assert_eq!(
        p_for[0],
        "BEGIN\n  FOR i IN 1 TO 10 DO\n    x := x + i;\n  END FOR;\n  y := 1;\nEND"
    );
    assert_eq!(
        p_while.len(),
        1,
        "Compound WHILE..END WHILE block must remain a single unified statement"
    );
    assert_eq!(
        p_while[0],
        "BEGIN\n  WHILE x > 0 DO\n    x := x - 1;\n  END WHILE;\n  y := 1;\nEND"
    );
    assert_eq!(
        p_loop.len(),
        1,
        "Compound LOOP..END LOOP block must remain a single unified statement"
    );
    assert_eq!(
        p_loop[0],
        "BEGIN\n  LOOP\n    x := x + 1;\n    IF x > 5 THEN\n      BREAK;\n    END IF;\n  END LOOP;\n  y := 1;\nEND"
    );

    assert_eq!(
        p_case.len(),
        2,
        "END CASE block must be split from subsequent SELECT 1"
    );
    assert!(p_case[0].starts_with("BEGIN\n  CASE"));
    assert_eq!(p_case[1].trim(), "SELECT 1");
}

#[test]
fn prepare_on_fragmented_snowflake_scripting_block_documents_defect() {
    let profile = make_snowflake_profile(SafeMode::Confirm);
    let sql =
        "BEGIN\n  IF x = 1 THEN\n    y := 2;\n  ELSE\n    y := 3;\n  END IF;\n  z := 4;\nEND;";
    let plan = prepare("session", &profile, sql).expect("prepare plan");
    assert_eq!(plan.statements.len(), 1);
    assert_eq!(
        plan.statements[0].kind,
        joinery_core::domain::StatementKind::Write
    );
    assert_eq!(
        plan.statements[0].sql,
        "BEGIN\n  IF x = 1 THEN\n    y := 2;\n  ELSE\n    y := 3;\n  END IF;\n  z := 4;\nEND"
    );
    assert!(plan.requires_approval);
}

#[test]
fn compound_snowflake_scripting_block_unified_requirement() {
    let dialect = SnowflakeDialect {};
    let sql =
        "BEGIN\n  IF x = 1 THEN\n    y := 2;\n  ELSE\n    y := 3;\n  END IF;\n  z := 4;\nEND;";
    let parts = split_sql(&dialect, sql).unwrap();
    if parts.len() > 1 {
        panic!(
            "Compound block should not be fragmented, got {} parts: {:?}",
            parts.len(),
            parts
        );
    }
    assert_eq!(parts.len(), 1);
}

#[test]
fn test_adversarial_procedural_repeat_end_repeat() {
    let dialect = SnowflakeDialect {};

    // 1. Basic REPEAT ... END REPEAT inside BEGIN ... END
    let sql1 = "BEGIN\n  REPEAT\n    x := x + 1;\n  UNTIL x > 10\n  END REPEAT;\n  y := 1;\nEND;";
    let parts1 = split_sql(&dialect, sql1).expect("split REPEAT block");
    assert_eq!(
        parts1.len(),
        1,
        "REPEAT ... END REPEAT block prematurely fragmented into {} parts: {:?}",
        parts1.len(),
        parts1
    );
    assert_eq!(
        parts1[0],
        "BEGIN\n  REPEAT\n    x := x + 1;\n  UNTIL x > 10\n  END REPEAT;\n  y := 1;\nEND"
    );

    // 2. Labeled REPEAT ... END REPEAT label
    let sql2 = "BEGIN\n  my_rep: REPEAT\n    x := x + 1;\n  UNTIL x > 10\n  END REPEAT my_rep;\n  y := 1;\nEND;";
    let parts2 = split_sql(&dialect, sql2).expect("split labeled REPEAT block");
    assert_eq!(
        parts2.len(),
        1,
        "Labeled REPEAT block prematurely fragmented: {:?}",
        parts2
    );

    // 3. REPEAT nested inside IF ... END IF inside BEGIN ... END
    let sql3 = r#"BEGIN
  IF condition THEN
    REPEAT
      x := x * 2;
    UNTIL x > 100
    END REPEAT;
  END IF;
END;"#;
    let parts3 = split_sql(&dialect, sql3).expect("split nested REPEAT inside IF");
    assert_eq!(
        parts3.len(),
        1,
        "Nested REPEAT inside IF block prematurely fragmented: {:?}",
        parts3
    );

    // 4. REPEAT block followed by SELECT 1 in a batch
    let sql4 = "BEGIN\n  REPEAT\n    x := x + 1;\n  UNTIL x > 10\n  END REPEAT;\nEND;\nSELECT 1;";
    let parts4 = split_sql(&dialect, sql4).expect("split REPEAT block followed by SELECT");
    assert_eq!(
        parts4.len(),
        2,
        "Expected exactly 2 parts for REPEAT block + SELECT 1, got {}: {:?}",
        parts4.len(),
        parts4
    );
    assert!(parts4[0].starts_with("BEGIN\n  REPEAT"));
    assert_eq!(parts4[1].trim(), "SELECT 1");
}

#[test]
fn test_adversarial_case_end_case_and_trailing_select() {
    let dialect = SnowflakeDialect {};

    // 1. Simple CASE ... END CASE; END; followed by SELECT 1;
    let sql1 = r#"BEGIN
  CASE (x)
    WHEN 1 THEN y := 'one';
    WHEN 2 THEN y := 'two';
    ELSE y := 'other';
  END CASE;
END;
SELECT 1;"#;
    let parts1 = split_sql(&dialect, sql1).expect("split CASE END CASE + SELECT");
    assert_eq!(
        parts1.len(),
        2,
        "Expected exactly 2 statements, got {}: {:?}",
        parts1.len(),
        parts1
    );
    assert!(parts1[0].starts_with("BEGIN\n  CASE"));
    assert_eq!(parts1[1].trim(), "SELECT 1");

    // 2. Searched CASE ... END CASE; followed by multiple queries
    let sql2 = r#"BEGIN
  CASE
    WHEN a > 10 THEN y := 'high';
    WHEN a > 0 THEN y := 'low';
    ELSE y := 'zero';
  END CASE;
END;
SELECT 1;
SELECT 2;"#;
    let parts2 = split_sql(&dialect, sql2).expect("split searched CASE END CASE + 2 selects");
    assert_eq!(
        parts2.len(),
        3,
        "Expected exactly 3 statements, got {}: {:?}",
        parts2.len(),
        parts2
    );
    assert!(parts2[0].starts_with("BEGIN\n  CASE"));
    assert_eq!(parts2[1].trim(), "SELECT 1");
    assert_eq!(parts2[2].trim(), "SELECT 2");

    // 3. Nested CASE ... END CASE inside FOR loop inside BEGIN ... END followed by SELECT 1
    let sql3 = r#"BEGIN
  FOR i IN 1 TO 3 DO
    CASE (i)
      WHEN 1 THEN y := 10;
      ELSE y := 20;
    END CASE;
  END FOR;
END;
SELECT 1;"#;
    let parts3 = split_sql(&dialect, sql3).expect("split nested CASE inside FOR");
    assert_eq!(
        parts3.len(),
        2,
        "Expected exactly 2 statements for nested CASE in FOR + SELECT, got {}: {:?}",
        parts3.len(),
        parts3
    );
    assert_eq!(parts3[1].trim(), "SELECT 1");

    // 4. Expression CASE ... END vs Statement CASE ... END CASE in same block
    let sql4 = r#"BEGIN
  x := CASE WHEN a THEN 1 ELSE 2 END;
  CASE (b)
    WHEN 1 THEN y := 'one';
  END CASE;
END;
SELECT 1;"#;
    let parts4 = split_sql(&dialect, sql4).expect("split mixed CASE expression and statement");
    assert_eq!(
        parts4.len(),
        2,
        "Expected exactly 2 statements for mixed CASE + SELECT, got {}: {:?}",
        parts4.len(),
        parts4
    );
    assert_eq!(parts4[1].trim(), "SELECT 1");
}

#[test]
fn test_adversarial_ansi_sql_for_update_queries() {
    let dialect = SnowflakeDialect {};

    // 1. Basic SELECT ... FOR UPDATE
    let sql1 = "SELECT * FROM users FOR UPDATE;";
    let parts1 = split_sql(&dialect, sql1).expect("split single FOR UPDATE");
    assert_eq!(parts1.len(), 1);
    assert_eq!(parts1[0], "SELECT * FROM users FOR UPDATE");

    // 2. Chained SELECT ... FOR UPDATE
    let sql2 = "SELECT * FROM users FOR UPDATE;\nSELECT * FROM accounts FOR UPDATE;";
    let parts2 = split_sql(&dialect, sql2).expect("split chained FOR UPDATE");
    assert_eq!(
        parts2.len(),
        2,
        "Expected 2 statements for chained FOR UPDATE, got {}: {:?}",
        parts2.len(),
        parts2
    );
    assert_eq!(parts2[0], "SELECT * FROM users FOR UPDATE");
    assert_eq!(parts2[1], "SELECT * FROM accounts FOR UPDATE");

    // 3. SELECT CASE ... END AS col FROM t FOR UPDATE followed by SELECT 1
    let sql3 =
        "SELECT CASE WHEN active THEN 1 ELSE 0 END AS is_active FROM users FOR UPDATE;\nSELECT 1;";
    let parts3 = split_sql(&dialect, sql3).expect("split CASE END + FOR UPDATE + SELECT");
    assert_eq!(
        parts3.len(),
        2,
        "Expected 2 statements for CASE END + FOR UPDATE, got {}: {:?}",
        parts3.len(),
        parts3
    );
    assert_eq!(
        parts3[0],
        "SELECT CASE WHEN active THEN 1 ELSE 0 END AS is_active FROM users FOR UPDATE"
    );
    assert_eq!(parts3[1], "SELECT 1");

    // 4. Direct SELECT CASE ... END FOR UPDATE (without FROM/alias) followed by SELECT 1
    let sql4 = "SELECT CASE WHEN active THEN 1 ELSE 0 END FOR UPDATE;\nSELECT 1;";
    let parts4 = split_sql(&dialect, sql4).expect("split direct CASE END FOR UPDATE");
    assert_eq!(
        parts4.len(),
        2,
        "Expected 2 statements for direct CASE END FOR UPDATE, got {}: {:?}",
        parts4.len(),
        parts4
    );
    assert_eq!(
        parts4[0],
        "SELECT CASE WHEN active THEN 1 ELSE 0 END FOR UPDATE"
    );
    assert_eq!(parts4[1], "SELECT 1");

    // 5. Locking clauses: FOR SHARE, FOR NO KEY UPDATE, FOR KEY SHARE
    let sql5 = "SELECT 1 FROM t FOR SHARE;\nSELECT 2 FROM t FOR NO KEY UPDATE;\nSELECT 3 FROM t FOR KEY SHARE;";
    let parts5 = split_sql(&dialect, sql5).expect("split locking clauses");
    assert_eq!(
        parts5.len(),
        3,
        "Expected 3 statements for locking clauses, got {}: {:?}",
        parts5.len(),
        parts5
    );
    assert_eq!(parts5[0], "SELECT 1 FROM t FOR SHARE");
    assert_eq!(parts5[1], "SELECT 2 FROM t FOR NO KEY UPDATE");
    assert_eq!(parts5[2], "SELECT 3 FROM t FOR KEY SHARE");
}

#[test]
fn test_adversarial_ansi_sql_if_exists_queries() {
    let dialect = SnowflakeDialect {};

    // 1. Chained DROP TABLE IF EXISTS
    let sql1 = "DROP TABLE IF EXISTS users;\nDROP VIEW IF EXISTS user_view;";
    let parts1 = split_sql(&dialect, sql1).expect("split DROP IF EXISTS");
    assert_eq!(
        parts1.len(),
        2,
        "Expected 2 statements for chained DROP IF EXISTS, got {}: {:?}",
        parts1.len(),
        parts1
    );
    assert_eq!(parts1[0], "DROP TABLE IF EXISTS users");
    assert_eq!(parts1[1], "DROP VIEW IF EXISTS user_view");

    // 2. Chained CREATE TABLE IF NOT EXISTS
    let sql2 =
        "CREATE TABLE IF NOT EXISTS users (id INT);\nCREATE TABLE IF NOT EXISTS orders (id INT);";
    let parts2 = split_sql(&dialect, sql2).expect("split CREATE IF NOT EXISTS");
    assert_eq!(
        parts2.len(),
        2,
        "Expected 2 statements for CREATE IF NOT EXISTS, got {}: {:?}",
        parts2.len(),
        parts2
    );
    assert_eq!(parts2[0], "CREATE TABLE IF NOT EXISTS users (id INT)");
    assert_eq!(parts2[1], "CREATE TABLE IF NOT EXISTS orders (id INT)");

    // 3. ALTER TABLE ... DROP COLUMN IF EXISTS followed by SELECT 1
    let sql3 = "ALTER TABLE users DROP COLUMN IF EXISTS temp_col;\nSELECT 1;";
    let parts3 = split_sql(&dialect, sql3).expect("split ALTER IF EXISTS + SELECT");
    assert_eq!(
        parts3.len(),
        2,
        "Expected 2 statements for ALTER IF EXISTS + SELECT, got {}: {:?}",
        parts3.len(),
        parts3
    );
    assert_eq!(
        parts3[0],
        "ALTER TABLE users DROP COLUMN IF EXISTS temp_col"
    );
    assert_eq!(parts3[1], "SELECT 1");

    // 4. IF EXISTS inside procedural BEGIN ... END block followed by SELECT 1
    let sql4 = r#"BEGIN
  DROP TABLE IF EXISTS temp_users;
  CREATE TABLE IF NOT EXISTS temp_users (id INT);
END;
SELECT 1;"#;
    let parts4 = split_sql(&dialect, sql4).expect("split IF EXISTS inside BEGIN END block");
    assert_eq!(
        parts4.len(),
        2,
        "Expected 2 statements for procedural block with IF EXISTS + SELECT, got {}: {:?}",
        parts4.len(),
        parts4
    );
    assert!(parts4[0].starts_with("BEGIN\n  DROP TABLE IF EXISTS temp_users;"));
    assert_eq!(parts4[1].trim(), "SELECT 1");

    // 5. Procedural IF EXISTS condition inside Snowflake Scripting
    let sql5 = r#"BEGIN
  IF EXISTS (SELECT 1 FROM prod_users WHERE active = 0) THEN
    DELETE FROM prod_users WHERE active = 0;
  END IF;
END;
SELECT 1;"#;
    let parts5 = split_sql(&dialect, sql5).expect("split IF EXISTS conditional block");
    assert_eq!(
        parts5.len(),
        2,
        "Expected 2 statements for IF EXISTS condition + SELECT, got {}: {:?}",
        parts5.len(),
        parts5
    );
    assert!(parts5[0].starts_with("BEGIN\n  IF EXISTS"));
    assert_eq!(parts5[1].trim(), "SELECT 1");
}

#[test]
fn test_adversarial_procedural_all_constructs_mega_batch() {
    let dialect = SnowflakeDialect {};

    // A comprehensive batch containing:
    // Stmt 1: DROP TABLE IF EXISTS
    // Stmt 2: CREATE TABLE IF NOT EXISTS
    // Stmt 3: SELECT CASE ... END FOR UPDATE
    // Stmt 4: Giant Snowflake Scripting block with DECLARE, IF, LOOP, FOR, WHILE, REPEAT, and CASE
    // Stmt 5: SELECT 1
    let sql = r#"DROP TABLE IF EXISTS stage_t;
CREATE TABLE IF NOT EXISTS stage_t (id INT, val VARCHAR);
SELECT CASE WHEN id > 0 THEN 'pos' ELSE 'zero' END AS tag FROM stage_t FOR UPDATE;
DECLARE
  total INT DEFAULT 0;
BEGIN
  IF total = 0 THEN
    total := 1;
  ELSEIF total < 0 THEN
    total := 0;
  ELSE
    total := 2;
  END IF;

  FOR i IN 1 TO 5 DO
    total := total + i;
  END FOR;

  WHILE total < 20 DO
    total := total + 1;
  END WHILE;

  LOOP
    total := total + 1;
    IF total >= 25 THEN
      BREAK;
    END IF;
  END LOOP;

  REPEAT
    total := total + 2;
  UNTIL total >= 30
  END REPEAT;

  CASE (total)
    WHEN 30 THEN total := 100;
    ELSE total := 200;
  END CASE;
END;
SELECT 1;"#;

    let parts = split_sql(&dialect, sql).expect("split mega batch");
    assert_eq!(
        parts.len(),
        5,
        "Mega batch must split into exactly 5 statements, got {}: {:?}",
        parts.len(),
        parts
    );
    assert_eq!(parts[0], "DROP TABLE IF EXISTS stage_t");
    assert_eq!(
        parts[1],
        "CREATE TABLE IF NOT EXISTS stage_t (id INT, val VARCHAR)"
    );
    assert_eq!(
        parts[2],
        "SELECT CASE WHEN id > 0 THEN 'pos' ELSE 'zero' END AS tag FROM stage_t FOR UPDATE"
    );
    assert!(parts[3].starts_with("DECLARE\n  total INT DEFAULT 0;\nBEGIN"));
    assert!(parts[3].ends_with("END CASE;\nEND"));
    assert_eq!(parts[4], "SELECT 1");
}

// =========================================================================
// AREA 3: Key-Pair JWT & Auth Edge Cases
// =========================================================================

#[test]
fn account_identifier_extraction_edge_cases() {
    // Normal cases
    assert_eq!(
        extract_account_identifier("https://myorg-myaccount.snowflakecomputing.com"),
        "MYORG-MYACCOUNT"
    );
    assert_eq!(
        extract_account_identifier("http://myorg-myaccount.snowflakecomputing.com"),
        "MYORG-MYACCOUNT"
    );
    assert_eq!(
        extract_account_identifier("myorg-myaccount.snowflakecomputing.com"),
        "MYORG-MYACCOUNT"
    );

    // Case insensitivity
    assert_eq!(
        extract_account_identifier("MyOrg-MyAccount.SnowflakeComputing.Com"),
        "MYORG-MYACCOUNT"
    );

    // Leading and trailing whitespace
    assert_eq!(
        extract_account_identifier("  https://myorg-myaccount.snowflakecomputing.com   "),
        "MYORG-MYACCOUNT"
    );

    // Plain account name without domain
    assert_eq!(
        extract_account_identifier("myorg-myaccount"),
        "MYORG-MYACCOUNT"
    );
    assert_eq!(extract_account_identifier("xy12345"), "XY12345");

    // Account with region (dots in domain)
    assert_eq!(
        extract_account_identifier("xy12345.us-east-1.snowflakecomputing.com"),
        "XY12345"
    );
    assert_eq!(
        extract_account_identifier("myorg-myaccount.privatelink.snowflakecomputing.com"),
        "MYORG-MYACCOUNT"
    );

    // Path suffix
    assert_eq!(
        extract_account_identifier("https://myorg-myaccount.snowflakecomputing.com/"),
        "MYORG-MYACCOUNT"
    );
}

#[test]
fn pem_parsing_and_jwt_generation_end_to_end() {
    // Valid 2048-bit PKCS#8 RSA key PEM
    let pkcs8_pem_lf = concat!(
        "-----BEGIN PRIVATE KEY-----\n",
        "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDAri1x4C2MdUEC\n",
        "wn+ukWwFZ305+Pcs/47bXcvCGjbo4cyzFI+7qSDXAifhBpko1x7xaPXIEHWVHIpt\n",
        "V4nt7JaR7qpYkS9lsWOP6tiUx0VY7JqtfYFZ8RQLNwiKnVkkvcOJoHyGTul7zKEg\n",
        "PApxkkmBzIfSJmn08KBcaduihekqFhD0oEw8bzEmRAUdiZKpQn4wi8SnPa3jH8Pz\n",
        "aUCPXEbs96jY5H8sZaaKYJwFoYGozxeR9eGffpmfDV0DQ0UNo6uo92pG9FjbWAV5\n",
        "VKJXAy3r7jOXUt9l2JjgBvSLLJ8d5k28k3ln53tNp5PHLpp4Ut4/ZNxytOA980Ku\n",
        "GmMQs2kRAgMBAAECggEAEOY17irvU8Zk8U0FzdXLV54ACbmgVD4F1Y+gc2HBNyO2\n",
        "xG4lPRxUWQdDYCUoAcD8YcgG5okrBuz95yEqY8fAXQRFxIbXVV/9j33ViRcBKg92\n",
        "cYj0c2MFkwY8pYuvad2K7OM6iL0bFVSZPS6G6/cco1nA/9mGqYkBR5mxPcvo61vl\n",
        "G5ZV+jsqI3TwxEuM41IAqd5xGhx0eErVOT6iAjxKyT2Q/U29GiNXwzhPSI7pkLCf\n",
        "8BHOyH4q+o+hmulUGK3RvpI38FsfUWAUxvT2n6VUUbh3cCMv1FltAAAJyIZvV7RP\n",
        "A7h3s40DYtQ9uifE4335PNByCLIRWISGEl9EGjCPkQKBgQDnCXySur1WymFbKtY8\n",
        "H1JLxs3eCha1+uDO9pmSDFn2bEQubi7u8TFdSnI2YMeP6GwoWwH/G4EHez5OThf4\n",
        "My2d8BnYAC7EwIB/hscQCLrNGfCro9dMwTLqulcdelxwUmJNrJcEXUz7n2yfXONx\n",
        "va/H4ro9USJFmcFh7GrUW4er0wKBgQDVf72TW4UfAfYSd2hI3ySSzB6ueOssbYhJ\n",
        "bTrITf8P8c9ZJz9X8RMQ1y0sEC1m0CTpIG2clCjic9wOD1woPlE/Oup2566cokMH\n",
        "gSw7V4xuwhmggj3BsfPT2e1mc1Hhj6xRp0H2BToq44NvxAOV951apaHxkDLY4NxA\n",
        "Syg7coN9CwKBgQCMAOGugN7v0yAAAQtlz40tQkHKhc6RZIAKqsEREc8KVz1R7Y8f\n",
        "WKWTZOOlNCx6dlvkcJHbJ62e/P4asZQw4cKI7NCcZi/xJr43iVLj0oZwd4GyYUy8\n",
        "xG64c0cLI/UUU7giXa3+BRP7fvZx8/YYHNyCglBdi+b6ylu3nW53VSf35QKBgQCa\n",
        "mZrMtywprQDklGUGx9czGe5pxxP9Iv/as+cqHxjBEghk9b+vzdlRi4+hZcGKvtTL\n",
        "n/jftoX4nY9O+x5v4dhxohcgrVpLAFDrgUVnX1lI5vMpwMY5mcQHt7pqQr2bSa+t\n",
        "nSFtppFdd0y5RUaXdSwT9PbU9udrF0FGqWXjzmOaywKBgCR3fdtec2VoR019470d\n",
        "hV9abpGfVg27c/OxPubkdqneKEpVVEzMuFEBmjT3lUaggFZoosEaFGgZfX8N4nDa\n",
        "65IszfVM97Q+nq2fkqwu4MceyiN1qhAI1vXbboYaS1GBGfd6EKf0Y3jvmtPqgyk+\n",
        "kX6Z+UIVucFlL1LTco7AMQy2\n",
        "-----END PRIVATE KEY-----\n"
    );

    // Valid 2048-bit PKCS#1 RSA key PEM
    let pkcs1_pem = concat!(
        "-----BEGIN RSA PRIVATE KEY-----\n",
        "MIIEowIBAAKCAQEA4XdDvlioAPi7WNu7TLM+U9xinTd9v0Gs9JlWWt7PMCbgQdGz\n",
        "xhJtYb85lt8VLWCjLtUJ/S4kqHEZhRtjzvBHcAuAAU/BLabi7JKltDAoSqU8xnJ3\n",
        "rm1gWCgSyF6tVt0bXmir/qvQuvhj1VfTGTXAwqav7S8EKP7bO06XEQ/CC5uCqWdE\n",
        "lScDlvBsXmfXdF4GsuKEdFOfatcYrxCntQGSVFxMtGgLlBeRVB8/4dM5WBI7Y0fE\n",
        "rffL/5QIjHV2gMtWajrqmZ0vfeJ2TNfU/5g01OVOr7nY1mtXG8TQxOv0un2CaQCL\n",
        "RW5y5sOnn/OgiaybMQq6gzpWXeiPWmAL3hImQQIDAQABAoIBAACDOecz4mXmKt7f\n",
        "2QQqoCimpp6+4o1Nw+vmcVjRit0MKZRcurDnl9ISEjpylWS+BID1wQkPUswDPdCc\n",
        "qFRvHWp2zDq0iLbm2zs4diH7vgu1Znb5Caae3wo0RR6pgHFjHU3QpRt2HopeSDPS\n",
        "+ErlYWPP5zR919+T/MGcwyEpzQIjvkehwSdYs/I46ThcdfQDNj5ysWM49S0g0YHE\n",
        "Q2qHgh/XaXxpeo68yvKYCpCGYAaPKTckQxrrQv8NYHl4PcR2uisBT0LZRXFzXK2m\n",
        "w4mXfv87uwwOPavMZLXeppELclStRkaT4RpHBZr6OkRN7XhTZm8HD+gDJGUxPYNb\n",
        "vwfwvHECgYEA8km+6VvlPEbfT45iR+/F3V1S2+cGt3oJQDDkjWLnnwg2mH+kiCzi\n",
        "j8aJewVil4Hro3aPhCUIX+gA3b2t97Ge07pXrjIENapXLZD8JRSVKrAgvkvn0YWT\n",
        "qZssz/8ylSUiS/p6CoZj3zXzIEYyMJ79wawRhyXvpH2e55GfjPo/l10CgYEA7jnM\n",
        "zioKS7yNxnBa7TOPIVPzl4Qd1OTJUXlppCBn3FxoJeeCRuW+/yur1RHOLtXUgsmJ\n",
        "cFTC5YoMP7BXaPr+wP/RmOlCkX+w3HgXfzMdhJhZjwn7u6ajQobYir7UN9rCYaOF\n",
        "VYYhHSQ3Mj+mVbWk9EZE6Vt9icjcM6Q50C4aEDUCgYBtvK8GXMPX6DWuf8vNzzCs\n",
        "EV+9D8dABLr54usxinFj20F3EqD/uGypN//+wzKjcom6kLFx5aAEaD00MXwA6ucH\n",
        "+KdL04WKlOtFTRyW4tqo6fr6lcCaaki2Ef1a+7f6S0ZcaUcrSzEi4PmMTZ7f3LMM\n",
        "LBYIQ0tmEiuNsKw2GwYAVQKBgD0wCWO2tphHYN7kQuaEBLJZvspe+86PyQmtNq4a\n",
        "SLL35SQ/N14AarNdwy/h9+Rdtcy1+SAqFTkh4Q2GvJzRQxfNXUd6Okfm1UV7bwWT\n",
        "2nQRMYek18yV+3795aaVZyrFfIHZn0idP+odQDQe1xlSo4C+f7q0biUyuyRkfRj0\n",
        "jms1AoGBAPJBxV+xh5U6lEuV7I1Um4TuVX2YI/P2kMSF7GZDloPiKVTWUyizzRqW\n",
        "k4D7C2XSFCU8AbED/+mNH6/QXN6B6hobta/SFQx1AU3ojcjKk5qSBX5F49E4RAJf\n",
        "EvBhDtoogMHL9f7ZI57f1RW9W2qO97VB7LelJm/g0gmy9COGQZTb\n",
        "-----END RSA PRIVATE KEY-----\n"
    );

    // Test with CRLF line breaks
    let pkcs8_pem_crlf = pkcs8_pem_lf.replace("\n", "\r\n");

    // Test with trailing whitespace on each line
    let pkcs8_pem_ws = pkcs8_pem_lf
        .lines()
        .map(|line| format!("{line}   \n"))
        .collect::<String>();

    // 1. Parsing PKCS#8 LF
    let key_lf = parse_pkcs8_pem(pkcs8_pem_lf, None).expect("parse LF PEM");

    // 2. Parsing PKCS#8 CRLF
    let key_crlf = parse_pkcs8_pem(&pkcs8_pem_crlf, None).expect("parse CRLF PEM");

    // 3. Parsing with trailing spaces
    let key_ws = parse_pkcs8_pem(&pkcs8_pem_ws, None).expect("parse trailing spaces PEM");

    // 4. Parsing PKCS#1
    let key_pkcs1 = parse_pkcs8_pem(pkcs1_pem, None).expect("parse PKCS#1 PEM");

    // 5. Generate JWT with PKCS#8
    let jwt = generate_jwt(&key_lf, "myorg-myaccount", "analyst_user").expect("generate JWT");
    let jwt_str = jwt.as_str();

    // Verify JWT structure: header.claims.signature
    let parts: Vec<&str> = jwt_str.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT must have 3 parts separated by dots");

    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    // Verify Header
    let header_bytes = URL_SAFE_NO_PAD.decode(parts[0]).expect("decode header");
    let header_json: serde_json::Value = serde_json::from_slice(&header_bytes).unwrap();
    assert_eq!(header_json["typ"], "JWT");
    assert_eq!(header_json["alg"], "RS256");

    // Verify Claims
    let claims_bytes = URL_SAFE_NO_PAD.decode(parts[1]).expect("decode claims");
    let claims_json: serde_json::Value = serde_json::from_slice(&claims_bytes).unwrap();
    let sub = claims_json["sub"].as_str().unwrap();
    let iss = claims_json["iss"].as_str().unwrap();

    assert_eq!(sub, "MYORG-MYACCOUNT.ANALYST_USER");
    assert!(
        iss.starts_with("MYORG-MYACCOUNT.ANALYST_USER.SHA256:"),
        "iss must contain account.user.SHA256:<fingerprint>, got: {iss}"
    );

    let iat = claims_json["iat"].as_u64().unwrap();
    let exp = claims_json["exp"].as_u64().unwrap();
    assert_eq!(
        exp - iat,
        3540,
        "Token expiry must be 3540 seconds (59 min)"
    );

    // Verify CRLF, WS, and PKCS#1 keys can also generate valid JWTs
    assert!(generate_jwt(&key_crlf, "myorg-myaccount", "user").is_ok());
    assert!(generate_jwt(&key_ws, "myorg-myaccount", "user").is_ok());
    assert!(generate_jwt(&key_pkcs1, "myorg-myaccount", "user").is_ok());

    // Corrupted PEM test
    let corrupt_pem = "-----BEGIN PRIVATE KEY-----\nNotValidBase64!!!\n-----END PRIVATE KEY-----";
    assert!(parse_pkcs8_pem(corrupt_pem, None).is_err());

    // Empty PEM test
    assert!(parse_pkcs8_pem("", None).is_err());
}
