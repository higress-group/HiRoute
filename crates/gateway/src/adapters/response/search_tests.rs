use super::*;
use crate::server::core_runtime::adapters::{
    IngressRequestBindings, decode_ingress_request_with_bindings, project_candidate_request,
};
use crate::server::core_runtime::model_ir::ResponseBlock;
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, fixed_reasoning,
};
use serde_json::json;

fn candidate() -> CandidateProtocolProfile {
    CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    )
}

fn events() -> Vec<Value> {
    vec![
        json!({"type":"response.created","response":{"id":"r","model":"physical"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"web_search_call","id":"ws_native","status":"in_progress"}}),
        json!({"type":"response.web_search_call.in_progress","output_index":0,"item_id":"ws_native"}),
        json!({"type":"response.web_search_call.searching","output_index":0,"item_id":"ws_native"}),
        json!({"type":"response.web_search_call.completed","output_index":0,"item_id":"ws_native"}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"type":"web_search_call","id":"ws_native","status":"completed","action":{"type":"search","queries":["fixture query"],"sources":[{"type":"url","url":"https://example.com/search"}]}}}),
        json!({"type":"response.completed","response":{"id":"r","model":"physical","status":"completed","usage":{"input_tokens":3,"output_tokens":2}}}),
    ]
}

#[test]
fn search_stream_replays_with_trusted_owner_and_original_action() {
    let wire: Vec<u8> = events()
        .iter()
        .flat_map(|event| {
            format!(
                "event: {}\ndata: {}\n\n",
                event["type"].as_str().unwrap(),
                event
            )
            .into_bytes()
        })
        .collect();
    let candidate = candidate();
    let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
    for size in [1, 17, wire.len()] {
        let mut decoder = NativeResponseDecoder::new_for_observation(
            &candidate,
            200,
            true,
            super::super::test_tool_projection(),
            None,
        )
        .unwrap();
        let mut renderer = IncrementalClientSseRenderer::new(profile.clone(), "alias").unwrap();
        let mut rendered = Vec::new();
        for part in wire.chunks(size) {
            decoder.feed(part, false).unwrap();
            for event in decoder.take_events() {
                rendered.extend(renderer.push(&event).unwrap());
            }
        }
        decoder.feed(&[], true).unwrap();
        for event in decoder.take_events() {
            rendered.extend(renderer.push(&event).unwrap());
        }
        let decoded = decoder.finish().unwrap();
        assert_eq!(renderer.buffered_semantic_bytes(), 0);
        let done = rendered
            .iter()
            .find(|event| event.event.as_deref() == Some("response.output_item.done"))
            .unwrap();
        assert_eq!(done.data["item"]["type"], "web_search_call");
        assert_eq!(done.data["item"]["action"], events()[5]["item"]["action"]);
        let body = json!({"model":"alias","input":[done.data["item"].clone()]});
        let request = decode_ingress_request_with_bindings(
            IngressProtocol::Responses,
            &body,
            &IngressRequestBindings {
                provider_state_owner: None,
            },
        )
        .unwrap();
        let native = project_candidate_request(&request, &candidate).unwrap();
        assert_eq!(native.body["input"][0], events()[5]["item"]);
        let another = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "another-model",
            fixed_reasoning("fixed"),
        );
        assert_eq!(
            project_candidate_request(&request, &another).unwrap().body["input"],
            native.body["input"]
        );
        assert!(matches!(
            &decoded.response.blocks[0],
            ResponseBlock::WebSearch { .. }
        ));
        let full = ClientResponseRenderer::render_nonstream(
            IngressProtocol::Responses,
            "alias",
            &decoded.response,
        )
        .unwrap();
        let RenderedClientResponse::Json { body, .. } = full else {
            panic!("json");
        };
        assert_eq!(body["output"][0], done.data["item"]);
    }
}

#[test]
fn incomplete_and_duplicate_search_lifecycles_fail_closed() {
    for duplicate in [false, true] {
        let mut frames = events();
        if duplicate {
            frames.insert(6, frames[5].clone());
        } else {
            frames.remove(5);
        }
        let wire: String = frames
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        let mut decoder = NativeResponseDecoder::new_for_observation(
            &candidate(),
            200,
            true,
            super::super::test_tool_projection(),
            None,
        )
        .unwrap();
        let failed = decoder.feed(wire.as_bytes(), true).is_err();
        assert!(failed || decoder.finish().is_err());
    }
}

#[test]
fn search_identity_index_and_progress_cannot_be_reused() {
    for extra in [
        json!({"type":"response.output_item.added","output_index":1,"item":{"type":"web_search_call","id":"ws_native","status":"in_progress"}}),
        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"item_id":"m","delta":"wrong item"}),
        json!({"type":"response.web_search_call.in_progress","output_index":0,"item_id":"ws_native"}),
    ] {
        let mut frames = events()[..4].to_vec();
        frames.push(extra);
        let wire: String = frames
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        let mut decoder = NativeResponseDecoder::new_for_observation(
            &candidate(),
            200,
            true,
            super::super::test_tool_projection(),
            None,
        )
        .unwrap();
        assert!(decoder.feed(wire.as_bytes(), false).is_err());
    }
}

#[test]
fn citation_offsets_are_preserved_without_guessing_native_index_units() {
    let citation = json!({"type":"url_citation","start_index":0,"end_index":3,
        "url":"https://example.com/search","title":"Fixture source"});
    let frames = [
        json!({"type":"response.created","response":{"id":"r","model":"physical"}}),
        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"item_id":"m","delta":"a😀"}),
        json!({"type":"response.output_text.annotation.added","output_index":0,"content_index":0,"item_id":"m","annotation_index":0,"annotation":citation}),
        json!({"type":"response.output_text.done","output_index":0,"content_index":0,"item_id":"m","text":"a😀"}),
        events().pop().unwrap(),
    ];
    let wire: String = frames
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    let candidate = candidate();
    let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
    let mut decoder = NativeResponseDecoder::new_for_observation(
        &candidate,
        200,
        true,
        super::super::test_tool_projection(),
        None,
    )
    .unwrap();
    let mut renderer = IncrementalClientSseRenderer::new(profile, "alias").unwrap();
    decoder.feed(wire.as_bytes(), true).unwrap();
    let rendered: Vec<_> = decoder
        .take_events()
        .iter()
        .flat_map(|event| renderer.push(event).unwrap())
        .collect();
    let done = rendered
        .iter()
        .find(|event| event.event.as_deref() == Some("response.output_item.done"))
        .unwrap();
    assert_eq!(
        done.data["item"]["content"][0]["annotations"],
        json!([citation])
    );
    let decoded = decoder.finish().unwrap();
    let RenderedClientResponse::Json { body, .. } = ClientResponseRenderer::render_nonstream(
        IngressProtocol::Responses,
        "alias",
        &decoded.response,
    )
    .unwrap() else {
        panic!("json")
    };
    assert_eq!(body["output"][0], done.data["item"]);
    // Native clients may omit item status when replaying their conversation.
    let mut item = done.data["item"].clone();
    item.as_object_mut().unwrap().remove("status");
    let request = decode_ingress_request_with_bindings(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":[item]}),
        &IngressRequestBindings::default(),
    )
    .unwrap();
    let native = project_candidate_request(&request, &candidate).unwrap();
    assert_eq!(
        native.body["input"][0]["content"][0]["annotations"],
        json!([citation])
    );
}
