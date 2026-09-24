use super::*;

fn fact<T>(value: T) -> ComputeManagementFactValueV2<T> {
    ComputeManagementFactValueV2 {
        value: Some(value),
        basis: ComputeManagementFactBasisV2::UserDeclared,
    }
}

fn source(authentication: GatewayAuthenticationSemanticsV1) -> ComputeManagementSourceV2 {
    let target = ComputeManagementTargetV2 {
        scheme: "https".into(),
        authority: "api.example.test".into(),
        port: 443,
        request_path: "/v1/responses".into(),
        upstream_protocol: UpstreamProtocol::Responses,
        protocol_profile_id: "profile/responses".into(),
        protocol_profile_revision: 1,
    };
    let destination = target.credential_destination().unwrap();
    let credentials = if matches!(authentication, GatewayAuthenticationSemanticsV1::None) {
        Vec::new()
    } else {
        vec![ComputeManagedCredentialV2 {
            key_id: "credential/source-a/key-a".into(),
            credential: CredentialRefV1::new(
                "credential/source-a/key-a",
                "source/source-a",
                "hirouted",
                "provider-auth",
                [destination],
                1,
            )
            .unwrap(),
            fingerprint: CanonicalDigest::of_bytes(b"key-a"),
            ordinal: 0,
            enabled: true,
        }]
    };
    ComputeManagementSourceV2 {
        schema: COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2.into(),
        source_id: "source-a".into(),
        revision: 1,
        lineage_digest: CanonicalDigest::of_bytes(b"lineage-a"),
        display_name: "Example".into(),
        provenance: ComputeManagementProvenanceV2::UserConfigured {
            configuration_revision: 1,
            evidence_digest: CanonicalDigest::of_bytes(b"evidence"),
        },
        target,
        authentication,
        state: MaterializationState::Ready,
        models: vec![ComputeManagedModelV2 {
            model_ref: "model/a".into(),
            binding_id: "binding/a".into(),
            revision: 1,
            upstream_model_id: "model-a".into(),
            display_name: "Model A".into(),
            catalog_configuration_id: None,
            membership: ComputeManagementMembershipV2::UserDeclared,
            execution_eligible: true,
            capabilities: ComputeManagedCapabilitiesV2 {
                tool: fact(true),
                vision: fact(false),
                streaming: fact(true),
                context_tokens: fact(16_384),
                max_output_tokens: fact(4_096),
                native_reasoning: ComputeManagementFactValueV2 {
                    value: None,
                    basis: ComputeManagementFactBasisV2::Unknown,
                },
            },
            capability_evidence_digest: CanonicalDigest::of_bytes(b"capabilities"),
        }],
        native_recheck: None,
        additional_native_endpoints: Vec::new(),
        credentials,
        validation: None,
        last_candidate_ref: "candidate/a".into(),
        last_candidate_revision: 4,
    }
}

#[test]
fn ready_authentication_requires_an_enabled_key_but_none_does_not() {
    assert_eq!(
        source(GatewayAuthenticationSemanticsV1::Bearer).validate(),
        Ok(())
    );
    assert_eq!(
        source(GatewayAuthenticationSemanticsV1::None).validate(),
        Ok(())
    );
    let mut missing = source(GatewayAuthenticationSemanticsV1::Bearer);
    missing.credentials.clear();
    assert_eq!(
        missing.validate(),
        Err(ComputeManagementErrorV2::InvalidState)
    );
    missing.state = MaterializationState::NeedsCredential;
    assert_eq!(missing.validate(), Ok(()));
}

#[test]
fn one_native_source_shares_one_key_across_distinct_protocol_endpoints() {
    let mut value = source(GatewayAuthenticationSemanticsV1::Bearer);
    let messages = ComputeNativeEndpointV3 {
        target: ComputeManagementTargetV2 {
            scheme: "https".into(),
            authority: "messages.example.test".into(),
            port: 443,
            request_path: "/v1/messages".into(),
            upstream_protocol: UpstreamProtocol::Messages,
            protocol_profile_id: "profile/messages".into(),
            protocol_profile_revision: 1,
        },
        authentication: GatewayAuthenticationSemanticsV1::ApiKeyHeader {
            header: "x-api-key".into(),
        },
        recheck: None,
    };
    let messages_destination = messages.target.credential_destination().unwrap();
    value.additional_native_endpoints.push(messages.clone());
    assert_eq!(
        value.validate(),
        Err(ComputeManagementErrorV2::InvalidCredential)
    );
    let destinations = value.native_destinations().unwrap();
    value.credentials[0].credential = CredentialRefV1::new(
        "credential/source-a/key-a",
        "source/source-a",
        "hirouted",
        "provider-auth",
        destinations,
        1,
    )
    .unwrap();
    assert_eq!(value.validate(), Ok(()));
    assert!(
        value.credentials[0]
            .credential
            .allowed_destinations()
            .contains(&messages_destination)
    );
    value.additional_native_endpoints.push(messages);
    assert_eq!(
        value.validate(),
        Err(ComputeManagementErrorV2::DuplicateIdentity)
    );
    value.additional_native_endpoints.pop();
    value.additional_native_endpoints[0].authentication = GatewayAuthenticationSemanticsV1::None;
    assert_eq!(
        value.validate(),
        Err(ComputeManagementErrorV2::InvalidCredential)
    );
}

#[test]
fn disabled_source_may_retain_ordered_keys_without_becoming_ready() {
    let mut disabled = source(GatewayAuthenticationSemanticsV1::Bearer);
    disabled.state = MaterializationState::Disabled;
    assert_eq!(disabled.validate(), Ok(()));
    disabled.credentials[0].ordinal = 2;
    assert_eq!(
        disabled.validate(),
        Err(ComputeManagementErrorV2::DuplicateIdentity)
    );
}

#[test]
fn custom_header_rejects_reserved_or_injected_names() {
    for header in ["Authorization", "X-Key\r\nInjected: yes", "Host"] {
        let invalid = source(GatewayAuthenticationSemanticsV1::ApiKeyHeader {
            header: header.into(),
        });
        assert_eq!(
            invalid.validate(),
            Err(ComputeManagementErrorV2::InvalidAuthentication)
        );
    }
}

#[test]
fn only_https_or_loopback_http_targets_are_accepted() {
    let mut value = source(GatewayAuthenticationSemanticsV1::None);
    value.target.scheme = "http".into();
    assert_eq!(
        value.validate(),
        Err(ComputeManagementErrorV2::InvalidTarget)
    );
    value.target.authority = "127.0.0.1".into();
    assert_eq!(value.validate(), Ok(()));
}

#[test]
fn native_recheck_context_is_optional_safe_and_backward_compatible() {
    let legacy = source(GatewayAuthenticationSemanticsV1::None);
    let legacy_json = serde_json::to_value(&legacy).unwrap();
    assert!(legacy_json.get("native_recheck").is_none());
    let decoded: ComputeManagementSourceV2 = serde_json::from_value(legacy_json).unwrap();
    assert!(decoded.native_recheck.is_none());
    assert_eq!(decoded.validate(), Ok(()));

    let mut current = legacy;
    current.native_recheck = Some(ComputeNativeRecheckDescriptorV2 {
        display_template_id: None,
        inventory_path: Some("/v1/models".into()),
        protocol_header_semantics: GatewayHeaderSemanticsV1 {
            content_type: "application/json".into(),
            required_headers: vec![("anthropic-version".into(), "2023-06-01".into())],
            forbidden_forward_headers: vec!["authorization".into(), "x-api-key".into()],
        },
    });
    assert_eq!(current.validate(), Ok(()));
    assert!(
        serde_json::to_value(&current).unwrap()["native_recheck"]["inventory_path"]
            == serde_json::json!("/v1/models")
    );

    current.native_recheck.as_mut().unwrap().inventory_path =
        Some("https://forged.test/models".into());
    assert_eq!(
        current.validate(),
        Err(ComputeManagementErrorV2::InvalidTarget)
    );
    current.native_recheck.as_mut().unwrap().inventory_path = Some("/v1/models".into());
    current
        .native_recheck
        .as_mut()
        .unwrap()
        .protocol_header_semantics
        .required_headers[0]
        .1 = "value\r\ninjected: true".into();
    assert_eq!(
        current.validate(),
        Err(ComputeManagementErrorV2::InvalidTarget)
    );
}
