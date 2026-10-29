mod runtime_support;

use std::time::Duration;

use runtime_support::*;

#[test]
fn real_listener_precommit_disconnect_reaches_next_frozen_candidate() {
    let first = NativeProvider::start(vec![ProviderReply::CloseBeforeSemantic]);
    let second = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: br#"{"id":"fallback-after-precommit","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#,
    }]);
    let fixture = RuntimeFixture::launch(&[&first, &second], 2);

    let response = fixture.request();

    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let document: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(document["id"], "fallback-after-precommit");
    assert_eq!(first.calls(), 1);
    assert_eq!(second.calls(), 1);
}

#[test]
fn real_listener_keeps_mechanical_failure_local_to_each_request() {
    let first = NativeProvider::start(vec![
        ProviderReply::CloseBeforeSemantic,
        ProviderReply::CloseBeforeSemantic,
    ]);
    let second = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: br#"{"id":"fallback-after-disconnect","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"first"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: br#"{"id":"fallback-after-second-disconnect","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"second"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#,
        },
    ]);
    let fixture = RuntimeFixture::launch(&[&first, &second], 2);

    let first_response = fixture.request();
    // A different first user also excludes the generic API's inferred hold
    // preference; this assertion measures health rather than same-turn affinity.
    let second_response = fixture
        .request_body(br#"{"model":"runtime-model","input":"independent request","stream":false}"#);

    assert_eq!(first_response.status, 200);
    assert_eq!(second_response.status, 200);
    assert_eq!(
        first.calls(),
        2,
        "the independent request must try its first frozen candidate again"
    );
    assert_eq!(second.calls(), 2);
    let document: serde_json::Value = serde_json::from_slice(&second_response.body).unwrap();
    assert_eq!(document["id"], "fallback-after-second-disconnect");
}

#[test]
fn fixed_postcommit_disconnect_never_retries_keys_or_enters_another_plan() {
    let primary = NativeProvider::start(vec![ProviderReply::StreamThenClose {
        body: b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"fixed-prefix\",\"model\":\"runtime-native\"}}\n\nevent: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"fixed-prefix\"}\n\n",
        declared_length: 1_024,
    }]);
    let other_account = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: br#"{"id":"forbidden-account"}"#,
    }]);
    let other_model = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: br#"{"id":"forbidden-model"}"#,
    }]);
    let fixture = RuntimeFixture::launch_fixed(
        &[&primary, &other_account, &other_model],
        3,
        &[2, 1, 1],
        None,
    );

    let response =
        fixture.request_body(br#"{"model":"runtime-model","input":"hello","stream":true}"#);

    assert_eq!(response.status, 200);
    assert!(
        !response.http_entity_complete,
        "post-commit failure must remain an incomplete HTTP entity"
    );
    let body = String::from_utf8_lossy(&response.body);
    assert!(body.contains("fixed-prefix"));
    assert!(!body.contains("forbidden-"));
    assert!(!body.contains("response.completed"));
    assert_eq!(primary.calls(), 1, "commit closes same-source key retry");
    assert_eq!((other_account.calls(), other_model.calls()), (0, 0));
}

#[test]
fn real_listener_postcommit_disconnect_never_creates_another_attempt() {
    hiroute_e2e::p0_execution_receipt!(
        "runtime.postcommit",
        [
            "runtime.postcommit_no_retry",
            "runtime.postcommit_provider_count"
        ]
    );
    let first = NativeProvider::start(vec![ProviderReply::StreamThenClose {
        body: b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"semantic-prefix\",\"model\":\"runtime-native\"}}\n\nevent: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"semantic-prefix\"}\n\n",
        declared_length: 1_024,
    }]);
    let second = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: br#"{"id":"forbidden-second"}"#,
    }]);
    let fixture = RuntimeFixture::launch(&[&first, &second], 2);

    let response =
        fixture.request_body(br#"{"model":"runtime-model","input":"hello","stream":true}"#);
    std::thread::sleep(Duration::from_millis(100));

    assert_eq!(response.status, 200);
    assert!(
        !response.http_entity_complete,
        "provider disconnect must reach the downstream HTTP boundary"
    );
    assert!(
        response
            .body
            .windows(b"semantic-prefix".len())
            .any(|window| window == b"semantic-prefix")
    );
    assert_eq!(first.calls(), 1);
    assert_eq!(
        second.calls(),
        0,
        "downstream semantic commit must permanently close fallback"
    );
}

#[test]
fn real_listener_postcommit_deadline_preserves_prefix_without_successful_eof() {
    let prefix = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"deadline-prefix\",\"model\":\"runtime-native\"}}\n\nevent: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"deadline-prefix\"}\n\n";
    let first = NativeProvider::start(vec![ProviderReply::StreamDrip {
        status: 200,
        chunks: vec![prefix.to_vec(), b": still waiting\n\n".to_vec()],
        interval: Duration::from_millis(1_500),
    }]);
    let second = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: br#"{"id":"forbidden-after-deadline"}"#,
    }]);
    let fixture = RuntimeFixture::launch_with_replay_timeout(&[&first, &second], 2, 64 * 1024, 800);
    let response = fixture
        .request_body(br#"{"model":"runtime-model","input":"deadline check","stream":true}"#);
    assert_eq!(response.status, 200);
    assert!(!response.http_entity_complete);
    assert!(String::from_utf8_lossy(&response.body).contains("deadline-prefix"));
    assert!(!String::from_utf8_lossy(&response.body).contains("response.completed"));
    assert_eq!((first.calls(), second.calls()), (1, 0));
}
