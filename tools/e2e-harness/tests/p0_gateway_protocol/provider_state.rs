//! Native Responses continuation through the real hirouted entry and two candidates.
use super::*;
use hiroute_gateway::server::core_runtime::profiles::{
    CandidateProtocolProfile, Fidelity, NativeProviderStateEmission, StateAffinity,
};

const FIRST: &[u8] = br#"event: response.created
data: {"type":"response.created","response":{"id":"native-first","model":"continuation-native-2","status":"in_progress","service_tier":"auto"}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_native","summary":[],"encrypted_content":"fixture-opaque-first"}}

event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs_native","status":"completed","summary":[],"encrypted_content":"fixture-opaque-final"}}

event: response.completed
data: {"type":"response.completed","response":{"id":"native-first","model":"continuation-native-2","status":"completed","service_tier":"default","metadata":{"source":"fixture"},"output":[{"type":"reasoning","id":"rs_native","status":"completed","summary":[],"encrypted_content":"fixture-opaque-final"},{"type":"message","id":"msg_native","role":"assistant","status":"completed","content":[{"type":"output_text","text":"first answer","annotations":[]}]}],"usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}}

"#;

#[test]
fn production_responses_continuation_keeps_actual_owner_in_multi_candidate_route() {
    production_continuation(IngressProtocol::Responses, IngressProtocol::Responses);
}

#[test]
fn production_messages_signature_keeps_responses_owner_in_multi_candidate_route() {
    production_continuation(IngressProtocol::Messages, IngressProtocol::Responses);
}

#[test]
fn production_native_messages_fragmented_signature_and_tool_result_keep_actual_owner() {
    production_continuation(IngressProtocol::Messages, IngressProtocol::Messages);
}

const FIRST_MESSAGES: &[u8] = br#"event: message_start
data: {"type":"message_start","message":{"id":"native-msg","model":"continuation-native-2","role":"assistant","content":[],"usage":{"input_tokens":4,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"fixture-"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"opaque-final"}}

event: content_block_stop
data: { "index": 0,
data: "type": "content_block_stop" }

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"physical-tool","name":"Bash","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"pwd\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":8}}

event: message_stop
data: {"type":"message_stop"}

"#;

const SECOND_MESSAGES: &[u8] = br#"{"id":"native-done","model":"continuation-native-2","type":"message","role":"assistant","content":[{"type":"text","text":"done"}],"stop_reason":"end_turn","usage":{"input_tokens":8,"output_tokens":1}}"#;

fn production_continuation(ingress: IngressProtocol, upstream: IngressProtocol) {
    let _serial = process_test_lock();
    let directory = tempfile::tempdir().unwrap();
    let provider = ContinuationProvider::start(vec![
        ProviderReply::Sse(if upstream == IngressProtocol::Messages {
            FIRST_MESSAGES
        } else {
            FIRST
        }),
        ProviderReply::Json(if upstream == IngressProtocol::Messages {
            SECOND_MESSAGES
        } else {
            SECOND_PROVIDER_JSON
        }),
    ]);
    let forbidden = ContinuationProvider::start(Vec::new());
    let mut publication = snapshot(
        provider.authority(),
        forbidden.authority(),
        forbidden.authority(),
    );
    publication.aliases[0].candidates.remove(0); // Two different native Responses models.
    if upstream == IngressProtocol::Messages {
        publication.aliases[0].candidates = vec![
            sealed_native_candidate(
                2,
                "continuation-target",
                &["continuation-credential".into()],
                provider.authority(),
                "continuation-native-2",
                &[(ingress, upstream)],
            ),
            sealed_native_candidate(
                3,
                "forbidden-fallback-target",
                &["forbidden-credential".into()],
                forbidden.authority(),
                "continuation-native-3",
                &[(ingress, upstream)],
            ),
        ];
    }
    publication.aliases[0].protocols = vec![ingress];
    for grant in &mut publication.grants {
        grant.protocol = ingress;
        if ingress == IngressProtocol::Messages {
            grant.routes.retain(|alias, _| alias == "continuation");
        }
    }
    for binding in &mut publication.aliases[0].candidates {
        for profile in &mut binding.protocol_profiles {
            let mut native: CandidateProtocolProfile =
                serde_json::from_value(serde_json::to_value(&*profile).unwrap()).unwrap();
            native.ingress_protocol = ingress;
            native.capability.native_provider_state = NativeProviderStateEmission::ExactOwnerAffine;
            native.capability.request.provider_state = Fidelity::Exact;
            native.capability.request.state_affinity = StateAffinity::ExactOwner;
            native.capability.response.provider_state = Fidelity::Exact;
            native.capability.response.state_affinity = StateAffinity::ExactOwner;
            *profile = serde_json::from_value(serde_json::to_value(native).unwrap()).unwrap();
        }
        binding.protocol_profile_digest =
            hiroute_domain::CanonicalDigest::of(&binding.protocol_profiles).unwrap();
    }
    publication.payload_digest = publication.canonical_digest().unwrap();
    publication.validate().unwrap();
    let publication_path = directory.path().join("publication.json");
    std::fs::write(&publication_path, serde_json::to_vec(&publication).unwrap()).unwrap();
    write_dial_config(
        directory.path(),
        &[provider.transport(), forbidden.transport()],
    )
    .unwrap();
    let mut credentials = serde_json::Map::new();
    for name in ["continuation", "forbidden", "rejecting"] {
        let credential_ref = format!("{name}-credential");
        let file = format!("{credential_ref}.json");
        credentials.insert(credential_ref.clone(), json!(file));
        std::fs::write(directory.path().join(file), serde_json::to_vec(&json!({
            "schema_version":"hiroute.gateway.credential-leases/v1", "credential_ref":credential_ref,
            "keys":[{"key_id":format!("{name}-key"),"generation":1,"authorization":"Bearer fixture"}]
        })).unwrap()).unwrap();
    }
    let credentials_path = directory.path().join("credentials.json");
    std::fs::write(
        &credentials_path,
        serde_json::to_vec(
            &json!({"schema_version":"hiroute.gateway.credentials/v1","credentials":credentials}),
        )
        .unwrap(),
    )
    .unwrap();
    let address = reserve_address();
    let mut process = Hirouted::spawn(
        &exact_hirouted_binary(),
        address,
        &directory.path().join("lkg.json"),
        &publication_path,
        &credentials_path,
        directory.path(),
    );
    process.wait_ready();
    if ingress == IngressProtocol::Messages {
        assert_messages_continuation(address, &provider, &forbidden, upstream);
        process.stop();
        return;
    }
    let initial = json!({"type":"message","role":"user","content":"first"});
    let metadata = json!({"session_id":"native-continuation-fixture"});
    let first = request(
        address,
        "continuation-token",
        &json!({"model":"continuation","stream":true,"input":[initial.clone()],"client_metadata":metadata}),
    );
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let events = decode_downstream_sse(&first.body);
    let completed: Vec<_> = events
        .iter()
        .filter(|(name, _)| name == "response.completed")
        .collect();
    assert_eq!(completed.len(), 1, "events={events:#?}");
    let response = &completed[0].1["response"];
    assert_eq!(response["service_tier"], "default");
    assert_eq!(response["metadata"], json!({"source":"fixture"}));
    assert_eq!(response["model"], "continuation");
    let mut input = response["output"].as_array().unwrap().clone();
    assert_eq!(input[0]["encrypted_content"], "fixture-opaque-final");
    input.insert(0, initial);
    input.push(json!({"type":"message","role":"user","content":"continue"}));
    let next =
        json!({"model":"continuation","stream":false,"input":input,"client_metadata":metadata});
    let second = request(address, "continuation-token", &next);
    assert_eq!(
        second.status,
        200,
        "{}",
        String::from_utf8_lossy(&second.body)
    );
    wait_for_calls(&provider, 2);
    let bodies = provider.requests();
    let forwarded: Value = serde_json::from_slice(http_body(&bodies[1])).unwrap();
    assert_eq!(forwarded["model"], "continuation-native-2");
    assert_eq!(
        forwarded["input"][1]["encrypted_content"],
        "fixture-opaque-final"
    );
    assert_eq!(forwarded["input"][1]["status"], "completed");
    assert_eq!(
        forwarded["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .count(),
        1
    );
    for (token, document) in [
        ("other-token", next.clone()),
        ("continuation-token", {
            let mut altered = next.clone();
            altered["input"][1]["encrypted_content"] = json!("altered");
            altered
        }),
    ] {
        let rejected = request(address, token, &document);
        assert_eq!(rejected.status, 400);
        assert_eq!(
            serde_json::from_slice::<Value>(&rejected.body).unwrap()["code"],
            "PROVIDER_STATE_CONTINUATION_UNAVAILABLE"
        );
    }
    assert_eq!(forbidden.connections(), 0);
    assert_eq!(provider.calls(), 2);
    process.stop();
}

fn assert_messages_continuation(
    address: SocketAddr,
    provider: &ContinuationProvider,
    forbidden: &ContinuationProvider,
    upstream: IngressProtocol,
) {
    let initial = json!({"role":"user","content":"first"});
    let first = request_at_path(
        address,
        "continuation-token",
        &json!({
            "model":"continuation","stream":true,"max_tokens":256,"messages":[initial.clone()],
            "tools":[{"name":"Bash","input_schema":{"type":"object","properties":{"command":{"type":"string"}}}}]
        }),
        "/v1/messages",
    );
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let events = decode_downstream_sse(&first.body);
    let signatures: Vec<_> = events
        .iter()
        .filter_map(|(_, value)| value["delta"]["signature"].as_str())
        .collect();
    assert_eq!(signatures.concat(), "fixture-opaque-final", "{events:#?}");
    let mut next = json!({"model":"continuation","stream":false,"max_tokens":256,"messages":[
        initial,
        {"role":"assistant","content":[{"type":"thinking","thinking":"","signature":signatures.concat()}]},
        {"role":"user","content":"continue"}
    ]});
    if upstream == IngressProtocol::Messages {
        let tool = events
            .iter()
            .find_map(|(_, event)| {
                let block = &event["content_block"];
                (block["type"] == "tool_use").then_some(block)
            })
            .expect("delivered tool use");
        next["messages"][1]["content"]
            .as_array_mut()
            .unwrap()
            .push(tool.clone());
        next["messages"][2]["content"] =
            json!([{"type":"tool_result","tool_use_id":tool["id"],"content":"/work"}]);
    }
    let second = request_at_path(address, "continuation-token", &next, "/v1/messages");
    assert_eq!(
        second.status,
        200,
        "{}",
        String::from_utf8_lossy(&second.body)
    );
    wait_for_calls(provider, 2);
    let bodies = provider.requests();
    let forwarded: Value = serde_json::from_slice(http_body(&bodies[1])).unwrap();
    assert_eq!(forwarded["model"], "continuation-native-2");
    if upstream == IngressProtocol::Messages {
        assert_eq!(
            forwarded["messages"][1]["content"][0]["signature"],
            "fixture-opaque-final"
        );
        assert_eq!(
            forwarded["messages"][2]["content"][0]["tool_use_id"],
            "physical-tool"
        );
    } else {
        assert!(
            forwarded["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "reasoning"
                    && item["encrypted_content"] == "fixture-opaque-final")
        );
    }
    assert_eq!(forbidden.connections(), 0);
    let mut altered = next;
    altered["messages"][1]["content"][0]["signature"] = json!("unknown");
    let rejected = request_at_path(address, "continuation-token", &altered, "/v1/messages");
    assert_eq!(rejected.status, 400);
    assert_eq!(provider.calls(), 2);
}
