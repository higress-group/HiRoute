use super::*;
use serde_json::Value;

fn document(output: &str) -> Value {
    json!({"model":"alias","input":[
        {"role":"user","content":"Complete the synthetic goal"},
        {"type":"function_call","id":"fc-goal","call_id":"call-goal","name":"create_goal",
         "arguments":"{\"objective\":\"Complete the synthetic goal\"}"},
        {"type":"function_call_output","call_id":"call-goal","output":"{\"status\":\"active\"}"},
        {"type":"function_call_output","id":"fco-goal","name":"goal_objective","namespace":"agent","output":output}
    ]})
}

#[test]
fn codex_goal_context_preserves_native_history_without_tool_execution_identity() {
    for extension in [
        json!({}),
        json!({"additional_context_tool_output":true}),
        json!({"provider_options":{"context":"native"}}),
    ] {
        let mut body = document("goal context");
        body["input"][3]
            .as_object_mut()
            .unwrap()
            .extend(extension.as_object().unwrap().clone());
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        assert!(request.native_only);
        assert!(!request.native_history_layout().unwrap()[3].modeled);
        assert_eq!(request.messages.len(), 4);
        assert_eq!(request.messages[3].role, MessageRole::Assistant);
        assert!(request.messages[3].content.is_empty());
        assert!(request.messages[3].name.is_none());
        assert!(request.tools.is_empty());
        let executable = request
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|part| match part {
                ContentPart::ToolCall { logical_id, .. }
                | ContentPart::ToolResult { logical_id, .. } => Some(logical_id.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(executable, ["call-goal", "call-goal"]);
        assert!(crate::context_hold::visible_history(&request, &[0; 32]).is_none());
        for codex in [false, true] {
            let mut profile =
                exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
            if codex {
                profile.connector.connector_id = "connector.cpa.codex".into();
                profile.connector.endpoint_id = "endpoint.cpa.codex".into();
            }
            profile.capability.native_model = "replacement-model".into();
            let projected = project_candidate_request(&request, &profile).unwrap();
            assert_eq!(projected.body["input"], body["input"]);
            assert_eq!(projected.body["model"], "replacement-model");
            let template = project_candidate_request_template(&request, &profile).unwrap();
            let wire: Value = serde_json::from_slice(&template.bytes).unwrap();
            assert_eq!(wire["input"], body["input"]);
            assert!(
                project_candidate_request_template_with_cleanup(&request, &profile, Some(1))
                    .is_err()
            );
        }
        for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
            let error = project_candidate_request(
                &request,
                &exact_state_profile(IngressProtocol::Responses, target),
            )
            .unwrap_err();
            assert_eq!(error.code(), "CLIENT_PROTOCOL_UNREPRESENTABLE");
        }
        assert_eq!(request.native_body, Some(body));
    }
}

#[test]
fn codex_goal_context_exception_keeps_ordinary_result_identities_strict() {
    let body = document("goal context");
    for (field, value) in [
        ("name", json!("create_goal")),
        ("name", Value::Null),
        ("namespace", json!("functions")),
        ("namespace", Value::Null),
        ("call_id", Value::Null),
        ("call_id", json!("")),
        ("call_id", json!(1)),
    ] {
        let mut invalid = body.clone();
        invalid["input"][3][field] = value;
        invalid["input"][3]["additional_context_tool_output"] = json!(true);
        assert_eq!(
            decode_ingress_request(IngressProtocol::Responses, &invalid).unwrap_err(),
            ModelIrError::InvalidField("call_id")
        );
    }
    for field in ["name", "namespace"] {
        let mut invalid = body.clone();
        invalid["input"][3].as_object_mut().unwrap().remove(field);
        assert_eq!(
            decode_ingress_request(IngressProtocol::Responses, &invalid).unwrap_err(),
            ModelIrError::InvalidField("call_id")
        );
    }
    let generic = json!({"model":"alias","input":[{"type":"function_call_output","output":"ordinary result"}]});
    assert_eq!(
        decode_ingress_request(IngressProtocol::Responses, &generic).unwrap_err(),
        ModelIrError::InvalidField("call_id")
    );

    let mut paired = body;
    paired["input"][3]["call_id"] = json!("call-explicit");
    let request = decode_ingress_request(IngressProtocol::Responses, &paired).unwrap();
    assert!(
        matches!(&request.messages[3].content[0], ContentPart::ToolResult { logical_id, .. } if logical_id == "call-explicit")
    );
    for (protocol, body, expected) in [
        (
            IngressProtocol::ChatCompletions,
            json!({"model":"alias","messages":[{"role":"tool","name":"goal_objective","namespace":"agent","content":"context"}]}),
            "tool_call_id",
        ),
        (
            IngressProtocol::Messages,
            json!({"model":"alias","max_tokens":128,"messages":[{"role":"user","content":[{"type":"tool_result","name":"goal_objective","namespace":"agent","content":"context"}]}]}),
            "tool_use_id",
        ),
    ] {
        assert_eq!(
            decode_ingress_request(protocol, &body).unwrap_err(),
            ModelIrError::InvalidField(expected)
        );
    }
}

#[test]
fn codex_goal_context_preserves_content_and_authority_validation() {
    for output in [Value::Null, json!({}), json!([]), json!(42)] {
        let mut body = document("context");
        body["input"][3]["output"] = output;
        assert_eq!(
            decode_ingress_request(IngressProtocol::Responses, &body).unwrap_err(),
            ModelIrError::InvalidField("output")
        );
    }
    let mut missing = document("context");
    missing["input"][3]
        .as_object_mut()
        .unwrap()
        .remove("output");
    assert_eq!(
        decode_ingress_request(IngressProtocol::Responses, &missing).unwrap_err(),
        ModelIrError::InvalidField("output")
    );
    for field in [
        "api_key",
        "authorization",
        "base_url",
        "headers",
        "endpoint",
        "credentials",
    ] {
        let mut body = document("context");
        body["input"][3][field] = json!("not-authorized");
        assert!(
            decode_ingress_request(IngressProtocol::Responses, &body).is_err(),
            "{field}"
        );
    }
}

#[test]
fn codex_goal_context_replays_complete_large_text_without_gaining_portability() {
    let original = document(&"Quoted \"goal\" 中文\n".repeat(3000));
    let mut body = original.clone();
    let root = std::env::temp_dir().join(format!(
        "hiroute-goal-context-{}-{}",
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
    let budget = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024)
        .unwrap()
        .stream(4 * 1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    compact_ingress_document(IngressProtocol::Responses, &mut body, &store).unwrap();
    let mut request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    externalize_model_request(&mut request, &store, 0).unwrap();
    assert!(!model_content_refs(&request).is_empty());
    store.prevalidate(&model_content_refs(&request)).unwrap();
    let profile = exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
    let template = project_candidate_request_template(&request, &profile).unwrap();
    let mut reader = sequential_attempt_body(template, store.clone(), &budget, 257).unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        bytes.extend_from_slice(chunk.bytes());
    }
    let wire: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(wire["input"], original["input"]);
    for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
        let error = project_candidate_request_template(
            &request,
            &exact_state_profile(IngressProtocol::Responses, target),
        )
        .map(|_| ())
        .unwrap_err();
        assert_eq!(error.code(), "CLIENT_PROTOCOL_UNREPRESENTABLE");
    }
    drop((reader, request, store, manager));
    assert_eq!(budget.snapshot().unwrap().live, 0);
    fs_err_remove_dir(&root);
}
