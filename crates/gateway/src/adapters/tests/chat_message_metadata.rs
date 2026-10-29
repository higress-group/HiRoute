use super::*;
use crate::context_hold::visible_history_with_replay;

fn document() -> serde_json::Value {
    json!({"model":"alias","messages":[
        {"role":"user","content":"Read the value."},
        {"role":"assistant","content":"42"},
        {"role":"user","content":"Add eight."}
    ]})
}

#[test]
fn user_message_annotations_preserve_native_payload_and_canonical_history() {
    let baseline = document();
    let expected = decode_ingress_request(IngressProtocol::ChatCompletions, &baseline).unwrap();
    let expected_history = visible_history_with_replay(&expected, &[7; 32], None)
        .unwrap()
        .measure(None)
        .unwrap();
    for id in ["platform-message-1".to_owned(), "x".repeat(128)] {
        let mut body = baseline.clone();
        body["messages"][0]["message_id"] = json!(id);
        body["messages"][2]["message_id"] = json!("platform-message-2");
        let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
        assert!(!request.native_only);
        assert_eq!(request.messages, expected.messages);
        let history = visible_history_with_replay(&request, &[7; 32], None)
            .unwrap()
            .measure(None)
            .unwrap();
        assert_eq!(history.complete_digest, expected_history.complete_digest);
        let profile = exact_state_profile(
            IngressProtocol::ChatCompletions,
            IngressProtocol::ChatCompletions,
        );
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap().body["messages"],
            body["messages"]
        );
        assert_eq!(request.native_body.as_ref(), Some(&body));
        body["messages"][2]["content"] = json!("Subtract eight.");
        let changed = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
        assert_ne!(
            visible_history_with_replay(&changed, &[7; 32], None)
                .unwrap()
                .measure(None)
                .unwrap()
                .complete_digest,
            history.complete_digest
        );
    }
}

#[test]
fn opaque_message_annotations_never_enable_history_reuse() {
    for id in [
        json!(null),
        json!(1),
        json!({"state":"opaque"}),
        json!(""),
        json!("line\nbreak"),
        json!("x".repeat(129)),
    ] {
        let mut body = document();
        body["messages"][0]["message_id"] = id;
        let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
        assert!(request.native_only);
        assert!(visible_history_with_replay(&request, &[7; 32], None).is_none());
        assert_eq!(request.native_body.as_ref(), Some(&body));
    }
    for role in ["assistant", "system", "developer", "tool"] {
        let mut body = document();
        body["messages"][0] = json!({"role":role,"content":"42","message_id":"opaque-id"});
        if role == "tool" {
            body["messages"][0]["tool_call_id"] = json!("call-1");
        }
        let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
        assert!(request.native_only);
        assert!(visible_history_with_replay(&request, &[7; 32], None).is_none());
    }
    let mut body = document();
    body["messages"][0]["message_id"] = json!("platform-message-1");
    body["messages"][0]["unknown_state"] = json!("must stay opaque");
    let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
    assert!(request.native_only);
    assert!(visible_history_with_replay(&request, &[7; 32], None).is_none());
}
