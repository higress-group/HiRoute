use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AgentActivationModeV1, AgentIngressProtocolV1, AgentPlanDisplayName, AgentPlanId,
    AgentPlanPurpose, CanonicalDigest, ModelAlias,
};

pub const AGENT_CONNECTION_SCHEMA_V1: &str = "hiroute.agent-connection/v1";
pub const AGENT_PLAN_GRANT_SCHEMA_V1: &str = "hiroute.agent-plan-grant/v1";
pub const GATEWAY_ACCESS_GRANT_SCHEMA_V1: &str = "hiroute.gateway-access-grant/v1";

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlanAllowedScopeV1 {
    AllPublished,
    #[default]
    Selected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedAgentPlanV1 {
    pub agent_plan_id: AgentPlanId,
    pub model_alias: ModelAlias,
    pub display_name: AgentPlanDisplayName,
    pub purpose: AgentPlanPurpose,
    pub agent_plan_revision: u64,
    pub active: bool,
    pub supported_ingress: BTreeSet<AgentIngressProtocolV1>,
}

impl PublishedAgentPlanV1 {
    pub fn validate(&self) -> Result<(), AgentConnectionError> {
        AgentPlanId::parse(self.agent_plan_id.as_str())
            .map_err(|_| AgentConnectionError::InvalidPlan)?;
        ModelAlias::parse(self.model_alias.as_str())
            .map_err(|_| AgentConnectionError::InvalidPlan)?;
        self.display_name
            .validate()
            .map_err(|_| AgentConnectionError::InvalidPlan)?;
        self.purpose
            .validate()
            .map_err(|_| AgentConnectionError::InvalidPlan)?;
        if self.agent_plan_revision == 0 || self.supported_ingress.is_empty() {
            return Err(AgentConnectionError::InvalidPlan);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanGrantV1 {
    pub schema: String,
    pub allowed_scope: AgentPlanAllowedScopeV1,
    pub default_agent_plan_id: AgentPlanId,
    pub allowed_agent_plan_ids: BTreeSet<AgentPlanId>,
    pub aliases: BTreeMap<AgentPlanId, ModelAlias>,
    pub digest: CanonicalDigest,
}

impl AgentPlanGrantV1 {
    pub fn derive(
        protocol: AgentIngressProtocolV1,
        default_agent_plan_id: AgentPlanId,
        allowed_agent_plan_ids: BTreeSet<AgentPlanId>,
        plans: &[PublishedAgentPlanV1],
    ) -> Result<Self, AgentConnectionError> {
        Self::derive_with_scope(
            protocol,
            AgentPlanAllowedScopeV1::Selected,
            default_agent_plan_id,
            allowed_agent_plan_ids,
            plans,
        )
    }

    pub fn derive_with_scope(
        protocol: AgentIngressProtocolV1,
        allowed_scope: AgentPlanAllowedScopeV1,
        default_agent_plan_id: AgentPlanId,
        allowed_agent_plan_ids: BTreeSet<AgentPlanId>,
        plans: &[PublishedAgentPlanV1],
    ) -> Result<Self, AgentConnectionError> {
        if allowed_agent_plan_ids.is_empty()
            || !allowed_agent_plan_ids.contains(&default_agent_plan_id)
            || allowed_agent_plan_ids.len() > 32
        {
            return Err(AgentConnectionError::InvalidGrant);
        }
        let mut published = BTreeMap::new();
        for plan in plans {
            plan.validate()?;
            if published.insert(&plan.agent_plan_id, plan).is_some() {
                return Err(AgentConnectionError::DuplicatePlan);
            }
        }
        if allowed_scope == AgentPlanAllowedScopeV1::AllPublished {
            let eligible = plans
                .iter()
                .filter(|plan| plan.active && plan.supported_ingress.contains(&protocol))
                .map(|plan| plan.agent_plan_id.clone())
                .collect::<BTreeSet<_>>();
            if allowed_agent_plan_ids != eligible {
                return Err(AgentConnectionError::InvalidGrant);
            }
        }
        let aliases = allowed_agent_plan_ids
            .iter()
            .map(|plan_id| {
                let plan = published
                    .get(plan_id)
                    .ok_or(AgentConnectionError::PlanNotPublished)?;
                if !plan.active || !plan.supported_ingress.contains(&protocol) {
                    return Err(AgentConnectionError::PlanNotRoutable);
                }
                Ok((plan_id.clone(), plan.model_alias.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let body = GrantBodyV1 {
            schema: AGENT_PLAN_GRANT_SCHEMA_V1,
            allowed_scope,
            default_agent_plan_id: &default_agent_plan_id,
            allowed_agent_plan_ids: &allowed_agent_plan_ids,
            aliases: &aliases,
        };
        let digest = CanonicalDigest::of(&body).map_err(|_| AgentConnectionError::Encoding)?;
        Ok(Self {
            schema: AGENT_PLAN_GRANT_SCHEMA_V1.to_owned(),
            allowed_scope,
            default_agent_plan_id,
            allowed_agent_plan_ids,
            aliases,
            digest,
        })
    }

    pub fn validate(&self) -> Result<(), AgentConnectionError> {
        if self.schema != AGENT_PLAN_GRANT_SCHEMA_V1
            || self.allowed_agent_plan_ids.is_empty()
            || !self
                .allowed_agent_plan_ids
                .contains(&self.default_agent_plan_id)
            || self.aliases.keys().cloned().collect::<BTreeSet<_>>() != self.allowed_agent_plan_ids
        {
            return Err(AgentConnectionError::InvalidGrant);
        }
        let body = GrantBodyV1 {
            schema: AGENT_PLAN_GRANT_SCHEMA_V1,
            allowed_scope: self.allowed_scope,
            default_agent_plan_id: &self.default_agent_plan_id,
            allowed_agent_plan_ids: &self.allowed_agent_plan_ids,
            aliases: &self.aliases,
        };
        if CanonicalDigest::of(&body).map_err(|_| AgentConnectionError::Encoding)? != self.digest {
            return Err(AgentConnectionError::DigestMismatch);
        }
        Ok(())
    }

    pub fn permits_alias(&self, alias: &ModelAlias) -> bool {
        self.aliases.values().any(|candidate| candidate == alias)
    }
}

/// Non-secret, publication-ready authority for one AgentConnection.
///
/// The bearer value never enters this type. `bearer_token_sha256` is the irreversible verifier
/// consumed by Gateway authentication, while the embedded Plan grant preserves the exact alias
/// scope that also drives catalog visibility.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayAccessGrantV1 {
    pub schema: String,
    pub grant_id: String,
    pub generation: u64,
    pub bearer_token_sha256: CanonicalDigest,
    pub protocol: AgentIngressProtocolV1,
    pub model_grant: crate::AgentModelGrantV2,
}

impl GatewayAccessGrantV1 {
    pub fn new(
        grant_id: impl Into<String>,
        generation: u64,
        bearer_token_sha256: CanonicalDigest,
        protocol: AgentIngressProtocolV1,
        model_grant: crate::AgentModelGrantV2,
    ) -> Result<Self, AgentConnectionError> {
        let value = Self {
            schema: GATEWAY_ACCESS_GRANT_SCHEMA_V1.to_owned(),
            grant_id: grant_id.into(),
            generation,
            bearer_token_sha256,
            protocol,
            model_grant,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), AgentConnectionError> {
        if self.schema != GATEWAY_ACCESS_GRANT_SCHEMA_V1
            || !valid_identifier(&self.grant_id)
            || self.generation == 0
            || !matches!(
                CanonicalDigest::parse(self.bearer_token_sha256.as_str()),
                Ok(ref parsed) if parsed == &self.bearer_token_sha256
            )
        {
            return Err(AgentConnectionError::InvalidGatewayGrant);
        }
        if self.model_grant.protocol != self.protocol {
            return Err(AgentConnectionError::InvalidGatewayGrant);
        }
        self.model_grant
            .validate()
            .map_err(|_| AgentConnectionError::InvalidGatewayGrant)
    }
}

#[derive(Serialize)]
struct GrantBodyV1<'a> {
    schema: &'static str,
    allowed_scope: AgentPlanAllowedScopeV1,
    default_agent_plan_id: &'a AgentPlanId,
    allowed_agent_plan_ids: &'a BTreeSet<AgentPlanId>,
    aliases: &'a BTreeMap<AgentPlanId, ModelAlias>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConnectionV1 {
    pub schema: String,
    pub agent_id: String,
    pub profile_id: String,
    pub integration_profile_ref: String,
    pub protocol: AgentIngressProtocolV1,
    pub activation_mode: AgentActivationModeV1,
    pub grant: AgentPlanGrantV1,
    pub native_subagent_routing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_digest: Option<CanonicalDigest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_digest: Option<CanonicalDigest>,
    pub revision: u64,
}

impl AgentConnectionV1 {
    pub fn validate(&self) -> Result<(), AgentConnectionError> {
        if self.schema != AGENT_CONNECTION_SCHEMA_V1
            || self.revision == 0
            || !valid_identifier(&self.agent_id)
            || !valid_identifier(&self.profile_id)
            || !valid_identifier(&self.integration_profile_ref)
            || (self.native_subagent_routing
                != (self.catalog_digest.is_some() && self.overlay_digest.is_some()))
        {
            return Err(AgentConnectionError::InvalidConnection);
        }
        self.grant.validate()
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentConnectionError {
    #[error("published AgentPlan is invalid")]
    InvalidPlan,
    #[error("published AgentPlan is duplicated")]
    DuplicatePlan,
    #[error("AgentConnection grant is invalid")]
    InvalidGrant,
    #[error("Gateway access grant is invalid")]
    InvalidGatewayGrant,
    #[error("allowed AgentPlan is not in the active publication")]
    PlanNotPublished,
    #[error("allowed AgentPlan is inactive or incompatible with the exact profile protocol")]
    PlanNotRoutable,
    #[error("AgentConnection is invalid")]
    InvalidConnection,
    #[error("canonical encoding failed")]
    Encoding,
    #[error("canonical digest does not match the value")]
    DigestMismatch,
}
