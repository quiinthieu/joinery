//! Real owned child identity with synthetic native metadata/rows; no SDK or SQL.
use super::*;
use crate::{
    domain::{ColumnKind, TableRef},
    drivers::{
        snowflake_metadata::{ResolvedSchema, SecondaryRoles},
        snowflake_process::{Bounds, Launch, Supervisor},
    },
    relation_inspection::{
        ColumnIdentity, InspectedColumnKind, RelationAvailability, RelationTraits,
        WriteDefaultObservationStatus,
    },
};
use std::os::unix::fs::PermissionsExt;

async fn helper(generation: &str) -> Supervisor {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().canonicalize().unwrap();
    let script = directory.join("snapshot-owned-child.sh");
    let hello = serde_json::to_vec(&serde_json::json!({
        "v":1,"type":"hello","protocol":"joinery-snowflake","sdkVersion":"3.4.1",
        "limits":{"frameBytes":1048576,"batchRows":128,"batchBytes":262144,"queued":8,"credits":32}
    }))
    .unwrap();
    let mut frame = (hello.len() as u32).to_be_bytes().to_vec();
    frame.extend(hello);
    let escaped: String = frame.iter().map(|byte| format!("\\{byte:03o}")).collect();
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '{escaped}'\nexec /bin/cat\n"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    Supervisor::spawn(
        Launch::new(script, None, directory, root).unwrap(),
        generation,
        Bounds::default(),
    )
    .await
    .unwrap()
}
async fn reaped_handle() -> HelperGeneration {
    let child = helper(&uuid::Uuid::new_v4().to_string()).await;
    let handle = child.helper_generation();
    reap(&child).await;
    handle
}
async fn reap(child: &Supervisor) {
    child.request_retirement();
    assert!(
        child
            .cleanup_observer()
            .wait(Duration::from_secs(5))
            .await
            .is_some_and(|r| r.child_started && r.reaped)
    );
}
struct Observations {
    context: LiveContext,
    relation: BrowseRelation,
    columns: Vec<ColumnInspection>,
    browse: String,
    target: String,
    inspection: String,
}
impl Observations {
    fn new() -> Self {
        Self {
            context: LiveContext {
                principal: "USER".into(),
                role: Some("ROLE".into()),
                database: Some("DB".into()),
                schema: Some("PUBLIC".into()),
                warehouse: Some("WH".into()),
                session_id: "123".into(),
                search_path: vec![ResolvedSchema {
                    raw: "PUBLIC".into(),
                    database: Some("DB".into()),
                    schema: Some("PUBLIC".into()),
                }],
                secondary_roles: SecondaryRoles {
                    roles: "[]".into(),
                    value: "NONE".into(),
                },
                quoted_identifiers_ignore_case: false,
            },
            relation: BrowseRelation {
                database: "DB".into(),
                table: TableRef {
                    schema: "PUBLIC".into(),
                    name: "keyless\"table".into(),
                },
                table_type: "BASE TABLE".into(),
                traits: RelationTraits {
                    transient: Some(false),
                    temporary: Some(false),
                    iceberg: Some(false),
                    dynamic: Some(false),
                    hybrid: Some(false),
                    immutable: Some(false),
                },
                availability: RelationAvailability::Available,
                created: Some("2026-01-01".into()),
                last_ddl: Some("2026-01-02".into()),
            },
            columns: vec![text(), number(), boolean()],
            browse: uuid::Uuid::new_v4().to_string(),
            target: uuid::Uuid::new_v4().to_string(),
            inspection: uuid::Uuid::new_v4().to_string(),
        }
    }
    fn binding(&self, helper: &HelperGeneration) -> NativeEditBinding {
        NativeEditBinding::new(
            "observer-session",
            helper,
            &self.context,
            &self.browse,
            &self.target,
            &self.relation,
            &self.inspection,
            &self.columns,
        )
        .unwrap()
    }
}
fn text() -> ColumnInspection {
    ColumnInspection {
        name: "text".into(),
        ordinal: 1,
        full_type: "VARCHAR(20)".into(),
        data_type: "TEXT".into(),
        data_type_alias: None,
        kind: InspectedColumnKind::Column,
        result_kind: ColumnKind::Text,
        nullable: true,
        default_expression: None,
        descriptor_default_expression: None,
        write_default_expression: None,
        write_default_status: WriteDefaultObservationStatus::NotReported,
        comment: None,
        character_maximum_length: Some("20".into()),
        character_octet_length: Some("80".into()),
        numeric_precision: None,
        numeric_precision_radix: None,
        numeric_scale: None,
        identity: ColumnIdentity {
            is_identity: Some(false),
            generation: None,
            start: None,
            increment: None,
            ordered: None,
        },
        expression: None,
    }
}
fn number() -> ColumnInspection {
    let mut c = text();
    c.name = "number".into();
    c.ordinal = 2;
    c.full_type = "NUMBER(38,2)".into();
    c.data_type = "NUMBER".into();
    c.result_kind = ColumnKind::Decimal;
    c.character_maximum_length = None;
    c.character_octet_length = None;
    c.numeric_precision = Some("38".into());
    c.numeric_precision_radix = Some("10".into());
    c.numeric_scale = Some("2".into());
    c
}
fn boolean() -> ColumnInspection {
    let mut c = text();
    c.name = "boolean".into();
    c.ordinal = 3;
    c.full_type = "BOOLEAN".into();
    c.data_type = "BOOLEAN".into();
    c.result_kind = ColumnKind::Boolean;
    c.character_maximum_length = None;
    c.character_octet_length = None;
    c
}
fn rows() -> Vec<TextRow> {
    vec![
        vec![
            Some("".into()),
            Some("9007199254740993.12".into()),
            Some("true".into()),
        ],
        vec![None, Some("0.00".into()), None],
    ]
}
fn delete(index: usize) -> Vec<SnapshotMutation> {
    vec![SnapshotMutation::Delete { row_index: index }]
}
fn conflict<T>(r: AppResult<T>) {
    assert!(matches!(r,Err(e) if e.code==ErrorCode::Conflict));
}
fn capacity<T>(r: AppResult<T>) {
    assert!(matches!(r,Err(e) if e.code==ErrorCode::ResultLimitReached));
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_rejects_same_uuid_foreign_physical_helper_and_is_one_use() {
    let generation = uuid::Uuid::new_v4().to_string();
    let a = helper(&generation).await;
    let b = helper(&generation).await;
    let ah = a.helper_generation();
    let bh = b.helper_generation();
    reap(&a).await;
    reap(&b).await;
    assert!(!ah.same_helper(&bh));
    let obs = Observations::new();
    let store = SnapshotStore::new();
    let id = store.retain(obs.binding(&ah), rows()).unwrap();
    conflict(store.prepare("observer-session", &id, &obs.binding(&bh), &delete(0)));
    conflict(store.prepare("other-session", &id, &obs.binding(&ah), &delete(0)));
    let plan = store
        .prepare("observer-session", &id, &obs.binding(&ah), &delete(0))
        .unwrap();
    assert_eq!(plan.snapshot_id(), id);
    assert!(plan.binding().helper_generation().same_helper(&ah));
    assert_eq!(plan.binding().session_id(), "observer-session");
    assert_eq!(plan.binding().context(), &obs.context);
    assert_eq!(plan.binding().relation(), &obs.relation);
    assert_eq!(plan.binding().browse_revision(), obs.browse);
    assert_eq!(plan.binding().target_id(), obs.target);
    assert_eq!(plan.binding().inspection_revision(), obs.inspection);
    assert_eq!(plan.binding().columns(), obs.columns);
    conflict(store.prepare("observer-session", &id, &obs.binding(&ah), &delete(0)));
    plan.validate_current(&obs.binding(&ah)).unwrap();
    assert_eq!(store.usage().1, 1);
    drop(plan);
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_pins_null_empty_exact_decimal_and_keyless_full_originals() {
    let h = reaped_handle().await;
    let obs = Observations::new();
    let store = SnapshotStore::new();
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let p = store
        .prepare(
            "observer-session",
            &id,
            &obs.binding(&h),
            &[
                SnapshotMutation::Update {
                    row_index: 0,
                    changes: vec![(0, CellValue::Null)],
                },
                SnapshotMutation::Delete { row_index: 1 },
            ],
        )
        .unwrap();
    let units = p.compiled().units();
    assert_eq!(units.len(), 2);
    assert_eq!(
        units[0]
            .bindings()
            .iter()
            .map(|b| b.value())
            .collect::<Vec<_>>(),
        vec![None, Some(""), Some("9007199254740993.12"), Some("true")]
    );
    assert_eq!(
        units[1]
            .bindings()
            .iter()
            .map(|b| b.value())
            .collect::<Vec<_>>(),
        vec![None, Some("0.00"), None]
    );
    for unit in units {
        assert_eq!(unit.expected_rows(), 1);
        assert_eq!(unit.sql().matches("IS NOT DISTINCT FROM").count(), 3);
        assert!(!unit.sql().contains("9007199254740993"));
    }
    assert!(units[0].sql().contains("TO_BINARY"));
    drop(p);
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let p = store
        .prepare(
            "observer-session",
            &id,
            &obs.binding(&h),
            &[SnapshotMutation::Insert {
                values: vec![
                    (0, CellValue::Default),
                    (1, CellValue::Text("1.23".into())),
                    (2, CellValue::Null),
                ],
            }],
        )
        .unwrap();
    assert!(p.compiled().units()[0].sql().contains("DEFAULT"));
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_rejects_context_metadata_revision_and_forged_token_drift() {
    let h = reaped_handle().await;
    let mut obs = Observations::new();
    let store = SnapshotStore::new();
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    conflict(store.prepare(
        "observer-session",
        &uuid::Uuid::new_v4().to_string(),
        &obs.binding(&h),
        &delete(0),
    ));
    let old = obs.context.clone();
    obs.context.role = Some("OTHER".into());
    conflict(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
    obs.context = old.clone();
    obs.context.search_path[0].raw = "OTHER".into();
    conflict(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
    obs.context = old.clone();
    obs.context.secondary_roles.value = "ALL".into();
    conflict(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
    obs.context = old;
    obs.columns[0].comment = Some("changed native metadata".into());
    conflict(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
    obs.columns[0].comment = None;
    let ddl = obs.relation.last_ddl.take();
    conflict(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
    obs.relation.last_ddl = ddl;
    for field in 0..3 {
        let saved = match field {
            0 => std::mem::replace(&mut obs.browse, uuid::Uuid::new_v4().to_string()),
            1 => std::mem::replace(&mut obs.target, uuid::Uuid::new_v4().to_string()),
            _ => std::mem::replace(&mut obs.inspection, uuid::Uuid::new_v4().to_string()),
        };
        conflict(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
        match field {
            0 => obs.browse = saved,
            1 => obs.target = saved,
            _ => obs.inspection = saved,
        }
    }
    assert!(
        store
            .prepare("observer-session", &id, &obs.binding(&h), &delete(0))
            .is_ok()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_replacement_and_revocation_reach_prepared_budget_leases() {
    let h = reaped_handle().await;
    let obs = Observations::new();
    let store = Arc::new(SnapshotStore::new());
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let p = store
        .prepare("observer-session", &id, &obs.binding(&h), &delete(0))
        .unwrap();
    let replacement = store.retain(obs.binding(&h), rows()).unwrap();
    conflict(p.validate_current(&obs.binding(&h)));
    assert_eq!(store.usage().1, 2);
    drop(p);
    assert_eq!(store.usage().1, 1);
    store.revoke_target("observer-session", &h, &obs.target);
    conflict(store.prepare(
        "observer-session",
        &replacement,
        &obs.binding(&h),
        &delete(0),
    ));
    assert_eq!(store.usage(), (0, 0));
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let p = store
        .prepare("observer-session", &id, &obs.binding(&h), &delete(0))
        .unwrap();
    store.revoke_session("observer-session");
    conflict(p.validate_current(&obs.binding(&h)));
    assert_eq!(store.usage().1, 1);
    drop(p);
    assert_eq!(store.usage(), (0, 0));
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let p = store
        .prepare("observer-session", &id, &obs.binding(&h), &delete(0))
        .unwrap();
    store.clear();
    conflict(p.validate_current(&obs.binding(&h)));
    drop(p);
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_and_plan_expiry_never_restore_authority() {
    let h = reaped_handle().await;
    let obs = Observations::new();
    let store = SnapshotStore::new();
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    *store.registry.lock().snapshots[&id]
        .authority
        .expires
        .lock() = Instant::now() - Duration::from_secs(1);
    conflict(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
    assert_eq!(store.usage(), (0, 0));
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let p = store
        .prepare("observer-session", &id, &obs.binding(&h), &delete(0))
        .unwrap();
    *p.authority.expires.lock() = Instant::now() - Duration::from_secs(1);
    conflict(p.validate_current(&obs.binding(&h)));
    assert_eq!(store.usage().1, 1);
    drop(p);
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_failed_preparation_consumes_token_without_cloning_unbounded_inputs() {
    let h = reaped_handle().await;
    let obs = Observations::new();
    let store = SnapshotStore::new();
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    conflict(store.prepare(
        "observer-session",
        &id,
        &obs.binding(&h),
        &[
            SnapshotMutation::Delete { row_index: 0 },
            SnapshotMutation::Delete { row_index: 0 },
        ],
    ));
    conflict(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
    assert_eq!(store.usage(), (0, 0));
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    capacity(store.prepare(
        "observer-session",
        &id,
        &obs.binding(&h),
        &[SnapshotMutation::Insert {
            values: vec![(0, CellValue::Text("x".repeat(MAX_CELL_BYTES + 1)))],
        }],
    ));
    assert_eq!(store.usage(), (0, 0));
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    assert!(
        store
            .prepare("observer-session", &id, &obs.binding(&h), &delete(9))
            .is_err()
    );
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_count_guard_combines_tokens_and_prepared_plans() {
    let h = reaped_handle().await;
    let mut obs = Observations::new();
    let store = SnapshotStore::new();
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let p = store
        .prepare("observer-session", &id, &obs.binding(&h), &delete(0))
        .unwrap();
    for _ in 1..MAX_ENTRIES {
        obs.target = uuid::Uuid::new_v4().to_string();
        store.retain(obs.binding(&h), rows()).unwrap();
    }
    assert_eq!(store.usage().1, MAX_ENTRIES);
    obs.target = uuid::Uuid::new_v4().to_string();
    capacity(store.retain(obs.binding(&h), rows()));
    assert_eq!(store.usage().1, MAX_ENTRIES);
    drop(p);
    assert_eq!(store.usage().1, MAX_ENTRIES - 1);
    store.retain(obs.binding(&h), rows()).unwrap();
    store.clear();
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_preview_bounds_and_compile_peak_fail_closed() {
    let h = reaped_handle().await;
    let obs = Observations::new();
    let store = SnapshotStore::new();
    capacity(store.retain(obs.binding(&h), vec![rows()[0].clone(); MAX_ROWS + 1]));
    let mut oversized = Vec::with_capacity(MAX_PREVIEW_BYTES);
    oversized.push(rows()[0].clone());
    capacity(store.retain(obs.binding(&h), oversized));
    let mut row = rows().remove(0);
    row[0] = Some("x".repeat(MAX_CELL_BYTES + 1));
    capacity(store.retain(obs.binding(&h), vec![row]));
    assert!(store.retain(obs.binding(&h), vec![vec![]]).is_err());
    assert_eq!(store.usage(), (0, 0));
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    // Logical peak-accounting guard, not an RSS/full-capacity workload claim.
    let held = BudgetLease::claim(&store.usage, MAX_BYTES - MAX_BATCH_BYTES).unwrap();
    capacity(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
    assert_eq!(store.usage().1, 1);
    drop(held);
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_concurrent_prepare_has_exactly_one_winner() {
    let h = reaped_handle().await;
    let obs = Observations::new();
    let store = Arc::new(SnapshotStore::new());
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let current = Arc::new(obs.binding(&h));
    let start = Arc::new(std::sync::Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            let id = id.clone();
            let current = current.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                match store.prepare("observer-session", &id, &current, &delete(0)) {
                    Ok(plan) => {
                        plan.validate_current(&current).unwrap();
                        true
                    }
                    Err(error) => {
                        assert_eq!(error.code, ErrorCode::Conflict);
                        false
                    }
                }
            })
        })
        .collect();
    start.wait();
    let wins = workers
        .into_iter()
        .map(|worker| usize::from(worker.join().unwrap()))
        .sum::<usize>();
    assert_eq!(wins, 1);
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn native_snapshot_dispatch_budget_is_shared_atomic_and_retained_until_drop() {
    let h = reaped_handle().await;
    let obs = Observations::new();
    let store = SnapshotStore::new();
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let mut plan = store
        .prepare("observer-session", &id, &obs.binding(&h), &delete(0))
        .unwrap();
    let baseline = store.usage();
    let held = BudgetLease::claim(&store.usage, MAX_BYTES - baseline.0 - 1).unwrap();
    let saturated = store.usage();
    capacity(plan.reserve_dispatch());
    assert_eq!(store.usage(), saturated, "failed reservation is atomic");
    drop(held);
    plan.reserve_dispatch().unwrap();
    let reserved = store.usage();
    assert!(reserved.0 > baseline.0 + 2 * super::super::snowflake_process::MAX_FRAME);
    assert_eq!(reserved.1, baseline.1);
    plan.reserve_dispatch().unwrap();
    assert_eq!(store.usage(), reserved, "rechecking cannot double-charge");
    store.revoke_session("observer-session");
    conflict(plan.validate_current(&obs.binding(&h)));
    assert_eq!(
        store.usage(),
        reserved,
        "retained payload still owns its budget"
    );
    drop(plan);
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_prepared_mints_fresh_token_preserves_original_ttl_and_rejects_drift_or_reserved() {
    let h = reaped_handle().await;
    let mut obs = Observations::new();
    let store = SnapshotStore::new();
    let id = store.retain(obs.binding(&h), rows()).unwrap();
    let orig_expires = store.registry.lock().snapshots[&id]
        .authority
        .snapshot_expires;
    let plan = store
        .prepare("observer-session", &id, &obs.binding(&h), &delete(0))
        .unwrap();
    let restored = store.cancel_prepared(plan, obs.binding(&h)).unwrap();
    assert_ne!(restored, id, "consumed token must never be revived");
    conflict(store.prepare("observer-session", &id, &obs.binding(&h), &delete(0)));
    assert_eq!(
        store.registry.lock().snapshots[&restored]
            .authority
            .snapshot_expires,
        orig_expires
    );
    assert_eq!(store.usage().1, 1);

    // Works at MAX_ENTRIES ceiling without overflowing entry count.
    let base_target = obs.target.clone();
    for _ in 1..MAX_ENTRIES {
        obs.target = uuid::Uuid::new_v4().to_string();
        store.retain(obs.binding(&h), rows()).unwrap();
    }
    obs.target = base_target;
    assert_eq!(store.usage().1, MAX_ENTRIES);
    let plan2 = store
        .prepare("observer-session", &restored, &obs.binding(&h), &delete(1))
        .unwrap();
    let restored2 = store.cancel_prepared(plan2, obs.binding(&h)).unwrap();
    assert_eq!(store.usage().1, MAX_ENTRIES);
    store.clear();
    assert_eq!(store.usage(), (0, 0));
    conflict(store.prepare("observer-session", &restored2, &obs.binding(&h), &delete(1)));

    // Dispatch-reserved plans and drifted bindings fail closed and release budget.
    let id3 = store.retain(obs.binding(&h), rows()).unwrap();
    let mut reserved_plan = store
        .prepare("observer-session", &id3, &obs.binding(&h), &delete(0))
        .unwrap();
    reserved_plan.reserve_dispatch().unwrap();
    conflict(store.cancel_prepared(reserved_plan, obs.binding(&h)));
    assert_eq!(store.usage(), (0, 0));

    let id4 = store.retain(obs.binding(&h), rows()).unwrap();
    let drifted_plan = store
        .prepare("observer-session", &id4, &obs.binding(&h), &delete(0))
        .unwrap();
    obs.context.role = Some("DRIFTED".into());
    conflict(store.cancel_prepared(drifted_plan, obs.binding(&h)));
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn thousand_column_binding_retains_and_prepares_within_snapshot_budget() {
    let h = reaped_handle().await;
    let mut obs = Observations::new();
    obs.columns = (0..MAX_COLUMNS)
        .map(|i| {
            let mut c = boolean();
            c.name = format!("c{i}");
            c.ordinal = i as u32 + 1;
            c
        })
        .collect();
    let store = SnapshotStore::new();
    let wide_row = vec![Some("true".into()); MAX_COLUMNS];
    let id = store.retain(obs.binding(&h), vec![wide_row]).unwrap();
    let plan = store
        .prepare(
            "observer-session",
            &id,
            &obs.binding(&h),
            &[SnapshotMutation::Update {
                row_index: 0,
                changes: vec![(MAX_COLUMNS - 1, CellValue::Text("false".into()))],
            }],
        )
        .unwrap();
    assert_eq!(plan.compiled().units().len(), 1);
    drop(plan);
    assert_eq!(store.usage(), (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn snapshot_binding_rejects_cross_page_or_sort_filter_slice_drift() {
    use crate::{
        drivers::snowflake_browsing::PreviewSlice,
        relation_inspection::{
            RelationFilter, RelationFilterOperator, RelationSort, RelationSortDirection,
        },
    };
    let h = reaped_handle().await;
    let obs = Observations::new();
    let slice_a = PreviewSlice {
        offset: 500,
        sort: Some(RelationSort {
            column: "text".into(),
            direction: RelationSortDirection::Asc,
            nulls: None,
        }),
        filters: vec![RelationFilter {
            column: "boolean".into(),
            operator: RelationFilterOperator::Equals,
            value: Some("true".into()),
        }],
    };
    let slice_b = PreviewSlice {
        offset: 0,
        ..slice_a.clone()
    };
    let make_a = || {
        NativeEditBinding::with_slice(
            "observer-session",
            &h,
            &obs.context,
            &obs.browse,
            &obs.target,
            &obs.relation,
            &obs.inspection,
            &obs.columns,
            slice_a.clone(),
        )
        .unwrap()
    };
    let binding_b = NativeEditBinding::with_slice(
        "observer-session",
        &h,
        &obs.context,
        &obs.browse,
        &obs.target,
        &obs.relation,
        &obs.inspection,
        &obs.columns,
        slice_b,
    )
    .unwrap();
    let store = SnapshotStore::new();
    let id = store.retain(make_a(), rows()).unwrap();
    // Cannot prepare against a different slice; the valid snapshot remains intact for slice_a.
    conflict(store.prepare("observer-session", &id, &binding_b, &delete(0)));
    assert_eq!(store.usage().1, 1);

    let binding_a = make_a();
    let plan = store
        .prepare("observer-session", &id, &binding_a, &delete(0))
        .unwrap();
    conflict(plan.validate_current(&binding_b));
    plan.validate_current(&binding_a).unwrap();
    drop(plan);
    assert_eq!(store.usage(), (0, 0));
}
