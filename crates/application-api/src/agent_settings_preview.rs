//! Non-secret settings Preview projections; planning stays in Application.
use crate::*;
use hiroute_domain::AgentModelGrantV2;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
pub struct AgentSettingsPreview {
    pub context_windows: BTreeMap<String, u64>,
    pub claude_context_window: Option<u64>,
    pub spec: AgentSettingsSpecV2,
    pub dependency_digest: CanonicalDigest,
    pub accept_digest: CanonicalDigest,
    pub changed_facets: BTreeSet<AgentSettingsFacet>,
    pub model_grant: Option<AgentModelGrantV2>,
    /// None means keep/restore; Configure always persists the selected Skill trigger mode.
    pub collaboration_trigger_mode: Option<AgentCollaborationTriggerModeV2>,
    pub blockers: Vec<AgentSettingsBlock>,
    pub unproven_native_model_ids: Vec<String>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSettingsBlock {
    pub facet: AgentSettingsFacet,
    pub reason: SettingsBlockReason,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<hiroute_domain::CapabilityBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_ids: Vec<String>,
}
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsBlockReason {
    QoderDefaultInUse,
    AdditionalDefaultInUse,
    /// Restore the owned metadata resource before applying the explicit OpenCode fallback.
    NativeRoutingAdapterRestoreRequired,
    /// A borrowed/native resource cannot be removed by this settings operation.
    NativeRoutingAdapterExternalResource,
    QoderModelFileConflict,
    HermesModelFileConflict,
    CodexContextOverride,
    ClaudeContextOverride,
    ClaudePlanCapabilityUnavailable,
    ClaudeContextWindowUnsupported,
    CapabilityUnavailable,
    ModelPlanUnavailable,
    NativeModelCoverageUnavailable,
    NativeDefaultInvalid,
    RestoreNativeModelInvalid,
    RestorePointUnavailable,
    SkillFileConflict,
}

#[derive(schemars::JsonSchema, Serialize, Deserialize)]
pub struct AgentSettingsPreviewV2 {
    #[serde(flatten)]
    pub preview: AgentSettingsPreview,
    pub schema: String,
    pub expected_revisions: RevisionSetV1,
    pub applicable: bool,
    pub resident_service: AgentResidentServicePreviewV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collaboration_effect: Option<AgentCollaborationEffectV2>,
    pub model_effect: AgentModelEffectPreviewV2,
}
#[derive(schemars::JsonSchema, Serialize, Deserialize)]
pub struct AgentResidentServicePreviewV2 {
    pub login_item_required: bool,
    pub login_item_removal_required: bool,
}
#[derive(schemars::JsonSchema, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentModelEffectPreviewV2 {
    Keep,
    Restore {
        restore_point_ref: String,
    },
    Configure {
        agent_class: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_id: Option<String>,
        endpoint: String,
        selection: Box<AgentModelSelectionV2>,
        authentication: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        native_adapter: Option<AgentNativeAdapterPreviewV1>,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_windows: Option<BTreeMap<String, u64>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        claude_context_window: Option<Option<u64>>,
    },
}

#[derive(schemars::JsonSchema, Serialize, Deserialize)]
pub struct AgentNativeAdapterPreviewV1 {
    pub resources: String,
    pub runtime_loading: AgentNativeRuntimeLoadingV1,
    pub next_action: AgentNativeAdapterNextActionV1,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_optional_block_details_round_trip_as_empty() {
        for value in [
            serde_json::json!({"facet":"management_skill", "reason":"capability_unavailable", "capabilities":[{"capability":"skill_loading", "reason":"unknown"}]}),
            serde_json::json!({"facet":"model", "reason":"native_default_invalid", "model_ids":["unavailable-model"]}),
        ] {
            let block: AgentSettingsBlock = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(block).unwrap(), value);
        }
    }
}
