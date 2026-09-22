use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{CanonicalDigest, ManagedLaunchProfileV1};

pub const AGENT_PROFILE_SCHEMA_V1: &str = "hiroute.agent-profile/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKindV1 {
    Codex,
    ClaudeCode,
}

impl AgentKindV1 {
    /// Native adapter order, highest priority first. Managed Claude policy is a
    /// constraint on every lower layer, including the caller's environment.
    pub const fn config_precedence(self) -> [ConfigLayerV1; 5] {
        match self {
            Self::Codex => [
                ConfigLayerV1::Process,
                ConfigLayerV1::Launch,
                ConfigLayerV1::Project,
                ConfigLayerV1::User,
                ConfigLayerV1::Managed,
            ],
            Self::ClaudeCode => [
                ConfigLayerV1::Managed,
                ConfigLayerV1::Process,
                ConfigLayerV1::Launch,
                ConfigLayerV1::Project,
                ConfigLayerV1::User,
            ],
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentIngressProtocolV1 {
    Responses,
    Messages,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigLayerV1 {
    Process,
    Launch,
    Project,
    User,
    Managed,
}

impl ConfigLayerV1 {
    pub fn precedence_for(self, kind: AgentKindV1) -> usize {
        kind.config_precedence()
            .iter()
            .position(|layer| *layer == self)
            .expect("each native order contains every layer")
    }

    /// Smaller values have higher effective precedence.
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Process => 0,
            Self::Launch => 1,
            Self::Project => 2,
            Self::User => 3,
            Self::Managed => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogDeliveryV1 {
    DynamicWithEtag,
    StaticRestartRequired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedConfigFieldV1 {
    pub field_id: String,
    pub path: String,
    pub write_layer: ConfigLayerV1,
}

impl OwnedConfigFieldV1 {
    pub fn new(
        field_id: impl Into<String>,
        path: impl Into<String>,
        write_layer: ConfigLayerV1,
    ) -> Result<Self, AgentProfileError> {
        let value = Self {
            field_id: field_id.into(),
            path: path.into(),
            write_layer,
        };
        validate_identifier(&value.field_id)?;
        validate_config_path(&value.path)?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnGuidanceProfileV1 {
    pub tool_name: String,
    pub registration_id: String,
    pub begin_marker: String,
    pub end_marker: String,
    pub expected_shape_digest: CanonicalDigest,
}

impl SpawnGuidanceProfileV1 {
    pub fn validate(&self) -> Result<(), AgentProfileError> {
        if self.tool_name != "spawn_agent"
            || self.registration_id.is_empty()
            || self.registration_id.len() > 128
            || self.begin_marker
                != "Available model overrides (optional; inherited parent model is preferred):"
            || self.end_marker.is_empty()
            || self.end_marker.len() > 128
            || self.begin_marker == self.end_marker
            || self.begin_marker.chars().any(char::is_control)
            || self.end_marker.chars().any(char::is_control)
        {
            return Err(AgentProfileError::InvalidSpawnGuidance);
        }
        CanonicalDigest::parse(self.expected_shape_digest.as_str().to_owned())
            .map_err(|_| AgentProfileError::InvalidSpawnGuidance)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileV1 {
    pub schema: String,
    pub profile_id: String,
    pub integration_profile_ref: String,
    pub kind: AgentKindV1,
    /// Recovery-only labels from historical profiles; never used for client admission.
    #[serde(
        default,
        rename = "exact_versions",
        skip_serializing_if = "BTreeSet::is_empty"
    )]
    pub legacy_exact_versions: BTreeSet<String>,
    pub ingress_protocol: AgentIngressProtocolV1,
    pub config_precedence: Vec<ConfigLayerV1>,
    pub owned_config_fields: Vec<OwnedConfigFieldV1>,
    pub dynamic_catalog: bool,
    pub static_catalog_fallback: bool,
    pub native_subagent_routing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn_guidance: Option<SpawnGuidanceProfileV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_launch: Option<ManagedLaunchProfileV1>,
}

impl AgentProfileV1 {
    pub fn validate(&self) -> Result<(), AgentProfileError> {
        if self.schema != AGENT_PROFILE_SCHEMA_V1 {
            return Err(AgentProfileError::UnsupportedSchema);
        }
        validate_identifier(&self.profile_id)?;
        validate_identifier(&self.integration_profile_ref)?;
        let expected_precedence = self.kind.config_precedence();
        if self.config_precedence.as_slice() != expected_precedence {
            return Err(AgentProfileError::InvalidConfigPrecedence);
        }
        if self.owned_config_fields.is_empty() {
            return Err(AgentProfileError::InvalidConfigField);
        }
        let mut ids = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for field in &self.owned_config_fields {
            validate_identifier(&field.field_id)?;
            validate_config_path(&field.path)?;
            if !ids.insert(&field.field_id) || !paths.insert(&field.path) {
                return Err(AgentProfileError::DuplicateConfigField);
            }
        }
        if self.dynamic_catalog && !self.native_subagent_routing {
            return Err(AgentProfileError::InvalidCatalogCapability);
        }
        if self.static_catalog_fallback && !self.native_subagent_routing {
            return Err(AgentProfileError::InvalidCatalogCapability);
        }
        if let Some(guidance) = &self.spawn_guidance {
            if !self.native_subagent_routing {
                return Err(AgentProfileError::InvalidSpawnGuidance);
            }
            guidance.validate()?;
        }
        if let Some(managed_launch) = &self.managed_launch
            && (self.kind != AgentKindV1::ClaudeCode
                || self.ingress_protocol != AgentIngressProtocolV1::Messages
                || managed_launch.validate().is_err())
        {
            return Err(AgentProfileError::InvalidManagedLaunch);
        }
        match (self.kind, self.ingress_protocol) {
            (AgentKindV1::Codex, AgentIngressProtocolV1::Responses)
            | (AgentKindV1::ClaudeCode, AgentIngressProtocolV1::Messages) => Ok(()),
            _ => Err(AgentProfileError::ProtocolKindMismatch),
        }
    }

    pub fn supports_managed_launch(&self) -> bool {
        self.managed_launch
            .as_ref()
            .is_some_and(|capability| capability.validate().is_ok())
    }

    pub const fn client_protocol(&self) -> AgentIngressProtocolV1 {
        self.ingress_protocol
    }

    pub fn catalog_delivery(&self, dynamic_available: bool) -> Option<CatalogDeliveryV1> {
        if !self.native_subagent_routing {
            None
        } else if self.dynamic_catalog && dynamic_available {
            Some(CatalogDeliveryV1::DynamicWithEtag)
        } else if self.static_catalog_fallback {
            Some(CatalogDeliveryV1::StaticRestartRequired)
        } else {
            None
        }
    }

    pub fn field(&self, field_id: &str) -> Option<&OwnedConfigFieldV1> {
        self.owned_config_fields
            .iter()
            .find(|field| field.field_id == field_id)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveConfigFieldV1 {
    pub path: String,
    pub layer: ConfigLayerV1,
    pub value: Value,
    pub source_digest: CanonicalDigest,
}

/// Registry-resolved facts passed across the integration port into Application. Construction in
/// production belongs to an exact profile adapter; Application does not depend on that adapter.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedAgentInstallationV1 {
    pub agent_id: String,
    pub version: String,
    pub profile: AgentProfileV1,
    pub effective_config: BTreeMap<String, EffectiveConfigFieldV1>,
    pub observation_digest: CanonicalDigest,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_evidence: Vec<crate::CapabilityEvidence>,
}

impl SupportedAgentInstallationV1 {
    pub fn require_action(
        &self,
        action: crate::AgentAction,
    ) -> Result<(), Vec<crate::CapabilityBlock>> {
        let evidence =
            crate::AgentCapabilitySet::new(self.capability_evidence.clone()).map_err(|_| {
                action
                    .requires()
                    .iter()
                    .map(|capability| crate::CapabilityBlock {
                        capability: *capability,
                        reason: crate::CapabilityBlockReason::Unknown,
                    })
                    .collect::<Vec<_>>()
            })?;
        evidence.require(action, &self.observation_digest)
    }
    /// Resolve exact writes. Higher-precedence values that disagree are blockers; matching values
    /// are already effective and therefore are not claimed or rewritten by HiRoute.
    pub fn writable_values(
        &self,
        desired: &BTreeMap<String, Option<Value>>,
    ) -> Result<BTreeMap<String, Option<Value>>, AgentDiscoveryError> {
        let owned = self
            .profile
            .owned_config_fields
            .iter()
            .map(|field| (&field.path, field))
            .collect::<BTreeMap<_, _>>();
        let mut writes = BTreeMap::new();
        for (path, value) in desired {
            let field = owned
                .get(path)
                .ok_or(AgentDiscoveryError::UnownedConfigField)?;
            match self.effective_config.get(path) {
                Some(effective)
                    if effective.layer.precedence_for(self.profile.kind)
                        < field.write_layer.precedence_for(self.profile.kind) =>
                {
                    if value.as_ref() != Some(&effective.value) {
                        return Err(AgentDiscoveryError::HigherPrecedenceConflict);
                    }
                }
                _ => {
                    writes.insert(path.clone(), value.clone());
                }
            }
        }
        Ok(writes)
    }
}

fn validate_identifier(value: &str) -> Result<(), AgentProfileError> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("//")
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'));
    if valid {
        Ok(())
    } else {
        Err(AgentProfileError::InvalidIdentifier)
    }
}

fn validate_config_path(value: &str) -> Result<(), AgentProfileError> {
    if value.is_empty()
        || value.len() > 256
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Err(AgentProfileError::InvalidConfigField)
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentProfileError {
    #[error("Agent profile schema is unsupported")]
    UnsupportedSchema,
    #[error("Agent profile identifier is invalid")]
    InvalidIdentifier,
    #[error("Agent profile config precedence is not the frozen effective order")]
    InvalidConfigPrecedence,
    #[error("Agent profile config field is invalid")]
    InvalidConfigField,
    #[error("Agent profile config fields are duplicated")]
    DuplicateConfigField,
    #[error("Agent kind and ingress protocol do not match")]
    ProtocolKindMismatch,
    #[error("Agent model catalog capability is inconsistent")]
    InvalidCatalogCapability,
    #[error("Agent spawn guidance profile is invalid")]
    InvalidSpawnGuidance,
    #[error("Agent managed-launch profile is invalid")]
    InvalidManagedLaunch,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentDiscoveryError {
    #[error("desired config field is not owned by the exact Agent profile")]
    UnownedConfigField,
    #[error("a higher-precedence Agent config source conflicts with the requested value")]
    HigherPrecedenceConflict,
}
