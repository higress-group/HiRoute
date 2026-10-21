//! Regressions for synthetic User envelopes after native compaction.
use super::*;

fn existing_turn(mut headers: Vec<(String, String)>) -> Vec<(String, String)> {
    if let Some((_, encoded)) = headers
        .iter_mut()
        .find(|(key, _)| key == "x-hiroute-routing-context")
    {
        let mut context: Value = serde_json::from_str(encoded).unwrap();
        context["continuation_requires_existing_turn"] = json!(true);
        *encoded = context.to_string();
    }
    headers
}

#[test]
fn real_listener_same_native_turn_appended_summary_keeps_original_user() {
    for agent in ["codex", "pi", "opencode", "hermes"] {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 3]);
        let primary = NativeProvider::start(vec![]);
        let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 2]);
        let fixture = native_fixture(agent, &[&simple, &primary, &classifier]);
        let user = json!({"role":"user","content":"ORIGINAL_NATIVE_USER"});
        accepted(&send(
            fixture.address,
            &headers(agent, "main", "main", "turn-1", "before"),
            json!([user]),
        ));
        let synthetic = json!({"role":"user","content":"SYNTHETIC_NATIVE_SUMMARY"});
        let continuation = existing_turn(headers(agent, "main", "main", "turn-1", "after"));
        accepted(&send(
            fixture.address,
            &continuation,
            json!([user, synthetic]),
        ));
        assert_eq!(
            classifier.calls(),
            1,
            "{agent}: synthetic User opened another decision"
        );
        accepted(&send(
            fixture.address,
            &headers(agent, "main", "main", "turn-2", "after"),
            json!([
                user, synthetic, {"role":"assistant","content":"ok"}, {"role":"user","content":"FRESH_NATIVE_USER"}
            ]),
        ));
        assert_eq!(
            (simple.calls(), primary.calls(), classifier.calls()),
            (3, 0, 2),
            "{agent}"
        );
        let history = classifier_body(&classifier, 1);
        let visible = history["visible_conversation"].to_string();
        assert!(
            visible.contains("ORIGINAL_NATIVE_USER"),
            "{agent}: {history}"
        );
        assert!(
            !visible.contains("SYNTHETIC_NATIVE_SUMMARY"),
            "{agent}: {history}"
        );
        assert_eq!(history["visible_conversation"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn real_listener_cold_native_continuation_cannot_warm_itself_into_history() {
    for agent in ["pi", "opencode", "hermes"] {
        let simple = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
        let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 2]);
        let fixture = native_fixture(agent, &[&simple, &primary, &classifier]);
        let synthetic = json!({"role":"user","content":"COLD_SYNTHETIC_SUMMARY"});
        for turn in ["uncaptured-turn", "another-opaque-turn"] {
            accepted(&send(
                fixture.address,
                &existing_turn(headers(agent, "main", "main", turn, "after")),
                json!([synthetic]),
            ));
        }
        assert_eq!(
            classifier.calls(),
            0,
            "{agent}: cold synthetic User was classified"
        );
        let fresh = json!({"role":"user","content":"VERIFIED_FRESH_USER"});
        accepted(&send(
            fixture.address,
            &headers(agent, "main", "main", "fresh-turn", "after"),
            json!([synthetic, fresh]),
        ));
        assert_eq!(
            classifier.calls(),
            1,
            "{agent}: verified fresh producer cannot recover"
        );
        assert_eq!(
            classifier_body(&classifier, 0)["visible_conversation"],
            json!([])
        );
        accepted(&send(
            fixture.address,
            &headers(agent, "main", "main", "next-turn", "after"),
            json!([
                synthetic,fresh,{"role":"assistant","content":"ok"},{"role":"user","content":"NEXT_VERIFIED_USER"}
            ]),
        ));
        let history = classifier_body(&classifier, 1);
        assert!(
            history["visible_conversation"]
                .to_string()
                .contains("VERIFIED_FRESH_USER"),
            "{agent}: {history}"
        );
        assert!(
            !history["visible_conversation"]
                .to_string()
                .contains("COLD_SYNTHETIC_SUMMARY"),
            "{agent}: {history}"
        );
        assert_eq!(
            (simple.calls(), primary.calls(), classifier.calls()),
            (2, 2, 2),
            "{agent}"
        );
    }
}

#[test]
fn real_listener_claude_without_persistent_user_proof_keeps_main_and_child_output() {
    let barrier = ProviderBodyBarrier::default();
    let simple = NativeProvider::start(vec![]);
    let primary = NativeProvider::start(vec![
        ProviderReply::BodyReadBarrier {
            barrier: barrier.clone(),
            status: 200,
            error_kind: None,
            body: RESPONSES_TOOL_CALL,
        },
        reply(RESPONSES_OK),
        reply(RESPONSES_OK),
        reply(RESPONSES_OK),
        reply(RESPONSES_OK),
        reply(RESPONSES_OK),
    ]);
    let classifier = NativeProvider::start(vec![]);
    let fixture = native_fixture("claude", &[&simple, &primary, &classifier]);
    let main = headers("claude", "session", "main", "prompt-1", "before");
    let user = json!({"role":"user","content":"REAL_CLAUDE_TASK"});
    let first = std::thread::scope(|scope| {
        let _release = Release(barrier.clone());
        let address = fixture.address;
        let original = user.clone();
        let main_headers = main.clone();
        let main_request = scope.spawn(move || send(address, &main_headers, json!([original])));
        barrier.wait_for_arrivals(1, Duration::from_secs(5));
        let mut child = headers("claude", "session", "subagent", "child-prompt", "before");
        child.push(("x-claude-code-agent-id".into(), "native-child".into()));
        accepted(&send(address, &child, json!("CHILD_TASK")));
        barrier.release();
        main_request.join().unwrap()
    });
    accepted(&first);
    let first: Value = serde_json::from_slice(&first.body).unwrap();
    let call = &first["output"][0]["call_id"];
    accepted(&send(
        fixture.address,
        &main,
        json!([user,
            {"type":"function_call","call_id":call,"namespace":"tools","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":call,"output":"tool completed"}
        ]),
    ));
    let mut compacted = main.clone();
    compacted.push(("x-claude-code-context-compacted".into(), "auto".into()));
    accepted(&send(
        fixture.address,
        &compacted,
        json!("ONLY_USER_SUMMARY"),
    ));
    // The native flag is consumed once. Its disappearance and an opaque prompt
    // change must not turn the same unproved summary into a fresh user.
    accepted(&send(fixture.address, &main, json!("ONLY_USER_SUMMARY")));
    accepted(&send(
        fixture.address,
        &headers("claude", "session", "main", "prompt-2", "after"),
        json!("NEXT_USER_OR_UNMARKED_SUMMARY"),
    ));
    assert_eq!(
        (simple.calls(), primary.calls(), classifier.calls()),
        (0, 6, 0)
    );
    let facts = wait_execution_facts(&fixture, 6);
    let decisions = facts
        .iter()
        .filter(|fact| fact.pointer("/fact/kind") == Some(&json!("route_decision")))
        .collect::<Vec<_>>();
    assert_eq!(decisions.len(), 6);
    let mut branches = std::collections::BTreeMap::<String, usize>::new();
    for decision in decisions {
        assert_eq!(
            decision.pointer("/fact/complexity/fallback_used"),
            Some(&json!(true))
        );
        assert_eq!(
            decision.pointer("/fact/complexity/fallback_reason"),
            Some(&json!("unavailable"))
        );
        assert_eq!(
            decision.pointer("/fact/complexity/decision_source"),
            Some(&json!("unresolved"))
        );
        let branch = decision
            .pointer("/correlation/conversation_id")
            .and_then(Value::as_str)
            .unwrap();
        *branches.entry(branch.into()).or_default() += 1;
    }
    let mut counts = branches.values().copied().collect::<Vec<_>>();
    counts.sort();
    assert_eq!(
        counts,
        vec![1, 5],
        "native child observation must remain independent"
    );
}

#[test]
fn real_listener_cold_codex_persistent_compaction_window_never_bootstraps_a_summary() {
    let simple = NativeProvider::start(vec![]);
    let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 3]);
    let classifier = NativeProvider::start(vec![]);
    let fixture = native_fixture("codex", &[&simple, &primary, &classifier]);
    for turn in ["uncaptured", "uncaptured", "changed-opaque-id"] {
        accepted(&send(
            fixture.address,
            &headers("codex", "session", "main", turn, "after"),
            json!("PURE_USER_COMPACTED_SUMMARY"),
        ));
    }
    assert_eq!(
        (simple.calls(), primary.calls(), classifier.calls()),
        (0, 3, 0)
    );
}

#[test]
fn real_listener_fresh_questions_do_not_recover_claude_or_cold_compacted_codex_without_proof() {
    for agent in ["claude", "codex"] {
        let simple = NativeProvider::start(vec![]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 3]);
        let classifier = NativeProvider::start(vec![]);
        let fixture = native_fixture(agent, &[&simple, &primary, &classifier]);
        let summary = json!({"role":"user","content":"COLD_SYNTHETIC_SUMMARY"});
        accepted(&send(
            fixture.address,
            &headers(agent, "session", "main", "old-turn", "after"),
            json!([summary]),
        ));
        let fresh = json!({"role":"user","content":"ACTUAL_NEW_QUESTION"});
        accepted(&send(
            fixture.address,
            &headers(agent, "session", "main", "fresh-turn", "after"),
            json!([
                summary,{"role":"assistant","content":"uncaptured continuation"},fresh
            ]),
        ));
        accepted(&send(
            fixture.address,
            &headers(agent, "session", "main", "followup-turn", "after"),
            json!([
                summary,{"role":"assistant","content":"uncaptured continuation"},fresh,
                {"role":"assistant","content":"uncaptured fresh answer"},{"role":"user","content":"ACTUAL_FOLLOWUP"}
            ]),
        ));
        // This asserts a capability limit, not successful recovery: genuine new
        // questions still lack persistent native user provenance after this gap.
        assert_eq!(
            (simple.calls(), primary.calls(), classifier.calls()),
            (0, 3, 0),
            "{agent}"
        );
    }
}

#[test]
fn real_listener_invalid_or_legacy_native_origin_cannot_classify_wire_summaries() {
    for agent in ["codex", "claude", "pi", "opencode", "hermes"] {
        for legacy in [false, true] {
            let simple = NativeProvider::start(vec![]);
            let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
            let classifier = NativeProvider::start(vec![]);
            let fixture = native_fixture(agent, &[&simple, &primary, &classifier]);
            let mut broken = headers(agent, "session", "main", "turn", "after");
            if agent == "codex" {
                if legacy {
                    broken.retain(|(key, _)| key != "x-codex-turn-metadata");
                } else {
                    broken.push(("x-codex-window-id".into(), "session:0".into()));
                }
            } else if agent == "claude" {
                broken.retain(|(key, _)| key != "x-claude-code-request-class");
                broken.push(("user-agent".into(), "claude-cli/2.1.287".into()));
            } else {
                let encoded = &mut broken
                    .iter_mut()
                    .find(|(key, _)| key == "x-hiroute-routing-context")
                    .unwrap()
                    .1;
                let mut context: Value = serde_json::from_str(encoded).unwrap();
                if legacy {
                    context
                        .as_object_mut()
                        .unwrap()
                        .remove("continuation_requires_existing_turn");
                } else {
                    context["continuation_requires_existing_turn"] = json!("invalid");
                }
                *encoded = context.to_string();
            }
            for _ in 0..2 {
                accepted(&send(
                    fixture.address,
                    &broken,
                    json!("UNPROVEN_WIRE_SUMMARY"),
                ));
            }
            assert_eq!(
                (simple.calls(), primary.calls(), classifier.calls()),
                (0, 2, 0),
                "{agent}: legacy={legacy}"
            );
        }
    }
}
