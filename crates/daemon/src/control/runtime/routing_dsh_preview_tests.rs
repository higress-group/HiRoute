use super::dsh_capability_preview;
use hiroute_application_api::{
    DshCapabilityUnavailableReasonV1 as Reason, DshClientCapabilityPreviewV1 as Preview,
};
use hiroute_domain::{
    AgentIngressProtocolV1, AgentKindV1, AgentModelGrantV2, GatewayCriticalFactV1,
    GatewayNativeReasoningRenderV1, GatewayPublicationV1, UpstreamProtocol,
};

#[test]
fn preview_uses_the_frozen_plan_budget_used_by_dsh_native_mapping() {
    let publication: GatewayPublicationV1 = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../e2e/product/fixtures/routing/current-publication.v3.json"
    )))
    .unwrap();
    let publication = publication.into_current().unwrap();
    let selected = publication
        .published_agent_plans()
        .unwrap()
        .into_iter()
        .find(|plan| {
            plan.active
                && plan
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let grant = AgentModelGrantV2::from_plan_ids(
        AgentIngressProtocolV1::Responses,
        [selected.agent_plan_id.clone()].into(),
        &publication,
    )
    .unwrap();
    let models = super::super::native_additional_model::additional_models_for_grant(
        AgentKindV1::DeepseekHarness,
        &publication,
        &grant,
    )
    .unwrap();
    assert_eq!(models.len(), 1);
    let plan = publication
        .plans
        .iter()
        .find(|plan| plan.agent_plan_id() == &selected.agent_plan_id)
        .unwrap()
        .clone()
        .into_current()
        .unwrap();
    assert_eq!(
        dsh_capability_preview(&plan),
        Preview::Available {
            protocol: AgentIngressProtocolV1::Responses,
            context_window: models[0].context_window_tokens,
            max_output_tokens: models[0].max_output_tokens
        }
    );

    let mut converted = plan.clone();
    for candidate in std::sync::Arc::make_mut(&mut converted.body)
        .materialized
        .attempt_owned
        .groups
        .iter_mut()
        .flat_map(|group| &mut group.candidates)
    {
        for profile in &mut candidate.protocol_profiles {
            if profile.ingress_protocol == UpstreamProtocol::Responses {
                profile.capability.upstream_protocol = UpstreamProtocol::Messages;
                // Keep the selected native reasoning rendering consistent with
                // the converted upstream protocol when exercising this seam.
                for reasoning in &mut profile.capability.reasoning_profiles {
                    match &mut reasoning.render {
                        GatewayNativeReasoningRenderV1::ExactFields { protocol, .. }
                        | GatewayNativeReasoningRenderV1::ExactBudget { protocol, .. } => {
                            *protocol = UpstreamProtocol::Messages;
                        }
                        GatewayNativeReasoningRenderV1::NoControlParameter => {}
                    }
                }
            }
        }
    }
    assert!(matches!(
        dsh_capability_preview(&converted),
        Preview::Available { .. }
    ));

    let mut missing_ingress = plan.clone();
    std::sync::Arc::make_mut(&mut missing_ingress.body)
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[0]
        .protocol_profiles
        .retain(|profile| profile.ingress_protocol != UpstreamProtocol::Responses);
    assert_eq!(
        dsh_capability_preview(&missing_ingress),
        Preview::Unavailable {
            reason: Reason::ResponsesUnavailable
        }
    );

    let mut missing_budget = plan;
    std::sync::Arc::make_mut(&mut missing_budget.body)
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[0]
        .protocol_profiles[0]
        .capability
        .context
        .max_output_tokens = GatewayCriticalFactV1::Unknown;
    assert_eq!(
        dsh_capability_preview(&missing_budget),
        Preview::Unavailable {
            reason: Reason::BudgetUnavailable
        }
    );
}
