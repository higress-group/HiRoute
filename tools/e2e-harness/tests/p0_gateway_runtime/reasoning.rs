use super::*;

fn reasoning_wire_value(wire: &[u8]) -> serde_json::Value {
    let offset = wire
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap()
        + 4;
    serde_json::from_slice::<serde_json::Value>(&wire[offset..]).unwrap()["reasoning"]["effort"]
        .clone()
}

fn assert_fixed_effort(effort: Option<serde_json::Value>, expected: Option<&str>) {
    let primary = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let other = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: FALLBACK_OK,
    }]);
    let fixture = RuntimeFixture::launch_reasoning(&[&primary, &other], true);
    let mut request = serde_json::json!({"model":MODEL,"input":"hello","stream":false});
    if let Some(effort) = effort {
        request["reasoning"] = serde_json::json!({"effort":effort});
    }
    let response = fixture.request_body(&serde_json::to_vec(&request).unwrap());
    if let Some(expected) = expected {
        assert_eq!(response.status, 200);
        assert_eq!(primary.calls(), 1);
        assert_eq!(reasoning_wire_value(&primary.requests()[0]), expected);
    } else {
        assert_eq!(response.status, 400);
        assert_eq!(primary.calls(), 0);
    }
    assert_eq!(other.calls(), 0);
}

#[test]
fn fixed_effort_low_reaches_actual_upstream() {
    assert_fixed_effort(Some(serde_json::json!("low")), Some("low"));
}

#[test]
fn fixed_effort_high_reaches_actual_upstream() {
    assert_fixed_effort(Some(serde_json::json!("high")), Some("high"));
}

#[test]
fn fixed_effort_absent_uses_bound_default() {
    assert_fixed_effort(None, Some("low"));
}

#[test]
fn fixed_effort_large_input_preserves_choice_after_content_externalization() {
    let primary = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let other = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: FALLBACK_OK,
    }]);
    let fixture = RuntimeFixture::launch_reasoning(&[&primary, &other], true);
    let input = serde_json::to_string(&"hello".repeat(4096)).unwrap();
    let response = fixture.request_body(
        format!(r#"{{"model":"{MODEL}","input":{input},"stream":false,"reasoning":{{"effort":"high"}}}}"#).as_bytes(),
    );
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!((primary.calls(), other.calls()), (1, 0));
    assert_eq!(reasoning_wire_value(&primary.requests()[0]), "high");
}

#[test]
fn fixed_effort_late_model_reaches_upstream_after_large_input() {
    let primary = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let other = NativeProvider::start(Vec::new());
    let fixture = RuntimeFixture::launch_reasoning(&[&primary, &other], true);
    let input = serde_json::to_string(&"hello".repeat(4096)).unwrap();
    let body = format!(
        r#"{{"input":{input},"model":"{MODEL}","stream":false,"reasoning":{{"effort":"high"}}}}"#
    );
    let response = fixture.request_body(body.as_bytes());
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!((primary.calls(), other.calls()), (1, 0));
    assert_eq!(reasoning_wire_value(&primary.requests()[0]), "high");
}

#[test]
fn fixed_effort_unsupported_is_rejected_before_upstream() {
    assert_fixed_effort(Some(serde_json::json!("ultra")), None);
}

#[test]
fn fixed_effort_non_string_is_rejected_before_upstream() {
    assert_fixed_effort(Some(serde_json::json!(42)), None);
}

#[test]
fn fixed_effort_null_is_rejected_before_upstream() {
    assert_fixed_effort(Some(serde_json::Value::Null), None);
}

#[test]
fn plan_effort_does_not_override_distinct_candidate_settings() {
    let primary = NativeProvider::start(vec![ProviderReply::Complete {
        status: 429,
        error_kind: None,
        body: QUOTA_ERROR,
    }]);
    let fallback = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: FALLBACK_OK,
    }]);
    let fixture = RuntimeFixture::launch_reasoning(&[&primary, &fallback], false);
    let response = fixture.request_body(
        &serde_json::to_vec(&serde_json::json!({
            "model":MODEL,"input":"hello","stream":false,"reasoning":{"effort":"medium"}
        }))
        .unwrap(),
    );
    assert_eq!(response.status, 200);
    assert_eq!((primary.calls(), fallback.calls()), (1, 1));
    assert_eq!(reasoning_wire_value(&primary.requests()[0]), "low");
    assert_eq!(reasoning_wire_value(&fallback.requests()[0]), "high");
}
