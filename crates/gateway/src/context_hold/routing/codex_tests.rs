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
fn compact_context_document_preserves_codex_metadata_and_conflicts() {
    for value in [
        metadata(json!("thread:1")),
        metadata(Value::Null),
        json!({"request_kind":"memory"}),
    ] {
        let mut body = document(value);
        body["input"] = json!([{"role":"user","content":"large-history".repeat(4096)}]);
        for conflict in [false, true] {
            if conflict {
                body["client_metadata"]["session_id"] = json!("different");
            }
            let compact = crate::context_hold::context_document(&body);
            assert!(compact.get("input").is_none());
            assert_eq!(
                facts(&known_headers(), &compact),
                facts(&known_headers(), &body)
            );
        }
    }
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

#[test]
fn known_auxiliaries_are_purpose_first_and_unknown_kinds_remain_routable() {
    for kind in ["memory", "prewarm", "compaction"] {
        for enrichment in [Value::Null, json!({"invalid":"optional turn"})] {
            let mut body = document(json!({"request_kind":kind,"turn_id":enrichment}));
            // Memory's canonical payload omits these dimensions even though
            // core still emits its flat compatibility identity and window.
            body["client_metadata"]["session_id"] = json!("session");
            body["client_metadata"]["thread_id"] = json!("thread");
            body["client_metadata"]["x-codex-window-id"] = json!("thread:4");
            let mut headers = known_headers();
            headers.insert("session-id", "session".parse().unwrap());
            headers.insert("thread-id", "thread".parse().unwrap());
            headers.insert("x-codex-window-id", "thread:4".parse().unwrap());
            assert!(!facts(&headers, &body).track, "{kind}: {body}");
        }
    }
    for kind in [
        json!("future-task"),
        json!("summary"),
        Value::Null,
        json!(9),
    ] {
        let mut value = metadata(json!("thread:0"));
        value["request_kind"] = kind.clone();
        let result = facts(&known_headers(), &document(value));
        assert!(result.track, "{kind}");
        assert_eq!(result.native.user_origin, NativeUserOrigin::Observed);
    }
}

#[test]
fn malformed_and_duplicate_canonical_metadata_do_not_detach_main_history() {
    for raw in [
        r#"{"request_kind":"turn","request_kind":"memory"}"#,
        r#"{"request_kind":"memory","thread_id":"one","thread_id":"two"}"#,
        r#"{"request_kind":"memory","window_id":"one","window_id":"two"}"#,
        r#"{"request_kind":"memory","parent_thread_id":"one","parent_thread_id":"two"}"#,
        r#"{"request_kind":"memory""#,
        r#"["memory"]"#,
    ] {
        for use_header in [false, true] {
            let mut headers = known_headers();
            let mut body = json!({});
            if use_header {
                headers.insert("x-codex-turn-metadata", raw.parse().unwrap());
            } else {
                body["client_metadata"] = json!({"x-codex-turn-metadata":raw});
            }
            let result = facts(&headers, &body);
            assert!(result.track, "{raw}");
            assert_eq!(result.native.user_origin, NativeUserOrigin::Observed);
        }
    }
    let mut headers = known_headers();
    for _ in 0..2 {
        headers.append(
            "x-codex-turn-metadata",
            r#"{"request_kind":"memory"}"#.parse().unwrap(),
        );
    }
    assert!(facts(&headers, &json!({})).track);
}

#[test]
fn auxiliary_purpose_cannot_be_spliced_from_conflicting_metadata_channels() {
    for field in [
        "request_kind",
        "session_id",
        "thread_id",
        "turn_id",
        "window_id",
        "parent_thread_id",
        "parent_turn_id",
        "forked_from_thread_id",
        "installation_id",
    ] {
        let mut canonical = metadata(json!("thread:1"));
        canonical["request_kind"] = json!("compaction");
        let mut header = canonical.clone();
        header[field] = json!("other");
        let mut headers = known_headers();
        headers.insert("x-codex-turn-metadata", header.to_string().parse().unwrap());
        let result = facts(&headers, &document(canonical));
        assert!(result.track, "{field}");
        assert_eq!(result.native.user_origin, NativeUserOrigin::Observed);
    }
    let canonical = json!({"request_kind":"memory"});
    let mut body = document(canonical);
    body["client_metadata"]["session_id"] = json!("other");
    let mut headers = known_headers();
    headers.insert("session-id", "session".parse().unwrap());
    assert!(facts(&headers, &body).track);
}

#[test]
fn canonical_body_can_enrich_code_mode_without_disagreeing_with_compatibility_header() {
    let mut canonical = metadata(json!("thread:0"));
    let mut headers = known_headers();
    headers.insert(
        "x-codex-turn-metadata",
        canonical.to_string().parse().unwrap(),
    );
    canonical["code_mode_tool_names"] = json!({"alias":"full native tool name"});
    assert_eq!(
        facts(&headers, &document(canonical)).native.user_origin,
        NativeUserOrigin::NativeTurn
    );
}

#[test]
fn child_and_fork_references_do_not_replace_the_current_thread_window() {
    let mut canonical = metadata(json!("child:2"));
    canonical["thread_id"] = json!("child");
    canonical["parent_thread_id"] = json!("parent");
    canonical["parent_turn_id"] = json!("parent-turn");
    canonical["forked_from_thread_id"] = json!("fork-source");
    let result = facts(&known_headers(), &document(canonical));
    assert!(result.track);
    assert_eq!(result.native.turn, Some(digest("turn")));
    assert_eq!(result.native.generation, Some(2));
    assert!(result.root.is_none());
}

#[test]
fn positive_identity_conflicts_are_distinct_from_missing_or_malformed_enrichment() {
    for field in ["session_id", "thread_id"] {
        let canonical = metadata(json!("thread:0"));
        let mut body = document(canonical.clone());
        body["client_metadata"][field] = json!("competing-native-id");
        let conflict = facts(&known_headers(), &body);
        assert!(conflict.identity_conflict);
        assert!(conflict.track);
        body["client_metadata"][field] = json!(7);
        assert!(!facts(&known_headers(), &body).identity_conflict);
        let mut alternate = canonical.clone();
        alternate[field] = json!("other-canonical-id");
        let mut headers = known_headers();
        headers.insert(
            "x-codex-turn-metadata",
            alternate.to_string().parse().unwrap(),
        );
        assert!(facts(&headers, &document(canonical)).identity_conflict);
    }
    let mut body = document(json!({"request_kind":"memory"}));
    body["client_metadata"]["session_id"] = json!("independent-session");
    body["client_metadata"]["thread_id"] = json!("independent-thread");
    assert!(!facts(&known_headers(), &body).identity_conflict);
}
