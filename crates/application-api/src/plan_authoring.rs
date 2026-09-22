//! V2 content authoring uses the existing routing preview/apply envelope and protected grant.
use hiroute_domain::*;
use serde::{Deserialize, Serialize};

pub const PLAN_CONTENT_CHANGE_SCHEMA_V2: &str = "hiroute.plan-content-change/v2";
pub const PLAN_CONTENT_PREVIEW_SCHEMA_V2: &str = "hiroute.plan-content-preview/v2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanContentTargetV2 {
    Create {
        creation_key: String,
    },
    Update {
        plan_id: AgentPlanId,
        expected_head_revision: u64,
    },
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDraftRefV1 {
    pub draft_id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanContentChangeV2 {
    pub schema: String,
    pub target: PlanContentTargetV2,
    pub editor: PlanEditorStateV2,
    pub consumed_draft: Option<PlanDraftRefV1>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanContentPreviewV2 {
    pub schema: String,
    pub change_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub alias_registry_digest: CanonicalDigest,
    pub before_head: Option<PlanHeadV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_source: Option<PlanVersionV1>,
    pub plan_head: PlanHeadV1,
    pub plan_version: PlanVersionV1,
    pub consumed_draft: Option<PlanDraftRefV1>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanContentPreviewRequestV2 {
    pub change: PlanContentChangeV2,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanContentApplyRequestV2 {
    pub change: PlanContentChangeV2,
    pub accept_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub idempotency_key: String,
}

/// Ordinary publication confirms the exact editing intent and observed resource versions.
/// Authorization remains backend-held and one-shot; this does not authorize a compiled preview.
pub fn plan_content_confirmation_digest(
    change: &PlanContentChangeV2,
    revisions: &RevisionSetV1,
) -> Result<CanonicalDigest, CanonicalDigestError> {
    CanonicalDigest::of(&(PLAN_CONTENT_CHANGE_SCHEMA_V2, change, revisions))
}

pub const PLAN_LIFECYCLE_CHANGE_SCHEMA_V1: &str = "hiroute.plan-lifecycle-change/v1";
pub const PLAN_LIFECYCLE_PREVIEW_SCHEMA_V1: &str = "hiroute.plan-lifecycle-preview/v1";
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanLifecycleChangeV1 {
    pub schema: String,
    pub plan_id: AgentPlanId,
    pub expected_head_revision: u64,
    pub status: PlanLifecycleV1,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanLifecyclePreviewV1 {
    pub schema: String,
    pub change_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub before_head: PlanHeadV1,
    pub plan_head: PlanHeadV1,
    pub references_digest: CanonicalDigest,
    pub has_agent_references: bool,
    pub has_version_holds: bool,
    pub no_new_calls: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanLifecyclePreviewRequestV1 {
    pub change: PlanLifecycleChangeV1,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanLifecycleApplyRequestV1 {
    pub change: PlanLifecycleChangeV1,
    pub accept_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub idempotency_key: String,
}

pub const PLAN_DRAFT_CHANGE_SCHEMA_V1: &str = hiroute_domain::PLAN_DRAFT_CHANGE_SCHEMA_V1;
pub const PLAN_DRAFT_PREVIEW_SCHEMA_V1: &str = "hiroute.plan-draft-preview/v1";
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDraftPreviewRequestV1 {
    pub change: PlanDraftChangeV1,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDraftApplyRequestV1 {
    pub change: PlanDraftChangeV1,
    pub accept_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub idempotency_key: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDraftPreviewV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_source: Option<PlanVersionV1>,
    pub schema: String,
    pub change_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub before: Option<PlanDraftV1>,
    pub after: Option<PlanDraftV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionUnavailableReasonV1 {
    NotFree,
    NotRoutable,
    CapabilityUnqualified,
    NativeSelectionRequired,
    NativeSelectionInvalid,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FreeSuggestionV1 {
    pub selection: CandidateSelectionV1,
    pub model_configuration_id: String,
    pub exact_native_reasoning: ExactNativeReasoningV1,
    /// None means the rating service has no snapshot; it is never converted to a zero score.
    pub rating: Option<ResolvedModelRatingV1>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FreeSuggestionsV1 {
    pub snapshot_ref: Option<RatingSnapshotRefV1>,
    pub candidates: Vec<FreeSuggestionV1>,
    pub unavailable: std::collections::BTreeMap<String, SuggestionUnavailableReasonV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanCandidateOptionV1 {
    pub binding_id: String,
    pub model_configuration_id: String,
    pub display_name: String,
    pub reasoning: NativeReasoningCapabilityV1,
    pub billing_class: BillingClass,
    pub routable: bool,
    pub ingress_protocols: Vec<UpstreamProtocol>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexInputModalityV1 {
    Text,
    Image,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexReasoningControlV1 {
    RouteConfiguration,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexCandidateCapabilityLimitKindV1 {
    ContextWindow,
    ImageInput,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexCandidateCapabilityLimitV1 {
    pub kind: CodexCandidateCapabilityLimitKindV1,
    pub binding_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexFixedCapabilityLimitV1 {
    ParallelToolCallsDisabled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexCapabilityIssueKindV1 {
    PlanCompilation,
    InvalidCompiledPlan,
    ResponsesProtocol,
    RequestCapabilities,
    InstructionRoles,
    ContextInput,
    ContextOutput,
    ContextTotal,
    ReasoningProfile,
    ContextWindow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexCapabilityIssueV1 {
    pub kind: CodexCapabilityIssueKindV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodexClientCapabilityPreviewV1 {
    Available {
        context_window: u64,
        input_modalities: Vec<CodexInputModalityV1>,
        reasoning: CodexReasoningControlV1,
        limitations: Vec<CodexCandidateCapabilityLimitV1>,
        fixed_limits: Vec<CodexFixedCapabilityLimitV1>,
    },
    Unavailable {
        issues: Vec<CodexCapabilityIssueV1>,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEditorOptionsRequestV1 {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub requirements: CapabilityRequirementsV1,
    #[serde(default)]
    pub native_selections: std::collections::BTreeMap<String, ReasoningSelectionV1>,
    #[serde(default)]
    pub suggest_free: bool,
    #[serde(default)]
    pub rating_items: Vec<crate::ModelRatingQueryItemV1>,
    #[serde(default)]
    pub editor: Option<PlanEditorStateV2>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEditorOptionsV1 {
    pub suggested_alias: Option<ModelAlias>,
    pub candidates: Vec<PlanCandidateOptionV1>,
    pub free_suggestions: Option<FreeSuggestionsV1>,
    pub ratings: Option<crate::ResolveModelRatingsResultV1>,
    pub codex_capabilities: Option<CodexClientCapabilityPreviewV1>,
    pub revisions: RevisionSetV1,
}
