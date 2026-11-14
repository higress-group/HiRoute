use super::native_history::{classifier_body, reply};
use super::*;
use hiroute_domain::{CanonicalDigest, STANDARD_RESPONSES_JSON_ADAPTER_V1, UpstreamProtocol};
use serde_json::{Value, json};

const TOOL: &[u8] = br#"{"id":"resp_tool","model":"runtime-native","status":"completed","output":[{"type":"function_call","id":"fc_lookup","call_id":"lookup-call","namespace":"records","name":"lookup","arguments":"{}","status":"completed"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;

#[test]
fn leading_declarations_use_candidate_dialect_through_tool_result() {
    let standard = NativeProvider::start(vec![reply(TOOL), reply(RESPONSES_OK)]);
    let native = NativeProvider::start(vec![reply(RESPONSES_OK)]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 4]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&standard, &native, &classifier],
        2,
    );
    select_standard_responses(&fixture);

    let baseline = json!({
        "type":"function","name":"baseline","strict":true,
        "parameters":{"type":"object","properties":{"query":{"type":"string",
            "enum":["existing","archived"]}},"required":["query"],"additionalProperties":false}
    });
    let declaration = json!({
        "type":"additional_tools","id":"at_codex","role":"developer","tools":[
            baseline.clone(), {"type":"namespace","name":"records","description":"Record tools","tools":[{
                "type":"function","name":"lookup","strict":true,
                "parameters":{"type":"object","properties":{"query":{
                    "type":"string","encrypted":true,"description":"Quoted \"schema\" 中文 ".repeat(1600),
                    "enum":["existing","archived"], "default":"existing"
                }}}
            }]}
        ]
    });
    let mut expected_namespace = declaration["tools"][1].clone();
    expected_namespace["tools"][0]["parameters"]["properties"]["query"]
        .as_object_mut()
        .unwrap()
        .remove("encrypted");
    let mut different_baseline = baseline.clone();
    different_baseline["parameters"]["properties"]["query"]["enum"] = json!(["other"]);
    let next_declaration = json!({"type":"additional_tools","id":null,"role":"developer",
        "tools":[different_baseline.clone(), declaration["tools"][1]]});
    let user = json!({"type":"message","role":"user","content":"Find the record."});
    let mut body = json!({"model":MODEL,"stream":false,"store":false,"parallel_tool_calls":true,
        "tool_choice":{"type":"function","name":"baseline"},"tools":[baseline.clone()],
        "input":[declaration.clone(),next_declaration.clone(),user.clone()]});
    let response = send(&fixture, &body);
    assert_eq!(response.status, 200);
    let output: Value = serde_json::from_slice(&response.body).unwrap();
    let call = &output["output"][0];
    assert_eq!(call["type"], "function_call");
    assert_eq!(call["encrypted_function_args"], json!([]));
    let receipt =
        json!({"type":"function_call_output","call_id":call["call_id"],"output":"SYNTHETIC_485"});
    body["input"] = json!([declaration, next_declaration, user, call, receipt]);
    let response = send(&fixture, &body);
    assert_eq!(response.status, 200);
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap()["output"][0]["content"][0]["text"],
        "ok"
    );
    assert_eq!((standard.calls(), native.calls()), (2, 0));

    let tools = json!([
        baseline,
        baseline,
        expected_namespace,
        different_baseline,
        expected_namespace
    ]);
    for attempt in 0..2 {
        let wire = classifier_body(&standard, attempt);
        assert_eq!(wire["tools"], tools);
        assert_eq!(wire["tool_choice"], body["tool_choice"]);
        assert_eq!(wire["parallel_tool_calls"], true);
        let mut expected_input = vec![user.clone()];
        if attempt == 1 {
            let mut projected_call = call.clone();
            projected_call
                .as_object_mut()
                .unwrap()
                .remove("encrypted_function_args");
            expected_input.extend([projected_call, receipt.clone()]);
        }
        assert_eq!(wire["input"], json!(expected_input));
    }
}

#[test]
fn positioned_declarations_skip_standard_candidate_and_reach_native_in_order() {
    let standard = NativeProvider::start(vec![]);
    let native = NativeProvider::start(vec![reply(RESPONSES_OK)]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&standard, &native, &classifier],
        2,
    );
    select_standard_responses(&fixture);
    let namespace = json!({"type":"namespace","name":"records","description":null,"tools":[{
        "type":"function","name":"lookup","description":null,"strict":null,
        "parameters":{"type":"object","properties":{"query":{
            "type":"string","enum":["existing","archived"],"default":"existing"
        }},"required":["query"]}
    }]});
    let input = json!([
        {"role":"user","content":"Earlier conversation."},
        {"type":"additional_tools","id":"at_positioned","role":"developer","tools":[namespace.clone(), namespace.clone()]},
        {"role":"user","content":"Find the record."}
    ]);
    let body = json!({"model":MODEL,"input":input,"stream":false,"tools":[namespace]});
    let mut malformed = body.clone();
    malformed["input"][1]["role"] = json!("user");
    let rejected = send(&fixture, &malformed);
    assert_eq!(rejected.status, 400);
    let error: Value = serde_json::from_slice(&rejected.body).unwrap();
    assert_eq!(error["phase"], "canonical_request");
    assert_eq!(error["code"], "PROTOCOL_INVALID_PAYLOAD");
    assert_eq!(
        (standard.calls(), native.calls(), classifier.calls()),
        (0, 0, 0)
    );

    let response = send(&fixture, &body);
    assert_eq!(response.status, 200);
    assert_eq!((standard.calls(), native.calls()), (0, 1));
    let wire = classifier_body(&native, 0);
    assert_eq!(wire["input"], body["input"]);
    assert_eq!(wire["tools"], body["tools"]);
}

fn select_standard_responses(fixture: &RuntimeFixture) {
    let mut publication = fixture.publication();
    let candidate = &mut publication.aliases[0].candidates[0];
    for profile in &mut candidate.protocol_profiles {
        if profile.ingress_protocol == UpstreamProtocol::Responses {
            profile.adapter_revision = STANDARD_RESPONSES_JSON_ADAPTER_V1.into();
        }
    }
    candidate.protocol_profile_digest = CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
}

fn send(fixture: &RuntimeFixture, body: &Value) -> WireResponse {
    request(
        fixture.address,
        "POST",
        "/v1/responses",
        &[
            ("X-HiRoute-Token", TOKEN),
            ("session-id", "codex-485-regression"),
        ],
        &serde_json::to_vec(body).unwrap(),
    )
}
