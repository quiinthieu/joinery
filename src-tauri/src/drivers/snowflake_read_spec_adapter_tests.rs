use super::*;

fn wire_reads(h: &Harness) -> Vec<Value> {
    std::fs::read_to_string(h.scripts.path().join("query-specs"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_wire_reads(actual: &[Value], expected: &[metadata::ReadSpec]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual["sql"], expected.sql());
        assert_eq!(actual["binds"], json!([]));
        assert!(expected.binds().is_empty());
        assert_eq!(actual["classification"], "read");
        assert_eq!(actual["maxRows"], expected.max_rows());
        assert_eq!(actual["maxBytes"], expected.max_bytes());
    }
}

#[tokio::test]
async fn named_read_specs_match_version_context_sample_and_catalog_wire_requests() {
    let h = Harness::open().await;
    assert_wire_reads(&wire_reads(&h), &[metadata::version_read()]);
    let start = wire_reads(&h).len();
    let context = h.driver.live_metadata_context().await.unwrap();
    assert_wire_reads(&wire_reads(&h)[start..], &metadata::context_reads());
    let start = wire_reads(&h).len();
    h.driver.session_sample().await.unwrap();
    assert_wire_reads(&wire_reads(&h)[start..], &session::sample_reads());
    let start = wire_reads(&h).len();
    h.driver.relation_catalog().await.unwrap();
    let mut expected = metadata::context_reads().to_vec();
    let schemas = metadata::schemas_read(&context).unwrap();
    assert_eq!(schemas.kind(), metadata::ReadKind::Schemas);
    assert_eq!(
        (schemas.max_rows(), schemas.max_bytes()),
        (1001, 2 * 1024 * 1024)
    );
    expected.push(schemas);
    let relations = metadata::relations_read(&context, "OTHER").unwrap();
    assert_eq!(relations.kind(), metadata::ReadKind::Relations);
    assert_eq!(
        (relations.max_rows(), relations.max_bytes()),
        (10001, 2 * 1024 * 1024)
    );
    expected.push(relations);
    expected.push(metadata::relations_read(&context, "PUBLIC").unwrap());
    expected.extend(metadata::context_reads());
    assert_wire_reads(&wire_reads(&h)[start..], &expected);
    h.finish().await;
}

#[tokio::test]
async fn named_read_specs_match_exact_relation_and_inspection_wire_requests() {
    let h = Harness::open().await;
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = catalog
        .relations
        .iter()
        .find(|target| target.table.schema == "PUBLIC")
        .unwrap();
    let (context, relation) = h
        .driver
        .browse_target(&catalog.revision, &target.target_id)
        .unwrap();
    let start = wire_reads(&h).len();
    h.driver
        .inspect_relation(&InspectRelationRequest {
            revision: catalog.revision,
            target_id: target.target_id.clone(),
        })
        .await
        .unwrap();
    let mut expected = metadata::context_reads().to_vec();
    let exact = metadata::exact_relation_read(&context, &relation).unwrap();
    assert_eq!(exact.kind(), metadata::ReadKind::ExactRelation);
    assert_eq!((exact.max_rows(), exact.max_bytes()), (3, 2 * 1024 * 1024));
    expected.push(exact);
    let inspection = metadata::inspection_reads(&relation).unwrap();
    assert_eq!(
        inspection.each_ref().map(|read| read.kind()),
        [
            metadata::ReadKind::Columns,
            metadata::ReadKind::Describe,
            metadata::ReadKind::ZeroRowResolution
        ]
    );
    assert_eq!(
        inspection.each_ref().map(|read| read.max_rows()),
        [1001, 1001, 0]
    );
    assert!(
        inspection
            .iter()
            .all(|read| read.max_bytes() == 1024 * 1024)
    );
    expected.extend(inspection);
    expected.push(metadata::exact_relation_read(&context, &relation).unwrap());
    expected.extend(metadata::context_reads());
    assert_wire_reads(&wire_reads(&h)[start..], &expected);
    h.finish().await;
}

#[test]
fn named_read_specs_keep_fixed_order_and_distinct_retention_and_result_bounds() {
    use metadata::ReadKind::*;
    let context = metadata::context_reads();
    assert_eq!(
        context.each_ref().map(|read| read.kind()),
        [Context, QuotedIdentifierPolicy]
    );
    assert!(
        context
            .iter()
            .all(|read| read.max_rows() == 1 && read.max_bytes() == 64 * 1024)
    );
    let sample = session::sample_reads();
    assert_eq!(
        sample.each_ref().map(|read| read.kind()),
        [
            Context,
            QuotedIdentifierPolicy,
            Autocommit,
            AbortOnError,
            CurrentTransaction
        ]
    );
    assert_eq!(sample[0].sql(), metadata::CONTEXT_SQL);
    assert_eq!(sample[1].sql(), metadata::QUOTED_IDENTIFIER_POLICY_SQL);
    assert_eq!(sample[2].sql(), session::AUTOCOMMIT_SQL);
    assert_eq!(sample[3].sql(), session::ABORT_ON_ERROR_SQL);
    assert_eq!(sample[4].sql(), session::CURRENT_TRANSACTION_SQL);
    for read in &sample {
        assert_eq!(read.max_rows(), 1);
        assert_eq!(read.max_bytes(), 64 * 1024);
        assert!(read.binds().is_empty());
    }
    let version = metadata::version_read();
    assert_eq!(version.kind(), Version);
    assert_eq!(version.sql(), "SELECT CURRENT_VERSION()");
    assert_eq!(version.max_rows(), 1_000_000);
    assert_eq!(version.max_bytes(), 8 * 1024 * 1024 * 1024);
}
