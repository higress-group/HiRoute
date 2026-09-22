use super::*;
use hiroute_domain::{
    GatewayNativeProviderStateEmissionV1, GatewayNativeReasoningRenderV1,
    GatewayNativeReasoningValueV1, GatewayReasoningControlKindV1, NativeReasoningCapabilityV1,
};
#[path = "candidate_execution_tests/native_protocol_profiles.rs"]
mod native_protocol_profiles;

fn user_fact<T>(value: T) -> hiroute_domain::ComputeManagementFactValueV2<T> {
    hiroute_domain::ComputeManagementFactValueV2 {
        value: Some(value),
        basis: hiroute_domain::ComputeManagementFactBasisV2::UserDeclared,
    }
}

fn runtime_fact<T>(value: T) -> hiroute_domain::ComputeManagementFactValueV2<T> {
    hiroute_domain::ComputeManagementFactValueV2 {
        value: Some(value),
        basis: hiroute_domain::ComputeManagementFactBasisV2::RuntimeFallback,
    }
}

fn registered_catalog_fact<T>(value: T) -> hiroute_integrations::NativeCandidateFactValueV1<T> {
    hiroute_integrations::NativeCandidateFactValueV1 {
        value: Some(value),
        basis: hiroute_integrations::NativeCandidateFactBasisV1::RegisteredCatalog,
    }
}

fn current_catalog() -> TrustedReleaseCatalog {
    const MANIFEST: &[u8] =
        include_bytes!("../../../../../assets/release-facts/current/bundle/manifest.json");
    TrustedReleaseCatalog::load_bundled_release_facts(
        MANIFEST,
        MANIFEST,
        include_bytes!(
            "../../../../../assets/release-facts/current/bundle/connector-registry.json"
        ),
        include_bytes!("../../../../../assets/release-facts/current/bundle/model-data.json"),
    )
    .unwrap()
}

fn source_local_fact(
    authentication: GatewayAuthenticationSemanticsV1,
) -> ComputeManagementCompilationFactV2 {
    let target = hiroute_domain::ComputeManagementTargetV2 {
        scheme: "http".into(),
        authority: "127.0.0.1".into(),
        port: 8_123,
        request_path: "/v1/responses".into(),
        upstream_protocol: UpstreamProtocol::Responses,
        protocol_profile_id: "profile/responses".into(),
        protocol_profile_revision: 3,
    };
    let ordered = match &authentication {
        GatewayAuthenticationSemanticsV1::None => {
            vec![hiroute_domain::ComputeCredentialSelectionV2::NoCredential]
        }
        _ => ["credential/manual-a", "credential/manual-b"]
            .into_iter()
            .enumerate()
            .map(|(index, credential_id)| {
                hiroute_domain::ComputeCredentialSelectionV2::Credential {
                    credential_ref: hiroute_domain::CredentialRefV1::new(
                        credential_id,
                        "source/source-manual",
                        "hirouted",
                        "provider-auth",
                        [target.credential_destination().unwrap()],
                        index as u64 + 1,
                    )
                    .unwrap(),
                }
            })
            .collect(),
    };
    ComputeManagementCompilationFactV2 {
        source_id: "source-manual".into(),
        source_revision: 2,
        source_lineage_digest: CanonicalDigest::of_bytes(b"source-local-lineage"),
        binding_id: "binding/manual".into(),
        binding_revision: 4,
        model_ref: "model/manual".into(),
        upstream_model_id: "manual-model".into(),
        display_name: "Manual model".into(),
        catalog_configuration_id: None,
        eligibility: ComputeManagementEligibilityV2::UserConfirmed,
        provenance: hiroute_domain::ComputeManagementProvenanceV2::UserConfigured {
            configuration_revision: 2,
            evidence_digest: CanonicalDigest::of_bytes(b"source-local-evidence"),
        },
        target,
        authentication,
        capabilities: hiroute_domain::ComputeManagedCapabilitiesV2 {
            tool: user_fact(true),
            vision: user_fact(false),
            streaming: user_fact(true),
            context_tokens: user_fact(32_768),
            max_output_tokens: user_fact(4_096),
            native_reasoning: user_fact(NativeReasoningCapabilityV1::Fixed {
                profile: "provider-default".into(),
            }),
        },
        capability_evidence_digest: CanonicalDigest::of_bytes(b"manual-capability"),
        native_reasoning: NativeReasoningCapabilityV1::Fixed {
            profile: "provider-default".into(),
        },
        credential: ComputeManagementCredentialCompilationV2::Native { ordered },
    }
}

#[test]
fn source_local_none_projection_keeps_port_and_has_no_credential_pool() {
    let candidate = materialize_management_candidate(
        &source_local_fact(GatewayAuthenticationSemanticsV1::None),
        None,
    )
    .unwrap();
    assert_eq!(
        candidate.authority,
        CandidateFactAuthorityV1::SourceLocalUser
    );
    assert!(!candidate.inventory_model_matched);
    assert!(candidate.is_routable());
    assert_eq!(
        candidate.protocol_endpoint.base_url,
        "http://127.0.0.1:8123"
    );
    assert_eq!(
        candidate.operational_target,
        GatewayOperationalTargetV1::UserConfiguredNative {
            uri: "http://127.0.0.1:8123/v1/responses".into(),
        }
    );
    assert!(candidate.binding.credential_pool_id.is_none());
    assert_eq!(candidate.credential_refs.len(), 1);
    assert!(candidate.credential_refs[0].starts_with("credential/none/"));
    assert!(candidate.rating.is_none());
    assert!(candidate.ordering_price.is_none());
    assert!(candidate.free_evidence.is_none());
    assert!(candidate.protocol_profiles.iter().all(|profile| {
        profile.connector.authentication.exact() == Some(&GatewayAuthenticationSemanticsV1::None)
    }));
}

#[test]
fn source_local_header_projection_preserves_order_and_exact_header() {
    let authentication = GatewayAuthenticationSemanticsV1::ApiKeyHeader {
        header: "x-provider-key".into(),
    };
    let candidate =
        materialize_management_candidate(&source_local_fact(authentication.clone()), None).unwrap();
    assert_eq!(
        candidate.credential_refs,
        ["credential/manual-a", "credential/manual-b"]
    );
    assert!(candidate.binding.credential_pool_id.is_some());
    assert!(
        candidate
            .protocol_profiles
            .iter()
            .all(|profile| profile.connector.authentication.exact() == Some(&authentication))
    );
}

#[test]
fn registered_runtime_fallback_preserves_exact_provider_endpoint_and_has_no_commercial_facts() {
    let catalog = current_catalog();
    let resolved = catalog
        .resolve_connection_option("deepseek.official.global.v1")
        .unwrap();
    let endpoint = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .find(|endpoint| endpoint.protocol == UpstreamProtocol::Responses)
        .unwrap();
    let target = hiroute_domain::ComputeManagementTargetV2 {
        scheme: "https".into(),
        authority: endpoint.base_url.strip_prefix("https://").unwrap().into(),
        port: 443,
        request_path: endpoint.request_path.clone(),
        upstream_protocol: endpoint.protocol,
        protocol_profile_id: endpoint.adapter_ref.clone(),
        protocol_profile_revision: endpoint.adapter_revision,
    };
    let credential = hiroute_domain::CredentialRefV1::new(
        "credential/deepseek-fallback",
        "source/source-deepseek-fallback",
        "hirouted",
        "provider-auth",
        [target.credential_destination().unwrap()],
        1,
    )
    .unwrap();
    let provenance = catalog.compute_catalog_provenance().unwrap();
    let reasoning = NativeReasoningCapabilityV1::Fixed {
        profile: "non-thinking".into(),
    };
    let fact = ComputeManagementCompilationFactV2 {
        source_id: "source-deepseek-fallback".into(),
        source_revision: 1,
        source_lineage_digest: CanonicalDigest::of_bytes(b"deepseek-fallback-lineage"),
        binding_id: "binding/deepseek-fallback".into(),
        binding_revision: 1,
        model_ref: "model/deepseek-future-text".into(),
        upstream_model_id: "deepseek-future-text".into(),
        display_name: "deepseek-future-text".into(),
        catalog_configuration_id: None,
        eligibility: ComputeManagementEligibilityV2::RuntimeQualified,
        provenance: hiroute_domain::ComputeManagementProvenanceV2::Registered {
            connection_option_id: resolved.option.connection_option_id.clone(),
            registry_version: catalog.registry().registry_version.clone(),
            catalog_digest: provenance.cross_reference_digest,
        },
        target,
        authentication: GatewayAuthenticationSemanticsV1::Bearer,
        capabilities: hiroute_domain::ComputeManagedCapabilitiesV2 {
            tool: runtime_fact(true),
            vision: runtime_fact(false),
            streaming: runtime_fact(true),
            context_tokens: runtime_fact(131_072),
            max_output_tokens: runtime_fact(8_192),
            native_reasoning: runtime_fact(reasoning.clone()),
        },
        capability_evidence_digest: CanonicalDigest::of_bytes(b"runtime-fallback-capability"),
        native_reasoning: reasoning,
        credential: ComputeManagementCredentialCompilationV2::Native {
            ordered: vec![hiroute_domain::ComputeCredentialSelectionV2::Credential {
                credential_ref: credential,
            }],
        },
    };

    let candidate = materialize_management_candidate(&fact, Some(&catalog)).unwrap();
    assert_eq!(
        candidate.authority,
        CandidateFactAuthorityV1::RuntimeFallback
    );
    assert_eq!(
        candidate.connection_option_id,
        resolved.option.connection_option_id
    );
    assert_eq!(
        candidate.capability.connector_id,
        resolved.connector.connector_id
    );
    assert_eq!(
        candidate.protocol_endpoint.protocol_endpoint_id,
        endpoint.protocol_endpoint_id
    );
    assert_eq!(
        candidate.operational_target,
        GatewayOperationalTargetV1::RegisteredHttps {
            uri: format!("{}{}", endpoint.base_url, endpoint.request_path),
        }
    );
    assert_eq!(
        candidate.capability.upstream_model_id,
        "deepseek-future-text"
    );
    assert!(candidate.rating.is_none());
    assert!(candidate.ordering_price.is_none());
    assert!(candidate.free_evidence.is_none());
    assert!(candidate.is_routable());
    assert!(!candidate.inventory_model_matched);

    let mut tampered = fact;
    tampered.target.authority = "attacker.invalid".into();
    assert!(materialize_management_candidate(&tampered, Some(&catalog)).is_none());
}

#[test]
fn declared_claude_messages_profiles_emit_both_exact_controls() {
    let bundle: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_str(include_str!(
        "../../../../../assets/model-data/current/model-data.json"
    ))
    .unwrap();
    for native in bundle
        .rating_snapshot
        .models
        .iter()
        .filter(|n| n.native_render_convention.is_some())
    {
        let profiles = reasoning_profiles(native, UpstreamProtocol::Messages).unwrap();
        assert!(!profiles.is_empty());
        for profile in profiles {
            assert_eq!(
                profile.render,
                GatewayNativeReasoningRenderV1::ExactFields {
                    protocol: UpstreamProtocol::Messages,
                    fields: vec![
                        field(
                            "output_config.effort",
                            GatewayNativeReasoningValueV1::String(profile.profile_id.clone()),
                            UpstreamProtocol::Messages
                        ),
                        field(
                            "thinking.type",
                            GatewayNativeReasoningValueV1::String("adaptive".into()),
                            UpstreamProtocol::Messages
                        )
                    ],
                }
            );
        }
        assert!(reasoning_profiles(native, UpstreamProtocol::Responses).is_none());
        assert!(reasoning_profiles(native, UpstreamProtocol::ChatCompletions).is_none());
    }
    assert_eq!(
        bundle
            .rating_snapshot
            .models
            .iter()
            .filter(|n| n.native_render_convention.is_some())
            .count(),
        2
    );
}

#[test]
fn cpa_codex_messages_face_maps_effort_to_adaptive_messages_controls() {
    let native = ModelNativeReasoningV1 {
        model_configuration_id: "model.openai.codex".into(),
        capability: NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".into(),
            profiles: vec!["low".into(), "high".into()],
        },
        native_render_convention: None,
    };
    let profiles = cpa_reasoning_profiles(
        &native,
        UpstreamProtocol::Responses,
        UpstreamProtocol::Messages,
    )
    .unwrap();
    assert_eq!(profiles.len(), 2);
    for profile in profiles {
        assert_eq!(
            profile.render,
            GatewayNativeReasoningRenderV1::ExactFields {
                protocol: UpstreamProtocol::Messages,
                fields: vec![
                    field(
                        "output_config.effort",
                        GatewayNativeReasoningValueV1::String(profile.profile_id.clone()),
                        UpstreamProtocol::Messages,
                    ),
                    field(
                        "thinking.type",
                        GatewayNativeReasoningValueV1::String("adaptive".into()),
                        UpstreamProtocol::Messages,
                    ),
                ],
            }
        );
    }
}

#[test]
fn cpa_profiles_prefer_same_protocol_and_messages_falls_back_to_responses() {
    let model = ModelDefinitionV1 {
        model_configuration_id: "model.openai.codex".into(),
        revision: 1,
        display_name: "Codex".into(),
        publisher_id: "publisher.openai".into(),
        capabilities: CapabilityFactsV1 {
            tool: true,
            vision: true,
            streaming: true,
            context_tokens: 128_000,
            max_output_tokens: 8_192,
        },
    };
    let capability = ModelEndpointCapabilityV1 {
        capability_id: "cap.codex.responses".into(),
        revision: 1,
        model_configuration_id: model.model_configuration_id.clone(),
        connector_id: "connector.cpa.codex".into(),
        connector_revision: 1,
        endpoint_profile_id: "endpoint.cpa.codex".into(),
        endpoint_profile_revision: 1,
        protocol_endpoint_id: "endpoint.cpa.codex.responses".into(),
        upstream_protocol: UpstreamProtocol::Responses,
        upstream_model_id: "codex-model".into(),
        required_adapter_ref: "adapter.openai-responses.v1".into(),
        required_adapter_revision: 1,
        evidence_digest: CanonicalDigest::of_bytes(b"cpa-profile-test"),
    };
    let reasoning = ModelNativeReasoningV1 {
        model_configuration_id: model.model_configuration_id.clone(),
        capability: NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".into(),
            profiles: vec!["low".into(), "high".into()],
        },
        native_render_convention: None,
    };
    let connector = ProtocolConnectorFacts {
        provider_id: "provider.openai".into(),
        endpoint_id: "endpoint.cpa.codex".into(),
        entitlement_id: "codex-subscription".into(),
        connector_id: "connector.cpa.codex".into(),
        connector_revision: "1".into(),
    };
    let faces = [
        ProtocolFace {
            protocol: UpstreamProtocol::Responses,
            request_path: "/v1/responses".into(),
            authentication: GatewayAuthenticationSemanticsV1::Bearer,
            required_headers: Vec::new(),
        },
        ProtocolFace {
            protocol: UpstreamProtocol::ChatCompletions,
            request_path: "/v1/chat/completions".into(),
            authentication: GatewayAuthenticationSemanticsV1::Bearer,
            required_headers: Vec::new(),
        },
        ProtocolFace {
            protocol: UpstreamProtocol::Messages,
            request_path: "/v1/messages".into(),
            authentication: GatewayAuthenticationSemanticsV1::Bearer,
            required_headers: vec![("anthropic-version".into(), "2023-06-01".into())],
        },
    ];
    let profiles = protocol_profiles(
        &connector,
        &model,
        &capability,
        &reasoning,
        "hiroute-account/codex-model",
        ConnectorRuntimeKind::CpaBridge,
        &faces,
    )
    .unwrap();
    assert!(profiles.iter().all(|profile| {
        let expected_path = match profile.ingress_protocol {
            UpstreamProtocol::Responses => "/v1/responses",
            UpstreamProtocol::ChatCompletions => "/v1/chat/completions",
            UpstreamProtocol::Messages => "/v1/messages",
        };
        profile.ingress_protocol == profile.capability.upstream_protocol
            && profile.connector.upstream_protocol == profile.capability.upstream_protocol
            && profile.connector.request_path == expected_path
    }));
    for protocol in [UpstreamProtocol::Responses, UpstreamProtocol::Messages] {
        let profile = profiles
            .iter()
            .find(|profile| profile.ingress_protocol == protocol)
            .unwrap();
        assert_eq!(
            profile.capability.native_provider_state,
            GatewayNativeProviderStateEmissionV1::ExactOwnerAffine
        );
    }

    let fallback = protocol_profiles(
        &connector,
        &model,
        &capability,
        &reasoning,
        "hiroute-account/codex-model",
        ConnectorRuntimeKind::CpaBridge,
        &faces[..2],
    )
    .unwrap();
    let messages = fallback
        .iter()
        .find(|profile| profile.ingress_protocol == UpstreamProtocol::Messages)
        .unwrap();
    assert_eq!(
        messages.capability.upstream_protocol,
        UpstreamProtocol::Responses
    );
    assert_eq!(messages.connector.request_path, "/v1/responses");
}

#[test]
fn cpa_messages_only_face_is_not_dropped_by_absent_responses() {
    let model = ModelDefinitionV1 {
        model_configuration_id: "model.messages-only".into(),
        revision: 1,
        display_name: "Messages only".into(),
        publisher_id: "publisher.fixture".into(),
        capabilities: CapabilityFactsV1 {
            tool: true,
            vision: false,
            streaming: true,
            context_tokens: 128_000,
            max_output_tokens: 8_192,
        },
    };
    let capability = ModelEndpointCapabilityV1 {
        capability_id: "cap.messages-only".into(),
        revision: 1,
        model_configuration_id: model.model_configuration_id.clone(),
        connector_id: "connector.cpa.messages".into(),
        connector_revision: 1,
        endpoint_profile_id: "endpoint.cpa.messages".into(),
        endpoint_profile_revision: 1,
        protocol_endpoint_id: "endpoint.cpa.messages.native".into(),
        upstream_protocol: UpstreamProtocol::Messages,
        upstream_model_id: "messages-model".into(),
        required_adapter_ref: "adapter.anthropic-messages.v1".into(),
        required_adapter_revision: 1,
        evidence_digest: CanonicalDigest::of_bytes(b"messages-only"),
    };
    let reasoning = ModelNativeReasoningV1 {
        model_configuration_id: model.model_configuration_id.clone(),
        capability: NativeReasoningCapabilityV1::Fixed {
            profile: "provider-default".into(),
        },
        native_render_convention: None,
    };
    let connector = ProtocolConnectorFacts {
        provider_id: "provider.fixture".into(),
        endpoint_id: "endpoint.cpa.messages".into(),
        entitlement_id: "fixture-subscription".into(),
        connector_id: "connector.cpa.messages".into(),
        connector_revision: "1".into(),
    };
    let profiles = protocol_profiles(
        &connector,
        &model,
        &capability,
        &reasoning,
        "messages-model",
        ConnectorRuntimeKind::CpaBridge,
        &[ProtocolFace {
            protocol: UpstreamProtocol::Messages,
            request_path: "/v1/messages".into(),
            authentication: GatewayAuthenticationSemanticsV1::Bearer,
            required_headers: vec![("anthropic-version".into(), "2023-06-01".into())],
        }],
    )
    .unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].ingress_protocol, UpstreamProtocol::Messages);
    assert_eq!(
        profiles[0].connector.authentication.exact(),
        Some(&GatewayAuthenticationSemanticsV1::Bearer)
    );
    assert_eq!(
        profiles[0]
            .connector
            .headers
            .exact()
            .unwrap()
            .required_headers,
        [("anthropic-version".into(), "2023-06-01".into())]
    );
}

#[test]
fn unmarked_native_controls_never_infer_adaptive_from_model_name() {
    for capability in [
        NativeReasoningCapabilityV1::Fixed {
            profile: "provider-default".into(),
        },
        NativeReasoningCapabilityV1::Toggle {
            parameter: "thinking.enabled".into(),
        },
        NativeReasoningCapabilityV1::Budget {
            parameter: "thinking.budget_tokens".into(),
            minimum_tokens: 1024,
            maximum_tokens: 2048,
            step_tokens: 1024,
        },
        NativeReasoningCapabilityV1::Discrete {
            parameter: "output_config.effort".into(),
            profiles: vec!["high".into()],
        },
    ] {
        let native = ModelNativeReasoningV1 {
            model_configuration_id: "model.anthropic.claude-opus-5".into(),
            capability,
            native_render_convention: None,
        };
        for protocol in [
            UpstreamProtocol::Messages,
            UpstreamProtocol::Responses,
            UpstreamProtocol::ChatCompletions,
        ] {
            let profiles = reasoning_profiles(&native, protocol).unwrap();
            assert!(
                !serde_json::to_string(&profiles)
                    .unwrap()
                    .contains("adaptive")
            );
            for profile in profiles {
                match profile.render {
                    GatewayNativeReasoningRenderV1::NoControlParameter => {
                        assert_eq!(profile.control_kind, GatewayReasoningControlKindV1::Fixed)
                    }
                    GatewayNativeReasoningRenderV1::ExactFields { fields, .. }
                    | GatewayNativeReasoningRenderV1::ExactBudget { fields, .. } => {
                        assert_eq!(fields.len(), 1)
                    }
                }
            }
        }
    }
}

fn registered_fact(
    catalog: &TrustedReleaseCatalog,
    option: &str,
    model_id: &str,
) -> ComputeManagementCompilationFactV2 {
    let mut fact = source_local_fact(GatewayAuthenticationSemanticsV1::Bearer);
    let resolved = catalog.resolve_connection_option(option).unwrap();
    let capability = catalog
        .model_data()
        .model_endpoint_capabilities
        .iter()
        .find(|capability| {
            capability.model_configuration_id == model_id
                && capability.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
        })
        .unwrap();
    let endpoint = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .find(|endpoint| endpoint.protocol_endpoint_id == capability.protocol_endpoint_id)
        .unwrap();
    let model = catalog.model_data().model(model_id).unwrap();
    let reasoning = catalog
        .native_reasoning()
        .iter()
        .find(|reasoning| reasoning.model_configuration_id == model_id)
        .unwrap();
    fact.eligibility = ComputeManagementEligibilityV2::CatalogMatched;
    fact.provenance = hiroute_domain::ComputeManagementProvenanceV2::Registered {
        connection_option_id: option.into(),
        registry_version: catalog.registry().registry_version.clone(),
        catalog_digest: catalog
            .compute_catalog_provenance()
            .unwrap()
            .cross_reference_digest,
    };
    fact.target = hiroute_domain::ComputeManagementTargetV2 {
        scheme: "https".into(),
        authority: endpoint.base_url.strip_prefix("https://").unwrap().into(),
        port: 443,
        request_path: endpoint.request_path.clone(),
        upstream_protocol: endpoint.protocol,
        protocol_profile_id: endpoint.adapter_ref.clone(),
        protocol_profile_revision: endpoint.adapter_revision,
    };
    fact.authentication = endpoint.authentication_semantics.clone().unwrap();
    fact.upstream_model_id = capability.upstream_model_id.clone();
    fact.catalog_configuration_id = Some(model_id.into());
    let registered = |value| hiroute_domain::ComputeManagementFactValueV2 {
        value: Some(value),
        basis: hiroute_domain::ComputeManagementFactBasisV2::RegisteredCatalog,
    };
    fact.capabilities = hiroute_domain::ComputeManagedCapabilitiesV2 {
        tool: registered(model.capabilities.tool),
        vision: registered(model.capabilities.vision),
        streaming: registered(model.capabilities.streaming),
        context_tokens: hiroute_domain::ComputeManagementFactValueV2 {
            value: Some(model.capabilities.context_tokens),
            basis: hiroute_domain::ComputeManagementFactBasisV2::RegisteredCatalog,
        },
        max_output_tokens: hiroute_domain::ComputeManagementFactValueV2 {
            value: Some(model.capabilities.max_output_tokens),
            basis: hiroute_domain::ComputeManagementFactBasisV2::RegisteredCatalog,
        },
        native_reasoning: hiroute_domain::ComputeManagementFactValueV2 {
            value: Some(reasoning.capability.clone()),
            basis: hiroute_domain::ComputeManagementFactBasisV2::RegisteredCatalog,
        },
    };
    fact.native_reasoning = reasoning.capability.clone();
    let declaration = hiroute_integrations::NativeModelCapabilityDeclarationV1 {
        tool: registered_catalog_fact(model.capabilities.tool),
        vision: registered_catalog_fact(model.capabilities.vision),
        streaming: registered_catalog_fact(model.capabilities.streaming),
        context_tokens: registered_catalog_fact(model.capabilities.context_tokens),
        max_output_tokens: registered_catalog_fact(model.capabilities.max_output_tokens),
        native_reasoning: registered_catalog_fact(reasoning.capability.clone()),
    };
    fact.capability_evidence_digest = CanonicalDigest::of(&(
        "declared-model-capabilities/v1",
        &fact.upstream_model_id,
        &declaration,
    ))
    .unwrap();
    fact.credential = ComputeManagementCredentialCompilationV2::Native {
        ordered: ["credential/registered-a", "credential/registered-b"]
            .into_iter()
            .map(
                |id| hiroute_domain::ComputeCredentialSelectionV2::Credential {
                    credential_ref: hiroute_domain::CredentialRefV1::new(
                        id,
                        format!("source/{}", fact.source_id),
                        "hirouted",
                        "provider-auth",
                        [fact.target.credential_destination().unwrap()],
                        1,
                    )
                    .unwrap(),
                },
            )
            .collect(),
    };
    fact
}

#[test]
fn registered_management_preserves_catalog_and_exact_credential_destination() {
    let catalog = crate::release_catalog::current_fixture_catalog();
    let (option, model, billing) = (
        "bailian.payg.cn.v1",
        "model.bailian.qwen3-max-2026-01-23",
        BillingClass::Paid,
    );
    let fact = registered_fact(&catalog, option, model);
    let candidate = materialize_registered_management_candidate(&fact, &catalog).unwrap();
    assert_eq!(
        candidate.authority,
        CandidateFactAuthorityV1::RegisteredCatalog
    );
    assert!(candidate.is_routable());
    assert_eq!(candidate.connection_option_id, option);
    assert_eq!(candidate.binding.model_configuration_id, model);
    assert_eq!(candidate.binding.billing_class, billing);
    assert_eq!(candidate.binding.binding_id, fact.binding_id);
    assert_eq!(candidate.binding.revision, fact.binding_revision);
    assert_eq!(candidate.binding.source_id, fact.source_id);
    assert_eq!(candidate.binding.source_revision, fact.source_revision);
    assert_eq!(
        candidate.binding.source_identity_digest,
        fact.source_lineage_digest
    );
    assert_eq!(
        candidate.capability.evidence_digest,
        catalog
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .find(|capability| { capability.capability_id == candidate.binding.capability_id })
            .unwrap()
            .evidence_digest
    );
    let offer = catalog
        .model_data()
        .offer(&candidate.binding.offer_ref)
        .unwrap();
    assert_eq!(
        candidate.binding.offer_evidence_digest,
        offer.evidence_digest
    );
    assert_eq!(candidate.offer_revision, offer.revision);
    assert_eq!(candidate.reasoning, fact.native_reasoning);
    assert_eq!(
        candidate.credential_refs,
        ["credential/registered-a", "credential/registered-b"]
    );
    assert_eq!(
        candidate.credential_destination_ref,
        Some(fact.target.credential_destination().unwrap())
    );
    assert!(
        candidate.protocol_profiles.iter().all(|profile| {
            profile.connector.authentication.exact() == Some(&fact.authentication)
        })
    );
    let mut wrong = candidate.clone();
    wrong.credential_destination_ref = Some("compute-target/substituted".into());
    assert!(wrong.validate().is_err());
    let mut legacy = candidate;
    legacy.credential_destination_ref = None;
    assert!(legacy.validate().is_ok());
    assert!(materialize_management_candidate(&fact, None).is_none());
}

#[test]
fn registered_management_rejects_unproved_catalog_target_capability_and_credentials() {
    let catalog = crate::release_catalog::current_fixture_catalog();
    let fact = registered_fact(
        &catalog,
        "bailian.payg.cn.v1",
        "model.bailian.qwen3-max-2026-01-23",
    );
    let mutations: &[fn(&mut ComputeManagementCompilationFactV2)] = &[
        |fact| fact.eligibility = ComputeManagementEligibilityV2::UserConfirmed,
        |fact| {
            if let hiroute_domain::ComputeManagementProvenanceV2::Registered {
                connection_option_id,
                ..
            } = &mut fact.provenance
            {
                *connection_option_id = "option/unregistered".into();
            }
        },
        |fact| fact.catalog_configuration_id = Some("model/unregistered".into()),
        |fact| fact.upstream_model_id = "unmatched".into(),
        |fact| fact.target.authority = "substituted.invalid".into(),
        |fact| fact.target.port = 8443,
        |fact| fact.target.scheme = "http".into(),
        |fact| fact.target.request_path = "/other/messages".into(),
        |fact| fact.target.upstream_protocol = UpstreamProtocol::Responses,
        |fact| fact.target.protocol_profile_id = "adapter/substituted".into(),
        |fact| fact.target.protocol_profile_revision += 1,
        |fact| fact.authentication = GatewayAuthenticationSemanticsV1::None,
        |fact| {
            fact.authentication = GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                header: "x-api-key".into(),
            }
        },
        |fact| fact.capabilities.tool.value = Some(false),
        |fact| fact.capabilities.vision.value = None,
        |fact| {
            fact.capabilities.streaming.basis =
                hiroute_domain::ComputeManagementFactBasisV2::UserDeclared
        },
        |fact| fact.capabilities.context_tokens.value = Some(1),
        |fact| fact.capabilities.max_output_tokens.value = Some(1),
        |fact| fact.capability_evidence_digest = CanonicalDigest::of_bytes(b"substituted-evidence"),
        |fact| {
            fact.native_reasoning = NativeReasoningCapabilityV1::Fixed {
                profile: "unproved".into(),
            }
        },
        |fact| {
            fact.credential = ComputeManagementCredentialCompilationV2::Native {
                ordered: Vec::new(),
            }
        },
        |fact| {
            fact.credential = ComputeManagementCredentialCompilationV2::Native {
                ordered: vec![hiroute_domain::ComputeCredentialSelectionV2::NoCredential],
            }
        },
        |fact| {
            fact.credential = source_local_fact(GatewayAuthenticationSemanticsV1::Bearer).credential
        },
        |fact| {
            fact.credential = ComputeManagementCredentialCompilationV2::Native {
                ordered: vec![hiroute_domain::ComputeCredentialSelectionV2::Credential {
                    credential_ref: hiroute_domain::CredentialRefV1::new(
                        "credential/other-source",
                        "source/other",
                        "hirouted",
                        "provider-auth",
                        [fact.target.credential_destination().unwrap()],
                        1,
                    )
                    .unwrap(),
                }],
            }
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut invalid = fact.clone();
        mutate(&mut invalid);
        assert!(
            materialize_registered_management_candidate(&invalid, &catalog).is_none(),
            "mutation {index}"
        );
    }
}

#[test]
fn registered_management_re_resolves_stable_option_across_global_catalog_identity_change() {
    let catalog = crate::release_catalog::current_fixture_catalog();
    let mut fact = registered_fact(
        &catalog,
        "bailian.payg.cn.v1",
        "model.bailian.qwen3-max-2026-01-23",
    );
    let hiroute_domain::ComputeManagementProvenanceV2::Registered {
        registry_version,
        catalog_digest,
        ..
    } = &mut fact.provenance
    else {
        unreachable!();
    };
    registry_version.push_str("-catalog-a");
    *catalog_digest = CanonicalDigest::of_bytes(b"catalog-a");

    let candidate = materialize_registered_management_candidate(&fact, &catalog).unwrap();
    assert_eq!(candidate.connection_option_id, "bailian.payg.cn.v1");
    assert_eq!(
        candidate.operational_target.request_path(),
        Some(fact.target.request_path.as_str())
    );
}

#[test]
fn registered_management_compilation_excludes_disabled_missing_keys_and_unknown_members() {
    use hiroute_application::compute_management::compile_compute_management_source;
    use hiroute_domain::*;
    let catalog = crate::release_catalog::current_fixture_catalog();
    let fact = registered_fact(
        &catalog,
        "bailian.payg.cn.v1",
        "model.bailian.qwen3-max-2026-01-23",
    );
    let ComputeManagementCredentialCompilationV2::Native { ordered } = &fact.credential else {
        unreachable!();
    };
    let source = ComputeManagementSourceV2 {
        schema: COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2.into(),
        source_id: fact.source_id.clone(),
        revision: fact.source_revision,
        lineage_digest: fact.source_lineage_digest.clone(),
        display_name: fact.display_name.clone(),
        provenance: fact.provenance.clone(),
        target: fact.target.clone(),
        authentication: fact.authentication.clone(),
        state: MaterializationState::Ready,
        models: vec![ComputeManagedModelV2 {
            model_ref: fact.model_ref.clone(),
            binding_id: fact.binding_id.clone(),
            revision: fact.binding_revision,
            upstream_model_id: fact.upstream_model_id.clone(),
            display_name: fact.display_name.clone(),
            catalog_configuration_id: fact.catalog_configuration_id.clone(),
            membership: ComputeManagementMembershipV2::Catalog,
            execution_eligible: true,
            capabilities: fact.capabilities.clone(),
            capability_evidence_digest: fact.capability_evidence_digest.clone(),
        }],
        native_recheck: None,
        credentials: ordered
            .iter()
            .enumerate()
            .map(|(index, selection)| {
                let ComputeCredentialSelectionV2::Credential { credential_ref } = selection else {
                    unreachable!()
                };
                ComputeManagedCredentialV2 {
                    key_id: credential_ref.credential_id().into(),
                    credential: credential_ref.clone(),
                    fingerprint: CanonicalDigest::of_bytes(&[index as u8]),
                    ordinal: index as u32,
                    enabled: true,
                }
            })
            .collect(),
        validation: None,
        last_candidate_ref: "candidate/native/registered".into(),
        last_candidate_revision: 1,
    };
    source.validate().unwrap();
    let compiled = compile_compute_management_source(&source).unwrap();
    assert_eq!(compiled.len(), 1);
    assert!(materialize_registered_management_candidate(&compiled[0], &catalog).is_some());
    let mut partially_disabled = source.clone();
    partially_disabled.credentials[0].enabled = false;
    let compiled = compile_compute_management_source(&partially_disabled).unwrap();
    let candidate = materialize_registered_management_candidate(&compiled[0], &catalog).unwrap();
    assert_eq!(candidate.credential_refs, ["credential/registered-b"]);
    let mut disabled = source.clone();
    disabled.state = MaterializationState::Disabled;
    assert!(compile_compute_management_source(&disabled).is_err());
    let mut missing = source.clone();
    missing.credentials.clear();
    missing.state = MaterializationState::NeedsCredential;
    assert!(compile_compute_management_source(&missing).is_err());
    let mut all_disabled = source.clone();
    all_disabled
        .credentials
        .iter_mut()
        .for_each(|key| key.enabled = false);
    assert!(compile_compute_management_source(&all_disabled).is_err());
    let mut unknown = source.clone();
    unknown.models[0].capabilities.tool = ComputeManagementFactValueV2 {
        value: None,
        basis: ComputeManagementFactBasisV2::Unknown,
    };
    assert!(compile_compute_management_source(&unknown).is_err());
    let mut observed = source;
    observed.models[0].membership = ComputeManagementMembershipV2::Observed;
    assert!(compile_compute_management_source(&observed).is_err());
}

#[test]
fn responses_effort_uses_the_exact_nested_native_field() {
    assert_eq!(
        parameter_path("reasoning_effort", UpstreamProtocol::Responses),
        ["reasoning", "effort"]
    );
    assert_eq!(
        parameter_path("reasoning_effort", UpstreamProtocol::ChatCompletions),
        ["reasoning_effort"]
    );
}

#[test]
fn unknown_native_text_keeps_facts_unknown_and_execution_conservative() {
    let mut fact = source_local_fact(GatewayAuthenticationSemanticsV1::None);
    fn unknown<T>() -> hiroute_domain::ComputeManagementFactValueV2<T> {
        hiroute_domain::ComputeManagementFactValueV2 {
            value: None,
            basis: hiroute_domain::ComputeManagementFactBasisV2::Unknown,
        }
    }
    fact.capabilities = hiroute_domain::ComputeManagedCapabilitiesV2 {
        tool: unknown(),
        vision: unknown(),
        streaming: unknown(),
        context_tokens: unknown(),
        max_output_tokens: unknown(),
        native_reasoning: unknown(),
    };
    fact.native_reasoning = NativeReasoningCapabilityV1::Fixed {
        profile: "non-thinking".into(),
    };
    let candidate = materialize_management_candidate(&fact, None).unwrap();
    assert!(candidate.is_routable());
    assert!(!candidate.model.capabilities.tool);
    assert!(!candidate.model.capabilities.vision);
    assert!(!candidate.model.capabilities.streaming);
    assert_eq!(candidate.model.capabilities.context_tokens, 4096);
    assert_eq!(candidate.model.capabilities.max_output_tokens, 1024);
    assert!(fact.capabilities.context_tokens.value.is_none());
    assert!(fact.capabilities.tool.value.is_none());
    assert!(candidate.free_evidence.is_none());
    assert!(candidate.ordering_price.is_none());
    let mut second_source = fact.clone();
    second_source.source_id = "second-source-same-model".into();
    second_source.binding_id = "binding/second-source".into();
    let second = materialize_management_candidate(&second_source, None).unwrap();
    assert_ne!(
        candidate.model.model_configuration_id,
        second.model.model_configuration_id
    );
}
