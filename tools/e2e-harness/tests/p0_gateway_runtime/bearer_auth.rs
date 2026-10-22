use super::*;
use hiroute_gateway::server::publication::token_sha256;

#[test]
fn native_messages_api_key_preserves_grant_scope_precedence_rotation_and_outbound_isolation() {
    use hiroute_gateway::server::request_plan::IngressProtocol;

    let upstream = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        };
        5
    ]);
    let fixture = RuntimeFixture::launch_with_runtime_state_control(&[&upstream], 1);
    let mut publication = fixture.publication();
    publication.aliases[0]
        .protocols
        .push(IngressProtocol::Messages);
    let candidate = &mut publication.aliases[0].candidates[0];
    *candidate = hiroute_e2e::gateway_fixture::sealed_native_candidate(
        candidate.local_id,
        &candidate.stable_target_key,
        &candidate.credential_refs,
        upstream.authority(),
        &candidate.upstream_model_id,
        &[
            (IngressProtocol::Responses, IngressProtocol::Responses),
            (IngressProtocol::Messages, IngressProtocol::Responses),
        ],
    );
    let mut grant = publication.grants[0].clone();
    grant.grant_id = "runtime-messages-grant".into();
    grant.protocol = IngressProtocol::Messages;
    grant.bearer_token_sha256 = token_sha256(MESSAGES_TOKEN);
    publication.grants.push(grant);
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    let body = br#"{"model":"runtime-model","max_tokens":16,"messages":[{"role":"user","content":"Approve the bounded native tool request."}]}"#;
    let bearer = format!("Bearer {MESSAGES_TOKEN}");
    let send = |headers: &[(&str, &str)], body: &[u8]| {
        request(fixture.address, "POST", "/v1/messages", headers, body)
    };
    // Prove the native Messages fixture and existing carriers before testing
    // the exact carrier used by native auxiliary Anthropic SDK calls.
    assert_eq!(
        send(
            &[
                ("X-HiRoute-Token", MESSAGES_TOKEN),
                ("Authorization", "Bearer native-oauth"),
                ("x-api-key", "native-key"),
            ],
            body,
        )
        .status,
        200
    );
    assert_eq!(
        send(
            &[("Authorization", &bearer), ("x-api-key", "native-key")],
            body
        )
        .status,
        200
    );
    let response = send(&[("x-api-key", MESSAGES_TOKEN)], body);
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );

    for token in [
        "native-key",
        "native-oauth",
        "provider-secret-1",
        "unissued-connection-token",
        "hr_run_control_fixture",
        "hr_run_model_unissued",
        "",
        "first,second",
        "first second",
    ] {
        assert_eq!(send(&[("x-api-key", token)], body).status, 401, "{token}");
    }
    for extra in [
        vec![("x-api-key", MESSAGES_TOKEN)],
        vec![("Authorization", "Bearer unissued")],
        vec![("Authorization", "")],
        vec![
            ("Authorization", bearer.as_str()),
            ("Authorization", bearer.as_str()),
        ],
        vec![("X-HiRoute-Token", "")],
        vec![("X-HiRoute-Token", "unissued")],
        vec![
            ("X-HiRoute-Token", MESSAGES_TOKEN),
            ("X-HiRoute-Token", MESSAGES_TOKEN),
        ],
    ] {
        let mut headers = vec![("x-api-key", MESSAGES_TOKEN)];
        headers.extend(extra);
        assert_eq!(
            send(&headers, body).status,
            401,
            "no fallback from a higher-priority carrier"
        );
    }
    assert_eq!(send(&[("x-api-key", TOKEN)], body).status, 422);
    assert_eq!(
        send(
            &[("x-api-key", MESSAGES_TOKEN)],
            br#"{"model":"not-in-grant","max_tokens":16,"messages":[]}"#,
        )
        .status,
        404
    );
    assert_eq!(
        request(
            fixture.address,
            "POST",
            "/v1/responses",
            &[("x-api-key", TOKEN)],
            br#"{"model":"runtime-model","input":"denied"}"#
        )
        .status,
        401
    );
    assert_eq!(
        upstream.calls(),
        3,
        "denied requests make no upstream attempt"
    );

    let grant = publication
        .grants
        .iter_mut()
        .find(|grant| grant.protocol == IngressProtocol::Messages)
        .unwrap();
    grant.bearer_token_sha256 = token_sha256("rotated-messages-token");
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    for header in ["x-api-key", "X-HiRoute-Token"] {
        assert_eq!(send(&[(header, MESSAGES_TOKEN)], body).status, 401);
    }
    assert_eq!(send(&[("Authorization", &bearer)], body).status, 401);
    assert_eq!(
        send(&[("x-api-key", "rotated-messages-token")], body).status,
        200
    );
    assert_eq!(
        send(&[("Authorization", "Bearer rotated-messages-token")], body).status,
        200
    );
    publication
        .grants
        .retain(|grant| grant.protocol != IngressProtocol::Messages);
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    assert_eq!(
        send(&[("x-api-key", "rotated-messages-token")], body).status,
        401
    );
    assert_eq!(upstream.calls(), 5);
    for wire in upstream.requests() {
        assert_eq!(
            wire_header(&wire, "authorization").as_deref(),
            Some("Bearer provider-secret-1")
        );
        assert!(wire_header(&wire, "x-api-key").is_none());
        assert!(wire_header(&wire, "x-hiroute-token").is_none());
        let wire = String::from_utf8_lossy(&wire);
        assert!(
            !wire.contains(MESSAGES_TOKEN)
                && !wire.contains("rotated-messages-token")
                && !wire.contains("native-oauth")
        );
    }
}

#[test]
fn standard_model_bearer_preserves_current_grants_and_legacy_carrier_precedence() {
    let upstream = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        };
        3
    ]);
    let fixture = RuntimeFixture::launch_with_runtime_state_control(&[&upstream], 1);
    let body = br#"{"model":"runtime-model","input":"bearer admission"}"#;
    let bearer = format!("Bearer {TOKEN}");
    let send = |headers: &[(&str, &str)], body: &[u8]| {
        request(fixture.address, "POST", "/v1/responses", headers, body)
    };
    assert_eq!(send(&[("Authorization", &bearer)], body).status, 200);
    assert_eq!(
        request(
            fixture.address,
            "GET",
            "/v1/models",
            &[("Authorization", &bearer)],
            b""
        )
        .status,
        200
    );

    for secret in [
        "native-oauth",
        "provider-secret-1",
        "unissued-connection-token",
        "hr_run_control_fixture",
        "hr_run_model_unissued",
    ] {
        let invalid = format!("Bearer {secret}");
        let response = send(&[("Authorization", &invalid)], body);
        assert_eq!(response.status, 401, "{secret}");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["code"],
            "GATEWAY_GRANT_UNAUTHORIZED"
        );
    }
    for headers in [
        vec![
            ("Authorization", bearer.as_str()),
            ("Authorization", bearer.as_str()),
        ],
        vec![("Authorization", "Bearer first,second")],
        vec![("Authorization", "Basic runtime-token")],
        vec![("Authorization", bearer.as_str()), ("X-HiRoute-Token", "")],
        vec![
            ("Authorization", bearer.as_str()),
            ("X-HiRoute-Token", "unissued"),
        ],
    ] {
        assert_eq!(send(&headers, body).status, 401);
    }
    assert_eq!(
        request(
            fixture.address,
            "POST",
            "/v1/messages",
            &[("Authorization", &bearer)],
            br#"{"model":"runtime-model","messages":[]}"#,
        )
        .status,
        422,
        "a valid Responses grant cannot authorize Messages"
    );
    assert_eq!(
        send(
            &[("Authorization", &bearer)],
            br#"{"model":"not-in-grant","input":"denied"}"#
        )
        .status,
        404
    );
    assert_eq!(
        send(
            &[
                ("X-HiRoute-Token", TOKEN),
                ("Authorization", "Bearer native-oauth")
            ],
            body
        )
        .status,
        200,
        "the retained explicit carrier still overrides native SDK authorization"
    );
    assert_eq!(
        upstream.calls(),
        2,
        "denied requests never reach an upstream"
    );

    let mut publication = fixture.publication();
    publication
        .grants
        .iter_mut()
        .find(|grant| {
            grant.protocol == hiroute_gateway::server::request_plan::IngressProtocol::Responses
        })
        .unwrap()
        .bearer_token_sha256 = token_sha256("rotated-runtime-token");
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    assert_eq!(send(&[("Authorization", &bearer)], body).status, 401);
    assert_eq!(send(&[("X-HiRoute-Token", TOKEN)], body).status, 401);
    assert_eq!(
        send(&[("Authorization", "Bearer rotated-runtime-token")], body).status,
        200
    );
    assert_eq!(upstream.calls(), 3);
    for wire in upstream.requests() {
        assert_eq!(
            wire_header(&wire, "authorization").as_deref(),
            Some("Bearer provider-secret-1")
        );
        assert!(wire_header(&wire, "x-hiroute-token").is_none());
        let wire = String::from_utf8_lossy(&wire);
        assert!(!wire.contains("runtime-token") && !wire.contains("native-oauth"));
    }
}
