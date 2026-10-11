#![cfg(unix)]
use hiroute_application_api::*;
use hiroute_client_core::{Client, FailureCode, LocalEndpoint, SubmissionState};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

fn setup() -> (tempfile::TempDir, UnixListener, Client) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("hiroute");
    std::fs::create_dir(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let endpoint = directory.join("control.sock");
    let listener = UnixListener::bind(&endpoint).unwrap();
    std::fs::set_permissions(endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
    let client = Client::new(
        "test",
        LocalEndpoint::for_child(
            std::fs::canonicalize(root.path()).unwrap(),
            std::process::id(),
        ),
    );
    (root, listener, client)
}
fn request() -> LocalControlWireRequestV2 {
    LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "request-1".into(),
        operation_id: "GetSystemStatus".into(),
        payload: json!({}),
        protected_grant: None,
    }
}
async fn write(stream: &mut UnixStream, value: &impl serde::Serialize) {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    stream.write_all(&bytes).await.unwrap();
}
async fn handshake(listener: UnixListener) -> BufReader<UnixStream> {
    let (stream, _) = listener.accept().await.unwrap();
    let mut stream = BufReader::new(stream);
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    let hello: ClientHelloV1 = serde_json::from_str(&line).unwrap();
    write(stream.get_mut(), &negotiate_hello(&hello).unwrap()).await;
    stream
}
async fn receive(stream: &mut BufReader<UnixStream>) -> LocalControlWireRequestV2 {
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}
#[tokio::test]
async fn real_call_records_stages_and_outcome_without_wire_content() {
    use hiroute_diagnostics::event::ProcessRole;
    use hiroute_diagnostics::level::DiagnosticLevel;
    use hiroute_diagnostics::record::Component;
    use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

    let (_root, listener, client) = setup();
    let diagnostics = tempfile::tempdir().unwrap();
    // Diagnostic roots must stay private even under a group-writable caller umask.
    std::fs::set_permissions(diagnostics.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = diagnostics.path().join("d");
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Desktop,
        component: Component::ClientCore,
        parent_session_id: None,
        // The test asserts Debug detail; production defaults to info.
        level_override: Some(DiagnosticLevel::Debug),
    });
    let client = client.with_diagnostics(runtime.handle().root_context());
    let sentinel = "wire-sentinel-6c31f0";
    let request = LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: sentinel.into(),
        operation_id: "DesktopSnapshot".into(),
        payload: json!({"private": "business-payload"}),
        protected_grant: None,
    };
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        let request = receive(&mut stream).await;
        write(
            stream.get_mut(),
            &MachineEnvelopeV2::succeeded(json!({"ok": true}), Some(request.request_id)),
        )
        .await;
    });
    assert!(client.call_wire(request).await.is_ok());
    server.await.unwrap();
    runtime.shutdown();
    let log = std::fs::read_to_string(root.join("desktop").join("current.jsonl")).unwrap();
    for stage in [
        "endpoint_validate",
        "peer_verify",
        "hello",
        "request_write",
        "response_read",
    ] {
        assert!(log.contains(stage), "{stage} is missing: {log}");
    }
    assert!(log.contains("\"operation\":\"desktop_snapshot\""));
    assert!(log.contains("\"submission\":\"sent\""));
    assert_eq!(log.matches("\"control_call_end\":").count(), 1);
    assert!(!log.contains(sentinel), "raw request id leaked");
    assert!(!log.contains("business-payload"), "wire payload leaked");
    assert!(!log.contains("GetSystemStatus"));
}

#[tokio::test]
async fn pre_send_failure_is_not_reported_as_sent() {
    use hiroute_diagnostics::event::ProcessRole;
    use hiroute_diagnostics::level::DiagnosticLevel;
    use hiroute_diagnostics::record::Component;
    use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

    let (root, listener, _client) = setup();
    drop(listener); // No listener: the connect attempt fails before any write.
    let diagnostics = tempfile::tempdir().unwrap();
    // Diagnostic roots must stay private even under a group-writable caller umask.
    std::fs::set_permissions(diagnostics.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let diagnostics_root = diagnostics.path().join("d");
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: ProcessRole::Desktop,
        component: Component::ClientCore,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Debug),
    });
    let client = _client.with_diagnostics(runtime.handle().root_context());
    let failure = client.call_wire(request()).await.unwrap_err();
    assert_eq!(failure.code, FailureCode::TransportUnavailable);
    assert_eq!(failure.submission, SubmissionState::NotSent);
    runtime.shutdown();
    let log =
        std::fs::read_to_string(diagnostics_root.join("desktop").join("current.jsonl")).unwrap();
    assert!(log.contains("\"submission\":\"not_sent\""));
    assert!(log.contains("transport_unavailable"));
    assert!(log.contains("\"sent\":false"));
    assert!(!log.contains("request-1"));
    drop(root);
}

#[tokio::test]
async fn real_socket_preserves_business_rejection_and_request_identity() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        let request = receive(&mut stream).await;
        let mut response: MachineEnvelopeV2<Value> = MachineEnvelopeV2::failed(
            ErrorV1::new(ErrorCode::RevisionConflict),
            Some(request.request_id),
        );
        response.warnings.push(WarningV1 {
            code: "kept".into(),
            details_schema: "test/v1".into(),
        });
        response.next_actions.push(NextActionV1 {
            command_id: "routing.preview".into(),
            input: json!({"id":"plan/one"}),
            reason_code: "stale".into(),
        });
        write(stream.get_mut(), &response).await;
    });
    let response = client.call_wire(request()).await.unwrap();
    assert_eq!(response.error.unwrap().code, ErrorCode::RevisionConflict);
    assert_eq!(response.warnings[0].code, "kept");
    assert_eq!(response.next_actions[0].command_id, "routing.preview");
    server.await.unwrap();
}

#[tokio::test]
async fn observation_session_correlation_survives_typed_transport() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        let request = receive(&mut stream).await;
        assert_eq!(request.operation_id, "ListSessions");
        let page = ObservationSessionPageV2 {
            sessions: vec![ObservationSessionSummaryV2 {
                session_id: "session/one".into(),
                agent_id: "agent/one".into(),
                first_request_at_ms: 1,
                last_request_at_ms: 2,
                request_count: 2,
                fallback_request_count: 0,
                unknown_model_request_count: 0,
                content_completeness: serde_json::from_value(json!("partial")).unwrap(),
                correlation_kind: ObservationSessionCorrelationKindV1::VerifiedWorker,
            }],
            next_cursor: None,
        };
        write(
            stream.get_mut(),
            &MachineEnvelopeV2::succeeded(page, Some(request.request_id)),
        )
        .await;
    });
    let query =
        ObservationReadRequestV2::new(ObservationReadIntentV2::Sessions(ObservationRequestQuery {
            from_ms: 0,
            to_ms: 10,
            session_id: None,
            request_id: None,
            limit: 10,
            cursor: None,
            agent_id: None,
            plan_id: None,
            native_model: None,
            outcome: None,
            only_model_switch: false,
        }));
    let response = client
        .observation_read::<ObservationSessionPageV2>(
            "observation-correlation",
            query,
            ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Desktop,
                capability: "test-capability".into(),
            },
        )
        .await
        .unwrap();
    let summary = &response.data.unwrap().sessions[0];
    assert_eq!(
        summary.correlation_kind,
        ObservationSessionCorrelationKindV1::VerifiedWorker
    );
    assert_eq!(
        serde_json::to_value(summary.content_completeness).unwrap(),
        json!("partial")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn registered_model_check_uses_the_protected_closed_operation() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        let request = receive(&mut stream).await;
        assert_eq!(
            request.operation_id,
            CHECK_REGISTERED_MODEL_CONNECTION_OPERATION_V1
        );
        assert_eq!(
            request
                .protected_grant
                .as_ref()
                .map(|grant| grant.principal_kind),
            Some(PrincipalKind::Desktop)
        );
        assert_eq!(
            request.payload["connection_option_id"],
            "bailian.payg.cn.v1"
        );
        assert!(request.payload.get("base_url").is_none());
        let mut error = ErrorV1::new(ErrorCode::ResourceNotFound);
        error.message_key = "compute.registered_option_unavailable".into();
        write(
            stream.get_mut(),
            &MachineEnvelopeV2::<Value>::failed(error, Some(request.request_id)),
        )
        .await;
    });
    let digest = |value: &[u8]| CanonicalDigest::of_bytes(value);
    let response = client
        .check_registered_model_connection(
            "registered-model-check",
            RegisteredModelConnectionCheckRequestV1 {
                inference_model_id: None,
                models: Vec::new(),
                connection_option_id: "bailian.payg.cn.v1".into(),
                expected_catalog: ComputeCatalogProvenanceViewV1 {
                    product_release: "mvp-current".into(),
                    catalog_binding_id: "fixture-catalog".into(),
                    release_sequence: 1,
                    connector_registry_digest: digest(b"registry"),
                    model_data_digest: digest(b"models"),
                    cross_reference_digest: digest(b"cross"),
                },
                candidate_ref: None,
                lineage_ref: "lineage/bailian".into(),
                edit_revision: 1,
                check_id: "check/bailian/1".into(),
                input_candidate: ComputeCandidateRefV2 {
                    candidate_ref: "candidate/native/bailian".into(),
                    candidate_revision: 1,
                },
                existing_source_id: None,
                expected_source_revision: None,
            },
            ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Desktop,
                capability: "registered-check-capability".into(),
            },
        )
        .await
        .unwrap();
    let error = response.error.unwrap();
    assert_eq!(error.code, ErrorCode::ResourceNotFound);
    assert_eq!(error.message_key, "compute.registered_option_unavailable");
    server.await.unwrap();
}

#[tokio::test]
async fn saved_model_check_uses_only_protected_source_correlation() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        let request = receive(&mut stream).await;
        assert_eq!(
            request.operation_id,
            CHECK_SAVED_MODEL_CONNECTION_OPERATION_V1
        );
        assert_eq!(
            request
                .protected_grant
                .as_ref()
                .map(|grant| grant.principal_kind),
            Some(PrincipalKind::Desktop)
        );
        assert_eq!(
            request.payload,
            json!({
                "source_id": "source/native",
                "expected_source_revision": 7,
                "candidate_ref": "candidate/native",
                "edit_revision": 8,
                "check_id": "check/saved/8",
            })
        );
        let mut error = ErrorV1::new(ErrorCode::RevisionConflict);
        error.message_key = "compute.saved_source_mismatch".into();
        write(
            stream.get_mut(),
            &MachineEnvelopeV2::<Value>::failed(error, Some(request.request_id)),
        )
        .await;
    });
    let response = client
        .check_saved_model_connection(
            "saved-model-check",
            SavedModelConnectionCheckRequestV1 {
                source_id: "source/native".into(),
                expected_source_revision: 7,
                candidate_ref: Some("candidate/native".into()),
                edit_revision: 8,
                check_id: "check/saved/8".into(),
            },
            ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Desktop,
                capability: "saved-check-capability".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(response.error.unwrap().code, ErrorCode::RevisionConflict);
    server.await.unwrap();
}

#[tokio::test]
async fn worker_executor_availability_is_an_empty_local_trust_query() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        let request = receive(&mut stream).await;
        assert_eq!(
            request.operation_id,
            WORKER_EXECUTOR_AVAILABILITY_OPERATION_V1
        );
        assert_eq!(request.payload, json!({}));
        assert!(request.protected_grant.is_none());
        write(
            stream.get_mut(),
            &MachineEnvelopeV2::succeeded(
                json!({
                    "schema": WORKER_EXECUTOR_AVAILABILITY_SCHEMA_V1,
                    "executors": [
                        {
                            "harness": "codex_cli",
                            "state": "unavailable",
                            "reason": "installation_not_configured",
                            "start_approve_all": {"state":"unavailable","reason":"installation_not_configured"},
                            "cancel": {"state":"unavailable","reason":"installation_not_configured"},
                            "continue_session": {"state":"unavailable","reason":"installation_not_configured"},
                            "restricted_policy": {"state":"unavailable","reason":"installation_not_configured"}
                        },
                        {
                            "harness": "claude_code",
                            "state": "unknown",
                            "reason": "runtime_unavailable",
                            "start_approve_all": {"state":"unknown","reason":"runtime_unavailable"},
                            "cancel": {"state":"unknown","reason":"runtime_unavailable"},
                            "continue_session": {"state":"unknown","reason":"runtime_unavailable"},
                            "restricted_policy": {"state":"unknown","reason":"runtime_unavailable"}
                        },
                        {
                            "harness": "qoder_cli",
                            "state": "ready",
                            "start_approve_all": {"state":"ready"},
                            "cancel": {"state":"ready"},
                            "continue_session": {"state":"unknown","reason":"capability_unverified"},
                            "restricted_policy": {"state":"unknown","reason":"restricted_policy_unverified"}
                        },
                        {
                            "harness": "pi",
                            "state": "unavailable",
                            "reason": "installation_not_configured",
                            "start_approve_all": {"state":"unavailable","reason":"installation_not_configured"},
                            "cancel": {"state":"unavailable","reason":"installation_not_configured"},
                            "continue_session": {"state":"unavailable","reason":"installation_not_configured"},
                            "restricted_policy": {"state":"unavailable","reason":"installation_not_configured"}
                        },
                        {
                            "harness": "deepseek_harness",
                            "state": "unavailable",
                            "reason": "installation_not_configured",
                            "start_approve_all": {"state":"unavailable","reason":"installation_not_configured"},
                            "cancel": {"state":"unavailable","reason":"installation_not_configured"},
                            "continue_session": {"state":"unavailable","reason":"installation_not_configured"},
                            "restricted_policy": {"state":"unavailable","reason":"installation_not_configured"}
                        },
                    ]
                }),
                Some(request.request_id),
            ),
        )
        .await;
    });
    let response = client
        .worker_executor_availability("worker-executor-availability")
        .await
        .unwrap()
        .data
        .unwrap();
    assert!(response.valid());
    assert_eq!(
        response
            .executors
            .iter()
            .map(|executor| executor.harness)
            .collect::<Vec<_>>(),
        [
            WorkerHarnessV1::CodexCli,
            WorkerHarnessV1::ClaudeCode,
            WorkerHarnessV1::QoderCli,
            WorkerHarnessV1::Pi,
            WorkerHarnessV1::DeepseekHarness
        ]
    );
    server.await.unwrap();
}

#[tokio::test]
async fn mismatched_response_is_unknown_not_a_failed_operation() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        receive(&mut stream).await;
        write(
            stream.get_mut(),
            &MachineEnvelopeV2::succeeded(json!({}), Some("other-request".into())),
        )
        .await;
    });
    let error = client.call_wire(request()).await.unwrap_err();
    assert_eq!(error.code, FailureCode::ResponseMismatch);
    assert_eq!(error.submission, SubmissionState::MayHaveReachedServer);
    server.await.unwrap();
}
#[tokio::test]
async fn incompatible_hello_does_not_send_business_payload() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        let mut hello =
            negotiate_hello(&serde_json::from_str::<ClientHelloV1>(&line).unwrap()).unwrap();
        hello.api_version = SchemaVersion::new(3, 0);
        write(stream.get_mut(), &hello).await;
        line.clear();
        assert_eq!(stream.read_line(&mut line).await.unwrap(), 0);
    });
    let error = client.call_wire(request()).await.unwrap_err();
    assert_eq!(error.code, FailureCode::SchemaIncompatible);
    assert_eq!(error.submission, SubmissionState::NotSent);
    server.await.unwrap();
}
#[tokio::test]
async fn success_envelope_cannot_replace_handshake() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 1024];
        use tokio::io::AsyncReadExt;
        let bytes_read = stream.read(&mut buffer).await.unwrap();
        assert!(bytes_read > 0);
        write(
            &mut stream,
            &MachineEnvelopeV2::succeeded(json!({"fake":true}), None),
        )
        .await;
    });
    let error = client.call_wire(request()).await.unwrap_err();
    assert_eq!(error.code, FailureCode::FrameInvalid);
    assert_eq!(error.submission, SubmissionState::NotSent);
    server.await.unwrap();
}
#[tokio::test]
async fn slow_trickle_has_one_absolute_deadline() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        receive(&mut stream).await;
        loop {
            if stream.get_mut().write_all(b"x").await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let started = std::time::Instant::now();
    let error = client
        .with_timeout(Duration::from_millis(100))
        .call_wire(request())
        .await
        .unwrap_err();
    assert_eq!(error.code, FailureCode::Deadline);
    assert_eq!(error.submission, SubmissionState::MayHaveReachedServer);
    assert!(started.elapsed() < Duration::from_secs(1));
    server.abort();
}
#[tokio::test]
async fn oversized_response_is_bounded() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        receive(&mut stream).await;
        let _ = stream
            .get_mut()
            .write_all(&vec![
                b'x';
                hiroute_application_api::LOCAL_CONTROL_MAX_FRAME_BYTES
                    + 1
            ])
            .await;
    });
    let error = client.call_wire(request()).await.unwrap_err();
    assert_eq!(error.code, FailureCode::FrameTooLarge);
    server.await.unwrap();
}
#[tokio::test]
async fn oversized_request_is_rejected_before_sending_it() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let mut stream = handshake(listener).await;
        let mut line = String::new();
        assert_eq!(stream.read_line(&mut line).await.unwrap(), 0);
    });
    let mut request = request();
    request.payload = json!({
        "large": "x".repeat(hiroute_application_api::LOCAL_CONTROL_MAX_FRAME_BYTES)
    });
    let error = client.call_wire(request).await.unwrap_err();
    assert_eq!(error.code, FailureCode::FrameTooLarge);
    assert_eq!(error.submission, SubmissionState::NotSent);
    server.await.unwrap();
}
#[tokio::test]
async fn desktop_grant_requires_native_child_identity() {
    let (root, _listener, _client) = setup();
    let client = Client::new(
        "hiroute-desktop",
        LocalEndpoint::from_runtime_root(root.path()),
    );
    let mut request = request();
    request.protected_grant = Some(ProtectedClientGrantV2 {
        principal_kind: PrincipalKind::Desktop,
        capability: "do-not-print-me".into(),
    });
    let error = client.call_wire(request).await.unwrap_err();
    assert_eq!(error.code, FailureCode::PeerRejected);
    assert_eq!(error.submission, SubmissionState::NotSent);
    assert!(!format!("{error:?}").contains("do-not-print-me"));
}
#[tokio::test]
async fn substituted_peer_and_symlinked_runtime_are_rejected() {
    let (root, _listener, _client) = setup();
    let client = Client::new(
        "desktop",
        LocalEndpoint::for_child(root.path(), std::process::id() + 1),
    );
    assert_eq!(
        client.call_wire(request()).await.unwrap_err().code,
        FailureCode::PeerRejected
    );
    let other = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(root.path(), other.path().join("linked")).unwrap();
    let client = Client::new(
        "desktop",
        LocalEndpoint::from_runtime_root(other.path().join("linked")),
    );
    assert_eq!(
        client.call_wire(request()).await.unwrap_err().code,
        FailureCode::PeerRejected
    );
}

#[tokio::test]
async fn malformed_and_truncated_responses_never_report_success() {
    for bytes in [b"{invalid}\n".as_slice(), b"{\"schema_version\":", b""] {
        let (_root, listener, client) = setup();
        let bytes = bytes.to_vec();
        let server = tokio::spawn(async move {
            let mut stream = handshake(listener).await;
            receive(&mut stream).await;
            stream.get_mut().write_all(&bytes).await.unwrap();
        });
        let failure = client.call_wire(request()).await.unwrap_err();
        assert_eq!(failure.submission, SubmissionState::MayHaveReachedServer);
        assert!(matches!(
            failure.code,
            FailureCode::FrameInvalid | FailureCode::TransportUnavailable
        ));
        server.await.unwrap();
    }
}
#[tokio::test]
async fn missing_required_hello_capability_sends_no_request() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        let hello: ClientHelloV1 = serde_json::from_str(&line).unwrap();
        let mut hello = negotiate_hello(&hello).unwrap();
        hello.capabilities.retain(|c| c != "local-control-v2");
        write(stream.get_mut(), &hello).await;
        line.clear();
        assert_eq!(stream.read_line(&mut line).await.unwrap(), 0);
    });
    let failure = client.call_wire(request()).await.unwrap_err();
    assert_eq!(failure.code, FailureCode::SchemaIncompatible);
    assert_eq!(failure.submission, SubmissionState::NotSent);
    server.await.unwrap();
}

#[tokio::test]
async fn client_access_extension_is_required_before_sending_new_queries() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        let mut hello =
            negotiate_hello(&serde_json::from_str::<ClientHelloV1>(&line).unwrap()).unwrap();
        hello.capabilities.retain(|c| c != "client-access-v1");
        write(stream.get_mut(), &hello).await;
        line.clear();
        assert_eq!(stream.read_line(&mut line).await.unwrap(), 0);
    });
    let mut request = request();
    request.operation_id = "GetClientServiceStatus".into();
    let failure = client.call_wire(request).await.unwrap_err();
    assert_eq!(failure.code, FailureCode::SchemaIncompatible);
    assert_eq!(failure.submission, SubmissionState::NotSent);
    server.await.unwrap();
}
#[tokio::test]
async fn handshake_stall_times_out_before_submission() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let failure = client
        .with_timeout(Duration::from_millis(50))
        .call_wire(request())
        .await
        .unwrap_err();
    assert_eq!(failure.code, FailureCode::Deadline);
    assert_eq!(failure.submission, SubmissionState::NotSent);
    server.abort();
}
#[tokio::test]
async fn write_backpressure_uses_the_same_bounded_submission_deadline() {
    let (_root, listener, client) = setup();
    let server = tokio::spawn(async move {
        let _stream = handshake(listener).await;
        std::future::pending::<()>().await;
    });
    let mut request = request();
    request.payload = json!({"large":"x".repeat(900_000)});
    let failure = client
        .with_timeout(Duration::from_millis(200))
        .call_wire(request)
        .await
        .unwrap_err();
    assert_eq!(failure.code, FailureCode::Deadline);
    assert_eq!(failure.submission, SubmissionState::MayHaveReachedServer);
    server.abort();
}
#[tokio::test]
async fn writable_directory_or_socket_keeps_handshake_and_child_pid_checks() {
    for directory in [true, false] {
        let (root, listener, client) = setup();
        let path = if directory {
            root.path().join("hiroute")
        } else {
            client.endpoint().path().to_owned()
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o777)).unwrap();
        let server = tokio::spawn(async move {
            let mut stream = handshake(listener).await;
            let request = receive(&mut stream).await;
            write(
                stream.get_mut(),
                &MachineEnvelopeV2::succeeded(json!({"ok":true}), Some(request.request_id)),
            )
            .await;
        });
        assert!(client.call_wire(request()).await.is_ok());
        server.await.unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o777
        );
    }
}

#[tokio::test]
async fn management_query_versions_use_distinct_operations_over_the_same_handshake() {
    for (source_id, operation_v2, operation_v3) in [
        (None, "ListCompute", "ListComputeV3"),
        (Some("source/test".to_owned()), "GetCompute", "GetComputeV3"),
    ] {
        for v3 in [false, true] {
            let (_root, listener, client) = setup();
            let expected_operation = if v3 { operation_v3 } else { operation_v2 };
            let expected_source = source_id.clone();
            let server = tokio::spawn(async move {
                let mut stream = handshake(listener).await;
                let request = receive(&mut stream).await;
                assert_eq!(request.operation_id, expected_operation);
                assert_eq!(request.schema_version, LOCAL_CONTROL_SCHEMA_V2);
                let query: ComputeManagementQueryV2 =
                    serde_json::from_value(request.payload).unwrap();
                assert_eq!(query.source_id, expected_source);
                let mut response = json!({"schema": if v3 { "hiroute.compute-management-snapshot/v3" } else { "hiroute.compute-management-snapshot/v2" }, "revisions":{"target":1,"dependencies":{}}, "runtime_state":"complete", "sources":[]});
                if v3 {
                    response["subscription_modes"] = json!([]);
                }
                write(
                    stream.get_mut(),
                    &MachineEnvelopeV2::succeeded(response, Some(request.request_id)),
                )
                .await;
            });
            let query = ComputeManagementQueryV2 {
                source_id: source_id.clone(),
            };
            if v3 {
                let result = client
                    .compute_management_snapshot_v3("v3-query", query)
                    .await
                    .unwrap()
                    .data
                    .unwrap();
                assert_eq!(result.schema, COMPUTE_MANAGEMENT_SNAPSHOT_SCHEMA_V3);
                assert!(result.subscription_modes.is_empty());
            } else {
                let result = client
                    .compute_management_snapshot("v2-query", query)
                    .await
                    .unwrap()
                    .data
                    .unwrap();
                assert_eq!(result.schema, COMPUTE_MANAGEMENT_SNAPSHOT_SCHEMA_V2);
            }
            server.await.unwrap();
        }
    }
}
