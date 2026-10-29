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
    for (agent, purpose) in [
        ("codex", "main"),
        ("claude", "main"),
        ("hermes", "main"),
        ("pi", "main"),
        ("dsh", "main"),
        ("opencode", "main"),
        ("qoder", "main"),
        ("claude", "workflow"),
        ("claude", "future-native-main"),
    ] {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier = NativeProvider::start(vec![
            reply(CLASSIFIER_SIMPLE),
            reply(br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#),
        ]);
        let fixture = observed_fixture(agent, &[&simple, &primary, &classifier]);
        let make_headers = |turn| {
            if agent == "qoder" {
                vec![
                    ("X-HiRoute-Token".into(), TOKEN.into()),
                    ("session-id".into(), "shared-qoder".into()),
                    ("User-Agent".into(), "qodercli/1.1.72".into()),
                ]
            } else {
                let mut h = headers(agent, "same-session", purpose, turn, "after");
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
        let unverified = |input: &serde_json::Value| {
            input["latest_user"].as_array().unwrap().iter().any(|part| {
                part["kind"] == "unavailable"
                    && part["source_kind"] == "unverified_request_user_origin"
            })
        };
        assert!(unverified(&classifier_body(&classifier, 0)));
        assert!(unverified(&b), "{agent}/{purpose}: {b}");
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
            !b["preceding_stage"].is_null(),
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
fn real_listener_hermes_legacy_text_hint_cannot_relocate_input_or_disable_protection() {
    use sha2::{Digest, Sha256};
    let simple = NativeProvider::start(vec![reply(RESPONSES_OK)]);
    let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
    let simple_reply = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#;
    let low = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q1":{"type":"score","score":0.2}}}"#;
    let classifier = NativeProvider::start(vec![reply(simple_reply), reply(low), reply(low)]);
    let fixture = RuntimeFixture::launch_builtin_branches(&[&simple, &primary, &classifier], true);
    let mut publication = fixture.publication();
    for grant in &mut publication.grants {
        grant.consumer = Some(hiroute_domain::AgentModelConsumerV1::Hermes);
    }
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    let mut input = json!([
        {"role":"user","content":"ORIGINAL_OBSERVED_INPUT"},
        {"role":"assistant","content":"execution before Gateway restart"},
        {"role":"user","content":"LATER_TODO_REMINDER"}
    ]);
    for (turn, text, reverse) in [
        ("cold-turn", "ORIGINAL_OBSERVED_INPUT", 1),
        ("fresh-a", "FRESH_A", 0),
        ("fresh-b", "FRESH_B", 0),
    ] {
        if reverse == 0 {
            input.as_array_mut().unwrap().extend([
                json!({"role":"assistant","content":"ok"}),
                json!({"role":"user","content":text}),
            ]);
        }
        let mut h = headers("hermes", "same-session", "main", turn, "after");
        h.push((
            "x-hermes-observed-input".into(),
            json!({
                "version":1,"kind":"observed_turn_input","reverse_user_index":reverse,
                "text_sha256":format!("{:x}", Sha256::digest(text.as_bytes()))
            })
            .to_string(),
        ));
        accepted(&send(fixture.address, &h, input.clone()));
    }
    assert_eq!(
        (simple.calls(), primary.calls(), classifier.calls()),
        (1, 2, 3)
    );
    let cold = classifier_body(&classifier, 0)["state"].clone();
    assert_eq!(cold["latest_user"][0]["text"], "LATER_TODO_REMINDER");
    assert_eq!(
        cold["latest_user"][1]["source_kind"],
        "unverified_request_user_origin"
    );
    let b = classifier_body(&classifier, 2)["state"].clone();
    assert_eq!(b["latest_user"][0]["text"], "FRESH_B");
    let from = b["assessment_from"].as_u64().unwrap() as usize;
    let stage = &b["visible_conversation"].as_array().unwrap()[from..];
    let stage = serde_json::to_string(stage).unwrap();
    assert!(stage.contains("FRESH_A") && stage.contains("ok"));
    assert!(!stage.contains("ORIGINAL_OBSERVED_INPUT") && !stage.contains("LATER_TODO_REMINDER"));
    let facts = wait_execution_facts(&fixture, 3);
    assert!(
        facts
            .iter()
            .any(|f| f["fact"]["complexity"]["competence_trigger"].is_object()),
        "uncertain origin must not disable actual quality protection"
    );
    let assessments = facts
        .iter()
        .filter(|f| f["fact"]["kind"] == "branch_assessment_recorded")
        .map(|f| &f["fact"])
        .collect::<Vec<_>>();
    assert!(
        assessments
            .iter()
            .any(|f| f["target_through_ordinal"] == 1 && f["partial"] == false)
    );
    assert!(assessments.iter().any(|f| f["target_from_ordinal"] == 2
        && f["target_through_ordinal"] == 2
        && f["partial"] == false));
}

#[test]
fn real_listener_managed_qoder_reserved_entry_recovers_routing_without_inventing_origin() {
    let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_SIMPLE),
        reply(br#"{"group":"regular","assessment":{"score":0.75,"partial":false}}"#),
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
    assert!(!b["preceding_stage"].is_null());
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
