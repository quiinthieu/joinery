use super::*;

async fn constraints_request(h: &Harness) -> InspectRelationConstraintsRequest {
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = catalog
        .relations
        .iter()
        .find(|target| target.table.schema == "PUBLIC")
        .unwrap();
    InspectRelationConstraintsRequest {
        revision: catalog.revision,
        target_id: target.target_id.clone(),
    }
}

fn count(h: &Harness) -> usize {
    h.queries()
        .iter()
        .filter(|sql| sql.contains(".INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS TC "))
        .count()
}

fn key_row(name: &str, kind: &str) -> Vec<Option<String>> {
    let mut row: Vec<_> = [
        "DB", "PUBLIC", "ORDERS", "DB", "PUBLIC", name, kind, "NO", "YES", "NO", "NO",
    ]
    .iter()
    .map(|value| Some((*value).to_owned()))
    .collect();
    row.extend(std::iter::repeat_n(None, 10));
    row
}

#[tokio::test]
async fn constraints_read_is_bound_bounded_and_independent_of_columns_and_definitions() {
    let h = Harness::open().await;
    let request = constraints_request(&h).await;
    let start = h.queries().len();
    let result = h
        .driver
        .inspect_relation_constraints(&request)
        .await
        .unwrap();
    assert_eq!(
        (&*result.revision, &*result.target_id),
        (&*request.revision, &*request.target_id)
    );
    assert_eq!(
        (
            &*result.database,
            &*result.table.schema,
            &*result.table.name
        ),
        ("DB", "PUBLIC", "ORDERS")
    );
    assert_eq!(result.constraints.len(), 3);
    assert_eq!(result.constraints[0].name, "ORDERS_PK");
    assert!(!result.constraints[0].is_deferrable && result.constraints[0].initially_deferred);
    assert!(!result.constraints[0].enforced && !result.constraints[0].rely);
    assert_eq!(
        serde_json::to_value(result.constraints[2].kind).unwrap(),
        "foreignKey"
    );
    assert_eq!(
        result.constraints[2].reference.as_ref().unwrap().name,
        "PARENT_PK"
    );
    assert_eq!(
        serde_json::to_value(result.constraints[2].reference_status).unwrap(),
        "available"
    );
    let queries = h.queries();
    let reads = &queries[start..];
    assert_eq!(reads.len(), 7);
    assert!(reads[3].contains(".INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS TC "));
    assert!(!reads.iter().any(|sql| sql.starts_with("DESCRIBE ")
        || sql.starts_with("SELECT GET_DDL(")
        || sql.contains("INFORMATION_SCHEMA.COLUMNS")));
    let wire: Vec<Value> = std::fs::read_to_string(h.scripts.path().join("query-specs"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|value| {
            value["sql"]
                .as_str()
                .unwrap()
                .contains(".INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS TC ")
        })
        .collect();
    assert_eq!(wire.len(), 1);
    assert_eq!(wire[0]["classification"], "read");
    assert_eq!(wire[0]["binds"], json!(["DB", "PUBLIC", "ORDERS"]));
    assert_eq!(wire[0]["maxRows"], 1001);
    assert_eq!(wire[0]["maxBytes"], 1024 * 1024);
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("constraint-binds"))
            .unwrap()
            .trim(),
        "[\"DB\",\"PUBLIC\",\"ORDERS\"]"
    );

    h.settings(json!({"rejectDefinition":true, "descriptorKind":"FUTURE_KIND"}));
    assert!(
        h.driver
            .define_relation(&DefineRelationRequest {
                revision: request.revision.clone(),
                target_id: request.target_id.clone()
            })
            .await
            .is_err()
    );
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
            .inspect_relation_constraints(&request)
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
async fn constraints_preserve_invisible_fk_details_and_an_empty_visible_inventory() {
    let h = Harness::open().await;
    let request = constraints_request(&h).await;
    h.settings(json!({"constraintRows":[key_row("HIDDEN_FK", "FOREIGN KEY")]}));
    let result = h
        .driver
        .inspect_relation_constraints(&request)
        .await
        .unwrap();
    assert_eq!(result.constraints.len(), 1);
    assert!(result.constraints[0].reference.is_none());
    assert_eq!(
        serde_json::to_value(result.constraints[0].reference_status).unwrap(),
        "notVisible"
    );
    h.settings(json!({"constraintRows":[]}));
    assert!(
        h.driver
            .inspect_relation_constraints(&request)
            .await
            .unwrap()
            .constraints
            .is_empty()
    );
    h.finish().await;
}

#[tokio::test]
async fn constraints_stale_unknown_unavailable_busy_and_closed_requests_do_not_dispatch() {
    let h = Harness::open().await;
    let request = constraints_request(&h).await;
    let start = h.queries().len();
    for invalid in [
        InspectRelationConstraintsRequest {
            revision: "old".into(),
            ..request.clone()
        },
        InspectRelationConstraintsRequest {
            target_id: "forged".into(),
            ..request.clone()
        },
    ] {
        assert_eq!(
            h.driver
                .inspect_relation_constraints(&invalid)
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
            .inspect_relation_constraints(&request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), start);
    drop(lane);
    h.settings(json!({"tableType":"FUTURE TABLE"}));
    let unavailable = constraints_request(&h).await;
    let start = h.queries().len();
    assert_eq!(
        h.driver
            .inspect_relation_constraints(&unavailable)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    assert_eq!(h.queries().len(), start);
    h.driver.close();
    assert!(
        h.driver
            .inspect_relation_constraints(&unavailable)
            .await
            .is_err()
    );
    assert_eq!(h.queries().len(), start);
    h.finish().await;
}

#[tokio::test]
async fn constraints_context_and_relation_drift_discard_results_and_revoke_targets() {
    for settings in [
        json!({"warehouse":"CHANGED"}),
        json!({"lastDdl":"changed-before"}),
        json!({"contextDrift":true}),
        json!({"afterConstraintsLastDdl":"changed-after"}),
    ] {
        let h = Harness::open().await;
        let request = constraints_request(&h).await;
        let before = settings.get("warehouse").is_some() || settings.get("lastDdl").is_some();
        h.settings(settings);
        let error = h
            .driver
            .inspect_relation_constraints(&request)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.completion_invalidated, Some(true));
        assert_eq!(count(&h), usize::from(!before));
        let start = h.queries().len();
        assert!(
            h.driver
                .inspect_relation_constraints(&request)
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), start);
        h.finish().await;
    }
}

#[tokio::test]
async fn constraints_database_denial_does_not_become_an_empty_inventory_or_replace_the_session() {
    let h = Harness::open().await;
    let request = constraints_request(&h).await;
    h.settings(json!({"rejectConstraints":true}));
    assert_eq!(
        h.driver
            .inspect_relation_constraints(&request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert_eq!(count(&h), 1);
    assert!(!h.driver.is_closed());
    h.settings(json!({}));
    assert_eq!(
        h.driver
            .inspect_relation_constraints(&request)
            .await
            .unwrap()
            .constraints
            .len(),
        3
    );
    assert_eq!(count(&h), 2);
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("connections")).unwrap(),
        "1"
    );
    h.finish().await;
}

#[tokio::test]
async fn constraints_malformed_and_over_limit_helper_results_are_errors() {
    let duplicated = key_row("DUPLICATE", "UNIQUE");
    let sentinel: Vec<_> = (0..=browsing::CONSTRAINT_LIMIT)
        .map(|i| key_row(&format!("UQ_{i}"), "UNIQUE"))
        .collect();
    let huge: Vec<_> = (0..20)
        .map(|i| {
            let mut row = key_row(&format!("UQ_{i}"), "UNIQUE");
            row[11] = Some("x".repeat(64 * 1024));
            row
        })
        .collect();
    for settings in [
        json!({"constraintColumnName":"UNEXPECTED"}),
        json!({"constraintRows":[duplicated.clone(), duplicated]}),
        json!({"constraintRows":sentinel}),
        json!({"constraintRows":huge}),
    ] {
        let h = Harness::open().await;
        let request = constraints_request(&h).await;
        h.settings(settings);
        assert!(
            h.driver
                .inspect_relation_constraints(&request)
                .await
                .is_err()
        );
        assert_eq!(count(&h), 1);
        h.finish().await;
    }
}

#[tokio::test]
async fn constraints_refresh_or_disconnect_during_a_held_read_prevents_publication() {
    for disconnect in [false, true] {
        let h = Harness::open().await;
        let request = constraints_request(&h).await;
        h.settings(json!({"holdConstraints":true}));
        let driver = h.driver.clone();
        let read = tokio::spawn(async move { driver.inspect_relation_constraints(&request).await });
        h.wait_until(|| h.scripts.path().join("held-constraints").exists())
            .await;
        if disconnect {
            h.driver.close();
        } else {
            h.driver.invalidate_relation_catalog();
            std::fs::write(h.scripts.path().join("release"), "").unwrap();
        }
        let result = tokio::time::timeout(Duration::from_secs(10), read)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        assert_eq!(count(&h), 1);
        h.finish().await;
    }
}
