//! Independent main-Agent settings. An omitted facet means keep, never implicit revocation.
use crate::{AgentPlanId, SchemaVersion};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const AGENT_SETTINGS_SCHEMA_V2: SchemaVersion = SchemaVersion::new(2, 0);

#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
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
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentModelSelectionV2 {
    #[serde(rename = "opencode_additional")]
    OpenCodeAdditional {
        /// Optional native metadata only; disabling it does not disable model routing.
        /// Omission preserves existing selection bytes and the normal plugin installation.
        #[serde(
            default = "routing_metadata_enabled",
            skip_serializing_if = "is_enabled"
        )]
        routing_metadata: bool,
        allowed_plan_ids: BTreeSet<AgentPlanId>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        plan_protocols: BTreeMap<AgentPlanId, crate::AgentIngressProtocolV1>,
    },
    HermesDefault {
        plan_id: AgentPlanId,
        protocol: crate::AgentIngressProtocolV1,
    },
    DshAdditional {
        allowed_plan_ids: BTreeSet<AgentPlanId>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        plan_protocols: BTreeMap<AgentPlanId, crate::AgentIngressProtocolV1>,
    },
    CodexDefault {
        native_model_mode: CodexNativeModelModeV2,
        fixed_models: Vec<AgentFixedModelSelectionV2>,
        allowed_plan_ids: BTreeSet<AgentPlanId>,
        default_selection: AgentModelDefaultSelectionV2,
    },
    PiAdditional {
        allowed_plan_ids: BTreeSet<AgentPlanId>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        plan_protocols: BTreeMap<AgentPlanId, crate::AgentIngressProtocolV1>,
    },
    QoderAdditional {
        allowed_plan_ids: BTreeSet<AgentPlanId>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        plan_protocols: BTreeMap<AgentPlanId, crate::AgentIngressProtocolV1>,
    },
    ClaudeLauncher {
        surfaces: BTreeSet<AgentModelSurfaceV2>,
        fixed_models: Vec<AgentFixedModelSelectionV2>,
        preset_mappings: AgentClaudePresetMappingsV2,
    },
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexNativeModelModeV2 {
    HirouteOnly,
    PreserveAvailable,
}

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelSurfaceV2 {
    CodexCli,
    CodexDesktop,
    ClaudeCli,
    QoderCli,
    PiCli,
    DshCli,
    DshDesktop,
    #[serde(rename = "opencode_cli")]
    OpenCodeCli,
    HermesCli,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentFixedModelSelectionV2 {
    pub client_model_id: String,
    pub candidate: crate::CandidateSelectionV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentModelDefaultSelectionV2 {
    PreserveNative,
    FixedModel { client_model_id: String },
    Plan { plan_id: AgentPlanId },
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentClaudePresetSelectionV2 {
    PreserveNative,
    Plan { plan_id: AgentPlanId },
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

fn routing_metadata_enabled() -> bool {
    true
}
fn is_enabled(value: &bool) -> bool {
    *value
}

impl AgentModelSelectionV2 {
    pub fn installs_native_routing_metadata(&self) -> bool {
        !matches!(
            self,
            Self::OpenCodeAdditional {
                routing_metadata: false,
                ..
            }
        )
    }

    pub fn plan_protocol(
        &self,
        id: &AgentPlanId,
        fallback: crate::AgentIngressProtocolV1,
    ) -> crate::AgentIngressProtocolV1 {
        match self {
            Self::HermesDefault { protocol, .. } => *protocol,
            Self::QoderAdditional { plan_protocols, .. }
            | Self::PiAdditional { plan_protocols, .. } => {
                plan_protocols.get(id).copied().unwrap_or(fallback)
            }
            Self::DshAdditional { plan_protocols, .. }
            | Self::OpenCodeAdditional { plan_protocols, .. } => {
                plan_protocols.get(id).copied().unwrap_or(fallback)
            }
            _ => fallback,
        }
    }

    pub fn fixed_models(&self) -> &[AgentFixedModelSelectionV2] {
        match self {
            Self::CodexDefault { fixed_models, .. } | Self::ClaudeLauncher { fixed_models, .. } => {
                fixed_models
            }
            Self::QoderAdditional { .. }
            | Self::PiAdditional { .. }
            | Self::DshAdditional { .. }
            | Self::OpenCodeAdditional { .. }
            | Self::HermesDefault { .. } => &[],
        }
    }

    pub fn allowed_plan_ids(&self) -> BTreeSet<AgentPlanId> {
        match self {
            Self::HermesDefault { plan_id, .. } => [plan_id.clone()].into(),
            Self::CodexDefault {
                allowed_plan_ids, ..
            }
            | Self::QoderAdditional {
                allowed_plan_ids, ..
            }
            | Self::PiAdditional {
                allowed_plan_ids, ..
            }
            | Self::DshAdditional {
                allowed_plan_ids, ..
            }
            | Self::OpenCodeAdditional {
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
            Self::QoderAdditional { plan_protocols, .. }
            | Self::PiAdditional { plan_protocols, .. }
            | Self::DshAdditional { plan_protocols, .. }
            | Self::OpenCodeAdditional { plan_protocols, .. } => {
                if plan_protocols.keys().any(|id| !plans.contains(id))
                    || plan_protocols
                        .values()
                        .any(|p| *p == crate::AgentIngressProtocolV1::ChatCompletions)
                {
                    return Err(crate::AgentConnectionError::InvalidGrant);
                }
            }
            Self::ClaudeLauncher { surfaces, .. } => {
                if *surfaces != BTreeSet::from([AgentModelSurfaceV2::ClaudeCli]) {
                    return Err(invalid);
                }
            }
            Self::HermesDefault { .. } => {}
        }
        Ok(())
    }
}

pub fn valid_client_model_name(value: &str) -> bool {
    !value.is_empty() && !value.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationSelectionV2 {
    pub trigger_mode: AgentCollaborationTriggerModeV2,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCollaborationTriggerModeV2 {
    Explicit,
    DelegateByDefault,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettingsSpecV2 {
    pub schema_version: SchemaVersion,
    pub context_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_target: Option<crate::AgentNativeTargetV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub management_skill: Option<AgentFacetIntent<crate::AgentManagementSkillSelectionV1>>,
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
#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
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
    /// The legacy slot remains readable with its original meaning in accepted Operations.
    pub fn skill_intent(&self) -> AgentFacetIntent<()> {
        self.skill_intent_for(self.skill_facet())
    }

    pub fn skill_facet(&self) -> AgentSettingsFacet {
        if self.context_id.starts_with("agent-context/hermes/") {
            AgentSettingsFacet::ManagementSkill
        } else {
            AgentSettingsFacet::Collaboration
        }
    }

    pub fn skill_intent_for(&self, facet: AgentSettingsFacet) -> AgentFacetIntent<()> {
        match facet {
            AgentSettingsFacet::ManagementSkill => match self.management_skill.as_ref() {
                None | Some(AgentFacetIntent::Keep) => AgentFacetIntent::Keep,
                Some(AgentFacetIntent::Configure { .. }) => {
                    AgentFacetIntent::Configure { settings: () }
                }
                Some(AgentFacetIntent::Restore { restore_point_ref }) => {
                    AgentFacetIntent::Restore {
                        restore_point_ref: restore_point_ref.clone(),
                    }
                }
            },
            AgentSettingsFacet::Collaboration => match &self.collaboration {
                AgentFacetIntent::Keep => AgentFacetIntent::Keep,
                AgentFacetIntent::Configure { .. } => AgentFacetIntent::Configure { settings: () },
                AgentFacetIntent::Restore { restore_point_ref } => AgentFacetIntent::Restore {
                    restore_point_ref: restore_point_ref.clone(),
                },
            },
            AgentSettingsFacet::Model => AgentFacetIntent::Keep,
        }
    }

    pub fn keeps_skills(&self) -> bool {
        [
            AgentSettingsFacet::ManagementSkill,
            AgentSettingsFacet::Collaboration,
        ]
        .into_iter()
        .all(|facet| matches!(self.skill_intent_for(facet), AgentFacetIntent::Keep))
    }

    /// Restore commands may keep other facets, but cannot configure or grant new access.
    pub fn is_restore_only(&self) -> bool {
        matches!(self.access_token, AgentAccessTokenIntentV1::Keep)
            && !matches!(self.model, AgentFacetIntent::Configure { .. })
            && !matches!(self.collaboration, AgentFacetIntent::Configure { .. })
            && !matches!(
                self.management_skill,
                Some(AgentFacetIntent::Configure { .. })
            )
            && (matches!(self.model, AgentFacetIntent::Restore { .. })
                || matches!(self.collaboration, AgentFacetIntent::Restore { .. })
                || matches!(
                    self.management_skill,
                    Some(AgentFacetIntent::Restore { .. })
                ))
    }
}

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentSettingsFacet {
    Model,
    Collaboration,
    ManagementSkill,
}

#[cfg(test)]
mod routing_metadata_tests {
    use super::*;
    #[test]
    fn opencode_metadata_omission_preserves_previous_selection_bytes() {
        let old =
            serde_json::json!({"mode":"opencode_additional","allowed_plan_ids":["plan/example"]});
        let selection: AgentModelSelectionV2 = serde_json::from_value(old.clone()).unwrap();
        assert!(selection.installs_native_routing_metadata());
        assert_eq!(serde_json::to_value(&selection).unwrap(), old);
        let mut degraded = old;
        degraded["routing_metadata"] = serde_json::json!(false);
        let selection: AgentModelSelectionV2 = serde_json::from_value(degraded.clone()).unwrap();
        assert!(!selection.installs_native_routing_metadata());
        assert_eq!(serde_json::to_value(&selection).unwrap(), degraded);
        assert_eq!(selection.allowed_plan_ids().len(), 1);
    }
}
