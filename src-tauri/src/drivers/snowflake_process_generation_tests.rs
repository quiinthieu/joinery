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

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires explicit JOINERY_TEST_NODE; local owned helper generation/reaping only"]
async fn retirement_observer_survives_last_supervisor_drop_and_cancelled_waiter() {
    let (supervisor, root) = actual_batch_supervisor("manual").await;
    let observer = supervisor.retirement_observer();
    let identity = supervisor.helper_generation();
    let owner = Arc::downgrade(&supervisor.inner);
    let cleanup_authority = Arc::downgrade(
        supervisor
            .inner
            .exited
            .authority
            .as_ref()
            .expect("owned cleanup authority"),
    );
    assert!(identity.same_helper(&observer.helper_generation()));
    assert!(observer.reaped_generation().is_none());
    assert!(observer.wait(Duration::ZERO).await.is_none());

    let waiting_observer = observer.clone();
    let waiter = tokio::spawn(async move { waiting_observer.wait(Duration::from_secs(60)).await });
    tokio::task::yield_now().await;
    assert!(
        !waiter.is_finished(),
        "live helper must not yield a reap proof"
    );
    waiter.abort();
    assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));

    supervisor.request_retirement();
    assert!(observer.reaped_generation().is_none());
    drop(supervisor);
    let proof = observer
        .wait(REAP_LIMIT)
        .await
        .expect("post-drop actual reap");
    wait_for_inner_drop(&owner).await;
    assert!(identity.matches_reaped(&proof));
    assert!(identity.matches_reaped(&observer.clone().wait(Duration::ZERO).await.unwrap()));
    assert_eq!(proof.generation(), identity.generation());
    assert!(
        cleanup_authority.upgrade().is_none(),
        "observer must not retain root-removal authority"
    );
    assert!(!root.exists());
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires explicit JOINERY_TEST_NODE; local owned helper generation/reaping only"]
async fn retirement_observers_keep_reused_uuid_physical_owners_distinct_after_drop() {
    let (first, first_root) = actual_batch_supervisor("manual").await;
    let (second, second_root) = actual_batch_supervisor("manual").await;
    let first_observer = first.retirement_observer();
    let second_observer = second.retirement_observer();
    let first_identity = first_observer.helper_generation();
    let second_identity = second_observer.helper_generation();
    let first_owner = Arc::downgrade(&first.inner);
    let second_owner = Arc::downgrade(&second.inner);
    assert_eq!(first_identity.generation(), second_identity.generation());

    drop(first);
    let first_proof = first_observer
        .wait(REAP_LIMIT)
        .await
        .expect("first child reap");
    wait_for_inner_drop(&first_owner).await;
    assert!(first_identity.matches_reaped(&first_proof));
    assert!(!second_identity.matches_reaped(&first_proof));
    assert!(!second.is_closed());
    assert!(second_observer.reaped_generation().is_none());

    drop(second);
    let second_proof = second_observer
        .wait(REAP_LIMIT)
        .await
        .expect("second child reap");
    wait_for_inner_drop(&second_owner).await;
    assert!(second_identity.matches_reaped(&second_proof));
    assert!(!first_identity.matches_reaped(&second_proof));
    assert!(!first_root.exists() && !second_root.exists());
}

fn observer_fixture(
    retired: bool,
    receipt: Option<CleanupReport>,
) -> (
    Supervisor,
    watch::Sender<bool>,
    watch::Sender<Option<CleanupReport>>,
) {
    let (mut inner, _normal, _control) = isolated();
    let (retired_sender, _retired) = watch::channel(retired);
    let (cleanup_sender, cleanup) = watch::channel(receipt);
    let inner_mut = Arc::get_mut(&mut inner).unwrap();
    inner_mut.stop = retired_sender.clone();
    inner_mut.exited = CleanupObserver {
        receiver: cleanup,
        authority: None,
    };
    // Internal watch guard tests control publication independently. Marking
    // State already retired prevents Supervisor::drop from injecting updates.
    // Actual-child tests above prove the production order and source of facts.
    inner_mut.state.get_mut().unwrap().retired = Some(Retirement::Requested);
    (
        Supervisor {
            inner,
            pid: 0,
            bounds: Bounds::default(),
        },
        retired_sender,
        cleanup_sender,
    )
}

fn observer_receipt(child_started: bool, reaped: bool) -> CleanupReport {
    CleanupReport {
        child_started,
        reaped,
        temporary_root_removed: false,
    }
}

#[tokio::test]
async fn observer_requires_private_retirement_and_both_reaper_flags() {
    for (retired, cleanup) in [
        (false, Some(observer_receipt(true, true))),
        (true, None),
        (true, Some(observer_receipt(false, false))),
        (true, Some(observer_receipt(false, true))),
        (true, Some(observer_receipt(true, false))),
    ] {
        let (supervisor, _retired, _cleanup) = observer_fixture(retired, cleanup);
        let observer = supervisor.retirement_observer();
        assert!(observer.reaped_generation().is_none());
        assert!(observer.wait(Duration::ZERO).await.is_none());
    }
    let (supervisor, _retired, _cleanup) =
        observer_fixture(true, Some(observer_receipt(true, true)));
    let observer = supervisor.retirement_observer();
    assert!(observer.reaped_generation().is_some());
    assert!(observer.wait(Duration::ZERO).await.is_some());
}

#[tokio::test]
async fn closed_channels_and_dead_identity_without_reap_never_grant_proof() {
    for (retired, cleanup) in [
        (false, Some(observer_receipt(true, true))),
        (true, None),
        (true, Some(observer_receipt(true, false))),
        (true, Some(observer_receipt(false, true))),
    ] {
        let (supervisor, retired_sender, cleanup_sender) = observer_fixture(retired, cleanup);
        let observer = supervisor.retirement_observer();
        let owner = Arc::downgrade(&supervisor.inner);
        drop(supervisor);
        drop(retired_sender);
        drop(cleanup_sender);
        assert!(owner.upgrade().is_none());
        assert!(observer.wait(Duration::from_secs(1)).await.is_none());
        assert!(observer.reaped_generation().is_none());
    }
}

#[tokio::test]
async fn closed_true_retirement_watch_allows_later_private_cleanup() {
    let (supervisor, retired_sender, cleanup_sender) = observer_fixture(true, None);
    let observer = supervisor.retirement_observer();
    drop(supervisor);
    drop(retired_sender);
    let mut waiting = Box::pin(observer.wait(Duration::from_secs(1)));
    assert!(futures_util::poll!(waiting.as_mut()).is_pending());
    cleanup_sender.send_replace(Some(observer_receipt(true, true)));
    assert!(waiting.await.is_some());
    drop(cleanup_sender);
    assert!(observer.wait(Duration::ZERO).await.is_some());
}

#[tokio::test]
async fn closed_true_cleanup_watch_allows_later_private_retirement() {
    // Production retires before publishing cleanup. The observer nevertheless
    // retains either final fact safely if private publication ordering changes.
    let (supervisor, retired_sender, cleanup_sender) =
        observer_fixture(false, Some(observer_receipt(true, true)));
    let observer = supervisor.retirement_observer();
    drop(supervisor);
    drop(cleanup_sender);
    let mut waiting = Box::pin(observer.wait(Duration::from_secs(1)));
    assert!(futures_util::poll!(waiting.as_mut()).is_pending());
    retired_sender.send_replace(true);
    assert!(waiting.await.is_some());
    drop(retired_sender);
    assert!(observer.wait(Duration::ZERO).await.is_some());
}
