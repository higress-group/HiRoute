use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hiroute_e2e::gateway_fixture::{
    TestTlsListener, TestTlsStream, sealed_native_candidate, write_dial_config,
};
use hiroute_gateway::server::core_runtime::adapters::{
    decode_ingress_request, project_candidate_request,
};
use hiroute_gateway::server::publication::{
    AliasComplexityClassifierV1, AliasGroupIdV1, AliasModelGroupV1, AliasPlanV1,
    AliasRequestOwnedRouteV1, AliasRoutingV1, GatewayPublicationSnapshotV3, GrantV1, token_sha256,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

use super::process::{
    Hirouted, WireResponse, drain_provider_request, exact_hirouted_binary, process_test_lock,
    read_provider_request_head, read_response, reserve_address,
};
use super::support::{PROTOCOLS, candidate_profile, request_fixture};

#[path = "provider_state.rs"]
mod provider_state;
#[path = "responses_chat_tools.rs"]
mod responses_chat_tools;

const FIRST_PROVIDER_STREAM: &[u8] = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"provider-response-7\",\"model\":\"provider-native\"}}\n\nevent: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_native_7\",\"call_id\":\"provider-weather-7\",\"namespace\":\"weather-services\",\"name\":\"weather\",\"arguments\":\"\",\"status\":\"in_progress\"}}\n\nevent: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_native_7\",\"output_index\":0,\"delta\":\"{\\\"city\\\":\\\"Paris\\\"}\"}\n\nevent: response.function_call_arguments.done\ndata: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_native_7\",\"output_index\":0,\"arguments\":\"{\\\"city\\\":\\\"Paris\\\"}\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"provider-response-7\",\"model\":\"provider-native\",\"status\":\"completed\",\"usage\":{\"input_tokens\":4,\"output_tokens\":3,\"total_tokens\":7}}}\n\n";

const SECOND_PROVIDER_JSON: &[u8] = br#"{"id":"provider-continuation-done","model":"provider-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"sunny"}]}],"usage":{"input_tokens":8,"output_tokens":1,"total_tokens":9}}"#;
const NAMESPACE_ONLY_PROVIDER_JSON: &[u8] = br#"{"id":"provider-namespace-only","model":"provider-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"namespace accepted"}]}],"usage":{"input_tokens":6,"output_tokens":2,"total_tokens":8}}"#;
const NAMESPACE_REJECTED: &[u8] =
    br#"{"error":{"type":"invalid_request_error","message":"namespace unsupported"}}"#;

/// Deliberately split headers, event names, JSON punctuation, and terminal
/// usage across writes. The production listener must reconstruct the native
/// stream rather than observing one fixture-sized body write.
const SSE_FRAGMENT_SIZES: &[usize] = &[1, 2, 7, 3, 11, 1, 17, 5, 2, 29, 13];

#[test]
fn production_native_tool_history_survives_restart_with_request_authentication() {
    let mut receipt = hiroute_e2e::p0_runtime_execution_receipt!("protocol.tool_and_sse");
    let _serial = process_test_lock();
    let directory = tempfile::tempdir().unwrap();
    let provider = ContinuationProvider::start(vec![
        ProviderReply::Sse(FIRST_PROVIDER_STREAM),
        ProviderReply::Json(SECOND_PROVIDER_JSON),
        ProviderReply::Json(NAMESPACE_ONLY_PROVIDER_JSON),
        ProviderReply::Json(SECOND_PROVIDER_JSON),
    ]);
    let rejecting_provider = ContinuationProvider::start(vec![ProviderReply::JsonStatus {
        status: 400,
        body: NAMESPACE_REJECTED,
    }]);
    let forbidden_provider = ContinuationProvider::start(Vec::new());
    let publication_path = directory.path().join("publication.json");
    let credentials_path = directory.path().join("credentials.json");
    let lkg_path = directory.path().join("publication-lkg.json");
    std::fs::write(
        &publication_path,
        serde_json::to_vec_pretty(&snapshot(
            provider.authority(),
            rejecting_provider.authority(),
            forbidden_provider.authority(),
        ))
        .unwrap(),
    )
    .unwrap();
    write_dial_config(
        directory.path(),
        &[
            provider.transport(),
            rejecting_provider.transport(),
            forbidden_provider.transport(),
        ],
    )
    .unwrap();
    std::fs::write(
        directory.path().join("forbidden-credential.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credential-leases/v1",
            "credential_ref": "forbidden-credential",
            "keys": [{
                "key_id": "forbidden-key",
                "generation": 1,
                "authorization": "Bearer forbidden-provider-secret"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("continuation-credential.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credential-leases/v1",
            "credential_ref": "continuation-credential",
            "keys": [{
                "key_id": "continuation-key",
                "generation": 1,
                "authorization": "Bearer continuation-provider-secret"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("rejecting-credential.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credential-leases/v1",
            "credential_ref": "rejecting-credential",
            "keys": [{
                "key_id": "rejecting-key",
                "generation": 1,
                "authorization": "Bearer rejecting-provider-secret"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        &credentials_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credentials/v1",
            "credentials": {
                "continuation-credential": "continuation-credential.json",
                "rejecting-credential": "rejecting-credential.json",
                "forbidden-credential": "forbidden-credential.json"
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let address = reserve_address();
    let replay_root = directory.path().join("replay");
    let replay_root = replay_root.to_str().unwrap().to_owned();
    let mut process = Hirouted::spawn_with_environment(
        &exact_hirouted_binary(),
        address,
        &lkg_path,
        &publication_path,
        &credentials_path,
        directory.path(),
        &[
            ("HIROUTE_REPLAY_ROOT", replay_root.as_str()),
            ("HIROUTE_REPLAY_MEMORY_THRESHOLD", "1024"),
            ("HIROUTE_REPLAY_RECORD_BYTES", "256"),
        ],
    );
    process.wait_ready();

    let first = request(
        address,
        "continuation-token",
        &namespace_request("continuation", true, "What is the weather?"),
    );
    assert_eq!(
        first.status,
        200,
        "body={} primary={} forbidden={}",
        String::from_utf8_lossy(&first.body),
        provider.calls(),
        forbidden_provider.calls()
    );
    wait_for_calls(&provider, 1);
    let logical_id = assert_fragmented_tool_stream(&first.body, provider.sse_fragments());
    receipt.mark_assertion("protocol.sse_fragmented_semantics");
    receipt.mark_assertion("protocol.stream_usage_terminal");
    assert_eq!(forbidden_provider.calls(), 0);
    assert_eq!(forbidden_provider.connections(), 0);
    assert_eq!(rejecting_provider.calls(), 0);
    assert_eq!(rejecting_provider.connections(), 0);

    let large_arguments = format!("{{\"city\":\"{}\"}}", "Paris".repeat(8_192));
    let large_result = "sunny-result-".repeat(4_096);
    let continuation =
        continuation_request("continuation", &logical_id, &large_arguments, &large_result);
    let second = request(address, "continuation-token", &continuation);
    assert_eq!(
        second.status,
        200,
        "{}",
        String::from_utf8_lossy(&second.body)
    );
    wait_for_calls(&provider, 2);
    assert_eq!(forbidden_provider.calls(), 0);
    assert_eq!(forbidden_provider.connections(), 0);
    assert_eq!(rejecting_provider.calls(), 0);
    assert_eq!(rejecting_provider.connections(), 0);
    let attempts = provider.requests();
    assert_eq!(attempts.len(), 2);
    let projected: Value = serde_json::from_slice(http_body(&attempts[1])).unwrap();
    let projected_ids = projected["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["call_id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(projected_ids, ["provider-weather-7", "provider-weather-7"]);
    assert_eq!(projected["input"][0]["namespace"], "weather-services");
    assert_eq!(projected["input"][0]["arguments"], large_arguments);
    assert_eq!(projected["input"][1]["output"], large_result);
    assert_eq!(logical_id, "provider-weather-7");
    receipt.mark_assertion("protocol.native_tool_identity");
    receipt.mark_assertion("protocol.tool_round_trip");

    let namespace_only = request(
        address,
        "continuation-token",
        &namespace_only_request("continuation"),
    );
    assert_eq!(
        namespace_only.status,
        200,
        "{}",
        String::from_utf8_lossy(&namespace_only.body)
    );
    wait_for_calls(&provider, 3);
    let namespace_only_requests = provider.requests();
    let namespace_only_projected: Value =
        serde_json::from_slice(http_body(&namespace_only_requests[2])).unwrap();
    assert_eq!(
        namespace_only_projected["tools"].as_array().unwrap().len(),
        1
    );
    assert_eq!(namespace_only_projected["tools"][0]["type"], "namespace");
    assert_eq!(
        namespace_only_projected["tools"][0]["name"],
        "lookup-stack-17"
    );
    assert_eq!(
        namespace_only_projected["tools"][0]["tools"][0]["strict"],
        true
    );
    assert_eq!(
        namespace_only_projected["tool_choice"],
        json!({"type":"function","name":"weather"})
    );
    receipt.mark_assertion("protocol.namespace_native_acceptance");

    let rejected = request(
        address,
        "continuation-token",
        &namespace_request(
            "namespace-rejected",
            false,
            "Use the same legal namespace shape",
        ),
    );
    assert_eq!(
        rejected.status,
        400,
        "{}",
        String::from_utf8_lossy(&rejected.body)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&rejected.body).unwrap(),
        json!({"error":{"type":"upstream_error","code":"UPSTREAM_ATTEMPT_FAILED"}})
    );
    wait_for_calls(&rejecting_provider, 1);
    assert_eq!(provider.calls(), 3);
    assert_eq!(forbidden_provider.calls(), 0);
    assert_eq!(forbidden_provider.connections(), 0);
    let rejected_request = rejecting_provider.requests();
    assert_eq!(rejected_request.len(), 1);
    let rejected_document: Value = serde_json::from_slice(http_body(&rejected_request[0])).unwrap();
    assert_eq!(rejected_document["tools"][1]["type"], "namespace");
    assert_eq!(rejected_document["tools"][1]["name"], "weather-services");
    assert_eq!(rejected_document["tools"][2]["name"], "archive-services");
    receipt.mark_assertion("protocol.namespace_upstream_rejection");

    let unauthorized = request(address, "invalid-token", &continuation);
    assert_eq!(unauthorized.status, 401);
    assert_eq!(provider.calls(), 3);
    assert_eq!(forbidden_provider.calls(), 0);
    receipt.mark_assertion("protocol.continuation_request_authentication");

    process.stop();
    let mut restarted = Hirouted::spawn_with_environment(
        &exact_hirouted_binary(),
        address,
        &lkg_path,
        &publication_path,
        &credentials_path,
        directory.path(),
        &[
            ("HIROUTE_REPLAY_ROOT", replay_root.as_str()),
            ("HIROUTE_REPLAY_MEMORY_THRESHOLD", "1024"),
            ("HIROUTE_REPLAY_RECORD_BYTES", "256"),
        ],
    );
    restarted.wait_ready();
    let mut repeated = continuation.clone();
    repeated["input"].as_array_mut().unwrap().extend([
        json!({"type":"function_call","call_id":logical_id,"namespace":"weather-services","name":"weather","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":logical_id,"output":"second occurrence"}),
    ]);
    let resumed = request(address, "continuation-token", &repeated);
    assert_eq!(
        resumed.status,
        200,
        "{}",
        String::from_utf8_lossy(&resumed.body)
    );
    wait_for_calls(&provider, 4);
    let resumed_requests = provider.requests();
    let resumed_body: Value = serde_json::from_slice(http_body(&resumed_requests[3])).unwrap();
    assert_eq!(resumed_body["input"][0]["call_id"], "provider-weather-7");
    assert_eq!(resumed_body["input"][1]["call_id"], "provider-weather-7");
    assert_eq!(resumed_body["input"][1]["output"], large_result);
    assert_eq!(resumed_body["input"][2]["call_id"], "provider-weather-7");
    assert_eq!(resumed_body["input"][3]["output"], "second occurrence");
    restarted.stop();
    receipt.mark_assertion("protocol.continuation_restart_native_history");
    receipt.finish();
}

#[test]
fn continuation_binding_projects_exact_native_id_across_three_by_three_corpus() {
    for ingress in PROTOCOLS {
        for upstream in PROTOCOLS {
            let document = request_fixture(ingress);
            let request = decode_ingress_request(ingress, &document).unwrap();
            let projected =
                project_candidate_request(&request, &candidate_profile(ingress, upstream)).unwrap();
            assert!(
                String::from_utf8_lossy(&projected.bytes).contains("call_weather"),
                "{ingress:?} -> {upstream:?}"
            );
        }
    }
}

fn namespace_request(model: &str, stream: bool, input: &str) -> Value {
    json!({
        "model": model,
        "stream": stream,
        "input": input,
        "parallel_tool_calls": false,
        "tool_choice": "auto",
        "tools": [
            {
                "type": "function",
                "name": "flat_lookup",
                "description": "A portable flat Tool",
                "parameters": {"type":"object","properties":{"key":{"type":"string"}}}
            },
            {
                "type": "namespace",
                "name": "weather-services",
                "description": "Live weather Tools",
                "tools": [{
                    "type": "function",
                    "name": "weather",
                    "description": "Read weather",
                    "parameters": {"type":"object","properties":{"city":{"type":"string"}},"required":["city"]},
                    "strict": true
                }]
            },
            {
                "type": "namespace",
                "name": "archive-services",
                "tools": [{
                    "type": "function",
                    "name": "weather",
                    "parameters": {"type":"object","properties":{"city":{"type":"string"}}}
                }]
            }
        ]
    })
}

fn namespace_only_request(model: &str) -> Value {
    json!({
        "model": model,
        "stream": false,
        "input": "Use one namespace",
        "parallel_tool_calls": false,
        "tool_choice": {"type":"function","name":"weather"},
        "tools": [{
            "type": "namespace",
            "name": "lookup-stack-17",
            "description": "An arbitrary caller-owned group name",
            "tools": [{
                "type": "function",
                "name": "weather",
                "parameters": {"type":"object","properties":{"city":{"type":"string"}},"required":["city"]},
                "strict": true
            }]
        }]
    })
}

fn continuation_request(model: &str, logical_id: &str, arguments: &str, output: &str) -> Value {
    json!({
        "model": model,
        "stream": false,
        "input": [
            {
                "type":"function_call",
                "call_id":logical_id,
                "namespace":"weather-services",
                "name":"weather",
                "arguments":arguments
            },
            {
                "type":"function_call_output",
                "call_id":logical_id,
                "output":output
            }
        ],
        "parallel_tool_calls": false,
        "tool_choice": "auto",
        "tools": namespace_request(model, false, "unused")["tools"].clone()
    })
}

fn assert_fragmented_tool_stream(body: &[u8], fragments_written: usize) -> String {
    assert_eq!(
        fragments_written,
        expected_sse_fragment_count(FIRST_PROVIDER_STREAM),
        "provider must send the native SSE across the frozen adversarial boundaries"
    );
    let events = decode_downstream_sse(body);
    assert_eq!(
        events.len(),
        5,
        "stream must contain every native semantic event"
    );
    let logical_id = events[1]
        .1
        .pointer("/item/call_id")
        .and_then(Value::as_str)
        .expect("downstream Tool-call logical ID")
        .to_owned();
    assert_eq!(logical_id, "provider-weather-7");
    assert_eq!(
        events,
        vec![
            (
                "response.created".into(),
                json!({"type":"response.created","response":{"id":"provider-response-7","model":"continuation"}}),
            ),
            (
                "response.output_item.added".into(),
                json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_native_7","call_id":logical_id,"namespace":"weather-services","name":"weather","arguments":"","status":"in_progress"}}),
            ),
            (
                "response.function_call_arguments.delta".into(),
                json!({"type":"response.function_call_arguments.delta","item_id":"fc_native_7","output_index":0,"delta":"{\"city\":\"Paris\"}"}),
            ),
            (
                "response.function_call_arguments.done".into(),
                json!({"type":"response.function_call_arguments.done","item_id":"fc_native_7","output_index":0,"arguments":"{\"city\":\"Paris\"}"}),
            ),
            (
                "response.completed".into(),
                json!({"type":"response.completed","response":{"id":"provider-response-7","model":"continuation","status":"completed","usage":{"input_tokens":4,"output_tokens":3,"total_tokens":7}}}),
            ),
        ],
        "fragmented native stream must preserve the exact provider event sequence and terminal usage"
    );
    logical_id
}

fn decode_downstream_sse(body: &[u8]) -> Vec<(String, Value)> {
    std::str::from_utf8(body)
        .expect("downstream SSE is UTF-8")
        .split("\n\n")
        .filter(|event| !event.is_empty())
        .map(|event| {
            let mut lines = event.lines();
            let name = lines
                .next()
                .and_then(|line| line.strip_prefix("event: "))
                .expect("SSE event name");
            let data = lines
                .map(|line| line.strip_prefix("data: ").expect("SSE JSON data line"))
                .collect::<Vec<_>>()
                .join("\n");
            let value: Value = serde_json::from_str(&data).expect("SSE JSON data");
            assert_eq!(value["type"], name, "SSE name and JSON type agree");
            (name.to_owned(), value)
        })
        .collect()
}

fn request(address: SocketAddr, token: &str, document: &Value) -> WireResponse {
    request_at_path(address, token, document, "/v1/responses")
}

fn request_at_path(address: SocketAddr, token: &str, document: &Value, path: &str) -> WireResponse {
    let body = serialize_document_with_model_first(document);
    let auth = if path == "/v1/messages" {
        format!("Authorization: Bearer {token}")
    } else {
        format!("X-HiRoute-Token: {token}")
    };
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\n{auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
    read_response(stream)
}

fn serialize_document_with_model_first(document: &Value) -> Vec<u8> {
    let mut remainder = document.as_object().expect("request object").clone();
    let model = remainder.remove("model").expect("top-level model");
    let remainder = serde_json::to_vec(&Value::Object(remainder)).unwrap();
    let mut body = format!("{{\"model\":{},", serde_json::to_string(&model).unwrap()).into_bytes();
    body.extend_from_slice(&remainder[1..]);
    body
}

fn snapshot(
    provider: &str,
    rejecting_provider: &str,
    forbidden_provider: &str,
) -> GatewayPublicationSnapshotV3 {
    let candidate =
        |local_id, stable_target_key: &str, credential_ref: &str, authority: &str, upstream| {
            sealed_native_candidate(
                local_id,
                stable_target_key,
                &[credential_ref.into()],
                authority,
                &format!("continuation-native-{local_id}"),
                &[(IngressProtocol::Responses, upstream)],
            )
        };
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "continuation-authority",
        21,
        34,
        "continuation-renderer/v1",
        vec![
            AliasPlanV1 {
                served_model_id: "continuation".into(),
                purpose: "Tool continuation".into(),
                agent_plan_revision: 55,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 10_000,
                max_attempts: 3,
                routing: None,
                candidates: vec![
                    candidate(
                        1,
                        "cross-protocol-target",
                        "forbidden-credential",
                        forbidden_provider,
                        IngressProtocol::Messages,
                    ),
                    candidate(
                        2,
                        "continuation-target",
                        "continuation-credential",
                        provider,
                        IngressProtocol::Responses,
                    ),
                    candidate(
                        3,
                        "forbidden-fallback-target",
                        "forbidden-credential",
                        forbidden_provider,
                        IngressProtocol::Responses,
                    ),
                ],
            },
            AliasPlanV1 {
                served_model_id: "namespace-rejected".into(),
                purpose: "Legal namespace rejected by one same-protocol upstream".into(),
                agent_plan_revision: 56,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 10_000,
                max_attempts: 2,
                routing: None,
                candidates: vec![
                    candidate(
                        4,
                        "rejecting-target",
                        "rejecting-credential",
                        rejecting_provider,
                        IngressProtocol::Responses,
                    ),
                    candidate(
                        5,
                        "rejecting-forbidden-fallback",
                        "forbidden-credential",
                        forbidden_provider,
                        IngressProtocol::Responses,
                    ),
                ],
            },
            AliasPlanV1 {
                served_model_id: "other-plan".into(),
                purpose: "Different AgentPlan scope".into(),
                agent_plan_revision: 57,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 10_000,
                max_attempts: 1,
                routing: None,
                candidates: vec![candidate(
                    6,
                    "other-target",
                    "forbidden-credential",
                    forbidden_provider,
                    IngressProtocol::Responses,
                )],
            },
        ],
        vec![
            GrantV1 {
                grant_id: "continuation-grant".into(),
                generation: 1,
                bearer_token_sha256: token_sha256("continuation-token"),
                protocol: IngressProtocol::Responses,
                routes: [
                    ("continuation", 55),
                    ("namespace-rejected", 56),
                    ("other-plan", 57),
                ]
                .into_iter()
                .map(|(alias, revision)| {
                    (
                        alias.into(),
                        hiroute_gateway::server::publication::ModelRouteV2::Plan {
                            plan_id: format!("legacy/{alias}"),
                            alias: alias.into(),
                            revision,
                            semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(
                                alias.as_bytes(),
                            ),
                        },
                    )
                })
                .collect(),
            },
            GrantV1 {
                grant_id: "other-grant".into(),
                generation: 1,
                bearer_token_sha256: token_sha256("other-token"),
                protocol: IngressProtocol::Responses,
                routes: [(
                    "continuation".into(),
                    hiroute_gateway::server::publication::ModelRouteV2::Plan {
                        plan_id: "legacy/continuation".into(),
                        alias: "continuation".into(),
                        revision: 55,
                        semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"continuation"),
                    },
                )]
                .into(),
            },
        ],
    )
    .unwrap()
}

enum ProviderReply {
    Sse(&'static [u8]),
    Json(&'static [u8]),
    JsonStatus { status: u16, body: &'static [u8] },
}

struct ContinuationProvider {
    transport: TestTlsListener,
    connections: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    sse_fragments: Arc<AtomicUsize>,
    requests: Arc<Mutex<BTreeMap<usize, Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ContinuationProvider {
    fn start(replies: Vec<ProviderReply>) -> Self {
        static PROVIDER_SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        let listener = TestTlsListener::bind(format!(
            "continuation-provider-{}.invalid",
            PROVIDER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
        .unwrap();
        listener.set_nonblocking(true).unwrap();
        let transport = listener.try_clone().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let sse_fragments = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(BTreeMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_connections = Arc::clone(&connections);
        let thread_calls = Arc::clone(&calls);
        let thread_sse_fragments = Arc::clone(&sse_fragments);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let thread_replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let thread = std::thread::spawn(move || {
            let attempts_started = Arc::new(AtomicUsize::new(0));
            let mut handlers = Vec::new();
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        thread_connections.fetch_add(1, Ordering::Relaxed);
                        let calls = Arc::clone(&thread_calls);
                        let sse_fragments = Arc::clone(&thread_sse_fragments);
                        let requests = Arc::clone(&thread_requests);
                        let replies = Arc::clone(&thread_replies);
                        let attempts_started = Arc::clone(&attempts_started);
                        handlers.push(std::thread::spawn(move || {
                            stream.set_nonblocking(false).unwrap();
                            let Some((mut request, expected_length)) =
                                read_provider_request_head(&mut stream)
                            else {
                                return;
                            };
                            let index = attempts_started.fetch_add(1, Ordering::Relaxed);
                            let reply = replies
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .pop_front()
                                .unwrap_or(ProviderReply::Json(
                                    br#"{"error":{"type":"unexpected_attempt"}}"#,
                                ));
                            drain_provider_request(&mut stream, &mut request, expected_length);
                            write_provider_reply(&mut stream, reply, &sse_fragments);
                            stream.finish().unwrap();
                            requests
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .insert(index, request);
                            calls.fetch_add(1, Ordering::Relaxed);
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
            for handler in handlers {
                handler.join().unwrap();
            }
        });
        Self {
            transport,
            connections,
            calls,
            sse_fragments,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn authority(&self) -> &str {
        self.transport.authority()
    }

    fn transport(&self) -> &TestTlsListener {
        &self.transport
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    fn sse_fragments(&self) -> usize {
        self.sse_fragments.load(Ordering::Relaxed)
    }

    fn requests(&self) -> Vec<Vec<u8>> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }
}

impl Drop for ContinuationProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn wait_for_calls(provider: &ContinuationProvider, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while provider.calls() < expected {
        assert!(
            Instant::now() < deadline,
            "Provider attempt was not observed"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(provider.calls(), expected);
}

fn http_body(wire: &[u8]) -> &[u8] {
    let split = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    &wire[split + 4..]
}

fn write_provider_reply(
    stream: &mut TestTlsStream,
    reply: ProviderReply,
    sse_fragments: &AtomicUsize,
) {
    let (status, content_type, body) = match &reply {
        ProviderReply::Sse(body) => (200, "text/event-stream", *body),
        ProviderReply::Json(body) => (200, "application/json", *body),
        ProviderReply::JsonStatus { status, body } => (*status, "application/json", *body),
    };
    write!(
        stream,
        "HTTP/1.1 {status} Provider Response\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    match reply {
        ProviderReply::Sse(body) => write_fragmented_sse(stream, body, sse_fragments),
        ProviderReply::Json(body) => {
            stream.write_all(body).unwrap();
            stream.flush().unwrap();
        }
        ProviderReply::JsonStatus { body, .. } => {
            stream.write_all(body).unwrap();
            stream.flush().unwrap();
        }
    }
}

fn write_fragmented_sse(stream: &mut TestTlsStream, body: &[u8], fragments: &AtomicUsize) {
    let mut offset = 0;
    let mut fragment_index = 0;
    while offset < body.len() {
        let bytes = SSE_FRAGMENT_SIZES[fragment_index % SSE_FRAGMENT_SIZES.len()];
        let end = (offset + bytes).min(body.len());
        stream.write_all(&body[offset..end]).unwrap();
        stream.flush().unwrap();
        fragments.fetch_add(1, Ordering::Relaxed);
        offset = end;
        fragment_index += 1;
    }
}

fn expected_sse_fragment_count(body: &[u8]) -> usize {
    let mut offset = 0;
    let mut count = 0;
    while offset < body.len() {
        offset = (offset + SSE_FRAGMENT_SIZES[count % SSE_FRAGMENT_SIZES.len()]).min(body.len());
        count += 1;
    }
    count
}
