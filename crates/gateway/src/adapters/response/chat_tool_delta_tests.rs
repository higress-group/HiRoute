//! Synthetic Chat tool deltas exercise native delivery, canonical decoding and replay.
use serde_json::{Value, json};

use super::native_passthrough::{NativeResponseProjector, NativeTerminalOutcome};
use super::{ClientResponseRenderer, NativeResponseDecoder, RenderedClientResponse};
use crate::server::core_runtime::adapters::{
    ToolIdProjection, decode_ingress_request, project_candidate_request,
};
use crate::server::core_runtime::model_ir::ContentPart;
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
use crate::server::request_plan::IngressProtocol;

fn profile() -> CandidateProtocolProfile {
    CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::ChatCompletions,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("fixed"),
    )
}

fn projector() -> NativeResponseProjector {
    NativeResponseProjector::new_for_observation(
        &profile(),
        true,
        "alias".into(),
        None,
        ToolIdProjection::new(IngressProtocol::ChatCompletions),
    )
    .unwrap()
}

fn chunk(call: Value, finish_reason: Value) -> Vec<u8> {
    let finished = !finish_reason.is_null();
    let mut value = json!({"id":"response-fixture","object":"chat.completion.chunk","model":"physical",
        "choices":[{"index":0,"delta":{"tool_calls":[call]},"finish_reason":finish_reason}]});
    if finished {
        value["usage"] = json!({"prompt_tokens":4,"completion_tokens":2,"total_tokens":6});
    }
    format!("data: {}\n\n", value).into_bytes()
}

fn opening() -> Vec<u8> {
    chunk(
        json!({"index":0,"id":"call-fixture","type":"function",
            "function":{"name":"inspect","arguments":""}}),
        Value::Null,
    )
}

#[test]
fn native_chat_identity_free_deltas_preserve_arguments_and_tool_result_roundtrip() {
    let argument_fragments = ["{\"path\":\"fixture.txt\",", "\"flags\":[true,7]}", ""];
    let expected = json!({"path":"fixture.txt","flags":[true,7]});
    let mut wire = opening();
    for (index, arguments) in argument_fragments.iter().enumerate() {
        let mut call = json!({"index":0,"function":{"arguments":arguments}});
        match index {
            0 => call["id"] = json!(""),
            1 => call["id"] = Value::Null,
            _ => {}
        }
        wire.extend(chunk(
            call,
            if index == argument_fragments.len() - 1 {
                json!("tool_calls")
            } else {
                Value::Null
            },
        ));
    }
    wire.extend_from_slice(b"data: [DONE]\n\n");

    for fragment_size in [1, 7, wire.len()] {
        let mut native = projector();
        let mut delivered = Vec::new();
        for bytes in wire.chunks(fragment_size) {
            delivered.extend(native.feed(bytes, false).unwrap());
        }
        delivered.extend(native.feed(&[], true).unwrap());
        assert_eq!(
            delivered.last().unwrap().terminal,
            Some(NativeTerminalOutcome::Complete)
        );
        let delivered_wire: Vec<u8> = delivered.into_iter().flat_map(|unit| unit.bytes).collect();
        let mut arguments = String::new();
        let mut client_id = String::new();
        let mut id_deltas = Vec::new();
        for line in std::str::from_utf8(&delivered_wire).unwrap().lines() {
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            if data.trim() == "[DONE]" {
                continue;
            }
            let value: Value = serde_json::from_str(data).unwrap();
            assert_eq!(value["model"], "alias");
            let call = &value["choices"][0]["delta"]["tool_calls"][0];
            id_deltas.push(call.get("id").cloned());
            // Chat SDKs append string deltas; the native decoder below also
            // tracks identity, so a decoder-only roundtrip would miss repeats.
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                client_id.push_str(id);
            }
            arguments.push_str(call["function"]["arguments"].as_str().unwrap());
        }
        assert_eq!(client_id, "call-fixture");
        assert_eq!(
            id_deltas,
            [
                Some(json!("call-fixture")),
                Some(json!("")),
                Some(Value::Null),
                None
            ]
        );
        assert_eq!(arguments, argument_fragments.concat());

        let mut decoder = NativeResponseDecoder::new(&profile(), 200, true).unwrap();
        decoder.feed(&delivered_wire, true).unwrap();
        let decoded = decoder.finish().unwrap();
        let RenderedClientResponse::Json { body, .. } = ClientResponseRenderer::render_nonstream(
            IngressProtocol::ChatCompletions,
            "alias",
            &decoded.response,
        )
        .unwrap() else {
            panic!("Chat JSON response")
        };
        let call = &body["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["id"], client_id);
        assert_eq!(call["function"]["name"], "inspect");
        assert_eq!(
            serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap()).unwrap(),
            expected
        );
        let followup = json!({"model":"alias","tools":[{"type":"function","function":{
            "name":"inspect","parameters":{"type":"object"}}}],"messages":[
            body["choices"][0]["message"].clone(),
            {"role":"tool","tool_call_id":client_id,"content":"fixture result"}
        ]});
        let request = decode_ingress_request(IngressProtocol::ChatCompletions, &followup)
            .expect("native client replays the delivered tool call and correlated result");
        assert!(matches!(&request.messages[0].content[0],
            ContentPart::ToolCall { arguments, .. } if arguments == &expected));
        let replay = project_candidate_request(&request, &profile()).unwrap();
        assert_eq!(
            replay.body["messages"][1]["tool_call_id"],
            replay.body["messages"][0]["tool_calls"][0]["id"]
        );
        assert_eq!(
            serde_json::from_str::<Value>(
                replay.body["messages"][0]["tool_calls"][0]["function"]["arguments"]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
            expected
        );
    }
}

#[test]
fn native_chat_empty_id_deltas_do_not_invent_or_replace_tool_identity() {
    for (prefix, delta) in [
        (
            vec![],
            json!({"index":0,"id":"","function":{"name":"inspect","arguments":"{}"}}),
        ),
        (
            vec![opening()],
            json!({"index":1,"id":"","function":{"arguments":"{}"}}),
        ),
        (
            vec![opening()],
            json!({"index":0,"id":"different-call","function":{"arguments":"{}"}}),
        ),
        (
            vec![opening()],
            json!({"index":0,"id":"","function":{"name":"different-tool","arguments":"{}"}}),
        ),
    ] {
        let mut native = projector();
        for bytes in prefix {
            native.feed(&bytes, false).unwrap();
        }
        assert!(native.feed(&chunk(delta, Value::Null), false).is_err());
    }
}

#[test]
fn canonical_chat_rejects_partial_tool_identity_changes() {
    for delta in [
        json!({"index":0,"id":"different-call","function":{"arguments":"{}"}}),
        json!({"index":0,"function":{"name":"different-tool","arguments":"{}"}}),
    ] {
        let mut decoder = NativeResponseDecoder::new(&profile(), 200, true).unwrap();
        decoder.feed(&opening(), false).unwrap();
        assert!(decoder.feed(&chunk(delta, Value::Null), false).is_err());
    }
}

#[test]
fn native_and_canonical_chat_reject_invalid_tool_identity_types() {
    for invalid in [json!(7), json!(false), json!([]), json!({})] {
        for delta in [
            json!({"index":0,"id":invalid,"function":{"arguments":"{}"}}),
            json!({"index":0,"function":{"name":invalid,"arguments":"{}"}}),
        ] {
            let bytes = chunk(delta, Value::Null);
            let mut native = projector();
            native.feed(&opening(), false).unwrap();
            assert!(native.feed(&bytes, false).is_err());
            let mut canonical = NativeResponseDecoder::new(&profile(), 200, true).unwrap();
            canonical.feed(&opening(), false).unwrap();
            assert!(canonical.feed(&bytes, false).is_err());
        }
    }
}

#[test]
fn native_and_canonical_chat_treat_null_identity_as_bound_delta_only() {
    let tail = chunk(
        json!({"index":0,"id":null,"function":{"name":null,"arguments":"{}"}}),
        json!("tool_calls"),
    );
    let wire = [opening(), tail, b"data: [DONE]\n\n".to_vec()].concat();
    let mut native = projector();
    let units = native.feed(&wire, true).unwrap();
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Complete)
    );
    let mut canonical = NativeResponseDecoder::new(&profile(), 200, true).unwrap();
    canonical.feed(&wire, true).unwrap();
    assert!(canonical.finish().is_ok());
    for delta in [
        json!({"index":0,"id":null,"function":{"name":"inspect","arguments":"{}"}}),
        json!({"index":0,"id":"call-fixture","function":{"name":null,"arguments":"{}"}}),
    ] {
        let bytes = chunk(delta, Value::Null);
        assert!(projector().feed(&bytes, false).is_err());
        let mut canonical = NativeResponseDecoder::new(&profile(), 200, true).unwrap();
        assert!(canonical.feed(&bytes, false).is_err());
    }
}
