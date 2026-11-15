use super::*;

const DESTINATION_SCHEMA: &str = "Ref.\"Schema'雪";
const DESTINATION_TABLE: &str = "Shared.\"Table'雪";
const DESTINATION_QUALIFIED: &str = "\"DB\".\"Ref.\"\"Schema'雪\".\"Shared.\"\"Table'雪\"";

fn exact_target<'a>(
    catalog: &'a BrowseCatalog,
    schema: &str,
    table: &str,
) -> &'a RelationTargetSummary {
    let matches = catalog
        .relations
        .iter()
        .filter(|relation| {
            relation.database == "DB"
                && relation.table.schema == schema
                && relation.table.name == table
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].availability, RelationAvailability::Available);
    matches[0]
}

fn assert_no_context_switch(h: &Harness) {
    assert!(
        h.queries()
            .iter()
            .all(|sql| !sql.starts_with("USE ") && !sql.starts_with("ALTER SESSION ")),
        "reference inspection must not borrow session-switch authority"
    );
}

#[tokio::test]
async fn reference_navigation_inspects_the_exact_quoted_cross_schema_target_on_one_connection() {
    let h = Harness::open().await;
    h.settings(json!({"referenceNavigation":true}));
    let catalog = h.driver.relation_catalog().await.unwrap();
    assert_eq!(catalog.database, "DB");
    assert_eq!(catalog.relations.len(), 4);
    let source = exact_target(&catalog, "PUBLIC", "ORDERS");
    // Both simple and quoted table names repeat across schemas. A name-only
    // resolver could choose a valid but different retained target.
    assert_ne!(
        source.target_id,
        exact_target(&catalog, DESTINATION_SCHEMA, "ORDERS").target_id
    );
    let same_name_in_public = exact_target(&catalog, "PUBLIC", DESTINATION_TABLE);
    let mappings = h
        .driver
        .inspect_relation_constraint_mappings(&InspectRelationConstraintMappingsRequest {
            revision: catalog.revision.clone(),
            target_id: source.target_id.clone(),
        })
        .await
        .unwrap();
    let foreign = mappings
        .mappings
        .iter()
        .find(|mapping| mapping.kind == RelationKeyConstraintKind::ForeignKey)
        .unwrap();
    assert_eq!(foreign.status, ConstraintMappingStatus::Available);
    let reference = foreign.referenced_table.as_ref().unwrap();
    assert_eq!(
        (
            &*reference.database,
            &*reference.table.schema,
            &*reference.table.name
        ),
        ("DB", DESTINATION_SCHEMA, DESTINATION_TABLE)
    );
    let target = exact_target(&catalog, &reference.table.schema, &reference.table.name);
    assert_ne!(target.target_id, source.target_id);
    assert_ne!(target.target_id, same_name_in_public.target_id);
    let start = h.queries().len();
    let inspected = h
        .driver
        .inspect_relation(&InspectRelationRequest {
            revision: catalog.revision.clone(),
            target_id: target.target_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(inspected.target_id, target.target_id);
    assert_eq!(
        (
            &*inspected.database,
            &*inspected.table.schema,
            &*inspected.table.name
        ),
        ("DB", DESTINATION_SCHEMA, DESTINATION_TABLE)
    );
    assert_eq!(
        inspected
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["ID", "NOTES"]
    );
    let queries = h.queries();
    let reads = &queries[start..];
    assert_eq!(reads.len(), 9); // context pair, exact, columns/descriptor/resolution, exact, context pair
    assert!(reads[2].contains("FROM \"DB\".INFORMATION_SCHEMA.TABLES"));
    assert!(reads[2].contains("TABLE_CATALOG = 'DB' AND TABLE_SCHEMA = 'Ref.\"Schema''雪' AND TABLE_NAME = 'Shared.\"Table''雪'"));
    assert!(reads[3].contains("FROM \"DB\".INFORMATION_SCHEMA.COLUMNS AS C"));
    assert!(reads[3].contains("TABLE_CATALOG = 'DB' AND TABLE_SCHEMA = 'Ref.\"Schema''雪' AND TABLE_NAME = 'Shared.\"Table''雪'"));
    assert_eq!(
        reads[4],
        format!("DESCRIBE TABLE {DESTINATION_QUALIFIED} TYPE = COLUMNS")
    );
    assert_eq!(
        reads[5],
        format!("SELECT * FROM {DESTINATION_QUALIFIED} LIMIT 0")
    );
    assert_eq!(reads[6], reads[2]);
    assert_eq!(reads[0], metadata::CONTEXT_SQL);
    assert_eq!(reads[7], metadata::CONTEXT_SQL);
    let wire = std::fs::read_to_string(h.scripts.path().join("query-specs"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    for (statement, sql) in wire[wire.len() - reads.len()..].iter().zip(reads) {
        assert_eq!(statement["sql"], sql.as_str());
        assert_eq!(statement["binds"], json!([]));
        assert_eq!(statement["classification"], "read");
    }
    let context = h.driver.live_metadata_context().await.unwrap();
    assert_eq!(context.database.as_deref(), Some("DB"));
    assert_eq!(context.schema.as_deref(), Some("PUBLIC"));
    assert_eq!(context.role.as_deref(), Some("ROLE"));
    assert_eq!(context.warehouse.as_deref(), Some("WH"));
    assert_no_context_switch(&h);
    assert_eq!(
        std::fs::read_to_string(h.scripts.path().join("connections")).unwrap(),
        "1"
    );
    h.finish().await;
}

#[tokio::test]
async fn reference_navigation_rejects_forged_and_stale_destination_ids_before_sql() {
    let h = Harness::open().await;
    h.settings(json!({"referenceNavigation":true}));
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = exact_target(&catalog, DESTINATION_SCHEMA, DESTINATION_TABLE);
    let request = InspectRelationRequest {
        revision: catalog.revision.clone(),
        target_id: target.target_id.clone(),
    };
    let start = h.queries().len();
    for invalid in [
        InspectRelationRequest {
            target_id: "forged-destination".into(),
            ..request.clone()
        },
        InspectRelationRequest {
            target_id: DESTINATION_QUALIFIED.into(),
            ..request.clone()
        },
        InspectRelationRequest {
            revision: "stale-catalog".into(),
            ..request.clone()
        },
    ] {
        assert_eq!(
            h.driver.inspect_relation(&invalid).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(h.queries().len(), start);
    }
    let refreshed = h.driver.relation_catalog().await.unwrap();
    assert_ne!(refreshed.revision, catalog.revision);
    assert_ne!(
        exact_target(&refreshed, DESTINATION_SCHEMA, DESTINATION_TABLE).target_id,
        request.target_id
    );
    let start = h.queries().len();
    assert_eq!(
        h.driver.inspect_relation(&request).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), start);
    assert_no_context_switch(&h);
    h.finish().await;
}

#[tokio::test]
async fn reference_navigation_post_inspection_context_drift_revokes_the_destination() {
    let h = Harness::open().await;
    h.settings(json!({"referenceNavigation":true}));
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = exact_target(&catalog, DESTINATION_SCHEMA, DESTINATION_TABLE);
    let request = InspectRelationRequest {
        revision: catalog.revision.clone(),
        target_id: target.target_id.clone(),
    };
    h.settings(json!({"referenceNavigation":true,"contextDrift":true}));
    let start = h.queries().len();
    let error = h.driver.inspect_relation(&request).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.completion_invalidated, Some(true));
    assert!(
        h.queries()[start..].contains(&format!("SELECT * FROM {DESTINATION_QUALIFIED} LIMIT 0")),
        "drift must be checked after the fully qualified inspection read"
    );
    let start = h.queries().len();
    assert_eq!(
        h.driver.inspect_relation(&request).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(h.queries().len(), start);
    assert_no_context_switch(&h);
    h.finish().await;
}
