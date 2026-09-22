use super::*;

#[test]
fn one_registered_native_source_uses_matching_messages_and_responses_provider_paths() {
    let _serial = process_test_lock();
    let directory = tempfile::tempdir().unwrap();
    let provider = TestTlsListener::bind("shared-zhipu-provider.invalid").unwrap();
    provider.set_nonblocking(true).unwrap();
    let alias = "shared-glm-5.3";
    let credential_ref = "protocol-credential".to_owned();
    let mut candidate = sealed_native_candidate(
        1,
        "shared-zhipu",
        std::slice::from_ref(&credential_ref),
        provider.authority(),
        "glm-5.3",
        &[(IngressProtocol::Responses, IngressProtocol::Responses)],
    );
    let mut messages = sealed_native_candidate(
        2,
        "shared-zhipu-messages-fixture",
        std::slice::from_ref(&credential_ref),
        provider.authority(),
        "glm-5.3",
        &[(IngressProtocol::Messages, IngressProtocol::Messages)],
    )
    .protocol_profiles
    .remove(0);
    candidate.protocol_profiles[0].connector.request_path = "/api/v1/responses".into();
    candidate.protocol_profiles[0].capability.capability_id =
        "cap.zhipu.glm-5.3.coding-plan.responses".into();
    candidate.protocol_profiles[0].adapter_revision = "adapter.openai-responses.v1@1".into();
    messages.connector.request_path = "/api/anthropic/v1/messages".into();
    messages.capability.capability_id = "cap.zhipu.glm-5.3.coding-plan.messages".into();
    messages.adapter_revision = "adapter.anthropic-messages.v1@1".into();
    let GatewayCriticalFactV1::Exact(headers) = &mut messages.connector.headers else {
        panic!("fixture requires exact protocol headers");
    };
    headers.required_headers = vec![("anthropic-version".into(), "2023-06-01".into())];
    candidate.protocol_profiles.push(messages);
    candidate.endpoint = format!("https://{}/api/v1/responses", provider.authority());
    candidate.operational_target = GatewayOperationalTargetV1::RegisteredHttps {
        uri: candidate.endpoint.clone(),
    };
    candidate.operational_target_digest =
        CanonicalDigest::of(&candidate.operational_target).unwrap();
    candidate.protocol_profile_digest = CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    let publication = GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "shared-native-protocols",
        13,
        55,
        "shared-native-protocols/v1",
        vec![AliasPlanV1 {
            served_model_id: alias.into(),
            purpose: "one source across native protocols".into(),
            agent_plan_revision: 89,
            protocols: vec![IngressProtocol::Responses, IngressProtocol::Messages],
            overall_timeout_ms: 10_000,
            max_attempts: 1,
            routing: None,
            candidates: vec![candidate],
        }],
        [IngressProtocol::Responses, IngressProtocol::Messages]
            .into_iter()
            .map(|protocol| GrantV1 {
                grant_id: format!("shared-grant-{}", protocol_name(protocol)),
                generation: 1,
                bearer_token_sha256: token_sha256(&format!(
                    "protocol-token-{}",
                    protocol_name(protocol)
                )),
                protocol,
                routes: [(
                    alias.into(),
                    hiroute_gateway::server::publication::ModelRouteV2::Plan {
                        plan_id: "legacy/shared-native-protocols".into(),
                        semantic_digest: CanonicalDigest::of_bytes(b"shared-native-protocols"),
                        alias: alias.into(),
                        revision: 89,
                    },
                )]
                .into(),
            })
            .collect(),
    )
    .unwrap();
    let publication_path = directory.path().join("publication.json");
    std::fs::write(&publication_path, serde_json::to_vec(&publication).unwrap()).unwrap();
    write_dial_config(directory.path(), &[&provider]).unwrap();
    let credential_file = directory.path().join("protocol-credential.json");
    let write_credential = |secret: &str| {
        std::fs::write(&credential_file, serde_json::to_vec(&json!({
            "schema_version":"hiroute.gateway.credential-leases/v1",
            "credential_ref":credential_ref,
            "keys":[{"key_id":"protocol-key","generation":1,"authorization":format!("Bearer {secret}")}]
        })).unwrap()).unwrap();
    };
    write_credential("original-provider-secret");
    let credentials_path = directory.path().join("credentials.json");
    std::fs::write(
        &credentials_path,
        serde_json::to_vec(&json!({
            "schema_version":"hiroute.gateway.credentials/v1",
            "credentials":{"protocol-credential":"protocol-credential.json"}
        }))
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
    let unsupported = single_write_request(
        address,
        "/v1/messages",
        &serde_json::to_vec(&json!({
            "model":alias,"max_tokens":64,"messages":[{"role":"user","content":"hello"}],
            "tools":[{"name":"lookup","input_schema":{"type":"object"}}],
            "tool_choice":{"type":"none"}
        }))
        .unwrap(),
    );
    assert_eq!(
        unsupported.status,
        500,
        "{}",
        String::from_utf8_lossy(&unsupported.body)
    );
    let error: Value = serde_json::from_slice(&unsupported.body).unwrap();
    assert_eq!(error["code"], "PLANNER_INPUT_UNAVAILABLE");
    assert_eq!(error["phase"], "planner");
    assert_eq!(
        provider_accept_error_kind(&provider),
        std::io::ErrorKind::WouldBlock
    );
    let upstream = provider.try_clone().unwrap();
    let provider_thread = std::thread::spawn(move || {
        for index in 0..6 {
            let deadline = Instant::now() + Duration::from_secs(15);
            let (mut stream, _) = loop {
                match upstream.accept() {
                    Ok(value) => break value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "missing shared source request {index}"
                        );
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("Provider accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            let (mut wire, expected_length) = read_provider_request_head(&mut stream).unwrap();
            drain_provider_request(&mut stream, &mut wire, expected_length);
            let (path, headers, body) = parse_native_request(&wire);
            let protocol = if matches!(index, 0 | 2 | 3) {
                IngressProtocol::Responses
            } else {
                IngressProtocol::Messages
            };
            assert_eq!(
                path,
                match protocol {
                    IngressProtocol::Responses => "POST /api/v1/responses HTTP/1.1",
                    IngressProtocol::Messages => "POST /api/anthropic/v1/messages HTTP/1.1",
                    _ => unreachable!(),
                }
            );
            assert_eq!(body["model"], "glm-5.3");
            assert_eq!(
                native_header(headers, "authorization"),
                Some(if index < 2 {
                    "Bearer original-provider-secret"
                } else {
                    "Bearer rotated-provider-secret"
                })
            );
            assert!(native_header(headers, "x-hiroute-token").is_none());
            assert_eq!(
                native_header(headers, "anthropic-version"),
                (protocol == IngressProtocol::Messages).then_some("2023-06-01")
            );
            if index == 2 || index == 4 {
                assert_eq!(body["tools"][0]["name"], "lookup");
            }
            if index == 3 {
                assert!(
                    body["input"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| item["type"] == "function_call_output")
                );
            }
            if index == 5 {
                assert!(body["messages"].as_array().unwrap().iter().any(|item| {
                    item["content"].as_array().is_some_and(|blocks| {
                        blocks.iter().any(|block| block["type"] == "tool_result")
                    })
                }));
            }
            let (reply, content_type): (&[u8], &str) = match index {
                0 | 1 => (native_same_protocol_stream(protocol), "text/event-stream"),
                2 => (br#"{"id":"native-tool","model":"glm-5.3","status":"completed","output":[{"type":"function_call","id":"fc-native","call_id":"native-call","name":"lookup","arguments":"{\"key\":\"v\"}","status":"completed"}],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}"#, "application/json"),
                4 => (br#"{"id":"native-tool","type":"message","role":"assistant","model":"glm-5.3","content":[{"type":"tool_use","id":"native-lookup","name":"lookup","input":{"key":"v"}}],"stop_reason":"tool_use","usage":{"input_tokens":2,"output_tokens":3}}"#, "application/json"),
                _ => (native_response(protocol), "application/json"),
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len()).unwrap();
            stream.write_all(reply).unwrap();
            stream.finish().unwrap();
        }
    });

    for protocol in [IngressProtocol::Responses, IngressProtocol::Messages] {
        let mut request = client_request(protocol, alias);
        request["stream"] = json!(true);
        let response = single_write_request(
            address,
            protocol_path(protocol),
            &serde_json::to_vec(&request).unwrap(),
        );
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert_same_protocol_stream(protocol, alias, &response.body);
    }
    write_credential("rotated-provider-secret");
    let tool = json!({"type":"function","name":"lookup","parameters":{"type":"object"}});
    let responses_tool = single_write_request(address, "/v1/responses", &serde_json::to_vec(&json!({
        "model":alias,"input":"hello","tools":[tool],"tool_choice":"auto","parallel_tool_calls":false
    })).unwrap());
    assert_eq!(
        responses_tool.status,
        200,
        "{}",
        String::from_utf8_lossy(&responses_tool.body)
    );
    let responses_tool: Value = serde_json::from_slice(&responses_tool.body).unwrap();
    let response_call_id = responses_tool["output"][0]["call_id"].as_str().unwrap();
    let response_final = single_write_request(address, "/v1/responses", &serde_json::to_vec(&json!({
        "model":alias,"input":[
            {"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]},
            {"type":"function_call","call_id":response_call_id,"name":"lookup","arguments":"{\"key\":\"v\"}"},
            {"type":"function_call_output","call_id":response_call_id,"output":"result"}
        ],
        "tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}],
        "tool_choice":"auto","parallel_tool_calls":false
    })).unwrap());
    assert_eq!(
        response_final.status,
        200,
        "{}",
        String::from_utf8_lossy(&response_final.body)
    );
    let messages_tool = single_write_request(
        address,
        "/v1/messages",
        &serde_json::to_vec(&json!({
            "model":alias,"max_tokens":64,"messages":[{"role":"user","content":"hello"}],
            "tools":[{"name":"lookup","input_schema":{"type":"object"}}]
        }))
        .unwrap(),
    );
    assert_eq!(
        messages_tool.status,
        200,
        "{}",
        String::from_utf8_lossy(&messages_tool.body)
    );
    let messages_tool: Value = serde_json::from_slice(&messages_tool.body).unwrap();
    let message_call_id = messages_tool["content"][0]["id"].as_str().unwrap();
    let messages_final = single_write_request(address, "/v1/messages", &serde_json::to_vec(&json!({
        "model":alias,"max_tokens":64,"messages":[
            {"role":"user","content":"hello"},
            {"role":"assistant","content":[{"type":"tool_use","id":message_call_id,"name":"lookup","input":{"key":"v"}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":message_call_id,"content":"result"}]}
        ],
        "tools":[{"name":"lookup","input_schema":{"type":"object"}}]
    })).unwrap());
    assert_eq!(
        messages_final.status,
        200,
        "{}",
        String::from_utf8_lossy(&messages_final.body)
    );
    provider_thread.join().unwrap();
    process.stop();
}

/// Opt-in product probe: use an isolated Gateway and a real GLM key from a local file.
/// The normal test suite never contacts a provider or reads the key.
#[test]
#[ignore = "requires HIROUTE_GLM_KEY_FILE and external GLM access"]
fn real_glm_messages_native_path_handles_repeated_small_and_large_requests() {
    let key_path = std::env::var_os("HIROUTE_GLM_KEY_FILE")
        .expect("set HIROUTE_GLM_KEY_FILE to a local file containing the GLM key");
    let key = std::fs::read_to_string(key_path).expect("GLM key file is unreadable");
    let key = key.trim();
    assert!(!key.is_empty(), "GLM key file is empty");

    let _serial = process_test_lock();
    let directory = tempfile::tempdir().unwrap();
    // The dial map is required by the process fixture. This unrelated entry leaves
    // open.bigmodel.cn on the real DNS and system-CA transport path.
    let unmapped = TestTlsListener::bind("unused-real-glm-probe.invalid").unwrap();
    write_dial_config(directory.path(), &[&unmapped]).unwrap();
    let authority = "open.bigmodel.cn";
    let alias = "real-glm-5.3";
    let credential_ref = "real-glm-credential".to_owned();
    let mut candidate = sealed_native_candidate(
        1,
        "real-glm",
        std::slice::from_ref(&credential_ref),
        authority,
        "glm-5.3",
        &[(IngressProtocol::Responses, IngressProtocol::Responses)],
    );
    let mut messages = sealed_native_candidate(
        2,
        "real-glm-messages-face",
        std::slice::from_ref(&credential_ref),
        authority,
        "glm-5.3",
        &[(IngressProtocol::Messages, IngressProtocol::Messages)],
    )
    .protocol_profiles
    .remove(0);
    candidate.protocol_profiles[0].connector.request_path = "/api/v1/responses".into();
    messages.connector.request_path = "/api/anthropic/v1/messages".into();
    let GatewayCriticalFactV1::Exact(headers) = &mut messages.connector.headers else {
        panic!("fixture requires exact protocol headers");
    };
    headers.required_headers = vec![("anthropic-version".into(), "2023-06-01".into())];
    candidate.protocol_profiles.push(messages);
    candidate.endpoint = format!("https://{authority}/api/v1/responses");
    candidate.operational_target = GatewayOperationalTargetV1::RegisteredHttps {
        uri: candidate.endpoint.clone(),
    };
    candidate.operational_target_digest =
        CanonicalDigest::of(&candidate.operational_target).unwrap();
    candidate.protocol_profile_digest = CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    let publication = GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "real-glm-probe",
        13,
        55,
        "real-glm-probe/v1",
        vec![AliasPlanV1 {
            served_model_id: alias.into(),
            purpose: "real GLM Messages native path".into(),
            agent_plan_revision: 89,
            protocols: vec![IngressProtocol::Messages],
            overall_timeout_ms: 30_000,
            max_attempts: 2,
            routing: None,
            candidates: vec![candidate],
        }],
        vec![GrantV1 {
            grant_id: "real-glm-probe-grant".into(),
            generation: 1,
            bearer_token_sha256: token_sha256("protocol-token-messages"),
            protocol: IngressProtocol::Messages,
            routes: [(
                alias.into(),
                hiroute_gateway::server::publication::ModelRouteV2::Plan {
                    plan_id: "legacy/real-glm-probe".into(),
                    semantic_digest: CanonicalDigest::of_bytes(b"real-glm-probe"),
                    alias: alias.into(),
                    revision: 89,
                },
            )]
            .into(),
        }],
    )
    .unwrap();
    let publication_path = directory.path().join("publication.json");
    std::fs::write(&publication_path, serde_json::to_vec(&publication).unwrap()).unwrap();
    let credential_file = directory.path().join("real-glm-credential.json");
    std::fs::write(
        &credential_file,
        serde_json::to_vec(&json!({
            "schema_version":"hiroute.gateway.credential-leases/v1",
            "credential_ref":credential_ref,
            "keys":[{"key_id":"real-glm-key","generation":1,"authorization":format!("Bearer {key}")}]
        }))
        .unwrap(),
    )
    .unwrap();
    let credentials_path = directory.path().join("credentials.json");
    std::fs::write(
        &credentials_path,
        serde_json::to_vec(&json!({
            "schema_version":"hiroute.gateway.credentials/v1",
            "credentials":{"real-glm-credential":"real-glm-credential.json"}
        }))
        .unwrap(),
    )
    .unwrap();
    let observation_root = directory.path().join("observation");
    std::fs::create_dir(&observation_root).unwrap();
    let observation_root_value = observation_root.to_str().unwrap();
    let address = reserve_address();
    let mut process = Hirouted::spawn_with_environment(
        &exact_hirouted_binary(),
        address,
        &directory.path().join("lkg.json"),
        &publication_path,
        &credentials_path,
        directory.path(),
        &[
            ("HIROUTE_E2E_OBSERVATION_CAPTURE", "1"),
            ("HIROUTE_OBSERVATION_DIRECTORY", observation_root_value),
            ("HIROUTE_OBSERVATION_QUEUE_BYTES", "524288"),
            ("HIROUTE_OBSERVATION_LIFECYCLE_SINK", "healthy"),
            ("HIROUTE_OBSERVATION_EXECUTION_SINK", "healthy"),
            ("HIROUTE_OBSERVATION_CONTENT_SINK", "healthy"),
            ("HIROUTE_OBSERVATION_RUN_RELATION_SINK", "healthy"),
            ("HIROUTE_OBSERVATION_OTEL_SINK", "healthy"),
        ],
    );
    process.wait_ready();
    for cycle in 0..5 {
        for (label, content) in [
            ("small", "Reply with OK".to_owned()),
            (
                "large",
                format!("Reply with OK after reading: {}", "alpha ".repeat(13_120)),
            ),
        ] {
            // Keep model in the first 16 KiB selector window. A map-backed JSON
            // serializer may sort it after a large messages value instead.
            let content_json = serde_json::to_string(&content).unwrap();
            let body = format!(
            r#"{{"model":"{alias}","max_tokens":128,"messages":[{{"role":"user","content":{content_json}}}],"stream":false}}"#
        )
        .into_bytes();
            let response = single_write_request(address, "/v1/messages", &body);
            let value: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
            let response_keys = value
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let facts = if response.status != 200 {
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    let facts =
                        std::fs::read_to_string(observation_root.join("execution-fact.jsonl"))
                            .unwrap_or_default()
                            .lines()
                            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                            .filter_map(|record| record.get("fact").cloned())
                            .filter(|fact| {
                                matches!(
                                    fact["kind"].as_str(),
                                    Some(
                                        "candidate_decision"
                                            | "attempt_started"
                                            | "attempt_finished"
                                            | "runtime_state"
                                            | "request_finished"
                                    )
                                )
                            })
                            .map(|fact| {
                                json!({
                                    "kind":fact["kind"],
                                    "eligible":fact["eligible"],
                                    "exclusion_reason":fact["exclusion_reason"],
                                    "target_serialized_bytes":fact["target_serialized_bytes"],
                                    "upstream_protocol":fact["upstream_protocol"],
                                    "outcome":fact["outcome"],
                                    "error_class":fact["error_class"],
                                    "retryable":fact["retryable"],
                                    "disposition":fact["disposition"],
                                    "operation":fact["operation"],
                                    "health":fact["health"],
                                    "provider_http_status":fact["provider_http_status"],
                                    "transport":fact["transport"],
                                    "commits":fact["commits"],
                                    "termination_reason":fact["termination_reason"]
                                })
                            })
                            .collect::<Vec<_>>();
                    if facts
                        .iter()
                        .filter(|fact| fact["kind"] == "request_finished")
                        .count()
                        > cycle * 2 + usize::from(label == "large")
                        || Instant::now() >= deadline
                    {
                        break facts;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            } else {
                Vec::new()
            };
            assert_eq!(
                response.status,
                200,
                "cycle={cycle} {label}: request_bytes={}, response_bytes={}, content_type={:?}, json_body={}, response_keys={response_keys:?}, code={}, phase={}, error_type={}, facts={facts:?}",
                body.len(),
                response.body.len(),
                response.headers.get("content-type"),
                !value.is_null(),
                value["code"],
                value["phase"],
                value["error"]["type"]
            );
            assert_eq!(value["type"], "message", "{label}");
            assert_eq!(value["model"], alias, "{label}");
            assert!(
                value["content"]
                    .as_array()
                    .is_some_and(|content| !content.is_empty()),
                "{label}: provider returned no content"
            );
        }
    }
    if let Some(claude_bin) = std::env::var_os("HIROUTE_CLAUDE_BIN") {
        let claude_home = directory.path().join("claude-home");
        let claude_work = directory.path().join("claude-work");
        std::fs::create_dir(&claude_home).unwrap();
        std::fs::create_dir(&claude_work).unwrap();
        let stdout_path = directory.path().join("claude.stdout");
        let stderr_path = directory.path().join("claude.stderr");
        let mut claude = std::process::Command::new(claude_bin)
            .args([
                "--print",
                "--no-session-persistence",
                "--model",
                alias,
                "--output-format",
                "json",
                "--max-turns",
                "1",
                "Reply with exactly OK.",
            ])
            .env_clear()
            .env("HOME", &claude_home)
            .env("XDG_CONFIG_HOME", &claude_home)
            .env("TMPDIR", directory.path())
            .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
            .env("ANTHROPIC_BASE_URL", format!("http://{address}"))
            .env("ANTHROPIC_AUTH_TOKEN", "protocol-token-messages")
            .env("ANTHROPIC_MODEL", alias)
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("DISABLE_TELEMETRY", "1")
            .current_dir(&claude_work)
            .stdin(std::process::Stdio::null())
            .stdout(std::fs::File::create(&stdout_path).unwrap())
            .stderr(std::fs::File::create(&stderr_path).unwrap())
            .spawn()
            .expect("isolated Claude CLI must start");
        let deadline = Instant::now() + Duration::from_secs(90);
        let status = loop {
            if let Some(status) = claude.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                claude.kill().unwrap();
                claude.wait().unwrap();
                panic!("isolated Claude CLI did not finish within 90 seconds");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let stdout = std::fs::read_to_string(&stdout_path).unwrap();
        let stderr = std::fs::read_to_string(&stderr_path).unwrap();
        assert!(
            status.success(),
            "isolated Claude CLI failed: status={status}, stdout={stdout}, stderr={stderr}"
        );
        let response: Value = serde_json::from_str(&stdout).unwrap();
        assert!(
            response["result"]
                .as_str()
                .is_some_and(|result| result.contains("OK")),
            "isolated Claude CLI returned no OK: {stdout}"
        );
    }
    process.stop();
}

#[test]
fn codex_responses_to_messages_real_listener_supports_tools_turns_and_stream_with_hard_boundaries()
{
    let _serial = process_test_lock();
    let directory = tempfile::tempdir().unwrap();
    let provider = TestTlsListener::bind("codex-messages-provider.invalid").unwrap();
    provider.set_nonblocking(true).unwrap();
    let pair = (IngressProtocol::Responses, IngressProtocol::Messages);
    let alias = pair_alias(pair.0, pair.1);
    let target = pair_target(pair.0, pair.1);
    let publication_path = directory.path().join("publication.json");
    let credentials_path = directory.path().join("credentials.json");
    std::fs::write(
        &publication_path,
        serde_json::to_vec(&snapshot(&[provider.authority().to_owned()], &[pair])).unwrap(),
    )
    .unwrap();
    write_dial_config(directory.path(), &[&provider]).unwrap();
    std::fs::write(directory.path().join("protocol-credential.json"), serde_json::to_vec(&json!({
        "schema_version":"hiroute.gateway.credential-leases/v1",
        "credential_ref":"protocol-credential",
        "keys":[{"key_id":"protocol-key","generation":1,"authorization":"Bearer protocol-provider-secret"}]
    })).unwrap()).unwrap();
    std::fs::write(
        &credentials_path,
        serde_json::to_vec(&json!({
            "schema_version":"hiroute.gateway.credentials/v1",
            "credentials":{"protocol-credential":"protocol-credential.json"}
        }))
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

    for input in [
        json!([
            {"type":"message","role":"developer","content":[{"type":"input_text","text":"Codex bounds"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}
        ]),
        json!([
            {"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]},
            {"type":"message","role":"developer","content":[{"type":"input_text","text":"late policy"}]}
        ]),
        json!([{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]),
    ] {
        let mut body = json!({"model":alias,"instructions":"top policy","input":input});
        if body["input"].as_array().unwrap().len() == 1 {
            body["store"] = json!(false);
        }
        let response = single_write_request(
            address,
            "/v1/responses",
            &serde_json::to_vec(&body).unwrap(),
        );
        assert_eq!(
            response.status,
            400,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let error: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            error["code"], "CLIENT_PROTOCOL_UNREPRESENTABLE",
            "input: {input}; response: {error}"
        );
    }
    assert_eq!(
        provider_accept_error_kind(&provider),
        std::io::ErrorKind::WouldBlock
    );

    let upstream = provider.try_clone().unwrap();
    let provider_thread = std::thread::spawn(move || {
        let stream_reply = native_stream(IngressProtocol::Messages);
        let replies: [&[u8]; 4] = [
            native_response(IngressProtocol::Messages),
            br#"{"id":"tool-turn","type":"message","role":"assistant","model":"glm-5.3","content":[{"type":"tool_use","id":"native-lookup","name":"lookup","input":{"key":"v"}}],"stop_reason":"tool_use","usage":{"input_tokens":2,"output_tokens":3}}"#,
            native_response(IngressProtocol::Messages),
            &stream_reply,
        ];
        for (index, reply) in replies.iter().enumerate() {
            let deadline = Instant::now() + Duration::from_secs(15);
            let (mut stream, _) = loop {
                match upstream.accept() {
                    Ok(value) => break value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "missing Messages attempt {index}"
                        );
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("Provider accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            let (mut wire, expected_length) = read_provider_request_head(&mut stream).unwrap();
            drain_provider_request(&mut stream, &mut wire, expected_length);
            let (path, headers, body) = parse_native_request(&wire);
            assert_eq!(path, "POST /v1/messages HTTP/1.1");
            assert_eq!(
                native_header(headers, "authorization"),
                Some("Bearer protocol-provider-secret")
            );
            assert!(native_header(headers, "x-hiroute-token").is_none());
            assert_eq!(body["model"], target);
            assert_eq!(body["system"], "top policy");
            assert_eq!(body["messages"][0]["role"], "user");
            assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
            if index == 1 || index == 2 {
                assert_eq!(body["tools"][0]["name"], "lookup");
                assert_eq!(
                    body["tool_choice"],
                    json!({"type":"auto","disable_parallel_tool_use":true})
                );
            }
            if index == 2 {
                assert_eq!(body["messages"][1]["content"][0]["id"], "native-lookup");
                assert_eq!(
                    body["messages"][2]["content"][0]["tool_use_id"],
                    "native-lookup"
                );
                assert_eq!(body["messages"][2]["content"][0]["content"], "result");
            }
            assert_eq!(body["stream"], index == 3);
            let content_type = if index == 3 {
                "text/event-stream"
            } else {
                "application/json"
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len()).unwrap();
            stream.write_all(reply).unwrap();
            stream.finish().unwrap();
        }
    });

    let first = single_write_request(
        address,
        "/v1/responses",
        &serde_json::to_vec(&json!({
            "model":alias,"instructions":"top policy","input":"hello"
        }))
        .unwrap(),
    );
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let first: Value = serde_json::from_slice(&first.body).unwrap();
    assert_eq!(first["output"][0]["content"][0]["text"], "ok");

    let tool = json!({"type":"function","name":"lookup","parameters":{"type":"object"}});
    let second = single_write_request(
        address,
        "/v1/responses",
        &serde_json::to_vec(&json!({
            "model":alias,"instructions":"top policy","input":"hello",
            "tools":[tool],"tool_choice":"auto","parallel_tool_calls":false
        }))
        .unwrap(),
    );
    assert_eq!(
        second.status,
        200,
        "{}",
        String::from_utf8_lossy(&second.body)
    );
    let second: Value = serde_json::from_slice(&second.body).unwrap();
    let logical_id = second["output"][0]["call_id"]
        .as_str()
        .expect("logical tool ID");
    assert!(!logical_id.is_empty());
    let third = single_write_request(address, "/v1/responses", &serde_json::to_vec(&json!({
        "model":alias,"instructions":"top policy",
        "input":[
            {"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]},
            {"type":"function_call","call_id":logical_id,"name":"lookup","arguments":"{\"key\":\"v\"}"},
            {"type":"function_call_output","call_id":logical_id,"output":"result"}
        ],
        "tools":[tool],"tool_choice":"auto","parallel_tool_calls":false
    })).unwrap());
    assert_eq!(
        third.status,
        200,
        "{}",
        String::from_utf8_lossy(&third.body)
    );
    let third: Value = serde_json::from_slice(&third.body).unwrap();
    assert_eq!(third["output"][0]["content"][0]["text"], "ok");

    let fourth = single_write_request(
        address,
        "/v1/responses",
        &serde_json::to_vec(&json!({
            "model":alias,"instructions":"top policy","input":"hello","stream":true
        }))
        .unwrap(),
    );
    assert_eq!(
        fourth.status,
        200,
        "{}",
        String::from_utf8_lossy(&fourth.body)
    );
    assert_eq!(
        fourth.headers.get("content-type").map(String::as_str),
        Some("text/event-stream")
    );
    let stream = String::from_utf8(fourth.body).unwrap();
    assert!(stream.contains("response.output_text.delta"), "{stream}");
    assert!(
        stream.contains("response.function_call_arguments.delta"),
        "{stream}"
    );
    assert!(stream.contains("response.completed"), "{stream}");
    provider_thread.join().unwrap();
    process.stop();
}

#[test]
fn codex_native_responses_real_listener_keeps_response_wire_and_stream() {
    run_protocol_matrix(
        false,
        vec![(IngressProtocol::Responses, IngressProtocol::Responses)],
    );
}

#[test]
fn codex_native_responses_real_listener_accepts_instruction_roles_tools_and_turns() {
    let _serial = process_test_lock();
    let directory = tempfile::tempdir().unwrap();
    let provider = TestTlsListener::bind("codex-native-provider.invalid").unwrap();
    provider.set_nonblocking(true).unwrap();
    let pair = (IngressProtocol::Responses, IngressProtocol::Responses);
    let alias = pair_alias(pair.0, pair.1);
    let target = pair_target(pair.0, pair.1);
    let publication_path = directory.path().join("publication.json");
    let credentials_path = directory.path().join("credentials.json");
    std::fs::write(
        &publication_path,
        serde_json::to_vec(&snapshot(&[provider.authority().to_owned()], &[pair])).unwrap(),
    )
    .unwrap();
    write_dial_config(directory.path(), &[&provider]).unwrap();
    std::fs::write(
        directory.path().join("protocol-credential.json"),
        serde_json::to_vec(&json!({
            "schema_version":"hiroute.gateway.credential-leases/v1",
            "credential_ref":"protocol-credential",
            "keys":[{"key_id":"protocol-key","generation":1,"authorization":"Bearer protocol-provider-secret"}]
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        &credentials_path,
        serde_json::to_vec(&json!({
            "schema_version":"hiroute.gateway.credentials/v1",
            "credentials":{"protocol-credential":"protocol-credential.json"}
        }))
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

    let upstream = provider.try_clone().unwrap();
    let provider_thread = std::thread::spawn(move || {
        for index in 0..2 {
            let deadline = Instant::now() + Duration::from_secs(15);
            let (mut stream, _) = loop {
                match upstream.accept() {
                    Ok(value) => break value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "missing native Codex request {index}"
                        );
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("Provider accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            let (mut wire, expected_length) = read_provider_request_head(&mut stream).unwrap();
            drain_provider_request(&mut stream, &mut wire, expected_length);
            let (path, headers, body) = parse_native_request(&wire);
            assert_eq!(path, "POST /v1/responses HTTP/1.1");
            assert_eq!(
                native_header(headers, "authorization"),
                Some("Bearer protocol-provider-secret")
            );
            assert!(native_header(headers, "x-hiroute-token").is_none());
            assert_eq!(body["model"], target);
            assert_eq!(body["instructions"], "isolated Worker");
            assert_eq!(body["input"][0]["role"], "developer");
            assert_eq!(body["input"][0]["content"][0]["text"], "Codex bounds");
            assert_eq!(body["input"][1]["role"], "user");
            assert_eq!(body["tools"][0]["name"], "lookup");
            assert_eq!(body["store"], false);
            assert_eq!(body["stream"], index == 1);
            if index == 1 {
                assert_eq!(body["input"][3]["role"], "developer");
                assert_eq!(body["input"][3]["content"][0]["text"], "late bounds");
                assert_eq!(body["input"][4]["call_id"], "native-call");
                assert_eq!(body["input"][5]["call_id"], "native-call");
                assert_eq!(body["input"][6]["content"][0]["text"], "next turn");
            }
            let tool_reply = br#"{"id":"native-tool","model":"runtime-native","status":"completed","output":[{"type":"function_call","id":"fc-native","call_id":"native-call","name":"lookup","arguments":"{\"key\":\"v\"}","status":"completed"}],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}"#;
            let (reply, content_type): (&[u8], &str) = if index == 0 {
                (tool_reply, "application/json")
            } else {
                (
                    native_same_protocol_stream(IngressProtocol::Responses),
                    "text/event-stream",
                )
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len()).unwrap();
            stream.write_all(reply).unwrap();
            stream.finish().unwrap();
        }
    });

    let tool = json!({"type":"function","name":"lookup","parameters":{"type":"object"}});
    let first = single_write_request(
        address,
        "/v1/responses",
        &serde_json::to_vec(&json!({
            "model":alias,"instructions":"isolated Worker","stream":false,"store":false,
            "input":[
                {"type":"message","role":"developer","content":[{"type":"input_text","text":"Codex bounds"}]},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"first turn"}]}
            ],
            "tools":[tool],"tool_choice":"auto","parallel_tool_calls":false
        }))
        .unwrap(),
    );
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let first: Value = serde_json::from_slice(&first.body).unwrap();
    let logical_id = first["output"][0]["call_id"]
        .as_str()
        .expect("logical tool ID");
    assert!(!logical_id.is_empty());

    let second = single_write_request(
        address,
        "/v1/responses",
        &serde_json::to_vec(&json!({
            "model":alias,"instructions":"isolated Worker","stream":true,"store":false,
            "input":[
                {"type":"message","role":"developer","content":[{"type":"input_text","text":"Codex bounds"}]},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"first turn"}]},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"checking"}]},
                {"type":"message","role":"developer","content":[{"type":"input_text","text":"late bounds"}]},
                {"type":"function_call","call_id":logical_id,"name":"lookup","arguments":"{\"key\":\"v\"}"},
                {"type":"function_call_output","call_id":logical_id,"output":"result"},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"next turn"}]}
            ],
            "tools":[tool],"tool_choice":"auto","parallel_tool_calls":false
        }))
        .unwrap(),
    );
    assert_eq!(
        second.status,
        200,
        "{}",
        String::from_utf8_lossy(&second.body)
    );
    assert_eq!(
        second.headers.get("content-type").map(String::as_str),
        Some("text/event-stream")
    );
    let stream = String::from_utf8(second.body).unwrap();
    assert!(stream.contains("response.output_text.delta"), "{stream}");
    assert!(stream.contains("response.completed"), "{stream}");
    assert!(stream.contains(&alias), "{stream}");
    provider_thread.join().unwrap();
    process.stop();
}
