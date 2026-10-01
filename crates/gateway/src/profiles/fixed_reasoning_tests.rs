use hiroute_domain::CanonicalDigest;
use serde_json::json;

use super::*;
use crate::content_ref::{JsonValueExt, externalize_model_request};
use crate::replay::{ReplayConfig, ReplayManager};
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::core_runtime::profiles::{
    CriticalFact, NativeReasoningFieldAssignment, NativeReasoningRender, ReasoningAccounting,
    ReasoningControlKind, ReasoningProfileCapability,
};
use hiroute_gateway_core::runtime::body::BudgetTree;

fn choices(ingress: IngressProtocol, upstream: IngressProtocol) -> CandidateProtocolProfile {
    let path = match upstream {
        IngressProtocol::Responses => vec!["reasoning".into(), "effort".into()],
        IngressProtocol::ChatCompletions => vec!["reasoning_effort".into()],
        IngressProtocol::Messages => vec!["output_config".into(), "effort".into()],
    };
    let profiles = ["low", "high"].map(|effort| ReasoningProfileCapability {
        profile_id: format!("sealed-{effort}"),
        control_kind: ReasoningControlKind::Discrete,
        render: NativeReasoningRender::ExactFields {
            protocol: upstream,
            fields: vec![NativeReasoningFieldAssignment {
                path: path.clone(),
                value: NativeReasoningValue::String(effort.into()),
            }],
        },
        accounting: ReasoningAccounting::WithinOutputCap,
        additional_reservation_tokens: 0,
    });
    let mut candidate = CandidateProtocolProfile::exact_portable_path(
        ingress,
        upstream,
        "physical",
        profiles[0].clone(),
    );
    candidate.capability.reasoning_profiles = profiles.to_vec();
    candidate
}

#[test]
fn fixed_reasoning_responses_to_chat_selects_exact_native_assignment() {
    let profile = choices(IngressProtocol::Responses, IngressProtocol::ChatCompletions);
    let selected = profile
        .select_native_reasoning(&json!({"effort":"high"}), IngressProtocol::Responses)
        .unwrap();
    assert_eq!(
        selected.selected_reasoning().unwrap().profile_id,
        "sealed-high"
    );
    assert_eq!(
        selected.selected_reasoning().unwrap().render.fields()[0].path,
        ["reasoning_effort"]
    );
}

#[test]
fn fixed_reasoning_accepts_valid_responses_history_policy_without_mistaking_it_for_effort() {
    let profile = choices(IngressProtocol::Responses, IngressProtocol::Responses);
    let selected = profile
        .select_native_reasoning(
            &json!({"effort":"high","context":"all_turns"}),
            IngressProtocol::Responses,
        )
        .unwrap();
    assert_eq!(
        selected.selected_reasoning().unwrap().profile_id,
        "sealed-high"
    );
    assert!(
        profile
            .select_native_reasoning(
                &json!({"effort":"high","context":"unsupported"}),
                IngressProtocol::Responses,
            )
            .is_ok()
    );
}

#[test]
fn fixed_reasoning_chat_to_responses_selects_exact_native_assignment() {
    let profile = choices(IngressProtocol::ChatCompletions, IngressProtocol::Responses);
    let selected = profile
        .select_native_reasoning(&json!("high"), IngressProtocol::ChatCompletions)
        .unwrap();
    assert_eq!(
        selected.selected_reasoning().unwrap().profile_id,
        "sealed-high"
    );
}

#[test]
fn fixed_reasoning_messages_effort_selects_sealed_profile() {
    let profile = choices(IngressProtocol::Messages, IngressProtocol::Messages);
    let selected = profile
        .select_native_reasoning(
            &json!({"output_config":{"effort":"high"}}),
            IngressProtocol::Messages,
        )
        .unwrap();
    assert_eq!(
        selected.selected_reasoning().unwrap().profile_id,
        "sealed-high"
    );
}

#[test]
fn fixed_reasoning_ambiguous_directory_does_not_reject_native_controls() {
    let mut profile = choices(IngressProtocol::Responses, IngressProtocol::Responses);
    let mut duplicate = profile.capability.reasoning_profiles[1].clone();
    duplicate.profile_id = "ambiguous".into();
    profile.capability.reasoning_profiles.push(duplicate);
    assert!(
        profile
            .select_native_reasoning(&json!({"effort":"high"}), IngressProtocol::Responses)
            .is_ok()
    );
}

#[test]
fn fixed_reasoning_thinking_cannot_guess_cross_protocol_budget() {
    let profile = choices(IngressProtocol::Messages, IngressProtocol::Responses);
    assert!(
        profile
            .select_native_reasoning(
                &json!({"thinking":{"type":"enabled","budget_tokens":1024}}),
                IngressProtocol::Messages
            )
            .is_err()
    );
}

#[test]
fn fixed_reasoning_unknown_native_effort_is_left_to_provider() {
    let profile = choices(IngressProtocol::Responses, IngressProtocol::Responses);
    let selected = profile
        .select_native_reasoning(&json!({"effort":"sealed-high"}), IngressProtocol::Responses)
        .unwrap();
    assert_eq!(selected.capability.context.estimator, CriticalFact::Unknown);
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias", "input":"hello", "reasoning":{"effort":"sealed-high"}}),
    )
    .unwrap();
    request.requested_reasoning.fixed_profile_digest =
        Some(CanonicalDigest::of(selected.selected_reasoning().unwrap()).unwrap());
    request.requested_reasoning.fixed_profile =
        Some(selected.selected_reasoning().unwrap().clone());
    request.requested_reasoning.disposition = RequestedReasoningDisposition::AppliedToFixedBinding;
    assert_eq!(
        profile
            .for_request_reasoning(&request)
            .unwrap()
            .capability
            .context
            .estimator,
        CriticalFact::Unknown
    );
}

#[test]
fn fixed_reasoning_preserves_exact_budget_and_reservation() {
    let mut profile = choices(IngressProtocol::Messages, IngressProtocol::Messages);
    let high = &mut profile.capability.reasoning_profiles[1];
    high.control_kind = ReasoningControlKind::Budget;
    high.accounting = ReasoningAccounting::Additive;
    high.additional_reservation_tokens = 2048;
    high.render = NativeReasoningRender::ExactBudget {
        protocol: IngressProtocol::Messages,
        fields: vec![
            NativeReasoningFieldAssignment {
                path: vec!["thinking".into(), "type".into()],
                value: NativeReasoningValue::String("enabled".into()),
            },
            NativeReasoningFieldAssignment {
                path: vec!["thinking".into(), "budget_tokens".into()],
                value: NativeReasoningValue::U64(2048),
            },
        ],
        budget_path: vec!["thinking".into(), "budget_tokens".into()],
        selected_tokens: 2048,
        min_tokens: 1024,
        max_tokens: 4096,
        step_tokens: 1024,
    };
    let expected = high.clone();
    let selected = profile
        .select_native_reasoning(
            &json!({"thinking":{"type":"enabled","budget_tokens":2048}}),
            IngressProtocol::Messages,
        )
        .unwrap();
    assert_eq!(selected.selected_reasoning().unwrap(), &expected);
    assert_eq!(
        profile.capability.selected_reasoning_profile_id,
        "sealed-low"
    );
}

#[test]
fn fixed_reasoning_content_references_preserve_sealed_choice() {
    let profile = choices(IngressProtocol::Responses, IngressProtocol::Responses);
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"alias", "input":"hello".repeat(4096), "reasoning":{"effort":"high"}
        }),
    )
    .unwrap();
    let selected = profile
        .select_native_reasoning(
            request.requested_reasoning.native_value.as_ref().unwrap(),
            IngressProtocol::Responses,
        )
        .unwrap();
    request.requested_reasoning.fixed_profile_digest =
        Some(CanonicalDigest::of(selected.selected_reasoning().unwrap()).unwrap());
    request.requested_reasoning.fixed_profile =
        Some(selected.selected_reasoning().unwrap().clone());
    request.requested_reasoning.disposition = RequestedReasoningDisposition::AppliedToFixedBinding;
    let root = std::env::temp_dir().join(format!(
        "hiroute-fixed-reasoning-{}-{}",
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
    let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).unwrap();
    let store = manager
        .begin_request(tree.stream(4 * 1024 * 1024).unwrap())
        .unwrap();
    externalize_model_request(&mut request, &store, 8).unwrap();
    assert!(
        request
            .requested_reasoning
            .native_value
            .as_ref()
            .unwrap()
            .content_ref()
            .is_some()
    );
    assert_eq!(
        profile
            .for_request_reasoning(&request)
            .unwrap()
            .selected_reasoning()
            .unwrap(),
        selected.selected_reasoning().unwrap()
    );
    request.requested_reasoning.fixed_profile_digest = Some(CanonicalDigest::of_bytes(b"unsealed"));
    assert!(profile.for_request_reasoning(&request).is_err());
    drop(store);
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn fixed_reasoning_plan_ignores_client_and_fixed_selection() {
    let profile = choices(IngressProtocol::Responses, IngressProtocol::Responses);
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"alias", "input":"hello", "reasoning":{"effort":"high"}
        }),
    )
    .unwrap();
    request.requested_reasoning.fixed_profile_digest = Some(CanonicalDigest::of_bytes(b"unsealed"));
    assert_eq!(profile.for_request_reasoning(&request).unwrap(), profile);
}
