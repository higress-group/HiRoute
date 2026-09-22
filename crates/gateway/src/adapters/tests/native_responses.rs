use super::*;

#[test]
fn responses_previous_response_id_has_an_explicit_http_boundary() {
    for body in [
        json!({"model":"alias","input":"hello"}),
        json!({"model":"alias","input":"hello","previous_response_id":null}),
    ] {
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        assert!(request.provider_state.is_empty());
    }
    for value in [json!("response"), json!({"id":"response"})] {
        let error = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"alias","input":"hello","previous_response_id":value}),
        )
        .unwrap_err();
        assert_eq!(error, ModelIrError::ResponsesPreviousResponseIdUnsupported);
        assert_eq!(
            ProtocolAdapterError::from(error).code(),
            "RESPONSES_PREVIOUS_RESPONSE_ID_UNSUPPORTED"
        );
    }
    for value in [json!(true), json!(1), json!([])] {
        assert_eq!(
            decode_ingress_request(
                IngressProtocol::Responses,
                &json!({"model":"alias","input":"hello","previous_response_id":value}),
            ),
            Err(ModelIrError::InvalidField("previous_response_id"))
        );
    }
}

#[test]
fn responses_to_messages_keeps_top_level_instructions_and_tool_history_but_rejects_mid_roles() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Messages,
        "glm-5.3",
        fixed_reasoning("fixed"),
    );
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model": "route", "stream": true, "instructions": "top-level policy",
            "input": [
                {"type":"message","role":"user","content":[{"type":"input_text","text":"first turn"}]},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"checking"}]},
                {"type":"function_call","call_id":"logical-call","name":"lookup","arguments":"{\"key\":\"v\"}"},
                {"type":"function_call_output","call_id":"logical-call","output":"result"},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"next turn"}]}
            ],
            "tools": [{"type":"function","name":"lookup","parameters":{"type":"object"}}],
            "tool_choice": "auto", "parallel_tool_calls": false
        }),
    )
    .unwrap();
    assert!(!request.requirements().mid_conversation_instructions);
    request.tool_id_map.push(ToolIdMapEntryV1 {
        logical_id: "logical-call".into(),
        native_id: "native-call".into(),
        kind: ToolKindV1::Function,
        name: "lookup".into(),
        namespace: None,
        owner: profile.exact_provider_path().unwrap(),
    });
    let body = project_candidate_request(&request, &profile).unwrap().body;
    assert_eq!(body["model"], "glm-5.3");
    assert_eq!(body["system"], "top-level policy");
    assert_eq!(body["stream"], true);
    assert_eq!(body["messages"][0]["content"][0]["text"], "first turn");
    assert_eq!(body["messages"][1]["content"][0]["text"], "checking");
    assert_eq!(body["messages"][2]["content"][0]["id"], "native-call");
    assert_eq!(
        body["messages"][2]["content"][0]["input"],
        json!({"key":"v"})
    );
    assert_eq!(
        body["messages"][3]["content"][0]["tool_use_id"],
        "native-call"
    );
    assert_eq!(body["messages"][4]["content"][0]["text"], "next turn");
    assert_eq!(
        body["tool_choice"],
        json!({"type":"auto","disable_parallel_tool_use":true})
    );

    let mut mid = request.clone();
    mid.messages.insert(
        1,
        crate::server::core_runtime::model_ir::CanonicalMessage {
            role: MessageRole::Developer,
            content: vec![ContentPart::Text {
                text: "late policy".into(),
            }],
            name: None,
        },
    );
    assert!(mid.requirements().mid_conversation_instructions);
    assert!(matches!(
        project_candidate_request(&mid, &profile),
        Err(ProtocolAdapterError::Capability(
            crate::server::core_runtime::profiles::CapabilityError::MidConversationInstructionsUnsupported
        ))
    ));
}

#[test]
fn codex_input_developer_prelude_is_initial_but_not_messages_representable() {
    let document = json!({
        "model": "route", "stream": true,
        "input": [
            {"type":"message","role":"developer","content":[{"type":"input_text","text":"bounds"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"first turn"}]},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"checking"}]},
            {"type":"function_call","call_id":"logical-call","name":"lookup","arguments":"{\"key\":\"v\"}"},
            {"type":"function_call_output","call_id":"logical-call","output":"result"},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"next turn"}]}
        ],
        "tools": [{"type":"function","name":"lookup","parameters":{"type":"object"}}],
        "tool_choice": "auto", "parallel_tool_calls": false
    });
    let mut request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
    let requirements = request.requirements();
    assert!(requirements.initial_instructions);
    assert!(!requirements.mid_conversation_instructions);
    assert!(requirements.function_tools && requirements.streaming);

    let native = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    request.tool_id_map.push(ToolIdMapEntryV1 {
        logical_id: "logical-call".into(),
        native_id: "native-call".into(),
        kind: ToolKindV1::Function,
        name: "lookup".into(),
        namespace: None,
        owner: native.exact_provider_path().unwrap(),
    });
    let projected = project_candidate_request(&request, &native).unwrap().body;
    assert_eq!(projected["input"][0]["role"], "developer");
    assert_eq!(projected["input"][3]["call_id"], "native-call");
    assert_eq!(projected["input"][5]["content"][0]["text"], "next turn");
    assert_eq!(projected["stream"], true);

    let messages = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Messages,
        "physical",
        fixed_reasoning("fixed"),
    );
    assert!(matches!(
        project_candidate_request(&request, &messages),
        Err(ProtocolAdapterError::ClientUnrepresentable(message))
            if message.contains("distinct Responses input system/developer role")
    ));

    let mut mid = document;
    mid["input"][5] = json!({"type":"message","role":"developer","content":[{"type":"input_text","text":"late bounds"}]});
    let mid = decode_ingress_request(IngressProtocol::Responses, &mid).unwrap();
    assert!(mid.requirements().mid_conversation_instructions);
    assert!(matches!(
        project_candidate_request(&mid, &messages),
        Err(ProtocolAdapterError::Capability(
            crate::server::core_runtime::profiles::CapabilityError::MidConversationInstructionsUnsupported
        ))
    ));
}

#[test]
fn responses_replayed_plain_reasoning_uses_the_existing_bounded_body_path() {
    let native = json!({"model":"alias","input":[{
        "type":"reasoning","id":"reasoning-item","summary":[],
        "content":[{"type":"reasoning_text","text":"plain reasoning ".repeat(4096)}],
        "provider_extension":{"version":1},"encrypted_content":null
    }]});
    let mut request = decode_ingress_request(IngressProtocol::Responses, &native).unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let expected = project_candidate_request(&request, &profile).unwrap().bytes;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&expected).unwrap()["input"],
        native["input"]
    );
    let cross_protocol = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("fixed"),
    );
    assert!(project_candidate_request(&request, &cross_protocol).is_err());

    let root = std::env::temp_dir().join(format!(
        "hiroute-reasoning-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
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
    externalize_model_request(&mut request, &store, 8).unwrap();
    let template = project_candidate_request_template(&request, &profile).unwrap();
    assert!(template.bytes.len() < expected.len());
    store.prevalidate(&model_content_refs(&request)).unwrap();
    let mut reader = sequential_attempt_body(template, store.clone(), &budget, 257).unwrap();
    let mut actual = Vec::new();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        actual.extend_from_slice(chunk.bytes());
    }
    reader.release();
    assert_eq!(actual, expected);
    drop(reader);
    drop(store);
    drop(manager);
    assert_eq!(budget.snapshot().unwrap().live, 0);
    fs_err_remove_dir(&root);
}

#[test]
fn responses_multitool_continuation_replays_externalized_reasoning_state() {
    let profile = exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
    let owner = profile.exact_provider_path().expect("exact owner");
    let opaque_first = "first-state".repeat(180);
    let opaque_second = "second-state".repeat(130);
    let bindings = IngressRequestBindings {
        provider_state_owner: Some(owner.clone()),
        tool_id_map: ["call-1", "call-2"]
            .map(|id| ToolIdMapEntryV1 {
                logical_id: id.into(),
                native_id: id.into(),
                kind: ToolKindV1::Function,
                name: "shell".into(),
                namespace: None,
                owner: owner.clone(),
            })
            .into(),
    };
    let mut request = decode_ingress_request_with_bindings(
        IngressProtocol::Responses,
        &json!({
            "model":"alias",
            "input":[
                {"type":"message","role":"user","content":[{"type":"input_text","text":"run two commands"}]},
                {"type":"reasoning","summary":[],"content":null,"encrypted_content":opaque_first},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"first"}]},
                {"type":"function_call","call_id":"call-1","name":"shell","arguments":"{\"command\":\"one\"}"},
                {"type":"function_call_output","call_id":"call-1","output":"done one"},
                {"type":"reasoning","summary":[],"content":null,"encrypted_content":opaque_second},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"second"}]},
                {"type":"function_call","call_id":"call-2","name":"shell","arguments":"{\"command\":\"two\"}"},
                {"type":"function_call_output","call_id":"call-2","output":"done two"}
            ]
        }),
        &bindings,
    )
    .expect("two round Responses history");
    let expected = project_candidate_request(&request, &profile)
        .expect("inline projection")
        .bytes;
    let root = std::env::temp_dir().join(format!(
        "hiroute-multitool-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).expect("budget tree");
    let budget = tree.stream(4 * 1024 * 1024).expect("stream budget");
    let store = manager.begin_request(budget.clone()).expect("replay store");
    externalize_model_request(&mut request, &store, 2300).expect("externalize late state");
    let states = request
        .messages
        .iter()
        .filter_map(|message| match message.content.first() {
            Some(ContentPart::ProviderState { state }) => Some(&state.value),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(states.len(), 2);
    assert!(states[0].content_ref().is_none());
    assert!(states[1].content_ref().is_some());
    let template = project_candidate_request_template(&request, &profile)
        .expect("externalized opaque continuation is still an exact Responses string");
    assert_eq!(template.wire_len, expected.len());
    store
        .prevalidate(&model_content_refs(&request))
        .expect("replay backing");
    let mut reader =
        sequential_attempt_body(template, store.clone(), &budget, 37).expect("attempt reader");
    let mut actual = Vec::new();
    while let Some(chunk) = reader.next_chunk().expect("continuation chunk") {
        actual.extend_from_slice(chunk.bytes());
    }
    reader.release();
    assert_eq!(actual, expected);
    drop(store);
    drop(manager);
    assert_eq!(budget.snapshot().expect("snapshot").live, 0);
    fs_err_remove_dir(&root);
}
