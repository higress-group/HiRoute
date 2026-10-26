use std::collections::{BTreeMap, BTreeSet};

use crate::{AgentPlanId, CanonicalDigest, WorkspaceId};

use super::*;

#[path = "agent_turn_admission_tests.rs"]
mod agent_turn_admission_tests;

#[path = "merged_contract_tests.rs"]
mod merged_contract_tests;

#[path = "stage_evaluation_tests.rs"]
mod stage_evaluation_tests;

#[test]
fn credential_denial_extension_keeps_released_outcomes_and_rejects_unknown_values() {
    // The three original outcomes were verified at supported v0.2.0/v0.3.0/v0.3.1 sources.
    for (wire, value) in [
        ("leased", CredentialLeaseOutcomeV1::Leased),
        ("exhausted", CredentialLeaseOutcomeV1::Exhausted),
        ("authority_error", CredentialLeaseOutcomeV1::AuthorityError),
        ("source_disabled", CredentialLeaseOutcomeV1::SourceDisabled),
        (
            "credential_disabled",
            CredentialLeaseOutcomeV1::CredentialDisabled,
        ),
    ] {
        let bytes = format!("\"{wire}\"");
        assert_eq!(
            serde_json::from_str::<CredentialLeaseOutcomeV1>(&bytes).unwrap(),
            value
        );
        assert_eq!(serde_json::to_string(&value).unwrap(), bytes);
    }
    assert!(
        serde_json::from_str::<CredentialLeaseOutcomeV1>("\"provider_private_detail\"").is_err()
    );
}

#[test]
fn adapter_fact_contract_roundtrips_every_gateway_variant_and_identity_field() {
    let envelopes = gateway_fact_envelopes();
    assert_eq!(envelopes.len(), 9);
    let mut kinds = BTreeSet::new();
    for envelope in envelopes {
        envelope.validate().unwrap();
        let envelope_digest = CanonicalDigest::of(&envelope).unwrap();
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let decoded: ExecutionFactEnvelopeV1 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, envelope);
        assert_eq!(CanonicalDigest::of(&decoded).unwrap(), envelope_digest);
        kinds.insert(
            serde_json::to_value(&decoded.fact).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
        assert_eq!(decoded.schema_version, EXECUTION_FACT_SCHEMA_V2);
        assert_eq!(
            decoded.schema_digest.as_str(),
            EXECUTION_FACT_PORT_DIGEST_V2
        );
        assert_eq!(decoded.channel, ExecutionFactChannelV1::ExecutionFact);
        assert_eq!(
            decoded.producer.component,
            ExecutionProducerComponentV1::GatewayExecution
        );
        assert_eq!(decoded.producer.revision, "gateway-g0");
        assert_eq!(decoded.producer.stream.producer_id.as_str(), "producer-1");
        assert_eq!(decoded.producer.stream.producer_epoch.as_str(), "epoch-1");
        assert_eq!(decoded.producer.stream.stream_id.as_str(), "stream-1");
        assert_eq!(decoded.correlation.workspace_id, WorkspaceId::default());
        assert_eq!(
            decoded.correlation.conversation_id.as_str(),
            "conversation-1"
        );
        assert_eq!(
            decoded.correlation.session_scope,
            SessionScopeV1::RequestScoped
        );
        assert_eq!(
            decoded.correlation.correlation_provenance,
            CorrelationProvenance::AgentSupplied
        );
        assert_eq!(decoded.correlation.turn_id.as_str(), "turn-1");
        assert_eq!(decoded.correlation.request_id.as_str(), "request-1");
        assert_eq!(decoded.trust.authority_id, "authority-1");
        assert_eq!(decoded.trust.authority_epoch, 7);
        assert_eq!(decoded.trust.served_model_id, "hiroute/coding");
        assert_eq!(
            decoded.trust.selector_source,
            SelectorSourceV1::TrustedModelAlias
        );
        assert_eq!(
            decoded.trust.agent_plan_id.as_ref().unwrap().as_str(),
            "plan/coding"
        );
        assert_eq!(
            decoded.trust.route,
            crate::ModelRequestRouteV2::Plan {
                revision: 17,
                semantic_digest: digest("plan"),
            }
        );
        assert_eq!(decoded.trust.gateway_publication_revision, "41");
        assert_eq!(
            decoded.trust.gateway_publication_digest,
            digest("publication")
        );
        assert_eq!(decoded.trust.grant_id, "grant-1");
        assert_eq!(decoded.trust.grant_generation, 3);
        assert_eq!(decoded.trust.ingress_protocol, IngressProtocolV1::Responses);
    }
    assert_eq!(
        kinds,
        BTreeSet::from([
            "attempt_finished".to_owned(),
            "attempt_started".to_owned(),
            "candidate_decision".to_owned(),
            "credential_lease".to_owned(),
            "request_finished".to_owned(),
            "route_decision".to_owned(),
            "runtime_state".to_owned(),
            "semantic_commit".to_owned(),
            "usage_and_cache".to_owned(),
        ])
    );
}

#[test]
fn fixed_execution_has_recorded_context_without_plan_identity() {
    let fixed = crate::ModelRequestRouteV2::Fixed {
        binding_digest: digest("fixed-binding"),
    };
    let mut decision = route();
    decision.plan_id = None;
    decision.route = fixed.clone();
    let mut fact = envelope(1, ExecutionFactV1::RouteDecision(Box::new(decision)), None);
    fact.trust.agent_plan_id = None;
    fact.trust.plan_display_name = None;
    fact.trust.route = fixed;
    fact.validate().unwrap();
    let context = ObservationRoutingContextV1::from_execution_trust(&fact.trust);
    assert_eq!(
        context.state,
        ObservationRoutingContextStateV1::RecordedFixed
    );
    assert!(context.valid());
    assert!(context.plan_id.is_none());
    assert!(context.plan_revision.is_none());
    let decoded: ExecutionFactEnvelopeV1 =
        serde_json::from_value(serde_json::to_value(&fact).unwrap()).unwrap();
    assert_eq!(decoded, fact);

    let mut false_context = context;
    false_context.plan_id = Some("plan/coding".into());
    assert!(!false_context.valid());
    fact.trust.agent_plan_id = Some(AgentPlanId::parse("plan/coding").unwrap());
    assert!(fact.validate().is_err());
    fact.trust.agent_plan_id = None;
    fact.trust.plan_display_name = Some("Coding".into());
    assert!(fact.validate().is_err());
}

#[test]
fn external_and_rule_fallback_classification_facts_are_explicit() {
    let mut external = route();
    external.complexity = Some(BranchDecisionV1 {
        extension_reason: None,
        decision_protocol: Some(crate::DecisionProtocolV1::LocalRules),
        routing_observation: None,
        policy: None,
        competence_trigger: None,
        execution_group: crate::ExecutionGroupV1::Regular,
        simple_probability: None,
        simple_threshold_millis: None,
        selection_reason: crate::ModelGroupReasonV1::Heuristic,
        strategy_id: "hiroute-complexity-v1".into(),
        schema_version: "hiroute-route-strategy-v1".into(),
        payload_digest: digest("complexity-model"),
        branch_id: crate::SMART_SAVING_SCOPE_ID.into(),
        complexity_score: None,
        threshold: None,
        decision_source: ComplexityDecisionSourceV1::ExternalClassifier,
        reason_codes: Vec::new(),
        matched_user_phrase_ids: Vec::new(),
        fallback_used: false,
        classification_duration_micros: Some(125),
        fallback_reason: None,
    });
    envelope(1, ExecutionFactV1::RouteDecision(Box::new(external)), None)
        .validate()
        .unwrap();

    let mut fallback = route();
    fallback.complexity = Some(BranchDecisionV1 {
        extension_reason: None,
        decision_protocol: Some(crate::DecisionProtocolV1::LocalRules),
        routing_observation: None,
        policy: None,
        competence_trigger: None,
        execution_group: crate::ExecutionGroupV1::Regular,
        simple_probability: None,
        simple_threshold_millis: None,
        selection_reason: crate::ModelGroupReasonV1::Heuristic,
        strategy_id: "hiroute-complexity-v1".into(),
        schema_version: "hiroute-route-strategy-v1".into(),
        payload_digest: digest("complexity-fallback"),
        branch_id: crate::SMART_SAVING_SCOPE_ID.into(),
        complexity_score: Some(0),
        threshold: Some(3),
        decision_source: ComplexityDecisionSourceV1::BuiltinRules,
        reason_codes: Vec::new(),
        matched_user_phrase_ids: Vec::new(),
        fallback_used: true,
        classification_duration_micros: Some(3_000_000),
        fallback_reason: Some(ClassifierFallbackReasonV1::Timeout),
    });
    envelope(1, ExecutionFactV1::RouteDecision(Box::new(fallback)), None)
        .validate()
        .unwrap();
}

#[test]
fn contract_version_digest_unknown_variant_and_canonical_tamper_fail_closed() {
    let envelope = gateway_fact_envelopes().remove(0);
    let original_digest = CanonicalDigest::of(&envelope).unwrap();

    let mut wrong_version = envelope.clone();
    wrong_version.schema_version = "hiroute.observation.execution-fact-envelope/v3".to_owned();
    assert_eq!(
        wrong_version.validate(),
        Err(ExecutionFactError::UnsupportedSchema)
    );

    let mut wrong_contract = envelope.clone();
    wrong_contract.schema_digest = CanonicalDigest::of_bytes(b"other-contract");
    assert_eq!(
        wrong_contract.validate(),
        Err(ExecutionFactError::UnsupportedSchema)
    );

    let mut tampered = envelope.clone();
    tampered.trust.grant_generation += 1;
    assert_ne!(CanonicalDigest::of(&tampered).unwrap(), original_digest);

    let mut unknown = serde_json::to_value(envelope).unwrap();
    unknown["fact"] = serde_json::json!({"kind":"future_fact","field":1});
    assert!(serde_json::from_value::<ExecutionFactEnvelopeV1>(unknown).is_err());
}

#[test]
fn authenticated_v1_fact_is_recovery_only_and_cannot_reenter_live_ingestion() {
    let mut legacy = gateway_fact_envelopes().remove(0);
    legacy.schema_version = "hiroute.observation.execution-fact-envelope/v1".to_owned();
    legacy.schema_digest = CanonicalDigest::parse(
        "sha256:5aad4c450a2a296ec557b7a934bfbf70ad69a8e8fe9bc7539db46830a2940069",
    )
    .unwrap();
    legacy.pricing = None;
    assert_eq!(
        legacy.validate(),
        Err(ExecutionFactError::UnsupportedSchema)
    );
    legacy.validate_persisted_contract().unwrap();
}

#[test]
fn pre_c_product_digest_is_read_only_and_cannot_authenticate_new_detail() {
    let mut old = gateway_fact_envelopes().remove(0);
    old.schema_digest = CanonicalDigest::parse(
        "sha256:4f0dbcc9f7862133e73e451f64d7a6d6e5de146d958ac8bf3a4d1acc885b6b1d",
    )
    .unwrap();
    let bytes = serde_json::to_vec(&old).unwrap();
    let digest = CanonicalDigest::of(&old).unwrap();
    assert_eq!(old.validate(), Err(ExecutionFactError::UnsupportedSchema));
    old.validate_persisted_contract().unwrap();
    let decoded: ExecutionFactEnvelopeV1 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
    assert_eq!(CanonicalDigest::of(&decoded).unwrap(), digest);
    let p = crate::NativeProtocolV1::default();
    let mut j = crate::JudgmentSettingsV1::default();
    p.stock_assessment(&mut j);
    let frozen = crate::FrozenNativeProtocolV1::freeze(
        &p,
        j.degree.definition().into(),
        [(crate::SMART_SAVING_SCOPE_ID.into(), j)],
    )
    .unwrap();
    let ExecutionFactV1::RouteDecision(route) = &mut old.fact else {
        unreachable!()
    };
    let decision = route.complexity.as_mut().unwrap();
    decision.decision_protocol = Some(crate::DecisionProtocolV1::SystemOneC);
    decision.routing_observation = Some(crate::RoutingObservationV1::not_dispatched(
        &frozen,
        crate::SMART_SAVING_SCOPE_ID,
    ));
    decision.decision_source = ComplexityDecisionSourceV1::ExternalClassifier;
    decision.complexity_score = None;
    decision.threshold = None;
    decision.classification_duration_micros = Some(125);
    assert_eq!(
        old.validate_persisted_contract(),
        Err(ExecutionFactError::InvalidFact)
    );
    old.schema_digest = CanonicalDigest::parse(EXECUTION_FACT_PORT_DIGEST_V2).unwrap();
    old.validate().unwrap();
}

#[test]
fn legacy_execution_trust_without_plan_display_name_remains_readable() {
    let mut value = serde_json::to_value(envelope(
        1,
        ExecutionFactV1::RequestFinished {
            outcome: ExecutionRequestOutcomeV1::Accepted,
            attempts_started: 1,
            attempts_finished: 1,
            accepted_attempt_ordinal: Some(1),
            facts_completeness: FactsCompleteness::Complete,
        },
        None,
    ))
    .unwrap();
    value["trust"]
        .as_object_mut()
        .unwrap()
        .remove("plan_display_name");
    let legacy: ExecutionFactEnvelopeV1 = serde_json::from_value(value).unwrap();
    assert!(legacy.trust.plan_display_name.is_none());
    legacy.validate().unwrap();
}

#[test]
fn released_product_execution_digest_is_recovery_only_with_pricing() {
    // Exact Product contract digest at supported 0.3.0 ce4c4802 and 0.3.1 13856abd.
    let released_digest = CanonicalDigest::parse(
        "sha256:80b4ff13af8abc79d7721fea42141300beade2acf1d2659983ceaafcdc6e4799",
    )
    .unwrap();
    for mut envelope in gateway_fact_envelopes() {
        envelope.schema_digest = released_digest.clone();
        if matches!(envelope.fact, ExecutionFactV1::AttemptStarted { .. }) {
            envelope.pricing = Some(
                serde_json::from_value(serde_json::json!({
                    "schema_version": "hiroute.observation.execution-pricing/v1",
                    "request_generation": null,
                    "captured_at_ms": 0,
                    "attempt_execution_at_ms": 0,
                    "quote": null,
                    "reference_quote": null,
                    "unknown_reason": "snapshot_unavailable"
                }))
                .unwrap(),
            );
        }
        assert_eq!(
            envelope.validate(),
            Err(ExecutionFactError::UnsupportedSchema)
        );
        envelope.validate_persisted_contract().unwrap();
        let original = CanonicalDigest::of(&envelope).unwrap();
        let decoded: ExecutionFactEnvelopeV1 =
            serde_json::from_slice(&serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert_eq!(decoded, envelope);
        assert_eq!(CanonicalDigest::of(&decoded).unwrap(), original);
    }
    let mut custom = gateway_fact_envelopes().remove(0);
    custom.schema_digest = released_digest;
    let ExecutionFactV1::RouteDecision(route) = &mut custom.fact else {
        unreachable!()
    };
    route.requirements.tool_choice = ToolChoiceRequirementV1::RequiredNamed {
        tool_kind: ToolKindRequirementV1::Custom,
        name: "read_number".into(),
    };
    assert_eq!(
        custom.validate_persisted_contract(),
        Err(ExecutionFactError::InvalidFact)
    );
}

#[test]
fn typed_dynamic_edges_roundtrip_without_an_untyped_field_sink() {
    let mut unavailable = candidate();
    unavailable.ingress_protocol = IngressProtocolV1::Unknown;
    unavailable.upstream_protocol = IngressProtocolV1::Unknown;
    unavailable.cache_cost = None;
    unavailable.eligible = false;
    unavailable.exclusion_reason = Some(ExclusionReasonCodeV1::ProtocolPathUnavailable);
    let encoded = serde_json::to_value(&unavailable).unwrap();
    assert!(encoded["cache_cost"].is_null());
    assert_eq!(
        serde_json::from_value::<CandidateDecisionFactV1>(encoded).unwrap(),
        unavailable
    );

    let native = NativeReasoningValueV1::Object(BTreeMap::from([
        (String::new(), NativeReasoningValueV1::Null),
        (
            "values".to_owned(),
            NativeReasoningValueV1::Array(vec![
                NativeReasoningValueV1::Bool(true),
                NativeReasoningValueV1::Signed(-1),
                NativeReasoningValueV1::Unsigned(2),
                NativeReasoningValueV1::Decimal(0.5),
                NativeReasoningValueV1::String("high".to_owned()),
            ]),
        ),
    ]));
    native.validate().unwrap();
    let encoded = serde_json::to_vec(&native).unwrap();
    assert_eq!(
        serde_json::from_slice::<NativeReasoningValueV1>(&encoded).unwrap(),
        native
    );
}

fn gateway_fact_envelopes() -> Vec<ExecutionFactEnvelopeV1> {
    let attempt_id = AttemptId::parse("attempt-1").unwrap();
    let facts = vec![
        ExecutionFactV1::RouteDecision(Box::new(route())),
        ExecutionFactV1::CandidateDecision(Box::new(candidate())),
        ExecutionFactV1::CredentialLease {
            stable_binding_id: "binding-1".to_owned(),
            credential_ref: "credential-1".to_owned(),
            key_id: Some("key-1".to_owned()),
            credential_generation: Some(2),
            excluded_key_count: 0,
            outcome: CredentialLeaseOutcomeV1::Leased,
        },
        ExecutionFactV1::RuntimeState {
            operation: RuntimeOperationV1::ReadExact,
            key_scope: RuntimeKeyScopeV1::Credential,
            stable_binding_id: "binding-1".to_owned(),
            credential_ref: Some("credential-1".to_owned()),
            key_id: Some("key-1".to_owned()),
            expected_generation: None,
            observed_generation: Some(2),
            health: Some(RuntimeHealthV1::Active),
            cooldown_remaining_millis: None,
            probe_lease_remaining_millis: None,
            transient_backoff_step: Some(0),
            outcome: RuntimeOperationOutcomeV1::Ok,
        },
        ExecutionFactV1::AttemptStarted {
            ordinal: 1,
            candidate_id: "candidate-1".to_owned(),
            stable_binding_id: "binding-1".to_owned(),
            profile_digest: digest("profile").into(),
            credential_ref: "credential-1".to_owned(),
            key_id: "key-1".to_owned(),
            provider_name: "provider-1".to_owned(),
            request_model: "native-model".to_owned(),
            upstream_protocol: IngressProtocolV1::Responses,
            model_configuration_id: "model-config-1".to_owned(),
            adapter_revision: "adapter-v1".to_owned(),
            start_reason: "initial_candidate".to_owned(),
            previous_attempt_id: None,
        },
        ExecutionFactV1::SemanticCommit {
            ordinal: 1,
            boundary: SemanticCommitBoundaryV1::FullFrameTransportAccepted,
            frame_id: "frame-1".to_owned(),
        },
        ExecutionFactV1::UsageAndCache {
            ordinal: 1,
            source: UsageSourceV1::AcceptedCanonicalModelEvent,
            input_tokens: Some(10),
            output_tokens: Some(4),
            billable_tokens: Some(14),
            cache_read_tokens: Some(3),
            cache_write_tokens: Some(1),
            reasoning_tokens: Some(2),
            input_provenance: UsageProvenanceV1::Reported,
            output_provenance: UsageProvenanceV1::Reported,
            billable_provenance: UsageProvenanceV1::Reported,
            cache_read_provenance: UsageProvenanceV1::Reported,
            cache_write_provenance: UsageProvenanceV1::Reported,
            reasoning_provenance: UsageProvenanceV1::Reported,
            effective_cost_micros: Some(20),
            cost_class: Some(CostClassV1::Paid),
            cache_status: CacheStatusV1::ConfirmedUsage,
        },
        ExecutionFactV1::AttemptFinished(Box::new(AttemptFinishedFactV1 {
            ordinal: 1,
            stable_binding_id: "binding-1".to_owned(),
            outcome: AttemptOutcomeV1::Accepted,
            error_class: None,
            retryable: None,
            duration_micros: 100,
            disposition: AttemptDispositionV1::Accept,
            provider_http_status: Some(200),
            provider_code: None,
            provider_request_id: Some("provider-request-1".to_owned()),
            retry_after_millis: None,
            reset_after_millis: None,
            provider_readiness: Some("ready".to_owned()),
            provider_model_event: Some("response.completed".to_owned()),
            time_to_first_model_event_micros: Some(30),
            provider_ended_micros_from_start: Some(90),
            transport: AttemptTransportFactV1 {
                connect_micros: Some(2),
                request_write_micros: Some(3),
                upstream_ttfb_micros: Some(20),
                last_upstream_progress_micros_from_start: Some(80),
                local_read_suppressed_micros: 0,
                upstream_body_bytes: 64,
                timeout_kind: None,
            },
            commits: AttemptCommitFactV1 {
                upstream_request: CommitFenceV1::WriteConfirmed,
                downstream_headers: CommitFenceV1::WriteConfirmed,
                downstream_semantic: CommitFenceV1::WriteConfirmed,
            },
            stream_outcome: AttemptStreamOutcomeV1::CompletedEos,
            downstream_outcome: AttemptDownstreamOutcomeV1::Completed,
            cleanup_outcome: AttemptCleanupOutcomeV1::Completed,
            termination_reason: AttemptTerminationReasonV1::AcceptedEos,
        })),
        ExecutionFactV1::RequestFinished {
            outcome: ExecutionRequestOutcomeV1::Accepted,
            attempts_started: 1,
            attempts_finished: 1,
            accepted_attempt_ordinal: Some(1),
            facts_completeness: FactsCompleteness::Complete,
        },
    ];
    facts
        .into_iter()
        .enumerate()
        .map(|(index, fact)| {
            let attempt_scoped = matches!(
                fact,
                ExecutionFactV1::AttemptStarted { .. }
                    | ExecutionFactV1::AttemptFinished(_)
                    | ExecutionFactV1::SemanticCommit { .. }
                    | ExecutionFactV1::UsageAndCache { .. }
            );
            envelope(
                u64::try_from(index).unwrap() + 1,
                fact,
                attempt_scoped.then(|| attempt_id.clone()),
            )
        })
        .collect()
}

fn envelope(
    sequence: u64,
    fact: ExecutionFactV1,
    attempt_id: Option<AttemptId>,
) -> ExecutionFactEnvelopeV1 {
    ExecutionFactEnvelopeV1 {
        pricing: None,
        schema_version: EXECUTION_FACT_SCHEMA_V2.to_owned(),
        schema_digest: CanonicalDigest::parse(EXECUTION_FACT_PORT_DIGEST_V2).unwrap(),
        channel: ExecutionFactChannelV1::ExecutionFact,
        producer: ExecutionProducerV1 {
            component: ExecutionProducerComponentV1::GatewayExecution,
            revision: "gateway-g0".to_owned(),
            stream: ObservationStreamV1 {
                producer_id: ProducerId::parse("producer-1").unwrap(),
                producer_epoch: ProducerEpoch::parse("epoch-1").unwrap(),
                stream_id: StreamId::parse("stream-1").unwrap(),
            },
        },
        sequence,
        event_id: EventId::parse(format!("event-{sequence}")).unwrap(),
        correlation: ExecutionCorrelationV1 {
            workspace_id: WorkspaceId::default(),
            conversation_id: SessionId::parse("conversation-1").unwrap(),
            session_scope: SessionScopeV1::RequestScoped,
            correlation_provenance: CorrelationProvenance::AgentSupplied,
            turn_id: TurnId::parse("turn-1").unwrap(),
            request_id: LogicalRequestId::parse("request-1").unwrap(),
        },
        attempt_id,
        trust: FrozenExecutionTrustV1 {
            authority_id: "authority-1".to_owned(),
            authority_epoch: 7,
            served_model_id: "hiroute/coding".to_owned(),
            selector_source: SelectorSourceV1::TrustedModelAlias,
            agent_plan_id: Some(AgentPlanId::parse("plan/coding").unwrap()),
            route: crate::ModelRequestRouteV2::Plan {
                revision: 17,
                semantic_digest: digest("plan"),
            },
            plan_display_name: Some("Coding".to_owned()),
            gateway_publication_revision: "41".to_owned(),
            gateway_publication_digest: digest("publication"),
            grant_id: "grant-1".to_owned(),
            grant_generation: 3,
            ingress_protocol: IngressProtocolV1::Responses,
        },
        occurred_at_unix_nanos: 1_000_000_000 + sequence,
        fact,
        loss_watermark: None,
        completeness_delta: None,
    }
}

#[test]
fn native_auxiliary_marker_is_optional_for_retained_route_facts() {
    let original = route();
    let bytes = serde_json::to_value(&original).unwrap();
    assert!(bytes.get("native_auxiliary").is_none());
    assert_eq!(
        serde_json::from_value::<RouteDecisionFactV1>(bytes.clone()).unwrap(),
        original
    );
    let mut marked = bytes;
    marked["native_auxiliary"] = serde_json::json!(true);
    assert!(
        serde_json::from_value::<RouteDecisionFactV1>(marked)
            .unwrap()
            .native_auxiliary
    );

    // Exact contract emitted by the retained pre-auxiliary producer. Recovery
    // authenticates its original bytes without inventing purpose evidence.
    let mut old = envelope(1, ExecutionFactV1::RouteDecision(Box::new(route())), None);
    old.schema_digest = CanonicalDigest::parse(
        "sha256:3cb9b100385c1f05a92097c2f28eb5c47d61ca495d32d41a8528b7227c81a465",
    )
    .unwrap();
    let original = serde_json::to_vec(&old).unwrap();
    assert!(old.validate().is_err());
    old.validate_persisted_contract().unwrap();
    let decoded: ExecutionFactEnvelopeV1 = serde_json::from_slice(&original).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), original);
    if let ExecutionFactV1::RouteDecision(route) = &mut old.fact {
        route.native_auxiliary = true;
    }
    assert_eq!(
        old.validate_persisted_contract(),
        Err(ExecutionFactError::InvalidFact)
    );
}

fn route() -> RouteDecisionFactV1 {
    RouteDecisionFactV1 {
        native_auxiliary: false,
        planner_version: "planner-v1".to_owned(),
        plan_id: Some(AgentPlanId::parse("plan/coding").unwrap()),
        route: crate::ModelRequestRouteV2::Plan {
            revision: 17,
            semantic_digest: digest("plan"),
        },
        input_digest: digest("input"),
        policy_digest: digest("policy"),
        output_digest: digest("output"),
        branch: PlannedBranchV1::CustomExactOrder,
        complexity: Some(BranchDecisionV1 {
            extension_reason: None,
            decision_protocol: Some(crate::DecisionProtocolV1::LocalRules),
            routing_observation: None,
            policy: None,
            competence_trigger: None,
            execution_group: crate::ExecutionGroupV1::Regular,
            simple_probability: None,
            simple_threshold_millis: None,
            selection_reason: crate::ModelGroupReasonV1::Heuristic,
            strategy_id: "hiroute-complexity-v1".to_owned(),
            schema_version: "hiroute-route-strategy-v1".to_owned(),
            payload_digest: digest("complexity"),
            branch_id: crate::SMART_SAVING_SCOPE_ID.into(),
            complexity_score: Some(4),
            threshold: Some(3),
            decision_source: ComplexityDecisionSourceV1::BuiltinRules,
            reason_codes: vec![ComplexityReasonCodeV1::MultiConstraint],
            matched_user_phrase_ids: vec![],
            fallback_used: false,
            classification_duration_micros: None,
            fallback_reason: None,
        }),
        groups: vec![GroupPlanV1 {
            ordinal: 0,
            group_id: "group-1".to_owned(),
            ranked_candidate_ids: vec!["candidate-1".to_owned()],
        }],
        reason_ledger: vec![ReasonLedgerEntryV1 {
            ordinal: 0,
            code: LedgerReasonCodeV1::CustomExactOrder,
            group_id: Some("group-1".to_owned()),
        }],
        requirements: requirements(),
        requested_reasoning_disposition: RequestedReasoningDispositionV1::OverriddenByAgentPlan,
        requested_reasoning_value: Some(NativeReasoningValueV1::String("high".to_owned())),
        requested_max_output_tokens: Some(1_024),
        stream: true,
        outcome: RouteDecisionOutcomeV1::Ready,
        outcome_code: None,
        max_attempts: 2,
    }
}

fn candidate() -> CandidateDecisionFactV1 {
    CandidateDecisionFactV1 {
        candidate_id: "candidate-1".to_owned(),
        stable_binding_id: "binding-1".to_owned(),
        group_id: "group-1".to_owned(),
        declared_order: 0,
        profile_digest: ObservedProfileDigestV1::parse("unknown").unwrap(),
        ingress_protocol: IngressProtocolV1::Responses,
        upstream_protocol: IngressProtocolV1::Responses,
        path_id: "responses-to-responses".to_owned(),
        provider_id: "provider-1".to_owned(),
        endpoint_id: "endpoint-1".to_owned(),
        entitlement_id: "entitlement-1".to_owned(),
        connector_id: "connector-1".to_owned(),
        connector_revision: "connector-v1".to_owned(),
        capability_id: "capability-1".to_owned(),
        capability_revision: "capability-v1".to_owned(),
        model_configuration_id: "model-config-1".to_owned(),
        native_model: "native-model".to_owned(),
        adapter_revision: "adapter-v1".to_owned(),
        serializer_revision: "serializer-v1".to_owned(),
        decoder_revision: "decoder-v1".to_owned(),
        target_serialized_bytes: 128,
        eligible: true,
        exclusion_reason: None,
        reasoning_profile_id: Some("reasoning-high".to_owned()),
        overall_score_tenths: Some(48),
        effective_cost_micros: Some(20),
        api_equivalent_cost_micros: Some(40),
        cost_class: CostClassV1::Paid,
        cache_cost: Some(CacheCostFactV1::None),
        cache_affinity: false,
        compute_scope_order: 0,
        ranking_reasons: vec![RankingReasonCodeV1::StableTieBreak],
    }
}

#[test]
fn released_named_function_choice_preserves_canonical_bytes_and_digest() {
    // Frozen Product serializer shape at supported 0.2.0 d625e653, 0.3.0 ce4c4802 and
    // 0.3.1 13856abd. Do not regenerate this input with the current serializer.
    let released = serde_json::json!({"kind": "required_named", "name": "read_number"});
    let decoded: ToolChoiceRequirementV1 = serde_json::from_value(released.clone()).unwrap();
    assert_eq!(
        decoded,
        ToolChoiceRequirementV1::RequiredNamed {
            tool_kind: ToolKindRequirementV1::Function,
            name: "read_number".into(),
        }
    );
    assert_eq!(serde_json::to_value(&decoded).unwrap(), released);
    assert_eq!(
        CanonicalDigest::of(&decoded).unwrap(),
        CanonicalDigest::of(&released).unwrap()
    );
    let mut explicit = released;
    explicit["tool_kind"] = serde_json::json!("function");
    assert_eq!(
        serde_json::from_value::<ToolChoiceRequirementV1>(explicit).unwrap(),
        decoded
    );
}

#[test]
fn custom_tool_choice_stays_distinct_and_strict() {
    let custom =
        serde_json::json!({"kind": "required_named", "tool_kind": "custom", "name": "read_number"});
    let decoded: ToolChoiceRequirementV1 = serde_json::from_value(custom.clone()).unwrap();
    assert_eq!(
        decoded,
        ToolChoiceRequirementV1::RequiredNamed {
            tool_kind: ToolKindRequirementV1::Custom,
            name: "read_number".into(),
        }
    );
    assert_eq!(serde_json::to_value(decoded).unwrap(), custom);
    for invalid in [
        serde_json::json!({"kind": "required_named", "tool_kind": "unknown", "name": "read_number"}),
        serde_json::json!({"kind": "required_named", "tool_kind": null, "name": "read_number"}),
        serde_json::json!({"kind": "required_named", "tool_kind": "function", "name": "read_number", "extra": true}),
    ] {
        assert!(serde_json::from_value::<ToolChoiceRequirementV1>(invalid).is_err());
    }
}

fn requirements() -> RequestCapabilityRequirementsV1 {
    RequestCapabilityRequirementsV1 {
        ingress_protocol: IngressProtocolV1::Responses,
        text: true,
        initial_instructions: false,
        mid_conversation_instructions: false,
        image_url: false,
        image_base64: false,
        image_media_types: vec![],
        function_tools: false,
        strict_tools: false,
        tool_choice: ToolChoiceRequirementV1::Auto,
        parallel_tools: false,
        tool_roundtrip: false,
        tool_result_text: false,
        tool_result_json: false,
        logical_tool_id_mapping: false,
        streaming: true,
        stream_text: true,
        stream_tool_arguments: false,
        stream_reasoning: true,
        stream_usage: true,
        provider_state: false,
    }
}

fn digest(value: &str) -> CanonicalDigest {
    CanonicalDigest::of_bytes(value.as_bytes())
}
