use super::*;
use hiroute_gateway::server::publication::token_sha256;
use serde_json::{Value, json};

#[test]
fn hermes_equal_prompts_and_unproven_ids_are_isolated_by_the_real_listener() {
    assert_native_identity(false);
}

#[test]
fn hermes_chat_identity_is_admitted_and_isolated_by_the_real_listener() {
    assert_native_identity(true);
}

fn assert_native_identity(chat: bool) {
    use hiroute_gateway::server::request_plan::IngressProtocol;
    let protocol = if chat {
        IngressProtocol::ChatCompletions
    } else {
        IngressProtocol::Responses
    };
    let token = if chat { CHAT_TOKEN } else { TOKEN };
    let simple = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#;
    let reply = |body| ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body,
    };
    let regular = NativeProvider::start(vec![reply(RESPONSES_OK); 11]);
    let primary = NativeProvider::start(vec![]);
    let decider = NativeProvider::start(vec![reply(simple.as_slice()); 11]);
    let fixture = RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], true);
    let mut publication = fixture.publication();
    let grant = publication
        .grants
        .iter_mut()
        .find(|grant| grant.protocol == protocol)
        .unwrap();
    grant.consumer = Some(hiroute_domain::AgentModelConsumerV1::Hermes);
    let mut other = grant.clone();
    other.grant_id = "another-hermes-connection".into();
    other.bearer_token_sha256 = token_sha256("another-hermes-fixture-token");
    publication.grants.push(other);
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    let body = serde_json::to_vec(&if chat {
        json!({"model":MODEL,"stream":false,"messages":[{"role":"user","content":"An identical real user task."}]})
    } else {
        json!({"model":MODEL,"stream":false,"input":[{"role":"user","content":"An identical real user task."}]})
    })
    .unwrap();
    let send = |headers: &[(&str, &str)]| {
        let response = request(
            fixture.address,
            "POST",
            if chat {
                "/v1/chat/completions"
            } else {
                "/v1/responses"
            },
            headers,
            &body,
        );
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
    };
    // Equal first-user content cannot join two concurrent native conversations.
    std::thread::scope(|scope| {
        for session in ["native-conversation-one", "native-conversation-two"] {
            let send = &send;
            scope.spawn(move || {
                let context = json!({"version":1,"agent":"hermes","session_id":session,
                    "purpose":"main","turn_id":"first-real-user",
                    "continuation_requires_existing_turn":false})
                .to_string();
                send(&[
                    ("X-HiRoute-Token", token),
                    ("x-hermes-session-id", session),
                    ("x-hiroute-agent", "hermes"),
                    ("x-hiroute-routing-context", &context),
                ])
            });
        }
    });
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (2, 0, 2)
    );
    let long = "a".repeat(129);
    for claims in [
        vec![],
        vec![
            ("x-hermes-session-id", "same"),
            ("x-hermes-session-id", "same"),
        ],
        vec![("x-hermes-session-id", "same"), ("session-id", "same")],
        vec![("x-hermes-session-id", long.as_str())],
    ] {
        let mut headers = vec![("X-HiRoute-Token", token)];
        headers.extend(claims);
        let classified = decider.calls();
        send(&headers);
        send(&headers);
        assert_eq!(
            decider.calls(),
            classified + 2,
            "invalid identity stays request-scoped while observed input remains classifiable"
        );
    }
    // An equal raw ID under a different admitted connection is another conversation.
    let context = json!({"version":1,"agent":"hermes","session_id":"native-conversation-one",
        "purpose":"main","turn_id":"first-real-user",
        "continuation_requires_existing_turn":false})
    .to_string();
    send(&[
        ("X-HiRoute-Token", "another-hermes-fixture-token"),
        ("x-hermes-session-id", "native-conversation-one"),
        ("x-hiroute-agent", "hermes"),
        ("x-hiroute-routing-context", &context),
    ]);
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (11, 0, 11)
    );
    let facts = wait_execution_facts(&fixture, 11);
    let decisions = facts
        .iter()
        .filter(|fact| fact.pointer("/fact/kind") == Some(&json!("route_decision")))
        .map(|fact| &fact["fact"]["complexity"])
        .collect::<Vec<_>>();
    assert_eq!(decisions.len(), 11);
    assert_eq!(
        decisions
            .iter()
            .filter(|decision| {
                decision["decision_source"] == "external_classifier"
                    && decision["fallback_used"] == false
            })
            .count(),
        11
    );
    assert_eq!(
        decisions
            .iter()
            .filter(|decision| {
                decision["fallback_used"] == true && decision["fallback_reason"] == "unavailable"
            })
            .count(),
        0
    );
    let finished = facts
        .iter()
        .filter(|fact| fact.pointer("/fact/kind") == Some(&json!("request_finished")))
        .collect::<Vec<_>>();
    let conversations = finished
        .iter()
        .map(|fact| {
            fact.pointer("/correlation/conversation_id")
                .and_then(Value::as_str)
                .expect("recorded conversation identity")
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(conversations.len(), 11);
    let rendered = serde_json::to_string(&facts).unwrap();
    assert!(
        !rendered.contains("native-conversation-one")
            && !rendered.contains("native-conversation-two")
    );
}

#[test]
fn hermes_context_fault_then_recovery_keeps_independent_lineage_and_exact_assessment() {
    use super::native_history::{accepted, classifier_body, native_fixture, reply, send};
    const ASSESSED: &[u8] = br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#;
    for fault in [
        None,
        Some("not-json"),
        Some(r#"{"version":99,"agent":"hermes","session_id":"child","purpose":"main"}"#),
    ] {
        let regular = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK)]);
        let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(ASSESSED)]);
        let fixture = native_fixture("hermes", &[&regular, &primary, &classifier]);
        let stamp = json!({"observer_id":"observer","sequence":1,"observed_at_ms":1000,"age_ms":0});
        let lineage = json!({"version":1,"ids":["ancestor","child"],"complete":true,
            "proof_observation":stamp,"current_state":{"value":"open","observation":stamp}});
        let mut h = vec![
            ("X-HiRoute-Token".into(), TOKEN.into()),
            ("x-hiroute-agent".into(), "hermes".into()),
            ("x-hermes-session-id".into(), "child".into()),
            ("x-hermes-compression-lineage".into(), lineage.to_string()),
        ];
        if let Some(fault) = fault {
            h.push(("x-hiroute-routing-context".into(), fault.into()));
        }
        let mut input = vec![json!({"role":"user","content":"A_ACTUAL_FAULT_EXECUTION"})];
        let a = send(fixture.address, &h, json!(input));
        accepted(&a);
        let a: Value = serde_json::from_slice(&a.body).unwrap();
        input.extend(a["output"].as_array().unwrap().iter().cloned());
        input.push(json!({"role":"user","content":"B_NEW_INDEPENDENT_TASK"}));
        h.retain(|(name, _)| name != "x-hiroute-routing-context");
        h.push((
            "x-hiroute-routing-context".into(),
            json!({"version":2,"agent":"hermes","session_id":"child","purpose":"main",
                "turn_id":"B","request_id":"B:1","input":{"kind":"observed_batch","event_id":"B"}})
            .to_string(),
        ));
        accepted(&send(fixture.address, &h, json!(input)));
        assert_eq!(
            (regular.calls(), primary.calls(), classifier.calls()),
            (1, 1, 2)
        );
        let judged = classifier_body(&classifier, 1);
        assert_eq!(
            judged["preceding_stage"]["complete"], true,
            "{fault:?}: {judged}"
        );
        let from = judged["preceding_stage"]["from"].as_u64().unwrap() as usize;
        assert_eq!(
            json!(&judged["visible_conversation"].as_array().unwrap()[from..]),
            json!([{"user":[{"kind":"text","text":"A_ACTUAL_FAULT_EXECUTION"},
                {"kind":"unavailable","source_kind":"unverified_request_user_origin"}],
                "status":"completed","steps":[[{"kind":"text","text":"ok"}]]}])
        );
        let facts = wait_execution_facts(&fixture, 2);
        let routes = facts
            .iter()
            .filter(|e| e["fact"]["kind"] == "route_decision")
            .collect::<Vec<_>>();
        assert_eq!(routes.len(), 2);
        for (route, group) in routes.iter().zip(["regular", "primary"]) {
            assert_eq!(
                route["fact"]["complexity"]["decision_source"],
                "external_classifier"
            );
            assert_eq!(route["fact"]["complexity"]["execution_group"], group);
            assert_eq!(route["fact"]["complexity"]["fallback_used"], false);
        }
        assert_eq!(
            routes[0]["correlation"]["conversation_id"],
            routes[1]["correlation"]["conversation_id"]
        );
        let score = facts
            .iter()
            .find(|e| e["fact"]["kind"] == "branch_assessment_recorded")
            .unwrap();
        assert_eq!(score["fact"]["score"], 0.75);
        assert_eq!(
            score["fact"]["history_membership"]["request_ids"],
            json!([routes[0]["correlation"]["request_id"]])
        );
        assert_eq!(
            score["fact"]["history_membership"]["capture_partial"],
            false
        );
        assert_eq!(
            score["fact"]["history_membership"]["source_unverified"],
            true
        );
    }
}
