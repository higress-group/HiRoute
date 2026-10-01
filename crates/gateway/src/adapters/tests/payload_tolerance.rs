use super::*;
use serde_json::Value;

#[test]
fn native_messages_reminder_keeps_replay_boundaries_and_cache_fields() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-native-reminder-{}-{}",
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
    let budget = tree.stream(4 * 1024 * 1024).unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let text = "instruction ".repeat(2000);
    let system = json!({"type":"text","text":text,"cache_control":{"type":"ephemeral"}});
    let body = json!({"model":"alias","max_tokens":128,"system":[system.clone()],"messages":[
        {"role":"user","content":"hi"},{"role":"system","content":"reminder"}]});
    let mut request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
    externalize_model_request(&mut request, &store, 8192).unwrap();
    let profile = exact_state_profile(IngressProtocol::Messages, IngressProtocol::Messages);
    let template = project_candidate_request_template(&request, &profile).unwrap();
    store.prevalidate(&model_content_refs(&request)).unwrap();
    let mut reader = sequential_attempt_body(template, store.clone(), &budget, 257).unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        bytes.extend_from_slice(chunk.bytes());
    }
    let output: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        output["system"],
        json!([system,{"type":"text","text":"reminder"}])
    );
    assert_eq!(output["messages"], json!([{"role":"user","content":"hi"}]));
    drop(reader);
    drop(store);
    drop(manager);
    fs_err_remove_dir(&root);
}

#[test]
fn messages_nested_output_semantics_are_native_only() {
    for (key, value) in [
        (
            "output_config",
            json!({"format":{"type":"provider_format","schema":{"type":"object"}}}),
        ),
        (
            "thinking",
            json!({"type":"adaptive","provider_mode":"deep"}),
        ),
    ] {
        let mut body =
            json!({"model":"alias","max_tokens":128,"messages":[{"role":"user","content":"hi"}]});
        body[key] = value;
        let request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
        assert!(request.native_only);
        assert!(crate::context_hold::visible_history(&request, &[0; 32]).is_none());
        assert!(
            project_candidate_request_template_with_cleanup(
                &request,
                &exact_state_profile(IngressProtocol::Messages, IngressProtocol::Messages),
                Some(1)
            )
            .is_err()
        );
        let native = project_candidate_request(
            &request,
            &exact_state_profile(IngressProtocol::Messages, IngressProtocol::Messages),
        )
        .unwrap();
        if key == "output_config" {
            assert_eq!(native.body[key], body[key]);
        } else {
            assert_eq!(
                native.body[key]["provider_mode"],
                body[key]["provider_mode"]
            );
        }
        assert!(
            project_candidate_request(
                &request,
                &exact_state_profile(IngressProtocol::Messages, IngressProtocol::Responses)
            )
            .is_err()
        );
    }
}

#[test]
fn messages_cleanup_omits_only_emptied_prefix_messages() {
    let body = json!({"model":"alias","max_tokens":128,"messages":[
        {"role":"assistant","content":[{"type":"thinking","thinking":"old","signature":"old-signature"}]},
        {"role":"user","content":"next"},
        {"role":"assistant","content":[{"type":"thinking","thinking":"new","signature":"new-signature"},{"type":"text","text":"answer"}]}
    ]});
    let request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
    let profile = exact_state_profile(IngressProtocol::Messages, IngressProtocol::Messages);
    let cleaned =
        project_candidate_request_template_with_cleanup(&request, &profile, Some(1)).unwrap();
    let cleaned: Value = serde_json::from_slice(&cleaned.bytes).unwrap();
    assert_eq!(
        cleaned["messages"],
        json!([body["messages"][1], body["messages"][2]])
    );
    assert_eq!(request.native_body.as_ref().unwrap(), &body);
}

#[test]
fn native_metadata_variants_do_not_require_a_local_provider_enum() {
    let body = json!({"model":"alias", "input":[{"type":"message","role":"assistant",
        "phase":"provider_phase", "status":"provider_status", "content":[{"type":"output_text","text":""}]}],
        "reasoning":{"effort":"","context":""},"prompt_cache_key":"","include":[""]});
    let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    assert!(request.native_only);
    let projected = project_candidate_request(
        &request,
        &exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses),
    )
    .unwrap();
    assert_eq!(projected.body["input"], body["input"]);
    assert_eq!(projected.body["include"], body["include"]);
    assert!(
        project_candidate_request(
            &request,
            &exact_state_profile(IngressProtocol::Responses, IngressProtocol::Messages)
        )
        .is_err()
    );
}

#[test]
fn native_chat_tool_names_are_not_subject_to_cross_protocol_projection_rules() {
    let body = json!({"model":"alias","messages":[{"role":"user","content":"hi"}],
        "tools":[{"type":"function","function":{"name":"provider.qualified.name","parameters":{"type":"object"}}}]});
    let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
    let projected = project_candidate_request(
        &request,
        &exact_state_profile(
            IngressProtocol::ChatCompletions,
            IngressProtocol::ChatCompletions,
        ),
    )
    .unwrap();
    assert_eq!(projected.body["tools"], body["tools"]);
}

#[test]
fn plan_owned_provider_reasoning_controls_override_only_configured_fields() {
    use crate::server::core_runtime::profiles::{
        NativeReasoningFieldAssignment, NativeReasoningRender, NativeReasoningValue,
        ReasoningControlKind,
    };
    let protocol = IngressProtocol::ChatCompletions;
    let request = decode_ingress_request(
        protocol,
        &json!({"model":"alias", "messages":[{"role":"user","content":"hi"}],
        "enable_thinking":false, "provider_hint":"keep"}),
    )
    .unwrap();
    let mut profile = exact_state_profile(protocol, protocol);
    let reasoning = &mut profile.capability.reasoning_profiles[0];
    reasoning.control_kind = ReasoningControlKind::Toggle;
    reasoning.render = NativeReasoningRender::ExactFields {
        protocol,
        fields: vec![NativeReasoningFieldAssignment {
            path: vec!["enable_thinking".into()],
            value: NativeReasoningValue::Bool(true),
        }],
    };
    let projected = project_candidate_request(&request, &profile).unwrap();
    assert_eq!(projected.body["enable_thinking"], true);
    assert_eq!(projected.body["provider_hint"], "keep");
    assert_eq!(
        request.native_body.as_ref().unwrap()["enable_thinking"],
        false
    );
}

#[test]
fn same_protocol_extensions_are_preserved_not_silently_converted() {
    for (protocol, body) in [
        (
            IngressProtocol::Messages,
            json!({"model":"alias","max_tokens":128,"temperature":0.7,
            "messages":[{"role":"user","content":[{"type":"text","text":"hi","provider_tag":"keep"}]}]}),
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"model":"alias","temperature":0.7,
            "messages":[{"role":"user","content":"hi","provider_tag":"keep"}]}),
        ),
        (
            IngressProtocol::Responses,
            json!({"model":"alias","temperature":0.7,
            "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi","provider_tag":"keep"}]}]}),
        ),
    ] {
        let request = decode_ingress_request(protocol, &body).unwrap();
        assert!(request.native_only);
        let native = project_candidate_request(&request, &exact_state_profile(protocol, protocol))
            .unwrap()
            .body;
        assert_eq!(native["temperature"], body["temperature"]);
        let history_key = if protocol == IngressProtocol::Responses {
            "input"
        } else {
            "messages"
        };
        assert_eq!(native[history_key], body[history_key]);
        let target = if protocol == IngressProtocol::Messages {
            IngressProtocol::Responses
        } else {
            IngressProtocol::Messages
        };
        assert!(
            project_candidate_request(&request, &exact_state_profile(protocol, target)).is_err()
        );
        for field in ["api_key", "base_url", "mcp_servers"] {
            let mut denied = body.clone();
            denied[field] = json!("not-authorized");
            assert!(decode_ingress_request(protocol, &denied).is_err());
        }
    }
}

#[test]
fn unknown_hosted_tools_are_not_native_payload_extensions() {
    let body = json!({"model":"alias","max_tokens":128,"messages":[{"role":"user","content":"hi"}],
        "tools":[{"type":"web_search_future","name":"web_search","input_schema":{}}]});
    assert!(decode_ingress_request(IngressProtocol::Messages, &body).is_err());
}

#[test]
fn optional_provider_metadata_does_not_prevent_a_plain_stream() {
    let request = decode_ingress_request(
        IngressProtocol::Messages,
        &json!({
            "model":"alias","stream":true,"messages":[{"role":"user","content":"hi"}]
        }),
    )
    .unwrap();
    let mut profile = exact_state_profile(IngressProtocol::Messages, IngressProtocol::Messages);
    profile.capability.native_streaming = CriticalFact::Unknown;
    profile.capability.native_provider_state = NativeProviderStateEmission::Unknown;
    profile.capability.response.reasoning = Fidelity::Unknown;
    profile.capability.response.refusal = Fidelity::Unknown;
    profile.capability.response.usage = Fidelity::Unknown;
    profile.capability.response.stream_reasoning_delta = Fidelity::Unknown;
    profile.capability.response.stream_usage = Fidelity::Unknown;
    project_candidate_request(&request, &profile).unwrap();
}

#[test]
fn messages_optional_signature_survives_tool_continuation() {
    let profile = exact_state_profile(IngressProtocol::Messages, IngressProtocol::Messages);
    for signature in [None, Some(""), Some("provider-owned-state")] {
        let mut thinking = json!({"type":"thinking","thinking":"plan"});
        if let Some(signature) = signature {
            thinking["signature"] = json!(signature);
        }
        let body = json!({"model":"alias","max_tokens":128,"messages":[
            {"role":"assistant","content":[thinking.clone(),
                {"type":"tool_use","id":"t1","name":"Read","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}
        ]});
        let request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
        let projected = project_candidate_request(&request, &profile).unwrap();
        assert_eq!(projected.body["messages"][0]["content"][0], thinking);
        let mut invalid = body;
        invalid["messages"][0]["content"][0]["signature"] = json!(42);
        assert!(decode_ingress_request(IngressProtocol::Messages, &invalid).is_err());
    }
}

#[test]
fn empty_content_strings_are_payload_not_identity() {
    for (protocol, body) in [
        (
            IngressProtocol::Messages,
            json!({"model":"alias","max_tokens":128,
            "system":[{"type":"text","text":""}],
            "messages":[{"role":"user","content":[{"type":"text","text":""}]}]}),
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"model":"alias",
            "messages":[{"role":"user","content":[{"type":"text","text":""}]}]}),
        ),
        (
            IngressProtocol::Responses,
            json!({"model":"alias",
            "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":""}]}]}),
        ),
    ] {
        let request = decode_ingress_request(protocol, &body).unwrap();
        let profile = exact_state_profile(protocol, protocol);
        project_candidate_request(&request, &profile).unwrap();
        let mut invalid = body;
        invalid["model"] = json!("");
        assert!(decode_ingress_request(protocol, &invalid).is_err());
    }
}

#[test]
fn explicit_false_does_not_require_strict_tool_support() {
    let request = decode_ingress_request(IngressProtocol::ChatCompletions, &json!({
        "model":"alias","messages":[{"role":"user","content":"hi"}],
        "tools":[{"type":"function","function":{"name":"read","parameters":{"type":"object"},"strict":false}}]
    })).unwrap();
    assert!(!request.requirements().strict_tools);
}

#[test]
fn provider_image_reference_and_no_usage_request_remain_native() {
    let body = json!({"model":"alias","stream":true,"stream_options":{"include_usage":false},
        "messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"provider-asset:123"}}]}]});
    let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
    assert!(request.native_only);
    let profile = exact_state_profile(
        IngressProtocol::ChatCompletions,
        IngressProtocol::ChatCompletions,
    );
    let native = project_candidate_request(&request, &profile).unwrap().body;
    assert_eq!(native["messages"], body["messages"]);
    assert_eq!(native["stream_options"], body["stream_options"]);
    assert!(
        project_candidate_request(
            &request,
            &exact_state_profile(IngressProtocol::ChatCompletions, IngressProtocol::Messages)
        )
        .is_err()
    );
}

#[test]
fn optional_context_estimate_does_not_invent_a_wire_limit() {
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":"hi"}),
    )
    .unwrap();
    let mut profile = exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
    profile.capability.context.max_output_tokens = CriticalFact::Unknown;
    profile.capability.context.estimator = CriticalFact::Unknown;
    let projected = project_candidate_request(&request, &profile).unwrap();
    assert!(projected.context.is_none());
    assert!(projected.body.get("max_output_tokens").is_none());
}

#[test]
fn explicit_output_limit_is_reflected_in_projection_and_estimate() {
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":"hi","max_output_tokens":12}),
    )
    .unwrap();
    let profile = exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
    let projected = project_candidate_request(&request, &profile).unwrap();
    assert_eq!(projected.body["max_output_tokens"], 12);
    assert_eq!(projected.context.unwrap().effective_output_cap, 12);
}

#[test]
fn native_chat_cleanup_uses_canonical_indexes_and_immutable_original() {
    let body = json!({"model":"alias","messages":[
        {"role":"system","content":"instructions"},
        {"role":"user","content":"first"},
        {"role":"assistant","content":"old answer","reasoning_content":"old reasoning"},
        {"role":"user","content":"next"},
        {"role":"assistant","content":"new answer","reasoning_content":"new reasoning"}
    ]});
    let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
    let profile = exact_state_profile(
        IngressProtocol::ChatCompletions,
        IngressProtocol::ChatCompletions,
    );
    let cleaned =
        project_candidate_request_template_with_cleanup(&request, &profile, Some(2)).unwrap();
    let cleaned: Value = serde_json::from_slice(&cleaned.bytes).unwrap();
    assert!(cleaned["messages"][2].get("reasoning_content").is_none());
    assert_eq!(cleaned["messages"][4]["reasoning_content"], "new reasoning");
    let original = project_candidate_request(&request, &profile).unwrap().body;
    assert_eq!(original["messages"], body["messages"]);
}

#[test]
fn messages_stop_sequence_is_completion_metadata_not_a_decode_failure() {
    let profile = exact_state_profile(IngressProtocol::Responses, IngressProtocol::Messages);
    let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
    decoder.feed(&serde_json::to_vec(&json!({"id":"m","type":"message","role":"assistant","model":"native",
        "content":[{"type":"text","text":"done"}],"stop_reason":"stop_sequence","stop_sequence":"END",
        "usage":{"input_tokens":1,"output_tokens":1}})).unwrap(), true).unwrap();
    let response = decoder.finish().unwrap();
    assert!(response.response.completed);
    assert!(!response.response.blocks.is_empty());
}
