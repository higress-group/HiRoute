use super::*;

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
    for (ingress, path, capability_id, headers) in [
        (
            UpstreamProtocol::Responses,
            "/api/v1/responses",
            "cap.zhipu.glm-5.3.coding-plan.responses",
            Vec::new(),
        ),
        (
            UpstreamProtocol::Messages,
            "/api/anthropic/v1/messages",
            "cap.zhipu.glm-5.3.coding-plan.messages",
            vec![("anthropic-version".to_owned(), "2023-06-01".to_owned())],
        ),
    ] {
        let profile = candidate
            .protocol_profiles
            .iter()
            .find(|profile| profile.ingress_protocol == ingress)
            .unwrap();
        assert_eq!(profile.capability.upstream_protocol, ingress);
        assert_eq!(profile.capability.capability_id, capability_id);
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
