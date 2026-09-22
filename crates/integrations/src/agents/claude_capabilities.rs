//! The minimum request shape emitted by an ordinary Claude Code conversation.
use hiroute_application_api::ClaudeClientCapabilityPreviewV1;
use hiroute_domain::{CompiledAgentPlanV1, UpstreamProtocol};
use hiroute_gateway::server::{
    core_runtime::profiles::CandidateProtocolProfile, request_plan::IngressProtocol,
};

pub fn claude_plan_capability_preview(
    plan: &CompiledAgentPlanV1,
) -> ClaudeClientCapabilityPreviewV1 {
    let unavailable = |reason: &str| ClaudeClientCapabilityPreviewV1::Unavailable {
        reason: reason.into(),
    };
    if plan.validate().is_err() {
        return unavailable("invalid_compiled_plan");
    }
    let mut requirements = super::codex_catalog_plan::requirements(false);
    requirements.ingress_protocol = IngressProtocol::Messages;
    requirements.mid_conversation_instructions = false;
    for candidate in plan
        .body
        .materialized
        .attempt_owned
        .groups
        .iter()
        .flat_map(|group| &group.candidates)
    {
        let Some(profile) = candidate
            .protocol_profiles
            .iter()
            .find(|profile| profile.ingress_protocol == UpstreamProtocol::Messages)
        else {
            return unavailable("messages_protocol");
        };
        let Ok(value) = serde_json::to_value(profile) else {
            return unavailable("request_capabilities");
        };
        let Ok(profile) = serde_json::from_value::<CandidateProtocolProfile>(value) else {
            return unavailable("request_capabilities");
        };
        if profile.validate(&requirements).is_err() {
            return unavailable("request_capabilities");
        }
    }
    let Ok(plan_window) = plan.body.materialized.context_window_tokens() else {
        return unavailable("context_window");
    };
    let Some(context_window) = hiroute_domain::claude_context_window(plan_window) else {
        return unavailable("context_window_below_minimum");
    };
    ClaudeClientCapabilityPreviewV1::Available {
        context_window,
        plan_window,
    }
}
