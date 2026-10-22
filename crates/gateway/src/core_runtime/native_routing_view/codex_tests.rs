use super::*;

fn peer(author: &str, text: &str) -> Value {
    json!({"type":"agent_message","author":author,"recipient":"parent",
        "content":[{"type":"input_text","text":text}]})
}

fn view(fixture: &Fixture, request: &ModelRequestIRV1) -> NativeRoutingView {
    NativeRoutingView::prepare(&HeaderMap::new(), request, &fixture.replay, &fixture.budget)
}

#[test]
fn plaintext_peer_batch_is_current_input_without_changing_native_history() {
    let fixture = Fixture::new();
    let original = request(
        IngressProtocol::Responses,
        json!([
            {"role":"user","content":"parent task"},
            {"role":"assistant","content":"actual earlier output"},
            peer("first-child", "first peer result"),
            peer("second-child", "second peer result"),
            {"type":"function_call","call_id":"call","name":"read","arguments":"{}"},
            {"type":"function_call_output","call_id":"call","output":"actual tool result"}
        ]),
    );
    let saved = original.clone();
    let selected = view(&fixture, &original);
    assert_eq!(
        selected.current_input().unwrap(),
        &[
            ContentPart::Text {
                text: "first peer result".into()
            },
            ContentPart::Text {
                text: "second peer result".into()
            },
        ]
    );
    assert!(selected.origin_unverified());
    assert_eq!(
        selected.current_input_position(),
        Some(CurrentInputPosition::Event(3))
    );
    assert!(!selected.current_input_unavailable());
    assert!(std::ptr::eq(selected.request(&original), &original));
    assert_eq!(original, saved);
    assert_eq!(original.messages[0].role, MessageRole::User);
    assert!(original.messages[2].content.is_empty());
    assert_eq!(original.messages[2].role, MessageRole::Assistant);
}

#[test]
fn peer_only_native_input_needs_no_invented_human_user() {
    let fixture = Fixture::new();
    let original = request(
        IngressProtocol::Responses,
        json!([peer("parent", "delegated work")]),
    );
    let selected = view(&fixture, &original);
    assert_eq!(
        selected.current_input().unwrap(),
        &[ContentPart::Text {
            text: "delegated work".into()
        }]
    );
    assert!(selected.origin_unverified());
    assert_eq!(
        selected.current_input_position(),
        Some(CurrentInputPosition::Event(0))
    );
    assert!(
        original
            .messages
            .iter()
            .all(|message| message.role != MessageRole::User)
    );
}

#[test]
fn old_peer_context_cannot_supersede_a_later_user_event_or_peer_batch() {
    let fixture = Fixture::new();
    let mut input = json!([
        {"role":"user","content":"parent task"},
        peer("old-child", "old peer result"),
        {"role":"assistant","content":"actual prior peer execution"},
        {"role":"user","content":"new ordinary task"}
    ]);
    let original = request(IngressProtocol::Responses, input.clone());
    assert!(view(&fixture, &original).current_input().is_none());
    assert!(!view(&fixture, &original).current_input_unavailable());
    input
        .as_array_mut()
        .unwrap()
        .push(peer("new-child", "new peer trigger"));
    let current = request(IngressProtocol::Responses, input);
    assert_eq!(
        view(&fixture, &current).current_input().unwrap(),
        &[ContentPart::Text {
            text: "new peer trigger".into()
        }]
    );
}

#[test]
fn mixed_ciphertext_empty_and_unknown_peer_content_remain_opaque() {
    let fixture = Fixture::new();
    for content in [
        json!([]),
        json!([{"type":"input_text","text":" \n"}]),
        json!([{"type":"input_text","text":"readable wrapper"},
            {"type":"encrypted_content","encrypted_content":"opaque task"}]),
    ] {
        let mut native_peer = peer("child", "ignored");
        native_peer["content"] = content;
        let original = request(
            IngressProtocol::Responses,
            json!([
                {"role":"user","content":"ancestor is not encrypted intent"}, native_peer
            ]),
        );
        assert!(view(&fixture, &original).current_input().is_none());
        assert!(view(&fixture, &original).origin_unverified());
        assert!(view(&fixture, &original).current_input_unavailable());
        assert_eq!(
            view(&fixture, &original).current_input_position(),
            Some(CurrentInputPosition::Event(1))
        );
    }
    let mut original = request(IngressProtocol::Responses, json!([peer("child", "text")]));
    original.native_body.as_mut().unwrap()["input"][0]["content"][0]["type"] =
        json!("future-content");
    assert!(view(&fixture, &original).current_input().is_none());
    assert!(view(&fixture, &original).current_input_unavailable());
}

#[test]
fn peer_text_replay_and_projection_capacity_failures_are_optional() {
    let fixture = Fixture::new();
    let text = "native peer 中文\n".repeat(1024);
    let marker = fixture
        .replay
        .store_content(text.as_bytes())
        .unwrap()
        .wire_marker();
    let original = request(IngressProtocol::Responses, json!([peer("child", &marker)]));
    assert_eq!(
        view(&fixture, &original).current_input().unwrap(),
        &[ContentPart::Text { text }]
    );
    let tiny = BudgetTree::new(4096, 4096).unwrap().stream(4096).unwrap();
    let selected = NativeRoutingView::prepare(&HeaderMap::new(), &original, &fixture.replay, &tiny);
    assert!(selected.current_input().is_none());
    assert!(selected.current_input_unavailable());
    assert_eq!(
        selected.current_input_position(),
        Some(CurrentInputPosition::Event(0))
    );
    assert!(std::ptr::eq(selected.request(&original), &original));
    let oversized = request(
        IngressProtocol::Responses,
        json!([peer("child", &"x".repeat(MAX_USER_TEXT_BYTES + 1))]),
    );
    assert!(view(&fixture, &oversized).current_input().is_none());
    let many = request(
        IngressProtocol::Responses,
        json!((0..33).map(|_| peer("child", "peer")).collect::<Vec<_>>()),
    );
    assert!(view(&fixture, &many).current_input().is_none());
}

#[test]
fn complete_peer_fingerprints_retain_kind_author_recipient_and_every_content_variant() {
    let fixture = Fixture::new();
    let input = json!([
        {"role":"user","content":"parent task"},
        peer("child", "peer text"),
        {"type":"agent_message","author":"another-child","recipient":"parent","content":[
            {"type":"input_text","text":"wrapper"},
            {"type":"encrypted_content","encrypted_content":"opaque bytes"}
        ]}
    ]);
    let digest = |input| {
        let original = request(IngressProtocol::Responses, input);
        crate::context_hold::observed_history_with_replay(
            &original,
            &[9; 32],
            Some(&fixture.replay),
        )
        .unwrap()
        .measure(None)
        .unwrap()
        .complete_digest
    };
    let original = digest(input.clone());
    for pointer in [
        "/1/author",
        "/1/recipient",
        "/1/content/0/text",
        "/2/content/0/text",
        "/2/content/1/encrypted_content",
    ] {
        let mut changed = input.clone();
        *changed.pointer_mut(pointer).unwrap() = json!("changed");
        assert_ne!(original, digest(changed), "{pointer}");
    }
    let mut reordered = input;
    reordered.as_array_mut().unwrap().swap(1, 2);
    assert_ne!(original, digest(reordered));
}

#[test]
fn additional_tools_and_web_search_keep_the_peer_event_at_its_canonical_position() {
    let fixture = Fixture::new();
    let input = json!([
        {"type":"additional_tools","tools":[]},
        {"role":"user","content":"task"},
        {"type":"web_search_call","id":"search","status":"completed", "action":{"type":"search","query":"weather"}},
        peer("child", "current peer")
    ]);
    let mut original = request(IngressProtocol::Responses, input.clone());
    let selected = view(&fixture, &original);
    assert_eq!(
        selected.current_input_position(),
        Some(CurrentInputPosition::Event(2))
    );
    assert_eq!(original.messages.len(), 3);
    let digest = |request: &ModelRequestIRV1| {
        crate::context_hold::observed_history_with_replay(request, &[7; 32], Some(&fixture.replay))
            .unwrap()
            .measure(None)
            .unwrap()
            .complete_digest
    };
    let mut changed = input;
    changed[3]["author"] = json!("different-child");
    assert_ne!(
        digest(&original),
        digest(&request(IngressProtocol::Responses, changed))
    );
    original.messages.remove(0);
    assert!(view(&fixture, &original).current_input().is_none());
}

#[test]
fn over_bound_native_scan_never_asserts_an_ancestor_as_current_peer_input() {
    let fixture = Fixture::new();
    let mut original = request(
        IngressProtocol::Responses,
        json!([
            {"role":"user","content":"ancestor task"},
            peer("child", "current peer")
        ]),
    );
    original.native_body.as_mut().unwrap()["input"] = json!(vec![Value::Null; 65_537]);
    let selected = view(&fixture, &original);
    assert!(selected.current_input().is_none());
    assert!(selected.current_input_unavailable());
    assert!(selected.current_input_position().is_none());
    assert!(std::ptr::eq(selected.request(&original), &original));
}

#[test]
fn optional_adapter_window_cannot_override_a_typed_plaintext_or_opaque_peer_event() {
    let fixture = Fixture::new();
    for opaque in [false, true] {
        let mut native_peer = peer("child", "current peer trigger");
        if opaque {
            native_peer["content"].as_array_mut().unwrap().push(json!({
                "type":"encrypted_content", "encrypted_content":"opaque task"
            }));
        }
        let mut input = json!([
            {"role":"user","content":"ancestor task"}, native_peer
        ]);
        for envelope in [
            json!({"version":2,"agent":"dsh","session_id":"session","purpose":"main","input":{"kind":"unsupported"}}),
            json!({"version":2,"agent":"dsh","session_id":"session","purpose":"main","input":{
                "kind":"native_messages","event_id":"event","coverage":"window",
                "messages":[{"index":0,"sha256":digest("ancestor task")}],
                "scope_digest":format!("{:x}",Sha256::digest(serde_json::to_vec(&vec![digest("ancestor task")]).unwrap()))
            }}),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("x-hiroute-agent", "dsh".parse().unwrap());
            headers.insert(
                "x-hiroute-routing-context",
                envelope.to_string().parse().unwrap(),
            );
            let original = request(IngressProtocol::Responses, input.clone());
            let selected =
                NativeRoutingView::prepare(&headers, &original, &fixture.replay, &fixture.budget);
            assert_eq!(
                selected.current_input_position(),
                Some(CurrentInputPosition::Event(1))
            );
            assert_eq!(selected.current_input_unavailable(), opaque);
            if !opaque {
                assert_eq!(
                    selected.current_input().unwrap(),
                    &[ContentPart::Text {
                        text: "current peer trigger".into()
                    }]
                );
            } else {
                assert!(selected.current_input().is_none());
            }
        }
        input
            .as_array_mut()
            .unwrap()
            .push(json!({"role":"user","content":"fresh recovery input"}));
        let recovered = request(IngressProtocol::Responses, input);
        let selected = view(&fixture, &recovered);
        assert!(!selected.current_input_unavailable());
        assert!(selected.current_input_position().is_none());
        assert!(selected.current_input().is_none());
        assert_eq!(
            recovered.messages.last().unwrap().content,
            vec![ContentPart::Text {
                text: "fresh recovery input".into()
            }]
        );
    }
}
