//! Public model-feed control views. Retrieval and persistence belong to the daemon.
use hiroute_domain::{CanonicalDigest, PlanExecutionRef};
use serde::{Deserialize, Serialize};

pub const MODEL_FEED_DISCLOSURE: &str = "Model information from hiroute.ai, retrieved automatically without account or usage identifiers. Turn it off here.";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedSettingsRequestV1 {
    pub retrieval_enabled: Option<bool>,
    pub acknowledge_disclosure: Option<bool>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedRemoveRequestV1 {
    pub accepted_digest: CanonicalDigest,
    pub also_disable_retrieval: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedHeldV1 {
    pub sequence: u64,
    pub issued_at: String,
    pub retrieved_at: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedAttemptV1 {
    pub at: u64,
    pub result: String,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelFeedVerificationV1 {
    Validated,
    None,
    Discarded { reason: String },
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedCountsV1 {
    pub applied: u64,
    pub ignored_embedded: u64,
    pub newer_client: u64,
    pub unresolved: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedStatusV1 {
    #[serde(default)]
    pub recovery_available: bool,
    pub retrieval_enabled: bool,
    pub held: Option<ModelFeedHeldV1>,
    pub highest_accepted_sequence: Option<u64>,
    pub last_attempt: Option<ModelFeedAttemptV1>,
    pub verification: ModelFeedVerificationV1,
    pub counts: ModelFeedCountsV1,
    pub next_automatic_attempt_at: Option<u64>,
    pub disclosure: String,
    pub disclosure_acknowledged: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedAffectedSourceV1 {
    pub source_id: String,
    pub display_name: String,
    pub model_configuration_ids: Vec<String>,
    pub published_revisions: Vec<PlanExecutionRef>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedRemovalPreviewV1 {
    pub sources: Vec<ModelFeedAffectedSourceV1>,
    pub published_revisions: Vec<PlanExecutionRef>,
    pub retained_sequence: Option<u64>,
    pub retrieval_enabled: bool,
    pub digest: CanonicalDigest,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelFactOriginV1 {
    Embedded { client_version: String },
    Feed { sequence: u64, included_on: String },
    Published,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelAvailabilityV1 {
    Executable,
    NeedsConnection,
    NeedsNewerClient { version: String },
    UsedByPublishedPlans { count: usize },
}
/// Selection-time reasoning shape. Native parameter names stay internal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AvailableModelReasoningV1 {
    Toggle,
    Profiles { profiles: Vec<String> },
    Budget,
}
/// Catalog facts for presenting a model before it is connected.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AvailableModelFactsV1 {
    pub publisher_id: String,
    pub context_tokens: u64,
    pub tool: bool,
    pub vision: bool,
    pub reasoning: Option<AvailableModelReasoningV1>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AvailableModelV1 {
    pub model_configuration_id: String,
    pub display_name: String,
    pub origin: ModelFactOriginV1,
    pub identity_stability: Option<String>,
    pub availability: ModelAvailabilityV1,
    pub binding_ids: Vec<String>,
    /// Absent for models known only from published Plans.
    pub facts: Option<AvailableModelFactsV1>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFeedTextV1 {
    pub en: String,
    pub zh: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAvailabilityResultV1 {
    pub featured: Vec<ModelAnnouncementV1>,
    pub models: Vec<AvailableModelV1>,
}
