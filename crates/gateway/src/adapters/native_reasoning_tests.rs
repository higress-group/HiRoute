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

fn project_reasoning(
    protocol: IngressProtocol,
    reasoning: crate::server::core_runtime::profiles::ReasoningProfileCapability,
    max_tokens: u64,
) -> Result<PreparedNativeRequest, ProtocolAdapterError> {
    project_reasoning_with_cap(protocol, reasoning, max_tokens, max_tokens)
}

fn project_reasoning_with_cap(
    protocol: IngressProtocol,
    reasoning: crate::server::core_runtime::profiles::ReasoningProfileCapability,
    max_tokens: u64,
    configured_cap: u64,
) -> Result<PreparedNativeRequest, ProtocolAdapterError> {
    let body = match protocol {
        IngressProtocol::Responses => {
            json!({"model":"alias", "input":"hello", "reasoning":{"effort":"high", "summary":"auto"}})
        }
        IngressProtocol::Messages => {
            json!({"model":"alias", "max_tokens":max_tokens, "messages":[{"role":"user","content":"hello"}], "thinking":{"type":"enabled","budget_tokens":4096}})
        }
        IngressProtocol::ChatCompletions => {
            json!({"model":"alias", "messages":[{"role":"user","content":"hello"}]})
        }
    };
    let request = decode_ingress_request(protocol, &body).unwrap();
    let mut profile =
        CandidateProtocolProfile::exact_portable_path(protocol, protocol, "physical", reasoning);
    profile.capability.context.max_output_tokens = CriticalFact::Exact(configured_cap);
    profile.capability.context.max_total_tokens =
        CriticalFact::Exact(Some(1024 * 1024 + configured_cap));
    project_candidate_request(&request, &profile)
}

#[test]
fn released_toggle_profiles_render_standard_controls_without_changing_frozen_bytes() {
    // Frozen output of the 0.2.0 producer d625e653 (enable_thinking toggle).
    // Do not build this input through the current daemon projection.
    let released = r#"{"profile_id":"enabled","control_kind":"toggle","render":{"kind":"exact_fields","protocol":"responses","fields":[{"path":["reasoning","effort"],"value":{"kind":"string","value":"high"}}]},"accounting":"within_output_cap","additional_reservation_tokens":0}"#;
    let enabled: crate::server::core_runtime::profiles::ReasoningProfileCapability =
        serde_json::from_str(released).unwrap();
    let before = serde_json::to_value(&enabled).unwrap();
    let rendered = project_reasoning(IngressProtocol::Responses, enabled.clone(), 2048).unwrap();
    assert_eq!(rendered.body["reasoning"]["effort"], "low");
    assert_eq!(rendered.body["reasoning"]["summary"], "auto");
    assert_eq!(serde_json::to_value(&enabled).unwrap(), before);
    let mut explicit = enabled.clone();
    explicit.profile_id = "high".into();
    explicit.control_kind = ReasoningControlKind::Discrete;
    assert_eq!(
        project_reasoning(IngressProtocol::Responses, explicit, 2048)
            .unwrap()
            .body["reasoning"]["effort"],
        "high"
    );
    for on in [false, true] {
        for protocol in [IngressProtocol::Responses, IngressProtocol::Messages] {
            let mut profile = enabled.clone();
            profile.profile_id = if on { "enabled" } else { "disabled" }.into();
            profile.render = NativeReasoningRender::ExactFields {
                protocol,
                fields: vec![NativeReasoningFieldAssignment {
                    path: if protocol == IngressProtocol::Messages {
                        vec!["thinking".into(), "type".into()]
                    } else {
                        vec!["reasoning".into(), "effort".into()]
                    },
                    value: NativeReasoningValue::String(
                        match (protocol, on) {
                            (IngressProtocol::Messages, true) => "enabled",
                            (IngressProtocol::Messages, false) => "disabled",
                            (_, true) => "low",
                            (_, false) => "none",
                        }
                        .into(),
                    ),
                }],
            };
            let rendered = project_reasoning(protocol, profile.clone(), 2048).unwrap();
            if protocol == IngressProtocol::Messages {
                assert_eq!(
                    rendered.body["thinking"],
                    if on {
                        json!({"type":"enabled", "budget_tokens":1024})
                    } else {
                        json!({"type":"disabled"})
                    }
                );
                if on {
                    assert!(project_reasoning(protocol, profile, 1024).is_err());
                }
            } else {
                assert_eq!(
                    rendered.body["reasoning"]["effort"],
                    if on { "low" } else { "none" }
                );
            }
        }
    }
}

#[test]
fn messages_explicit_budget_is_preserved_and_checked_against_output_cap() {
    for budget in [512, 1024, 2048, 4096] {
        let mut profile = fixed_reasoning(format!("budget-{budget}"));
        profile.control_kind = ReasoningControlKind::Budget;
        profile.render = NativeReasoningRender::ExactBudget {
            protocol: IngressProtocol::Messages,
            fields: vec![NativeReasoningFieldAssignment {
                path: vec!["thinking".into(), "budget_tokens".into()],
                value: NativeReasoningValue::U64(budget),
            }],
            budget_path: vec!["thinking".into(), "budget_tokens".into()],
            selected_tokens: budget,
            min_tokens: 512,
            max_tokens: 4096,
            step_tokens: 512,
        };
        let rendered = project_reasoning(IngressProtocol::Messages, profile, 4096);
        if (1024..4096).contains(&budget) {
            assert_eq!(
                rendered.unwrap().body["thinking"],
                json!({"type":"enabled", "budget_tokens":budget})
            );
        } else {
            assert!(rendered.is_err());
        }
    }
}

#[test]
fn messages_manual_toggle_checks_the_effective_candidate_output_cap() {
    let mut reasoning = fixed_reasoning("enabled");
    reasoning.control_kind = ReasoningControlKind::Toggle;
    reasoning.render = NativeReasoningRender::ExactFields {
        protocol: IngressProtocol::Messages,
        fields: vec![NativeReasoningFieldAssignment {
            path: vec!["thinking".into(), "type".into()],
            value: NativeReasoningValue::String("enabled".into()),
        }],
    };
    assert!(
        project_reasoning_with_cap(IngressProtocol::Messages, reasoning.clone(), 4096, 1024,)
            .is_err()
    );
    let rendered =
        project_reasoning_with_cap(IngressProtocol::Messages, reasoning, 4096, 2048).unwrap();
    assert_eq!(rendered.body["max_tokens"], 2048);
    assert_eq!(rendered.body["thinking"]["budget_tokens"], 1024);
}

#[test]
fn legacy_custom_boolean_profile_keeps_its_exact_assignment() {
    for protocol in [IngressProtocol::Responses, IngressProtocol::Messages] {
        let mut reasoning = fixed_reasoning("custom-boolean-name");
        reasoning.control_kind = ReasoningControlKind::Toggle;
        reasoning.render = NativeReasoningRender::ExactFields {
            protocol,
            fields: vec![NativeReasoningFieldAssignment {
                path: vec!["custom_toggle".into()],
                value: NativeReasoningValue::Bool(true),
            }],
        };
        let rendered = project_reasoning(protocol, reasoning, 4096).unwrap();
        assert_eq!(rendered.body["custom_toggle"], true);
        assert!(rendered.body.get("thinking").is_none());
        if protocol == IngressProtocol::Responses {
            assert_eq!(rendered.body["reasoning"], json!({"summary":"auto"}));
        }
    }
}

#[test]
fn plan_reasoning_removes_legacy_switch_aliases_without_changing_fixed_native_controls() {
    for protocol in [IngressProtocol::Responses, IngressProtocol::Messages] {
        let body = match protocol {
            IngressProtocol::Responses => json!({
                "model":"alias", "input":"hello", "enable_thinking":false,
                "thinking":{"enabled":false}, "reasoning":{"effort":"high","summary":"auto"}
            }),
            IngressProtocol::Messages => json!({
                "model":"alias", "max_tokens":4096,
                "messages":[{"role":"user","content":"hello"}],
                "enable_thinking":false, "thinking":{"enabled":false}
            }),
            IngressProtocol::ChatCompletions => unreachable!(),
        };
        let mut request = decode_ingress_request(protocol, &body).unwrap();
        let mut reasoning = fixed_reasoning("enabled");
        reasoning.control_kind = ReasoningControlKind::Toggle;
        reasoning.render = NativeReasoningRender::ExactFields {
            protocol,
            fields: vec![NativeReasoningFieldAssignment {
                path: if protocol == IngressProtocol::Responses {
                    vec!["reasoning".into(), "effort".into()]
                } else {
                    vec!["thinking".into(), "type".into()]
                },
                value: NativeReasoningValue::String(
                    if protocol == IngressProtocol::Responses {
                        "low"
                    } else {
                        "enabled"
                    }
                    .into(),
                ),
            }],
        };
        let mut profile = CandidateProtocolProfile::exact_portable_path(
            protocol, protocol, "physical", reasoning,
        );
        profile.capability.context.max_output_tokens = CriticalFact::Exact(4096);
        let rendered = project_candidate_request(&request, &profile).unwrap();
        assert!(rendered.body.get("enable_thinking").is_none());
        assert!(rendered.body["thinking"].get("enabled").is_none());
        if protocol == IngressProtocol::Responses {
            assert!(rendered.body.get("thinking").is_none());
            assert_eq!(
                rendered.body["reasoning"],
                json!({"effort":"low","summary":"auto"})
            );
        } else {
            assert_eq!(
                rendered.body["thinking"],
                json!({"type":"enabled","budget_tokens":1024})
            );
        }
        request.requested_reasoning.disposition =
            RequestedReasoningDisposition::AppliedToFixedBinding;
        let fixed = project_candidate_request(&request, &profile).unwrap();
        assert_eq!(fixed.body["enable_thinking"], body["enable_thinking"]);
        assert_eq!(fixed.body["thinking"], body["thinking"]);
        if protocol == IngressProtocol::Responses {
            assert_eq!(fixed.body["reasoning"], body["reasoning"]);
        }
    }
}
