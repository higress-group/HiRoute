//! Native routing metadata through the actual Gateway listener, classifier and providers.
use super::*;
use serde_json::{Value, json};

#[path = "claude_history.rs"]
mod claude;

pub(super) fn reply(body: &'static [u8]) -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body,
    }
}
pub(super) fn classifier_body(classifier: &NativeProvider, index: usize) -> Value {
    let requests = classifier.requests();
    let raw = &requests[index];
    let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&raw[start..]).unwrap()
}
pub(super) fn headers(
    agent: &str,
    session: &str,
    purpose: &str,
    turn: &str,
    checkpoint: &str,
) -> Vec<(String, String)> {
    let mut headers = vec![("X-HiRoute-Token".into(), TOKEN.into())];
    if agent == "codex" {
        headers.extend([
            ("user-agent".into(), "codex_exec/0.147.0 (test-os; test-arch)".into()),
            ("thread-id".into(), session.into()), ("session-id".into(), session.into()),
            ("x-codex-turn-metadata".into(), json!({"session_id":session,"thread_id":session,"turn_id":turn,"window_id":format!("{session}:{}", if checkpoint == "after" { 1 } else { 0 }),"request_kind":if purpose == "main" { "turn" } else { purpose }}).to_string()),
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
            "hermes" => "x-hermes-session-id",
            _ => unreachable!(),
        };
        headers.extend([
            (native.into(), session.into()), ("x-hiroute-agent".into(), agent.into()),
            ("x-hiroute-routing-context".into(), json!({"version":1,"agent":agent,"session_id":session,"purpose":purpose,"turn_id":turn,"checkpoint":checkpoint,"continuation_requires_existing_turn":false}).to_string()),
        ]);
        if agent == "pi" {
            headers.push(("User-Agent".into(), "pi (linux; x64)".into()));
            headers.push(("x-client-request-id".into(), session.into()));
        }
    }
    headers
}
pub(super) fn native_fixture(agent: &str, providers: &[&NativeProvider]) -> RuntimeFixture {
    let fixture = if agent == "hermes" {
        RuntimeFixture::launch_rest_classified_with_native_control(providers, 1)
    } else {
        RuntimeFixture::launch_rest_classified(providers, 1)
    };
    if agent == "hermes" {
        let mut publication = fixture.publication();
        publication.grants.iter_mut().for_each(|grant| {
            grant.consumer = Some(hiroute_domain::AgentModelConsumerV1::Hermes);
        });
        publication.publication_revision += 1;
        fixture.publish(&mut publication);
    }
    fixture
}
pub(super) fn send(
    address: std::net::SocketAddr,
    headers: &[(String, String)],
    input: Value,
) -> WireResponse {
    let mut headers = headers.to_vec();
    if let Some((_, encoded)) = headers
        .iter_mut()
        .find(|(name, _)| name == "x-hiroute-routing-context")
        && let Ok(mut context) = serde_json::from_str::<Value>(encoded)
        && context["agent"] == "dsh"
        && context["purpose"] == "main"
        && let Some(turn) = context["turn_id"].as_str()
        && let Some(proof) = super::dsh_native_routing::fixture_user_turns(&input, turn)
    {
        context["user_turns"] = proof;
        *encoded = context.to_string();
    }
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
pub(super) fn accepted(response: &WireResponse) {
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
    for agent in ["codex", "pi", "dsh", "opencode"] {
        for fault in [false, true] {
            let simple = NativeProvider::start(vec![
                reply(RESPONSES_TOOL_CALL),
                reply(RESPONSES_OK),
                reply(RESPONSES_OK),
                reply(RESPONSES_OK),
            ]);
            let complex = NativeProvider::start(vec![reply(RESPONSES_OK)]);
            let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 3]);
            let fixture = native_fixture(agent, &[&simple, &complex, &classifier]);
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
            let calls_before_next = if fault { 2 } else { 1 };
            assert_eq!(
                classifier.calls(),
                calls_before_next,
                "{agent}: known auxiliaries detach; broken evidence uses the observed lane"
            );
            let first: Value = serde_json::from_slice(&first.body).unwrap();
            let call = &first["output"][0]["call_id"];
            let continued = json!([user,
                {"type":"function_call","call_id":call,"namespace":"tools","name":"lookup","arguments":"{}"},
                {"type":"function_call_output","call_id":call,"output":"found"}]);
            accepted(&send(fixture.address, &main, continued.clone()));
            assert_eq!(
                classifier.calls(),
                calls_before_next,
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
            let history = classifier_body(&classifier, calls_before_next);
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
                (
                    if fault { 4 } else { 3 },
                    if fault { 0 } else { 1 },
                    calls_before_next + 1
                )
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
    for agent in ["codex", "pi", "dsh", "opencode"] {
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
        let fixture = native_fixture(agent, &[&simple, &complex, &classifier]);
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
                if agent == "claude" {
                    "subagent"
                } else {
                    "main"
                },
                "child-turn",
                "checkpoint-1",
            );
            if agent == "claude" {
                child.push(("x-claude-code-agent-id".into(), "child-agent".into()));
            }
            if agent == "codex" {
                child
                    .iter_mut()
                    .find(|(key, _)| key == "session-id")
                    .unwrap()
                    .1 = "parent".into();
                child.iter_mut().find(|(key, _)| key == "x-codex-turn-metadata").unwrap().1 = json!({
                    "session_id":"parent","thread_id":"child","turn_id":"child-turn","window_id":"child:0","request_kind":"turn"
                }).to_string();
            }
            accepted(&send(address, &child, json!("CHILD_USER")));
            accepted(&send(
                address,
                &headers(agent, "parent", "compaction", "turn-1", "checkpoint-1"),
                json!("AUXILIARY_USER"),
            ));
            let classified_auxiliary = 2;
            assert_eq!(classifier.calls(), classified_auxiliary);
            for index in 1..classified_auxiliary {
                let history = classifier_body(&classifier, index);
                assert_eq!(
                    history["visible_conversation"],
                    json!([]),
                    "{agent}: {history}"
                );
                assert!(history["preceding_stage"].is_null());
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
            (2, 2, 2),
            "{agent}"
        );
    }
}

#[test]
fn real_listener_native_cursor_alone_cannot_inherit_a_pre_compaction_decision() {
    // These legacy contexts explicitly lack a known-continuation descriptor.
    // A cursor alone neither proves an exact boundary nor a human input source.
    for agent in ["codex", "pi", "opencode"] {
        let simple = NativeProvider::start(vec![
            reply(RESPONSES_OK),
            reply(RESPONSES_OK),
            reply(RESPONSES_OK),
        ]);
        let complex = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 3]);
        let fixture = native_fixture(agent, &[&simple, &complex, &classifier]);
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
            "{agent}: a native cursor cannot replace the missing captured boundary"
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
fn real_listener_restart_and_qoder_observed_routing_recovers_without_origin_claims() {
    for _restart in 0..2 {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
        let primary = NativeProvider::start(vec![]);
        let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 2]);
        let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &primary, &classifier], 1);
        let mut qoder = vec![
            ("X-HiRoute-Token".into(), TOKEN.into()),
            ("session-id".into(), "shared-qoder".into()),
            ("User-Agent".into(), "qodercli/1.1.72".into()),
        ];
        for text in ["real main user", "unmarked native child or summary"] {
            accepted(&send(fixture.address, &qoder, json!(text)));
        }
        qoder[0].1 = "invalid-grant".into();
        assert_ne!(send(fixture.address, &qoder, json!("denied")).status, 200);
        assert_eq!(
            (simple.calls(), primary.calls(), classifier.calls()),
            (2, 0, 2)
        );
    }
}

fn hermes_lineage(session: &str, turn: &str, ids: &[&str]) -> Vec<(String, String)> {
    let mut result = headers("hermes", session, "main", turn, "native-checkpoint");
    let stamp =
        json!({"observer_id":"native-profile","sequence":1,"observed_at_ms":1000,"age_ms":0});
    result.push((
        "x-hermes-compression-lineage".into(),
        json!({
            "version":1,"ids":ids,"complete":true,"proof_observation":stamp,
            "current_state":{"value":"open","observation":stamp}
        })
        .to_string(),
    ));
    result
}

#[test]
fn real_listener_hermes_compression_and_fresh_gateway_retain_observed_routing_without_user_origin()
{
    for _restart in 0..2 {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 4]);
        let complex = NativeProvider::start(vec![]);
        let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 4]);
        let fixture = native_fixture("hermes", &[&simple, &complex, &classifier]);
        accepted(&send(
            fixture.address,
            &hermes_lineage("root", "turn-1", &["root"]),
            json!("ORIGINAL_MAIN_TASK"),
        ));
        // Native auxiliary clients still authenticate without a conversation hook.
        accepted(&send(
            fixture.address,
            &[("X-HiRoute-Token".into(), TOKEN.into())],
            json!("AUXILIARY_CALCULATION"),
        ));
        let summary = json!({"role":"user","content":"accepted compacted transcript"});
        accepted(&send(
            fixture.address,
            &hermes_lineage("compressed", "turn-1", &["root", "compressed"]),
            json!([summary]),
        ));
        accepted(&send(
            fixture.address,
            &hermes_lineage("compressed", "turn-2", &["root", "compressed"]),
            json!([summary, {"role":"assistant","content":"ok"}, {"role":"user","content":"NEW_MAIN_TASK"}]),
        ));
        assert_eq!(
            (simple.calls(), complex.calls(), classifier.calls()),
            (4, 0, 4)
        );
        // A native auxiliary client without purpose/session evidence remains
        // an independently classified request. Do not manufacture an auxiliary
        // label from missing middleware, or lend it the main conversation.
        assert!(
            classifier_body(&classifier, 1)
                .to_string()
                .contains("AUXILIARY_CALCULATION")
        );
        for index in [0, 2, 3] {
            assert!(
                !classifier_body(&classifier, index)
                    .to_string()
                    .contains("AUXILIARY_CALCULATION"),
                "unidentified auxiliary execution must not enter main history"
            );
        }
        let facts = wait_execution_facts(&fixture, 4);
        let decisions = facts
            .iter()
            .filter(|fact| fact["fact"]["kind"] == "route_decision")
            .collect::<Vec<_>>();
        assert_eq!(decisions.len(), 4);
        assert!(
            decisions
                .iter()
                .all(|fact| fact["fact"]["complexity"]["fallback_used"] == false)
        );
    }
}

#[test]
fn real_listener_auxiliary_provider_failure_cannot_change_main_tool_continuation() {
    let simple = NativeProvider::start(vec![reply(RESPONSES_TOOL_CALL), reply(RESPONSES_OK)]);
    let complex = NativeProvider::start(vec![ProviderReply::Complete {
        status: 400,
        error_kind: None,
        body: PERMANENT_ERROR,
    }]);
    let classifier =
        NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_COMPLEX)]);
    let fixture = native_fixture("opencode", &[&simple, &complex, &classifier]);
    let main = headers("opencode", "parent", "main", "turn", "before");
    let user = json!({"role":"user","content":"main task"});
    let response = send(fixture.address, &main, json!([user]));
    accepted(&response);
    assert_ne!(
        send(
            fixture.address,
            &headers("opencode", "parent", "compaction", "turn", "before"),
            json!("FAILED_AUXILIARY")
        )
        .status,
        200
    );
    let response: Value = serde_json::from_slice(&response.body).unwrap();
    let call = &response["output"][0]["call_id"];
    accepted(&send(
        fixture.address,
        &main,
        json!([user,
            {"type":"function_call","call_id":call,"namespace":"tools","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":call,"output":"found"}
        ]),
    ));
    assert_eq!(classifier.calls(), 1);
}

#[test]
fn real_listener_native_new_turn_does_not_reuse_a_decision_for_identical_content() {
    for agent in ["codex", "pi", "dsh", "opencode"] {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let complex = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier =
            NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_COMPLEX)]);
        let fixture = native_fixture(agent, &[&simple, &complex, &classifier]);
        for turn in ["native-turn-1", "native-turn-2"] {
            accepted(&send(
                fixture.address,
                &headers(agent, "session", "main", turn, "checkpoint"),
                if turn == "native-turn-1" {
                    json!("identical real user task")
                } else {
                    json!([{"role":"user","content":"identical real user task"},{"role":"assistant","content":"ok"},{"role":"user","content":"identical real user task"}])
                },
            ));
        }
        assert_eq!(
            (simple.calls(), complex.calls(), classifier.calls()),
            (1, 1, 2),
            "{agent}"
        );
    }
}

#[test]
fn real_listener_known_native_clients_without_loaded_adapters_retain_observed_routing() {
    for user_agent in [
        "pi (linux; x64)",
        "deepseek-harness/0.2.0-rc.2",
        "opencode/1.19.3",
        "codex_cli_rs/0.147.0",
        "claude-cli/2.1.287 (external,cli)",
        "qodercli/1.1.72",
    ] {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
        let primary = NativeProvider::start(vec![]);
        let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 2]);
        let fixture = native_fixture("unknown", &[&simple, &primary, &classifier]);
        let headers = [
            ("X-HiRoute-Token".into(), TOKEN.into()),
            ("User-Agent".into(), user_agent.into()),
        ];
        for _ in 0..2 {
            accepted(&send(
                fixture.address,
                &headers,
                json!("identical native input"),
            ));
        }
        assert_eq!(
            (simple.calls(), primary.calls(), classifier.calls()),
            (2, 0, 2),
            "{user_agent}"
        );
    }
}

#[test]
fn real_listener_native_history_keeps_accepted_execution_across_plan_revisions() {
    use hiroute_gateway::server::publication::ModelRouteV2;
    let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
    let complex = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 2]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&simple, &complex, &classifier],
        1,
    );
    let first = json!({"role":"user","content":"PREVIOUS_ACCEPTED_TASK"});
    accepted(&send(
        fixture.address,
        &headers("opencode", "session", "main", "turn-1", "checkpoint"),
        json!([first]),
    ));
    let mut publication = fixture.publication();
    publication.aliases[0].agent_plan_revision += 1;
    for grant in &mut publication.grants {
        if let Some(ModelRouteV2::Plan { revision, .. }) = grant.routes.get_mut(MODEL) {
            *revision += 1;
        }
    }
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    accepted(&send(
        fixture.address,
        &headers("opencode", "session", "main", "turn-2", "checkpoint"),
        json!([
            first, {"role":"assistant","content":"ok"}, {"role":"user","content":"NEW_PLAN_TASK"}
        ]),
    ));
    let history = classifier_body(&classifier, 1).to_string();
    assert!(history.contains("PREVIOUS_ACCEPTED_TASK"), "{history}");
    assert_eq!(classifier.calls(), 2);
}

#[test]
fn real_listener_unexplained_native_rewrites_cannot_replace_main_history() {
    for agent in ["pi", "dsh", "opencode"] {
        let simple = NativeProvider::start(vec![reply(RESPONSES_TOOL_CALL), reply(RESPONSES_OK)]);
        let complex = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier =
            NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_COMPLEX)]);
        let fixture = native_fixture(agent, &[&simple, &complex, &classifier]);
        let main = headers(agent, "parent", "main", "turn-1", "checkpoint-1");
        let user = json!({"role":"user","content":"REAL_MAIN_TASK"});
        let response = send(fixture.address, &main, json!([user]));
        accepted(&response);
        // A stale/copied main marker is insufficient proof of native compaction.
        accepted(&send(
            fixture.address,
            &main,
            json!("UNEXPLAINED_BACKGROUND_REWRITE"),
        ));
        let detached = classifier_body(&classifier, 1);
        assert!(
            detached["visible_conversation"]
                .to_string()
                .contains("REAL_MAIN_TASK"),
            "{agent}: {detached}"
        );
        assert!(
            !detached["preceding_stage"].is_null(),
            "{agent}: actual context remains eligible"
        );
        assert!(
            !detached["visible_conversation"]
                .to_string()
                .contains("UNEXPLAINED_BACKGROUND_REWRITE")
        );
        let response: Value = serde_json::from_slice(&response.body).unwrap();
        let call = &response["output"][0]["call_id"];
        accepted(&send(
            fixture.address,
            &main,
            json!([user,
                {"type":"function_call","call_id":call,"namespace":"tools","name":"lookup","arguments":"{}"},
                {"type":"function_call_output","call_id":call,"output":"found"}
            ]),
        ));
        assert_eq!(
            (simple.calls(), complex.calls(), classifier.calls()),
            (2, 1, 2),
            "{agent}"
        );
    }
}

#[test]
fn real_listener_hermes_in_place_compression_retains_observed_original_task() {
    let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
    let complex = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 2]);
    let fixture = native_fixture("hermes", &[&simple, &complex, &classifier]);
    let context = hermes_lineage("root", "native-conversation-turn", &["root"]);
    accepted(&send(
        fixture.address,
        &context,
        json!("ORIGINAL_MAIN_TASK"),
    ));
    accepted(&send(
        fixture.address,
        &context,
        json!("NATIVE_IN_PLACE_COMPRESSION_SUMMARY"),
    ));
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (2, 0, 2)
    );
    let after = classifier_body(&classifier, 1);
    assert!(
        after["visible_conversation"]
            .to_string()
            .contains("ORIGINAL_MAIN_TASK")
    );
    assert!(after["visible_conversation"].to_string().contains("ok"));
    assert!(!after["preceding_stage"].is_null());
}

#[path = "native_history_continuation.rs"]
mod continuation;
