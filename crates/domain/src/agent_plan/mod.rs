//! Stable AgentPlan identity and alias lifecycle.
//!
//! Aliases are Application-assigned, immutable for every revision of a Plan, and retained as
//! tombstones after retirement.  The registry is serializable so the allocator can be recovered
//! without ever consulting a client, model catalog, or Gateway runtime.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const AGENT_PLAN_ALIAS_REGISTRY_SCHEMA_V1: &str = "hiroute.agent-plan-alias-registry/v1";

#[derive(
    schemars::JsonSchema, Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct AgentPlanId(String);

impl AgentPlanId {
    pub fn parse(value: impl Into<String>) -> Result<Self, AgentPlanIdentityError> {
        let value = value.into();
        if valid_identifier(&value) {
            Ok(Self(value))
        } else {
            Err(AgentPlanIdentityError::InvalidPlanId)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(
    schemars::JsonSchema, Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct ModelAlias(String);

impl ModelAlias {
    /// User-selected first-publication alias. Legacy automatic aliases remain read-compatible
    /// through `parse`, but cannot be supplied as a custom allocator namespace.
    pub fn parse_custom(value: impl Into<String>) -> Result<Self, AgentPlanIdentityError> {
        let value = value.into();
        if valid_custom_alias(&value) {
            Ok(Self(value))
        } else {
            Err(AgentPlanIdentityError::InvalidAlias)
        }
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, AgentPlanIdentityError> {
        let value = value.into();
        let suffix = value.strip_prefix("hiroute/");
        if suffix.is_some_and(|suffix| {
            suffix.len() == 16
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) || valid_custom_alias(&value)
        {
            Ok(Self(value))
        } else {
            Err(AgentPlanIdentityError::InvalidAlias)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentPlanPurpose(String);

impl AgentPlanPurpose {
    pub fn parse(value: impl Into<String>) -> Result<Self, AgentPlanIdentityError> {
        let value = value.into();
        if (1..=512).contains(&value.chars().count())
            && value.trim() == value
            && !value.chars().any(char::is_control)
            && !looks_like_sensitive_material(&value)
        {
            Ok(Self(value))
        } else {
            Err(AgentPlanIdentityError::InvalidPurpose)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn validate(&self) -> Result<(), AgentPlanIdentityError> {
        Self::parse(self.as_str()).map(|_| ())
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentPlanDisplayName(String);

impl AgentPlanDisplayName {
    pub fn parse(value: impl Into<String>) -> Result<Self, AgentPlanIdentityError> {
        let value = value.into();
        if (1..=128).contains(&value.chars().count())
            && value.trim() == value
            && !value.chars().any(char::is_control)
            && !looks_like_sensitive_material(&value)
        {
            Ok(Self(value))
        } else {
            Err(AgentPlanIdentityError::InvalidDisplayName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn validate(&self) -> Result<(), AgentPlanIdentityError> {
        Self::parse(self.as_str()).map(|_| ())
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanIdentityV1 {
    pub agent_plan_id: AgentPlanId,
    pub model_alias: ModelAlias,
    pub display_name: AgentPlanDisplayName,
    pub purpose: AgentPlanPurpose,
}

impl AgentPlanIdentityV1 {
    pub fn validate(&self) -> Result<(), AgentPlanIdentityError> {
        AgentPlanId::parse(self.agent_plan_id.as_str())?;
        ModelAlias::parse(self.model_alias.as_str())?;
        self.display_name.validate()?;
        self.purpose.validate()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AliasRegistryV1 {
    pub schema: String,
    pub next_sequence: u64,
    #[serde(default)]
    pub active: BTreeMap<AgentPlanId, ModelAlias>,
    #[serde(default)]
    pub tombstones: BTreeSet<ModelAlias>,
    #[serde(default)]
    pub retired_plan_ids: BTreeSet<AgentPlanId>,
}

impl Default for AliasRegistryV1 {
    fn default() -> Self {
        Self {
            schema: AGENT_PLAN_ALIAS_REGISTRY_SCHEMA_V1.to_owned(),
            next_sequence: 1,
            active: BTreeMap::new(),
            tombstones: BTreeSet::new(),
            retired_plan_ids: BTreeSet::new(),
        }
    }
}

impl AliasRegistryV1 {
    pub fn validate(&self) -> Result<(), AgentPlanIdentityError> {
        if self.schema != AGENT_PLAN_ALIAS_REGISTRY_SCHEMA_V1 || self.next_sequence == 0 {
            return Err(AgentPlanIdentityError::UnsupportedRegistry);
        }
        let mut aliases = BTreeSet::new();
        for (plan_id, alias) in &self.active {
            AgentPlanId::parse(plan_id.as_str())?;
            ModelAlias::parse(alias.as_str())?;
            if !aliases.insert(alias) || self.tombstones.contains(alias) {
                return Err(AgentPlanIdentityError::AliasReused);
            }
        }
        for alias in &self.tombstones {
            ModelAlias::parse(alias.as_str())?;
        }
        for plan_id in &self.retired_plan_ids {
            AgentPlanId::parse(plan_id.as_str())?;
        }
        if self
            .retired_plan_ids
            .iter()
            .any(|plan_id| self.active.contains_key(plan_id))
        {
            return Err(AgentPlanIdentityError::RetiredPlanReused);
        }
        Ok(())
    }

    /// Preserves an existing alias, or uses a readable numbered default when no display name
    /// is supplied. Product authoring should use `suggest_alias` with the actual display name.
    pub fn allocate(&mut self, plan_id: AgentPlanId) -> Result<ModelAlias, AgentPlanIdentityError> {
        self.allocate_named(plan_id, "")
    }

    /// Called on the publication's working registry under the existing writer. Preview may
    /// use a clone; only successful publication commits ownership of the selected alias.
    pub fn allocate_custom(
        &mut self,
        plan_id: AgentPlanId,
        alias: ModelAlias,
    ) -> Result<ModelAlias, AgentPlanIdentityError> {
        self.validate()?;
        AgentPlanId::parse(plan_id.as_str())?;
        ModelAlias::parse_custom(alias.as_str())?;
        if self.retired_plan_ids.contains(&plan_id) {
            return Err(AgentPlanIdentityError::RetiredPlanReused);
        }
        if let Some(existing) = self.active.get(&plan_id) {
            return if existing == &alias {
                Ok(alias)
            } else {
                Err(AgentPlanIdentityError::AliasReused)
            };
        }
        if self.tombstones.contains(&alias) || self.active.values().any(|value| value == &alias) {
            return Err(AgentPlanIdentityError::AliasReused);
        }
        self.active.insert(plan_id, alias.clone());
        Ok(alias)
    }

    pub fn retire(&mut self, plan_id: &AgentPlanId) -> Result<ModelAlias, AgentPlanIdentityError> {
        self.validate()?;
        let alias = self
            .active
            .remove(plan_id)
            .ok_or(AgentPlanIdentityError::UnknownPlan)?;
        self.tombstones.insert(alias.clone());
        self.retired_plan_ids.insert(plan_id.clone());
        self.validate()?;
        Ok(alias)
    }

    pub fn alias_for(&self, plan_id: &AgentPlanId) -> Option<&ModelAlias> {
        self.active.get(plan_id)
    }
}

fn valid_custom_alias(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && value.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("//")
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

fn looks_like_sensitive_material(value: &str) -> bool {
    let folded = value.to_ascii_lowercase();
    folded.contains("://")
        || [
            "authorization: bearer ",
            "api_key=",
            "api-key=",
            "credential_value=",
            "-----begin private key-----",
            "-----begin rsa private key-----",
        ]
        .iter()
        .any(|marker| folded.contains(marker))
        || folded
            .split(|value: char| value.is_whitespace() || value == '=' || value == ':')
            .any(|part| {
                part.strip_prefix("sk-").is_some_and(|suffix| {
                    suffix.len() >= 20 && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
                })
            })
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentPlanIdentityError {
    #[error("AgentPlan ID is not a bounded portable identifier")]
    InvalidPlanId,
    #[error("model alias is not an Application-assigned hiroute alias")]
    InvalidAlias,
    #[error("AgentPlan purpose is empty, unbounded, or contains unsafe material")]
    InvalidPurpose,
    #[error("AgentPlan display name is empty, unbounded, or contains unsafe material")]
    InvalidDisplayName,
    #[error("alias registry schema or sequence is unsupported")]
    UnsupportedRegistry,
    #[error("an active alias was reused or was already tombstoned")]
    AliasReused,
    #[error("a retired AgentPlan ID cannot be reused")]
    RetiredPlanReused,
    #[error("alias allocation sequence is exhausted")]
    SequenceExhausted,
    #[error("AgentPlan is not active")]
    UnknownPlan,
}

#[cfg(test)]
mod routing_identity_tests {
    use super::*;

    #[test]
    fn plan_display_name_matches_shared_frontend_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/routing/plan-display-name-conformance.v1.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let value = case["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|part| match part.as_str() {
                    Some(text) => text.to_owned(),
                    None => part["repeat"]
                        .as_str()
                        .unwrap()
                        .repeat(part["count"].as_u64().unwrap() as usize),
                })
                .collect::<String>();
            assert_eq!(
                AgentPlanDisplayName::parse(value).is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["case"]
            );
        }
    }

    #[test]
    fn routing_alias_is_stable_and_tombstoned_after_retirement() {
        let id = AgentPlanId::parse("plan/daily").unwrap();
        let mut registry = AliasRegistryV1::default();
        let alias = registry.allocate(id.clone()).unwrap();
        assert_eq!(registry.allocate(id.clone()).unwrap(), alias);
        assert_eq!(registry.retire(&id).unwrap(), alias);
        assert!(registry.tombstones.contains(&alias));
        assert_eq!(
            registry.allocate(id).unwrap_err(),
            AgentPlanIdentityError::RetiredPlanReused
        );

        let next = registry
            .allocate(AgentPlanId::parse("plan/research").unwrap())
            .unwrap();
        assert_ne!(alias, next);
    }

    #[test]
    fn routing_alias_registry_rejects_active_tombstone_overlap() {
        let id = AgentPlanId::parse("plan/daily").unwrap();
        let alias = ModelAlias::parse("hiroute/0011223344556677").unwrap();
        let registry = AliasRegistryV1 {
            active: BTreeMap::from([(id, alias.clone())]),
            tombstones: BTreeSet::from([alias]),
            ..AliasRegistryV1::default()
        };
        assert_eq!(
            registry.validate().unwrap_err(),
            AgentPlanIdentityError::AliasReused
        );
    }

    #[test]
    fn routing_plan_metadata_rejects_obvious_secret_material() {
        assert_eq!(
            AgentPlanPurpose::parse("Use api_key=abcdefghijklmnopqrstuvwxyz").unwrap_err(),
            AgentPlanIdentityError::InvalidPurpose
        );
        assert!(AgentPlanPurpose::parse("Review API key rotation behavior").is_ok());
        assert!(AgentPlanPurpose::parse("Send requests to https://arbitrary.invalid").is_err());
    }
}

#[cfg(test)]
mod alias_tests;

mod version;
mod version_consistency;
pub use version::*;

mod draft;
pub use draft::*;

mod readable_alias;

mod legacy;
