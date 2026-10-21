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
    let regular = NativeProvider::start(vec![reply(RESPONSES_OK); 3]);
    let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 8]);
    let decider = NativeProvider::start(vec![reply(simple.as_slice()); 3]);
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
            classified,
            "missing or unproven native IDs cannot prove a current original User"
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
        (3, 8, 3)
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
        3
    );
    assert_eq!(
        decisions
            .iter()
            .filter(|decision| {
                decision["fallback_used"] == true && decision["fallback_reason"] == "unavailable"
            })
            .count(),
        8
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
