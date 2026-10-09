use crate::fixture::*;

#[tokio::test]
async fn production_sse_framer_replays_accept_trigger_once_and_preserves_semantic_provenance()
-> Result<(), TestError> {
    const SSE: &[u8] = b": heartbeat\n\ndata: choose\n\ndata: later\n\n";
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, SSE, true).await
    });
    let plan = PlanRevision(97);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let sse_plan = BodyPlan::SseFramedStreaming {
        max_event_bytes: 20,
        max_pending_bytes: 20,
        max_output_event_bytes: 20,
        expansion_ratio_numerator: 1,
        expansion_ratio_denominator: 1,
        expansion_slack_bytes: 0,
    };
    let mut plans = lifecycle_body_plans();
    plans.attempt_response_precommit = sse_plan.clone();
    plans.accepted_response = sse_plan;
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 27)
            .route("gateway.test", "/sse-handoff", 1, plain_target(address, 34))?
            .body_plans(1, plans)?
            .precommit_event_capacity(1, 2)?,
    )?;
    let selection = TestSelection::new([binding]);
    let provider = PassthroughProvider {
        accept_on_first_semantic_sse: true,
        ..PassthroughProvider::default()
    };
    let classified = Arc::clone(&provider.classified_sse);
    let encoded = Arc::clone(&provider.encoded_sse);
    let decoded_prefix = Arc::clone(&provider.decoded_prefix_sse);
    let raw_tail = Arc::clone(&provider.raw_tail_sse);
    let finalized_attempts = Arc::clone(&provider.finalized_attempts);
    let gateway = lifecycle_with_provider(
        publications,
        selection.clone(),
        provider,
        TrackingFilters::default(),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/sse-handoff",
        Bytes::from(vec![b'p'; 32 * 1024]),
        HttpProtocol::Http1,
    );

    assert_eq!(gateway.process(&mut session).await?, SessionReuse::Reusable);
    assert_eq!(session.response_body, SSE);
    assert_eq!(
        session.response_body_writes,
        vec![
            (Bytes::from_static(b": heartbeat\n\n"), false),
            (Bytes::from_static(b"data: choose\n\n"), false),
            (Bytes::from_static(b"data: later\n\n"), false),
            (Bytes::new(), true),
        ],
        "each complete SSE event is one downstream emission unit"
    );
    assert_eq!(selection.published(), [Disposition::Accept]);
    let completed = selection.completed();
    assert_eq!(completed.len(), 1, "one published stream completes once");
    assert_eq!(finalized_attempts.load(Ordering::Relaxed), 1);
    assert_eq!(completed[0].stream, AttemptStreamOutcome::CompletedEos);
    assert_eq!(completed[0].cleanup, AttemptCleanupOutcome::Completed);
    assert_eq!(
        completed[0].termination_reason,
        AttemptTerminationReason::AcceptedEos
    );
    let provider_facts = completed[0]
        .provider
        .as_ref()
        .expect("provider final facts");
    assert!(provider_facts.ended_at.is_some());
    assert!(
        provider_facts.ttft.is_some(),
        "semantic event establishes TTFT"
    );
    assert!(
        completed[0].transport.upstream_ttfb.is_some(),
        "the earlier heartbeat establishes transport TTFB independently"
    );
    let usage = provider_facts.usage.expect("EOS provider Usage");
    assert_eq!(usage.input.units, Some(11));
    assert_eq!(usage.cache_write.units, None);
    assert_eq!(usage.reasoning.units, Some(2));
    assert_eq!(
        *classified.lock().expect("classified SSE facts"),
        vec![
            (0, SemanticProvenance::NonSemantic),
            (1, SemanticProvenance::ProducesSemantic),
        ],
        "classification stops at the first semantic event"
    );
    assert_eq!(
        *encoded.lock().expect("encoded SSE facts"),
        vec![
            (0, SemanticProvenance::NonSemantic),
            (1, SemanticProvenance::ProducesSemantic),
            (2, SemanticProvenance::ProducesSemantic),
        ],
        "the comment, Accept trigger and later event are emitted exactly once"
    );
    assert_eq!(
        *decoded_prefix.lock().expect("decoded prefix facts"),
        vec![0, 1],
        "classified prefix IR must enter the accepted encoder without raw reparsing"
    );
    assert_eq!(
        *raw_tail.lock().expect("raw tail facts"),
        vec![2],
        "only the event already framed but not classified at Accept may decode from raw"
    );
    assert!(
        !server.await??,
        "Accept must still drive the early-response request writer to normal EOS"
    );
    Ok(())
}

#[tokio::test]
async fn buffered_attempt_sse_drop_preserves_the_forwarded_source_sequence() -> Result<(), TestError>
{
    const FORWARDED: &[u8] = b"data: choose\n\n";
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let events_sent = Arc::new(Notify::new());
    let release_eos = Arc::new(Notify::new());
    let server_events_sent = Arc::clone(&events_sent);
    let server_release_eos = Arc::clone(&release_eos);
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_chunked_sse_until_release(socket, server_events_sent, server_release_eos, false)
            .await
    });
    let plan = PlanRevision(111);
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
        BootstrapPublicationBuilder::new(plan.0, 111)
            .route(
                "gateway.test",
                "/sse-drop-first",
                1,
                plain_target(address, 111),
            )?
            .body_plans(1, plans)?
            .attempt_response_filters(
                1,
                Arc::from([compiled_filter("attempt-sse-buffer-drop-first")]),
            )?,
    )?;
    let facts = Arc::new(NativeFilterFacts::default());
    let resume_facts = Arc::clone(&facts);
    let resume = tokio::spawn(async move {
        events_sent.notified().await;
        let continuation = loop {
            if let Some(continuation) = resume_facts
                .watermark_continuation
                .lock()
                .expect("attempt buffer continuation")
                .take()
            {
                break continuation;
            }
            tokio::task::yield_now().await;
        };
        continuation
            .resume(ResumeAction::Continue(HeaderPatch::default()))
            .map_err(|_| io::Error::other("attempt buffer continuation was stale"))?;
        release_eos.notify_one();
        Ok::<_, io::Error>(())
    });
    let provider = PassthroughProvider {
        accept_on_first_semantic_sse: true,
        ..PassthroughProvider::default()
    };
    let classified = Arc::clone(&provider.classified_sse);
    let gateway = GatewayCoreLifecycle::new(
        publications,
        TestSelection::new([binding]),
        provider,
        native_filter_manager(facts, &["attempt-sse-buffer-drop-first"])?,
        PingoraConnectorAdapter::new(),
        bounded_test_limits(Duration::from_secs(5)),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/sse-drop-first",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );

    let result = gateway.process(&mut session).await;
    assert!(
        matches!(result, Ok(SessionReuse::Reusable)),
        "buffered SSE lifecycle failed: {result:?}; classified={:?}",
        *classified.lock().expect("classified SSE facts"),
    );
    resume.await??;
    assert_eq!(session.response_body, FORWARDED);
    assert_eq!(
        *classified.lock().expect("classified SSE facts"),
        vec![(1, SemanticProvenance::ProducesSemantic)],
        "dropping source zero must not relabel source one by output position",
    );
    server.await??;
    Ok(())
}

#[tokio::test]
async fn accepted_sse_merge_carries_both_sources_and_enforces_each_source_limit()
-> Result<(), TestError> {
    const SOURCE: &[u8] = b"data:a\n\ndata:b\n\n";
    const MERGED: &[u8] = b"data: merged\n\n";
    let _network_guard = NETWORK_TEST_LOCK.lock().await;

    for (revision, ratio, succeeds) in [(112_u64, 2_usize, true), (113, 1, false)] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await?;
            serve_h1_once(socket, StatusCode::OK, SOURCE, false).await
        });
        let plan = PlanRevision(revision);
        let binding = ResolvedTargetBindingId::new(plan, 1);
        let sse_plan = BodyPlan::SseFramedStreaming {
            max_event_bytes: 64,
            max_pending_bytes: 64,
            max_output_event_bytes: 64,
            expansion_ratio_numerator: ratio,
            expansion_ratio_denominator: 1,
            expansion_slack_bytes: 0,
        };
        let mut plans = lifecycle_body_plans();
        plans.attempt_response_precommit = sse_plan.clone();
        plans.accepted_response = sse_plan;
        let publications = install_publication(
            BootstrapPublicationBuilder::new(plan.0, revision)
                .route(
                    "gateway.test",
                    "/sse-merge",
                    1,
                    plain_target(address, revision),
                )?
                .body_plans(1, plans)?
                .route_accepted_filters(
                    1,
                    Arc::from([compiled_filter("accepted-sse-merge-two")]),
                )?,
        )?;
        let provider = PassthroughProvider {
            accept_on_first_semantic_sse: true,
            ..PassthroughProvider::default()
        };
        let gateway = GatewayCoreLifecycle::new(
            publications,
            TestSelection::new([binding]),
            provider,
            native_filter_manager(
                Arc::new(NativeFilterFacts::default()),
                &["accepted-sse-merge-two"],
            )?,
            PingoraConnectorAdapter::new(),
            bounded_test_limits(Duration::from_secs(5)),
        )?;
        let mut session = RecordingSession::new(
            "gateway.test",
            "/sse-merge",
            Bytes::from_static(b"prompt"),
            HttpProtocol::Http1,
        );

        if succeeds {
            assert_eq!(gateway.process(&mut session).await?, SessionReuse::Reusable);
            assert_eq!(session.response_body, MERGED);
        } else {
            let error = gateway.process(&mut session).await.unwrap_err();
            assert!(error.to_string().contains("SSE output exceeds"));
            assert!(session.response_body.is_empty());
        }
        let _ = server.await??;
    }
    Ok(())
}

#[tokio::test]
async fn accepted_sse_mixed_provenance_merge_uses_conservative_or() -> Result<(), TestError> {
    const SOURCE: &[u8] = b"data:a\n\n: heartbeat\n\n";
    const MERGED: &[u8] = b"data: merged\n\n";
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, SOURCE, false).await
    });
    let plan = PlanRevision(122);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let sse_plan = BodyPlan::SseFramedStreaming {
        max_event_bytes: 32,
        max_pending_bytes: 32,
        max_output_event_bytes: 32,
        expansion_ratio_numerator: 2,
        expansion_ratio_denominator: 1,
        expansion_slack_bytes: 0,
    };
    let mut plans = lifecycle_body_plans();
    plans.attempt_response_precommit = sse_plan.clone();
    plans.accepted_response = sse_plan;
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 122)
            .route(
                "gateway.test",
                "/sse-mixed-provenance",
                1,
                plain_target(address, 122),
            )?
            .body_plans(1, plans)?
            .route_accepted_filters(1, Arc::from([compiled_filter("accepted-sse-merge-two")]))?,
    )?;
    let gateway = GatewayCoreLifecycle::new(
        publications,
        TestSelection::new([binding]),
        PassthroughProvider {
            accept_on_first_semantic_sse: true,
            ..PassthroughProvider::default()
        },
        native_filter_manager(
            Arc::new(NativeFilterFacts::default()),
            &["accepted-sse-merge-two"],
        )?,
        PingoraConnectorAdapter::new(),
        bounded_test_limits(Duration::from_secs(5)),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/sse-mixed-provenance",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );

    assert_eq!(gateway.process(&mut session).await?, SessionReuse::Reusable);
    assert_eq!(session.response_body, MERGED);
    let _ = server.await??;
    Ok(())
}

#[tokio::test]
async fn accepted_sse_one_to_many_uses_one_source_ledger_and_cumulative_expansion_limit()
-> Result<(), TestError> {
    const SOURCE: &[u8] = b"data:x\n\n";
    const EXPANDED: &[u8] = b"data:a\n\ndata:b\n\n";
    let _network_guard = NETWORK_TEST_LOCK.lock().await;

    for (revision, ratio, succeeds) in [(108_u64, 2_usize, true), (109, 1, false)] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await?;
            serve_h1_once(socket, StatusCode::OK, SOURCE, false).await
        });
        let plan = PlanRevision(revision);
        let binding = ResolvedTargetBindingId::new(plan, 1);
        let sse_plan = BodyPlan::SseFramedStreaming {
            max_event_bytes: SOURCE.len(),
            max_pending_bytes: SOURCE.len(),
            max_output_event_bytes: SOURCE.len(),
            expansion_ratio_numerator: ratio,
            expansion_ratio_denominator: 1,
            expansion_slack_bytes: 0,
        };
        let mut plans = lifecycle_body_plans();
        plans.attempt_response_precommit = sse_plan.clone();
        plans.accepted_response = sse_plan;
        let publications = install_publication(
            BootstrapPublicationBuilder::new(plan.0, revision)
                .route(
                    "gateway.test",
                    "/sse-expand",
                    1,
                    plain_target(address, revision),
                )?
                .body_plans(1, plans)?
                .route_accepted_filters(1, Arc::from([compiled_filter("accepted-sse-expand")]))?,
        )?;
        let selection = TestSelection::new([binding]);
        let provider = PassthroughProvider {
            accept_on_first_semantic_sse: true,
            ..PassthroughProvider::default()
        };
        let gateway = GatewayCoreLifecycle::new(
            publications,
            selection.clone(),
            provider,
            native_filter_manager(
                Arc::new(NativeFilterFacts::default()),
                &["accepted-sse-expand"],
            )?,
            PingoraConnectorAdapter::new(),
            bounded_test_limits(Duration::from_secs(5)),
        )?;
        let mut session = RecordingSession::new(
            "gateway.test",
            "/sse-expand",
            Bytes::from_static(b"prompt"),
            HttpProtocol::Http1,
        );

        if succeeds {
            assert_eq!(gateway.process(&mut session).await?, SessionReuse::Reusable);
            assert_eq!(session.response_body, EXPANDED);
            assert_eq!(
                session.response_body_writes,
                vec![
                    (Bytes::from_static(b"data:a\n\n"), false),
                    (Bytes::from_static(b"data:b\n\n"), false),
                    (Bytes::new(), true),
                ],
                "both 1:N outputs must remain atomic units of source sequence zero"
            );
        } else {
            let error = gateway.process(&mut session).await.unwrap_err();
            assert!(error.to_string().contains("SSE output exceeds"));
            assert!(
                session.response_body.is_empty(),
                "the cumulative two-unit overflow must reject before either unit is written"
            );
        }
        assert_eq!(selection.published(), [Disposition::Accept]);
        let _ = server.await??;
    }
    Ok(())
}

#[tokio::test]
async fn accepted_sse_promoted_source_keeps_one_cumulative_limit_across_callbacks()
-> Result<(), TestError> {
    const SOURCE: &[u8] = b"data:x\n\ndata:y\n\ndata:z\n\n";
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, SOURCE, false).await
    });
    let plan = PlanRevision(120);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let sse_plan = BodyPlan::SseFramedStreaming {
        max_event_bytes: 32,
        max_pending_bytes: 32,
        max_output_event_bytes: 32,
        expansion_ratio_numerator: 1,
        expansion_ratio_denominator: 1,
        expansion_slack_bytes: 0,
    };
    let mut plans = lifecycle_body_plans();
    plans.attempt_response_precommit = sse_plan.clone();
    plans.accepted_response = sse_plan;
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 120)
            .route(
                "gateway.test",
                "/sse-repeat-promoted",
                1,
                plain_target(address, 120),
            )?
            .body_plans(1, plans)?
            .route_accepted_filters(
                1,
                Arc::from([compiled_filter("accepted-sse-repeat-promoted")]),
            )?,
    )?;
    let gateway = GatewayCoreLifecycle::new(
        publications,
        TestSelection::new([binding]),
        PassthroughProvider {
            accept_on_first_semantic_sse: true,
            ..PassthroughProvider::default()
        },
        native_filter_manager(
            Arc::new(NativeFilterFacts::default()),
            &["accepted-sse-repeat-promoted"],
        )?,
        PingoraConnectorAdapter::new(),
        bounded_test_limits(Duration::from_secs(5)),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/sse-repeat-promoted",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );

    let error = gateway.process(&mut session).await.unwrap_err();
    assert!(
        error.to_string().contains("SSE output exceeds"),
        "the third callback must not receive a fresh allowance: {error}"
    );
    assert_eq!(
        session.response_body,
        Bytes::from_static(b"data:o\n\n"),
        "the first legal replacement may commit; the cumulative overflow must not"
    );
    let _ = server.await??;
    Ok(())
}

#[tokio::test]
async fn accepted_sse_tiny_promotions_hit_the_compiled_live_source_cap() -> Result<(), TestError> {
    const SOURCE: &[u8] = b"data:0\n\ndata:1\n\ndata:2\n\ndata:3\n\ndata:4\n\ndata:5\n\ndata:6\n\ndata:7\n\ndata:8\n\ndata:9\n\n";
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        serve_h1_once(socket, StatusCode::OK, SOURCE, false).await
    });
    let plan = PlanRevision(121);
    let binding = ResolvedTargetBindingId::new(plan, 1);
    let sse_plan = BodyPlan::SseFramedStreaming {
        max_event_bytes: 32,
        max_pending_bytes: 32,
        max_output_event_bytes: 32,
        expansion_ratio_numerator: 1,
        expansion_ratio_denominator: 1,
        expansion_slack_bytes: 0,
    };
    let mut plans = lifecycle_body_plans();
    plans.attempt_response_precommit = sse_plan.clone();
    plans.accepted_response = sse_plan;
    let publications = install_publication(
        BootstrapPublicationBuilder::new(plan.0, 121)
            .route(
                "gateway.test",
                "/sse-retain-many",
                1,
                plain_target(address, 121),
            )?
            .body_plans(1, plans)?
            .route_accepted_filters(1, Arc::from([compiled_filter("accepted-sse-retain-many")]))?,
    )?;
    let gateway = GatewayCoreLifecycle::new(
        publications,
        TestSelection::new([binding]),
        PassthroughProvider {
            accept_on_first_semantic_sse: true,
            ..PassthroughProvider::default()
        },
        native_filter_manager(
            Arc::new(NativeFilterFacts::default()),
            &["accepted-sse-retain-many"],
        )?,
        PingoraConnectorAdapter::new(),
        bounded_test_limits(Duration::from_secs(5)),
    )?;
    let mut session = RecordingSession::new(
        "gateway.test",
        "/sse-retain-many",
        Bytes::from_static(b"prompt"),
        HttpProtocol::Http1,
    );

    let error = gateway.process(&mut session).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("accepted live source hard limit"),
        "the tenth tiny promoted source must fail before another token allocation: {error}"
    );
    let _ = server.await??;
    Ok(())
}

#[test]
fn publication_rejects_attempt_precommit_sse_one_to_many_capability() -> Result<(), TestError> {
    let plan = PlanRevision(110);
    let mut plans = lifecycle_body_plans();
    plans.attempt_response_precommit = BodyPlan::SseFramedStreaming {
        max_event_bytes: 32,
        max_pending_bytes: 32,
        max_output_event_bytes: 32,
        expansion_ratio_numerator: 2,
        expansion_ratio_denominator: 1,
        expansion_slack_bytes: 0,
    };
    let envelope = BootstrapPublicationBuilder::new(plan.0, 110)
        .route(
            "gateway.test",
            "/attempt-sse-expand",
            1,
            plain_target("127.0.0.1:9".parse()?, 110),
        )?
        .body_plans(1, plans)?
        .attempt_response_filters(1, Arc::from([compiled_filter("attempt-sse-expand")]))?
        .build()?;
    let installer = PublicationInstaller::new();
    let error = installer
        .prepare(
            envelope,
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();
    assert_eq!(error, InstallError::IncompatibleFilterBodyPlan);
    Ok(())
}
