use hiroute_domain::{MetadataExecutionFitStateV1, MetadataTokenStateV1, OfficialModelSnapshotV1};
use hiroute_integrations::TrustedReleaseCatalog;

fn current() -> OfficialModelSnapshotV1 {
    let manifest = include_bytes!("../../../../assets/release-facts/current/bundle/manifest.json");
    TrustedReleaseCatalog::load_bundled_release_facts(
        manifest,
        manifest,
        include_bytes!("../../../../assets/release-facts/current/bundle/connector-registry.json"),
        include_bytes!("../../../../assets/release-facts/current/bundle/model-data.json"),
    )
    .expect("accept the actual current bundled catalog")
    .with_embedded_official_data(
        serde_json::from_slice(include_bytes!(
            "../../../../assets/release-facts/current/bundle/official-model-data.json"
        ))
        .unwrap(),
    )
    .expect("accept the matching complete official snapshot")
    .official_snapshot()
}

#[test]
fn current_reader_preserves_hydrogen_native_only_shared_budget() {
    let snapshot = current();
    snapshot.validate().unwrap();
    let catalog = &snapshot.data.metadata_catalog;
    let model = catalog
        .canonical_models
        .iter()
        .find(|model| model.model_key == "gemini-4-hydrogen")
        .unwrap();
    assert_eq!(model.context_tokens.value, Some(1_048_576));
    assert_eq!(
        model.max_output_tokens.state,
        MetadataTokenStateV1::SharedTotalBudget
    );
    assert_eq!(model.max_output_tokens.value, None);
    assert_eq!(model.reasoning.default.as_deref(), Some("high"));
    assert_eq!(model.reasoning.profiles, ["low", "medium", "high", "xhigh"]);
    let record = catalog
        .model_records
        .iter()
        .find(|record| record.upstream_model_id == "gemini-4-hydrogen")
        .unwrap();
    assert_eq!(record.provider_record_key, "official/gemini-api-native");
    assert_eq!(
        record.execution_fit.state,
        MetadataExecutionFitStateV1::Unsupported
    );
    let bindings: Vec<_> = catalog
        .endpoint_bindings
        .iter()
        .filter(|binding| binding.model_key == model.model_key)
        .collect();
    assert_eq!(bindings.len(), 2);
    for binding in bindings {
        assert_eq!(binding.product_key, "gemini-api");
        assert_eq!(binding.availability.state, "conditional");
        assert_eq!(binding.protocol_qualification, "runtime-required");
        assert_eq!(binding.interface_candidates.len(), 1);
        let interface = binding.interface_candidates[0].as_str();
        assert!(matches!(
            interface,
            "gemini-api/google-generate-content" | "gemini-api/gemini-interactions"
        ));
        assert_eq!(
            binding.capability_overrides.max_output_tokens,
            (interface == "gemini-api/google-generate-content").then_some(65_536)
        );
    }
    assert!(
        snapshot
            .data
            .model_endpoint_capabilities
            .iter()
            .all(|capability| capability.upstream_model_id != "gemini-4-hydrogen")
    );
}

#[test]
fn current_reader_preserves_exact_qwen4_messages_plan_associations() {
    let snapshot = current();
    let catalog = &snapshot.data.metadata_catalog;
    let bindings: Vec<_> = catalog
        .endpoint_bindings
        .iter()
        .filter(|binding| {
            binding.model_key == "qwen-4-max-preview"
                && binding
                    .interface_candidates
                    .iter()
                    .any(|key| key.ends_with("/anthropic-messages"))
        })
        .collect();
    assert_eq!(bindings.len(), 2);
    for product_key in [
        "bailian-token-personal-cn-beijing",
        "bailian-token-team-cn-beijing",
    ] {
        let binding = bindings
            .iter()
            .find(|binding| binding.product_key == product_key)
            .unwrap();
        assert_eq!(binding.upstream_model_id, "qwen4-max-preview");
        assert_eq!(
            binding.interface_candidates,
            [format!("{product_key}/anthropic-messages")]
        );
        assert_eq!(binding.availability.state, "conditional");
        assert_eq!(binding.protocol_qualification, "runtime-required");
        assert!(
            binding
                .availability
                .condition
                .contains("2026-11-25T16:00:00Z")
        );
    }
    assert!(
        snapshot
            .data
            .pricing_facts
            .iter()
            .all(|price| price.model_key != "qwen-4-max-preview")
    );
}
