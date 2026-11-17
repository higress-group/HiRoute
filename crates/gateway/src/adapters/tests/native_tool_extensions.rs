use super::*;

fn requests() -> Vec<(IngressProtocol, Value, [&'static str; 3])> {
    vec![
        (
            IngressProtocol::Responses,
            json!({"model":"alias","input":[
            {"role":"user","content":"Read the fixture"},
            {"type":"function_call","call_id":"call-1","name":"read","arguments":"{}"},
            {"type":"function_call_output","call_id":"call-1","output":"done"}
        ],"tools":[{"type":"function","name":"read","parameters":{"type":"object"}}]}),
            ["/tools/0", "/input/1", "/input/2"],
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"model":"alias","messages":[
            {"role":"user","content":"Read the fixture"},
            {"role":"assistant","tool_calls":[{"type":"function","id":"call-1",
                "function":{"name":"read","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"call-1","content":"done"}
        ],"tools":[{"type":"function","function":{"name":"read","parameters":{"type":"object"}}}]}),
            [
                "/tools/0/function",
                "/messages/1/tool_calls/0",
                "/messages/2",
            ],
        ),
        (
            IngressProtocol::Messages,
            json!({"model":"alias","max_tokens":128,"messages":[
            {"role":"user","content":"Read the fixture"},
            {"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"read","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"done"}]}
        ],"tools":[{"name":"read","input_schema":{"type":"object"}}]}),
            ["/tools/0", "/messages/1/content/0", "/messages/2/content/0"],
        ),
    ]
}

#[test]
fn ordinary_tool_extensions_remain_native_across_all_protocols_and_models() {
    for (protocol, original, positions) in requests() {
        for position in positions {
            let mut body = original.clone();
            body.pointer_mut(position)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(
                    "provider_options".into(),
                    json!({"delivery":"provider-owned"}),
                );
            let request = decode_ingress_request(protocol, &body).unwrap();
            assert!(request.native_only, "{protocol:?} {position}");
            assert!(crate::context_hold::visible_history(&request, &[0; 32]).is_none());
            let history = if protocol == IngressProtocol::Responses {
                "input"
            } else {
                "messages"
            };
            for model in ["original-model", "replacement-model"] {
                let mut native = exact_state_profile(protocol, protocol);
                native.capability.native_model = model.into();
                native.capability.request.function_tools = Fidelity::Unsupported;
                native.capability.response.tool_calls = Fidelity::Unknown;
                let projected = project_candidate_request(&request, &native).unwrap();
                assert_eq!(projected.body["model"], model);
                assert_eq!(projected.body["tools"], body["tools"]);
                assert_eq!(projected.body[history], body[history]);
                assert!(
                    project_candidate_request_template_with_cleanup(&request, &native, Some(1))
                        .is_err()
                );
            }
            for target in [
                IngressProtocol::Responses,
                IngressProtocol::ChatCompletions,
                IngressProtocol::Messages,
            ] {
                if target != protocol {
                    assert!(
                        project_candidate_request(&request, &exact_state_profile(protocol, target))
                            .is_err()
                    );
                }
            }
            for field in [
                "api_key",
                "authorization",
                "base_url",
                "endpoint",
                "headers",
                "credentials",
            ] {
                let mut denied = original.clone();
                denied
                    .pointer_mut(position)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(field.into(), json!("not-authorized"));
                assert!(
                    decode_ingress_request(protocol, &denied).is_err(),
                    "{protocol:?} {position} {field}"
                );
            }
        }
    }
}
