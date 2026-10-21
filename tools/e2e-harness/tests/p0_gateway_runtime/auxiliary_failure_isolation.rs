use super::*;
use serde_json::{Value, json};

const AUXILIARY_FAILURE: &[u8] = b"private title generation transient failure";

fn send_agent_request(address: std::net::SocketAddr, input: &str) -> WireResponse {
    // Hermes 0.20 title generation sends the Agent grant with no native purpose
    // marker or session. The protection must also cover this ordinary wire call.
    request(
        address,
        "POST",
        "/v1/responses",
        &[("X-HiRoute-Token", TOKEN)],
        &serde_json::to_vec(&json!({"model": MODEL, "input": input, "stream": false})).unwrap(),
    )
}

fn assert_original_failure(response: &WireResponse) {
    assert_eq!(response.status, 503);
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap(),
        json!({"error": {"code": "UPSTREAM_ATTEMPT_FAILED", "type": "upstream_error"}})
    );
}

fn assert_success(response: &WireResponse) {
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["id"], "accepted");
}

#[test]
fn real_listener_unmarked_title_failure_does_not_cool_the_next_main_request() {
    let provider = NativeProvider::start(vec![
        ProviderReply::CompleteWithRetryAfter {
            status: 503,
            retry_after_secs: 77,
            body: AUXILIARY_FAILURE,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let fixture = RuntimeFixture::launch(&[&provider], 1);

    let title = send_agent_request(fixture.address, "Generate a short session title.");
    assert_original_failure(&title);
    // One raw HTTP request, without sleeps or a native SDK retry, must reach
    // the same source despite the failed title's trusted Retry-After.
    let main = send_agent_request(fixture.address, "Continue the user's task.");
    assert_success(&main);
    assert_eq!(provider.calls(), 2);
}

#[test]
fn real_listener_title_failure_releases_an_expired_quota_probe_for_the_next_main() {
    let provider = NativeProvider::start(vec![
        ProviderReply::CompleteWithRetryAfter {
            status: 429,
            retry_after_secs: 0,
            body: QUOTA_ERROR,
        },
        ProviderReply::Complete {
            status: 503,
            error_kind: None,
            body: AUXILIARY_FAILURE,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let fixture = RuntimeFixture::launch(&[&provider], 1);

    assert_eq!(
        fixture.request().status,
        429,
        "seed an expired exact Key cooldown"
    );
    let title = send_agent_request(fixture.address, "Generate a short session title.");
    assert_original_failure(&title);
    let main = send_agent_request(fixture.address, "Continue the user's task.");
    assert_success(&main);
    assert_eq!(provider.calls(), 3);
}

#[test]
fn real_listener_permanent_title_error_releases_an_expired_quota_probe_for_the_next_main() {
    let provider = NativeProvider::start(vec![
        ProviderReply::CompleteWithRetryAfter {
            status: 429,
            retry_after_secs: 0,
            body: QUOTA_ERROR,
        },
        ProviderReply::Complete {
            status: 400,
            error_kind: None,
            body: PERMANENT_ERROR,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let fixture = RuntimeFixture::launch(&[&provider], 1);

    assert_eq!(
        fixture.request().status,
        429,
        "seed an expired exact Key cooldown"
    );
    let title = send_agent_request(fixture.address, "Generate a short session title.");
    assert_eq!(title.status, 400);
    assert_eq!(
        serde_json::from_slice::<Value>(&title.body).unwrap(),
        json!({"error": {"code": "UPSTREAM_ATTEMPT_FAILED", "type": "upstream_error"}})
    );
    let main = send_agent_request(fixture.address, "Continue the user's task.");
    assert_success(&main);
    assert_eq!(provider.calls(), 3);
}

#[test]
fn real_listener_agent_quota_failure_retains_shared_cooldown_and_retry_contract() {
    let provider = NativeProvider::start(vec![ProviderReply::CompleteWithRetryAfter {
        status: 429,
        retry_after_secs: 2,
        body: QUOTA_ERROR,
    }]);
    let fixture = RuntimeFixture::launch(&[&provider], 1);

    assert_eq!(fixture.request().status, 429);
    let main = send_agent_request(fixture.address, "Continue the user's task.");
    assert_eq!(main.status, 503);
    assert_eq!(
        provider.calls(),
        1,
        "an exact Key quota still suppresses upstream I/O"
    );
    let retry_after = main.headers["retry-after"].parse::<u64>().unwrap();
    assert!((1..=2).contains(&retry_after));
    assert_eq!(
        serde_json::from_slice::<Value>(&main.body).unwrap(),
        json!({
            "schema_version": "hiroute.gateway.error/v1",
            "code": "CANDIDATES_COOLING_DOWN",
            "phase": "runtime_state",
            "retry_after_seconds": retry_after,
        })
    );
}

struct ReleaseOnDrop(ProviderBodyBarrier);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[test]
fn real_listener_title_failure_does_not_invalidate_an_inflight_main_permit() {
    let barrier = ProviderBodyBarrier::default();
    let provider = NativeProvider::start(vec![
        ProviderReply::BodyReadBarrier {
            barrier: barrier.clone(),
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
        ProviderReply::Complete {
            status: 503,
            error_kind: None,
            body: AUXILIARY_FAILURE,
        },
    ]);
    let fixture = RuntimeFixture::launch(&[&provider], 1);
    let address = fixture.address;

    let main = std::thread::scope(|scope| {
        let release = ReleaseOnDrop(barrier.clone());
        let main = scope.spawn(move || send_agent_request(address, "Finish the user's task."));
        barrier.wait_for_arrivals(1, Duration::from_secs(5));
        let title = send_agent_request(address, "Generate a short session title.");
        assert_original_failure(&title);
        release.0.release();
        main.join().unwrap()
    });
    assert_success(&main);
    assert_eq!(provider.calls(), 2);
}
