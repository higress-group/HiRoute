//! Native forwarding does not certify conflicting provider tool metadata.
use super::*;
use crate::server::core_runtime::{
    adapters::{NativeResponseDecoder, ProtocolAdapterError},
    model_ir::ModelIrError,
};

fn response_stream(name: &str, namespace: Option<&str>, conflicting_arguments: bool) -> Vec<u8> {
    let arguments = r#"{"cmd":"printf current"}"#;
    let mut done = json!({"type":"function_call","id":"item-1","call_id":"call-1",
        "name":name,"arguments":arguments,"status":"completed"});
    if let Some(namespace) = namespace {
        done["namespace"] = json!(namespace);
    }
    let events = [
        json!({"type":"response.created","response":{"id":"response-1","model":"physical",
            "object":"response","status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{
            "type":"function_call","id":"item-1","call_id":"call-1","name":"exec_command",
            "arguments":"","status":"in_progress"}}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"item-1",
            "delta":if conflicting_arguments { r#"{"cmd":"printf previous"}"# } else { arguments }}),
        json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"item-1",
            "arguments":arguments}),
        json!({"type":"response.output_item.done","output_index":0,"item":done}),
        json!({"type":"response.completed","response":{"id":"response-1","model":"physical",
            "object":"response","status":"completed","output":[done]}}),
    ];
    events.iter().flat_map(|event| sse(None, event)).collect()
}

#[test]
fn native_response_tool_metadata_changes_forward_without_certifying_completion() {
    for (name, namespace) in [
        ("spawn_agent", None),
        ("exec_command", Some("collaboration")),
        ("spawn_agent", Some("collaboration")),
    ] {
        let wire = response_stream(name, namespace, false);
        for fragment in [1, 7, wire.len()] {
            let mut native = projector(IngressProtocol::Responses, true);
            let mut units = feed_fragmented(&mut native, &wire, fragment);
            units.extend(native.feed(&[], true).unwrap());
            assert_eq!(
                units.last().unwrap().terminal,
                Some(NativeTerminalOutcome::Unknown)
            );
            let values: Vec<_> = units
                .iter()
                .filter(|unit| !unit.bytes.is_empty())
                .map(unit_json)
                .collect();
            assert_eq!(values.len(), 6);
            // Names and namespaces are provider-owned. Neither a replacement
            // tool nor a repaired executable instruction is manufactured.
            assert_eq!(values[1]["item"]["name"], "exec_command");
            assert_eq!(values[4]["item"]["name"], name);
            assert_eq!(
                values[4]["item"].get("namespace").and_then(Value::as_str),
                namespace
            );
            assert_eq!(values[5]["response"]["output"][0]["name"], name);
            let mapped = &values[1]["item"]["call_id"];
            assert_eq!(mapped, "call-1");
            assert_eq!(&values[4]["item"]["call_id"], mapped);
            assert_eq!(&values[5]["response"]["output"][0]["call_id"], mapped);
            let (_, evidence) = native.take_completion_diagnostic().unwrap();
            assert_eq!(
                evidence.reason,
                hiroute_diagnostics::event::NativeCompletionReason::ItemSnapshot
            );
        }
    }
}

#[test]
fn native_response_tool_metadata_does_not_repair_conflicting_arguments_or_capture() {
    let valid = response_stream("exec_command", None, false);
    let mut decoder =
        NativeResponseDecoder::new(&profile(IngressProtocol::Responses), 200, true).unwrap();
    assert!(decoder.feed(&valid, true).is_ok());

    let wire = response_stream("spawn_agent", Some("collaboration"), true);
    let mut native = projector(IngressProtocol::Responses, true);
    let mut units = feed_fragmented(&mut native, &wire, 11);
    units.extend(native.feed(&[], true).unwrap());
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
    let values: Vec<_> = units
        .iter()
        .filter(|unit| !unit.bytes.is_empty())
        .map(unit_json)
        .collect();
    assert_eq!(values[2]["delta"], r#"{"cmd":"printf previous"}"#);
    assert_eq!(values[3]["arguments"], r#"{"cmd":"printf current"}"#);
    let mut decoder =
        NativeResponseDecoder::new(&profile(IngressProtocol::Responses), 200, true).unwrap();
    let error = decoder.feed(&wire, true).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("native final tool arguments disagree with deltas")
    );
}

#[test]
fn native_response_tool_metadata_alone_cannot_certify_canonical_capture() {
    for (name, namespace) in [
        ("spawn_agent", None),
        ("exec_command", Some("collaboration")),
    ] {
        let wire = response_stream(name, namespace, false);
        let mut decoder =
            NativeResponseDecoder::new(&profile(IngressProtocol::Responses), 200, true).unwrap();
        let error = decoder.feed(&wire, true).unwrap_err();
        assert!(
            matches!(error, ProtocolAdapterError::ModelIr(ModelIrError::MissingToolIdentity(ref index))
            if index == "responses index 0"),
            "{error}"
        );
    }
}

#[test]
fn native_response_tool_metadata_still_requires_stable_owned_call_identity() {
    let start =
        json!({"type":"response.created","response":{"id":"response-1","model":"physical"}});
    let added = json!({"type":"response.output_item.added","output_index":0,"item":{
        "type":"function_call","id":"item-1","call_id":"call-1","name":"exec_command","arguments":""}});
    for (index, call, kind) in [
        (0, "changed-call", "function_call"),
        (0, "call-1", "custom_tool_call"),
        (1, "call-1", "function_call"),
    ] {
        let mut native = projector(IngressProtocol::Responses, true);
        native.feed(&sse(None, &start), false).unwrap();
        native.feed(&sse(None, &added), false).unwrap();
        let changed = json!({"type":"response.output_item.done","output_index":index,"item":{
            "type":kind,"id":"item-1","call_id":call,"name":"exec_command","arguments":"{}"}});
        assert!(native.feed(&sse(None, &changed), false).is_err());
    }
    // The web-search carrier uses `id`, not `call_id`; shared canonical
    // Function kind does not authorize switching native identity fields.
    for reverse in [false, true] {
        let web = json!({"type":"response.output_item.added","output_index":0,"item":{
            "type":"web_search_call","id":"call-1","status":"in_progress"}});
        let mut native = projector(IngressProtocol::Responses, true);
        native.feed(&sse(None, &start), false).unwrap();
        let (first, changed) = if reverse {
            (&web, &added)
        } else {
            (&added, &web)
        };
        native.feed(&sse(None, first), false).unwrap();
        assert!(native.feed(&sse(None, changed), false).is_err());
    }
}

#[test]
fn native_response_tool_metadata_policy_does_not_change_other_protocols() {
    let mut messages = projector(IngressProtocol::Messages, true);
    messages
        .feed(
            &sse(
                None,
                &json!({"type":"message_start","message":{"id":"response-1","model":"physical"}}),
            ),
            false,
        )
        .unwrap();
    messages
        .feed(
            &sse(
                None,
                &json!({"type":"content_block_start","index":0,"content_block":{
        "type":"tool_use","id":"call-1","name":"first","input":{}}}),
            ),
            false,
        )
        .unwrap();
    assert!(
        messages
            .feed(
                &sse(
                    None,
                    &json!({"type":"content_block_start","index":0,"content_block":{
        "type":"tool_use","id":"call-1","name":"changed","input":{}}})
                ),
                false
            )
            .is_err()
    );

    let mut chat = projector(IngressProtocol::ChatCompletions, true);
    for (name, success) in [("first", true), ("changed", false)] {
        let chunk = json!({"id":"response-1","model":"physical","choices":[{"index":0,"delta":{
            "tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":name,"arguments":""}}]}}]});
        assert_eq!(chat.feed(&sse(None, &chunk), false).is_ok(), success);
    }
}
