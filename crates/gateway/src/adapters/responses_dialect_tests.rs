//! Synthetic content with the complete tool/schema shape produced by Codex 0.147 V2.
//! Protocol fixtures prove negotiation and continuation, not live model obedience.
use super::*;
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, fixed_reasoning,
};
use serde_json::{Value, json};

fn profile(standard: bool) -> CandidateProtocolProfile {
    let mut p = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    if standard {
        p.adapter_revision = hiroute_domain::STANDARD_RESPONSES_JSON_ADAPTER_V1.into();
    }
    p
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/codex-v2-request.json")).unwrap()
}
fn item(encoding: Option<Value>, status: &str, args: &str) -> Value {
    let mut call = json!({"type":"function_call","id":"fc_native","call_id":"call_native","namespace":"collaboration","name":"spawn_agent","arguments":args,"status":status});
    if let Some(value) = encoding {
        call["encrypted_function_args"] = value;
    }
    call
}
fn envelope(call: Value) -> Value {
    json!({"id":"response","model":"physical","status":"completed","output":[call],"usage":{"input_tokens":3,"output_tokens":2}})
}
fn decode(
    p: &CandidateProtocolProfile,
    body: &Value,
) -> Result<DecodedNativeResponse, ProtocolAdapterError> {
    let mut d = NativeResponseDecoder::new_for_observation(
        p,
        200,
        false,
        ToolIdProjection::new(IngressProtocol::Responses),
        None,
    )?;
    d.feed(&serde_json::to_vec(body).unwrap(), true)?;
    d.finish()
}
fn json_output(p: &CandidateProtocolProfile, decoded: &DecodedNativeResponse) -> Value {
    let RenderedClientResponse::Json { body, .. } =
        ClientResponseRenderer::render_nonstream_with_profile(
            &ClientProtocolProfile::for_candidate(p).unwrap(),
            "alias",
            &decoded.response,
        )
        .unwrap()
    else {
        panic!("JSON")
    };
    body
}

#[test]
fn codex_v2_full_tool_request_negotiates_without_dropping_native_declarations() {
    let original = fixture();
    let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    let prepared = project_candidate_request(&request, &profile(true)).unwrap();
    let mut expected = original.clone();
    expected["model"] = json!("physical");
    for i in [0, 3, 4] {
        expected["tools"][5]["tools"][i]["parameters"]["properties"]["message"]
            .as_object_mut()
            .unwrap()
            .remove("encrypted");
    }
    // Every original tool, namespace description, hosted search and native option survives.
    assert_eq!(prepared.body, expected);
    assert_eq!(request.native_body.as_ref().unwrap(), &original);
    assert_eq!(
        project_candidate_request(&request, &profile(false))
            .unwrap()
            .body["tools"],
        original["tools"]
    );
}

#[test]
fn codex_v2_schema_annotation_is_not_a_business_property_or_example_key() {
    let mut body = fixture();
    body["tools"][0]["parameters"] = json!({"type":"object","properties":{"encrypted":{"type":"boolean"},"message":{"type":"string","encrypted":true,"default":"text"}},"examples":[{"encrypted":true}]});
    let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    let output = project_candidate_request(&request, &profile(true)).unwrap();
    assert_eq!(
        output.body["tools"][0]["parameters"]["properties"]["encrypted"],
        json!({"type":"boolean"})
    );
    assert_eq!(
        output.body["tools"][0]["parameters"]["examples"],
        body["tools"][0]["parameters"]["examples"]
    );
    body["tools"][0]["parameters"]["properties"]["message"]["encrypted"] = json!("yes");
    let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    assert!(project_candidate_request(&request, &profile(true)).is_err());
}

#[test]
fn codex_v2_plain_peer_preserves_author_recipient_order_and_never_becomes_user() {
    let mut body = fixture();
    body["input"].as_array_mut().unwrap().push(json!({"type":"agent_message","id":"peer_original","author":"parent","recipient":"child","content":[{"type":"input_text","text":"Read the file"},{"type":"input_text","text":"Return its exact contents"}]}));
    let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    assert!(
        request.native_only,
        "original peer origin remains native, not fabricated human history"
    );
    let prepared = project_candidate_request(&request, &profile(true)).unwrap();
    let peer = &prepared.body["input"][1];
    assert_eq!(peer["role"], "assistant");
    assert_eq!(
        serde_json::from_str::<Value>(peer["content"][0]["text"].as_str().unwrap()).unwrap(),
        json!({"type":"agent_message","id":"peer_original","author":"parent","recipient":"child"})
    );
    assert_eq!(peer["content"][1]["text"], "Read the file");
    assert_eq!(peer["content"][2]["text"], "Return its exact contents");
    assert_eq!(
        project_candidate_request(&request, &profile(false))
            .unwrap()
            .body["input"],
        body["input"]
    );
    for content in [
        json!([{"type":"encrypted_content","encrypted_content":"readable is not plaintext proof"}]),
        json!([{"type":"input_text","text":"a"},{"type":"encrypted_content","encrypted_content":"b"}]),
    ] {
        body["input"][1]["content"] = content;
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        assert!(project_candidate_request(&request, &profile(true)).is_err());
        assert!(project_candidate_request(&request, &profile(false)).is_ok());
    }
}

#[test]
fn codex_v2_json_three_states_survive_native_decode_render_and_replay() {
    for encoding in [None, Some(json!([])), Some(json!(["message"]))] {
        let p = profile(false);
        let decoded = decode(
            &p,
            &envelope(item(
                encoding.clone(),
                "completed",
                r#"{"message":"opaque-or-plain"}"#,
            )),
        )
        .unwrap();
        let output = json_output(&p, &decoded);
        assert_eq!(
            output["output"][0].get("encrypted_function_args"),
            encoding.as_ref()
        );
        let request = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"alias","input":output["output"]}),
        )
        .unwrap();
        let replay = project_candidate_request(&request, &p).unwrap();
        assert_eq!(
            replay.body["input"][0].get("encrypted_function_args"),
            encoding.as_ref()
        );
        assert_eq!(replay.body["input"][0]["call_id"], "call_native");
        assert_eq!(replay.body["input"][0]["namespace"], "collaboration");
        if encoding
            .as_ref()
            .is_some_and(|v| v.as_array().unwrap().is_empty())
        {
            assert!(!request.native_only);
        }
    }
    for malformed in [Value::Null, json!(true), json!([1]), json!("message")] {
        assert!(
            decode(
                &profile(false),
                &envelope(item(Some(malformed), "completed", "{}"))
            )
            .is_err()
        );
    }
}

#[test]
fn codex_v2_standard_json_declaration_is_explicit_and_contradictions_fail() {
    let p = profile(true);
    let decoded = decode(
        &p,
        &envelope(item(None, "completed", r#"{"message":"read"}"#)),
    )
    .unwrap();
    let output = json_output(&p, &decoded);
    assert_eq!(output["output"][0]["encrypted_function_args"], json!([]));
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":output["output"]}),
    )
    .unwrap();
    assert!(!request.native_only);
    assert!(
        project_candidate_request(&request, &p).unwrap().body["input"][0]
            .get("encrypted_function_args")
            .is_none()
    );
    assert!(
        decode(
            &p,
            &envelope(item(Some(json!(["message"])), "completed", "{}"))
        )
        .is_err()
    );
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":[item(Some(json!(["message"])),"completed","{}")]}),
    )
    .unwrap();
    assert!(project_candidate_request(&request, &p).is_err());
}

fn lifecycle(encoding: Option<Value>) -> Vec<Value> {
    let arguments = r#"{"message":"read fixture"}"#;
    vec![
        json!({"type":"response.created","response":{"id":"response","model":"physical","status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":item(encoding.clone(),"in_progress","")}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc_native","delta":arguments}),
        json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"fc_native","arguments":arguments}),
        json!({"type":"response.output_item.done","output_index":0,"item":item(encoding.clone(),"completed",arguments)}),
        json!({"type":"response.completed","response":envelope(item(encoding,"completed",arguments))}),
    ]
}
fn wire(events: &[Value]) -> Vec<u8> {
    events
        .iter()
        .flat_map(|v| {
            format!("event: {}\ndata: {}\n\n", v["type"].as_str().unwrap(), v).into_bytes()
        })
        .collect()
}

#[test]
fn codex_v2_complete_sse_matches_json_and_native_projection_at_all_fragmentations() {
    for standard in [false, true] {
        for encoding in [None, Some(json!([])), Some(json!(["message"]))] {
            if standard && encoding == Some(json!(["message"])) {
                continue;
            }
            let p = profile(standard);
            let expected = if standard {
                Some(json!([]))
            } else {
                encoding.clone()
            };
            let events = lifecycle(encoding);
            let bytes = wire(&events);
            for chunk in [1, 31, bytes.len()] {
                let mut decoder = NativeResponseDecoder::new_for_observation(
                    &p,
                    200,
                    true,
                    ToolIdProjection::new(IngressProtocol::Responses),
                    None,
                )
                .unwrap();
                let mut renderer = IncrementalClientSseRenderer::new(
                    ClientProtocolProfile::for_candidate(&p).unwrap(),
                    "alias",
                );
                let mut projector = NativeResponseProjector::new_for_observation(
                    &p,
                    true,
                    "alias".into(),
                    None,
                    ToolIdProjection::new(IngressProtocol::Responses),
                )
                .unwrap();
                let mut rendered = Vec::new();
                let mut projected = Vec::new();
                for part in bytes.chunks(chunk) {
                    decoder.feed(part, false).unwrap();
                    for event in decoder.take_events() {
                        rendered.extend(renderer.push(&event).unwrap());
                    }
                    projected.extend(projector.feed(part, false).unwrap());
                }
                decoder.feed(&[], true).unwrap();
                for event in decoder.take_events() {
                    rendered.extend(renderer.push(&event).unwrap());
                }
                projected.extend(projector.feed(&[], true).unwrap());
                let decoded = decoder.finish().unwrap();
                assert_eq!(
                    json_output(&p, &decoded)["output"][0].get("encrypted_function_args"),
                    expected.as_ref()
                );
                let done = rendered
                    .iter()
                    .find(|v| v.event.as_deref() == Some("response.output_item.done"))
                    .unwrap();
                assert_eq!(
                    done.data["item"].get("encrypted_function_args"),
                    expected.as_ref()
                );
                let native = projected
                    .iter()
                    .find_map(|unit| {
                        let text = std::str::from_utf8(&unit.bytes).unwrap();
                        let v: Value = serde_json::from_str(
                            text.lines().find_map(|line| line.strip_prefix("data: "))?,
                        )
                        .ok()?;
                        (v["type"] == "response.output_item.done").then_some(v)
                    })
                    .unwrap();
                assert_eq!(
                    native["item"].get("encrypted_function_args"),
                    expected.as_ref()
                );
                assert_eq!(
                    projected.last().unwrap().terminal,
                    Some(NativeTerminalOutcome::Complete)
                );
            }
        }
    }
}

#[test]
fn codex_v2_late_cipher_declaration_cannot_be_laundered_by_terminal_success() {
    for standard in [false, true] {
        let p = profile(standard);
        let mut events = lifecycle(Some(json!([])));
        events[4]["item"]["encrypted_function_args"] = json!(["message"]);
        let mut decoder = NativeResponseDecoder::new_for_observation(
            &p,
            200,
            true,
            ToolIdProjection::new(IngressProtocol::Responses),
            None,
        )
        .unwrap();
        assert!(decoder.feed(&wire(&events), true).is_err());
        let mut projector = NativeResponseProjector::new_for_observation(
            &p,
            true,
            "alias".into(),
            None,
            ToolIdProjection::new(IngressProtocol::Responses),
        )
        .unwrap();
        assert!(projector.feed(&wire(&events), true).is_err());
    }
}
