use crate::fixture::*;

#[tokio::test]
async fn early_accept_finishes_request_without_reset_and_preserves_response()
-> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, b"early-accepted", true).await
    });

    let plan = PlanRevision(77);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(BootstrapPublicationBuilder::new(plan.0, 7).route(
        "gateway.test",
        "/early-accept",
        1,
        plain_target(address, 8),
    )?)?;
    let selection = TestSelection::new([binding]);
    let gateway = lifecycle(publications, selection.clone(), TrackingFilters::default())?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/early-accept",
        Bytes::from(vec![b'p'; 32 * 1024]),
        HttpProtocol::Http1,
    );

    assert_eq!(gateway.process(&mut session).await?, SessionReuse::Reusable);
    assert_eq!(session.response_body, b"early-accepted");
    assert_eq!(selection.published(), [Disposition::Accept]);
    assert!(
        !server.await??,
        "Accept must reach normal request EOS without resetting its response"
    );
    Ok(())
}

#[tokio::test]
async fn delivered_protocol_terminal_survives_disconnect_while_upstream_tail_is_released()
-> Result<(), TestError> {
    delivered_terminal_cleanup(false, true).await
}

#[tokio::test]
async fn delivered_protocol_terminal_drains_upstream_tail_without_cleanup_timeout()
-> Result<(), TestError> {
    delivered_terminal_cleanup(true, false).await
}

async fn delivered_terminal_cleanup(tail: bool, disconnect: bool) -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let events_sent = Arc::new(Notify::new());
    let release_eos = Arc::new(Notify::new());
    let server_events_sent = Arc::clone(&events_sent);
    let server_release_eos = Arc::clone(&release_eos);
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_chunked_sse_until_release(socket, server_events_sent, server_release_eos, tail)
            .await
    });

    let plan = PlanRevision(7701);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let sse_plan = BodyPlan::SseFramedStreaming {
        max_event_bytes: 64,
        max_pending_bytes: 64,
        max_output_event_bytes: 64,
        expansion_ratio_numerator: 1,
        expansion_ratio_denominator: 1,
        expansion_slack_bytes: 0,
    };
    let mut plans = lifecycle_body_plans();
    plans.attempt_response_precommit = sse_plan.clone();
    plans.accepted_response = sse_plan;
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 701)
            .route(
                "gateway.test",
                "/delivered-terminal-upstream-tail",
                1,
                plain_target(address, 701),
            )?
            .body_plans(1, plans)?,
    )?;
    let selection = TestSelection::new([binding]);
    let provider = PassthroughProvider {
        accept_on_first_semantic_sse: true,
        end_stream_on_semantic_sse: true,
        ..PassthroughProvider::default()
    };
    let gateway = lifecycle_with_provider(
        publications,
        selection.clone(),
        provider,
        TrackingFilters::default(),
    )?;
    let session = RecordingSession::new(
        "gateway.test",
        "/delivered-terminal-upstream-tail",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );
    let cancellation = session.cancellation_handle();
    let observed_writes = Arc::clone(&session.observed_response_body_writes);
    let write_observed = Arc::clone(&session.response_body_write_observed);
    let terminal_written = tokio::spawn(async move {
        loop {
            let notified = write_observed.notified();
            if observed_writes
                .lock()
                .expect("observed response writes")
                .iter()
                .any(|(_, end_stream)| *end_stream)
            {
                return;
            }
            notified.await;
        }
    });
    let process = tokio::spawn(async move {
        let mut session = session;
        let result = gateway.process(&mut session).await;
        (result, session)
    });

    events_sent.notified().await;
    terminal_written.await?;
    if disconnect {
        cancellation.cancel();
    }
    release_eos.notify_one();

    let (result, session) = process.await?;
    assert_eq!(
        result?,
        if disconnect {
            SessionReuse::Close
        } else {
            SessionReuse::Reusable
        }
    );
    assert!(session.response_eos);
    let completed = selection.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].stream, AttemptStreamOutcome::CompletedEos);
    assert_eq!(completed[0].downstream, AttemptDownstreamOutcome::Completed);
    assert_eq!(completed[0].cleanup, AttemptCleanupOutcome::Completed);
    assert_eq!(
        completed[0].termination_reason,
        AttemptTerminationReason::AcceptedEos
    );
    server.await??;
    Ok(())
}

#[tokio::test]
async fn delivered_response_survives_disconnect_and_failed_filter_cleanup() -> Result<(), TestError>
{
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, b"delivered", false).await
    });

    let plan = PlanRevision(7702);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 702)
            .route(
                "gateway.test",
                "/delivered-cleanup-failure",
                1,
                plain_target(address, 702),
            )?
            .route_accepted_filters(1, Arc::from([compiled_filter("accepted-cleanup")]))?,
    )?;
    let selection = TestSelection::new([binding]);
    let cleanup_release = Arc::new(Notify::new());
    let filters = TrackingFilters {
        accepted_cleanup_release: Some(Arc::clone(&cleanup_release)),
        fail_accepted_cleanup: true,
        ..TrackingFilters::default()
    };
    let cleanup_entered_signal = Arc::clone(&filters.accepted_cleanup_entered);
    let cleanup_entered = cleanup_entered_signal.notified();
    tokio::pin!(cleanup_entered);
    let gateway = lifecycle_with_provider(
        publications,
        selection.clone(),
        PassthroughProvider::default(),
        filters,
    )?;
    let session = RecordingSession::new(
        "gateway.test",
        "/delivered-cleanup-failure",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );
    let cancellation = session.cancellation_handle();
    let process = tokio::spawn(async move {
        let mut session = session;
        let result = gateway.process(&mut session).await;
        (result, session)
    });

    cleanup_entered.as_mut().await;
    cancellation.cancel();
    cleanup_release.notify_one();

    let (result, session) = process.await?;
    assert_eq!(result?, SessionReuse::Close);
    assert_eq!(session.response_body, b"delivered");
    assert!(session.response_eos);
    let completed = selection.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].stream, AttemptStreamOutcome::CompletedEos);
    assert_eq!(completed[0].downstream, AttemptDownstreamOutcome::Completed);
    assert_eq!(completed[0].cleanup, AttemptCleanupOutcome::Failed);
    assert_eq!(
        completed[0].termination_reason,
        AttemptTerminationReason::CleanupFailure
    );
    assert!(!server.await??);
    Ok(())
}

#[tokio::test]
async fn early_terminate_resets_upstream_before_publishing_local_response() -> Result<(), TestError>
{
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, b"must-not-forward", true).await
    });

    let plan = PlanRevision(78);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(BootstrapPublicationBuilder::new(plan.0, 8).route(
        "gateway.test",
        "/early-terminate",
        1,
        plain_target(address, 9),
    )?)?;
    let selection = TestSelection::new([binding]);
    selection.force_disposition(Disposition::Terminate);
    let provider = PassthroughProvider::default();
    let terminal_head_encodes = Arc::clone(&provider.terminal_head_encodes);
    let terminal_body_encodes = Arc::clone(&provider.terminal_body_encodes);
    let encoded_effects = Arc::clone(&provider.encoded_terminal_effects);
    let finalized_attempts = Arc::clone(&provider.finalized_attempts);
    let gateway = lifecycle_with_provider(
        publications,
        selection.clone(),
        provider,
        TrackingFilters::default(),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/early-terminate",
        Bytes::from(vec![b'p'; 32 * 1024]),
        HttpProtocol::Http1,
    );

    assert_eq!(gateway.process(&mut session).await?, SessionReuse::Reusable);
    assert_eq!(
        session.response_head.as_ref().unwrap().status,
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(selection.published(), [Disposition::Terminate]);
    assert_eq!(terminal_head_encodes.load(Ordering::Relaxed), 1);
    assert_eq!(terminal_body_encodes.load(Ordering::Relaxed), 1);
    assert_eq!(finalized_attempts.load(Ordering::Relaxed), 1);
    let completed = selection.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].stream, AttemptStreamOutcome::CompletedEos);
    assert_eq!(
        completed[0].termination_reason,
        AttemptTerminationReason::TerminatedResponseComplete
    );
    {
        let encoded_effects = encoded_effects.lock().expect("terminal effects");
        assert_eq!(encoded_effects.len(), 1);
        assert_eq!(encoded_effects[0].semantic_upstream_calls, 1);
        assert_eq!(encoded_effects[0].reset_count, 1);
    }
    assert!(
        server.await??,
        "Terminate must destructively converge the early-response writer"
    );
    Ok(())
}

#[tokio::test]
async fn terminate_encoder_failure_is_observed_once_after_cleanup() -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, b"must-not-forward", true).await
    });
    let plan = PlanRevision(7801);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(BootstrapPublicationBuilder::new(plan.0, 801).route(
        "gateway.test",
        "/terminate-encoder-failure",
        1,
        plain_target(address, 801),
    )?)?;
    let selection = TestSelection::new([binding]);
    selection.force_disposition(Disposition::Terminate);
    let provider = PassthroughProvider {
        fail_terminal_encoder: true,
        ..PassthroughProvider::default()
    };
    let finalized_attempts = Arc::clone(&provider.finalized_attempts);
    let gateway = lifecycle_with_provider(
        publications,
        selection.clone(),
        provider,
        TrackingFilters::default(),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/terminate-encoder-failure",
        Bytes::from(vec![b'p'; 32 * 1024]),
        HttpProtocol::Http1,
    );

    let error = gateway.process(&mut session).await.unwrap_err();
    assert!(error.to_string().contains("terminal encoder failure"));
    assert_eq!(selection.published(), [Disposition::Terminate]);
    assert_eq!(finalized_attempts.load(Ordering::Relaxed), 1);
    let completed = selection.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].cleanup, AttemptCleanupOutcome::Completed);
    assert_eq!(
        completed[0].termination_reason,
        AttemptTerminationReason::ProviderEncoderFailure
    );
    assert!(server.await??);
    Ok(())
}

#[tokio::test]
async fn terminate_filter_failure_is_observed_once_after_cleanup() -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, b"must-not-forward", true).await
    });
    let plan = PlanRevision(7802);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 802)
            .route(
                "gateway.test",
                "/terminate-filter-failure",
                1,
                plain_target(address, 802),
            )?
            .route_accepted_filters(1, Arc::from([compiled_filter("accepted-fail")]))?,
    )?;
    let selection = TestSelection::new([binding]);
    selection.force_disposition(Disposition::Terminate);
    let provider = PassthroughProvider::default();
    let finalized_attempts = Arc::clone(&provider.finalized_attempts);
    let gateway = lifecycle_with_provider(
        publications,
        selection.clone(),
        provider,
        TrackingFilters {
            fail_accepted_head: true,
            ..TrackingFilters::default()
        },
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/terminate-filter-failure",
        Bytes::from(vec![b'p'; 32 * 1024]),
        HttpProtocol::Http1,
    );

    let error = gateway.process(&mut session).await.unwrap_err();
    assert!(error.to_string().contains("accepted filter failure"));
    assert_eq!(selection.published(), [Disposition::Terminate]);
    assert_eq!(finalized_attempts.load(Ordering::Relaxed), 1);
    let completed = selection.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(
        completed[0].termination_reason,
        AttemptTerminationReason::FilterFailure
    );
    assert!(server.await??);
    Ok(())
}

#[tokio::test]
async fn terminate_completion_waits_for_accepted_filter_cleanup_failure() -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, b"must-not-forward", false).await
    });
    let plan = PlanRevision(7803);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 803)
            .route(
                "gateway.test",
                "/terminate-cleanup-failure",
                1,
                plain_target(address, 803),
            )?
            .route_accepted_filters(1, Arc::from([compiled_filter("accepted-cleanup")]))?,
    )?;
    let selection = TestSelection::new([binding]);
    selection.force_disposition(Disposition::Terminate);
    let provider = PassthroughProvider::default();
    let finalized_attempts = Arc::clone(&provider.finalized_attempts);
    let filters = TrackingFilters {
        accepted_cleanup_delay: Some(Duration::from_millis(50)),
        fail_accepted_cleanup: true,
        ..TrackingFilters::default()
    };
    let cleanup_entered = filters.accepted_cleanup_entered.notified();
    tokio::pin!(cleanup_entered);
    let gateway =
        lifecycle_with_provider(publications, selection.clone(), provider, filters.clone())?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/terminate-cleanup-failure",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );
    let process = tokio::spawn(async move {
        let result = gateway.process(&mut session).await;
        (result, session)
    });

    tokio::time::timeout(Duration::from_secs(2), &mut cleanup_entered).await?;
    assert_eq!(finalized_attempts.load(Ordering::Relaxed), 0);
    assert!(selection.completed().is_empty());

    let (result, session) = process.await?;
    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("synthetic accepted filter cleanup failure")
    );
    assert_eq!(
        session
            .response_head
            .as_ref()
            .expect("terminal head")
            .status,
        StatusCode::BAD_GATEWAY
    );
    assert!(session.response_eos);
    assert_eq!(filters.attempt_cleanup.load(Ordering::Relaxed), 1);
    assert_eq!(filters.accepted_cleanup.load(Ordering::Relaxed), 1);
    assert_eq!(filters.finalized.load(Ordering::Relaxed), 1);
    assert_eq!(finalized_attempts.load(Ordering::Relaxed), 1);
    let completed = selection.completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].cleanup, AttemptCleanupOutcome::Failed);
    assert_eq!(
        completed[0].termination_reason,
        AttemptTerminationReason::CleanupFailure
    );
    assert!(!server.await??);
    Ok(())
}

#[tokio::test]
async fn request_cancellation_aborts_real_upstream_without_publishing_disposition()
-> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (request_head_tx, request_head_rx) = oneshot::channel();
    let (upstream_closed_tx, upstream_closed_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let _head = read_h1_head(&mut socket).await?;
        let _ = request_head_tx.send(());
        let mut buffer = [0_u8; 1024];
        let closed = loop {
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => break true,
                Ok(_) => continue,
            }
        };
        let _ = upstream_closed_tx.send(closed);
        Ok::<(), TestError>(())
    });

    let plan = PlanRevision(79);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let publications = install_publication(BootstrapPublicationBuilder::new(plan.0, 9).route(
        "gateway.test",
        "/cancel",
        1,
        plain_target(address, 10),
    )?)?;
    let selection = TestSelection::new([binding]);
    let sink = Arc::new(RecordingFailingSink::default());
    let telemetry = Arc::new(Telemetry::new(sink.clone()));
    let gateway = lifecycle(publications, selection.clone(), TrackingFilters::default())?
        .with_telemetry(telemetry.clone());
    let mut session = RecordingSession::new(
        "gateway.test",
        "/cancel",
        Bytes::from(vec![b'p'; 32 * 1024]),
        HttpProtocol::Http1,
    );
    let cancellation = session.cancellation_handle();
    let process = gateway.process(&mut session);
    tokio::pin!(process);

    tokio::select! {
        result = &mut process => return Err(format!("gateway completed before cancel: {result:?}").into()),
        result = request_head_rx => result?,
    }
    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_secs(2), &mut process)
        .await?
        .expect_err("cancelled lifecycle must fail closed");
    assert!(error.to_string().contains("request was cancelled"));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), upstream_closed_rx).await??,
        "cancellation must close/reset the active upstream"
    );
    server.await??;
    assert!(selection.published().is_empty());
    telemetry.flush(Duration::from_secs(1))?;
    assert!(
        sink.events
            .lock()
            .expect("telemetry events")
            .iter()
            .any(|event| matches!(
                event.kind,
                LifecycleKind::Error(ref fact) if fact.class == ErrorClass::Cancelled
            ))
    );
    assert!(telemetry.sink_failures() > 0);
    Ok(())
}
