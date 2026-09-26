mod runtime_support;

use runtime_support::*;
use serde_json::{Value, json};

const RESPONSES_OK: &[u8] = br#"{"id":"context-hold","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
const RESPONSES_SEARCH_OK: &[u8] = br#"{"id":"context-search","model":"runtime-native","status":"completed","output":[{"type":"web_search_call","id":"native-search-1","status":"completed","action":{"type":"search","query":"weather today"}},{"type":"message","id":"native-message-1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"sunny","annotations":[{"type":"url_citation","start_index":0,"end_index":5,"title":"forecast","url":"https://example.test/forecast"}]}]}],"usage":{"input_tokens":2,"output_tokens":2,"total_tokens":4}}"#;
const RESPONSES_INCOMPLETE: &[u8] = br#"{"id":"context-incomplete","model":"runtime-native","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"message","id":"incomplete-message","status":"incomplete","role":"assistant","content":[{"type":"output_text","text":"partial","annotations":[]}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
const RESPONSES_FAILED_AFTER_SEMANTIC: &[u8] = br#"event: response.created
data: {"type":"response.created","response":{"id":"context-failed","model":"runtime-native"}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"failed-message","status":"in_progress","role":"assistant","content":[]}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"failed-message","output_index":0,"content_index":0,"delta":"partial"}

event: response.failed
data: {"type":"response.failed","response":{"id":"context-failed","model":"runtime-native","status":"failed","output":[{"type":"message","id":"failed-message","status":"incomplete","role":"assistant","content":[{"type":"output_text","text":"partial","annotations":[]}]}],"error":{"type":"server_error","message":"failed after semantic commit"}}}

"#;
const RESPONSES_CONFLICTING_SNAPSHOT: &[u8] = br#"event: response.created
data: {"type":"response.created","response":{"id":"context-conflict","model":"runtime-native"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"message","output_index":0,"content_index":0,"delta":"first"}

event: response.completed
data: {"type":"response.completed","response":{"id":"context-conflict","model":"runtime-native","status":"completed","output":[{"type":"message","id":"message","role":"assistant","content":[{"type":"output_text","text":"second"}]}]}}

"#;
const CHAT_LENGTH: &[u8] = br#"{"id":"chat-length","object":"chat.completion","model":"runtime-native","choices":[{"index":0,"message":{"role":"assistant","content":"partial"},"finish_reason":"length","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const CHAT_REFUSAL: &[u8] = br#"{"id":"chat-refusal","object":"chat.completion","model":"runtime-native","choices":[{"index":0,"message":{"role":"assistant","content":null,"refusal":"cannot comply"},"finish_reason":"content_filter","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const CHAT_OK: &[u8] = br#"{"id":"chat-ok","object":"chat.completion","model":"runtime-native","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const PROTOCOL_ERROR: &[u8] =
    br#"{"error":{"type":"protocol_error","message":"retry another candidate"}}"#;

fn complete() -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }
}

fn message(role: &str, text: &str) -> Value {
    json!({
        "type": "message",
        "role": role,
        "content": [{
            "type": if role == "assistant" { "output_text" } else { "input_text" },
            "text": text,
        }],
    })
}

fn send(fixture: &RuntimeFixture, session: &str, input: Vec<Value>) -> WireResponse {
    send_document(
        fixture,
        session,
        json!({
        "model": MODEL,
        "input": input,
        "stream": false,
        }),
    )
}

fn send_document(fixture: &RuntimeFixture, session: &str, document: Value) -> WireResponse {
    let body = serde_json::to_vec(&document).unwrap();
    request(
        fixture.address,
        "POST",
        "/v1/responses",
        &[
            ("X-HiRoute-Token", "runtime-token"),
            ("session-id", session),
        ],
        &body,
    )
}

fn request_json_body(wire: &[u8]) -> Value {
    let split = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("provider request has an HTTP head");
    serde_json::from_slice(&wire[split + 4..]).expect("provider request body is JSON")
}

#[test]
fn real_hirouted_keeps_conflicting_native_wire_but_does_not_hold_its_candidate() {
    let simple = NativeProvider::start(vec![
        ProviderReply::StreamComplete {
            status: 200,
            body: RESPONSES_CONFLICTING_SNAPSHOT,
        },
        complete(),
    ]);
    let complex = NativeProvider::start(vec![complete()]);
    let fixture = RuntimeFixture::launch_classified(&[&simple, &complex], 2);

    let first = send_document(
        &fixture,
        "conflicting-origin",
        json!({
            "model":MODEL,"input":[message("user","rename the readme")],"stream":true
        }),
    );
    assert_eq!(first.status, 200);
    let wire = String::from_utf8_lossy(&first.body);
    assert!(wire.contains("\"delta\":\"first\""));
    assert!(wire.contains("\"text\":\"second\""));
    assert_eq!((simple.calls(), complex.calls()), (1, 0));

    let second = send(
        &fixture,
        "conflicting-origin",
        vec![
            message("user", "rename the readme"),
            message("assistant", "second"),
            message("user", "complex-route explain the architecture"),
        ],
    );
    assert_eq!(second.status, 200);
    assert_eq!((simple.calls(), complex.calls()), (1, 1));
}

#[test]
fn real_hirouted_holds_only_inside_each_classified_branch() {
    hiroute_e2e::p0_execution_receipt!(
        "runtime.context_hold.classified",
        [
            "runtime.context_hold.simple_to_complex",
            "runtime.context_hold.complex_to_simple",
            "runtime.context_hold.rebuild",
            "runtime.context_hold.responses_search_append"
        ]
    );
    let simple = NativeProvider::start(vec![
        complete(),
        complete(),
        complete(),
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_SEARCH_OK,
        },
        complete(),
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_INCOMPLETE,
        },
        ProviderReply::StreamComplete {
            status: 200,
            body: RESPONSES_FAILED_AFTER_SEMANTIC,
        },
    ]);
    let complex = NativeProvider::start(vec![
        complete(),
        complete(),
        complete(),
        complete(),
        complete(),
    ]);
    let fixture = RuntimeFixture::launch_classified(&[&simple, &complex], 2);

    assert_eq!(
        send(
            &fixture,
            "simple-origin",
            vec![message("user", "rename the readme title")],
        )
        .status,
        200
    );
    assert_eq!((simple.calls(), complex.calls()), (1, 0));

    assert_eq!(
        send(
            &fixture,
            "simple-origin",
            vec![
                message("user", "rename the readme title"),
                message("assistant", "ok"),
                message("user", "complex-route architecture redesign"),
            ],
        )
        .status,
        200
    );
    assert_eq!(
        (simple.calls(), complex.calls()),
        (1, 1),
        "a newly complex turn must not inherit a hold from the simple branch"
    );

    assert_eq!(
        send(
            &fixture,
            "simple-origin",
            vec![message("user", "complex-route rebuilt conversation")],
        )
        .status,
        200
    );
    assert_eq!((simple.calls(), complex.calls()), (1, 2));

    assert_eq!(
        send(
            &fixture,
            "complex-origin",
            vec![message("user", "complex-route architecture redesign")],
        )
        .status,
        200
    );
    assert_eq!((simple.calls(), complex.calls()), (1, 3));

    assert_eq!(
        send(
            &fixture,
            "complex-origin",
            vec![
                message("user", "complex-route architecture redesign"),
                message("assistant", "ok"),
                message("user", "rename the readme title"),
            ],
        )
        .status,
        200
    );
    assert_eq!(
        (simple.calls(), complex.calls()),
        (2, 3),
        "a newly simple turn must not inherit a hold from the complex branch"
    );

    assert_eq!(
        send(
            &fixture,
            "complex-origin",
            vec![message("user", "rename after rebuilding conversation")],
        )
        .status,
        200
    );
    assert_eq!((simple.calls(), complex.calls()), (3, 3));

    let initial_search = json!({
        "model": MODEL,
        "input": [message("user", "weather")],
        "tools": [{"type":"web_search"}],
        "reasoning": {"effort":"low"},
        "stream": false,
    });
    let first_search = send_document(&fixture, "search-origin", initial_search);
    assert_eq!(
        first_search.status,
        200,
        "initial hosted-search request failed: body={} simple_calls={} complex_calls={}",
        String::from_utf8_lossy(&first_search.body),
        simple.calls(),
        complex.calls()
    );
    assert_eq!((simple.calls(), complex.calls()), (4, 3));
    let first_search: Value = serde_json::from_slice(&first_search.body).unwrap();
    let mut continued_input = vec![message("user", "weather")];
    continued_input.extend(
        first_search["output"]
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .map(|mut item| {
                if item["type"] == "message" {
                    item.as_object_mut().unwrap().remove("status");
                }
                item
            }),
    );
    continued_input.push(message("user", "explain tomorrow briefly"));
    let continued_search = send_document(
        &fixture,
        "search-origin",
        json!({
            "model": MODEL,
            "input": continued_input,
            "tools": [{"type":"web_search"}],
            "reasoning": {"effort":"high"},
            "stream": false,
        }),
    );
    assert_eq!(
        continued_search.status,
        200,
        "continued search failed: {}",
        String::from_utf8_lossy(&continued_search.body)
    );
    assert_eq!(
        (simple.calls(), complex.calls()),
        (5, 3),
        "completed search history remains eligible when the new turn selects the same branch"
    );
    let provider_request = simple.requests().last().cloned().unwrap();
    let provider_request = request_json_body(&provider_request);
    assert_eq!(
        provider_request["input"][1]["id"], "native-search-1",
        "the real Gateway must preserve prior hosted-search history inside the selected branch"
    );

    let incomplete = send(
        &fixture,
        "incomplete-origin",
        vec![message("user", "rename the partial readme")],
    );
    assert_eq!(incomplete.status, 200);
    let incomplete_body: Value = serde_json::from_slice(&incomplete.body).unwrap();
    assert_eq!(incomplete_body["status"], "incomplete");
    assert_eq!(incomplete_body["output"][0]["status"], "incomplete");
    assert_eq!((simple.calls(), complex.calls()), (6, 3));

    assert_eq!(
        send(
            &fixture,
            "incomplete-origin",
            vec![
                message("user", "rename the partial readme"),
                message("assistant", "partial"),
                message("user", "complex-route finish the architecture"),
            ],
        )
        .status,
        200
    );
    assert_eq!(
        (simple.calls(), complex.calls()),
        (6, 4),
        "an incomplete response must not establish a context hold"
    );

    let failed = send_document(
        &fixture,
        "failed-origin",
        json!({
            "model": MODEL,
            "input": [message("user", "rename before a streamed failure")],
            "stream": true,
        }),
    );
    assert_eq!(
        failed.status,
        200,
        "streamed provider failure was not delivered after semantic commit: {}",
        String::from_utf8_lossy(&failed.body)
    );
    let failed_body = String::from_utf8_lossy(&failed.body);
    assert!(failed_body.contains("partial"));
    assert!(failed_body.contains("event: response.failed"));
    assert!(failed_body.contains("failed after semantic commit"));
    assert_eq!((simple.calls(), complex.calls()), (7, 4));

    assert_eq!(
        send(
            &fixture,
            "failed-origin",
            vec![
                message("user", "rename before a streamed failure"),
                message("assistant", "partial"),
                message("user", "complex-route recover the architecture"),
            ],
        )
        .status,
        200
    );
    assert_eq!(
        (simple.calls(), complex.calls()),
        (7, 5),
        "a response.failed after semantic commit must not establish a context hold"
    );
    drop(fixture);

    let chat_simple = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CHAT_LENGTH,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CHAT_REFUSAL,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CHAT_OK,
        },
    ]);
    let chat_complex = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CHAT_OK,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CHAT_OK,
        },
    ]);
    let chat_candidates = [
        PublicationCandidate {
            provider_index: 0,
            upstream_protocol: "chat_completions",
            statically_enabled: true,
        },
        PublicationCandidate {
            provider_index: 1,
            upstream_protocol: "chat_completions",
            statically_enabled: true,
        },
    ];
    let chat_fixture = RuntimeFixture::launch_classified_with_publication_candidates(
        &[&chat_simple, &chat_complex],
        2,
        &chat_candidates,
    );

    let length = send(
        &chat_fixture,
        "chat-length-origin",
        vec![message("user", "rename until the token limit")],
    );
    assert_eq!(length.status, 200);
    let length_body: Value = serde_json::from_slice(&length.body).unwrap();
    assert_eq!(length_body["status"], "incomplete");
    assert_eq!(length_body["output"][0]["status"], "incomplete");
    assert_eq!(
        send(
            &chat_fixture,
            "chat-length-origin",
            vec![
                message("user", "rename until the token limit"),
                message("assistant", "partial"),
                message("user", "complex-route continue after length"),
            ],
        )
        .status,
        200
    );
    assert_eq!(
        (chat_simple.calls(), chat_complex.calls()),
        (1, 1),
        "a Chat length terminal must not establish a context hold"
    );

    let refusal = send(
        &chat_fixture,
        "chat-refusal-origin",
        vec![message("user", "rename content that will be refused")],
    );
    assert_eq!(refusal.status, 200);
    let refusal_body: Value = serde_json::from_slice(&refusal.body).unwrap();
    assert_eq!(refusal_body["status"], "incomplete");
    assert_eq!(refusal_body["output"][0]["status"], "incomplete");
    assert_eq!(
        send(
            &chat_fixture,
            "chat-refusal-origin",
            vec![
                message("user", "rename content that will be refused"),
                message("assistant", "cannot comply"),
                message("user", "complex-route recover after refusal"),
            ],
        )
        .status,
        200
    );
    assert_eq!(
        (chat_simple.calls(), chat_complex.calls()),
        (2, 2),
        "a Chat refusal terminal must not establish a context hold"
    );
}

#[test]
fn real_hirouted_holds_the_successful_fallback_and_releases_it_on_rebuild() {
    hiroute_e2e::p0_execution_receipt!(
        "runtime.context_hold.fallback",
        [
            "runtime.context_hold.success_only",
            "runtime.context_hold.fallback_origin"
        ]
    );
    let first = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 400,
            error_kind: None,
            body: PROTOCOL_ERROR,
        },
        complete(),
    ]);
    let fallback = NativeProvider::start(vec![complete(), complete()]);
    let fixture = RuntimeFixture::launch(&[&first, &fallback], 2);

    assert_eq!(
        send(
            &fixture,
            "fallback-origin",
            vec![message("user", "initial request")],
        )
        .status,
        200
    );
    assert_eq!((first.calls(), fallback.calls()), (1, 1));

    assert_eq!(
        send(
            &fixture,
            "fallback-origin",
            vec![
                message("user", "initial request"),
                message("assistant", "ok"),
                message("user", "continue"),
            ],
        )
        .status,
        200
    );
    assert_eq!(
        (first.calls(), fallback.calls()),
        (1, 2),
        "only the exact successful fallback becomes the next request's hold"
    );

    assert_eq!(
        send(
            &fixture,
            "fallback-origin",
            vec![message("user", "rebuilt request")],
        )
        .status,
        200
    );
    assert_eq!((first.calls(), fallback.calls()), (2, 2));
}

#[test]
fn real_hirouted_reverts_a_failed_branch_switch_to_the_last_completed_model() {
    let simple = NativeProvider::start(vec![ProviderReply::Complete {
        status: 400,
        error_kind: None,
        body: PROTOCOL_ERROR,
    }]);
    let complex = NativeProvider::start(vec![complete(), complete()]);
    let fixture = RuntimeFixture::launch_classified(&[&simple, &complex], 2);

    assert_eq!(
        send(
            &fixture,
            "switch-fallback",
            vec![message("user", "complex-route design the component")],
        )
        .status,
        200
    );
    assert_eq!((simple.calls(), complex.calls()), (0, 1));

    let switched = send(
        &fixture,
        "switch-fallback",
        vec![
            message("user", "complex-route design the component"),
            message("assistant", "ok"),
            message("user", "rename one label"),
        ],
    );
    assert_eq!(switched.status, 200);
    assert_eq!(
        (simple.calls(), complex.calls()),
        (1, 2),
        "the selected model's 400 must precede one precommit recovery on the last completed model"
    );
}

#[test]
fn real_hirouted_rejects_opaque_previous_response_id_before_provider_io() {
    hiroute_e2e::p0_execution_receipt!(
        "protocol.previous_response_id",
        ["protocol.previous_response_id_preconnect_rejection"]
    );
    let provider = NativeProvider::start(Vec::new());
    let fixture = RuntimeFixture::launch(&[&provider], 1);
    let response = fixture.request_body(
        br#"{"model":"runtime-model","input":"hello","stream":false,"previous_response_id":"response-1"}"#,
    );

    assert_eq!(response.status, 400);
    assert_eq!(provider.calls(), 0);
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap()["code"],
        "RESPONSES_PREVIOUS_RESPONSE_ID_UNSUPPORTED"
    );
}
