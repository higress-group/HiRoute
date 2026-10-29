//! A typed synthetic continuation is an execution, but cannot supply a new task.
use super::native_history::{accepted, classifier_body, headers, reply};
use super::*;
use serde_json::{Value, json};

fn send(fixture: &RuntimeFixture, agent: &str, turn: &str, input: Value, guarded: bool) {
    let mut h = headers(agent, "session", "main", turn, "after");
    let context = json!({
        "version":2,"agent":agent,"session_id":"session","purpose":"main","turn_id":turn,
        "checkpoint":"summary",
        "continuation":{"kind":"compression","from_turn_id":"old","to_checkpoint":"summary"},
        "input":{"kind":if guarded { "existing_turn" } else { "observed_batch" },"event_id":turn}
    });
    h.iter_mut()
        .find(|(k, _)| k == "x-hiroute-routing-context")
        .unwrap()
        .1 = context.to_string();
    accepted(&request(
        fixture.address,
        "POST",
        "/v1/responses",
        &h.iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        &serde_json::to_vec(&json!({"model":MODEL,"input":input,"stream":false})).unwrap(),
    ));
}

#[test]
fn real_listener_v2_existing_turn_captures_output_then_recovers_a_b_without_summary_input() {
    for agent in ["pi", "dsh", "opencode"] {
        for warm in [false, true] {
            let regular = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
            let primary = NativeProvider::start(vec![reply(FALLBACK_OK), reply(RESPONSES_OK)]);
            let mut replies = vec![reply(CLASSIFIER_SIMPLE)];
            if warm {
                replies.push(reply(CLASSIFIER_SIMPLE));
            }
            replies.push(reply(
                br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#,
            ));
            let classifier = NativeProvider::start(replies);
            let fixture = RuntimeFixture::launch_rest_classified_with_observation(
                &[&regular, &primary, &classifier],
                1,
                ObservationFaults::healthy(),
            );
            if warm {
                send(
                    &fixture,
                    agent,
                    "old",
                    json!([{"role":"user","content":"PRE_COMPACTION_TASK"}]),
                    false,
                );
            }
            let summary = json!({"role":"user","content":"SYNTHETIC_CONTINUE_AFTER_COMPACTION"});
            // A new native message ID and the named old turn do not prove ancestry.
            send(&fixture, agent, "synthetic", json!([summary]), true);
            assert_eq!(
                classifier.calls(),
                usize::from(warm),
                "{agent}: synthetic input was classified"
            );
            assert_eq!(
                primary.calls(),
                1,
                "{agent}: unproved source inherited a decision"
            );
            assert!(
                classifier_body(&primary, 0)["input"]
                    .to_string()
                    .contains("SYNTHETIC_CONTINUE_AFTER_COMPACTION")
            );
            let a = json!([summary, {"role":"assistant","content":"fallback"}, {"role":"user","content":"ACTUAL_A"}]);
            // Compression cursors remain on normal traffic, as with Pi. They must
            // not latch the continuation guard after a new actual input arrives.
            send(&fixture, agent, "a", a.clone(), false);
            let a_decision = classifier_body(&classifier, usize::from(warm));
            assert!(a_decision["latest_user"].to_string().contains("ACTUAL_A"));
            let visible = a_decision["visible_conversation"].to_string();
            assert!(
                visible.contains("native_continuation_input") && visible.contains("fallback"),
                "{visible}"
            );
            assert!(
                !visible.contains("SYNTHETIC_CONTINUE_AFTER_COMPACTION"),
                "{visible}"
            );
            let mut b = a.as_array().unwrap().clone();
            b.extend([
                json!({"role":"assistant","content":"ok"}),
                json!({"role":"user","content":"ACTUAL_B"}),
            ]);
            send(&fixture, agent, "b", json!(b), false);
            assert_eq!(
                (regular.calls(), primary.calls(), classifier.calls()),
                (1 + usize::from(warm), 2, 2 + usize::from(warm))
            );
            let b_decision = classifier_body(&classifier, 1 + usize::from(warm));
            let from = b_decision["preceding_stage"]["from"]
                .as_u64()
                .expect("B must assess A") as usize;
            let assessed =
                json!(&b_decision["visible_conversation"].as_array().unwrap()[from..]).to_string();
            assert!(
                assessed.contains("ACTUAL_A") && assessed.contains("ok"),
                "{assessed}"
            );
            assert!(
                !assessed.contains("ACTUAL_B")
                    && !assessed.contains("SYNTHETIC_CONTINUE_AFTER_COMPACTION")
                    && !assessed.contains("PRE_COMPACTION_TASK"),
                "{assessed}"
            );
            assert!(
                b_decision
                    .to_string()
                    .contains("unverified_request_user_origin")
            );
            let facts = wait_execution_facts(&fixture, 3 + usize::from(warm));
            let routes = facts
                .iter()
                .filter(|f| f["fact"]["kind"] == "route_decision")
                .collect::<Vec<_>>();
            assert_eq!(routes.len(), 3 + usize::from(warm));
            assert_eq!(
                routes
                    .iter()
                    .filter(|f| f["fact"]["complexity"]["fallback_used"] == true)
                    .count(),
                1
            );
            assert_eq!(
                facts
                    .iter()
                    .filter(|f| f["fact"]["kind"] == "agent_turn_finished")
                    .count(),
                3 + usize::from(warm)
            );
            let score = facts
                .iter()
                .map(|f| &f["fact"])
                .find(|f| f["kind"] == "branch_assessment_recorded")
                .expect("assessment must be recorded");
            assert_eq!(score["score"], 0.75);
            assert_eq!(
                score["history_membership"]["request_ids"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}
