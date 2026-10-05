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
        #[serde(with = "crate::binding_codec")]
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub route_protocols: BTreeMap<String, AgentIngressProtocolV1>,
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
                AgentModelSelectionV2::CodexDefault { .. }
                    | AgentModelSelectionV2::QoderAdditional { .. }
                    | AgentModelSelectionV2::PiAdditional { .. },
                AgentIngressProtocolV1::Responses
            ) | (
                AgentModelSelectionV2::ClaudeLauncher { .. },
                AgentIngressProtocolV1::Messages
            )
        ) {
            return Err(AgentConnectionError::InvalidGrant);
        }
        let mut routes = BTreeMap::new();
        let mut route_protocols = BTreeMap::new();
        for id in selection.allowed_plan_ids() {
            let selected = selection.plan_protocol(&id, protocol);
            let plan_routes = Self::plan_routes(selected, [id].into(), publication)?;
            for name in plan_routes.keys() {
                if selected != protocol {
                    route_protocols.insert(name.clone(), selected);
                }
            }
            routes.extend(plan_routes);
        }
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
        Self::seal_routes(protocol, routes, route_protocols)
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
        Self::seal_routes(protocol, routes, BTreeMap::new())
    }

    pub fn seal_routes(
        protocol: AgentIngressProtocolV1,
        mut routes: BTreeMap<String, AgentModelRouteV2>,
        route_protocols: BTreeMap<String, AgentIngressProtocolV1>,
    ) -> Result<Self, AgentConnectionError> {
        for route in routes.values_mut() {
            if let AgentModelRouteV2::Fixed { binding, .. } = route {
                **binding = crate::StoredCandidateV1::freeze(binding)
                    .and_then(|c| c.build())
                    .map_err(|_| AgentConnectionError::InvalidGrant)?;
            }
        }
        let digest = Self::route_digest(protocol, &routes, &route_protocols)?;
        let grant = Self {
            protocol,
            routes,
            route_protocols,
            digest,
        };
        grant.validate()?;
        Ok(grant)
    }

    pub fn protocol_for(&self, name: &str) -> AgentIngressProtocolV1 {
        self.route_protocols
            .get(name)
            .copied()
            .unwrap_or(self.protocol)
    }
    fn route_digest(
        protocol: AgentIngressProtocolV1,
        routes: &BTreeMap<String, AgentModelRouteV2>,
        protocols: &BTreeMap<String, AgentIngressProtocolV1>,
    ) -> Result<CanonicalDigest, AgentConnectionError> {
        if protocols.is_empty() {
            CanonicalDigest::of(&("hiroute.agent-model-grant/v3", protocol, routes))
        } else {
            CanonicalDigest::of(&("hiroute.agent-model-grant/v3", protocol, routes, protocols))
        }
        .map_err(|_| AgentConnectionError::Encoding)
    }

    pub fn validate(&self) -> Result<(), AgentConnectionError> {
        if self.routes.is_empty()
            || self.route_protocols.iter().any(|(name, protocol)| {
                *protocol == self.protocol
                    || !matches!(self.routes.get(name), Some(AgentModelRouteV2::Plan { .. }))
            })
        {
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
        if Self::route_digest(self.protocol, &self.routes, &self.route_protocols)? != self.digest {
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
                AgentModelSelectionV2::CodexDefault { .. }
                    | AgentModelSelectionV2::QoderAdditional { .. }
                    | AgentModelSelectionV2::PiAdditional { .. },
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
        if self.routes.iter().any(|(name, route)| match route {
            AgentModelRouteV2::Plan { plan_id, .. } => self.protocol_for(name) != selection.plan_protocol(plan_id, self.protocol),
            _ => false,
        }) || plans != selection.allowed_plan_ids()
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
    fn qoder_additional_routes_authorize_only_selected_plans_without_native_defaults() {
        let selection = AgentModelSelectionV2::QoderAdditional {
            plan_protocols: Default::default(),
            allowed_plan_ids: [AgentPlanId::parse("plan/example").unwrap()].into(),
        };
        let grant = grant();
        assert!(grant.validate_selection(&selection).is_ok());
        assert!(grant.codex_default_override(&selection).is_err());
        assert!(selection.fixed_models().is_empty());
        let other = AgentModelSelectionV2::QoderAdditional {
            plan_protocols: Default::default(),
            allowed_plan_ids: [AgentPlanId::parse("plan/other").unwrap()].into(),
        };
        assert!(grant.validate_selection(&other).is_err());
        let wrong_protocol =
            AgentModelGrantV2::seal(AgentIngressProtocolV1::Messages, grant.routes).unwrap();
        assert!(wrong_protocol.validate_selection(&selection).is_err());
        assert!(
            AgentModelSelectionV2::QoderAdditional {
                plan_protocols: Default::default(),
                allowed_plan_ids: Default::default(),
            }
            .validate()
            .is_err()
        );
        for forbidden in ["default_selection", "fixed_models", "native_model_mode"] {
            let mut wire = serde_json::to_value(&selection).unwrap();
            wire.as_object_mut()
                .unwrap()
                .insert(forbidden.into(), serde_json::Value::Null);
            assert!(serde_json::from_value::<AgentModelSelectionV2>(wire).is_err());
        }
    }

    #[test]
    fn explicit_plan_protocol_is_sealed_and_cannot_expand_other_clients() {
        let base = grant();
        let id = AgentPlanId::parse("plan/example").unwrap();
        let selected = AgentModelSelectionV2::PiAdditional {
            allowed_plan_ids: [id.clone()].into(),
            plan_protocols: [(id.clone(), AgentIngressProtocolV1::Messages)].into(),
        };
        assert!(base.validate_selection(&selected).is_err());
        let messages = AgentModelGrantV2::seal_routes(
            base.protocol,
            base.routes.clone(),
            [("hiroute-example".into(), AgentIngressProtocolV1::Messages)].into(),
        )
        .unwrap();
        assert_ne!(messages.digest, base.digest);
        assert!(messages.validate_selection(&selected).is_ok());
        assert_eq!(
            messages.protocol_for("hiroute-example"),
            AgentIngressProtocolV1::Messages
        );
        let legacy = AgentModelSelectionV2::PiAdditional {
            allowed_plan_ids: [id].into(),
            plan_protocols: BTreeMap::new(),
        };
        assert!(messages.validate_selection(&legacy).is_err());
        assert!(
            AgentModelGrantV2::seal_routes(
                base.protocol,
                base.routes,
                [("ungranted".into(), AgentIngressProtocolV1::Messages)].into()
            )
            .is_err()
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
