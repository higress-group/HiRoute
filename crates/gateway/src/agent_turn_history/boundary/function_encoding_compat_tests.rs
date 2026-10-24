//! Compatibility is an uncertain history edge; original encoding digests stay exact.
use super::*;
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::core_runtime::model_ir::ModelIrError;
use crate::server::core_runtime::profiles::{Fidelity, NativeProviderStateEmission};

fn output(encoding: Option<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut output = vec![
        json!({"type":"reasoning","id":"rs","status":"completed","content":null,
            "encrypted_content":null,"summary":[{"type":"summary_text","text":"Keep this reasoning."}]}),
        json!({"type":"message","id":"msg","role":"assistant","status":"completed",
            "content":[{"type":"output_text","text":"Keep this text."}]}),
        json!({"type":"function_call","id":"fc-1","call_id":"call-1","namespace":"tools",
            "name":"lookup","status":"completed","arguments":json!({"z":3,"a":{"x":1,"y":2},
                "payload":"synthetic argument ".repeat(1024)}).to_string()}),
        json!({"type":"function_call","id":"fc-2","call_id":"call-2","namespace":"tools",
            "name":"verify","status":"completed","arguments":"\"payload\""}),
    ];
    for call in &mut output[2..] {
        if let Some(encoding) = &encoding {
            call["encrypted_function_args"] = encoding.clone();
        }
    }
    output
}

fn capture(output: &[serde_json::Value]) -> BoundaryCapture {
    let mut p = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    p.capability.native_provider_state = NativeProviderStateEmission::Native;
    p.capability.request.provider_state = Fidelity::Exact;
    p.capability.response.provider_state = Fidelity::Exact;
    let mut decoder = NativeResponseDecoder::new_for_observation(
        &p,
        200,
        false,
        ToolIdProjection::new(IngressProtocol::Responses),
        None,
    )
    .unwrap();
    decoder
        .feed(
            &serde_json::to_vec(&json!({"id":"accepted","model":"physical",
        "status":"completed","output":output,"usage":{"input_tokens":1,"output_tokens":1}}))
            .unwrap(),
            true,
        )
        .unwrap();
    let decoded = decoder.finish().unwrap();
    let mut capture = BoundaryCapture::default();
    for event in &decoded.events {
        capture.accept(&event.event);
    }
    assert!(capture.evidence().is_some());
    capture
}

fn wire(output: Vec<serde_json::Value>) -> serde_json::Value {
    let mut input = output;
    input.push(json!({"role":"user","content":"next input"}));
    json!({"model":"alias","instructions":"instruction ".repeat(1024),"input":input})
}

#[test]
fn plaintext_function_encoding_compatibility_is_contextual_and_keeps_exact_digests() {
    for (source, replay_encoding) in [(None, Some(json!([]))), (Some(json!([])), None)] {
        let accepted = capture(&output(source.clone()));
        let mut items = output(replay_encoding);
        // JSON object key order is not tool argument identity.
        items[2]["arguments"] = json!(
            json!({"payload":"synthetic argument ".repeat(1024),
            "a":{"y":2,"x":1},"z":3})
            .to_string()
        );
        let body = wire(items);
        for inline in [0, 8192, 65536] {
            let (_dir, replay) = replay();
            let request = super::super::reasoning_boundary::production_request_for(
                IngressProtocol::Responses,
                &body,
                &replay,
                inline,
            );
            let original = request.clone();
            let incoming =
                incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default()).unwrap();
            assert_ne!(
                incoming_boundary(&request, 0, &replay, &mut FingerprintBudget::default()),
                accepted.digest()
            );
            assert_eq!(
                accepted.evidence().unwrap().matches(incoming),
                Some(true),
                "only contextual association may bridge absent and explicit plaintext encoding"
            );
            assert_eq!(request, original);
        }
    }
}

#[test]
fn plaintext_encoding_keeps_empty_reasoning_presence_and_position() {
    for summary in [json!([]), json!([{"type":"summary_text","text":""}])] {
        let mut source = output(Some(json!([])));
        source[0]["summary"] = summary.clone();
        let accepted = capture(&source);
        for changed in ["none", "omitted", "extra", "reordered"] {
            let mut items = output(None);
            items[0]["summary"] = summary.clone();
            match changed {
                "none" => {}
                "omitted" => {
                    items.remove(0);
                }
                "extra" => {
                    items.insert(0, items[0].clone());
                }
                "reordered" => items.swap(0, 1),
                _ => unreachable!(),
            }
            let body = wire(items);
            for inline in [0, 8192] {
                let (_dir, replay) = replay();
                let request = super::super::reasoning_boundary::production_request_for(
                    IngressProtocol::Responses,
                    &body,
                    &replay,
                    inline,
                );
                let original = request.clone();
                let matched =
                    incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default())
                        .and_then(|incoming| accepted.evidence().unwrap().matches(incoming));
                assert_eq!(
                    matched,
                    (changed == "none").then_some(true),
                    "empty reasoning {changed} at allowance {inline}"
                );
                assert_eq!(request, original);
            }
        }
    }
}

#[test]
fn plaintext_encoding_cannot_omit_an_empty_reasoning_item() {
    let mut source = output(Some(json!([])));
    source[0]["summary"] = json!([]);
    let accepted = capture(&source);
    let mut incoming = output(None);
    incoming.remove(0);
    let (_dir, replay) = replay();
    let request = super::super::reasoning_boundary::production_request_for(
        IngressProtocol::Responses,
        &wire(incoming),
        &replay,
        0,
    );
    let matched = incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default())
        .and_then(|incoming| accepted.evidence().unwrap().matches(incoming));
    assert_eq!(
        matched, None,
        "encoding compatibility retains the entire reasoning boundary"
    );
}

#[test]
fn plaintext_encoding_compatibility_binds_the_whole_ordered_accepted_boundary() {
    let accepted = capture(&output(Some(json!([]))));
    for changed in [
        "call_id",
        "name",
        "namespace",
        "kind",
        "arguments",
        "order",
        "half",
        "reasoning",
        "text",
        "unknown",
        "opaque",
        "encrypted",
    ] {
        let mut items = output(None);
        match changed {
            "call_id" => items[2]["call_id"] = json!("other-call"),
            "name" => items[2]["name"] = json!("other-tool"),
            "namespace" => items[2]["namespace"] = json!("other-namespace"),
            "kind" => {
                items[3] = json!({"type":"custom_tool_call","id":"fc-2","call_id":"call-2",
                "namespace":"tools","name":"verify","status":"completed","input":"payload"})
            }
            "arguments" => {
                items[2]["arguments"] = json!(
                    json!({"z":3,"a":{"x":1,"y":9},
                "payload":"synthetic argument ".repeat(1024)})
                    .to_string()
                )
            }
            "order" => items.swap(2, 3),
            "half" => {
                items.pop();
            }
            "reasoning" => items[0]["summary"][0]["text"] = json!("changed reasoning"),
            "text" => items[1]["content"][0]["text"] = json!("changed text"),
            "unknown" => items[0]["future_state"] = json!(true),
            "opaque" => items[0]["encrypted_content"] = json!("opaque state"),
            "encrypted" => items[2]["encrypted_function_args"] = json!(["payload"]),
            _ => unreachable!(),
        }
        let body = wire(items);
        for inline in [0, 8192] {
            let (_dir, replay) = replay();
            let request = super::super::reasoning_boundary::production_request_for(
                IngressProtocol::Responses,
                &body,
                &replay,
                inline,
            );
            let matched =
                incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default())
                    .and_then(|incoming| accepted.evidence().unwrap().matches(incoming));
            assert_eq!(matched, None, "changed {changed} at allowance {inline}");
        }
    }
}

#[test]
fn opaque_state_and_exhausted_budget_disable_plaintext_encoding_compatibility() {
    let (_dir, replay) = replay();
    for encrypted_tool in [false, true] {
        let mut source = output(Some(json!([])));
        let mut incoming = output(None);
        if encrypted_tool {
            source[2]["encrypted_function_args"] = json!(["payload"]);
        } else {
            source[0]["encrypted_content"] = json!("same opaque state");
            incoming[0]["encrypted_content"] = json!("same opaque state");
        }
        let accepted = capture(&source);
        let request = super::super::reasoning_boundary::production_request_for(
            IngressProtocol::Responses,
            &wire(incoming),
            &replay,
            0,
        );
        assert!(
            incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default())
                .and_then(|incoming| accepted.evidence().unwrap().matches(incoming))
                .is_none()
        );
    }
    let accepted = capture(&output(Some(json!([]))));
    let request = super::super::reasoning_boundary::production_request_for(
        IngressProtocol::Responses,
        &wire(output(None)),
        &replay,
        0,
    );
    let mut budget = FingerprintBudget::default();
    budget
        .bytes(crate::context_hold::fingerprint::MAX_BYTES)
        .unwrap();
    assert!(
        incoming_evidence(&request, 0, &replay, &mut budget)
            .and_then(|incoming| accepted.evidence().unwrap().matches(incoming))
            .is_none()
    );
}

#[test]
fn malformed_function_encoding_is_still_invalid_ingress() {
    for encoding in [
        serde_json::Value::Null,
        json!({}),
        json!(true),
        json!(7),
        json!("field"),
        json!([1]),
    ] {
        let error =
            decode_ingress_request(IngressProtocol::Responses, &wire(output(Some(encoding))))
                .unwrap_err();
        assert!(matches!(
            error,
            ModelIrError::InvalidField("encrypted_function_args" | "encrypted_function_args[]")
        ));
    }
}
