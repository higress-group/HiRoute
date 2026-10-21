use super::*;

#[test]
fn hermes_chat_aliases_preserve_tools_without_claiming_input_origin() {
    use hiroute_gateway::server::request_plan::IngressProtocol;

    for reasoning_bytes in [32, 32 * 1024] {
        let plain = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#;
        let chat_tool = br#"{"id":"chat-tool","model":"runtime-native-model-1","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"native-call","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let chat_ok = br#"{"id":"chat-ok","model":"runtime-native-model-1","choices":[{"index":0,"message":{"role":"assistant","content":"42"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let primary = NativeProvider::start(vec![
            reply(chat_tool),
            reply(chat_ok),
            reply(chat_ok),
            reply(chat_ok),
            reply(chat_ok),
        ]);
        let regular = NativeProvider::start(vec![]);
        let decider = NativeProvider::start(vec![
            reply(plain),
            reply(c_answer(true, false)),
            reply(plain),
            reply(plain),
        ]);
        let fixture =
            RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], true);
        let mut publication = fixture.publication();
        for (candidate, provider) in publication.aliases[0]
            .candidates
            .iter_mut()
            .zip([&regular, &primary])
        {
            *candidate = hiroute_e2e::gateway_fixture::sealed_native_candidate(
                candidate.local_id,
                &candidate.stable_target_key,
                &candidate.credential_refs,
                provider.authority(),
                &candidate.upstream_model_id,
                &[
                    (IngressProtocol::Responses, IngressProtocol::ChatCompletions),
                    (
                        IngressProtocol::ChatCompletions,
                        IngressProtocol::ChatCompletions,
                    ),
                    (IngressProtocol::Messages, IngressProtocol::ChatCompletions),
                ],
            );
        }
        publication
            .grants
            .iter_mut()
            .find(|grant| grant.protocol == IngressProtocol::ChatCompletions)
            .unwrap()
            .consumer = Some(hiroute_domain::AgentModelConsumerV1::Hermes);
        enable_c_in_publication(&mut publication);
        fixture.publish(&mut publication);
        let send = |history: &Vec<Value>, session, turn| {
            let context = hermes_context(session, turn);
            let response = request(
                fixture.address,
                "POST",
                "/v1/chat/completions",
                &[
                    ("X-HiRoute-Token", CHAT_TOKEN),
                    ("x-hermes-session-id", session),
                    ("x-hiroute-agent", "hermes"),
                    ("x-hiroute-routing-context", &context),
                ],
                &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"messages":history,
                    "tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}]}))
                    .unwrap(),
            );
            assert_eq!(
                response.status,
                200,
                "history length {}, session {session}: {}",
                history.len(),
                String::from_utf8_lossy(&response.body)
            );
            let body: Value = serde_json::from_slice(&response.body).unwrap();
            body["choices"][0]["message"].clone()
        };
        let thinking = "r".repeat(reasoning_bytes);
        let mut history = vec![json!({"role":"user","content":"Read the value with lookup."})];
        let mut assistant = send(&history, "hermes-main", "lookup-user-1");
        assert_eq!(assistant["tool_calls"][0]["function"]["name"], "lookup");
        assistant["reasoning_content"] = json!(thinking);
        assistant["reasoning"] = json!(thinking);
        let call_id = assistant["tool_calls"][0]["id"].clone();
        history.push(assistant);
        history.push(json!({"role":"tool","tool_call_id":call_id,"content":"42"}));
        let mut assistant = send(&history, "hermes-main", "lookup-user-1");
        assert_eq!(
            decider.calls(),
            0,
            "a tool continuation cannot establish original-user provenance"
        );
        assistant["reasoning_content"] = json!(thinking);
        assistant["reasoning"] = json!(thinking);
        history.push(assistant);
        history.push(json!({"role":"user","content":"Add eight to that value."}));
        send(&history, "hermes-main", "lookup-user-2");
        assert_eq!(decider.calls(), 0);
        // Another native session and rewritten main history also keep execution
        // without manufacturing a complete assessment target.
        send(&history, "hermes-other", "other-user-1");
        history[2]["content"] = json!("43");
        history.last_mut().unwrap()["content"] = json!("Add eight to the corrected value.");
        send(&history, "hermes-main", "lookup-user-3");
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (0, 5, 0)
        );
        let facts = wait_execution_facts(&fixture, 5);
        let decisions = facts
            .iter()
            .filter(|fact| fact["fact"]["kind"] == "route_decision")
            .collect::<Vec<_>>();
        assert_eq!(decisions.len(), 5);
        assert!(
            decisions
                .iter()
                .all(|fact| fact["fact"]["complexity"]["fallback_reason"] == "unavailable")
        );
    }
}

#[test]
fn hermes_legacy_freshness_does_not_recover_classification_for_real_users() {
    use hiroute_gateway::server::request_plan::IngressProtocol;

    let uncaptured = br#"{"id":"uncaptured","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"LEGACY_UNCAPTURED_FALLBACK_OUTPUT"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
    let regular = NativeProvider::start(vec![]);
    let primary = NativeProvider::start(vec![reply(uncaptured); 4]);
    let decider = NativeProvider::start(vec![]);
    let fixture = RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], true);
    let mut publication = fixture.publication();
    publication
        .grants
        .iter_mut()
        .find(|grant| grant.protocol == IngressProtocol::ChatCompletions)
        .unwrap()
        .consumer = Some(hiroute_domain::AgentModelConsumerV1::Hermes);
    enable_c_in_publication(&mut publication);
    fixture.publish(&mut publication);
    let send = |history: &[Value], context: Option<&str>| {
        let mut headers = vec![("X-HiRoute-Token", CHAT_TOKEN)];
        if let Some(context) = context {
            headers.extend([
                ("x-hermes-session-id", "hermes-recovery"),
                ("x-hiroute-agent", "hermes"),
                ("x-hiroute-routing-context", context),
            ]);
        }
        let response = request(
            fixture.address,
            "POST",
            "/v1/chat/completions",
            &headers,
            &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"messages":history})).unwrap(),
        );
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        serde_json::from_slice::<Value>(&response.body).unwrap()["choices"][0]["message"].clone()
    };
    let mut history = vec![json!({"role":"user","content":"LEGACY_SYNTHETIC_SUMMARY"})];
    // No adapter and a legacy adapter without the new boolean both keep native
    // model execution, but neither can create a classifier User or shared history.
    let legacy = json!({"version":1,"agent":"hermes","session_id":"hermes-recovery",
        "purpose":"main","turn_id":"legacy-turn"})
    .to_string();
    for context in [None, Some(legacy.as_str())] {
        let assistant = send(&history, context);
        assert!(
            assistant
                .to_string()
                .contains("LEGACY_UNCAPTURED_FALLBACK_OUTPUT")
        );
        history.push(assistant);
        assert_eq!(
            decider.calls(),
            0,
            "legacy metadata cannot establish an original User"
        );
    }
    assert_eq!((regular.calls(), primary.calls()), (0, 2));

    // Even actual new questions cannot recover without independent origin proof.
    for (turn, user) in [
        ("recovery-user-1", "FIRST_REAL_USER"),
        ("recovery-user-2", "SECOND_REAL_USER"),
    ] {
        history.push(json!({"role":"user","content":user}));
        let assistant = send(&history, Some(&hermes_context("hermes-recovery", turn)));
        assert!(
            assistant
                .to_string()
                .contains("LEGACY_UNCAPTURED_FALLBACK_OUTPUT")
        );
        history.push(assistant);
    }
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (0, 4, 0)
    );
    let facts = wait_execution_facts(&fixture, 4);
    let decisions = facts
        .iter()
        .filter(|fact| fact["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_eq!(decisions.len(), 4);
    assert!(
        decisions
            .iter()
            .all(|fact| fact["fact"]["complexity"]["fallback_reason"] == "unavailable")
    );
}
