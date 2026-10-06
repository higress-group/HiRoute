use super::*;

#[test]
fn qoder_model_entry_is_explicit_and_never_adopts_worker_or_mixed_credentials() {
    let path = format!("{}/responses", hiroute_domain::QODER_MODEL_BASE_PATH);
    let messages = format!("{}/messages", hiroute_domain::QODER_MODEL_BASE_PATH);
    assert_eq!(
        IngressProtocol::from_path(&messages),
        Some(IngressProtocol::Messages)
    );
    let catalog = format!("{}/models", hiroute_domain::QODER_MODEL_BASE_PATH);
    assert_eq!(
        IngressProtocol::from_path(&path),
        Some(IngressProtocol::Responses)
    );
    let mut headers = HeaderMap::new();
    for target in [&path, &messages, &catalog] {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer local-model-grant"),
        );
        assert_eq!(
            inbound_authorization(target, &headers).as_deref(),
            Some("Bearer local-model-grant")
        );
        assert!(inbound_authorization("/v1/responses", &headers).is_none());
        headers.insert(
            "x-hiroute-token",
            HeaderValue::from_static("local-model-grant"),
        );
        assert!(inbound_authorization(target, &headers).is_none());
        headers.remove("x-hiroute-token");
        for invalid in [
            "Bearer hr_run_model_fixture",
            "Bearer hr_run_control_fixture",
            "Bearer ",
            "Bearer first,second",
            "Bearer first second",
            "Basic local-model-grant",
        ] {
            headers.insert(AUTHORIZATION, HeaderValue::from_static(invalid));
            assert!(
                inbound_authorization(target, &headers).is_none(),
                "{invalid}"
            );
        }
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer first"));
        headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer second"));
        assert!(inbound_authorization(target, &headers).is_none());
        headers.clear();
    }
    assert!(
        IngressProtocol::from_path(&format!(
            "{}/responses/extra",
            hiroute_domain::QODER_MODEL_BASE_PATH
        ))
        .is_none()
    );
}

#[test]
fn ordinary_x_api_key_never_authenticates_and_bearer_wins_when_both_are_present() {
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
fn native_messages_api_key_carrier_is_limited_to_run_model_authority() {
    let mut headers = HeaderMap::new();
    for token in [
        "native-key",
        "local-model-grant",
        "hr_run_control_fixture",
        "hr_run_model_first,second",
        "hr_run_model_first second",
    ] {
        headers.insert("x-api-key", HeaderValue::from_str(token).unwrap());
        assert!(
            inbound_authorization("/v1/messages", &headers).is_none(),
            "{token}"
        );
    }
    headers.insert(
        "x-api-key",
        HeaderValue::from_static("hr_run_model_fixture"),
    );
    assert_eq!(
        inbound_authorization("/v1/messages", &headers).as_deref(),
        Some("Bearer hr_run_model_fixture")
    );
    for path in [
        "/v1/models",
        "/v1/responses",
        "/v1/messages/extra",
        "/qoder/messages",
    ] {
        assert!(inbound_authorization(path, &headers).is_none(), "{path}");
    }
    headers.append("x-api-key", HeaderValue::from_static("hr_run_model_second"));
    assert!(inbound_authorization("/v1/messages", &headers).is_none());
    headers.remove("x-api-key");
    headers.insert(
        "x-api-key",
        HeaderValue::from_static("hr_run_model_fixture"),
    );
    headers.insert(
        "x-hiroute-token",
        HeaderValue::from_static("hr_run_model_fixture"),
    );
    assert!(inbound_authorization("/v1/messages", &headers).is_none());
}

#[test]
fn model_entry_uses_independent_grant_without_native_bearer_fallback() {
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
    assert_eq!(
        inbound_authorization("/v1/messages", &headers).as_deref(),
        Some("Bearer model-grant")
    );
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
