//! Descriptive product overrides are matched to an already qualified installed wire endpoint.
use super::*;

pub fn matching_endpoint_metadata<'a>(
    catalog: &'a TrustedReleaseCatalog,
    capability: &ModelEndpointCapabilityV1,
) -> Vec<&'a MetadataEndpointBindingV1> {
    let Some(profile) = catalog
        .registry()
        .endpoint_profile(&capability.endpoint_profile_id)
        .filter(|p| p.revision == capability.endpoint_profile_revision)
    else {
        return vec![];
    };
    let Some(endpoint) = profile.protocol_endpoints.iter().find(|e| {
        e.protocol_endpoint_id == capability.protocol_endpoint_id
            && e.protocol == capability.upstream_protocol
            && e.adapter_ref == capability.required_adapter_ref
            && e.adapter_revision == capability.required_adapter_revision
    }) else {
        return vec![];
    };
    let protocol = match endpoint.protocol {
        UpstreamProtocol::ChatCompletions => "openai-chat",
        UpstreamProtocol::Responses => "openai-responses",
        UpstreamProtocol::Messages => "anthropic-messages",
    };
    let uri = format!("{}{}", endpoint.base_url, endpoint.request_path);
    let mut result = Vec::new();
    for product in &catalog.model_metadata().access_products {
        for interface in &product.interfaces {
            if interface.protocol != protocol
                || !interface
                    .base_url
                    .as_ref()
                    .zip(interface.request_path.as_ref())
                    .is_some_and(|(base, path)| format!("{base}{path}") == uri)
            {
                continue;
            }
            result.extend(
                catalog
                    .model_metadata()
                    .endpoint_bindings
                    .iter()
                    .filter(|b| {
                        b.product_key == product.product_key
                            && b.upstream_model_id == capability.upstream_model_id
                            && b.interface_candidates.contains(&interface.interface_key)
                    }),
            );
        }
    }
    result.sort_by(|a, b| a.binding_key.cmp(&b.binding_key));
    result.dedup_by(|a, b| a.binding_key == b.binding_key);
    result
}

pub fn official_capability_defaults(
    catalog: &TrustedReleaseCatalog,
    capability: &ModelEndpointCapabilityV1,
    model: &ModelDefinitionV1,
    reasoning: &ModelNativeReasoningV1,
) -> ComputeManagedCapabilitiesV2 {
    let bindings = matching_endpoint_metadata(catalog, capability);
    fn fact<T>(value: Option<T>) -> ComputeManagementFactValueV2<T> {
        let basis = if value.is_some() {
            ComputeManagementFactBasisV2::RegisteredCatalog
        } else {
            ComputeManagementFactBasisV2::Unknown
        };
        ComputeManagementFactValueV2 { value, basis }
    }
    macro_rules! field {
        ($name:ident) => {{
            let values = bindings
                .iter()
                .map(|b| {
                    b.capability_overrides
                        .$name
                        .unwrap_or(model.capabilities.$name)
                })
                .collect::<Vec<_>>();
            let value = values.first().copied().unwrap_or(model.capabilities.$name);
            fact(values.iter().all(|other| *other == value).then_some(value))
        }};
    }
    ComputeManagedCapabilitiesV2 {
        tool: field!(tool),
        vision: field!(vision),
        streaming: field!(streaming),
        context_tokens: field!(context_tokens),
        max_output_tokens: field!(max_output_tokens),
        native_reasoning: fact(Some(reasoning.capability.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_feed::{FeedPayload, admit};
    fn installed() -> TrustedReleaseCatalog {
        TrustedReleaseCatalog::load_release_facts(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/release-facts/current/bundle/manifest.json"
            )),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/release-facts/current/bundle/connector-registry.json"
            )),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/release-facts/current/bundle/model-data.json"
            )),
        )
        .unwrap()
    }
    fn selected(ambiguous: bool) -> (TrustedReleaseCatalog, String) {
        let installed = installed();
        let cap = installed
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .find(|c| c.endpoint_profile_id == "endpoint.bailian.payg.cn.v1")
            .unwrap();
        let mut snapshot = installed.official_snapshot();
        let data = &mut snapshot.data;
        let canonical = data
            .metadata_catalog
            .canonical_models
            .iter()
            .find(|m| m.upstream_ids.contains(&cap.upstream_model_id))
            .unwrap();
        let template = data
            .metadata_catalog
            .endpoint_bindings
            .iter()
            .find(|b| b.product_key == "bailian-payg-cn")
            .unwrap()
            .clone();
        let mut binding = template.clone();
        binding.binding_key = "fixture/exact-bailian-scope".into();
        binding.model_key = canonical.model_key.clone();
        binding.upstream_model_id = cap.upstream_model_id.clone();
        binding.interface_candidates = vec!["bailian-payg-cn/openai-chat".into()];
        binding.capability_overrides.context_tokens = Some(65536);
        binding.capability_overrides.vision = Some(false);
        data.metadata_catalog
            .endpoint_bindings
            .push(binding.clone());
        if ambiguous {
            let mut product = data
                .metadata_catalog
                .access_products
                .iter()
                .find(|p| p.product_key == "bailian-payg-cn")
                .unwrap()
                .clone();
            product.product_key = "fixture-other-product".into();
            for interface in &mut product.interfaces {
                interface.interface_key = interface
                    .interface_key
                    .replace("bailian-payg-cn", "fixture-other-product");
            }
            data.metadata_catalog.access_products.push(product);
            binding.binding_key = "fixture/other-exact-wire-scope".into();
            binding.product_key = "fixture-other-product".into();
            binding.interface_candidates = vec!["fixture-other-product/openai-chat".into()];
            binding.capability_overrides.context_tokens = Some(131072);
            data.metadata_catalog.endpoint_bindings.push(binding);
        }
        data.metadata_catalog
            .endpoint_bindings
            .sort_by(|a, b| a.binding_key.cmp(&b.binding_key));
        data.metadata_catalog
            .access_products
            .sort_by(|a, b| a.product_key.cmp(&b.product_key));
        snapshot.data_revision = 2;
        snapshot.data_digest = CanonicalDigest::of(data).unwrap();
        let payload = FeedPayload::from_snapshot(
            snapshot,
            2,
            "2026-10-18T00:00:00Z".into(),
            "1".repeat(40),
            vec![],
        );
        let admission = admit(&installed, &payload, "0.3.2").unwrap();
        (
            installed.with_feed(&admission).unwrap(),
            cap.model_configuration_id.clone(),
        )
    }
    #[test]
    fn model_defaults_exact_endpoint_overrides_general_and_ambiguous_product_limits_stay_unknown() {
        let (catalog, id) = selected(false);
        let cap = catalog
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .find(|c| c.model_configuration_id == id)
            .unwrap();
        let model = catalog.model_data().model(&id).unwrap();
        let reasoning = catalog
            .native_reasoning()
            .iter()
            .find(|r| r.model_configuration_id == id)
            .unwrap();
        let defaults = official_capability_defaults(&catalog, cap, model, reasoning);
        assert_eq!(defaults.context_tokens.value, Some(65536));
        assert_eq!(defaults.vision.value, Some(false));
        let (catalog, id) = selected(true);
        let cap = catalog
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .find(|c| c.model_configuration_id == id)
            .unwrap();
        let model = catalog.model_data().model(&id).unwrap();
        let reasoning = catalog
            .native_reasoning()
            .iter()
            .find(|r| r.model_configuration_id == id)
            .unwrap();
        let mut defaults = official_capability_defaults(&catalog, cap, model, reasoning);
        assert_eq!(defaults.context_tokens.value, None);
        defaults.context_tokens = ComputeManagementFactValueV2 {
            value: Some(123456),
            basis: ComputeManagementFactBasisV2::UserDeclared,
        };
        assert_eq!(
            resolve_managed_defaults(&catalog, &defaults, &BTreeMap::new(), cap, model, reasoning)
                .unwrap()
                .context_tokens
                .value,
            Some(123456)
        );
    }
}
