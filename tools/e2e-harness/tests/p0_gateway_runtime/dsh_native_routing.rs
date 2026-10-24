//! Native semantic user proofs affect routing only; provider requests stay byte-semantic intact.
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn digest(text: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&vec![text]).unwrap())
    )
}

/// The legacy cross-client fixture has no native producer. Supply its declared
/// semantic users explicitly in the current DSH producer contract.
pub(super) fn fixture_user_turns(input: &Value, current_turn: &str) -> Option<Value> {
    let texts = if let Some(text) = input.as_str() {
        vec![vec![text]]
    } else {
        input
            .as_array()?
            .iter()
            .filter(|m| m["role"] == "user")
            .filter_map(|m| {
                if let Some(text) = m["content"].as_str() {
                    return Some(vec![text]);
                }
                let content = m["content"].as_array()?;
                let text = content
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>();
                (!text.is_empty()).then_some(text)
            })
            .collect()
    };
    if texts.is_empty() || texts.len() > 16 {
        return None;
    }
    Some(json!(texts.iter().enumerate().map(|(index, parts)| json!({
        "turn_id":if index+1 == texts.len() { current_turn.to_owned() } else { format!("fixture-prior-{index}") },
        "messages":[{"index":index,"sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(parts).unwrap()))}]
    })).collect::<Vec<_>>()))
}

fn header(turn: &str, proof: Option<Value>) -> Vec<(String, String)> {
    let mut context = json!({"version":1,"agent":"dsh","session_id":"native-dsh","purpose":"main","turn_id":turn});
    if let Some(proof) = proof {
        context["user_turns"] = proof;
    }
    vec![
        ("X-HiRoute-Token".into(), TOKEN.into()),
        ("x-hiroute-agent".into(), "dsh".into()),
        ("x-deepseek-harness-session-id".into(), "native-dsh".into()),
        ("x-hiroute-routing-context".into(), context.to_string()),
    ]
}
fn send(fixture: &RuntimeFixture, headers: &[(String, String)], input: Value) -> WireResponse {
    request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        &serde_json::to_vec(&json!({"model":MODEL,"input":input,"stream":false})).unwrap(),
    )
}
fn body(provider: &NativeProvider, index: usize) -> Value {
    let raw = &provider.requests()[index];
    let offset = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&raw[offset..]).unwrap()
}
fn reply(body: &'static [u8]) -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body,
    }
}
fn proof(turn: &str, index: usize, text: &str) -> Value {
    json!({"turn_id":turn,"messages":[{"index":index,"sha256":digest(text)}]})
}

#[test]
fn real_listener_dsh_semantic_user_excludes_runtime_context_and_keeps_tool_continuation() {
    let model = NativeProvider::start(vec![
        reply(RESPONSES_TOOL_CALL),
        reply(RESPONSES_OK),
        reply(RESPONSES_OK),
    ]);
    let classifier =
        NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_SIMPLE)]);
    let unused_primary = NativeProvider::start(vec![]);
    let fixture = RuntimeFixture::launch_rest_classified_with_diagnostics(
        &[&model, &unused_primary, &classifier],
        1,
    );
    let first = json!({"role":"user","content":"REAL_NATIVE_TASK"});
    let policy = json!({"role":"user","content":"NATIVE_RUNTIME_POLICY"});
    let h1 = header("1", Some(json!([proof("1", 0, "REAL_NATIVE_TASK")])));
    let response = send(&fixture, &h1, json!([first, policy]));
    assert_eq!(response.status, 200);
    let decision = body(&classifier, 0);
    assert_eq!(decision["history_partial"], true);
    assert_eq!(decision["visible_conversation"], json!([]));
    assert!(
        decision["latest_user"]
            .to_string()
            .contains("REAL_NATIVE_TASK")
    );
    assert!(!decision.to_string().contains("NATIVE_RUNTIME_POLICY"));
    let call: Value = serde_json::from_slice(&response.body).unwrap();
    let continued = json!([first,policy,
        {"type":"function_call","call_id":call["output"][0]["call_id"],"namespace":"tools","name":"lookup","arguments":"{}"},
        {"type":"function_call_output","call_id":call["output"][0]["call_id"],"output":"native tool receipt"}]);
    assert_eq!(send(&fixture, &h1, continued.clone()).status, 200);
    assert_eq!(
        classifier.calls(),
        1,
        "same native tool turn lost its decision"
    );
    let mut next = continued.as_array().unwrap().clone();
    next.push(json!({"role":"assistant","content":"ok"}));
    next.push(json!({"role":"user","content":"NEXT_NATIVE_TASK"}));
    assert_eq!(
        send(
            &fixture,
            &header(
                "2",
                Some(json!([
                    proof("1", 0, "REAL_NATIVE_TASK"),
                    proof("2", 2, "NEXT_NATIVE_TASK")
                ]))
            ),
            json!(next)
        )
        .status,
        200
    );
    let decision = body(&classifier, 1);
    // The current-input descriptor cannot prove that other User envelopes are
    // complete historical execution, or demote them to system instructions.
    assert_eq!(decision["history_partial"], true);
    assert!(
        decision["latest_user"]
            .to_string()
            .contains("NEXT_NATIVE_TASK")
    );
    assert!(
        decision["visible_conversation"]
            .to_string()
            .contains("REAL_NATIVE_TASK")
    );
    assert!(!decision.to_string().contains("NATIVE_RUNTIME_POLICY"));
    for index in 0..3 {
        let executed = body(&model, index);
        let input = executed["input"].as_array().unwrap();
        assert!(
            input.iter().any(|m| m["role"] == "user"
                && m["content"].to_string().contains("NATIVE_RUNTIME_POLICY")),
            "routing-only projection changed the real provider request: {executed}"
        );
    }
}

#[test]
fn real_listener_dsh_missing_compacted_or_bad_user_proofs_are_independent_and_complete() {
    let malformed = [
        None,
        Some(json!([proof("1", 127, "REAL_NATIVE_TASK")])),
        Some(json!([proof("1", 0, "wrong digest")])),
        Some(
            json!([{"turn_id":"1","messages":[{"index":-1,"sha256":digest("REAL_NATIVE_TASK")}]}]),
        ),
        Some(
            json!([{"turn_id":"1","messages":[{"index":0,"sha256":digest("REAL_NATIVE_TASK"),"unknown":true}]}]),
        ),
    ];
    for invalid in malformed {
        let model = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let unused_economy = NativeProvider::start(vec![]);
        let classifier = NativeProvider::start(vec![reply(CLASSIFIER_COMPLEX)]);
        let fixture = RuntimeFixture::launch_rest_classified_with_diagnostics(
            &[&unused_economy, &model, &classifier],
            1,
        );
        let response = send(
            &fixture,
            &header("1", invalid),
            json!([
                {"role":"user","content":"REAL_NATIVE_TASK"},{"role":"user","content":"NATIVE_RUNTIME_POLICY"}
            ]),
        );
        assert_eq!(response.status, 200);
        let output: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(output["status"], "completed");
        assert_eq!(
            classifier.calls(),
            1,
            "an unproved native envelope remains observed input, not human proof"
        );
        assert_eq!(model.calls(), 1);
        assert_eq!(unused_economy.calls(), 0);
        assert!(
            body(&model, 0)["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| {
                    m["role"] == "user"
                        && m["content"].to_string().contains("NATIVE_RUNTIME_POLICY")
                }),
            "conservative classification must preserve the executed native request"
        );
    }

    // A native checkpoint can retain the turn ID while removing the real user's
    // source message. The old digest cannot authorize the summary as that user.
    let model = NativeProvider::start(vec![reply(RESPONSES_OK)]);
    let unused_economy = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_COMPLEX)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_diagnostics(
        &[&unused_economy, &model, &classifier],
        1,
    );
    let response = send(
        &fixture,
        &header("1", Some(json!([proof("1", 0, "REAL_NATIVE_TASK")]))),
        json!([
            {"role":"user","content":"native accepted compacted summary"},
            {"role":"user","content":"NATIVE_RUNTIME_POLICY"}
        ]),
    );
    assert_eq!(response.status, 200);
    assert_eq!(classifier.calls(), 1);
    assert_eq!(model.calls(), 1);
}

#[test]
fn real_listener_dsh_invalid_input_proof_keeps_actual_capture_and_auxiliary_isolation() {
    for failure in ["missing", "mismatched", "auxiliary"] {
        let model = NativeProvider::start(vec![reply(RESPONSES_OK), reply(RESPONSES_OK)]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier = NativeProvider::start(if failure == "auxiliary" {
            vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_SIMPLE)]
        } else {
            vec![
                reply(CLASSIFIER_SIMPLE),
                reply(CLASSIFIER_COMPLEX),
                reply(CLASSIFIER_SIMPLE),
            ]
        });
        let fixture = RuntimeFixture::launch_rest_classified_with_diagnostics(
            &[&model, &primary, &classifier],
            1,
        );
        let first = json!({"role":"user","content":"ORIGINAL_MAIN_TASK"});
        assert_eq!(
            send(
                &fixture,
                &header("1", Some(json!([proof("1", 0, "ORIGINAL_MAIN_TASK")]))),
                json!([first])
            )
            .status,
            200
        );
        let evidence = (failure == "mismatched").then(|| json!([proof("2", 1, "wrong digest")]));
        let mut detached = header("2", evidence);
        if failure == "auxiliary" {
            let (_, value) = detached
                .iter_mut()
                .find(|(key, _)| key == "x-hiroute-routing-context")
                .unwrap();
            let mut context: Value = serde_json::from_str(value).unwrap();
            context["purpose"] = json!("session-title");
            *value = context.to_string();
        }
        assert_eq!(
            send(
                &fixture,
                &detached,
                json!([
                    first, {"role":"assistant","content":"ok"},
                    {"role":"user","content":"NATIVE_AGENT_RELAY"}
                ])
            )
            .status,
            200
        );
        assert_eq!(
            classifier.calls(),
            if failure == "auxiliary" { 1 } else { 2 },
            "known auxiliary stays detached; unknown relay uses its observed lane"
        );
        assert_eq!(primary.calls(), 1);
        let mut next = vec![first, json!({"role":"assistant","content":"ok"})];
        let user_index = if failure == "auxiliary" {
            1
        } else {
            next.push(json!({"role":"user","content":"NATIVE_AGENT_RELAY"}));
            next.push(json!({"role":"assistant","content":"UNCAPTURED_RELAY_OUTPUT"}));
            2
        };
        next.push(json!({"role":"user","content":"FRESH_TRUE_USER"}));
        assert_eq!(
            send(
                &fixture,
                &header(
                    "3",
                    Some(json!([
                        proof("1", 0, "ORIGINAL_MAIN_TASK"),
                        proof("3", user_index, "FRESH_TRUE_USER")
                    ]))
                ),
                json!(next)
            )
            .status,
            200
        );
        let decision = body(&classifier, if failure == "auxiliary" { 1 } else { 2 });
        assert_eq!(
            decision["history_partial"],
            failure != "auxiliary",
            "{failure}: missing main output must be disclosed"
        );
        assert!(
            !decision["preceding_stage"].is_null(),
            "{failure}: {decision}"
        );
        let visible = decision["visible_conversation"].to_string();
        assert!(visible.contains("ORIGINAL_MAIN_TASK"));
        assert_eq!(
            visible.contains("NATIVE_AGENT_RELAY"),
            failure != "auxiliary"
        );
        assert!(
            visible.contains("ok"),
            "actual Gateway output must remain captured"
        );
        assert!(!visible.contains("UNCAPTURED_RELAY_OUTPUT"));
        assert_eq!(model.calls(), 2);
    }
}
