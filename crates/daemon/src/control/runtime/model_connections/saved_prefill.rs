//! Editable capability prefill for an explicit recheck of an existing registered source.

use hiroute_domain::{MetadataEndpointBindingV1, ModelMetadataCatalogV1, UpstreamProtocol};
use hiroute_integrations::{NativeModelCapabilityDeclarationV1, NativeModelDeclarationV1};

/// Same-product IDs offered for explicit append. These are descriptive metadata matches
/// without an exact admitted capability binding, so every optional fact stays unknown.
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
            NativeModelDeclarationV1 {
                upstream_model_id: id,
                display_name,
                catalog_configuration_id: None,
                membership: hiroute_application_api::ComputeModelMembershipV2::UserDeclared,
                capabilities: NativeModelCapabilityDeclarationV1::default(),
            }
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_integrations::{NativeCandidateFactBasisV1, NativeCandidateFactValueV1};

    #[test]
    fn saved_token_plan_append_candidates_are_unknown_product_scoped_declarations() {
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
        let facts = |fact: &NativeCandidateFactValueV1<_>| {
            (
                fact.value.is_none(),
                fact.basis == NativeCandidateFactBasisV1::Unknown,
            )
        };
        assert!(candidates.iter().all(|model| {
            model.catalog_configuration_id.is_none()
                && model.membership == hiroute_application_api::ComputeModelMembershipV2::UserDeclared
                && facts(&model.capabilities.tool) == (true, true)
                && facts(&model.capabilities.vision) == (true, true)
                && facts(&model.capabilities.streaming) == (true, true)
                && facts(&model.capabilities.context_tokens) == (true, true)
                && facts(&model.capabilities.max_output_tokens) == (true, true)
                && facts(&model.capabilities.native_reasoning) == (true, true)
        }));
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
