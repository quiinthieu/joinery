use super::*;
use crate::domain::QueryParamKind;

fn input(key: &str, kind: QueryParamKind, value: Option<&str>) -> QueryParamInput {
    QueryParamInput {
        key: key.into(),
        kind,
        value: value.map(str::to_owned),
    }
}
fn compiled(sql: &str, mode: Mode, inputs: &[QueryParamInput]) -> CompiledScript {
    compile(sql, mode, Some(inputs)).unwrap_or_else(|_| panic!("expected valid fixture"))
}

#[test]
fn mixed_sequence_preserves_order_boundaries_and_full_names() {
    let sql = "SELECT 1; BEGIN; INSERT INTO DB.SC.T(ID,V) VALUES(1,'x'); UPDATE DB.SC.T SET V='y' WHERE ID=1; SELECT V FROM DB.SC.T; DELETE FROM DB.SC.T WHERE ID=1; COMMIT; BEGIN; INSERT INTO DB.SC.T VALUES(2,'z'); ROLLBACK; SELECT 2";
    let script = compiled(sql, Mode::ParsedStatements, &[]);
    assert_eq!(script.units().len(), 11);
    assert_eq!(script.units()[1].bound().rewritten_sql, "BEGIN");
    assert_eq!(script.units()[6].original_sql(), "COMMIT");
    assert_eq!(script.units()[9].original_sql(), "ROLLBACK");
    assert!(
        script
            .units()
            .iter()
            .all(|u| u.validation() == Validation::LocalSyntax)
    );
    assert_eq!(script.original_sql(), sql);
    assert_eq!(script.mode(), Mode::ParsedStatements);
}

#[test]
fn broader_sql_is_not_restricted_to_generated_mutation_grammar() {
    let sql = "SHOW TABLES; SELECT * FROM TABLE(RESULT_SCAN(LAST_QUERY_ID())); USE DATABASE DB; SET V=1; CREATE TEMP TABLE DB.SC.T(ID INT); TRUNCATE TABLE DB.SC.T; CALL DB.SC.P()";
    let script = compiled(sql, Mode::ParsedStatements, &[]);
    assert_eq!(script.units().len(), 7);
    assert_eq!(script.units()[6].original_sql(), "CALL DB.SC.P()");
}

#[test]
fn anonymous_bind_slots_are_global_while_wire_order_is_local() {
    let inputs = [
        input("?1", QueryParamKind::Integer, Some("9007199254740993")),
        input("?2", QueryParamKind::Text, Some("'); COMMIT; --")),
        input("?3", QueryParamKind::Boolean, Some("true")),
    ];
    let script = compiled(
        "SELECT ?; INSERT INTO DB.SC.T(V) VALUES(?); SELECT ?",
        Mode::ParsedStatements,
        &inputs,
    );
    assert_eq!(
        script.units()[0].bound().rewritten_sql,
        "SELECT CAST(? AS NUMBER(38,0))"
    );
    assert_eq!(script.units()[0].bound().binds[0].key, "?1");
    assert_eq!(
        script.units()[1].bound().rewritten_sql,
        "INSERT INTO DB.SC.T(V) VALUES(?)"
    );
    assert_eq!(script.units()[1].bound().binds[0].key, "?2");
    assert_eq!(
        script.units()[1].bound().binds[0].value.as_deref(),
        Some("'); COMMIT; --")
    );
    assert_eq!(script.units()[2].bound().binds[0].key, "?3");
}

#[test]
fn repeated_named_values_and_null_share_inputs_without_interpolation() {
    let inputs = [
        input("id", QueryParamKind::Integer, Some("17")),
        input("v", QueryParamKind::Text, None),
    ];
    let script = compiled(
        "SELECT :id; UPDATE DB.SC.T SET V=:v WHERE ID=:id; SELECT :v,:id",
        Mode::ParsedStatements,
        &inputs,
    );
    assert_eq!(script.inputs().len(), 2);
    assert_eq!(
        script.units()[1]
            .bound()
            .binds
            .iter()
            .map(|b| b.key.as_str())
            .collect::<Vec<_>>(),
        vec!["v", "id"]
    );
    assert!(script.units()[1].bound().binds[0].value.is_none());
    assert!(!script.units()[1].bound().rewritten_sql.contains("17"));
}

#[test]
fn semicolons_and_parameter_lookalikes_in_literals_comments_stay_inert() {
    let sql = "-- 😀 :ignored ;\nSELECT 'snow;? :not_a_bind', $$body;:ignored$$; /* ; */ SELECT 2";
    let script = compiled(sql, Mode::ParsedStatements, &[]);
    assert_eq!(script.units().len(), 2);
    assert!(script.units().iter().all(|u| u.bound().binds.is_empty()));
    for unit in script.units() {
        let extracted = String::from_utf16(
            &sql.encode_utf16()
                .skip(unit.span().start_utf16)
                .take(unit.span().end_utf16 - unit.span().start_utf16)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(extracted, unit.original_sql());
    }
}

#[test]
fn procedural_selection_remains_one_exact_server_validated_unit() {
    let sql = "EXECUTE IMMEDIATE $$ BEGIN LET X INTEGER := 1; RETURN X; END; $$;";
    let script = compiled(sql, Mode::SelectedUnit, &[]);
    assert_eq!(script.units().len(), 1);
    assert_eq!(
        script.units()[0].validation(),
        Validation::ServerAtExecution
    );
    assert_eq!(script.units()[0].bound().rewritten_sql, sql);
    // A bare scripting block may be unsupported locally, but cannot be naively
    // divided into independently executable fragments.
    assert!(
        compile(
            "BEGIN LET X INTEGER := 1; RETURN X; END;",
            Mode::ParsedStatements,
            None
        )
        .is_err()
    );
}

#[test]
fn selected_unit_does_not_claim_to_prevalidate_server_dialect_or_statement_count() {
    for sql in ["SELECT 1; SELECT 2", "FUTURE_SNOWFLAKE_SYNTAX X"] {
        let script = compiled(sql, Mode::SelectedUnit, &[]);
        assert_eq!(script.units().len(), 1);
        assert_eq!(
            script.units()[0].validation(),
            Validation::ServerAtExecution
        );
        assert_eq!(script.units()[0].original_sql(), sql);
    }
}

#[test]
fn later_local_syntax_or_binding_error_rejects_complete_plan() {
    assert!(compile("SELECT 1; INSERT INTO", Mode::ParsedStatements, None).is_err());
    assert!(compile("SELECT 1; SELECT :missing", Mode::ParsedStatements, None).is_err());
    let provided = [
        input("x", QueryParamKind::Text, Some("ok")),
        input("unused", QueryParamKind::Text, Some("no")),
    ];
    assert!(compile("SELECT :x", Mode::ParsedStatements, Some(&provided)).is_err());
    assert!(
        compile(
            "SELECT :x",
            Mode::ParsedStatements,
            Some(&[provided[0].clone(), provided[0].clone()])
        )
        .is_err()
    );
}

#[test]
fn canonical_verification_rejects_forged_sql_order_and_bind_metadata() {
    let script = compiled(
        "SELECT :x; SELECT :y",
        Mode::ParsedStatements,
        &[
            input("x", QueryParamKind::Integer, Some("17")),
            input("y", QueryParamKind::Integer, Some("29")),
        ],
    );
    let original = script
        .units()
        .iter()
        .map(|u| u.bound().clone())
        .collect::<Vec<_>>();
    assert!(script.verify(&original).is_ok());
    let mut swapped = original.clone();
    swapped.swap(0, 1);
    assert!(script.verify(&swapped).is_err());
    let mut forged = original.clone();
    forged[1].rewritten_sql.push_str("; COMMIT");
    assert!(script.verify(&forged).is_err());
    let mut forged = original.clone();
    forged[0].binds[0].value = Some("99".into());
    assert!(script.verify(&forged).is_err());
    let mut forged = original;
    forged[0].binds[0].kind = QueryParamKind::Text;
    assert!(script.verify(&forged).is_err());
    assert!(script.verify(&[]).is_err());
}

#[test]
fn file_transfer_branch_is_rejected_before_any_unit_can_be_admitted() {
    for sql in [
        "PUT file:///tmp/x @stage",
        "get @stage file:///tmp/x",
        "\u{feff}\u{a0}GeT @stage file:///tmp/x",
        "SELECT 1; PUT file:///tmp/x @stage",
    ] {
        assert!(
            compile(
                sql,
                if sql.starts_with("SELECT") {
                    Mode::ParsedStatements
                } else {
                    Mode::SelectedUnit
                },
                None
            )
            .is_err()
        );
    }
    // Same prefix behavior as pinned SDK Util, not a broad SQL classifier.
    assert!(!client_file_transfer(
        "/* comment */ PUT file:///tmp/x @stage"
    ));
    assert!(!client_file_transfer("\u{85}GET @stage file:///tmp/x"));
    assert!(client_file_transfer("\u{2003}PUTTING"));
}

#[test]
fn limits_reject_empty_excess_units_oversized_units_and_parameters() {
    for sql in ["", "  ", "-- comment only", "SELECT '\0'"] {
        assert!(compile(sql, Mode::ParsedStatements, None).is_err());
    }
    assert!(
        compile(
            &"SELECT 1;".repeat(MAX_UNITS + 1),
            Mode::ParsedStatements,
            None
        )
        .is_err()
    );
    assert!(compile(&"x".repeat(MAX_UNIT_BYTES + 1), Mode::SelectedUnit, None).is_err());
    assert!(compile(&"x".repeat(MAX_SCRIPT_BYTES + 1), Mode::SelectedUnit, None).is_err());
    let sql = (0..65)
        .map(|i| format!(":v{i}"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(compile(&format!("SELECT {sql}"), Mode::ParsedStatements, None).is_err());
}

#[test]
fn binds_and_rewritten_sql_must_fit_each_complete_helper_envelope() {
    let value = "x".repeat(256 * 1024);
    let input = input("x", QueryParamKind::Text, Some(&value));
    let failure = compile("SELECT :x,:x,:x,:x", Mode::ParsedStatements, Some(&[input]))
        .err()
        .expect("oversized frame");
    assert!(failure.message.contains("frame budget"));
    let sql = format!("SELECT :x{}", " ".repeat(MAX_UNIT_BYTES - 9));
    // Selection preserves whitespace, so generated casts cannot escape the
    // helper's 256 KiB wire SQL limit.
    let failure = compile(&sql, Mode::SelectedUnit, Some(&[input_for_integer()]))
        .err()
        .expect("oversized rewritten SQL");
    assert!(failure.message.contains("SQL/bind budget"));
}

fn input_for_integer() -> QueryParamInput {
    input("x", QueryParamKind::Integer, Some("1"))
}

#[test]
fn aggregate_budget_counts_json_expansion_of_repeated_bound_values() {
    let value = "\n".repeat(4096);
    let input = input("x", QueryParamKind::Text, Some(&value));
    let statement = format!("SELECT {};", vec![":x"; 16].join(","));
    // 1024 occurrences stay within the scanner limit and raw typed data stays
    // below 8 MiB, but JSON escaping makes the complete review exceed 8 MiB.
    assert!(
        compile(
            &statement.repeat(32),
            Mode::ParsedStatements,
            Some(std::slice::from_ref(&input))
        )
        .is_ok()
    );
    let failure = compile(
        &statement.repeat(64),
        Mode::ParsedStatements,
        Some(&[input]),
    )
    .err()
    .expect("encoded aggregate overflow");
    assert!(failure.message.contains("Encoded script"));
}
