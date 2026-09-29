use super::*;

#[test]
fn directory_is_optional_for_registered_and_manual_text_models() {
    for registered in [false, true] {
        for directory in [
            None,
            Some((404, "")),
            Some((200, r#"{"data":[]}"#)),
            Some((200, r#"{"data":[{"id":"another-model"},{"bad":true}]}"#)),
        ] {
            let server = ControlledServer::start(
                directory
                    .map(|(status, body)| vec![(status, Vec::new(), body.into())])
                    .unwrap_or_default(),
            );
            let service = NativeModelConnectionServiceV1::new(
                TestOnlyComputeCandidatePort::new(),
                ReqwestModelDirectoryTransportV1,
            );
            let mut input = draft(
                server.base_url.clone(),
                GatewayAuthenticationSemanticsV1::None,
            );
            if directory.is_none() {
                input.inventory_path_override = None;
            }
            if registered {
                input.provenance = NativeConnectionProvenanceInputV1::Registered {
                    connection_option_id: "fixture.api".into(),
                    registry_version: "current".into(),
                    catalog_digest: hiroute_domain::CanonicalDigest::of_bytes(b"fixture"),
                };
                input.models = vec![registered_model("manual-model")];
            } else {
                input.models[0].capabilities = NativeModelCapabilityDeclarationV1::default();
                input.models.push(model("second-model"));
            }
            let result = service
                .check(
                    input,
                    NativeModelConnectionCredentialV1::NotRequired,
                    &ModelConnectionProbeCancellationV1::default(),
                )
                .unwrap();
            let model = result
                .candidate
                .models
                .iter()
                .find(|model| model.upstream_model_id == "manual-model")
                .unwrap();
            assert!(model.selectable, "{registered:?} {directory:?}");
            assert_eq!(result.inference, ModelConnectionInferenceStatusV1::NotRun);
            assert_eq!(
                result.authentication,
                ModelConnectionAuthenticationStatusV1::NotRequired
            );
            assert_eq!(server.finish().len(), usize::from(directory.is_some()));
        }
    }
}

#[test]
fn explicit_inference_verifies_tool_call_for_one_model_without_directory_probe() {
    for (protocol, path, body) in [
        (
            UpstreamProtocol::Responses,
            "/v1/responses",
            r#"{"output":[{"type":"function_call","call_id":"call1","name":"hiroute_probe","arguments":"{\"value\":\"ok\"}"}]}"#,
        ),
        (
            UpstreamProtocol::ChatCompletions,
            "/v1/chat/completions",
            r#"{"choices":[{"message":{"tool_calls":[{"type":"function","id":"call1","function":{"name":"hiroute_probe","arguments":"{\"value\":\"ok\"}"}}]}}]}"#,
        ),
        (
            UpstreamProtocol::Messages,
            "/v1/messages",
            r#"{"content":[{"type":"tool_use","id":"call1","name":"hiroute_probe","input":{"value":"ok"}}]}"#,
        ),
    ] {
        let server = ControlledServer::start(vec![(200, Vec::new(), body.into())]);
        let service = NativeModelConnectionServiceV1::new(
            TestOnlyComputeCandidatePort::new(),
            ReqwestModelDirectoryTransportV1,
        );
        let mut input = draft(
            server.base_url.clone(),
            GatewayAuthenticationSemanticsV1::None,
        );
        input.protocol = protocol;
        input.request_path_override = Some(path.into());
        input.inference_model_id = Some("manual-model".into());
        let result = service
            .check(
                input,
                NativeModelConnectionCredentialV1::NotRequired,
                &ModelConnectionProbeCancellationV1::default(),
            )
            .unwrap();
        assert_eq!(result.inference, ModelConnectionInferenceStatusV1::Verified);
        assert_eq!(result.directory, ModelConnectionDirectoryStatusV1::NotRun);
        let requests = server.finish();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with(&format!("POST {path} HTTP/1.1")));
        let body: serde_json::Value =
            serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["model"], "manual-model");
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert!(
            body.get("tool_choice").is_none(),
            "thinking providers reject forced tool selection"
        );
        let messages = if protocol == UpstreamProtocol::Responses {
            &body["input"]
        } else {
            &body["messages"]
        };
        assert_eq!(messages.as_array().unwrap().len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert!(
            messages[0]["content"]
                .as_str()
                .unwrap()
                .contains("hiroute_probe")
        );
        assert!(body.get("enable_thinking").is_none());
    }
}

#[test]
fn failed_inference_does_not_become_a_save_ready_candidate() {
    for (status, body) in [
        (401, "secret-echo-canary"),
        (500, "error"),
        (200, "{}"),
        (302, ""),
    ] {
        let server = ControlledServer::start(vec![(status, Vec::new(), body.into())]);
        let service = NativeModelConnectionServiceV1::new(
            TestOnlyComputeCandidatePort::new(),
            ReqwestModelDirectoryTransportV1,
        );
        let mut input = draft(
            server.base_url.clone(),
            GatewayAuthenticationSemanticsV1::None,
        );
        input.inference_model_id = Some("manual-model".into());
        let result = service
            .check(
                input,
                NativeModelConnectionCredentialV1::NotRequired,
                &ModelConnectionProbeCancellationV1::default(),
            )
            .unwrap();
        assert_eq!(result.inference, ModelConnectionInferenceStatusV1::Failed);
        assert!(!result.candidate.models.iter().any(|model| model.selectable));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("secret-echo-canary")
        );
        assert_eq!(server.finish().len(), 1);
    }
}

#[test]
fn text_only_or_invalid_tool_calls_cannot_claim_agent_tool_support() {
    for (protocol, path, bodies) in [
        (
            UpstreamProtocol::Responses,
            "/v1/responses",
            vec![
                r#"{"output":[{"content":[{"type":"output_text","text":"OK"}]}]}"#,
                r#"{"output":[{"type":"function_call","call_id":"1","name":"hiroute_probe","arguments":"not json"}]}"#,
            ],
        ),
        (
            UpstreamProtocol::ChatCompletions,
            "/v1/chat/completions",
            vec![
                r#"{"choices":[{"message":{"content":"OK"}}]}"#,
                r#"{"choices":[{"message":{"tool_calls":[{"type":"function","id":"1","function":{"name":"other","arguments":"{\"value\":\"ok\"}"}}]}}]}"#,
            ],
        ),
        (
            UpstreamProtocol::Messages,
            "/v1/messages",
            vec![
                r#"{"content":[{"type":"text","text":"OK"}]}"#,
                r#"{"content":[{"type":"tool_use","id":"1","name":"hiroute_probe","input":{"value":"wrong"}}]}"#,
            ],
        ),
    ] {
        for body in bodies {
            let server = ControlledServer::start(vec![(200, Vec::new(), body.into())]);
            let service = NativeModelConnectionServiceV1::new(
                TestOnlyComputeCandidatePort::new(),
                ReqwestModelDirectoryTransportV1,
            );
            let mut input = draft(
                server.base_url.clone(),
                GatewayAuthenticationSemanticsV1::None,
            );
            input.protocol = protocol;
            input.request_path_override = Some(path.into());
            input.inference_model_id = Some("manual-model".into());
            let result = service
                .check(
                    input,
                    NativeModelConnectionCredentialV1::NotRequired,
                    &ModelConnectionProbeCancellationV1::default(),
                )
                .unwrap();
            assert_eq!(result.inference, ModelConnectionInferenceStatusV1::Failed);
            assert!(
                result
                    .issues
                    .iter()
                    .any(|issue| issue.code == "TOOL_CALL_NOT_VERIFIED")
            );
            assert!(!result.candidate.models.iter().any(|model| model.selectable));
            assert_eq!(server.finish().len(), 1);
        }
    }
}
