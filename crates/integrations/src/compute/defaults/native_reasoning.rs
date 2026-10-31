//! Optional reasoning defaults for an already admitted official API target.
//! This projection is not membership, credential, price or protocol qualification.
use super::*;
mod provider_records;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiReasoningDefault {
    pub capability: NativeReasoningCapabilityV1,
    pub render_convention: Option<NativeReasoningRenderConventionV1>,
}

pub fn missing_api_reasoning_default(
    catalog: &TrustedReleaseCatalog,
    provenance: &ComputeManagementProvenanceV2,
    target: &ComputeManagementTargetV2,
    authentication: &GatewayAuthenticationSemanticsV1,
    upstream_model_id: &str,
    saved: &ComputeManagementFactValueV2<NativeReasoningCapabilityV1>,
    preference: Option<&ModelDefaultPreferenceV1>,
) -> Option<ApiReasoningDefault> {
    // PreserveSaved is also established automatically on imports with no fact.
    // Preserve meaningful source values, not the absence of reasoning metadata;
    // an explicit official locator still keeps its own exact-scoped semantics.
    if matches!(
        preference,
        Some(ModelDefaultPreferenceV1::FollowOfficial { .. })
    ) || !matches!(
        (&saved.value, saved.basis),
        (None, ComputeManagementFactBasisV2::Unknown)
            | (Some(_), ComputeManagementFactBasisV2::RuntimeFallback)
    ) || target.scheme != "https"
        || target.port != 443
    {
        return None;
    }
    let ComputeManagementProvenanceV2::Registered {
        connection_option_id,
        ..
    } = provenance
    else {
        return None;
    };
    let resolved = catalog
        .resolve_connection_option(connection_option_id)
        .ok()?;
    if resolved.connector.runtime_kind != ConnectorRuntimeKind::BuiltinNative
        || resolved.connector.authentication != AuthenticationKind::ProviderApiKey
        || resolved.option.origin == ConnectionOrigin::AgentSubscription
    {
        return None;
    }
    let mut endpoints = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .filter(|endpoint| {
            endpoint.protocol == target.upstream_protocol
                && endpoint.adapter_ref == target.protocol_profile_id
                && endpoint.adapter_revision == target.protocol_profile_revision
                && endpoint.base_url.strip_prefix("https://") == Some(target.authority.as_str())
                && endpoint.request_path == target.request_path
                && endpoint.authentication_semantics.as_ref() == Some(authentication)
        });
    let endpoint = endpoints.next()?;
    if endpoints.next().is_some() {
        return None;
    }
    let protocol = match endpoint.protocol {
        UpstreamProtocol::Responses => "openai-responses",
        UpstreamProtocol::ChatCompletions => "openai-chat",
        UpstreamProtocol::Messages => "anthropic-messages",
    };
    let wire_target = format!("{}{}", endpoint.base_url, endpoint.request_path);
    let metadata = catalog.model_metadata();
    let mut result = None;
    let mut openai_interface = false;
    for product in &metadata.access_products {
        // These are the two existing native wire conventions, not a rule for
        // every provider that happens to expose an OpenAI-compatible protocol.
        let publisher = match product.product_key.as_str() {
            "openai-platform-global" => "OpenAI",
            "anthropic-platform" => "Anthropic",
            _ => continue,
        };
        for interface in &product.interfaces {
            if interface.protocol != protocol
                || interface
                    .base_url
                    .as_ref()
                    .zip(interface.request_path.as_ref())
                    .map(|(base, path)| format!("{}{}", base.trim_end_matches('/'), path))
                    .as_deref()
                    != Some(wire_target.as_str())
            {
                continue;
            }
            openai_interface |= product.product_key == "openai-platform-global";
            for binding in metadata.endpoint_bindings.iter().filter(|binding| {
                binding.product_key == product.product_key
                    && binding.upstream_model_id == upstream_model_id
                    && binding
                        .interface_candidates
                        .contains(&interface.interface_key)
                    && !matches!(binding.lifecycle.as_deref(), Some("retired" | "deprecated"))
            }) {
                let model = metadata.canonical_models.iter().find(|model| {
                    model.model_key == binding.model_key
                        && model.publisher == publisher
                        && !matches!(model.lifecycle.as_str(), "retired" | "deprecated")
                })?;
                let profiles = &model.reasoning.profiles;
                if profiles.is_empty() {
                    return None;
                }
                let (parameter, render_convention) =
                    match (publisher, model.reasoning.kind.as_str(), endpoint.protocol) {
                        (
                            "OpenAI",
                            "discrete",
                            UpstreamProtocol::Responses | UpstreamProtocol::ChatCompletions,
                        ) => ("reasoning_effort", None),
                        (
                            "Anthropic",
                            "adaptive" | "adaptive-discrete" | "adaptive-fixed-on",
                            UpstreamProtocol::Messages,
                        ) => (
                            "output_config.effort",
                            Some(NativeReasoningRenderConventionV1::ClaudeAdaptiveEffortMessages),
                        ),
                        _ => return None,
                    };
                let value = ApiReasoningDefault {
                    capability: NativeReasoningCapabilityV1::Discrete {
                        parameter: parameter.into(),
                        profiles: profiles.clone(),
                        default_profile: model.reasoning.default.clone(),
                    },
                    render_convention,
                };
                ModelNativeReasoningV1 {
                    model_configuration_id: "model/scoped-api-default".into(),
                    capability: value.capability.clone(),
                    native_render_convention: value.render_convention,
                }
                .validate_for_protocol(endpoint.protocol)
                .ok()?;
                if result.as_ref().is_some_and(|existing| existing != &value) {
                    return None;
                }
                result = Some(value);
            }
        }
    }
    result.or_else(|| {
        openai_interface
            .then(|| {
                provider_records::openai_provider_default(
                    metadata,
                    endpoint.protocol,
                    &wire_target,
                    upstream_model_id,
                )
            })
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> TrustedReleaseCatalog {
        TrustedReleaseCatalog::load_release_facts(
            include_bytes!("../../../../../assets/release-facts/current/bundle/manifest.json"),
            include_bytes!(
                "../../../../../assets/release-facts/current/bundle/connector-registry.json"
            ),
            include_bytes!("../../../../../assets/release-facts/current/bundle/model-data.json"),
        )
        .unwrap()
    }

    fn scope(
        catalog: &TrustedReleaseCatalog,
        option: &str,
        protocol: UpstreamProtocol,
    ) -> (
        ComputeManagementProvenanceV2,
        ComputeManagementTargetV2,
        GatewayAuthenticationSemanticsV1,
    ) {
        let resolved = catalog.resolve_connection_option(option).unwrap();
        let endpoint = resolved
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .find(|endpoint| endpoint.protocol == protocol)
            .unwrap();
        let provenance = ComputeManagementProvenanceV2::Registered {
            connection_option_id: option.into(),
            registry_version: "fixture/registry".into(),
            catalog_digest: CanonicalDigest::of_bytes(b"fixture/registry"),
        };
        let target = ComputeManagementTargetV2 {
            scheme: "https".into(),
            authority: endpoint.base_url.strip_prefix("https://").unwrap().into(),
            port: 443,
            request_path: endpoint.request_path.clone(),
            upstream_protocol: protocol,
            protocol_profile_id: endpoint.adapter_ref.clone(),
            protocol_profile_revision: endpoint.adapter_revision,
        };
        (
            provenance,
            target,
            endpoint.authentication_semantics.clone().unwrap(),
        )
    }

    fn unknown() -> ComputeManagementFactValueV2<NativeReasoningCapabilityV1> {
        ComputeManagementFactValueV2 {
            value: None,
            basis: ComputeManagementFactBasisV2::Unknown,
        }
    }

    #[test]
    fn existing_openai_observed_models_get_current_scoped_effort_profiles() {
        let catalog = catalog();
        for protocol in [
            UpstreamProtocol::Responses,
            UpstreamProtocol::ChatCompletions,
        ] {
            let (provenance, target, authentication) =
                scope(&catalog, "openai.platform.global.v1", protocol);
            let saved = unknown();
            let result = missing_api_reasoning_default(
                &catalog,
                &provenance,
                &target,
                &authentication,
                "gpt-6.1-sol",
                &saved,
                None,
            )
            .unwrap();
            assert!(
                matches!(result.capability, NativeReasoningCapabilityV1::Discrete { parameter, profiles, default_profile }
                if parameter == "reasoning_effort" && profiles == ["low", "medium", "high", "xhigh", "max"] && default_profile.as_deref() == Some("medium"))
            );
            assert_eq!(result.render_convention, None);
            assert_eq!(saved, unknown());
        }
    }

    #[test]
    fn anthropic_api_uses_the_messages_adaptive_render_convention() {
        let catalog = catalog();
        let (provenance, target, authentication) = scope(
            &catalog,
            "anthropic.platform.global.v1",
            UpstreamProtocol::Messages,
        );
        let result = missing_api_reasoning_default(
            &catalog,
            &provenance,
            &target,
            &authentication,
            "claude-opus-5-5",
            &unknown(),
            None,
        )
        .unwrap();
        assert!(
            matches!(result.capability, NativeReasoningCapabilityV1::Discrete { parameter, .. } if parameter == "output_config.effort")
        );
        assert_eq!(
            result.render_convention,
            Some(NativeReasoningRenderConventionV1::ClaudeAdaptiveEffortMessages)
        );
        assert!(
            missing_api_reasoning_default(
                &catalog,
                &provenance,
                &target,
                &authentication,
                "claude-opus-5",
                &unknown(),
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn manual_configuration_is_never_replaced_but_empty_saved_defaults_can_be_completed() {
        let catalog = catalog();
        let (provenance, target, authentication) = scope(
            &catalog,
            "openai.platform.global.v1",
            UpstreamProtocol::Responses,
        );
        for capability in [
            NativeReasoningCapabilityV1::Fixed {
                profile: "non-thinking".into(),
            },
            NativeReasoningCapabilityV1::Discrete {
                parameter: "reasoning_effort".into(),
                profiles: vec!["low".into(), "high".into()],
                default_profile: Some("low".into()),
            },
        ] {
            let saved = ComputeManagementFactValueV2 {
                value: Some(capability),
                basis: ComputeManagementFactBasisV2::UserDeclared,
            };
            assert!(
                missing_api_reasoning_default(
                    &catalog,
                    &provenance,
                    &target,
                    &authentication,
                    "gpt-6.1-sol",
                    &saved,
                    Some(&ModelDefaultPreferenceV1::PreserveSaved)
                )
                .is_none()
            );
        }
        for saved in [
            unknown(),
            ComputeManagementFactValueV2 {
                value: Some(NativeReasoningCapabilityV1::Toggle {
                    parameter: "enable_thinking".into(),
                }),
                basis: ComputeManagementFactBasisV2::RuntimeFallback,
            },
        ] {
            assert!(
                missing_api_reasoning_default(
                    &catalog,
                    &provenance,
                    &target,
                    &authentication,
                    "gpt-6.1-sol",
                    &saved,
                    Some(&ModelDefaultPreferenceV1::PreserveSaved),
                )
                .is_some()
            );
        }
    }

    #[test]
    fn labels_unknown_ids_and_other_targets_do_not_acquire_official_controls() {
        let catalog = catalog();
        let (provenance, mut target, authentication) = scope(
            &catalog,
            "openai.platform.global.v1",
            UpstreamProtocol::Responses,
        );
        for id in ["GPT-6.1 Sol", "gpt-6.1-sol-unknown", "gpt-6.1-terra"] {
            assert!(
                missing_api_reasoning_default(
                    &catalog,
                    &provenance,
                    &target,
                    &authentication,
                    id,
                    &unknown(),
                    None
                )
                .is_none()
            );
        }
        target.authority = "other.example".into();
        assert!(
            missing_api_reasoning_default(
                &catalog,
                &provenance,
                &target,
                &authentication,
                "gpt-6.1-sol",
                &unknown(),
                None
            )
            .is_none()
        );
    }
}
