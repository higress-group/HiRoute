//! Exact catalog controls replace incoming controls before native request serialization.
use super::*;
use crate::server::core_runtime::model_ir::RequestedReasoningDisposition;
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, CriticalFact, NativeReasoningFieldAssignment, NativeReasoningRender,
    NativeReasoningValue, ReasoningControlKind, fixed_reasoning,
};
use crate::server::request_plan::IngressProtocol;
use serde_json::json;
#[test]
fn messages_adaptive_effort_overwrites_caller_disabled_thinking_and_effort() {
    let request = decode_ingress_request(
        IngressProtocol::Messages,
        &json!({
            "model":"alias", "max_tokens":2048, "messages":[{"role":"user","content":"hello"}],
            "thinking":{"type":"disabled"}, "output_config":{"effort":"low"}
        }),
    )
    .unwrap();
    let mut reasoning = fixed_reasoning("high");
    reasoning.control_kind = ReasoningControlKind::Discrete;
    reasoning.render = NativeReasoningRender::ExactFields {
        protocol: IngressProtocol::Messages,
        fields: vec![
            NativeReasoningFieldAssignment {
                path: vec!["output_config".into(), "effort".into()],
                value: NativeReasoningValue::String("high".into()),
            },
            NativeReasoningFieldAssignment {
                path: vec!["thinking".into(), "type".into()],
                value: NativeReasoningValue::String("adaptive".into()),
            },
        ],
    };
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        "physical",
        reasoning,
    );
    let native = project_candidate_request(&request, &profile).unwrap();
    assert_eq!(native.body["thinking"], json!({"type":"adaptive"}));
    assert_eq!(native.body["output_config"], json!({"effort":"high"}));
}

fn context_request() -> serde_json::Value {
    json!({"model":"alias", "max_tokens":2048,
        "messages":[{"role":"user","content":"hello"}],
        "thinking":{"type":"enabled","budget_tokens":1024},
        "output_config":{"format":{"type":"json_schema","schema":{"type":"object","properties":{"ok":{"type":"boolean"}}}}}})
}

fn context_profile(mode: &str) -> CandidateProtocolProfile {
    let mut reasoning = fixed_reasoning(mode);
    reasoning.control_kind = ReasoningControlKind::Discrete;
    let mut fields = vec![NativeReasoningFieldAssignment {
        path: vec!["thinking".into(), "type".into()],
        value: NativeReasoningValue::String(mode.into()),
    }];
    if mode == "enabled" {
        fields.push(NativeReasoningFieldAssignment {
            path: vec!["thinking".into(), "budget_tokens".into()],
            value: NativeReasoningValue::U64(1024),
        });
    }
    reasoning.render = NativeReasoningRender::ExactFields {
        protocol: IngressProtocol::Messages,
        fields,
    };
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        "physical",
        reasoning,
    );
    // Manual thinking must fit strictly below the effective output limit.
    profile.capability.context.max_output_tokens = CriticalFact::Exact(2048);
    profile
}

#[test]
fn clear_thinking_strategy_follows_selected_plan_mode() {
    let mut body = context_request();
    body["context_management"] = json!({"edits":[{"type":"clear_thinking_20251015","keep":"all"}]});
    let mut request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
    request.requested_reasoning.disposition = RequestedReasoningDisposition::OverriddenByAgentPlan;
    for mode in ["default", "disabled", "enabled", "adaptive"] {
        let profile = if mode == "default" {
            CandidateProtocolProfile::exact_portable_path(
                IngressProtocol::Messages,
                IngressProtocol::Messages,
                "physical",
                fixed_reasoning("default"),
            )
        } else {
            context_profile(mode)
        };
        let native = project_candidate_request(&request, &profile).unwrap();
        if matches!(mode, "enabled" | "adaptive") {
            assert_eq!(
                native.body["context_management"],
                body["context_management"]
            );
        } else {
            assert!(
                native.body.get("context_management").is_none(),
                "{mode}: {}",
                native.body
            );
        }
        assert_eq!(
            native.body["output_config"]["format"],
            body["output_config"]["format"]
        );
    }
    // Every candidate starts from the same immutable caller payload.
    assert_eq!(
        request.native_body.as_ref().unwrap()["context_management"],
        body["context_management"]
    );
}

#[test]
fn clear_thinking_removal_preserves_other_context_controls_and_fixed_payload() {
    let strategy = json!({"type":"clear_thinking_20251015","keep":"all"});
    let other = json!({"type":"clear_tool_uses_20250919","keep":{"type":"tool_uses","value":3}});
    for (context, expected) in [
        (
            json!({"edits":[strategy.clone(),other.clone()],"future_control":true}),
            json!({"edits":[other],"future_control":true}),
        ),
        (
            json!({"edits":[strategy],"future_control":true}),
            json!({"future_control":true}),
        ),
        (
            json!({"edits":[{"type":"future_strategy"}]}),
            json!({"edits":[{"type":"future_strategy"}]}),
        ),
        (json!({"edits":[]}), json!({"edits":[]})),
    ] {
        let mut body = context_request();
        body["context_management"] = context.clone();
        let mut request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
        request.requested_reasoning.disposition =
            RequestedReasoningDisposition::OverriddenByAgentPlan;
        let native = project_candidate_request(&request, &context_profile("disabled")).unwrap();
        assert_eq!(native.body["context_management"], expected);
        request.requested_reasoning.disposition =
            RequestedReasoningDisposition::AppliedToFixedBinding;
        let fixed = project_candidate_request(&request, &context_profile("disabled")).unwrap();
        assert_eq!(fixed.body["context_management"], context);
        assert_eq!(fixed.body["thinking"], body["thinking"]);
    }
}
