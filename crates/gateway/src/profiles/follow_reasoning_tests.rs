use super::*;
use crate::server::core_runtime::profiles::{NativeReasoningFieldAssignment, ReasoningControlKind};
use crate::server::core_runtime::{adapters, model_ir::RequestedReasoningDisposition};
use hiroute_domain::ReasoningResolutionV1::*;
use serde_json::json;

fn candidate(
    ingress: IngressProtocol,
    upstream: IngressProtocol,
    efforts: &[&str],
) -> CandidateProtocolProfile {
    let profiles = efforts
        .iter()
        .map(|effort| ReasoningProfileCapability {
            profile_id: (*effort).into(),
            control_kind: ReasoningControlKind::Discrete,
            render: NativeReasoningRender::ExactFields {
                protocol: upstream,
                fields: vec![NativeReasoningFieldAssignment {
                    path: match upstream {
                        IngressProtocol::Responses => vec!["reasoning".into(), "effort".into()],
                        IngressProtocol::ChatCompletions => vec!["reasoning_effort".into()],
                        IngressProtocol::Messages => vec!["output_config".into(), "effort".into()],
                    },
                    value: NativeReasoningValue::String((*effort).into()),
                }],
            },
            accounting: ReasoningAccounting::WithinOutputCap,
            additional_reservation_tokens: 0,
        })
        .collect::<Vec<_>>();
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        ingress,
        upstream,
        "physical",
        profiles[0].clone(),
    );
    profile.capability.reasoning_profiles = profiles;
    profile.follow_request_reasoning = true;
    profile
}

#[test]
fn follow_reasoning_maps_exact_effort_and_renders_it_for_each_protocol() {
    for ingress in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let document = match ingress {
            IngressProtocol::Responses => {
                json!({"model":"route","input":"hi","reasoning":{"effort":"high"}})
            }
            IngressProtocol::ChatCompletions => {
                json!({"model":"route","messages":[{"role":"user","content":"hi"}],"reasoning_effort":"high"})
            }
            IngressProtocol::Messages => {
                json!({"model":"route","messages":[{"role":"user","content":"hi"}],"max_tokens":4096,"output_config":{"effort":"high"}})
            }
        };
        let control = RequestedNativeReasoning::from_document(&document, ingress).unwrap();
        let mut request = adapters::decode_ingress_request(ingress, &document).unwrap();
        request.requested_reasoning.followed_control = Some(control);
        request.requested_reasoning.disposition =
            RequestedReasoningDisposition::FollowRequestWithDefault;
        for upstream in [
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            IngressProtocol::Messages,
        ] {
            let profile = candidate(ingress, upstream, &["low", "high"]);
            let selected = profile.for_request_reasoning(&request).unwrap();
            assert_eq!(selected.selected_reasoning().unwrap().profile_id, "high");
            let encoded = adapters::project_candidate_request(&request, &selected).unwrap();
            let path = match upstream {
                IngressProtocol::Responses => "/reasoning/effort",
                IngressProtocol::ChatCompletions => "/reasoning_effort",
                IngressProtocol::Messages => "/output_config/effort",
            };
            assert_eq!(encoded.body.pointer(path), Some(&json!("high")));
        }
    }
}

#[test]
fn follow_reasoning_each_fallback_candidate_reuses_the_original_request() {
    let request = RequestedNativeReasoning::from_document(
        &json!({"reasoning":{"effort":"xhigh"}}),
        IngressProtocol::Responses,
    )
    .unwrap();
    let a = candidate(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        &["low", "xhigh"],
    );
    let b = candidate(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        &["medium", "high"],
    );
    let c = candidate(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        &["high", "xhigh"],
    );
    for (profile, effort, reason) in [
        (a, "xhigh", Request),
        (b, "medium", DefaultUnsupported),
        (c, "xhigh", Request),
    ] {
        let (selected, actual) = profile
            .follow_reasoning_with_resolution(&request, None)
            .unwrap();
        assert_eq!(selected.selected_reasoning().unwrap().profile_id, effort);
        assert_eq!(actual, reason);
    }
}

#[test]
fn follow_reasoning_uses_exact_normalized_toggle_wire_without_guessing_high() {
    let mut profile = candidate(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        &["low"],
    );
    profile.capability.reasoning_profiles = [false, true]
        .into_iter()
        .map(|enabled| ReasoningProfileCapability {
            profile_id: if enabled { "enabled" } else { "disabled" }.into(),
            control_kind: ReasoningControlKind::Toggle,
            render: NativeReasoningRender::ExactFields {
                protocol: IngressProtocol::Responses,
                fields: vec![NativeReasoningFieldAssignment {
                    path: vec!["enable_thinking".into()],
                    value: NativeReasoningValue::Bool(enabled),
                }],
            },
            accounting: ReasoningAccounting::WithinOutputCap,
            additional_reservation_tokens: 0,
        })
        .collect();
    profile.capability.selected_reasoning_profile_id = "disabled".into();
    for (effort, expected, reason) in [
        ("low", "enabled", Request),
        ("none", "disabled", Request),
        ("high", "disabled", DefaultUnsupported),
    ] {
        let control = RequestedNativeReasoning::from_document(
            &json!({"reasoning":{"effort":effort}}),
            IngressProtocol::Responses,
        )
        .unwrap();
        let (selected, actual) = profile
            .follow_reasoning_with_resolution(&control, None)
            .unwrap();
        assert_eq!(selected.selected_reasoning().unwrap().profile_id, expected);
        assert_eq!(actual, reason);
    }
}

#[test]
fn follow_reasoning_absent_unsupported_and_legacy_fixed_keep_saved_default() {
    let mut profile = candidate(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        &["low", "high"],
    );
    for (document, reason) in [
        (json!({}), DefaultMissing),
        (json!({"reasoning":{"summary":"auto"}}), DefaultMissing),
        (json!({"reasoning":{"effort":"future"}}), DefaultUnsupported),
    ] {
        let control =
            RequestedNativeReasoning::from_document(&document, IngressProtocol::Responses).unwrap();
        let (actual, resolution) = profile
            .follow_reasoning_with_resolution(&control, None)
            .unwrap();
        assert_eq!(actual, profile);
        assert_eq!(resolution, reason);
    }
    profile.follow_request_reasoning = false;
    let request = adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","input":"hi","reasoning":{"effort":"high"}}),
    )
    .unwrap();
    assert_eq!(profile.for_request_reasoning(&request).unwrap(), profile);
}

#[test]
fn follow_reasoning_rejects_malformed_controls_before_replay() {
    for (protocol, document) in [
        (IngressProtocol::Responses, json!({"reasoning":[]})),
        (
            IngressProtocol::Responses,
            json!({"reasoning":{"effort":12}}),
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"reasoning_effort":null}),
        ),
        (
            IngressProtocol::Messages,
            json!({"thinking":{"type":"enabled","budget_tokens":1.5}}),
        ),
        (
            IngressProtocol::Messages,
            json!({"thinking":{"type":"disabled","budget_tokens":1024}}),
        ),
        (
            IngressProtocol::Messages,
            json!({"thinking":{"type":"disabled"},"output_config":{"effort":"high"}}),
        ),
    ] {
        assert!(
            RequestedNativeReasoning::from_document(&document, protocol).is_err(),
            "{document}"
        );
    }
}

#[test]
fn follow_reasoning_budget_range_step_and_accounting_are_request_local() {
    let mut profile = candidate(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        &["low"],
    );
    profile.capability.context.max_output_tokens = super::super::CriticalFact::Exact(8192);
    let baseline = ReasoningProfileCapability {
        profile_id: "budget-1024".into(),
        control_kind: ReasoningControlKind::Budget,
        render: NativeReasoningRender::ExactBudget {
            protocol: IngressProtocol::Messages,
            fields: vec![
                NativeReasoningFieldAssignment {
                    path: vec!["thinking".into(), "type".into()],
                    value: NativeReasoningValue::String("enabled".into()),
                },
                NativeReasoningFieldAssignment {
                    path: vec!["thinking".into(), "budget_tokens".into()],
                    value: NativeReasoningValue::U64(1024),
                },
            ],
            budget_path: vec!["thinking".into(), "budget_tokens".into()],
            selected_tokens: 1024,
            min_tokens: 1024,
            max_tokens: 4096,
            step_tokens: 1024,
        },
        accounting: ReasoningAccounting::Additive,
        additional_reservation_tokens: 1024,
    };
    profile.capability.selected_reasoning_profile_id = baseline.profile_id.clone();
    profile.capability.reasoning_profiles = vec![baseline];
    for (tokens, expected, reason) in [
        (3072, 3072, Request),
        (1500, 1024, DefaultUnsupported),
        (8192, 1024, DefaultUnsupported),
    ] {
        let control = RequestedNativeReasoning::from_document(
            &json!({"thinking":{"type":"enabled","budget_tokens":tokens}}),
            IngressProtocol::Messages,
        )
        .unwrap();
        let (selected, actual) = profile
            .follow_reasoning_with_resolution(&control, None)
            .unwrap();
        assert_eq!(actual, reason);
        assert_eq!(
            selected
                .selected_reasoning()
                .unwrap()
                .additional_reservation_tokens,
            expected
        );
        assert!(
            selected
                .selected_reasoning()
                .unwrap()
                .validate_for(IngressProtocol::Messages)
        );
    }
    let control = RequestedNativeReasoning::from_document(
        &json!({"thinking":{"type":"enabled","budget_tokens":3072}}),
        IngressProtocol::Messages,
    )
    .unwrap();
    let (selected, reason) = profile
        .follow_reasoning_with_resolution(&control, Some(2048))
        .unwrap();
    assert_eq!(reason, DefaultUnsupported);
    assert_eq!(
        selected
            .selected_reasoning()
            .unwrap()
            .additional_reservation_tokens,
        1024
    );
    assert_eq!(
        profile
            .selected_reasoning()
            .unwrap()
            .additional_reservation_tokens,
        1024
    );
}
