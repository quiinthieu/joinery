use super::super::grid_freshness::GridReadOwner;
use super::*;
use crate::drivers::snowflake_edit_snapshot::NativeEditBinding;

// Actual owned helper/framing/guard with an injected synthetic SDK. No HTTP,
// account authentication, or remote transaction semantics are established here.
async fn grid_fixture() -> Harness {
    let fixture = grid_fixture_source();
    Harness::open_fixture(SafeMode::Confirm, true, false, false, None, None, &fixture).await
}

pub(super) fn grid_fixture_source() -> String {
    let guard = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../snowflake-helper/src/query-submit-guard.mjs");
    FIXTURE
        .replace("const file =", &format!("const {{ installQuerySubmitGuard }} = await import({});\nconst file =", serde_json::to_string(&guard).unwrap()))
        .replace("currentTransaction = null;", "currentTransaction = null, relationReads = 0;")
        .replace("contextReads = 0; transactionReads = 0;", "contextReads = 0; transactionReads = 0; relationReads = 0;")
        .replace("? 'opaque-TX-雪' : null", "? '123456789' : null")
        .replace("} else if (sql === 'SELECT CURRENT_VERSION()')", "if (sql.includes('CURRENT_TRANSACTION()')) { names.push('TX'); rows[0].push(currentTransaction); }\n      } else if (sql === 'SELECT CURRENT_VERSION()')")
        .replace("} else if (relations) {", "} else if (relations) {\n        relationReads++;")
        .replace("      let cancelled = false;", r#"
      if (context && settings.ownerContextDrift && contextReads >= 2) rows[0][4] = 'CHANGED';
      if (context && settings.malformedContext) rows[0][6] = 'not-json';
      if (relations && settings.finalRelationDrift && relationReads >= 2) rows[0][11] = 'changed-ddl';
      if (columnMetadata && settings.changedColumn) rows[1][9] = '99';
      if (columnMetadata && settings.excessColumns) rows = Array.from({length:1002}, () => rows[0]);
      if (describe && settings.missingDescribe) rows.pop();
      if (zeroRows && settings.missingResolution) names.pop();
      let cancelled = false;"#)
        .replace("streamRows() {", r#"streamRows() {
          if (columnMetadata && settings.holdGrid) {
            writeFileSync(file('held-grid'), '');
            return Readable.from((async function*() {
              while (!existsSync(file('release'))) await new Promise(resolve => setTimeout(resolve,5));
              yield* rows;
            })());
          }
          if (columnMetadata && settings.protocolGrid) {
            send.flush().then(() => process.stdout.write(Buffer.alloc(4)));
            return Readable.from((async function*() { await new Promise(() => {}); })());
          }"#)
        .replace("const helper = new Helper", "const guard = installQuerySubmitGuard({onViolation:retire});\nconst helper = new Helper")
        .replace("{ clientConfigFile:", "{ submitGuard:guard, isFileTransfer:()=>false, clientConfigFile:")
        .replace("  helper.receive(frame);", "  appendFileSync(file('grid-frames'), JSON.stringify({op:frame.op,payload:frame.payload}) + '\\n');\n  helper.receive(frame);")
}

async fn binding(h: &Harness) -> NativeEditBinding {
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = catalog
        .relations
        .iter()
        .find(|r| r.table.schema == "PUBLIC")
        .unwrap();
    h.driver
        .inspect_relation(&InspectRelationRequest {
            revision: catalog.revision,
            target_id: target.target_id.clone(),
        })
        .await
        .unwrap();
    let state = h.driver.browsing.lock();
    let snapshot = state.snapshot.as_ref().unwrap();
    let inspection = &snapshot.inspections[&target.target_id];
    NativeEditBinding::new(
        "observer-session",
        &h.driver.process.helper_generation(),
        &snapshot.context,
        &state.revision,
        &target.target_id,
        &snapshot.targets[&target.target_id],
        &inspection.revision,
        inspection.columns.as_deref().unwrap(),
    )
    .unwrap()
}

fn frames(h: &Harness) -> Vec<Value> {
    std::fs::read_to_string(h.scripts.path().join("grid-frames"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn pending(h: &Harness) -> (String, String, Report) {
    h.settings(json!({"affectedRows":"1"}));
    let mut operation = h.driver.process.request("fixture-batch", "mutationBatch", json!({
        "mode":"manual","owner":null,"rollbackPrior":false,
        "expected":{"user":"SERVER_USER","session":"SESSION","role":"ROLE","database":"DB","schema":"PUBLIC","warehouse":"WH"},
        "units":[{"kind":"update","sql":"UPDATE synthetic SET value = ? WHERE id = ?","binds":["value","1"]}]
    }), Impact::Mutating).unwrap();
    let report = operation.wait_bounded(Duration::from_secs(5)).await;
    assert!(report.ok, "{report:?}");
    let summary = report.batch.as_ref().unwrap().summary.as_ref().unwrap();
    assert_eq!(
        summary.outcome,
        crate::drivers::snowflake_batch::Outcome::Pending
    );
    (
        summary.owner.unwrap().to_string(),
        summary.transaction_id.clone().unwrap(),
        report,
    )
}

const KINDS: [&str; 11] = [
    "context",
    "quotedIdentifierPolicy",
    "currentTransaction",
    "exactRelation",
    "columns",
    "describe",
    "zeroRowResolution",
    "exactRelation",
    "context",
    "quotedIdentifierPolicy",
    "currentTransaction",
];

fn assert_no_new_writes(h: &Harness, baseline: usize) {
    assert!(h.queries()[baseline..].iter().all(|sql| {
        !["BEGIN TRANSACTION", "COMMIT", "ROLLBACK"].contains(&sql.as_str())
            && !sql.starts_with("UPDATE ")
            && !sql.starts_with("INSERT ")
            && !sql.starts_with("DELETE ")
    }));
}

fn expected_sql(binding: &NativeEditBinding) -> Vec<String> {
    let relation = metadata::exact_relation_read(binding.context(), binding.relation()).unwrap();
    let [columns, describe, resolved] = metadata::inspection_reads(binding.relation()).unwrap();
    [
        metadata::CONTEXT_SQL,
        metadata::QUOTED_IDENTIFIER_POLICY_SQL,
        session::CURRENT_TRANSACTION_SQL,
        relation.sql(),
        columns.sql(),
        describe.sql(),
        resolved.sql(),
        relation.sql(),
        metadata::CONTEXT_SQL,
        metadata::QUOTED_IDENTIFIER_POLICY_SQL,
        session::CURRENT_TRANSACTION_SQL,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[tokio::test]
async fn grid_freshness_idle_brackets_all_sources_and_requires_idle_transaction() {
    let h = grid_fixture().await;
    let binding = binding(&h).await;
    h.settings(json!({}));
    let baseline = h.queries().len();
    {
        let _lane = h.driver.lane.lock().await;
        h.driver
            .verify_grid_freshness(&binding, None)
            .await
            .unwrap();
    }
    let sql = &h.queries()[baseline..];
    assert_eq!(sql, expected_sql(&binding));
    assert_eq!(sql[0], metadata::CONTEXT_SQL);
    assert_eq!(sql[8], metadata::CONTEXT_SQL);
    assert_eq!(sql[2], session::CURRENT_TRANSACTION_SQL);
    assert_eq!(sql[10], session::CURRENT_TRANSACTION_SQL);
    assert!(sql[4].contains(".INFORMATION_SCHEMA.COLUMNS AS C "));
    assert!(sql[5].starts_with("DESCRIBE TABLE "));
    assert!(sql[6].ends_with(" LIMIT 0"));
    assert_no_new_writes(&h, baseline);
    h.settings(json!({"transactionId":"123456789"}));
    let _lane = h.driver.lane.lock().await;
    assert_eq!(
        h.driver
            .verify_grid_freshness(&binding, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    drop(_lane);
    h.finish().await;
}

#[tokio::test]
async fn grid_freshness_rejects_drift_missing_sources_and_incomplete_metadata() {
    let h = grid_fixture().await;
    let binding = binding(&h).await;
    for settings in [
        json!({"contextDrift":true}),
        json!({"transactionDrift":true}),
        json!({"finalRelationDrift":true}),
        json!({"emptyRelations":true}),
        json!({"malformedContext":true}),
        json!({"changedColumn":true}),
        json!({"missingDescribe":true}),
        json!({"missingResolution":true}),
        json!({"excessColumns":true}),
    ] {
        h.settings(settings.clone());
        let baseline = h.queries().len();
        let _lane = h.driver.lane.lock().await;
        assert!(
            h.driver
                .verify_grid_freshness(&binding, None)
                .await
                .is_err(),
            "{settings}"
        );
        drop(_lane);
        assert_no_new_writes(&h, baseline);
        // Fully drained semantic rejection does not prevent explicit recovery.
        assert!(!h.driver.is_closed(), "{settings}");
    }
    h.settings(json!({}));
    let _lane = h.driver.lane.lock().await;
    h.driver
        .verify_grid_freshness(&binding, None)
        .await
        .unwrap();
    drop(_lane);
    h.finish().await;
}

#[tokio::test]
async fn grid_freshness_pending_uses_only_fixed_owner_reads_and_keeps_original_immutable() {
    let h = grid_fixture().await;
    let binding = binding(&h).await;
    let (token, transaction, original) = pending(&h).await;
    let retained = original.batch.clone();
    let baseline = h.queries().len();
    let frame_start = frames(&h).len();
    {
        let _lane = h.driver.lane.lock().await;
        h.driver
            .verify_grid_freshness(
                &binding,
                Some(GridReadOwner {
                    token: &token,
                    transaction_id: &transaction,
                }),
            )
            .await
            .unwrap();
    }
    let observed = frames(&h);
    let requests: Vec<_> = observed[frame_start..]
        .iter()
        .filter(|frame| frame["op"] == "mutationRead")
        .collect();
    assert_eq!(
        requests
            .iter()
            .map(|f| f["payload"]["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        KINDS
    );
    assert!(
        requests
            .iter()
            .all(|f| f["payload"]["owner"] == token && f["payload"]["credits"] == 1)
    );
    assert!(!observed[frame_start..].iter().any(|f| f["op"] == "query"));
    // Small reads may publish their terminal before another credit is needed.
    // The 1002-row case below proves actual multi-batch credit delivery.
    assert_eq!(h.queries().len() - baseline, 11);
    assert_eq!(h.queries()[baseline..], expected_sql(&binding));
    assert_no_new_writes(&h, baseline);
    assert_eq!(original.batch, retained);
    let mut outsider = h
        .driver
        .process
        .request(
            "outsider",
            "query",
            json!({"sql":"SELECT 1","binds":[],"classification":"read","credits":1}),
            Impact::Read,
        )
        .unwrap();
    assert!(!outsider.wait_bounded(Duration::from_secs(5)).await.ok);
    assert_eq!(h.queries().len() - baseline, 11);
    h.finish().await;
}

#[tokio::test]
async fn grid_freshness_commit_context_rejects_full_context_policy_and_transaction_drift() {
    let h = grid_fixture().await;
    let binding = binding(&h).await;
    let (token, transaction, _) = pending(&h).await;
    for settings in [
        json!({"ownerContextDrift":true}),
        json!({"afterPolicy":"true"}),
        json!({"afterSecondaryRoles":"{\"roles\":\"OTHER\",\"value\":\"ALL\"}"}),
        json!({"searchPath":"[\"DB.OTHER\"]"}),
        json!({"afterBoundaryTransactionId":null}),
        json!({"afterBoundaryTransactionId":"234567890"}),
    ] {
        h.settings(settings.clone());
        let baseline = h.queries().len();
        let _lane = h.driver.lane.lock().await;
        assert!(
            h.driver
                .verify_grid_owner_context(
                    binding.context(),
                    GridReadOwner {
                        token: &token,
                        transaction_id: &transaction
                    }
                )
                .await
                .is_err(),
            "{settings}"
        );
        drop(_lane);
        assert_no_new_writes(&h, baseline);
        assert!(!h.driver.is_closed());
    }
    h.settings(json!({}));
    let baseline = h.queries().len();
    let _lane = h.driver.lane.lock().await;
    h.driver
        .verify_grid_owner_context(
            binding.context(),
            GridReadOwner {
                token: &token,
                transaction_id: &transaction,
            },
        )
        .await
        .unwrap();
    drop(_lane);
    assert_eq!(h.queries().len() - baseline, 6);
    h.finish().await;
}

#[tokio::test]
async fn grid_freshness_pending_rejects_foreign_owner_before_sql_and_drains_bounded_rows() {
    let h = grid_fixture().await;
    let binding = binding(&h).await;
    let (token, transaction, _) = pending(&h).await;
    let baseline = h.queries().len();
    let _lane = h.driver.lane.lock().await;
    assert!(
        h.driver
            .verify_grid_freshness(
                &binding,
                Some(GridReadOwner {
                    token: &new_id(),
                    transaction_id: &transaction
                })
            )
            .await
            .is_err()
    );
    assert_eq!(h.queries().len(), baseline);
    drop(_lane);
    h.settings(json!({"excessColumns":true}));
    let frame_start = frames(&h).len();
    let _lane = h.driver.lane.lock().await;
    assert_eq!(
        h.driver
            .verify_grid_freshness(
                &binding,
                Some(GridReadOwner {
                    token: &token,
                    transaction_id: &transaction
                })
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResultLimitReached
    );
    drop(_lane);
    assert!(
        frames(&h)[frame_start..]
            .iter()
            .filter(|f| f["op"] == "credit")
            .count()
            > 2
    );
    assert!(!h.driver.is_closed());
    assert_no_new_writes(&h, baseline);
    h.finish().await;
}

#[tokio::test]
async fn grid_freshness_abandon_or_protocol_loss_retires_owned_helper_without_boundary_or_replay() {
    for protocol in [false, true] {
        let h = grid_fixture().await;
        let binding = binding(&h).await;
        let (token, transaction, _) = pending(&h).await;
        h.settings(if protocol {
            json!({"protocolGrid":true})
        } else {
            json!({"holdGrid":true})
        });
        let baseline = h.queries().len();
        let driver = h.driver.clone();
        let task = tokio::spawn(async move {
            let _lane = driver.lane.lock().await;
            driver
                .verify_grid_freshness(
                    &binding,
                    Some(GridReadOwner {
                        token: &token,
                        transaction_id: &transaction,
                    }),
                )
                .await
        });
        if protocol {
            assert!(
                tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
        } else {
            h.wait_until(|| h.scripts.path().join("held-grid").exists())
                .await;
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        assert!(h.driver.is_closed());
        let receipt = h.cleanup.wait(Duration::from_secs(10)).await.unwrap();
        assert!(receipt.reaped);
        assert_no_new_writes(&h, baseline);
        assert_eq!(
            h.queries()[baseline..]
                .iter()
                .filter(|sql| sql.contains(".INFORMATION_SCHEMA.COLUMNS AS C "))
                .count(),
            1
        );
        h.finish().await;
    }
}
