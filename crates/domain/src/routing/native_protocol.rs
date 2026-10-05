//! Advice only: selecting a protocol remains an explicit user decision.
use crate::{AgentIngressProtocolV1, MaterializedAgentPlanV1, UpstreamProtocol};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProtocolAdviceV1 {
    pub supported: Vec<AgentIngressProtocolV1>,
    pub native: Vec<AgentIngressProtocolV1>,
}

pub fn agent_protocol_advice(plan: &MaterializedAgentPlanV1) -> AgentProtocolAdviceV1 {
    let candidates = plan
        .attempt_owned
        .groups
        .iter()
        .flat_map(|g| &g.candidates)
        .collect::<Vec<_>>();
    let mut advice = AgentProtocolAdviceV1::default();
    if candidates.is_empty() {
        return advice;
    }
    for (ingress, upstream) in [
        (
            AgentIngressProtocolV1::Responses,
            UpstreamProtocol::Responses,
        ),
        (AgentIngressProtocolV1::Messages, UpstreamProtocol::Messages),
    ] {
        if candidates.iter().all(|c| {
            c.protocol_profiles
                .iter()
                .any(|p| p.ingress_protocol == upstream)
        }) {
            advice.supported.push(ingress);
        }
        if candidates.iter().all(|c| {
            c.protocol_profiles.iter().any(|p| {
                p.ingress_protocol == upstream && p.capability.upstream_protocol == upstream
            })
        }) {
            advice.native.push(ingress);
        }
    }
    advice
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recommendation_requires_every_candidate_and_fallback_to_be_native() {
        let publication: crate::GatewayPublicationV1 = serde_json::from_slice(include_bytes!(
            "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
        ))
        .unwrap();
        let mut plan = publication.plans[0].body.materialized.clone();
        for c in plan
            .attempt_owned
            .groups
            .iter_mut()
            .flat_map(|g| &mut g.candidates)
        {
            c.protocol_profiles
                .retain(|p| p.ingress_protocol == UpstreamProtocol::Responses);
            for p in &mut c.protocol_profiles {
                p.capability.upstream_protocol = UpstreamProtocol::Responses;
            }
        }
        assert_eq!(
            agent_protocol_advice(&plan).native,
            vec![AgentIngressProtocolV1::Responses]
        );
        let mut fallback = plan.attempt_owned.groups[0].clone();
        for c in &mut fallback.candidates {
            for p in &mut c.protocol_profiles {
                p.capability.upstream_protocol = UpstreamProtocol::Messages;
            }
        }
        plan.attempt_owned.groups.push(fallback);
        let advice = agent_protocol_advice(&plan);
        assert_eq!(advice.supported, vec![AgentIngressProtocolV1::Responses]);
        assert!(
            advice.native.is_empty(),
            "conversion support is not native support"
        );
        plan.attempt_owned.groups.clear();
        assert_eq!(
            agent_protocol_advice(&plan),
            AgentProtocolAdviceV1::default()
        );
    }
}
