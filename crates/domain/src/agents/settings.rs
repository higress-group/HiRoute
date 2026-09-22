//! Independent main-Agent settings. An omitted facet means keep, never implicit revocation.
use crate::{AgentPlanId, SchemaVersion};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const AGENT_SETTINGS_SCHEMA_V2: SchemaVersion = SchemaVersion::new(2, 0);

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentFacetIntent<T> {
    #[default]
    Keep,
    Configure {
        settings: T,
    },
    Restore {
        restore_point_ref: String,
    },
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentModelSelectionV2 {
    CodexDefault {
        native_model_mode: CodexNativeModelModeV2,
        fixed_models: Vec<AgentFixedModelSelectionV2>,
        allowed_plan_ids: BTreeSet<AgentPlanId>,
        default_selection: AgentModelDefaultSelectionV2,
    },
    ClaudeLauncher {
        surfaces: BTreeSet<AgentModelSurfaceV2>,
        fixed_models: Vec<AgentFixedModelSelectionV2>,
        preset_mappings: AgentClaudePresetMappingsV2,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexNativeModelModeV2 {
    HirouteOnly,
    PreserveAvailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelSurfaceV2 {
    CodexCli,
    CodexDesktop,
    ClaudeCli,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentFixedModelSelectionV2 {
    pub client_model_id: String,
    pub candidate: crate::CandidateSelectionV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentModelDefaultSelectionV2 {
    PreserveNative,
    FixedModel { client_model_id: String },
    Plan { plan_id: AgentPlanId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentClaudePresetSelectionV2 {
    PreserveNative,
    Plan { plan_id: AgentPlanId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentClaudePresetMappingsV2 {
    pub opus: AgentClaudePresetSelectionV2,
    pub sonnet: AgentClaudePresetSelectionV2,
    pub haiku: AgentClaudePresetSelectionV2,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentClaudePresetValuesV2 {
    pub opus: Option<String>,
    pub sonnet: Option<String>,
    pub haiku: Option<String>,
}

impl AgentModelSelectionV2 {
    pub fn fixed_models(&self) -> &[AgentFixedModelSelectionV2] {
        match self {
            Self::CodexDefault { fixed_models, .. } | Self::ClaudeLauncher { fixed_models, .. } => {
                fixed_models
            }
        }
    }

    pub fn allowed_plan_ids(&self) -> BTreeSet<AgentPlanId> {
        match self {
            Self::CodexDefault {
                allowed_plan_ids, ..
            } => allowed_plan_ids.clone(),
            Self::ClaudeLauncher {
                preset_mappings, ..
            } => [
                &preset_mappings.opus,
                &preset_mappings.sonnet,
                &preset_mappings.haiku,
            ]
            .into_iter()
            .filter_map(|selection| match selection {
                AgentClaudePresetSelectionV2::PreserveNative => None,
                AgentClaudePresetSelectionV2::Plan { plan_id } => Some(plan_id.clone()),
            })
            .collect(),
        }
    }

    pub fn validate(&self) -> Result<(), crate::AgentConnectionError> {
        let invalid = crate::AgentConnectionError::InvalidGrant;
        let mut names = BTreeSet::new();
        for fixed in self.fixed_models() {
            if !valid_client_model_name(&fixed.client_model_id)
                || !names.insert(fixed.client_model_id.as_str())
                || !crate::routing::valid_reference(&fixed.candidate.binding_id)
            {
                return Err(invalid);
            }
        }
        let plans = self.allowed_plan_ids();
        if names.is_empty() && plans.is_empty() {
            return Err(invalid);
        }
        match self {
            Self::CodexDefault {
                native_model_mode,
                default_selection,
                ..
            } => match default_selection {
                AgentModelDefaultSelectionV2::PreserveNative
                    if *native_model_mode == CodexNativeModelModeV2::HirouteOnly =>
                {
                    return Err(invalid);
                }
                AgentModelDefaultSelectionV2::PreserveNative => {}
                AgentModelDefaultSelectionV2::FixedModel { client_model_id }
                    if names.contains(client_model_id.as_str()) => {}
                AgentModelDefaultSelectionV2::Plan { plan_id } if plans.contains(plan_id) => {}
                _ => return Err(invalid),
            },
            Self::ClaudeLauncher { surfaces, .. } => {
                if *surfaces != BTreeSet::from([AgentModelSurfaceV2::ClaudeCli]) {
                    return Err(invalid);
                }
            }
        }
        Ok(())
    }
}

pub fn valid_client_model_name(value: &str) -> bool {
    !value.is_empty() && !value.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationSelectionV2 {
    pub trigger_mode: AgentCollaborationTriggerModeV2,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCollaborationTriggerModeV2 {
    Explicit,
    DelegateByDefault,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettingsSpecV2 {
    pub schema_version: SchemaVersion,
    pub context_id: String,
    #[serde(default)]
    pub model: AgentFacetIntent<AgentModelSelectionV2>,
    #[serde(default)]
    pub collaboration: AgentFacetIntent<AgentCollaborationSelectionV2>,
    /// Explicit native Codex model to select while restoring a managed connection. Present only
    /// for a Codex model restore; the trusted adapter checks it against the original catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_native_model: Option<String>,
    /// Set by trusted Preview, then sealed in the successful Operation. This records the
    /// original names whose same-account routes must survive later full-state edits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_native_model_ids: Vec<String>,
    #[serde(default)]
    pub access_token: AgentAccessTokenIntentV1,
}

/// Token changes belong to the model connection. A custom value is supplied separately through
/// the protected local input channel; only its one-use slot enters the settings journal.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentAccessTokenIntentV1 {
    #[default]
    Keep,
    Regenerate,
    Set {
        input_slot: String,
    },
}

impl AgentSettingsSpecV2 {
    /// Restore commands may keep other facets, but cannot configure or grant new access.
    pub fn is_restore_only(&self) -> bool {
        matches!(self.access_token, AgentAccessTokenIntentV1::Keep)
            && !matches!(self.model, AgentFacetIntent::Configure { .. })
            && !matches!(self.collaboration, AgentFacetIntent::Configure { .. })
            && (matches!(self.model, AgentFacetIntent::Restore { .. })
                || matches!(self.collaboration, AgentFacetIntent::Restore { .. }))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSettingsFacet {
    Model,
    Collaboration,
}
