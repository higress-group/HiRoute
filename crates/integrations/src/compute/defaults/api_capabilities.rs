//! Missing facts for an already admitted official API source. No membership or price inference.
use super::*;
use std::collections::BTreeSet;

pub struct ApiCapabilityCompletion {
    pub capabilities: ComputeManagedCapabilitiesV2,
    pub filled_fields: BTreeSet<ModelDefaultFieldV1>,
    pub inferred_fields: BTreeSet<ModelDefaultFieldV1>,
}

pub fn complete_missing_api_capabilities(
    catalog: &TrustedReleaseCatalog,
    provenance: &ComputeManagementProvenanceV2,
    target: &ComputeManagementTargetV2,
    authentication: &GatewayAuthenticationSemanticsV1,
    upstream: &str,
    saved: &ComputeManagedCapabilitiesV2,
    preferences: &BTreeMap<ModelDefaultFieldV1, ModelDefaultPreferenceV1>,
) -> ApiCapabilityCompletion {
    let mut result = ApiCapabilityCompletion {
        capabilities: apply_custom_capability_defaults(saved, preferences),
        filled_fields: BTreeSet::new(),
        inferred_fields: BTreeSet::new(),
    };
    let defaults = super::api_scope::api_endpoint(catalog, provenance, target, authentication)
        .map(|endpoint| metadata_defaults(catalog.model_metadata(), &endpoint, upstream))
        .unwrap_or_else(unknown);
    if matches!(
        preferences.get(&ModelDefaultFieldV1::NativeReasoning),
        Some(ModelDefaultPreferenceV1::FollowMetadata)
    ) {
        result.capabilities.native_reasoning = fact(None);
    }
    macro_rules! fill {
        ($name:ident, $key:ident) => {
            if !matches!(
                preferences.get(&ModelDefaultFieldV1::$key),
                Some(
                    ModelDefaultPreferenceV1::FollowOfficial { .. }
                        | ModelDefaultPreferenceV1::Custom { .. }
                )
            ) && matches!(
                (&saved.$name.value, saved.$name.basis),
                (None, ComputeManagementFactBasisV2::Unknown)
                    | (Some(_), ComputeManagementFactBasisV2::RuntimeFallback)
            ) && defaults.$name.value.is_some()
                || matches!(
                    preferences.get(&ModelDefaultFieldV1::$key),
                    Some(ModelDefaultPreferenceV1::FollowMetadata)
                )
            {
                result.capabilities.$name = ComputeManagementFactValueV2 {
                    value: defaults.$name.value,
                    basis: if defaults.$name.value.is_some() {
                        ComputeManagementFactBasisV2::RuntimeFallback
                    } else {
                        ComputeManagementFactBasisV2::Unknown
                    },
                };
                result.filled_fields.insert(ModelDefaultFieldV1::$key);
                if defaults.$name.basis == ComputeManagementFactBasisV2::RuntimeFallback {
                    result.inferred_fields.insert(ModelDefaultFieldV1::$key);
                }
            }
        };
    }
    fill!(tool, Tool);
    fill!(vision, Vision);
    fill!(streaming, Streaming);
    fill!(context_tokens, ContextTokens);
    fill!(max_output_tokens, MaxOutputTokens);
    if let Some(default) = missing_api_reasoning_default(
        catalog,
        provenance,
        target,
        authentication,
        upstream,
        &saved.native_reasoning,
        preferences.get(&ModelDefaultFieldV1::NativeReasoning),
    ) {
        result.capabilities.native_reasoning = ComputeManagementFactValueV2 {
            value: Some(default.capability),
            basis: ComputeManagementFactBasisV2::RuntimeFallback,
        };
        result
            .filled_fields
            .insert(ModelDefaultFieldV1::NativeReasoning);
    }
    result
}

fn boolean(value: MetadataCapabilityStateV1) -> Option<bool> {
    match value {
        MetadataCapabilityStateV1::Supported => Some(true),
        MetadataCapabilityStateV1::Unsupported => Some(false),
        _ => None,
    }
}
fn limit(value: &MetadataTokenLimitV1) -> Option<u64> {
    (value.state == MetadataTokenStateV1::Known)
        .then_some(value.value)
        .flatten()
        .filter(|v| *v > 0)
}
fn agreed<T: Eq + Copy>(values: impl Iterator<Item = Option<T>>) -> Option<T> {
    let values = values.collect::<Option<Vec<_>>>()?;
    let first = *values.first()?;
    values.iter().all(|value| *value == first).then_some(first)
}
fn fact<T>(value: Option<T>) -> ComputeManagementFactValueV2<T> {
    ComputeManagementFactValueV2 {
        basis: if value.is_some() {
            ComputeManagementFactBasisV2::RegisteredCatalog
        } else {
            ComputeManagementFactBasisV2::Unknown
        },
        value,
    }
}
fn unknown() -> ComputeManagedCapabilitiesV2 {
    ComputeManagedCapabilitiesV2 {
        tool: fact(None),
        vision: fact(None),
        streaming: fact(None),
        context_tokens: fact(None),
        max_output_tokens: fact(None),
        native_reasoning: fact(None),
    }
}

fn metadata_defaults(
    metadata: &ModelMetadataCatalogV1,
    endpoint: &ProtocolEndpointV1,
    upstream: &str,
) -> ComputeManagedCapabilitiesV2 {
    let wire_target = format!("{}{}", endpoint.base_url, endpoint.request_path);
    let protocol = match endpoint.protocol {
        UpstreamProtocol::Responses => "openai-responses",
        UpstreamProtocol::ChatCompletions => "openai-chat",
        UpstreamProtocol::Messages => "anthropic-messages",
    };
    let products = metadata
        .access_products
        .iter()
        .filter(|product| {
            matches!(
                product.product_key.as_str(),
                "openai-platform-global" | "anthropic-platform"
            ) && product.interfaces.iter().any(|interface| {
                interface.protocol == protocol
                    && interface
                        .base_url
                        .as_ref()
                        .zip(interface.request_path.as_ref())
                        .is_some_and(|(base, path)| {
                            format!("{}{path}", base.trim_end_matches('/')) == wire_target
                        })
            })
        })
        .collect::<Vec<_>>();
    if products.len() != 1 {
        return unknown();
    }
    let product = products[0];
    let bindings = metadata
        .endpoint_bindings
        .iter()
        .filter(|binding| {
            binding.product_key == product.product_key && binding.upstream_model_id == upstream
        })
        .collect::<Vec<_>>();
    if !bindings.is_empty() {
        // An explicit binding, including withdrawal/ambiguity, owns its complete scope.
        let models = bindings
            .iter()
            .map(|binding| {
                if matches!(binding.lifecycle.as_deref(), Some("retired" | "deprecated"))
                    || !product.interfaces.iter().any(|interface| {
                        interface.protocol == protocol
                            && binding
                                .interface_candidates
                                .contains(&interface.interface_key)
                            && interface
                                .base_url
                                .as_ref()
                                .zip(interface.request_path.as_ref())
                                .is_some_and(|(base, path)| {
                                    format!("{}{path}", base.trim_end_matches('/')) == wire_target
                                })
                    })
                {
                    return None;
                }
                let publisher = if product.product_key == "openai-platform-global" {
                    "OpenAI"
                } else {
                    "Anthropic"
                };
                metadata
                    .canonical_models
                    .iter()
                    .find(|model| {
                        model.model_key == binding.model_key
                            && model.publisher == publisher
                            && model.lifecycle == "active"
                    })
                    .map(|model| (*binding, model))
            })
            .collect::<Option<Vec<_>>>();
        let Some(models) = models else {
            return unknown();
        };
        return ComputeManagedCapabilitiesV2 {
            tool: fact(agreed(models.iter().map(|(b, m)| {
                b.capability_overrides
                    .tool
                    .or_else(|| m.capabilities.get("tool").copied().and_then(boolean))
            }))),
            vision: fact(agreed(models.iter().map(|(b, m)| {
                b.capability_overrides
                    .vision
                    .or_else(|| m.modalities.get("image_input").copied().and_then(boolean))
            }))),
            streaming: fact(agreed(models.iter().map(|(b, m)| {
                b.capability_overrides
                    .streaming
                    .or_else(|| m.capabilities.get("streaming").copied().and_then(boolean))
            }))),
            context_tokens: fact(agreed(models.iter().map(|(b, m)| {
                b.capability_overrides
                    .context_tokens
                    .or_else(|| limit(&m.context_tokens))
            }))),
            max_output_tokens: fact(agreed(models.iter().map(|(b, m)| {
                b.capability_overrides
                    .max_output_tokens
                    .or_else(|| limit(&m.max_output_tokens))
            }))),
            native_reasoning: fact(None),
        };
    }
    // Provider records never cross products, endpoints, authentication, protocols or exact IDs.
    let suffix = if endpoint.protocol == UpstreamProtocol::Responses {
        "/responses"
    } else if endpoint.protocol == UpstreamProtocol::ChatCompletions {
        "/chat/completions"
    } else {
        "/messages"
    };
    let records = metadata
        .provider_records
        .iter()
        .filter(|provider| {
            let identity = if product.product_key == "openai-platform-global" {
                matches!(provider.provider_id.as_str(), "openai" | "openai-api")
            } else {
                matches!(provider.provider_id.as_str(), "anthropic" | "anthropic-api")
            };
            identity
                && provider.base_url_candidates.len() == 1
                && format!(
                    "{}{}",
                    provider.base_url_candidates[0].trim_end_matches('/'),
                    suffix
                ) == wire_target
                && provider.protocol_candidates.contains(&endpoint.protocol)
                && provider.authentication_candidates == [MetadataAuthenticationHintV1::ApiKey]
        })
        .flat_map(|provider| {
            metadata.model_records.iter().filter(move |model| {
                model.provider_record_key == provider.provider_record_key
                    && model.provider_id == provider.provider_id
                    && model.upstream_model_id == upstream
            })
        })
        .collect::<Vec<_>>();
    if records.iter().any(|model| {
        model.lifecycle != "active"
            || model.execution_fit.state != MetadataExecutionFitStateV1::NativeTextRepresentable
    }) {
        return unknown();
    }
    let mut result = ComputeManagedCapabilitiesV2 {
        tool: fact(agreed(
            records.iter().map(|m| boolean(m.capability_hints.tool)),
        )),
        vision: fact(agreed(
            records.iter().map(|m| boolean(m.capability_hints.vision)),
        )),
        streaming: fact(agreed(
            records
                .iter()
                .map(|m| boolean(m.capability_hints.streaming)),
        )),
        context_tokens: fact(agreed(records.iter().map(|m| limit(&m.context_tokens)))),
        max_output_tokens: fact(agreed(records.iter().map(|m| limit(&m.max_output_tokens)))),
        native_reasoning: fact(None),
    };
    macro_rules! inferred {
        ($field:ident, $path:literal) => {
            if result.$field.value.is_some()
                && records
                    .iter()
                    .any(|model| model.field_provenance.contains_key($path))
            {
                result.$field.basis = ComputeManagementFactBasisV2::RuntimeFallback;
            }
        };
    }
    inferred!(tool, "capability_hints.tool");
    inferred!(vision, "capability_hints.vision");
    inferred!(streaming, "capability_hints.streaming");
    inferred!(context_tokens, "context_tokens");
    inferred!(max_output_tokens, "max_output_tokens");
    result
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
        c: &TrustedReleaseCatalog,
    ) -> (
        ComputeManagementProvenanceV2,
        ComputeManagementTargetV2,
        GatewayAuthenticationSemanticsV1,
    ) {
        let option = "openai.platform.global.v1";
        let resolved = c.resolve_connection_option(option).unwrap();
        let e = resolved
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .find(|e| e.protocol == UpstreamProtocol::Responses)
            .unwrap();
        (
            ComputeManagementProvenanceV2::Registered {
                connection_option_id: option.into(),
                registry_version: "fixture/registry".into(),
                catalog_digest: CanonicalDigest::of_bytes(b"fixture"),
            },
            ComputeManagementTargetV2 {
                scheme: "https".into(),
                authority: e.base_url.strip_prefix("https://").unwrap().into(),
                port: 443,
                request_path: e.request_path.clone(),
                upstream_protocol: e.protocol,
                protocol_profile_id: e.adapter_ref.clone(),
                protocol_profile_revision: e.adapter_revision,
            },
            e.authentication_semantics.clone().unwrap(),
        )
    }
    fn complete(
        c: &TrustedReleaseCatalog,
        id: &str,
        saved: &ComputeManagedCapabilitiesV2,
        p: &BTreeMap<ModelDefaultFieldV1, ModelDefaultPreferenceV1>,
    ) -> ApiCapabilityCompletion {
        let (origin, target, auth) = scope(c);
        complete_missing_api_capabilities(c, &origin, &target, &auth, id, saved, p)
    }
    #[test]
    fn api_defaults_complete_all_missing_fields_and_keep_source_bytes() {
        let c = catalog();
        let saved = unknown();
        let before = saved.clone();
        let p = BTreeMap::from([(
            ModelDefaultFieldV1::Tool,
            ModelDefaultPreferenceV1::PreserveSaved,
        )]);
        let result = complete(&c, "gpt-6-astra", &saved, &p);
        assert_eq!(result.capabilities.tool.value, Some(true));
        assert_eq!(result.capabilities.vision.value, Some(true));
        assert_eq!(result.capabilities.streaming.value, Some(true));
        assert_eq!(result.capabilities.context_tokens.value, Some(1_050_000));
        assert_eq!(result.capabilities.max_output_tokens.value, Some(128_000));
        assert_eq!(result.filled_fields.len(), 6);
        assert!(result.inferred_fields.contains(&ModelDefaultFieldV1::Tool));
        assert!(
            result
                .inferred_fields
                .contains(&ModelDefaultFieldV1::Streaming)
        );
        assert_eq!(saved, before);
        result.capabilities.validate().unwrap();
        assert!(
            complete(&c, "gpt-reserve", &saved, &p)
                .filled_fields
                .is_empty()
        );
    }
    #[test]
    fn custom_false_unknown_limits_and_profiles_survive_metadata_updates() {
        let c = catalog();
        let saved = unknown();
        let native = NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".into(),
            profiles: vec!["low".into(), "high".into()],
            default_profile: Some("low".into()),
        };
        let p = BTreeMap::from([
            (
                ModelDefaultFieldV1::Tool,
                ModelDefaultPreferenceV1::Custom {
                    value: ModelDefaultValueV1::Boolean(false),
                },
            ),
            (
                ModelDefaultFieldV1::Vision,
                ModelDefaultPreferenceV1::Custom {
                    value: ModelDefaultValueV1::Unknown(()),
                },
            ),
            (
                ModelDefaultFieldV1::ContextTokens,
                ModelDefaultPreferenceV1::Custom {
                    value: ModelDefaultValueV1::Tokens(64_000),
                },
            ),
            (
                ModelDefaultFieldV1::NativeReasoning,
                ModelDefaultPreferenceV1::Custom {
                    value: ModelDefaultValueV1::Reasoning(native.clone()),
                },
            ),
        ]);
        let result = complete(&c, "gpt-6-astra", &saved, &p);
        assert_eq!(result.capabilities.tool.value, Some(false));
        assert_eq!(result.capabilities.vision.value, None);
        assert_eq!(result.capabilities.context_tokens.value, Some(64_000));
        assert_eq!(result.capabilities.native_reasoning.value, Some(native));
        assert!(
            [
                ModelDefaultFieldV1::Tool,
                ModelDefaultFieldV1::Vision,
                ModelDefaultFieldV1::ContextTokens,
                ModelDefaultFieldV1::NativeReasoning
            ]
            .iter()
            .any(|field| result.filled_fields.contains(field))
        );
        result.capabilities.validate().unwrap();
    }
    #[test]
    fn following_metadata_replaces_old_values_and_unknown_id_never_keeps_stale_values() {
        let c = catalog();
        let mut saved = unknown();
        saved.tool = ComputeManagementFactValueV2 {
            value: Some(false),
            basis: ComputeManagementFactBasisV2::UserDeclared,
        };
        let p = BTreeMap::from([(
            ModelDefaultFieldV1::Tool,
            ModelDefaultPreferenceV1::FollowMetadata,
        )]);
        assert_eq!(
            complete(&c, "gpt-6-astra", &saved, &p)
                .capabilities
                .tool
                .value,
            Some(true)
        );
        assert_eq!(
            complete(&c, "gpt-reserve", &saved, &p)
                .capabilities
                .tool
                .value,
            None
        );
        assert_eq!(
            complete(&c, "gpt-6-astra", &saved, &BTreeMap::new())
                .capabilities
                .tool
                .value,
            Some(false)
        );
        let (origin, mut target, auth) = scope(&c);
        target.authority = "api.example.test/v1".into();
        let result = complete_missing_api_capabilities(
            &c,
            &origin,
            &target,
            &auth,
            "gpt-6-astra",
            &unknown(),
            &BTreeMap::new(),
        );
        assert!(result.filled_fields.is_empty());
    }
    #[test]
    fn conflicting_provider_fact_keeps_only_that_field_unknown() {
        let c = catalog();
        let (_, target, _) = scope(&c);
        let e =
            super::super::api_scope::api_endpoint(&c, &scope(&c).0, &target, &scope(&c).2).unwrap();
        let mut m = c.model_metadata().clone();
        let mut duplicate = m
            .model_records
            .iter()
            .find(|r| r.provider_id == "openai" && r.upstream_model_id == "gpt-6-astra")
            .unwrap()
            .clone();
        duplicate.capability_hints.vision = MetadataCapabilityStateV1::Unsupported;
        m.model_records.push(duplicate);
        let result = metadata_defaults(&m, &e, "gpt-6-astra");
        assert_eq!(result.vision.value, None);
        assert_eq!(result.tool.value, Some(true));
        assert_eq!(result.context_tokens.value, Some(1_050_000));
    }
}
