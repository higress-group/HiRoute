//! Public model-feed control views. Retrieval and persistence belong to the daemon.
use hiroute_domain::{CanonicalDigest, PlanExecutionRef};
use serde::{Deserialize, Serialize};

pub const MODEL_FEED_DISCLOSURE: &str = "Model information from hiroute.ai, retrieved automatically without account or usage identifiers. Turn it off here.";

#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedSettingsRequestV1 {
    pub retrieval_enabled: Option<bool>,
    pub acknowledge_disclosure: Option<bool>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedRemoveRequestV1 {
    pub accepted_digest: CanonicalDigest,
    pub also_disable_retrieval: bool,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedHeldV1 {
    pub sequence: u64,
    pub issued_at: String,
    pub retrieved_at: u64,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedAttemptV1 {
    pub at: u64,
    pub result: String,
    pub reason: Option<String>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelFeedVerificationV1 {
    Validated,
    None,
    Discarded { reason: String },
}
#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedCountsV2 {
    pub descriptive_records: u64,
    pub qualified_models: u64,
    pub restricted_models: u64,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedStatusV2 {
    pub selected: crate::ModelMetadataSnapshotRefV1,
    pub selected_origin: String,
    pub highest_accepted_data_revision: Option<u64>,
    pub last_success_at: Option<u64>,
    #[serde(default)]
    pub recovery_available: bool,
    pub retrieval_enabled: bool,
    pub held: Option<ModelFeedHeldV1>,
    pub highest_accepted_sequence: Option<u64>,
    pub last_attempt: Option<ModelFeedAttemptV1>,
    pub verification: ModelFeedVerificationV1,
    pub counts: ModelFeedCountsV2,
    pub next_automatic_attempt_at: Option<u64>,
    pub disclosure: String,
    pub disclosure_acknowledged: bool,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedAffectedSourceV1 {
    pub source_id: String,
    pub display_name: String,
    pub model_configuration_ids: Vec<String>,
    pub published_revisions: Vec<PlanExecutionRef>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedRemovalPreviewV2 {
    pub impact: Vec<ModelFeedDefaultImpactV2>,
    pub total: usize,
    pub next_cursor: Option<String>,
    pub complete: bool,
    pub published_plan_count: usize,
    pub sources: Vec<ModelFeedAffectedSourceV1>,
    pub published_revisions: Vec<PlanExecutionRef>,
    pub retained_sequence: Option<u64>,
    pub retrieval_enabled: bool,
    pub digest: CanonicalDigest,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelFactOriginV1 {
    Embedded { client_version: String },
    Feed { sequence: u64, included_on: String },
    Published,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelAvailabilityV2 {
    Executable,
    NeedsConnection,
    DescriptiveOnly,
    NeedsNewerClient { version: String },
    UsedByPublishedPlans { count: usize },
}
/// Selection-time reasoning shape. Native parameter names stay internal.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AvailableModelReasoningV1 {
    Toggle,
    Profiles { profiles: Vec<String> },
    Budget,
}
/// Catalog facts for presenting a model before it is connected.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AvailableModelFactsV1 {
    pub publisher_id: String,
    pub context_tokens: u64,
    pub tool: bool,
    pub vision: bool,
    pub reasoning: Option<AvailableModelReasoningV1>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AvailableModelV2 {
    pub model_configuration_id: String,
    pub display_name: String,
    pub origin: ModelFactOriginV1,
    pub identity_stability: Option<String>,
    pub availability: ModelAvailabilityV2,
    pub binding_ids: Vec<String>,
    pub published_plan_references: usize,
    /// Absent for models known only from published Plans.
    pub facts: Option<AvailableModelFactsV1>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedTextV1 {
    pub en: String,
    pub zh: String,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAnnouncementV1 {
    pub id: String,
    pub kind: String,
    pub model_configuration_id: String,
    pub headline: ModelFeedTextV1,
    pub note: ModelFeedTextV1,
    pub links: Vec<String>,
    pub included_on: String,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAvailabilityResultV2 {
    pub featured: Vec<ModelAnnouncementV1>,
    pub models: Vec<AvailableModelV2>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedRemovalQueryV2 {
    pub cursor: Option<String>,
    pub page_size: Option<u16>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedDefaultImpactV2 {
    pub source_id: String,
    pub source_name: String,
    pub binding_id: String,
    pub model_configuration_id: String,
    pub changes: std::collections::BTreeMap<String, serde_json::Value>,
    pub published_revisions: Vec<PlanExecutionRef>,
}
