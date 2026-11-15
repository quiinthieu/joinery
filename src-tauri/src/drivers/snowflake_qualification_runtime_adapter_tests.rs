use super::*;
use crate::drivers::snowflake_qualification_metadata::MetadataObserver;

#[derive(Default)]
struct RuntimeRecorder {
    states: Mutex<Vec<SessionRuntimeState>>,
    deny: AtomicBool,
}

impl MetadataObserver for RuntimeRecorder {
    fn check(&self) -> AppResult<()> {
        Ok(())
    }
    fn observe_schemas(&self, _: SchemaObservation<'_>) -> AppResult<()> {
        Ok(())
    }
    fn observe_relations(&self, _: RelationObservation<'_>) -> AppResult<()> {
        Ok(())
    }
    fn observe_runtime(&self, state: &SessionRuntimeState) -> AppResult<()> {
        self.states.lock().push(state.clone());
        if self.deny.load(Ordering::SeqCst) {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "synthetic runtime denial",
            ));
        }
        Ok(())
    }
}

async fn runtime_harness(observer: &Arc<RuntimeRecorder>) -> Harness {
    Harness::open_policy_and_observer(
        SafeMode::Confirm,
        true,
        true,
        Some(MetadataAttachment::new(observer.clone())),
    )
    .await
}

fn assert_preflight_budget(queries: &[String]) {
    assert_eq!(queries.len(), 12);
    assert_eq!(queries[..5], sample_sql());
    assert!(queries[5].contains(".INFORMATION_SCHEMA.SCHEMATA "));
    assert!(queries[6].contains(".INFORMATION_SCHEMA.TABLES "));
    assert_eq!(queries[7..], sample_sql());
}

#[tokio::test]
async fn qualification_runtime_preflight_observes_actual_authority_without_extra_sql() {
    let observer = Arc::new(RuntimeRecorder::default());
    let h = runtime_harness(&observer).await;
    h.settings(json!({"emptyRelations":true}));
    let baseline = h.queries().len();
    let request = h.prepare_create_ddl().await;
    assert_preflight_budget(&h.queries()[baseline..]);
    {
        let states = observer.states.lock();
        assert_eq!(states.len(), 1);
        let state = &states[0];
        assert_eq!(state.session_id, "observer-session");
        assert_eq!(state.database, "DB");
        assert_eq!(state.active_context.as_deref(), Some("PUBLIC"));
        assert_eq!(state.transaction_mode, TransactionMode::AutoCommit);
        assert_eq!(state.transaction_state, TransactionState::Idle);
        assert!(matches!(
            &state.transaction_observation,
            Some(TransactionObservation::Observed {
                transaction_id: None,
                abort_on_error: false,
                ..
            })
        ));
        let TransactionControlAuthority::Observed {
            revision,
            allowed_actions,
        } = state.transaction_control_authority.as_ref().unwrap()
        else {
            panic!("observed authority required")
        };
        assert_eq!(
            allowed_actions,
            &[
                ExplicitTransactionAction::Begin,
                ExplicitTransactionAction::Rollback
            ]
        );
        assert_eq!(
            &h.driver
                .transaction_authority
                .lock()
                .grant
                .as_ref()
                .unwrap()
                .revision,
            revision
        );
    }
    h.driver.validate_snowflake_ddl(&request).unwrap();
    h.finish().await;
}

#[tokio::test]
async fn qualification_runtime_denial_blocks_preflight_publication_and_latches_without_sql() {
    let observer = Arc::new(RuntimeRecorder::default());
    let h = runtime_harness(&observer).await;
    h.settings(json!({"emptyRelations":true}));
    observer.deny.store(true, Ordering::SeqCst);
    let baseline = h.queries().len();
    let action = SnowflakeDdlAction::CreateTable {
        database: "DB".into(),
        schema: "PUBLIC".into(),
        name: "NEW_ORDERS".into(),
        columns: vec![SnowflakeDdlColumnDraft {
            name: "ID".into(),
            data_type: "NUMBER(38,0)".into(),
            nullable: true,
        }],
    };
    let error = h
        .driver
        .preflight_snowflake_ddl(
            "observer-session",
            &SnowflakeDdlPreflightRequest {
                session_id: "observer-session".into(),
                action,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PolicyBlocked);
    assert_preflight_budget(&h.queries()[baseline..]);
    assert_eq!(observer.states.lock().len(), 1);
    assert!(h.driver.transaction_authority.lock().grant.is_none());
    assert!(h.driver.ddl_authority.lock().grant.is_none());
    observer.deny.store(false, Ordering::SeqCst);
    let baseline = h.queries().len();
    // The independent state observer intentionally returns unavailable state
    // for a policy failure without replacing an earlier statement outcome.
    let state = h.driver.session_state("observer-session").await.unwrap();
    assert_eq!(state.transaction_state, TransactionState::Unknown);
    assert_eq!(state.transaction_mode, TransactionMode::Unknown);
    assert!(state.transaction_control_authority.is_none());
    assert!(matches!(
        state.transaction_observation,
        Some(TransactionObservation::Unavailable)
    ));
    assert_eq!(h.queries().len(), baseline);
    h.finish().await;
}

#[tokio::test]
async fn qualification_runtime_denial_retains_original_ddl_dml_and_transaction_acks() {
    for operation in ["ddl", "dml", "transaction"] {
        let observer = Arc::new(RuntimeRecorder::default());
        let h = runtime_harness(&observer).await;
        h.settings(json!({"emptyRelations":true}));
        let ddl = if operation == "ddl" {
            Some(h.prepare_create_ddl().await)
        } else {
            None
        };
        let state = if operation != "ddl" {
            Some(h.driver.session_state("observer-session").await.unwrap())
        } else {
            None
        };
        observer.deny.store(true, Ordering::SeqCst);
        let baseline = h.queries().len();
        match operation {
            "ddl" => {
                let result = h
                    .driver
                    .snowflake_ddl("observer-session", ddl.as_ref().unwrap())
                    .await
                    .unwrap();
                assert_eq!(result.statement_outcome, SnowflakeDdlOutcome::Succeeded);
                assert_eq!(
                    result.summary_status,
                    SnowflakeDdlSummaryStatus::Acknowledged
                );
                assert!(result.retired);
                assert_eq!(result.error.unwrap().code, ErrorCode::ConnectionClosed);
                assert_eq!(result.state.transaction_state, TransactionState::Unknown);
                assert_eq!(h.queries().len() - baseline, 18); // 5 + target checks2 + final pre5 + DDL1 + post5
            }
            "dml" => {
                let request = dml_request(
                    state.as_ref().unwrap(),
                    "DELETE FROM DB.PUBLIC.T WHERE V = 1",
                );
                let result = h
                    .driver
                    .snowflake_dml("observer-session", &request)
                    .await
                    .unwrap();
                assert_eq!(result.statement_outcome, SnowflakeDmlOutcome::Succeeded);
                assert_eq!(
                    result.summary.affected_rows.as_deref(),
                    Some("9007199254740993")
                );
                assert!(result.retired);
                assert_eq!(result.error.unwrap().code, ErrorCode::ConnectionClosed);
                assert_eq!(result.state.transaction_state, TransactionState::Unknown);
                assert_eq!(h.queries().len() - baseline, 11);
            }
            "transaction" => {
                let request =
                    transaction_request(state.as_ref().unwrap(), ExplicitTransactionAction::Begin);
                let result = h
                    .driver
                    .transaction_control("observer-session", &request)
                    .await
                    .unwrap();
                assert_eq!(
                    result.boundary_outcome,
                    TransactionBoundaryOutcome::Acknowledged
                );
                assert!(result.retired);
                assert_eq!(result.error.unwrap().code, ErrorCode::ConnectionClosed);
                assert_eq!(result.state.transaction_state, TransactionState::Unknown);
                assert_eq!(h.queries().len() - baseline, 11);
            }
            _ => unreachable!(),
        }
        assert_eq!(observer.states.lock().len(), 2);
        assert!(h.driver.is_closed());
        assert!(h.driver.transaction_authority.lock().grant.is_none());
        h.finish().await;
    }
}
