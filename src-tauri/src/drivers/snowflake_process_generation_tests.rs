use super::*;

const REAP_LIMIT: Duration = Duration::from_secs(5);

async fn wait_for_inner_drop(owner: &std::sync::Weak<Inner>) {
    timeout(REAP_LIMIT, async {
        while owner.strong_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("generation handles must not retain the supervisor or child tasks");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires explicit JOINERY_TEST_NODE; local owned helper generation/reaping only"]
async fn helper_generation_requires_actual_reaping_after_logical_retirement() {
    let (supervisor, root) = actual_batch_supervisor("manual").await;
    let identity = supervisor.helper_generation();
    let cleanup = supervisor.cleanup_observer();
    assert_eq!(
        identity.generation(),
        "00000000-0000-4000-8000-000000000001"
    );
    assert!(!supervisor.is_closed());
    assert!(cleanup.report().is_none());
    assert!(supervisor.reaped_generation().is_none());

    supervisor.request_retirement();
    assert!(supervisor.is_closed());
    // This current-thread task has not yielded since retirement. The actual
    // child wait and private receipt publication must still run independently.
    assert!(cleanup.report().is_none());
    assert!(supervisor.reaped_generation().is_none());

    let receipt = cleanup.wait(REAP_LIMIT).await.expect("owned child reap");
    assert!(receipt.child_started && receipt.reaped);
    let reaped = supervisor.reaped_generation().expect("actual reap proof");
    assert_eq!(reaped.generation(), identity.generation());
    assert!(identity.matches_reaped(&reaped));
    assert!(!root.exists());
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires explicit JOINERY_TEST_NODE; local owned helper generation/reaping only"]
async fn identical_wire_generations_cannot_prove_another_physical_helper_reaped() {
    let (first, first_root) = actual_batch_supervisor("manual").await;
    let (second, second_root) = actual_batch_supervisor("manual").await;
    let first_identity = first.helper_generation();
    let second_identity = second.helper_generation();
    assert_eq!(first_identity.generation(), second_identity.generation());
    assert!(first_identity.same_helper(&first_identity.clone()));
    assert!(first_identity.same_helper(&first.helper_generation()));
    assert!(!first_identity.same_helper(&second_identity));
    assert!(!second_identity.same_helper(&first_identity));

    let first_shutdown = first.shutdown().await;
    assert!(
        first_shutdown
            .cleanup
            .is_some_and(|r| r.child_started && r.reaped)
    );
    let first_reaped = first.reaped_generation().expect("first actual reap");
    assert!(first_identity.matches_reaped(&first_reaped));
    assert!(!second_identity.matches_reaped(&first_reaped));
    assert!(!second.is_closed());
    assert!(second.reaped_generation().is_none());

    let second_shutdown = second.shutdown().await;
    assert!(
        second_shutdown
            .cleanup
            .is_some_and(|r| r.child_started && r.reaped)
    );
    let second_reaped = second.reaped_generation().expect("second actual reap");
    assert!(second_identity.matches_reaped(&second_reaped));
    assert!(!first_identity.matches_reaped(&second_reaped));
    assert!(!first_root.exists() && !second_root.exists());
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires explicit JOINERY_TEST_NODE; local owned helper generation/reaping only"]
async fn retained_generation_handles_and_proofs_do_not_retain_the_helper() {
    // A stale identity cannot prevent Drop from retiring and reaping its child.
    let (supervisor, root) = actual_batch_supervisor("manual").await;
    let identity = supervisor.helper_generation();
    let owner = Arc::downgrade(&supervisor.inner);
    let cleanup = supervisor.cleanup_observer();
    drop(supervisor);
    let receipt = cleanup.wait(REAP_LIMIT).await.expect("dropped owner reap");
    assert!(receipt.child_started && receipt.reaped);
    wait_for_inner_drop(&owner).await;
    assert_eq!(
        identity.generation(),
        "00000000-0000-4000-8000-000000000001"
    );
    assert!(!root.exists());

    // A retained proof also carries only allocation identity, not process life.
    let (supervisor, root) = actual_batch_supervisor("manual").await;
    let identity = supervisor.helper_generation();
    let owner = Arc::downgrade(&supervisor.inner);
    supervisor.shutdown().await;
    let proof = supervisor.reaped_generation().expect("shutdown reap proof");
    let proof_clone = proof.clone();
    drop(supervisor);
    wait_for_inner_drop(&owner).await;
    assert!(identity.matches_reaped(&proof));
    assert!(identity.clone().matches_reaped(&proof_clone));
    assert!(!root.exists());
}

#[test]
fn helper_generation_canonicalizes_uuid_without_changing_wire_generation() {
    let (mut inner, _normal, _control) = isolated();
    Arc::get_mut(&mut inner).unwrap().generation = "ABCDEFAB-1234-4000-8000-ABCDEFABCDEF".into();
    let supervisor = Supervisor {
        inner,
        pid: 0,
        bounds: Bounds::default(),
    };
    let identity = supervisor.helper_generation();
    assert_eq!(
        identity.generation(),
        "abcdefab-1234-4000-8000-abcdefabcdef"
    );
    assert_eq!(
        supervisor.inner.generation,
        "ABCDEFAB-1234-4000-8000-ABCDEFABCDEF"
    );
    assert!(supervisor.reaped_generation().is_none());
}

#[test]
fn reaped_generation_requires_sealed_admission_and_both_private_reaper_flags() {
    for (sealed, child_started, reaped) in [
        (false, true, true),
        (true, false, false),
        (true, false, true),
        (true, true, false),
    ] {
        let (mut inner, _normal, _control) = isolated();
        let (_updates, receiver) = watch::channel(Some(CleanupReport {
            child_started,
            reaped,
            temporary_root_removed: true,
        }));
        Arc::get_mut(&mut inner).unwrap().exited = CleanupObserver {
            receiver,
            authority: None,
        };
        inner.state.lock().unwrap().closing = sealed;
        let supervisor = Supervisor {
            inner,
            pid: 0,
            bounds: Bounds::default(),
        };
        assert!(
            supervisor.reaped_generation().is_none(),
            "sealed={sealed}, child_started={child_started}, reaped={reaped}"
        );
    }
}
