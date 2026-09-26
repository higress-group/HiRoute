use super::*;
use base64::Engine as _;

const CHAT_TOOL_PROVIDER_STREAM: &[u8] = br#"data: {"id":"chat-tool-round","object":"chat.completion.chunk","model":"chat-native","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"chat-call-lookup","type":"function","function":{"name":"records__lookup","arguments":"{\"key\":\"lookup-"}},{"index":1,"id":"chat-call-shell","type":"function","function":{"name":"shell","arguments":"{\"input\":\"custom-"}}]},"finish_reason":null}]}

data: {"id":"chat-tool-round","object":"chat.completion.chunk","model":"chat-native","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"input-secret-121\"}"}},{"index":1,"function":{"arguments":"input-secret-121\"}"}}]},"finish_reason":null}]}

data: {"id":"chat-tool-round","object":"chat.completion.chunk","model":"chat-native","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

data: {"id":"chat-tool-round","object":"chat.completion.chunk","model":"chat-native","choices":[],"usage":{"prompt_tokens":17,"completion_tokens":9,"total_tokens":26}}

data: [DONE]

"#;

const CHAT_FINAL_PROVIDER_JSON: &[u8] = br#"{"id":"chat-final","object":"chat.completion","model":"chat-native","choices":[{"index":0,"message":{"role":"assistant","content":"tools complete"},"finish_reason":"stop"}],"usage":{"prompt_tokens":29,"completion_tokens":3,"total_tokens":32}}"#;
const RESPONSES_GRAMMAR_FALLBACK_JSON: &[u8] = br#"{"id":"grammar-fallback","model":"responses-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"grammar fallback"}]}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}"#;
const RESPONSES_COLLISION_FALLBACK_JSON: &[u8] = br#"{"id":"collision-fallback","model":"responses-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"collision fallback"}]}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}"#;
const RESPONSES_OPTIONS_FALLBACK_JSON: &[u8] = br#"{"id":"options-fallback","model":"responses-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"options fallback"}]}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}"#;
const RESPONSES_DESCRIPTION_FALLBACK_JSON: &[u8] = br#"{"id":"description-fallback","model":"responses-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"description fallback"}]}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}"#;

#[test]
fn production_responses_to_chat_tools_complete_two_turns_and_fallback_before_attempt() {
    let _serial = process_test_lock();
    let directory = tempfile::tempdir().unwrap();
    let chat_provider = ContinuationProvider::start(vec![
        ProviderReply::Sse(CHAT_TOOL_PROVIDER_STREAM),
        ProviderReply::Json(CHAT_FINAL_PROVIDER_JSON),
    ]);
    let responses_fallback = ContinuationProvider::start(vec![
        ProviderReply::Json(RESPONSES_GRAMMAR_FALLBACK_JSON),
        ProviderReply::Json(RESPONSES_COLLISION_FALLBACK_JSON),
        ProviderReply::Json(RESPONSES_OPTIONS_FALLBACK_JSON),
        ProviderReply::Json(RESPONSES_DESCRIPTION_FALLBACK_JSON),
    ]);
    let publication_path = directory.path().join("publication.json");
    let credentials_path = directory.path().join("credentials.json");
    let lkg_path = directory.path().join("publication-lkg.json");
    std::fs::write(
        &publication_path,
        serde_json::to_vec_pretty(&responses_chat_tools_snapshot(
            chat_provider.authority(),
            responses_fallback.authority(),
        ))
        .unwrap(),
    )
    .unwrap();
    write_dial_config(
        directory.path(),
        &[chat_provider.transport(), responses_fallback.transport()],
    )
    .unwrap();
    for (credential_ref, key_id, authorization) in [
        (
            "chat-tools-credential",
            "chat-tools-key",
            "Bearer chat-tools-provider-secret",
        ),
        (
            "responses-fallback-credential",
            "responses-fallback-key",
            "Bearer responses-fallback-provider-secret",
        ),
    ] {
        std::fs::write(
            directory.path().join(format!("{credential_ref}.json")),
            serde_json::to_vec_pretty(&json!({
                "schema_version":"hiroute.gateway.credential-leases/v1",
                "credential_ref":credential_ref,
                "keys":[{"key_id":key_id,"generation":1,"authorization":authorization}]
            }))
            .unwrap(),
        )
        .unwrap();
    }
    std::fs::write(
        &credentials_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version":"hiroute.gateway.credentials/v1",
            "credentials":{
                "chat-tools-credential":"chat-tools-credential.json",
                "responses-fallback-credential":"responses-fallback-credential.json"
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let observation_root = directory.path().join("observation");
    std::fs::create_dir(&observation_root).unwrap();
    let replay_root = directory.path().join("replay");
    let replay_root = replay_root.to_str().unwrap().to_owned();
    let observation_root_value = observation_root.to_str().unwrap().to_owned();
    let address = reserve_address();
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
            ("HIROUTE_E2E_OBSERVATION_CAPTURE", "1"),
            (
                "HIROUTE_OBSERVATION_DIRECTORY",
                observation_root_value.as_str(),
            ),
            ("HIROUTE_OBSERVATION_QUEUE_BYTES", "524288"),
            ("HIROUTE_OBSERVATION_LIFECYCLE_SINK", "healthy"),
            ("HIROUTE_OBSERVATION_EXECUTION_SINK", "healthy"),
            ("HIROUTE_OBSERVATION_CONTENT_SINK", "healthy"),
            ("HIROUTE_OBSERVATION_RUN_RELATION_SINK", "healthy"),
            ("HIROUTE_OBSERVATION_OTEL_SINK", "healthy"),
        ],
    );
    process.wait_ready();

    let first = request(
        address,
        "responses-chat-tools-token",
        &codex_tools_request(true),
    );
    assert_eq!(
        first.status,
        200,
        "{}\n{}",
        String::from_utf8_lossy(&first.body),
        process.stderr()
    );
    wait_for_calls(&chat_provider, 1);
    assert_eq!(responses_fallback.calls(), 0);
    let first_upstream = chat_provider.requests();
    let projected: Value = serde_json::from_slice(http_body(&first_upstream[0])).unwrap();
    assert_eq!(projected["stream"], true);
    assert_eq!(projected["stream_options"], json!({"include_usage":true}));
    assert_eq!(projected["parallel_tool_calls"], true);
    assert_eq!(projected["tool_choice"], "auto");
    assert_eq!(projected["tools"].as_array().unwrap().len(), 2);
    assert_eq!(projected["tools"][0]["function"]["name"], "records__lookup");
    assert_eq!(projected["tools"][0]["function"]["strict"], true);
    assert_eq!(
        projected["tools"][0]["function"]["parameters"]["properties"]["key"]["description"],
        "tool-schema-secret-121",
        "{projected:#}"
    );
    assert_eq!(projected["tools"][1]["function"]["name"], "shell");
    assert_eq!(
        projected["tools"][1]["function"]["parameters"],
        json!({
            "type":"object",
            "properties":{"input":{"type":"string"}},
            "required":["input"],
            "additionalProperties":false
        })
    );

    let events = decode_downstream_sse(&first.body);
    let function_item = events
        .iter()
        .find_map(|(event, data)| {
            (event == "response.output_item.done" && data["item"]["type"] == "function_call")
                .then_some(&data["item"])
        })
        .expect("Responses function item completed");
    assert_eq!(function_item["namespace"], "records");
    assert_eq!(function_item["name"], "lookup");
    assert_eq!(
        function_item["arguments"],
        "{\"key\":\"lookup-input-secret-121\"}"
    );
    let function_logical_id = function_item["call_id"].as_str().unwrap().to_owned();
    let custom_item = events
        .iter()
        .find_map(|(event, data)| {
            (event == "response.output_item.done" && data["item"]["type"] == "custom_tool_call")
                .then_some(&data["item"])
        })
        .expect("Responses custom item completed");
    assert!(custom_item.get("namespace").is_none());
    assert_eq!(custom_item["name"], "shell");
    assert_eq!(custom_item["input"], "custom-input-secret-121");
    let custom_logical_id = custom_item["call_id"].as_str().unwrap().to_owned();
    let completed = events
        .iter()
        .find(|(event, _)| event == "response.completed")
        .unwrap();
    assert_eq!(
        completed.1["response"]["usage"],
        json!({"input_tokens":17,"output_tokens":9,"total_tokens":26})
    );

    let second = request(
        address,
        "responses-chat-tools-token",
        &codex_tool_results_request(&function_logical_id, &custom_logical_id),
    );
    assert_eq!(
        second.status,
        200,
        "{}\n{}",
        String::from_utf8_lossy(&second.body),
        process.stderr()
    );
    wait_for_calls(&chat_provider, 2);
    assert_eq!(responses_fallback.calls(), 0);
    let upstream = chat_provider.requests();
    let continued: Value = serde_json::from_slice(http_body(&upstream[1])).unwrap();
    assert_eq!(continued["messages"].as_array().unwrap().len(), 4);
    assert_eq!(continued["messages"][0]["role"], "user");
    assert_eq!(continued["messages"][1]["role"], "assistant");
    assert_eq!(
        continued["messages"][1]["tool_calls"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        continued["messages"][1]["tool_calls"][0]["id"],
        "chat-call-lookup"
    );
    assert_eq!(
        continued["messages"][1]["tool_calls"][0]["function"]["name"],
        "records__lookup"
    );
    assert_eq!(
        continued["messages"][1]["tool_calls"][1]["id"],
        "chat-call-shell"
    );
    assert_eq!(
        continued["messages"][1]["tool_calls"][1]["function"]["arguments"],
        "{\"input\":\"custom-input-secret-121\"}"
    );
    assert_eq!(continued["messages"][2]["role"], "tool");
    assert_eq!(continued["messages"][2]["tool_call_id"], "chat-call-lookup");
    assert_eq!(
        continued["messages"][2]["content"],
        "lookup-result-secret-121"
    );
    assert_eq!(continued["messages"][3]["role"], "tool");
    assert_eq!(continued["messages"][3]["tool_call_id"], "chat-call-shell");
    assert_eq!(
        continued["messages"][3]["content"],
        "custom-result-secret-121"
    );
    let final_response: Value = serde_json::from_slice(&second.body).unwrap();
    assert_eq!(
        final_response["output"][0]["content"][0]["text"],
        "tools complete"
    );

    let grammar = request(
        address,
        "responses-chat-tools-token",
        &json!({
            "model":"responses-chat-tools",
            "input":"grammar request",
            "tools":[{"type":"custom","name":"grammar_tool","format":{"type":"grammar","syntax":"lark","definition":"start: WORD"}}]
        }),
    );
    assert_eq!(
        grammar.status,
        200,
        "{}",
        String::from_utf8_lossy(&grammar.body)
    );
    wait_for_calls(&responses_fallback, 1);
    assert_eq!(chat_provider.calls(), 2);
    assert_eq!(
        serde_json::from_slice::<Value>(&grammar.body).unwrap()["output"][0]["content"][0]["text"],
        "grammar fallback"
    );

    let collision = request(
        address,
        "responses-chat-tools-token",
        &json!({
            "model":"responses-chat-tools",
            "input":"collision request",
            "tools":[
                {"type":"function","name":"records__lookup","parameters":{"type":"object"}},
                {"type":"namespace","name":"records","tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]}
            ]
        }),
    );
    assert_eq!(
        collision.status,
        200,
        "{}",
        String::from_utf8_lossy(&collision.body)
    );
    wait_for_calls(&responses_fallback, 2);
    assert_eq!(chat_provider.calls(), 2);
    assert_eq!(
        serde_json::from_slice::<Value>(&collision.body).unwrap()["output"][0]["content"][0]["text"],
        "collision fallback"
    );
    let fallback_requests = responses_fallback.requests();
    assert_eq!(
        serde_json::from_slice::<Value>(http_body(&fallback_requests[0])).unwrap()["tools"][0]["format"]
            ["type"],
        "grammar"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(http_body(&fallback_requests[1])).unwrap()["tools"][1]["type"],
        "namespace"
    );

    let options = request(
        address,
        "responses-chat-tools-token",
        &json!({
            "model":"responses-chat-tools",
            "input":"native options request",
            "store":false,
            "include":["reasoning.encrypted_content"],
            "prompt_cache_key":"session-121",
            "client_metadata":{"session_id":"session-121"}
        }),
    );
    assert_eq!(
        options.status,
        200,
        "{}",
        String::from_utf8_lossy(&options.body)
    );
    wait_for_calls(&responses_fallback, 3);
    assert_eq!(chat_provider.calls(), 2);

    let described_namespace = request(
        address,
        "responses-chat-tools-token",
        &json!({
            "model":"responses-chat-tools",
            "input":"described namespace request",
            "tools":[{"type":"namespace","name":"records","description":"record tools","tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]}]
        }),
    );
    assert_eq!(
        described_namespace.status,
        200,
        "{}",
        String::from_utf8_lossy(&described_namespace.body)
    );
    wait_for_calls(&responses_fallback, 4);
    assert_eq!(chat_provider.calls(), 2);
    let fallback_requests = responses_fallback.requests();
    let forwarded_options: Value =
        serde_json::from_slice(http_body(&fallback_requests[2])).unwrap();
    assert_eq!(forwarded_options["store"], false);
    assert_eq!(
        forwarded_options["include"],
        json!(["reasoning.encrypted_content"])
    );
    assert_eq!(forwarded_options["prompt_cache_key"], "session-121");
    assert_eq!(
        forwarded_options["client_metadata"],
        json!({"session_id":"session-121"})
    );
    let forwarded_description: Value =
        serde_json::from_slice(http_body(&fallback_requests[3])).unwrap();
    assert_eq!(
        forwarded_description["tools"][0]["description"],
        "record tools"
    );

    let facts = wait_for_observation_requests(&observation_root, 6);
    assert!(facts.iter().any(|record| {
        record.pointer("/fact/kind").and_then(Value::as_str) == Some("attempt_started")
            && record
                .pointer("/fact/upstream_protocol")
                .and_then(Value::as_str)
                == Some("chat_completions")
    }));
    assert!(facts.iter().any(|record| {
        record.pointer("/fact/kind").and_then(Value::as_str) == Some("candidate_decision")
            && record
                .pointer("/fact/upstream_protocol")
                .and_then(Value::as_str)
                == Some("chat_completions")
            && record.pointer("/fact/eligible").and_then(Value::as_bool) == Some(false)
            && record
                .pointer("/fact/exclusion_reason")
                .and_then(Value::as_str)
                == Some("TOOL_INTERFACE_UNSUPPORTED")
    }));
    assert!(facts.iter().any(|record| {
        record.pointer("/fact/kind").and_then(Value::as_str) == Some("candidate_decision")
            && record
                .pointer("/fact/upstream_protocol")
                .and_then(Value::as_str)
                == Some("chat_completions")
            && record.pointer("/fact/eligible").and_then(Value::as_bool) == Some(false)
            && record
                .pointer("/fact/exclusion_reason")
                .and_then(Value::as_str)
                == Some("PROTOCOL_PATH_UNAVAILABLE")
    }));
    assert!(facts.iter().any(|record| {
        record.pointer("/fact/kind").and_then(Value::as_str) == Some("attempt_started")
            && record
                .pointer("/fact/upstream_protocol")
                .and_then(Value::as_str)
                == Some("responses")
    }));
    let content = wait_for_observation_content(&observation_root, 6);
    let response_events = decoded_response_events(&content);
    assert!(response_events.iter().any(|event| {
        event.pointer("/event/kind").and_then(Value::as_str) == Some("tool_call_started")
            && event.pointer("/event/tool_kind").and_then(Value::as_str) == Some("function")
            && event.pointer("/event/namespace").and_then(Value::as_str) == Some("records")
            && event.pointer("/event/name").and_then(Value::as_str) == Some("lookup")
            && event.pointer("/event/logical_id").and_then(Value::as_str)
                == Some(function_logical_id.as_str())
    }));
    assert!(response_events.iter().any(|event| {
        event.pointer("/event/kind").and_then(Value::as_str) == Some("tool_call_started")
            && event.pointer("/event/tool_kind").and_then(Value::as_str) == Some("custom")
            && event.pointer("/event/namespace").is_none()
            && event.pointer("/event/name").and_then(Value::as_str) == Some("shell")
            && event.pointer("/event/logical_id").and_then(Value::as_str)
                == Some(custom_logical_id.as_str())
    }));
    let typed_observation = ["lifecycle.jsonl", "execution-fact.jsonl", "otel.jsonl"]
        .into_iter()
        .map(|name| std::fs::read_to_string(observation_root.join(name)).unwrap_or_default())
        .collect::<String>();
    for secret in [
        "tool-schema-secret-121",
        "lookup-input-secret-121",
        "custom-input-secret-121",
        "lookup-result-secret-121",
        "custom-result-secret-121",
        "chat-tools-provider-secret",
        "responses-fallback-provider-secret",
        "authorization",
    ] {
        assert!(
            !typed_observation.contains(secret),
            "typed observation leaked {secret}"
        );
    }

    process.stop();
}

fn codex_tools_request(stream: bool) -> Value {
    json!({
        "model":"responses-chat-tools",
        "stream":stream,
        "instructions":"isolated Codex worker",
        "input":[
            {"type":"message","role":"developer","content":[{"type":"input_text","text":"use declared tools"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"look up and execute"}]},
            {"type":"additional_tools","tools":[
                {"type":"custom","name":"shell","description":"execute freeform input"}
            ]}
        ],
        "parallel_tool_calls":true,
        "tool_choice":"auto",
        "tools":[{
            "type":"namespace",
            "name":"records",
            "tools":[{
                "type":"function",
                "name":"lookup",
                "description":"lookup a record",
                "parameters":{
                    "type":"object",
                    "properties":{"key":{"type":"string","description":"tool-schema-secret-121"}},
                    "required":["key"],
                    "additionalProperties":false
                },
                "strict":true
            }]
        }]
    })
}

fn codex_tool_results_request(function_id: &str, custom_id: &str) -> Value {
    json!({
        "model":"responses-chat-tools",
        "stream":false,
        "input":[
            {"type":"message","role":"user","content":[{"type":"input_text","text":"continue the tool roundtrip"}]},
            {"type":"function_call","call_id":function_id,"namespace":"records","name":"lookup","arguments":"{\"key\":\"lookup-input-secret-121\"}"},
            {"type":"custom_tool_call","call_id":custom_id,"name":"shell","input":"custom-input-secret-121"},
            {"type":"function_call_output","call_id":function_id,"output":"lookup-result-secret-121"},
            {"type":"custom_tool_call_output","call_id":custom_id,"output":"custom-result-secret-121"},
            {"type":"additional_tools","tools":[
                {"type":"custom","name":"shell","description":"execute freeform input"}
            ]}
        ],
        "parallel_tool_calls":true,
        "tool_choice":"auto",
        "tools":codex_tools_request(false)["tools"].clone()
    })
}

fn responses_chat_tools_snapshot(
    chat_provider: &str,
    responses_fallback: &str,
) -> GatewayPublicationSnapshotV3 {
    let chat = sealed_native_candidate(
        1,
        "responses-chat-tools-primary",
        &["chat-tools-credential".into()],
        chat_provider,
        "chat-native",
        &[(IngressProtocol::Responses, IngressProtocol::ChatCompletions)],
    );
    let fallback = sealed_native_candidate(
        2,
        "responses-chat-tools-fallback",
        &["responses-fallback-credential".into()],
        responses_fallback,
        "responses-native",
        &[(IngressProtocol::Responses, IngressProtocol::Responses)],
    );
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "responses-chat-tools-authority",
        31,
        44,
        "responses-chat-tools-renderer/v1",
        vec![AliasPlanV1 {
            served_model_id: "responses-chat-tools".into(),
            purpose: "Responses to Chat reversible tools".into(),
            agent_plan_revision: 121,
            protocols: vec![IngressProtocol::Responses],
            overall_timeout_ms: 10_000,
            max_attempts: 2,
            routing: Some(AliasRoutingV1 {
                agent_plan_id: "responses-chat-tools-plan".into(),
                plan_display_name: Some("Responses Chat tools".into()),
                request_owned: AliasRequestOwnedRouteV1::Classified {
                    reselect_on_user_message: false,
                    classifier: AliasComplexityClassifierV1 {
                        revision: "responses-chat-tools-complexity/v1".into(),
                        mode: hiroute_domain::ComplexityClassifierModeV1::LocalRules,
                        user_keywords: vec!["force-complex-route-121".into()],
                    },
                    simple_groups: vec![AliasGroupIdV1::Economy, AliasGroupIdV1::Primary],
                    complex_groups: vec![AliasGroupIdV1::Primary],
                },
                groups: vec![
                    AliasModelGroupV1 {
                        group_id: AliasGroupIdV1::Economy,
                        candidate_local_ids: vec![chat.local_id],
                    },
                    AliasModelGroupV1 {
                        group_id: AliasGroupIdV1::Primary,
                        candidate_local_ids: vec![fallback.local_id],
                    },
                ],
            }),
            candidates: vec![chat, fallback],
        }],
        vec![GrantV1 {
            grant_id: "responses-chat-tools-grant".into(),
            generation: 1,
            bearer_token_sha256: token_sha256("responses-chat-tools-token"),
            protocol: IngressProtocol::Responses,
            routes: [(
                "responses-chat-tools".into(),
                hiroute_gateway::server::publication::ModelRouteV2::Plan {
                    plan_id: "responses-chat-tools-plan".into(),
                    alias: "responses-chat-tools".into(),
                    revision: 121,
                    semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(
                        b"responses-chat-tools",
                    ),
                },
            )]
            .into(),
        }],
    )
    .unwrap()
}

fn wait_for_observation_requests(root: &std::path::Path, expected: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = std::fs::read_to_string(root.join("execution-fact.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect::<Vec<_>>();
        let completed = records
            .iter()
            .filter(|record| {
                record.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished")
            })
            .count();
        if completed >= expected {
            return records;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected} observed requests: {records:#?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_observation_content(root: &std::path::Path, expected: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = std::fs::read_to_string(root.join("conversation-content.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect::<Vec<_>>();
        let completed = records
            .iter()
            .filter(|record| {
                record.get("direction").and_then(Value::as_str) == Some("response_delivered")
                    && record.get("phase").and_then(Value::as_str) == Some("finish")
            })
            .count();
        if completed >= expected {
            return records;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected} observed response streams: {records:#?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn decoded_response_events(records: &[Value]) -> Vec<Value> {
    let mut parts = BTreeMap::<(String, u64), Vec<(u64, Vec<u8>)>>::new();
    for record in records.iter().filter(|record| {
        record.get("direction").and_then(Value::as_str) == Some("response_delivered")
            && record.get("phase").and_then(Value::as_str) == Some("append")
    }) {
        let Some(request_id) = record
            .pointer("/correlation/request_id")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(part) = record.get("part_ordinal").and_then(Value::as_u64) else {
            continue;
        };
        let Some(chunk) = record.get("chunk_ordinal").and_then(Value::as_u64) else {
            continue;
        };
        let Some(encoded) = record.get("canonical_bytes_base64").and_then(Value::as_str) else {
            continue;
        };
        parts
            .entry((request_id.to_owned(), part))
            .or_default()
            .push((
                chunk,
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .unwrap(),
            ));
    }
    parts
        .into_values()
        .map(|mut chunks| {
            chunks.sort_by_key(|(ordinal, _)| *ordinal);
            let bytes = chunks
                .into_iter()
                .flat_map(|(_, bytes)| bytes)
                .collect::<Vec<_>>();
            serde_json::from_slice(&bytes).unwrap()
        })
        .collect()
}
