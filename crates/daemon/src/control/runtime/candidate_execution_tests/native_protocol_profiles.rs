use super::*;

#[test]
fn source_local_profiles_keep_independent_authorities_and_reject_mismatched_destinations() {
    let mut fact = source_local_fact(GatewayAuthenticationSemanticsV1::None);
    let mut messages_target = fact.target.clone();
    messages_target.authority = "127.0.0.2".into();
    messages_target.port = 8_124;
    messages_target.request_path = "/apps/anthropic/v1/messages".into();
    messages_target.upstream_protocol = UpstreamProtocol::Messages;
    messages_target.protocol_profile_id = "profile/messages".into();
    fact.additional_native_endpoints
        .push(hiroute_domain::ComputeNativeEndpointV3 {
            target: messages_target.clone(),
            authentication: GatewayAuthenticationSemanticsV1::None,
            recheck: None,
        });

    let candidate = materialize_management_candidate(&fact, None).unwrap();
    for (protocol, authority, destination) in [
        (
            UpstreamProtocol::Responses,
            "127.0.0.1:8123",
            fact.target.credential_destination().unwrap(),
        ),
        (
            UpstreamProtocol::Messages,
            "127.0.0.2:8124",
            messages_target.credential_destination().unwrap(),
        ),
    ] {
        let profile = candidate
            .protocol_profiles
            .iter()
            .find(|profile| profile.ingress_protocol == protocol)
            .unwrap();
        let target = profile.native_target.as_ref().unwrap();
        assert!(target.operational_target.uri().contains(authority));
        assert_eq!(target.credential_destination_ref, destination);
        assert!(target.validate_for(profile));
        let mut mismatched = target.clone();
        mismatched.credential_destination_ref = "source/other-destination".into();
        assert!(!mismatched.validate_for(profile));
    }
}

#[test]
fn source_local_ipv6_profiles_keep_exact_destinations() {
    let mut fact = source_local_fact(GatewayAuthenticationSemanticsV1::None);
    fact.target.authority = "::1".into();
    let mut messages_target = fact.target.clone();
    messages_target.port = 8_124;
    messages_target.request_path = "/v1/messages".into();
    messages_target.upstream_protocol = UpstreamProtocol::Messages;
    messages_target.protocol_profile_id = "profile/messages".into();
    fact.additional_native_endpoints
        .push(hiroute_domain::ComputeNativeEndpointV3 {
            target: messages_target.clone(),
            authentication: GatewayAuthenticationSemanticsV1::None,
            recheck: None,
        });

    let candidate = materialize_management_candidate(&fact, None).unwrap();
    for (protocol, authority, destination) in [
        (
            UpstreamProtocol::Responses,
            "[::1]:8123",
            fact.target.credential_destination().unwrap(),
        ),
        (
            UpstreamProtocol::Messages,
            "[::1]:8124",
            messages_target.credential_destination().unwrap(),
        ),
    ] {
        let profile = candidate
            .protocol_profiles
            .iter()
            .find(|profile| profile.ingress_protocol == protocol)
            .unwrap();
        let native_target = profile.native_target.as_ref().unwrap();
        assert!(native_target.operational_target.uri().contains(authority));
        assert_eq!(native_target.credential_destination_ref, destination);
        assert!(native_target.validate_for(profile));
    }
}

#[test]
fn source_local_messages_target_keeps_responses_conversion_on_the_same_path() {
    let mut fact = source_local_fact(GatewayAuthenticationSemanticsV1::None);
    fact.target.upstream_protocol = UpstreamProtocol::Messages;
    fact.target.request_path = "/v1/messages".into();
    fact.target.protocol_profile_id = "profile/messages".into();
    let candidate = materialize_management_candidate(&fact, None).unwrap();
    let responses = candidate
        .protocol_profiles
        .iter()
        .find(|profile| profile.ingress_protocol == UpstreamProtocol::Responses)
        .expect("the supported Responses-to-Messages conversion remains available");
    assert_eq!(
        candidate.operational_target.request_path(),
        Some("/v1/messages")
    );
    assert_eq!(
        responses.capability.upstream_protocol,
        UpstreamProtocol::Messages
    );
    assert_eq!(responses.connector.request_path, "/v1/messages");
    assert!(responses.path_id.starts_with("responses-to-messages-"));
}

#[test]
fn registered_zhipu_model_uses_one_source_credential_for_its_exact_protocol_faces() {
    let catalog = crate::release_catalog::current_fixture_catalog();
    let fact = registered_fact(&catalog, "zhipu.coding-plan.cn.v1", "model.zhipu.glm-5.3");
    let candidate = materialize_registered_management_candidate(&fact, &catalog).unwrap();
    assert_eq!(candidate.binding.source_id, fact.source_id);
    assert_eq!(
        candidate.credential_refs,
        ["credential/registered-a", "credential/registered-b"]
    );
    assert_eq!(
        candidate.credential_destination_ref,
        Some(fact.target.credential_destination().unwrap())
    );
    for (ingress, path, headers) in [
        (UpstreamProtocol::Responses, "/api/v1/responses", Vec::new()),
        (
            UpstreamProtocol::Messages,
            "/api/anthropic/v1/messages",
            vec![("anthropic-version".to_owned(), "2023-06-01".to_owned())],
        ),
    ] {
        let profile = candidate
            .protocol_profiles
            .iter()
            .find(|profile| profile.ingress_protocol == ingress)
            .unwrap();
        assert_eq!(profile.capability.upstream_protocol, ingress);
        assert_eq!(
            profile.capability.capability_id,
            candidate.capability.capability_id
        );
        assert_eq!(profile.capability.native_model, "glm-5.3");
        assert_eq!(profile.connector.request_path, path);
        assert_eq!(
            profile.capability.request.image_base64,
            hiroute_domain::GatewayFidelityV1::Unsupported
        );
        if ingress == UpstreamProtocol::Messages {
            assert_eq!(
                profile.capability.request.tool_choice_none,
                hiroute_domain::GatewayFidelityV1::Unsupported
            );
        }
        assert_eq!(
            profile.connector.authentication.exact(),
            Some(&GatewayAuthenticationSemanticsV1::Bearer)
        );
        assert_eq!(
            profile.connector.headers.exact().unwrap().required_headers,
            headers
        );
    }
}

#[test]
fn native_messages_only_preserves_explicit_responses_conversion() {
    let catalog = crate::release_catalog::current_fixture_catalog();
    let resolved = catalog
        .resolve_connection_option("zhipu.coding-plan.cn.v1")
        .unwrap();
    let model = catalog.model_data().model("model.zhipu.glm-5.3").unwrap();
    let capability = catalog
        .model_data()
        .model_endpoint_capabilities
        .iter()
        .find(|value| value.capability_id == "cap.zhipu.glm-5.3.coding-plan.messages")
        .unwrap();
    let endpoint = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .find(|value| value.protocol_endpoint_id == capability.protocol_endpoint_id)
        .unwrap();
    let reasoning = catalog
        .native_reasoning()
        .iter()
        .find(|value| value.model_configuration_id == model.model_configuration_id)
        .unwrap();
    let connector = ProtocolConnectorFacts {
        provider_id: resolved.endpoint_profile.provider_platform_id.clone(),
        endpoint_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
        entitlement_id: resolved.endpoint_profile.entitlement_id.clone(),
        connector_id: resolved.connector.connector_id.clone(),
        connector_revision: resolved.connector.revision.to_string(),
    };
    let faces = [ProtocolFace {
        protocol: UpstreamProtocol::Messages,
        request_path: endpoint.request_path.clone(),
        authentication: GatewayAuthenticationSemanticsV1::Bearer,
        required_headers: endpoint.required_headers.clone(),
        native_target: None,
        adapter_ref: None,
        adapter_revision: None,
    }];
    let profiles = protocol_profiles(
        &connector,
        model,
        capability,
        reasoning,
        "glm-5.3",
        ConnectorRuntimeKind::BuiltinNative,
        &faces,
    )
    .unwrap();
    let responses = profiles
        .iter()
        .find(|profile| profile.ingress_protocol == UpstreamProtocol::Responses)
        .expect("Responses ingress may convert to the Messages upstream");
    assert_eq!(
        responses.capability.upstream_protocol,
        UpstreamProtocol::Messages
    );
    assert_eq!(responses.connector.request_path, endpoint.request_path);
    assert!(responses.path_id.starts_with("responses-to-messages-"));
}
