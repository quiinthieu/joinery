use joinery_core::domain::{
    ConnectionProfile, EngineKind, Environment, QueryParamInput, QueryParamKind, SafeMode,
    Scratchpad, TlsMode,
};
use joinery_core::params::{compile_statement, extract_query_params, validate_inputs};
use joinery_core::persistence::Persistence;
use serde_json::json;
use std::sync::Arc;
use tempfile::tempdir;

fn make_test_profile(id: &str, name: &str) -> ConnectionProfile {
    ConnectionProfile {
        id: id.to_owned(),
        name: name.to_owned(),
        group_id: "local".to_owned(),
        engine: EngineKind::Postgres,
        host: "127.0.0.1".to_owned(),
        port: 5432,
        database: "test_db".to_owned(),
        username: "test_user".to_owned(),
        environment: Environment::Local,
        safe_mode: SafeMode::Silent,
        tls_mode: TlsMode::Disable,
        ca_cert_path: None,
        ssh: None,
        remember_password: false,
        store_query_text: true,
        store_drafts: true,
        snowflake: None,
    }
}

// =========================================================================
// CHALLENGE AREA 1: Rapid Concurrent Auto-Saves to Consoles & Scratchpads
// =========================================================================

#[test]
fn adversarial_concurrent_rapid_autosaves_consoles_no_deadlock_or_corruption() {
    let dir = tempdir().unwrap();
    let persistence = Arc::new(Persistence::open(dir.path()).unwrap());

    let profile = make_test_profile("prof-concurrent", "Concurrent Test");
    persistence.save_profile(&profile).unwrap();

    let num_threads = 24;
    let iterations_per_thread = 50;

    let mut handles = Vec::new();
    for thread_idx in 0..num_threads {
        let p = Arc::clone(&persistence);
        handles.push(std::thread::spawn(move || {
            for iter in 0..iterations_per_thread {
                let content = format!(
                    "-- Thread {thread_idx} Iteration {iter}\nSELECT {thread_idx}, {iter};"
                );
                p.save_console("prof-concurrent", &content, Some("console.sql"))
                    .unwrap();
                let loaded = p.get_console("prof-concurrent").unwrap();
                assert_eq!(loaded.profile_id, "prof-concurrent");
                assert_eq!(loaded.name, "console.sql");
                assert!(!loaded.content.is_empty());
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Verify final state is valid and non-corrupted
    let final_console = persistence.get_console("prof-concurrent").unwrap();
    assert_eq!(final_console.profile_id, "prof-concurrent");
    assert!(final_console.content.contains("SELECT "));
}

#[test]
fn adversarial_concurrent_rapid_autosaves_scratchpads_multi_instance() {
    let dir = tempdir().unwrap();
    let persistence = Arc::new(Persistence::open(dir.path()).unwrap());

    let num_scratchpads = 10;
    let num_writers = 20;
    let iterations = 30;

    // Pre-create scratchpads
    for i in 0..num_scratchpads {
        persistence
            .save_scratchpad(&Scratchpad {
                id: format!("sp-concurrent-{i}"),
                name: format!("Scratchpad {i}"),
                content: format!("-- Initial {i}\n"),
                connection_id: None,
                updated_at: 0,
            })
            .unwrap();
    }

    let mut handles = Vec::new();
    for writer_idx in 0..num_writers {
        let p = Arc::clone(&persistence);
        handles.push(std::thread::spawn(move || {
            for iter in 0..iterations {
                let sp_idx = (writer_idx + iter) % num_scratchpads;
                let id = format!("sp-concurrent-{sp_idx}");
                let content = format!("-- Writer {writer_idx} Iter {iter}\nSELECT {writer_idx};");
                let saved = p
                    .save_scratchpad(&Scratchpad {
                        id: id.clone(),
                        name: format!("Scratchpad {sp_idx}"),
                        content,
                        connection_id: None,
                        updated_at: 0,
                    })
                    .unwrap();
                assert_eq!(saved.id, id);
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let all = persistence.scratchpads().unwrap();
    assert_eq!(all.len(), num_scratchpads);
    for sp in all {
        assert!(sp.content.contains("SELECT "));
    }
}

// =========================================================================
// CHALLENGE AREA 2: Scratchpad Creation with Tricky Inputs & Serialization Mismatch
// =========================================================================

#[test]
fn adversarial_scratchpad_special_symbols_unicode_and_extreme_sizes() {
    let dir = tempdir().unwrap();
    let persistence = Persistence::open(dir.path()).unwrap();

    // 1. Unicode, emojis, RTL, zero-width spaces in name
    let tricky_names = [
        "🧪 数据库 🚀 Scratchpad",
        "العربية_تجربة",
        "zero\u{200B}width\u{FEFF}spaces",
        "<script>alert('xss')</script>",
        "Robert'); DROP TABLE scratchpads;--",
        "\"quotes\" and 'single' and `backticks`",
        "",                // empty name
        "   ",             // whitespace only
        &"A".repeat(1000), // very long name
    ];

    for (idx, &name) in tricky_names.iter().enumerate() {
        let id = format!("sp-tricky-{idx}");
        let content = format!("-- Content for {name}\nSELECT 1;");
        let saved = persistence
            .save_scratchpad(&Scratchpad {
                id: id.clone(),
                name: name.to_owned(),
                content: content.clone(),
                connection_id: None,
                updated_at: 0,
            })
            .unwrap();

        assert_eq!(saved.id, id);
        assert_eq!(saved.name, name);
        assert_eq!(saved.content, content);
    }

    // Retrieve all and verify they are stored verbatim
    let all = persistence.scratchpads().unwrap();
    assert_eq!(all.len(), tricky_names.len());

    // 2. Giant content (1 MB)
    let giant_sql = format!("SELECT '{}';", "x".repeat(1024 * 1024));
    let giant_sp = persistence
        .save_scratchpad(&Scratchpad {
            id: "sp-giant".into(),
            name: "Giant Content".into(),
            content: giant_sql.clone(),
            connection_id: None,
            updated_at: 0,
        })
        .unwrap();
    assert_eq!(giant_sp.content.len(), giant_sql.len());

    let loaded = persistence.scratchpads().unwrap();
    let found = loaded.iter().find(|s| s.id == "sp-giant").unwrap();
    assert_eq!(found.content.len(), giant_sql.len());

    // 3. Deletion lifecycle
    persistence.delete_scratchpad("sp-giant").unwrap();
    let reloaded = persistence.scratchpads().unwrap();
    assert!(reloaded.iter().all(|s| s.id != "sp-giant"));
}

#[test]
fn adversarial_cascade_profile_deletion_disassociates_scratchpads_and_purges_consoles() {
    let dir = tempdir().unwrap();
    let persistence = Persistence::open(dir.path()).unwrap();

    let prof_id = "prof-to-delete";
    let profile = make_test_profile(prof_id, "Delete Target");
    persistence.save_profile(&profile).unwrap();

    // Attach console
    persistence
        .save_console(prof_id, "SELECT 'console';", None)
        .unwrap();

    // Attach parameter history
    persistence
        .save_parameter_history(
            prof_id,
            &[QueryParamInput {
                key: "param1".into(),
                kind: QueryParamKind::Text,
                value: Some("val1".into()),
            }],
        )
        .unwrap();

    // Attach scratchpad bound to connection
    persistence
        .save_scratchpad(&Scratchpad {
            id: "sp-bound".into(),
            name: "Bound SP".into(),
            content: "SELECT 1;".into(),
            connection_id: Some(prof_id.into()),
            updated_at: 0,
        })
        .unwrap();

    // Delete profile
    persistence.delete_profile(prof_id).unwrap();

    // Verify console is purged (get_console returns default template)
    let console = persistence.get_console(prof_id).unwrap();
    assert!(console.content.contains("-- Persistent database console"));

    // Verify parameters are purged
    let params = persistence.parameter_history(prof_id).unwrap();
    assert!(params.is_empty());

    // Verify scratchpad is NOT deleted, but its connection_id is nullified!
    let sps = persistence.scratchpads().unwrap();
    let bound_sp = sps.iter().find(|s| s.id == "sp-bound").unwrap();
    assert_eq!(bound_sp.connection_id, None);
}

#[test]
fn adversarial_scratchpad_ipc_payload_mismatch_reproduction() {
    // This test empirically validates the contract mismatch between what frontend studio.ts
    // sends vs what Scratchpad deserializer expects.

    // 1. Frontend createScratchpad passes: { name: "Scratchpad 1", content: "-- ...", connectionId: undefined }
    let frontend_create_payload = json!({
        "name": "Scratchpad 1",
        "content": "-- Scratchpad\n",
    });

    // Deserializing this directly into Scratchpad FAILS because 'id' and 'updatedAt' are missing:
    let direct_deser = serde_json::from_value::<Scratchpad>(frontend_create_payload);
    assert!(
        direct_deser.is_err(),
        "Empirically confirmed: Scratchpad deserialization fails when 'id' is missing!"
    );

    // 2. Frontend auto-save debounced writer passes: { id: "sp-1", content: "SELECT 2;" }
    let frontend_autosave_payload = json!({
        "id": "sp-1",
        "content": "SELECT 2;",
    });

    let autosave_deser = serde_json::from_value::<Scratchpad>(frontend_autosave_payload);
    assert!(
        autosave_deser.is_err(),
        "Empirically confirmed: Scratchpad deserialization fails when 'name' is missing!"
    );

    // 3. Test-harness routes save_scratchpad via: arg(&args, "scratchpad")
    // When frontend calls client.call("save_scratchpad", { ... }), the payload does NOT have a "scratchpad" key:
    let rpc_args_without_wrapper = json!({
        "name": "Scratchpad 1",
        "content": "-- Scratchpad\n",
    });
    assert!(
        rpc_args_without_wrapper.get("scratchpad").is_none(),
        "Empirically confirmed: frontend does not wrap save_scratchpad arguments under 'scratchpad' key!"
    );
}

// =========================================================================
// CHALLENGE AREA 3: Parameter History LRU Evictions & Profile Isolation
// =========================================================================

#[test]
fn adversarial_parameter_history_profile_isolation() {
    let dir = tempdir().unwrap();
    let persistence = Persistence::open(dir.path()).unwrap();

    let prof_a = "tenant-alpha";
    let prof_b = "tenant-beta";

    persistence
        .save_parameter_history(
            prof_a,
            &[
                QueryParamInput {
                    key: "secret_token".into(),
                    kind: QueryParamKind::Text,
                    value: Some("alpha_secret_999".into()),
                },
                QueryParamInput {
                    key: "org_id".into(),
                    kind: QueryParamKind::Integer,
                    value: Some("1001".into()),
                },
            ],
        )
        .unwrap();

    persistence
        .save_parameter_history(
            prof_b,
            &[
                QueryParamInput {
                    key: "secret_token".into(),
                    kind: QueryParamKind::Text,
                    value: Some("beta_secret_777".into()),
                },
                QueryParamInput {
                    key: "shared_param".into(),
                    kind: QueryParamKind::Boolean,
                    value: Some("true".into()),
                },
            ],
        )
        .unwrap();

    let hist_a = persistence.parameter_history(prof_a).unwrap();
    let hist_b = persistence.parameter_history(prof_b).unwrap();

    assert_eq!(hist_a.len(), 2);
    assert_eq!(hist_b.len(), 2);

    let token_a = hist_a
        .iter()
        .find(|e| e.param_key == "secret_token")
        .unwrap();
    let token_b = hist_b
        .iter()
        .find(|e| e.param_key == "secret_token")
        .unwrap();

    assert_eq!(token_a.value.as_deref(), Some("alpha_secret_999"));
    assert_eq!(token_b.value.as_deref(), Some("beta_secret_777"));

    assert!(hist_a.iter().all(|e| e.param_key != "shared_param"));
    assert!(hist_b.iter().all(|e| e.param_key != "org_id"));
}

#[test]
fn adversarial_parameter_history_bounds_and_null_handling() {
    let dir = tempdir().unwrap();
    let persistence = Persistence::open(dir.path()).unwrap();
    let prof_id = "prof-bounds";

    // 1. Test updating parameter in-place preserves single entry with updated timestamp
    persistence
        .save_parameter_history(
            prof_id,
            &[QueryParamInput {
                key: "counter".into(),
                kind: QueryParamKind::Integer,
                value: Some("1".into()),
            }],
        )
        .unwrap();

    persistence
        .save_parameter_history(
            prof_id,
            &[QueryParamInput {
                key: "counter".into(),
                kind: QueryParamKind::Integer,
                value: Some("2".into()),
            }],
        )
        .unwrap();

    let history = persistence.parameter_history(prof_id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].value.as_deref(), Some("2"));

    // 2. Test NULL vs empty string preservation
    persistence
        .save_parameter_history(
            prof_id,
            &[
                QueryParamInput {
                    key: "null_param".into(),
                    kind: QueryParamKind::Text,
                    value: None,
                },
                QueryParamInput {
                    key: "empty_param".into(),
                    kind: QueryParamKind::Text,
                    value: Some("".into()),
                },
            ],
        )
        .unwrap();

    let reloaded = persistence.parameter_history(prof_id).unwrap();
    let null_entry = reloaded
        .iter()
        .find(|e| e.param_key == "null_param")
        .unwrap();
    assert!(null_entry.is_null);
    assert_eq!(null_entry.value, None);

    let empty_entry = reloaded
        .iter()
        .find(|e| e.param_key == "empty_param")
        .unwrap();
    assert!(!empty_entry.is_null);
    assert_eq!(empty_entry.value.as_deref(), Some(""));

    // 3. Test parameter_history LIMIT 500
    let bulk_inputs: Vec<QueryParamInput> = (0..600)
        .map(|i| QueryParamInput {
            key: format!("param_{i:04}"),
            kind: QueryParamKind::Integer,
            value: Some(i.to_string()),
        })
        .collect();

    persistence
        .save_parameter_history(prof_id, &bulk_inputs)
        .unwrap();
    let limited = persistence.parameter_history(prof_id).unwrap();
    assert_eq!(
        limited.len(),
        500,
        "parameter_history should enforce LIMIT 500"
    );
}

// =========================================================================
// CHALLENGE AREA 4: Parameter Value Substitutions with Tricky Inputs
// =========================================================================

#[test]
fn adversarial_parameter_value_sql_injection_attempts_never_interpolate() {
    let injection_payloads = [
        "' OR '1'='1",
        "'; DROP TABLE orders; --",
        "' UNION SELECT username, password FROM users --",
        "admin'--",
        "\\'; DROP TABLE users; --",
        "\" OR 1=1 --",
        "1; EXEC xp_cmdshell('dir')",
        "/* comment */ ' OR 1=1 /*",
    ];

    for engine in [
        EngineKind::Postgres,
        EngineKind::Mysql,
        EngineKind::Snowflake,
    ] {
        let sql = "SELECT * FROM users WHERE username = :user AND role = $role";
        let slots = extract_query_params(engine, sql).unwrap();
        assert_eq!(slots.len(), 2);

        for &injection in &injection_payloads {
            let inputs = vec![
                QueryParamInput {
                    key: "user".into(),
                    kind: QueryParamKind::Text,
                    value: Some(injection.to_owned()),
                },
                QueryParamInput {
                    key: "$role".into(),
                    kind: QueryParamKind::Text,
                    value: Some("standard".into()),
                },
            ];

            let (canonical, map) = validate_inputs(&slots, Some(&inputs)).unwrap();
            assert_eq!(canonical[0].value.as_deref(), Some(injection));

            let mut offset = 0;
            let bound = compile_statement(engine, sql, &map, &mut offset)
                .unwrap()
                .unwrap();

            // The rewritten SQL MUST NOT contain the unescaped injection text!
            assert!(
                !bound.rewritten_sql.contains(injection),
                "VULNERABILITY: Injection string was directly interpolated into SQL: {}",
                bound.rewritten_sql
            );

            // Verify placeholders were used properly
            match engine {
                EngineKind::Postgres => {
                    assert!(bound.rewritten_sql.contains("$1"));
                    assert!(bound.rewritten_sql.contains("$2"));
                }
                EngineKind::Mysql => {
                    assert!(bound.rewritten_sql.contains("CONVERT(? USING utf8mb4)"));
                }
                EngineKind::Snowflake => {
                    assert!(bound.rewritten_sql.contains("?"));
                }
                EngineKind::Redis => unreachable!(),
            }

            // Verify binds hold the literal value untouched
            assert_eq!(bound.binds[0].value.as_deref(), Some(injection));
        }
    }
}

#[test]
fn adversarial_parameter_validation_boundary_conditions() {
    let sql = "SELECT :json_val, :dec_val, :int_val, :bool_val, :date_val, :ts_val, :uuid_val";
    let slots = extract_query_params(EngineKind::Postgres, sql).unwrap();

    // 1. Malformed JSON must be rejected
    for bad_json in [
        "{unclosed",
        "null_unquoted",
        "NaN",
        "undefined",
        "{'single_quotes': 1}",
    ] {
        let input = QueryParamInput {
            key: "json_val".into(),
            kind: QueryParamKind::Json,
            value: Some(bad_json.into()),
        };
        assert!(validate_inputs(&slots[0..1], Some(&[input])).is_err());
    }

    // 2. Valid complex JSON must be accepted
    let complex_json = r#"{"nested": [1, true, null, {"a": "b"}], "unicode": "🧪🚀"}"#;
    let ok_json = QueryParamInput {
        key: "json_val".into(),
        kind: QueryParamKind::Json,
        value: Some(complex_json.into()),
    };
    assert!(validate_inputs(&slots[0..1], Some(&[ok_json])).is_ok());

    // 3. Exponent notation in Decimal must be rejected
    for bad_dec in ["1e5", "1.5E-3", "1E+2", "1.2.3", "abc", "+-12"] {
        let input = QueryParamInput {
            key: "dec_val".into(),
            kind: QueryParamKind::Decimal,
            value: Some(bad_dec.into()),
        };
        assert!(validate_inputs(&slots[1..2], Some(&[input])).is_err());
    }

    // 4. Decimal exceeding precision bounds (65 digits)
    let huge_dec = format!("{}.{}", "9".repeat(40), "9".repeat(30)); // 70 digits
    let input_huge_dec = QueryParamInput {
        key: "dec_val".into(),
        kind: QueryParamKind::Decimal,
        value: Some(huge_dec),
    };
    assert!(validate_inputs(&slots[1..2], Some(&[input_huge_dec])).is_err());

    // 5. Snowflake decimal precision > 38 rejected at compile time
    let sf_dec = "1234567890123456789012345678901234567890"; // 40 digits
    let (_, map) = validate_inputs(
        &slots[1..2],
        Some(&[QueryParamInput {
            key: "dec_val".into(),
            kind: QueryParamKind::Decimal,
            value: Some(sf_dec.into()),
        }]),
    )
    .unwrap();
    let mut offset = 0;
    let compile_res =
        compile_statement(EngineKind::Snowflake, "SELECT :dec_val", &map, &mut offset);
    assert!(compile_res.is_err());

    // 6. Integer overflow rejected
    for bad_int in [
        "9223372036854775808",
        "-9223372036854775809",
        "0x1F",
        "1.0",
        "1 2",
    ] {
        let input = QueryParamInput {
            key: "int_val".into(),
            kind: QueryParamKind::Integer,
            value: Some(bad_int.into()),
        };
        assert!(validate_inputs(&slots[2..3], Some(&[input])).is_err());
    }

    // 7. Boolean invalid strings rejected
    for bad_bool in ["maybe", "2", "-1", "ok", "truce"] {
        let input = QueryParamInput {
            key: "bool_val".into(),
            kind: QueryParamKind::Boolean,
            value: Some(bad_bool.into()),
        };
        assert!(validate_inputs(&slots[3..4], Some(&[input])).is_err());
    }

    // 8. Date leap year and calendar validation
    let bad_dates = [
        "2023-02-29",
        "2024-02-30",
        "2024-04-31",
        "2024-13-01",
        "2024-00-01",
    ];
    for bad_date in bad_dates {
        let input = QueryParamInput {
            key: "date_val".into(),
            kind: QueryParamKind::Date,
            value: Some(bad_date.into()),
        };
        assert!(validate_inputs(&slots[4..5], Some(&[input])).is_err());
    }

    // 9. Timestamp timezone offset bounds (+14:00 is max, +14:01 is invalid)
    let bad_ts = [
        "2026-10-31T12:00:00+14:01",
        "2026-10-31T12:00:00-15:00",
        "2026-10-31 25:00:00",
    ];
    for ts in bad_ts {
        let input = QueryParamInput {
            key: "ts_val".into(),
            kind: QueryParamKind::Timestamp,
            value: Some(ts.into()),
        };
        assert!(validate_inputs(&slots[5..6], Some(&[input])).is_err());
    }

    // 10. UUID validation
    for bad_uuid in [
        "not-a-uuid",
        "550e8400-e29b-41d4-a716",
        "550e8400-e29b-41d4-a716-44665544000z",
    ] {
        let input = QueryParamInput {
            key: "uuid_val".into(),
            kind: QueryParamKind::Uuid,
            value: Some(bad_uuid.into()),
        };
        assert!(validate_inputs(&slots[6..7], Some(&[input])).is_err());
    }
}

#[test]
fn adversarial_parameter_history_lru_pruning_bounds() {
    let dir = tempdir().unwrap();
    let persistence = Persistence::open(dir.path()).unwrap();
    let prof_id = "prof-lru";

    // Insert 1200 distinct parameters in batches
    let bulk_inputs: Vec<QueryParamInput> = (0..1200)
        .map(|i| QueryParamInput {
            key: format!("param_{i:04}"),
            kind: QueryParamKind::Integer,
            value: Some(i.to_string()),
        })
        .collect();

    persistence
        .save_parameter_history(prof_id, &bulk_inputs)
        .unwrap();

    // Verify raw SQLite row count is pruned to exactly 1000
    let raw_conn = rusqlite::Connection::open(dir.path().join("joinery.sqlite")).unwrap();
    let count: i64 = raw_conn
        .query_row(
            "SELECT COUNT(*) FROM parameters WHERE profile_id = ?1",
            [prof_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1000,
        "parameters table must be pruned to max 1000 rows per profile"
    );
}
