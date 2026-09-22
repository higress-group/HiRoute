use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    AgentConnectionError, AgentIngressProtocolV1, AgentModelSelectionV2, AgentPlanId,
    AttemptOwnedCandidateV1, CandidateSelectionV1, CanonicalDigest, GatewayPublicationV1,
    ModelAlias, UpstreamProtocol, valid_client_model_name,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentModelRouteV2 {
    Plan {
        plan_id: AgentPlanId,
        alias: ModelAlias,
        revision: u64,
        semantic_digest: CanonicalDigest,
    },
    Fixed {
        candidate: CandidateSelectionV1,
        binding: Box<AttemptOwnedCandidateV1>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelRequestRouteV2 {
    Plan {
        revision: u64,
        semantic_digest: CanonicalDigest,
    },
    Fixed {
        binding_digest: CanonicalDigest,
    },
}

impl ModelRequestRouteV2 {
    pub fn plan_revision(&self) -> Option<u64> {
        match self {
            Self::Plan { revision, .. } => Some(*revision),
            Self::Fixed { .. } => None,
        }
    }

    pub fn validate(&self) -> Result<(), AgentConnectionError> {
        let digest = match self {
            Self::Plan {
                revision,
                semantic_digest,
            } if *revision > 0 => semantic_digest,
            Self::Fixed { binding_digest } => binding_digest,
            _ => return Err(AgentConnectionError::InvalidGrant),
        };
        CanonicalDigest::parse(digest.as_str()).map_err(|_| AgentConnectionError::InvalidGrant)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelGrantV2 {
    pub protocol: AgentIngressProtocolV1,
    pub routes: BTreeMap<String, AgentModelRouteV2>,
    pub digest: CanonicalDigest,
}

impl AgentModelGrantV2 {
    pub fn derive(
        protocol: AgentIngressProtocolV1,
        selection: &AgentModelSelectionV2,
        publication: &GatewayPublicationV1,
        fixed_bindings: &BTreeMap<String, AttemptOwnedCandidateV1>,
    ) -> Result<Self, AgentConnectionError> {
        selection.validate()?;
        if !matches!(
            (selection, protocol),
            (
                AgentModelSelectionV2::CodexDefault { .. },
                AgentIngressProtocolV1::Responses
            ) | (
                AgentModelSelectionV2::ClaudeLauncher { .. },
                AgentIngressProtocolV1::Messages
            )
        ) {
            return Err(AgentConnectionError::InvalidGrant);
        }
        let mut routes = Self::plan_routes(protocol, selection.allowed_plan_ids(), publication)?;
        for fixed in selection.fixed_models() {
            let binding = fixed_bindings
                .get(&fixed.client_model_id)
                .filter(|binding| binding.binding_id == fixed.candidate.binding_id)
                .ok_or(AgentConnectionError::InvalidGrant)?;
            if routes
                .insert(
                    fixed.client_model_id.clone(),
                    AgentModelRouteV2::Fixed {
                        candidate: fixed.candidate.clone(),
                        binding: Box::new(binding.clone()),
                    },
                )
                .is_some()
            {
                return Err(AgentConnectionError::InvalidGrant);
            }
        }
        Self::seal(protocol, routes)
    }

    pub fn from_plan_ids(
        protocol: AgentIngressProtocolV1,
        plan_ids: std::collections::BTreeSet<AgentPlanId>,
        publication: &GatewayPublicationV1,
    ) -> Result<Self, AgentConnectionError> {
        Self::seal(
            protocol,
            Self::plan_routes(protocol, plan_ids, publication)?,
        )
    }

    fn plan_routes(
        protocol: AgentIngressProtocolV1,
        plan_ids: std::collections::BTreeSet<AgentPlanId>,
        publication: &GatewayPublicationV1,
    ) -> Result<BTreeMap<String, AgentModelRouteV2>, AgentConnectionError> {
        let published = publication
            .published_agent_plans()
            .map_err(|_| AgentConnectionError::InvalidPlan)?;
        let mut routes = BTreeMap::new();
        for plan_id in plan_ids {
            let plan = published
                .iter()
                .find(|plan| plan.agent_plan_id == plan_id)
                .ok_or(AgentConnectionError::PlanNotPublished)?;
            if !plan.active || !plan.supported_ingress.contains(&protocol) {
                return Err(AgentConnectionError::PlanNotRoutable);
            }
            let compiled = publication
                .plans
                .iter()
                .find(|compiled| compiled.agent_plan_id() == &plan_id)
                .ok_or(AgentConnectionError::PlanNotPublished)?;
            // Sealing a publication upgrades every Plan to the current compiler contract, so a
            // route must bind the Plan in that upgraded form; the persisted legacy digest names a
            // compiled body no current publication will ever carry again.
            let compiled = compiled
                .clone()
                .into_current()
                .map_err(|_| AgentConnectionError::InvalidPlan)?;
            if routes
                .insert(
                    plan.model_alias.as_str().to_owned(),
                    AgentModelRouteV2::Plan {
                        plan_id,
                        alias: plan.model_alias.clone(),
                        revision: plan.agent_plan_revision,
                        semantic_digest: compiled.body.materialized_route_digest.clone(),
                    },
                )
                .is_some()
            {
                return Err(AgentConnectionError::InvalidGrant);
            }
        }
        Ok(routes)
    }

    pub fn seal(
        protocol: AgentIngressProtocolV1,
        routes: BTreeMap<String, AgentModelRouteV2>,
    ) -> Result<Self, AgentConnectionError> {
        let digest = CanonicalDigest::of(&("hiroute.agent-model-grant/v2", protocol, &routes))
            .map_err(|_| AgentConnectionError::Encoding)?;
        let grant = Self {
            protocol,
            routes,
            digest,
        };
        grant.validate()?;
        Ok(grant)
    }

    pub fn validate(&self) -> Result<(), AgentConnectionError> {
        if self.routes.is_empty() {
            return Err(AgentConnectionError::InvalidGrant);
        }
        let ingress = match self.protocol {
            AgentIngressProtocolV1::Responses => UpstreamProtocol::Responses,
            AgentIngressProtocolV1::Messages => UpstreamProtocol::Messages,
        };
        for (name, route) in &self.routes {
            if !valid_client_model_name(name) {
                return Err(AgentConnectionError::InvalidGrant);
            }
            match route {
                AgentModelRouteV2::Plan {
                    plan_id,
                    alias,
                    revision,
                    semantic_digest,
                } => {
                    if name != alias.as_str()
                        || *revision == 0
                        || AgentPlanId::parse(plan_id.as_str()).is_err()
                        || ModelAlias::parse(alias.as_str()).is_err()
                        || CanonicalDigest::parse(semantic_digest.as_str()).is_err()
                    {
                        return Err(AgentConnectionError::InvalidGrant);
                    }
                }
                AgentModelRouteV2::Fixed { candidate, binding } => {
                    binding
                        .validate()
                        .map_err(|_| AgentConnectionError::InvalidGrant)?;
                    if candidate.binding_id != binding.binding_id
                        || binding
                            .protocol_profiles
                            .iter()
                            .filter(|profile| profile.ingress_protocol == ingress)
                            .count()
                            != 1
                    {
                        return Err(AgentConnectionError::InvalidGrant);
                    }
                }
            }
        }
        if CanonicalDigest::of(&("hiroute.agent-model-grant/v2", self.protocol, &self.routes))
            .map_err(|_| AgentConnectionError::Encoding)?
            != self.digest
        {
            return Err(AgentConnectionError::DigestMismatch);
        }
        Ok(())
    }

    pub fn validate_selection(
        &self,
        selection: &AgentModelSelectionV2,
    ) -> Result<(), AgentConnectionError> {
        self.validate()?;
        selection.validate()?;
        if !matches!(
            (selection, self.protocol),
            (
                AgentModelSelectionV2::CodexDefault { .. },
                AgentIngressProtocolV1::Responses
            ) | (
                AgentModelSelectionV2::ClaudeLauncher { .. },
                AgentIngressProtocolV1::Messages
            )
        ) {
            return Err(AgentConnectionError::InvalidGrant);
        }
        let plans = self
            .routes
            .values()
            .filter_map(|route| match route {
                AgentModelRouteV2::Plan { plan_id, .. } => Some(plan_id.clone()),
                AgentModelRouteV2::Fixed { .. } => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        if plans != selection.allowed_plan_ids()
            || self.routes.len() != plans.len() + selection.fixed_models().len()
            || selection.fixed_models().iter().any(|fixed| {
                !matches!(self.routes.get(&fixed.client_model_id),
                    Some(AgentModelRouteV2::Fixed { candidate, .. }) if candidate == &fixed.candidate)
            })
        {
            return Err(AgentConnectionError::InvalidGrant);
        }
        Ok(())
    }

    pub fn codex_default_override(
        &self,
        selection: &AgentModelSelectionV2,
    ) -> Result<Option<String>, AgentConnectionError> {
        self.validate_selection(selection)?;
        let AgentModelSelectionV2::CodexDefault {
            default_selection, ..
        } = selection
        else {
            return Err(AgentConnectionError::InvalidGrant);
        };
        match default_selection {
            crate::AgentModelDefaultSelectionV2::PreserveNative => Ok(None),
            crate::AgentModelDefaultSelectionV2::FixedModel { client_model_id } => Ok(Some(client_model_id.clone())),
            crate::AgentModelDefaultSelectionV2::Plan { plan_id } => self.routes.iter()
                .find_map(|(name, route)| matches!(route, AgentModelRouteV2::Plan { plan_id: selected, .. } if selected == plan_id).then_some(Some(name.clone())))
                .ok_or(AgentConnectionError::InvalidGrant),
        }
    }

    pub fn claude_preset_values(
        &self,
        selection: &AgentModelSelectionV2,
        native: &crate::AgentClaudePresetValuesV2,
    ) -> Result<crate::AgentClaudePresetValuesV2, AgentConnectionError> {
        self.validate_selection(selection)?;
        let AgentModelSelectionV2::ClaudeLauncher {
            preset_mappings, ..
        } = selection
        else {
            return Err(AgentConnectionError::InvalidGrant);
        };
        let resolve = |mapping: &crate::AgentClaudePresetSelectionV2, original: &Option<String>| {
            match mapping {
                crate::AgentClaudePresetSelectionV2::PreserveNative => {
                    if original.as_deref().is_some_and(|name| !valid_client_model_name(name)) {
                        return Err(AgentConnectionError::InvalidGrant);
                    }
                    Ok(original.clone())
                }
                crate::AgentClaudePresetSelectionV2::Plan { plan_id } => self.routes.iter()
                    .find_map(|(name, route)| matches!(route, AgentModelRouteV2::Plan { plan_id: selected, .. } if selected == plan_id)
                        .then_some(Some(name.clone())))
                    .ok_or(AgentConnectionError::InvalidGrant),
            }
        };
        Ok(crate::AgentClaudePresetValuesV2 {
            opus: resolve(&preset_mappings.opus, &native.opus)?,
            sonnet: resolve(&preset_mappings.sonnet, &native.sonnet)?,
            haiku: resolve(&preset_mappings.haiku, &native.haiku)?,
        })
    }

    pub fn permits_name(&self, name: &str) -> bool {
        self.routes.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentModelDefaultSelectionV2, AgentModelSurfaceV2};

    fn grant() -> AgentModelGrantV2 {
        AgentModelGrantV2::seal(
            AgentIngressProtocolV1::Responses,
            BTreeMap::from([(
                "hiroute-example".into(),
                AgentModelRouteV2::Plan {
                    plan_id: AgentPlanId::parse("plan/example").unwrap(),
                    alias: ModelAlias::parse_custom("hiroute-example").unwrap(),
                    revision: 1,
                    semantic_digest: CanonicalDigest::of_bytes(b"exact-plan"),
                },
            )]),
        )
        .unwrap()
    }

    fn selection(default_selection: AgentModelDefaultSelectionV2) -> AgentModelSelectionV2 {
        AgentModelSelectionV2::CodexDefault {
            native_model_mode: crate::CodexNativeModelModeV2::PreserveAvailable,
            fixed_models: Vec::new(),
            allowed_plan_ids: [AgentPlanId::parse("plan/example").unwrap()].into(),
            default_selection,
        }
    }

    #[test]
    fn preserving_native_default_does_not_materialize_a_model_write() {
        assert_eq!(
            grant()
                .codex_default_override(&selection(AgentModelDefaultSelectionV2::PreserveNative)),
            Ok(None)
        );
        assert_eq!(
            grant().codex_default_override(&selection(AgentModelDefaultSelectionV2::Plan {
                plan_id: AgentPlanId::parse("plan/example").unwrap(),
            })),
            Ok(Some("hiroute-example".into()))
        );
    }

    #[test]
    fn claude_presets_keep_absence_and_native_values_without_a_default() {
        use crate::{
            AgentClaudePresetMappingsV2, AgentClaudePresetSelectionV2, AgentClaudePresetValuesV2,
        };
        let grant =
            AgentModelGrantV2::seal(AgentIngressProtocolV1::Messages, grant().routes).unwrap();
        let mapped = AgentClaudePresetSelectionV2::Plan {
            plan_id: AgentPlanId::parse("plan/example").unwrap(),
        };
        let mut selection = AgentModelSelectionV2::ClaudeLauncher {
            surfaces: [AgentModelSurfaceV2::ClaudeCli].into(),
            fixed_models: Vec::new(),
            preset_mappings: AgentClaudePresetMappingsV2 {
                opus: mapped.clone(),
                sonnet: AgentClaudePresetSelectionV2::PreserveNative,
                haiku: AgentClaudePresetSelectionV2::PreserveNative,
            },
        };
        let native = AgentClaudePresetValuesV2 {
            opus: Some("native-opus".into()),
            sonnet: Some("Vendor/Sonnet[1m]".into()),
            haiku: None,
        };
        let values = grant.claude_preset_values(&selection, &native).unwrap();
        assert_eq!(values.opus.as_deref(), Some("hiroute-example"));
        assert_eq!(values.sonnet, native.sonnet);
        assert_eq!(values.haiku, None);
        if let AgentModelSelectionV2::ClaudeLauncher {
            preset_mappings, ..
        } = &mut selection
        {
            preset_mappings.sonnet = mapped.clone();
            preset_mappings.haiku = mapped;
        }
        let values = grant.claude_preset_values(&selection, &native).unwrap();
        assert_eq!(values.opus, values.sonnet);
        assert_eq!(values.sonnet, values.haiku);
        assert_eq!(grant.routes.len(), 1);
    }

    #[test]
    fn grant_digest_binds_protocol_route_name_and_plan_provenance() {
        let original = grant();
        let mut changed = original.clone();
        changed.protocol = AgentIngressProtocolV1::Messages;
        assert!(changed.validate().is_err());
        let mut changed = original.clone();
        if let AgentModelRouteV2::Plan { revision, .. } =
            changed.routes.values_mut().next().unwrap()
        {
            *revision += 1;
        }
        assert!(changed.validate().is_err());
        let mut changed = original;
        let route = changed.routes.remove("hiroute-example").unwrap();
        changed.routes.insert("another-name".into(), route);
        assert!(changed.validate().is_err());
    }

    #[test]
    fn journal_selection_cannot_expand_plan_scope_or_switch_protocol() {
        let original = grant();
        let mut changed = selection(AgentModelDefaultSelectionV2::PreserveNative);
        if let AgentModelSelectionV2::CodexDefault {
            allowed_plan_ids, ..
        } = &mut changed
        {
            allowed_plan_ids.insert(AgentPlanId::parse("plan/other").unwrap());
        }
        assert!(original.validate_selection(&changed).is_err());
        let wrong_protocol =
            AgentModelGrantV2::seal(AgentIngressProtocolV1::Messages, original.routes).unwrap();
        assert!(
            wrong_protocol
                .validate_selection(&selection(AgentModelDefaultSelectionV2::PreserveNative))
                .is_err()
        );
    }
}
