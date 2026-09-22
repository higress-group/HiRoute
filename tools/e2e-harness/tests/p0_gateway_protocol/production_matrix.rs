use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use hiroute_domain::{CanonicalDigest, GatewayCriticalFactV1, GatewayOperationalTargetV1};
use hiroute_e2e::gateway_fixture::{TestTlsListener, sealed_native_candidate, write_dial_config};
use hiroute_gateway::server::publication::{
    AliasPlanV1, GatewayPublicationSnapshotV3, GrantV1, token_sha256,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

use super::process::{
    Hirouted, drain_provider_request, exact_hirouted_binary, process_test_lock,
    read_provider_request_head, read_response, reserve_address,
};
use super::support::native_stream;

#[path = "production_matrix/native_protocol_paths.rs"]
mod native_protocol_paths;

#[test]
fn protocol_real_hirouted_executes_every_native_protocol_pair_and_rejects_before_provider() {
    hiroute_e2e::p0_execution_receipt!(
        "protocol.nine_native_pairs",
        [
            "protocol.responses_to_responses",
            "protocol.responses_to_chat_completions",
            "protocol.responses_to_messages",
            "protocol.chat_completions_to_responses",
            "protocol.chat_completions_to_chat_completions",
            "protocol.chat_completions_to_messages",
            "protocol.messages_to_responses",
            "protocol.messages_to_chat_completions",
            "protocol.messages_to_messages",
            "protocol.exact_native_request",
            "protocol.exact_client_projection",
            "protocol.fragmented_ingress",
            "protocol.preconnect_rejection",
        ]
    );
    HiroutedProtocolMatrix::run(false);
}

#[test]
fn fixed_real_hirouted_executes_native_protocol_pairs_without_plan_aliases() {
    HiroutedProtocolMatrix::run(true);
}

struct HiroutedProtocolMatrix;

impl HiroutedProtocolMatrix {
    fn run(fixed: bool) {
        run_protocol_matrix(fixed, protocol_pairs());
    }
}

fn run_protocol_matrix(fixed: bool, pairs: Vec<(IngressProtocol, IngressProtocol)>) {
    let _serial = process_test_lock();
    let directory = tempfile::tempdir().unwrap();
    let providers = pairs
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let provider =
                TestTlsListener::bind(format!("protocol-matrix-provider-{index}.invalid")).unwrap();
            provider.set_nonblocking(true).unwrap();
            provider
        })
        .collect::<Vec<_>>();
    let provider_authorities = providers
        .iter()
        .map(|provider| provider.authority().to_owned())
        .collect::<Vec<_>>();
    let publication_path = directory.path().join("publication.json");
    let credentials_path = directory.path().join("credentials.json");
    let lkg_path = directory.path().join("publication-lkg.json");
    let mut publication = snapshot(&provider_authorities, &pairs);
    if fixed {
        for grant in &mut publication.grants {
            for (name, route) in &mut grant.routes {
                let alias = publication
                    .aliases
                    .iter()
                    .find(|alias| alias.served_model_id == *name)
                    .unwrap();
                assert_eq!(alias.candidates.len(), 1);
                let binding = alias.candidates[0].clone();
                *route = hiroute_gateway::server::publication::ModelRouteV2::Fixed {
                    binding_digest: hiroute_domain::CanonicalDigest::of(&binding).unwrap(),
                    binding: Box::new(binding),
                    overall_timeout_ms: alias.overall_timeout_ms,
                    max_attempts: alias.max_attempts,
                };
            }
        }
        publication.aliases.clear();
        publication.payload_digest = publication.canonical_digest().unwrap();
        publication.validate().unwrap();
        assert!(publication.aliases.is_empty());
    }
    std::fs::write(
        &publication_path,
        serde_json::to_vec_pretty(&publication).unwrap(),
    )
    .unwrap();
    write_dial_config(directory.path(), &providers.iter().collect::<Vec<_>>()).unwrap();
    std::fs::write(
        directory.path().join("protocol-credential.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credential-leases/v1",
            "credential_ref": "protocol-credential",
            "keys": [{
                "key_id": "protocol-key",
                "generation": 1,
                "authorization": "Bearer protocol-provider-secret"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        &credentials_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credentials/v1",
            "credentials": {"protocol-credential": "protocol-credential.json"}
        }))
        .unwrap(),
    )
    .unwrap();
    let address = reserve_address();
    let mut process = Hirouted::spawn(
        &exact_hirouted_binary(),
        address,
        &lkg_path,
        &publication_path,
        &credentials_path,
        directory.path(),
    );
    process.wait_ready();

    let mut client_stream_gates = Vec::with_capacity(pairs.len());
    let provider_threads = providers
        .iter()
        .zip(&pairs)
        .map(|(provider, pair)| {
            let expected = if pair.0 == pair.1 {
                vec![*pair, *pair]
            } else {
                vec![*pair]
            };
            let (provider_gate, client_gate) = if pair.0 == pair.1 {
                let (provider_gate, client_gate) = native_stream_gate();
                (Some(provider_gate), Some(client_gate))
            } else {
                (None, None)
            };
            client_stream_gates.push(client_gate);
            serve_native_provider(provider.try_clone().unwrap(), expected, provider_gate)
        })
        .collect::<Vec<_>>();
    for (ingress, upstream) in &pairs {
        let (ingress, upstream) = (*ingress, *upstream);
        let path = protocol_path(ingress);
        let alias = pair_alias(ingress, upstream);
        let response = fragmented_request(
            address,
            path,
            &serde_json::to_vec(&client_request(ingress, &alias)).unwrap(),
        );
        assert_eq!(
            response.status,
            200,
            "{alias} {path}: {}; daemon stderr: {}",
            String::from_utf8_lossy(&response.body),
            process.stderr()
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&response.body).unwrap(),
            expected_client_body(ingress, upstream, &alias),
            "{path}: {}",
            String::from_utf8_lossy(&response.body)
        );
    }
    for protocol in pairs
        .iter()
        .filter(|(ingress, upstream)| ingress == upstream)
        .map(|(protocol, _)| *protocol)
    {
        let path = protocol_path(protocol);
        let alias = pair_alias(protocol, protocol);
        let mut request = client_request(protocol, &alias);
        request["stream"] = Value::Bool(true);
        let pair_index = pairs
            .iter()
            .position(|pair| *pair == (protocol, protocol))
            .unwrap();
        let response = fragmented_streaming_request(
            address,
            path,
            &serde_json::to_vec(&request).unwrap(),
            client_stream_gates[pair_index]
                .take()
                .expect("same-protocol stream gate"),
            protocol,
        );
        assert_eq!(
            response.status,
            200,
            "streaming {alias} {path}: {}; daemon stderr: {}",
            String::from_utf8_lossy(&response.body),
            process.stderr()
        );
        assert_eq!(
            response.headers.get("content-type").map(String::as_str),
            Some("text/event-stream")
        );
        assert_same_protocol_stream(protocol, &alias, &response.body);
    }
    let invalid = single_write_request(
        address,
        "/v1/responses",
        format!(
            r#"{{"model":"{}","input":"hello","temperature":0.2}}"#,
            pair_alias(IngressProtocol::Responses, IngressProtocol::Responses)
        )
        .as_bytes(),
    );
    assert_eq!(invalid.status, 400);
    let invalid: Value = serde_json::from_slice(&invalid.body).unwrap();
    assert_eq!(invalid["phase"], "canonical_request");
    assert_eq!(invalid["code"], "PROTOCOL_SEMANTICS_UNSUPPORTED");
    for provider in &providers {
        assert_eq!(
            provider_accept_error_kind(provider),
            std::io::ErrorKind::WouldBlock,
            "rejected ingress must not open a native provider connection"
        );
    }
    process.stop();
    for provider_thread in provider_threads {
        provider_thread.join().unwrap();
    }
}

fn protocol_pairs() -> Vec<(IngressProtocol, IngressProtocol)> {
    [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ]
    .into_iter()
    .flat_map(|ingress| {
        [
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            IngressProtocol::Messages,
        ]
        .into_iter()
        .map(move |upstream| (ingress, upstream))
    })
    .collect()
}

fn snapshot(
    provider_authorities: &[String],
    pairs: &[(IngressProtocol, IngressProtocol)],
) -> GatewayPublicationSnapshotV3 {
    assert_eq!(provider_authorities.len(), pairs.len());
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "protocol-authority",
        13,
        55,
        "protocol-renderer/v1",
        pairs
            .iter()
            .enumerate()
            .map(|(index, (ingress, upstream))| AliasPlanV1 {
                served_model_id: pair_alias(*ingress, *upstream),
                purpose: "canonical protocol projection".into(),
                agent_plan_revision: 89 + u64::try_from(index).unwrap(),
                protocols: vec![*ingress],
                overall_timeout_ms: 10_000,
                max_attempts: 1,
                routing: None,
                candidates: vec![sealed_native_candidate(
                    u32::try_from(index + 1).unwrap(),
                    &pair_target(*ingress, *upstream),
                    &["protocol-credential".into()],
                    &provider_authorities[index],
                    &pair_target(*ingress, *upstream),
                    &[(*ingress, *upstream)],
                )],
            })
            .collect(),
        [
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            IngressProtocol::Messages,
        ]
        .into_iter()
        .filter(|protocol| pairs.iter().any(|(ingress, _)| ingress == protocol))
        .map(|protocol| GrantV1 {
            grant_id: format!("protocol-grant-{}", protocol_name(protocol)),
            generation: 1,
            bearer_token_sha256: token_sha256(&format!(
                "protocol-token-{}",
                protocol_name(protocol)
            )),
            protocol,
            routes: pairs
                .iter()
                .enumerate()
                .filter(|(_, (ingress, _))| *ingress == protocol)
                .map(|(index, (ingress, upstream))| {
                    let alias = pair_alias(*ingress, *upstream);
                    (
                        alias.clone(),
                        hiroute_gateway::server::publication::ModelRouteV2::Plan {
                            plan_id: format!("legacy/{alias}"),
                            semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(
                                alias.as_bytes(),
                            ),
                            alias,
                            revision: 89 + u64::try_from(index).unwrap(),
                        },
                    )
                })
                .collect(),
        })
        .collect(),
    )
    .unwrap()
}

fn pair_alias(ingress: IngressProtocol, upstream: IngressProtocol) -> String {
    format!(
        "wire-{}-to-{}",
        protocol_name(ingress),
        protocol_name(upstream)
    )
}

fn pair_target(ingress: IngressProtocol, upstream: IngressProtocol) -> String {
    format!(
        "protocol-target-{}-to-{}",
        protocol_name(ingress),
        protocol_name(upstream)
    )
}

fn protocol_name(protocol: IngressProtocol) -> &'static str {
    match protocol {
        IngressProtocol::Responses => "responses",
        IngressProtocol::ChatCompletions => "chat_completions",
        IngressProtocol::Messages => "messages",
    }
}

fn protocol_path(protocol: IngressProtocol) -> &'static str {
    match protocol {
        IngressProtocol::Responses => "/v1/responses",
        IngressProtocol::ChatCompletions => "/v1/chat/completions",
        IngressProtocol::Messages => "/v1/messages",
    }
}

fn client_request(ingress: IngressProtocol, alias: &str) -> Value {
    match ingress {
        IngressProtocol::Responses => json!({"model":alias,"input":"hello","stream":false}),
        IngressProtocol::ChatCompletions => {
            json!({"model":alias,"messages":[{"role":"user","content":"hello"}],"stream":false})
        }
        IngressProtocol::Messages => {
            json!({"model":alias,"max_tokens":8,"messages":[{"role":"user","content":"hello"}],"stream":false})
        }
    }
}

fn expected_client_body(ingress: IngressProtocol, upstream: IngressProtocol, alias: &str) -> Value {
    if ingress == upstream {
        let mut value: Value = serde_json::from_slice(native_same_protocol_response(upstream))
            .expect("same-protocol fixture response is JSON");
        value["model"] = json!(alias);
        return value;
    }
    match ingress {
        IngressProtocol::Responses => json!({
            "created_at": 0,
            "error": null,
            "id": "native-provider",
            "incomplete_details": null,
            "model": alias,
            "object": "response",
            "output": [{
                "content": [{"annotations": [], "text": "ok", "type": "output_text"}],
                "id": "msg_0",
                "role": "assistant",
                "status": "completed",
                "type": "message"
            }],
            "status": "completed",
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
        }),
        IngressProtocol::ChatCompletions => json!({
            "choices": [{
                "finish_reason": "stop",
                "index": 0,
                "logprobs": null,
                "message": {"content": "ok", "role": "assistant"}
            }],
            "created": 0,
            "id": "native-provider",
            "model": alias,
            "object": "chat.completion",
            "usage": {"completion_tokens": 1, "prompt_tokens": 1, "total_tokens": 2}
        }),
        IngressProtocol::Messages => json!({
            "content": [{"text": "ok", "type": "text"}],
            "id": "native-provider",
            "model": alias,
            "role": "assistant",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "type": "message",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }),
    }
}

fn expected_native_body(upstream: IngressProtocol, target: &str, streaming: bool) -> Value {
    match upstream {
        IngressProtocol::Responses => json!({
            "model": target,
            "input": [{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}],
            "max_output_tokens": 256,
            "stream": streaming
        }),
        IngressProtocol::ChatCompletions => {
            let mut value = json!({
                "model": target,
                "messages": [{"role":"user","content":"hello"}],
                "max_completion_tokens": 256,
                "stream": streaming
            });
            if streaming {
                value["stream_options"] = json!({"include_usage":true});
            }
            value
        }
        IngressProtocol::Messages => json!({
            "model": target,
            "messages": [{"role":"user","content":[{"type":"text","text":"hello"}]}],
            "max_tokens": 256,
            "stream": streaming
        }),
    }
}

fn native_response(upstream: IngressProtocol) -> &'static [u8] {
    match upstream {
        IngressProtocol::Responses => br#"{"id":"native-provider","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#,
        IngressProtocol::ChatCompletions => br#"{"id":"native-provider","object":"chat.completion","created":0,"model":"runtime-native","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
        IngressProtocol::Messages => br#"{"id":"native-provider","type":"message","role":"assistant","model":"runtime-native","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}"#,
    }
}

fn native_same_protocol_response(upstream: IngressProtocol) -> &'static [u8] {
    match upstream {
        IngressProtocol::Responses => br#"{"id":"native-provider","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2},"provider_extension":{"model":"nested-untouched","future":true}}"#,
        IngressProtocol::ChatCompletions => br#"{"id":"native-provider","object":"chat.completion","created":0,"model":"runtime-native","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2},"provider_extension":{"model":"nested-untouched","future":true}}"#,
        IngressProtocol::Messages => br#"{"id":"native-provider","type":"message","role":"assistant","model":"runtime-native","content":[{"type":"text","text":"ok","provider_extension":{"future":true}}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1},"provider_extension":{"model":"nested-untouched"}}"#,
    }
}

fn native_same_protocol_stream(upstream: IngressProtocol) -> &'static [u8] {
    match upstream {
        IngressProtocol::Responses => br#"event: response.vendor_extension
id: opaque
data: {"type":"response.vendor_extension","provider_extension":{"model":"nested-untouched","future":true}}

event: response.created
data: {"type":"response.created","response":{"id":"native-stream","model":"runtime-native","status":"in_progress","provider_extension":{"future":true}}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"message","output_index":0,"content_index":0,"delta":"ok","provider_extension":{"future":true}}

event: response.completed
data: {"type":"response.completed","response":{"id":"native-stream","model":"runtime-native","status":"completed","output":[{"type":"message","id":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2},"provider_extension":{"model":"nested-untouched"}}}

"#,
        IngressProtocol::ChatCompletions => br#"data: {"id":"native-stream","object":"chat.completion.chunk","created":1,"model":"runtime-native","choices":[{"index":0,"delta":{"role":"assistant","content":"ok","provider_extension":{"future":true}},"finish_reason":null}],"provider_extension":{"model":"nested-untouched"}}

data: {"id":"native-stream","object":"chat.completion.chunk","created":1,"model":"runtime-native","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"provider_extension":{"future":true}}

data: {"id":"native-stream","object":"chat.completion.chunk","created":1,"model":"runtime-native","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2},"provider_extension":{"future":true}}

data: [DONE]

"#,
        IngressProtocol::Messages => br#"event: message.vendor_extension
id: opaque
data: {"type":"message.vendor_extension","provider_extension":{"model":"nested-untouched","future":true}}

event: message_start
data: {"type":"message_start","message":{"id":"native-stream","type":"message","role":"assistant","model":"runtime-native","content":[],"stop_reason":null,"usage":{"input_tokens":1},"provider_extension":{"future":true}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":"","provider_extension":{"future":true}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok","provider_extension":{"future":true}}}

event: content_block_stop
data: {"type":"content_block_stop","index":0,"provider_extension":{"future":true}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1},"provider_extension":{"future":true}}

event: message_stop
data: {"type":"message_stop","provider_extension":{"model":"nested-untouched"}}

"#,
    }
}

fn assert_same_protocol_stream(protocol: IngressProtocol, alias: &str, body: &[u8]) {
    let wire = std::str::from_utf8(body).expect("native client stream is UTF-8");
    assert!(
        wire.contains(&format!(r#""model":"{alias}""#)),
        "owned model path was not rewritten: {wire}"
    );
    assert!(wire.contains("provider_extension"));
    assert!(wire.contains("nested-untouched"));
    assert!(!wire.contains(r#""model":"runtime-native""#));
    match protocol {
        IngressProtocol::Responses => {
            assert!(wire.contains("event: response.vendor_extension\nid: opaque\n"));
            assert!(wire.contains("event: response.completed\n"));
        }
        IngressProtocol::ChatCompletions => {
            assert!(wire.contains("data: [DONE]\n\n"));
        }
        IngressProtocol::Messages => {
            assert!(wire.contains("event: message.vendor_extension\nid: opaque\n"));
            assert!(wire.contains("event: message_stop\n"));
        }
    }
}

struct ProviderStreamGate {
    prefix_sent: mpsc::Sender<()>,
    release_terminal: mpsc::Receiver<()>,
}

struct ClientStreamGate {
    prefix_sent: mpsc::Receiver<()>,
    release_terminal: mpsc::Sender<()>,
}

fn native_stream_gate() -> (ProviderStreamGate, ClientStreamGate) {
    let (prefix_sent_tx, prefix_sent_rx) = mpsc::channel();
    let (release_terminal_tx, release_terminal_rx) = mpsc::channel();
    (
        ProviderStreamGate {
            prefix_sent: prefix_sent_tx,
            release_terminal: release_terminal_rx,
        },
        ClientStreamGate {
            prefix_sent: prefix_sent_rx,
            release_terminal: release_terminal_tx,
        },
    )
}

fn stream_terminal_partition(protocol: IngressProtocol) -> &'static [u8] {
    match protocol {
        IngressProtocol::Responses => b"event: response.completed\n",
        IngressProtocol::ChatCompletions => {
            b"data: {\"id\":\"native-stream\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"runtime-native\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}"
        }
        IngressProtocol::Messages => b"event: content_block_stop\n",
    }
}

fn stream_visible_marker(protocol: IngressProtocol) -> &'static [u8] {
    match protocol {
        IngressProtocol::Responses => b"response.output_text.delta",
        IngressProtocol::ChatCompletions => b"\"content\":\"ok\"",
        IngressProtocol::Messages => b"content_block_delta",
    }
}

fn stream_terminal_marker(protocol: IngressProtocol) -> &'static [u8] {
    match protocol {
        IngressProtocol::Responses => b"response.completed",
        IngressProtocol::ChatCompletions => b"[DONE]",
        IngressProtocol::Messages => b"message_stop",
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn serve_native_provider(
    listener: TestTlsListener,
    pairs: Vec<(IngressProtocol, IngressProtocol)>,
    stream_gate: Option<ProviderStreamGate>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let expected = pairs.len();
        let remaining = Arc::new(Mutex::new(pairs));
        let served = Arc::new(AtomicUsize::new(0));
        let stream_gate = Arc::new(Mutex::new(stream_gate));
        // This deadline bounds a fixture thread, not a single Gateway attempt. The
        // matrix intentionally performs nine sequential requests whose individual
        // product timeout is ten seconds, so one short global deadline can make a
        // healthy late pair observe a closed fixture under loaded runners.
        let deadline = Instant::now() + Duration::from_secs(15 * expected as u64);
        let mut handlers = Vec::new();
        while served.load(Ordering::SeqCst) < expected {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let remaining = Arc::clone(&remaining);
                    let served = Arc::clone(&served);
                    let stream_gate = Arc::clone(&stream_gate);
                    handlers.push(std::thread::spawn(move || {
                        // A connection without a request is not Provider execution. The
                        // native client can maintain idle pooled connections, so handling
                        // each accepted socket independently prevents one idle socket from
                        // blocking another real request.
                        // `TcpListener` is nonblocking only so this accept loop can keep
                        // accepting pooled sockets.  The request-body reader must be
                        // blocking after an HTTP/1.1 `100 Continue`: macOS preserves the
                        // accepted socket's nonblocking flag, and otherwise it can observe
                        // EAGAIN between the headers and the deferred body.
                        stream.set_nonblocking(false).unwrap();
                        let Some((mut request, expected_length)) =
                            read_provider_request_head(&mut stream)
                        else {
                            return;
                        };
                        drain_provider_request(&mut stream, &mut request, expected_length);
                        let (request_line, headers, body) = parse_native_request(&request);
                        let streaming = body["stream"].as_bool().expect("native request stream");
                        let target = body["model"].as_str().expect("native request model");
                        let (ingress, upstream) = {
                            let mut remaining = remaining
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            let position = remaining
                                .iter()
                                .position(|(ingress, upstream)| {
                                    target == pair_target(*ingress, *upstream)
                                })
                                .expect("native Provider target must be a unique P0 pair");
                            remaining.remove(position)
                        };
                        assert_eq!(
                            request_line,
                            format!("POST {} HTTP/1.1", protocol_path(upstream))
                        );
                        assert_eq!(
                            native_header(headers, "authorization"),
                            Some("Bearer protocol-provider-secret")
                        );
                        assert!(native_header(headers, "x-hiroute-token").is_none());
                        assert!(native_header(headers, "chatgpt-account-id").is_none());
                        assert!(!headers.contains("native-client-secret"));
                        assert!(!headers.contains("protocol-token-"));
                        assert_eq!(native_header(headers, "content-type"), Some("application/json"));
                        assert_eq!(
                            body,
                            expected_native_body(
                                upstream,
                                &pair_target(ingress, upstream),
                                streaming,
                            )
                        );
                        let response = if streaming {
                            assert_eq!(ingress, upstream);
                            native_same_protocol_stream(upstream)
                        } else if ingress == upstream {
                            native_same_protocol_response(upstream)
                        } else {
                            native_response(upstream)
                        };
                        let content_type = if streaming {
                            "text/event-stream"
                        } else {
                            "application/json"
                        };
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            response.len()
                        )
                        .unwrap();
                        if streaming {
                            let terminal = stream_terminal_partition(upstream);
                            let terminal_offset = response
                                .windows(terminal.len())
                                .position(|window| window == terminal)
                                .expect("stream fixture contains its terminal partition");
                            let gate = stream_gate
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .take()
                                .expect("same-protocol provider stream gate");
                            stream.write_all(&response[..terminal_offset]).unwrap();
                            stream.flush().unwrap();
                            gate.prefix_sent.send(()).unwrap();
                            gate.release_terminal
                                .recv_timeout(Duration::from_secs(15))
                                .expect("client observes stream prefix before terminal release");
                            stream.write_all(&response[terminal_offset..]).unwrap();
                        } else {
                            stream.write_all(response).unwrap();
                        }
                        stream.finish().unwrap();
                        served.fetch_add(1, Ordering::SeqCst);
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "native provider received {}/{} expected P0 requests",
                        served.load(Ordering::SeqCst),
                        expected
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("native provider accept failed: {error}"),
            }
        }
        for handler in handlers {
            handler.join().unwrap();
        }
        assert!(remaining.lock().unwrap().is_empty());
    })
}

fn provider_accept_error_kind(listener: &TestTlsListener) -> std::io::ErrorKind {
    match listener.accept() {
        Err(error) => error.kind(),
        Ok(_) => panic!("unexpected Provider connection"),
    }
}

fn parse_native_request(request: &[u8]) -> (&str, &str, Value) {
    let split = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("native request has HTTP headers");
    let headers = std::str::from_utf8(&request[..split]).expect("native request headers are UTF-8");
    let request_line = headers.lines().next().expect("native request line");
    let body = serde_json::from_slice(&request[split + 4..]).expect("native request JSON body");
    (request_line, headers, body)
}

fn native_header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().skip(1).find_map(|line| {
        let (actual, value) = line.split_once(':')?;
        actual.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

fn protocol_authorization(path: &str) -> String {
    let protocol = IngressProtocol::from_path(path).unwrap();
    let token = format!("protocol-token-{}", protocol_name(protocol));
    if protocol == IngressProtocol::Responses {
        format!(
            "X-HiRoute-Token: {token}\r\nAuthorization: Bearer native-client-secret\r\nChatgpt-Account-Id: native-client-account"
        )
    } else {
        format!("Authorization: Bearer {token}")
    }
}

fn fragmented_request(
    address: SocketAddr,
    path: &str,
    body: &[u8],
) -> super::process::WireResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\n{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        protocol_authorization(path), body.len()
    )
    .unwrap();
    for (index, chunk) in body.chunks(3).enumerate() {
        stream.write_all(chunk).unwrap();
        stream.flush().unwrap();
        if index < 3 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    read_response(stream)
}

fn fragmented_streaming_request(
    address: SocketAddr,
    path: &str,
    body: &[u8],
    gate: ClientStreamGate,
    protocol: IngressProtocol,
) -> super::process::WireResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\n{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        protocol_authorization(path), body.len()
    )
    .unwrap();
    for chunk in body.chunks(3) {
        stream.write_all(chunk).unwrap();
    }
    stream.flush().unwrap();

    gate.prefix_sent
        .recv_timeout(Duration::from_secs(15))
        .expect("Provider writes the non-terminal stream prefix");
    let visible = stream_visible_marker(protocol);
    let terminal = stream_terminal_marker(protocol);
    let mut wire = Vec::new();
    let mut buffer = [0_u8; 4096];
    while !contains_bytes(&wire, visible) {
        let read = stream
            .read(&mut buffer)
            .expect("client reads native stream prefix before terminal");
        assert_ne!(read, 0, "native stream ended before its visible delta");
        wire.extend_from_slice(&buffer[..read]);
    }
    assert!(
        !contains_bytes(&wire, terminal),
        "terminal marker arrived before the fixture released it: {}",
        String::from_utf8_lossy(&wire)
    );
    gate.release_terminal.send(()).unwrap();
    if let Err(error) = stream.read_to_end(&mut wire) {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    super::process::parse_response(wire)
}

fn single_write_request(
    address: SocketAddr,
    path: &str,
    body: &[u8],
) -> super::process::WireResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\n{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        protocol_authorization(path), body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    read_response(stream)
}
