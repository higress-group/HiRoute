//! The shipped subscription bundle must qualify exact Claude inventory, not API aliases
//! or a fixture catalog. Product regressions separately exercise saved routing and Gateway.
use hiroute_domain::{InventoryDisposition, NativeReasoningCapabilityV1, ObservedModelV1};
use hiroute_integrations::TrustedReleaseCatalog;

fn bundled_catalog() -> TrustedReleaseCatalog {
    const MANIFEST: &[u8] =
        include_bytes!("../../../assets/release-facts/current/bundle/manifest.json");
    TrustedReleaseCatalog::load_bundled_release_facts(
        MANIFEST,
        MANIFEST,
        include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json"),
        include_bytes!("../../../assets/release-facts/current/bundle/model-data.json"),
    )
    .unwrap()
}

#[test]
fn bundled_claude_subscription_inventory_retains_reviewed_capabilities_and_exact_aliases() {
    let catalog = bundled_catalog();
    let expected = [
        ("claude-opus-5", "model.anthropic.claude-opus-5"),
        ("claude-sonnet-5", "model.anthropic.claude-sonnet-5"),
        ("claude-fable-5-1", "model.anthropic.claude-fable-5-1"),
        ("claude-opus-5-5", "model.anthropic.claude-opus-5-5"),
        ("claude-sonnet-5-5", "model.anthropic.claude-sonnet-5-5"),
        ("claude-haiku-5-5", "model.anthropic.claude-haiku-5-5"),
        ("claude-haiku-4-5", "model.anthropic.claude-haiku-4-5"),
        (
            "claude-haiku-4-5-20251001",
            "model.anthropic.claude-haiku-4-5",
        ),
    ];
    for (upstream_model_id, configuration_id) in expected {
        let observed = || ObservedModelV1 {
            upstream_model_id: upstream_model_id.into(),
            metadata: Default::default(),
        };
        let inventory = catalog
            .reconcile_observed_inventory("endpoint.cpa.claude", [observed()])
            .unwrap();
        assert_eq!(inventory.len(), 1);
        assert_eq!(
            inventory[0].disposition,
            InventoryDisposition::CatalogMatched,
            "{upstream_model_id} must not silently use text-only runtime fallback"
        );
        assert_eq!(
            inventory[0].model_configuration_id.as_deref(),
            Some(configuration_id)
        );
        let model = catalog.model_data().model(configuration_id).unwrap();
        assert!(model.capabilities.vision, "{upstream_model_id}");
        assert!(
            model.capabilities.context_tokens > 131_072,
            "{upstream_model_id}"
        );
        assert!(
            model.capabilities.max_output_tokens > 8_192,
            "{upstream_model_id}"
        );
        assert!(
            catalog
                .native_reasoning()
                .iter()
                .any(|entry| entry.model_configuration_id == configuration_id),
            "{upstream_model_id} needs maintained native reasoning facts"
        );
        let foreign = catalog
            .reconcile_observed_inventory("endpoint.cpa.codex", [observed()])
            .unwrap();
        assert_eq!(foreign[0].disposition, InventoryDisposition::InventoryOnly);
    }
    // Unregistered inventory must not borrow a different model's reviewed capabilities.
    let unreviewed = catalog
        .reconcile_observed_inventory(
            "endpoint.cpa.claude",
            [ObservedModelV1 {
                upstream_model_id: "claude-unregistered-fixture".into(),
                metadata: Default::default(),
            }],
        )
        .unwrap();
    assert_eq!(
        unreviewed[0].disposition,
        InventoryDisposition::InventoryOnly
    );
    assert!(unreviewed[0].model_configuration_id.is_none());

    let reasoning = |id: &str| {
        &catalog
            .native_reasoning()
            .iter()
            .find(|entry| entry.model_configuration_id == id)
            .unwrap()
            .capability
    };
    assert_eq!(
        reasoning("model.anthropic.claude-haiku-4-5"),
        &NativeReasoningCapabilityV1::Toggle {
            parameter: "enable_thinking".into(),
        }
    );
    assert!(matches!(
        reasoning("model.anthropic.claude-sonnet-5"),
        NativeReasoningCapabilityV1::Discrete { parameter, profiles, .. }
            if parameter == "output_config.effort" && profiles.iter().any(|profile| profile == "high")
    ));
}
