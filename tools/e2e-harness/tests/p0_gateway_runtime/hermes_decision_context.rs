//! Real listener regressions; native lineage headers are controlled contract inputs.
use super::super::native_history::{accepted, classifier_body, reply, send};
use super::*;

const SIMPLE: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q1":{"type":"score","score":1.8},"q2":{"type":"choice","probabilities":{"applicable":0.0,"not_established":1.0}}}}"#;
const COMPLEX: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.1,"1":0.9}},"q1":{"type":"score","score":1.8},"q2":{"type":"choice","probabilities":{"applicable":0.0,"not_established":1.0}}}}"#;

fn fixture(providers: &[&NativeProvider]) -> RuntimeFixture {
    let fixture = RuntimeFixture::launch_builtin_branches(providers, true);
    let mut publication = fixture.publication();
    for grant in &mut publication.grants {
        grant.consumer = Some(hiroute_domain::AgentModelConsumerV1::Hermes);
    }
    super::super::decision_branches::enable_c_in_publication_with(
        &mut publication,
        &hiroute_domain::NativeProtocolV1::c_v4(),
    );
    fixture.publish(&mut publication);
    fixture
}

fn headers(session: &str, turn: &str, ids: &[&str]) -> Vec<(String, String)> {
    let stamp =
        json!({"observer_id":"native-fixture","sequence":1,"observed_at_ms":1000,"age_ms":0});
    vec![
        ("X-HiRoute-Token".into(), TOKEN.into()),
        ("x-hiroute-agent".into(), "hermes".into()),
        ("x-hermes-session-id".into(), session.into()),
        (
            "x-hiroute-routing-context".into(),
            json!({"version":2,"agent":"hermes",
            "session_id":session,"purpose":"main","turn_id":turn,
            "input":{"kind":"observed_batch","event_id":turn}})
            .to_string(),
        ),
        (
            "x-hermes-compression-lineage".into(),
            json!({"version":1,"ids":ids,"complete":true,
            "proof_observation":stamp,"current_state":{"value":"open","observation":stamp}})
            .to_string(),
        ),
    ]
}

#[test]
fn hermes_cold_history_delivers_replayed_summary_to_decision_without_scoring_it() {
    let regular = NativeProvider::start(vec![reply(RESPONSES_OK)]);
    let primary = NativeProvider::start(vec![]);
    let decider = NativeProvider::start(vec![reply(SIMPLE)]);
    let gateway = fixture(&[&regular, &primary, &decider]);
    accepted(&send(
        gateway.address,
        &headers("cold", "B", &["cold"]),
        json!([
            {"role":"assistant","content":"SUMMARY: publish artifact A, then update manifest B."},
            {"role":"user","content":"Proceed with those actions."}
        ]),
    ));
    let judged = classifier_body(&decider, 0);
    assert!(
        judged["state"]["request_context"]
            .to_string()
            .contains("SUMMARY: publish artifact A")
    );
    assert_eq!(judged["state"]["prior_context"], json!([]));
    assert_eq!(judged["state"]["assessment_stage"], json!([]));
    assert!(judged["questions"].get("q1").is_none());
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (1, 0, 1)
    );
    let facts = wait_execution_facts(&gateway, 1);
    assert!(
        !facts
            .iter()
            .any(|e| e["fact"]["kind"] == "branch_assessment_recorded")
    );
}

#[test]
fn hermes_rotated_session_keeps_captured_stage_and_background_separate_from_summary() {
    let regular = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
    let primary = NativeProvider::start(vec![reply(RESPONSES_OK)]);
    let decider = NativeProvider::start(vec![reply(SIMPLE), reply(COMPLEX), reply(SIMPLE)]);
    let gateway = fixture(&[&regular, &primary, &decider]);
    let mut input = vec![json!({"role":"user","content":"CAPTURED_BACKGROUND_A"})];
    let first = send(
        gateway.address,
        &headers("parent", "A0", &["parent"]),
        json!(input),
    );
    accepted(&first);
    let first: Value = serde_json::from_slice(&first.body).unwrap();
    input.extend(first["output"].as_array().unwrap().iter().cloned());
    input.push(json!({"role":"user","content":"CAPTURED_STAGE_A"}));
    accepted(&send(
        gateway.address,
        &headers("parent", "A1", &["parent"]),
        json!(input),
    ));
    accepted(&send(
        gateway.address,
        &headers("child", "B", &["parent", "child"]),
        json!([
            {"role":"assistant","content":"REPLAY_SUMMARY claims uncaptured execution succeeded."},
            {"role":"user","content":"Continue the work described above."}
        ]),
    ));
    let judged = classifier_body(&decider, 2);
    let state = &judged["state"];
    assert!(
        state["prior_context"]
            .to_string()
            .contains("CAPTURED_BACKGROUND_A")
    );
    assert!(
        state["assessment_stage"]
            .to_string()
            .contains("CAPTURED_STAGE_A")
    );
    assert!(state["assessment_stage"].to_string().contains("ok"));
    assert!(
        !state["assessment_stage"]
            .to_string()
            .contains("REPLAY_SUMMARY")
    );
    assert!(
        !state["prior_context"]
            .to_string()
            .contains("REPLAY_SUMMARY")
    );
    assert!(
        state["request_context"]
            .to_string()
            .contains("REPLAY_SUMMARY")
    );
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (2, 1, 3)
    );
    let facts = wait_execution_facts(&gateway, 3);
    let admissions = facts
        .iter()
        .filter(|e| e["fact"]["kind"] == "agent_turn_admitted")
        .collect::<Vec<_>>();
    assert_eq!(admissions.len(), 3);
    assert_ne!(
        admissions[0]["correlation"]["conversation_id"],
        admissions[2]["correlation"]["conversation_id"]
    );
    assert_ne!(
        admissions[2]["fact"]["opening_cause"],
        "no_comparable_history"
    );
}
