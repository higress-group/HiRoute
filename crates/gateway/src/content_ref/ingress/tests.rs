use super::*;
use hiroute_gateway_core::runtime::body::BudgetTree;
use serde_json::json;

#[test]
fn large_valid_input_spills_and_reaches_compacted_document_under_eight_mib_budget() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-large-ingress-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 1024 * 1024,
        record_bytes: 16 * 1024,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
        .unwrap()
        .stream(8 * 1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let input = "long context 中文\n".repeat(300_000);
    let body = serde_json::to_vec(&json!({"model":"alias","input":input,"stream":false})).unwrap();
    assert!(body.len() > 4 * 1024 * 1024);
    let mut writer = store.begin_raw().unwrap();
    for chunk in body.chunks(16 * 1024) {
        writer.append(chunk).unwrap();
    }
    let raw = writer.seal().unwrap();
    assert!(store.snapshot().disk_backed);
    let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, &store, &raw, stats).unwrap();
    compact_ingress_document_with_markers(
        IngressProtocol::Responses,
        &mut document,
        &store,
        workspace.generated_markers(),
    )
    .unwrap();
    let reference =
        super::super::ContentRef::from_wire_marker(document["input"].as_str().unwrap()).unwrap();
    let mut actual = String::new();
    store
        .reader(&reference)
        .unwrap()
        .read_to_string(&mut actual)
        .unwrap();
    assert_eq!(actual, input);
    assert!(budget.snapshot().unwrap().peak <= 8 * 1024 * 1024);
    drop((workspace, document, store, manager));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn escaped_wire_string_reserves_decoded_scratch_not_wire_length() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-escaped-ingress-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 1024 * 1024,
        record_bytes: 16 * 1024,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
        .unwrap()
        .stream(8 * 1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let body = format!(
        r#"{{"model":"alias","input":"{}"}}"#,
        "\\u0041".repeat(1_500_000)
    );
    assert!(body.len() > 8 * 1024 * 1024);
    let mut writer = store.begin_raw().unwrap();
    for chunk in body.as_bytes().chunks(16 * 1024) {
        writer.append(chunk).unwrap();
    }
    let raw = writer.seal().unwrap();
    let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
    assert_eq!(stats.max_string_bytes, 1_500_000);
    let (document, workspace) =
        parse_ingress_document(IngressProtocol::Responses, &store, &raw, stats).unwrap();
    let reference =
        super::super::ContentRef::from_wire_marker(document["input"].as_str().unwrap()).unwrap();
    assert_eq!(reference.byte_len(), 1_500_000);
    assert!(budget.snapshot().unwrap().peak <= 8 * 1024 * 1024);
    drop((workspace, document, store, manager));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn large_image_arguments_and_native_reasoning_fit_the_eight_mib_budget() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-large-semantic-ingress-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 1024 * 1024,
        record_bytes: 16 * 1024,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let large = "QUJD".repeat(1_150_000);
    for kind in 0..4 {
        let (protocol, body) = match kind {
            0 => (
                IngressProtocol::Responses,
                json!({"model":"alias","input":[{"type":"message","role":"user",
                    "content":[{"type":"input_image",
                        "image_url":format!("data:image/png;base64,{large}")}]}]}),
            ),
            1 => (
                IngressProtocol::Responses,
                json!({"model":"alias","input":[{"type":"function_call",
                    "call_id":"call-1","name":"lookup",
                    "arguments":format!("{{ \"blob\" : \"{large}\" }}")}]}),
            ),
            2 => (
                IngressProtocol::Messages,
                json!({"model":"alias","max_tokens":128,
                    "messages":[{"role":"assistant","content":[{"type":"thinking",
                        "thinking":large,"signature":"opaque"}]},
                        {"role":"user","content":"continue"}]}),
            ),
            _ => (
                IngressProtocol::Responses,
                json!({"model":"alias","input":[
                    {"type":"reasoning","encrypted_content":large},
                    {"type":"message","role":"user","content":[
                        {"type":"input_text","text":"continue"}]}]}),
            ),
        };
        let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
            .unwrap()
            .stream(8 * 1024 * 1024)
            .unwrap();
        let store = manager.begin_request(budget.clone()).unwrap();
        let body = serde_json::to_vec(&body).unwrap();
        assert!(body.len() > 4 * 1024 * 1024);
        let mut writer = store.begin_raw().unwrap();
        for chunk in body.chunks(16 * 1024) {
            writer.append(chunk).unwrap();
        }
        let raw = writer.seal().unwrap();
        let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
        let (mut document, mut workspace) =
            parse_ingress_document(protocol, &store, &raw, stats).unwrap();
        compact_ingress_document_with_markers(
            protocol,
            &mut document,
            &store,
            workspace.generated_markers(),
        )
        .unwrap();
        match kind {
            0 => {
                let request = crate::server::core_runtime::adapters::decode_ingress_request(
                    protocol, &document,
                )
                .unwrap();
                let crate::server::core_runtime::model_ir::ContentPart::Image {
                    source:
                        crate::server::core_runtime::model_ir::ImageSource::Base64 { media_type, data },
                } = &request.messages[0].content[0]
                else {
                    panic!("large data URI changed its image discriminator")
                };
                assert_eq!(media_type, "image/png");
                let reference = super::super::ContentRef::from_wire_marker(data).unwrap();
                assert_eq!(reference.byte_len(), large.len() as u64);
            }
            1 => {
                let request = crate::server::core_runtime::adapters::decode_ingress_request(
                    protocol, &document,
                )
                .unwrap();
                let crate::server::core_runtime::model_ir::ContentPart::ToolCall {
                    arguments,
                    raw_arguments,
                    ..
                } = &request.messages[0].content[0]
                else {
                    panic!("large tool arguments did not reach the canonical request")
                };
                assert!(raw_arguments.is_none());
                let reference = super::super::JsonValueExt::content_ref(arguments).unwrap();
                let mut actual = String::new();
                store
                    .reader(&reference)
                    .unwrap()
                    .read_to_string(&mut actual)
                    .unwrap();
                assert_eq!(actual, format!("{{ \"blob\" : \"{large}\" }}"));
            }
            2 | 3 => {
                let state_wire = if kind == 2 {
                    document["messages"][0]["content"][0]["thinking"]
                        .as_str()
                        .unwrap()
                } else {
                    document["input"][0]["encrypted_content"].as_str().unwrap()
                };
                let reference = super::super::ContentRef::from_wire_marker(state_wire).unwrap();
                assert_eq!(reference.byte_len(), large.len() as u64);
                let mut profile = crate::server::core_runtime::profiles::CandidateProtocolProfile::exact_portable_path(
                    protocol,
                    protocol,
                    "physical",
                    crate::server::core_runtime::profiles::fixed_reasoning("fixed"),
                );
                use crate::server::core_runtime::profiles::{
                    Fidelity, NativeProviderStateEmission, StateAffinity,
                };
                profile.capability.native_provider_state =
                    NativeProviderStateEmission::ExactOwnerAffine;
                profile.capability.request.provider_state = Fidelity::Exact;
                profile.capability.request.state_affinity = StateAffinity::ExactOwner;
                profile.capability.response.provider_state = Fidelity::Exact;
                profile.capability.response.state_affinity = StateAffinity::ExactOwner;
                let owner = profile.exact_provider_path().unwrap();
                let mut request =
                    crate::server::core_runtime::adapters::decode_ingress_request_with_bindings(
                        protocol,
                        &document,
                        &crate::server::core_runtime::adapters::IngressRequestBindings {
                            provider_state_owner: Some(owner),
                        },
                    )
                    .unwrap();
                let crate::server::core_runtime::model_ir::ContentPart::ProviderState { state } =
                    &request.messages[0].content[0]
                else {
                    panic!("large native thinking was not decoded as provider state")
                };
                if kind == 2 {
                    assert_eq!(state.value["thinking"], state_wire);
                } else {
                    assert_eq!(state.value, Value::String(state_wire.to_owned()));
                }
                super::super::externalize_model_request(&mut request, &store, 8 * 1024).unwrap();
                assert!(super::super::model_content_refs(&request).contains(&reference));
                let template =
                    crate::server::core_runtime::adapters::project_candidate_request_template(
                        &request, &profile,
                    )
                    .unwrap();
                store
                    .prevalidate(&super::super::model_content_refs(&request))
                    .unwrap();
                let mut reader = crate::server::core_runtime::adapters::sequential_attempt_body(
                    template,
                    store.clone(),
                    &budget,
                    16 * 1024,
                )
                .unwrap();
                let mut projected = Vec::new();
                while let Some(chunk) = reader.next_chunk().unwrap() {
                    projected.extend_from_slice(chunk.bytes());
                }
                let projected: Value = serde_json::from_slice(&projected).unwrap();
                if kind == 2 {
                    assert_eq!(projected["messages"][0]["content"][0]["thinking"], large);
                } else {
                    assert_eq!(projected["input"][0]["encrypted_content"], large);
                }
            }
            _ => unreachable!(),
        }
        assert!(budget.snapshot().unwrap().peak <= 8 * 1024 * 1024);
        drop((workspace, document));
        assert!(
            budget.snapshot().unwrap().live < 1024 * 1024,
            "parse scratch and retained native text must be released at their last consumer"
        );
        drop(store);
    }
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn many_content_fields_do_not_turn_total_body_bytes_into_a_memory_quota() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-many-ingress-fields-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 1024 * 1024,
        record_bytes: 16 * 1024,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
        .unwrap()
        .stream(8 * 1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let body = serde_json::to_vec(&json!({
        "model":"alias",
        "input": (0..1024).map(|_| json!({
            "type":"message", "role":"user", "content":[{
                "type":"input_text", "text":"x".repeat(10_000)
            }]
        })).collect::<Vec<_>>()
    }))
    .unwrap();
    assert!(body.len() > 8 * 1024 * 1024);
    let mut writer = store.begin_raw().unwrap();
    for chunk in body.chunks(16 * 1024) {
        writer.append(chunk).unwrap();
    }
    let raw = writer.seal().unwrap();
    let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
    assert!(stats.max_string_bytes < 16 * 1024);
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, &store, &raw, stats).unwrap();
    compact_ingress_document_with_markers(
        IngressProtocol::Responses,
        &mut document,
        &store,
        workspace.generated_markers(),
    )
    .unwrap();
    let marker = document["input"][1023]["content"][0]["text"]
        .as_str()
        .unwrap();
    let reference = super::super::ContentRef::from_wire_marker(marker).unwrap();
    assert_eq!(reference.byte_len(), 10_000);
    assert!(budget.snapshot().unwrap().peak <= 8 * 1024 * 1024);
    drop((workspace, document, store, manager));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn streaming_parse_preserves_literal_markers_and_opaque_reasoning() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-streaming-markers-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
        .unwrap()
        .stream(8 * 1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget).unwrap();
    let literal = "__hiroute_content_ref_v2_2_0_7_7__";
    // `text` precedes `type`: the parser still recognizes the whole native
    // reasoning block, and its large text stays request-local until emission.
    let body = format!(
        "{{\"model\":\"alias\",\"input\":[{{\"text\":\"{}\",\"type\":\"reasoning\"}},{{\"text\":\"{}\",\"type\":\"input_text\"}}]}}",
        "thought ".repeat(2000),
        literal
    );
    let mut writer = store.begin_raw().unwrap();
    writer.append(body.as_bytes()).unwrap();
    let raw = writer.seal().unwrap();
    let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, &store, &raw, stats).unwrap();
    let reasoning_text = document["input"][0]["text"].as_str().unwrap();
    let reasoning_ref = super::super::ContentRef::from_wire_marker(reasoning_text).unwrap();
    assert_eq!(
        reasoning_ref.byte_len(),
        "thought ".repeat(2000).len() as u64
    );
    compact_ingress_document_with_markers(
        IngressProtocol::Responses,
        &mut document,
        &store,
        workspace.generated_markers(),
    )
    .unwrap();
    let client_literal = document["input"][1]["text"].as_str().unwrap();
    assert_ne!(client_literal, literal);
    let reference = super::super::ContentRef::from_wire_marker(client_literal).unwrap();
    let mut actual = String::new();
    store
        .reader(&reference)
        .unwrap()
        .read_to_string(&mut actual)
        .unwrap();
    assert_eq!(actual, literal);
    drop((workspace, document, store, manager));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn streaming_parse_restores_nested_content_before_whole_json_compaction() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-streaming-whole-json-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
        .unwrap()
        .stream(8 * 1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget).unwrap();
    let original = "tool body 中文 ".repeat(1000);
    let body = format!(
        "{{\"model\":\"alias\",\"input\":[{{\"input\":{{\"text\":\"{original}\"}},\"type\":\"tool_use\"}}]}}"
    );
    let mut writer = store.begin_raw().unwrap();
    writer.append(body.as_bytes()).unwrap();
    let raw = writer.seal().unwrap();
    let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Messages, &store, &raw, stats).unwrap();
    compact_ingress_document_with_markers(
        IngressProtocol::Messages,
        &mut document,
        &store,
        workspace.generated_markers(),
    )
    .unwrap();
    let reference = super::super::JsonValueExt::content_ref(&document["input"][0]["input"])
        .expect("whole JSON reference");
    let mut actual = String::new();
    store
        .reader(&reference)
        .unwrap()
        .read_to_string(&mut actual)
        .unwrap();
    let decoded: Value = serde_json::from_str(&actual).unwrap();
    assert_eq!(decoded["text"], original);
    drop((workspace, document, store, manager));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn long_control_strings_are_preserved_without_a_generic_length_limit() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-long-control-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let original = "control 中文 ".repeat(4096);
    for protocol in [
        IngressProtocol::Responses,
        IngressProtocol::Messages,
        IngressProtocol::ChatCompletions,
    ] {
        let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
        let store = manager
            .begin_request(tree.stream(1024 * 1024).unwrap())
            .unwrap();
        let mut document = json!({"model":original,"metadata":{"extension":original}});
        let expected = document.clone();
        compact_ingress_document(protocol, &mut document, &store).unwrap();
        assert_eq!(document, expected);
    }
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn long_native_reasoning_history_is_content_not_control() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-long-reasoning-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    for (protocol, field) in [
        (IngressProtocol::Messages, "thinking"),
        (IngressProtocol::Messages, "signature"),
        (IngressProtocol::Responses, "encrypted_content"),
    ] {
        let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
        let store = manager
            .begin_request(tree.stream(1024 * 1024).unwrap())
            .unwrap();
        let original = "reasoning 中文 ".repeat(1024);
        let mut document = json!({"model":"alias", "history":[{field:original}]});
        compact_ingress_document(protocol, &mut document, &store).unwrap();
        let marker = document["history"][0][field].as_str().unwrap();
        let reference = crate::content_ref::ContentRef::from_wire_marker(marker).unwrap();
        let mut actual = String::new();
        store
            .reader(&reference)
            .unwrap()
            .read_to_string(&mut actual)
            .unwrap();
        assert_eq!(actual, original);
    }
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn responses_reasoning_stays_visible_for_typed_summary_decode() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-responses-reasoning-compaction-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).expect("budget tree");
    let budget = tree.stream(1024 * 1024).expect("stream budget");
    let store = manager.begin_request(budget).expect("replay store");
    let reasoning = json!({"effort":"low","summary":"auto"});
    let mut document = json!({
        "model":"alias",
        "input":"hello",
        "reasoning":reasoning,
    });

    compact_ingress_document(IngressProtocol::Responses, &mut document, &store)
        .expect("compact Responses document");
    assert_eq!(document["reasoning"], reasoning);
    let request = crate::server::core_runtime::adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &document,
    )
    .expect("typed Responses decode");
    assert_eq!(
        request
            .responses_options
            .and_then(|options| options.reasoning_summary),
        Some("auto".into())
    );

    drop(store);
    drop(manager);
    std::fs::remove_dir_all(root).expect("remove replay root");
}

#[test]
fn client_tool_arguments_cannot_alias_an_existing_request_content_ref() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-argument-marker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).expect("budget tree");
    let store = manager
        .begin_request(tree.stream(1024 * 1024).expect("stream budget"))
        .expect("replay store");
    let marker = store
        .store_content(b"other content")
        .expect("existing range")
        .wire_marker();
    let mut document = json!({
        "model":"alias",
        "input":[{"type":"function_call","call_id":"c","name":"lookup","arguments":marker}]
    });
    assert!(matches!(
        compact_ingress_document(IngressProtocol::Responses, &mut document, &store),
        Err(ReplayError::InvalidJson)
    ));
    drop((store, manager));
    std::fs::remove_dir_all(root).expect("remove replay root");
}

#[test]
fn malformed_native_tool_arguments_remain_raw_after_streaming_parse() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-malformed-argument-stream-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let store = manager
        .begin_request(
            BudgetTree::new(1024 * 1024, 1024 * 1024)
                .unwrap()
                .stream(1024 * 1024)
                .unwrap(),
        )
        .unwrap();
    let malformed = format!("{{\"status\":\"paused\"{}", "<tool_call>".repeat(1400));
    let body = serde_json::to_vec(&json!({
        "model":"alias",
        "input":[{"type":"function_call","call_id":"call-1",
            "name":"update_goal","arguments":malformed}]
    }))
    .unwrap();
    let mut writer = store.begin_raw().unwrap();
    writer.append(&body).unwrap();
    let raw = writer.seal().unwrap();
    let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, &store, &raw, stats).unwrap();
    compact_ingress_document_with_markers(
        IngressProtocol::Responses,
        &mut document,
        &store,
        workspace.generated_markers(),
    )
    .unwrap();
    let request = crate::server::core_runtime::adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &document,
    )
    .unwrap();
    let crate::server::core_runtime::model_ir::ContentPart::ToolCall {
        raw_arguments: Some(actual),
        ..
    } = &request.messages[0].content[0]
    else {
        panic!("native invalid JSON arguments must stay on the raw-string path")
    };
    assert_eq!(actual, &malformed);
    drop((request, document, workspace, store, manager));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn large_value_incompatible_arguments_keep_the_native_raw_path() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-invalid-number-argument-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    for suffix in ["1e1000", r#""\uD800""#] {
        let store = manager
            .begin_request(
                BudgetTree::new(1024 * 1024, 1024 * 1024)
                    .unwrap()
                    .stream(1024 * 1024)
                    .unwrap(),
            )
            .unwrap();
        let raw_argument = format!(r#"{{"padding":"{}","value":{suffix}}}"#, "x".repeat(9000));
        let body = serde_json::to_vec(&json!({
            "model":"alias",
            "input":[{"type":"function_call","call_id":"call-1",
                "name":"lookup","arguments":raw_argument}]
        }))
        .unwrap();
        let mut writer = store.begin_raw().unwrap();
        writer.append(&body).unwrap();
        let raw = writer.seal().unwrap();
        let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
        let (mut document, mut workspace) =
            parse_ingress_document(IngressProtocol::Responses, &store, &raw, stats).unwrap();
        compact_ingress_document_with_markers(
            IngressProtocol::Responses,
            &mut document,
            &store,
            workspace.generated_markers(),
        )
        .unwrap();
        let request = crate::server::core_runtime::adapters::decode_ingress_request(
            IngressProtocol::Responses,
            &document,
        )
        .unwrap();
        let crate::server::core_runtime::model_ir::ContentPart::ToolCall {
            raw_arguments: Some(actual),
            ..
        } = &request.messages[0].content[0]
        else {
            panic!("Value-incompatible native arguments must remain raw")
        };
        assert_eq!(actual, &raw_argument);
    }
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn large_json_key_in_native_arguments_is_validated_without_a_second_owned_key() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-large-key-argument-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 1024 * 1024,
        record_bytes: 16 * 1024,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
        .unwrap()
        .stream(8 * 1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let arguments = format!(r#"{{"{}":1}}"#, "k".repeat(4_600_000));
    let body = serde_json::to_vec(&json!({
        "model":"alias",
        "input":[{"type":"function_call","call_id":"call-1","name":"lookup",
            "arguments":arguments}]
    }))
    .unwrap();
    let mut writer = store.begin_raw().unwrap();
    for chunk in body.chunks(16 * 1024) {
        writer.append(chunk).unwrap();
    }
    let raw = writer.seal().unwrap();
    let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, &store, &raw, stats).unwrap();
    compact_ingress_document_with_markers(
        IngressProtocol::Responses,
        &mut document,
        &store,
        workspace.generated_markers(),
    )
    .unwrap();
    let request = crate::server::core_runtime::adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &document,
    )
    .unwrap();
    let crate::server::core_runtime::model_ir::ContentPart::ToolCall {
        arguments: parsed,
        raw_arguments: None,
        ..
    } = &request.messages[0].content[0]
    else {
        panic!("large valid JSON key was not accepted")
    };
    assert_eq!(
        super::super::JsonValueExt::content_ref(parsed)
            .unwrap()
            .byte_len(),
        arguments.len() as u64
    );
    assert!(budget.snapshot().unwrap().peak <= 8 * 1024 * 1024);
    drop((request, workspace, document, store, manager));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn literal_marker_in_native_reasoning_extension_is_not_replaced_by_another_field() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-native-literal-marker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
        .unwrap()
        .stream(1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let summary = "x".repeat(16_000);
    // The raw request owns stream 1; the first semantic string gets stream 2.
    let literal = super::super::ContentRef::new(2, 0, 16_000, 16_000).wire_marker();
    let body = format!(
        r#"{{"model":"alias","input":[{{"type":"reasoning","summary":[{{"type":"summary_text","text":"{summary}"}}],"provider_extension":{{"foo":"{literal}"}}}}]}}"#
    );
    let mut writer = store.begin_raw().unwrap();
    writer.append(body.as_bytes()).unwrap();
    let raw = writer.seal().unwrap();
    let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, &store, &raw, stats).unwrap();
    compact_ingress_document_with_markers(
        IngressProtocol::Responses,
        &mut document,
        &store,
        workspace.generated_markers(),
    )
    .unwrap();
    assert_eq!(document["input"][0]["summary"][0]["text"], literal);
    assert_ne!(document["input"][0]["provider_extension"]["foo"], literal);
    let mut request = crate::server::core_runtime::adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &document,
    )
    .unwrap();
    let different = crate::server::core_runtime::adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":[{"type":"reasoning",
            "summary":[{"type":"summary_text","text":summary}],
            "provider_extension":{"foo":summary}}]}),
    )
    .unwrap();
    let key = [43; 32];
    let actual_history =
        crate::context_hold::visible_history_with_replay(&request, &key, Some(&store))
            .unwrap()
            .measure(None)
            .unwrap()
            .complete_digest;
    let different_history = crate::context_hold::visible_history(&different, &key)
        .unwrap()
        .measure(None)
        .unwrap()
        .complete_digest;
    assert_ne!(actual_history, different_history);
    super::super::externalize_model_request(&mut request, &store, 8 * 1024).unwrap();
    let profile =
        crate::server::core_runtime::profiles::CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            crate::server::core_runtime::profiles::fixed_reasoning("fixed"),
        );
    let template = crate::server::core_runtime::adapters::project_candidate_request_template(
        &request, &profile,
    )
    .unwrap();
    store
        .prevalidate(&super::super::model_content_refs(&request))
        .unwrap();
    let mut reader = crate::server::core_runtime::adapters::sequential_attempt_body(
        template,
        store.clone(),
        &budget,
        257,
    )
    .unwrap();
    let mut projected = Vec::new();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        projected.extend_from_slice(chunk.bytes());
    }
    let projected: Value = serde_json::from_slice(&projected).unwrap();
    assert_eq!(projected["input"][0]["summary"][0]["text"], summary);
    assert_eq!(projected["input"][0]["provider_extension"]["foo"], literal);
    drop((request, workspace, document, store, manager));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn marker_prefix_in_tool_identity_is_not_rewritten_or_hashed_as_content() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-tool-id-marker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let mut digests = Vec::new();
    for suffix in ["a", "b"] {
        let store = manager
            .begin_request(
                BudgetTree::new(1024 * 1024, 1024 * 1024)
                    .unwrap()
                    .stream(1024 * 1024)
                    .unwrap(),
            )
            .unwrap();
        let call_id = format!("call__hiroute_content_ref_v2_1_2_3_4__{suffix}");
        let body = serde_json::to_vec(&json!({
            "model":"alias","input":[{"type":"function_call","call_id":call_id,
                "name":"lookup","arguments":"{}"}]
        }))
        .unwrap();
        let mut writer = store.begin_raw().unwrap();
        writer.append(&body).unwrap();
        let raw = writer.seal().unwrap();
        let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
        let (mut document, mut workspace) =
            parse_ingress_document(IngressProtocol::Responses, &store, &raw, stats).unwrap();
        compact_ingress_document_with_markers(
            IngressProtocol::Responses,
            &mut document,
            &store,
            workspace.generated_markers(),
        )
        .unwrap();
        let request = crate::server::core_runtime::adapters::decode_ingress_request(
            IngressProtocol::Responses,
            &document,
        )
        .unwrap();
        let crate::server::core_runtime::model_ir::ContentPart::ToolCall { logical_id, .. } =
            &request.messages[0].content[0]
        else {
            panic!("tool call missing")
        };
        assert_eq!(logical_id, &call_id);
        digests.push(
            crate::context_hold::visible_history_with_replay(&request, &[44; 32], Some(&store))
                .unwrap()
                .measure(None)
                .unwrap()
                .complete_digest,
        );
    }
    assert_ne!(digests[0], digests[1]);
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}
