use super::*;
use serde_json::{Value, json};

fn complete(body: &'static [u8]) -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body,
    }
}

fn document(input: Value, stream: bool) -> Vec<u8> {
    serde_json::to_vec(&json!({"model":MODEL,"input":input,"stream":stream})).unwrap()
}

fn send(address: std::net::SocketAddr, headers: &[(&str, &str)], input: Value) -> WireResponse {
    request(
        address,
        "POST",
        "/v1/responses",
        headers,
        &document(input, false),
    )
}

fn assert_ok(response: &WireResponse) {
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["status"], "completed", "{body}");
}

fn classifier_body(classifier: &NativeProvider, index: usize) -> Value {
    let requests = classifier.requests();
    let raw = &requests[index];
    let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&raw[start..]).unwrap()
}

struct ReleaseOnDrop(ProviderBodyBarrier);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[test]
fn real_listener_history_bypass_keeps_overlapping_native_requests_and_main_continuation() {
    let identities: &[&[(&str, &str)]] = &[
        &[("x-claude-code-session-id", "shared-claude")],
        &[
            ("thread-id", "shared-thread"),
            ("session-id", "shared-codex"),
        ],
        &[("session-id", "shared-qoder")],
        &[
            ("User-Agent", "pi (linux; x64)"),
            ("x-client-request-id", "shared-pi"),
        ],
        &[("x-deepseek-harness-session-id", "shared-dsh")],
        &[("x-hermes-session-id", "shared-hermes")],
        &[("x-opencode-session-id", "shared-opencode")],
    ];
    let mut executed = 0;
    for identity in identities {
        let barrier = ProviderBodyBarrier::default();
        let simple = NativeProvider::start(vec![
            ProviderReply::BodyReadBarrier {
                barrier: barrier.clone(),
                status: 200,
                error_kind: None,
                body: RESPONSES_TOOL_CALL,
            },
            complete(RESPONSES_OK),
        ]);
        let complex = NativeProvider::start(vec![complete(RESPONSES_OK)]);
        let classifier = NativeProvider::start(vec![
            complete(CLASSIFIER_SIMPLE),
            complete(CLASSIFIER_COMPLEX),
        ]);
        let fixture = RuntimeFixture::launch_rest_classified_with_observation(
            &[&simple, &complex, &classifier],
            1,
            ObservationFaults::healthy(),
        );
        let mut headers = identity.to_vec();
        headers.push(("X-HiRoute-Token", TOKEN));
        let address = fixture.address;
        let user = json!({"role":"user","content":"Look up the record."});
        let first = std::thread::scope(|scope| {
            // Release inside the scoped closure even if an assertion fails, so
            // the provider and the first client cannot deadlock test cleanup.
            let release = ReleaseOnDrop(barrier.clone());
            let main_headers = headers.clone();
            let main_user = user.clone();
            let main = scope.spawn(move || send(address, &main_headers, json!([main_user])));
            barrier.wait_for_arrivals(1, Duration::from_secs(5));
            let background = send(address, &headers, json!("An independent background task."));
            assert_ok(&background);
            assert_eq!(
                complex.calls(),
                1,
                "a conflicting request must execute independently"
            );
            let detached = classifier_body(&classifier, 1);
            assert_eq!(detached["history_partial"], true, "{detached}");
            assert_eq!(detached["visible_conversation"], json!([]), "{detached}");
            assert!(detached["assessment_target"].is_null(), "{detached}");
            release.0.release();
            main.join().unwrap()
        });
        assert_ok(&first);
        let first: Value = serde_json::from_slice(&first.body).unwrap();
        let call_id = &first["output"][0]["call_id"];
        let continued = send(
            address,
            &headers,
            json!([
                user,
                {"type":"function_call","call_id":call_id,"namespace":"tools","name":"lookup","arguments":"{}"},
                {"type":"function_call_output","call_id":call_id,"output":"record found"}
            ]),
        );
        assert_ok(&continued);
        assert_eq!((simple.calls(), complex.calls()), (2, 1), "{identity:?}");
        assert_eq!(
            classifier.calls(),
            2,
            "bypass must not invalidate the main tool continuation"
        );
        let facts = wait_execution_facts(&fixture, 3);
        assert!(
            facts.iter().any(|record| {
                record
                    .pointer("/fact/complexity/reason_codes")
                    .and_then(Value::as_array)
                    .is_some_and(|codes| codes.contains(&json!("task_context_unresolved")))
            }),
            "the detached request must disclose unavailable history"
        );
        executed += 1;
    }
    assert_eq!(
        executed, 7,
        "every supported native identity family must execute"
    );
}

#[test]
fn real_listener_history_bypass_capacity_exhaustion_keeps_valid_responses_and_authority() {
    let simple = NativeProvider::start(vec![complete(RESPONSES_OK), complete(RESPONSES_OK)]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![
        complete(CLASSIFIER_SIMPLE),
        complete(CLASSIFIER_SIMPLE),
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_history_capacity(
        &[&simple, &complex, &classifier],
        1,
    );
    let headers = [
        ("X-HiRoute-Token", TOKEN),
        ("session-id", "capacity-session"),
    ];
    for text in ["first task", "second task"] {
        assert_ok(&send(fixture.address, &headers, json!(text)));
    }
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (2, 0, 2)
    );
    for index in 0..2 {
        let body = classifier_body(&classifier, index);
        assert_eq!(body["history_partial"], true, "{body}");
        assert_eq!(body["visible_conversation"], json!([]), "{body}");
        assert!(body["assessment_target"].is_null(), "{body}");
    }
    let denied = send(
        fixture.address,
        &[("X-HiRoute-Token", "unissued-token")],
        json!("denied"),
    );
    assert_ne!(denied.status, 200);
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (2, 0, 2),
        "history degradation cannot grant model access"
    );
}

#[test]
fn real_listener_history_bypass_request_without_user_turn_still_executes() {
    let simple = NativeProvider::start(vec![complete(RESPONSES_OK)]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(vec![complete(CLASSIFIER_SIMPLE)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&simple, &complex, &classifier],
        1,
        ObservationFaults::healthy(),
    );
    let response = send(
        fixture.address,
        &[
            ("X-HiRoute-Token", TOKEN),
            ("session-id", "auxiliary-session"),
        ],
        json!([{"role":"assistant","content":"Summarize the previous result."}]),
    );
    assert_ok(&response);
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (1, 0, 1)
    );
    let body = classifier_body(&classifier, 0);
    assert_eq!(body["history_partial"], true, "{body}");
    assert_eq!(body["visible_conversation"], json!([]), "{body}");
    assert!(body["assessment_target"].is_null(), "{body}");
}

#[test]
fn real_listener_history_bypass_classifier_projection_exhaustion_keeps_model_payload() {
    let simple = NativeProvider::start(Vec::new());
    let complex = NativeProvider::start(vec![complete(RESPONSES_OK)]);
    let classifier = NativeProvider::start(Vec::new());
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&simple, &complex, &classifier],
        1,
        ObservationFaults::healthy(),
    );
    // Larger than the production per-request semantic memory budget; the
    // request-owned Replay stream can still forward the complete input.
    let user = "z".repeat(9 * 1024 * 1024);
    let response = request_with_timeout(
        fixture.address,
        "POST",
        "/v1/responses",
        &[
            ("X-HiRoute-Token", TOKEN),
            ("session-id", "large-projection"),
        ],
        &document(json!(user), false),
        Duration::from_secs(30),
    );
    assert_ok(&response);
    assert_eq!(
        (simple.calls(), complex.calls(), classifier.calls()),
        (0, 1, 0)
    );
    let captured = classifier_body(&complex, 0);
    assert_eq!(
        captured["input"].as_str(),
        Some(user.as_str()),
        "judgment resource fallback cannot truncate model input"
    );
    let facts = wait_execution_facts(&fixture, 1);
    assert_eq!(
        facts[0].pointer("/fact/complexity/fallback_used"),
        Some(&json!(true))
    );
}

#[test]
fn real_listener_history_bypass_observation_faults_preserve_the_complete_stream() {
    let faults = [
        ObservationFaults::healthy(),
        ObservationFaults {
            queue_bytes: 64,
            ..ObservationFaults::healthy()
        },
        ObservationFaults {
            lifecycle_mode: "fail_once",
            execution_mode: "fail_once",
            content_mode: "fail_once",
            otel_mode: "fail_once",
            ..ObservationFaults::healthy()
        },
        ObservationFaults {
            lifecycle_mode: "panic_once",
            execution_mode: "panic_once",
            content_mode: "panic_once",
            otel_mode: "panic_once",
            ..ObservationFaults::healthy()
        },
        ObservationFaults {
            lifecycle_mode: "slow_once:10000",
            execution_mode: "slow_once:10000",
            content_mode: "slow_once:10000",
            otel_mode: "slow_once:10000",
            ..ObservationFaults::healthy()
        },
    ];
    let mut baseline = None;
    for fault in faults {
        let simple = NativeProvider::start(vec![ProviderReply::StreamComplete {
            status: 200,
            body: RESPONSES_STREAM_OK,
        }]);
        let complex = NativeProvider::start(Vec::new());
        let classifier = NativeProvider::start(vec![complete(CLASSIFIER_SIMPLE)]);
        let fixture = RuntimeFixture::launch_rest_classified_with_observation(
            &[&simple, &complex, &classifier],
            1,
            fault,
        );
        let response = request_with_timeout(
            fixture.address,
            "POST",
            "/v1/responses",
            &[("X-HiRoute-Token", TOKEN), ("session-id", "fault-session")],
            &document(json!("Answer the task."), true),
            Duration::from_secs(5),
        );
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let body = String::from_utf8(response.body.clone()).unwrap();
        assert!(body.contains("event: response.completed"), "{body}");
        assert!(body.contains("\"delta\":\"ok\""), "{body}");
        if let Some(expected) = &baseline {
            assert_eq!(
                &response.body, expected,
                "side-path faults cannot change delivered bytes"
            );
        } else {
            baseline = Some(response.body);
        }
        assert_eq!(
            (simple.calls(), complex.calls(), classifier.calls()),
            (1, 0, 1),
            "side-path failure cannot retry or replace the model result"
        );
    }
}
