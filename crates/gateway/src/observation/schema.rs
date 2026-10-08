use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::server::core_runtime::model_ir::RequestCapabilityRequirementsV1;

pub const LIFECYCLE_FACT_SCHEMA: &str = "hiroute.gateway.lifecycle-fact-envelope/v2";
/// The sole production execution-fact envelope. Pricing is optional data on AttemptStarted;
/// its presence is state, not a second wire version.
pub const EXECUTION_FACT_SCHEMA: &str = "hiroute.observation.execution-fact-envelope/v3";
pub const CONVERSATION_CONTENT_SCHEMA: &str =
    "hiroute.observation.conversation-content-envelope/v2";
pub const OTEL_GEN_AI_SCHEMA: &str = "hiroute.otel.gen-ai-mapping/v1";
pub const OBSERVATION_ACK_SCHEMA: &str = "hiroute.observation.ack/v2";
pub const OBSERVATION_NACK_SCHEMA: &str = "hiroute.observation.nack/v1";
pub const OBSERVATION_GAP_HEARTBEAT_SCHEMA: &str = "hiroute.observation.gap-heartbeat/v1";
pub const CONVERSATION_CONTENT_CHANNEL: &str = "conversation_content";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerDescriptorV1 {
    pub component: String,
    pub revision: String,
    pub producer_id: String,
    pub producer_epoch: String,
    pub stream_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LossWatermarkV1 {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationGapHeartbeatV1 {
    pub schema_version: String,
    pub channel: String,
    pub producer: ProducerDescriptorV1,
    pub sequence: u64,
    pub event_id: String,
    pub loss_watermark: LossWatermarkV1,
    pub completeness_delta: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorrelationV1 {
    pub workspace_id: String,
    pub conversation_id: String,
    pub session_scope: String,
    pub correlation_provenance: String,
    pub turn_id: String,
    pub request_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleFactEnvelopeV1 {
    pub schema_version: String,
    pub schema_digest: String,
    pub channel: String,
    pub producer: ProducerDescriptorV1,
    pub sequence: u64,
    pub event_id: String,
    pub correlation: CorrelationV1,
    pub occurred_at_unix_nanos: u64,
    pub fact: LifecycleFactV1,
    pub loss_watermark: Option<LossWatermarkV1>,
    pub completeness_delta: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleFactV1 {
    RequestAccepted {
        ingress_protocol: String,
    },
    CanonicalRequestAccepted {
        canonicalization_version: String,
    },
    AttemptStarted {
        ordinal: u32,
    },
    AttemptFinished {
        ordinal: u32,
        outcome: String,
    },
    ResponseFrameAccepted {
        frame_id: String,
        byte_count: u64,
        downstream_delivery: String,
    },
    RequestFinished {
        outcome: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionFactEnvelopeV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<hiroute_domain::ExecutionPricingEvidenceV1>,
    pub schema_version: String,
    pub schema_digest: String,
    pub channel: String,
    pub producer: ProducerDescriptorV1,
    pub sequence: u64,
    pub event_id: String,
    pub correlation: CorrelationV1,
    pub attempt_id: Option<String>,
    pub authority_id: String,
    pub authority_epoch: u64,
    pub served_model_id: String,
    pub selector_source: String,
    pub agent_plan_id: Option<String>,
    pub route: hiroute_domain::ModelRequestRouteV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_display_name: Option<String>,
    pub gateway_publication_revision: String,
    pub gateway_publication_digest: String,
    pub grant_id: String,
    pub grant_generation: u64,
    pub ingress_protocol: String,
    pub occurred_at_unix_nanos: u64,
    pub fact: ExecutionFactV1,
    pub loss_watermark: Option<LossWatermarkV1>,
    pub completeness_delta: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)] // Stable wire fact enum; keep its typed payloads inline.
pub enum ExecutionFactV1 {
    RouteDecision {
        planner_version: String,
        plan_id: Option<String>,
        route: hiroute_domain::ModelRequestRouteV2,
        input_digest: String,
        policy_digest: String,
        output_digest: String,
        branch: Value,
        complexity: Option<Value>,
        groups: Value,
        reason_ledger: Value,
        requirements: RequestCapabilityRequirementsV1,
        requested_reasoning_disposition: String,
        requested_reasoning_value: Option<Value>,
        requested_max_output_tokens: Option<u64>,
        stream: bool,
        outcome: String,
        outcome_code: Option<String>,
        max_attempts: u32,
    },
    CandidateDecision(Box<CandidateDecisionFactV1>),
    CredentialLease {
        stable_binding_id: String,
        credential_ref: String,
        key_id: Option<String>,
        credential_generation: Option<u64>,
        excluded_key_count: usize,
        outcome: String,
    },
    RuntimeState {
        operation: String,
        key_scope: String,
        stable_binding_id: String,
        credential_ref: Option<String>,
        key_id: Option<String>,
        expected_generation: Option<u64>,
        observed_generation: Option<u64>,
        health: Option<String>,
        cooldown_remaining_millis: Option<u64>,
        probe_lease_remaining_millis: Option<u64>,
        transient_backoff_step: Option<u8>,
        outcome: String,
    },
    AttemptStarted {
        ordinal: u32,
        candidate_id: String,
        stable_binding_id: String,
        profile_digest: String,
        credential_ref: String,
        key_id: String,
        provider_name: String,
        request_model: String,
        upstream_protocol: String,
        model_configuration_id: String,
        adapter_revision: String,
        start_reason: String,
        previous_attempt_id: Option<String>,
    },
    AttemptFinished(Box<AttemptFinishedFactV1>),
    SemanticCommit {
        ordinal: u32,
        boundary: String,
        frame_id: String,
    },
    UsageAndCache {
        ordinal: u32,
        source: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        billable_tokens: Option<u64>,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        input_provenance: String,
        output_provenance: String,
        billable_provenance: String,
        cache_read_provenance: String,
        cache_write_provenance: String,
        reasoning_provenance: String,
        effective_cost_micros: Option<u64>,
        cost_class: Option<String>,
        cache_status: String,
    },
    RequestFinished {
        outcome: String,
        attempts_started: u32,
        attempts_finished: u32,
        accepted_attempt_ordinal: Option<u32>,
        facts_completeness: String,
    },
    AgentTurnFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch_execution: Option<hiroute_domain::BranchExecutionV1>,
        agent_turn_id: String,
        segment_id: String,
        ordinal: u64,
        plan_id: String,
        plan_revision: u64,
        selected_branch_id: String,
        executed_branch_id: Option<String>,
        model_configuration_id: Option<String>,
        profile_digest: Option<String>,
        attribution: String,
        started_at_ms: u64,
        finished_at_ms: u64,
        status: String,
        history_partial: bool,
        first_request_id: Option<String>,
        last_request_id: Option<String>,
    },
    BranchAssessmentRecorded {
        segment_id: String,
        plan_id: String,
        plan_revision: u64,
        model_configuration_id: String,
        profile_digest: String,
        trigger_request_id: String,
        target_from_turn_id: String,
        target_through_turn_id: String,
        target_from_ordinal: u64,
        target_through_ordinal: u64,
        assessed_at_ms: u64,
        score: f64,
        partial: bool,
        reason: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptFinishedFactV1 {
    pub ordinal: u32,
    pub stable_binding_id: String,
    pub outcome: String,
    pub error_class: Option<String>,
    pub retryable: Option<bool>,
    pub duration_micros: u64,
    pub disposition: String,
    pub provider_http_status: Option<u16>,
    pub provider_code: Option<String>,
    pub provider_request_id: Option<String>,
    pub retry_after_millis: Option<u64>,
    pub reset_after_millis: Option<u64>,
    pub provider_readiness: Option<String>,
    pub provider_model_event: Option<String>,
    pub time_to_first_model_event_micros: Option<u64>,
    pub provider_ended_micros_from_start: Option<u64>,
    pub transport: AttemptTransportFactV1,
    pub commits: AttemptCommitFactV1,
    pub stream_outcome: String,
    pub downstream_outcome: String,
    pub cleanup_outcome: String,
    pub termination_reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptTransportFactV1 {
    pub connect_micros: Option<u64>,
    pub request_write_micros: Option<u64>,
    pub upstream_ttfb_micros: Option<u64>,
    pub last_upstream_progress_micros_from_start: Option<u64>,
    pub local_read_suppressed_micros: u64,
    pub upstream_body_bytes: u64,
    pub timeout_kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptCommitFactV1 {
    pub upstream_request: String,
    pub downstream_headers: String,
    pub downstream_semantic: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateDecisionFactV1 {
    pub candidate_id: String,
    pub stable_binding_id: String,
    pub group_id: String,
    pub declared_order: u32,
    pub profile_digest: String,
    pub ingress_protocol: String,
    pub upstream_protocol: String,
    pub path_id: String,
    pub provider_id: String,
    pub endpoint_id: String,
    pub entitlement_id: String,
    pub connector_id: String,
    pub connector_revision: String,
    pub capability_id: String,
    pub capability_revision: String,
    pub model_configuration_id: String,
    pub native_model: String,
    pub adapter_revision: String,
    pub serializer_revision: String,
    pub decoder_revision: String,
    pub target_serialized_bytes: u64,
    pub eligible: bool,
    pub exclusion_reason: Option<String>,
    pub reasoning_profile_id: Option<String>,
    pub overall_score_tenths: Option<i32>,
    pub effective_cost_micros: Option<u64>,
    pub api_equivalent_cost_micros: Option<u64>,
    pub cost_class: String,
    pub cache_cost: Value,
    pub cache_affinity: bool,
    pub compute_scope_order: u32,
    pub ranking_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationContentEnvelopeV1 {
    pub schema_version: String,
    pub schema_digest: String,
    pub channel: String,
    pub producer: ProducerDescriptorV1,
    pub sequence: u64,
    pub event_id: String,
    pub correlation: CorrelationV1,
    pub direction: String,
    pub phase: String,
    pub attempt_id: Option<String>,
    pub fork_id: String,
    pub parent_transcript_root: Option<String>,
    pub result_transcript_root: Option<String>,
    pub message_instance_id: Option<String>,
    pub message_role: Option<String>,
    pub content_kind: Option<String>,
    pub content_id: Option<String>,
    pub content_blob_digest: Option<String>,
    pub message_ordinal: Option<u32>,
    pub part_ordinal: Option<u32>,
    pub chunk_ordinal: Option<u32>,
    pub transport_frame_id: Option<String>,
    pub canonical_media_type: Option<String>,
    pub canonical_bytes_base64: Option<String>,
    pub content_ref: Option<ContentRefV1>,
    pub downstream_delivery: Option<String>,
    pub abort_reason: Option<String>,
    pub occurred_at_unix_nanos: u64,
    pub loss_watermark: Option<LossWatermarkV1>,
    pub completeness_delta: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentRefV1 {
    pub content_id: String,
    pub digest: String,
    pub byte_count: u64,
    pub media_type: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OtelGenAiRecordV1 {
    pub schema_version: String,
    pub schema_digest: String,
    pub mapper_version: String,
    pub semantic_conventions_version: String,
    pub production_exporter: String,
    pub producer: ProducerDescriptorV1,
    pub sequence: u64,
    pub event_id: String,
    pub correlation: CorrelationV1,
    pub signal: OtelSignalV1,
    pub loss_watermark: Option<LossWatermarkV1>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OtelSignalV1 {
    Span {
        name: String,
        span_kind: String,
        status: String,
        attributes: Vec<OtelAttributeV1>,
        events: Vec<OtelEventV1>,
    },
    ExtensionEvent {
        name: String,
        attributes: Vec<OtelAttributeV1>,
        content_ref: ContentRefV1,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OtelEventV1 {
    pub name: String,
    pub attributes: Vec<OtelAttributeV1>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OtelAttributeV1 {
    pub key: String,
    pub value: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFeedbackIdentityV1 {
    pub channel: String,
    pub producer_id: String,
    pub producer_epoch: String,
    pub stream_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationContentDirectionV1 {
    RequestInput,
    ResponseDelivered,
}

impl ObservationContentDirectionV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequestInput => "request_input",
            Self::ResponseDelivered => "response_delivered",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationContentPhaseV1 {
    Begin,
    Append,
    Finish,
    Abort,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationBlobAcknowledgementV1 {
    pub content_id: String,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationContentAcknowledgementV1 {
    pub request_id: String,
    pub direction: ObservationContentDirectionV1,
    pub fork_id: String,
    pub next_chunk_ordinal: u32,
    pub transcript_root: Option<String>,
    pub delta_parent_transcript_root: Option<String>,
    pub acknowledged_blobs: Vec<ObservationBlobAcknowledgementV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationAckV2 {
    pub schema_version: String,
    pub identity: ObservationFeedbackIdentityV1,
    pub highest_contiguous_sequence: u64,
    pub highest_accounted_sequence: u64,
    pub content_acknowledgement: Option<ObservationContentAcknowledgementV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationSequenceRangeV1 {
    pub first_sequence: u64,
    pub last_sequence: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationEnvelopeViolationV1 {
    Malformed,
    MissingField,
    UnknownField,
    InvalidFieldValue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationDigestSubjectV1 {
    EnvelopeSchema,
    Event,
    Transcript,
    ContentBlob,
    Projection,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservationNackDetailV1 {
    ReceiverUnavailable {
        retry_after_millis: Option<u64>,
    },
    UnsupportedSchema {
        rejected_schema_version: String,
        supported_schema_versions: Vec<String>,
    },
    InvalidEnvelope {
        violation: ObservationEnvelopeViolationV1,
        field: Option<String>,
    },
    MissingSequenceRanges {
        ranges: Vec<ObservationSequenceRangeV1>,
    },
    SequenceEventConflict {
        sequence: u64,
        expected_event_id: String,
        rejected_event_id: String,
    },
    MissingPrerequisite {
        prerequisite_sequence: u64,
        prerequisite_event_id: Option<String>,
    },
    UnknownTranscriptRoot {
        request_id: String,
        direction: ObservationContentDirectionV1,
        fork_id: String,
        transcript_root: String,
    },
    MissingBlob {
        request_id: String,
        direction: ObservationContentDirectionV1,
        fork_id: String,
        blobs: Vec<ObservationBlobAcknowledgementV1>,
    },
    ChunkOrdinalConflict {
        request_id: String,
        direction: ObservationContentDirectionV1,
        fork_id: String,
        expected_chunk_ordinal: u32,
        rejected_chunk_ordinal: u32,
    },
    ContentStateConflict {
        request_id: String,
        direction: ObservationContentDirectionV1,
        fork_id: String,
        expected_phase: ObservationContentPhaseV1,
        rejected_phase: ObservationContentPhaseV1,
    },
    DigestMismatch {
        subject: ObservationDigestSubjectV1,
        subject_id: String,
        expected_digest: String,
        rejected_digest: String,
    },
    ImmutableProjectionConflict {
        projection_key: String,
        existing_digest: String,
        rejected_digest: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationNackV1 {
    pub schema_version: String,
    pub identity: ObservationFeedbackIdentityV1,
    pub rejected_sequence: u64,
    pub expected_sequence: u64,
    pub retryable: bool,
    pub detail: ObservationNackDetailV1,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ObservationFeedbackValidationError {
    #[error("unsupported observation feedback schema")]
    UnsupportedSchema,
    #[error("observation feedback field `{0}` must not be empty")]
    EmptyField(&'static str),
    #[error("highest_contiguous_sequence exceeds highest_accounted_sequence")]
    InvalidFrontier,
    #[error("content feedback is only valid for the conversation_content channel")]
    InvalidContentChannel,
    #[error("observation feedback sequence must be non-zero")]
    InvalidSequence,
    #[error("observation sequence range is empty or reversed")]
    InvalidSequenceRange,
    #[error("observation repair detail must not be empty")]
    EmptyRepairDetail,
    #[error("observation blob acknowledgement is duplicated")]
    DuplicateBlobAcknowledgement,
}
