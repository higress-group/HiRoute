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
            "hermes" => "x-hermes-session-id",
            _ => unreachable!(),
        };
        headers.extend([
            (native.into(), session.into()), ("x-hiroute-agent".into(), agent.into()),
            ("x-hiroute-routing-context".into(), json!({"version":1,"agent":agent,"session_id":session,"purpose":purpose,"turn_id":turn,"checkpoint":checkpoint}).to_string()),
        ]);
        if agent == "pi" {
            headers.push(("User-Agent".into(), "pi (linux; x64)".into()));
            headers.push(("x-client-request-id".into(), session.into()));
        }
    }
    headers
}
fn native_fixture(agent: &str, providers: &[&NativeProvider]) -> RuntimeFixture {
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
fn send(address: std::net::SocketAddr, headers: &[(String, String)], input: Value) -> WireResponse {
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
    for agent in ["codex", "claude", "pi", "dsh", "opencode", "hermes"] {
        for fault in [false, true] {
            let simple = NativeProvider::start(vec![
                reply(RESPONSES_TOOL_CALL),
                reply(RESPONSES_OK),
                reply(RESPONSES_OK),
            ]);
            let complex = NativeProvider::start(vec![reply(RESPONSES_OK)]);
            let classifier = NativeProvider::start(if agent == "dsh" {
                vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_SIMPLE)]
            } else {
                vec![
                    reply(CLASSIFIER_SIMPLE),
                    reply(CLASSIFIER_COMPLEX),
                    reply(CLASSIFIER_SIMPLE),
                ]
            });
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
            let calls_before_next = if agent == "dsh" { 1 } else { 2 };
            if agent == "dsh" {
                assert_eq!(classifier.calls(), 1, "unproved DSH input was classified");
            } else {
                let detached = classifier_body(&classifier, 1);
                assert_eq!(detached["history_partial"], true, "{agent}: {detached}");
                assert_eq!(detached["visible_conversation"], json!([]));
                assert!(detached["assessment_target"].is_null());
            }
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
                (3, 1, calls_before_next + 1)
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
    for agent in ["codex", "claude", "pi", "dsh", "opencode", "hermes"] {
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
                    "session_id":"parent","thread_id":"child","turn_id":"child-turn","request_kind":"turn"
                }).to_string();
            }
            accepted(&send(address, &child, json!("CHILD_USER")));
            accepted(&send(
                address,
                &headers(agent, "parent", "compaction", "turn-1", "checkpoint-1"),
                json!("AUXILIARY_USER"),
            ));
            let classified_auxiliary = if agent == "dsh" { 2 } else { 3 };
            assert_eq!(classifier.calls(), classified_auxiliary);
            for index in 1..classified_auxiliary {
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
            (2, 2, if agent == "dsh" { 2 } else { 3 }),
            "{agent}"
        );
    }
}

#[test]
fn real_listener_native_compaction_reuses_semantic_decision_without_reusing_prefix_affinity() {
    // DSH checkpoints omit source.kind=user after compaction. Its stricter
    // missing-user fallback is covered by dsh_native_routing, not by declaring
    // this synthetic summary to be a real user input.
    for agent in ["codex", "claude", "pi", "opencode", "hermes"] {
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
fn real_listener_hermes_rotating_compression_and_gateway_restart_keep_only_proven_history() {
    for _restart in 0..2 {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 3]);
        let complex = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier = NativeProvider::start(vec![
            reply(CLASSIFIER_SIMPLE),
            reply(CLASSIFIER_COMPLEX),
            reply(CLASSIFIER_SIMPLE),
        ]);
        let fixture = native_fixture("hermes", &[&simple, &complex, &classifier]);
        accepted(&send(
            fixture.address,
            &hermes_lineage("root", "turn-1", &["root"]),
            json!("ORIGINAL_MAIN_TASK"),
        ));
        assert_eq!(
            classifier_body(&classifier, 0)["visible_conversation"],
            json!([])
        );
        // Native auxiliary clients have authorization but no selected-conversation hook.
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
        assert_eq!(
            classifier.calls(),
            2,
            "rotating native compaction lost its decision"
        );
        accepted(&send(
            fixture.address,
            &hermes_lineage("compressed", "turn-2", &["root", "compressed"]),
            json!([
                summary, {"role":"assistant","content":"ok"}, {"role":"user","content":"NEW_MAIN_TASK"}
            ]),
        ));
        let history = classifier_body(&classifier, 2).to_string();
        assert!(history.contains("ORIGINAL_MAIN_TASK"), "{history}");
        assert!(!history.contains("AUXILIARY_CALCULATION"), "{history}");
        assert_eq!(
            (simple.calls(), complex.calls(), classifier.calls()),
            (3, 1, 3)
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
    assert_eq!(classifier.calls(), 2);
}

#[test]
fn real_listener_native_new_turn_does_not_reuse_a_decision_for_identical_content() {
    for agent in ["codex", "claude", "pi", "dsh", "opencode", "hermes"] {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let complex = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier =
            NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_COMPLEX)]);
        let fixture = native_fixture(agent, &[&simple, &complex, &classifier]);
        for turn in ["native-turn-1", "native-turn-2"] {
            accepted(&send(
                fixture.address,
                &headers(agent, "session", "main", turn, "checkpoint"),
                json!("identical real user task"),
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
fn real_listener_known_native_clients_without_loaded_adapters_cannot_infer_shared_history() {
    for user_agent in [
        "pi (linux; x64)",
        "deepseek-harness/0.2.0-rc.2",
        "opencode/1.19.3",
        "codex_cli_rs/0.147.0",
        "claude-cli/2.1.287 (external,cli)",
        "qodercli/1.1.72",
    ] {
        let dsh = user_agent.starts_with("deepseek-harness/");
        let simple = NativeProvider::start(if dsh {
            vec![]
        } else {
            vec![reply(RESPONSES_OK); 2]
        });
        let complex = NativeProvider::start(if dsh {
            vec![reply(RESPONSES_OK); 2]
        } else {
            vec![]
        });
        let classifier = NativeProvider::start(if dsh {
            vec![]
        } else {
            vec![reply(CLASSIFIER_SIMPLE); 2]
        });
        let fixture = native_fixture("unknown", &[&simple, &complex, &classifier]);
        let unadapted = [
            ("X-HiRoute-Token".into(), TOKEN.into()),
            ("User-Agent".into(), user_agent.into()),
        ];
        for _ in 0..2 {
            accepted(&send(
                fixture.address,
                &unadapted,
                json!("identical native input"),
            ));
        }
        if dsh {
            assert_eq!(
                (simple.calls(), complex.calls(), classifier.calls()),
                (0, 2, 0),
                "DSH without a semantic user proof must complete both requests through Primary"
            );
        } else {
            for index in 0..2 {
                let history = classifier_body(&classifier, index);
                assert_eq!(history["history_partial"], true, "{user_agent}: {history}");
                assert_eq!(
                    history["visible_conversation"],
                    json!([]),
                    "{user_agent}: {history}"
                );
                assert!(history["assessment_target"].is_null());
            }
            assert_eq!(
                (simple.calls(), complex.calls(), classifier.calls()),
                (2, 0, 2),
                "{user_agent}"
            );
        }
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
        assert_eq!(
            detached["visible_conversation"],
            json!([]),
            "{agent}: {detached}"
        );
        assert!(detached["assessment_target"].is_null());
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
fn real_listener_hermes_in_place_compression_uses_its_native_conversation_turn() {
    let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
    let complex = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE)]);
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
    assert_eq!((simple.calls(), classifier.calls()), (2, 1));
}
