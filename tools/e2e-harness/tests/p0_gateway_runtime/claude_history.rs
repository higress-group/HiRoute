//! Claude native request-class isolation through the actual HTTP listener.
use super::*;

#[test]
fn real_listener_claude_known_auxiliary_survives_missing_identity_and_bad_prompt() {
    let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
    let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 6]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_SIMPLE),
        reply(CLASSIFIER_SIMPLE_WITH_ASSESSMENT),
    ]);
    let fixture = native_fixture("claude", &[&simple, &primary, &classifier]);
    accepted(&send(
        fixture.address,
        &headers("claude", "main", "main", "prompt-a", ""),
        json!([{"role":"user","content":"CLAUDE_ACTUAL_A"}]),
    ));
    for purpose in ["auxiliary", "compaction"] {
        for identity in [None, Some(""), Some("main")] {
            let mut auxiliary = vec![
                ("X-HiRoute-Token".into(), TOKEN.into()),
                ("x-claude-code-request-class".into(), purpose.into()),
                ("x-claude-code-prompt-id".into(), "".into()),
                ("x-claude-code-context-compacted".into(), "invalid".into()),
            ];
            if let Some(identity) = identity {
                auxiliary.push(("x-claude-code-session-id".into(), identity.into()));
            }
            accepted(&send(
                fixture.address,
                &auxiliary,
                json!([{"role":"user","content":"CLAUDE_AUXILIARY_EXCLUDED"}]),
            ));
        }
    }
    assert_eq!(
        classifier.calls(),
        1,
        "known auxiliary classes must detach before optional identity/prompt parsing"
    );
    accepted(&send(
        fixture.address,
        &headers("claude", "main", "main", "prompt-b", ""),
        json!([
            {"role":"user","content":"CLAUDE_ACTUAL_A"},
            {"role":"assistant","content":"ok"},
            {"role":"user","content":[
                {"type":"input_text","text":"CLAUDE_ACTUAL_B"},
                {"type":"input_text","text":"CLAUDE_SUPPLEMENTAL_TEXT"}
            ]}
        ]),
    ));
    assert_eq!(
        (simple.calls(), primary.calls(), classifier.calls()),
        (2, 6, 2)
    );
    let body = classifier_body(&classifier, 1);
    let current = body["latest_user"].to_string();
    assert!(current.contains("CLAUDE_ACTUAL_B") && current.contains("CLAUDE_SUPPLEMENTAL_TEXT"));
    assert!(current.contains("unverified_request_user_origin"));
    assert!(
        body["preceding_stage"].is_object(),
        "actual A must remain evaluable"
    );
    assert!(
        body["visible_conversation"]
            .to_string()
            .contains("CLAUDE_ACTUAL_A")
    );
    assert!(!body.to_string().contains("CLAUDE_AUXILIARY_EXCLUDED"));
}
