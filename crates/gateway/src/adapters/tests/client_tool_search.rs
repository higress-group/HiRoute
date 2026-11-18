use super::*;
use crate::server::core_runtime::model_ir::NativeHandoffItem;
use hiroute_diagnostics::event::RequestProjectionReason;
use serde_json::Value;

fn declaration() -> Value {
    json!({"type":"tool_search","execution":"client","parameters":{"type":"object"},"description":"Lookup"})
}

fn history() -> Vec<Value> {
    vec![
        json!({"role":"user","content":"Look up the parcel"}),
        json!({"type":"tool_search_call","id":"ts-1","call_id":"call-search","execution":"client",
               "arguments":{"query":"shipping"},"status":"completed"}),
        json!({"type":"tool_search_output","call_id":"call-search","execution":"client","status":"completed",
        "tools":[{"type":"namespace","name":"shipping","tools":[
            {"type":"function","name":"lookup","description":null,"strict":null,"parameters":{"type":"object"}}
        ]}]}),
        json!({"type":"function_call","namespace":"shipping","call_id":"call-lookup","name":"lookup","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"call-lookup","output":"confirmed"}),
    ]
}

#[test]
fn discovery_roundtrip_preserves_positions_call_identity_and_native_tool_definitions() {
    for length in [1, 2, 3, 5] {
        for declare in [false, true] {
            if !declare && length == 1 {
                continue;
            }
            let mut body = json!({"model":"route","input":&history()[..length]});
            if declare {
                body["tools"] = json!([declaration()]);
            }
            let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
            assert!(request.responses_client_tool_search && request.native_only);
            assert!(request.tools.is_empty() && request.tool_namespaces.is_empty());
            assert!(request.web_search.is_none());
            if length >= 3 {
                let layout = request.native_history_layout().unwrap();
                assert_eq!(layout[1].handoff, NativeHandoffItem::ToolSearchCall);
                assert_eq!(layout[2].handoff, NativeHandoffItem::ToolSearchOutput);
                assert!(!layout[1].modeled && !layout[2].modeled);
            }
            let native =
                exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
            let template = project_candidate_request_template(&request, &native).unwrap();
            let wire: Value = serde_json::from_slice(&template.bytes).unwrap();
            assert_eq!(wire["tools"], body["tools"]);
            assert_eq!(wire["input"], body["input"]);
            for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
                let profile = exact_state_profile(IngressProtocol::Responses, target);
                assert!(matches!(
                    project_candidate_request_template(&request, &profile),
                    Err(ProtocolAdapterError::RequestUnrepresentable {
                        reason: RequestProjectionReason::ResponsesToolSearchUnsupported,
                        ..
                    })
                ));
            }
        }
    }
}

#[test]
fn discovery_rejects_unadmitted_execution_and_invalid_client_call_identity() {
    for tool in [
        json!({"type":"tool_search"}),
        json!({"type":"tool_search","execution":"server"}),
        json!({"type":"tool_search","execution":"client","headers":{}}),
    ] {
        assert!(
            decode_ingress_request(
                IngressProtocol::Responses,
                &json!({"model":"route","input":"ok","tools":[tool]})
            )
            .is_err()
        );
    }
    for tool in [
        json!({"type":"web_search"}),
        json!({"type":"mcp","server_url":"https://example.invalid"}),
        json!({"type":"namespace","name":"n","tools":[{"type":"computer"}]}),
        json!({"type":"function","name":"f","execution":"server"}),
        json!({"type":"function","name":"f","headers":{}}),
    ] {
        let mut items = history();
        items[2]["tools"] = json!([tool]);
        assert!(
            decode_ingress_request(
                IngressProtocol::Responses,
                &json!({"model":"route","input":items})
            )
            .is_err()
        );
    }
    for index in [1, 2] {
        for bad in [Value::Null, json!(" "), json!(42)] {
            let mut items = history();
            items[index]["call_id"] = bad;
            assert!(
                decode_ingress_request(
                    IngressProtocol::Responses,
                    &json!({"model":"route","input":items})
                )
                .is_err()
            );
        }
    }
    let mut items = history();
    items[1]["arguments"] = json!("{}");
    assert!(
        decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"route","input":items})
        )
        .is_err()
    );
}
