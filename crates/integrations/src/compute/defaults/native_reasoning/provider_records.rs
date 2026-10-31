//! Model hints supply efforts; the already matched official connector owns their native wire path.
use super::ApiReasoningDefault;
use hiroute_domain::*;

pub(super) fn openai_provider_default(
    metadata: &ModelMetadataCatalogV1,
    protocol: UpstreamProtocol,
    wire_target: &str,
    upstream_model_id: &str,
) -> Option<ApiReasoningDefault> {
    // An explicit product binding, including a withdrawal or unsupported interface,
    // keeps authority over provider-prefill hints. Do not revive a rejected binding.
    if metadata.endpoint_bindings.iter().any(|binding| {
        binding.product_key == "openai-platform-global"
            && binding.upstream_model_id == upstream_model_id
    }) {
        return None;
    }
    let suffix = match protocol {
        UpstreamProtocol::Responses => "/responses",
        UpstreamProtocol::ChatCompletions => "/chat/completions",
        UpstreamProtocol::Messages => return None,
    };
    let mut profiles = None;
    for provider in &metadata.provider_records {
        if !matches!(provider.provider_id.as_str(), "openai-api" | "openai")
            || provider.base_url_candidates.len() != 1
            || format!(
                "{}{}",
                provider.base_url_candidates[0].trim_end_matches('/'),
                suffix
            ) != wire_target
            || !provider.protocol_candidates.contains(&protocol)
            || provider.authentication_candidates != [MetadataAuthenticationHintV1::ApiKey]
        {
            continue;
        }
        for model in metadata.model_records.iter().filter(|model| {
            model.provider_record_key == provider.provider_record_key
                && model.provider_id == provider.provider_id
                && model.upstream_model_id == upstream_model_id
        }) {
            if model.lifecycle != "active"
                || model.capability_hints.reasoning != MetadataCapabilityStateV1::Supported
                || model.execution_fit.state != MetadataExecutionFitStateV1::NativeTextRepresentable
                || model.metadata_completeness.reasoning_rendering
                    != MetadataCompletenessV1::Complete
                || !model
                    .usable_for
                    .contains(&MetadataUsageScenarioV1::ReasoningCapabilityPrefill)
                || !model
                    .usable_for
                    .contains(&MetadataUsageScenarioV1::ReasoningRenderingHint)
            {
                return None;
            }
            let supported = &model.reasoning_rendering_hints.supported_reasoning_efforts;
            // Records store sets in lexical order. Native low-to-high order is
            // required by shared authoring's economy/primary defaults.
            let ordered: Vec<String> = ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
                .into_iter()
                .filter(|effort| supported.iter().any(|value| value == effort))
                .map(str::to_owned)
                .collect();
            if ordered.is_empty() || ordered.len() != supported.len() {
                return None;
            }
            if profiles
                .as_ref()
                .is_some_and(|existing| existing != &ordered)
            {
                return None;
            }
            profiles = Some(ordered);
        }
    }
    Some(ApiReasoningDefault {
        capability: NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".into(),
            profiles: profiles?,
            // These records do not assert a provider default. Ordinary route
            // authoring chooses its own visible economy/primary selection.
            default_profile: None,
        },
        render_convention: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> ModelMetadataCatalogV1 {
        let catalog = crate::TrustedReleaseCatalog::load_release_facts(
            include_bytes!("../../../../../../assets/release-facts/current/bundle/manifest.json"),
            include_bytes!(
                "../../../../../../assets/release-facts/current/bundle/connector-registry.json"
            ),
            include_bytes!("../../../../../../assets/release-facts/current/bundle/model-data.json"),
        )
        .unwrap();
        catalog.model_metadata().clone()
    }
    fn resolve(m: &ModelMetadataCatalogV1, id: &str) -> Option<ApiReasoningDefault> {
        openai_provider_default(
            m,
            UpstreamProtocol::Responses,
            "https://api.openai.com/v1/responses",
            id,
        )
    }
    #[test]
    fn api_scoped_records_complete_existing_imports_in_native_effort_order() {
        let metadata = metadata();
        for id in [
            "gpt-5.6-luna",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-6-astra",
        ] {
            let result = resolve(&metadata, id).unwrap();
            let expected: Vec<String> = if id == "gpt-6-astra" {
                vec!["low", "medium", "high", "xhigh", "max"]
            } else {
                vec!["none", "low", "medium", "high", "xhigh", "max"]
            }
            .into_iter()
            .map(str::to_owned)
            .collect();
            assert_eq!(
                result.capability,
                NativeReasoningCapabilityV1::Discrete {
                    parameter: "reasoning_effort".into(),
                    profiles: expected,
                    default_profile: None,
                }
            );
        }
        assert!(resolve(&metadata, "gpt-6.1-terra").is_none());
        assert!(resolve(&metadata, "gpt-reserve").is_none());
    }
    #[test]
    fn conflicting_unknown_and_incomplete_records_never_supply_controls() {
        for mutation in [0, 1, 2] {
            let mut m = metadata();
            let model = m
                .model_records
                .iter_mut()
                .find(|r| r.model_record_key == "hermes-agent/openai-api/gpt-5.6-luna")
                .unwrap();
            match mutation {
                0 => {
                    model
                        .reasoning_rendering_hints
                        .supported_reasoning_efforts
                        .retain(|p| p != "max");
                }
                1 => {
                    model
                        .reasoning_rendering_hints
                        .supported_reasoning_efforts
                        .push("unrecognized-effort".into());
                }
                _ => {
                    model.metadata_completeness.reasoning_rendering =
                        MetadataCompletenessV1::Partial
                }
            }
            assert!(resolve(&m, "gpt-5.6-luna").is_none());
        }
    }
    #[test]
    fn other_products_urls_protocols_and_withdrawn_bindings_are_not_rescued() {
        let mut m = metadata();
        m.provider_records
            .retain(|p| p.provider_record_key == "openclaw/openai");
        let original = m.clone();
        for mutation in [0, 1, 2, 3] {
            let mut m = original.clone();
            let provider = &mut m.provider_records[0];
            match mutation {
                0 => provider.provider_id = "openai-codex".into(),
                1 => provider.base_url_candidates = vec!["https://other.example/v1".into()],
                2 => provider
                    .base_url_candidates
                    .push("https://other.example/v1".into()),
                _ => provider.protocol_candidates = vec![UpstreamProtocol::ChatCompletions],
            }
            assert!(resolve(&m, "gpt-5.6-luna").is_none());
        }
        let mut binding = m
            .endpoint_bindings
            .iter()
            .find(|b| b.product_key == "openai-platform-global")
            .unwrap()
            .clone();
        binding.upstream_model_id = "gpt-5.6-luna".into();
        binding.lifecycle = Some("retired".into());
        m.endpoint_bindings.push(binding);
        assert!(resolve(&m, "gpt-5.6-luna").is_none());
        assert!(
            openai_provider_default(
                &original,
                UpstreamProtocol::Messages,
                "https://api.openai.com/v1/messages",
                "gpt-5.6-luna"
            )
            .is_none()
        );
    }
}
