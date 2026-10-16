//! Evidence for individual native actions. Version labels are diagnostic installation facts;
//! neither model configuration nor Skill installation uses a version allowlist as permission.
use crate::CanonicalDigest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapability {
    EffectiveConfiguration,
    AtomicManagedReplace,
    IngressAuthentication,
    ModelCatalog,
    SkillLoading,
    TrustedCliExecution,
    IsolatedVerification,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Proven,
    Unavailable,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityEvidence {
    pub capability: AgentCapability,
    pub state: CapabilityState,
    pub adapter_contract: String,
    pub observed_at_unix_ms: u64,
    /// Includes the action's target context, consumed layers, and any adapter-specific helper
    /// identities. Codex model configuration deliberately excludes client binary identity and
    /// version; Apply reconstructs the exact dependencies declared by the adapter.
    pub dependency_digest: CanonicalDigest,
    /// Closed diagnostic code supplied by the adapter, never probe output or config bytes.
    pub reason: Option<CapabilityReason>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReason {
    NotProbed,
    UnsupportedSchema,
    HigherPrecedenceConflict,
    UnsafeTarget,
    MissingTrustedHelper,
    IsolationUnproven,
    UnsupportedNativeFeature,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentAction {
    ConfigureModel,
    InstallCollaborationSkill,
    InstallManagementSkill,
    RemoveManagementSkill,
    RestoreModel,
    RemoveCollaborationSkill,
    VerifyModel,
    VerifySkillDirectory,
}

impl AgentAction {
    pub fn requires(self) -> &'static [AgentCapability] {
        use AgentCapability::*;
        match self {
            Self::ConfigureModel => &[
                EffectiveConfiguration,
                AtomicManagedReplace,
                IngressAuthentication,
            ],
            Self::InstallManagementSkill => &[AtomicManagedReplace, SkillLoading],
            Self::RemoveManagementSkill => &[AtomicManagedReplace],
            Self::InstallCollaborationSkill => {
                &[AtomicManagedReplace, SkillLoading, TrustedCliExecution]
            }
            Self::RestoreModel => &[EffectiveConfiguration, AtomicManagedReplace],
            Self::RemoveCollaborationSkill => &[AtomicManagedReplace],
            Self::VerifyModel => &[
                EffectiveConfiguration,
                IngressAuthentication,
                IsolatedVerification,
            ],
            Self::VerifySkillDirectory => {
                &[SkillLoading, TrustedCliExecution, IsolatedVerification]
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentCapabilitySet(BTreeMap<AgentCapability, CapabilityEvidence>);

impl AgentCapabilitySet {
    pub fn new(
        evidence: impl IntoIterator<Item = CapabilityEvidence>,
    ) -> Result<Self, InvalidCapabilityEvidence> {
        let mut entries = BTreeMap::new();
        for item in evidence {
            if item.observed_at_unix_ms == 0
                || item.adapter_contract.is_empty()
                || item.adapter_contract.len() > 128
                || !item
                    .adapter_contract
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b))
                || CanonicalDigest::parse(item.dependency_digest.as_str().to_owned()).is_err()
                || entries.insert(item.capability, item).is_some()
            {
                return Err(InvalidCapabilityEvidence);
            }
        }
        Ok(Self(entries))
    }

    pub fn get(&self, capability: AgentCapability) -> Option<&CapabilityEvidence> {
        self.0.get(&capability)
    }

    /// Extend a trusted fact snapshot without turning stale capability evidence into proof.
    pub fn rebind_dependency(&mut self, previous: &CanonicalDigest, current: &CanonicalDigest) {
        for evidence in self.0.values_mut() {
            if &evidence.dependency_digest == previous {
                evidence.dependency_digest = current.clone();
            }
        }
    }

    pub fn require(
        &self,
        action: AgentAction,
        current_dependencies: &CanonicalDigest,
    ) -> Result<(), Vec<CapabilityBlock>> {
        let missing = action
            .requires()
            .iter()
            .filter_map(|capability| {
                let reason = match self.0.get(capability) {
                    None => CapabilityBlockReason::Unknown,
                    Some(proof) if &proof.dependency_digest != current_dependencies => {
                        CapabilityBlockReason::Stale
                    }
                    Some(proof) => match proof.state {
                        CapabilityState::Proven => return None,
                        CapabilityState::Unknown => CapabilityBlockReason::Unknown,
                        CapabilityState::Unavailable => CapabilityBlockReason::Unavailable,
                    },
                };
                Some(CapabilityBlock {
                    capability: *capability,
                    reason,
                })
            })
            .collect::<Vec<_>>();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("native capability evidence is invalid or duplicated")]
pub struct InvalidCapabilityEvidence;

#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityBlock {
    pub capability: AgentCapability,
    pub reason: CapabilityBlockReason,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityBlockReason {
    Unknown,
    Unavailable,
    Stale,
}

#[cfg(test)]
mod tests;
