use crate::fixture::*;

#[tokio::test]
async fn executor_admission_is_bounded_before_payload_copy_and_timing_is_split() {
    let executor = BoundedExecutor::new(ExecutorKind::SidecallIo, 1, 1).unwrap();
    let first = executor.try_admit(8 * 1024 * 1024).unwrap();
    let copied = AtomicUsize::new(0);
    if executor.try_admit(8 * 1024 * 1024).is_ok() {
        copied.fetch_add(8 * 1024 * 1024, Ordering::Relaxed);
    }
    assert_eq!(copied.load(Ordering::Relaxed), 0);
    assert_eq!(executor.snapshot().overloaded, 1);

    let scope = ChildScope::new(Instant::now() + Duration::from_secs(1));
    let completion = scope
        .run(&executor, first, async {
            tokio::time::sleep(Duration::from_millis(2)).await;
            42
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(3)).await;
    let (value, timing) = scope.resume(&executor, completion).unwrap();
    assert_eq!(value, 42);
    assert!(timing.scheduler_resume_lag >= Duration::from_millis(2));
    scope
        .cancel_and_finalize(Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(scope.active_children(), 0);
}

#[tokio::test]
async fn finalized_child_scope_drops_late_result_without_second_resume() {
    let executor = BoundedExecutor::new(ExecutorKind::Compute, 1, 2).unwrap();
    let scope = ChildScope::new(Instant::now() + Duration::from_secs(1));
    let completion = scope
        .run_offloaded(&executor, executor.try_admit(1).unwrap(), || "done")
        .await
        .unwrap();
    scope
        .cancel_and_finalize(Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(
        scope.resume(&executor, completion).unwrap_err(),
        ExecutorError::ResultDropped
    );
    assert_eq!(executor.snapshot().late_dropped, 1);
}

#[tokio::test]
async fn fixed_executor_contains_job_panic_and_reuses_the_same_worker() {
    let executor = BoundedExecutor::new(ExecutorKind::Compute, 1, 2).unwrap();
    let scope = ChildScope::new(Instant::now() + Duration::from_secs(1));

    let error = scope
        .run_offloaded(&executor, executor.try_admit(1).unwrap(), || -> usize {
            panic!("isolated compute failure")
        })
        .await
        .unwrap_err();
    assert_eq!(error, ExecutorError::JobPanicked);
    assert_eq!(scope.active_children(), 0);
    assert_eq!(executor.snapshot().running, 0);
    assert_eq!(executor.snapshot().panicked, 1);

    let completion = scope
        .run_offloaded(&executor, executor.try_admit(1).unwrap(), || 42_usize)
        .await
        .expect("the sole fixed worker must survive the preceding panic");
    let (value, _) = scope.resume(&executor, completion).unwrap();
    assert_eq!(value, 42);
    scope
        .cancel_and_finalize(Duration::from_millis(50))
        .await
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn fixed_blocking_backend_keeps_child_active_until_late_job_really_exits() {
    let executor = BoundedExecutor::new(ExecutorKind::BlockingControl, 1, 1).unwrap();
    let scope = ChildScope::new(Instant::now() + Duration::from_secs(1));
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let job_scope = scope.clone();
    let job_executor = executor.clone();
    let job_started = Arc::clone(&started);
    let job_release = Arc::clone(&release);
    let task = tokio::spawn(async move {
        job_scope
            .run_offloaded(
                &job_executor,
                job_executor.try_admit(1).unwrap(),
                move || {
                    job_started.store(true, Ordering::Release);
                    while !job_release.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    7_u8
                },
            )
            .await
    });
    while !started.load(Ordering::Acquire) {
        tokio::task::yield_now().await;
    }

    scope.cancel();
    assert_eq!(
        scope
            .cancel_and_finalize(Duration::from_millis(10))
            .await
            .unwrap_err(),
        ExecutorError::JoinTimeout,
        "cancellation must not pretend the still-running blocking job joined"
    );
    assert_eq!(scope.active_children(), 1);
    release.store(true, Ordering::Release);
    assert_eq!(task.await.unwrap().unwrap_err(), ExecutorError::Cancelled);
    scope
        .cancel_and_finalize(Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(scope.active_children(), 0);
    assert_eq!(executor.snapshot().late_dropped, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_blocking_result_cleanup_finishes_before_scope_join() {
    struct HeldResult {
        dropping: Option<tokio::sync::oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl Drop for HeldResult {
        fn drop(&mut self) {
            if let Some(dropping) = self.dropping.take() {
                let _ = dropping.send(());
            }
            let _ = self.release.recv();
        }
    }

    let executor = BoundedExecutor::new(ExecutorKind::BlockingControl, 1, 1).unwrap();
    let scope = ChildScope::new(Instant::now() + Duration::from_secs(1));
    let (started, job_started) = tokio::sync::oneshot::channel();
    let (release_job, job_release) = std::sync::mpsc::channel();
    let (dropping, result_dropping) = tokio::sync::oneshot::channel();
    let (release_result, result_release) = std::sync::mpsc::channel();
    let job_scope = scope.clone();
    let job_executor = executor.clone();
    let task = tokio::spawn(async move {
        job_scope
            .run_offloaded(
                &job_executor,
                job_executor.try_admit(1).unwrap(),
                move || {
                    let _ = started.send(());
                    let _ = job_release.recv();
                    HeldResult {
                        dropping: Some(dropping),
                        release: result_release,
                    }
                },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), job_started)
        .await
        .unwrap()
        .unwrap();
    scope.cancel();
    assert!(matches!(task.await.unwrap(), Err(ExecutorError::Cancelled)));
    release_job.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), result_dropping)
        .await
        .unwrap()
        .unwrap();

    let active_during_cleanup = scope.active_children();
    let join_during_cleanup = scope.cancel_and_finalize(Duration::from_millis(10)).await;
    // Release the owned worker before asserting, so a regression cannot strand it.
    release_result.send(()).unwrap();
    scope
        .cancel_and_finalize(Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(
        active_during_cleanup, 1,
        "discarded result still owns cleanup"
    );
    assert_eq!(join_during_cleanup, Err(ExecutorError::JoinTimeout));
    assert_eq!(scope.active_children(), 0);
    assert_eq!(executor.snapshot().late_dropped, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn public_cancel_does_not_skip_the_later_bounded_child_join() {
    let scope = ChildScope::new(Instant::now() + Duration::from_secs(1));
    let child_scope = scope.clone();
    let child =
        tokio::spawn(async move { child_scope.run_inline(std::future::pending::<()>()).await });
    while scope.active_children() == 0 {
        tokio::task::yield_now().await;
    }

    scope.cancel();
    scope
        .cancel_and_finalize(Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(
        scope.active_children(),
        0,
        "cancelled and joined are separate states; finalize must still drain the child",
    );
    assert_eq!(child.await.unwrap().unwrap_err(), ExecutorError::Cancelled);
}
