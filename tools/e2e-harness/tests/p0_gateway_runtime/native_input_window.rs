//! A valid input window keeps its original position through real listener admission.
use super::native_history::{accepted, classifier_body, headers, reply};
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn send_window(
    fixture: &RuntimeFixture,
    agent: &str,
    turn: &str,
    input: Value,
    selected: &[usize],
) {
    let hashes = input
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| {
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&[m["content"].as_str().unwrap()]).unwrap())
            )
        })
        .collect::<Vec<_>>();
    let mut h = headers(agent, "same-session", "main", turn, "after");
    h.iter_mut().find(|(k,_)| k == "x-hiroute-routing-context").unwrap().1 = json!({
        "version":2,"agent":agent,"session_id":"same-session","purpose":"main","turn_id":turn,
        "input":{"kind":"native_messages","event_id":turn,"coverage":"window",
            "messages":selected.iter().map(|index| json!({"index":index,"sha256":hashes[*index]})).collect::<Vec<_>>(),
            "scope_digest":format!("{:x}",Sha256::digest(serde_json::to_vec(&hashes).unwrap()))}
    }).to_string();
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
fn real_listener_current_window_evaluates_a_after_shortening_and_trailing_context_on_b() {
    for agent in ["dsh", "pi", "opencode"] {
        let regular = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier = NativeProvider::start(vec![
            reply(CLASSIFIER_SIMPLE),
            reply(CLASSIFIER_SIMPLE),
            reply(br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#),
        ]);
        let fixture = RuntimeFixture::launch_rest_classified_with_observation(
            &[&regular, &primary, &classifier],
            1,
            ObservationFaults::healthy(),
        );
        send_window(
            &fixture,
            agent,
            "old",
            json!([
                {"role":"user","content":"old task"}, {"role":"assistant","content":"older output"},
                {"role":"user","content":"pre-compaction task"}, {"role":"user","content":"old runtime context"}
            ]),
            &[1],
        );
        let a = json!([
            {"role":"user","content":"compacted summary"}, {"role":"assistant","content":"old retained output"},
            {"role":"user","content":"ACTUAL_A"}
        ]);
        send_window(&fixture, agent, "a", a.clone(), &[1]);
        let mut b = a.as_array().unwrap().clone();
        b.extend([
            json!({"role":"assistant","content":"ok"}),
            json!({"role":"user","content":"ACTUAL_B"}),
            json!({"role":"user","content":"B additional input"}),
            json!({"role":"user","content":"TRAILING_RUNTIME_CONTEXT"}),
        ]);
        send_window(&fixture, agent, "b", json!(b), &[2, 3]);
        assert_eq!(
            (regular.calls(), primary.calls(), classifier.calls()),
            (2, 1, 3)
        );
        let decision = classifier_body(&classifier, 2);
        assert!(decision["latest_user"].to_string().contains("ACTUAL_B"));
        assert!(
            !decision["latest_user"]
                .to_string()
                .contains("TRAILING_RUNTIME_CONTEXT")
        );
        let from = decision["preceding_stage"]["from"]
            .as_u64()
            .expect("B must evaluate completed A") as usize;
        let stage =
            serde_json::to_string(&decision["visible_conversation"].as_array().unwrap()[from..])
                .unwrap();
        assert!(stage.contains("ACTUAL_A") && stage.contains("ok"));
        assert!(
            !stage.contains("compacted summary")
                && !stage.contains("pre-compaction task")
                && !stage.contains("ACTUAL_B")
        );
        let facts = wait_execution_facts(&fixture, 3);
        let a_fact = facts
            .iter()
            .map(|f| &f["fact"])
            .find(|f| {
                f["kind"] == "agent_turn_finished" && f["opening_cause"] == "history_shortened"
            })
            .unwrap();
        let score = facts
            .iter()
            .map(|f| &f["fact"])
            .find(|f| f["kind"] == "branch_assessment_recorded")
            .expect("actual score must be retained");
        assert_eq!(score["score"], 0.75);
        assert_eq!(score["partial"], false);
        assert_eq!(score["segment_id"], a_fact["segment_id"]);
        assert_eq!(
            score["history_membership"]["request_ids"],
            a_fact["history_membership"]["request_ids"]
        );
        let upstream = classifier_body(&primary, 0);
        assert!(
            upstream["input"]
                .to_string()
                .contains("TRAILING_RUNTIME_CONTEXT")
        );
    }
}
