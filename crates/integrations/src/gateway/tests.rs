use std::collections::BTreeSet;

use hiroute_domain::{
    ConversationContentDirectionV2, ConversationContentPhaseV2, OBSERVATION_ACK_SCHEMA_V2,
    OBSERVATION_NACK_SCHEMA_V1, ObservationAckV2, ObservationBlobAcknowledgementV1,
    ObservationContentAcknowledgementV1, ObservationDigestSubjectV1, ObservationFeedback,
    ObservationFeedbackIdentityV1, ObservationNackDetailV1, ObservationNackV1,
    ObservationSequenceRangeV1,
};
use hiroute_gateway::server::core_runtime::observation::{
    CONVERSATION_CONTENT_PORT_DIGEST, CONVERSATION_CONTENT_SCHEMA, EXECUTION_FACT_PORT_DIGEST,
    EXECUTION_FACT_SCHEMA, LIFECYCLE_FACT_PORT_DIGEST, LIFECYCLE_FACT_SCHEMA,
};
use serde_json::{Value, json};

use super::*;

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn producer(component: &str) -> Value {
    json!({
        "component": component,
        "revision": "gateway-observation/1",
        "producer_id": "producer-main",
        "producer_epoch": "epoch-main",
        "stream_id": "stream-main"
    })
}

fn correlation() -> Value {
    json!({
        "workspace_id": "personal/default",
        "conversation_id": "conversation-main",
        "session_scope": "conversation",
        "correlation_provenance": "agent_supplied",
        "turn_id": "turn-main",
        "request_id": "request-main"
    })
}

#[test]
fn fixed_publication_digest_matches_the_actual_gateway_binding() {
    use hiroute_domain as domain;
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap();
    let plan: domain::CompiledAgentPlanV1 =
        serde_json::from_value(fixture["plans"][0].clone()).unwrap();
    let plan = plan.into_current().unwrap();
    let binding = plan.body.materialized.attempt_owned.groups[0].candidates[0].clone();
    let grant = domain::AgentModelGrantV2::seal(
        domain::AgentIngressProtocolV1::Responses,
        [(
            "Native.Model/v1".into(),
            domain::AgentModelRouteV2::Fixed {
                candidate: domain::CandidateSelectionV1 {
                    binding_id: binding.binding_id.clone(),
                    reasoning: None,
                },
                binding: Box::new(binding.clone()),
            },
        )]
        .into(),
    )
    .unwrap();
    let publication = domain::GatewayPublicationV1::new(
        domain::WorkspaceId::default(),
        domain::GatewayPublicationRevision::new(1).unwrap(),
        domain::AliasRegistryV1::default(),
        Vec::new(),
    )
    .unwrap()
    .next_with_access_grant(
        domain::GatewayPublicationRevision::new(2).unwrap(),
        domain::GatewayAccessGrantV1::new(
            "fixed-adapter-grant",
            1,
            domain::CanonicalDigest::of_bytes(b"fixed-adapter-token"),
            domain::AgentIngressProtocolV1::Responses,
            grant,
        )
        .unwrap(),
    )
    .unwrap();
    let product = publication.gateway_snapshot().unwrap();
    let gateway = project_publication(&product).unwrap();
    assert!(gateway.aliases.is_empty());
    let hiroute_gateway::server::publication::ModelRouteV2::Fixed {
        binding: projected,
        binding_digest,
        ..
    } = &gateway.grants[0].routes["Native.Model/v1"]
    else {
        panic!("expected fixed route");
    };
    assert_eq!(
        *binding_digest,
        domain::CanonicalDigest::of(projected.as_ref()).unwrap()
    );
    assert_eq!(projected.credential_refs, binding.credential_refs);
    assert_eq!(projected.upstream_model_id, binding.upstream_model_id);
    assert_eq!(
        projected
            .pricing_identity
            .as_ref()
            .unwrap()
            .source_identity_digest,
        binding.source_identity_digest
    );
    let mut changed = product;
    let domain::GatewayModelRouteV2::Fixed { binding_digest, .. } =
        changed.grants[0].routes.values_mut().next().unwrap()
    else {
        panic!("expected fixed route");
    };
    *binding_digest = domain::CanonicalDigest::of(&binding).unwrap();
    changed.payload_digest = changed.canonical_digest().unwrap().to_string();
    assert!(project_publication(&changed).is_err());
}

#[test]
fn gateway_adapter_projects_lifecycle_exactly_and_rejects_unknown_field() {
    let payload = json!({
        "schema_version": LIFECYCLE_FACT_SCHEMA,
        "schema_digest": LIFECYCLE_FACT_PORT_DIGEST,
        "channel": "lifecycle",
        "producer": producer("gateway-lifecycle"),
        "sequence": 1,
        "event_id": "lifecycle-event-1",
        "correlation": correlation(),
        "occurred_at_unix_nanos": 1,
        "fact": {"kind": "request_accepted", "ingress_protocol": "responses"},
        "loss_watermark": null,
        "completeness_delta": null
    });
    let encoded = serde_json::to_vec(&payload).unwrap();
    let projected = project_lifecycle_payload(&encoded).unwrap();
    assert_eq!(serde_json::to_value(projected).unwrap(), payload);

    let mut unknown = payload;
    unknown["adapter_cache"] = json!(true);
    assert_eq!(
        project_lifecycle_payload(&serde_json::to_vec(&unknown).unwrap()),
        Err(GatewayProjectionError::InvalidInput)
    );
}

#[test]
fn gateway_adapter_projects_flat_execution_receipt_into_frozen_trust() {
    let payload = execution_payload(
        json!({
            "kind": "request_finished",
            "outcome": "accepted",
            "attempts_started": 1,
            "attempts_finished": 1,
            "accepted_attempt_ordinal": 1,
            "facts_completeness": "complete"
        }),
        9,
        false,
    );
    let projected = project_execution_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
    let projected = serde_json::to_value(projected).unwrap();
    assert_eq!(
        projected["schema_version"],
        hiroute_domain::EXECUTION_FACT_SCHEMA_V2
    );
    assert!(projected["pricing"].is_null());
    assert_eq!(projected["trust"]["authority_id"], "authority-main");
    assert_eq!(projected["trust"]["plan_display_name"], "Coding route");
    assert_eq!(
        projected["producer"]["stream"]["producer_id"],
        "producer-main"
    );
    assert!(projected.get("authority_id").is_none());
    assert!(projected["producer"].get("producer_id").is_none());
    assert_eq!(projected["fact"], payload["fact"]);
    assert_eq!(projected["sequence"], 9);

    let mut legacy = payload.clone();
    legacy.as_object_mut().unwrap().remove("plan_display_name");
    let legacy = project_execution_payload(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(legacy.trust.plan_display_name.is_none());

    let mut wrong_digest = payload;
    wrong_digest["schema_digest"] = json!(DIGEST);
    assert_eq!(
        project_execution_payload(&serde_json::to_vec(&wrong_digest).unwrap()),
        Err(GatewayProjectionError::UnsupportedSchema)
    );
}

#[test]
fn fixed_gateway_projection_preserves_binding_without_plan_provenance() {
    let mut payload = execution_payload(complete_execution_facts().remove(0), 1, false);
    let route = json!({"kind": "fixed", "binding_digest": DIGEST});
    payload["agent_plan_id"] = Value::Null;
    payload.as_object_mut().unwrap().remove("plan_display_name");
    payload["route"] = route.clone();
    payload["fact"]["plan_id"] = Value::Null;
    payload["fact"]["route"] = route.clone();
    let projected = project_execution_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert!(projected.trust.agent_plan_id.is_none());
    assert!(projected.trust.plan_display_name.is_none());
    assert_eq!(serde_json::to_value(&projected.trust.route).unwrap(), route);
    assert_eq!(
        serde_json::to_value(&projected.fact).unwrap()["route"],
        route
    );

    payload["agent_plan_id"] = json!("plan/coding");
    assert_eq!(
        project_execution_payload(&serde_json::to_vec(&payload).unwrap()),
        Err(GatewayProjectionError::InvalidOutput)
    );
    payload["agent_plan_id"] = Value::Null;
    payload["plan_display_name"] = json!("False plan");
    assert_eq!(
        project_execution_payload(&serde_json::to_vec(&payload).unwrap()),
        Err(GatewayProjectionError::InvalidOutput)
    );
}

#[test]
fn priced_and_unpriced_gateway_facts_project_to_one_product_contract() {
    let fact = complete_execution_facts()
        .into_iter()
        .find(|fact| fact["kind"] == "attempt_started")
        .unwrap();
    let mut priced = execution_payload(fact, 1, true);
    priced["pricing"] = json!({
        "schema_version": "hiroute.observation.execution-pricing/v1",
        "request_generation": null,
        "captured_at_ms": 0,
        "attempt_execution_at_ms": 0,
        "quote": null,
        "reference_quote": null,
        "unknown_reason": "snapshot_unavailable"
    });
    let priced = project_execution_payload(&serde_json::to_vec(&priced).unwrap()).unwrap();
    assert_eq!(
        priced.schema_version,
        hiroute_domain::EXECUTION_FACT_SCHEMA_V2
    );
    assert!(priced.pricing.is_some());

    let unpriced = execution_payload(
        json!({
            "kind": "request_finished",
            "outcome": "accepted",
            "attempts_started": 1,
            "attempts_finished": 1,
            "accepted_attempt_ordinal": 1,
            "facts_completeness": "complete"
        }),
        2,
        false,
    );
    let unpriced = project_execution_payload(&serde_json::to_vec(&unpriced).unwrap()).unwrap();
    assert_eq!(
        unpriced.schema_version,
        hiroute_domain::EXECUTION_FACT_SCHEMA_V2
    );
    assert!(unpriced.pricing.is_none());
}

#[test]
fn gateway_adapter_tags_provider_native_reasoning_for_product_receipts() {
    let payload = execution_payload(
        json!({
            "kind": "route_decision",
            "planner_version": "hiroute-deterministic-planner/v1",
            "plan_id": "plan/coding",
            "route": {"kind": "plan", "revision": 7, "semantic_digest": DIGEST},
            "input_digest": DIGEST,
            "policy_digest": DIGEST,
            "output_digest": DIGEST,
            "branch": "smart_saving_complex",
            "complexity": null,
            "groups": [],
            "reason_ledger": [],
            "requirements": {
                "text": true,
                "image_url": false,
                "image_base64": false,
                "image_media_types": [],
                "function_tools": false,
                "strict_tools": false,
                "parallel_tools": false,
                "tool_choice": {"kind": "auto"},
                "tool_result_text": false,
                "tool_result_json": false,
                "tool_roundtrip": false,
                "logical_tool_id_mapping": false,
                "provider_state": false,
                "initial_instructions": false,
                "mid_conversation_instructions": false,
                "streaming": true,
                "stream_text": true,
                "stream_reasoning": true,
                "stream_tool_arguments": false,
                "stream_usage": true,
                "ingress_protocol": "messages"
            },
            "requested_reasoning_disposition": "overridden_by_agent_plan",
            "requested_reasoning_value": {
                "thinking": {"type": "enabled", "budget_tokens": 1024},
                "output_config": null
            },
            "requested_max_output_tokens": 2048,
            "stream": true,
            "outcome": "ready",
            "outcome_code": null,
            "max_attempts": 2
        }),
        1,
        false,
    );
    let projected = project_execution_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
    let projected = serde_json::to_value(projected).unwrap();
    assert_eq!(
        projected["fact"]["requested_reasoning_value"]["kind"],
        "object"
    );
    assert_eq!(
        projected["fact"]["requested_reasoning_value"]["value"]["thinking"]["value"]["budget_tokens"]
            ["value"],
        1024
    );
}

#[test]
fn gateway_adapter_projects_every_execution_fact_variant_without_loss() {
    let facts = complete_execution_facts();
    assert_eq!(facts.len(), 9);
    let mut projected_kinds = BTreeSet::new();

    for (index, fact) in facts.into_iter().enumerate() {
        let sequence = u64::try_from(index).unwrap() + 1;
        let kind = fact["kind"].as_str().unwrap().to_owned();
        let attempt_scoped = matches!(
            kind.as_str(),
            "attempt_started" | "attempt_finished" | "semantic_commit" | "usage_and_cache"
        );
        let payload = execution_payload(fact.clone(), sequence, attempt_scoped);
        let projected = project_execution_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
        let projected = serde_json::to_value(projected).unwrap();

        assert_eq!(projected["fact"], fact, "{kind} changed in projection");
        assert_eq!(projected["sequence"], sequence);
        assert_eq!(projected["attempt_id"].is_null(), !attempt_scoped);
        projected_kinds.insert(kind);
    }

    assert_eq!(
        projected_kinds,
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
fn gateway_adapter_projects_context_hold_reasons_without_loss() {
    for (sequence, code) in ["CONTEXT_HOLD_APPLIED", "CONTEXT_HOLD_INVALIDATED"]
        .into_iter()
        .enumerate()
    {
        let mut route = complete_execution_facts().remove(0);
        route["reason_ledger"] = json!([{
            "ordinal": 0,
            "code": code,
            "group_id": null
        }]);
        let sequence = u64::try_from(sequence).unwrap() + 1;
        let payload = execution_payload(route.clone(), sequence, false);
        let projected = project_execution_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(projected).unwrap()["fact"], route);
    }

    let mut candidate = complete_execution_facts().remove(1);
    candidate["ranking_reasons"] = json!(["CONTEXT_MODEL_HOLD", "PUBLISHED_MANUAL_ORDER"]);
    let payload = execution_payload(candidate.clone(), 3, false);
    let projected = project_execution_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(projected).unwrap()["fact"], candidate);
}

#[test]
fn gateway_adapter_projects_previous_success_fallback_without_loss() {
    let mut route = complete_execution_facts().remove(0);
    route["reason_ledger"] = json!([{
        "ordinal": 0,
        "code": "PREVIOUS_SUCCESS_FALLBACK",
        "group_id": "primary"
    }]);
    let payload = execution_payload(route.clone(), 1, false);
    let projected = project_execution_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(projected).unwrap()["fact"], route);

    let mut candidate = complete_execution_facts().remove(1);
    candidate["ranking_reasons"] = json!(["PREVIOUS_SUCCESS_FALLBACK"]);
    let payload = execution_payload(candidate.clone(), 2, false);
    let projected = project_execution_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(projected).unwrap()["fact"], candidate);
}

#[test]
fn gateway_adapter_requires_plan_identity_for_branch_assessment() {
    let fact = json!({
        "kind": "branch_assessment_recorded",
        "segment_id": "segment-main",
        "plan_id": "plan/coding",
        "plan_revision": 7,
        "model_configuration_id": "model-config-main",
        "profile_digest": DIGEST,
        "trigger_request_id": "request-main",
        "target_from_turn_id": "turn-main",
        "target_through_turn_id": "turn-main",
        "target_from_ordinal": 1,
        "target_through_ordinal": 1,
        "assessed_at_ms": 1,
        "score": 0.5,
        "partial": false,
        "reason": null
    });
    let payload = execution_payload(fact.clone(), 3, false);
    let projected = project_execution_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(projected).unwrap()["fact"], fact);

    let mut before_planner = payload;
    before_planner["agent_plan_id"] = Value::Null;
    assert_eq!(
        project_execution_payload(&serde_json::to_vec(&before_planner).unwrap()),
        Err(GatewayProjectionError::InvalidOutput)
    );
}

#[test]
fn gateway_adapter_content_event_and_rich_feedback_round_trip_without_loss() {
    let mut sequence = 1;
    for direction in ["request_input", "response_delivered"] {
        for phase in ["begin", "append", "finish", "abort"] {
            let payload = content_payload(direction, phase, sequence);
            let projected =
                project_content_payload(&serde_json::to_vec(&payload).unwrap()).unwrap();
            assert_eq!(serde_json::to_value(projected).unwrap(), payload);
            sequence += 1;
        }
    }

    let identity = ObservationFeedbackIdentityV1 {
        channel: "conversation_content".into(),
        producer_id: "producer-main".into(),
        producer_epoch: "epoch-main".into(),
        stream_id: "stream-main".into(),
    };
    let ack = ObservationAckV2 {
        schema_version: OBSERVATION_ACK_SCHEMA_V2.into(),
        identity: identity.clone(),
        highest_contiguous_sequence: 4,
        highest_accounted_sequence: 7,
        content_acknowledgement: Some(ObservationContentAcknowledgementV1 {
            request_id: "request-main".into(),
            direction: ConversationContentDirectionV2::RequestInput,
            fork_id: "fork-main".into(),
            next_chunk_ordinal: 3,
            transcript_root: Some("transcript-current".into()),
            delta_parent_transcript_root: Some("transcript-parent".into()),
            acknowledged_blobs: vec![ObservationBlobAcknowledgementV1 {
                content_id: "content-main".into(),
                digest: "blob-main".into(),
            }],
        }),
    };
    let projected_ack = project_feedback(ObservationFeedback::Ack(ack.clone()))
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(projected_ack).unwrap(),
        serde_json::to_value(ack).unwrap()
    );

    let details = vec![
        ObservationNackDetailV1::MissingSequenceRanges {
            ranges: vec![ObservationSequenceRangeV1 {
                first_sequence: 5,
                last_sequence: 7,
            }],
        },
        ObservationNackDetailV1::UnknownTranscriptRoot {
            request_id: "request-main".into(),
            direction: ConversationContentDirectionV2::RequestInput,
            fork_id: "fork-main".into(),
            transcript_root: format!("transcript-{}", "33".repeat(32)),
        },
        ObservationNackDetailV1::MissingBlob {
            request_id: "request-main".into(),
            direction: ConversationContentDirectionV2::ResponseDelivered,
            fork_id: "fork-main".into(),
            blobs: vec![ObservationBlobAcknowledgementV1 {
                content_id: "content-main".into(),
                digest: format!("blob-{}", "22".repeat(32)),
            }],
        },
        ObservationNackDetailV1::ChunkOrdinalConflict {
            request_id: "request-main".into(),
            direction: ConversationContentDirectionV2::RequestInput,
            fork_id: "fork-main".into(),
            expected_chunk_ordinal: 3,
            rejected_chunk_ordinal: 5,
        },
        ObservationNackDetailV1::ContentStateConflict {
            request_id: "request-main".into(),
            direction: ConversationContentDirectionV2::RequestInput,
            fork_id: "fork-main".into(),
            expected_phase: ConversationContentPhaseV2::Append,
            rejected_phase: ConversationContentPhaseV2::Finish,
        },
        ObservationNackDetailV1::DigestMismatch {
            subject: ObservationDigestSubjectV1::Transcript,
            subject_id: "request-main/fork-main".into(),
            expected_digest: format!("transcript-{}", "33".repeat(32)),
            rejected_digest: format!("transcript-{}", "44".repeat(32)),
        },
    ];
    for detail in details {
        let nack = ObservationNackV1 {
            schema_version: OBSERVATION_NACK_SCHEMA_V1.into(),
            identity: identity.clone(),
            rejected_sequence: 8,
            expected_sequence: 5,
            retryable: true,
            detail,
        };
        let projected_nack = project_feedback(ObservationFeedback::Nack(nack.clone()))
            .unwrap()
            .unwrap_err();
        assert_eq!(
            serde_json::to_value(projected_nack).unwrap(),
            serde_json::to_value(nack).unwrap()
        );
    }
}

fn content_payload(direction: &str, phase: &str, sequence: u64) -> Value {
    let mut payload = json!({
        "schema_version": CONVERSATION_CONTENT_SCHEMA,
        "schema_digest": CONVERSATION_CONTENT_PORT_DIGEST,
        "channel": "conversation_content",
        "producer": producer("gateway-content"),
        "sequence": sequence,
        "event_id": format!("content-event-{sequence}"),
        "correlation": correlation(),
        "direction": direction,
        "phase": phase,
        "attempt_id": null,
        "fork_id": format!("fork-{direction}"),
        "parent_transcript_root": null,
        "result_transcript_root": null,
        "message_instance_id": null,
        "message_role": null,
        "content_kind": null,
        "content_id": null,
        "content_blob_digest": null,
        "message_ordinal": null,
        "part_ordinal": null,
        "chunk_ordinal": null,
        "transport_frame_id": null,
        "canonical_media_type": null,
        "canonical_bytes_base64": null,
        "content_ref": null,
        "downstream_delivery": null,
        "abort_reason": null,
        "occurred_at_unix_nanos": sequence,
        "loss_watermark": null,
        "completeness_delta": null
    });
    let response = direction == "response_delivered";
    if response {
        payload["attempt_id"] = json!("attempt-main");
        payload["parent_transcript_root"] = json!(format!("transcript-{}", "11".repeat(32)));
    }
    match phase {
        "begin" => {}
        "append" => {
            let digest = format!("blob-{}", "22".repeat(32));
            payload["message_instance_id"] = json!(format!("message-{direction}"));
            payload["message_role"] = json!(if response { "assistant" } else { "user" });
            payload["content_kind"] = json!("text");
            payload["content_id"] = json!(format!("content-{direction}"));
            payload["content_blob_digest"] = json!(digest);
            payload["message_ordinal"] = json!(0);
            payload["part_ordinal"] = json!(0);
            payload["chunk_ordinal"] = json!(0);
            payload["canonical_bytes_base64"] = json!("e30=");
            if response {
                payload["transport_frame_id"] = json!("frame-main");
                payload["canonical_media_type"] =
                    json!("application/vnd.hiroute.model-stream-event+json;version=1");
                payload["content_ref"] = json!({
                    "content_id": format!("content-{direction}"),
                    "digest": format!("blob-{}", "22".repeat(32)),
                    "byte_count": 2,
                    "media_type": "application/vnd.hiroute.model-stream-event+json;version=1"
                });
                payload["downstream_delivery"] = json!("full_frame_transport_accepted");
            } else {
                payload["canonical_media_type"] = json!("application/json");
            }
        }
        "finish" | "abort" => {
            payload["result_transcript_root"] = json!(format!("transcript-{}", "33".repeat(32)));
            payload["completeness_delta"] = json!(if phase == "finish" {
                "complete"
            } else {
                "partial"
            });
            if response {
                payload["downstream_delivery"] = json!("full_frame_transport_accepted");
            }
            if phase == "abort" {
                payload["abort_reason"] = json!("producer_aborted");
            }
        }
        _ => unreachable!(),
    }
    payload
}

fn execution_payload(fact: Value, sequence: u64, attempt_scoped: bool) -> Value {
    json!({
        "schema_version": EXECUTION_FACT_SCHEMA,
        "schema_digest": EXECUTION_FACT_PORT_DIGEST,
        "channel": "execution_fact",
        "producer": producer("gateway-execution"),
        "sequence": sequence,
        "event_id": format!("execution-event-{sequence}"),
        "correlation": correlation(),
        "attempt_id": attempt_scoped.then_some("attempt-main"),
        "authority_id": "authority-main",
        "authority_epoch": 3,
        "served_model_id": "hiroute/coding",
        "selector_source": "trusted_model_alias",
        "agent_plan_id": "plan/coding",
        "route": {"kind": "plan", "revision": 7, "semantic_digest": DIGEST},
        "plan_display_name": "Coding route",
        "gateway_publication_revision": "11",
        "gateway_publication_digest": DIGEST,
        "grant_id": "grant-main",
        "grant_generation": 2,
        "ingress_protocol": "responses",
        "occurred_at_unix_nanos": sequence,
        "fact": fact,
        "loss_watermark": null,
        "completeness_delta": null
    })
}

fn complete_execution_facts() -> Vec<Value> {
    vec![
        json!({
            "kind": "route_decision",
            "planner_version": "hiroute-deterministic-planner/v1",
            "plan_id": "plan/coding",
            "route": {"kind": "plan", "revision": 7, "semantic_digest": DIGEST},
            "input_digest": DIGEST,
            "policy_digest": DIGEST,
            "output_digest": DIGEST,
            "branch": "custom_exact_order",
            "complexity": null,
            "groups": [],
            "reason_ledger": [],
            "requirements": {
                "text": true,
                "image_url": false,
                "image_base64": false,
                "image_media_types": [],
                "function_tools": false,
                "strict_tools": false,
                "parallel_tools": false,
                "tool_choice": {"kind": "auto"},
                "tool_result_text": false,
                "tool_result_json": false,
                "tool_roundtrip": false,
                "logical_tool_id_mapping": false,
                "provider_state": false,
                "initial_instructions": false,
                "mid_conversation_instructions": false,
                "streaming": false,
                "stream_text": false,
                "stream_reasoning": false,
                "stream_tool_arguments": false,
                "stream_usage": false,
                "ingress_protocol": "responses"
            },
            "requested_reasoning_disposition": "absent",
            "requested_reasoning_value": null,
            "requested_max_output_tokens": null,
            "stream": false,
            "outcome": "ready",
            "outcome_code": null,
            "max_attempts": 1
        }),
        json!({
            "kind": "candidate_decision",
            "candidate_id": "candidate-main",
            "stable_binding_id": "binding-main",
            "group_id": "authorized",
            "declared_order": 0,
            "profile_digest": DIGEST,
            "ingress_protocol": "responses",
            "upstream_protocol": "responses",
            "path_id": "responses-to-responses",
            "provider_id": "provider-main",
            "endpoint_id": "endpoint-main",
            "entitlement_id": "entitlement-main",
            "connector_id": "connector-main",
            "connector_revision": "1",
            "capability_id": "exact-responses-to-responses",
            "capability_revision": "1",
            "model_configuration_id": "model-config-main",
            "native_model": "native-model-main",
            "adapter_revision": "protocol-adapter/v1",
            "serializer_revision": "target-json/v1",
            "decoder_revision": "native-response/v1",
            "target_serialized_bytes": 1,
            "eligible": true,
            "exclusion_reason": null,
            "reasoning_profile_id": "fixed",
            "overall_score_tenths": null,
            "effective_cost_micros": null,
            "api_equivalent_cost_micros": null,
            "cost_class": "free",
            "cache_cost": {"kind": "none"},
            "cache_affinity": false,
            "compute_scope_order": 0,
            "ranking_reasons": ["PUBLISHED_MANUAL_ORDER"]
        }),
        json!({
            "kind": "credential_lease",
            "stable_binding_id": "binding-main",
            "credential_ref": "credential-main",
            "key_id": "key-main",
            "credential_generation": 1,
            "excluded_key_count": 0,
            "outcome": "leased"
        }),
        json!({
            "kind": "runtime_state",
            "operation": "read_exact",
            "key_scope": "binding",
            "stable_binding_id": "binding-main",
            "credential_ref": null,
            "key_id": null,
            "expected_generation": null,
            "observed_generation": 0,
            "health": "active",
            "cooldown_remaining_millis": null,
            "probe_lease_remaining_millis": null,
            "transient_backoff_step": 0,
            "outcome": "ok"
        }),
        json!({
            "kind": "attempt_started",
            "ordinal": 1,
            "candidate_id": "candidate-main",
            "stable_binding_id": "binding-main",
            "profile_digest": DIGEST,
            "credential_ref": "credential-main",
            "key_id": "key-main",
            "provider_name": "provider-main",
            "request_model": "native-model-main",
            "upstream_protocol": "responses",
            "model_configuration_id": "model-config-main",
            "adapter_revision": "protocol-adapter/v1",
            "start_reason": "initial_candidate",
            "previous_attempt_id": null
        }),
        json!({
            "kind": "attempt_finished",
            "ordinal": 1,
            "stable_binding_id": "binding-main",
            "outcome": "accepted",
            "error_class": null,
            "retryable": null,
            "duration_micros": 1,
            "disposition": "accept",
            "provider_http_status": 200,
            "provider_code": null,
            "provider_request_id": null,
            "retry_after_millis": null,
            "reset_after_millis": null,
            "provider_readiness": "semantic_response",
            "provider_model_event": "semantic_response",
            "time_to_first_model_event_micros": null,
            "provider_ended_micros_from_start": 1,
            "transport": {
                "connect_micros": 1,
                "request_write_micros": 1,
                "upstream_ttfb_micros": 1,
                "last_upstream_progress_micros_from_start": 1,
                "local_read_suppressed_micros": 0,
                "upstream_body_bytes": 1,
                "timeout_kind": null
            },
            "commits": {
                "upstream_request": "write_confirmed",
                "downstream_headers": "write_confirmed",
                "downstream_semantic": "write_confirmed"
            },
            "stream_outcome": "completed_eos",
            "downstream_outcome": "completed",
            "cleanup_outcome": "completed",
            "termination_reason": "accepted_eos"
        }),
        json!({
            "kind": "semantic_commit",
            "ordinal": 1,
            "boundary": "full_frame_transport_accepted",
            "frame_id": "frame-main"
        }),
        json!({
            "kind": "usage_and_cache",
            "ordinal": 1,
            "source": "accepted_canonical_model_event",
            "input_tokens": 3,
            "output_tokens": 2,
            "billable_tokens": null,
            "cache_read_tokens": null,
            "cache_write_tokens": null,
            "reasoning_tokens": null,
            "input_provenance": "reported",
            "output_provenance": "reported",
            "billable_provenance": "unknown",
            "cache_read_provenance": "unknown",
            "cache_write_provenance": "unknown",
            "reasoning_provenance": "unknown",
            "effective_cost_micros": null,
            "cost_class": "free",
            "cache_status": "unknown"
        }),
        json!({
            "kind": "request_finished",
            "outcome": "accepted",
            "attempts_started": 1,
            "attempts_finished": 1,
            "accepted_attempt_ordinal": 1,
            "facts_completeness": "complete"
        }),
    ]
}
