use super::*;
use serde_json::json;

fn metadata(window: Value) -> Value {
    json!({"session_id":"session","thread_id":"thread","turn_id":"turn",
        "request_kind":"turn","window_id":window})
}

fn document(metadata: Value) -> Value {
    json!({"client_metadata":{"x-codex-turn-metadata":metadata.to_string()}})
}

fn known_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "user-agent",
        "codex_exec/0.147.0 (test-os; test-arch)".parse().unwrap(),
    );
    headers
}

#[test]
fn codex_window_counter_is_negative_evidence_and_unknown_shapes_stay_conservative() {
    for (window, expected) in [
        (json!("thread:0"), Some(0)),
        (json!("thread:1"), Some(1)),
        (json!("thread:18446744073709551615"), Some(u64::MAX)),
        (json!("thread:18446744073709551616"), None),
        (json!("thread:01"), None),
        (json!("thread:+1"), None),
        (json!("thread:-1"), None),
        (json!("thread:"), None),
        (json!("other:0"), None),
        (Value::Null, None),
    ] {
        let facts = facts(&known_headers(), &document(metadata(window.clone())));
        assert!(facts.track, "{window}");
        assert_eq!(facts.native.generation, expected, "{window}");
        assert_eq!(
            facts.native.compacted,
            expected.is_some_and(|value| value > 0),
            "{window}"
        );
        assert_eq!(
            facts.native.user_origin,
            if expected.is_some() {
                NativeUserOrigin::NativeTurn
            } else {
                NativeUserOrigin::Observed
            },
            "{window}"
        );
    }
}

#[test]
fn unsupported_codex_producer_records_uncertainty_even_at_zero_window() {
    for producer in [
        None,
        Some("codex_exec/0.146.0"),
        Some("codex_exec/0.148.0"),
        Some("unknown/0.147.0"),
    ] {
        let mut headers = HeaderMap::new();
        if let Some(producer) = producer {
            headers.insert("user-agent", producer.parse().unwrap());
        }
        let native = facts(&headers, &document(metadata(json!("thread:0")))).native;
        assert_eq!(native.user_origin, NativeUserOrigin::Observed);
    }
}

#[test]
fn codex_window_must_agree_in_every_present_native_channel() {
    for channel in ["http-window", "http-metadata", "body-flat"] {
        let mut headers = HeaderMap::new();
        let mut body = document(metadata(json!("thread:1")));
        match channel {
            "http-window" => {
                headers.insert("x-codex-window-id", "thread:0".parse().unwrap());
            }
            "http-metadata" => {
                headers.insert(
                    "x-codex-turn-metadata",
                    metadata(json!("thread:0")).to_string().parse().unwrap(),
                );
            }
            _ => {
                body["client_metadata"]["x-codex-window-id"] = json!("thread:0");
            }
        }
        let result = facts(&headers, &body);
        assert!(result.track, "{channel}");
        assert_eq!(
            result.native.user_origin,
            NativeUserOrigin::Observed,
            "{channel}"
        );
    }
    let mut headers = HeaderMap::new();
    headers.insert("x-codex-window-id", "thread:1".parse().unwrap());
    headers.insert(
        "x-codex-turn-metadata",
        metadata(json!("thread:1")).to_string().parse().unwrap(),
    );
    let mut body = document(metadata(json!("thread:1")));
    body["client_metadata"]["x-codex-window-id"] = json!("thread:1");
    assert!(facts(&headers, &body).track);
}
