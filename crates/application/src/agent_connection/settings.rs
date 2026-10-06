//! Facet-independent, read-only settings planning. The daemon supplies current native/Plan/20 facts.
use hiroute_application_api::{
    AGENT_SETTINGS_SCHEMA_V2, AgentFacetIntent, AgentSettingsFacet, AgentSettingsSpecV2,
};
use hiroute_domain::{
    AgentAccessTokenIntentV1, AgentAction, AgentCapability, AgentCapabilitySet,
    AgentCollaborationTriggerModeV2, AgentIngressProtocolV1, AgentModelDefaultSelectionV2,
    AgentModelGrantV2, AgentModelRouteV2, AgentModelSelectionV2, CanonicalDigest,
    GatewayPublicationV1,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub struct AgentSettingsFacts {
    pub context_id: String,
    pub dependency_digest: CanonicalDigest,
    pub capabilities: AgentCapabilitySet,
    /// No model facts or configuration target are needed by collaboration-only installations.
    pub model: Option<SettingsModelFacts>,
    pub collaboration_file_conflict: bool,
    pub restore_points: BTreeMap<String, AgentSettingsFacet>,
}

mod model_facts;
pub use model_facts::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SettingsModelCatalogFacts {
    pub source_revision: String,
    /// Digest of the private merged artifact written by HiRoute.
    pub content_digest: CanonicalDigest,
    /// The exact content-addressed artifact may already exist from an earlier save. Bind its
    /// observed fingerprint to Preview/Apply instead of assuming every save creates a new path.
    pub before_fingerprint: Option<CanonicalDigest>,
    pub producer_kind: CodexCatalogProducerKindV1,
    pub producer_path: String,
    pub producer_content_digest: CanonicalDigest,
    pub producer_context_digest: CanonicalDigest,
    pub producer_dependency_digest: CanonicalDigest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexCatalogProducerKindV1 {
    UserConfigured,
    TargetCache,
    TargetBundled,
    HirouteGenerated,
}

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentSettingsBlock {
    pub facet: AgentSettingsFacet,
    pub reason: SettingsBlockReason,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<hiroute_domain::CapabilityBlock>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub model_ids: Vec<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsBlockReason {
    QoderDefaultInUse,
    AdditionalDefaultInUse,
    QoderModelFileConflict,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SettingsPlanningError {
    #[error("Agent settings schema or context is invalid")]
    InvalidContext,
    #[error("Agent settings do not change a facet")]
    NoChanges,
    #[error("Agent settings input exceeds its bound")]
    InvalidSelection,
    #[error("Agent settings digest could not be encoded")]
    Encoding,
}

pub fn preview_agent_settings(
    mut spec: AgentSettingsSpecV2,
    facts: &AgentSettingsFacts,
) -> Result<AgentSettingsPreview, SettingsPlanningError> {
    if AGENT_SETTINGS_SCHEMA_V2
        .negotiate(spec.schema_version)
        .is_none()
        || !identity(&spec.context_id)
        || spec.context_id != facts.context_id
    {
        return Err(SettingsPlanningError::InvalidContext);
    }
    if facts.model.is_none()
        && (!matches!(spec.model, AgentFacetIntent::Keep)
            || !matches!(spec.access_token, AgentAccessTokenIntentV1::Keep)
            || spec.restore_native_model.is_some())
    {
        return Err(SettingsPlanningError::InvalidSelection);
    }
    if spec.restore_native_model.is_some()
        && !matches!(&spec.model, AgentFacetIntent::Restore { .. })
    {
        return Err(SettingsPlanningError::InvalidSelection);
    }
    if !matches!(spec.access_token, AgentAccessTokenIntentV1::Keep)
        && !matches!(spec.model, AgentFacetIntent::Configure { .. })
    {
        return Err(SettingsPlanningError::InvalidSelection);
    }
    spec.protected_native_model_ids.clear();
    if let Some(model) = facts.model.as_ref().and_then(SettingsModelFacts::codex) {
        spec.protected_native_model_ids = if model.require_native_model_routes {
            model.required_native_model_ids.clone().unwrap_or_default()
        } else {
            Vec::new()
        };
        if let AgentFacetIntent::Configure {
            settings: AgentModelSelectionV2::CodexDefault { fixed_models, .. },
        } = &mut spec.model
        {
            for preserved in &model.preserved_codex_models {
                if !fixed_models
                    .iter()
                    .any(|selected| selected.client_model_id == preserved.client_model_id)
                {
                    fixed_models.push(preserved.clone());
                }
            }
            fixed_models.sort_by(|left, right| left.client_model_id.cmp(&right.client_model_id));
        }
    }
    let mut changed_facets = BTreeSet::new();
    let mut blockers = Vec::new();
    if let Some(reason) = facts.model.as_ref().and_then(|model| match &model.native {
        SettingsModelNativeFacts::Additional(facts) => facts.model_conflict,
        _ => None,
    }) {
        if !matches!(
            reason,
            SettingsBlockReason::QoderDefaultInUse
                | SettingsBlockReason::QoderModelFileConflict
                | SettingsBlockReason::AdditionalDefaultInUse
        ) {
            return Err(SettingsPlanningError::InvalidSelection);
        }
        blockers.push(AgentSettingsBlock {
            facet: AgentSettingsFacet::Model,
            reason,
            capabilities: Vec::new(),
            model_ids: Vec::new(),
        });
    }
    let mut model_grant = None;
    let mut collaboration_trigger_mode = None;
    match &spec.model {
        AgentFacetIntent::Keep => {}
        AgentFacetIntent::Configure { settings } => {
            let model = facts
                .model
                .as_ref()
                .ok_or(SettingsPlanningError::InvalidSelection)?;
            changed_facets.insert(AgentSettingsFacet::Model);
            if matches!(
                settings,
                AgentModelSelectionV2::CodexDefault { .. }
                    | AgentModelSelectionV2::QoderAdditional { .. }
                    | AgentModelSelectionV2::PiAdditional { .. }
                    | AgentModelSelectionV2::DshAdditional { .. }
            ) {
                // A short-lived isolated HTTP challenge diagnoses a native client; it is not
                // authorization to write a safely observed Codex configuration. Actual client
                // calls have independent per-surface verification results.
                let capabilities: Vec<_> = [
                    AgentCapability::EffectiveConfiguration,
                    AgentCapability::AtomicManagedReplace,
                ]
                .into_iter()
                .filter_map(|capability| {
                    missing_capability(facts, capability)
                        .map(|reason| hiroute_domain::CapabilityBlock { capability, reason })
                })
                .collect();
                if !capabilities.is_empty() {
                    blockers.push(AgentSettingsBlock {
                        facet: AgentSettingsFacet::Model,
                        reason: SettingsBlockReason::CapabilityUnavailable,
                        capabilities,
                        model_ids: Vec::new(),
                    });
                }
            } else {
                require(
                    facts,
                    AgentAction::ConfigureModel,
                    AgentSettingsFacet::Model,
                    &mut blockers,
                );
            }
            if matches!(settings, AgentModelSelectionV2::QoderAdditional { .. })
                && !model
                    .common
                    .available_surfaces
                    .contains(&hiroute_domain::AgentModelSurfaceV2::QoderCli)
            {
                return Err(SettingsPlanningError::InvalidSelection);
            }
            if matches!(settings, AgentModelSelectionV2::PiAdditional { .. })
                && !model
                    .common
                    .available_surfaces
                    .contains(&hiroute_domain::AgentModelSurfaceV2::PiCli)
            {
                return Err(SettingsPlanningError::InvalidSelection);
            }
            if matches!(settings, AgentModelSelectionV2::DshAdditional { .. })
                && !model
                    .common
                    .available_surfaces
                    .contains(&hiroute_domain::AgentModelSurfaceV2::DshCli)
            {
                return Err(SettingsPlanningError::InvalidSelection);
            }
            settings
                .validate()
                .map_err(|_| SettingsPlanningError::InvalidSelection)?;
            if let AgentModelSelectionV2::ClaudeLauncher { surfaces, .. } = settings
                && !surfaces.is_subset(&model.common.available_surfaces)
            {
                return Err(SettingsPlanningError::InvalidSelection);
            }
            if matches!(settings, AgentModelSelectionV2::CodexDefault { .. })
                && !settings.allowed_plan_ids().is_empty()
                && model.codex().is_some_and(|facts| facts.context_override)
            {
                blockers.push(AgentSettingsBlock {
                    facet: AgentSettingsFacet::Model,
                    reason: SettingsBlockReason::CodexContextOverride,
                    capabilities: Vec::new(),
                    model_ids: Vec::new(),
                });
            }
            let grant = derive_model_grant(settings, model);
            match grant {
                Ok(grant) => {
                    // Every Codex selection publishes one exact client catalog, including
                    // HiRoute-only fixed routes without a Plan.
                    if matches!(settings, AgentModelSelectionV2::CodexDefault { .. })
                        && model
                            .codex()
                            .and_then(|facts| facts.model_catalog.as_ref())
                            .is_none()
                    {
                        blockers.push(AgentSettingsBlock {
                            facet: AgentSettingsFacet::Model,
                            reason: SettingsBlockReason::CapabilityUnavailable,
                            capabilities: vec![hiroute_domain::CapabilityBlock {
                                capability: AgentCapability::ModelCatalog,
                                reason: missing_capability(facts, AgentCapability::ModelCatalog)
                                    .unwrap_or(hiroute_domain::CapabilityBlockReason::Unavailable),
                            }],
                            model_ids: Vec::new(),
                        });
                    }
                    if let Some(model) = model.codex()
                        && let Some(required) = &model.required_native_model_ids
                    {
                        let missing = required
                            .iter()
                            .filter(|name| {
                                !model.preserved_codex_models.iter().any(|preserved| {
                                    preserved.client_model_id == name.as_str()
                                        && settings
                                            .fixed_models()
                                            .iter()
                                            .any(|selected| selected == preserved)
                                })
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        if model.require_native_model_routes
                            && (required.is_empty() || !missing.is_empty())
                        {
                            blockers.push(AgentSettingsBlock {
                                facet: AgentSettingsFacet::Model,
                                reason: SettingsBlockReason::NativeModelCoverageUnavailable,
                                capabilities: Vec::new(),
                                model_ids: missing,
                            });
                        }
                        if model.native_default_must_be_original
                            && model
                                .native_default_model
                                .as_ref()
                                .is_some_and(|default| !required.contains(default))
                        {
                            blockers.push(AgentSettingsBlock {
                                facet: AgentSettingsFacet::Model,
                                reason: SettingsBlockReason::NativeDefaultInvalid,
                                capabilities: Vec::new(),
                                model_ids: model.native_default_model.iter().cloned().collect(),
                            });
                        }
                    }
                    model_grant = Some(grant)
                }
                Err(_) => blockers.push(AgentSettingsBlock {
                    facet: AgentSettingsFacet::Model,
                    reason: SettingsBlockReason::ModelPlanUnavailable,
                    capabilities: Vec::new(),
                    model_ids: Vec::new(),
                }),
            }
        }
        AgentFacetIntent::Restore { restore_point_ref } => {
            let model = facts
                .model
                .as_ref()
                .ok_or(SettingsPlanningError::InvalidSelection)?;
            changed_facets.insert(AgentSettingsFacet::Model);
            restore(
                facts,
                AgentAction::RestoreModel,
                AgentSettingsFacet::Model,
                restore_point_ref,
                &mut blockers,
            );
            if let Some(model) = model.codex()
                && !model.restore_inherits_root
            {
                let chosen = spec
                    .restore_native_model
                    .as_ref()
                    .or(model.restored_native_model.as_ref());
                let valid = model
                    .restore_native_model_ids
                    .as_ref()
                    .is_some_and(|models| {
                        !models.is_empty() && chosen.is_none_or(|model| models.contains(model))
                    });
                if !valid {
                    blockers.push(AgentSettingsBlock {
                        facet: AgentSettingsFacet::Model,
                        reason: SettingsBlockReason::RestoreNativeModelInvalid,
                        capabilities: Vec::new(),
                        model_ids: model.restored_native_model.iter().cloned().collect(),
                    });
                }
            } else if spec.restore_native_model.is_some() {
                return Err(SettingsPlanningError::InvalidSelection);
            }
        }
    }
    match &spec.collaboration {
        AgentFacetIntent::Keep => {}
        AgentFacetIntent::Configure { settings } => {
            changed_facets.insert(AgentSettingsFacet::Collaboration);
            require(
                facts,
                AgentAction::InstallCollaborationSkill,
                AgentSettingsFacet::Collaboration,
                &mut blockers,
            );
            collaboration_trigger_mode = Some(settings.trigger_mode);
            if facts.collaboration_file_conflict {
                blockers.push(AgentSettingsBlock {
                    facet: AgentSettingsFacet::Collaboration,
                    reason: SettingsBlockReason::SkillFileConflict,
                    capabilities: Vec::new(),
                    model_ids: Vec::new(),
                });
            }
        }
        AgentFacetIntent::Restore { restore_point_ref } => {
            changed_facets.insert(AgentSettingsFacet::Collaboration);
            restore(
                facts,
                AgentAction::RemoveCollaborationSkill,
                AgentSettingsFacet::Collaboration,
                restore_point_ref,
                &mut blockers,
            );
            if facts.collaboration_file_conflict {
                blockers.push(AgentSettingsBlock {
                    facet: AgentSettingsFacet::Collaboration,
                    reason: SettingsBlockReason::SkillFileConflict,
                    capabilities: Vec::new(),
                    model_ids: Vec::new(),
                });
            }
        }
    }
    if changed_facets.is_empty() {
        return Err(SettingsPlanningError::NoChanges);
    }
    // Exclude diagnostic probe timestamps from the semantic confirmation digest. Include every
    // action-relevant proof and selection so recapture cannot silently change a confirmed facet.
    let proofs = [
        AgentCapability::EffectiveConfiguration,
        AgentCapability::AtomicManagedReplace,
        AgentCapability::IngressAuthentication,
        AgentCapability::ModelCatalog,
        AgentCapability::SkillLoading,
        AgentCapability::TrustedCliExecution,
        AgentCapability::IsolatedVerification,
    ]
    .into_iter()
    .map(|capability| {
        (
            capability,
            facts.capabilities.get(capability).map(|proof| {
                (
                    proof.state,
                    &proof.adapter_contract,
                    &proof.dependency_digest,
                )
            }),
        )
    })
    .collect::<Vec<_>>();
    let context_windows = match model_grant
        .as_ref()
        .map(|grant| {
            plan_context_windows(
                grant,
                facts
                    .model
                    .as_ref()
                    .and_then(|model| model.common.model_publication.as_ref()),
            )
        })
        .transpose()
    {
        Ok(windows) => windows.unwrap_or_default(),
        Err(_) => {
            blockers.push(AgentSettingsBlock {
                facet: AgentSettingsFacet::Model,
                reason: SettingsBlockReason::ModelPlanUnavailable,
                capabilities: Vec::new(),
                model_ids: Vec::new(),
            });
            BTreeMap::new()
        }
    };
    let claude_context_window = if matches!(
        &spec.model,
        AgentFacetIntent::Configure {
            settings: AgentModelSelectionV2::ClaudeLauncher { .. }
        }
    ) && !context_windows.is_empty()
    {
        let model = facts
            .model
            .as_ref()
            .ok_or(SettingsPlanningError::InvalidSelection)?;
        let model = model
            .claude()
            .ok_or(SettingsPlanningError::InvalidSelection)?;
        if model.plan_capability_unavailable {
            blockers.push(AgentSettingsBlock {
                facet: AgentSettingsFacet::Model,
                reason: SettingsBlockReason::ClaudePlanCapabilityUnavailable,
                capabilities: Vec::new(),
                model_ids: Vec::new(),
            });
        }
        let window = context_windows
            .values()
            .copied()
            .min()
            .and_then(hiroute_domain::claude_context_window);
        if model.context_override || window.is_none() {
            blockers.push(AgentSettingsBlock {
                facet: AgentSettingsFacet::Model,
                reason: if model.context_override {
                    SettingsBlockReason::ClaudeContextOverride
                } else {
                    SettingsBlockReason::ClaudeContextWindowUnsupported
                },
                capabilities: Vec::new(),
                model_ids: Vec::new(),
            });
        }
        window
    } else {
        None
    };
    let accept_digest = CanonicalDigest::of(&(
        "hiroute.agent-settings-preview/v2",
        &spec,
        &facts.dependency_digest,
        proofs,
        &model_grant,
        &context_windows,
        claude_context_window,
        &collaboration_trigger_mode,
        &facts.restore_points,
        &blockers,
    ))
    .map_err(|_| SettingsPlanningError::Encoding)?;
    Ok(AgentSettingsPreview {
        context_windows,
        claude_context_window,
        spec,
        dependency_digest: facts.dependency_digest.clone(),
        accept_digest,
        changed_facets,
        model_grant,
        collaboration_trigger_mode,
        blockers,
        unproven_native_model_ids: facts
            .model
            .as_ref()
            .and_then(SettingsModelFacts::codex)
            .map(|model| model.unproven_native_model_ids.clone())
            .unwrap_or_default(),
    })
}

fn derive_model_grant(
    settings: &AgentModelSelectionV2,
    facts: &SettingsModelFacts,
) -> Result<AgentModelGrantV2, SettingsPlanningError> {
    let invalid = || SettingsPlanningError::InvalidSelection;
    let ordinary_fixed = settings
        .fixed_models()
        .iter()
        .filter(|selection| {
            !facts.codex().is_some_and(|facts| {
                facts
                    .preserved_codex_bindings
                    .contains_key(&selection.client_model_id)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut fixed = crate::compiler::compile_fixed_model_bindings(
        &ordinary_fixed,
        &facts.common.fixed_candidate_facts,
    )
    .map_err(|_| invalid())?;
    for selection in settings.fixed_models() {
        if let Some(binding) = facts.codex().and_then(|facts| {
            facts
                .preserved_codex_bindings
                .get(&selection.client_model_id)
        }) {
            fixed.insert(selection.client_model_id.clone(), binding.clone());
        }
    }
    let grant = AgentModelGrantV2::derive(
        facts.common.ingress,
        settings,
        facts
            .common
            .model_publication
            .as_ref()
            .ok_or_else(invalid)?,
        &fixed,
    )
    .map_err(|_| invalid())?;
    validate_model_default(settings, facts, &grant)?;
    Ok(grant)
}

fn validate_model_default(
    settings: &AgentModelSelectionV2,
    facts: &SettingsModelFacts,
    grant: &AgentModelGrantV2,
) -> Result<(), SettingsPlanningError> {
    let invalid = || SettingsPlanningError::InvalidSelection;
    let claude_presets;
    let default_name = match settings {
        AgentModelSelectionV2::CodexDefault { default_selection, .. } => match default_selection {
            AgentModelDefaultSelectionV2::PreserveNative => facts.codex().ok_or_else(invalid)?.native_default_model.as_deref(),
            AgentModelDefaultSelectionV2::FixedModel { client_model_id } => Some(client_model_id.as_str()),
            AgentModelDefaultSelectionV2::Plan { plan_id } => grant.routes.iter().find_map(|(name, route)| {
                matches!(route, AgentModelRouteV2::Plan { plan_id: selected, .. } if selected == plan_id)
                    .then_some(name.as_str())
            }),
        },
        AgentModelSelectionV2::QoderAdditional { .. } | AgentModelSelectionV2::PiAdditional { .. } | AgentModelSelectionV2::DshAdditional { .. } => return Ok(()),
        AgentModelSelectionV2::ClaudeLauncher { .. } => {
            let facts = facts.claude().ok_or_else(invalid)?;
            claude_presets = grant
                .claude_preset_values(
                    settings,
                    facts.presets.as_ref().ok_or_else(invalid)?,
                )
                .map_err(|_| invalid())?;
            match facts.native_default_model.as_deref() {
                // Claude chooses Default from account/organization state when neither native
                // setting selects a concrete model (or the setting literally says "default").
                // Preset mappings can be saved, but they do not prove that initial selection.
                None | Some("default") => return Ok(()),
                Some("opus") => claude_presets.opus.as_deref(),
                Some("sonnet") => claude_presets.sonnet.as_deref(),
                Some("haiku") => claude_presets.haiku.as_deref(),
                other => other,
            }
        }
    };
    if !default_name.is_some_and(|name| grant.permits_name(name)) {
        return Err(invalid());
    }
    Ok(())
}

fn require(
    facts: &AgentSettingsFacts,
    action: AgentAction,
    facet: AgentSettingsFacet,
    blockers: &mut Vec<AgentSettingsBlock>,
) {
    if let Err(capabilities) = facts.capabilities.require(action, &facts.dependency_digest) {
        blockers.push(AgentSettingsBlock {
            facet,
            reason: SettingsBlockReason::CapabilityUnavailable,
            capabilities,
            model_ids: Vec::new(),
        });
    }
}
fn restore(
    facts: &AgentSettingsFacts,
    action: AgentAction,
    facet: AgentSettingsFacet,
    reference: &str,
    blockers: &mut Vec<AgentSettingsBlock>,
) {
    require(facts, action, facet, blockers);
    if !identity(reference) || facts.restore_points.get(reference) != Some(&facet) {
        blockers.push(AgentSettingsBlock {
            facet,
            reason: SettingsBlockReason::RestorePointUnavailable,
            capabilities: Vec::new(),
            model_ids: Vec::new(),
        });
    }
}
/// None means the capability is proven against the current dependency digest.
fn missing_capability(
    facts: &AgentSettingsFacts,
    capability: AgentCapability,
) -> Option<hiroute_domain::CapabilityBlockReason> {
    match facts.capabilities.get(capability) {
        None => Some(hiroute_domain::CapabilityBlockReason::Unknown),
        Some(proof) if proof.dependency_digest != facts.dependency_digest => {
            Some(hiroute_domain::CapabilityBlockReason::Stale)
        }
        Some(proof) => match proof.state {
            hiroute_domain::CapabilityState::Proven => None,
            hiroute_domain::CapabilityState::Unknown => {
                Some(hiroute_domain::CapabilityBlockReason::Unknown)
            }
            hiroute_domain::CapabilityState::Unavailable => {
                Some(hiroute_domain::CapabilityBlockReason::Unavailable)
            }
        },
    }
}
pub(super) fn identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./:-".contains(&byte))
}

mod confirmation;
mod transaction;
pub use confirmation::*;

#[cfg(test)]
mod tests;

/// Bind displayed and installed windows to the same exact publication used to derive the grant.
pub fn plan_context_windows(
    grant: &AgentModelGrantV2,
    publication: Option<&GatewayPublicationV1>,
) -> Result<BTreeMap<String, u64>, SettingsPlanningError> {
    let mut windows = BTreeMap::new();
    for (name, route) in &grant.routes {
        if let AgentModelRouteV2::Plan {
            plan_id,
            revision,
            semantic_digest,
            ..
        } = route
        {
            let plan = publication
                .and_then(|p| {
                    p.plans.iter().find(|p| {
                        p.agent_plan_id() == plan_id && p.body.agent_plan_revision == *revision
                    })
                })
                .ok_or(SettingsPlanningError::InvalidSelection)?;
            let plan = plan
                .clone()
                .into_current()
                .map_err(|_| SettingsPlanningError::InvalidSelection)?;
            if &plan.body.materialized_route_digest != semantic_digest {
                return Err(SettingsPlanningError::InvalidSelection);
            }
            windows.insert(
                name.clone(),
                plan.body
                    .materialized
                    .context_window_tokens()
                    .map_err(|_| SettingsPlanningError::InvalidSelection)?,
            );
        }
    }
    Ok(windows)
}
