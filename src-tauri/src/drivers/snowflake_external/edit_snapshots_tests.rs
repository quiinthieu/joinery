use super::*;
use crate::drivers::snowflake_edit_snapshot::SnapshotMutation;
use crate::drivers::snowflake_mutations::CellValue;

async fn open_edit_fixture(writable: bool) -> Harness {
    // Extend only this fixture. The actual production framed helper runs with
    // this injected synthetic SDK; no account, socket or external login is used.
    let fixture = FIXTURE
        .replace(
            "const zeroRows =",
            "const editPreview = sql.startsWith('SELECT \"ID\", \"NOTES\" FROM ');\n      const zeroRows =",
        )
        .replace("} else if (zeroRows) {", r#"} else if (editPreview) {
        names = settings.badPreview ? ['NOTES','ID'] : ['ID','NOTES'];
        rows = [['9007199254740993',null],['9007199254740993',''],['2','雪']];
      } else if (zeroRows) {"#)
        .replace("(zeroRows && name === 'ID')", "((zeroRows || editPreview) && name === 'ID')")
        .replace("streamRows() {", r#"streamRows() {
          if (editPreview && settings.holdPreview) {
            writeFileSync(file('held-preview'), '');
            return Readable.from((async function*() {
              while (!existsSync(file('release'))) await new Promise(resolve => setTimeout(resolve,5));
              yield* rows;
            })());
          }"#);
    Harness::open_fixture(
        if writable {
            SafeMode::Confirm
        } else {
            SafeMode::ReadOnly
        },
        writable,
        false,
        false,
        None,
        None,
        &fixture,
    )
    .await
}

async fn edit_request(h: &Harness) -> PreviewRelationRequest {
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target_id = catalog
        .relations
        .iter()
        .find(|r| r.table.schema == "PUBLIC")
        .unwrap()
        .target_id
        .clone();
    let inspection = h
        .driver
        .inspect_relation(&InspectRelationRequest {
            revision: catalog.revision.clone(),
            target_id: target_id.clone(),
        })
        .await
        .unwrap();
    PreviewRelationRequest {
        revision: catalog.revision,
        target_id,
        inspection_revision: inspection.revision,
        ..Default::default()
    }
}

fn delete(row_index: usize) -> SnapshotMutation {
    SnapshotMutation::Delete { row_index }
}

#[tokio::test]
async fn native_edit_close_during_preview_cannot_publish_or_prepare_authority() {
    let h = open_edit_fixture(true).await;
    let request = edit_request(&h).await;
    h.settings(json!({"holdPreview":true}));
    let driver = h.driver.clone();
    let in_flight = request.clone();
    let task = tokio::spawn(async move { driver.preview_relation(&in_flight).await });
    h.wait_until(|| h.scripts.path().join("held-preview").exists())
        .await;
    h.driver.close();
    assert!(task.await.unwrap().is_err());
    assert!(
        h.driver
            .prepare_relation_edits(&request, &new_id(), &[delete(0)])
            .await
            .is_err()
    );
    h.finish().await;
}

#[tokio::test]
async fn native_edit_preview_prepares_exact_keyless_originals_without_sql_or_write_enable() {
    let h = open_edit_fixture(true).await;
    let request = edit_request(&h).await;
    let preview = h.driver.preview_relation(&request).await.unwrap();
    let token = preview.edit_snapshot_id.as_deref().unwrap();
    assert_eq!(preview.rows[0], [Some("9007199254740993".into()), None]);
    assert_eq!(
        preview.rows[1],
        [Some("9007199254740993".into()), Some("".into())]
    );
    assert!(
        preview
            .columns
            .iter()
            .all(|column| !column.editable && !column.insertable && !column.primary_key)
    );
    let baseline = h.queries().len();
    let plan = h
        .driver
        .prepare_relation_edits(&request, token, &[delete(0), delete(1)])
        .await
        .unwrap();
    assert_eq!(plan.compiled().units().len(), 2);
    let units = plan.compiled().units();
    assert!(
        units[0]
            .bindings()
            .iter()
            .any(|binding| binding.value() == Some("9007199254740993"))
    );
    assert!(units[0].sql().contains("IS NOT DISTINCT FROM"));
    assert!(
        units[0]
            .bindings()
            .iter()
            .any(|binding| binding.value().is_none())
    );
    assert!(
        units[1]
            .bindings()
            .iter()
            .any(|binding| binding.value() == Some(""))
    );
    assert_eq!(h.queries().len(), baseline);
    assert!(
        h.driver
            .prepare_relation_edits(&request, token, &[delete(0)])
            .await
            .is_err()
    );
    plan.validate_current(plan.binding()).unwrap();
    h.driver.close();
    assert!(plan.validate_current(plan.binding()).is_err());
    drop(plan);
    h.finish().await;
}

#[tokio::test]
async fn native_edit_intent_is_bound_and_failed_preparation_consumes_only_local_token() {
    let h = open_edit_fixture(true).await;
    let request = edit_request(&h).await;
    let preview = h.driver.preview_relation(&request).await.unwrap();
    let token = preview.edit_snapshot_id.unwrap();
    let baseline = h.queries().len();
    let value = "雪'; DROP TABLE ORDERS; --";
    let plan = h
        .driver
        .prepare_relation_edits(
            &request,
            &token,
            &[SnapshotMutation::Update {
                row_index: 2,
                changes: vec![(1, CellValue::Text(value.into()))],
            }],
        )
        .await
        .unwrap();
    assert!(!plan.compiled().units()[0].sql().contains(value));
    assert!(
        plan.compiled().units()[0]
            .bindings()
            .iter()
            .any(|binding| binding.value() == Some(value))
    );
    assert_eq!(h.queries().len(), baseline);
    drop(plan);
    let preview = h.driver.preview_relation(&request).await.unwrap();
    let token = preview.edit_snapshot_id.unwrap();
    let baseline = h.queries().len();
    assert!(
        h.driver
            .prepare_relation_edits(&request, &token, &[delete(99)])
            .await
            .is_err()
    );
    assert!(
        h.driver
            .prepare_relation_edits(&request, &token, &[delete(0)])
            .await
            .is_err()
    );
    assert_eq!(h.queries().len(), baseline);
    h.finish().await;
}

#[tokio::test]
async fn native_edit_failed_replacement_and_inspection_refresh_revoke_prepared_plans() {
    let h = open_edit_fixture(true).await;
    let request = edit_request(&h).await;
    let preview = h.driver.preview_relation(&request).await.unwrap();
    let plan = h
        .driver
        .prepare_relation_edits(
            &request,
            preview.edit_snapshot_id.as_deref().unwrap(),
            &[delete(0)],
        )
        .await
        .unwrap();
    h.settings(json!({"badPreview":true}));
    assert!(h.driver.preview_relation(&request).await.is_err());
    assert!(plan.validate_current(plan.binding()).is_err());
    drop(plan);
    h.settings(json!({}));
    let preview = h.driver.preview_relation(&request).await.unwrap();
    let plan = h
        .driver
        .prepare_relation_edits(
            &request,
            preview.edit_snapshot_id.as_deref().unwrap(),
            &[delete(0)],
        )
        .await
        .unwrap();
    h.driver
        .inspect_relation(&InspectRelationRequest {
            revision: request.revision.clone(),
            target_id: request.target_id.clone(),
        })
        .await
        .unwrap();
    assert!(plan.validate_current(plan.binding()).is_err());
    drop(plan);
    h.finish().await;
}

#[tokio::test]
async fn native_edit_abandoned_replacement_cannot_resurrect_older_inflight_preview() {
    let h = open_edit_fixture(true).await;
    let request = edit_request(&h).await;
    h.settings(json!({"holdPreview":true}));
    let driver = h.driver.clone();
    let first_request = request.clone();
    let first = tokio::spawn(async move { driver.preview_relation(&first_request).await });
    h.wait_until(|| h.scripts.path().join("held-preview").exists())
        .await;
    let original = h
        .driver
        .browsing
        .lock()
        .snapshot
        .as_ref()
        .unwrap()
        .inspections[&request.target_id]
        .preview_revision
        .clone();
    let driver = h.driver.clone();
    let second_request = request.clone();
    let second = tokio::spawn(async move { driver.preview_relation(&second_request).await });
    h.wait_until(|| {
        h.driver
            .browsing
            .lock()
            .snapshot
            .as_ref()
            .unwrap()
            .inspections[&request.target_id]
            .preview_revision
            != original
    })
    .await;
    second.abort();
    assert!(second.await.unwrap_err().is_cancelled());
    std::fs::write(h.scripts.path().join("release"), "").unwrap();
    assert_eq!(first.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert!(!h.driver.is_closed());
    h.settings(json!({}));
    let fresh = h.driver.preview_relation(&request).await.unwrap();
    assert!(fresh.edit_snapshot_id.is_some());
    h.driver.invalidate_relation_catalog();
    let baseline = h.queries().len();
    assert!(
        h.driver
            .prepare_relation_edits(
                &request,
                fresh.edit_snapshot_id.as_deref().unwrap(),
                &[delete(0)]
            )
            .await
            .is_err()
    );
    assert_eq!(h.queries().len(), baseline);
    h.finish().await;
}

#[tokio::test]
async fn native_edit_readonly_and_unsupported_preview_stay_readonly_and_revoke_prior_authority() {
    let h = open_edit_fixture(false).await;
    let request = edit_request(&h).await;
    let preview = h.driver.preview_relation(&request).await.unwrap();
    assert!(preview.edit_snapshot_id.is_none());
    assert_eq!(preview.rows.len(), 3);
    assert_eq!(
        h.driver
            .prepare_relation_edits(&request, &new_id(), &[delete(0)])
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::PolicyBlocked
    );
    h.finish().await;

    let h = open_edit_fixture(true).await;
    let request = edit_request(&h).await;
    let preview = h.driver.preview_relation(&request).await.unwrap();
    let plan = h
        .driver
        .prepare_relation_edits(
            &request,
            preview.edit_snapshot_id.as_deref().unwrap(),
            &[delete(0)],
        )
        .await
        .unwrap();
    let revision = h.driver.begin_edit_preview(&request).unwrap();
    // Model an independently reported unsupported write-default observation in
    // native metadata. It must never be guessed into a generated assignment.
    {
        let mut state = h.driver.browsing.lock();
        let column = &mut state
            .snapshot
            .as_mut()
            .unwrap()
            .inspections
            .get_mut(&request.target_id)
            .unwrap()
            .columns
            .as_mut()
            .unwrap()[0];
        column.write_default_status =
            crate::relation_inspection::WriteDefaultObservationStatus::Available;
        column.write_default_expression = Some("42".into());
    }
    let fallback = h
        .driver
        .retain_edit_preview(&request, &revision, preview)
        .unwrap();
    assert!(fallback.edit_snapshot_id.is_none());
    assert_eq!(fallback.rows.len(), 3);
    assert!(plan.validate_current(plan.binding()).is_err());
    drop(plan);
    h.finish().await;
}
