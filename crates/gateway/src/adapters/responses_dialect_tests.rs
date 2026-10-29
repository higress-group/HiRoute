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
fn negotiation(p: &CandidateProtocolProfile) -> FunctionEncodingProjection {
    let request = decode_ingress_request(IngressProtocol::Responses, &fixture()).unwrap();
    project_candidate_request(&request, p)
        .unwrap()
        .function_encoding_projection
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
    )?
    .with_function_encoding(negotiation(p));
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
    // A business property named `description` is itself a schema, not annotation text.
    // The real request has an object here; fixture sanitization may edit only its string.
    assert_eq!(
        original.pointer("/tools/3/parameters/properties/questions/items/properties/options/items/properties/description"),
        Some(&json!({"description":"Fixture description retained.","type":"string"}))
    );
    let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    let prepared = project_candidate_request(&request, &profile(true)).unwrap();
    let mut expected = original.clone();
    expected["model"] = json!("physical");
    // The portable profile caps output at 256 even when the client requests 4096.
    expected["max_output_tokens"] = json!(256);
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

#[test]
fn codex_v2_custom_provider_replays_plaintext_source_without_vendor_metadata() {
    let p = profile(true);
    let decoded = decode(
        &p,
        &envelope(item(None, "completed", r#"{"cmd":"read-owned-file"}"#)),
    )
    .unwrap();
    let returned = json_output(&p, &decoded);
    assert_eq!(returned["output"][0]["encrypted_function_args"], json!([]));

    // rust-v0.147.0 core/client.rs::build_responses_request clears this field
    // for non-OpenAI providers after the original response has been recorded.
    let mut historical = returned["output"][0].clone();
    historical
        .as_object_mut()
        .unwrap()
        .remove("encrypted_function_args");
    let result = json!({"type":"function_call_output","call_id":"call_native","output":"owned-file-contents"});
    let mut replay = fixture();
    replay["input"]
        .as_array_mut()
        .unwrap()
        .extend([historical.clone(), result.clone()]);
    let request = decode_ingress_request(IngressProtocol::Responses, &replay).unwrap();
    let original = request.native_body.as_ref().unwrap().clone();
    let prepared = project_candidate_request(&request, &p).unwrap();
    assert_eq!(
        prepared.function_encoding_projection,
        FunctionEncodingProjection::Plaintext
    );
    assert_eq!(prepared.body["input"][1], historical);
    assert_eq!(prepared.body["input"][2], result);
    assert_eq!(request.native_body.as_ref().unwrap(), &original);
    assert!(
        original["input"][1]
            .get("encrypted_function_args")
            .is_none()
    );
    assert_eq!(returned["output"][0]["encrypted_function_args"], json!([]));
    let native = project_candidate_request(&request, &profile(false)).unwrap();
    assert_eq!(native.body["input"], replay["input"]);

    // The absence allowance does not admit explicit encryption or malformed
    // declarations. In particular, raw null keeps its existing strict contract.
    for invalid in [
        json!(["cmd"]),
        Value::Null,
        json!(true),
        json!([1]),
        json!("cmd"),
    ] {
        let mut body = replay.clone();
        body["input"][1]["encrypted_function_args"] = invalid;
        if let Ok(request) = decode_ingress_request(IngressProtocol::Responses, &body) {
            assert!(project_candidate_request(&request, &p).is_err());
        }
    }
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
                .unwrap()
                .with_function_encoding(negotiation(&p));
                let mut renderer = IncrementalClientSseRenderer::new(
                    ClientProtocolProfile::for_candidate(&p).unwrap(),
                    "alias",
                )
                .unwrap();
                let mut projector = NativeResponseProjector::new_for_observation(
                    &p,
                    true,
                    "alias".into(),
                    None,
                    ToolIdProjection::new(IngressProtocol::Responses),
                )
                .unwrap()
                .with_function_encoding(negotiation(&p));
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

#[test]
fn codex_v2_plaintext_chat_and_messages_producers_negotiate_real_json_arguments() {
    for upstream in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
        let p = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            upstream,
            "physical",
            fixed_reasoning("fixed"),
        );
        let request=decode_ingress_request(IngressProtocol::Responses,&json!({"model":"alias","input":"read","max_output_tokens":128,"tools":[{"type":"function","name":"read","parameters":{"type":"object","properties":{"message":{"type":"string","encrypted":true}}}}]})).unwrap();
        let prepared = project_candidate_request(&request, &p).unwrap();
        let schema = if upstream == IngressProtocol::ChatCompletions {
            &prepared.body["tools"][0]["function"]["parameters"]
        } else {
            &prepared.body["tools"][0]["input_schema"]
        };
        assert!(schema["properties"]["message"].get("encrypted").is_none());
        let body = if upstream == IngressProtocol::ChatCompletions {
            json!({"id":"r","model":"physical","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"c","type":"function","function":{"name":"read","arguments":"{\"message\":\"read fixture\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1}})
        } else {
            json!({"id":"r","type":"message","model":"physical","role":"assistant","content":[{"type":"tool_use","id":"c","name":"read","input":{"message":"read fixture"}}],"stop_reason":"tool_use","usage":{"input_tokens":1,"output_tokens":1}})
        };
        let mut decoder = NativeResponseDecoder::new_for_observation(
            &p,
            200,
            false,
            ToolIdProjection::new(IngressProtocol::Responses),
            prepared.chat_tool_projection,
        )
        .unwrap()
        .with_function_encoding(prepared.function_encoding_projection);
        decoder
            .feed(&serde_json::to_vec(&body).unwrap(), true)
            .unwrap();
        let decoded = decoder.finish().unwrap();
        let output = json_output(&p, &decoded);
        assert_eq!(output["output"][0]["encrypted_function_args"], json!([]));
        assert_eq!(
            serde_json::from_str::<Value>(output["output"][0]["arguments"].as_str().unwrap())
                .unwrap(),
            json!({"message":"read fixture"})
        );
    }
}

#[path = "responses_dialect_reasoning_tests.rs"]
mod reasoning;
#[path = "responses_dialect_replay_tests.rs"]
mod replay;

#[test]
fn codex_v2_plain_standard_response_client_continues_without_codex_extension() {
    let p = profile(true);
    let original = json!({"model":"alias","input":[{"role":"user","content":"read"}],"tools":[{"type":"function","name":"read","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}]});
    let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    let prepared = project_candidate_request(&request, &p).unwrap();
    assert_eq!(
        prepared.function_encoding_projection,
        FunctionEncodingProjection::Preserve
    );
    let mut response = item(None, "completed", r#"{"path":"fixture"}"#);
    response["name"] = json!("read");
    response.as_object_mut().unwrap().remove("namespace");
    let bytes = serde_json::to_vec(&envelope(response)).unwrap();
    let mut decoder = NativeResponseDecoder::new_for_observation(
        &p,
        200,
        false,
        ToolIdProjection::new(IngressProtocol::Responses),
        None,
    )
    .unwrap()
    .with_function_encoding(prepared.function_encoding_projection);
    decoder.feed(&bytes, true).unwrap();
    let out = json_output(&p, &decoder.finish().unwrap());
    assert!(out["output"][0].get("encrypted_function_args").is_none());
    let mut replay = original;
    replay["input"]
        .as_array_mut()
        .unwrap()
        .push(out["output"][0].clone());
    replay["input"].as_array_mut().unwrap().push(
        json!({"type":"function_call_output","call_id":"call_native","output":"fixture contents"}),
    );
    let request = decode_ingress_request(IngressProtocol::Responses, &replay).unwrap();
    let next = project_candidate_request(&request, &p).unwrap();
    assert_eq!(
        next.function_encoding_projection,
        FunctionEncodingProjection::Preserve
    );
    assert_eq!(next.body["input"], replay["input"]);

    // Schema negotiation cannot invalidate ordinary historical function calls.
    // Codex 0.147 build_responses_request clears this optional metadata for
    // custom providers even when its original response/journal carried [].
    let mut v2 = fixture();
    v2["input"]
        .as_array_mut()
        .unwrap()
        .extend(replay["input"].as_array().unwrap()[1..].iter().cloned());
    let request = decode_ingress_request(IngressProtocol::Responses, &v2).unwrap();
    let next = project_candidate_request(&request, &p).unwrap();
    assert_eq!(
        next.function_encoding_projection,
        FunctionEncodingProjection::Plaintext
    );
    assert_eq!(next.body["input"][1], replay["input"][1]);
    assert_eq!(next.body["input"][2], replay["input"][2]);
    assert!(
        request.native_body.as_ref().unwrap()["input"][1]
            .get("encrypted_function_args")
            .is_none()
    );
    assert!(project_candidate_request(&request, &profile(false)).is_ok());
}

#[test]
fn codex_v2_request_bound_projection_does_not_leak_to_retries_or_other_requests() {
    let request = decode_ingress_request(IngressProtocol::Responses, &fixture()).unwrap();
    let standard = project_candidate_request(&request, &profile(true)).unwrap();
    let native = project_candidate_request(&request, &profile(false)).unwrap();
    let ordinary = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":"next"}),
    )
    .unwrap();
    let ordinary = project_candidate_request(&ordinary, &profile(true)).unwrap();
    for (p, projection, expected) in [
        (
            profile(true),
            standard.function_encoding_projection,
            Some(json!([])),
        ),
        (profile(false), native.function_encoding_projection, None),
        (profile(true), ordinary.function_encoding_projection, None),
        (
            profile(true),
            standard.function_encoding_projection,
            Some(json!([])),
        ),
    ] {
        let bytes = serde_json::to_vec(&envelope(item(None, "completed", "{}"))).unwrap();
        let mut projector = NativeResponseProjector::new_for_observation(
            &p,
            false,
            "alias".into(),
            None,
            ToolIdProjection::new(IngressProtocol::Responses),
        )
        .unwrap()
        .with_function_encoding(projection);
        let units = projector.feed(&bytes, true).unwrap();
        let rendered: Value = serde_json::from_slice(
            &units
                .iter()
                .flat_map(|u| u.bytes.iter().copied())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            rendered["output"][0].get("encrypted_function_args"),
            expected.as_ref()
        );
        assert_eq!(
            units.last().unwrap().terminal,
            Some(NativeTerminalOutcome::Complete)
        );
    }
    assert_eq!(request.native_body.as_ref().unwrap(), &fixture());
}

#[test]
fn codex_v2_unnegotiated_upstream_empty_encoding_is_projected_consistently() {
    let p = profile(true);
    let encoding = FunctionEncodingProjection::Preserve;
    for streaming in [false, true] {
        let bytes = if streaming {
            wire(&lifecycle(Some(json!([]))))
        } else {
            serde_json::to_vec(&envelope(item(Some(json!([])), "completed", "{}"))).unwrap()
        };
        let mut decoder = NativeResponseDecoder::new_for_observation(
            &p,
            200,
            streaming,
            ToolIdProjection::new(IngressProtocol::Responses),
            None,
        )
        .unwrap()
        .with_function_encoding(encoding);
        let mut projector = NativeResponseProjector::new_for_observation(
            &p,
            streaming,
            "alias".into(),
            None,
            ToolIdProjection::new(IngressProtocol::Responses),
        )
        .unwrap()
        .with_function_encoding(encoding);
        decoder.feed(&bytes, true).unwrap();
        let captured = json_output(&p, &decoder.finish().unwrap());
        assert!(
            captured["output"][0]
                .get("encrypted_function_args")
                .is_none()
        );
        let units = projector.feed(&bytes, true).unwrap();
        let delivered = units
            .iter()
            .flat_map(|u| u.bytes.iter().copied())
            .collect::<Vec<_>>();
        assert!(
            !std::str::from_utf8(&delivered)
                .unwrap()
                .contains("encrypted_function_args")
        );
        assert_eq!(
            units.last().unwrap().terminal,
            Some(NativeTerminalOutcome::Complete)
        );
    }
}
