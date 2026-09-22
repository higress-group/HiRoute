use serde_json::{Value, json};

use super::*;
use crate::server::core_runtime::model_ir::{ContentPart, ResponseBlock, ToolKindV1};
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, fixed_reasoning,
};
use crate::server::request_plan::IngressProtocol;

#[test]
fn responses_namespaced_tool_calls_roundtrip_trusted_namespace_and_native_ids() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let projection = ToolIdProjection::new(IngressProtocol::Responses);
    let mut decoder =
        NativeResponseDecoder::new_for_observation(&profile, 200, false, projection, None).unwrap();
    decoder
        .feed(
            &serde_json::to_vec(&json!({
                "id":"response","model":"physical","status":"completed",
                "output":[
                    {"type":"function_call","id":"item-a","call_id":"native-a","namespace":"group-a","name":"shared","arguments":"{\"value\":1}","status":"completed"},
                    {"type":"function_call","id":"item-b","call_id":"native-b","namespace":"group-b","name":"shared","arguments":"{\"value\":2}","status":"completed"}
                ],
                "usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}
            }))
            .unwrap(),
            true,
        )
        .unwrap();
    let decoded = decoder.finish().unwrap();
    let calls = decoded
        .response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ResponseBlock::ToolCall {
                logical_id,
                namespace,
                name,
                ..
            } => Some((logical_id.clone(), namespace.clone(), name.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1.as_deref(), Some("group-a"));
    assert_eq!(calls[1].1.as_deref(), Some("group-b"));
    assert_eq!(calls[0].2, "shared");
    assert_eq!(calls[1].2, "shared");
    assert_ne!(calls[0].0, calls[1].0);
    assert_eq!(decoded.response.tool_id_map[0].native_id, "native-a");
    assert_eq!(
        decoded.response.tool_id_map[0].namespace.as_deref(),
        Some("group-a")
    );

    let client_profile = ClientProtocolProfile::for_candidate(&profile).unwrap();
    let mut incremental =
        IncrementalClientSseRenderer::new(client_profile.clone(), "alias").unwrap();
    let mut streamed = Vec::new();
    for event in &decoded.events {
        streamed.extend(incremental.push(event).unwrap());
    }
    let streamed_calls = streamed
        .iter()
        .filter(|event| {
            matches!(
                event.event.as_deref(),
                Some("response.output_item.added") | Some("response.output_item.done")
            ) && event.data["item"]["type"] == "function_call"
        })
        .collect::<Vec<_>>();
    assert_eq!(streamed_calls.len(), 4);
    assert_eq!(streamed_calls[0].data["item"]["namespace"], "group-a");
    assert_eq!(streamed_calls[1].data["item"]["namespace"], "group-a");
    assert_eq!(streamed_calls[2].data["item"]["namespace"], "group-b");
    assert_eq!(streamed_calls[3].data["item"]["namespace"], "group-b");

    let RenderedClientResponse::Json { body, .. } =
        ClientResponseRenderer::render_nonstream_with_profile(
            &client_profile,
            "alias",
            &decoded.response,
        )
        .unwrap()
    else {
        panic!("expected JSON")
    };
    assert_eq!(body["output"][0]["namespace"], "group-a");
    assert_eq!(body["output"][1]["namespace"], "group-b");

    let continued = json!({
        "model":"alias",
        "input":[
            {"type":"function_call","call_id":calls[0].0,"namespace":"group-a","name":"shared","arguments":"{\"value\":1}"},
            {"type":"function_call_output","call_id":calls[0].0,"output":"done-a","status":"incomplete"},
            {"type":"function_call","call_id":calls[1].0,"namespace":"group-b","name":"shared","arguments":"{\"value\":2}"},
            {"type":"function_call_output","call_id":calls[1].0,"output":"done-b"}
        ]
    });
    let request = decode_ingress_request(IngressProtocol::Responses, &continued).unwrap();
    assert_eq!(
        request.messages[1].content[0],
        ContentPart::ToolResult {
            logical_id: calls[0].0.clone(),
            tool_kind: crate::server::core_runtime::model_ir::ToolKindV1::Function,
            output: crate::server::core_runtime::model_ir::ToolOutput::Text("done-a".into()),
            status: crate::server::core_runtime::model_ir::ToolResultStatusV1::Failed,
        }
    );
    let projected = project_candidate_request(&request, &profile).unwrap();
    assert_eq!(projected.body["input"][0]["call_id"], "native-a");
    assert_eq!(projected.body["input"][0]["namespace"], "group-a");
    assert_eq!(projected.body["input"][1]["call_id"], "native-a");
    assert_eq!(projected.body["input"][1]["status"], "incomplete");
    assert!(projected.body["input"][1].get("namespace").is_none());
    assert_eq!(projected.body["input"][2]["call_id"], "native-b");
    assert_eq!(projected.body["input"][2]["namespace"], "group-b");
}

#[test]
fn responses_namespaced_stream_keeps_one_identity_through_added_delta_done_and_completion() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let projection = ToolIdProjection::new(IngressProtocol::Responses);
    let mut decoder =
        NativeResponseDecoder::new_for_observation(&profile, 200, true, projection, None).unwrap();
    let status = decoder
        .feed(
            br#"event: response.created
data: {"type":"response.created","sequence_number":0,"response":{"id":"response","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"function_call","id":"item","call_id":"native","namespace":"generic-group","name":"generic-child","arguments":"","status":"in_progress"}}

event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","sequence_number":2,"item_id":"item","output_index":0,"delta":"{\"value\":1}"}

event: response.function_call_arguments.done
data: {"type":"response.function_call_arguments.done","sequence_number":3,"item_id":"item","output_index":0,"arguments":"{\"value\":1}"}

event: response.output_item.done
data: {"type":"response.output_item.done","sequence_number":4,"output_index":0,"item":{"type":"function_call","id":"item","call_id":"native","namespace":"generic-group","name":"generic-child","arguments":"{\"value\":1}","status":"completed"}}

event: response.completed
data: {"type":"response.completed","sequence_number":5,"response":{"id":"response","model":"physical","status":"completed","usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}}

"#,
            true,
        )
        .unwrap();
    assert_eq!(status, ResponseDecodeStatus::Terminal);
    let decoded = decoder.finish().unwrap();
    let (started_id, finished_id) =
        decoded
            .events
            .iter()
            .fold((None, None), |(started, finished), event| {
                match &event.event {
                    crate::server::core_runtime::model_ir::ModelEvent::ToolCallStarted {
                        logical_id,
                        namespace,
                        name,
                        ..
                    } => {
                        assert_eq!(namespace.as_deref(), Some("generic-group"));
                        assert_eq!(name, "generic-child");
                        (Some(logical_id.clone()), finished)
                    }
                    crate::server::core_runtime::model_ir::ModelEvent::ToolCallFinished {
                        logical_id,
                        namespace,
                        name,
                        ..
                    } => {
                        assert_eq!(namespace.as_deref(), Some("generic-group"));
                        assert_eq!(name, "generic-child");
                        (started, Some(logical_id.clone()))
                    }
                    _ => (started, finished),
                }
            });
    assert_eq!(started_id, finished_id);
    assert_eq!(started_id.unwrap(), "native");
    assert_eq!(decoded.response.tool_id_map.len(), 1);
    assert_eq!(
        decoded.response.tool_id_map[0].namespace.as_deref(),
        Some("generic-group")
    );

    let client_profile = ClientProtocolProfile::for_candidate(&profile).unwrap();
    let mut renderer = IncrementalClientSseRenderer::new(client_profile, "alias").unwrap();
    let rendered = decoded
        .events
        .iter()
        .flat_map(|event| renderer.push(event).unwrap())
        .collect::<Vec<_>>();
    let tool_snapshots = rendered
        .iter()
        .filter(|event| {
            matches!(
                event.event.as_deref(),
                Some("response.output_item.added") | Some("response.output_item.done")
            ) && event.data["item"]["type"] == "function_call"
        })
        .collect::<Vec<_>>();
    assert_eq!(tool_snapshots.len(), 2);
    for snapshot in tool_snapshots {
        assert_eq!(snapshot.data["item"]["namespace"], "generic-group");
        assert_eq!(snapshot.data["item"]["name"], "generic-child");
        assert_eq!(snapshot.data["item"]["call_id"], "native");
    }
    assert_eq!(
        rendered.last().unwrap().event.as_deref(),
        Some("response.completed")
    );
}

#[test]
fn chat_projection_reverses_namespaced_function_and_custom_calls_for_responses_clients() {
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"alias",
            "input":"goal",
            "tools":[
                {"type":"namespace","name":"records","tools":[
                    {"type":"function","name":"lookup","parameters":{"type":"object"},"strict":true}
                ]},
                {"type":"custom","name":"shell","description":"run a command"}
            ]
        }),
    )
    .unwrap();
    let projection = ChatToolProjection::for_request(&request).unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("fixed"),
    );
    let tool_id_projection = ToolIdProjection::new(IngressProtocol::Responses);
    let mut decoder = NativeResponseDecoder::new_for_observation(
        &profile,
        200,
        false,
        tool_id_projection,
        Some(projection.clone()),
    )
    .unwrap();
    decoder
        .feed(
            &serde_json::to_vec(&json!({
                "id":"chat-response",
                "object":"chat.completion",
                "model":"physical",
                "choices":[{
                    "index":0,
                    "message":{
                        "role":"assistant",
                        "content":null,
                        "tool_calls":[
                            {"id":"native-function","type":"function","function":{"name":"records__lookup","arguments":"{\"id\":7}"}},
                            {"id":"native-custom","type":"function","function":{"name":"shell","arguments":"{\"input\":\"echo ok\"}"}}
                        ]
                    },
                    "finish_reason":"tool_calls"
                }],
                "usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}
            }))
            .unwrap(),
            true,
        )
        .unwrap();
    let decoded = decoder.finish().unwrap();
    let calls = decoded
        .response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ResponseBlock::ToolCall {
                tool_kind,
                namespace,
                name,
                arguments,
                ..
            } => Some((*tool_kind, namespace.as_deref(), name.as_str(), arguments)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0],
        (
            ToolKindV1::Function,
            Some("records"),
            "lookup",
            &json!({"id":7})
        )
    );
    assert_eq!(
        calls[1],
        (
            ToolKindV1::Custom,
            None,
            "shell",
            &Value::String("echo ok".into())
        )
    );
    assert_eq!(decoded.response.tool_id_map[0].native_id, "native-function");
    assert_eq!(decoded.response.tool_id_map[0].kind, ToolKindV1::Function);
    assert_eq!(decoded.response.tool_id_map[1].native_id, "native-custom");
    assert_eq!(decoded.response.tool_id_map[1].kind, ToolKindV1::Custom);

    let continued = json!({
        "model":"alias",
        "input":[
            {"type":"function_call","call_id":decoded.response.tool_id_map[0].logical_id,"namespace":"records","name":"lookup","arguments":"{\"id\":7}"},
            {"type":"custom_tool_call","call_id":decoded.response.tool_id_map[1].logical_id,"name":"shell","input":"echo ok"},
            {"type":"function_call_output","call_id":decoded.response.tool_id_map[0].logical_id,"output":"record found"},
            {"type":"custom_tool_call_output","call_id":decoded.response.tool_id_map[1].logical_id,"output":"command done"}
        ],
        "parallel_tool_calls":true,
        "tool_choice":"auto",
        "tools":[
            {"type":"namespace","name":"records","tools":[
                {"type":"function","name":"lookup","parameters":{"type":"object"},"strict":true}
            ]},
            {"type":"custom","name":"shell","description":"run a command"}
        ]
    });
    let continued = decode_ingress_request(IngressProtocol::Responses, &continued).unwrap();
    let projected = project_candidate_request(&continued, &profile).unwrap();
    assert_eq!(projected.body["messages"].as_array().unwrap().len(), 3);
    assert_eq!(
        projected.body["messages"][0]["tool_calls"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        projected.body["messages"][0]["tool_calls"][0]["function"]["name"],
        "records__lookup"
    );
    assert_eq!(
        projected.body["messages"][0]["tool_calls"][1]["function"]["name"],
        "shell"
    );
    assert_eq!(
        projected.body["messages"][0]["tool_calls"][1]["function"]["arguments"],
        "{\"input\":\"echo ok\"}"
    );
    assert_eq!(projected.body["messages"][1]["role"], "tool");
    assert_eq!(
        projected.body["messages"][1]["tool_call_id"],
        "native-function"
    );
    assert_eq!(projected.body["messages"][2]["role"], "tool");
    assert_eq!(
        projected.body["messages"][2]["tool_call_id"],
        "native-custom"
    );

    let client = ClientProtocolProfile::for_candidate(&profile).unwrap();
    let RenderedClientResponse::Json { body, .. } =
        ClientResponseRenderer::render_nonstream_with_profile(&client, "alias", &decoded.response)
            .unwrap()
    else {
        panic!("expected JSON")
    };
    assert_eq!(body["output"][0]["type"], "function_call");
    assert_eq!(body["output"][0]["namespace"], "records");
    assert_eq!(body["output"][0]["name"], "lookup");
    assert_eq!(body["output"][1]["type"], "custom_tool_call");
    assert_eq!(body["output"][1]["name"], "shell");
    assert_eq!(body["output"][1]["input"], "echo ok");

    let mut unknown = NativeResponseDecoder::new_for_observation(
        &profile,
        200,
        false,
        super::response::test_tool_projection(),
        Some(projection),
    )
    .unwrap();
    let error = unknown
        .feed(
            &serde_json::to_vec(&json!({
                "id":"chat-response",
                "model":"physical",
                "choices":[{
                    "index":0,
                    "message":{"role":"assistant","content":null,"tool_calls":[
                        {"id":"native-unknown","type":"function","function":{"name":"unknown","arguments":"{}"}}
                    ]},
                    "finish_reason":"tool_calls"
                }]
            }))
            .unwrap(),
            true,
        )
        .unwrap_err();
    assert_eq!(error.code(), "PROTOCOL_SEMANTICS_UNSUPPORTED");
}

#[test]
fn chat_stream_buffers_split_custom_wrapper_and_emits_raw_responses_custom_input() {
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"alias",
            "input":"goal",
            "stream":true,
            "tools":[{"type":"custom","name":"shell"}]
        }),
    )
    .unwrap();
    let projection = ChatToolProjection::for_request(&request).unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new_for_observation(
        &profile,
        200,
        true,
        super::response::test_tool_projection(),
        Some(projection),
    )
    .unwrap();
    let status = decoder
        .feed(
            br#"data: {"id":"chat-response","model":"physical","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"native-custom","type":"function","function":{"name":"shell","arguments":"{\"in"}}]},"finish_reason":null}]}

data: {"id":"chat-response","model":"physical","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"put\":\"echo "}}]},"finish_reason":null}]}

data: {"id":"chat-response","model":"physical","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ok\"}"}}]},"finish_reason":"tool_calls"}]}

data: {"id":"chat-response","model":"physical","choices":[],"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}

data: [DONE]

"#,
            true,
        )
        .unwrap();
    assert_eq!(status, ResponseDecodeStatus::Terminal);
    let decoded = decoder.finish().unwrap();
    let ResponseBlock::ToolCall {
        tool_kind,
        namespace,
        name,
        arguments,
        ..
    } = &decoded.response.blocks[0]
    else {
        panic!("expected custom tool call")
    };
    assert_eq!(*tool_kind, ToolKindV1::Custom);
    assert_eq!(namespace, &None);
    assert_eq!(name, "shell");
    assert_eq!(arguments, &Value::String("echo ok".into()));

    let client = ClientProtocolProfile::for_candidate(&profile).unwrap();
    let mut renderer = IncrementalClientSseRenderer::new(client, "alias").unwrap();
    let rendered = decoded
        .events
        .iter()
        .flat_map(|event| renderer.push(event).unwrap())
        .collect::<Vec<_>>();
    let delta = rendered
        .iter()
        .find(|event| event.event.as_deref() == Some("response.custom_tool_call_input.delta"))
        .unwrap();
    assert_eq!(delta.data["delta"], "echo ok");
    let done = rendered
        .iter()
        .find(|event| event.event.as_deref() == Some("response.custom_tool_call_input.done"))
        .unwrap();
    assert_eq!(done.data["input"], "echo ok");
}
