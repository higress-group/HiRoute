//! Editable capability prefill for an explicit recheck of an existing registered source.

use hiroute_domain::{
    MetadataCanonicalModelV1, MetadataCapabilityStateV1, MetadataEndpointBindingV1,
    MetadataTokenStateV1, ModelMetadataCatalogV1, NativeReasoningCapabilityV1, UpstreamProtocol,
};
use hiroute_integrations::{
    NativeCandidateFactBasisV1, NativeCandidateFactValueV1, NativeModelCapabilityDeclarationV1,
    NativeModelDeclarationV1,
};

const FALLBACK_CONTEXT: u64 = 200_000;
const FALLBACK_OUTPUT: u64 = 32_768;

pub(super) fn registered_product_candidates(
    catalog: &ModelMetadataCatalogV1,
    base_url: &str,
    request_path: &str,
    protocol: UpstreamProtocol,
) -> Vec<NativeModelDeclarationV1> {
    let protocol_name = match protocol {
        UpstreamProtocol::ChatCompletions => "openai-chat",
        UpstreamProtocol::Responses => "openai-responses",
        UpstreamProtocol::Messages => "anthropic-messages",
    };
    let endpoint = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        request_path.trim_start_matches('/')
    );
    let matches_interface = |interface: &hiroute_domain::MetadataProductInterfaceV1| {
        interface.protocol == protocol_name
            && interface
                .base_url
                .as_ref()
                .zip(interface.request_path.as_ref())
                .is_some_and(|(base, path)| {
                    format!(
                        "{}/{}",
                        base.trim_end_matches('/'),
                        path.trim_start_matches('/')
                    ) == endpoint
                })
    };
    let products: Vec<_> = catalog
        .access_products
        .iter()
        .filter(|product| product.interfaces.iter().any(&matches_interface))
        .collect();
    let interfaces: std::collections::BTreeSet<_> = products
        .iter()
        .flat_map(|product| {
            product
                .interfaces
                .iter()
                .filter(|interface| matches_interface(interface))
                .map(move |interface| {
                    (
                        product.product_key.as_str(),
                        interface.interface_key.as_str(),
                    )
                })
        })
        .collect();
    let retired: std::collections::BTreeSet<_> = catalog
        .endpoint_bindings
        .iter()
        .filter(|binding| {
            products
                .iter()
                .any(|product| product.product_key == binding.product_key)
                && matches!(binding.lifecycle.as_deref(), Some("deprecated" | "retired"))
        })
        .map(|binding| binding.upstream_model_id.as_str())
        .collect();
    let mut ids: std::collections::BTreeSet<String> = products
        .iter()
        .flat_map(|product| product.documented_upstream_model_ids.iter().cloned())
        .collect();
    ids.extend(
        catalog
            .endpoint_bindings
            .iter()
            .filter(|binding| {
                binding
                    .interface_candidates
                    .iter()
                    .any(|key| interfaces.contains(&(binding.product_key.as_str(), key.as_str())))
            })
            .map(|binding| binding.upstream_model_id.clone()),
    );
    fn unknown<T>() -> NativeCandidateFactValueV1<T> {
        NativeCandidateFactValueV1 {
            value: None,
            basis: NativeCandidateFactBasisV1::Unknown,
        }
    }
    ids.into_iter()
        .filter(|id| !retired.contains(id.as_str()))
        .map(|id| {
            let display_name = matching_binding(catalog, base_url, request_path, protocol, &id)
                .and_then(|(binding, _)| {
                    catalog
                        .canonical_models
                        .iter()
                        .find(|model| model.model_key == binding.model_key)
                })
                .map(|model| model.display_name.clone())
                .unwrap_or_else(|| id.clone());
            let mut declaration = NativeModelDeclarationV1 {
                upstream_model_id: id,
                display_name,
                catalog_configuration_id: None,
                membership: hiroute_application_api::ComputeModelMembershipV2::UserDeclared,
                capabilities: NativeModelCapabilityDeclarationV1 {
                    tool: unknown(),
                    vision: unknown(),
                    streaming: unknown(),
                    context_tokens: unknown(),
                    max_output_tokens: unknown(),
                    native_reasoning: unknown(),
                },
            };
            fill_unknown_registered_model(
                &mut declaration,
                catalog,
                base_url,
                request_path,
                protocol,
            );
            declaration
        })
        .collect()
}

pub(super) fn fill_unknown_registered_model(
    declaration: &mut NativeModelDeclarationV1,
    catalog: &ModelMetadataCatalogV1,
    base_url: &str,
    request_path: &str,
    protocol: UpstreamProtocol,
) {
    let metadata = matching_binding(
        catalog,
        base_url,
        request_path,
        protocol,
        &declaration.upstream_model_id,
    )
    .and_then(|(binding, product_key)| {
        catalog
            .canonical_models
            .iter()
            .find(|model| model.model_key == binding.model_key)
            .map(|model| (model, binding, product_key))
    });
    let (model, binding, product_key) = match metadata {
        Some((model, binding, product_key)) => (Some(model), Some(binding), Some(product_key)),
        None => (None, None, None),
    };
    let overrides = binding.map(|value| &value.capability_overrides);
    let context = overrides
        .and_then(|value| value.context_tokens)
        .or_else(|| model.and_then(|value| known_limit(&value.context_tokens)))
        .unwrap_or(FALLBACK_CONTEXT);
    fill_unknown(&mut declaration.capabilities.context_tokens, context);
    let effective_context = declaration
        .capabilities
        .context_tokens
        .value
        .unwrap_or(context);
    let output = overrides
        .and_then(|value| value.max_output_tokens)
        .or_else(|| model.and_then(|value| known_limit(&value.max_output_tokens)))
        .unwrap_or(FALLBACK_OUTPUT)
        .min(effective_context);
    fill_unknown(&mut declaration.capabilities.max_output_tokens, output);
    fill_unknown(
        &mut declaration.capabilities.tool,
        overrides
            .and_then(|value| value.tool)
            .unwrap_or_else(|| capability(model, "tool", true)),
    );
    fill_unknown(
        &mut declaration.capabilities.vision,
        overrides
            .and_then(|value| value.vision)
            .unwrap_or_else(|| modality(model, "image_input", false)),
    );
    fill_unknown(
        &mut declaration.capabilities.streaming,
        overrides
            .and_then(|value| value.streaming)
            .unwrap_or_else(|| capability(model, "streaming", true)),
    );
    fill_unknown(
        &mut declaration.capabilities.native_reasoning,
        reasoning(model, product_key),
    );
}

fn matching_binding<'a>(
    catalog: &'a ModelMetadataCatalogV1,
    base_url: &str,
    request_path: &str,
    protocol: UpstreamProtocol,
    upstream_model_id: &str,
) -> Option<(&'a MetadataEndpointBindingV1, &'a str)> {
    let protocol_name = match protocol {
        UpstreamProtocol::ChatCompletions => "openai-chat",
        UpstreamProtocol::Responses => "openai-responses",
        UpstreamProtocol::Messages => "anthropic-messages",
    };
    let endpoint = format!("{}{}", base_url.trim_end_matches('/'), request_path);
    let interfaces: Vec<_> = catalog
        .access_products
        .iter()
        .flat_map(|product| {
            product
                .interfaces
                .iter()
                .map(move |interface| (product, interface))
        })
        .filter(|(_, interface)| {
            interface.protocol == protocol_name
                && interface
                    .base_url
                    .as_ref()
                    .zip(interface.request_path.as_ref())
                    .is_some_and(|(base, path)| {
                        format!("{}{}", base.trim_end_matches('/'), path) == endpoint
                    })
        })
        .collect();
    let mut matches = catalog.endpoint_bindings.iter().filter(|binding| {
        binding.upstream_model_id == upstream_model_id
            && !matches!(binding.lifecycle.as_deref(), Some("deprecated" | "retired"))
            && interfaces.iter().any(|(product, interface)| {
                product.product_key == binding.product_key
                    && binding
                        .interface_candidates
                        .contains(&interface.interface_key)
            })
    });
    let first = matches.next()?;
    if matches.any(|other| {
        other.model_key != first.model_key
            || other.capability_overrides != first.capability_overrides
    }) {
        return None;
    }
    Some((first, first.product_key.as_str()))
}

fn known_limit(limit: &hiroute_domain::MetadataTokenLimitV1) -> Option<u64> {
    (limit.state == MetadataTokenStateV1::Known)
        .then_some(limit.value)
        .flatten()
}

fn state(value: Option<MetadataCapabilityStateV1>, fallback: bool) -> bool {
    match value {
        Some(MetadataCapabilityStateV1::Supported) => true,
        Some(MetadataCapabilityStateV1::Unsupported) => false,
        _ => fallback,
    }
}

fn capability(model: Option<&MetadataCanonicalModelV1>, key: &str, fallback: bool) -> bool {
    state(
        model.and_then(|value| value.capabilities.get(key).copied()),
        fallback,
    )
}

fn modality(model: Option<&MetadataCanonicalModelV1>, key: &str, fallback: bool) -> bool {
    state(
        model.and_then(|value| value.modalities.get(key).copied()),
        fallback,
    )
}

fn reasoning(
    model: Option<&MetadataCanonicalModelV1>,
    product_key: Option<&str>,
) -> NativeReasoningCapabilityV1 {
    let Some(model) = model else {
        return NativeReasoningCapabilityV1::Toggle {
            parameter: "enable_thinking".into(),
        };
    };
    let hint = &model.reasoning;
    let profiles = hint
        .profiles
        .iter()
        .filter(|profile| profile.as_str() != "ultra")
        .cloned()
        .collect::<Vec<_>>();
    if matches!(
        hint.kind.as_str(),
        "discrete"
            | "discrete-or-budget"
            | "discrete-fixed-on"
            | "toggle-plus-discrete"
            | "adaptive-fixed-on"
    ) && !profiles.is_empty()
        && profiles[0] != "provider-default"
    {
        return NativeReasoningCapabilityV1::Discrete {
            parameter: if hint.kind == "adaptive-fixed-on" {
                "claude_adaptive_effort"
            } else {
                "reasoning_effort"
            }
            .into(),
            default_profile: hint
                .default
                .as_ref()
                .filter(|value| profiles.contains(*value))
                .cloned(),
            profiles,
        };
    }
    match hint.kind.as_str() {
        "unsupported" => NativeReasoningCapabilityV1::Fixed {
            profile: "non-thinking".into(),
        },
        "fixed-on" | "discrete-fixed-on" | "adaptive-fixed-on" => {
            NativeReasoningCapabilityV1::Fixed {
                profile: "provider-default".into(),
            }
        }
        _ => NativeReasoningCapabilityV1::Toggle {
            parameter: if product_key == Some("deepseek-platform") {
                "deepseek_thinking"
            } else {
                "enable_thinking"
            }
            .into(),
        },
    }
}

fn fill_unknown<T>(fact: &mut NativeCandidateFactValueV1<T>, value: T) {
    if fact.basis == NativeCandidateFactBasisV1::Unknown {
        *fact = NativeCandidateFactValueV1 {
            value: Some(value),
            basis: NativeCandidateFactBasisV1::UserDeclared,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_token_plan_append_candidates_are_product_scoped_declarations() {
        let catalog = crate::release_catalog::current_fixture_catalog();
        let candidates = registered_product_candidates(
            catalog.model_metadata(),
            "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
            "/chat/completions",
            UpstreamProtocol::ChatCompletions,
        );
        assert!(
            candidates.len() > 1,
            "a saved single model must offer other product models"
        );
        assert!(
            candidates
                .iter()
                .any(|model| model.upstream_model_id == "qwen3.6-plus")
        );
        assert!(
            candidates
                .iter()
                .all(|model| model.catalog_configuration_id.is_none()
                    && model.membership
                        == hiroute_application_api::ComputeModelMembershipV2::UserDeclared
                    && model.capabilities.context_tokens.basis
                        == NativeCandidateFactBasisV1::UserDeclared)
        );
        assert!(
            registered_product_candidates(
                catalog.model_metadata(),
                "https://unrelated.example.test/v1",
                "/chat/completions",
                UpstreamProtocol::ChatCompletions
            )
            .is_empty()
        );
    }
}
