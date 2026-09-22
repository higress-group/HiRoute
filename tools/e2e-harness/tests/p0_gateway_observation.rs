mod runtime_support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use runtime_support::{NativeProvider, ObservationFaults, ProviderReply, RuntimeFixture, request};
use serde_json::Value;

const REJECTED_BODY: &[u8] =
    br#"{"error":{"message":"rejected-attempt-secret-22008","type":"server_error"}}"#;
const ACCEPTED_BODY: &[u8] = br#"{"id":"observation-ok","model":"native-observed","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"accepted-response-22008"}]},{"type":"function_call","id":"fc-native-22008","call_id":"call-22008","namespace":"weather-services","name":"weather","arguments":"{\"city\":\"Hangzhou\"}","status":"completed"}],"usage":{"input_tokens":11,"output_tokens":7,"total_tokens":18,"input_tokens_details":{"cached_tokens":3},"output_tokens_details":{"reasoning_tokens":2}}}"#;
const ACCEPTED_STREAM_CREATED: &[u8] = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"observation-stream\",\"model\":\"native-observed\"}}\n\n";
const ACCEPTED_STREAM_TEXT: &[u8] = b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"observation-message\",\"output_index\":0,\"content_index\":0,\"delta\":\"accepted-stream-22008\"}\n\n";
const ACCEPTED_STREAM_COMPLETED: &[u8] = b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"observation-stream\",\"model\":\"native-observed\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"id\":\"observation-message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"accepted-stream-22008\",\"annotations\":[]}]}],\"usage\":{\"input_tokens\":13,\"output_tokens\":5,\"total_tokens\":18,\"input_tokens_details\":{\"cached_tokens\":4},\"output_tokens_details\":{\"reasoning_tokens\":3}}}}\n\n";

#[test]
fn real_hirouted_emits_request_route_attempt_commit_usage_and_accepted_only_content() {
    hiroute_e2e::p0_execution_receipt!(
        "observation.accepted_content",
        [
            "observation.execution_sequence",
            "observation.accepted_only_content",
            "observation.content_commit_boundary",
            "observation.otel_zero_body",
        ]
    );
    let rejected = NativeProvider::start(vec![ProviderReply::CompleteWithRetryAfter {
        status: 503,
        retry_after_secs: 2,
        body: REJECTED_BODY,
    }]);
    let accepted = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: ACCEPTED_BODY,
    }]);
    let fixture = RuntimeFixture::launch_with_observation(
        &[&rejected, &accepted],
        2,
        ObservationFaults::healthy(),
    );
    let request_secret = "canonical-request-secret-22008".repeat(6_000);
    let response = fixture.request_body(
        format!(r#"{{"model":"runtime-model","instructions":"{}","input":"{request_secret}","stream":false}}"#, "disk-backed instruction ".repeat(4_000)).as_bytes(),
    );
    assert_eq!(response.status, 200);
    assert!(String::from_utf8_lossy(&response.body).contains("accepted-response-22008"));
    let downstream_logical_id = response_tool_logical_id(&response.body);
    assert_eq!(downstream_logical_id, "call-22008");
    assert_eq!(rejected.calls(), 1);
    assert_eq!(accepted.calls(), 1);
    wait_replay_empty(&fixture.replay_root);

    let root = fixture.observation_root.as_deref().unwrap();
    let facts = wait_complete_accepted_request(root);
    assert_complete_accepted_request(&facts, "runtime-native-model-2", 11, 7);
    let request_id = request_id(&facts);
    let kinds = fact_kinds(&facts);
    for expected in [
        "route_decision",
        "candidate_decision",
        "attempt_started",
        "attempt_finished",
        "runtime_state",
        "semantic_commit",
        "usage_and_cache",
        "request_finished",
    ] {
        assert!(kinds.contains(&expected), "missing {expected}: {kinds:?}");
    }
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("route_decision")
            && fact.pointer("/fact/plan_id").and_then(Value::as_str) == Some("agent-plan-91")
            && fact
                .pointer("/fact/requested_reasoning_disposition")
                .and_then(Value::as_str)
                == Some("absent")
            && fact.pointer("/fact/stream").and_then(Value::as_bool) == Some(false)
            && fact
                .pointer("/fact/requirements/ingress_protocol")
                .and_then(Value::as_str)
                == Some("responses")
            && fact
                .pointer("/fact/requirements/text")
                .and_then(Value::as_bool)
                == Some(true)
            && fact
                .pointer("/fact/requirements/streaming")
                .and_then(Value::as_bool)
                == Some(false)
            && fact
                .pointer("/fact/reason_ledger")
                .and_then(Value::as_array)
                .is_some_and(|ledger| {
                    ledger.iter().any(|entry| {
                        entry.get("code").and_then(Value::as_str) == Some("CUSTOM_EXACT_ORDER")
                    })
                })
    }));
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("candidate_decision")
            && fact
                .pointer("/fact/upstream_protocol")
                .and_then(Value::as_str)
                == Some("responses")
            && fact
                .pointer("/fact/adapter_revision")
                .and_then(Value::as_str)
                == Some("builtin-protocol-adapter/v1")
            && fact
                .pointer("/fact/profile_digest")
                .and_then(Value::as_str)
                .is_some_and(|digest| digest.starts_with("sha256:"))
    }));
    assert!(facts.iter().all(|fact| {
        fact.get("authority_id").and_then(Value::as_str).is_some()
            && fact
                .get("gateway_publication_digest")
                .and_then(Value::as_str)
                .is_some_and(|digest| digest.starts_with("sha256:"))
            && fact.pointer("/route/kind").and_then(Value::as_str) == Some("plan")
            && fact.pointer("/route/revision").and_then(Value::as_u64) == Some(91)
            && fact
                .pointer("/route/semantic_digest")
                .and_then(Value::as_str)
                .is_some_and(|digest| digest.starts_with("sha256:"))
            && fact.get("grant_id").and_then(Value::as_str).is_some()
    }));
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind") == Some(&Value::String("attempt_finished".into()))
            && fact.pointer("/fact/outcome") == Some(&Value::String("accepted".into()))
    }));
    let rejected_attempt = facts
        .iter()
        .find(|fact| {
            fact.pointer("/fact/kind").and_then(Value::as_str) == Some("attempt_finished")
                && fact.pointer("/fact/ordinal").and_then(Value::as_u64) == Some(1)
        })
        .expect("rejected attempt receipt");
    for (pointer, expected) in [
        ("/fact/outcome", "rejected"),
        ("/fact/error_class", "transient"),
        ("/fact/disposition", "continue"),
        ("/fact/commits/upstream_request", "write_confirmed"),
        ("/fact/downstream_outcome", "not_started"),
        ("/fact/cleanup_outcome", "completed"),
        ("/fact/termination_reason", "fallback_complete"),
    ] {
        assert_eq!(
            rejected_attempt.pointer(pointer).and_then(Value::as_str),
            Some(expected),
            "rejected attempt receipt: {rejected_attempt:#?}"
        );
    }
    assert_eq!(
        rejected_attempt
            .pointer("/fact/provider_http_status")
            .and_then(Value::as_u64),
        Some(503)
    );
    assert_eq!(
        rejected_attempt
            .pointer("/fact/retry_after_millis")
            .and_then(Value::as_u64),
        Some(2_000)
    );
    assert_eq!(
        rejected_attempt
            .pointer("/fact/retryable")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.pointer("/fact/kind").and_then(Value::as_str)
                == Some("attempt_started"))
            .filter_map(|fact| fact.pointer("/fact/ordinal").and_then(Value::as_u64))
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("attempt_started")
            && fact.pointer("/fact/ordinal").and_then(Value::as_u64) == Some(2)
            && fact.pointer("/fact/start_reason").and_then(Value::as_str)
                == Some("precommit_fallback_after_transient")
            && fact
                .pointer("/fact/previous_attempt_id")
                .and_then(Value::as_str)
                .is_some()
    }));
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("runtime_state")
            && fact.pointer("/fact/operation").and_then(Value::as_str)
                == Some("compare_and_swap_exact")
            && fact.pointer("/fact/health").and_then(Value::as_str) == Some("cooling_down")
            && fact
                .pointer("/fact/cooldown_remaining_millis")
                .and_then(Value::as_u64)
                .is_some_and(|millis| millis > 0)
            && fact.pointer("/fact/outcome").and_then(Value::as_str) == Some("applied")
    }));
    assert!(
        facts
            .iter()
            .filter(|fact| {
                fact.pointer("/fact/kind").and_then(Value::as_str) == Some("attempt_finished")
            })
            .all(|fact| {
                fact.pointer("/fact/duration_micros")
                    .and_then(Value::as_u64)
                    .is_some_and(|micros| micros > 0)
            })
    );
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("semantic_commit")
            && fact.pointer("/fact/ordinal").and_then(Value::as_u64) == Some(2)
            && fact.pointer("/fact/boundary").and_then(Value::as_str)
                == Some("full_frame_transport_accepted")
    }));
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished")
            && fact
                .pointer("/fact/attempts_started")
                .and_then(Value::as_u64)
                == Some(2)
            && fact
                .pointer("/fact/attempts_finished")
                .and_then(Value::as_u64)
                == Some(2)
            && fact
                .pointer("/fact/accepted_attempt_ordinal")
                .and_then(Value::as_u64)
                == Some(2)
    }));
    let usage_facts = facts
        .iter()
        .filter(|fact| {
            fact.pointer("/fact/kind").and_then(Value::as_str) == Some("usage_and_cache")
        })
        .collect::<Vec<_>>();
    assert!(
        usage_facts.iter().any(|fact| {
            fact.pointer("/fact/ordinal").and_then(Value::as_u64) == Some(2)
                && fact.pointer("/fact/source").and_then(Value::as_str)
                    == Some("provider_completion")
                && fact.pointer("/fact/input_tokens").and_then(Value::as_u64) == Some(11)
                && fact.pointer("/fact/output_tokens").and_then(Value::as_u64) == Some(7)
                && fact
                    .pointer("/fact/billable_tokens")
                    .is_some_and(Value::is_null)
                && fact
                    .pointer("/fact/cache_read_tokens")
                    .and_then(Value::as_u64)
                    == Some(3)
                && fact
                    .pointer("/fact/reasoning_tokens")
                    .and_then(Value::as_u64)
                    == Some(2)
                && fact
                    .pointer("/fact/effective_cost_micros")
                    .is_some_and(Value::is_null)
                && fact
                    .pointer("/fact/input_provenance")
                    .and_then(Value::as_str)
                    == Some("reported")
                && fact
                    .pointer("/fact/output_provenance")
                    .and_then(Value::as_str)
                    == Some("reported")
                && fact
                    .pointer("/fact/billable_provenance")
                    .and_then(Value::as_str)
                    == Some("unknown")
                && fact.pointer("/fact/cost_class").and_then(Value::as_str) == Some("free")
                && fact.pointer("/fact/cache_status").and_then(Value::as_str)
                    == Some("confirmed_usage")
        }),
        "unexpected usage facts: {usage_facts:#?}"
    );

    let content = wait_content_complete(root);
    assert_content_request_id(&content, request_id);
    let request_content = decoded_content(&content, "request_input");
    let response_content = decoded_content(&content, "response_delivered");
    assert!(request_content.contains(request_secret.as_str()));
    let request_appends = content
        .iter()
        .filter(|record| {
            record.get("direction").and_then(Value::as_str) == Some("request_input")
                && record.get("phase").and_then(Value::as_str) == Some("append")
        })
        .collect::<Vec<_>>();
    assert!(
        request_appends.len() >= 3,
        "canonical request content must be delivered incrementally"
    );
    assert!(request_content.contains(&"disk-backed instruction ".repeat(4_000)));
    for role in ["system", "user"] {
        let parts = request_appends
            .iter()
            .filter(|event| event["message_role"] == role)
            .collect::<Vec<_>>();
        assert_eq!(parts.len(), if role == "system" { 2 } else { 3 });
        let content_id = &parts[0]["content_id"];
        let blob_digest = &parts[0]["content_blob_digest"];
        assert!(content_id.is_string() && blob_digest.is_string());
        assert!(parts.iter().all(|event| {
            event["content_id"] == *content_id
                && event["content_blob_digest"] == *blob_digest
                && event["content_ref"]["content_id"] == *content_id
                && event["content_ref"]["digest"] == *blob_digest
        }));
    }
    assert!(response_content.contains("accepted-response-22008"));
    assert!(response_content.contains("\"kind\":\"text_delta\""));
    assert!(response_content.contains("\"kind\":\"tool_call_started\""));
    assert!(response_content.contains("\"kind\":\"tool_arguments_delta\""));
    assert!(response_content.contains("\"kind\":\"tool_call_finished\""));
    let response_events = decoded_content_events(&content, "response_delivered");
    let observed_tool = response_events
        .iter()
        .find_map(|event| {
            (event.pointer("/event/kind").and_then(Value::as_str) == Some("tool_call_started"))
                .then(|| event.pointer("/event"))
                .flatten()
        })
        .expect("accepted canonical Tool start");
    assert_eq!(
        observed_tool.get("logical_id").and_then(Value::as_str),
        Some(downstream_logical_id.as_str())
    );
    assert_eq!(observed_tool["namespace"], "weather-services");
    assert!(observed_tool.get("native_id").is_none());
    assert!(observed_tool.get("owner").is_none());
    // The public call ID is preserved; provider-only item IDs remain off the
    // canonical observation path.
    assert!(response_content.contains("call-22008"));
    assert!(!response_content.contains("fc-native-22008"));
    assert!(!response_content.contains("rejected-attempt-secret-22008"));
    assert!(content.iter().all(|event| {
        event.get("content_kind").and_then(Value::as_str) != Some("downstream_protocol_frame")
    }));
    assert_stream_lifecycle(&content, "request_input");
    assert_stream_lifecycle(&content, "response_delivered");
    let request_transcript_root = content
        .iter()
        .find(|event| {
            event.get("direction").and_then(Value::as_str) == Some("request_input")
                && event.get("phase").and_then(Value::as_str) == Some("finish")
        })
        .and_then(|event| event.get("result_transcript_root"))
        .and_then(Value::as_str)
        .expect("request transcript root");
    assert!(content.iter().any(|event| {
        event.get("direction").and_then(Value::as_str) == Some("response_delivered")
            && event.get("phase").and_then(Value::as_str) == Some("begin")
            && event.get("parent_transcript_root").and_then(Value::as_str)
                == Some(request_transcript_root)
    }));
    let committed_frame = facts
        .iter()
        .find(|fact| fact.pointer("/fact/kind").and_then(Value::as_str) == Some("semantic_commit"))
        .and_then(|fact| fact.pointer("/fact/frame_id"))
        .and_then(Value::as_str)
        .expect("semantic commit frame identity");
    assert!(
        content
            .iter()
            .filter(|event| {
                event.get("direction").and_then(Value::as_str) == Some("response_delivered")
                    && event.get("phase").and_then(Value::as_str) == Some("append")
            })
            .all(|event| {
                event.get("transport_frame_id").and_then(Value::as_str) == Some(committed_frame)
                    && event.get("message_ordinal").and_then(Value::as_u64) == Some(0)
            })
    );

    let lifecycle = wait_request_finished(root, "lifecycle.jsonl");
    assert_attempt_lifecycle(&lifecycle, 1);
    assert_attempt_lifecycle(&lifecycle, 2);

    let otel = wait_records(root, "otel.jsonl", 3);
    let rendered = serde_json::to_string(&otel).unwrap();
    assert!(rendered.contains("gen_ai.client.inference.operation.details"));
    assert!(rendered.contains("gen_ai.operation.name"));
    assert!(!rendered.contains(request_secret.as_str()));
    assert!(!rendered.contains("accepted-response-22008"));
    assert!(!rendered.contains("canonical_bytes_base64"));
}

#[test]
fn real_streaming_hirouted_captures_canonical_content_and_nested_responses_usage() {
    let reply = ProviderReply::StreamDrip {
        status: 200,
        chunks: vec![
            ACCEPTED_STREAM_CREATED.to_vec(),
            ACCEPTED_STREAM_TEXT.to_vec(),
            ACCEPTED_STREAM_COMPLETED.to_vec(),
        ],
        interval: Duration::from_millis(5),
    };
    let provider = NativeProvider::start(vec![reply.clone(), reply]);
    let fixture =
        RuntimeFixture::launch_with_observation(&[&provider], 1, ObservationFaults::healthy());

    let body = br#"{"model":"runtime-model","input":"stream-observation","stream":true}"#;
    let root = fixture.observation_root.as_deref().unwrap();
    for expected in 1..=2 {
        let response = request(
            fixture.address,
            "POST",
            "/v1/responses",
            &[
                ("X-HiRoute-Token", "runtime-token"),
                ("session-id", "stream-observation-session"),
            ],
            body,
        );
        assert_eq!(response.status, 200);
        assert!(String::from_utf8_lossy(&response.body).contains("accepted-stream-22008"));
        drop(wait_complete_accepted_requests(root, expected));
    }
    assert_eq!(provider.calls(), 2);
    let request_facts = wait_complete_accepted_requests(root, 2);
    assert_eq!(request_facts.len(), 2);
    let request_ids = request_facts
        .iter()
        .map(|facts| request_id(facts).to_owned())
        .collect::<Vec<_>>();
    assert_ne!(request_ids[0], request_ids[1]);
    for facts in &request_facts {
        assert_complete_accepted_request(facts, "runtime-native-model-1", 13, 5);
        assert!(
            facts.iter().any(|fact| {
                fact.pointer("/fact/kind").and_then(Value::as_str) == Some("usage_and_cache")
                    && fact.pointer("/fact/source").and_then(Value::as_str)
                        == Some("provider_completion")
                    && fact.pointer("/fact/input_tokens").and_then(Value::as_u64) == Some(13)
                    && fact.pointer("/fact/output_tokens").and_then(Value::as_u64) == Some(5)
                    && fact
                        .pointer("/fact/cache_read_tokens")
                        .and_then(Value::as_u64)
                        == Some(4)
                    && fact
                        .pointer("/fact/reasoning_tokens")
                        .and_then(Value::as_u64)
                        == Some(3)
            }),
            "streaming usage facts: {facts:#?}"
        );
    }
    assert!(request_facts[1].iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("route_decision")
            && fact
                .pointer("/fact/reason_ledger")
                .and_then(Value::as_array)
                .is_some_and(|ledger| {
                    ledger.iter().any(|entry| {
                        entry.get("code").and_then(Value::as_str) == Some("CONTEXT_HOLD_APPLIED")
                    })
                })
    }));
    assert!(request_facts[1].iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("candidate_decision")
            && fact
                .pointer("/fact/ranking_reasons")
                .and_then(Value::as_array)
                .is_some_and(|reasons| {
                    reasons
                        .iter()
                        .any(|reason| reason.as_str() == Some("CONTEXT_MODEL_HOLD"))
                })
    }));

    let content = wait_content_complete_for_requests(root, &request_ids);
    for request_id in &request_ids {
        let request_content = content
            .iter()
            .filter(|record| {
                record
                    .pointer("/correlation/request_id")
                    .and_then(Value::as_str)
                    == Some(request_id.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        assert_content_request_id(&request_content, request_id);
        assert_eq!(
            decoded_content(&request_content, "request_input"),
            "stream-observation"
        );
        let response_content = decoded_content(&request_content, "response_delivered");
        assert!(response_content.contains("accepted-stream-22008"));
        assert!(response_content.contains("\"kind\":\"text_delta\""));
        assert!(!response_content.contains("response.completed"));
        assert!(request_content.iter().all(|event| {
            event.get("content_kind").and_then(Value::as_str) != Some("downstream_protocol_frame")
        }));
    }
}

#[test]
fn real_hirouted_emits_zero_attempt_when_runtime_state_excludes_the_sealed_candidate() {
    let provider = NativeProvider::start(vec![ProviderReply::Complete {
        status: 401,
        error_kind: None,
        body: br#"{"error":{"type":"invalid_api_key","message":"disabled"}}"#,
    }]);
    let fixture =
        RuntimeFixture::launch_with_observation(&[&provider], 1, ObservationFaults::healthy());
    assert_eq!(
        fixture.request().status,
        401,
        "seed disabled credential state"
    );
    assert_eq!(
        fixture.request().status,
        502,
        "all runtime candidates excluded"
    );
    assert_eq!(provider.calls(), 1);

    let root = fixture.observation_root.as_deref().unwrap();
    let facts = wait_request_finished_count(root, "execution-fact.jsonl", 2);
    let request_id = facts
        .iter()
        .rev()
        .find(|fact| fact.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished"))
        .and_then(|fact| fact.pointer("/correlation/request_id"))
        .and_then(Value::as_str)
        .unwrap();
    let request_facts = facts
        .iter()
        .filter(|fact| {
            fact.pointer("/correlation/request_id")
                .and_then(Value::as_str)
                == Some(request_id)
        })
        .collect::<Vec<_>>();
    assert!(
        request_facts.iter().any(|fact| {
            fact.pointer("/fact/kind").and_then(Value::as_str) == Some("route_decision")
                && fact.pointer("/fact/outcome").and_then(Value::as_str) == Some("ready")
        }),
        "second request facts: {request_facts:#?}"
    );
    assert!(
        request_facts.iter().any(|fact| {
            fact.pointer("/fact/kind").and_then(Value::as_str) == Some("candidate_decision")
                && fact.pointer("/fact/eligible").and_then(Value::as_bool) == Some(true)
                && fact.pointer("/fact/candidate_id").and_then(Value::as_str)
                    == Some("runtime-target-1")
        }),
        "second request facts: {request_facts:#?}"
    );
    assert!(
        request_facts.iter().any(|fact| {
            fact.pointer("/fact/kind").and_then(Value::as_str) == Some("runtime_state")
                && fact.pointer("/fact/key_scope").and_then(Value::as_str) == Some("credential")
                && fact.pointer("/fact/credential_ref").and_then(Value::as_str)
                    == Some("credential-1")
                && fact.pointer("/fact/health").and_then(Value::as_str) == Some("disabled")
                && fact.pointer("/fact/outcome").and_then(Value::as_str) == Some("ok")
        }),
        "second request facts: {request_facts:#?}"
    );
    assert!(
        request_facts.iter().any(|fact| {
            fact.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished")
                && fact
                    .pointer("/fact/attempts_started")
                    .and_then(Value::as_u64)
                    == Some(0)
                && fact
                    .pointer("/fact/attempts_finished")
                    .and_then(Value::as_u64)
                    == Some(0)
                && fact
                    .pointer("/fact/accepted_attempt_ordinal")
                    .is_some_and(Value::is_null)
        }),
        "second request facts: {request_facts:#?}"
    );
    assert!(
        !request_facts.iter().any(|fact| {
            matches!(
                fact.pointer("/fact/kind").and_then(Value::as_str),
                Some("attempt_started" | "semantic_commit" | "usage_and_cache")
            )
        }),
        "second request facts: {request_facts:#?}"
    );
    let content = wait_records(root, "conversation-content.jsonl", 6);
    assert!(
        content
            .iter()
            .filter(|event| {
                event
                    .pointer("/correlation/request_id")
                    .and_then(Value::as_str)
                    == Some(request_id)
            })
            .all(|event| event.get("direction").and_then(Value::as_str) == Some("request_input"))
    );
}

#[test]
fn real_hirouted_never_records_a_terminated_attempt_response_as_accepted_content() {
    let rejected = NativeProvider::start(vec![ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: REJECTED_BODY,
    }]);
    let fixture =
        RuntimeFixture::launch_with_observation(&[&rejected], 1, ObservationFaults::healthy());

    let response = fixture.request();
    assert_eq!(response.status, 503);
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap(),
        serde_json::json!({
            "error": {
                "code": "UPSTREAM_ATTEMPT_FAILED",
                "type": "upstream_error",
            }
        })
    );
    let root = fixture.observation_root.as_deref().unwrap();
    let facts = wait_request_finished(root, "execution-fact.jsonl");
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("attempt_finished")
            && fact.pointer("/fact/ordinal").and_then(Value::as_u64) == Some(1)
            && fact.pointer("/fact/outcome").and_then(Value::as_str) == Some("rejected")
            && fact.pointer("/fact/error_class").and_then(Value::as_str) == Some("transient")
            && fact.pointer("/fact/retryable").and_then(Value::as_bool) == Some(true)
    }));
    assert!(!facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("semantic_commit")
    }));
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished")
            && fact
                .pointer("/fact/attempts_started")
                .and_then(Value::as_u64)
                == Some(1)
            && fact
                .pointer("/fact/attempts_finished")
                .and_then(Value::as_u64)
                == Some(1)
            && fact
                .pointer("/fact/accepted_attempt_ordinal")
                .is_some_and(Value::is_null)
    }));

    let content = wait_records(root, "conversation-content.jsonl", 3);
    let rendered = serde_json::to_string(&content).unwrap();
    assert!(!rendered.contains("response_delivered"));
    assert!(!rendered.contains("rejected-attempt-secret-22008"));
}

#[test]
fn real_hirouted_isolates_slow_fail_and_panic_sinks_and_reports_each_gap() {
    hiroute_e2e::p0_execution_receipt!(
        "observation.slow_sink",
        [
            "observation.slow_sink",
            "observation.fact_gap",
            "observation.content_gap"
        ]
    );
    let first = NativeProvider::start(vec![ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: REJECTED_BODY,
    }]);
    let second = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: ACCEPTED_BODY,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: ACCEPTED_BODY,
        },
    ]);
    let fixture = RuntimeFixture::launch_with_observation(
        &[&first, &second],
        2,
        ObservationFaults {
            lifecycle_mode: "slow_once:1000",
            execution_mode: "fail_once",
            content_mode: "panic_once",
            otel_mode: "healthy",
            queue_bytes: 4 * 1024,
        },
    );

    let started = Instant::now();
    let first_response = fixture.request();
    assert_eq!(first_response.status, 200);
    assert_eq!(response_tool_logical_id(&first_response.body), "call-22008");
    assert!(
        started.elapsed() < Duration::from_millis(750),
        "a one-second sink must not backpressure the model response"
    );

    let root = fixture.observation_root.as_deref().unwrap();
    for (file, expected_reason) in [
        ("lifecycle.jsonl", "queue_bytes_exceeded"),
        ("execution-fact.jsonl", "sink_nack"),
        ("conversation-content.jsonl", "sink_panicked"),
    ] {
        let records = wait_for_gap(root, file);
        assert!(
            records.iter().any(|record| {
                record
                    .pointer("/loss_watermark/reason")
                    .and_then(Value::as_str)
                    == Some(expected_reason)
            }),
            "{file} must surface its own {expected_reason} gap: {records:#?}"
        );
    }

    // Every gap above became durable while traffic was idle. Only after that
    // terminal-heartbeat proof do we issue a second request for the separate
    // cooldown-isolation assertion.
    let second_response = fixture.request();
    assert_eq!(second_response.status, 200);
    assert_eq!(
        response_tool_logical_id(&second_response.body),
        "call-22008"
    );
    assert_eq!(first.calls(), 1, "cooldown must survive observation faults");
    assert_eq!(second.calls(), 2);
}

fn wait_records(root: &Path, file: &str, minimum: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = read_records(&root.join(file));
        if records.len() >= minimum {
            return records;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {file}: {records:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_replay_empty(root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if root.is_dir()
            && fs::read_dir(root)
                .expect("read replay root")
                .next()
                .is_none()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "observation retained the Replay backing after request completion"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_request_finished(root: &Path, file: &str) -> Vec<Value> {
    wait_request_finished_count(root, file, 1)
}

fn wait_request_finished_count(root: &Path, file: &str, minimum: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = read_records(&root.join(file));
        if records
            .iter()
            .filter(|record| {
                record.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished")
            })
            .count()
            >= minimum
        {
            return records;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for request_finished in {file}: {records:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_complete_accepted_request(root: &Path) -> Vec<Value> {
    wait_complete_accepted_requests(root, 1).remove(0)
}

fn wait_complete_accepted_requests(root: &Path, minimum: usize) -> Vec<Vec<Value>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = read_records(&root.join("execution-fact.jsonl"));
        let request_records = records
            .iter()
            .filter(|record| {
                record.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished")
                    && record.pointer("/fact/outcome").and_then(Value::as_str) == Some("accepted")
            })
            .filter_map(|finished| {
                let request_id = finished
                    .pointer("/correlation/request_id")
                    .and_then(Value::as_str)?;
                let request_records = records
                    .iter()
                    .filter(|record| {
                        record
                            .pointer("/correlation/request_id")
                            .and_then(Value::as_str)
                            == Some(request_id)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                request_records
                    .iter()
                    .any(|record| {
                        record.pointer("/fact/kind").and_then(Value::as_str)
                            == Some("usage_and_cache")
                            && record.pointer("/fact/source").and_then(Value::as_str)
                                == Some("provider_completion")
                    })
                    .then_some(request_records)
            })
            .collect::<Vec<_>>();
        if request_records.len() >= minimum {
            return request_records;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {minimum} accepted requests with provider usage: {records:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn assert_complete_accepted_request(
    facts: &[Value],
    native_model: &str,
    input_tokens: u64,
    output_tokens: u64,
) {
    let kinds = fact_kinds(facts);
    for expected in [
        "route_decision",
        "attempt_started",
        "attempt_finished",
        "request_finished",
        "usage_and_cache",
    ] {
        assert!(kinds.contains(&expected), "missing {expected}: {facts:#?}");
    }
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("attempt_started")
            && fact.pointer("/fact/request_model").and_then(Value::as_str) == Some(native_model)
    }));
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("attempt_finished")
            && fact.pointer("/fact/outcome").and_then(Value::as_str) == Some("accepted")
    }));
    assert!(facts.iter().any(|fact| {
        fact.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished")
            && fact.pointer("/fact/outcome").and_then(Value::as_str) == Some("accepted")
    }));
    let usage = facts
        .iter()
        .filter(|fact| {
            fact.pointer("/fact/kind").and_then(Value::as_str) == Some("usage_and_cache")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        usage.len(),
        1,
        "usage must have one Native authority: {facts:#?}"
    );
    assert_eq!(
        usage[0].pointer("/fact/source").and_then(Value::as_str),
        Some("provider_completion")
    );
    assert_eq!(
        usage[0]
            .pointer("/fact/input_tokens")
            .and_then(Value::as_u64),
        Some(input_tokens)
    );
    assert_eq!(
        usage[0]
            .pointer("/fact/output_tokens")
            .and_then(Value::as_u64),
        Some(output_tokens)
    );
}

fn request_id(facts: &[Value]) -> &str {
    let request_id = facts
        .first()
        .and_then(|fact| fact.pointer("/correlation/request_id"))
        .and_then(Value::as_str)
        .expect("request correlation");
    assert!(facts.iter().all(|fact| {
        fact.pointer("/correlation/request_id")
            .and_then(Value::as_str)
            == Some(request_id)
    }));
    request_id
}

fn assert_content_request_id(records: &[Value], request_id: &str) {
    assert!(records.iter().all(|record| {
        record
            .pointer("/correlation/request_id")
            .and_then(Value::as_str)
            == Some(request_id)
    }));
}

fn wait_for_gap(root: &Path, file: &str) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = read_records(&root.join(file));
        if records
            .iter()
            .any(|record| record.get("loss_watermark").is_some_and(Value::is_object))
        {
            return records;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {file}: {records:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_content_complete(root: &Path) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = read_records(&root.join("conversation-content.jsonl"));
        let finished = |direction: &str| {
            records.iter().any(|record| {
                record.get("direction").and_then(Value::as_str) == Some(direction)
                    && record.get("phase").and_then(Value::as_str) == Some("finish")
            })
        };
        if finished("request_input") && finished("response_delivered") {
            return records;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for completed content streams: {records:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_content_complete_for_requests(root: &Path, request_ids: &[String]) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = read_records(&root.join("conversation-content.jsonl"));
        if content_complete_for_requests(&records, request_ids) {
            return records;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for completed content streams: {records:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn content_complete_for_requests(records: &[Value], request_ids: &[String]) -> bool {
    request_ids.iter().all(|request_id| {
        ["request_input", "response_delivered"]
            .into_iter()
            .all(|direction| {
                records.iter().any(|record| {
                    record
                        .pointer("/correlation/request_id")
                        .and_then(Value::as_str)
                        == Some(request_id.as_str())
                        && record.get("direction").and_then(Value::as_str) == Some(direction)
                        && record.get("phase").and_then(Value::as_str) == Some("finish")
                })
            })
    })
}

fn read_records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .ok()
        .into_iter()
        .flat_map(|contents| contents.lines().map(str::to_owned).collect::<Vec<_>>())
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(&line).unwrap())
        .collect()
}

fn fact_kinds(facts: &[Value]) -> Vec<&str> {
    facts
        .iter()
        .filter_map(|fact| fact.pointer("/fact/kind").and_then(Value::as_str))
        .collect()
}

fn decoded_content(records: &[Value], direction: &str) -> String {
    let mut bytes = Vec::new();
    for encoded in records.iter().filter_map(|record| {
        (record.get("direction").and_then(Value::as_str) == Some(direction))
            .then(|| record.get("canonical_bytes_base64").and_then(Value::as_str))
            .flatten()
    }) {
        bytes.extend(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
        );
    }
    String::from_utf8(bytes).unwrap()
}

fn decoded_content_events(records: &[Value], direction: &str) -> Vec<Value> {
    let mut parts = BTreeMap::<u64, Vec<(u64, Vec<u8>)>>::new();
    for record in records.iter().filter(|record| {
        record.get("direction").and_then(Value::as_str) == Some(direction)
            && record.get("phase").and_then(Value::as_str) == Some("append")
    }) {
        let Some(part) = record.get("part_ordinal").and_then(Value::as_u64) else {
            continue;
        };
        let Some(chunk) = record.get("chunk_ordinal").and_then(Value::as_u64) else {
            continue;
        };
        let Some(encoded) = record.get("canonical_bytes_base64").and_then(Value::as_str) else {
            continue;
        };
        parts.entry(part).or_default().push((
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

fn response_tool_logical_id(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .unwrap()
        .pointer("/output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|item| {
            (item.get("type").and_then(Value::as_str) == Some("function_call"))
                .then(|| {
                    item.get("call_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .flatten()
        })
        .expect("downstream Tool logical ID")
}

fn assert_stream_lifecycle(records: &[Value], direction: &str) {
    let phases = records
        .iter()
        .filter(|record| record.get("direction").and_then(Value::as_str) == Some(direction))
        .filter_map(|record| record.get("phase").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(phases.first(), Some(&"begin"));
    assert!(phases.contains(&"append"));
    assert_eq!(phases.last(), Some(&"finish"));
}

fn assert_attempt_lifecycle(records: &[Value], ordinal: u64) {
    let position = |kind: &str| {
        records.iter().position(|record| {
            record.pointer("/fact/kind").and_then(Value::as_str) == Some(kind)
                && record.pointer("/fact/ordinal").and_then(Value::as_u64) == Some(ordinal)
        })
    };
    let started = position("attempt_started").expect("attempt start lifecycle fact");
    let finished = position("attempt_finished").expect("attempt finish lifecycle fact");
    assert!(started < finished, "attempt lifecycle must remain ordered");
}
