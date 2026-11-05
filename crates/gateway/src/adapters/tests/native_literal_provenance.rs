use super::*;
use crate::content_ref::{
    ContentRef, compact_ingress_document_with_markers, parse_ingress_document,
    scan_ingress_document,
};
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use hiroute_gateway_core::runtime::body::StreamBudget;

struct Fixture {
    store: ReplayStore,
    budget: StreamBudget,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "hiroute-review-markers-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 16 * 1024,
            record_bytes: 1024,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let budget = BudgetTree::new(16 * 1024 * 1024, 16 * 1024 * 1024)
            .unwrap()
            .stream(16 * 1024 * 1024)
            .unwrap();
        Self {
            store: manager.begin_request(budget.clone()).unwrap(),
            budget,
            root,
        }
    }

    fn parse_owned(&self, bytes: &[u8]) -> ModelRequestIRV1 {
        self.parse_protocol(IngressProtocol::Responses, bytes, false)
    }

    fn parse_protocol(
        &self,
        protocol: IngressProtocol,
        bytes: &[u8],
        clone: bool,
    ) -> ModelRequestIRV1 {
        let mut raw = self.store.begin_raw().unwrap();
        for chunk in bytes.chunks(16 * 1024) {
            raw.append(chunk).unwrap();
        }
        let raw = raw.seal().unwrap();
        assert_eq!(raw.stream_ordinal(), 1);
        let stats = scan_ingress_document(self.store.reader(&raw).unwrap()).unwrap();
        let (mut document, mut workspace) =
            parse_ingress_document(protocol, &self.store, &raw, stats).unwrap();
        compact_ingress_document_with_markers(
            protocol,
            &mut document,
            &self.store,
            workspace.generated_markers(),
        )
        .unwrap();
        let native_literals = workspace.native_literals(&document, &self.store).unwrap();
        let document = if clone { document.clone() } else { document };
        let mut request =
            super::super::decode_owned_ingress_request_with_replay(protocol, document, &self.store)
                .unwrap();
        native_literals
            .externalize(&mut request, &self.store)
            .unwrap();
        externalize_model_request(&mut request, &self.store, 8 * 1024).unwrap();
        drop(workspace);
        request
    }

    fn wire(&self, request: &ModelRequestIRV1, target: IngressProtocol) -> serde_json::Value {
        let profile = CandidateProtocolProfile::exact_portable_path(
            request.ingress_protocol,
            target,
            "physical",
            fixed_reasoning("fixed"),
        );
        let projected = project_candidate_request_template(request, &profile);
        assert!(
            projected.is_ok(),
            "target={target:?}, error={:?}",
            projected.err()
        );
        self.store
            .prevalidate(&model_content_refs(request))
            .expect("client literals must not become Replay locators");
        let mut reader =
            sequential_attempt_body(projected.unwrap(), self.store.clone(), &self.budget, 257)
                .unwrap();
        let mut wire = Vec::new();
        while let Some(chunk) = reader.next_chunk().unwrap() {
            wire.extend_from_slice(chunk.bytes());
        }
        serde_json::from_slice(&wire).expect("provider wire must be complete valid JSON")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn metadata_literal(prefix: &str) {
    let fixture = Fixture::new();
    let input = "Z".repeat(16_000);
    let marker = ContentRef::new(2, 0, 16_000, 16_000).wire_marker();
    let literal = format!("{prefix}{marker}");
    let body =
        format!(r#"{{"model":"alias","input":"{input}","metadata":{{"note":"{literal}"}}}}"#);
    let request = fixture.parse_owned(body.as_bytes());
    assert!(
        fixture
            .store
            .has_reference(&ContentRef::new(2, 0, 16_000, 16_000))
    );
    let wire = fixture.wire(&request, IngressProtocol::Responses);
    let actual = wire["metadata"]["note"].as_str().unwrap();
    assert!(
        actual == literal,
        "metadata literal changed: expected {} bytes, actual {} bytes",
        literal.len(),
        actual.len()
    );
}

#[test]
fn native_literal_metadata_data_uri_stays_literal() {
    metadata_literal("data:text/plain;base64,");
}

#[test]
fn native_literal_metadata_plain_locator_stays_literal() {
    metadata_literal("");
}

#[test]
fn native_literal_metadata_object_is_not_an_internal_json_reference() {
    let fixture = Fixture::new();
    let body = br#"{"model":"alias","input":"hello","metadata":{"__hiroute_json_content_ref_v2":"__hiroute_content_ref_v2_999_0_1_1__"}}"#;
    let request = fixture.parse_owned(body);
    let wire = fixture.wire(&request, IngressProtocol::Responses);
    assert_eq!(
        wire["metadata"]["__hiroute_json_content_ref_v2"],
        "__hiroute_content_ref_v2_999_0_1_1__"
    );
}

fn tool_identity(target: IngressProtocol) {
    let fixture = Fixture::new();
    let name = "tool__hiroute_content_ref_v2_999_0_1_1__";
    let body = serde_json::to_vec(&json!({
        "model":"alias","input":"hello",
        "tools":[{"type":"function","name":name,"description":"A tool",
            "parameters":{"type":"object","properties":{}}}]
    }))
    .unwrap();
    let request = fixture.parse_owned(&body);
    let native = fixture.wire(&request, IngressProtocol::Responses);
    assert_eq!(native["tools"][0]["name"], name);
    let wire = fixture.wire(&request, target);
    match target {
        IngressProtocol::Messages => assert_eq!(wire["tools"][0]["name"], name),
        IngressProtocol::ChatCompletions => assert_eq!(wire["tools"][0]["function"]["name"], name),
        _ => unreachable!(),
    }
}

#[test]
fn native_literal_tool_name_survives_conversion_to_messages() {
    tool_identity(IngressProtocol::Messages);
}

#[test]
fn native_literal_tool_name_survives_conversion_to_chat() {
    tool_identity(IngressProtocol::ChatCompletions);
}

#[test]
fn native_literal_owned_image_releases_original_string_capacity() {
    let fixture = Fixture::new();
    let image_url = format!("data:image/png;base64,{}", "QUJD".repeat(400_000));
    let body = serde_json::to_vec(&json!({
        "model":"alias","input":[{"role":"user","content":[
            {"type":"input_image","image_url":image_url,"detail":"high"}
        ]}]
    }))
    .unwrap();
    let request = fixture.parse_owned(&body);
    let native = request.native_body.as_ref().unwrap();
    let serde_json::Value::String(url) = &native["input"][0]["content"][0]["image_url"] else {
        panic!("image URL must be a string");
    };
    assert!(url.len() < 128);
    assert!(
        url.capacity() < 1024,
        "short image URL retains {} bytes for len {}; entire live budget is {} bytes",
        url.capacity(),
        url.len(),
        fixture.budget.snapshot().unwrap().live
    );
}

#[test]
fn native_literal_json_metadata_cannot_inject_provider_controls() {
    let fixture = Fixture::new();
    let first = format!("{{}}{}", " ".repeat(9_000));
    let second = format!(
        r#","model":"unexpected-upstream-model","top_p":0.001,"metadata":{{}}{}"#,
        " ".repeat(9_000)
    );
    let payload = format!("{first}{second}");
    let escaped_len = serde_json::to_string(&payload).unwrap().len() - 2;
    let marker = ContentRef::new(2, 0, payload.len() as u64, escaped_len as u64).wire_marker();
    let first = serde_json::to_string(&first).unwrap();
    let second = serde_json::to_string(&second).unwrap();
    let body = format!(
        r#"{{"model":"alias","input":[{{"role":"user","content":{first}}},{{"role":"user","content":{second}}}],"metadata":{{"__hiroute_json_content_ref_v2":"{marker}"}}}}"#
    );
    let request = fixture.parse_owned(body.as_bytes());
    let wire = fixture.wire(&request, IngressProtocol::Responses);
    assert_eq!(
        wire["model"], "physical",
        "client metadata injected provider controls: model={}, top_p={}",
        wire["model"], wire["top_p"]
    );
    assert!(
        wire.get("top_p").is_none(),
        "client metadata injected top_p"
    );
    assert_eq!(wire["metadata"]["__hiroute_json_content_ref_v2"], marker);
}

#[test]
fn native_literal_chat_namespace_description_preserves_literal_and_replay_pieces() {
    let fixture = Fixture::new();
    let namespace = "ns__hiroute_content_ref_v2_999_0_1_1__";
    let namespace_description = "N".repeat(16_000);
    let child_description = "C".repeat(16_000);
    let body = serde_json::to_vec(&json!({
        "model":"alias","input":"hello",
        "tools":[{"type":"namespace","name":namespace,
            "description":namespace_description,
            "tools":[{"type":"function","name":"f","description":child_description,
                "parameters":{"type":"object","properties":{}}}]}]
    }))
    .unwrap();
    let request = fixture.parse_owned(&body);
    let wire = fixture.wire(&request, IngressProtocol::ChatCompletions);
    assert_eq!(
        wire["tools"][0]["function"]["name"],
        format!("{namespace}__f")
    );
    assert_eq!(
        wire["tools"][0]["function"]["description"],
        format!("[{namespace}]\n{namespace_description}\n\n{child_description}")
    );
}

#[test]
fn native_literal_client_wire_parts_envelope_keeps_its_json_shape() {
    let fixture = Fixture::new();
    let literal = json!({(crate::content_ref::WIRE_STRING_PARTS_KEY):["left","right"]});
    let body = serde_json::to_vec(&json!({
        "model":"alias","input":"hello","metadata":literal
    }))
    .unwrap();
    let request = fixture.parse_owned(&body);
    let wire = fixture.wire(&request, IngressProtocol::Responses);
    assert_eq!(wire["metadata"], literal);
}

#[test]
fn native_literal_reserved_envelopes_expand_only_generated_descendants() {
    for key in [
        "__hiroute_json_content_ref_v2",
        "__hiroute_literal_wire_value_v1",
        "__hiroute_wire_string_parts_v1",
    ] {
        let fixture = Fixture::new();
        let text = "Z中\\\"\n".repeat(4_000);
        let mut extension = serde_json::Map::new();
        extension.insert(key.into(), json!({"text":text}));
        let body = serde_json::to_vec(
            &json!({"model":"alias","input":"hello","provider_extension":extension}),
        )
        .unwrap();
        let request = fixture.parse_owned(&body);
        let wire = fixture.wire(&request, IngressProtocol::Responses);
        assert!(
            wire["provider_extension"][key]["text"] == text,
            "reserved envelope {key} hid its generated text reference"
        );
    }
}

#[test]
fn native_literal_reserved_shape_never_becomes_a_reference_after_compaction() {
    let fixture = Fixture::new();
    let literal = json!({"__hiroute_json_content_ref_v2":"{} ,\"max_output_tokens\":99999"});
    let body =
        serde_json::to_vec(&json!({"model":"alias","input":"Z".repeat(8192),"metadata":literal}))
            .unwrap();
    let request = fixture.parse_owned(&body);
    let wire = fixture.wire(&request, IngressProtocol::Responses);
    assert_eq!(wire["metadata"], literal);
    assert_ne!(wire["max_output_tokens"], 99999);
}

#[test]
fn native_literal_same_locator_in_identity_and_content_keeps_distinct_meaning() {
    for target in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let fixture = Fixture::new();
        let marker = ContentRef::new(2, 0, 16000, 16000).wire_marker();
        let body = format!(
            r#"{{"model":"alias","input":"{}","tools":[{{"type":"function","name":"{marker}","parameters":{{"type":"object","properties":{{}}}}}}],"tool_choice":{{"type":"function","name":"{marker}"}}}}"#,
            "Z".repeat(16000)
        );
        let request = fixture.parse_protocol(IngressProtocol::Responses, body.as_bytes(), true);
        let wire = fixture.wire(&request, target);
        if target == IngressProtocol::ChatCompletions {
            assert_eq!(wire["tools"][0]["function"]["name"], marker);
            assert_eq!(wire["tool_choice"]["function"]["name"], marker);
        } else {
            assert_eq!(wire["tools"][0]["name"], marker);
            assert_eq!(wire["tool_choice"]["name"], marker);
        }
    }
}

#[test]
fn native_literal_chat_message_name_is_not_an_alias_for_prompt_content() {
    let fixture = Fixture::new();
    let marker = ContentRef::new(2, 0, 16000, 16000).wire_marker();
    let body = format!(
        r#"{{"model":"alias","messages":[{{"role":"user","content":"{}"}},{{"role":"user","name":"{marker}","content":"hello"}}]}}"#,
        "Z".repeat(16000)
    );
    let request = fixture.parse_protocol(IngressProtocol::ChatCompletions, body.as_bytes(), false);
    let wire = fixture.wire(&request, IngressProtocol::ChatCompletions);
    assert_eq!(wire["messages"][1]["name"], marker);
}

#[test]
fn native_literal_direct_projection_consumes_internal_envelopes() {
    let name = "a__hiroute_content_ref_v2_x__hiroute_content_ref_v2_";
    let mut request = decode_ingress_request(IngressProtocol::Responses, &json!({
        "model":"alias","input":"hello","tools":[{"type":"function","name":name,"parameters":{"type":"object","properties":{}}}]
    })).unwrap();
    request.native_body = None;
    for target in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            target,
            "physical",
            fixed_reasoning("fixed"),
        );
        let projected = project_candidate_request(&request, &profile).unwrap();
        assert_eq!(
            projected.body,
            serde_json::from_slice::<serde_json::Value>(&projected.bytes).unwrap()
        );
        let actual = if target == IngressProtocol::ChatCompletions {
            &projected.body["tools"][0]["function"]["name"]
        } else {
            &projected.body["tools"][0]["name"]
        };
        assert_eq!(actual, name);
    }
}

#[test]
fn native_literal_direct_projection_preserves_client_envelope_shapes() {
    for metadata in [
        json!({"__hiroute_literal_wire_value_v1":"left"}),
        json!({"__hiroute_wire_string_parts_v1":["left","right"]}),
    ] {
        let request = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"alias","input":"hello","metadata":metadata}),
        )
        .unwrap();
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        let projected = project_candidate_request(&request, &profile).unwrap();
        assert_eq!(projected.body["metadata"], metadata);
        assert_eq!(
            projected.body,
            serde_json::from_slice::<serde_json::Value>(&projected.bytes).unwrap()
        );
    }
}

#[test]
fn native_literal_direct_schema_defaults_keep_reserved_object_shapes() {
    for literal in [
        json!({"__hiroute_literal_wire_value_v1":"left"}),
        json!({"__hiroute_wire_string_parts_v1":["left","right"]}),
    ] {
        let schema =
            json!({"type":"object","properties":{"tag":{"type":"object","default":literal}}});
        let mut request = decode_ingress_request(IngressProtocol::Responses, &json!({"model":"alias","input":"hello","tools":[{"type":"function","name":"f","parameters":schema}]})).unwrap();
        request.native_body = None;
        for target in [
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            IngressProtocol::Messages,
        ] {
            let profile = CandidateProtocolProfile::exact_portable_path(
                IngressProtocol::Responses,
                target,
                "physical",
                fixed_reasoning("fixed"),
            );
            let projected = project_candidate_request(&request, &profile).unwrap();
            let actual = match target {
                IngressProtocol::Responses => &projected.body["tools"][0]["parameters"],
                IngressProtocol::ChatCompletions => {
                    &projected.body["tools"][0]["function"]["parameters"]
                }
                IngressProtocol::Messages => &projected.body["tools"][0]["input_schema"],
            };
            assert_eq!(actual, &schema);
        }
    }
}

#[test]
fn native_literal_raw_json_accepts_only_one_registered_json_value() {
    use super::super::request::{
        ReplacementEncoding, RequestedReplacement, prepare_replay_json_template,
        sequential_replay_body,
    };
    for (parts, valid) in [
        (vec![br#" {"ok":true} "#.as_slice()], true),
        (vec![br#"{} ,"model":"other""#.as_slice()], false),
        (
            vec![b"{}".as_slice(), br#","model":"other""#.as_slice()],
            false,
        ),
    ] {
        let fixture = Fixture::new();
        let mut pool = fixture.store.begin_content_pool().unwrap();
        let mut refs = Vec::new();
        for part in parts {
            refs.push(pool.append(part).unwrap());
        }
        pool.seal().unwrap();
        let content = if refs.len() == 1 {
            refs[0].clone()
        } else {
            ContentRef::new(
                refs[0].stream_ordinal(),
                0,
                refs.iter().map(ContentRef::byte_len).sum(),
                refs.iter().map(ContentRef::json_escaped_len).sum(),
            )
        };
        fixture
            .store
            .prevalidate(std::slice::from_ref(&content))
            .unwrap();
        let template = prepare_replay_json_template(
            &json!({"metadata":content.wire_marker()}),
            [RequestedReplacement {
                content,
                encoding: ReplacementEncoding::RawJson,
            }],
        )
        .unwrap();
        let result = sequential_replay_body(template, fixture.store.clone(), &fixture.budget, 257);
        assert_eq!(result.is_ok(), valid);
        if let Ok(mut reader) = result {
            let mut wire = Vec::new();
            while let Some(chunk) = reader.next_chunk().unwrap() {
                wire.extend_from_slice(chunk.bytes());
            }
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&wire).unwrap(),
                json!({"metadata":{"ok":true}})
            );
        }
    }
}

#[test]
fn native_literal_canonical_search_history_preserves_typed_marker_strings() {
    let marker = "literal __hiroute_content_ref_v2_999_0_1_1__";
    for action in [
        json!({"type":"search","query":marker,"queries":[marker],"sources":[{"type":"url","url":marker}]}),
        json!({"type":"open_page","url":marker}),
        json!({"type":"find","url":marker,"pattern":marker}),
    ] {
        let fixture = Fixture::new();
        let search =
            json!({"type":"web_search_call","id":marker,"status":"completed","action":action});
        let mut request = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"alias","input":[search,{"role":"user","content":"continue"}]}),
        )
        .unwrap();
        request.native_body = None;
        let wire = fixture.wire(&request, IngressProtocol::Responses);
        assert_eq!(wire["input"][0], search);
    }
}

#[test]
fn native_literal_direct_messages_format_keeps_reserved_schema_defaults() {
    for literal in [
        json!({"__hiroute_literal_wire_value_v1":"left"}),
        json!({"__hiroute_wire_string_parts_v1":["left","right"]}),
    ] {
        let schema =
            json!({"type":"object","properties":{"tag":{"type":"object","default":literal}}});
        let request = decode_ingress_request(
            IngressProtocol::Messages,
            &json!({"model":"alias","max_tokens":128,"messages":[{"role":"user","content":"hello"}],"output_config":{"format":{"type":"json_schema","schema":schema}}}),
        )
        .unwrap();
        for target in [IngressProtocol::Responses, IngressProtocol::ChatCompletions] {
            let projected = project_candidate_request(
                &request,
                &exact_state_profile(IngressProtocol::Messages, target),
            )
            .unwrap();
            let actual = match target {
                IngressProtocol::Responses => &projected.body["text"]["format"]["schema"],
                IngressProtocol::ChatCompletions => {
                    &projected.body["response_format"]["json_schema"]["schema"]
                }
                IngressProtocol::Messages => unreachable!(),
            };
            assert_eq!(actual, &schema);
            assert_eq!(
                projected.body,
                serde_json::from_slice::<serde_json::Value>(&projected.bytes).unwrap()
            );
        }
    }
}

#[test]
fn native_literal_backed_messages_format_keeps_json_encoding_across_protocols() {
    for literal in [
        json!({"__hiroute_literal_wire_value_v1":"left"}),
        json!({"__hiroute_wire_string_parts_v1":["left","right"]}),
    ] {
        let fixture = Fixture::new();
        let schema = json!({"type":"object","description":"schema ".repeat(4000),"properties":{"tag":{"type":"object","default":literal}}});
        let bytes = serde_json::to_vec(&json!({"model":"alias","max_tokens":128,"messages":[{"role":"user","content":"hello"}],"output_config":{"format":{"type":"json_schema","schema":schema}}})).unwrap();
        let request = fixture.parse_protocol(IngressProtocol::Messages, &bytes, false);
        for target in [IngressProtocol::Responses, IngressProtocol::ChatCompletions] {
            let wire = fixture.wire(&request, target);
            let actual = match target {
                IngressProtocol::Responses => &wire["text"]["format"]["schema"],
                IngressProtocol::ChatCompletions => {
                    &wire["response_format"]["json_schema"]["schema"]
                }
                IngressProtocol::Messages => unreachable!(),
            };
            assert_eq!(actual, &schema);
        }
    }
}

#[test]
fn native_literal_large_reserved_subtree_does_not_materialize_all_backed_strings() {
    let fixture = Fixture::new();
    let text = "Z".repeat(512 * 1024);
    let entries = (0..10).map(|_| json!({"text":text})).collect::<Vec<_>>();
    let body = serde_json::to_vec(&json!({"model":"alias","input":"hello","provider_extension":{"__hiroute_literal_wire_value_v1":entries}})).unwrap();
    let request = fixture.parse_owned(&body);
    assert!(
        fixture.budget.snapshot().unwrap().peak < 4 * 1024 * 1024,
        "literal preservation gathered the complete subtree"
    );
    let wire = fixture.wire(&request, IngressProtocol::Responses);
    let entries = wire["provider_extension"]["__hiroute_literal_wire_value_v1"]
        .as_array()
        .unwrap();
    assert_eq!(entries.len(), 10);
    assert!(entries.iter().all(|entry| entry["text"] == text));
}

#[test]
fn native_literal_duplicate_keys_drop_stale_generated_node_proof() {
    let fixture = Fixture::new();
    let marker = ContentRef::new(2, 0, 16000, 16000).wire_marker();
    let bytes = format!(
        r#"{{"model":"alias","input":"{}","input":"hello","metadata":{{"note":"{marker}"}}}}"#,
        "Z".repeat(16000)
    );
    let mut raw = fixture.store.begin_raw().unwrap();
    raw.append(bytes.as_bytes()).unwrap();
    let raw = raw.seal().unwrap();
    let stats = scan_ingress_document(fixture.store.reader(&raw).unwrap()).unwrap();
    let (document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, &fixture.store, &raw, stats).unwrap();
    assert!(
        workspace.generated_markers().is_empty(),
        "overwritten nodes retained trusted allocation addresses"
    );
    let literals = workspace
        .native_literals(&document, &fixture.store)
        .unwrap();
    let mut request = super::super::decode_owned_ingress_request_with_replay(
        IngressProtocol::Responses,
        document,
        &fixture.store,
    )
    .unwrap();
    literals.externalize(&mut request, &fixture.store).unwrap();
    externalize_model_request(&mut request, &fixture.store, 8 * 1024).unwrap();
    let wire = fixture.wire(&request, IngressProtocol::Responses);
    assert_eq!(wire["input"], "hello");
    assert_eq!(wire["metadata"]["note"], marker);
}

#[test]
fn native_literal_unknown_arguments_field_is_not_a_tool_argument() {
    let fixture = Fixture::new();
    let literal = format!("{{  \"x\" : \"{}\"  }}", "Z".repeat(16000));
    let body = serde_json::to_vec(&json!({"model":"alias","input":"hello","provider_extension":{"__hiroute_literal_wire_value_v1":{"arguments":literal}}})).unwrap();
    let request = fixture.parse_owned(&body);
    let wire = fixture.wire(&request, IngressProtocol::Responses);
    assert_eq!(
        wire["provider_extension"]["__hiroute_literal_wire_value_v1"]["arguments"],
        literal
    );
}

#[test]
fn native_literal_reasoning_backed_leaf_does_not_hide_an_inline_sibling() {
    let fixture = Fixture::new();
    let blob = "B".repeat(1_000_000);
    let body = serde_json::to_vec(&json!({"model":"alias","input":[{"type":"reasoning","summary":[{"type":"summary_text","text":"N".repeat(16000)}],"provider_extension":{"text":"T".repeat(16000),"blob":blob}}]})).unwrap();
    let request = fixture.parse_owned(&body);
    let extension = &request.responses_reasoning_history[&0].native_fields["provider_extension"];
    assert!(
        extension["blob"]
            .as_str()
            .is_some_and(|value| value.len() < 128),
        "a backed sibling left the whole blob resident in canonical history"
    );
    let wire = fixture.wire(&request, IngressProtocol::Responses);
    assert!(wire["input"][0]["provider_extension"]["blob"] == blob);
}

#[test]
fn native_literal_json_validation_optional_scratch_does_not_deny_a_valid_reader() {
    use super::super::request::{
        ReplacementEncoding, RequestedReplacement, prepare_replay_json_template,
        sequential_replay_body,
    };
    use hiroute_gateway_core::runtime::body::MemoryRole;
    let fixture = Fixture::new();
    let bytes = format!("{}{{\"ok\":true}}", " ".repeat(20000));
    let mut pool = fixture.store.begin_content_pool().unwrap();
    let content = pool.append(bytes.as_bytes()).unwrap();
    pool.seal().unwrap();
    fixture
        .store
        .prevalidate(std::slice::from_ref(&content))
        .unwrap();
    let template = prepare_replay_json_template(
        &json!({"metadata":content.wire_marker()}),
        [RequestedReplacement {
            content,
            encoding: ReplacementEncoding::RawJson,
        }],
    )
    .unwrap();
    let _pressure = fixture
        .budget
        .reserve(
            MemoryRole::ModelIrBacking,
            fixture.budget.available_bytes().unwrap() - 8704,
        )
        .unwrap();
    let mut reader =
        sequential_replay_body(template, fixture.store.clone(), &fixture.budget, 257).unwrap();
    let mut wire = Vec::new();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        wire.extend_from_slice(chunk.bytes());
    }
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&wire).unwrap(),
        json!({"metadata":{"ok":true}})
    );
}
