use super::*;

async fn check_request(h: &Harness) -> InspectRelationCheckConstraintsRequest {
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = catalog
        .relations
        .iter()
        .find(|target| target.table.schema == "PUBLIC")
        .unwrap();
    InspectRelationCheckConstraintsRequest {
        revision: catalog.revision,
        target_id: target.target_id.clone(),
    }
}

fn count(h: &Harness) -> usize {
    h.queries()
        .iter()
        .filter(|sql| sql.contains(".INFORMATION_SCHEMA.CHECK_CONSTRAINTS "))
        .count()
}

fn check_row(name: &str, clause: &str) -> Vec<Option<String>> {
    ["DB", "PUBLIC", "ORDERS", name, clause]
        .into_iter()
        .map(|value| Some(value.into()))
        .collect()
}

#[tokio::test]
async fn check_constraints_read_is_bound_bounded_and_independent_of_other_inspections() {
    let h = Harness::open().await;
    let request = check_request(&h).await;
    let start = h.queries().len();
    let result = h
        .driver
        .inspect_relation_check_constraints(&request)
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
    assert_eq!(result.constraints.len(), 2);
    assert_eq!(result.constraints[0].name, "POSITIVE_ID");
    assert_eq!(result.constraints[0].table_name, "ORDERS");
    assert_eq!(
        result.constraints[1].check_clause,
        "\"NOTES\" IS NULL OR\nLENGTH(\"NOTES\") <= 100 /* 雪 */"
    );
    let queries = h.queries();
    let reads = &queries[start..];
    assert_eq!(reads.len(), 7);
    assert!(reads[3].contains(".INFORMATION_SCHEMA.CHECK_CONSTRAINTS "));
    assert!(!reads.iter().any(|sql| sql.starts_with("DESCRIBE ")
        || sql.starts_with("SELECT GET_DDL(")
        || sql.starts_with("SHOW PRIMARY ")
        || sql.contains("INFORMATION_SCHEMA.COLUMNS")
        || sql.contains("INFORMATION_SCHEMA.TABLE_CONSTRAINTS")));
    let wire: Vec<Value> = std::fs::read_to_string(h.scripts.path().join("query-specs"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|value| {
            value["sql"]
                .as_str()
                .unwrap()
                .contains(".INFORMATION_SCHEMA.CHECK_CONSTRAINTS ")
        })
        .collect();
    assert_eq!(wire.len(), 1);
    assert_eq!(wire[0]["classification"], "read");
    assert_eq!(wire[0]["binds"], json!(["DB", "PUBLIC", "ORDERS"]));
    assert_eq!(wire[0]["maxRows"], 1001);
    assert_eq!(wire[0]["maxBytes"], 1024 * 1024);
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("check-binds"))
            .unwrap()
            .trim(),
        "[\"DB\",\"PUBLIC\",\"ORDERS\"]"
    );
    h.settings(
        json!({"rejectDefinition":true,"rejectConstraints":true,"descriptorKind":"FUTURE_KIND"}),
    );
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
            .inspect_relation_constraints(&InspectRelationConstraintsRequest {
                revision: request.revision.clone(),
                target_id: request.target_id.clone()
            })
            .await
            .is_err()
    );
    assert!(
        h.driver
            .inspect_relation_check_constraints(&request)
            .await
            .is_ok()
    );
    h.finish().await;
}

#[tokio::test]
async fn check_constraints_empty_visibility_and_denial_remain_distinct_on_retained_session() {
    let h = Harness::open().await;
    let request = check_request(&h).await;
    h.settings(json!({"checkRows":[]}));
    assert!(
        h.driver
            .inspect_relation_check_constraints(&request)
            .await
            .unwrap()
            .constraints
            .is_empty()
    );
    h.settings(json!({"rejectChecks":true}));
    assert_eq!(
        h.driver
            .inspect_relation_check_constraints(&request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert_eq!(count(&h), 2);
    assert!(!h.driver.is_closed());
    h.settings(json!({}));
    assert_eq!(
        h.driver
            .inspect_relation_check_constraints(&request)
            .await
            .unwrap()
            .constraints
            .len(),
        2
    );
    assert_eq!(count(&h), 3);
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("connections")).unwrap(),
        "1"
    );
    h.finish().await;
}

#[tokio::test]
async fn check_constraints_stale_forged_busy_unavailable_and_closed_targets_issue_no_sql() {
    let h = Harness::open().await;
    let request = check_request(&h).await;
    let start = h.queries().len();
    for invalid in [
        InspectRelationCheckConstraintsRequest {
            revision: "old".into(),
            ..request.clone()
        },
        InspectRelationCheckConstraintsRequest {
            target_id: "forged".into(),
            ..request.clone()
        },
    ] {
        assert_eq!(
            h.driver
                .inspect_relation_check_constraints(&invalid)
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
            .inspect_relation_check_constraints(&request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), start);
    drop(lane);
    // Unknown traits retain the existing whole-action gate before SQL.
    h.settings(json!({"isDynamic":null}));
    let unavailable = check_request(&h).await;
    let start = h.queries().len();
    assert_eq!(
        h.driver
            .inspect_relation_check_constraints(&unavailable)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    assert_eq!(h.queries().len(), start);
    h.driver.close();
    assert!(
        h.driver
            .inspect_relation_check_constraints(&unavailable)
            .await
            .is_err()
    );
    assert_eq!(h.queries().len(), start);
    h.finish().await;
}

#[tokio::test]
async fn check_constraints_context_and_relation_drift_discard_results_and_revoke_targets() {
    for settings in [
        json!({"warehouse":"CHANGED"}),
        json!({"lastDdl":"changed-before"}),
        json!({"contextDrift":true}),
        json!({"afterCheckLastDdl":"changed-after"}),
    ] {
        let h = Harness::open().await;
        let request = check_request(&h).await;
        let before = settings.get("warehouse").is_some() || settings.get("lastDdl").is_some();
        h.settings(settings);
        let error = h
            .driver
            .inspect_relation_check_constraints(&request)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.completion_invalidated, Some(true));
        assert_eq!(count(&h), usize::from(!before));
        let start = h.queries().len();
        assert!(
            h.driver
                .inspect_relation_check_constraints(&request)
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), start);
        h.finish().await;
    }
}

#[tokio::test]
async fn check_constraints_malformed_and_over_limit_helper_results_fail_whole_response() {
    let valid = check_row("CK", "ID > 0");
    let mut null = valid.clone();
    null[4] = None;
    let mut wrong = valid.clone();
    wrong[2] = Some("OTHER".into());
    let sentinel: Vec<_> = (0..=browsing::CHECK_LIMIT)
        .map(|i| check_row(&format!("CK_{i}"), "ID > 0"))
        .collect();
    let huge: Vec<_> = (0..20)
        .map(|i| check_row(&format!("CK_{i}"), &"x".repeat(64 * 1024)))
        .collect();
    for settings in [
        json!({"checkColumnName":"TABLE_NAME"}),
        json!({"checkType":"FIXED"}),
        json!({"checkRows":[valid.clone(), valid]}),
        json!({"checkRows":[null]}),
        json!({"checkRows":[wrong]}),
        json!({"checkRows":sentinel}),
        json!({"checkRows":huge}),
    ] {
        let h = Harness::open().await;
        let request = check_request(&h).await;
        h.settings(settings);
        assert!(
            h.driver
                .inspect_relation_check_constraints(&request)
                .await
                .is_err()
        );
        assert_eq!(count(&h), 1);
        h.finish().await;
    }
}

#[tokio::test]
async fn check_constraints_refresh_or_disconnect_during_held_read_prevents_publication() {
    for disconnect in [false, true] {
        let h = Harness::open().await;
        let request = check_request(&h).await;
        h.settings(json!({"holdChecks":true}));
        let driver = h.driver.clone();
        let read =
            tokio::spawn(async move { driver.inspect_relation_check_constraints(&request).await });
        h.wait_until(|| h.scripts.path().join("held-checks").exists())
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
        assert_eq!(count(&h), 1);
        h.finish().await;
    }
}

#[tokio::test]
async fn check_constraints_quoted_cross_schema_target_uses_opaque_identity_without_context_switch()
{
    let h = Harness::open().await;
    h.settings(json!({"referenceNavigation":true}));
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = catalog
        .relations
        .iter()
        .find(|target| {
            target.table.schema == "Ref.\"Schema'雪" && target.table.name == "Shared.\"Table'雪"
        })
        .unwrap();
    let request = InspectRelationCheckConstraintsRequest {
        revision: catalog.revision,
        target_id: target.target_id.clone(),
    };
    let result = h
        .driver
        .inspect_relation_check_constraints(&request)
        .await
        .unwrap();
    assert_eq!(result.table, target.table);
    assert!(result.constraints.iter().all(|check| check.catalog == "DB"
        && check.schema == target.table.schema
        && check.table_name == target.table.name));
    let binds: Value = serde_json::from_str(
        std::fs::read_to_string(h.scripts.path().join("check-binds"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert_eq!(binds, json!(["DB", target.table.schema, target.table.name]));
    assert!(!h.queries().iter().any(|sql| sql.starts_with("USE ")));
    assert_eq!(
        h.driver
            .browse_context(None)
            .await
            .unwrap()
            .schema
            .as_deref(),
        Some("PUBLIC")
    );
    h.finish().await;
}
