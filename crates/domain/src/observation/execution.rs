use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{AgentPlanId, CanonicalDigest, WorkspaceId};

use super::execution_attempt::{
    AttemptFinishedFactV1, CacheStatusV1, CredentialLeaseOutcomeV1, ExecutionRequestOutcomeV1,
    RuntimeHealthV1, RuntimeKeyScopeV1, RuntimeOperationOutcomeV1, RuntimeOperationV1,
    SemanticCommitBoundaryV1, UsageProvenanceV1, UsageSourceV1,
};
use super::{
    AttemptId, CorrelationProvenance, EventId, FactsCompleteness, FrozenValueFactsV1,
    LogicalRequestId, ObservationStreamV1, SessionId, TrafficKind, TurnId, UsageFactsV1,
};

/// Sole live Product execution-fact contract. Existing v1 rows are authenticated and read only
/// through `validate_persisted_contract`; they are never accepted by live ingestion.
pub const EXECUTION_FACT_SCHEMA_V2: &str = "hiroute.observation.product-execution-envelope/v2";
pub const EXECUTION_FACT_PORT_DIGEST_V2: &str =
    "sha256:de91d4f2333db66f0ec3f8b63ce192267dee0a56b65f34180b06996b232fb2c6";

/// Gateway profile digests are canonical except on its explicit degraded-observation path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ObservedProfileDigestV1(String);

impl ObservedProfileDigestV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, ExecutionFactError> {
        let value = value.into();
        if value == "unknown" || CanonicalDigest::parse(value.clone()).is_ok() {
            Ok(Self(value))
        } else {
            Err(ExecutionFactError::InvalidFact)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn validate(&self) -> Result<(), ExecutionFactError> {
        Self::parse(self.0.clone()).map(|_| ())
    }
}

impl From<CanonicalDigest> for ObservedProfileDigestV1 {
    fn from(value: CanonicalDigest) -> Self {
        Self(value.as_str().to_owned())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFactChannelV1 {
    ExecutionFact,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ExecutionProducerComponentV1 {
    #[serde(rename = "gateway-execution")]
    GatewayExecution,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProducerV1 {
    pub component: ExecutionProducerComponentV1,
    pub revision: String,
    pub stream: ObservationStreamV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionScopeV1 {
    Conversation,
    RequestScoped,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCorrelationV1 {
    pub workspace_id: WorkspaceId,
    pub conversation_id: SessionId,
    pub session_scope: SessionScopeV1,
    pub correlation_provenance: CorrelationProvenance,
    pub turn_id: TurnId,
    pub request_id: LogicalRequestId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorSourceV1 {
    TrustedModelAlias,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressProtocolV1 {
    Responses,
    ChatCompletions,
    Messages,
    /// Candidate-only value emitted when no materialized protocol profile was available.
    Unknown,
}

/// Request-frozen authority/publication/Plan/grant identity repeated by every Gateway fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenExecutionTrustV1 {
    pub authority_id: String,
    pub authority_epoch: u64,
    pub served_model_id: String,
    pub selector_source: SelectorSourceV1,
    pub agent_plan_id: Option<AgentPlanId>,
    pub route: crate::ModelRequestRouteV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_display_name: Option<String>,
    /// Exact decimal string carried by the Gateway envelope.
    pub gateway_publication_revision: String,
    pub gateway_publication_digest: CanonicalDigest,
    pub grant_id: String,
    pub grant_generation: u64,
    pub ingress_protocol: IngressProtocolV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLossWatermarkV1 {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub reason: ExecutionLossReasonV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLossReasonV1 {
    PublishContended,
    QueueBytesExceeded,
    QueueEventsExceeded,
    EventTooLarge,
    SinkFailed,
    SinkPanicked,
    SinkNack,
    WorkerDisconnected,
    Compacted,
}

impl ExecutionLossReasonV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PublishContended => "publish_contended",
            Self::QueueBytesExceeded => "queue_bytes_exceeded",
            Self::QueueEventsExceeded => "queue_events_exceeded",
            Self::EventTooLarge => "event_too_large",
            Self::SinkFailed => "sink_failed",
            Self::SinkPanicked => "sink_panicked",
            Self::SinkNack => "sink_nack",
            Self::WorkerDisconnected => "worker_disconnected",
            Self::Compacted => "compacted",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletenessDeltaV1 {
    Partial,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionFactEnvelopeV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<super::ExecutionPricingEvidenceV1>,
    pub schema_version: String,
    pub schema_digest: CanonicalDigest,
    pub channel: ExecutionFactChannelV1,
    pub producer: ExecutionProducerV1,
    pub sequence: u64,
    pub event_id: EventId,
    pub correlation: ExecutionCorrelationV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<AttemptId>,
    pub trust: FrozenExecutionTrustV1,
    pub occurred_at_unix_nanos: u64,
    pub fact: ExecutionFactV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss_watermark: Option<ExecutionLossWatermarkV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completeness_delta: Option<CompletenessDeltaV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannedBranchV1 {
    SmartSavingSimple,
    SmartSavingComplex,
    FreeFirstFreeOnly,
    FreeFirstPrimaryFallback,
    CustomExactOrder,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityDecisionSourceV1 {
    Inherited,
    ExternalClassifier,
    UserPhrase,
    BuiltinRules,
    Unresolved,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierFallbackReasonV1 {
    Timeout,
    Unavailable,
    RejectedInput,
    InvalidOutput,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ComplexityReasonCodeV1 {
    InheritedToolContinuation,
    InheritedTaskRoot,
    UserComplexPhrase,
    DeepReasoning,
    MultiFileScope,
    MultiConstraint,
    ImplementationAction,
    FailureOrDiffStructure,
    HumanLengthMedium,
    HumanLengthLarge,
    TaskContextUnresolved,
    TaskLanguageFallback,
    ClassifierAssessmentInvalid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BranchDecisionV1 {
    pub strategy_id: String,
    pub schema_version: String,
    pub payload_digest: CanonicalDigest,
    pub branch_id: String,
    pub complexity_score: Option<u8>,
    pub threshold: Option<u8>,
    pub decision_source: ComplexityDecisionSourceV1,
    pub reason_codes: Vec<ComplexityReasonCodeV1>,
    pub matched_user_phrase_ids: Vec<String>,
    pub fallback_used: bool,
    pub classification_duration_micros: Option<u64>,
    pub fallback_reason: Option<ClassifierFallbackReasonV1>,
}

impl BranchDecisionV1 {
    pub(crate) fn validate(&self) -> Result<(), ExecutionFactError> {
        if self.strategy_id.trim().is_empty()
            || self.schema_version.trim().is_empty()
            || CanonicalDigest::parse(self.payload_digest.as_str()).is_err()
            || self.branch_id.trim().is_empty()
            || self.branch_id.len() > 128
            || self.branch_id.chars().any(char::is_control)
            || self.complexity_score.is_some() != self.threshold.is_some()
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        let timed = self.classification_duration_micros.is_some();
        let fallback_reason = self.fallback_reason.is_some();
        let valid = match self.decision_source {
            ComplexityDecisionSourceV1::ExternalClassifier => {
                timed && self.complexity_score.is_none() && !self.fallback_used && !fallback_reason
            }
            ComplexityDecisionSourceV1::Inherited => {
                if self.complexity_score.is_none() {
                    timed && !self.fallback_used && !fallback_reason
                } else if timed {
                    self.fallback_used && fallback_reason
                } else {
                    !fallback_reason
                }
            }
            ComplexityDecisionSourceV1::UserPhrase
            | ComplexityDecisionSourceV1::BuiltinRules
            | ComplexityDecisionSourceV1::Unresolved => {
                self.complexity_score.is_some()
                    && if timed {
                        timed && self.fallback_used && fallback_reason
                    } else {
                        !timed && !fallback_reason
                    }
            }
        };
        if valid {
            Ok(())
        } else {
            Err(ExecutionFactError::InvalidFact)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GroupPlanV1 {
    pub ordinal: u32,
    pub group_id: String,
    pub ranked_candidate_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LedgerReasonCodeV1 {
    ContextHoldApplied,
    ContextHoldInvalidated,
    PreviousSuccessFallback,
    SmartSavingSimple,
    SmartSavingComplex,
    ProviderStateOwnerContinuation,
    FreeFirstNoClassification,
    CustomNoClassification,
    GroupExhaustedFallback,
    ComplexNoDowngrade,
    FreeOnlyBoundary,
    CustomExactOrder,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReasonLedgerEntryV1 {
    pub ordinal: u32,
    pub code: LedgerReasonCodeV1,
    pub group_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolChoiceRequirementV1 {
    None,
    Auto,
    RequiredAny,
    RequiredNamed { name: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestCapabilityRequirementsV1 {
    pub ingress_protocol: IngressProtocolV1,
    pub text: bool,
    pub initial_instructions: bool,
    pub mid_conversation_instructions: bool,
    pub image_url: bool,
    pub image_base64: bool,
    pub image_media_types: Vec<String>,
    pub function_tools: bool,
    pub strict_tools: bool,
    pub tool_choice: ToolChoiceRequirementV1,
    pub parallel_tools: bool,
    pub tool_roundtrip: bool,
    pub tool_result_text: bool,
    pub tool_result_json: bool,
    pub logical_tool_id_mapping: bool,
    pub streaming: bool,
    pub stream_text: bool,
    pub stream_tool_arguments: bool,
    pub stream_reasoning: bool,
    pub stream_usage: bool,
    pub provider_state: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedReasoningDispositionV1 {
    Absent,
    OverriddenByAgentPlan,
}

/// Typed carrier for the one intentionally provider-native reasoning field.
/// Stable route/candidate fields never use this representation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum NativeReasoningValueV1 {
    Null,
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    Decimal(f64),
    String(String),
    Array(Vec<NativeReasoningValueV1>),
    Object(BTreeMap<String, NativeReasoningValueV1>),
}

impl NativeReasoningValueV1 {
    pub(super) fn validate(&self) -> Result<(), ExecutionFactError> {
        match self {
            Self::Decimal(value) if !value.is_finite() => Err(ExecutionFactError::InvalidFact),
            Self::Array(values) => values.iter().try_for_each(Self::validate),
            Self::Object(values) => values.values().try_for_each(Self::validate),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteDecisionOutcomeV1 {
    Ready,
    NoEligibleCandidates,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RouteDecisionOutcomeCodeV1 {
    NoEligibleCandidate,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteDecisionFactV1 {
    pub planner_version: String,
    pub plan_id: Option<AgentPlanId>,
    pub route: crate::ModelRequestRouteV2,
    pub input_digest: CanonicalDigest,
    pub policy_digest: CanonicalDigest,
    pub output_digest: CanonicalDigest,
    pub branch: PlannedBranchV1,
    pub complexity: Option<BranchDecisionV1>,
    pub groups: Vec<GroupPlanV1>,
    pub reason_ledger: Vec<ReasonLedgerEntryV1>,
    pub requirements: RequestCapabilityRequirementsV1,
    pub requested_reasoning_disposition: RequestedReasoningDispositionV1,
    pub requested_reasoning_value: Option<NativeReasoningValueV1>,
    pub requested_max_output_tokens: Option<u64>,
    pub stream: bool,
    pub outcome: RouteDecisionOutcomeV1,
    pub outcome_code: Option<RouteDecisionOutcomeCodeV1>,
    pub max_attempts: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CacheCostFactV1 {
    None,
    EligibleUnconfirmed,
    Confirmed { projected_cost_micros: u64 },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClassV1 {
    Free,
    Subscription,
    Paid,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExclusionReasonCodeV1 {
    ProtocolPathUnavailable,
    VisionUnsupported,
    ImageSourceUnsupported,
    ToolInterfaceUnsupported,
    ToolChoiceUnsupported,
    ToolRoundtripUnsupported,
    ReasoningProfileMismatch,
    ContextLimitUnknown,
    MaxOutputUnsupported,
    StreamFeatureUnsupported,
    ProviderStateAffinityMismatch,
    OpaqueStateUnportable,
    CostPolicyExcluded,
    PaidBudgetQuoteUnavailable,
    QualityAnchorUnavailable,
    RatingUnknown,
    RatingGuardExcluded,
    StaticPlanExcluded,
    DuplicateCandidate,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RankingReasonCodeV1 {
    ContextModelHold,
    PreviousSuccessFallback,
    PublishedManualOrder,
    QualityFirst,
    LowestApiEquivalentCost,
    ConfirmedCacheHold,
    UnconfirmedAffinityTieBreak,
    StableTieBreak,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateDecisionFactV1 {
    pub candidate_id: String,
    pub stable_binding_id: String,
    pub group_id: String,
    pub declared_order: u32,
    pub profile_digest: ObservedProfileDigestV1,
    pub ingress_protocol: IngressProtocolV1,
    pub upstream_protocol: IngressProtocolV1,
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
    pub exclusion_reason: Option<ExclusionReasonCodeV1>,
    pub reasoning_profile_id: Option<String>,
    pub overall_score_tenths: Option<i32>,
    pub effective_cost_micros: Option<u64>,
    pub api_equivalent_cost_micros: Option<u64>,
    pub cost_class: CostClassV1,
    /// Gateway emits null when the candidate profile was unavailable.
    pub cache_cost: Option<CacheCostFactV1>,
    pub cache_affinity: bool,
    pub compute_scope_order: u32,
    pub ranking_reasons: Vec<RankingReasonCodeV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnAttributionV1 {
    Single,
    Mixed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnStatusV1 {
    Completed,
    Failed,
    Interrupted,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionFactV1 {
    RouteDecision(Box<RouteDecisionFactV1>),
    CandidateDecision(Box<CandidateDecisionFactV1>),
    CredentialLease {
        stable_binding_id: String,
        credential_ref: String,
        key_id: Option<String>,
        credential_generation: Option<u64>,
        excluded_key_count: u64,
        outcome: CredentialLeaseOutcomeV1,
    },
    RuntimeState {
        operation: RuntimeOperationV1,
        key_scope: RuntimeKeyScopeV1,
        stable_binding_id: String,
        credential_ref: Option<String>,
        key_id: Option<String>,
        expected_generation: Option<u64>,
        observed_generation: Option<u64>,
        health: Option<RuntimeHealthV1>,
        cooldown_remaining_millis: Option<u64>,
        probe_lease_remaining_millis: Option<u64>,
        transient_backoff_step: Option<u8>,
        outcome: RuntimeOperationOutcomeV1,
    },
    AttemptStarted {
        ordinal: u32,
        candidate_id: String,
        stable_binding_id: String,
        profile_digest: ObservedProfileDigestV1,
        credential_ref: String,
        key_id: String,
        provider_name: String,
        request_model: String,
        upstream_protocol: IngressProtocolV1,
        model_configuration_id: String,
        adapter_revision: String,
        start_reason: String,
        previous_attempt_id: Option<AttemptId>,
    },
    AttemptFinished(Box<AttemptFinishedFactV1>),
    SemanticCommit {
        ordinal: u32,
        boundary: SemanticCommitBoundaryV1,
        frame_id: String,
    },
    UsageAndCache {
        ordinal: u32,
        source: UsageSourceV1,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        billable_tokens: Option<u64>,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        input_provenance: UsageProvenanceV1,
        output_provenance: UsageProvenanceV1,
        billable_provenance: UsageProvenanceV1,
        cache_read_provenance: UsageProvenanceV1,
        cache_write_provenance: UsageProvenanceV1,
        reasoning_provenance: UsageProvenanceV1,
        effective_cost_micros: Option<u64>,
        cost_class: Option<CostClassV1>,
        cache_status: CacheStatusV1,
    },
    RequestFinished {
        outcome: ExecutionRequestOutcomeV1,
        attempts_started: u32,
        attempts_finished: u32,
        accepted_attempt_ordinal: Option<u32>,
        facts_completeness: FactsCompleteness,
    },
    AgentTurnFinished {
        agent_turn_id: String,
        segment_id: String,
        ordinal: u64,
        plan_id: AgentPlanId,
        plan_revision: u64,
        selected_branch_id: String,
        executed_branch_id: Option<String>,
        model_configuration_id: Option<String>,
        profile_digest: Option<ObservedProfileDigestV1>,
        attribution: AgentTurnAttributionV1,
        started_at_ms: u64,
        finished_at_ms: u64,
        status: AgentTurnStatusV1,
        history_partial: bool,
        first_request_id: Option<LogicalRequestId>,
        last_request_id: Option<LogicalRequestId>,
    },
    BranchAssessmentRecorded {
        segment_id: String,
        plan_id: AgentPlanId,
        plan_revision: u64,
        model_configuration_id: String,
        profile_digest: ObservedProfileDigestV1,
        trigger_request_id: LogicalRequestId,
        target_from_turn_id: TurnId,
        target_through_turn_id: TurnId,
        target_from_ordinal: u64,
        target_through_ordinal: u64,
        assessed_at_ms: u64,
        score: f64,
        partial: bool,
        reason: Option<String>,
    },
    /// Product-owned immutable value input. Gateway mapping never aggregates into this event.
    ValueSnapshot {
        traffic_kind: TrafficKind,
        usage: UsageFactsV1,
        value: FrozenValueFactsV1,
    },
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ExecutionFactError {
    #[error("execution fact schema or contract digest is unsupported")]
    UnsupportedSchema,
    #[error("execution fact envelope is invalid")]
    InvalidEnvelope,
    #[error("execution fact trust identity is invalid")]
    InvalidTrustIdentity,
    #[error("execution fact attempt scope is invalid")]
    InvalidAttemptScope,
    #[error("execution fact loss watermark is invalid")]
    InvalidLossWatermark,
    #[error("execution fact payload is invalid")]
    InvalidFact,
}
