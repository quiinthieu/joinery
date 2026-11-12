use super::*;

async fn mapping_request(h: &Harness) -> InspectRelationConstraintMappingsRequest {
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = catalog
        .relations
        .iter()
        .find(|target| target.table.schema == "PUBLIC")
        .unwrap();
    InspectRelationConstraintMappingsRequest {
        revision: catalog.revision,
        target_id: target.target_id.clone(),
    }
}
fn mapping_count(h: &Harness) -> usize {
    h.queries()
        .iter()
        .filter(|sql| {
            sql.starts_with("SHOW PRIMARY KEYS IN TABLE ")
                || sql.starts_with("SHOW UNIQUE KEYS IN TABLE ")
                || sql.starts_with("SHOW IMPORTED KEYS IN TABLE ")
        })
        .count()
}
fn primary(column: &str, ordinal: &str) -> Value {
    json!(["DB", "PUBLIC", "ORDERS", column, ordinal, "ORDERS_PK"])
}
fn declared(name: &str, kind: &str) -> Value {
    let mut row = vec![
        json!("DB"),
        json!("PUBLIC"),
        json!("ORDERS"),
        json!("DB"),
        json!("PUBLIC"),
        json!(name),
        json!(kind),
        json!("NO"),
        json!("YES"),
        json!("NO"),
        json!("NO"),
    ];
    row.extend(std::iter::repeat_n(Value::Null, 10));
    json!(row)
}

#[tokio::test]
async fn mappings_read_composite_pk_unique_fk_independently_on_the_retained_connection() {
    let h = Harness::open().await;
    let request = mapping_request(&h).await;
    let start = h.queries().len();
    let result = h
        .driver
        .inspect_relation_constraint_mappings(&request)
        .await
        .unwrap();
    assert_eq!(
        (
            &*result.revision,
            &*result.target_id,
            &*result.database,
            &*result.table.schema,
            &*result.table.name
        ),
        (
            &*request.revision,
            &*request.target_id,
            "DB",
            "PUBLIC",
            "ORDERS"
        )
    );
    assert_eq!(result.mappings.len(), 3);
    assert!(
        result
            .mappings
            .iter()
            .all(|m| serde_json::to_value(m.status).unwrap() == "available")
    );
    for mapping in &result.mappings {
        assert_eq!(
            mapping
                .columns
                .iter()
                .map(|c| c.ordinal)
                .collect::<Vec<_>>(),
            [1, 2]
        );
    }
    assert_eq!(result.mappings[0].columns[0].column_name, "A");
    assert!(result.mappings[0].referenced_table.is_none());
    assert_eq!(result.mappings[2].columns[0].column_name, "PARENT_A");
    assert_eq!(
        result.mappings[2].columns[0]
            .referenced_column_name
            .as_deref(),
        Some("A")
    );
    assert_eq!(
        result.mappings[2]
            .referenced_table
            .as_ref()
            .unwrap()
            .table
            .name,
        "PARENTS"
    );
    let queries = h.queries();
    let reads = &queries[start..];
    assert_eq!(reads.len(), 10);
    assert!(reads[3].contains("TABLE_CONSTRAINTS AS TC"));
    assert!(reads[4].starts_with("SHOW PRIMARY"));
    assert!(reads[5].starts_with("SHOW UNIQUE"));
    assert!(reads[6].starts_with("SHOW IMPORTED"));
    assert!(!reads.iter().any(|sql| sql.starts_with("DESCRIBE ")
        || sql.starts_with("SELECT GET_DDL(")
        || sql.contains("INFORMATION_SCHEMA.COLUMNS")));
    let wire: Vec<Value> = std::fs::read_to_string(h.scripts.path().join("query-specs"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|v| {
            v["sql"].as_str().unwrap().starts_with("SHOW ")
                && v["sql"].as_str().unwrap().contains(" KEYS IN TABLE ")
        })
        .collect();
    assert_eq!(wire.len(), 3);
    for item in wire {
        assert_eq!(item["classification"], "read");
        assert_eq!(item["binds"], json!([]));
        assert_eq!(item["maxRows"], 1001);
        assert_eq!(item["maxBytes"], 1024 * 1024);
    }
    h.settings(json!({"descriptorKind":"FUTURE_KIND","rejectDefinition":true}));
    assert!(
        h.driver
            .inspect_relation(&InspectRelationRequest {
                revision: request.revision.clone(),
                target_id: request.target_id.clone()
            })
            .await
            .is_err()
    );
    assert!(
        h.driver
            .inspect_relation_constraint_mappings(&request)
            .await
            .is_ok()
    );
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("connections")).unwrap(),
        "1"
    );
    h.finish().await;
}

#[tokio::test]
async fn mappings_distinguish_missing_show_rows_from_hidden_rc_and_never_promote_orphans() {
    let h = Harness::open().await;
    let request = mapping_request(&h).await;
    h.settings(json!({"mappingPrimaryRows":[],"constraintRows":[declared("ORDERS_PK","PRIMARY KEY"),declared("ORDERS_UQ","UNIQUE"),declared("ORDERS_PARENT_FK","FOREIGN KEY")]}));
    let result = h
        .driver
        .inspect_relation_constraint_mappings(&request)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(result.mappings[0].status).unwrap(),
        "notVisible"
    );
    assert!(result.mappings[0].columns.is_empty());
    assert!(result.mappings[0].referenced_table.is_none());
    assert_eq!(
        serde_json::to_value(result.mappings[2].status).unwrap(),
        "available"
    );
    h.settings(json!({"constraintRows":[]}));
    assert!(
        h.driver
            .inspect_relation_constraint_mappings(&request)
            .await
            .is_err()
    );
    h.settings(json!({"constraintRows":[],"mappingPrimaryRows":[],"mappingUniqueRows":[],"mappingForeignRows":[]}));
    assert!(
        h.driver
            .inspect_relation_constraint_mappings(&request)
            .await
            .unwrap()
            .mappings
            .is_empty()
    );
    h.finish().await;
}

#[tokio::test]
async fn mappings_dynamic_is_explicitly_unsupported_and_unknown_traits_remain_closed_before_sql() {
    for dynamic in [json!("YES"), Value::Null] {
        let h = Harness::open().await;
        h.settings(json!({"isDynamic":dynamic}));
        let request = mapping_request(&h).await;
        let start = h.queries().len();
        let result = h
            .driver
            .inspect_relation_constraint_mappings(&request)
            .await;
        if dynamic.is_null() {
            assert_eq!(result.unwrap_err().code, ErrorCode::Unsupported);
            assert_eq!(h.queries().len(), start);
        } else {
            let result = result.unwrap();
            assert_eq!(result.mappings.len(), 3);
            assert!(
                result
                    .mappings
                    .iter()
                    .all(|m| serde_json::to_value(m.status).unwrap() == "unsupported"
                        && m.columns.is_empty()
                        && m.referenced_table.is_none())
            );
        }
        assert_eq!(mapping_count(&h), 0);
        h.finish().await;
    }
}

#[tokio::test]
async fn mappings_reject_stale_forged_and_busy_requests_before_any_read() {
    let h = Harness::open().await;
    let request = mapping_request(&h).await;
    let start = h.queries().len();
    for invalid in [
        InspectRelationConstraintMappingsRequest {
            revision: "old".into(),
            ..request.clone()
        },
        InspectRelationConstraintMappingsRequest {
            target_id: "forged".into(),
            ..request.clone()
        },
    ] {
        assert_eq!(
            h.driver
                .inspect_relation_constraint_mappings(&invalid)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(h.queries().len(), start);
    }
    let lane = h.driver.lane.lock().await;
    assert_eq!(
        h.driver
            .inspect_relation_constraint_mappings(&request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), start);
    drop(lane);
    h.finish().await;
}

#[tokio::test]
async fn mappings_database_denial_malformed_conflicting_or_incomplete_results_are_whole_errors() {
    let sentinel: Vec<_> = (1..=1001)
        .map(|i| primary(&format!("C{i}"), &i.to_string()))
        .collect();
    let huge_name = "🦀".repeat(250);
    let huge_rows: Vec<_> = (1..=1000)
        .map(|i| {
            json!([
                "DB",
                "PUBLIC",
                "ORDERS",
                format!("{}{:04}", "🦀".repeat(250), i),
                i.to_string(),
                huge_name
            ])
        })
        .collect();
    for settings in [
        json!({"constraintRows":[declared(&huge_name,"PRIMARY KEY")],"mappingPrimaryRows":huge_rows,"mappingUniqueRows":[],"mappingForeignRows":[]}),
        json!({"rejectMappings":true}),
        json!({"mappingColumnName":"WRONG"}),
        json!({"mappingType":"FIXED"}),
        json!({"mappingPrimaryRows":[primary("A","1"),primary("B","1")]}),
        json!({"mappingPrimaryRows":[primary("A","2")]}),
        json!({"mappingForeignRows":[["DB","PUBLIC","ORDERS","PARENT_A","1","ORDERS_PARENT_FK","DB","PUBLIC","PARENTS","A","DIFFERENT_PK"]]}),
        json!({"mappingPrimaryRows":sentinel}),
        json!({"mappingPrimaryRows":[["OTHER","PUBLIC","ORDERS","A","1","ORDERS_PK"]]}),
        json!({"mappingPrimaryRows":[["DB","PUBLIC","ORDERS",null,"1","ORDERS_PK"]]}),
    ] {
        let h = Harness::open().await;
        let request = mapping_request(&h).await;
        let denied = settings.get("rejectMappings").is_some();
        h.settings(settings);
        let error = h
            .driver
            .inspect_relation_constraint_mappings(&request)
            .await
            .unwrap_err();
        if denied {
            assert_eq!(error.code, ErrorCode::Database);
            assert!(!h.driver.is_closed());
        }
        assert!(mapping_count(&h) >= 1);
        h.finish().await;
    }
}

#[tokio::test]
async fn mappings_context_and_relation_drift_discard_the_whole_result() {
    for settings in [
        json!({"warehouse":"CHANGED"}),
        json!({"lastDdl":"changed-before"}),
        json!({"contextDrift":true}),
        json!({"afterMappingLastDdl":"changed-after"}),
    ] {
        let h = Harness::open().await;
        let request = mapping_request(&h).await;
        let before = settings.get("warehouse").is_some() || settings.get("lastDdl").is_some();
        h.settings(settings);
        let error = h
            .driver
            .inspect_relation_constraint_mappings(&request)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.completion_invalidated, Some(true));
        assert_eq!(mapping_count(&h), if before { 0 } else { 3 });
        let start = h.queries().len();
        assert!(
            h.driver
                .inspect_relation_constraint_mappings(&request)
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), start);
        h.finish().await;
    }
}

#[tokio::test]
async fn mappings_refresh_or_disconnect_while_show_is_held_prevents_publication() {
    for disconnect in [false, true] {
        let h = Harness::open().await;
        let request = mapping_request(&h).await;
        h.settings(json!({"holdMappings":true}));
        let driver = h.driver.clone();
        let read =
            tokio::spawn(
                async move { driver.inspect_relation_constraint_mappings(&request).await },
            );
        h.wait_until(|| h.scripts.path().join("held-mappings").exists())
            .await;
        if disconnect {
            h.driver.close();
        } else {
            h.driver.invalidate_relation_catalog();
            std::fs::write(h.scripts.path().join("release"), "").unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(10), read)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        h.finish().await;
    }
}
