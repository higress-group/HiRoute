use super::*;

#[test]
fn hermes_chat_aliases_keep_tool_attribution_and_request_all_three_questions() {
    use hiroute_gateway::server::request_plan::IngressProtocol;

    for reasoning_bytes in [32, 32 * 1024] {
        let plain = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#;
        let chat_tool = br#"{"id":"chat-tool","model":"runtime-native-model-1","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"native-call","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let chat_ok = br#"{"id":"chat-ok","model":"runtime-native-model-1","choices":[{"index":0,"message":{"role":"assistant","content":"42"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let regular = NativeProvider::start(vec![
            reply(chat_tool),
            reply(chat_ok),
            reply(chat_ok),
            reply(chat_ok),
            reply(chat_ok),
        ]);
        let primary = NativeProvider::start(vec![]);
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
        let send = |history: &Vec<Value>, session| {
            let response = request(
                fixture.address,
                "POST",
                "/v1/chat/completions",
                &[
                    ("X-HiRoute-Token", CHAT_TOKEN),
                    ("x-hermes-session-id", session),
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
        let mut history = vec![
            json!({"role":"user","content":"Read the value with lookup.",
            "message_id":"platform-message-1"}),
        ];
        let mut assistant = send(&history, "hermes-main");
        assert_eq!(assistant["tool_calls"][0]["function"]["name"], "lookup");
        assistant["reasoning_content"] = json!(thinking);
        assistant["reasoning"] = json!(thinking);
        let call_id = assistant["tool_calls"][0]["id"].clone();
        history.push(assistant);
        history.push(json!({"role":"tool","tool_call_id":call_id,"content":"42"}));
        let mut assistant = send(&history, "hermes-main");
        assert_eq!(
            decider.calls(),
            1,
            "tool continuation inherits its own opening decision"
        );
        assistant["reasoning_content"] = json!(thinking);
        assistant["reasoning"] = json!(thinking);
        history.push(assistant);
        history.push(json!({"role":"user","content":"Add eight to that value.",
            "message_id":"platform-message-2"}));
        send(&history, "hermes-main");
        assert_eq!(decider.calls(), 2);
        // Neither another native session nor a rewritten main history may borrow
        // the complete assessment target from the original conversation.
        send(&history, "hermes-other");
        history[2]["content"] = json!("43");
        send(&history, "hermes-main");
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (5, 0, 4)
        );
        for (index, request) in decider.requests().iter().enumerate() {
            let offset = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let body: Value = serde_json::from_slice(&request[offset..]).unwrap();
            assert_eq!(body["questions"].get("q2").is_some(), index == 1);
            assert_eq!(body["questions"].get("q1").is_some(), index == 1);
        }
        let facts = wait_execution_facts(&fixture, 5);
        assert!(facts.iter().any(|fact| {
            fact.pointer("/fact/complexity/routing_observation/applicability_state")
                == Some(&json!("valid"))
        }));
    }
}
