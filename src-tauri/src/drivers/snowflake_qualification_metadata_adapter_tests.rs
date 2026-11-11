use super::*;
use crate::drivers::snowflake_qualification_metadata::MetadataObserver;
use std::sync::atomic::AtomicUsize;

type RecordedSchemas = (MetadataSite, Vec<String>, Vec<String>);
type RecordedRelations = (MetadataSite, Option<String>, Vec<BrowseRelation>);

#[derive(Default)]
struct Recorder {
    schemas: Mutex<Vec<RecordedSchemas>>,
    relations: Mutex<Vec<RecordedRelations>>,
    bounded_census: AtomicBool,
    census: Mutex<Option<Vec<String>>>,
    reject_schema: Mutex<Option<MetadataSite>>,
    reject_relation: Mutex<Option<MetadataSite>>,
    checks: AtomicUsize,
    deny: AtomicBool,
}

fn denied() -> AppError {
    AppError::new(ErrorCode::PolicyBlocked, "synthetic metadata rejection")
}

impl MetadataObserver for Recorder {
    fn check(&self) -> AppResult<()> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        if self.deny.load(Ordering::SeqCst) {
            return Err(denied());
        }
        Ok(())
    }
    fn observe_schemas(&self, observation: SchemaObservation<'_>) -> AppResult<()> {
        assert_eq!(observation.context.database.as_deref(), Some("DB"));
        self.schemas.lock().push((
            observation.site,
            observation.wire_names.to_vec(),
            observation.parsed_names.to_vec(),
        ));
        if *self.reject_schema.lock() == Some(observation.site) {
            return Err(denied());
        }
        if self.bounded_census.load(Ordering::SeqCst) {
            // Test policy only: the seam itself must not bake in a run ledger.
            if observation.wire_names.len() > 32
                || observation.wire_names != observation.parsed_names
                || !observation.parsed_names.iter().any(|name| name == "PUBLIC")
            {
                return Err(denied());
            }
            let mut census = self.census.lock();
            if census
                .as_ref()
                .is_some_and(|previous| previous != observation.parsed_names)
            {
                return Err(denied());
            }
            *census = Some(observation.parsed_names.to_vec());
        }
        Ok(())
    }
    fn observe_relations(&self, observation: RelationObservation<'_>) -> AppResult<()> {
        assert_eq!(observation.context.database.as_deref(), Some("DB"));
        self.relations.lock().push((
            observation.site,
            observation.schema.map(str::to_owned),
            observation.relations.to_vec(),
        ));
        if *self.reject_relation.lock() == Some(observation.site) {
            return Err(denied());
        }
        Ok(())
    }
}

async fn harness(observer: &Arc<Recorder>) -> Harness {
    Harness::open_policy_and_observer(
        SafeMode::Confirm,
        false,
        true,
        Some(MetadataAttachment::new(observer.clone())),
    )
    .await
}

fn create_request() -> SnowflakeDdlPreflightRequest {
    SnowflakeDdlPreflightRequest {
        session_id: "observer-session".into(),
        action: SnowflakeDdlAction::CreateTable {
            database: "DB".into(),
            schema: "PUBLIC".into(),
            name: "NEW_ORDERS".into(),
            columns: vec![SnowflakeDdlColumnDraft {
                name: "ID".into(),
                data_type: "NUMBER(38,0)".into(),
                nullable: true,
            }],
        },
    }
}

#[tokio::test]
async fn qualification_metadata_sees_original_wire_order_after_production_validation() {
    let observer = Arc::new(Recorder::default());
    let h = harness(&observer).await;
    assert!(observer.checks.load(Ordering::SeqCst) > 0);
    assert!(observer.schemas.lock().is_empty());
    h.settings(json!({"schemas":["PUBLIC","OTHER"]}));
    h.driver.relation_catalog().await.unwrap();
    let schemas = observer.schemas.lock().clone();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].0, MetadataSite::RelationCatalogBatch);
    assert_eq!(schemas[0].1, ["PUBLIC", "OTHER"]);
    assert_eq!(schemas[0].2, ["OTHER", "PUBLIC"]);
    let relations = observer.relations.lock().clone();
    assert_eq!(relations.len(), 3); // two parsed batches plus one validated snapshot
    assert_eq!(relations[2].0, MetadataSite::RelationCatalogSnapshot);
    assert_eq!(relations[2].1, None);
    for (_, _, batch) in relations {
        for relation in batch {
            assert_eq!(relation.created.as_deref(), Some("created"));
            assert_eq!(relation.last_ddl.as_deref(), Some("last-ddl"));
            assert_eq!(relation.traits.temporary, Some(false));
        }
    }
    h.finish().await;
}

#[tokio::test]
async fn qualification_metadata_rejects_overflow_order_missing_and_drift_before_dependent_sql() {
    let overflow: Vec<_> = (0..32)
        .map(|index| format!("A{index:02}"))
        .chain(std::iter::once("PUBLIC".into()))
        .collect();
    for schemas in [
        overflow,
        vec!["PUBLIC".into(), "OTHER".into()],
        vec!["OTHER".into()],
    ] {
        let observer = Arc::new(Recorder::default());
        observer.bounded_census.store(true, Ordering::SeqCst);
        let h = harness(&observer).await;
        h.settings(json!({"schemas":schemas}));
        assert_eq!(
            h.driver
                .preflight_snowflake_ddl("observer-session", &create_request())
                .await
                .unwrap_err()
                .code,
            ErrorCode::PolicyBlocked
        );
        assert!(
            !h.queries()
                .iter()
                .any(|sql| sql.contains(".INFORMATION_SCHEMA.TABLES ")
                    || sql.starts_with("CREATE TABLE"))
        );
        let count = h.queries().len();
        assert!(
            h.driver
                .session_context_options("observer-session")
                .await
                .is_err()
        );
        assert_eq!(h.queries().len(), count); // local latch survives another route
        h.finish().await;
    }
    let observer = Arc::new(Recorder::default());
    observer.bounded_census.store(true, Ordering::SeqCst);
    let h = harness(&observer).await;
    h.driver
        .session_context_options("observer-session")
        .await
        .unwrap();
    h.settings(json!({"schemas":["A","PUBLIC"]}));
    let before = h.queries().len();
    assert!(h.driver.relation_catalog().await.is_err());
    assert_eq!(h.queries().len() - before, 3); // context, policy, schema list only
    assert!(h.driver.browsing.lock().snapshot.is_none());
    h.finish().await;
}

#[tokio::test]
async fn qualification_metadata_never_labels_malformed_metadata_validated() {
    let observer = Arc::new(Recorder::default());
    let h = harness(&observer).await;
    h.settings(json!({"schemas":["PUBLIC","PUBLIC"]}));
    assert!(h.driver.relation_catalog().await.is_err());
    assert!(observer.schemas.lock().is_empty());
    assert!(observer.relations.lock().is_empty());
    assert!(
        !h.queries()
            .iter()
            .any(|sql| sql.contains(".INFORMATION_SCHEMA.TABLES "))
    );
    h.finish().await;
}

#[tokio::test]
async fn qualification_metadata_marker_denial_blocks_catalog_publication_and_later_mutation() {
    let observer = Arc::new(Recorder::default());
    *observer.reject_relation.lock() = Some(MetadataSite::RelationCatalogSnapshot);
    let h = harness(&observer).await;
    assert!(h.driver.relation_catalog().await.is_err());
    assert!(h.driver.browsing.lock().snapshot.is_none());
    let observed = observer.relations.lock().clone();
    assert!(observed.iter().any(|(site, _, batch)| {
        *site == MetadataSite::RelationCatalogSnapshot
            && batch
                .iter()
                .all(|relation| relation.created.as_deref() == Some("created"))
    }));
    let before = h.queries().len();
    assert!(
        h.driver
            .preflight_snowflake_ddl("observer-session", &create_request())
            .await
            .is_err()
    );
    assert_eq!(h.queries().len(), before);
    h.finish().await;
}

#[tokio::test]
async fn qualification_metadata_exact_relation_denial_precedes_column_inspection() {
    let observer = Arc::new(Recorder::default());
    let h = harness(&observer).await;
    let catalog = h.driver.relation_catalog().await.unwrap();
    let target = catalog
        .relations
        .iter()
        .find(|target| target.table.schema == "PUBLIC")
        .unwrap();
    *observer.reject_relation.lock() = Some(MetadataSite::ExactRelationCheck);
    assert!(
        h.driver
            .inspect_relation(&InspectRelationRequest {
                revision: catalog.revision,
                target_id: target.target_id.clone()
            })
            .await
            .is_err()
    );
    assert!(
        !h.queries()
            .iter()
            .any(|sql| sql.contains(".INFORMATION_SCHEMA.COLUMNS "))
    );
    {
        let observed = observer.relations.lock();
        let (_, schema, targets) = observed.last().unwrap();
        assert_eq!(schema.as_deref(), Some("PUBLIC"));
        assert_eq!(targets[0].created.as_deref(), Some("created"));
    }
    h.finish().await;
}

#[tokio::test]
async fn qualification_metadata_create_collision_callback_precedes_grant_publication() {
    let observer = Arc::new(Recorder::default());
    *observer.reject_relation.lock() = Some(MetadataSite::CreateTargetCheck);
    let h = harness(&observer).await;
    assert!(
        h.driver
            .preflight_snowflake_ddl("observer-session", &create_request())
            .await
            .is_err()
    );
    assert!(h.driver.ddl_authority.lock().grant.is_none());
    assert_eq!(
        observer.schemas.lock()[0].0,
        MetadataSite::CreateTargetCheck
    );
    assert_eq!(
        observer.relations.lock()[0].0,
        MetadataSite::CreateTargetCheck
    );
    assert!(
        !h.queries()
            .iter()
            .any(|sql| sql.starts_with("CREATE TABLE"))
    );
    h.finish().await;
}

#[tokio::test]
async fn qualification_metadata_options_and_completion_callbacks_precede_downstream_reads() {
    for site in [
        MetadataSite::SessionContextOptions,
        MetadataSite::CompletionCatalog,
    ] {
        let observer = Arc::new(Recorder::default());
        *observer.reject_schema.lock() = Some(site);
        let h = harness(&observer).await;
        if site == MetadataSite::SessionContextOptions {
            assert!(
                h.driver
                    .session_context_options("observer-session")
                    .await
                    .is_err()
            );
            assert!(h.driver.session_options.lock().snapshot.is_none());
        } else {
            let identity = h.driver.completion_identity().await.unwrap();
            assert!(h.driver.completion_catalog(&identity).await.is_err());
        }
        assert_eq!(observer.schemas.lock()[0].0, site);
        assert!(
            !h.queries()
                .iter()
                .any(|sql| sql.contains(".INFORMATION_SCHEMA.TABLES "))
        );
        h.finish().await;
    }
}

#[tokio::test]
async fn qualification_metadata_observer_denial_after_ddl_ack_preserves_statement_success() {
    let observer = Arc::new(Recorder::default());
    let h = harness(&observer).await;
    let request = h.prepare_create_ddl().await;
    h.settings(json!({"holdDdlObserver":true}));
    let driver = h.driver.clone();
    let submitted = request.clone();
    let task =
        tokio::spawn(async move { driver.snowflake_ddl("observer-session", &submitted).await });
    h.wait_until(|| h.scripts.path().join("held-ddl-observer").exists())
        .await;
    observer.deny.store(true, Ordering::SeqCst);
    std::fs::write(h.scripts.path().join("release"), "").unwrap();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
    assert_eq!(
        result.summary_status,
        SnowflakeDdlSummaryStatus::Acknowledged
    );
    assert!(result.retired && result.error.is_some() && result.catalog_reconciliation_required);
    assert_eq!(
        h.queries()
            .iter()
            .filter(|sql| *sql == &request.sql)
            .count(),
        1
    );
    h.finish().await;
}

#[tokio::test]
async fn qualification_metadata_schema_switch_callback_precedes_use_dispatch() {
    let observer = Arc::new(Recorder::default());
    let h = harness(&observer).await;
    let options = h
        .driver
        .session_context_options("observer-session")
        .await
        .unwrap();
    *observer.reject_schema.lock() = Some(MetadataSite::SchemaSwitch);
    assert!(
        h.driver
            .switch_schema("observer-session", &switch_request(&options, "OTHER"))
            .await
            .is_err()
    );
    assert_eq!(
        observer.schemas.lock().last().unwrap().0,
        MetadataSite::SchemaSwitch
    );
    assert!(!h.queries().iter().any(|sql| sql.starts_with("USE SCHEMA")));
    h.finish().await;
}

struct PublicationBarrierObserver {
    final_snapshot: AtomicBool,
    deny: AtomicBool,
    reached: std::sync::mpsc::SyncSender<()>,
    resume: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl MetadataObserver for PublicationBarrierObserver {
    fn check(&self) -> AppResult<()> {
        if self.deny.load(Ordering::SeqCst) {
            return Err(denied());
        }
        if self.final_snapshot.swap(false, Ordering::SeqCst) {
            // This check has already accepted. The test changes policy while
            // publication is still behind a held adapter state mutex.
            self.reached.send(()).unwrap();
            self.resume
                .lock()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        Ok(())
    }

    fn observe_schemas(&self, _: SchemaObservation<'_>) -> AppResult<()> {
        Ok(())
    }

    fn observe_relations(&self, observation: RelationObservation<'_>) -> AppResult<()> {
        if observation.site == MetadataSite::RelationCatalogSnapshot {
            self.final_snapshot.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_metadata_failure_before_catalog_commit_prevents_snapshot_publication() {
    let (reached_tx, reached_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let observer = Arc::new(PublicationBarrierObserver {
        final_snapshot: AtomicBool::new(false),
        deny: AtomicBool::new(false),
        reached: reached_tx,
        resume: Mutex::new(resume_rx),
    });
    let h = Harness::open_policy_and_observer(
        SafeMode::Confirm,
        false,
        true,
        Some(MetadataAttachment::new(observer.clone())),
    )
    .await;
    let driver = h.driver.clone();
    let catalog = tokio::spawn(async move { driver.relation_catalog().await });
    reached_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    {
        let state = h.driver.browsing.lock();
        assert!(state.snapshot.is_none());
        observer.deny.store(true, Ordering::SeqCst);
        resume_tx.send(()).unwrap();
    }
    assert_eq!(
        catalog.await.unwrap().unwrap_err().code,
        ErrorCode::PolicyBlocked
    );
    assert!(h.driver.browsing.lock().snapshot.is_none());
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_metadata_failure_before_transaction_grant_commit_prevents_authority() {
    let (reached_tx, reached_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let observer = Arc::new(PublicationBarrierObserver {
        final_snapshot: AtomicBool::new(false),
        deny: AtomicBool::new(false),
        reached: reached_tx,
        resume: Mutex::new(resume_rx),
    });
    let h = Harness::open_policy_and_observer(
        SafeMode::Confirm,
        false,
        true,
        Some(MetadataAttachment::new(observer.clone())),
    )
    .await;
    let sample = h.driver.session_sample().await.unwrap();
    let generation = h.driver.revoke_transaction_grant();
    observer.final_snapshot.store(true, Ordering::SeqCst);
    let driver = h.driver.clone();
    let publication =
        tokio::spawn(async move { driver.publish_transaction_sample(&sample, &generation) });
    reached_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    {
        let state = h.driver.transaction_authority.lock();
        assert!(state.grant.is_none());
        observer.deny.store(true, Ordering::SeqCst);
        resume_tx.send(()).unwrap();
    }
    assert_eq!(
        publication.await.unwrap().unwrap_err().code,
        ErrorCode::PolicyBlocked
    );
    assert!(h.driver.transaction_authority.lock().grant.is_none());
    h.finish().await;
}

#[tokio::test]
async fn qualification_metadata_failure_before_read_return_prevents_success() {
    let (reached_tx, _) = std::sync::mpsc::sync_channel(1);
    let (_, resume_rx) = std::sync::mpsc::sync_channel(1);
    let observer = Arc::new(PublicationBarrierObserver {
        final_snapshot: AtomicBool::new(false),
        deny: AtomicBool::new(false),
        reached: reached_tx,
        resume: Mutex::new(resume_rx),
    });
    let h = Harness::open_policy_and_observer(
        SafeMode::Confirm,
        false,
        true,
        Some(MetadataAttachment::new(observer.clone())),
    )
    .await;
    let queries = h.queries().len();
    let error = h
        .driver
        .metadata_operation(async {
            observer.deny.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PolicyBlocked);
    assert_eq!(error.completion_invalidated, Some(true));
    assert_eq!(h.queries().len(), queries);
    h.finish().await;
}
