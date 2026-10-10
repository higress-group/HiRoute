use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hiroute_gateway_core::core::execution_plan::{
    CredentialRef, DEFAULT_OVERALL_REQUEST_TIMEOUT, PlanRevision, ResolvedTargetBindingId,
};
use hiroute_gateway_core::core::filter::CompiledFilterDescriptor;
use hiroute_gateway_core::core::publication::{PrepareOutcome, PublicationInstaller};
use hiroute_gateway_core::runtime::attempt::{
    AcceptBlockedReason, AttemptGeneration, AttemptId, AttemptTransportFacts, Disposition,
    PublishedDisposition, RequestId,
};
use hiroute_gateway_core::runtime::body::BodyPlan;
use hiroute_gateway_core::runtime::driver::{
    AttemptBudgetGrant, ClassifiedAttemptResult, DecisionSessionPort, DecisionSessionRequest,
    FactConfidence, FactScope, FactSubject, FreshRoutingFact, ObservationLabel,
    ProviderClassificationFacts, RealtimeRoutingFact, RealtimeRoutingFacts, RetryabilityFact,
    RouteDecisionId, RoutingFactState, RoutingFactsSnapshotId, SelectedGatewayAttempt,
    SelectionPublicationPort, SelectionRequest, UsageDimension, UsageFact, UsageProvenance,
};
use hiroute_gateway_core::test_support::{
    BootstrapBodyPlans, BootstrapPublicationBuilder, plain_target,
};
use tokio_util::sync::CancellationToken;

fn install(
    envelope: hiroute_gateway_core::core::publication::CompiledGatewayPublicationEnvelope,
) -> PublicationInstaller {
    let installer = PublicationInstaller::new();
    let cancellation = CancellationToken::new();
    let deadline = Instant::now() + Duration::from_secs(1);
    let PrepareOutcome::Prepared(prepared) = installer
        .prepare(envelope, &cancellation, deadline)
        .expect("publication prepares")
    else {
        panic!("fresh installer cannot report a duplicate publication");
    };
    installer
        .publish(prepared, &cancellation, deadline)
        .expect("publication installs");
    installer
}

fn route_plans(logical_limit: usize, accepted_limit: usize) -> BootstrapBodyPlans {
    BootstrapBodyPlans {
        logical_request: BodyPlan::BufferedTransform {
            max_body_bytes: logical_limit,
        },
        attempt_request: BodyPlan::StreamingReplay {
            max_chunk_bytes: 1024,
            max_replay_bytes: 64 * 1024,
        },
        attempt_response_precommit: BodyPlan::PassThrough {
            max_chunk_bytes: 2048,
        },
        accepted_response: BodyPlan::BufferedTransform {
            max_body_bytes: accepted_limit,
        },
    }
}

#[test]
fn matched_route_owns_both_downstream_plans_across_fallback() {
    let revision = PlanRevision(301);
    let route_a = ResolvedTargetBindingId::new(revision, 1);
    let fallback = ResolvedTargetBindingId::new(revision, 2);
    let envelope = BootstrapPublicationBuilder::new(revision.0, 1)
        .route(
            "a.example",
            "/",
            1,
            plain_target("127.0.0.1:18081".parse().unwrap(), 1),
        )
        .unwrap()
        .route(
            "b.example",
            "/",
            2,
            plain_target("127.0.0.1:18082".parse().unwrap(), 2),
        )
        .unwrap()
        .body_plans(1, route_plans(11, 101))
        .unwrap()
        .body_plans(2, route_plans(22, 202))
        .unwrap()
        .route_accepted_filters(
            1,
            Arc::from([CompiledFilterDescriptor::new("route-a-accepted", 1).unwrap()]),
        )
        .unwrap()
        .route_accepted_filters(
            2,
            Arc::from([CompiledFilterDescriptor::new("route-b-accepted", 1).unwrap()]),
        )
        .unwrap()
        .route_candidates(1, [1, 2])
        .unwrap()
        .build()
        .unwrap();
    let installer = install(envelope);

    let mut request = installer.bind_request().unwrap();
    let matched = request
        .ingress_plan()
        .unwrap()
        .routes
        .iter()
        .find(|route| route.binding == route_a)
        .unwrap()
        .clone();
    request.bind_route(&matched).unwrap();
    let _request_configs = request.take_request_configs().unwrap();
    let logical = request.take_logical_request().unwrap();
    assert!(matches!(
        logical.plan().body_plan,
        BodyPlan::BufferedTransform { max_body_bytes: 11 }
    ));
    drop(logical);

    let fallback_attempt = request.resolve_attempt(fallback).unwrap();
    assert_eq!(fallback_attempt.plan().binding, fallback);
    drop(fallback_attempt);

    let accepted = request.take_accepted_response().unwrap();
    assert!(matches!(
        accepted.plan().body_plan,
        BodyPlan::BufferedTransform {
            max_body_bytes: 101
        }
    ));
    assert_eq!(accepted.plan().filters[0].id.as_ref(), "route-a-accepted");
}

#[test]
fn route_timeout_defaults_to_one_hour_and_is_not_attempt_owned() {
    let revision = PlanRevision(302);
    let envelope = BootstrapPublicationBuilder::new(revision.0, 1)
        .route(
            "timeout.example",
            "/",
            1,
            plain_target("127.0.0.1:18083".parse().unwrap(), 3),
        )
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        envelope.ingress_plan_handle.routes[0]
            .request_plan
            .overall_request_timeout,
        Duration::from_secs(60 * 60)
    );
    assert_eq!(
        envelope.ingress_plan_handle.routes[0]
            .request_plan
            .overall_request_timeout,
        DEFAULT_OVERALL_REQUEST_TIMEOUT
    );
    assert_eq!(
        hiroute_gateway_core::runtime::driver::GatewayCoreLifecycleLimits::default()
            .bootstrap_hard_cap,
        None
    );
}

#[derive(Clone)]
struct ContractSelection;

struct ContractDecisionSession {
    route_decision_id: RouteDecisionId,
    candidates: VecDeque<ResolvedTargetBindingId>,
    overall_deadline: Instant,
    next_attempt_id: u64,
    fact_revision: u64,
}

impl DecisionSessionPort for ContractDecisionSession {
    fn route_decision_id(&self) -> RouteDecisionId {
        self.route_decision_id
    }

    fn snapshot_realtime_facts(&mut self, now: Instant) -> Result<RealtimeRoutingFacts, Arc<str>> {
        self.fact_revision += 1;
        let binding = *self
            .candidates
            .front()
            .ok_or_else(|| Arc::from("no fact subject candidate"))?;
        Ok(RealtimeRoutingFacts {
            snapshot_id: Some(RoutingFactsSnapshotId(self.fact_revision)),
            facts: Arc::from([FreshRoutingFact {
                source: ObservationLabel::new(format!("health-refresh-{}", self.fact_revision))?,
                subject: FactSubject {
                    plan_revision: binding.plan_revision(),
                    binding,
                    stable_target: ObservationLabel::new(format!("target-{}", binding.local_id()))?,
                    provider: None,
                    model: None,
                    entitlement: None,
                    credential: None,
                },
                scope: FactScope::Candidate,
                confidence: FactConfidence::Measured,
                observed_at: now,
                valid_until: now + Duration::from_secs(1),
                state: RoutingFactState::Known(RealtimeRoutingFact::HealthScore {
                    basis_points: 9_500,
                }),
            }]),
        })
    }

    fn select_next(
        &mut self,
        request: SelectionRequest<'_>,
    ) -> Result<Option<SelectedGatewayAttempt>, Arc<str>> {
        let Some(binding) = self.candidates.pop_front() else {
            return Ok(None);
        };
        let attempt_id = AttemptId(self.next_attempt_id);
        self.next_attempt_id += 1;
        let issued_at = Instant::now();
        let allocated = self
            .overall_deadline
            .saturating_duration_since(issued_at)
            .min(request.remaining_total);
        Ok(Some(SelectedGatewayAttempt {
            request_id: request.request_id,
            attempt_id,
            generation: request.generation,
            binding,
            credential_ref: CredentialRef::new(format!("credential-{}", binding.local_id()))
                .map_err(|error| Arc::from(error.to_string()))?,
            route_decision_id: self.route_decision_id,
            budget: AttemptBudgetGrant {
                issued_at,
                allocated,
                deadline: self.overall_deadline,
            },
        }))
    }

    fn decide(
        &mut self,
        _selected: &SelectedGatewayAttempt,
        facts: &ProviderClassificationFacts,
        _transport: &AttemptTransportFacts,
    ) -> Result<Disposition, Arc<str>> {
        Ok(if facts.retryability == RetryabilityFact::Retryable {
            Disposition::Continue
        } else {
            Disposition::Accept
        })
    }

    fn decide_failure(
        &mut self,
        _selected: &SelectedGatewayAttempt,
        _failure: &hiroute_gateway_core::runtime::driver::AttemptFailureFacts,
    ) -> Result<Disposition, Arc<str>> {
        Ok(if !self.candidates.is_empty() {
            Disposition::Continue
        } else {
            Disposition::Terminate
        })
    }

    fn replace_blocked_accept(
        &mut self,
        _selected: &SelectedGatewayAttempt,
        _reason: AcceptBlockedReason,
    ) -> Result<Disposition, Arc<str>> {
        Ok(Disposition::Terminate)
    }

    fn observe_published(&mut self, _disposition: &PublishedDisposition) -> Result<(), Arc<str>> {
        Ok(())
    }

    fn observe_completed(
        &mut self,
        _observation: &hiroute_gateway_core::runtime::driver::CompletedAttemptObservation,
    ) -> Result<(), Arc<str>> {
        Ok(())
    }
}

impl SelectionPublicationPort<Arc<str>> for ContractSelection {
    type Session = ContractDecisionSession;

    fn begin_session(
        &self,
        request: DecisionSessionRequest<Arc<str>>,
    ) -> Result<Self::Session, Arc<str>> {
        Ok(ContractDecisionSession {
            route_decision_id: RouteDecisionId(request.request_id.0 + 10_000),
            candidates: request.candidate_bindings.iter().copied().collect(),
            overall_deadline: request.overall_deadline,
            next_attempt_id: 1,
            fact_revision: 0,
        })
    }
}

fn transport_facts() -> AttemptTransportFacts {
    AttemptTransportFacts {
        started_at: Instant::now(),
        upstream_protocol: None,
        connect_elapsed: Some(Duration::from_millis(1)),
        request_write_elapsed: Some(Duration::from_millis(2)),
        upstream_ttfb: Some(Duration::from_millis(3)),
        last_upstream_progress_at: Some(Instant::now()),
        local_read_suppressed: Duration::ZERO,
        upstream_body_bytes: 0,
        timeout: None,
    }
}

#[test]
fn realtime_facts_and_usage_preserve_subject_state_and_provenance() {
    let now = Instant::now();
    let revision = PlanRevision(303);
    let first = ResolvedTargetBindingId::new(revision, 1);
    let second = ResolvedTargetBindingId::new(revision, 2);
    let subject = |binding, target: &'static str| FactSubject {
        plan_revision: revision,
        binding,
        stable_target: ObservationLabel::new(target).unwrap(),
        provider: Some(ObservationLabel::new("provider-a").unwrap()),
        model: Some(ObservationLabel::new("model-a").unwrap()),
        entitlement: None,
        credential: None,
    };
    let source = ObservationLabel::new("realtime-control-plane").unwrap();
    let first_credential = CredentialRef::new("credential-a").unwrap();
    let second_credential = CredentialRef::new("credential-b").unwrap();
    let facts = RealtimeRoutingFacts {
        snapshot_id: Some(RoutingFactsSnapshotId(91)),
        facts: Arc::from([
            FreshRoutingFact {
                source: source.clone(),
                subject: subject(first, "target-a"),
                scope: FactScope::Provider,
                confidence: FactConfidence::Reported,
                observed_at: now,
                valid_until: now + Duration::from_secs(10),
                state: RoutingFactState::Known(RealtimeRoutingFact::QuotaRemaining { units: 0 }),
            },
            FreshRoutingFact {
                source: source.clone(),
                subject: subject(second, "target-b"),
                scope: FactScope::Model,
                confidence: FactConfidence::Measured,
                observed_at: now,
                valid_until: now + Duration::from_secs(10),
                state: RoutingFactState::Known(RealtimeRoutingFact::ComplianceEligible {
                    eligible: false,
                }),
            },
            FreshRoutingFact {
                source: source.clone(),
                subject: subject(first, "target-a"),
                scope: FactScope::Candidate,
                confidence: FactConfidence::Estimated,
                observed_at: now,
                valid_until: now + Duration::from_secs(10),
                state: RoutingFactState::Unknown,
            },
            FreshRoutingFact {
                source: source.clone(),
                subject: subject(second, "target-b"),
                scope: FactScope::Candidate,
                confidence: FactConfidence::Reported,
                observed_at: now - Duration::from_secs(20),
                valid_until: now - Duration::from_secs(10),
                state: RoutingFactState::Stale {
                    last_known: Some(RealtimeRoutingFact::HealthScore {
                        basis_points: 8_000,
                    }),
                },
            },
            FreshRoutingFact {
                source: source.clone(),
                subject: FactSubject {
                    credential: Some(first_credential.clone()),
                    ..subject(first, "target-a")
                },
                scope: FactScope::Credential,
                confidence: FactConfidence::Reported,
                observed_at: now,
                valid_until: now + Duration::from_secs(10),
                state: RoutingFactState::Known(RealtimeRoutingFact::CooldownUntil {
                    until: now + Duration::from_secs(2),
                }),
            },
            FreshRoutingFact {
                source,
                subject: FactSubject {
                    credential: Some(second_credential.clone()),
                    ..subject(first, "target-a")
                },
                scope: FactScope::Credential,
                confidence: FactConfidence::Reported,
                observed_at: now,
                valid_until: now + Duration::from_secs(10),
                state: RoutingFactState::Known(RealtimeRoutingFact::CooldownUntil {
                    until: now + Duration::from_secs(4),
                }),
            },
        ]),
    };
    assert_eq!(facts.facts[0].subject.binding, first);
    assert_eq!(facts.facts[1].subject.binding, second);
    assert!(matches!(
        facts.facts[0].state,
        RoutingFactState::Known(RealtimeRoutingFact::QuotaRemaining { units: 0 })
    ));
    assert!(matches!(
        facts.facts[1].state,
        RoutingFactState::Known(RealtimeRoutingFact::ComplianceEligible { eligible: false })
    ));
    assert!(matches!(facts.facts[2].state, RoutingFactState::Unknown));
    assert!(matches!(
        facts.facts[3].state,
        RoutingFactState::Stale { .. }
    ));
    assert_eq!(
        facts.facts[4].subject.credential.as_ref(),
        Some(&first_credential)
    );
    assert_eq!(
        facts.facts[5].subject.credential.as_ref(),
        Some(&second_credential)
    );
    assert_ne!(
        facts.facts[4].subject.credential, facts.facts[5].subject.credential,
        "two credentials of one binding remain separately attributable"
    );

    let usage = UsageFact {
        input: UsageDimension::unknown(),
        output: UsageDimension::reported(0),
        billable: UsageDimension::reported(17),
        cache_read: UsageDimension::reported(4),
        cache_write: UsageDimension::unknown(),
        reasoning: UsageDimension::estimated(3),
    };
    let failed_but_billable = ProviderClassificationFacts {
        error_class: Some(ObservationLabel::new("rate_limited").unwrap()),
        http_status: Some(http::StatusCode::TOO_MANY_REQUESTS),
        usage: Some(usage),
        ..ProviderClassificationFacts::default()
    };
    let usage = failed_but_billable.usage.unwrap();
    assert_eq!(usage.input.units, None);
    assert_eq!(usage.input.provenance, UsageProvenance::Unknown);
    assert_eq!(usage.output.units, Some(0));
    assert_eq!(usage.billable.units, Some(17));
    assert_eq!(usage.reasoning.provenance, UsageProvenance::Estimated);
}

#[test]
fn observation_labels_reject_raw_or_secret_bearing_content() {
    let too_long = "x".repeat(129);
    for unsafe_value in [
        r#"{"token":"raw-json"}"#,
        "line-one\nline-two",
        too_long.as_str(),
        "Bearer sk-simulated-secret-material",
        "Authorization:Bearer-redacted-but-still-raw",
        "sk-simulated-secret-material",
    ] {
        assert!(
            ObservationLabel::new(unsafe_value).is_err(),
            "unsafe observation label must be rejected: {unsafe_value:?}"
        );
    }

    let facts = ProviderClassificationFacts {
        error_class: Some(ObservationLabel::new("rate_limited").unwrap()),
        readiness: ObservationLabel::new("response_head").unwrap(),
        model_event: Some(ObservationLabel::new("semantic_response").unwrap()),
        http_status: Some(http::StatusCode::TOO_MANY_REQUESTS),
        retry_after: Some(Duration::from_secs(2)),
        reset_at: Some(Instant::now() + Duration::from_secs(2)),
        usage: Some(UsageFact {
            billable: UsageDimension::reported(1),
            ..UsageFact::default()
        }),
        ..ProviderClassificationFacts::default()
    };
    assert_eq!(
        facts.error_class.as_ref().map(ObservationLabel::as_str),
        Some("rate_limited")
    );
    assert_eq!(facts.retry_after, Some(Duration::from_secs(2)));
    assert_eq!(facts.usage.unwrap().billable.units, Some(1));
}

#[test]
fn provider_facts_are_decided_by_independent_linear_request_sessions() {
    let selection = ContractSelection;
    let revision = PlanRevision(303);
    let candidates: Arc<[ResolvedTargetBindingId]> = Arc::from([
        ResolvedTargetBindingId::new(revision, 1),
        ResolvedTargetBindingId::new(revision, 2),
    ]);
    let now = Instant::now();
    let overall_a = now + Duration::from_secs(30);
    let overall_b = now + Duration::from_secs(40);
    let mut session_a = selection
        .begin_session(DecisionSessionRequest {
            request_id: RequestId(7),
            route_binding: candidates[0],
            candidate_bindings: Arc::clone(&candidates),
            route_context: Arc::from("route-context-a"),
            overall_deadline: overall_a,
            max_attempts: 2,
        })
        .unwrap();
    let mut session_b = selection
        .begin_session(DecisionSessionRequest {
            request_id: RequestId(8),
            route_binding: candidates[0],
            candidate_bindings: Arc::clone(&candidates),
            route_context: Arc::from("route-context-b"),
            overall_deadline: overall_b,
            max_attempts: 2,
        })
        .unwrap();
    let empty = [];
    let facts_a1 = session_a.snapshot_realtime_facts(now).unwrap();
    let facts_b1 = session_b.snapshot_realtime_facts(now).unwrap();
    assert!(facts_a1.facts[0].is_fresh_at(now));
    let first_a = session_a
        .select_next(SelectionRequest {
            request_id: RequestId(7),
            generation: AttemptGeneration(1),
            route_binding: candidates[0],
            remaining_total: Duration::from_secs(30),
            remaining_attempts: 2,
            realtime_facts: &facts_a1,
            previous_attempts: &empty,
        })
        .unwrap()
        .unwrap();
    let first_b = session_b
        .select_next(SelectionRequest {
            request_id: RequestId(8),
            generation: AttemptGeneration(1),
            route_binding: candidates[0],
            remaining_total: Duration::from_secs(40),
            remaining_attempts: 2,
            realtime_facts: &facts_b1,
            previous_attempts: &empty,
        })
        .unwrap()
        .unwrap();
    assert_eq!(first_a.attempt_id, AttemptId(1));
    assert_eq!(first_b.attempt_id, AttemptId(1));
    assert_eq!(first_a.route_decision_id, RouteDecisionId(10_007));
    assert_eq!(first_b.route_decision_id, RouteDecisionId(10_008));
    assert_eq!(first_a.budget.deadline, overall_a);
    assert_eq!(first_b.budget.deadline, overall_b);
    assert_eq!(first_a.credential_ref.as_str(), "credential-1");

    let retryable = ClassifiedAttemptResult {
        facts: ProviderClassificationFacts {
            error_class: Some(ObservationLabel::new("rate_limited").unwrap()),
            retryability: RetryabilityFact::Retryable,
            retry_after: Some(Duration::from_secs(2)),
            readiness: ObservationLabel::new("classified").unwrap(),
            ..ProviderClassificationFacts::default()
        },
        readiness: "retry-readiness",
    };
    let decision = session_a
        .decide(&first_a, &retryable.facts, &transport_facts())
        .unwrap();
    assert_eq!(decision, Disposition::Continue);
    assert_eq!(retryable.readiness, "retry-readiness");

    let facts_a2 = session_a.snapshot_realtime_facts(Instant::now()).unwrap();
    assert_ne!(facts_a1.facts[0].source, facts_a2.facts[0].source);
    let second_a = session_a
        .select_next(SelectionRequest {
            request_id: RequestId(7),
            generation: AttemptGeneration(2),
            route_binding: candidates[0],
            remaining_total: Duration::from_secs(25),
            remaining_attempts: 1,
            realtime_facts: &facts_a2,
            previous_attempts: &empty,
        })
        .unwrap()
        .unwrap();
    assert_eq!(second_a.binding, candidates[1]);
    assert_eq!(second_a.budget.deadline, overall_a);
    assert!(second_a.budget.allocated <= Duration::from_secs(25));
}
