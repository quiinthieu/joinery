//! Qualification attachment through the actual Studio/helper path with a
//! synthetic SDK. No account activity, execution approval or physical SQL ledger.
use super::*;
use crate::drivers::{
    snowflake_qualification::QualificationAttachment,
    snowflake_qualification_metadata::{MetadataObserver, RelationObservation, SchemaObservation},
    snowflake_qualification_transport::{PhysicalRequest, TransportLimits, TransportPolicy},
};
use std::{future::Future, pin::Pin};

#[derive(Default)]
struct Policy {
    denied_operation: Option<&'static str>,
    deny_schema: bool,
    operations: Mutex<Vec<String>>,
    schemas: Mutex<Vec<Vec<String>>>,
    relations: Mutex<usize>,
    revoked: AtomicBool,
}

impl TransportPolicy for Policy {
    fn check(&self) -> AppResult<()> {
        if self.revoked.load(Ordering::SeqCst) {
            Err(denied())
        } else {
            Ok(())
        }
    }

    fn admit<'a>(
        &'a self,
        request: PhysicalRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.operations.lock().push(request.operation.to_owned());
            if self.denied_operation == Some(request.operation) {
                return Err(denied());
            }
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_rechecks_rejection_after_setup_before_publication() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let policy = Arc::new(Policy::default());
    let mut attachment =
        QualificationAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let transport = attachment.transport.clone();
    let (setup_finished, setup_receiver) = std::sync::mpsc::sync_channel(0);
    let (resume, resume_receiver) = std::sync::mpsc::sync_channel(0);
    let resume_receiver = std::sync::Mutex::new(resume_receiver);
    attachment.before_publication = Some(Arc::new(move || {
        setup_finished.send(()).unwrap();
        resume_receiver
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
    }));
    let rejection = std::thread::spawn({
        let studio = studio.clone();
        let transport = transport.clone();
        move || {
            // Runtime verification precedes the hook and has its own lifetime.
            // An early connection error drops every sender and wakes this wait.
            setup_receiver.recv().unwrap();
            let _configuration = studio.configuration_gate.lock();
            policy.revoked.store(true, Ordering::SeqCst);
            assert_eq!(transport.check().unwrap_err().message, denied().message);
            resume.send(()).unwrap();
        }
    });
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    let result = studio
        .connect_prepared_qualification(&attempt, ConnectionSecrets::default(), attachment)
        .await;
    assert!(
        rejection.join().is_ok(),
        "publication rejection coordination failed; connection result: {result:?}; transport: {:?}",
        transport.report()
    );
    let published = result.is_ok();
    if let Ok(info) = result {
        // Keep a red regression from leaking its successfully published child.
        studio.disconnect_and_wait(&info.id).await.unwrap();
    }
    assert!(transport.wait(Duration::from_secs(10)).await.is_some());
    no_roots(&fixture);
    assert!(
        !published,
        "a rejected qualification must not publish a session"
    );
    assert!(studio.sessions.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qualification_studio_rechecks_helper_exit_before_publication() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    let mut attachment =
        QualificationAttachment::new(Arc::new(Policy::default()), TransportLimits::default())
            .unwrap();
    let transport = attachment.transport.clone();
    let (setup_finished, setup_receiver) = std::sync::mpsc::sync_channel(0);
    let (resume, resume_receiver) = std::sync::mpsc::sync_channel(0);
    let resume_receiver = std::sync::Mutex::new(resume_receiver);
    attachment.before_publication = Some(Arc::new(move || {
        setup_finished.send(()).unwrap();
        resume_receiver
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(15))
            .unwrap();
    }));
    let reaper = std::thread::spawn({
        let studio = studio.clone();
        let runtime = tokio::runtime::Handle::current();
        let attempt = attempt.clone();
        let authentication = fixture.controls.join("auth-started");
        move || {
            // Wait for setup's lifetime, not a separate wall-clock deadline that
            // can expire while the real Node binary is synchronously verified.
            // Early setup failure drops the captured sender and disconnects us.
            setup_receiver.recv().unwrap();
            let _configuration = studio.configuration_gate.lock();
            let marker: serde_json::Value =
                serde_json::from_slice(&std::fs::read(authentication).unwrap()).unwrap();
            let pid = marker["pid"].as_u64().unwrap();
            // This PID belongs to this test's disposable helper, never a user process.
            assert!(
                std::process::Command::new("/bin/kill")
                    .args(["-TERM", &pid.to_string()])
                    .status()
                    .unwrap()
                    .success()
            );
            runtime
                .block_on(
                    studio
                        .snowflake_cleanup
                        .wait_for(&attempt, Duration::from_secs(10)),
                )
                .unwrap();
            resume.send(()).unwrap();
        }
    });
    let result = studio
        .connect_prepared_qualification(&attempt, ConnectionSecrets::default(), attachment)
        .await;
    assert!(
        reaper.join().is_ok(),
        "helper exit coordination failed; connection result: {result:?}; transport: {:?}",
        transport.report()
    );
    let published = result.is_ok();
    if let Ok(info) = result {
        studio.disconnect_and_wait(&info.id).await.unwrap();
    }
    assert!(transport.wait(Duration::from_secs(10)).await.is_some());
    no_roots(&fixture);
    assert!(!published, "an exited helper must not publish a session");
    assert!(studio.sessions.lock().is_empty());
}

impl MetadataObserver for Policy {
    fn check(&self) -> AppResult<()> {
        Ok(())
    }

    fn observe_schemas(&self, observation: SchemaObservation<'_>) -> AppResult<()> {
        self.schemas.lock().push(observation.wire_names.to_vec());
        if self.deny_schema {
            return Err(denied());
        }
        Ok(())
    }

    fn observe_relations(&self, _observation: RelationObservation<'_>) -> AppResult<()> {
        *self.relations.lock() += 1;
        Ok(())
    }
}

fn denied() -> AppError {
    AppError::new(
        ErrorCode::PolicyBlocked,
        "Synthetic qualification rejection",
    )
}

#[tokio::test]
async fn qualification_studio_attaches_both_domains_before_startup_and_drains_after_close() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let policy = Arc::new(Policy::default());
    let attachment =
        QualificationAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let transport = attachment.transport.clone();
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    let info = studio
        .connect_prepared_qualification(&attempt, ConnectionSecrets::default(), attachment)
        .await
        .unwrap();
    assert_eq!(queries(&fixture).len(), 2);
    assert_eq!(&policy.operations.lock()[..2], ["connect", "context"]);
    assert!(policy.schemas.lock().is_empty());
    let catalog = studio.relation_catalog(&info.id).await.unwrap();
    assert!(!catalog.relations.is_empty());
    assert!(!policy.schemas.lock().is_empty());
    assert!(*policy.relations.lock() > 0);
    assert!(transport.first_error().is_none());
    studio.disconnect_and_wait(&info.id).await.unwrap();
    assert!(transport.wait(Duration::from_secs(10)).await.is_some());
    no_roots(&fixture);
}

#[tokio::test]
async fn qualification_studio_denies_connect_before_synthetic_authentication() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let policy = Arc::new(Policy {
        denied_operation: Some("connect"),
        ..Policy::default()
    });
    let attachment =
        QualificationAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let transport = attachment.transport.clone();
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    assert!(
        studio
            .connect_prepared_qualification(&attempt, ConnectionSecrets::default(), attachment)
            .await
            .is_err()
    );
    assert_eq!(&*policy.operations.lock(), &["connect"]);
    assert!(!fixture.controls.join("auth-started").exists());
    assert!(!fixture.controls.join("queries").exists());
    assert!(studio.sessions.lock().is_empty());
    assert_eq!(transport.first_error().unwrap().message, denied().message);
    assert!(transport.wait(Duration::from_secs(10)).await.is_some());
    no_roots(&fixture);
}

#[tokio::test]
async fn qualification_studio_context_denial_prevents_hidden_identity_sql_and_publication() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let policy = Arc::new(Policy {
        denied_operation: Some("context"),
        ..Policy::default()
    });
    let attachment =
        QualificationAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let transport = attachment.transport.clone();
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    assert!(
        studio
            .connect_prepared_qualification(&attempt, ConnectionSecrets::default(), attachment)
            .await
            .is_err()
    );
    assert_eq!(&*policy.operations.lock(), &["connect", "context"]);
    assert!(fixture.controls.join("auth-started").exists());
    assert!(!fixture.controls.join("queries").exists());
    assert!(studio.sessions.lock().is_empty());
    assert!(transport.wait(Duration::from_secs(10)).await.is_some());
    no_roots(&fixture);
}

#[tokio::test]
async fn qualification_studio_pipe_limit_revokes_metadata_and_reaps_the_actual_helper() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let sdk = fixture
        .controls
        .join("node_modules/snowflake-sdk/index.mjs");
    let source = std::fs::read_to_string(&sdk).unwrap();
    assert_eq!(source.matches("event('create');").count(), 1);
    std::fs::write(
        sdk,
        format!(
            "import {{ writeSync }} from 'node:fs';\n{}",
            source.replace(
                "event('create');",
                "writeSync(2, Buffer.alloc(128, 'x')); event('create');",
            )
        ),
    )
    .unwrap();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let policy = Arc::new(Policy::default());
    let limits = TransportLimits {
        stderr_bytes: 32,
        ..TransportLimits::default()
    };
    let attachment = QualificationAttachment::new(policy, limits).unwrap();
    let transport = attachment.transport.clone();
    let metadata = attachment.metadata.clone();
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    let result = studio
        .connect_prepared_qualification(&attempt, ConnectionSecrets::default(), attachment)
        .await;
    let report = transport.wait(Duration::from_secs(10)).await.unwrap();
    if let Ok(info) = result {
        // Independent stderr draining may observe the overflow just after setup.
        // It must still revoke metadata, retire/reap, and permit explicit removal.
        studio.disconnect_and_wait(&info.id).await.unwrap();
    }
    assert!(report.failed);
    assert_eq!(report.stderr_bytes, 33);
    assert_eq!(
        report.stderr,
        crate::drivers::snowflake_qualification_transport::Disposition::Limit
    );
    let failure = transport.first_error().unwrap();
    assert_eq!(metadata.check().unwrap_err().message, failure.message);
    assert!(studio.sessions.lock().is_empty());
    no_roots(&fixture);
}

#[tokio::test]
async fn qualification_studio_schema_rejection_latches_transport_and_prevents_more_sql() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let profile = studio.save_profile(profile(), None).unwrap();
    let policy = Arc::new(Policy {
        deny_schema: true,
        ..Policy::default()
    });
    let attachment =
        QualificationAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let transport = attachment.transport.clone();
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    let info = studio
        .connect_prepared_qualification(&attempt, ConnectionSecrets::default(), attachment)
        .await
        .unwrap();
    assert!(studio.relation_catalog(&info.id).await.is_err());
    assert_eq!(policy.schemas.lock().len(), 1);
    assert_eq!(*policy.relations.lock(), 0);
    let before = queries(&fixture);
    assert_eq!(transport.check().unwrap_err().message, denied().message);
    assert!(studio.relation_catalog(&info.id).await.is_err());
    assert_eq!(queries(&fixture), before);
    studio.disconnect_and_wait(&info.id).await.unwrap();
    assert!(transport.wait(Duration::from_secs(10)).await.is_some());
    no_roots(&fixture);
}

#[tokio::test]
async fn qualification_studio_rejects_other_engine_before_launch() {
    let _fixture_lane = FIXTURE_LANE.lock().await;
    let fixture = Fixture::new();
    let studio = studio(&fixture);
    let mut other = profile();
    other.engine = EngineKind::Postgres;
    other.host = "127.0.0.1".into();
    other.port = 5432;
    other.snowflake = None;
    let profile = studio.save_profile(other, None).unwrap();
    let policy = Arc::new(Policy::default());
    let attachment =
        QualificationAttachment::new(policy.clone(), TransportLimits::default()).unwrap();
    let transport = attachment.transport.clone();
    let attempt = studio.prepare_connection(&profile.id).unwrap();
    let error = studio
        .connect_prepared_qualification(&attempt, ConnectionSecrets::default(), attachment)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PolicyBlocked);
    assert!(policy.operations.lock().is_empty());
    assert!(!fixture.controls.join("sdk-events").exists());
    assert!(!fixture.controls.join("auth-started").exists());
    assert!(studio.sessions.lock().is_empty());
    let report = transport.wait(Duration::from_secs(1)).await.unwrap();
    assert_eq!(report.stdout_bytes, 0);
    assert_eq!(report.stderr_bytes, 0);
    assert_eq!(report.admitted_writes, 0);
    assert_eq!(report.outbound_bytes, 0);
    no_roots(&fixture);
}
