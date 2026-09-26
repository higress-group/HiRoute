use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use crate::server::core_runtime::profiles::{CandidateContextDemand, CandidateProtocolProfile};
use crate::server::request_plan::IngressProtocol;

use super::canonical::{canonical_bytes, canonical_digest};

pub const PLANNER_INPUT_SCHEMA: &str = "hiroute.planner-input/v1";
pub const PLANNER_OUTPUT_SCHEMA: &str = "hiroute.planner-output/v1";
pub const PLANNER_POLICY_SCHEMA: &str = "hiroute.compiled-planner-policy/v1";
pub const CANDIDATE_FACTS_SCHEMA: &str = "hiroute.planner-candidate-facts/v1";
pub const CANDIDATE_LEDGER_SCHEMA: &str = "hiroute.frozen-candidate-ledger/v1";
pub const COMPLEXITY_STRATEGY_ID: &str = "hiroute-complexity-v1";
pub const COMPLEXITY_SCHEMA: &str = "hiroute-route-strategy-v1";
pub const COMPLEXITY_THRESHOLD: u8 = 3;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityBranchV1 {
    Simple,
    Complex,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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
    pub payload_digest: String,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationKindV1 {
    ToolContinuation,
    TaskRoot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorrelatedBranchDecisionV1 {
    pub kind: ContinuationKindV1,
    pub decision: BranchDecisionV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledComplexPhraseV1 {
    pub phrase_id: String,
    pub phrase: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledComplexityStrategyV1 {
    pub strategy_id: String,
    pub schema_version: String,
    pub strategy_version: u32,
    pub threshold: u8,
    pub failure_branch: ComplexityBranchV1,
    pub classifier_kind: CompiledClassifierKindV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier_config_digest: Option<String>,
    pub complex_phrases: Vec<CompiledComplexPhraseV1>,
    pub payload_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompiledClassifierKindV1 {
    LocalRules,
    Rest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SanitizedStructuralFactsV1 {
    pub distinct_file_or_module_refs: u32,
    pub code_block_count: u32,
    pub diff_present: bool,
    pub stack_trace_or_diagnostic: bool,
    pub numbered_requirement_count: u32,
    pub normalized_non_whitespace_scalar_count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupPolicyV1 {
    Manual,
    QualityFirst,
    CheapestWithRatingGuard {
        quality_anchor_ref: String,
        max_gap_tenths: u16,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedModelGroupV1 {
    pub group_id: String,
    pub policy: GroupPolicyV1,
    pub candidate_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterializedRouteV1 {
    SmartSaving {
        simple_group_id: String,
        simple_fallback_group_ids: Vec<String>,
        complex_group_id: String,
        reselect_on_user_message: bool,
    },
    FreeFirst {
        free_group_id: String,
        exhaustion: FreeFirstExhaustionV1,
        candidate_mode: FreeCandidateModeV1,
    },
    Custom {
        group_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FreeFirstExhaustionV1 {
    FreeOnly,
    PrimaryFallback { primary_group_id: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreeCandidateModeV1 {
    AutomaticAllAvailable,
    Manual,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StaticCostPolicyV1 {
    StrictFree,
    SubscriptionAndFree,
    BudgetedPaid,
    ExplicitFixed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestOwnedLimitsV1 {
    pub max_candidate_bindings: u32,
    pub max_attempts: u32,
    pub deadline_cap_ms: u64,
    pub paid_budget_ceiling_micros: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlannerRouteIdentityV2 {
    Plan {
        plan_id: String,
        revision: u64,
    },
    Fixed {
        binding_digest: hiroute_domain::CanonicalDigest,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledPlannerPolicyV1 {
    pub schema_version: String,
    pub served_model_id: String,
    pub identity: PlannerRouteIdentityV2,
    pub route: MaterializedRouteV1,
    pub groups: Vec<MaterializedModelGroupV1>,
    pub complexity_strategy: Option<CompiledComplexityStrategyV1>,
    pub cost_policy: StaticCostPolicyV1,
    pub limits: RequestOwnedLimitsV1,
    pub policy_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClassV1 {
    Free,
    Subscription,
    Paid,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CacheCostFactV1 {
    None,
    EligibleUnconfirmed,
    Confirmed { projected_cost_micros: u64 },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PaidBudgetQuoteFactV1 {
    NotRequired,
    Available { upper_bound_micros: u64 },
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerCandidateFactsV1 {
    pub schema_version: String,
    pub candidate_id: String,
    pub stable_binding_id: String,
    pub statically_enabled: bool,
    pub protocol_profile: CandidateProtocolProfile,
    pub profile_digest: String,
    /// Exact adapter-materialized native request length. Content is not
    /// retained in planner facts or emitted into its ledger.
    pub target_serialized_bytes: u64,
    /// Request-specific projection failures are candidate facts, not a reason
    /// to reject construction of the whole Planner input. When present, the
    /// candidate is excluded before its placeholder size can reach context
    /// projection or materialization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_projection_exclusion: Option<ExclusionReasonCodeV1>,
    pub overall_score_tenths: Option<i32>,
    pub cost_class: CostClassV1,
    pub api_equivalent_cost_micros: Option<u64>,
    pub cache_cost: CacheCostFactV1,
    pub cache_affinity: bool,
    pub compute_scope_order: u32,
    pub paid_budget_quote: PaidBudgetQuoteFactV1,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerInputV1 {
    pub schema_version: String,
    pub request: ModelRequestIRV1,
    pub correlated_branch: Option<CorrelatedBranchDecisionV1>,
    pub classification_decision: Option<BranchDecisionV1>,
    pub classification_facts: Option<SanitizedStructuralFactsV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_hold: Option<HoldPreferenceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_success_candidate_id: Option<String>,
    pub policy: CompiledPlannerPolicyV1,
    pub candidates: Vec<PlannerCandidateFactsV1>,
}

/// Exact successful candidate preference supplied by the request-scoped
/// context continuity owner. It contains no credential or client identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HoldPreferenceV1 {
    pub stable_binding_id: String,
    pub candidate_id: String,
    pub profile_digest: String,
    pub reasoning_profile_id: String,
    pub origin_group_id: String,
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
pub struct CandidateEvaluationV1 {
    pub candidate_id: String,
    pub stable_binding_id: String,
    pub group_id: String,
    pub declared_order: u32,
    pub profile_digest: String,
    pub eligible: bool,
    pub first_exclusion: Option<ExclusionReasonCodeV1>,
    pub reasoning_profile_id: Option<String>,
    pub context: Option<CandidateContextDemand>,
    pub overall_score_tenths: Option<i32>,
    pub effective_cost_micros: Option<u64>,
    pub cost_class: CostClassV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenCandidateV1 {
    pub ordinal: u32,
    pub candidate_id: String,
    pub stable_binding_id: String,
    pub group_id: String,
    pub profile_digest: String,
    pub upstream_protocol: IngressProtocol,
    pub reasoning_profile_id: String,
    pub context: CandidateContextDemand,
    pub overall_score_tenths: Option<i32>,
    pub effective_cost_micros: Option<u64>,
    pub cost_class: CostClassV1,
    pub ranking_reasons: Vec<RankingReasonCodeV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenCandidateLedgerV1 {
    pub schema_version: String,
    pub ordered_candidates: Vec<FrozenCandidateV1>,
    pub evaluations: Vec<CandidateEvaluationV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GroupPlanV1 {
    pub ordinal: u32,
    pub group_id: String,
    pub ranked_candidate_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReasonLedgerEntryV1 {
    pub ordinal: u32,
    pub code: LedgerReasonCodeV1,
    pub group_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlannerOutcomeV1 {
    Ready,
    NoEligibleCandidates { code: PlannerOutcomeCodeV1 },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PlannerOutcomeCodeV1 {
    NoEligibleCandidate,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerOutputV1 {
    pub schema_version: String,
    pub planner_version: String,
    pub input_digest: String,
    pub policy_digest: String,
    pub output_digest: String,
    pub served_model_id: String,
    pub identity: PlannerRouteIdentityV2,
    pub branch: PlannedBranchV1,
    pub complexity: Option<BranchDecisionV1>,
    pub limits: RequestOwnedLimitsV1,
    pub outcome: PlannerOutcomeV1,
    pub groups: Vec<GroupPlanV1>,
    pub ledger: FrozenCandidateLedgerV1,
    pub reason_ledger: Vec<ReasonLedgerEntryV1>,
}

impl PlannerCandidateFactsV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        candidate_id: impl Into<String>,
        stable_binding_id: impl Into<String>,
        protocol_profile: CandidateProtocolProfile,
        target_serialized_bytes: u64,
        overall_score_tenths: Option<i32>,
        cost_class: CostClassV1,
        api_equivalent_cost_micros: Option<u64>,
    ) -> Result<Self, PlannerError> {
        let profile_digest = canonical_digest(&protocol_profile)?;
        Ok(Self {
            schema_version: CANDIDATE_FACTS_SCHEMA.into(),
            candidate_id: candidate_id.into(),
            stable_binding_id: stable_binding_id.into(),
            statically_enabled: true,
            protocol_profile,
            profile_digest,
            target_serialized_bytes,
            request_projection_exclusion: None,
            overall_score_tenths,
            cost_class,
            api_equivalent_cost_micros,
            cache_cost: CacheCostFactV1::None,
            cache_affinity: false,
            compute_scope_order: 0,
            paid_budget_quote: if cost_class == CostClassV1::Paid {
                PaidBudgetQuoteFactV1::Unavailable
            } else {
                PaidBudgetQuoteFactV1::NotRequired
            },
        })
    }

    pub fn recompute_profile_digest(&self) -> Result<String, PlannerError> {
        canonical_digest(&self.protocol_profile)
    }

    pub fn effective_cost_micros(&self) -> Option<u64> {
        match self.cache_cost {
            CacheCostFactV1::Confirmed {
                projected_cost_micros,
            } => Some(projected_cost_micros),
            CacheCostFactV1::None | CacheCostFactV1::EligibleUnconfirmed => {
                self.api_equivalent_cost_micros
            }
        }
    }

    pub fn affinity_penalty(&self) -> u8 {
        if self.cache_affinity
            || matches!(
                self.cache_cost,
                CacheCostFactV1::Confirmed { .. } | CacheCostFactV1::EligibleUnconfirmed
            )
        {
            0
        } else {
            1
        }
    }
}

impl CompiledPlannerPolicyV1 {
    pub fn seal(mut self) -> Result<Self, PlannerError> {
        self.schema_version = PLANNER_POLICY_SCHEMA.into();
        self.policy_digest.clear();
        self.policy_digest = canonical_digest(&self)?;
        Ok(self)
    }

    pub fn recompute_digest(&self) -> Result<String, PlannerError> {
        let mut digestless = self.clone();
        digestless.policy_digest.clear();
        canonical_digest(&digestless)
    }
}

impl PlannerOutputV1 {
    pub fn seal(mut self) -> Result<Self, PlannerError> {
        self.output_digest.clear();
        self.output_digest = canonical_digest(&self)?;
        Ok(self)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerError> {
        canonical_bytes(self)
    }

    pub fn recompute_digest(&self) -> Result<String, PlannerError> {
        let mut digestless = self.clone();
        digestless.output_digest.clear();
        canonical_digest(&digestless)
    }
}

#[derive(Debug, Error)]
pub enum PlannerError {
    #[error("planner input schema is unsupported")]
    InputSchema,
    #[error("compiled planner policy is invalid: {0}")]
    InvalidPolicy(&'static str),
    #[error("compiled planner policy digest does not match its canonical payload")]
    PolicyDigestMismatch,
    #[error("complexity strategy digest does not match its builtin rules and phrases")]
    ComplexityDigestMismatch,
    #[error("candidate profile digest does not match for `{0}`")]
    ProfileDigestMismatch(String),
    #[error("planner candidate facts are invalid: {0}")]
    InvalidCandidate(String),
    #[error("materialized group references an unknown candidate `{0}`")]
    UnknownCandidate(String),
    #[error("correlated branch decision is incompatible with the compiled strategy")]
    CorrelatedDecisionMismatch,
    #[error("planner canonical JSON failed: {0}")]
    Serialization(serde_json::Error),
    #[error("planner arithmetic overflowed")]
    ArithmeticOverflow,
}
