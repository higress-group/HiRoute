use std::sync::Arc;

use hiroute_domain::{
    AgentPlanId, AttemptCommitFactV1, AttemptFinishedFactV1, AttemptId, AttemptTransportFactV1,
    BranchDecisionV1, CONVERSATION_CONTENT_PORT_DIGEST_V2, CONVERSATION_CONTENT_SCHEMA_V2,
    CacheCostFactV1, CandidateDecisionFactV1, ComplexityDecisionSourceV1, ComplexityReasonCodeV1,
    ContentCompletenessDeltaV2, ContentCorrelationV2, ContentId, ConversationContentChannelV2,
    ConversationContentDirectionV2, ConversationContentEnvelopeV1, ConversationContentPhaseV2,
    CorrelationProvenance, CostClassV1, EXECUTION_FACT_PORT_DIGEST_V2, EXECUTION_FACT_SCHEMA_V2,
    EventId, ExecutionCorrelationV1, ExecutionFactChannelV1, ExecutionFactEnvelopeV1,
    ExecutionFactV1, ExecutionProducerV1, FrozenExecutionTrustV1, FrozenValueFactsV1, GroupPlanV1,
    IngressProtocolV1, LedgerReasonCodeV1, LogicalRequestId, MessageInstanceId,
    ObservationProducerV2, ObservationStreamV1, PlannedBranchV1, ProducerEpoch, ProducerId,
    RequestCapabilityRequirementsV1, RequestedReasoningDispositionV1, RouteDecisionFactV1,
    RouteDecisionOutcomeV1, SessionId, SessionScopeV1, StreamId, ToolChoiceRequirementV1,
    TrafficKind, TranscriptRoot, TurnId, UsageFactsV1, ValueGroupByV1, ValueQueryV1, WorkspaceId,
};

use crate::{
    ConversationContentChannel, DigestAuthority, FactChannel, LocalObservationStore,
    LocalObservationWriter, OfferOutcome, WriterCycleOutcome,
};

#[derive(Clone)]
pub(crate) struct Fixture {
    pub workspace: WorkspaceId,
    pub session: SessionId,
    pub turn: TurnId,
    pub request: LogicalRequestId,
    pub fact_stream: ObservationStreamV1,
    pub content_stream: ObservationStreamV1,
}

impl Fixture {
    pub(crate) fn new(suffix: &str) -> Self {
        Self {
            workspace: WorkspaceId::default(),
            session: SessionId::parse(format!("session-{suffix}")).unwrap(),
            turn: TurnId::parse(format!("turn-{suffix}")).unwrap(),
            request: LogicalRequestId::parse(format!("request-{suffix}")).unwrap(),
            fact_stream: stream("fact", suffix),
            content_stream: stream("content", suffix),
        }
    }

    pub(crate) fn fact(
        &self,
        sequence: u64,
        fact: ExecutionFactV1,
        occurred_at_ms: i64,
    ) -> ExecutionFactEnvelopeV1 {
        self.fact_for_plan(sequence, fact, occurred_at_ms, "plan/codex-daily", None)
    }

    pub(crate) fn content_begin(
        &self,
        sequence: u64,
        occurred_at_ms: i64,
    ) -> ConversationContentEnvelopeV1 {
        self.content_base(sequence, ConversationContentPhaseV2::Begin, occurred_at_ms)
    }

    fn content_base(
        &self,
        sequence: u64,
        phase: ConversationContentPhaseV2,
        occurred_at_ms: i64,
    ) -> ConversationContentEnvelopeV1 {
        ConversationContentEnvelopeV1 {
            schema_version: CONVERSATION_CONTENT_SCHEMA_V2.into(),
            schema_digest: CONVERSATION_CONTENT_PORT_DIGEST_V2.into(),
            channel: ConversationContentChannelV2::ConversationContent,
            producer: ObservationProducerV2 {
                component: "gateway-content".into(),
                revision: "g-star".into(),
                stream: self.content_stream.clone(),
            },
            sequence,
            event_id: EventId::parse(format!("content-event-{sequence}")).unwrap(),
            correlation: ContentCorrelationV2 {
                workspace_id: self.workspace.clone(),
                conversation_id: self.session.clone(),
                session_scope: SessionScopeV1::Conversation,
                correlation_provenance: CorrelationProvenance::AgentSupplied,
                turn_id: self.turn.clone(),
                request_id: self.request.clone(),
            },
            direction: ConversationContentDirectionV2::RequestInput,
            phase,
            attempt_id: None,
            fork_id: "fork-request".into(),
            parent_transcript_root: None,
            result_transcript_root: None,
            message_instance_id: None,
            message_role: None,
            content_kind: None,
            content_id: None,
            content_blob_digest: None,
            message_ordinal: None,
            part_ordinal: None,
            chunk_ordinal: None,
            transport_frame_id: None,
            canonical_media_type: None,
            canonical_bytes_base64: None,
            content_ref: None,
            downstream_delivery: None,
            abort_reason: None,
            occurred_at_unix_nanos: u64::try_from(occurred_at_ms).unwrap() * 1_000_000,
            loss_watermark: None,
            completeness_delta: None,
        }
    }

    fn request_content_events(
        &self,
        bytes: &[u8],
        occurred_at_ms: i64,
    ) -> [ConversationContentEnvelopeV1; 3] {
        let message = MessageInstanceId::parse(format!("message-{}", self.session)).unwrap();
        let content = ContentId::parse(format!("content-{}", self.session)).unwrap();
        let digest = authority().content_blob_digest("text/plain", bytes);
        let root_hash = hiroute_domain::CanonicalDigest::of_bytes(self.request.as_str().as_bytes());
        let root = TranscriptRoot::parse(format!(
            "transcript-{}",
            root_hash.as_str().strip_prefix("sha256:").unwrap()
        ))
        .unwrap();
        let begin = self.content_base(1, ConversationContentPhaseV2::Begin, occurred_at_ms);
        let mut append =
            self.content_base(2, ConversationContentPhaseV2::Append, occurred_at_ms + 1);
        append.message_instance_id = Some(message);
        append.message_role = Some("user".into());
        append.content_kind = Some("text".into());
        append.content_id = Some(content);
        append.content_blob_digest = Some(digest);
        append.message_ordinal = Some(0);
        append.part_ordinal = Some(0);
        append.chunk_ordinal = Some(0);
        append.canonical_media_type = Some("text/plain".into());
        append.canonical_bytes_base64 = Some(base64(bytes));
        let mut finish =
            self.content_base(3, ConversationContentPhaseV2::Finish, occurred_at_ms + 2);
        finish.result_transcript_root = Some(root);
        finish.completeness_delta = Some(ContentCompletenessDeltaV2::Complete);
        [begin, append, finish]
    }

    pub(crate) fn open_content_events(
        &self,
        begin_sequence: u64,
        bytes: &[u8],
        occurred_at_ms: i64,
    ) -> [ConversationContentEnvelopeV1; 2] {
        let mut begin = self.content_base(
            begin_sequence,
            ConversationContentPhaseV2::Begin,
            occurred_at_ms,
        );
        begin.fork_id = "fork-open".into();
        let mut append = self.content_base(
            begin_sequence + 1,
            ConversationContentPhaseV2::Append,
            occurred_at_ms + 1,
        );
        append.fork_id = begin.fork_id.clone();
        append.message_instance_id = Some(MessageInstanceId::parse("message-open").unwrap());
        append.message_role = Some("user".into());
        append.content_kind = Some("text".into());
        append.content_id = Some(ContentId::parse("content-open").unwrap());
        append.content_blob_digest = Some(authority().content_blob_digest("text/plain", bytes));
        append.message_ordinal = Some(0);
        append.part_ordinal = Some(0);
        append.chunk_ordinal = Some(0);
        append.canonical_media_type = Some("text/plain".into());
        append.canonical_bytes_base64 = Some(base64(bytes));
        [begin, append]
    }

    pub(crate) fn attempt_fact(
        &self,
        sequence: u64,
        attempt_id: &str,
        fact: ExecutionFactV1,
        occurred_at_ms: i64,
        plan_id: &str,
    ) -> ExecutionFactEnvelopeV1 {
        self.fact_for_plan(
            sequence,
            fact,
            occurred_at_ms,
            plan_id,
            Some(AttemptId::parse(attempt_id).unwrap()),
        )
    }

    pub(crate) fn fact_for_plan(
        &self,
        sequence: u64,
        fact: ExecutionFactV1,
        occurred_at_ms: i64,
        plan_id: &str,
        attempt_id: Option<AttemptId>,
    ) -> ExecutionFactEnvelopeV1 {
        ExecutionFactEnvelopeV1 {
            pricing: None,
            schema_version: EXECUTION_FACT_SCHEMA_V2.to_owned(),
            schema_digest: hiroute_domain::CanonicalDigest::parse(EXECUTION_FACT_PORT_DIGEST_V2)
                .unwrap(),
            channel: ExecutionFactChannelV1::ExecutionFact,
            producer: ExecutionProducerV1 {
                component: hiroute_domain::ExecutionProducerComponentV1::GatewayExecution,
                revision: "gateway-g0".to_owned(),
                stream: self.fact_stream.clone(),
            },
            sequence,
            event_id: EventId::parse(format!("fact-event-{sequence}")).unwrap(),
            correlation: ExecutionCorrelationV1 {
                workspace_id: self.workspace.clone(),
                conversation_id: self.session.clone(),
                session_scope: SessionScopeV1::Conversation,
                correlation_provenance: CorrelationProvenance::AgentSupplied,
                turn_id: self.turn.clone(),
                request_id: self.request.clone(),
            },
            attempt_id,
            trust: trust(plan_id),
            occurred_at_unix_nanos: u64::try_from(occurred_at_ms).unwrap() * 1_000_000,
            fact,
            loss_watermark: None,
            completeness_delta: None,
        }
    }
}

pub(crate) fn stream(channel: &str, suffix: &str) -> ObservationStreamV1 {
    ObservationStreamV1 {
        producer_id: ProducerId::parse(format!("producer-{channel}")).unwrap(),
        producer_epoch: ProducerEpoch::parse(format!("epoch-{suffix}")).unwrap(),
        stream_id: StreamId::parse(format!("stream-{channel}")).unwrap(),
    }
}

pub(crate) fn authority() -> DigestAuthority {
    DigestAuthority::new([19; 32])
}

pub(crate) fn open_store(path: &std::path::Path) -> Arc<LocalObservationStore> {
    Arc::new(LocalObservationStore::open(path, authority()).unwrap())
}

pub(crate) fn writer(store: &Arc<LocalObservationStore>) -> LocalObservationWriter {
    LocalObservationWriter::new(store.clone())
}

pub(crate) fn fact_channel(fixture: &Fixture, bytes: usize) -> FactChannel {
    FactChannel::new(fixture.fact_stream.clone(), bytes)
}

pub(crate) fn content_channel(fixture: &Fixture, bytes: usize) -> ConversationContentChannel {
    ConversationContentChannel::new(fixture.content_stream.clone(), bytes)
}

pub(crate) fn offer_fact(
    writer: &LocalObservationWriter,
    channel: &FactChannel,
    envelope: ExecutionFactEnvelopeV1,
) -> WriterCycleOutcome {
    assert_eq!(channel.offer(envelope), OfferOutcome::Accepted);
    writer.consume_fact(channel)
}

pub(crate) fn offer_content(
    writer: &LocalObservationWriter,
    channel: &ConversationContentChannel,
    envelope: ConversationContentEnvelopeV1,
) -> WriterCycleOutcome {
    assert_eq!(channel.offer(envelope), OfferOutcome::Accepted);
    writer.consume_content(channel)
}

pub(crate) fn trust(plan_id: &str) -> FrozenExecutionTrustV1 {
    FrozenExecutionTrustV1 {
        authority_id: "authority-local".to_owned(),
        authority_epoch: 9,
        served_model_id: "hiroute/coding".to_owned(),
        selector_source: hiroute_domain::SelectorSourceV1::TrustedModelAlias,
        agent_plan_id: Some(AgentPlanId::parse(plan_id).unwrap()),
        route: hiroute_domain::ModelRequestRouteV2::Plan {
            revision: 17,
            semantic_digest: digest("agent-plan-semantic"),
        },
        plan_display_name: Some("Coding plan".to_owned()),
        gateway_publication_revision: "41".to_owned(),
        gateway_publication_digest: digest("gateway-publication"),
        grant_id: "grant-7".to_owned(),
        grant_generation: 3,
        ingress_protocol: IngressProtocolV1::Responses,
    }
}

pub(crate) fn route_decision(plan_id: &str) -> ExecutionFactV1 {
    ExecutionFactV1::RouteDecision(Box::new(RouteDecisionFactV1 {
        planner_version: "planner-v1".to_owned(),
        plan_id: Some(AgentPlanId::parse(plan_id).unwrap()),
        route: hiroute_domain::ModelRequestRouteV2::Plan {
            revision: 17,
            semantic_digest: digest("agent-plan-semantic"),
        },
        input_digest: digest("planner-input"),
        policy_digest: digest("planner-policy"),
        output_digest: digest("planner-output"),
        branch: PlannedBranchV1::CustomExactOrder,
        complexity: Some(BranchDecisionV1 {
            policy: None,
            competence_trigger: None,
            execution_group: hiroute_domain::ExecutionGroupV1::Regular,
            simple_probability: None,
            simple_threshold_millis: None,
            selection_reason: hiroute_domain::ModelGroupReasonV1::Heuristic,
            strategy_id: "hiroute-complexity-v1".to_owned(),
            schema_version: "hiroute-route-strategy-v1".to_owned(),
            payload_digest: digest("complexity"),
            branch_id: hiroute_domain::SMART_SAVING_SCOPE_ID.into(),
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
            group_id: "group-primary".to_owned(),
            ranked_candidate_ids: vec!["candidate-a".to_owned()],
        }],
        reason_ledger: vec![hiroute_domain::ReasonLedgerEntryV1 {
            ordinal: 0,
            code: LedgerReasonCodeV1::CustomExactOrder,
            group_id: Some("group-primary".to_owned()),
        }],
        requirements: requirements(),
        requested_reasoning_disposition: RequestedReasoningDispositionV1::OverriddenByAgentPlan,
        requested_reasoning_value: Some(hiroute_domain::NativeReasoningValueV1::String(
            "high".to_owned(),
        )),
        requested_max_output_tokens: Some(4_096),
        stream: true,
        outcome: RouteDecisionOutcomeV1::Ready,
        outcome_code: None,
        max_attempts: 2,
    }))
}

pub(crate) fn candidate(model: &str) -> ExecutionFactV1 {
    ExecutionFactV1::CandidateDecision(Box::new(CandidateDecisionFactV1 {
        candidate_id: "candidate-a".to_owned(),
        stable_binding_id: "binding-a".to_owned(),
        group_id: "group-primary".to_owned(),
        declared_order: 0,
        profile_digest: digest("profile-a").into(),
        ingress_protocol: IngressProtocolV1::Responses,
        upstream_protocol: IngressProtocolV1::Responses,
        path_id: "responses-to-responses".to_owned(),
        provider_id: "provider-a".to_owned(),
        endpoint_id: "endpoint-a".to_owned(),
        entitlement_id: "entitlement-a".to_owned(),
        connector_id: "connector-a".to_owned(),
        connector_revision: "connector-v1".to_owned(),
        capability_id: "capability-a".to_owned(),
        capability_revision: "capability-v4".to_owned(),
        model_configuration_id: model.to_owned(),
        native_model: "native-a".to_owned(),
        adapter_revision: "adapter-v1".to_owned(),
        serializer_revision: "serializer-v1".to_owned(),
        decoder_revision: "decoder-v1".to_owned(),
        target_serialized_bytes: 512,
        eligible: true,
        exclusion_reason: None,
        reasoning_profile_id: Some("reasoning-high".to_owned()),
        overall_score_tenths: Some(47),
        effective_cost_micros: Some(300),
        api_equivalent_cost_micros: Some(900),
        cost_class: CostClassV1::Subscription,
        cache_cost: Some(CacheCostFactV1::Confirmed {
            projected_cost_micros: 300,
        }),
        cache_affinity: true,
        compute_scope_order: 1,
        ranking_reasons: vec![hiroute_domain::RankingReasonCodeV1::ConfirmedCacheHold],
    }))
}

pub(crate) fn credential() -> ExecutionFactV1 {
    ExecutionFactV1::CredentialLease {
        stable_binding_id: "binding-a".to_owned(),
        credential_ref: "credential-ref-a".to_owned(),
        key_id: Some("key-a".to_owned()),
        credential_generation: Some(6),
        excluded_key_count: 0,
        outcome: hiroute_domain::CredentialLeaseOutcomeV1::Leased,
    }
}

pub(crate) fn runtime_state() -> ExecutionFactV1 {
    ExecutionFactV1::RuntimeState {
        operation: hiroute_domain::RuntimeOperationV1::ReadExact,
        key_scope: hiroute_domain::RuntimeKeyScopeV1::Credential,
        stable_binding_id: "binding-a".to_owned(),
        credential_ref: Some("credential-ref-a".to_owned()),
        key_id: Some("key-a".to_owned()),
        expected_generation: None,
        observed_generation: Some(6),
        health: Some(hiroute_domain::RuntimeHealthV1::Active),
        cooldown_remaining_millis: None,
        probe_lease_remaining_millis: None,
        transient_backoff_step: Some(0),
        outcome: hiroute_domain::RuntimeOperationOutcomeV1::Ok,
    }
}

pub(crate) fn attempt_started(model: &str) -> ExecutionFactV1 {
    ExecutionFactV1::AttemptStarted {
        ordinal: 1,
        candidate_id: "candidate-a".to_owned(),
        stable_binding_id: "binding-a".to_owned(),
        profile_digest: digest("profile-a").into(),
        credential_ref: "credential-ref-a".to_owned(),
        key_id: "key-a".to_owned(),
        provider_name: "provider-a".to_owned(),
        request_model: "native-a".to_owned(),
        upstream_protocol: IngressProtocolV1::Responses,
        model_configuration_id: model.to_owned(),
        adapter_revision: "adapter-v1".to_owned(),
        start_reason: "initial_candidate".to_owned(),
        previous_attempt_id: None,
    }
}

pub(crate) fn semantic_commit() -> ExecutionFactV1 {
    ExecutionFactV1::SemanticCommit {
        ordinal: 1,
        boundary: hiroute_domain::SemanticCommitBoundaryV1::FullFrameTransportAccepted,
        frame_id: "frame-1".to_owned(),
    }
}

pub(crate) fn usage_and_cache() -> ExecutionFactV1 {
    ExecutionFactV1::UsageAndCache {
        ordinal: 1,
        source: hiroute_domain::UsageSourceV1::AcceptedCanonicalModelEvent,
        input_tokens: Some(100),
        output_tokens: Some(25),
        billable_tokens: Some(125),
        cache_read_tokens: Some(40),
        cache_write_tokens: Some(5),
        reasoning_tokens: Some(7),
        input_provenance: hiroute_domain::UsageProvenanceV1::Reported,
        output_provenance: hiroute_domain::UsageProvenanceV1::Reported,
        billable_provenance: hiroute_domain::UsageProvenanceV1::Reported,
        cache_read_provenance: hiroute_domain::UsageProvenanceV1::Reported,
        cache_write_provenance: hiroute_domain::UsageProvenanceV1::Reported,
        reasoning_provenance: hiroute_domain::UsageProvenanceV1::Reported,
        effective_cost_micros: Some(300),
        cost_class: Some(CostClassV1::Subscription),
        cache_status: hiroute_domain::CacheStatusV1::ConfirmedUsage,
    }
}

pub(crate) fn attempt_finished() -> ExecutionFactV1 {
    ExecutionFactV1::AttemptFinished(Box::new(AttemptFinishedFactV1 {
        ordinal: 1,
        stable_binding_id: "binding-a".to_owned(),
        outcome: hiroute_domain::AttemptOutcomeV1::Accepted,
        error_class: None,
        retryable: None,
        duration_micros: 9_000,
        disposition: hiroute_domain::AttemptDispositionV1::Accept,
        provider_http_status: Some(200),
        provider_code: None,
        provider_request_id: Some("provider-request-1".to_owned()),
        retry_after_millis: None,
        reset_after_millis: None,
        provider_readiness: Some("ready".to_owned()),
        provider_model_event: Some("response.completed".to_owned()),
        time_to_first_model_event_micros: Some(2_000),
        provider_ended_micros_from_start: Some(8_500),
        transport: AttemptTransportFactV1 {
            connect_micros: Some(100),
            request_write_micros: Some(200),
            upstream_ttfb_micros: Some(1_900),
            last_upstream_progress_micros_from_start: Some(8_000),
            local_read_suppressed_micros: 0,
            upstream_body_bytes: 640,
            timeout_kind: None,
        },
        commits: AttemptCommitFactV1 {
            upstream_request: hiroute_domain::CommitFenceV1::WriteConfirmed,
            downstream_headers: hiroute_domain::CommitFenceV1::WriteConfirmed,
            downstream_semantic: hiroute_domain::CommitFenceV1::WriteConfirmed,
        },
        stream_outcome: hiroute_domain::AttemptStreamOutcomeV1::CompletedEos,
        downstream_outcome: hiroute_domain::AttemptDownstreamOutcomeV1::Completed,
        cleanup_outcome: hiroute_domain::AttemptCleanupOutcomeV1::Completed,
        termination_reason: hiroute_domain::AttemptTerminationReasonV1::AcceptedEos,
    }))
}

pub(crate) fn request_finished() -> ExecutionFactV1 {
    ExecutionFactV1::RequestFinished {
        outcome: hiroute_domain::ExecutionRequestOutcomeV1::Accepted,
        attempts_started: 1,
        attempts_finished: 1,
        accepted_attempt_ordinal: Some(1),
        facts_completeness: hiroute_domain::FactsCompleteness::Complete,
    }
}

pub(crate) fn finished(attempt_id: &str) -> ExecutionFactV1 {
    finished_with_value(
        attempt_id,
        "plan/codex-daily",
        "USD",
        Some(1_200),
        Some(900),
        Some(300),
    )
}

pub(crate) fn finished_with_value(
    _attempt_id: &str,
    agent_plan_id: &str,
    currency: &str,
    baseline: Option<u64>,
    chosen: Option<u64>,
    actual: Option<u64>,
) -> ExecutionFactV1 {
    ExecutionFactV1::ValueSnapshot {
        traffic_kind: TrafficKind::Normal,
        usage: UsageFactsV1 {
            input_tokens: 100,
            output_tokens: 25,
            cache_read_tokens: 40,
            cache_write_tokens: 5,
            reasoning_tokens: 7,
        },
        value: FrozenValueFactsV1 {
            agent_plan_id: AgentPlanId::parse(agent_plan_id).unwrap(),
            currency: currency.to_owned(),
            billing_unit: format!("micro_{}", currency.to_ascii_lowercase()),
            price_version: "price-v9".to_owned(),
            price_override_revision: Some("override-v2".to_owned()),
            baseline_api_equivalent_cost_micros: baseline,
            chosen_api_equivalent_cost_micros: chosen,
            actual_incremental_cost_micros: actual,
            routing_savings_micros: difference(baseline, chosen),
            entitlement_savings_micros: difference(chosen, actual),
            estimated_total_savings_micros: difference(baseline, actual),
        },
    }
}

pub(crate) fn request_facts(
    fixture: &Fixture,
    attempt_id: &str,
    value_snapshot: ExecutionFactV1,
    occurred_at_ms: i64,
) -> Vec<ExecutionFactEnvelopeV1> {
    let plan_id = match &value_snapshot {
        ExecutionFactV1::ValueSnapshot { value, .. } => value.agent_plan_id.as_str().to_owned(),
        _ => panic!("request fixture requires a ValueSnapshot"),
    };
    vec![
        fixture.fact_for_plan(1, route_decision(&plan_id), occurred_at_ms, &plan_id, None),
        fixture.fact_for_plan(2, candidate("model-a"), occurred_at_ms + 1, &plan_id, None),
        fixture.fact_for_plan(3, credential(), occurred_at_ms + 2, &plan_id, None),
        fixture.fact_for_plan(4, runtime_state(), occurred_at_ms + 3, &plan_id, None),
        fixture.attempt_fact(
            5,
            attempt_id,
            attempt_started("model-a"),
            occurred_at_ms + 4,
            &plan_id,
        ),
        fixture.attempt_fact(
            6,
            attempt_id,
            semantic_commit(),
            occurred_at_ms + 5,
            &plan_id,
        ),
        fixture.attempt_fact(
            7,
            attempt_id,
            usage_and_cache(),
            occurred_at_ms + 5,
            &plan_id,
        ),
        fixture.attempt_fact(
            8,
            attempt_id,
            attempt_finished(),
            occurred_at_ms + 5,
            &plan_id,
        ),
        fixture.fact_for_plan(9, value_snapshot, occurred_at_ms + 5, &plan_id, None),
        fixture.fact_for_plan(10, request_finished(), occurred_at_ms + 5, &plan_id, None),
    ]
}

fn requirements() -> RequestCapabilityRequirementsV1 {
    RequestCapabilityRequirementsV1 {
        ingress_protocol: IngressProtocolV1::Responses,
        text: true,
        initial_instructions: true,
        mid_conversation_instructions: false,
        image_url: false,
        image_base64: false,
        image_media_types: vec![],
        function_tools: true,
        strict_tools: true,
        tool_choice: ToolChoiceRequirementV1::Auto,
        parallel_tools: true,
        tool_roundtrip: true,
        tool_result_text: true,
        tool_result_json: true,
        logical_tool_id_mapping: true,
        streaming: true,
        stream_text: true,
        stream_tool_arguments: true,
        stream_reasoning: true,
        stream_usage: true,
        provider_state: false,
    }
}

fn digest(value: &str) -> hiroute_domain::CanonicalDigest {
    hiroute_domain::CanonicalDigest::of_bytes(value.as_bytes())
}

fn difference(left: Option<u64>, right: Option<u64>) -> Option<i64> {
    Some(i64::try_from(left?).unwrap() - i64::try_from(right?).unwrap())
}

pub(crate) fn value_query(
    agent_plan_id: &str,
    currency: &str,
    session_id: Option<SessionId>,
) -> ValueQueryV1 {
    ValueQueryV1 {
        agent_plan_id: AgentPlanId::parse(agent_plan_id).unwrap(),
        from_ms: 0,
        to_ms: i64::MAX,
        group_by: ValueGroupByV1::None,
        currency: currency.to_owned(),
        session_id,
    }
}

pub(crate) fn install_value_request(
    store: &Arc<LocalObservationStore>,
    fixture: &Fixture,
    _receipt_id: &str,
    attempt_id: &str,
    value_snapshot: ExecutionFactV1,
    occurred_at_ms: i64,
) {
    let writer = writer(store);
    let channel = fact_channel(fixture, 256 * 1024);
    for envelope in request_facts(fixture, attempt_id, value_snapshot, occurred_at_ms) {
        assert!(matches!(
            offer_fact(&writer, &channel, envelope),
            WriterCycleOutcome::Ack(_)
        ));
    }
}

pub(crate) fn install_content(
    store: &Arc<LocalObservationStore>,
    fixture: &Fixture,
    bytes: &[u8],
    occurred_at_ms: i64,
) {
    let writer = writer(store);
    let channel = content_channel(fixture, 128 * 1024);
    for envelope in fixture.request_content_events(bytes, occurred_at_ms) {
        assert!(matches!(
            offer_content(&writer, &channel, envelope),
            WriterCycleOutcome::Ack(_)
        ));
    }
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(TABLE[((value >> 18) & 63) as usize] as char);
        output.push(TABLE[((value >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}
