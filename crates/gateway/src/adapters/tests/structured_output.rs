use super::*;
use serde_json::Value;

fn messages(format: Option<Value>) -> Value {
    let mut body = json!({"model":"alias","max_tokens":128,
        "messages":[{"role":"user","content":"Write a title"}],
        "output_config":{"effort":"high"}});
    if let Some(format) = format {
        body["output_config"]["format"] = format;
    }
    body
}

fn schema_format() -> Value {
    json!({"type":"json_schema","schema":{"type":"object",
        "properties":{"title":{"type":"string"}},
        "required":["title"],"additionalProperties":false}})
}

fn output_format(body: &Value, target: IngressProtocol) -> &Value {
    match target {
        IngressProtocol::Responses => &body["text"]["format"],
        IngressProtocol::ChatCompletions => &body["response_format"]["json_schema"],
        IngressProtocol::Messages => &body["output_config"]["format"],
    }
}

#[test]
fn messages_structured_output_maps_schema_without_weakening_it() {
    for optional_property in [false, true] {
        let mut format = schema_format();
        if optional_property {
            format["schema"]["properties"]["optional"] = json!({"type":"string"});
        }
        let request =
            decode_ingress_request(IngressProtocol::Messages, &messages(Some(format.clone())))
                .unwrap();
        assert!(!request.native_only);
        for target in [IngressProtocol::Responses, IngressProtocol::ChatCompletions] {
            let projected = project_candidate_request(
                &request,
                &exact_state_profile(IngressProtocol::Messages, target),
            )
            .unwrap();
            let actual = output_format(&projected.body, target);
            assert_eq!(actual["schema"], format["schema"]);
            assert_eq!(actual["name"], "hiroute_structured_output");
            assert_eq!(actual["strict"], true);
            if target == IngressProtocol::ChatCompletions {
                assert_eq!(projected.body["response_format"]["type"], "json_schema");
            } else {
                assert_eq!(actual["type"], "json_schema");
            }
        }
    }
}

#[test]
fn messages_structured_output_preserves_explicit_controls_and_native_shape() {
    let mut format = schema_format();
    format["name"] = json!("session_title");
    format["description"] = json!("Concise title");
    format["strict"] = json!(false);
    let body = messages(Some(format.clone()));
    let request = decode_ingress_request(IngressProtocol::Messages, &body).unwrap();
    for target in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let projected = project_candidate_request(
            &request,
            &exact_state_profile(IngressProtocol::Messages, target),
        )
        .unwrap();
        let actual = output_format(&projected.body, target);
        assert_eq!(actual["schema"], format["schema"]);
        assert_eq!(actual["name"], "session_title");
        assert_eq!(actual["description"], "Concise title");
        assert_eq!(actual["strict"], false);
        if target == IngressProtocol::Messages {
            assert_eq!(actual, &format);
        }
    }
}

#[test]
fn messages_structured_output_absent_does_not_invent_format() {
    let request = decode_ingress_request(IngressProtocol::Messages, &messages(None)).unwrap();
    for target in [IngressProtocol::Responses, IngressProtocol::ChatCompletions] {
        let projected = project_candidate_request(
            &request,
            &exact_state_profile(IngressProtocol::Messages, target),
        )
        .unwrap();
        assert!(projected.body.get("text").is_none());
        assert!(projected.body.get("response_format").is_none());
    }
}

#[test]
fn messages_structured_output_changes_context_not_message_continuity() {
    let base = messages(Some(schema_format()));
    let key = [17; 32];
    let digest = |body: &Value| {
        let request = decode_ingress_request(IngressProtocol::Messages, body).unwrap();
        let history = crate::context_hold::visible_history(&request, &key).unwrap();
        (
            history.instruction_digest,
            history.measure(None).unwrap().complete_digest,
        )
    };
    let original = digest(&base);
    let mut effort = base.clone();
    effort["output_config"]["effort"] = json!("low");
    assert_eq!(original, digest(&effort));
    let mut schema = base;
    schema["output_config"]["format"]["schema"]["properties"]["title"]["type"] = json!("number");
    let changed = digest(&schema);
    assert_ne!(original.0, changed.0);
    assert_eq!(original.1, changed.1);
    schema["output_config"]
        .as_object_mut()
        .unwrap()
        .remove("format");
    assert_ne!(original.0, digest(&schema).0);
}

#[test]
fn messages_structured_output_replays_large_schema_for_each_protocol() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-structured-output-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).unwrap();
    let budget = tree.stream(4 * 1024 * 1024).unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let mut format = schema_format();
    format["schema"]["properties"]["title"]["description"] = json!("title schema ".repeat(2000));
    format["description"] = json!("format description ".repeat(2000));
    let mut request =
        decode_ingress_request(IngressProtocol::Messages, &messages(Some(format.clone()))).unwrap();
    let key = [23; 32];
    let inline_digest = crate::context_hold::visible_history(&request, &key)
        .unwrap()
        .instruction_digest;
    externalize_model_request(&mut request, &store, 128).unwrap();
    assert_eq!(
        inline_digest,
        crate::context_hold::visible_history_with_replay(&request, &key, Some(&store))
            .unwrap()
            .instruction_digest
    );
    assert!(
        project_candidate_request(
            &request,
            &exact_state_profile(IngressProtocol::Messages, IngressProtocol::Responses)
        )
        .is_err()
    );
    store.prevalidate(&model_content_refs(&request)).unwrap();
    for target in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let profile = exact_state_profile(IngressProtocol::Messages, target);
        let template = project_candidate_request_template(&request, &profile).unwrap();
        let mut reader = sequential_attempt_body(template, store.clone(), &budget, 127).unwrap();
        let mut bytes = Vec::new();
        while let Some(chunk) = reader.next_chunk().unwrap() {
            bytes.extend_from_slice(chunk.bytes());
        }
        let projected: Value = serde_json::from_slice(&bytes).unwrap();
        let actual = output_format(&projected, target);
        assert_eq!(actual["schema"], format["schema"]);
        assert_eq!(actual["description"], format["description"]);
    }
    drop(store);
    drop(manager);
    fs_err_remove_dir(&root);
}
