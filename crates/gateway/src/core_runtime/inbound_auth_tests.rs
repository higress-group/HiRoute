use super::*;

#[test]
fn x_api_key_never_authenticates_and_bearer_wins_when_both_are_present() {
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", HeaderValue::from_static("not-an-agent-grant"));
    assert_eq!(inbound_authorization("/v1/messages", &headers), None);

    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer agent-grant"),
    );
    assert_eq!(
        inbound_authorization("/v1/messages", &headers).as_deref(),
        Some("Bearer agent-grant")
    );
}

#[test]
fn codex_requires_independent_grant_without_native_bearer_fallback() {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer native-oauth"),
    );
    assert!(inbound_authorization("/v1/responses", &headers).is_none());
    headers.insert("x-hiroute-token", HeaderValue::from_static("model-grant"));
    assert_eq!(
        inbound_authorization("/v1/responses", &headers).as_deref(),
        Some("Bearer model-grant")
    );
    assert!(inbound_authorization("/v1/messages", &headers).is_none());
    headers.insert("x-hiroute-token", HeaderValue::from_static(""));
    assert!(inbound_authorization("/v1/responses", &headers).is_none());
}

#[test]
fn repeated_or_combined_grant_headers_are_rejected() {
    let mut headers = HeaderMap::new();
    headers.append("x-hiroute-token", HeaderValue::from_static("first"));
    headers.append("x-hiroute-token", HeaderValue::from_static("second"));
    assert!(inbound_authorization("/v1/responses", &headers).is_none());
    headers.insert("x-hiroute-token", HeaderValue::from_static("first,second"));
    assert!(inbound_authorization("/v1/responses", &headers).is_none());
    headers.remove("x-hiroute-token");
    headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer first"));
    headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer second"));
    assert!(inbound_authorization("/v1/messages", &headers).is_none());
}

#[test]
fn worker_run_authority_retains_its_separate_bearer_channel() {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer hr_run_model_fixture"),
    );
    assert_eq!(
        inbound_authorization("/v1/responses", &headers).as_deref(),
        Some("Bearer hr_run_model_fixture")
    );
    headers.insert(
        "x-hiroute-token",
        HeaderValue::from_static("hr_run_model_fixture"),
    );
    assert!(inbound_authorization("/v1/responses", &headers).is_none());
}
