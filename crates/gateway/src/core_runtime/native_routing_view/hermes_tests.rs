use super::*;

fn headers(text: &str, reverse: usize) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-hiroute-agent", "hermes".parse().unwrap());
    headers.insert("x-hermes-session-id", "session".parse().unwrap());
    headers.insert(
        "x-hiroute-routing-context",
        json!({
            "version":1,"agent":"hermes","session_id":"session","purpose":"main","turn_id":"turn"
        })
        .to_string()
        .parse()
        .unwrap(),
    );
    headers.insert(
        "x-hermes-observed-input",
        json!({
            "version":1,"kind":"observed_turn_input","reverse_user_index":reverse,
            "text_sha256":format!("{:x}", Sha256::digest(text.as_bytes()))
        })
        .to_string()
        .parse()
        .unwrap(),
    );
    headers
}

fn input(text: &str) -> ModelRequestIRV1 {
    request(
        IngressProtocol::Responses,
        json!([
            {"role":"user","content":text},
            {"role":"assistant","content":"actual earlier execution"},
            {"type":"function_call","call_id":"call","name":"read","arguments":"{}"},
            {"type":"function_call_output","call_id":"call","output":"actual tool result"},
            {"role":"user","content":"a later native reminder"}
        ]),
    )
}

#[test]
fn legacy_clean_text_hint_is_non_authoritative_even_when_its_hash_matches() {
    let fixture = Fixture::new();
    let mut input = input("原任务\nexact text");
    let original = serde_json::to_value(&input).unwrap();
    let view = NativeRoutingView::prepare(
        &headers("原任务\nexact text", 1),
        &input,
        &fixture.replay,
        &fixture.budget,
    );
    assert!(view.current_input().is_none());
    assert!(view.origin_unverified());
    let projected = view.request(&input);
    assert_eq!(projected.messages[0].role, MessageRole::User);
    assert_eq!(projected.messages.last().unwrap().role, MessageRole::User);
    assert_eq!(projected.messages[1].role, MessageRole::Assistant);
    assert_eq!(projected.messages[3].content, input.messages[3].content);
    assert_eq!(serde_json::to_value(&input).unwrap(), original);
    // The source-only optional hint does not survive in execution/native IR.
    input.native_only = true;
    assert!(
        !serde_json::to_string(&input)
            .unwrap()
            .contains("observed_turn_input")
    );
}

#[test]
fn observed_hint_misses_keep_the_ordinary_request_without_claiming_fresh_user_proof() {
    let fixture = Fixture::new();
    let input = input("actual task");
    let valid = headers("actual task", 1);
    for variant in 0..10 {
        let mut headers = valid.clone();
        match variant {
            0 => {
                headers.insert("x-hermes-observed-input", "bad-json".parse().unwrap());
            }
            1 => {
                headers.insert("x-hermes-observed-input", serde_json::json!({"version":2,"kind":"observed_turn_input","reverse_user_index":1,"text_sha256":"a".repeat(64)}).to_string().parse().unwrap());
            }
            2 => {
                headers = self::headers("different task", 1);
            }
            3 => {
                headers = self::headers("actual task", 0);
            }
            4 => {
                headers = self::headers("actual task", 128);
            }
            5 => {
                headers.insert("x-hermes-session-id", "foreign-session".parse().unwrap());
            }
            6 => {
                headers.insert("x-hiroute-agent", "pi".parse().unwrap());
            }
            7 => {
                headers.append("x-hermes-observed-input", "duplicate".parse().unwrap());
            }
            8 => {
                headers.insert("x-hermes-observed-input", "x".repeat(513).parse().unwrap());
            }
            9 => {
                headers = self::headers("actual task\n", 1);
            }
            _ => unreachable!(),
        }
        let view = NativeRoutingView::prepare(&headers, &input, &fixture.replay, &fixture.budget);
        assert!(view.current_input().is_none(), "variant {variant}");
        assert!(std::ptr::eq(view.request(&input), &input));
    }
    let repeated = request(
        IngressProtocol::Responses,
        json!([
            {"role":"user","content":"actual task"},
            {"role":"user","content":"actual task"},
            {"role":"user","content":"a later native reminder"}
        ]),
    );
    let view = NativeRoutingView::prepare(&valid, &repeated, &fixture.replay, &fixture.budget);
    assert!(
        view.current_input().is_none(),
        "identical text is not an input-position proof"
    );
    assert!(view.origin_unverified());
}

#[test]
fn legacy_hint_does_not_scan_replay_or_reserve_projection_memory() {
    let fixture = Fixture::new();
    let text = "界\n".repeat(10_000);
    let mut input = input(&text);
    crate::content_ref::externalize_model_request(&mut input, &fixture.replay, 4096).unwrap();
    let headers = headers(&text, 1);
    let view = NativeRoutingView::prepare(&headers, &input, &fixture.replay, &fixture.budget);
    assert!(view.current_input().is_none());
    drop(view);
    let tiny = BudgetTree::new(4096, 4096).unwrap().stream(4096).unwrap();
    let view = NativeRoutingView::prepare(&headers, &input, &fixture.replay, &tiny);
    assert!(view.current_input().is_none());
    assert!(view.origin_unverified());
    assert_eq!(tiny.snapshot().unwrap().live, 0);
}

#[test]
fn repeated_continue_with_decorated_current_input_and_reminder_keeps_every_user() {
    let fixture = Fixture::new();
    for protocol in [
        IngressProtocol::ChatCompletions,
        IngressProtocol::Responses,
        IngressProtocol::Messages,
    ] {
        let text_type = if protocol == IngressProtocol::Responses {
            "input_text"
        } else {
            "text"
        };
        for reminder in [false, true] {
            let mut messages = vec![
                json!({"role":"user","content":"continue"}),
                json!({"role":"assistant","content":"accepted earlier output"}),
                json!({"role":"user","content":[
                    {"type":text_type,"text":"native memory and persistence override"},
                    {"type":text_type,"text":"continue"}
                ]}),
            ];
            if reminder {
                messages.push(json!({"role":"user","content":"native trailing reminder"}));
            }
            let body = match protocol {
                IngressProtocol::Responses => json!({"model":"route","input":messages}),
                _ => json!({"model":"route","messages":messages,"max_tokens":1024}),
            };
            let input = adapters::decode_ingress_request(protocol, &body).unwrap();
            let original = serde_json::to_value(&input).unwrap();
            let legacy = headers("continue", if reminder { 2 } else { 1 });
            let mut current = legacy.clone();
            current.insert(
                "x-hiroute-routing-context",
                json!({"version":2,"agent":"hermes",
                "session_id":"session","purpose":"main","turn_id":"turn",
                "input":{"kind":"observed_batch","event_id":"turn"}})
                .to_string()
                .parse()
                .unwrap(),
            );
            for headers in [&legacy, &current] {
                let view =
                    NativeRoutingView::prepare(headers, &input, &fixture.replay, &fixture.budget);
                assert!(view.current_input().is_none());
                assert!(view.current_input_position().is_none());
                assert!(!view.current_input_unavailable());
                assert!(view.origin_unverified());
                assert!(std::ptr::eq(view.request(&input), &input));
                assert_eq!(
                    serde_json::to_value(view.request(&input)).unwrap(),
                    original
                );
                assert_eq!(
                    input
                        .messages
                        .iter()
                        .filter(|m| m.role == MessageRole::User)
                        .count(),
                    if reminder { 3 } else { 2 }
                );
                assert_eq!(input.messages[2].content.len(), 2);
            }
        }
    }
}
