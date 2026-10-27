//! Declared reasoning loss cannot authorize dropping unknown native semantics.
use super::*;
use crate::context_hold::visible_history;
use hiroute_gateway_core::runtime::body::StreamBudget;

fn document(reasoning: serde_json::Value) -> serde_json::Value {
    json!({"model":"alias","input":[reasoning,
        {"type":"message","role":"user","content":[
            {"type":"input_text","text":"continue"}
        ]}
    ]})
}

fn assert_unrepresentable(result: Result<(), ProtocolAdapterError>) {
    let error = result.expect_err("unknown reasoning semantics must not silently disappear");
    assert_eq!(error.code(), "CLIENT_PROTOCOL_UNREPRESENTABLE");
}

fn assert_cross_protocol_rejected(
    request: &crate::server::core_runtime::model_ir::ModelRequestIRV1,
) {
    for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
        let profile = exact_state_profile(IngressProtocol::Responses, target);
        assert_unrepresentable(project_candidate_request_template(request, &profile).map(|_| ()));
        assert_unrepresentable(
            project_candidate_request_template_with_cleanup(request, &profile, Some(1)).map(|_| ()),
        );
    }
}

fn with_replay(
    mut request: crate::server::core_runtime::model_ir::ModelRequestIRV1,
    check: impl FnOnce(
        &crate::server::core_runtime::model_ir::ModelRequestIRV1,
        &crate::replay::ReplayStore,
        &StreamBudget,
    ),
) {
    let root = std::env::temp_dir().join(format!(
        "hiroute-reasoning-extension-{}-{}",
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
    let budget = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024)
        .unwrap()
        .stream(4 * 1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    externalize_model_request(&mut request, &store, 0).unwrap();
    assert!(!model_content_refs(&request).is_empty());
    store.prevalidate(&model_content_refs(&request)).unwrap();
    check(&request, &store, &budget);
    drop((request, store, manager));
    assert_eq!(budget.snapshot().unwrap().live, 0);
    fs_err_remove_dir(&root);
}

fn replay_body(
    request: &crate::server::core_runtime::model_ir::ModelRequestIRV1,
    target: IngressProtocol,
    store: &crate::replay::ReplayStore,
    budget: &StreamBudget,
) -> serde_json::Value {
    let profile = exact_state_profile(request.ingress_protocol, target);
    let template = project_candidate_request_template(request, &profile).unwrap();
    let mut reader = sequential_attempt_body(template, store.clone(), budget, 127).unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        bytes.extend_from_slice(chunk.bytes());
    }
    reader.release();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn unknown_responses_reasoning_siblings_cannot_use_declared_loss_as_a_mapping() {
    for encrypted in [
        None,
        Some(json!(null)),
        Some(json!("")),
        Some(json!("ciphertext")),
    ] {
        let mut reasoning = json!({"type":"reasoning","summary":[],
            "provider_extension":{"continuation":"unknown native state"}});
        if let Some(value) = encrypted {
            reasoning["encrypted_content"] = value;
        }
        let original = document(reasoning);
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
            let profile = exact_state_profile(IngressProtocol::Responses, target);
            assert_unrepresentable(project_candidate_request(&request, &profile).map(|_| ()));
        }
        assert_cross_protocol_rejected(&request);
        assert!(
            visible_history(&request, &[19; 32]).is_none(),
            "unknown history cannot authorize affinity or cleanup"
        );
        let native = project_candidate_request(
            &request,
            &exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses),
        )
        .unwrap();
        assert_eq!(native.body["input"], original["input"]);
        assert_eq!(request.native_body.as_ref(), Some(&original));
    }
}

#[test]
fn unknown_responses_reasoning_parts_are_not_ordinary_plaintext() {
    for reasoning in [
        json!({"type":"reasoning","summary":[{"type":"summary_text","text":"visible",
            "provider_extension":{"continuation":"opaque"}}]}),
        json!({"type":"reasoning","content":[{"type":"reasoning_text","text":"visible",
            "provider_extension":{"continuation":"opaque"}}]}),
        json!({"type":"reasoning","summary":[{"type":"future_summary","text":"visible"}]}),
        json!({"type":"reasoning","content":{"continuation":"opaque"}}),
    ] {
        let original = document(reasoning);
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        assert_cross_protocol_rejected(&request);
        let native = project_candidate_request(
            &request,
            &exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses),
        )
        .unwrap();
        assert_eq!(native.body["input"], original["input"]);
    }
}

#[test]
fn replay_externalization_preserves_unknown_reasoning_and_its_projection_rejection() {
    for reasoning in [
        json!({"type":"reasoning","summary":[],"encrypted_content":null,
            "provider_extension":{"continuation":"opaque ".repeat(512)}}),
        json!({"type":"reasoning","summary":[{"type":"summary_text","text":"visible ".repeat(512),
            "provider_extension":{"continuation":"opaque ".repeat(512)}}]}),
    ] {
        let original = document(reasoning);
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        with_replay(request, |request, store, budget| {
            assert_cross_protocol_rejected(request);
            let native = replay_body(request, IngressProtocol::Responses, store, budget);
            assert_eq!(native["input"], original["input"]);
            assert_unrepresentable(
                project_candidate_request_template_with_cleanup(
                    request,
                    &exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses),
                    Some(1),
                )
                .map(|_| ()),
            );
        });
    }
}

#[test]
fn unknown_reasoning_extensions_still_roundtrip_on_the_original_protocol() {
    let original = document(json!({"type":"reasoning","id":"native-item",
        "summary":[{"type":"summary_text","text":"visible ".repeat(512)}],
        "encrypted_content":"native ciphertext",
        "provider_extension":{"continuation":"opaque ".repeat(512)}}));
    let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    let profile = exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
    let native = project_candidate_request(&request, &profile).unwrap();
    assert_eq!(native.body["input"], original["input"]);
    assert_eq!(request.native_body.as_ref(), Some(&original));
    let inline = project_candidate_request_template(&request, &profile).unwrap();
    assert_eq!(inline.bytes.as_ref(), native.bytes.as_slice());
    with_replay(request, |request, store, budget| {
        assert_eq!(
            replay_body(request, IngressProtocol::Responses, store, budget),
            native.body
        );
    });
}

#[test]
fn known_responses_reasoning_remains_portable_inline_and_in_replay_with_declared_loss() {
    for encrypted in [
        None,
        Some(json!(null)),
        Some(json!("")),
        Some(json!("ciphertext")),
    ] {
        let mut reasoning = json!({"type":"reasoning","id":"reasoning-id","status":null,
            "summary":[{"type":"summary_text","text":"summary"}],
            "content":[{"type":"reasoning_text","text":"thought"}]});
        let opaque = encrypted
            .as_ref()
            .is_some_and(|value| value == "ciphertext");
        if let Some(value) = encrypted {
            reasoning["encrypted_content"] = value;
        }
        let original = document(reasoning);
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        assert!(!request.native_only);
        let loss = |target| super::super::reasoning_loss::request_loss(&request, target, None);
        assert_eq!(loss(IngressProtocol::Responses), 0);
        assert_eq!(loss(IngressProtocol::ChatCompletions), usize::from(opaque));
        assert_eq!(loss(IngressProtocol::Messages), 1);
        let expected = [
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            IngressProtocol::Messages,
        ]
        .map(|target| {
            let profile = exact_state_profile(IngressProtocol::Responses, target);
            let direct = project_candidate_request(&request, &profile).unwrap().body;
            let template = project_candidate_request_template(&request, &profile).unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&template.bytes).unwrap(),
                direct
            );
            direct
        });
        assert_eq!(expected[0]["input"], original["input"]);
        assert_eq!(expected[1]["messages"][0]["reasoning_content"], "summary");
        assert_eq!(expected[1]["messages"][1]["reasoning_content"], "thought");
        assert_eq!(expected[2]["messages"].as_array().unwrap().len(), 1);
        with_replay(request, |request, store, budget| {
            for (target, expected) in [
                IngressProtocol::Responses,
                IngressProtocol::ChatCompletions,
                IngressProtocol::Messages,
            ]
            .into_iter()
            .zip(expected)
            {
                assert_eq!(replay_body(request, target, store, budget), expected);
            }
            assert_eq!(
                super::super::reasoning_loss::request_loss(
                    request,
                    IngressProtocol::ChatCompletions,
                    None
                ),
                usize::from(opaque)
            );
        });
    }
}

#[test]
fn known_messages_signed_unsigned_and_redacted_reasoning_keep_their_existing_mappings() {
    let original = json!({"model":"alias","max_tokens":128,"messages":[
        {"role":"assistant","content":[
            {"type":"thinking","thinking":"signed thought","signature":"native signature"},
            {"type":"thinking","thinking":"unsigned thought"},
            {"type":"redacted_thinking","data":"native redacted state"}
        ]}, {"role":"user","content":"continue"}
    ]});
    let request = decode_ingress_request(IngressProtocol::Messages, &original).unwrap();
    assert!(!request.native_only);
    let expected = [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ]
    .map(|target| {
        let body = project_candidate_request(
            &request,
            &exact_state_profile(IngressProtocol::Messages, target),
        )
        .unwrap()
        .body;
        assert_eq!(
            super::super::reasoning_loss::request_loss(&request, target, None),
            if target == IngressProtocol::Messages {
                0
            } else {
                3
            }
        );
        body
    });
    assert_eq!(expected[2]["messages"], original["messages"]);
    assert!(
        expected[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["reasoning_content"] == "signed thought")
    );
    assert!(
        expected[0]["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["summary"][0]["text"] == "signed thought")
    );
    with_replay(request, |request, store, budget| {
        for (target, expected) in [
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            IngressProtocol::Messages,
        ]
        .into_iter()
        .zip(expected)
        {
            assert_eq!(replay_body(request, target, store, budget), expected);
        }
    });
}
