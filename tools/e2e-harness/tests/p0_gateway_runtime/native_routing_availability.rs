//! Continuous routing is independent of proving human/child input origin.
use super::native_history::{accepted, classifier_body, headers, native_fixture, reply, send};
use super::*;
use serde_json::json;

fn observed_fixture(agent: &str, providers: &[&NativeProvider]) -> RuntimeFixture {
    if agent == "hermes" {
        native_fixture(agent, providers)
    } else {
        RuntimeFixture::launch_rest_classified_with_observation(
            providers,
            1,
            ObservationFaults::healthy(),
        )
    }
}

fn assert_actual_evaluation(fixture: &RuntimeFixture) {
    let facts = wait_execution_facts(fixture, 2);
    let assessment = facts
        .iter()
        .find(|f| f["fact"]["kind"] == "branch_assessment_recorded")
        .expect("B must record an actual evaluation of A");
    assert_eq!(assessment["fact"]["score"], 0.75);
    assert_eq!(assessment["fact"]["partial"], false);
    assert_eq!(assessment["fact"]["target_from_ordinal"], 1);
    assert_eq!(assessment["fact"]["target_through_ordinal"], 1);
    let completed = facts
        .iter()
        .find(|f| f["fact"]["kind"] == "agent_turn_finished")
        .expect("A must have a captured completed execution");
    assert_eq!(
        assessment["fact"]["model_configuration_id"],
        completed["fact"]["model_configuration_id"]
    );
}

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
        let fixture = observed_fixture(agent, &[&simple, &primary, &classifier]);
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
        assert_actual_evaluation(&fixture);
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
    let fixture = observed_fixture("unknown", &[&simple, &primary, &classifier]);
    let path = format!("{}/responses", hiroute_domain::QODER_MODEL_BASE_PATH);
    let bearer = format!("Bearer {TOKEN}");
    for input in [
        json!([
            {"role":"user","content":"old uncaptured summary"},
            {"role":"user","content":"QODER_A"}
        ]),
        json!([
            {"role":"user","content":"old uncaptured summary"},
            {"role":"user","content":"QODER_A\n"}, {"role":"assistant","content":"ok"},
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
    assert_eq!(b["history_partial"], true);
    assert!(
        !b["visible_conversation"]
            .to_string()
            .contains("old uncaptured summary")
    );
    assert!(
        b["visible_conversation"]
            .to_string()
            .contains("unverified_request_user_origin")
    );
    assert_actual_evaluation(&fixture);
}
