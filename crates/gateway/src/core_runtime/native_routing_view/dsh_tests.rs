use super::*;

fn envelope() -> Value {
    json!({"version":2,"agent":"dsh","purpose":"main",
        "session_id":"session","turn_id":"1"})
}

#[test]
fn absent_optional_input_is_correlation_only_without_selecting_a_human_window() {
    for protocol in [IngressProtocol::Responses, IngressProtocol::Messages] {
        let fixture = Fixture::new();
        let input = request(
            protocol,
            json!([
                {"role":"user","content":"actual human task"},
                {"role":"user","content":"native model-selection notification"}
            ]),
        );
        let original = serde_json::to_value(&input).unwrap();
        let mut explicit = envelope();
        explicit["input"] = json!({"kind":"observed_batch","event_id":"1"});
        for context in [envelope(), explicit] {
            let view = fixture.view(&input, context);
            assert_eq!(view.projection(), NativeInputProjection::CorrelationOnly);
            assert!(view.current_input().is_none());
            assert!(view.current_input_position().is_none());
            assert!(!view.current_input_unavailable());
            assert!(view.origin_unverified());
            assert!(view._reservation.is_none());
            assert!(std::ptr::eq(view.request(&input), &input));
            assert_eq!(
                serde_json::to_value(view.request(&input)).unwrap(),
                original
            );
        }
    }
}

#[test]
fn malformed_declared_input_remains_invalid_and_a_later_real_window_recovers() {
    let fixture = Fixture::new();
    let input = request(
        IngressProtocol::Responses,
        json!([
            {"role":"user","content":"actual human task"},
            {"role":"user","content":"native model-selection notification"},
            {"role":"assistant","content":"actual accepted output"},
            {"role":"user","content":"next actual task"}
        ]),
    );
    let original = serde_json::to_value(&input).unwrap();
    for invalid in [
        Value::Null,
        json!(false),
        json!([]),
        json!({}),
        json!({"kind":"future_input"}),
        json!({"kind":"observed_batch","event_id":false}),
        json!({"kind":"native_messages","messages":[]}),
    ] {
        let mut context = envelope();
        context["input"] = invalid;
        let view = fixture.view(&input, context);
        assert_eq!(view.projection(), NativeInputProjection::InvalidInput);
        assert!(view.current_input().is_none());
        assert!(view.current_input_position().is_none());
        assert!(!view.current_input_unavailable());
        assert!(view.origin_unverified());
        assert!(std::ptr::eq(view.request(&input), &input));
    }
    for (field, invalid) in [
        ("session_id", "foreign-session"),
        ("agent", "pi"),
        ("purpose", "unknown"),
    ] {
        let mut context = envelope();
        context[field] = invalid.into();
        assert_eq!(
            fixture.view(&input, context).projection(),
            NativeInputProjection::InvalidInput,
            "missing input must not bypass {field} validation"
        );
    }
    let hashes = [
        digest("actual human task"),
        digest("native model-selection notification"),
        digest("next actual task"),
    ];
    let mut context = envelope();
    context["turn_id"] = "2".into();
    context["input"] = json!({"kind":"native_messages","event_id":"2","coverage":"window",
        "messages":[{"index":2,"sha256":hashes[2]}],
        "scope_digest":format!("{:x}", Sha256::digest(serde_json::to_vec(&hashes).unwrap()))});
    let view = fixture.view(&input, context.clone());
    assert_eq!(view.projection(), NativeInputProjection::WindowApplied);
    assert_eq!(
        view.current_input().unwrap(),
        input.messages[3].content.as_slice()
    );
    assert_eq!(
        view.current_input_position(),
        Some(CurrentInputPosition::Window { first: 3, last: 3 })
    );
    assert!(view.origin_unverified());
    context["input"]["scope_digest"] = "0".repeat(64).into();
    let rejected = fixture.view(&input, context);
    assert_eq!(rejected.projection(), NativeInputProjection::WindowRejected);
    assert!(rejected.current_input().is_none());
    assert_eq!(serde_json::to_value(&input).unwrap(), original);
}
