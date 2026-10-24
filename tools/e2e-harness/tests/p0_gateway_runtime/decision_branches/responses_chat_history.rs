//! Controlled providers force the native phase that previously made Regular permanently
//! ineligible. This is listener/Replay evidence, not an installed DSH producer test.
use super::*;
use hiroute_gateway::server::request_plan::IngressProtocol;

const CHAT_OK: &[u8] = br#"{"id":"regular-ok","model":"regular-chat","choices":[{"index":0,"message":{"role":"assistant","content":"regular answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
const PRIMARY_TOOLS: &[u8] = br#"{"id":"primary-tools","model":"primary-responses","status":"completed","output":[{"type":"message","id":"msg-commentary","role":"assistant","phase":"commentary","status":"completed","content":[{"type":"output_text","text":"I will check the record."}]},{"type":"function_call","id":"fc-lookup","call_id":"lookup-call","name":"lookup","arguments":"{\"key\":\"alpha\"}","status":"completed"},{"type":"function_call","id":"fc-check","call_id":"check-call","name":"check","arguments":"{\"key\":\"beta\"}","status":"completed"}],"usage":{"input_tokens":5,"output_tokens":4,"total_tokens":9}}"#;
const PRIMARY_FINAL: &[u8] = br#"{"id":"primary-final","model":"primary-responses","status":"completed","output":[{"type":"message","id":"msg-final","role":"assistant","phase":"final_answer","status":"completed","content":[{"type":"output_text","text":"Both records are valid."}]}],"usage":{"input_tokens":9,"output_tokens":3,"total_tokens":12}}"#;
const SIMPLE: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#;
const COMPLEX_WITH_ASSESSMENT: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.1,"1":0.9}},"q1":{"type":"score","score":1.8},"q2":{"type":"choice","probabilities":{"applicable":0.9,"not_established":0.1}}}}"#;
const SIMPLE_WITH_ASSESSMENT: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q1":{"type":"score","score":1.8},"q2":{"type":"choice","probabilities":{"applicable":0.9,"not_established":0.1}}}}"#;

#[test]
fn completed_responses_commentary_returns_to_regular_chat_with_tools_and_assessment() {
    let regular = NativeProvider::start(vec![reply(CHAT_OK); 2]);
    let primary = NativeProvider::start(vec![reply(PRIMARY_TOOLS), reply(PRIMARY_FINAL)]);
    let decider = NativeProvider::start(vec![
        reply(SIMPLE),
        reply(COMPLEX_WITH_ASSESSMENT),
        reply(SIMPLE_WITH_ASSESSMENT),
    ]);
    let fixture = RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], true);
    let mut publication = fixture.publication();
    let candidate = &mut publication.aliases[0].candidates[0];
    *candidate = hiroute_e2e::gateway_fixture::sealed_native_candidate(
        candidate.local_id,
        &candidate.stable_target_key,
        &candidate.credential_refs,
        regular.authority(),
        &candidate.upstream_model_id,
        &[
            (IngressProtocol::Responses, IngressProtocol::ChatCompletions),
            (IngressProtocol::ChatCompletions, IngressProtocol::ChatCompletions),
            (IngressProtocol::Messages, IngressProtocol::ChatCompletions),
        ],
    );
    enable_c_in_publication(&mut publication);
    fixture.publish(&mut publication);
    let send = |input: &[Value]| {
        let response = request(
            fixture.address,
            "POST",
            "/v1/responses",
            &[("X-HiRoute-Token", TOKEN), ("session-id", "phase-return")],
            &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"input":input,
                "tools":[
                    {"type":"function","name":"lookup","parameters":{"type":"object"}},
                    {"type":"function","name":"check","parameters":{"type":"object"}}
                ]})).unwrap(),
        );
        let final_user = input.last().and_then(|item| item.get("content"));
        if final_user.is_some_and(|content| content.as_str() == Some("Now answer a simple question.")
            || content.pointer("/0/text").and_then(Value::as_str) == Some("Now answer a simple question.")) {
            let facts = wait_execution_facts(&fixture, 4);
            let decision = facts.iter().filter(|fact| fact["fact"]["kind"] == "route_decision").last().unwrap();
            let complexity = &decision["fact"]["complexity"];
            assert_eq!(complexity["execution_group"], "regular", "the target failure must not be Primary protection");
            assert_eq!(complexity["selection_reason"], "simple_task");
            assert_eq!(complexity["simple_probability"], 0.9);
            assert_eq!(complexity["routing_observation"]["score"], 0.9);
            assert!(complexity["competence_trigger"].is_null());
            assert_ne!(complexity["routing_observation"]["protection_action"], "applied");
            if response.status != 200 {
                assert!(facts.iter().any(|fact| fact["correlation"]["request_id"] == decision["correlation"]["request_id"]
                    && fact["fact"]["kind"] == "candidate_decision"
                    && fact["fact"]["upstream_protocol"] == "chat_completions"
                    && fact["fact"]["exclusion_reason"] == "PROTOCOL_PATH_UNAVAILABLE"),
                    "the old candidate must fail at Chat history projection");
            }
        }
        assert_eq!(response.status, 200, "{}", String::from_utf8_lossy(&response.body));
        serde_json::from_slice::<Value>(&response.body).unwrap()
    };
    // Exceeds the production Replay memory threshold, forcing external strings.
    let first_user = "starting context ".repeat(8192);
    let mut input = vec![json!({"role":"user","content":first_user})];
    let first = send(&input);
    assert_eq!((regular.calls(), primary.calls(), decider.calls()), (1, 0, 1));
    input.extend(first["output"].as_array().unwrap().iter().cloned());
    input.push(json!({"role":"user","content":"Check both records with tools."}));
    let tools = send(&input);
    assert_eq!((regular.calls(), primary.calls(), decider.calls()), (1, 1, 2));
    assert_eq!(tools["output"][0]["phase"], "commentary");
    assert_eq!(tools["output"][0]["id"], "msg-commentary");
    input.extend(tools["output"].as_array().unwrap().iter().cloned());
    for (id, output) in [("lookup-call", "alpha=valid"), ("check-call", "beta=valid")] {
        input.push(json!({"type":"function_call_output","call_id":id,"output":output}));
    }
    let final_answer = send(&input);
    assert_eq!((regular.calls(), primary.calls(), decider.calls()), (1, 2, 2));
    let primary_request: Value = serde_json::from_slice(http_body(&primary.requests()[1])).unwrap();
    assert_eq!(primary_request["input"], json!(input), "native replay keeps phases and identities");
    input.extend(final_answer["output"].as_array().unwrap().iter().cloned());
    input.push(json!({"role":"user","content":"Now answer a simple question."}));
    let original = input.clone();
    let last = send(&input);
    assert_eq!(last["output"][0]["content"][0]["text"], "regular answer");
    assert_eq!((regular.calls(), primary.calls(), decider.calls()), (2, 2, 3),
        "Regular must execute again; Primary success cannot substitute for this assertion");
    assert_eq!(input, original);
    let chat: Value = serde_json::from_slice(http_body(&regular.requests()[1])).unwrap();
    let messages = chat["messages"].as_array().unwrap();
    assert_eq!(messages[0]["content"], first_user);
    assert_eq!(messages[3], json!({"role":"assistant","content":"I will check the record."}));
    assert_eq!(messages[4], json!({"role":"assistant","tool_calls":[
        {"id":"lookup-call","type":"function","function":{"name":"lookup","arguments":"{\"key\":\"alpha\"}"}},
        {"id":"check-call","type":"function","function":{"name":"check","arguments":"{\"key\":\"beta\"}"}}
    ]}));
    assert_eq!(messages[5], json!({"role":"tool","tool_call_id":"lookup-call","content":"alpha=valid"}));
    assert_eq!(messages[6], json!({"role":"tool","tool_call_id":"check-call","content":"beta=valid"}));
    assert_eq!(messages[7], json!({"role":"assistant","content":"Both records are valid."}));
    assert_eq!(messages[8], json!({"role":"user","content":"Now answer a simple question."}));
    let facts = wait_execution_facts(&fixture, 4);
    let decisions = facts.iter().filter(|f| f["fact"]["kind"] == "route_decision").collect::<Vec<_>>();
    assert_eq!(decisions.len(), 4);
    assert!(decisions.iter().all(|f| f["fact"]["complexity"]["fallback_used"] == false));
    let tools_membership = json!([decisions[1]["correlation"]["request_id"], decisions[2]["correlation"]["request_id"]]);
    let assessment = facts.iter().find(|f| f["fact"]["kind"] == "branch_assessment_recorded"
        && f["correlation"]["request_id"] == decisions[3]["correlation"]["request_id"])
        .expect("the new Regular request assesses the Primary tool stage");
    assert_eq!(assessment["fact"]["history_membership"]["request_ids"], tools_membership);
    assert_eq!(assessment["fact"]["partial"], false);
}

fn http_body(request: &[u8]) -> &[u8] {
    let offset = request.windows(4).position(|bytes| bytes == b"\r\n\r\n").unwrap() + 4;
    &request[offset..]
}
