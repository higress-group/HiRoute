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
        assert_eq!(
            inbound_authorization("/v1/responses", &headers).as_deref(),
            Some("Bearer local-model-grant")
        );
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
fn messages_api_key_is_only_a_carrier_and_bearer_wins_when_both_are_present() {
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", HeaderValue::from_static("not-an-agent-grant"));
    // The listener's current-grant verifier, not extraction, rejects this value.
    assert_eq!(
        inbound_authorization("/v1/messages", &headers).as_deref(),
        Some("Bearer not-an-agent-grant")
    );

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
fn native_messages_api_key_carrier_preserves_run_model_and_rejects_malformed_values() {
    let mut headers = HeaderMap::new();
    for token in [
        "",
        "first,second",
        "first second",
        "hr_run_unknown",
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
fn ordinary_model_entries_share_the_standard_bearer_carrier() {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer local-model-grant"),
    );
    for path in [
        "/v1/responses",
        "/v1/chat/completions",
        "/v1/messages",
        "/v1/models",
    ] {
        assert_eq!(
            inbound_authorization(path, &headers).as_deref(),
            Some("Bearer local-model-grant")
        );
    }
    headers.remove(AUTHORIZATION);
    headers.insert("x-api-key", HeaderValue::from_static("local-model-grant"));
    assert_eq!(
        inbound_authorization("/v1/messages", &headers).as_deref(),
        Some("Bearer local-model-grant")
    );
}

#[test]
fn explicit_model_grant_keeps_precedence_over_native_sdk_authorization() {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer native-oauth"),
    );
    // Extraction alone does not authenticate native-oauth; the issued-grant
    // verifier denies it. A retained explicit carrier still takes precedence.
    headers.insert("x-hiroute-token", HeaderValue::from_static("model-grant"));
    assert_eq!(
        inbound_authorization("/v1/responses", &headers).as_deref(),
        Some("Bearer model-grant")
    );
    assert_eq!(
        inbound_authorization("/v1/messages", &headers).as_deref(),
        Some("Bearer model-grant")
    );
    assert_eq!(
        inbound_authorization("/v1/chat/completions", &headers).as_deref(),
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
    assert!(inbound_authorization("/v1/chat/completions", &headers).is_none());
    headers.insert("x-hiroute-token", HeaderValue::from_static("first,second"));
    assert!(inbound_authorization("/v1/responses", &headers).is_none());
    headers.remove("x-hiroute-token");
    headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer first"));
    headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer second"));
    assert!(inbound_authorization("/v1/messages", &headers).is_none());
    headers.clear();
    for invalid in [
        "Basic local-model-grant",
        "Bearer ",
        "Bearer first,second",
        "Bearer first second",
    ] {
        headers.insert(AUTHORIZATION, HeaderValue::from_static(invalid));
        for path in [
            "/v1/responses",
            "/v1/chat/completions",
            "/v1/messages",
            "/v1/models",
        ] {
            assert!(
                inbound_authorization(path, &headers).is_none(),
                "{path}: {invalid}"
            );
        }
    }
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
