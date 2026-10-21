//! Continuous routing is independent of proving human/child input origin.
use super::native_history::{accepted, classifier_body, headers, native_fixture, reply, send};
use super::*;
use serde_json::json;

#[test]
fn real_listener_uncertain_native_a_b_classifies_captures_and_evaluates_actual_output() {
    for agent in [
        "codex", "claude", "hermes", "pi", "dsh", "opencode", "qoder",
    ] {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(
            br#"{"decision":{"kind":"ordinal","probabilities":{"simple":0.1,"complex":0.9}},"assessment":{"score":0.75,"partial":false}}"#
        )]);
        let fixture = native_fixture(agent, &[&simple, &primary, &classifier]);
        let make_headers = |turn| {
            if agent == "qoder" {
                vec![
                    ("X-HiRoute-Token".into(), TOKEN.into()),
                    ("session-id".into(), "shared-qoder".into()),
                    ("User-Agent".into(), "qodercli/1.1.72".into()),
                ]
            } else {
                let mut h = headers(agent, "same-session", "main", turn, "after");
                if matches!(agent, "pi" | "dsh" | "opencode") {
                    h.retain(|(k, _)| k != "x-hiroute-routing-context");
                }
                h
            }
        };
        let old = json!({"role":"user","content":"UNCAPTURED_OLD_SUMMARY"});
        let a = json!({"role":"user","content":"REAL_NEW_A"});
        accepted(&send(
            fixture.address,
            &make_headers("turn-a"),
            json!([old, a]),
        ));
        accepted(&send(
            fixture.address,
            &make_headers("turn-b"),
            json!([old, a,
                {"role":"assistant","content":"ok"},{"role":"user","content":"REAL_NEW_B"}
            ]),
        ));
        assert_eq!(
            (simple.calls(), primary.calls(), classifier.calls()),
            (1, 1, 2),
            "{agent}"
        );
        let b = classifier_body(&classifier, 1);
        let visible = b["visible_conversation"].to_string();
        assert!(
            visible.contains("REAL_NEW_A") && visible.contains("ok"),
            "{agent}: {b}"
        );
        assert!(
            visible.contains("unverified_request_user_origin"),
            "{agent}: {b}"
        );
        assert!(
            !visible.contains("UNCAPTURED_OLD_SUMMARY") && !visible.contains("REAL_NEW_B"),
            "{agent}: {b}"
        );
        assert!(
            !b["assessment_target"].is_null(),
            "{agent}: evaluation was disabled: {b}"
        );
        assert_eq!(
            b["history_partial"], true,
            "old uncaptured context stays partial"
        );
    }
}

#[test]
fn real_listener_managed_qoder_reserved_entry_recovers_routing_without_inventing_origin() {
    let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_SIMPLE),
        reply(CLASSIFIER_SIMPLE_WITH_ASSESSMENT),
    ]);
    let fixture = native_fixture("unknown", &[&simple, &primary, &classifier]);
    let path = format!("{}/responses", hiroute_domain::QODER_MODEL_BASE_PATH);
    let bearer = format!("Bearer {TOKEN}");
    for input in [
        json!("QODER_A"),
        json!([
            {"role":"user","content":"QODER_A"}, {"role":"assistant","content":"ok"},
            {"role":"user","content":"QODER_B"}
        ]),
    ] {
        accepted(&request(
            fixture.address,
            "POST",
            &path,
            &[
                ("Authorization", bearer.as_str()),
                ("User-Agent", "Bun/1.4.2"),
                ("session-id", "qoder"),
            ],
            &serde_json::to_vec(&json!({"model":MODEL,"input":input,"stream":false})).unwrap(),
        ));
    }
    assert_eq!(
        (simple.calls(), primary.calls(), classifier.calls()),
        (2, 0, 2)
    );
    let b = classifier_body(&classifier, 1);
    assert!(!b["assessment_target"].is_null());
    assert!(
        b["visible_conversation"]
            .to_string()
            .contains("unverified_request_user_origin")
    );
}
