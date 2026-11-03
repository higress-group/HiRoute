//! Producer-shaped Hermes Chat fields through the real listener, including spill.
use super::native_history::{classifier_body, reply};
use super::*;
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

fn streamed(tool: Option<&str>) -> ProviderReply {
    let mut deltas = vec![
        json!({"role":"assistant","reasoning_content":"Check the observed input."}),
        json!({"content":"Use the synthetic result."}),
    ];
    if let Some(id) = tool {
        deltas.push(json!({"tool_calls":[{"index":0,"id":id,"type":"function",
            "function":{"name":"lookup","arguments":"{}"}}]}));
    }
    let mut chunks: Vec<_> = deltas
        .into_iter()
        .map(|delta| {
            format!(
                "data: {}\n\n",
                json!({"id":"native-chat","model":"runtime-native",
            "choices":[{"index":0,"delta":delta,"finish_reason":null}]})
            )
            .into_bytes()
        })
        .collect();
    chunks.push(
        format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"id":"native-chat",
        "model":"runtime-native","choices":[{"index":0,"delta":{},
        "finish_reason":if tool.is_some(){"tool_calls"}else{"stop"}}],
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})
        )
        .into_bytes(),
    );
    ProviderReply::StreamDrip {
        status: 200,
        chunks,
        interval: Duration::from_millis(1),
    }
}

fn send(fixture: &RuntimeFixture, input: &[Value], turn: &str) -> Value {
    let context = json!({"version":2,"agent":"hermes","session_id":"retained-hermes",
        "purpose":"main","turn_id":turn,"input":{"kind":"observed_batch","event_id":turn}})
    .to_string();
    let response = request(
        fixture.address,
        "POST",
        "/v1/chat/completions",
        &[
            ("X-HiRoute-Token", CHAT_TOKEN),
            ("x-hiroute-agent", "hermes"),
            ("x-hermes-session-id", "retained-hermes"),
            ("x-hiroute-routing-context", &context),
        ],
        &serde_json::to_vec(&json!({"model":MODEL,"stream":true,"messages":input,
        "tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}]}))
        .unwrap(),
    );
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tools = BTreeMap::<usize, Value>::new();
    for data in std::str::from_utf8(&response.body)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
    {
        let event: Value = serde_json::from_str(data).expect("invalid Chat SSE JSON");
        for choice in event["choices"].as_array().unwrap() {
            let delta = &choice["delta"];
            if let Some(value) = delta["content"].as_str() {
                text.push_str(value);
            }
            if let Some(value) = delta["reasoning_content"].as_str() {
                reasoning.push_str(value);
            }
            for call in delta["tool_calls"].as_array().into_iter().flatten() {
                let tool = tools
                    .entry(call["index"].as_u64().unwrap() as usize)
                    .or_insert_with(
                        || json!({"id":"","type":"function","function":{"name":"","arguments":""}}),
                    );
                if let Some(id) = call["id"].as_str() {
                    tool["id"] = json!(id);
                }
                for field in ["name", "arguments"] {
                    if let Some(value) = call["function"][field].as_str() {
                        tool["function"][field] = json!(format!(
                            "{}{value}",
                            tool["function"][field].as_str().unwrap()
                        ));
                    }
                }
            }
        }
    }
    assert_eq!(text, "Use the synthetic result.");
    assert_eq!(reasoning, "Check the observed input.");
    let mut assistant = json!({"role":"assistant","content":text,"reasoning_content":reasoning.clone(),"reasoning":reasoning});
    if !tools.is_empty() {
        assistant["tool_calls"] = json!(tools.into_values().collect::<Vec<_>>());
    }
    assistant
}

#[test]
fn retained_long_hermes_chat_keeps_tool_stage_and_complete_next_input_assessment() {
    let regular = NativeProvider::start(vec![
        streamed(Some("first")),
        streamed(Some("second")),
        streamed(None),
        streamed(None),
    ]);
    let primary = NativeProvider::start(vec![streamed(None)]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_COMPLEX),
        reply(br#"{"group":"regular","assessment":{"score":0.75,"partial":false}}"#),
        reply(br#"{"group":"regular","assessment":{"score":0.75,"partial":false}}"#),
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    let mut publication = fixture.publication();
    publication.publication_revision += 1;
    publication.aliases[0].protocols = vec![IngressProtocol::ChatCompletions];
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
            &[(
                IngressProtocol::ChatCompletions,
                IngressProtocol::ChatCompletions,
            )],
        );
    }
    for grant in &mut publication.grants {
        grant.consumer = Some(hiroute_domain::AgentModelConsumerV1::Hermes);
    }
    fixture.publish(&mut publication);
    let mut input = Vec::new();
    for i in 0..50 {
        input.push(json!({"role":"user","content":format!("Old input {i}: {}","x".repeat(16384))}));
        input.push(json!({"role":"assistant","content":format!("Uncaptured background {i}: {}","y".repeat(16384))}));
    }
    input.push(json!({"role":"user","content":"Bootstrap this retained session."}));
    assert!(serde_json::to_vec(&input).unwrap().len() > 1024 * 1024);
    let bootstrap = send(&fixture, &input, "bootstrap");
    input.extend([
        bootstrap,
        json!({"role":"user","content":"Run two harmless lookups."}),
    ]);
    for _ in 0..2 {
        let assistant = send(&fixture, &input, "actual-A");
        assert_eq!(
            classifier.calls(),
            2,
            "a tool continuation opened another decision"
        );
        let id = assistant["tool_calls"][0]["id"].clone();
        input.extend([
            assistant,
            json!({"role":"tool","tool_call_id":id,"content":"42"}),
        ]);
    }
    let final_answer = send(&fixture, &input, "actual-A");
    assert_eq!(classifier.calls(), 2);
    input.extend([
        final_answer,
        json!({"role":"user","content":"Assess the completed task."}),
    ]);
    send(&fixture, &input, "actual-B");
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (4, 1, 3)
    );
    let judged = classifier_body(&classifier, 2);
    assert_eq!(judged["preceding_stage"]["complete"], true, "{judged}");
    let facts = wait_execution_facts(&fixture, 5);
    let score = facts
        .iter()
        .rev()
        .find(|e| e["fact"]["kind"] == "branch_assessment_recorded")
        .unwrap();
    assert_eq!(score["fact"]["partial"], false);
    assert_eq!(
        score["fact"]["history_membership"]["request_ids"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
