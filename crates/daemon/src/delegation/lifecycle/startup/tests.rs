use super::*;
use std::future::{Future, poll_fn};
use std::task::Poll;

fn identity(nonce: &str) -> WorkerProcessIdentity {
    WorkerProcessIdentity {
        launch_nonce: nonce.into(),
        handle_id: "held-process".into(),
        creation_identity: "creation".into(),
    }
}

async fn assert_pending<F: Future + Unpin>(future: &mut F) {
    poll_fn(|cx| match std::pin::Pin::new(&mut *future).poll(cx) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(_) => panic!("a second initializer bypassed the held root"),
    })
    .await;
}

#[tokio::test]
async fn only_initialization_is_serialized_and_an_old_receipt_cannot_release_a_new_owner() {
    let roots = StartupRoots::default();
    let cancel = CancellationToken::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let root = Path::new("/same-native-root");
    let first = roots.acquire(root, deadline, &cancel).await.unwrap();
    first.bind_process(&identity("first"));
    let mut second = Box::pin(roots.acquire(root, deadline, &cancel));
    assert_pending(&mut second).await;
    let unrelated = roots
        .acquire(Path::new("/different-native-root"), deadline, &cancel)
        .await
        .unwrap();
    drop(unrelated);
    first.initialized();
    let second = second.await.unwrap();
    second.bind_process(&identity("second"));
    first.initialized();
    let mut third = Box::pin(roots.acquire(root, deadline, &cancel));
    assert_pending(&mut third).await;
    assert_eq!(*lock(&second.root.process), Some(identity("second")));
    second.initialized();
    assert!(third.await.is_ok());
}

#[tokio::test]
async fn waiting_obeys_cancellation_and_deadline_without_discarding_the_active_initializer() {
    let roots = StartupRoots::default();
    let root = Path::new("/native-root");
    let cancel = CancellationToken::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let owner = roots.acquire(root, deadline, &cancel).await.unwrap();
    let waiting_cancel = CancellationToken::new();
    let mut waiting = Box::pin(roots.acquire(root, deadline, &waiting_cancel));
    assert_pending(&mut waiting).await;
    waiting_cancel.cancel();
    assert!(matches!(waiting.await, Err(DelegationErrorV1::Cancelled)));
    assert!(matches!(
        roots.acquire(root, Instant::now(), &cancel).await,
        Err(DelegationErrorV1::DeadlineExceeded)
    ));
    // A start abandoned before spawn needs no residual identity or recovery operation.
    drop(owner);
    assert!(roots.acquire(root, deadline, &cancel).await.is_ok());
}

#[tokio::test]
async fn uncertain_initialization_requires_verified_stop_of_the_exact_process() {
    let roots = StartupRoots::default();
    let root = Path::new("/native-root");
    let cancel = CancellationToken::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let owner = roots.acquire(root, deadline, &cancel).await.unwrap();
    let process = identity("first");
    owner.bind_process(&process);
    drop(owner);
    let mut evidence = WorkerStopEvidence {
        scope: WorkerStopScope::ProcessGroup,
        observation: WorkerObservation::Exited { code: None },
        scope_stopped: true,
        residual_unknown: false,
    };
    for wrong in [
        identity("another-launch"),
        WorkerProcessIdentity {
            handle_id: "another-held-object".into(),
            ..process.clone()
        },
        WorkerProcessIdentity {
            creation_identity: "reused-object".into(),
            ..process.clone()
        },
    ] {
        roots.stopped(&wrong, &evidence);
        assert!(matches!(
            roots.acquire(root, deadline, &cancel).await,
            Err(DelegationErrorV1::Busy)
        ));
    }
    evidence.residual_unknown = true;
    roots.stopped(&process, &evidence);
    evidence.residual_unknown = false;
    evidence.scope_stopped = false;
    roots.stopped(&process, &evidence);
    evidence.scope_stopped = true;
    evidence.observation = WorkerObservation::Running;
    roots.stopped(&process, &evidence);
    assert!(matches!(
        roots.acquire(root, deadline, &cancel).await,
        Err(DelegationErrorV1::Busy)
    ));
    evidence.observation = WorkerObservation::Exited { code: None };
    roots.stopped(&process, &evidence);
    assert!(roots.acquire(root, deadline, &cancel).await.is_ok());
}
