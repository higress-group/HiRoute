//! Common planning inputs and the native facts owned by each ecosystem.
use super::*;

pub struct SettingsModelFacts {
    pub common: SettingsModelCommonFacts,
    pub native: SettingsModelNativeFacts,
}

pub struct SettingsModelCommonFacts {
    pub ingress: AgentIngressProtocolV1,
    pub available_surfaces: BTreeSet<hiroute_domain::AgentModelSurfaceV2>,
    pub model_publication: Option<GatewayPublicationV1>,
    pub login_item_required: bool,
    pub login_item_removal_required: bool,
    pub fixed_candidate_facts: Vec<crate::compiler::CandidateCompilationFactV1>,
}

pub enum SettingsModelNativeFacts {
    Codex(Box<SettingsCodexModelFacts>),
    Claude(SettingsClaudeNativeFacts),
    Additional(SettingsAdditionalModelFacts),
}

#[derive(Default)]
pub struct SettingsCodexModelFacts {
    pub context_override: bool,
    pub model_catalog: Option<SettingsModelCatalogFacts>,
    pub preserved_codex_models: Vec<hiroute_domain::AgentFixedModelSelectionV2>,
    pub preserved_codex_bindings: BTreeMap<String, hiroute_domain::AttemptOwnedCandidateV1>,
    pub required_native_model_ids: Option<Vec<String>>,
    pub unproven_native_model_ids: Vec<String>,
    pub require_native_model_routes: bool,
    pub native_default_must_be_original: bool,
    pub native_default_model: Option<String>,
    pub restore_native_model_ids: Option<Vec<String>>,
    pub restore_inherits_root: bool,
    pub restored_native_model: Option<String>,
}

#[derive(Default)]
pub struct SettingsClaudeNativeFacts {
    pub context_override: bool,
    pub plan_capability_unavailable: bool,
    pub native_default_model: Option<String>,
    pub presets: Option<hiroute_domain::AgentClaudePresetValuesV2>,
}

pub struct SettingsAdditionalModelFacts {
    pub model_conflict: Option<SettingsBlockReason>,
}

impl SettingsModelFacts {
    pub fn codex(&self) -> Option<&SettingsCodexModelFacts> {
        match &self.native {
            SettingsModelNativeFacts::Codex(facts) => Some(facts),
            _ => None,
        }
    }

    pub fn claude(&self) -> Option<&SettingsClaudeNativeFacts> {
        match &self.native {
            SettingsModelNativeFacts::Claude(facts) => Some(facts),
            _ => None,
        }
    }
}
