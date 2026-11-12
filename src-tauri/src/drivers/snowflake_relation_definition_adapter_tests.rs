use super::*;

async fn definition_request(h: &Harness) -> DefineRelationRequest {
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = catalog
        .relations
        .iter()
        .find(|target| target.table.schema == "PUBLIC")
        .unwrap();
    DefineRelationRequest {
        revision: catalog.revision,
        target_id: target.target_id.clone(),
    }
}

fn definition_count(h: &Harness) -> usize {
    h.queries()
        .iter()
        .filter(|sql| sql.starts_with("SELECT GET_DDL("))
        .count()
}

#[tokio::test]
async fn definition_reads_before_inspection_and_survives_unsupported_descriptors_on_one_session() {
    let h = Harness::open().await;
    let request = definition_request(&h).await;
    let start = h.queries().len();
    let result = h.driver.define_relation(&request).await.unwrap();
    assert_eq!(result.revision, request.revision);
    assert_eq!(result.target_id, request.target_id);
    assert_eq!(result.database, "DB");
    assert_eq!(result.table.schema, "PUBLIC");
    assert_eq!(result.table.name, "ORDERS");
    assert_eq!(
        result.ddl,
        "create or replace table \"DB\".\"PUBLIC\".\"ORDERS\" (ID NUMBER);"
    );
    let queries = h.queries();
    let reads = &queries[start..];
    assert_eq!(reads.len(), 7); // context pair, exact, DDL, exact, context pair
    assert_eq!(
        reads[3],
        "SELECT GET_DDL('TABLE', '\"DB\".\"PUBLIC\".\"ORDERS\"', TRUE) AS DDL"
    );
    assert!(
        !reads
            .iter()
            .any(|sql| sql.contains("INFORMATION_SCHEMA.COLUMNS") || sql.starts_with("DESCRIBE "))
    );

    h.settings(json!({"descriptorKind":"FUTURE_KIND"}));
    assert!(
        h.driver
            .inspect_relation(&InspectRelationRequest {
                revision: request.revision.clone(),
                target_id: request.target_id.clone(),
            })
            .await
            .is_err()
    );
    assert!(h.driver.define_relation(&request).await.is_ok());
    assert_eq!(definition_count(&h), 2);
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("connections")).unwrap(),
        "1"
    );
    assert!(!h.driver.is_closed());
    h.finish().await;
}

#[tokio::test]
async fn definition_rejects_stale_unknown_and_unavailable_targets_without_sql() {
    let h = Harness::open().await;
    let request = definition_request(&h).await;
    let count = h.queries().len();
    for invalid in [
        DefineRelationRequest {
            revision: "stale".into(),
            ..request.clone()
        },
        DefineRelationRequest {
            target_id: "forged".into(),
            ..request.clone()
        },
    ] {
        assert_eq!(
            h.driver.define_relation(&invalid).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(h.queries().len(), count);
    }
    h.driver.invalidate_relation_catalog();
    assert!(h.driver.define_relation(&request).await.is_err());
    assert_eq!(h.queries().len(), count);

    h.settings(json!({"tableType":"FUTURE TABLE"}));
    let unavailable = definition_request(&h).await;
    let count = h.queries().len();
    assert_eq!(
        h.driver
            .define_relation(&unavailable)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    assert_eq!(h.queries().len(), count);
    h.finish().await;
}

#[tokio::test]
async fn definition_context_and_relation_drift_reject_publication_and_revoke_the_target() {
    for settings in [
        json!({"warehouse":"CHANGED"}),
        json!({"contextDrift":true}),
        json!({"afterDefinitionLastDdl":"changed-ddl"}),
    ] {
        let h = Harness::open().await;
        let request = definition_request(&h).await;
        let before_read = settings.get("warehouse").is_some();
        h.settings(settings);
        let error = h.driver.define_relation(&request).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.completion_invalidated, Some(true));
        assert_eq!(definition_count(&h), usize::from(!before_read));
        let count = h.queries().len();
        assert!(h.driver.define_relation(&request).await.is_err());
        assert_eq!(h.queries().len(), count);
        h.finish().await;
    }
}

#[tokio::test]
async fn definition_database_rejection_is_an_error_and_keeps_the_original_session() {
    let h = Harness::open().await;
    let request = definition_request(&h).await;
    h.settings(json!({"rejectDefinition":true}));
    assert_eq!(
        h.driver.define_relation(&request).await.unwrap_err().code,
        ErrorCode::Database
    );
    assert_eq!(definition_count(&h), 1);
    assert!(!h.driver.is_closed());
    h.settings(json!({"definition":"create or replace table retained (id number);"}));
    assert_eq!(
        h.driver.define_relation(&request).await.unwrap().ddl,
        "create or replace table retained (id number);"
    );
    assert_eq!(definition_count(&h), 2); // explicit new read, no fallback/replay
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("connections")).unwrap(),
        "1"
    );
    h.finish().await;
}

#[tokio::test]
async fn definition_helper_rejects_null_wrong_shape_and_incomplete_retention() {
    for settings in [
        json!({"definitionRows":[[null]]}),
        json!({"definitionColumns":["UNEXPECTED"]}),
        json!({"definitionType":"FIXED", "definitionRows":[["1"]]}),
        json!({"definitionRows":[["first"],["second"]]}),
        json!({"definition":"x".repeat(browsing::DEFINITION_MAX_BYTES + 1)}),
    ] {
        let h = Harness::open().await;
        let request = definition_request(&h).await;
        h.settings(settings);
        assert!(h.driver.define_relation(&request).await.is_err());
        assert_eq!(definition_count(&h), 1);
        h.finish().await;
    }
}

#[tokio::test]
async fn definition_refresh_during_an_active_read_prevents_stale_publication() {
    let h = Harness::open().await;
    let request = definition_request(&h).await;
    h.settings(json!({"holdDefinition":true}));
    let driver = h.driver.clone();
    let read = tokio::spawn(async move { driver.define_relation(&request).await });
    h.wait_until(|| h.scripts.path().join("held-definition").exists())
        .await;
    h.driver.invalidate_relation_catalog();
    std::fs::write(h.scripts.path().join("release"), "").unwrap();
    assert_eq!(read.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(definition_count(&h), 1);
    h.finish().await;
}
