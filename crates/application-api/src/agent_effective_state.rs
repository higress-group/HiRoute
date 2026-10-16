//! Opt-in, read-only completion guidance. No field grants authority or starts a native process.
use hiroute_domain::{AgentPlanId, ObservationRoutingContextV1, RequestOutcome};
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEffectiveStateRequestV1 {
    pub plan_id: AgentPlanId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl AgentEffectiveStateRequestV1 {
    pub fn valid(&self) -> bool {
        AgentPlanId::parse(self.plan_id.as_str()).is_ok()
            && [&self.session_id, &self.request_id]
                .into_iter()
                .all(|value| {
                    value.as_ref().is_none_or(|value| {
                        !value.is_empty() && value.len() <= 256 && !value.contains(char::is_control)
                    })
                })
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEffectiveStateV1 {
    pub schema: String,
    pub context_id: String,
    pub plan_id: AgentPlanId,
    pub preparation: AgentEffectiveFactV1,
    pub native_prerequisites: AgentEffectiveFactV1,
    pub native_loading: AgentEffectiveFactV1,
    pub current_selection: AgentEffectiveFactV1,
    pub request_evidence: AgentEffectiveFactV1,
    /// Exact recent matching request only. Does not establish every running conversation.
    pub observed_request: Option<AgentEffectiveRequestEvidenceV1>,
    /// A bounded search may not find an older matching request. Use its explicit request ID.
    pub search_incomplete: bool,
    pub next_actions: Vec<AgentNativeNextActionV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEffectiveFactV1 {
    pub state: AgentEffectiveFactStateV1,
    pub reason: String,
}
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEffectiveFactStateV1 {
    Known,
    Pending,
    Blocked,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentNativeNextActionV1 {
    pub action: String,
    pub actor: String,
    pub surface: String,
    pub effects: Vec<String>,
    pub prerequisite: String,
    pub guidance: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEffectiveRequestEvidenceV1 {
    pub session_id: String,
    pub request_id: String,
    pub receipt_id: String,
    pub started_at_ms: i64,
    pub routing_context: ObservationRoutingContextV1,
    pub served_model_id: String,
    pub actual_model: Option<String>,
    pub outcome: RequestOutcome,
    pub facts_complete: bool,
    pub publication_revision: String,
    pub publication_digest: String,
    pub grant_id: String,
    pub grant_generation: u64,
    /// Per-attempt reported usage. Missing counters and prices stay unknown.
    pub usage: Vec<AgentEffectiveUsageV1>,
    pub scope: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEffectiveUsageV1 {
    pub attempt_ordinal: u32,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}
