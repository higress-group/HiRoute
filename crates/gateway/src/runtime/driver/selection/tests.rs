use std::time::Duration;

use hiroute_gateway_core::core::execution_plan::{
    CredentialRef, PlanRevision, ResolvedTargetBindingId,
};
use hiroute_gateway_core::runtime::attempt::{
    AttemptGeneration, AttemptTransportFacts, CommitFence, RequestId,
};
use hiroute_gateway_core::runtime::driver::{
    AttemptCommitFacts, AttemptFailureClass, AttemptFailureFacts,
};

use super::*;

fn candidate(binding_local_id: u32, credential_refs: &[&str]) -> DecisionCandidateAuthority {
    DecisionCandidateAuthority {
        binding: ResolvedTargetBindingId::new(PlanRevision(1), binding_local_id),
        stable_target: ObservationLabel::new(format!("binding-{binding_local_id}")).unwrap(),
        credential_refs: credential_refs
            .iter()
            .map(|value| CredentialRef::new(*value).unwrap())
            .collect::<Vec<_>>()
            .into(),
        candidate_id: None,
        profile_digest: None,
        reason_ledger_identity: None,
        provider_profile: None,
    }
}

fn selection_request<'a>(
    _now: Instant,
    generation: u64,
    remaining_attempts: u32,
    realtime_facts: &'a RealtimeRoutingFacts,
) -> SelectionRequest<'a> {
    SelectionRequest {
        request_id: RequestId(1),
        generation: AttemptGeneration(generation),
        route_binding: ResolvedTargetBindingId::new(PlanRevision(1), 1),
        remaining_total: Duration::from_secs(1),
        remaining_attempts,
        realtime_facts,
        previous_attempts: &[],
    }
}

fn session(now: Instant, candidates: Vec<DecisionCandidateAuthority>) -> ProductionDecisionSession {
    ProductionDecisionSession {
        request_id: RequestId(1),
        route_decision_id: RouteDecisionId(1),
        candidates: candidates.into(),
        candidate_index: 0,
        credential_index: 0,
        overall_deadline: now + Duration::from_secs(10),
        hold: None,
        switch_fallback: None,
        reasoning_cleanup: Arc::new(super::super::super::ReasoningCleanup::new(None, None)),
        temporary_block_seen: false,
        probe_busy_seen: false,
        earliest_retry_at: None,
        non_temporary_block_seen: false,
        upstream_attempted: false,
        last_context_overflow: false,
        preparation_failure: None,
    }
}

fn materialization_failure(
    now: Instant,
    error_class: &str,
    retry_after: Option<Duration>,
) -> AttemptFailureFacts {
    AttemptFailureFacts {
        class: AttemptFailureClass::Materialization,
        provider: Some(ProviderClassificationFacts {
            error_class: Some(ObservationLabel::new(error_class).unwrap()),
            retryability: RetryabilityFact::Retryable,
            retry_after,
            readiness: ObservationLabel::new("materialization_failed").unwrap(),
            ..ProviderClassificationFacts::default()
        }),
        transport: AttemptTransportFacts {
            started_at: now,
            upstream_protocol: None,
            connect_elapsed: None,
            request_write_elapsed: None,
            upstream_ttfb: None,
            last_upstream_progress_at: None,
            local_read_suppressed: Duration::ZERO,
            upstream_body_bytes: 0,
            timeout: None,
        },
        request_committed: false,
        termination_reason: ObservationLabel::new("materialization").unwrap(),
    }
}

#[test]
fn admitted_pool_denial_combines_with_later_preparation_but_preserves_real_attempts() {
    for (class, expected_code) in [
        ("source_disabled", "SOURCE_DISABLED"),
        ("credential_exhausted", "CANDIDATE_PREPARATION_FAILED"),
        ("runtime_state_cooling_down", "CANDIDATE_PREPARATION_FAILED"),
    ] {
        for formal_attempt in [false, true] {
            let now = Instant::now();
            let candidates: Arc<[DecisionCandidateAuthority]> =
                vec![candidate(2, &["key-b"])].into();
            let request = DecisionSessionRequest {
                request_id: RequestId(1),
                route_binding: candidates[0].binding,
                candidate_bindings: vec![candidates[0].binding].into(),
                route_context: ProductionRouteContext {
                    ingress: crate::server::request_plan::IngressProtocol::Responses,
                    context_holds: Arc::new(ContextHoldStore::default()),
                    switch_fallback: None,
                    hold_completion: None,
                    reasoning_cleanup: Arc::new(super::super::super::ReasoningCleanup::new(
                        None, None,
                    )),
                    admission_preparation_failure: Some(PreparationFailure::SourceDisabled),
                },
                overall_deadline: now + Duration::from_secs(10),
                max_attempts: 3,
            };
            let mut session = ProductionSelection
                .begin_authorized_session(request, candidates)
                .unwrap();
            let realtime = RealtimeRoutingFacts::default();
            let mut selected = session
                .select_next(selection_request(now, 1, 3, &realtime))
                .unwrap()
                .unwrap();
            if formal_attempt {
                selected.attempt_id = AttemptId(1);
            }
            session
                .decide_failure(&selected, &materialization_failure(now, class, None))
                .unwrap();
            let reply = session.exhaustion_local_reply(now).unwrap();
            if formal_attempt {
                assert!(
                    reply.is_none(),
                    "an earlier admission denial cannot rewrite an upstream reply"
                );
            } else {
                let reply = reply.unwrap();
                assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
                assert!(!reply.headers.contains_key(RETRY_AFTER));
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&reply.body).unwrap()["code"],
                    expected_code
                );
            }
        }
    }
}

#[test]
fn uniform_provisional_denials_retain_the_closed_code_without_retry_or_attempts() {
    for (class, code) in [
        ("source_disabled", "SOURCE_DISABLED"),
        ("credential_disabled", "CREDENTIAL_DISABLED"),
        ("credential_exhausted", "CREDENTIAL_UNAVAILABLE"),
        (
            "private_unrecognized_detail",
            "CANDIDATE_PREPARATION_FAILED",
        ),
    ] {
        let now = Instant::now();
        let realtime = RealtimeRoutingFacts::default();
        let mut session = session(
            now,
            vec![candidate(1, &["key-a"]), candidate(2, &["key-b"])],
        );
        for index in 0..2 {
            let selected = session
                .select_next(selection_request(now, 1, 3, &realtime))
                .unwrap()
                .unwrap();
            assert_eq!(selected.attempt_id, AttemptId(0));
            assert_eq!(
                session
                    .decide_failure(&selected, &materialization_failure(now, class, None))
                    .unwrap(),
                if index == 0 {
                    Disposition::Continue
                } else {
                    Disposition::Terminate
                }
            );
        }
        let reply = session.exhaustion_local_reply(now).unwrap().unwrap();
        assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
        assert_eq!(reply.headers[CONTENT_TYPE], "application/json");
        assert!(!reply.headers.contains_key(RETRY_AFTER));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&reply.body).unwrap(),
            serde_json::json!({
                "schema_version": "hiroute.gateway.error/v1", "code": code,
                "phase": if code == "CANDIDATE_PREPARATION_FAILED" { "candidate_preparation" } else { "credential_lease" }
            })
        );
    }
}

#[test]
fn provisional_reply_does_not_override_a_previous_formal_attempt() {
    let now = Instant::now();
    let realtime = RealtimeRoutingFacts::default();
    let mut session = session(
        now,
        vec![candidate(1, &["key-a"]), candidate(2, &["key-b"])],
    );
    let mut first = session
        .select_next(selection_request(now, 1, 3, &realtime))
        .unwrap()
        .unwrap();
    first.attempt_id = AttemptId(1);
    assert_eq!(
        session
            .decide_failure(
                &first,
                &materialization_failure(now, "materialization_retryable", None)
            )
            .unwrap(),
        Disposition::Continue
    );
    let second = session
        .select_next(selection_request(now, 2, 2, &realtime))
        .unwrap()
        .unwrap();
    assert_eq!(
        session
            .decide_failure(
                &second,
                &materialization_failure(now, "source_disabled", None)
            )
            .unwrap(),
        Disposition::Terminate
    );
    assert!(session.exhaustion_local_reply(now).unwrap().is_none());
}

#[test]
fn overflow_reply_requires_a_real_last_response_and_cannot_outlive_deadline_or_later_failure() {
    let now = Instant::now();
    let realtime = RealtimeRoutingFacts::default();
    let failure = materialization_failure(now, "source_disabled", None);
    let overflow = ProviderClassificationFacts {
        error_class: Some(ObservationLabel::new("context_length").unwrap()),
        retryability: RetryabilityFact::Retryable,
        ..ProviderClassificationFacts::default()
    };
    for promoted in [false, true] {
        let mut session = session(now, vec![candidate(1, &["a"]), candidate(2, &["b"])]);
        let mut selected = session
            .select_next(selection_request(now, 1, 2, &realtime))
            .unwrap()
            .unwrap();
        if promoted {
            selected.attempt_id = AttemptId(1);
        }
        assert_eq!(
            session
                .decide(&selected, &overflow, &failure.transport)
                .unwrap(),
            Disposition::Continue
        );
        assert_eq!(
            session.exhaustion_local_reply(now).unwrap().is_some(),
            promoted
        );
        assert!(
            session
                .exhaustion_local_reply(session.overall_deadline)
                .unwrap()
                .is_none()
        );
        let next = session
            .select_next(selection_request(now, 2, 1, &realtime))
            .unwrap()
            .unwrap();
        session.decide_failure(&next, &failure).unwrap();
        assert!(!session.last_context_overflow);
        if promoted {
            assert!(session.exhaustion_local_reply(now).unwrap().is_none());
        }
    }
}

#[test]
fn retryable_quota_advances_exact_credential_before_binding() {
    let now = Instant::now();
    let realtime_facts = RealtimeRoutingFacts::default();
    let transport = AttemptTransportFacts {
        started_at: now,
        upstream_protocol: None,
        connect_elapsed: None,
        request_write_elapsed: None,
        upstream_ttfb: None,
        last_upstream_progress_at: None,
        local_read_suppressed: Duration::ZERO,
        upstream_body_bytes: 0,
        timeout: None,
    };
    let mut session = ProductionDecisionSession {
        request_id: RequestId(1),
        route_decision_id: RouteDecisionId(1),
        candidates: vec![
            candidate(1, &["credential-a", "credential-b"]),
            candidate(2, &["credential-c"]),
        ]
        .into(),
        candidate_index: 0,
        credential_index: 0,
        overall_deadline: now + Duration::from_secs(1),
        hold: None,
        switch_fallback: None,
        reasoning_cleanup: Arc::new(super::super::super::ReasoningCleanup::new(None, None)),
        temporary_block_seen: false,
        probe_busy_seen: false,
        earliest_retry_at: None,
        non_temporary_block_seen: false,
        upstream_attempted: false,
        last_context_overflow: false,
        preparation_failure: None,
    };
    let first = session
        .select_next(selection_request(now, 1, 3, &realtime_facts))
        .unwrap()
        .unwrap();
    assert_eq!(first.binding.local_id(), 1);
    assert_eq!(first.credential_ref.as_str(), "credential-a");

    let quota = ProviderClassificationFacts {
        error_class: Some(ObservationLabel::new("quota").unwrap()),
        retryability: RetryabilityFact::Retryable,
        readiness: ObservationLabel::new("rejected").unwrap(),
        ..ProviderClassificationFacts::default()
    };
    assert_eq!(
        session.decide(&first, &quota, &transport).unwrap(),
        Disposition::Continue
    );
    let second = session
        .select_next(selection_request(now, 2, 2, &realtime_facts))
        .unwrap()
        .unwrap();
    assert_eq!(second.binding.local_id(), 1);
    assert_eq!(second.credential_ref.as_str(), "credential-b");

    assert_eq!(
        session.decide(&second, &quota, &transport).unwrap(),
        Disposition::Continue
    );
    let third = session
        .select_next(selection_request(now, 3, 1, &realtime_facts))
        .unwrap()
        .unwrap();
    assert_eq!(third.binding.local_id(), 2);
    assert_eq!(third.credential_ref.as_str(), "credential-c");
}

#[test]
fn switched_candidate_precommit_400_falls_back_to_previous_once() {
    let now = Instant::now();
    let realtime_facts = RealtimeRoutingFacts::default();
    let mut session = session(
        now,
        vec![candidate(1, &["new-key"]), candidate(2, &["original-key"])],
    );
    session.switch_fallback = Some(ResolvedTargetBindingId::new(PlanRevision(1), 2));
    let mut selected = session
        .select_next(selection_request(now, 1, 2, &realtime_facts))
        .unwrap()
        .unwrap();
    // The core promotes the candidate after pre-exchange materialization.
    selected.attempt_id = AttemptId(1);
    let rejected = ProviderClassificationFacts {
        error_class: Some(ObservationLabel::new("permanent_client").unwrap()),
        retryability: RetryabilityFact::NonRetryable,
        http_status: Some(StatusCode::BAD_REQUEST),
        ..ProviderClassificationFacts::default()
    };
    assert_eq!(
        session
            .decide(
                &selected,
                &rejected,
                &AttemptTransportFacts {
                    started_at: now,
                    upstream_protocol: None,
                    connect_elapsed: None,
                    request_write_elapsed: None,
                    upstream_ttfb: None,
                    last_upstream_progress_at: None,
                    local_read_suppressed: Duration::ZERO,
                    upstream_body_bytes: 0,
                    timeout: None,
                }
            )
            .unwrap(),
        Disposition::Continue
    );
    let original = session
        .select_next(selection_request(now, 2, 1, &realtime_facts))
        .unwrap()
        .unwrap();
    assert_eq!(original.binding.local_id(), 2);
    assert_eq!(
        session
            .decide(
                &original,
                &rejected,
                &AttemptTransportFacts {
                    started_at: now,
                    upstream_protocol: None,
                    connect_elapsed: None,
                    request_write_elapsed: None,
                    upstream_ttfb: None,
                    last_upstream_progress_at: None,
                    local_read_suppressed: Duration::ZERO,
                    upstream_body_bytes: 0,
                    timeout: None,
                }
            )
            .unwrap(),
        Disposition::Terminate
    );
}

#[test]
fn explicit_history_rejection_retries_only_eligible_first_candidate_once() {
    let now = Instant::now();
    let facts = RealtimeRoutingFacts::default();
    let mut session = session(now, vec![candidate(1, &["b"]), candidate(2, &["a"])]);
    let binding = session.candidates[0].binding;
    session.reasoning_cleanup = Arc::new(super::super::ReasoningCleanup::new(
        Some((binding, 3)),
        None,
    ));
    let rejected = ProviderClassificationFacts {
        error_class: Some(ObservationLabel::new("reasoning_history").unwrap()),
        retryability: RetryabilityFact::NonRetryable,
        http_status: Some(StatusCode::BAD_REQUEST),
        ..ProviderClassificationFacts::default()
    };
    let transport = AttemptTransportFacts {
        started_at: now,
        upstream_protocol: None,
        connect_elapsed: None,
        request_write_elapsed: None,
        upstream_ttfb: None,
        last_upstream_progress_at: None,
        local_read_suppressed: Duration::ZERO,
        upstream_body_bytes: 0,
        timeout: None,
    };
    let first = session
        .select_next(selection_request(now, 1, 3, &facts))
        .unwrap()
        .unwrap();
    assert_eq!(session.reasoning_cleanup.prefix_for(binding), None);
    assert_eq!(
        session.decide(&first, &rejected, &transport).unwrap(),
        Disposition::Continue
    );
    let retry = session
        .select_next(selection_request(now, 2, 2, &facts))
        .unwrap()
        .unwrap();
    assert_eq!(retry.binding, binding);
    assert_eq!(session.reasoning_cleanup.prefix_for(binding), Some(3));
    assert_eq!(
        session.decide(&retry, &rejected, &transport).unwrap(),
        Disposition::Continue
    );
    let fallback = session
        .select_next(selection_request(now, 3, 1, &facts))
        .unwrap()
        .unwrap();
    assert_ne!(fallback.binding, binding);
    assert_eq!(session.reasoning_cleanup.prefix_for(fallback.binding), None);
    assert_eq!(
        session.decide(&fallback, &rejected, &transport).unwrap(),
        Disposition::Terminate
    );
    assert!(
        session
            .select_next(selection_request(now, 4, 0, &facts))
            .unwrap()
            .is_none()
    );
}

#[test]
fn history_error_without_context_break_authorization_does_not_enable_cleanup() {
    let now = Instant::now();
    let facts = RealtimeRoutingFacts::default();
    let mut session = session(now, vec![candidate(1, &["a"]), candidate(2, &["b"])]);
    let selected = session
        .select_next(selection_request(now, 1, 3, &facts))
        .unwrap()
        .unwrap();
    let mut failure = materialization_failure(now, "reasoning_history", None);
    failure.provider.as_mut().unwrap().retryability = RetryabilityFact::NonRetryable;
    assert!(!session.reasoning_cleanup.consume(selected.binding));
    assert_eq!(session.reasoning_cleanup.prefix_for(selected.binding), None);
    assert_eq!(
        session.decide_failure(&selected, &failure).unwrap(),
        Disposition::Terminate
    );
}

#[test]
fn unknown_or_nonretryable_materialization_never_advances_selection() {
    let now = Instant::now();
    let selected = SelectedGatewayAttempt {
        request_id: RequestId(1),
        attempt_id: AttemptId(0),
        generation: AttemptGeneration(1),
        binding: ResolvedTargetBindingId::new(PlanRevision(1), 1),
        credential_ref: CredentialRef::new("credential").unwrap(),
        route_decision_id: RouteDecisionId(1),
        budget: AttemptBudgetGrant {
            issued_at: now,
            allocated: Duration::from_secs(1),
            deadline: now + Duration::from_secs(1),
        },
    };
    let mut session = ProductionDecisionSession {
        request_id: RequestId(1),
        route_decision_id: RouteDecisionId(1),
        candidates: Arc::new([]),
        candidate_index: 0,
        credential_index: 0,
        overall_deadline: now + Duration::from_secs(1),
        hold: None,
        switch_fallback: None,
        reasoning_cleanup: Arc::new(super::super::super::ReasoningCleanup::new(None, None)),
        temporary_block_seen: false,
        probe_busy_seen: false,
        earliest_retry_at: None,
        non_temporary_block_seen: false,
        upstream_attempted: false,
        last_context_overflow: false,
        preparation_failure: None,
    };
    for retryability in [RetryabilityFact::Unknown, RetryabilityFact::NonRetryable] {
        let failure = AttemptFailureFacts {
            class: AttemptFailureClass::Materialization,
            provider: Some(ProviderClassificationFacts {
                retryability,
                readiness: ObservationLabel::new("materialization_failed").unwrap(),
                ..ProviderClassificationFacts::default()
            }),
            transport: AttemptTransportFacts {
                started_at: now,
                upstream_protocol: None,
                connect_elapsed: None,
                request_write_elapsed: None,
                upstream_ttfb: None,
                last_upstream_progress_at: None,
                local_read_suppressed: Duration::ZERO,
                upstream_body_bytes: 0,
                timeout: None,
            },
            request_committed: false,
            termination_reason: ObservationLabel::new("materialization").unwrap(),
        };
        assert_eq!(
            session.decide_failure(&selected, &failure).unwrap(),
            Disposition::Terminate
        );
        assert_eq!(session.candidate_index, 0);
        assert_eq!(session.credential_index, 0);
    }
}

#[test]
fn switched_provisional_nonretryable_failure_does_not_call_previous_model() {
    let now = Instant::now();
    let realtime_facts = RealtimeRoutingFacts::default();
    let mut session = session(
        now,
        vec![candidate(1, &["new-key"]), candidate(2, &["original-key"])],
    );
    session.switch_fallback = Some(ResolvedTargetBindingId::new(PlanRevision(1), 2));
    let selected = session
        .select_next(selection_request(now, 1, 2, &realtime_facts))
        .unwrap()
        .unwrap();
    assert_eq!(selected.attempt_id, AttemptId(0));

    let mut failure = materialization_failure(now, "materialization_fail_closed", None);
    failure.provider.as_mut().unwrap().retryability = RetryabilityFact::NonRetryable;
    assert_eq!(
        session.decide_failure(&selected, &failure).unwrap(),
        Disposition::Terminate
    );
    assert_eq!(session.candidate_index, 0);
}

#[test]
fn switched_promoted_preexchange_failure_does_not_call_previous_model() {
    let now = Instant::now();
    let realtime_facts = RealtimeRoutingFacts::default();
    let mut session = session(
        now,
        vec![candidate(1, &["new-key"]), candidate(2, &["original-key"])],
    );
    session.switch_fallback = Some(ResolvedTargetBindingId::new(PlanRevision(1), 2));
    let mut selected = session
        .select_next(selection_request(now, 1, 2, &realtime_facts))
        .unwrap()
        .unwrap();
    selected.attempt_id = AttemptId(1);

    let mut failure = materialization_failure(now, "preexchange_filter_failure", None);
    failure.provider.as_mut().unwrap().retryability = RetryabilityFact::NonRetryable;
    assert!(!failure.request_committed);
    assert_eq!(
        session.decide_failure(&selected, &failure).unwrap(),
        Disposition::Terminate
    );
    assert_eq!(session.candidate_index, 0);
}

#[test]
fn pure_runtime_state_exhaustion_returns_retryable_503_and_probe_busy_wins() {
    let now = Instant::now();
    let realtime = RealtimeRoutingFacts::default();
    let mut session = session(
        now,
        vec![
            candidate(1, &["credential-a"]),
            candidate(2, &["credential-b"]),
        ],
    );
    let first = session
        .select_next(selection_request(now, 1, 3, &realtime))
        .unwrap()
        .unwrap();
    assert_eq!(
        session
            .decide_failure(
                &first,
                &materialization_failure(
                    now,
                    "runtime_state_cooling_down",
                    Some(Duration::from_millis(1_500)),
                ),
            )
            .unwrap(),
        Disposition::Continue
    );
    let second = session
        .select_next(selection_request(now, 1, 3, &realtime))
        .unwrap()
        .unwrap();
    assert_eq!(
        session
            .decide_failure(
                &second,
                &materialization_failure(
                    now,
                    "runtime_state_probe_busy",
                    Some(Duration::from_secs(1)),
                ),
            )
            .unwrap(),
        Disposition::Continue
    );
    assert!(
        session
            .select_next(selection_request(now, 1, 3, &realtime))
            .unwrap()
            .is_none()
    );
    let reply = session
        .exhaustion_local_reply(now)
        .unwrap()
        .expect("pure temporary exhaustion has a retry response");
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.headers.get(RETRY_AFTER).unwrap(), "1");
    assert_eq!(reply.headers.get(CONTENT_TYPE).unwrap(), "application/json");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&reply.body).unwrap(),
        serde_json::json!({
            "schema_version": "hiroute.gateway.error/v1",
            "code": "RECOVERY_PROBE_IN_PROGRESS",
            "phase": "runtime_state",
            "retry_after_seconds": 1,
        })
    );
}

#[test]
fn cooling_exhaustion_uses_earliest_deadline_and_rounds_up() {
    assert_eq!(retry_after_seconds(Duration::from_nanos(1)), 1);
    assert_eq!(retry_after_seconds(Duration::from_nanos(1_000_000_001)), 2);

    let now = Instant::now();
    let realtime = RealtimeRoutingFacts::default();
    let mut session = session(
        now,
        vec![
            candidate(1, &["credential-a"]),
            candidate(2, &["credential-b"]),
        ],
    );
    let first = session
        .select_next(selection_request(now, 1, 3, &realtime))
        .unwrap()
        .unwrap();
    assert_eq!(
        session
            .decide_failure(
                &first,
                &materialization_failure(
                    now,
                    "runtime_state_cooling_down",
                    Some(Duration::from_secs(12)),
                ),
            )
            .unwrap(),
        Disposition::Continue
    );
    let second = session
        .select_next(selection_request(now, 1, 3, &realtime))
        .unwrap()
        .unwrap();
    assert_eq!(
        session
            .decide_failure(
                &second,
                &materialization_failure(
                    now,
                    "runtime_state_cooling_down",
                    Some(Duration::from_millis(3_200)),
                ),
            )
            .unwrap(),
        Disposition::Continue
    );
    assert!(
        session
            .select_next(selection_request(now, 1, 3, &realtime))
            .unwrap()
            .is_none()
    );
    let reply = session
        .exhaustion_local_reply(now)
        .unwrap()
        .expect("pure cooldown exhaustion has a retry response");
    assert_eq!(reply.headers.get(RETRY_AFTER).unwrap(), "4");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&reply.body).unwrap(),
        serde_json::json!({
            "schema_version": "hiroute.gateway.error/v1",
            "code": "CANDIDATES_COOLING_DOWN",
            "phase": "runtime_state",
            "retry_after_seconds": 4,
        })
    );
}

#[test]
fn mixed_runtime_state_exhaustion_returns_a_generic_typed_preparation_failure() {
    let now = Instant::now();
    let realtime = RealtimeRoutingFacts::default();
    let mut session = session(
        now,
        vec![
            candidate(1, &["credential-a"]),
            candidate(2, &["credential-b"]),
        ],
    );
    let first = session
        .select_next(selection_request(now, 1, 3, &realtime))
        .unwrap()
        .unwrap();
    session
        .decide_failure(
            &first,
            &materialization_failure(
                now,
                "runtime_state_cooling_down",
                Some(Duration::from_secs(2)),
            ),
        )
        .unwrap();
    let second = session
        .select_next(selection_request(now, 1, 3, &realtime))
        .unwrap()
        .unwrap();
    assert_eq!(
        session
            .decide_failure(
                &second,
                &materialization_failure(now, "runtime_state_disabled", None),
            )
            .unwrap(),
        Disposition::Terminate
    );
    let reply = session.exhaustion_local_reply(now).unwrap().unwrap();
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert!(!reply.headers.contains_key(RETRY_AFTER));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&reply.body).unwrap(),
        serde_json::json!({"schema_version": "hiroute.gateway.error/v1",
            "code": "CANDIDATE_PREPARATION_FAILED", "phase": "candidate_preparation"})
    );
}

#[test]
fn hold_success_requires_full_accepted_eos_downstream_and_cleanup() {
    let now = Instant::now();
    let accepted = CompletedAttemptObservation {
        route_decision_id: RouteDecisionId(1),
        attempt_id: AttemptId(1),
        generation: AttemptGeneration(1),
        binding: ResolvedTargetBindingId::new(PlanRevision(1), 1),
        credential_ref: CredentialRef::new("credential").unwrap(),
        budget: AttemptBudgetGrant {
            issued_at: now,
            allocated: Duration::from_secs(1),
            deadline: now + Duration::from_secs(1),
        },
        routing_facts: RealtimeRoutingFacts::default(),
        provider: Some(ProviderClassificationFacts {
            model_event: Some(ObservationLabel::new("response_complete").unwrap()),
            ..ProviderClassificationFacts::default()
        }),
        failure: None,
        transport: AttemptTransportFacts {
            started_at: now,
            upstream_protocol: None,
            connect_elapsed: None,
            request_write_elapsed: None,
            upstream_ttfb: None,
            last_upstream_progress_at: None,
            local_read_suppressed: Duration::ZERO,
            upstream_body_bytes: 1,
            timeout: None,
        },
        disposition: Disposition::Accept,
        commits: AttemptCommitFacts {
            upstream_request: CommitFence::WriteConfirmed,
            downstream_headers: CommitFence::WriteConfirmed,
            downstream_semantic: CommitFence::WriteConfirmed,
        },
        stream: AttemptStreamOutcome::CompletedEos,
        downstream: AttemptDownstreamOutcome::Completed,
        cleanup: AttemptCleanupOutcome::Completed,
        ended_at: now,
        termination_reason: AttemptTerminationReason::AcceptedEos,
    };
    assert!(is_complete_accept(&accepted));
    let mut interrupted = accepted.clone();
    interrupted.stream = AttemptStreamOutcome::StreamStartedNoRetry;
    assert!(!is_complete_accept(&interrupted));
    let mut downstream_failed = accepted.clone();
    downstream_failed.downstream = AttemptDownstreamOutcome::Failed;
    assert!(!is_complete_accept(&downstream_failed));
    let mut cleanup_failed = accepted.clone();
    cleanup_failed.cleanup = AttemptCleanupOutcome::Failed;
    assert!(!is_complete_accept(&cleanup_failed));
    let mut incomplete = accepted.clone();
    incomplete.provider.as_mut().unwrap().model_event =
        Some(ObservationLabel::new("response_incomplete").unwrap());
    assert!(!is_complete_accept(&incomplete));
    let mut failed = accepted.clone();
    failed.provider.as_mut().unwrap().model_event =
        Some(ObservationLabel::new("response_failed").unwrap());
    assert!(!is_complete_accept(&failed));
    let mut missing_terminal = accepted.clone();
    missing_terminal.provider.as_mut().unwrap().model_event = None;
    assert!(!is_complete_accept(&missing_terminal));
    let mut terminated = accepted;
    terminated.disposition = Disposition::Terminate;
    assert!(!is_complete_accept(&terminated));
}
