use super::*;

fn catalog() -> super::super::TrustedReleaseCatalog {
    super::super::TrustedReleaseCatalog::load_release_facts(
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
    .with_embedded_official_data(
        serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/release-facts/current/bundle/official-model-data.json"
        )))
        .unwrap(),
    )
    .unwrap()
}

fn binding() -> DecisionCredentialBindingV1 {
    let connection = serde_json::from_value(serde_json::json!({
        "kind":"native", "provider":"openai", "wire_protocol":"openai_decisions",
        "model":"gpt-6-luna", "endpoint":"https://api.openai.com/v1/decisions", "timeout_ms":10000,
        "authentication":{"kind":"shared_source","source_id":"source/decision-price",
            "source_lineage_digest":CanonicalDigest::of_bytes(b"source")}
    }))
    .unwrap();
    DecisionCredentialBindingV1::for_connection("decision-price", 1, &connection)
        .unwrap()
        .unwrap()
}

#[test]
fn official_decisions_reserve_conservatively_and_settle_the_actual_input_tier_without_output_charges()
 {
    let catalog = catalog();
    let binding = binding();
    let short = catalog
        .decision_spend_estimates(&binding, 1000, 1000)
        .remove("USD")
        .unwrap();
    assert_eq!(short.upper_bound_micros(), Some(125));
    assert_eq!(
        short.pricing.quote.as_ref().unwrap().rates.output,
        TokenRateV1::known(0)
    );
    assert_eq!(
        short.usage_micros(&KeySpendUsageV1 {
            input_tokens: Some(100),
            output_tokens: Some(99_999),
            cache_read_tokens: Some(30),
            cache_write_tokens: Some(20),
            reasoning_tokens: None,
        }),
        Some(8)
    );
    let long = catalog
        .decision_spend_estimates(&binding, 300_000, 1000)
        .remove("USD")
        .unwrap();
    assert_eq!(long.upper_bound_micros(), Some(75_000));
    let usage = |input| KeySpendUsageV1 {
        input_tokens: Some(input),
        output_tokens: None,
        cache_read_tokens: Some(0),
        cache_write_tokens: Some(0),
        reasoning_tokens: None,
    };
    assert_eq!(long.usage_micros(&usage(272_000)), Some(27_200));
    assert_eq!(long.usage_micros(&usage(272_001)), Some(54_401));
    assert_eq!(long.usage_micros(&usage(100)), Some(10));
    assert_eq!(
        long.pricing.quote.as_ref().unwrap().actual_source_ref,
        binding.source_id
    );
    assert!(matches!(
        long.pricing.quote.unwrap().exact_target.model_identity,
        PriceModelIdentityV1::LocalModel(_)
    ));
}

#[test]
fn unrelated_endpoints_models_codecs_and_unsupported_conditions_never_borrow_inference_prices() {
    let catalog = catalog();
    for (endpoint, protocol, model) in [
        (
            "https://proxy.example/v1/decisions",
            DecisionWireProtocolV2::OpenaiDecisions,
            "gpt-6-luna",
        ),
        (
            "https://api.openai.com/v1/responses",
            DecisionWireProtocolV2::OpenaiDecisions,
            "gpt-6-luna",
        ),
        (
            "https://api.openai.com/v1/decisions",
            DecisionWireProtocolV2::SystemOne,
            "gpt-6-luna",
        ),
        (
            "https://api.openai.com/v1/decisions",
            DecisionWireProtocolV2::OpenaiDecisions,
            "unknown-model",
        ),
    ] {
        let mut binding = binding();
        binding.endpoint = endpoint.into();
        binding.wire_protocol = protocol;
        binding.model = model.into();
        assert!(
            catalog
                .decision_spend_estimates(&binding, 1000, 1000)
                .is_empty()
        );
    }
    let mut changed = catalog.clone();
    let price = changed
        .official_snapshot
        .as_mut()
        .unwrap()
        .data
        .pricing_facts
        .iter_mut()
        .find(|p| p.product_key == "openai-platform-global" && p.upstream_model_id == "gpt-6-luna")
        .unwrap();
    price.schedules[0]
        .condition
        .insert("processing".into(), serde_json::json!("unknown"));
    changed.rebuild_decision_spending_index();
    assert!(
        changed
            .decision_spend_estimates(&binding(), 1000, 1000)
            .is_empty()
    );
}

#[test]
fn captured_prices_survive_catalog_refresh_and_resolution_does_not_enumerate_other_prices() {
    let mut catalog = catalog();
    let binding = binding();
    let captured = catalog.decision_spend_estimates(&binding, 1000, 1000);
    let prices = &mut catalog
        .official_snapshot
        .as_mut()
        .unwrap()
        .data
        .pricing_facts;
    let mut unrelated = prices[0].clone();
    unrelated.product_key = "unrelated".into();
    prices.extend(std::iter::repeat_n(unrelated, 10_000));
    catalog.rebuild_decision_spending_index();
    assert_eq!(
        catalog.decision_spend_estimates(&binding, 1000, 1000),
        captured
    );
    assert_eq!(catalog.decision_spending_index.prices.len(), 1);
}
