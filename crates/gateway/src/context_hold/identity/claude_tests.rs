use super::*;
use serde_json::json;

fn native(fields: &[(&'static str, &str)], document: Value) -> Field<NativeIdentity> {
    let mut headers = HeaderMap::new();
    for (name, value) in fields {
        headers.append(*name, value.parse().unwrap());
    }
    identity(IngressProtocol::Messages, &headers, &document)
}

fn accepted(field: Field<NativeIdentity>) -> NativeIdentity {
    match field {
        Field::Value(value) => value,
        Field::Absent | Field::Invalid => panic!("coherent Claude identity was not accepted"),
    }
}

#[test]
fn compact_context_document_preserves_claude_session_and_conflicting_header() {
    let body = json!({"metadata":{"user_id":"{\"session_id\":\"claude-session\"}"},
        "messages":[{"role":"user","content":"large-history".repeat(4096)}]});
    let compact = crate::context_hold::context_document(&body);
    assert!(compact.get("messages").is_none());
    assert_eq!(
        accepted(native(&[], compact.clone())),
        accepted(native(&[], body.clone()))
    );
    for document in [body, compact] {
        assert!(matches!(
            native(&[("x-claude-code-session-id", "different")], document),
            Field::Invalid
        ));
    }
}

#[test]
fn metadata_accepts_only_a_bounded_json_session_and_ignores_device_dimensions() {
    assert!(matches!(
        metadata_identity(&json!({
            "metadata":{"user_id":"{\"session_id\":\"claude-session\",\"device_id\":\"ignored\"}"}
        })),
        Field::Value(value) if value == "claude-session"
    ));
    for value in [
        json!({"metadata":{"user_id":"not-json"}}),
        json!({"metadata":{"user_id":"[\"native\"]"}}),
        json!({"metadata":{"user_id":"null"}}),
        json!({"metadata":{"user_id":"true"}}),
        json!({"metadata":{"user_id":"1"}}),
        json!({"metadata":{"user_id":"\"native\""}}),
        json!({"metadata":{"user_id":1}}),
        json!({"metadata":{"user_id":"{\"session_id\":\"\"}"}}),
        json!({"metadata":{"user_id":"{\"session_id\":\"one\",\"session_id\":\"two\"}"}}),
        json!({"metadata":{"user_id":"{\"session_id\":\"one\",\"session_id\":\"one\"}"}}),
        json!({"metadata":{"user_id":"x".repeat(2049)}}),
    ] {
        assert!(matches!(metadata_identity(&value), Field::Invalid));
    }
}

#[test]
fn header_and_metadata_keep_the_released_namespace_and_require_agreement() {
    let metadata = json!({"metadata":{"user_id":"{\"session_id\":\"native\"}"}});
    for document in [json!({}), metadata.clone()] {
        assert_eq!(
            accepted(native(&[("x-claude-code-session-id", "native")], document)),
            ("claude_metadata_session", vec!["native".into()])
        );
    }
    assert_eq!(
        accepted(native(&[], metadata.clone())),
        ("claude_metadata_session", vec!["native".into()])
    );
    for (fields, document) in [
        (vec![("x-claude-code-session-id", "other")], metadata),
        (
            vec![("x-claude-code-session-id", "native")],
            json!({"metadata":{"user_id":"not-json"}}),
        ),
        (
            vec![
                ("x-claude-code-session-id", "native"),
                ("x-claude-code-session-id", "native"),
            ],
            Value::Null,
        ),
    ] {
        assert!(matches!(native(&fields, document), Field::Invalid));
    }
}

#[test]
fn actual_agent_dimension_separates_siblings_without_inventing_a_main_agent() {
    let session = ("x-claude-code-session-id", "session");
    let parent = accepted(native(&[session], Value::Null));
    let first = accepted(native(
        &[session, ("x-claude-code-agent-id", "child-one")],
        Value::Null,
    ));
    let second = accepted(native(
        &[session, ("x-claude-code-agent-id", "child-two")],
        Value::Null,
    ));
    assert_eq!(parent, ("claude_metadata_session", vec!["session".into()]));
    assert_eq!(
        first,
        (
            "claude_native_agent",
            vec!["session".into(), "child-one".into()]
        )
    );
    assert_ne!(first, second);
    assert_ne!(parent, first);
    for fields in [
        vec![("x-claude-code-agent-id", "child-one")],
        vec![session, ("x-claude-code-agent-id", "")],
        vec![
            session,
            ("x-claude-code-agent-id", "child-one"),
            ("x-claude-code-agent-id", "child-one"),
        ],
    ] {
        assert!(matches!(native(&fields, Value::Null), Field::Invalid));
    }
}

#[test]
fn routing_ids_classes_and_notification_text_do_not_create_identity() {
    assert!(matches!(
        native(
            &[
                ("x-claude-code-prompt-id", "prompt-native"),
                ("x-claude-code-request-class", "subagent"),
                ("x-claude-code-context-compacted", "auto"),
            ],
            json!({"messages":[{"role":"user","content":"<task-notification>child-one</task-notification>"}]}),
        ),
        Field::Absent
    ));
}
