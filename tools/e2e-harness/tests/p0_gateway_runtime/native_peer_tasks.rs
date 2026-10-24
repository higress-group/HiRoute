//! Typed peer triggers keep routing input separate from original execution wire.
use super::native_history::{accepted, classifier_body, headers, send};
use super::*;
use serde_json::{Value, json};

fn reply() -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }
}

#[test]
fn real_listener_native_peer_tasks_retain_observed_routing_and_original_wire() {
    for external in [false, true] {
        for opaque in [false, true] {
            let content = if opaque {
                json!([
                    {"type":"input_text","text":"readable wrapper is not the encrypted task"},
                    {"type":"encrypted_content","encrypted_content":"opaque native child task"}
                ])
            } else {
                json!([{"type":"input_text","text":"complex-route current peer task"}])
            };
            let economy = NativeProvider::start(vec![reply()]);
            let primary = NativeProvider::start(vec![reply()]);
            let classifier = NativeProvider::start(vec![ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: CLASSIFIER_SIMPLE,
            }]);
            let fixture = if external {
                RuntimeFixture::launch_rest_classified_with_observation(
                    &[&economy, &primary, &classifier],
                    1,
                    ObservationFaults::healthy(),
                )
            } else {
                RuntimeFixture::launch_classified_with_observation(
                    &[&economy, &primary],
                    1,
                    ObservationFaults::healthy(),
                )
            };
            let input = json!([
                {"role":"user","content":"hello"},
                {"type":"agent_message","author":"parent","recipient":"child","content":content}
            ]);
            let result = request(
                fixture.address,
                "POST",
                "/v1/responses",
                &[("X-HiRoute-Token", TOKEN)],
                &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"input":input})).unwrap(),
            );
            assert_eq!(
                result.status,
                200,
                "{}",
                String::from_utf8_lossy(&result.body)
            );
            let regular = external && !opaque;
            assert_eq!(economy.calls(), usize::from(regular));
            assert_eq!(classifier.calls(), usize::from(regular));
            assert_eq!(primary.calls(), usize::from(!regular));
            let facts = wait_execution_facts(&fixture, 1);
            let decision = &facts
                .iter()
                .find(|row| row["fact"]["kind"] == "route_decision")
                .unwrap()["fact"]["complexity"];
            assert_eq!(decision["fallback_used"], opaque);
            if opaque {
                if external {
                    assert_eq!(decision["fallback_reason"], "unavailable");
                } else {
                    assert_eq!(decision["decision_source"], "unresolved");
                    assert!(
                        decision["reason_codes"]
                            .as_array()
                            .unwrap()
                            .contains(&json!("TASK_CONTEXT_UNRESOLVED"))
                    );
                }
            } else if external {
                let classified = classifier_body(&classifier, 0);
                assert_eq!(
                    classified["latest_user"][0]["text"],
                    "complex-route current peer task"
                );
                assert!(!classified["latest_user"].to_string().contains("hello"));
            } else {
                assert_eq!(decision["decision_source"], "user_phrase");
            }
            let requests = if regular {
                economy.requests()
            } else {
                primary.requests()
            };
            let raw = &requests[0];
            let offset = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let executed: Value = serde_json::from_slice(&raw[offset..]).unwrap();
            assert_eq!(executed["input"], input);
        }
    }
}

#[test]
fn real_listener_opaque_peer_is_not_ancestor_input_and_same_session_a_b_recovers() {
    let economy = NativeProvider::start(vec![reply()]);
    let primary = NativeProvider::start(vec![reply(), reply()]);
    let classifier = NativeProvider::start(vec![
        super::native_history::reply(CLASSIFIER_SIMPLE_WITH_ASSESSMENT),
        super::native_history::reply(
            br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#,
        ),
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&economy, &primary, &classifier],
        1,
    );
    // The generic fixture intentionally reuses one capability configuration ID.
    // Publish two distinct actual configurations for this attribution assertion.
    let mut publication = fixture.publication();
    for candidate in &mut publication.aliases[0].candidates {
        for profile in &mut candidate.protocol_profiles {
            profile.capability.model_configuration_id =
                format!("codex-peer-config-{}", candidate.local_id);
        }
        candidate.protocol_profile_digest =
            hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    }
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    let mut input = json!([
        {"role":"user","content":"UNCAPTURED_ANCESTOR_TASK"},
        {"type":"agent_message","author":"child","recipient":"parent","content":[
            {"type":"input_text","text":"readable wrapper"},
            {"type":"encrypted_content","encrypted_content":"opaque task"}
        ]}
    ]);
    accepted(&send(
        fixture.address,
        &headers("codex", "same-peer-session", "main", "opaque", "before"),
        input.clone(),
    ));
    assert_eq!(
        classifier.calls(),
        0,
        "encrypted intent must not be replaced by its ancestor"
    );
    for (turn, user) in [("fresh-a", "REAL_NEW_A"), ("fresh-b", "REAL_NEW_B")] {
        input.as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":"ok"}),
            json!({"role":"user","content":user}),
        ]);
        accepted(&send(
            fixture.address,
            &headers("codex", "same-peer-session", "main", turn, "before"),
            input.clone(),
        ));
    }
    assert_eq!(
        (economy.calls(), primary.calls(), classifier.calls()),
        (1, 2, 2)
    );
    let a = classifier_body(&classifier, 0);
    assert_eq!(a["latest_user"][0]["text"], "REAL_NEW_A");
    let opaque_capture = &a["visible_conversation"];
    assert!(opaque_capture.to_string().contains("native_event_input"));
    assert!(
        !opaque_capture
            .to_string()
            .contains("UNCAPTURED_ANCESTOR_TASK")
    );
    assert!(!opaque_capture.to_string().contains("readable wrapper"));
    let b = classifier_body(&classifier, 1);
    assert_eq!(b["latest_user"][0]["text"], "REAL_NEW_B");
    let from = b["preceding_stage"]["from"]
        .as_u64()
        .expect("B evaluates actual captured A") as usize;
    let target =
        serde_json::to_string(&b["visible_conversation"].as_array().unwrap()[from..]).unwrap();
    assert!(target.contains("REAL_NEW_A") && target.contains("ok"));
    assert!(!target.contains("REAL_NEW_B") && !target.contains("UNCAPTURED_ANCESTOR_TASK"));
    let facts = wait_execution_facts(&fixture, 3);
    let routes = facts
        .iter()
        .filter(|row| row["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 3);
    for route in &routes[1..] {
        assert_eq!(route["fact"]["complexity"]["fallback_used"], false);
        assert_eq!(
            route["fact"]["complexity"]["decision_source"],
            "external_classifier"
        );
    }
    let b_id = &routes[2]["correlation"]["request_id"];
    let grade = facts
        .iter()
        .find(|row| {
            row["fact"]["kind"] == "branch_assessment_recorded"
                && &row["correlation"]["request_id"] == b_id
        })
        .unwrap();
    assert_eq!(grade["fact"]["score"], 0.75);
    assert_eq!(grade["fact"]["partial"], false);
    let members = grade["fact"]["history_membership"]["request_ids"]
        .as_array()
        .unwrap();
    assert_eq!(members, &[routes[1]["correlation"]["request_id"].clone()]);
    assert!(!members.contains(b_id));
    let completed = facts
        .iter()
        .filter(|row| row["fact"]["kind"] == "agent_turn_finished")
        .collect::<Vec<_>>();
    let a_model = &completed
        .iter()
        .find(|row| row["correlation"]["request_id"] == routes[1]["correlation"]["request_id"])
        .unwrap()["fact"]["model_configuration_id"];
    let b_model = &completed
        .iter()
        .find(|row| &row["correlation"]["request_id"] == b_id)
        .unwrap()["fact"]["model_configuration_id"];
    assert_ne!(
        a_model, b_model,
        "A and B execute distinct actual model configurations"
    );
    assert_eq!(&grade["fact"]["model_configuration_id"], a_model);
}
