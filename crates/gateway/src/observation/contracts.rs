use serde::{Deserialize, Serialize};
#[cfg(test)]
use sha2::{Digest, Sha256};

pub const GATEWAY_PORT_SET_SCHEMA: &str = "hiroute.gateway.external-port-set/v2";
pub const RUNTIME_PUBLICATION_PORT_SCHEMA: &str = "hiroute.gateway.port.runtime-publication/v1";
pub const CREDENTIAL_PORT_SCHEMA: &str = "hiroute.gateway.port.credential/v1";
pub const RUNTIME_STATE_PORT_SCHEMA: &str = "hiroute.gateway.port.runtime-state/v1";
pub const LIFECYCLE_FACT_PORT_SCHEMA: &str = "hiroute.gateway.port.lifecycle-fact/v2";
pub const EXECUTION_FACT_PORT_SCHEMA: &str = "hiroute.gateway.port.execution-fact/v3";
pub const CONVERSATION_CONTENT_PORT_SCHEMA: &str = "hiroute.gateway.port.conversation-content/v2";

pub const RUNTIME_PUBLICATION_PORT_DIGEST: &str =
    "sha256:6b73f180e1618e27d110aa0d0ee5cc96136637a383dae49ad6a88f15b3ba8afa";
pub const CREDENTIAL_PORT_DIGEST: &str =
    "sha256:65e98a61df2676e21e78c250c73dc72104485c719de517bde7fa36022b6d1c1c";
pub const RUNTIME_STATE_PORT_DIGEST: &str =
    "sha256:c0a7cf1bcacc6d2034653620409e6d495c55584c0b1d75c80e7f6445ce5cefef";
pub const LIFECYCLE_FACT_PORT_DIGEST: &str =
    "sha256:762143e3ab3121bf40cb7039a73ea0b47a5724c38837ba4152f75fc54e09e316";
pub const EXECUTION_FACT_PORT_DIGEST: &str =
    "sha256:f8b98014f3f7dae247ca3e6c5046691cbe43ccbe705f6441dfd3301af4a5af0d";
pub const CONVERSATION_CONTENT_PORT_DIGEST: &str =
    "sha256:6f17cd772a0fb322d25dd31dc95e34325cbabbd68912b32f5bb9bed526adfe44";
pub const GATEWAY_PORT_SET_DIGEST: &str =
    "sha256:5d0b1cafa66bfe816867c2fc68dfecccb4b8f7958506d25d19193c51475a34a6";

const RUNTIME_PUBLICATION_CONTRACT: &str = r#"{"schema":"hiroute.gateway.port.runtime-publication/v1","operation":{"name":"pin","effect":"read_only","result":"hiroute.gateway.publication-snapshot/v3|null"},"invariants":["one immutable aggregate revision per request","no credential material","no runtime session identifier"]}"#;
const CREDENTIAL_CONTRACT: &str = r#"{"schema":"hiroute.gateway.port.credential/v1","operations":[{"name":"lease_exact","request":{"stable_binding_id":"string","credential_ref":"opaque_string","credential_destination_ref":"connection-option/string|compute-target/sha256","excluded_key_ids":"ordered_string_array","connector_runtime":"builtin_native|cpa_bridge","connector_id":"string","upstream_protocol":"responses|chat_completions|messages","upstream_model_id":"logical_string","native_transport_model":"operational_string","logical_endpoint":"sealed_uri","operational_target":"sealed_uri","operational_target_digest":"sha256","runtime_epoch":"u64|null","target_epoch":"u64|null","protocol_profile_digest":"sha256","request_path":"absolute_path","authentication":"bearer|api_key_header(header:string)|none"},"result":{"credential_ref":"opaque_string","key_id":"opaque_string","generation":"u64","authorization":"secret_non_debug_non_serialize"}},{"name":"lease_header_secret","request":{"secret_ref":"opaque_string","header_name":"http_header_name"},"result":{"credential_ref":"opaque_string","key_id":"opaque_string","generation":"u64","authorization":"sensitive_exact_header_capability"}}],"invariants":["exact selected credential only","no pool enumeration","no-credential profiles bypass the credential resolver and credential key state","logical and operational model identities remain distinct and sealed","destination and authentication come from the selected profile","target and profile digests match the sealed publication","header-secret leases carry no provider model protocol endpoint or destination authority","header-secret leases may apply only the configured header name","managed bridge epochs are rechecked before authorization","authorization never crosses observation"]}"#;
const RUNTIME_STATE_CONTRACT: &str = r#"{"schema":"hiroute.gateway.port.runtime-state/v1","operations":[{"name":"read_exact","result":"generation+active|disabled|cooling_down+probe_lease+transient_backoff_step"},{"name":"compare_and_swap_exact","precondition":"next.generation=expected+1","result":"applied(generation)|conflict"},{"name":"acquire_probe_lease_exact","result":"acquired(generation)|busy|conflict"}],"key":"binding|credential(stable_binding_id,credential_ref,key_id,credential_generation)","invariants":["binding transient_backoff_step is 0..5 and credential step is zero","lease acquisition and cancellation preserve transient_backoff_step","execution scope owns deadline and cancellation","observation never owns correctness"]}"#;
const LIFECYCLE_FACT_CONTRACT: &str = r#"{"schema":"hiroute.gateway.port.lifecycle-fact/v2","envelope":"hiroute.gateway.lifecycle-fact-envelope/v2","identity":["producer_id","producer_epoch","stream_id","sequence","event_id"],"facts":[{"kind":"request_accepted","fields":["ingress_protocol"]},{"kind":"canonical_request_accepted","fields":["canonicalization_version"]},{"kind":"attempt_started","fields":["ordinal"]},{"kind":"attempt_finished","fields":["ordinal","outcome"]},{"kind":"response_frame_accepted","fields":["frame_id","byte_count","downstream_delivery"]},{"kind":"request_finished","fields":["outcome"]}],"delivery":["ack","nack","loss_watermark","producer_gap_heartbeat"],"feedback":{"identity_fields":["channel","producer_id","producer_epoch","stream_id"],"ack":{"schema":"hiroute.observation.ack/v2","fields":["identity","highest_contiguous_sequence","highest_accounted_sequence","content_acknowledgement?"],"invariants":["highest_contiguous_sequence<=highest_accounted_sequence","durable_gap_advances_accounted_only","identity_matches_delivered_record"]},"content_acknowledgement":"forbidden","nack":{"schema":"hiroute.observation.nack/v1","fields":["identity","rejected_sequence","expected_sequence","retryable","detail"],"detail_tag":"kind","detail_kinds":["receiver_unavailable","unsupported_schema","invalid_envelope","missing_sequence_ranges","sequence_event_conflict","missing_prerequisite","unknown_transcript_root","missing_blob","chunk_ordinal_conflict","content_state_conflict","digest_mismatch","immutable_projection_conflict"],"free_form_reason":"forbidden"}},"content":"forbidden"}"#;
const EXECUTION_FACT_CONTRACT: &str = r#"{"schema":"hiroute.gateway.port.execution-fact/v3","envelope":"hiroute.observation.execution-fact-envelope/v3","optional_pricing":{"product_schema":"hiroute.observation.product-execution-envelope/v2","optional_field":"pricing","pricing_schema":"hiroute.observation.execution-pricing/v1","scope":"attempt_started_only"},"identity":["producer_id","producer_epoch","stream_id","sequence","event_id","request_id","attempt_id?"],"route_identity":{"tag":"kind","plan":{"revision":"positive_u64","semantic_digest":"sha256"},"fixed":{"binding_digest":"sha256"},"plan_identity":"required_for_plan_null_for_fixed","fixed_plan_display_name":"forbidden","route_decision_matches_trust":true},"receipt_context":["authority_id","authority_epoch","served_model_id","selector_source","agent_plan_id?","route","gateway_publication_revision","gateway_publication_digest","grant_id","grant_generation","ingress_protocol"],"facts":[{"kind":"route_decision","fields":["planner_version","plan_id?","route","input_digest","policy_digest","output_digest","branch","complexity?","groups","reason_ledger","requirements","requested_reasoning_disposition","requested_reasoning_value?","requested_max_output_tokens?","stream","outcome","outcome_code?","max_attempts"],"complexity_fields":["strategy_id","schema_version","payload_digest","branch_id","policy?","competence_trigger?","execution_group","simple_probability?","simple_threshold_millis?","selection_reason","complexity_score?","threshold?","decision_source","reason_codes","matched_user_phrase_ids","fallback_used","classification_duration_micros?","fallback_reason?"],"complexity_decision_sources":["inherited","external_classifier","user_phrase","builtin_rules","unresolved"],"classifier_fallback_reasons":["timeout","unavailable","rejected_input","invalid_output"]},{"kind":"candidate_decision","fields":["candidate_id","stable_binding_id","group_id","declared_order","profile_digest","ingress_protocol","upstream_protocol","path_id","provider_id","endpoint_id","entitlement_id","connector_id","connector_revision","capability_id","capability_revision","model_configuration_id","native_model","adapter_revision","serializer_revision","decoder_revision","target_serialized_bytes","eligible","exclusion_reason?","reasoning_profile_id?","overall_score_tenths?","effective_cost_micros?","api_equivalent_cost_micros?","cost_class","cache_cost","cache_affinity","compute_scope_order","ranking_reasons"]},{"kind":"credential_lease","fields":["stable_binding_id","credential_ref","key_id?","credential_generation?","excluded_key_count","outcome"]},{"kind":"runtime_state","fields":["operation","key_scope","stable_binding_id","credential_ref?","key_id?","expected_generation?","observed_generation?","health?","cooldown_remaining_millis?","probe_lease_remaining_millis?","transient_backoff_step?","outcome"]},{"kind":"attempt_started","fields":["ordinal","candidate_id","stable_binding_id","profile_digest","credential_ref","key_id","provider_name","request_model","upstream_protocol","model_configuration_id","adapter_revision","start_reason","previous_attempt_id?"]},{"kind":"attempt_finished","fields":["ordinal","stable_binding_id","outcome","error_class?","retryable?","duration_micros","disposition","provider_http_status?","provider_code?","provider_request_id?","retry_after_millis?","reset_after_millis?","provider_readiness?","provider_model_event?","time_to_first_model_event_micros?","provider_ended_micros_from_start?","transport","commits","stream_outcome","downstream_outcome","cleanup_outcome","termination_reason"],"transport_fields":["connect_micros?","request_write_micros?","upstream_ttfb_micros?","last_upstream_progress_micros_from_start?","local_read_suppressed_micros","upstream_body_bytes","timeout_kind?"],"commit_fields":["upstream_request","downstream_headers","downstream_semantic"]},{"kind":"semantic_commit","fields":["ordinal","boundary","frame_id"],"boundary":"full_frame_transport_accepted"},{"kind":"usage_and_cache","fields":["ordinal","source","input_tokens?","output_tokens?","billable_tokens?","cache_read_tokens?","cache_write_tokens?","reasoning_tokens?","input_provenance","output_provenance","billable_provenance","cache_read_provenance","cache_write_provenance","reasoning_provenance","effective_cost_micros?","cost_class?","cache_status"]},{"kind":"request_finished","fields":["outcome","attempts_started","attempts_finished","accepted_attempt_ordinal?","facts_completeness"]}],"delivery":["ack","nack","loss_watermark","producer_gap_heartbeat"],"feedback":{"identity_fields":["channel","producer_id","producer_epoch","stream_id"],"ack":{"schema":"hiroute.observation.ack/v2","fields":["identity","highest_contiguous_sequence","highest_accounted_sequence","content_acknowledgement?"],"invariants":["highest_contiguous_sequence<=highest_accounted_sequence","durable_gap_advances_accounted_only","identity_matches_delivered_record"]},"content_acknowledgement":"forbidden","nack":{"schema":"hiroute.observation.nack/v1","fields":["identity","rejected_sequence","expected_sequence","retryable","detail"],"detail_tag":"kind","detail_kinds":["receiver_unavailable","unsupported_schema","invalid_envelope","missing_sequence_ranges","sequence_event_conflict","missing_prerequisite","unknown_transcript_root","missing_blob","chunk_ordinal_conflict","content_state_conflict","digest_mismatch","immutable_projection_conflict"],"free_form_reason":"forbidden"}},"content":"forbidden"}"#;
const CONVERSATION_CONTENT_CONTRACT: &str = r#"{"schema":"hiroute.gateway.port.conversation-content/v2","envelope":"hiroute.observation.conversation-content-envelope/v2","identity":["producer_id","producer_epoch","stream_id","sequence","event_id","request_id","attempt_id?","fork_id","message_instance_id?","content_id?"],"lineage":["parent_transcript_root?","result_transcript_root?","message_role?","content_kind?","content_blob_digest?","message_ordinal?","part_ordinal?","chunk_ordinal?","transport_frame_id?"],"payload":["canonical_media_type?","canonical_bytes_base64?","content_ref?","downstream_delivery?","abort_reason?","occurred_at_unix_nanos","completeness_delta?"],"content_ref_fields":["content_id","digest","byte_count","media_type"],"canonical_response_media_type":"application/vnd.hiroute.model-stream-event+json;version=1","canonical_response_content_kinds":["content_block_started","text_delta","reasoning_delta","refusal_delta","tool_call_started","tool_arguments_delta","tool_call_finished"],"forbidden_response_content_kinds":["provider_state"],"phases":["begin","append","finish","abort"],"directions":["request_input","response_delivered"],"delivery":["ack","nack","loss_watermark","producer_gap_heartbeat"],"feedback":{"identity_fields":["channel","producer_id","producer_epoch","stream_id"],"ack":{"schema":"hiroute.observation.ack/v2","fields":["identity","highest_contiguous_sequence","highest_accounted_sequence","content_acknowledgement?"],"invariants":["highest_contiguous_sequence<=highest_accounted_sequence","durable_gap_advances_accounted_only","identity_matches_delivered_record"]},"content_acknowledgement":{"channel":"conversation_content","fields":["request_id","direction","fork_id","next_chunk_ordinal","transcript_root?","delta_parent_transcript_root?","acknowledged_blobs"],"blob_fields":["content_id","digest"]},"nack":{"schema":"hiroute.observation.nack/v1","fields":["identity","rejected_sequence","expected_sequence","retryable","detail"],"detail_tag":"kind","detail_kinds":["receiver_unavailable","unsupported_schema","invalid_envelope","missing_sequence_ranges","sequence_event_conflict","missing_prerequisite","unknown_transcript_root","missing_blob","chunk_ordinal_conflict","content_state_conflict","digest_mismatch","immutable_projection_conflict"],"free_form_reason":"forbidden"}},"digest_scope":"workspace_hmac_sha256","request_boundary":"authenticated_canonical_model_ir","response_boundary":"canonical_model_event_after_full_frame_transport_accepted","rejected_attempt_content":"forbidden"}"#;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayPortContractV1 {
    pub port: String,
    pub schema_version: String,
    pub schema_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayPortSetV1 {
    pub schema_version: String,
    pub schema_digest: String,
    pub gate: String,
    pub ports: Vec<GatewayPortContractV1>,
}

pub fn gateway_port_contracts() -> GatewayPortSetV1 {
    GatewayPortSetV1 {
        schema_version: GATEWAY_PORT_SET_SCHEMA.into(),
        schema_digest: GATEWAY_PORT_SET_DIGEST.into(),
        gate: "G_port_process_22020".into(),
        ports: vec![
            descriptor(
                "RuntimePublication",
                RUNTIME_PUBLICATION_PORT_SCHEMA,
                RUNTIME_PUBLICATION_PORT_DIGEST,
            ),
            descriptor("Credential", CREDENTIAL_PORT_SCHEMA, CREDENTIAL_PORT_DIGEST),
            descriptor(
                "RuntimeState",
                RUNTIME_STATE_PORT_SCHEMA,
                RUNTIME_STATE_PORT_DIGEST,
            ),
            descriptor(
                "LifecycleFact",
                LIFECYCLE_FACT_PORT_SCHEMA,
                LIFECYCLE_FACT_PORT_DIGEST,
            ),
            descriptor(
                "ExecutionFact",
                EXECUTION_FACT_PORT_SCHEMA,
                EXECUTION_FACT_PORT_DIGEST,
            ),
            descriptor(
                "ConversationContent",
                CONVERSATION_CONTENT_PORT_SCHEMA,
                CONVERSATION_CONTENT_PORT_DIGEST,
            ),
        ],
    }
}

/// Returns the canonical compact JSON that is hashed by the published digest.
/// Demo and enterprise adapters bind to this versioned contract instead of
/// depending on a gateway implementation type.
pub fn gateway_port_contract_source(schema_version: &str) -> Option<&'static str> {
    match schema_version {
        RUNTIME_PUBLICATION_PORT_SCHEMA => Some(RUNTIME_PUBLICATION_CONTRACT),
        CREDENTIAL_PORT_SCHEMA => Some(CREDENTIAL_CONTRACT),
        RUNTIME_STATE_PORT_SCHEMA => Some(RUNTIME_STATE_CONTRACT),
        LIFECYCLE_FACT_PORT_SCHEMA => Some(LIFECYCLE_FACT_CONTRACT),
        EXECUTION_FACT_PORT_SCHEMA => Some(EXECUTION_FACT_CONTRACT),
        CONVERSATION_CONTENT_PORT_SCHEMA => Some(CONVERSATION_CONTENT_CONTRACT),
        _ => None,
    }
}

fn descriptor(port: &str, schema_version: &str, schema_digest: &str) -> GatewayPortContractV1 {
    GatewayPortContractV1 {
        port: port.into(),
        schema_version: schema_version.into(),
        schema_digest: schema_digest.into(),
    }
}

#[cfg(test)]
pub(crate) fn contract_sources() -> [(&'static str, &'static str, &'static str); 6] {
    [
        (
            RUNTIME_PUBLICATION_CONTRACT,
            RUNTIME_PUBLICATION_PORT_SCHEMA,
            RUNTIME_PUBLICATION_PORT_DIGEST,
        ),
        (
            CREDENTIAL_CONTRACT,
            CREDENTIAL_PORT_SCHEMA,
            CREDENTIAL_PORT_DIGEST,
        ),
        (
            RUNTIME_STATE_CONTRACT,
            RUNTIME_STATE_PORT_SCHEMA,
            RUNTIME_STATE_PORT_DIGEST,
        ),
        (
            LIFECYCLE_FACT_CONTRACT,
            LIFECYCLE_FACT_PORT_SCHEMA,
            LIFECYCLE_FACT_PORT_DIGEST,
        ),
        (
            EXECUTION_FACT_CONTRACT,
            EXECUTION_FACT_PORT_SCHEMA,
            EXECUTION_FACT_PORT_DIGEST,
        ),
        (
            CONVERSATION_CONTENT_CONTRACT,
            CONVERSATION_CONTENT_PORT_SCHEMA,
            CONVERSATION_CONTENT_PORT_DIGEST,
        ),
    ]
}

#[cfg(test)]
pub(crate) fn schema_digest(source: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(source.as_bytes()))
}

#[cfg(test)]
pub(crate) fn port_set_digest(ports: &[GatewayPortContractV1]) -> String {
    #[derive(Serialize)]
    struct DigestProjection<'a> {
        schema_version: &'static str,
        gate: &'static str,
        ports: &'a [GatewayPortContractV1],
    }
    let bytes = serde_json::to_vec(&DigestProjection {
        schema_version: GATEWAY_PORT_SET_SCHEMA,
        gate: "G_port_process_22020",
        ports,
    })
    .expect("port descriptors are JSON serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}
