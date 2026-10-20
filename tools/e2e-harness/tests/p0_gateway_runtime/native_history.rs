//! Native routing metadata through the actual Gateway listener, classifier and providers.
use super::*;
use serde_json::{Value, json};

fn reply(body: &'static [u8]) -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body,
    }
}
fn classifier_body(classifier: &NativeProvider, index: usize) -> Value {
    let requests = classifier.requests();
    let raw = &requests[index];
    let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&raw[start..]).unwrap()
}
fn headers(
    agent: &str,
    session: &str,
    purpose: &str,
    turn: &str,
    checkpoint: &str,
) -> Vec<(String, String)> {
    let mut headers = vec![("X-HiRoute-Token".into(), TOKEN.into())];
    if agent == "codex" {
        headers.extend([
            ("thread-id".into(), session.into()), ("session-id".into(), session.into()),
            ("x-codex-turn-metadata".into(), json!({"session_id":session,"thread_id":session,"turn_id":turn,"request_kind":if purpose == "main" { "turn" } else { purpose }}).to_string()),
        ]);
    } else if agent == "claude" {
        headers.extend([
            ("x-claude-code-session-id".into(), session.into()),
            ("x-claude-code-request-class".into(), purpose.into()),
            ("x-claude-code-prompt-id".into(), turn.into()),
        ]);
    } else {
        let native = match agent {
            "pi" => "x-session-affinity",
            "dsh" => "x-deepseek-harness-session-id",
            "opencode" => "x-opencode-session-id",
            _ => unreachable!(),
        };
        headers.extend([
            (native.into(), session.into()), ("x-hiroute-agent".into(), agent.into()),
            ("x-hiroute-routing-context".into(), json!({"version":1,"agent":agent,"session_id":session,"purpose":purpose,"turn_id":turn,"checkpoint":checkpoint}).to_string()),
        ]);
        if agent == "pi" {
            headers.push(("User-Agent".into(), "pi (linux; x64)".into()));
        }
    }
    headers
}
fn send(address: std::net::SocketAddr, headers: &[(String, String)], input: Value) -> WireResponse {
    request(
        address,
        "POST",
        "/v1/responses",
        &headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        &serde_json::to_vec(&json!({"model":MODEL,"input":input,"stream":false})).unwrap(),
    )
}
fn accepted(response: &WireResponse) {
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let value: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(value["status"], "completed", "{value}");
}

#[test]
fn real_listener_native_auxiliary_and_adapter_failures_never_enter_main_history() {
    for agent in ["codex", "claude", "pi", "dsh", "opencode"] {
        for fault in [false, true] {
            let simple = NativeProvider::start(vec![
                reply(RESPONSES_TOOL_CALL),
                reply(RESPONSES_OK),
                reply(RESPONSES_OK),
            ]);
            let complex = NativeProvider::start(vec![reply(RESPONSES_OK)]);
            let classifier = NativeProvider::start(vec![
                reply(CLASSIFIER_SIMPLE),
                reply(CLASSIFIER_COMPLEX),
                reply(CLASSIFIER_SIMPLE),
            ]);
            let fixture =
                RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 1);
            let main = headers(agent, "main", "main", "turn-1", "checkpoint-1");
            let user = json!({"role":"user","content":"MAIN_USER"});
            let first = send(fixture.address, &main, json!([user]));
            accepted(&first);
            let mut auxiliary = headers(
                agent,
                "main",
                if agent == "codex" {
                    "compaction"
                } else {
                    "summary"
                },
                "turn-1",
                "checkpoint-1",
            );
            if fault {
                let key = if agent == "codex" {
                    "x-codex-turn-metadata"
                } else if agent == "claude" {
                    "x-claude-code-request-class"
                } else {
                    "x-hiroute-routing-context"
                };
                auxiliary
                    .iter_mut()
                    .find(|(name, _)| name == key)
                    .unwrap()
                    .1 = "invalid-adapter-metadata".into();
            }
            accepted(&send(
                fixture.address,
                &auxiliary,
                json!("AUXILIARY_MUST_NOT_ENTER_HISTORY"),
            ));
            let detached = classifier_body(&classifier, 1);
            assert_eq!(detached["history_partial"], true, "{agent}: {detached}");
            assert_eq!(detached["visible_conversation"], json!([]));
            assert!(detached["assessment_target"].is_null());
            let first: Value = serde_json::from_slice(&first.body).unwrap();
            let call = &first["output"][0]["call_id"];
            let continued = json!([user,
                {"type":"function_call","call_id":call,"namespace":"tools","name":"lookup","arguments":"{}"},
                {"type":"function_call_output","call_id":call,"output":"found"}]);
            accepted(&send(fixture.address, &main, continued.clone()));
            assert_eq!(
                classifier.calls(),
                2,
                "{agent}: main continuation lost its decision"
            );
            let mut next = continued.as_array().unwrap().clone();
            next.push(json!({"role":"assistant","content":"ok"}));
            next.push(json!({"role":"user","content":"NEXT_MAIN_USER"}));
            accepted(&send(
                fixture.address,
                &headers(agent, "main", "main", "turn-2", "checkpoint-1"),
                json!(next),
            ));
            let history = classifier_body(&classifier, 2);
            assert!(
                !history
                    .to_string()
                    .contains("AUXILIARY_MUST_NOT_ENTER_HISTORY"),
                "{agent}: {history}"
            );
            assert!(
                history.to_string().contains("MAIN_USER"),
                "{agent}: {history}"
            );
            assert_eq!(
                (simple.calls(), complex.calls(), classifier.calls()),
                (3, 1, 3)
            );
        }
    }
}

struct Release(ProviderBodyBarrier);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[test]
fn real_listener_native_children_and_auxiliary_can_overlap_main_without_sharing_history() {
    for agent in ["codex", "claude", "pi", "dsh", "opencode"] {
        let barrier = ProviderBodyBarrier::default();
        let simple = NativeProvider::start(vec![
            ProviderReply::BodyReadBarrier {
                barrier: barrier.clone(),
                status: 200,
                error_kind: None,
                body: RESPONSES_TOOL_CALL,
            },
            reply(RESPONSES_OK),
        ]);
        let complex = NativeProvider::start(vec![reply(RESPONSES_OK), reply(RESPONSES_OK)]);
        let classifier = NativeProvider::start(vec![
            reply(CLASSIFIER_SIMPLE),
            reply(CLASSIFIER_COMPLEX),
            reply(CLASSIFIER_COMPLEX),
        ]);
        let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 1);
        let main = headers(agent, "parent", "main", "turn-1", "checkpoint-1");
        let user = json!({"role":"user","content":"PARENT_USER"});
        let first = std::thread::scope(|scope| {
            let _release = Release(barrier.clone());
            let parent_headers = main.clone();
            let parent_user = user.clone();
            let address = fixture.address;
            let parent = scope.spawn(move || send(address, &parent_headers, json!([parent_user])));
            barrier.wait_for_arrivals(1, Duration::from_secs(5));
            let mut child = headers(
                agent,
                if agent == "claude" { "parent" } else { "child" },
                "main",
                "child-turn",
                "checkpoint-1",
            );
            if agent == "claude" {
                child.push(("x-claude-code-agent-id".into(), "child-agent".into()));
            }
            accepted(&send(address, &child, json!("CHILD_USER")));
            accepted(&send(
                address,
                &headers(agent, "parent", "compaction", "turn-1", "checkpoint-1"),
                json!("AUXILIARY_USER"),
            ));
            for index in [1, 2] {
                let history = classifier_body(&classifier, index);
                assert_eq!(
                    history["visible_conversation"],
                    json!([]),
                    "{agent}: {history}"
                );
                assert!(history["assessment_target"].is_null());
            }
            barrier.release();
            parent.join().unwrap()
        });
        accepted(&first);
        let first: Value = serde_json::from_slice(&first.body).unwrap();
        let call = &first["output"][0]["call_id"];
        accepted(&send(
            fixture.address,
            &main,
            json!([user,
            {"type":"function_call","call_id":call,"namespace":"tools","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":call,"output":"found"}]),
        ));
        assert_eq!(
            (simple.calls(), complex.calls(), classifier.calls()),
            (2, 2, 3),
            "{agent}"
        );
    }
}

#[test]
fn real_listener_native_compaction_reuses_semantic_decision_without_reusing_prefix_affinity() {
    for agent in ["codex", "claude", "pi", "dsh", "opencode"] {
        let simple = NativeProvider::start(vec![
            reply(RESPONSES_OK),
            reply(RESPONSES_OK),
            reply(RESPONSES_OK),
        ]);
        let complex = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier = NativeProvider::start(vec![
            reply(CLASSIFIER_SIMPLE),
            reply(CLASSIFIER_COMPLEX),
            reply(CLASSIFIER_SIMPLE),
        ]);
        let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 1);
        accepted(&send(
            fixture.address,
            &headers(agent, "main", "main", "original-turn", "before"),
            json!([{"role":"user","content":"original task"}]),
        ));
        accepted(&send(
            fixture.address,
            &headers(agent, "main", "compaction", "original-turn", "before"),
            json!("COMPACTION_CALCULATION"),
        ));
        let compacted = json!({"role":"user","content":"native accepted summary"});
        accepted(&send(
            fixture.address,
            &headers(agent, "main", "main", "original-turn", "after"),
            json!([compacted]),
        ));
        assert_eq!(
            classifier.calls(),
            2,
            "{agent}: proven same turn reclassified after compression"
        );
        accepted(&send(
            fixture.address,
            &headers(agent, "main", "main", "next-turn", "after"),
            json!([
            compacted,{"role":"assistant","content":"ok"},{"role":"user","content":"a new actual task"}]),
        ));
        let history = classifier_body(&classifier, 2);
        assert!(
            !history.to_string().contains("COMPACTION_CALCULATION"),
            "{agent}: {history}"
        );
        assert!(
            history.to_string().contains("original task"),
            "{agent}: {history}"
        );
        assert_eq!(
            (simple.calls(), complex.calls(), classifier.calls()),
            (3, 1, 3)
        );
    }
}

#[test]
fn real_listener_restart_and_qoder_degradation_never_recover_an_unproven_history() {
    for _restart in 0..2 {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK), reply(RESPONSES_OK)]);
        let complex = NativeProvider::start(Vec::new());
        let classifier =
            NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_SIMPLE)]);
        let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 1);
        let mut qoder = vec![
            ("X-HiRoute-Token".into(), TOKEN.into()),
            ("session-id".into(), "shared-qoder".into()),
            ("User-Agent".into(), "qodercli/1.1.72".into()),
        ];
        for text in ["real main user", "unmarked native child or summary"] {
            accepted(&send(fixture.address, &qoder, json!(text)));
        }
        for index in 0..2 {
            let history = classifier_body(&classifier, index);
            assert_eq!(history["visible_conversation"], json!([]));
            assert!(history["assessment_target"].is_null());
            assert_eq!(history["history_partial"], true);
        }
        qoder[0].1 = "invalid-grant".into();
        assert_ne!(send(fixture.address, &qoder, json!("denied")).status, 200);
        assert_eq!((simple.calls(), classifier.calls()), (2, 2));
    }
}
