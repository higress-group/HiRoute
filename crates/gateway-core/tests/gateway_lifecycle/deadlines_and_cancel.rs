use crate::fixture::*;

#[tokio::test]
async fn absolute_deadline_bounds_request_body_and_provider_materialization()
-> Result<(), TestError> {
    let plan = PlanRevision(85);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(BootstrapPublicationBuilder::new(plan.0, 15).route(
        "gateway.test",
        "/body-deadline",
        1,
        plain_target("127.0.0.1:9".parse()?, 22),
    )?)?;
    let selection = TestSelection::new([binding]);
    let filters = TrackingFilters::default();
    let gateway = lifecycle_with_provider_and_limits(
        publications,
        selection.clone(),
        PassthroughProvider::default(),
        filters.clone(),
        bounded_test_limits(Duration::from_millis(75)),
    )?;
    let mut body_blocked = RecordingSession::new(
        "gateway.test",
        "/body-deadline",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    )
    .with_request_body_delay(Duration::from_secs(5));
    let started = Instant::now();
    let error = gateway.process(&mut body_blocked).await.unwrap_err();
    assert!(error.to_string().contains("request deadline exceeded"));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(selection.selected(), 0);
    assert_eq!(filters.finalized.load(Ordering::Relaxed), 1);

    let plan = PlanRevision(86);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(BootstrapPublicationBuilder::new(plan.0, 16).route(
        "gateway.test",
        "/provider-deadline",
        1,
        plain_target("127.0.0.1:9".parse()?, 23),
    )?)?;
    let selection = TestSelection::new([binding]);
    let filters = TrackingFilters::default();
    let provider = PassthroughProvider {
        materialize_delay: Some(Duration::from_secs(5)),
        ..PassthroughProvider::default()
    };
    let gateway = lifecycle_with_provider_and_limits(
        publications,
        selection.clone(),
        provider,
        filters.clone(),
        bounded_test_limits(Duration::from_millis(75)),
    )?;
    let mut provider_blocked = RecordingSession::new(
        "gateway.test",
        "/provider-deadline",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );
    let started = Instant::now();
    let error = gateway.process(&mut provider_blocked).await.unwrap_err();
    assert!(error.to_string().contains("request deadline exceeded"));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(selection.selected(), 1);
    assert!(selection.published().is_empty());
    assert_eq!(filters.finalized.load(Ordering::Relaxed), 1);
    Ok(())
}

#[tokio::test]
async fn hanging_attempt_filter_deadline_aborts_upstream_and_finalizes_once()
-> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_then_observe_client_close(socket, b"response").await
    });
    let plan = PlanRevision(87);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 17)
            .route(
                "gateway.test",
                "/filter-deadline",
                1,
                plain_target(address, 24),
            )?
            .attempt_response_filters(1, Arc::from([compiled_filter("attempt-delay")]))?,
    )?;
    let selection = TestSelection::new([binding]);
    let filters = TrackingFilters {
        attempt_delay: Some(Duration::from_secs(5)),
        ..TrackingFilters::default()
    };
    let gateway = lifecycle_with_provider_and_limits(
        publications,
        selection.clone(),
        PassthroughProvider::default(),
        filters.clone(),
        // Leave room for a cold Pingora connector startup so this test
        // deterministically reaches the callback it is intended to bound.
        bounded_test_limits(Duration::from_millis(500)),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/filter-deadline",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );

    let started = Instant::now();
    let error = gateway.process(&mut session).await.unwrap_err();
    assert!(error.to_string().contains("request deadline exceeded"));
    assert!(selection.published().is_empty());
    assert!(filters.attempt.load(Ordering::Relaxed) > 0);
    assert_eq!(filters.finalized.load(Ordering::Relaxed), 1);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(server.await??, "deadline must close the upstream attempt");
    Ok(())
}

#[tokio::test]
async fn attempt_filter_panic_isolated_then_aborts_upstream_and_finalizes_once()
-> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_then_observe_client_close(socket, b"response").await
    });
    let plan = PlanRevision(88);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 18)
            .route(
                "gateway.test",
                "/filter-panic",
                1,
                plain_target(address, 25),
            )?
            .attempt_response_filters(1, Arc::from([compiled_filter("attempt-panic")]))?,
    )?;
    let selection = TestSelection::new([binding]);
    let filters = TrackingFilters {
        panic_attempt: true,
        ..TrackingFilters::default()
    };
    let gateway = lifecycle_with_provider_and_limits(
        publications,
        selection.clone(),
        PassthroughProvider::default(),
        filters.clone(),
        bounded_test_limits(Duration::from_secs(2)),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/filter-panic",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );

    let error = gateway.process(&mut session).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("request-owned operation panicked")
    );
    assert!(selection.published().is_empty());
    assert_eq!(filters.finalized.load(Ordering::Relaxed), 1);
    assert!(server.await??, "panic containment must close the upstream");
    Ok(())
}

#[tokio::test]
async fn downstream_write_uses_request_deadline_and_aborts_accepted_upstream()
-> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_then_observe_client_close(socket, b"response").await
    });
    let plan = PlanRevision(89);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(BootstrapPublicationBuilder::new(plan.0, 19).route(
        "gateway.test",
        "/downstream-deadline",
        1,
        plain_target(address, 26),
    )?)?;
    let selection = TestSelection::new([binding]);
    let filters = TrackingFilters::default();
    let provider = PassthroughProvider::default();
    let finalized_attempts = Arc::clone(&provider.finalized_attempts);
    let gateway = lifecycle_with_provider_and_limits(
        publications,
        selection.clone(),
        provider,
        filters.clone(),
        // A cold Pingora connector can consume most of a 150 ms test window;
        // keep the callback delay much larger while guaranteeing the absolute
        // request deadline expires during the downstream write itself.
        bounded_test_limits(Duration::from_millis(500)),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/downstream-deadline",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    )
    .with_response_head_delay(Duration::from_secs(5));

    let started = Instant::now();
    let error = gateway.process(&mut session).await.unwrap_err();
    assert!(error.to_string().contains("request deadline exceeded"));
    assert_eq!(selection.published(), [Disposition::Accept]);
    assert!(session.response_head.is_none());
    let completed = selection.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].downstream, AttemptDownstreamOutcome::Failed);
    assert_eq!(completed[0].cleanup, AttemptCleanupOutcome::Completed);
    assert_eq!(
        completed[0].commits.downstream_headers,
        CommitFence::WriteStartedMayHaveCommitted
    );
    assert!(completed[0].ended_at >= completed[0].transport.started_at);
    assert_eq!(finalized_attempts.load(Ordering::Relaxed), 1);
    assert_eq!(filters.finalized.load(Ordering::Relaxed), 1);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        server.await??,
        "downstream timeout must close the accepted upstream"
    );
    Ok(())
}

#[tokio::test]
async fn post_commit_stream_idle_completes_without_reentering_fallback() -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let first_listener = TcpListener::bind("127.0.0.1:0").await?;
    let first_address = first_listener.local_addr()?;
    let first_server = tokio::spawn(async move {
        let (socket, _) = first_listener.accept().await?;
        serve_h1_semantic_chunk_then_gap(socket, Duration::from_millis(200)).await
    });
    let second_listener = TcpListener::bind("127.0.0.1:0").await?;
    let second_address = second_listener.local_addr()?;
    let (second_connect_tx, second_connect_rx) = oneshot::channel();
    let second_server = tokio::spawn(async move {
        let (socket, _) = second_listener.accept().await?;
        let _ = second_connect_tx.send(());
        serve_h1_once(socket, StatusCode::OK, b"must-not-fallback", false).await
    });

    let plan = PlanRevision(8901);
    let first = ResolvedTargetBindingId::new(plan, 1);
    let second = ResolvedTargetBindingId::new(plan, 2);
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 1901)
            .route(
                "gateway.test",
                "/post-commit-idle",
                1,
                plain_target(first_address, 1901),
            )?
            .route(
                "gateway.test",
                "/unused-idle-fallback",
                2,
                plain_target(second_address, 1902),
            )?
            .route_candidates(1, [1, 2])?
            .attempt_timeouts(
                1,
                AttemptTimeouts {
                    request_write: Duration::from_secs(1),
                    first_byte: Duration::from_secs(1),
                    stream_idle: Duration::from_millis(40),
                },
            )?,
    )?;
    let selection = TestSelection::new([first, second]);
    let provider = PassthroughProvider::default();
    let finalized_attempts = Arc::clone(&provider.finalized_attempts);
    let gateway = lifecycle_with_provider(
        publications,
        selection.clone(),
        provider,
        TrackingFilters::default(),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/post-commit-idle",
        Bytes::new(),
        HttpProtocol::Http1,
    );

    let error = gateway.process(&mut session).await.unwrap_err();
    assert!(error.to_string().contains("idle"));
    assert_eq!(selection.selected(), 1);
    assert_eq!(selection.published(), [Disposition::Accept]);
    let completed = selection.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(
        completed[0].stream,
        AttemptStreamOutcome::StreamStartedNoRetry
    );
    assert_eq!(
        completed[0].termination_reason,
        AttemptTerminationReason::StreamStartedNoRetry
    );
    assert_eq!(
        completed[0].commits.downstream_semantic,
        CommitFence::WriteConfirmed
    );
    assert_eq!(
        completed[0].transport.timeout,
        Some(AttemptTimeoutKind::StreamIdle)
    );
    assert_eq!(finalized_attempts.load(Ordering::Relaxed), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), second_connect_rx)
            .await
            .is_err(),
        "post-commit failure cannot enter fallback"
    );
    assert!(first_server.await??);
    second_server.abort();
    let _ = second_server.await;
    Ok(())
}

#[tokio::test]
async fn cancellation_after_downstream_semantic_write_completes_once_without_fallback()
-> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_semantic_chunk_until_client_close(socket).await
    });
    let plan = PlanRevision(8902);
    let first = ResolvedTargetBindingId::new(plan, 1);
    let second = ResolvedTargetBindingId::new(plan, 2);
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 1902)
            .route(
                "gateway.test",
                "/post-commit-cancel",
                1,
                plain_target(address, 1903),
            )?
            .route(
                "gateway.test",
                "/unused-cancel-fallback",
                2,
                plain_target("127.0.0.1:9".parse()?, 1904),
            )?
            .route_candidates(1, [1, 2])?,
    )?;
    let selection = TestSelection::new([first, second]);
    let provider = PassthroughProvider::default();
    let finalized_attempts = Arc::clone(&provider.finalized_attempts);
    let gateway = lifecycle_with_provider(
        publications,
        selection.clone(),
        provider,
        TrackingFilters::default(),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/post-commit-cancel",
        Bytes::new(),
        HttpProtocol::Http1,
    );
    let cancellation = session.cancellation_handle();
    let body_writes = Arc::clone(&session.observed_response_body_writes);
    let mut process = Box::pin(gateway.process(&mut session));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !body_writes.lock().expect("body writes").is_empty() {
                break;
            }
            tokio::select! {
                result = process.as_mut() => {
                    panic!("request completed before cancellation point: {result:?}");
                }
                _ = tokio::time::sleep(Duration::from_millis(1)) => {}
            }
        }
    })
    .await?;
    cancellation.cancel();
    let error = process.as_mut().await.unwrap_err();
    drop(process);

    assert_eq!(
        error,
        hiroute_gateway_core::transport::TransportError::Cancelled
    );
    assert_eq!(selection.selected(), 1);
    assert_eq!(selection.published(), [Disposition::Accept]);
    let completed = selection.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].downstream, AttemptDownstreamOutcome::Cancelled);
    assert_eq!(
        completed[0].stream,
        AttemptStreamOutcome::StreamStartedNoRetry
    );
    assert_eq!(
        completed[0].termination_reason,
        AttemptTerminationReason::StreamStartedNoRetry
    );
    assert_eq!(finalized_attempts.load(Ordering::Relaxed), 1);
    assert!(server.await??);
    Ok(())
}
