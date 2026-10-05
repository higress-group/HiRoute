//! Body-free native verification expectations from the actual Gateway compiler.
use hiroute_domain::{
    AgentPlanId, CanonicalDigest, FrozenExecutionTrustV1, IngressProtocolV1, ModelRequestRouteV2,
    SelectorSourceV1,
};

use super::{CompiledGrantRoute, GatewayPublicationSnapshotV3, PublicationInstallError};
use crate::server::core_runtime::profiles::PlannerRouteIdentityV2;
use crate::server::request_plan::IngressProtocol;

impl GatewayPublicationSnapshotV3 {
    /// Derive the provenance a native call must prove, without publishing, authenticating,
    /// acquiring a credential or calling a Provider. Plan/grant projection digests describe
    /// a different object; receipts freeze the compiler's executable alias identity.
    pub fn native_agent_execution_trust(
        &self,
        grant_id: &str,
        served_model_id: &str,
    ) -> Result<FrozenExecutionTrustV1, PublicationInstallError> {
        let compiled = super::compiler::compile(self)?;
        let invalid = || PublicationInstallError::InvalidDurableState;
        let grant = compiled
            .grants
            .iter()
            .find(|grant| grant.grant_id.as_ref() == grant_id)
            .ok_or_else(invalid)?;
        let projected = grant.routes.get(served_model_id).ok_or_else(invalid)?;
        let (agent_plan_id, route, plan_display_name) = match projected {
            CompiledGrantRoute::Plan { alias } => {
                let alias = compiled.aliases.get(alias).ok_or_else(invalid)?;
                let PlannerRouteIdentityV2::Plan { plan_id, .. } =
                    &alias.execution.planner_policy.identity
                else {
                    return Err(invalid());
                };
                (
                    Some(AgentPlanId::parse(plan_id).map_err(|_| invalid())?),
                    ModelRequestRouteV2::Plan {
                        revision: alias.agent_plan_revision,
                        semantic_digest: CanonicalDigest::parse(
                            alias.agent_plan_semantic_digest.as_ref(),
                        )
                        .map_err(|_| invalid())?,
                    },
                    alias.plan_display_name.as_deref().map(str::to_owned),
                )
            }
            CompiledGrantRoute::Fixed { binding_digest, .. } => (
                None,
                ModelRequestRouteV2::Fixed {
                    binding_digest: binding_digest.clone(),
                },
                None,
            ),
        };
        let ingress_protocol = match grant.protocol_for(served_model_id) {
            IngressProtocol::Responses => IngressProtocolV1::Responses,
            IngressProtocol::Messages => IngressProtocolV1::Messages,
            IngressProtocol::ChatCompletions => IngressProtocolV1::ChatCompletions,
        };
        let trust = FrozenExecutionTrustV1 {
            authority_id: self.authority_id.clone(),
            authority_epoch: self.authority_epoch,
            served_model_id: served_model_id.to_owned(),
            selector_source: SelectorSourceV1::TrustedModelAlias,
            agent_plan_id,
            route,
            plan_display_name,
            gateway_publication_revision: self.publication_revision.to_string(),
            gateway_publication_digest: CanonicalDigest::parse(&self.payload_digest)
                .map_err(|_| invalid())?,
            grant_id: grant.grant_id.to_string(),
            grant_generation: grant.generation,
            ingress_protocol,
        };
        trust.validate().map_err(|_| invalid())?;
        Ok(trust)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::server::dispatch::GatewayRequestAuthority;
    use crate::server::publication::tests::{TestDirectory, publish, snapshot};
    use crate::server::publication::{
        AliasCostPolicyV1, AliasGroupIdV1, AliasModelGroupV1, AliasRequestOwnedRouteV1,
        AliasRoutingV1, GatewayPublicationInstaller, ModelRouteV2, exact_test_candidate,
    };

    #[test]
    fn native_agent_execution_trust_matches_authorized_plan_and_fixed_receipts() {
        let mut snapshot = snapshot(7, "native-live-check");
        for (index, alias) in snapshot.aliases.iter_mut().enumerate() {
            alias.routing = Some(AliasRoutingV1 {
                agent_plan_id: format!("legacy/{}", alias.served_model_id),
                plan_display_name: Some(format!("{} display", alias.served_model_id)),
                request_owned: AliasRequestOwnedRouteV1::Ordered {
                    cost_policy: AliasCostPolicyV1::ApiEquivalent,
                    ordered_groups: vec![AliasGroupIdV1::Custom],
                },
                groups: vec![AliasModelGroupV1 {
                    group_id: AliasGroupIdV1::Custom,
                    candidate_local_ids: vec![index as u32 + 1],
                }],
            });
        }
        let binding = exact_test_candidate(3, IngressProtocol::Responses, None);
        snapshot.grants[0].routes.insert(
            "native".into(),
            ModelRouteV2::Fixed {
                binding_digest: CanonicalDigest::of(&binding).unwrap(),
                binding: Box::new(binding),
                overall_timeout_ms: 1000,
                max_attempts: 1,
            },
        );
        snapshot.payload_digest = snapshot.canonical_digest().unwrap();
        let directory = TestDirectory::new();
        let path = directory.path().join("publication.json");
        let installer = Arc::new(GatewayPublicationInstaller::open(&path).unwrap());
        let cases = [
            (
                "grant-alpha",
                "alpha",
                "token-alpha",
                IngressProtocol::Responses,
            ),
            (
                "grant-beta",
                "beta",
                "token-beta",
                IngressProtocol::Messages,
            ),
            (
                "grant-alpha",
                "native",
                "token-alpha",
                IngressProtocol::Responses,
            ),
        ];
        let expected: Vec<_> = cases
            .iter()
            .map(|(grant, model, _, _)| {
                snapshot.native_agent_execution_trust(grant, model).unwrap()
            })
            .collect();
        assert!(installer.active().is_none());
        assert!(!path.exists(), "computing expected trust must not publish");
        publish(&installer, snapshot.clone());
        let authority = GatewayRequestAuthority::new(installer);
        for ((grant, model, token, protocol), expected) in cases.into_iter().zip(expected) {
            let body = serde_json::to_vec(&serde_json::json!({"model": model})).unwrap();
            let authorized = authority
                .authorize_bytes(
                    protocol,
                    Some(&format!("Bearer {token}")),
                    &body,
                    std::time::Instant::now(),
                )
                .unwrap();
            let receipt = authorized.receipt();
            let agent_plan_id = match &authorized.planner_policy().identity {
                PlannerRouteIdentityV2::Plan { plan_id, .. } => {
                    Some(AgentPlanId::parse(plan_id).unwrap())
                }
                PlannerRouteIdentityV2::Fixed { .. } => None,
            };
            let actual = FrozenExecutionTrustV1 {
                authority_id: receipt.authority_id.to_string(),
                authority_epoch: receipt.authority_epoch,
                served_model_id: receipt.served_model_id.to_string(),
                selector_source: SelectorSourceV1::TrustedModelAlias,
                agent_plan_id,
                route: receipt.route.clone(),
                plan_display_name: receipt.plan_display_name.as_deref().map(str::to_owned),
                gateway_publication_revision: receipt.publication_revision.to_string(),
                gateway_publication_digest: CanonicalDigest::parse(
                    receipt.publication_digest.as_ref(),
                )
                .unwrap(),
                grant_id: receipt.grant_id.to_string(),
                grant_generation: receipt.grant_generation,
                ingress_protocol: match receipt.ingress_protocol {
                    IngressProtocol::Responses => IngressProtocolV1::Responses,
                    IngressProtocol::Messages => IngressProtocolV1::Messages,
                    IngressProtocol::ChatCompletions => IngressProtocolV1::ChatCompletions,
                },
            };
            assert_eq!(expected, actual, "native route {model}");
            if let ModelRouteV2::Plan {
                semantic_digest, ..
            } = &snapshot
                .grants
                .iter()
                .find(|value| value.grant_id == grant)
                .unwrap()
                .routes[model]
            {
                let ModelRequestRouteV2::Plan {
                    semantic_digest: compiled_digest,
                    ..
                } = actual.route
                else {
                    panic!("expected Plan execution receipt");
                };
                assert_ne!(semantic_digest, &compiled_digest);
            }
        }
    }

    #[test]
    fn native_agent_execution_trust_rejects_invalid_snapshot_or_ungranted_identity() {
        let mut snapshot = snapshot(1, "native-live-check");
        for (grant, model) in [
            ("unknown-grant", "alpha"),
            ("grant-alpha", "beta"),
            ("grant-alpha", "missing"),
        ] {
            assert!(snapshot.native_agent_execution_trust(grant, model).is_err());
        }
        snapshot.aliases[0].purpose = "changed after sealing".into();
        assert!(
            snapshot
                .native_agent_execution_trust("grant-alpha", "alpha")
                .is_err()
        );
    }
}
