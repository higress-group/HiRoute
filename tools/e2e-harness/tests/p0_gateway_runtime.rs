mod runtime_support;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use hiroute_gateway::attempt_outcome::{
    AttemptFailureClass, ConnectorErrorProfile, FailureStateScope, ProviderFailureKind,
    RawAttemptFailure, classify_failure,
};

use runtime_support::*;

const RESPONSES_OK: &[u8] = br#"{"id":"accepted","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
const RESPONSES_TOOL_CALL: &[u8] = br#"{"id":"tool-call","model":"runtime-native","status":"completed","output":[{"type":"function_call","id":"fc-native","call_id":"native-call","namespace":"tools","name":"lookup","arguments":"{}","status":"completed"}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
const RESPONSES_STREAM_OK: &[u8] = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"stream-ok\",\"model\":\"runtime-native\"}}\n\nevent: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"runtime-message\",\"output_index\":0,\"content_index\":0,\"delta\":\"ok\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"stream-ok\",\"model\":\"runtime-native\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"id\":\"runtime-message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"ok\",\"annotations\":[]}]}],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n";
const FALLBACK_OK: &[u8] = br#"{"id":"accepted-fallback","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"fallback"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
const CLASSIFIER_COMPLEX: &[u8] = br#"{"branch_id":"smart_saving_complex"}"#;
const CLASSIFIER_SIMPLE: &[u8] = br#"{"branch_id":"smart_saving_simple"}"#;
const CLASSIFIER_SIMPLE_WITH_ASSESSMENT: &[u8] =
    br#"{"branch_id":"smart_saving_simple","assessment":{"score":0.75,"partial":false}}"#;
const CLASSIFIER_INVALID: &[u8] = br#"{"id":"classification","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"{\"category\":\"simple\"}"}]}]}"#;
const AUTH_ERROR: &[u8] =
    br#"{"error":{"type":"invalid_api_key","message":"private auth marker"}}"#;
const QUOTA_ERROR: &[u8] =
    br#"{"error":{"type":"insufficient_quota","message":"private quota marker"}}"#;
const OVERLOAD_ERROR: &[u8] =
    br#"{"error":{"type":"rate_limit_exceeded","message":"private overload marker"}}"#;
const PROTOCOL_ERROR: &[u8] =
    br#"{"error":{"type":"protocol_error","message":"private protocol marker"}}"#;
const PERMANENT_ERROR: &[u8] =
    br#"{"error":{"type":"invalid_request_error","message":"private permanent marker"}}"#;

#[path = "p0_gateway_runtime/reasoning.rs"]
mod reasoning;

#[path = "p0_gateway_runtime/accepted_history.rs"]
mod accepted_history;

#[test]
fn fixed_requests_retry_only_same_source_keys_and_never_enter_an_allowed_plan() {
    for (status, error) in [(401, AUTH_ERROR), (403, AUTH_ERROR), (429, QUOTA_ERROR)] {
        let primary = NativeProvider::start(vec![
            ProviderReply::Complete {
                status,
                error_kind: None,
                body: error,
            },
            ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: RESPONSES_OK,
            },
        ]);
        let other_account = NativeProvider::start(vec![ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: FALLBACK_OK,
        }]);
        let other_model = NativeProvider::start(vec![ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: FALLBACK_OK,
        }]);
        let fixture = RuntimeFixture::launch_fixed(
            &[&primary, &other_account, &other_model],
            2,
            &[2, 1, 1],
            None,
        );
        let response = fixture.request();
        assert_eq!(response.status, 200, "status={status}");
        assert_eq!(primary.calls(), 2, "status={status}");
        assert_eq!((other_account.calls(), other_model.calls()), (0, 0));
        let requests = primary.requests();
        for (index, wire) in requests.iter().enumerate() {
            assert_eq!(
                wire_header(wire, "authorization"),
                Some(format!("Bearer provider-secret-1-{}", index + 1))
            );
            assert!(String::from_utf8_lossy(wire).contains("runtime-native-model-1"));
            assert!(wire_header(wire, "x-hiroute-token").is_none());
        }
    }
}

#[path = "p0_gateway_runtime/fixed_isolation.rs"]
mod fixed_isolation;

#[test]
fn real_hirouted_reaches_native_provider_and_falls_back_before_commit() {
    hiroute_e2e::p0_execution_receipt!(
        "runtime.fallback",
        [
            "runtime.binding_fault",
            "runtime.frozen_budget",
            "runtime.precommit_fallback"
        ]
    );
    let first = NativeProvider::start(vec![ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: b"private first-provider error",
    }]);
    let second = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: br#"{"id":"accepted-second","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#,
    }]);
    let fixture = RuntimeFixture::launch(&[&first, &second], 2);

    let response = fixture.request();

    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_ne!(response.status, 501);
    let document: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(document["id"], "accepted-second");
    assert_eq!(document["output"][0]["content"][0]["text"], "ok");
    assert_eq!(first.calls(), 1);
    assert_eq!(second.calls(), 1);
    let first_wire = String::from_utf8_lossy(&first.requests()[0]).to_ascii_lowercase();
    let second_wire = String::from_utf8_lossy(&second.requests()[0]).to_ascii_lowercase();
    assert!(first_wire.contains("authorization: bearer provider-secret-1"));
    assert!(second_wire.contains("authorization: bearer provider-secret-2"));
    assert!(first_wire.contains("post /v1/responses http/1.1"));
    assert!(
        !response
            .body
            .windows(b"private first-provider error".len())
            .any(|window| window == b"private first-provider error")
    );
}

#[test]
fn real_listener_consumes_planner_order_and_binds_reason_ledger_identity() {
    hiroute_e2e::p0_execution_receipt!(
        "planner.runtime_consumption",
        [
            "planner.frozen_order_consumed",
            "planner.reason_ledger_identity"
        ]
    );
    let publication_first = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let planner_first = NativeProvider::start(vec![ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: br#"{"error":{"type":"server_error","message":"retry"}}"#,
    }]);
    let fixture = RuntimeFixture::launch_with_publication_candidates(
        &[&publication_first, &planner_first],
        2,
        Some(&[
            PublicationCandidate {
                provider_index: 1,
                upstream_protocol: "responses",
                statically_enabled: true,
            },
            PublicationCandidate {
                provider_index: 0,
                upstream_protocol: "responses",
                statically_enabled: true,
            },
        ]),
    );

    assert_eq!(fixture.request().status, 200);
    assert_eq!(planner_first.calls(), 1);
    assert_eq!(publication_first.calls(), 1);
    let planner_wire = planner_first.requests();
    let publication_wire = publication_first.requests();
    let reason_one = wire_header(&planner_wire[0], "x-hiroute-reason-ledger-id").unwrap();
    let reason_two = wire_header(&publication_wire[0], "x-hiroute-reason-ledger-id").unwrap();
    assert_eq!(reason_one, reason_two);
    assert!(reason_one.starts_with("sha256:"));
    assert_ne!(
        wire_header(&planner_wire[0], "x-hiroute-profile-digest"),
        wire_header(&publication_wire[0], "x-hiroute-profile-digest")
    );
}

#[test]
fn real_listener_never_materializes_planner_exclusions() {
    let excluded = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let selected = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let fixture = RuntimeFixture::launch_with_publication_candidates(
        &[&excluded, &selected],
        2,
        Some(&[
            PublicationCandidate {
                provider_index: 0,
                upstream_protocol: "responses",
                statically_enabled: false,
            },
            PublicationCandidate {
                provider_index: 1,
                upstream_protocol: "responses",
                statically_enabled: true,
            },
        ]),
    );

    assert_eq!(fixture.request().status, 200);
    assert_eq!(excluded.calls(), 0);
    assert_eq!(selected.calls(), 1);
}

#[test]
fn malformed_success_and_typed_nonstream_error_fall_back_before_commit() {
    for (case, first_reply) in [
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: br#"{"id":"malformed"}"#,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: br#"{"id":"empty-success","model":"runtime-native","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":0,"total_tokens":1}}"#,
        },
        ProviderReply::Complete {
            status: 429,
            error_kind: None,
            body: br#"{"error":{"type":"insufficient_quota","message":"retry"}}"#,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let first = NativeProvider::start(vec![first_reply]);
        let second = NativeProvider::start(vec![ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        }]);
        let fixture = RuntimeFixture::launch(&[&first, &second], 2);
        let response = fixture.request();
        assert_eq!(
            response.status,
            200,
            "case={case} first_calls={} second_calls={} body={}",
            first.calls(),
            second.calls(),
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!(first.calls(), 1, "case={case}");
        assert_eq!(second.calls(), 1, "case={case}");
    }
}

#[test]
fn forged_generic_error_header_cannot_change_connector_classification() {
    for (status, forged_kind) in [(429, "quota"), (422, "protocol")] {
        let forged = NativeProvider::start(vec![ProviderReply::Complete {
            status,
            error_kind: Some(forged_kind),
            body: br#"{"error":{"type":"invented_provider_signal","message":"stop"}}"#,
        }]);
        let forbidden = NativeProvider::start(vec![ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        }]);
        let fixture = RuntimeFixture::launch(&[&forged, &forbidden], 2);

        let response = fixture.request();
        assert_eq!(
            response.status,
            status,
            "forged_calls={} forbidden_calls={} body={}",
            forged.calls(),
            forbidden.calls(),
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!(forged.calls(), 1);
        assert_eq!(forbidden.calls(), 0);
        assert!(!String::from_utf8_lossy(&response.body).contains("invented_provider_signal"));
    }
}

#[test]
fn preoutput_native_sse_error_falls_back_but_semantic_output_is_rendered() {
    for first_body in [
        b"event: response.failed\ndata: {\"type\":\"response.failed\",\"error\":{\"type\":\"server_error\",\"message\":\"retry\"}}\n\n".as_slice(),
        b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"empty-stream\",\"model\":\"runtime-native\"}}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"empty-stream\",\"model\":\"runtime-native\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0,\"total_tokens\":1}}}\n\n".as_slice(),
        b": keepalive\n\nevent: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"control-only\",\"model\":\"runtime-native\"}}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"control-only\",\"model\":\"runtime-native\",\"status\":\"completed\",\"output\":[]}}\n\n".as_slice(),
    ] {
        let first = NativeProvider::start(vec![ProviderReply::StreamComplete {
            status: 200,
            body: first_body,
        }]);
        let second = NativeProvider::start(vec![ProviderReply::StreamComplete {
            status: 200,
            body: RESPONSES_STREAM_OK,
        }]);
        let fixture = RuntimeFixture::launch(&[&first, &second], 2);

        let response =
            fixture.request_body(br#"{"model":"runtime-model","input":"hello","stream":true}"#);
        assert_eq!(response.status, 200);
        assert_eq!(first.calls(), 1);
        assert_eq!(second.calls(), 1);
        assert!(String::from_utf8_lossy(&response.body).contains("response.output_text.delta"));
    }
}

#[test]
fn real_listener_preserves_native_terminal_tail_split_across_provider_writes() {
    let tail = b"event: response.vendor_extension\ndata: opaque\n\n";
    let split = tail.len() / 2;
    let mut first = RESPONSES_STREAM_OK.to_vec();
    first.extend_from_slice(&tail[..split]);
    let provider = NativeProvider::start(vec![ProviderReply::StreamDrip {
        status: 200,
        chunks: vec![first, tail[split..].to_vec()],
        interval: Duration::from_millis(120),
    }]);
    let fixture = RuntimeFixture::launch(&[&provider], 1);
    let response =
        fixture.request_body(br#"{"model":"runtime-model","input":"hello","stream":true}"#);

    assert_eq!(response.status, 200);
    assert_eq!(provider.calls(), 1);
    assert!(response.body.windows(tail.len()).any(|part| part == tail));
    assert!(
        String::from_utf8_lossy(&response.body).contains("event: response.completed"),
        "{}",
        String::from_utf8_lossy(&response.body)
    );
}

#[test]
fn cross_protocol_candidate_uses_native_decoder_and_ingress_renderer() {
    let provider = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: br#"{"id":"chat-native","object":"chat.completion","model":"native-chat","choices":[{"index":0,"message":{"role":"assistant","content":"cross-protocol"},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
    }]);
    let fixture = RuntimeFixture::launch_with_publication_candidates(
        &[&provider],
        1,
        Some(&[PublicationCandidate {
            provider_index: 0,
            upstream_protocol: "chat_completions",
            statically_enabled: true,
        }]),
    );

    let response = fixture.request();
    assert_eq!(response.status, 200);
    let document: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(document["object"], "response");
    assert_eq!(
        document["output"][0]["content"][0]["text"],
        "cross-protocol"
    );
    assert!(
        String::from_utf8_lossy(&provider.requests()[0])
            .to_ascii_lowercase()
            .contains("post /v1/chat/completions http/1.1")
    );
}

#[test]
fn dns_failure_does_not_create_or_consume_the_only_attempt() {
    let dns_candidate = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let reachable = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let fixture = RuntimeFixture::launch_with_endpoints(
        &[&dns_candidate, &reachable],
        1,
        &[
            "http://does-not-resolve.invalid:9".into(),
            format!("http://localhost:{}", reachable.address().port()),
        ],
    );

    assert_eq!(fixture.request().status, 200);
    assert_eq!(dns_candidate.calls(), 0);
    assert_eq!(reachable.calls(), 1);
}

#[test]
fn real_listener_401_and_403_disable_only_the_exact_credential_key() {
    for status in [401, 403] {
        let primary = NativeProvider::start(vec![
            ProviderReply::Complete {
                status,
                error_kind: None,
                body: AUTH_ERROR,
            },
            ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: RESPONSES_OK,
            },
            ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: RESPONSES_OK,
            },
        ]);
        let forbidden_fallback = NativeProvider::start(vec![ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: FALLBACK_OK,
        }]);
        let fixture =
            RuntimeFixture::launch_with_key_counts(&[&primary, &forbidden_fallback], 3, &[2, 1]);

        let first = fixture.request();
        assert_eq!(first.status, 200, "status={status}");
        assert_eq!(primary.calls(), 2, "status={status}");
        assert_eq!(forbidden_fallback.calls(), 0, "status={status}");
        assert!(!String::from_utf8_lossy(&first.body).contains("private auth marker"));

        let second = fixture.request();
        assert_eq!(second.status, 200, "status={status}");
        assert_eq!(primary.calls(), 3, "status={status}");
        assert_eq!(forbidden_fallback.calls(), 0, "status={status}");
        let requests = primary.requests();
        assert_eq!(
            wire_header(&requests[0], "authorization").as_deref(),
            Some("Bearer provider-secret-1-1")
        );
        assert_eq!(
            wire_header(&requests[1], "authorization").as_deref(),
            Some("Bearer provider-secret-1-2")
        );
        assert_eq!(
            wire_header(&requests[2], "authorization").as_deref(),
            Some("Bearer provider-secret-1-2"),
            "the disabled first key must remain skipped on the next request"
        );
    }
}

#[test]
fn real_listener_distinguishes_quota_key_scope_from_overload_binding_scope() {
    hiroute_e2e::p0_execution_receipt!(
        "runtime.key_handoff",
        [
            "runtime.multiple_keys",
            "runtime.quota_key_handoff",
            "runtime.binding_overload"
        ]
    );
    struct Case {
        name: &'static str,
        primary_replies: Vec<ProviderReply>,
        max_attempts: u32,
        primary_after_first: usize,
    }

    for case in [
        Case {
            name: "quota",
            primary_replies: vec![
                ProviderReply::Complete {
                    status: 429,
                    error_kind: None,
                    body: QUOTA_ERROR,
                },
                ProviderReply::Complete {
                    status: 429,
                    error_kind: None,
                    body: QUOTA_ERROR,
                },
            ],
            max_attempts: 3,
            primary_after_first: 2,
        },
        Case {
            name: "binding-overload",
            primary_replies: vec![ProviderReply::Complete {
                status: 429,
                error_kind: None,
                body: OVERLOAD_ERROR,
            }],
            max_attempts: 2,
            primary_after_first: 1,
        },
    ] {
        let primary = NativeProvider::start(case.primary_replies);
        let fallback = NativeProvider::start(vec![
            ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: FALLBACK_OK,
            },
            ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: FALLBACK_OK,
            },
        ]);
        let fixture = RuntimeFixture::launch_with_key_counts(
            &[&primary, &fallback],
            case.max_attempts,
            &[2, 1],
        );

        let first = fixture.request();
        assert_eq!(first.status, 200, "case={}", case.name);
        assert_eq!(
            primary.calls(),
            case.primary_after_first,
            "case={}",
            case.name
        );
        assert_eq!(fallback.calls(), 1, "case={}", case.name);
        assert!(String::from_utf8_lossy(&first.body).contains("accepted-fallback"));

        let second = fixture.request();
        assert_eq!(second.status, 200, "case={}", case.name);
        assert_eq!(
            primary.calls(),
            case.primary_after_first,
            "case={} must remain excluded by its exact state scope",
            case.name
        );
        assert_eq!(fallback.calls(), 2, "case={}", case.name);

        let requests = primary.requests();
        assert_eq!(
            wire_header(&requests[0], "authorization").as_deref(),
            Some("Bearer provider-secret-1-1")
        );
        if case.name == "quota" {
            assert_eq!(
                wire_header(&requests[1], "authorization").as_deref(),
                Some("Bearer provider-secret-1-2"),
                "quota must cool one key and consume the next exact key before candidate fallback"
            );
        }
    }
}

#[test]
fn real_listener_typed_400_and_422_preserve_protocol_and_permanent_boundaries() {
    struct Case {
        name: &'static str,
        status: u16,
        body: &'static [u8],
        second_primary_reply: ProviderReply,
        expected_downstream: u16,
        expected_fallback_calls: usize,
    }

    for case in [
        Case {
            name: "protocol-400",
            status: 400,
            body: PROTOCOL_ERROR,
            second_primary_reply: ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: RESPONSES_OK,
            },
            expected_downstream: 200,
            expected_fallback_calls: 1,
        },
        Case {
            name: "permanent-422",
            status: 422,
            body: PERMANENT_ERROR,
            second_primary_reply: ProviderReply::Complete {
                status: 422,
                error_kind: None,
                body: PERMANENT_ERROR,
            },
            expected_downstream: 422,
            expected_fallback_calls: 0,
        },
    ] {
        let primary = NativeProvider::start(vec![
            ProviderReply::Complete {
                status: case.status,
                error_kind: None,
                body: case.body,
            },
            case.second_primary_reply,
        ]);
        let fallback = NativeProvider::start(vec![ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: FALLBACK_OK,
        }]);
        let fixture = RuntimeFixture::launch(&[&primary, &fallback], 2);

        let first = fixture.request();
        assert_eq!(first.status, case.expected_downstream, "case={}", case.name);
        assert_eq!(primary.calls(), 1, "case={}", case.name);
        assert_eq!(
            fallback.calls(),
            case.expected_fallback_calls,
            "case={}",
            case.name
        );
        assert!(!String::from_utf8_lossy(&first.body).contains("private"));

        let second = fixture
            .request_body(br#"{"model":"runtime-model","input":"hello-rebuilt","stream":false}"#);
        assert_eq!(
            second.status, case.expected_downstream,
            "case={}",
            case.name
        );
        assert_eq!(
            primary.calls(),
            2,
            "case={} must not persist protocol/permanent failures into RuntimeState",
            case.name
        );
        assert_eq!(
            fallback.calls(),
            case.expected_fallback_calls,
            "case={}",
            case.name
        );
    }
}

#[test]
fn real_listener_first_byte_timeout_cools_binding_and_falls_back() {
    let stalled = NativeProvider::start(vec![ProviderReply::Stall {
        duration: Duration::from_millis(500),
    }]);
    let fallback = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: FALLBACK_OK,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: FALLBACK_OK,
        },
    ]);
    let fixture = RuntimeFixture::launch_with_attempt_timeout(
        &[&stalled, &fallback],
        2,
        Duration::from_millis(200),
    );

    let started = Instant::now();
    let first = fixture.request();
    assert_eq!(first.status, 200);
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert_eq!(stalled.calls(), 1);
    assert_eq!(fallback.calls(), 1);

    let second = fixture.request();
    assert_eq!(second.status, 200);
    assert_eq!(stalled.calls(), 1, "timed out binding must be cooling down");
    assert_eq!(fallback.calls(), 2);
}

#[test]
fn real_listener_candidate_exhaustion_returns_a_sanitized_gateway_failure() {
    let first = NativeProvider::start(vec![ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: b"private exhausted first",
    }]);
    let second = NativeProvider::start(vec![ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: b"private exhausted second",
    }]);
    let fixture = RuntimeFixture::launch(&[&first, &second], 2);

    let response = fixture.request();

    assert_eq!(response.status, 503);
    assert_eq!(first.calls(), 1);
    assert_eq!(second.calls(), 1);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap(),
        serde_json::json!({
            "error": {
                "code": "UPSTREAM_ATTEMPT_FAILED",
                "type": "upstream_error",
            }
        })
    );
}

#[test]
fn real_listener_reports_cooling_recovers_once_and_resets_the_backoff_step() {
    let provider = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 503,
            error_kind: None,
            body: b"private transient failure",
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
        ProviderReply::Complete {
            status: 503,
            error_kind: None,
            body: b"private transient failure after recovery",
        },
    ]);
    let fixture = RuntimeFixture::launch(&[&provider], 1);

    assert_eq!(fixture.request().status, 503, "seed the binding cooldown");
    let response = fixture.request();

    assert_eq!(response.status, 503);
    assert_eq!(
        provider.calls(),
        1,
        "cooling candidates must not reach upstream"
    );
    let retry_after = response.headers["retry-after"].parse::<u64>().unwrap();
    assert!((1..=2).contains(&retry_after));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap(),
        serde_json::json!({
            "schema_version": "hiroute.gateway.error/v1",
            "code": "CANDIDATES_COOLING_DOWN",
            "phase": "runtime_state",
            "retry_after_seconds": retry_after,
        })
    );

    std::thread::sleep(Duration::from_millis(2_100));
    assert_eq!(
        fixture.request().status,
        200,
        "the sole recovery probe wins"
    );
    assert_eq!(provider.calls(), 2);

    assert_eq!(
        fixture.request().status,
        503,
        "a new transient failure follows the successful probe"
    );
    let reset_response = fixture.request();
    assert_eq!(
        provider.calls(),
        3,
        "the reset cooldown blocks upstream I/O"
    );
    let reset_retry_after = reset_response.headers["retry-after"]
        .parse::<u64>()
        .unwrap();
    assert!(
        (1..=2).contains(&reset_retry_after),
        "successful recovery must reset the sequence to the two-second first step"
    );
}

#[test]
fn real_listener_rest_classifier_uses_the_existing_gateway_and_selects_complex() {
    let simple = NativeProvider::start(Vec::new());
    let complex = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let classifier = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: CLASSIFIER_COMPLEX,
    }]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&simple, &complex, &classifier],
        2,
        ObservationFaults::healthy(),
    );

    let response = fixture
        .request_body(br#"{"model":"runtime-model","input":"latest-only-marker","stream":false}"#);

    assert_eq!(
        response.status,
        200,
        "body={}, calls={:?}",
        String::from_utf8_lossy(&response.body),
        (simple.calls(), complex.calls(), classifier.calls())
    );
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (0, 1, 1)
    );
    let requests = classifier.requests();
    assert_eq!(
        wire_header(&requests[0], "authorization"),
        Some("Bearer provider-secret-3".into())
    );
    assert!(wire_header(&requests[0], "x-hiroute-token").is_none());
    assert!(String::from_utf8_lossy(&requests[0]).starts_with("POST /v1/decisions HTTP/1.1"));
    let split = requests[0]
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0][split + 4..]).unwrap();
    assert_eq!(
        body.pointer("/latest_user/0/text")
            .and_then(|value| value.as_str()),
        Some("latest-only-marker")
    );
    assert_eq!(body["visible_conversation"], serde_json::json!([]));
    assert_eq!(
        body.pointer("/branches/smart_saving_simple")
            .and_then(|value| value.as_str()),
        Some(hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_DESCRIPTION)
    );
    assert_eq!(
        body.pointer("/branches/smart_saving_complex")
            .and_then(|value| value.as_str()),
        Some(hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_DESCRIPTION)
    );
    assert_eq!(body.as_object().unwrap().len(), 5);
    assert!(body.get("model").is_none());

    let facts = wait_execution_facts(&fixture, 1);
    let main_request_id = facts
        .iter()
        .find(|fact| {
            fact.get("served_model_id")
                .and_then(serde_json::Value::as_str)
                == Some("runtime-model")
        })
        .and_then(|fact| fact.pointer("/correlation/request_id"))
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .to_owned();
    assert!(facts.iter().any(|fact| {
        fact.pointer("/correlation/request_id")
            .and_then(serde_json::Value::as_str)
            == Some(main_request_id.as_str())
            && fact
                .pointer("/fact/complexity/decision_source")
                .and_then(serde_json::Value::as_str)
                == Some("external_classifier")
    }));
    let content = wait_content_for_request(&fixture, &main_request_id);
    assert!(content.iter().any(|record| {
        record
            .pointer("/correlation/request_id")
            .and_then(serde_json::Value::as_str)
            == Some(main_request_id.as_str())
    }));
}

#[test]
fn real_listener_rest_classifier_runs_once_for_each_supported_ingress() {
    let simple = NativeProvider::start(Vec::new());
    let complex = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let classifier = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_COMPLEX,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_COMPLEX,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_COMPLEX,
        },
    ]);
    let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 2);
    let cases = [
        (
            "/v1/responses",
            ("X-HiRoute-Token", TOKEN),
            serde_json::json!({
                "model": "runtime-model",
                "input": "responses classifier input",
                "stream": false,
            }),
            "responses classifier input",
        ),
        (
            "/v1/chat/completions",
            ("Authorization", "Bearer runtime-chat-token"),
            serde_json::json!({
                "model": "runtime-model",
                "messages": [{"role":"user","content":"chat classifier input"}],
                "stream": false,
            }),
            "chat classifier input",
        ),
        (
            "/v1/messages",
            ("Authorization", "Bearer runtime-messages-token"),
            serde_json::json!({
                "model": "runtime-model",
                "max_tokens": 8,
                "messages": [{"role":"user","content":"messages classifier input"}],
                "stream": false,
            }),
            "messages classifier input",
        ),
    ];

    for (path, header, body, _) in &cases {
        let response = request(
            fixture.address,
            "POST",
            path,
            &[*header],
            &serde_json::to_vec(body).unwrap(),
        );
        assert_eq!(
            response.status,
            200,
            "{path}: {}",
            String::from_utf8_lossy(&response.body)
        );
    }

    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (0, 3, 3)
    );
    for (wire, (_, _, _, expected_latest)) in classifier.requests().iter().zip(cases) {
        let split = wire
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&wire[split + 4..]).unwrap();
        assert_eq!(
            body.pointer("/latest_user/0/text")
                .and_then(serde_json::Value::as_str),
            Some(expected_latest)
        );
    }
}

#[test]
fn real_listener_rest_classifier_streams_complete_replay_backed_context_without_truncation() {
    let simple = NativeProvider::start(Vec::new());
    let complex = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let classifier = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: CLASSIFIER_COMPLEX,
    }]);
    let fixture = RuntimeFixture::launch_rest_classified_with_replay(
        &[&simple, &complex, &classifier],
        2,
        256,
        128,
    );
    let instructions = "先前说明".repeat(2_048);
    let latest_user = format!(
        "{} literal-marker=__hiroute_content_ref_v2_0_0_1_1__",
        "分类上下文".repeat(1_024)
    );
    assert!(latest_user.chars().count() >= 4_096);
    let request_body = format!(
        r#"{{"model":"runtime-model","instructions":{},"input":{},"stream":false}}"#,
        serde_json::to_string(&instructions).unwrap(),
        serde_json::to_string(&latest_user).unwrap(),
    )
    .into_bytes();

    let response = fixture.request_body(&request_body);

    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (0, 1, 1)
    );
    let requests = classifier.requests();
    let split = requests[0]
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0][split + 4..]).unwrap();
    assert_eq!(
        body.pointer("/latest_user/0/text"),
        Some(&serde_json::Value::String(latest_user.clone()))
    );
    assert_eq!(body["visible_conversation"], serde_json::json!([]));
    assert!(
        !serde_json::to_string(&body)
            .unwrap()
            .contains(&instructions)
    );
}

#[test]
fn real_listener_rest_classifier_can_select_the_simple_group() {
    let simple = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: CLASSIFIER_SIMPLE,
    }]);
    let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 2);

    let response = fixture.request();

    assert_eq!(response.status, 200);
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (1, 0, 1)
    );
}

#[test]
fn real_listener_rest_classifier_reclassifies_each_decision_boundary() {
    let simple = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let complex = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let classifier = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_COMPLEX,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_SIMPLE,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_COMPLEX,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_SIMPLE,
        },
    ]);
    let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 2);
    let send = |input: serde_json::Value| {
        request(
            fixture.address,
            "POST",
            "/v1/responses",
            &[
                ("X-HiRoute-Token", "runtime-token"),
                ("session-id", "rest-classifier-inheritance"),
            ],
            &serde_json::to_vec(&serde_json::json!({
                "model": "runtime-model",
                "input": input,
                "stream": false,
            }))
            .unwrap(),
        )
    };
    let message = |role: &str, kind: &str, text: &str| {
        serde_json::json!({
            "type": "message",
            "role": role,
            "content": [{"type": kind, "text": text}],
        })
    };

    assert_eq!(
        send(serde_json::json!([message(
            "user",
            "input_text",
            "first task"
        )]))
        .status,
        200
    );
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (0, 1, 1)
    );

    assert_eq!(
        send(serde_json::json!([
            message("user", "input_text", "first task"),
            message("assistant", "output_text", "ok"),
            message("user", "input_text", "continue the same task")
        ]))
        .status,
        200
    );
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (1, 1, 2),
        "an appended user message must not inherit the prior round's branch"
    );

    assert_eq!(
        send(serde_json::json!([message(
            "user",
            "input_text",
            "continue the same task"
        )]))
        .status,
        200
    );
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (1, 2, 3),
        "a rebuilt history must classify again even when latest_user is unchanged"
    );

    assert_eq!(
        send(serde_json::json!([message(
            "user",
            "input_text",
            "Summary: continue the same work"
        )]))
        .status,
        200
    );
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (2, 2, 4),
        "a summary-shaped latest_user must use the same ContextHold decision boundary"
    );
}

#[test]
fn real_listener_context_hold_boundary_seals_unknown_and_scores_the_previous_model() {
    let simple = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let complex = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_TOOL_CALL,
    }]);
    let classifier = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_COMPLEX,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_SIMPLE_WITH_ASSESSMENT,
        },
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&simple, &complex, &classifier],
        2,
        ObservationFaults::healthy(),
    );
    let headers = [
        ("X-HiRoute-Token", "runtime-token"),
        ("session-id", "context-hold-decision-boundary"),
    ];
    let user = serde_json::json!({
        "type": "message",
        "role": "user",
        "content": [{"type": "input_text", "text": "look up the record"}],
    });
    let send = |rebuilt_history: bool| {
        let input = if rebuilt_history {
            vec![
                serde_json::json!({
                    "type":"message","role":"user",
                    "content":[{"type":"input_text","text":"A rebuilt earlier context"}]
                }),
                serde_json::json!({
                    "type":"message","role":"assistant",
                    "content":[{"type":"output_text","text":"ack"}]
                }),
                user.clone(),
            ]
        } else {
            vec![user.clone()]
        };
        let body = serde_json::json!({
            "model": "runtime-model",
            "input": input,
            "stream": false,
        });
        request(
            fixture.address,
            "POST",
            "/v1/responses",
            &headers,
            &serde_json::to_vec(&body).unwrap(),
        )
    };

    let first = send(false);
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let second = send(true);
    assert_eq!(
        second.status,
        200,
        "{}",
        String::from_utf8_lossy(&second.body)
    );
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (1, 1, 2),
        "a rebuilt message history must use the newly selected business branch"
    );

    let classifier_requests = classifier.requests();
    let second_wire = &classifier_requests[1];
    let body_start = second_wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("classifier request head terminator")
        + 4;
    let body: serde_json::Value = serde_json::from_slice(&second_wire[body_start..]).unwrap();
    assert_eq!(
        body.pointer("/latest_user/0/text")
            .and_then(serde_json::Value::as_str),
        Some("look up the record"),
        "unchanged latest_user must not suppress a ContextHold boundary"
    );
    assert_eq!(body["assessment_from"], 0);
    assert_eq!(body["visible_conversation"].as_array().unwrap().len(), 1);
    assert_eq!(body["visible_conversation"][0]["status"], "unknown");
    assert_eq!(
        body["visible_conversation"][0]["steps"][0][0]["tool"],
        "tools.lookup"
    );
    assert_eq!(
        body["visible_conversation"][0]["steps"][0][0]["status"],
        "unknown"
    );

    let facts = wait_execution_facts(&fixture, 2);
    let rounds = facts
        .iter()
        .filter(|fact| {
            fact.pointer("/fact/kind")
                .and_then(serde_json::Value::as_str)
                == Some("agent_turn_finished")
        })
        .collect::<Vec<_>>();
    assert_eq!(rounds.len(), 2, "each actual execution round seals once");
    let prior = rounds
        .iter()
        .find(|fact| {
            fact.pointer("/fact/status")
                .and_then(serde_json::Value::as_str)
                == Some("unknown")
        })
        .expect("the ContextHold boundary preserves the prior unknown status");
    let current = rounds
        .iter()
        .find(|fact| {
            fact.pointer("/fact/status")
                .and_then(serde_json::Value::as_str)
                == Some("completed")
        })
        .expect("the newly selected branch completes its own round");
    let assessment = facts
        .iter()
        .find(|fact| {
            fact.pointer("/fact/kind")
                .and_then(serde_json::Value::as_str)
                == Some("branch_assessment_recorded")
        })
        .expect("the preceding model stage receives the assessment");
    assert_eq!(
        assessment.pointer("/fact/model_configuration_id"),
        prior.pointer("/fact/model_configuration_id"),
        "the boundary assessment belongs to the model that produced prior accepted output"
    );
    assert_eq!(
        assessment.pointer("/fact/segment_id"),
        prior.pointer("/fact/segment_id"),
        "the score must bind to the segment sealed before the new branch executes"
    );
    assert_ne!(
        assessment.pointer("/fact/segment_id"),
        current.pointer("/fact/segment_id"),
        "the newly executed branch must start unscored"
    );
    assert_eq!(
        assessment.pointer("/fact/target_through_turn_id"),
        prior.pointer("/fact/agent_turn_id"),
        "the score range must end at the prior routing round"
    );
    assert_ne!(
        assessment.pointer("/fact/target_through_turn_id"),
        current.pointer("/fact/agent_turn_id"),
        "the current routing round must not enter the preceding-stage score"
    );
}

#[test]
fn real_listener_rest_classifier_sends_and_persists_the_previous_segment_assessment() {
    let simple = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_SIMPLE,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_SIMPLE_WITH_ASSESSMENT,
        },
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&simple, &complex, &classifier],
        2,
        ObservationFaults::healthy(),
    );
    let headers = [
        ("X-HiRoute-Token", "runtime-token"),
        ("session-id", "rest-classifier-assessment"),
    ];
    let message = |role: &str, kind: &str, text: &str| {
        serde_json::json!({
            "type": "message",
            "role": role,
            "content": [{"type": kind, "text": text}],
        })
    };
    let first_user = message("user", "input_text", "Correct one typo.");
    let send = |input: serde_json::Value| {
        request(
            fixture.address,
            "POST",
            "/v1/responses",
            &headers,
            &serde_json::to_vec(&serde_json::json!({
                "model": "runtime-model",
                "input": input,
                "stream": false,
            }))
            .unwrap(),
        )
    };

    assert_eq!(send(serde_json::json!([first_user.clone()])).status, 200);
    assert_eq!(
        send(serde_json::json!([
            first_user,
            message("assistant", "output_text", "Corrected it."),
            message("user", "input_text", "That worked. Correct another typo.")
        ]))
        .status,
        200
    );
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (2, 0, 2)
    );

    let classifier_requests = classifier.requests();
    let second_wire = &classifier_requests[1];
    let body_start = second_wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("classifier request head terminator")
        + 4;
    let body: serde_json::Value = serde_json::from_slice(&second_wire[body_start..]).unwrap();
    assert_eq!(body.as_object().unwrap().len(), 5);
    assert_eq!(body["assessment_from"], 0);
    assert_eq!(body["visible_conversation"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["visible_conversation"][0]["steps"][0][0]["text"], "ok",
        "history must use accepted output, not the client's rewritten assistant text"
    );

    let facts = wait_execution_facts(&fixture, 2);
    let assessment = facts
        .iter()
        .find(|fact| {
            fact.pointer("/fact/kind")
                .and_then(serde_json::Value::as_str)
                == Some("branch_assessment_recorded")
        })
        .expect("a valid second-turn assessment must be persisted");
    assert_eq!(
        assessment
            .pointer("/fact/score")
            .and_then(|value| value.as_f64()),
        Some(0.75)
    );
    let second_decision = facts
        .iter()
        .rev()
        .find(|fact| fact.pointer("/fact/complexity").is_some())
        .expect("second classification fact");
    assert_eq!(
        second_decision
            .pointer("/fact/complexity/decision_source")
            .and_then(serde_json::Value::as_str),
        Some("external_classifier")
    );
    assert_eq!(
        second_decision
            .pointer("/fact/complexity/fallback_used")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

#[test]
fn real_listener_rest_classifier_freezes_the_branch_for_a_tool_continuation() {
    let simple = NativeProvider::start(Vec::new());
    let complex = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_TOOL_CALL,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let classifier = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: CLASSIFIER_COMPLEX,
    }]);
    let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 2);
    let headers = [
        ("X-HiRoute-Token", "runtime-token"),
        ("session-id", "rest-classifier-tool-continuation"),
    ];
    let user = serde_json::json!({
        "type": "message",
        "role": "user",
        "content": [{"type": "input_text", "text": "look up the record"}],
    });
    let first = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers,
        &serde_json::to_vec(&serde_json::json!({
            "model": "runtime-model",
            "input": [user.clone()],
            "stream": false,
        }))
        .unwrap(),
    );
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let first_body: serde_json::Value = serde_json::from_slice(&first.body).unwrap();
    let logical_call_id = first_body["output"][0]["call_id"]
        .as_str()
        .expect("Gateway response exposes the logical tool call ID");

    let second = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers,
        &serde_json::to_vec(&serde_json::json!({
            "model": "runtime-model",
            "input": [
                user,
                {"type":"function_call","call_id":logical_call_id,"namespace":"tools","name":"lookup","arguments":"{}"},
                {"type":"function_call_output","call_id":logical_call_id,"output":"found"}
            ],
            "stream": false,
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
        (simple.calls(), complex.calls(), classifier.calls()),
        (0, 2, 1),
        "a tool continuation must keep the turn's frozen branch without another REST decision"
    );
}

#[test]
#[ignore = "explicit paid smoke: requires a locally running trusted classifier service"]
fn live_hiroute_to_jev_smoke_selects_and_records_an_assessment() {
    let endpoint = std::env::var("HIROUTE_LIVE_CLASSIFIER_ENDPOINT")
        .expect("set HIROUTE_LIVE_CLASSIFIER_ENDPOINT to the local Jev service /v1/decisions URL");
    let replies = || {
        vec![
            ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: RESPONSES_OK,
            },
            ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: RESPONSES_OK,
            },
        ]
    };
    let simple = NativeProvider::start(replies());
    let complex = NativeProvider::start(replies());
    let unused_classifier = NativeProvider::start(Vec::new());
    let fixture = RuntimeFixture::launch_rest_classified_at(
        &[&simple, &complex, &unused_classifier],
        2,
        &endpoint,
    );
    let headers = [
        ("X-HiRoute-Token", "runtime-token"),
        ("session-id", "live-jev-smoke"),
    ];
    let message = |role: &str, kind: &str, text: &str| {
        serde_json::json!({
            "type": "message",
            "role": role,
            "content": [{"type": kind, "text": text}],
        })
    };
    let first_user = message(
        "user",
        "input_text",
        "Correct the typo 'recieve' in README.",
    );
    let first = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers,
        &serde_json::to_vec(&serde_json::json!({
            "model": "runtime-model",
            "input": [first_user.clone()],
            "stream": false,
        }))
        .unwrap(),
    );
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );

    let second = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers,
        &serde_json::to_vec(&serde_json::json!({
            "model": "runtime-model",
            "input": [
                first_user,
                message("assistant", "output_text", "Fixed the typo."),
                message("user", "input_text", "That worked. Correct the typo 'seperate' too.")
            ],
            "stream": false,
        }))
        .unwrap(),
    );
    assert_eq!(
        second.status,
        200,
        "{}",
        String::from_utf8_lossy(&second.body)
    );
    assert_eq!(unused_classifier.calls(), 0);
    assert_eq!(simple.calls() + complex.calls(), 2);
    let facts = wait_execution_facts(&fixture, 2);
    let decisions = facts
        .iter()
        .filter_map(|fact| fact.pointer("/fact/complexity"))
        .collect::<Vec<_>>();
    assert_eq!(
        decisions.len(),
        2,
        "both turns must record a classification"
    );
    assert!(
        decisions.iter().all(|decision| {
            decision
                .get("decision_source")
                .and_then(serde_json::Value::as_str)
                == Some("external_classifier")
                && decision
                    .get("fallback_used")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
        }),
        "both Jev decisions must be accepted without local fallback: {decisions:?}"
    );
    let assessment = facts
        .iter()
        .find(|fact| {
            fact.pointer("/fact/kind")
                .and_then(serde_json::Value::as_str)
                == Some("branch_assessment_recorded")
        })
        .expect("the second decision must persist Jev's optional competence assessment");
    let score = assessment
        .pointer("/fact/score")
        .and_then(serde_json::Value::as_f64)
        .expect("assessment score");
    assert!((0.0..=1.0).contains(&score));
}

#[test]
fn real_listener_rejects_the_removed_llm_envelope_and_falls_back_to_local_rules() {
    let simple = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: CLASSIFIER_INVALID,
    }]);
    let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 2);

    let response = fixture.request();

    assert_eq!(response.status, 200);
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (1, 0, 1),
        "invalid classifier output must use the local simple decision"
    );
}

#[test]
fn real_listener_unavailable_rest_classifier_falls_back_to_local_rules() {
    let simple = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: b"classifier unavailable",
    }]);
    let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 2);

    let response = fixture.request();

    assert_eq!(response.status, 200);
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (1, 0, 1),
        "an unavailable classifier must use exactly one local-rule fallback"
    );
}

#[test]
fn real_listener_rejected_classifier_input_falls_back_once_and_records_reason() {
    let simple = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![ProviderReply::Complete {
        status: 413,
        error_kind: None,
        body: b"classifier input rejected",
    }]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&simple, &complex, &classifier],
        2,
        ObservationFaults::healthy(),
    );

    let response = fixture.request();

    assert_eq!(response.status, 200);
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (1, 0, 1)
    );
    let facts = wait_execution_facts(&fixture, 1);
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/complexity/fallback_used")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            && fact
                .pointer("/fact/complexity/fallback_reason")
                .and_then(serde_json::Value::as_str)
                == Some("rejected_input")
    }));
}

#[test]
fn real_listener_classifier_timeout_falls_back_once_within_the_source_deadline() {
    let simple = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![ProviderReply::Stall {
        duration: Duration::from_secs(4),
    }]);
    let fixture = RuntimeFixture::launch_rest_classified_with_classifier_timeout(
        &[&simple, &complex, &classifier],
        2,
        Duration::from_millis(300),
    );

    let started = Instant::now();
    let response = fixture.request();

    assert_eq!(response.status, 200);
    assert!(started.elapsed() >= Duration::from_millis(250));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (1, 0, 1)
    );
}

#[test]
fn real_listener_source_deadline_during_rest_classification_never_falls_back() {
    let simple = NativeProvider::start(Vec::new());
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![ProviderReply::Stall {
        duration: Duration::from_secs(2),
    }]);
    let fixture = RuntimeFixture::launch_rest_classified_with_overall_timeout(
        &[&simple, &complex, &classifier],
        2,
        Duration::from_millis(500),
    );

    let started = Instant::now();
    let response = fixture.request();

    assert_eq!(response.status, 408);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap(),
        serde_json::json!({
            "schema_version": "hiroute.gateway.error/v1",
            "code": "REQUEST_DEADLINE_EXCEEDED",
            "phase": "request_authority",
        })
    );
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (0, 0, 1)
    );
}

#[test]
fn real_listener_disconnect_during_rest_classification_never_runs_the_main_model() {
    let simple = NativeProvider::start(Vec::new());
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![ProviderReply::Stall {
        duration: Duration::from_secs(4),
    }]);
    let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 2);
    let downstream = open_request(
        fixture.address,
        "POST",
        "/v1/responses",
        &[("X-HiRoute-Token", "runtime-token")],
        br#"{"model":"runtime-model","input":"cancel me","stream":false}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while classifier.calls() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        classifier.calls(),
        1,
        "classification call must be in flight"
    );
    drop(downstream);

    std::thread::sleep(Duration::from_millis(3_250));
    assert_eq!(
        (simple.calls(), complex.calls()),
        (0, 0),
        "disconnect must not run a rule fallback or main-model attempt"
    );
    assert_eq!(
        request(fixture.address, "GET", "/_hiroute/ready", &[], b"").status,
        200
    );
}

#[test]
fn real_listener_runtime_state_cas_write_failure_is_fail_closed_and_one_shot() {
    hiroute_e2e::p0_execution_receipt!(
        "runtime.cas_write_fault",
        [
            "runtime.cas_write_fault",
            "runtime.fail_closed_503",
            "runtime.zero_fallback_after_fault",
            "runtime.zero_bad_state_persist",
        ]
    );
    let primary = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
        ProviderReply::Complete {
            status: 429,
            error_kind: None,
            body: QUOTA_ERROR,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let fallback = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: FALLBACK_OK,
    }]);
    let fixture = RuntimeFixture::launch_with_runtime_state_control(&[&primary, &fallback], 2);

    let invalid = fixture.arm_runtime_state_fault("invalid-cas-count", "compare_and_swap_exact", 0);
    assert_fault_control_nack(&invalid);
    assert_eq!(
        fixture.request().status,
        200,
        "invalid command must not arm a fault"
    );
    assert_eq!((primary.calls(), fallback.calls()), (1, 0));

    let armed = fixture.arm_runtime_state_fault("arm-cas-once", "compare_and_swap_exact", 1);
    assert_fault_control_ack(&armed, "compare_and_swap_exact", 1);
    assert_runtime_state_503(&fixture.request());
    assert_eq!(primary.calls(), 2);
    assert_eq!(fallback.calls(), 0, "failed CAS must close before relay");

    assert_eq!(fixture.request().status, 200);
    assert_eq!(
        primary.calls(),
        3,
        "the failed CAS must not persist cooldown"
    );
    assert_eq!(fallback.calls(), 0);
    let facts = wait_execution_facts(&fixture, 3);
    assert_single_planner_run_per_request(&facts, 3);
    assert_runtime_state_outcomes(&facts, "compare_and_swap_exact", &["authority_error"]);
    let fault_request =
        request_id_for_runtime_state_outcome(&facts, "compare_and_swap_exact", "authority_error");
    assert_exact_attempt_lifecycle(&facts, &fault_request, 1, 1);
}

#[test]
fn real_listener_probe_write_failure_creates_no_attempt_and_recovers_once() {
    hiroute_e2e::p0_execution_receipt!(
        "runtime.probe_write_fault",
        [
            "runtime.probe_write_fault",
            "runtime.probe_zero_attempt",
            "runtime.probe_one_shot_recovery",
        ]
    );
    let primary = NativeProvider::start(vec![
        ProviderReply::CompleteWithRetryAfter {
            status: 429,
            retry_after_secs: 0,
            body: QUOTA_ERROR,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let fallback = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: FALLBACK_OK,
    }]);
    let fixture = RuntimeFixture::launch_with_runtime_state_control(&[&primary, &fallback], 1);

    assert_eq!(fixture.request().status, 502, "seed expired key cooldown");
    assert_eq!((primary.calls(), fallback.calls()), (1, 0));
    let armed = fixture.arm_runtime_state_fault("arm-probe-once", "acquire_probe_lease_exact", 1);
    assert_fault_control_ack(&armed, "acquire_probe_lease_exact", 1);

    assert_runtime_state_503(&fixture.request());
    assert_eq!(
        (primary.calls(), fallback.calls()),
        (1, 0),
        "a failed probe write must not create an Attempt or relay"
    );
    assert_eq!(fixture.request().status, 200);
    assert_eq!((primary.calls(), fallback.calls()), (2, 0));
    let facts = wait_execution_facts(&fixture, 3);
    assert_single_planner_run_per_request(&facts, 3);
    assert_runtime_state_outcomes(
        &facts,
        "acquire_probe_lease_exact",
        &["acquired", "authority_error"],
    );
    let fault_request = request_id_for_runtime_state_outcome(
        &facts,
        "acquire_probe_lease_exact",
        "authority_error",
    );
    assert_exact_attempt_lifecycle(&facts, &fault_request, 0, 0);
    let recovery_request =
        request_id_for_runtime_state_outcome(&facts, "acquire_probe_lease_exact", "acquired");
    assert_exact_attempt_lifecycle(&facts, &recovery_request, 1, 1);
}

#[test]
fn typed_failure_matrix_keeps_quota_binding_and_permanent_failures_distinct() {
    let profile = ConnectorErrorProfile::exact();
    let quota = classify_failure(
        &RawAttemptFailure::Http {
            status: 429,
            kind: Some(ProviderFailureKind::Quota),
            retry_after: None,
        },
        profile,
    );
    let overload = classify_failure(
        &RawAttemptFailure::Http {
            status: 429,
            kind: Some(ProviderFailureKind::BindingOverload),
            retry_after: None,
        },
        profile,
    );
    let permanent = classify_failure(
        &RawAttemptFailure::Http {
            status: 422,
            kind: Some(ProviderFailureKind::PermanentClient),
            retry_after: None,
        },
        profile,
    );
    assert_eq!(quota.class, AttemptFailureClass::Quota);
    assert_eq!(quota.state_scope(), Some(FailureStateScope::Credential));
    assert_eq!(overload.class, AttemptFailureClass::BindingOverload);
    assert_eq!(overload.state_scope(), Some(FailureStateScope::Binding));
    assert_eq!(permanent.class, AttemptFailureClass::PermanentClient);
    assert!(!permanent.is_precommit_relayable());
}

fn assert_fault_control_ack(response: &WireResponse, operation: &str, remaining: u64) {
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["command"], "runtime_state_write_fault");
    assert_eq!(body["outcome"], "ack");
    assert_eq!(body["code"], "RUNTIME_STATE_WRITE_FAULT_ARMED");
    assert_eq!(body["runtime_state_operation"], operation);
    assert_eq!(body["remaining_failures"], remaining);
}

fn assert_fault_control_nack(response: &WireResponse) {
    assert_eq!(response.status, 422);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["command"], "runtime_state_write_fault");
    assert_eq!(body["outcome"], "nack");
    assert_eq!(body["code"], "RUNTIME_STATE_WRITE_FAULT_INVALID");
}

fn assert_runtime_state_503(response: &WireResponse) {
    assert_eq!(
        response.status,
        503,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap(),
        serde_json::json!({
            "schema_version": "hiroute.gateway.error/v1",
            "code": "RUNTIME_STATE_AUTHORITY_UNAVAILABLE",
            "phase": "runtime_state",
        })
    );
}

fn wait_execution_facts(
    fixture: &RuntimeFixture,
    finished_requests: usize,
) -> Vec<serde_json::Value> {
    let path = fixture
        .observation_root
        .as_deref()
        .expect("observation capture")
        .join("execution-fact.jsonl");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let facts = std::fs::read_to_string(&path)
            .ok()
            .into_iter()
            .flat_map(|contents| contents.lines().map(str::to_owned).collect::<Vec<_>>())
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).unwrap())
            .collect::<Vec<_>>();
        let finished = facts
            .iter()
            .filter(|fact| {
                fact.pointer("/fact/kind").and_then(|value| value.as_str())
                    == Some("request_finished")
            })
            .count();
        if finished >= finished_requests {
            return facts;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for execution facts: {facts:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_content_for_request(fixture: &RuntimeFixture, request_id: &str) -> Vec<serde_json::Value> {
    let path = fixture
        .observation_root
        .as_deref()
        .expect("observation capture")
        .join("conversation-content.jsonl");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = std::fs::read_to_string(&path)
            .ok()
            .into_iter()
            .flat_map(|contents| contents.lines().map(str::to_owned).collect::<Vec<_>>())
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).unwrap())
            .collect::<Vec<_>>();
        if records.iter().any(|record| {
            record
                .pointer("/correlation/request_id")
                .and_then(serde_json::Value::as_str)
                == Some(request_id)
                && record.get("phase").and_then(serde_json::Value::as_str) == Some("finish")
        }) {
            return records;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for content observations: {records:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_single_planner_run_per_request(facts: &[serde_json::Value], expected_requests: usize) {
    let mut requests = BTreeMap::<String, usize>::new();
    for fact in facts {
        let Some(request_id) = fact
            .pointer("/correlation/request_id")
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        if fact.pointer("/fact/kind").and_then(|value| value.as_str()) == Some("route_decision") {
            *requests.entry(request_id.to_owned()).or_default() += 1;
        }
    }
    assert_eq!(requests.len(), expected_requests);
    assert!(requests.values().all(|planner_runs| *planner_runs == 1));
}

fn assert_runtime_state_outcomes(facts: &[serde_json::Value], operation: &str, expected: &[&str]) {
    let mut outcomes = facts
        .iter()
        .filter(|fact| {
            fact.pointer("/fact/operation")
                .and_then(|value| value.as_str())
                == Some(operation)
        })
        .filter_map(|fact| {
            fact.pointer("/fact/outcome")
                .and_then(|value| value.as_str())
        })
        .collect::<Vec<_>>();
    outcomes.sort_unstable();
    assert_eq!(outcomes, expected);
}

fn request_id_for_runtime_state_outcome(
    facts: &[serde_json::Value],
    operation: &str,
    outcome: &str,
) -> String {
    let request_ids = facts
        .iter()
        .filter(|fact| {
            fact.pointer("/fact/kind")
                .and_then(serde_json::Value::as_str)
                == Some("runtime_state")
                && fact
                    .pointer("/fact/operation")
                    .and_then(serde_json::Value::as_str)
                    == Some(operation)
                && fact
                    .pointer("/fact/outcome")
                    .and_then(serde_json::Value::as_str)
                    == Some(outcome)
        })
        .filter_map(|fact| {
            fact.pointer("/correlation/request_id")
                .and_then(serde_json::Value::as_str)
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        request_ids.len(),
        1,
        "runtime state {operation}/{outcome} must identify one request: {request_ids:?}"
    );
    request_ids.into_iter().next().unwrap().to_owned()
}

fn assert_exact_attempt_lifecycle(
    facts: &[serde_json::Value],
    request_id: &str,
    expected_started: u64,
    expected_finished: u64,
) {
    let request_facts = facts
        .iter()
        .filter(|fact| {
            fact.pointer("/correlation/request_id")
                .and_then(serde_json::Value::as_str)
                == Some(request_id)
        })
        .collect::<Vec<_>>();
    let attempt_ordinals = |kind| {
        request_facts
            .iter()
            .filter(|fact| {
                fact.pointer("/fact/kind")
                    .and_then(serde_json::Value::as_str)
                    == Some(kind)
            })
            .map(|fact| {
                fact.pointer("/fact/ordinal")
                    .and_then(serde_json::Value::as_u64)
                    .expect("attempt receipt ordinal")
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        attempt_ordinals("attempt_started"),
        (1..=expected_started).collect::<Vec<_>>(),
        "request {request_id} has an unexpected started Attempt sequence"
    );
    assert_eq!(
        attempt_ordinals("attempt_finished"),
        (1..=expected_finished).collect::<Vec<_>>(),
        "request {request_id} has an unexpected finished Attempt sequence"
    );
    let terminals = request_facts
        .iter()
        .filter(|fact| {
            fact.pointer("/fact/kind")
                .and_then(serde_json::Value::as_str)
                == Some("request_finished")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        terminals.len(),
        1,
        "request {request_id} has one terminal execution receipt"
    );
    let terminal = terminals[0];
    assert_eq!(
        terminal
            .pointer("/fact/attempts_started")
            .and_then(serde_json::Value::as_u64),
        Some(expected_started),
        "request {request_id} terminal started-attempt count"
    );
    assert_eq!(
        terminal
            .pointer("/fact/attempts_finished")
            .and_then(serde_json::Value::as_u64),
        Some(expected_finished),
        "request {request_id} terminal finished-attempt count"
    );
}
