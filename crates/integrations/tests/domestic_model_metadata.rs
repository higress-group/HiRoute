use hiroute_domain::OfficialModelSnapshotV1;
use hiroute_integrations::TrustedReleaseCatalog;

fn current() -> OfficialModelSnapshotV1 {
    let manifest = include_bytes!("../../../assets/release-facts/current/bundle/manifest.json");
    TrustedReleaseCatalog::load_bundled_release_facts(
        manifest,
        manifest,
        include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json"),
        include_bytes!("../../../assets/release-facts/current/bundle/model-data.json"),
    )
    .expect("accept the exact current bundled catalog")
    .with_embedded_official_data(
        serde_json::from_slice(include_bytes!(
            "../../../assets/release-facts/current/bundle/official-model-data.json"
        ))
        .unwrap(),
    )
    .expect("accept the matching official fallback")
    .official_snapshot()
}

#[test]
fn current_reader_retains_regional_prices_without_executable_entitlement() {
    let snapshot = current();
    snapshot.validate().unwrap();
    let data = &snapshot.data;
    let prices: Vec<_> = data
        .pricing_facts
        .iter()
        .filter(|price| price.product_key == "minimax-payg-cn")
        .collect();
    assert_eq!(prices.len(), 2);
    assert!(prices.iter().all(|price| price.currency == "CNY"));
    let flash = prices
        .iter()
        .find(|price| price.upstream_model_id == "MiniMax-M3.1-Flash")
        .unwrap();
    assert_eq!(flash.schedules.len(), 1);
    assert_eq!(flash.schedules[0].rates["input"], "1.00");
    assert_eq!(flash.schedules[0].rates["output"], "4.00");
    assert_eq!(flash.schedules[0].rates["cache-read"], "0.10");
    let m3 = prices
        .iter()
        .find(|price| price.upstream_model_id == "MiniMax-M3")
        .unwrap();
    assert_eq!(m3.schedules.len(), 4);
    for schedule in &m3.schedules {
        assert_eq!(schedule.condition["threshold_token_unit"], "unresolved");
        assert!(!schedule.rates.contains_key("cache-write"));
    }
    for id in ["MiniMax-M3", "MiniMax-M3.1-Flash"] {
        let bindings: Vec<_> = data
            .metadata_catalog
            .endpoint_bindings
            .iter()
            .filter(|binding| {
                binding.product_key == "minimax-payg-cn" && binding.upstream_model_id == id
            })
            .collect();
        assert_eq!(bindings.len(), 3);
        for binding in bindings {
            assert_eq!(binding.availability.state, "conditional");
            assert_eq!(binding.protocol_qualification, "runtime-required");
            assert_eq!(
                binding.capability_overrides.max_output_tokens,
                binding
                    .interface_candidates
                    .iter()
                    .any(|key| key == "minimax-payg-cn/openai-chat")
                    .then_some(524_288)
            );
        }
        assert!(
            data.model_endpoint_capabilities
                .iter()
                .all(|capability| capability.upstream_model_id != id),
            "documentation must not grant a registered executable binding"
        );
    }
}

#[test]
fn current_reader_keeps_direct_lifecycle_separate_from_hosted_models() {
    let snapshot = current();
    let records = &snapshot.data.metadata_catalog.model_records;
    for (key, lifecycle, replacement) in [
        (
            "openclaw/mistral/mistral-medium-3-5",
            "deprecated",
            "mistral-large-4",
        ),
        (
            "hermes-agent/anthropic/claude-sonnet-4-5-20250929",
            "deprecated",
            "claude-sonnet-5-5",
        ),
        (
            "hermes-agent/anthropic/claude-opus-4-20250514",
            "retired",
            "claude-opus-4-8",
        ),
        (
            "hermes-agent/anthropic/claude-sonnet-4-20250514",
            "retired",
            "claude-sonnet-5-5",
        ),
        (
            "openclaw/google/gemini-2.5-pro",
            "deprecated",
            "gemini-3.8-flash",
        ),
        (
            "openclaw/google/gemini-2.5-flash",
            "deprecated",
            "gemini-3.5-flash-lite",
        ),
    ] {
        let record = records
            .iter()
            .find(|record| record.model_record_key == key)
            .unwrap();
        assert_eq!(record.lifecycle, lifecycle, "{key}");
        assert_eq!(record.replacement_upstream_ids, [replacement], "{key}");
    }
    let hosted = records
        .iter()
        .find(|record| record.model_record_key == "hermes-agent/copilot/gemini-2.5-pro")
        .unwrap();
    assert_eq!(hosted.lifecycle, "active");
    assert!(
        hosted
            .provenance_refs
            .iter()
            .all(|reference| reference.source_key != "source-google-deprecations-20261112")
    );
}
