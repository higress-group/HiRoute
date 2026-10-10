use super::*;
use serde_json::Value;

fn profile(target: IngressProtocol) -> CandidateProtocolProfile {
    exact_state_profile(IngressProtocol::Responses, target)
}

fn document(options: Value) -> Value {
    let mut body = json!({"model":"alias","input":"hello","stream":true,
        "max_output_tokens":128,"tools":[{"type":"function","name":"lookup",
        "description":"Look up a value","parameters":{"type":"object",
        "properties":{"key":{"type":"string"}},"required":["key"]},"strict":false}]});
    body.as_object_mut()
        .unwrap()
        .extend(options.as_object().unwrap().clone());
    body
}

#[test]
fn pi_options_four_cases_preserve_actual_chat_wire() {
    for options in [
        json!({}),
        json!({"store":false}),
        json!({"prompt_cache_key":"pi-session"}),
        json!({"store":false,"prompt_cache_key":"pi-session"}),
    ] {
        let original = document(options);
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        let projected =
            project_candidate_request(&request, &profile(IngressProtocol::ChatCompletions))
                .expect("Pi's stateless options have Chat equivalents");
        let wire: Value = serde_json::from_slice(&projected.bytes).unwrap();
        assert_eq!(projected.path, "/v1/chat/completions");
        for field in ["store", "prompt_cache_key"] {
            assert_eq!(wire.get(field), original.get(field), "{field}");
        }
        assert_eq!(wire["model"], "physical");
        assert_eq!(wire["max_completion_tokens"], 128);
        assert_eq!(wire["stream"], true);
        assert_eq!(wire["tools"][0]["function"]["name"], "lookup");
        assert_eq!(
            wire["tools"][0]["function"]["parameters"],
            original["tools"][0]["parameters"]
        );
        assert_eq!(wire["tools"][0]["function"]["strict"], false);
    }
}

#[test]
fn pi_options_preserve_native_responses_values() {
    let original = document(json!({"store":false,"prompt_cache_key":"pi-session",
        "include":["reasoning.encrypted_content"],"client_metadata":{"source":"pi"},
        "reasoning":{"summary":"auto","context":"all_turns"}}));
    let mut request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    // Exercise both the native body and canonical serializer.
    for canonical in [false, true] {
        if canonical {
            request.native_body = None;
        }
        let projected =
            project_candidate_request(&request, &profile(IngressProtocol::Responses)).unwrap();
        let wire: Value = serde_json::from_slice(&projected.bytes).unwrap();
        for field in [
            "store",
            "prompt_cache_key",
            "include",
            "client_metadata",
            "reasoning",
        ] {
            assert_eq!(
                wire[field], original[field],
                "{field}, canonical={canonical}"
            );
        }
    }
}

#[test]
fn pi_options_native_fields_and_messages_remain_field_specific_rejections() {
    for (field, options) in [
        ("include", json!({"include":[]})),
        (
            "include",
            json!({"include":["reasoning.encrypted_content"]}),
        ),
        ("client_metadata", json!({"client_metadata":{}})),
        ("reasoning.summary", json!({"reasoning":{"summary":"auto"}})),
        (
            "reasoning.context",
            json!({"reasoning":{"context":"all_turns"}}),
        ),
    ] {
        let mut original = document(options);
        original["store"] = json!(false);
        original["prompt_cache_key"] = json!("pi-session");
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        let error = project_candidate_request(&request, &profile(IngressProtocol::ChatCompletions))
            .unwrap_err();
        assert_eq!(error.code(), "CLIENT_PROTOCOL_UNREPRESENTABLE");
        assert!(error.to_string().contains(field), "{field}: {error}");
    }
    for (field, options) in [
        ("store", json!({"store":false})),
        ("prompt_cache_key", json!({"prompt_cache_key":"pi-session"})),
    ] {
        let request =
            decode_ingress_request(IngressProtocol::Responses, &document(options)).unwrap();
        let error =
            project_candidate_request(&request, &profile(IngressProtocol::Messages)).unwrap_err();
        assert_eq!(error.code(), "CLIENT_PROTOCOL_UNREPRESENTABLE");
        assert!(error.to_string().contains(field), "{error}");
    }
}

#[test]
fn pi_options_do_not_make_messages_thinking_display_a_responses_control() {
    // thinking.display belongs to Messages. Mapping known Responses options
    // must not admit a mixed-protocol extension through the native-only guard.
    let original = document(json!({"store":false,"prompt_cache_key":"pi-session",
        "thinking":{"display":"omitted"}}));
    let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    assert!(request.native_only);
    let native = project_candidate_request(&request, &profile(IngressProtocol::Responses)).unwrap();
    for field in ["store", "prompt_cache_key", "thinking"] {
        assert_eq!(native.body[field], original[field]);
    }
    let error = project_candidate_request(&request, &profile(IngressProtocol::ChatCompletions))
        .unwrap_err();
    assert_eq!(error.code(), "CLIENT_PROTOCOL_UNREPRESENTABLE");
}

#[test]
fn pi_options_do_not_admit_invalid_types_or_server_state() {
    for options in [
        json!({"store":true}),
        json!({"store":"false"}),
        json!({"store":null}),
        json!({"prompt_cache_key":42}),
        json!({"prompt_cache_key":null}),
        json!({"previous_response_id":"resp-previous"}),
        json!({"conversation":"conv-1"}),
    ] {
        assert!(decode_ingress_request(IngressProtocol::Responses, &document(options)).is_err());
    }
}

#[test]
fn pi_options_continue_multiple_tools_and_failed_results_with_plan_effort() {
    use crate::server::core_runtime::profiles::{
        NativeReasoningFieldAssignment, NativeReasoningRender, NativeReasoningValue,
        ReasoningControlKind,
    };
    let mut original = document(json!({"store":false,"prompt_cache_key":"pi-session",
        "reasoning":{"effort":"low"}}));
    original["input"] = json!([
        {"role":"user","content":"look up two values"},
        {"type":"function_call","call_id":"call-1","name":"lookup","arguments":"{\"key\":\"one\"}"},
        {"type":"function_call","call_id":"call-2","name":"lookup","arguments":"{\"key\":\"two\"}"},
        {"type":"function_call_output","call_id":"call-1","output":"value one"},
        {"type":"function_call_output","call_id":"call-2","output":"tool failed: missing key"},
        {"role":"user","content":"Continue and handle the missing key"}]);
    let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    let mut profile = profile(IngressProtocol::ChatCompletions);
    let reasoning = &mut profile.capability.reasoning_profiles[0];
    reasoning.control_kind = ReasoningControlKind::Discrete;
    reasoning.render = NativeReasoningRender::ExactFields {
        protocol: IngressProtocol::ChatCompletions,
        fields: vec![NativeReasoningFieldAssignment {
            path: vec!["reasoning_effort".into()],
            value: NativeReasoningValue::String("high".into()),
        }],
    };
    let projected = project_candidate_request(&request, &profile).unwrap();
    let wire: Value = serde_json::from_slice(&projected.bytes).unwrap();
    assert_eq!(wire["store"], false);
    assert_eq!(wire["prompt_cache_key"], "pi-session");
    assert_eq!(wire["reasoning_effort"], "high");
    assert_eq!(
        wire["messages"][1]["tool_calls"].as_array().unwrap().len(),
        2
    );
    for (index, id, args) in [
        (0, "call-1", "{\"key\":\"one\"}"),
        (1, "call-2", "{\"key\":\"two\"}"),
    ] {
        assert_eq!(wire["messages"][1]["tool_calls"][index]["id"], id);
        assert_eq!(
            wire["messages"][1]["tool_calls"][index]["function"]["arguments"],
            args
        );
    }
    assert_eq!(wire["messages"][2]["tool_call_id"], "call-1");
    assert_eq!(wire["messages"][3]["content"], "tool failed: missing key");
    assert_eq!(
        wire["messages"][4]["content"],
        "Continue and handle the missing key"
    );
}

#[test]
fn pi_options_survive_externalized_history_and_attempt_replay() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-pi-options-{}-{}",
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
    let mut original = document(json!({"store":false,"prompt_cache_key":"pi-session"}));
    original["input"] = json!("history ".repeat(4096));
    let mut request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    externalize_model_request(&mut request, &store, 128).unwrap();
    assert!(!model_content_refs(&request).is_empty());
    store.prevalidate(&model_content_refs(&request)).unwrap();
    for _ in 0..2 {
        let template = project_candidate_request_template(
            &request,
            &profile(IngressProtocol::ChatCompletions),
        )
        .unwrap();
        let mut reader = sequential_attempt_body(template, store.clone(), &budget, 127).unwrap();
        let mut bytes = Vec::new();
        while let Some(chunk) = reader.next_chunk().unwrap() {
            bytes.extend_from_slice(chunk.bytes());
        }
        let wire: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(wire["store"], false);
        assert_eq!(wire["prompt_cache_key"], "pi-session");
        assert_eq!(wire["messages"][0]["content"], original["input"]);
    }
    drop(store);
    drop(manager);
    fs_err_remove_dir(&root);
}
