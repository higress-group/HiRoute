//! Exact catalog controls replace incoming controls before native request serialization.
use super::*;
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, NativeReasoningFieldAssignment, NativeReasoningRender,
    NativeReasoningValue, ReasoningControlKind, fixed_reasoning,
};
use crate::server::request_plan::IngressProtocol;
use serde_json::json;

fn display_request() -> serde_json::Value {
    json!({"model":"alias", "max_tokens":2048,
        "messages":[{"role":"user","content":"hello"}],
        "thinking":{"type":"enabled","budget_tokens":1024,"display":"omitted"},
        "output_config":{"format":{"type":"json_schema","schema":{"type":"object","properties":{"ok":{"type":"boolean"}}}}}})
}

#[test]
fn thinking_display_plan_default_never_emits_display_only_object() {
    let body = display_request();
    let request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        "physical",
        fixed_reasoning("default"),
    );
    let native = project_candidate_request(&request, &profile).unwrap();
    assert!(native.body.get("thinking").is_none(), "{}", native.body);
    assert_eq!(
        native.body["output_config"]["format"],
        body["output_config"]["format"]
    );
}

#[test]
fn thinking_display_omitted_can_cross_protocol_without_disabling_reasoning() {
    let request = decode_ingress_request(IngressProtocol::Messages, &display_request()).unwrap();
    for upstream in [IngressProtocol::ChatCompletions, IngressProtocol::Responses] {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Messages,
            upstream,
            "physical",
            fixed_reasoning("default"),
        );
        let native = project_candidate_request(&request, &profile).unwrap();
        assert!(native.body.get("thinking").is_none());
        assert!(native.body.get("reasoning_effort").is_none());
    }
}
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

fn display_profile(mode: &str) -> CandidateProtocolProfile {
    let mut reasoning = fixed_reasoning(mode);
    reasoning.control_kind = ReasoningControlKind::Discrete;
    let mut fields = vec![NativeReasoningFieldAssignment {
        path: vec!["thinking".into(), "type".into()],
        value: NativeReasoningValue::String(mode.into()),
    }];
    if mode == "enabled" {
        fields.push(NativeReasoningFieldAssignment {
            path: vec!["thinking".into(), "budget_tokens".into()],
            value: NativeReasoningValue::U64(128),
        });
    }
    reasoning.render = NativeReasoningRender::ExactFields {
        protocol: IngressProtocol::Messages,
        fields,
    };
    CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        "physical",
        reasoning,
    )
}

#[test]
fn thinking_display_plan_modes_use_only_plan_budget_and_preserve_format() {
    let body = display_request();
    let request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
    for (mode, expected) in [
        ("disabled", json!({"type":"disabled"})),
        ("adaptive", json!({"type":"adaptive","display":"omitted"})),
        (
            "enabled",
            json!({"type":"enabled","budget_tokens":128,"display":"omitted"}),
        ),
    ] {
        let native = project_candidate_request(&request, &display_profile(mode)).unwrap();
        assert_eq!(native.body["thinking"], expected);
        assert_eq!(
            native.body["output_config"]["format"],
            body["output_config"]["format"]
        );
    }
    assert_eq!(request.native_body.as_ref().unwrap(), &body);
}

#[test]
fn thinking_display_fixed_preserves_native_controls_and_unknown_values_stay_bounded() {
    use crate::server::core_runtime::model_ir::RequestedReasoningDisposition;
    for display in [
        json!("omitted"),
        json!("summarized"),
        json!("future-mode"),
        json!(42),
        json!(null),
    ] {
        let mut body = display_request();
        body["thinking"]["display"] = display.clone();
        let mut request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
        assert_eq!(request.native_only, display != "omitted");
        request.requested_reasoning.disposition =
            RequestedReasoningDisposition::AppliedToFixedBinding;
        let native = project_candidate_request(&request, &display_profile("disabled")).unwrap();
        assert_eq!(native.body["thinking"], body["thinking"]);
        request.requested_reasoning.disposition =
            RequestedReasoningDisposition::OverriddenByAgentPlan;
        if display != "omitted" {
            let native = project_candidate_request(&request, &display_profile("adaptive")).unwrap();
            assert_eq!(native.body["thinking"]["display"], display);
            assert!(project_candidate_request(&request, &display_profile("disabled")).is_err());
            for upstream in [IngressProtocol::ChatCompletions, IngressProtocol::Responses] {
                let profile = CandidateProtocolProfile::exact_portable_path(
                    IngressProtocol::Messages,
                    upstream,
                    "physical",
                    fixed_reasoning("default"),
                );
                assert!(project_candidate_request(&request, &profile).is_err());
            }
        }
    }
}

#[test]
fn thinking_display_does_not_allow_unrelated_extensions_to_cross_protocol() {
    let mut body = display_request();
    body["thinking"]["future_control"] = json!(true);
    let request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
    assert!(request.native_only);
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("default"),
    );
    assert!(project_candidate_request(&request, &profile).is_err());
}

#[test]
fn thinking_display_policy_survives_replay_externalization() {
    use crate::{
        content_ref::externalize_model_request,
        replay::{ReplayConfig, ReplayManager},
    };
    use hiroute_gateway_core::runtime::body::BudgetTree;
    let root = std::env::temp_dir().join(format!(
        "hiroute-thinking-display-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
        .unwrap()
        .stream(1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget).unwrap();
    let mut request =
        decode_ingress_request(IngressProtocol::Messages, &display_request()).unwrap();
    externalize_model_request(&mut request, &store, 0).unwrap();
    assert!(request.requested_reasoning.messages_omit_thinking);
    assert_ne!(
        request.native_body.as_ref().unwrap()["thinking"]["display"],
        "omitted"
    );
    assert!(project_candidate_request_template(&request, &display_profile("disabled")).is_ok());
    let serialized = serde_json::to_value(&request).unwrap();
    assert!(
        serialized["requested_reasoning"]
            .get("messages_omit_thinking")
            .is_none()
    );
    drop(request);
    drop(store);
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}
